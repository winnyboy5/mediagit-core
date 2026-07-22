// MediaGit - Git for Media Files
// Copyright (C) 2025 MediaGit Contributors
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU Affero General Public License as published
// by the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE. See the
// GNU Affero General Public License for more details.

//! Checkout operations for restoring working directory from commits
//!
//! This module provides functionality to update the working directory
//! to match a specific commit's tree structure. Per-file ODB I/O is
//! parallelized (env knob `MEDIAGIT_CHECKOUT_PARALLELISM`, default = CPUs
//! capped at 8).

use crate::sparse::SparseFilter;
use crate::{Commit, FileMode, ObjectDatabase, Oid, Tree, is_stage_debris_key};
use anyhow::{Context, Result};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::Semaphore;
use tokio::task::JoinSet;
use tracing::{debug, info, warn};

/// Bounded parallelism for checkout I/O (ODB reads/writes per file).
///
/// `MEDIAGIT_CHECKOUT_PARALLELISM` overrides the default (number of CPUs,
/// capped at 8) — copies the knob pattern used by `add.rs`'s parallel file
/// processing.
fn checkout_parallelism() -> usize {
    std::env::var("MEDIAGIT_CHECKOUT_PARALLELISM")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .filter(|&n| n > 0)
        .unwrap_or_else(|| num_cpus::get().min(8))
}

/// Write one file/symlink entry from the ODB to disk. Standalone (not `&self`)
/// so it can run inside a spawned tokio task.
///
/// `symlink_write_as_file_on_non_unix` selects between the two pre-existing
/// non-Unix symlink behaviors in this module: `checkout_tree` historically
/// skipped symlinks entirely on non-Unix (just logged), while
/// `checkout_tree_optimized`/`checkout_single_file` wrote the symlink target
/// as a regular file. Both are preserved exactly per call site.
async fn write_entry_to_disk(
    odb: &ObjectDatabase,
    full_path: &Path,
    oid: &Oid,
    mode: FileMode,
    _symlink_write_as_file_on_non_unix: bool,
) -> Result<()> {
    if let Some(parent) = full_path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("Failed to create directory: {}", parent.display()))?;
    }

    match mode {
        FileMode::Regular | FileMode::Executable => {
            odb.read_to_file(oid, full_path)
                .await
                .with_context(|| format!("Failed to checkout file: {}", full_path.display()))?;

            #[cfg(unix)]
            if mode == FileMode::Executable {
                use std::os::unix::fs::PermissionsExt;
                let mut perms = fs::metadata(full_path)?.permissions();
                perms.set_mode(0o755);
                fs::set_permissions(full_path, perms)?;
            }

            debug!("Checked out file: {}", full_path.display());
        }
        FileMode::Symlink => {
            let target_data = odb
                .read(oid)
                .await
                .with_context(|| format!("Failed to read symlink blob: {}", oid))?;
            let target =
                String::from_utf8(target_data).context("Symlink target is not valid UTF-8")?;

            #[cfg(unix)]
            {
                use std::os::unix::fs::symlink;
                let _ = fs::remove_file(full_path);
                symlink(&target, full_path).with_context(|| {
                    format!("Failed to create symlink: {}", full_path.display())
                })?;
            }

            #[cfg(not(unix))]
            {
                if _symlink_write_as_file_on_non_unix {
                    fs::write(full_path, target.as_bytes()).with_context(|| {
                        format!("Failed to write symlink file: {}", full_path.display())
                    })?;
                } else {
                    debug!(
                        "Symlinks not supported on this platform, skipping: {}",
                        full_path.display()
                    );
                }
            }
        }
        FileMode::Directory => {
            // Directories are flattened away before this point.
        }
    }

    Ok(())
}

/// Differential write: skip if the on-disk file already matches `oid` (size
/// and streaming hash). Only applies to Regular/Executable — mirrors the
/// original `checkout_tree_optimized` skip-check exactly.
///
/// Returns `Ok(true)` if a write happened, `Ok(false)` if skipped unchanged.
async fn checkout_entry_differential(
    odb: &ObjectDatabase,
    full_path: &Path,
    oid: &Oid,
    mode: FileMode,
) -> Result<bool> {
    if matches!(mode, FileMode::Regular | FileMode::Executable)
        && full_path.exists()
        && let Ok(metadata) = fs::metadata(full_path)
        && let Ok(expected_size) = odb.get_object_size(oid).await
        && metadata.len() == expected_size as u64
        && let Ok(file_oid) = Oid::from_file(full_path)
        && file_oid == *oid
    {
        debug!("Skipped unchanged file: {}", full_path.display());
        return Ok(false);
    }

    write_entry_to_disk(odb, full_path, oid, mode, true).await?;
    Ok(true)
}

/// Checkout manager for working directory operations
pub struct CheckoutManager<'a> {
    odb: &'a ObjectDatabase,
    repo_root: PathBuf,
    sparse: SparseFilter,
}

impl<'a> CheckoutManager<'a> {
    /// Create a new checkout manager
    pub fn new(odb: &'a ObjectDatabase, repo_root: impl Into<PathBuf>) -> Self {
        let repo_root = repo_root.into();
        let sparse = SparseFilter::load(&repo_root).unwrap_or_else(|e| {
            warn!("Failed to load sparse-checkout filter, treating as disabled: {e}");
            SparseFilter::disabled()
        });
        Self {
            odb,
            repo_root,
            sparse,
        }
    }

    /// Drop entries excluded by the active sparse-checkout filter from a flat
    /// tree map. Applied once per flattening call site (checkout_tree,
    /// checkout_tree_optimized, checkout_diff) rather than per-hook, since M3
    /// rewrote all three into a flatten-then-parallel shape.
    fn filter_sparse<V>(&self, mut flat: HashMap<PathBuf, V>) -> HashMap<PathBuf, V> {
        if self.sparse.is_enabled() {
            flat.retain(|path, _| self.sparse.is_included(path));
        }
        flat
    }

    /// List all tracked files (path -> (oid, mode)) at `commit_oid`,
    /// **ignoring** the sparse filter — used by the `sparse-checkout`
    /// command to compute which files to materialize/remove when patterns
    /// change (it needs the full tree, not the currently-filtered view).
    pub async fn tracked_files_at(
        &self,
        commit_oid: &Oid,
    ) -> Result<HashMap<PathBuf, (Oid, FileMode)>> {
        let commit = Commit::read(self.odb, commit_oid).await?;
        self.get_tree_files_with_oid(&commit.tree, Path::new(""))
            .await
    }

    /// Write a single tracked file to disk directly, bypassing the sparse
    /// filter — used by the `sparse-checkout set`/`disable` commands to
    /// explicitly materialize a newly-included file.
    pub async fn materialize_file(&self, rel_path: &Path, oid: &Oid, mode: FileMode) -> Result<()> {
        let full_path = self.repo_root.join(rel_path);
        write_entry_to_disk(self.odb, &full_path, oid, mode, true).await
    }

    /// Checkout a commit, updating the working directory to match its tree
    ///
    /// This operation:
    /// 1. Removes files not in the target commit
    /// 2. Writes all files from the target commit's tree
    /// 3. Preserves the .mediagit directory
    ///
    /// # Arguments
    ///
    /// * `commit_oid` - The commit to checkout
    ///
    /// # Returns
    ///
    /// Number of files updated
    pub async fn checkout_commit(&self, commit_oid: &Oid) -> Result<usize> {
        info!("Checking out commit: {}", commit_oid);

        // Read the commit
        let commit = Commit::read(self.odb, commit_oid).await?;

        debug!("Commit tree: {}", commit.tree);

        // Optimized: Single-pass checkout that collects files and writes them
        // This eliminates the redundant tree traversal
        let (target_files, files_updated) = self
            .checkout_tree_optimized(&commit.tree, Path::new(""))
            .await?;
        debug!("Target files: {} entries", target_files.len());

        // Clean working directory (remove files not in target)
        self.clean_working_directory(&target_files)?;

        info!("Checked out {} files", files_updated);
        Ok(files_updated)
    }

    /// Get all file paths from a tree recursively
    #[allow(dead_code)]
    fn get_tree_files<'b>(
        &'b self,
        tree_oid: &'b Oid,
        prefix: &'b Path,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<HashSet<PathBuf>>> + 'b>> {
        Box::pin(async move {
            let tree = Tree::read(self.odb, tree_oid).await?;

            let mut files = HashSet::new();

            for entry in tree.iter() {
                let entry_path = prefix.join(&entry.name);

                match entry.mode {
                    FileMode::Regular | FileMode::Executable | FileMode::Symlink => {
                        files.insert(entry_path);
                    }
                    FileMode::Directory => {
                        // Recursively get files from subdirectory
                        let subdir_files = self.get_tree_files(&entry.oid, &entry_path).await?;
                        files.extend(subdir_files);
                    }
                }
            }

            Ok(files)
        })
    }

    /// Clean working directory, removing files not in target set
    fn clean_working_directory(&self, target_files: &HashSet<PathBuf>) -> Result<()> {
        debug!("Cleaning working directory");

        // Normalize target paths to use forward slashes for consistent comparison
        // This handles cross-platform path separator differences
        let normalized_target: HashSet<String> = target_files
            .iter()
            .map(|p| p.to_string_lossy().replace('\\', "/"))
            .collect();

        // Get all files in working directory (excluding .mediagit)
        let existing_files = self.list_working_directory_files()?;

        for file in existing_files {
            // Normalize existing file path for comparison
            let file_normalized = file.to_string_lossy().replace('\\', "/");

            if !normalized_target.contains(&file_normalized) {
                if !self.sparse.is_included(&file) {
                    // Outside the sparse cone: absence is the expected state,
                    // presence is simply untouched — never deleted by checkout.
                    continue;
                }
                let file_path = self.repo_root.join(&file);
                debug!("Removing file not in target: {}", file.display());

                if file_path.exists() {
                    fs::remove_file(&file_path).with_context(|| {
                        format!("Failed to remove file: {}", file_path.display())
                    })?;
                }
            }
        }

        // Remove empty directories
        self.remove_empty_directories()?;

        Ok(())
    }

    /// List all files in working directory (excluding .mediagit)
    fn list_working_directory_files(&self) -> Result<Vec<PathBuf>> {
        let mut files = Vec::new();
        self.list_files_recursive(&self.repo_root, &mut files)?;
        Ok(files)
    }

    /// Recursively list files in a directory
    fn list_files_recursive(&self, dir: &Path, files: &mut Vec<PathBuf>) -> Result<()> {
        if !dir.exists() || !dir.is_dir() {
            return Ok(());
        }

        for entry in fs::read_dir(dir)? {
            let entry = entry?;
            let path = entry.path();

            // Skip .mediagit directory
            if path.file_name().and_then(|n| n.to_str()) == Some(".mediagit") {
                continue;
            }

            if path.is_file() {
                // Store relative path from repo root
                let rel_path = path
                    .strip_prefix(&self.repo_root)
                    .context("Failed to compute relative path")?;
                files.push(rel_path.to_path_buf());
            } else if path.is_dir() {
                self.list_files_recursive(&path, files)?;
            }
        }

        Ok(())
    }

    /// Remove empty directories in working tree
    ///
    /// Recursively removes all empty directories to match Git behavior.
    /// Directories are only removed if they contain no files and no non-empty subdirectories.
    fn remove_empty_directories(&self) -> Result<()> {
        // Improved: Recursively clean until no more empty dirs
        // Multiple passes handle nested empty directories
        loop {
            let mut removed_any = false;
            self.try_remove_empty_dirs(&self.repo_root, &mut removed_any)?;

            if !removed_any {
                break; // No more empty dirs to remove
            }
        }
        Ok(())
    }

    /// Try to remove empty directories recursively
    ///
    /// Returns true if directory still exists (has contents or couldn't be removed)
    fn try_remove_empty_dirs(&self, dir: &Path, removed_any: &mut bool) -> Result<bool> {
        if !dir.exists() || !dir.is_dir() {
            return Ok(false); // Directory doesn't exist
        }

        // Don't try to remove the repo root
        if dir == self.repo_root {
            // But still process its contents
            for entry in fs::read_dir(dir)? {
                let entry = entry?;
                let path = entry.path();

                // Skip .mediagit directory
                if path.file_name().and_then(|n| n.to_str()) == Some(".mediagit") {
                    continue;
                }

                if path.is_dir() {
                    self.try_remove_empty_dirs(&path, removed_any)?;
                }
            }
            return Ok(true); // Repo root always "has contents"
        }

        let mut has_contents = false;

        // First pass: recursively process subdirectories
        let entries: Vec<_> = fs::read_dir(dir)?.filter_map(|e| e.ok()).collect();

        for entry in &entries {
            let path = entry.path();

            // Skip .mediagit directory
            if path.file_name().and_then(|n| n.to_str()) == Some(".mediagit") {
                continue;
            }

            if path.is_dir() {
                // Recursively try to remove subdirectories
                if self.try_remove_empty_dirs(&path, removed_any)? {
                    has_contents = true;
                }
            } else {
                // Directory contains files
                has_contents = true;
            }
        }

        // Remove directory if empty (re-check after subdirectory processing)
        if !has_contents {
            // On Windows, there can be timing issues with file handles
            // Retry a few times with small delays
            let mut attempts = 0;
            const MAX_ATTEMPTS: u32 = 3;

            loop {
                match fs::remove_dir(dir) {
                    Ok(_) => {
                        debug!("Removed empty directory: {}", dir.display());
                        *removed_any = true;
                        return Ok(false); // Successfully removed
                    }
                    Err(e) => {
                        attempts += 1;
                        if attempts >= MAX_ATTEMPTS {
                            // Log but don't fail (might be permission issue or file locks)
                            debug!(
                                "Failed to remove empty directory {} after {} attempts: {}",
                                dir.display(),
                                attempts,
                                e
                            );
                            return Ok(true); // Still has directory (couldn't remove)
                        }
                        // Small delay before retry (Windows file handle release)
                        std::thread::sleep(std::time::Duration::from_millis(50));
                    }
                }
            }
        } else {
            Ok(true) // Has contents
        }
    }

    /// Checkout a tree, writing all files to the working directory.
    ///
    /// Flattens the tree (via [`Self::get_tree_files_with_oid`]) then writes
    /// every entry with bounded parallelism (`MEDIAGIT_CHECKOUT_PARALLELISM`).
    /// Always writes (no differential skip) — matches the original semantics.
    async fn checkout_tree(&self, tree_oid: &Oid, prefix: &Path) -> Result<usize> {
        let flat = self.get_tree_files_with_oid(tree_oid, prefix).await?;
        let flat = self.filter_sparse(flat);
        let entries: Vec<(PathBuf, Oid, FileMode)> =
            flat.into_iter().map(|(p, (o, m))| (p, o, m)).collect();
        self.run_parallel_writes(entries, false, false).await
    }

    /// Run a batch of entry writes with bounded parallelism (JoinSet +
    /// Semaphore, gated by `MEDIAGIT_CHECKOUT_PARALLELISM`). First error
    /// aborts all remaining tasks and fails the whole batch — no
    /// partial-silent success.
    ///
    /// `differential` enables the skip-if-unchanged check (Regular/Executable
    /// only); `symlink_write_as_file_on_non_unix` is forwarded to
    /// [`write_entry_to_disk`]. Returns the number of files actually written.
    ///
    /// # Invariant (F1+F2)
    ///
    /// A failed checkout returns only after all spawned I/O has stopped —
    /// on error we `abort_all()` and then drain the `JoinSet` to completion
    /// before returning, so no aborted task's `write_all` can land after
    /// this function has already reported failure. Combined with
    /// [`ObjectDatabase::read_to_file`]'s tmp-file-then-rename writes, no
    /// partial file can exist at a final path — at worst a stale `.mgtmp`
    /// sibling, which the next checkout of that file overwrites.
    async fn run_parallel_writes(
        &self,
        entries: Vec<(PathBuf, Oid, FileMode)>,
        differential: bool,
        symlink_write_as_file_on_non_unix: bool,
    ) -> Result<usize> {
        if entries.is_empty() {
            return Ok(0);
        }

        let semaphore = Arc::new(Semaphore::new(checkout_parallelism()));
        let mut tasks: JoinSet<Result<bool>> = JoinSet::new();

        for (path, oid, mode) in entries {
            let sem = semaphore.clone();
            let odb = self.odb.clone();
            let full_path = self.repo_root.join(&path);
            tasks.spawn(async move {
                let _permit = sem
                    .acquire_owned()
                    .await
                    .map_err(|_| anyhow::anyhow!("checkout semaphore closed"))?;
                if differential {
                    checkout_entry_differential(&odb, &full_path, &oid, mode).await
                } else {
                    write_entry_to_disk(
                        &odb,
                        &full_path,
                        &oid,
                        mode,
                        symlink_write_as_file_on_non_unix,
                    )
                    .await?;
                    Ok(true)
                }
            });
        }

        let mut updated = 0usize;
        while let Some(res) = tasks.join_next().await {
            match res {
                Ok(Ok(wrote)) => {
                    if wrote {
                        updated += 1;
                    }
                }
                Ok(Err(e)) => {
                    tasks.abort_all();
                    while tasks.join_next().await.is_some() {}
                    return Err(e);
                }
                Err(join_err) => {
                    tasks.abort_all();
                    while tasks.join_next().await.is_some() {}
                    return Err(anyhow::anyhow!("checkout task failed: {join_err}"));
                }
            }
        }

        Ok(updated)
    }

    /// Delete a batch of files with bounded parallelism. First error aborts
    /// remaining deletes. Returns the number of files actually deleted.
    async fn run_parallel_deletes(&self, paths: Vec<PathBuf>) -> Result<usize> {
        if paths.is_empty() {
            return Ok(0);
        }

        let semaphore = Arc::new(Semaphore::new(checkout_parallelism()));
        let mut tasks: JoinSet<Result<()>> = JoinSet::new();

        for full_path in paths {
            let sem = semaphore.clone();
            tasks.spawn(async move {
                let _permit = sem
                    .acquire_owned()
                    .await
                    .map_err(|_| anyhow::anyhow!("checkout semaphore closed"))?;
                fs::remove_file(&full_path)
                    .with_context(|| format!("Failed to delete: {}", full_path.display()))
            });
        }

        let mut deleted = 0usize;
        while let Some(res) = tasks.join_next().await {
            match res {
                Ok(Ok(())) => deleted += 1,
                Ok(Err(e)) => {
                    tasks.abort_all();
                    while tasks.join_next().await.is_some() {}
                    return Err(e);
                }
                Err(join_err) => {
                    tasks.abort_all();
                    while tasks.join_next().await.is_some() {}
                    return Err(anyhow::anyhow!("checkout delete task failed: {join_err}"));
                }
            }
        }

        Ok(deleted)
    }

    /// Optimized checkout: flattens the tree, then writes every entry with
    /// bounded parallelism, skipping files already on disk that match the
    /// target OID (differential fast path). Returns (file_paths,
    /// files_updated) for cleanup and counting.
    async fn checkout_tree_optimized(
        &self,
        tree_oid: &Oid,
        prefix: &Path,
    ) -> Result<(HashSet<PathBuf>, usize)> {
        let flat = self.get_tree_files_with_oid(tree_oid, prefix).await?;
        let flat = self.filter_sparse(flat);
        let file_paths: HashSet<PathBuf> = flat.keys().cloned().collect();
        let entries: Vec<(PathBuf, Oid, FileMode)> =
            flat.into_iter().map(|(p, (o, m))| (p, o, m)).collect();
        let files_updated = self.run_parallel_writes(entries, true, true).await?;
        Ok((file_paths, files_updated))
    }

    /// Apply a commit's tree on top of the current working directory without cleaning.
    ///
    /// Unlike `checkout_commit`, this does NOT remove files that aren't in the target tree.
    /// This is used for stash apply, which should overlay stashed files on top of HEAD.
    pub async fn apply_tree_overlay(&self, commit_oid: &Oid) -> Result<usize> {
        info!("Applying tree overlay from commit: {}", commit_oid);
        let commit = Commit::read(self.odb, commit_oid).await?;
        self.checkout_tree(&commit.tree, Path::new("")).await
    }

    /// Checkout to an empty working directory
    ///
    /// Useful for initial clone or reset operations
    pub async fn checkout_fresh(&self, commit_oid: &Oid) -> Result<usize> {
        info!("Performing fresh checkout of commit: {}", commit_oid);

        // Read the commit
        let commit = Commit::read(self.odb, commit_oid).await?;

        // Checkout tree without cleaning (assume empty directory)
        self.checkout_tree(&commit.tree, Path::new("")).await
    }

    /// Differential checkout - only update changed files
    ///
    /// This is the fast path for branch switching when most files are unchanged.
    /// Compares the current tree with the target tree and only updates files
    /// that have different OIDs, skipping unchanged files entirely.
    ///
    /// # Arguments
    ///
    /// * `from_commit_oid` - The current commit (what's currently checked out)
    /// * `to_commit_oid` - The target commit to checkout
    ///
    /// # Returns
    ///
    /// The number of files that were actually updated
    ///
    /// # Performance
    ///
    /// For branches with identical content, this completes in < 1s regardless
    /// of repository size, as no file I/O is performed for unchanged files.
    pub async fn checkout_diff(
        &self,
        from_commit_oid: &Oid,
        to_commit_oid: &Oid,
    ) -> Result<CheckoutStats> {
        use std::time::Instant;
        let start = Instant::now();

        info!(
            "Differential checkout: {} -> {}",
            from_commit_oid, to_commit_oid
        );

        // Early exit if same commit
        if from_commit_oid == to_commit_oid {
            info!("Same commit, nothing to do");
            return Ok(CheckoutStats {
                files_added: 0,
                files_modified: 0,
                files_deleted: 0,
                files_unchanged: 0,
                elapsed_ms: start.elapsed().as_millis() as u64,
            });
        }

        // Read both commits
        let from_commit = Commit::read(self.odb, from_commit_oid).await?;
        let to_commit = Commit::read(self.odb, to_commit_oid).await?;

        // Fast path if same tree: still need to verify each file exists on disk,
        // since a manually deleted file must be re-materialized (bug #23).
        // Classification (including the exists() check) stays sequential —
        // it's cheap (stat only); only the actual restores run in parallel.
        if from_commit.tree == to_commit.tree {
            let to_files = self
                .get_tree_files_with_oid(&to_commit.tree, Path::new(""))
                .await?;
            let to_files = self.filter_sparse(to_files);

            let mut to_restore = Vec::new();
            let mut files_unchanged = 0usize;
            for (path, (oid, mode)) in &to_files {
                let full_path = self.repo_root.join(path);
                if full_path.exists() {
                    files_unchanged += 1;
                } else {
                    to_restore.push((path.clone(), *oid, *mode));
                }
            }

            let files_added = self.run_parallel_writes(to_restore, false, true).await?;

            let mut stats = CheckoutStats {
                files_added,
                files_modified: 0,
                files_deleted: 0,
                files_unchanged,
                elapsed_ms: 0,
            };

            info!("Same tree, {} file(s) restored", stats.files_added);
            stats.elapsed_ms = start.elapsed().as_millis() as u64;
            return Ok(stats);
        }

        // Get file mappings from both trees. Both sides are sparse-filtered
        // consistently so an excluded path never enters the add/modify/delete
        // classification below, whether or not it happens to exist on disk.
        let from_files = self
            .get_tree_files_with_oid(&from_commit.tree, Path::new(""))
            .await?;
        let from_files = self.filter_sparse(from_files);
        let to_files = self
            .get_tree_files_with_oid(&to_commit.tree, Path::new(""))
            .await?;
        let to_files = self.filter_sparse(to_files);

        // Classify every target-tree file (cheap, sequential, includes the
        // #23 exists() checks) before doing any parallel I/O.
        let mut to_add = Vec::new();
        let mut to_modify = Vec::new();
        let mut files_unchanged = 0usize;

        for (path, (to_oid, mode)) in &to_files {
            let full_path = self.repo_root.join(path);

            match from_files.get(path) {
                Some((from_oid, _)) if from_oid == to_oid => {
                    if full_path.exists() {
                        // File unchanged - skip
                        files_unchanged += 1;
                    } else {
                        // OID unchanged but manually deleted from disk - restore it
                        to_add.push((path.clone(), *to_oid, *mode));
                    }
                }
                Some(_) => {
                    // File modified - update it
                    to_modify.push((path.clone(), *to_oid, *mode));
                }
                None => {
                    // File added - create it
                    to_add.push((path.clone(), *to_oid, *mode));
                }
            }
        }

        // Files not in target tree - delete.
        let mut to_delete = Vec::new();
        for path in from_files.keys() {
            if !to_files.contains_key(path) {
                let full_path = self.repo_root.join(path);
                if full_path.exists() {
                    to_delete.push(full_path);
                }
            }
        }

        // Parallel I/O phases (bounded by MEDIAGIT_CHECKOUT_PARALLELISM).
        // First error in any phase aborts that phase's remaining tasks and
        // fails the whole checkout.
        let files_added = self.run_parallel_writes(to_add, false, true).await?;
        let files_modified = self.run_parallel_writes(to_modify, false, true).await?;
        let files_deleted = self.run_parallel_deletes(to_delete).await?;

        let mut stats = CheckoutStats {
            files_added,
            files_modified,
            files_deleted,
            files_unchanged,
            elapsed_ms: 0,
        };

        // Clean up empty directories
        self.remove_empty_directories()?;

        stats.elapsed_ms = start.elapsed().as_millis() as u64;

        info!(
            "Differential checkout complete: {} added, {} modified, {} deleted, {} unchanged in {}ms",
            stats.files_added,
            stats.files_modified,
            stats.files_deleted,
            stats.files_unchanged,
            stats.elapsed_ms
        );

        Ok(stats)
    }

    /// Get all files from a tree with their OIDs and modes
    ///
    /// Returns a map of path -> (OID, FileMode) for all files in the tree.
    #[allow(clippy::type_complexity)]
    fn get_tree_files_with_oid<'b>(
        &'b self,
        tree_oid: &'b Oid,
        prefix: &'b Path,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<HashMap<PathBuf, (Oid, FileMode)>>> + 'b>,
    > {
        Box::pin(async move {
            let tree = Tree::read(self.odb, tree_oid).await?;
            let mut files = HashMap::new();

            for entry in tree.iter() {
                if is_stage_debris_key(&entry.name) {
                    // Legacy poisoned tree from before the merge-conflict
                    // ::stageN debris was fixed at the source: skip rather
                    // than materialize an illegal colon path (breaks on
                    // Windows, os error 123).
                    warn!(
                        "Skipping stage-debris tree entry (not materialized): {}",
                        prefix.join(&entry.name).display()
                    );
                    continue;
                }
                let entry_path = prefix.join(&entry.name);

                match entry.mode {
                    FileMode::Regular | FileMode::Executable | FileMode::Symlink => {
                        files.insert(entry_path, (entry.oid, entry.mode));
                    }
                    FileMode::Directory => {
                        // Recursively get files from subdirectory
                        let subdir_files = self
                            .get_tree_files_with_oid(&entry.oid, &entry_path)
                            .await?;
                        files.extend(subdir_files);
                    }
                }
            }

            Ok(files)
        })
    }
}

/// Statistics from a differential checkout operation
#[derive(Debug, Clone, Default)]
pub struct CheckoutStats {
    /// Number of files that were added
    pub files_added: usize,
    /// Number of files that were modified
    pub files_modified: usize,
    /// Number of files that were deleted
    pub files_deleted: usize,
    /// Number of files that were unchanged (skipped)
    pub files_unchanged: usize,
    /// Time elapsed in milliseconds
    pub elapsed_ms: u64,
}

impl CheckoutStats {
    /// Total number of files changed (added + modified + deleted)
    pub fn files_changed(&self) -> usize {
        self.files_added + self.files_modified + self.files_deleted
    }

    /// Total number of files processed
    pub fn total_files(&self) -> usize {
        self.files_changed() + self.files_unchanged
    }
}

#[cfg(test)]
#[allow(unsafe_code)] // edition-2024: test-only env::set_var/remove_var requires unsafe
mod tests {
    use super::*;
    use crate::{ObjectType, Signature, TreeEntry};
    use mediagit_storage::LocalBackend;
    use std::sync::Arc;
    use tempfile::TempDir;

    #[tokio::test]
    async fn test_checkout_commit() -> Result<()> {
        let temp_dir = TempDir::new()?;
        let repo_root = temp_dir.path();
        let storage_path = repo_root.join(".mediagit");
        fs::create_dir_all(&storage_path)?;

        let storage = Arc::new(LocalBackend::new(&storage_path).await?);
        let odb = ObjectDatabase::new(storage, 100);

        // Create a test commit with a file
        let file_data = b"Hello, MediaGit!";
        let blob_oid = odb.write(ObjectType::Blob, file_data).await?;

        let mut tree = Tree::new();
        tree.add_entry(TreeEntry::new(
            "README.md".to_string(),
            FileMode::Regular,
            blob_oid,
        ));

        let tree_oid = tree.write(&odb).await?;

        let commit = Commit::new(
            tree_oid,
            Signature::now("Test".to_string(), "test@example.com".to_string()),
            Signature::now("Test".to_string(), "test@example.com".to_string()),
            "Initial commit".to_string(),
        );

        let commit_oid = commit.write(&odb).await?;

        // Checkout the commit
        let checkout_mgr = CheckoutManager::new(&odb, repo_root);
        let files_updated = checkout_mgr.checkout_commit(&commit_oid).await?;

        assert_eq!(files_updated, 1);

        // Verify file exists
        let file_path = repo_root.join("README.md");
        assert!(file_path.exists());

        let contents = fs::read(&file_path)?;
        assert_eq!(contents, file_data);

        Ok(())
    }

    #[tokio::test]
    async fn test_differential_checkout_same_commit() -> Result<()> {
        let temp_dir = TempDir::new()?;
        let repo_root = temp_dir.path();
        let storage_path = repo_root.join(".mediagit");
        fs::create_dir_all(&storage_path)?;

        let storage = Arc::new(LocalBackend::new(&storage_path).await?);
        let odb = ObjectDatabase::new(storage, 100);

        // Create a commit
        let blob_oid = odb.write(ObjectType::Blob, b"content").await?;
        let mut tree = Tree::new();
        tree.add_entry(TreeEntry::new(
            "file.txt".to_string(),
            FileMode::Regular,
            blob_oid,
        ));
        let tree_oid = tree.write(&odb).await?;

        let commit = Commit::new(
            tree_oid,
            Signature::now("Test".to_string(), "test@example.com".to_string()),
            Signature::now("Test".to_string(), "test@example.com".to_string()),
            "commit".to_string(),
        );
        let commit_oid = commit.write(&odb).await?;

        let checkout_mgr = CheckoutManager::new(&odb, repo_root);

        // Differential checkout same commit should complete instantly
        let stats = checkout_mgr.checkout_diff(&commit_oid, &commit_oid).await?;

        assert_eq!(stats.files_changed(), 0);
        assert!(stats.elapsed_ms < 100); // Should be near-instant

        Ok(())
    }

    #[tokio::test]
    async fn test_differential_checkout_unchanged_files() -> Result<()> {
        let temp_dir = TempDir::new()?;
        let repo_root = temp_dir.path();
        let storage_path = repo_root.join(".mediagit");
        fs::create_dir_all(&storage_path)?;

        let storage = Arc::new(LocalBackend::new(&storage_path).await?);
        let odb = ObjectDatabase::new(storage, 100);

        // Create first commit with two files
        let blob1 = odb.write(ObjectType::Blob, b"unchanged content").await?;
        let blob2 = odb.write(ObjectType::Blob, b"will change").await?;

        let mut tree1 = Tree::new();
        tree1.add_entry(TreeEntry::new(
            "unchanged.txt".to_string(),
            FileMode::Regular,
            blob1,
        ));
        tree1.add_entry(TreeEntry::new(
            "changed.txt".to_string(),
            FileMode::Regular,
            blob2,
        ));
        let tree1_oid = tree1.write(&odb).await?;

        let commit1 = Commit::new(
            tree1_oid,
            Signature::now("Test".to_string(), "test@example.com".to_string()),
            Signature::now("Test".to_string(), "test@example.com".to_string()),
            "commit 1".to_string(),
        );
        let commit1_oid = commit1.write(&odb).await?;

        // Create second commit - only one file changes
        let blob3 = odb.write(ObjectType::Blob, b"new content").await?;

        let mut tree2 = Tree::new();
        tree2.add_entry(TreeEntry::new(
            "unchanged.txt".to_string(),
            FileMode::Regular,
            blob1,
        )); // Same OID
        tree2.add_entry(TreeEntry::new(
            "changed.txt".to_string(),
            FileMode::Regular,
            blob3,
        )); // Different OID
        let tree2_oid = tree2.write(&odb).await?;

        let mut commit2 = Commit::new(
            tree2_oid,
            Signature::now("Test".to_string(), "test@example.com".to_string()),
            Signature::now("Test".to_string(), "test@example.com".to_string()),
            "commit 2".to_string(),
        );
        commit2.add_parent(commit1_oid);
        let commit2_oid = commit2.write(&odb).await?;

        // First checkout commit1
        let checkout_mgr = CheckoutManager::new(&odb, repo_root);
        checkout_mgr.checkout_commit(&commit1_oid).await?;

        // Differential checkout to commit2
        let stats = checkout_mgr
            .checkout_diff(&commit1_oid, &commit2_oid)
            .await?;

        // Only one file should have been modified
        assert_eq!(stats.files_unchanged, 1);
        assert_eq!(stats.files_modified, 1);
        assert_eq!(stats.files_added, 0);
        assert_eq!(stats.files_deleted, 0);

        // Verify file content
        let changed_content = fs::read(repo_root.join("changed.txt"))?;
        assert_eq!(changed_content, b"new content");

        let unchanged_content = fs::read(repo_root.join("unchanged.txt"))?;
        assert_eq!(unchanged_content, b"unchanged content");

        Ok(())
    }

    #[tokio::test]
    async fn test_differential_checkout_add_delete_files() -> Result<()> {
        let temp_dir = TempDir::new()?;
        let repo_root = temp_dir.path();
        let storage_path = repo_root.join(".mediagit");
        fs::create_dir_all(&storage_path)?;

        let storage = Arc::new(LocalBackend::new(&storage_path).await?);
        let odb = ObjectDatabase::new(storage, 100);

        // Create first commit with file A
        let blob_a = odb.write(ObjectType::Blob, b"file A").await?;
        let mut tree1 = Tree::new();
        tree1.add_entry(TreeEntry::new(
            "a.txt".to_string(),
            FileMode::Regular,
            blob_a,
        ));
        let tree1_oid = tree1.write(&odb).await?;

        let commit1 = Commit::new(
            tree1_oid,
            Signature::now("Test".to_string(), "test@example.com".to_string()),
            Signature::now("Test".to_string(), "test@example.com".to_string()),
            "commit 1".to_string(),
        );
        let commit1_oid = commit1.write(&odb).await?;

        // Create second commit with file B (no file A)
        let blob_b = odb.write(ObjectType::Blob, b"file B").await?;
        let mut tree2 = Tree::new();
        tree2.add_entry(TreeEntry::new(
            "b.txt".to_string(),
            FileMode::Regular,
            blob_b,
        ));
        let tree2_oid = tree2.write(&odb).await?;

        let commit2 = Commit::new(
            tree2_oid,
            Signature::now("Test".to_string(), "test@example.com".to_string()),
            Signature::now("Test".to_string(), "test@example.com".to_string()),
            "commit 2".to_string(),
        );
        let commit2_oid = commit2.write(&odb).await?;

        // First checkout commit1
        let checkout_mgr = CheckoutManager::new(&odb, repo_root);
        checkout_mgr.checkout_commit(&commit1_oid).await?;
        assert!(repo_root.join("a.txt").exists());

        // Differential checkout to commit2
        let stats = checkout_mgr
            .checkout_diff(&commit1_oid, &commit2_oid)
            .await?;

        assert_eq!(stats.files_added, 1); // b.txt added
        assert_eq!(stats.files_deleted, 1); // a.txt deleted
        assert_eq!(stats.files_modified, 0);
        assert_eq!(stats.files_unchanged, 0);

        // Verify files
        assert!(!repo_root.join("a.txt").exists());
        assert!(repo_root.join("b.txt").exists());
        assert_eq!(fs::read(repo_root.join("b.txt"))?, b"file B");

        Ok(())
    }

    #[tokio::test]
    async fn test_differential_checkout_stats() -> Result<()> {
        let stats = CheckoutStats {
            files_added: 2,
            files_modified: 3,
            files_deleted: 1,
            files_unchanged: 10,
            elapsed_ms: 50,
        };

        assert_eq!(stats.files_changed(), 6);
        assert_eq!(stats.total_files(), 16);

        Ok(())
    }

    /// Bug #23 repro: branch switch between two commits sharing the same tree
    /// must not skip a manually-deleted working-tree file. `checkout_diff` has a
    /// same-tree early-return (from_commit.tree == to_commit.tree) that must still
    /// verify the file exists on disk before treating it as unchanged.
    #[tokio::test]
    async fn test_checkout_diff_restores_deleted_file_same_tree() -> Result<()> {
        let temp_dir = TempDir::new()?;
        let repo_root = temp_dir.path();
        let storage_path = repo_root.join(".mediagit");
        fs::create_dir_all(&storage_path)?;

        let storage = Arc::new(LocalBackend::new(&storage_path).await?);
        let odb = ObjectDatabase::new(storage, 100);

        let blob_oid = odb.write(ObjectType::Blob, b"content").await?;
        let mut tree = Tree::new();
        tree.add_entry(TreeEntry::new(
            "file.txt".to_string(),
            FileMode::Regular,
            blob_oid,
        ));
        let tree_oid = tree.write(&odb).await?;

        // Commit A
        let commit_a = Commit::new(
            tree_oid,
            Signature::now("Test".to_string(), "test@example.com".to_string()),
            Signature::now("Test".to_string(), "test@example.com".to_string()),
            "commit A".to_string(),
        );
        let commit_a_oid = commit_a.write(&odb).await?;

        // Commit B: same tree as A (e.g. an empty commit / metadata-only change)
        let mut commit_b = Commit::new(
            tree_oid,
            Signature::now("Test".to_string(), "test@example.com".to_string()),
            Signature::now("Test".to_string(), "test@example.com".to_string()),
            "commit B".to_string(),
        );
        commit_b.add_parent(commit_a_oid);
        let commit_b_oid = commit_b.write(&odb).await?;

        let checkout_mgr = CheckoutManager::new(&odb, repo_root);
        checkout_mgr.checkout_commit(&commit_a_oid).await?;
        assert!(repo_root.join("file.txt").exists());

        // Manually delete the working-tree file (simulating accidental deletion)
        fs::remove_file(repo_root.join("file.txt"))?;
        assert!(!repo_root.join("file.txt").exists());

        // Switch to commit B, which has the identical tree
        checkout_mgr
            .checkout_diff(&commit_a_oid, &commit_b_oid)
            .await?;

        // The file must be re-materialized, not silently left missing
        assert!(
            repo_root.join("file.txt").exists(),
            "manually deleted file must be restored on checkout even when tree is unchanged"
        );
        assert_eq!(fs::read(repo_root.join("file.txt"))?, b"content");

        Ok(())
    }

    /// Bug #23 repro (differing trees variant): when the target tree differs from
    /// the source tree but a specific file has an identical OID in both, the
    /// OID-equality fast path must not skip re-materializing it if it was
    /// manually deleted from the working tree.
    #[tokio::test]
    async fn test_checkout_diff_restores_deleted_file_identical_oid() -> Result<()> {
        let temp_dir = TempDir::new()?;
        let repo_root = temp_dir.path();
        let storage_path = repo_root.join(".mediagit");
        fs::create_dir_all(&storage_path)?;

        let storage = Arc::new(LocalBackend::new(&storage_path).await?);
        let odb = ObjectDatabase::new(storage, 100);

        let blob_unchanged = odb.write(ObjectType::Blob, b"unchanged content").await?;
        let blob1 = odb.write(ObjectType::Blob, b"v1").await?;

        let mut tree1 = Tree::new();
        tree1.add_entry(TreeEntry::new(
            "unchanged.txt".to_string(),
            FileMode::Regular,
            blob_unchanged,
        ));
        tree1.add_entry(TreeEntry::new(
            "changed.txt".to_string(),
            FileMode::Regular,
            blob1,
        ));
        let tree1_oid = tree1.write(&odb).await?;

        let commit1 = Commit::new(
            tree1_oid,
            Signature::now("Test".to_string(), "test@example.com".to_string()),
            Signature::now("Test".to_string(), "test@example.com".to_string()),
            "commit 1".to_string(),
        );
        let commit1_oid = commit1.write(&odb).await?;

        // Second tree differs (changed.txt gets a new OID) but unchanged.txt keeps
        // the same OID as tree1 — this is the per-file OID-equality skip path.
        let blob2 = odb.write(ObjectType::Blob, b"v2").await?;
        let mut tree2 = Tree::new();
        tree2.add_entry(TreeEntry::new(
            "unchanged.txt".to_string(),
            FileMode::Regular,
            blob_unchanged,
        ));
        tree2.add_entry(TreeEntry::new(
            "changed.txt".to_string(),
            FileMode::Regular,
            blob2,
        ));
        let tree2_oid = tree2.write(&odb).await?;

        let mut commit2 = Commit::new(
            tree2_oid,
            Signature::now("Test".to_string(), "test@example.com".to_string()),
            Signature::now("Test".to_string(), "test@example.com".to_string()),
            "commit 2".to_string(),
        );
        commit2.add_parent(commit1_oid);
        let commit2_oid = commit2.write(&odb).await?;

        let checkout_mgr = CheckoutManager::new(&odb, repo_root);
        checkout_mgr.checkout_commit(&commit1_oid).await?;
        assert!(repo_root.join("unchanged.txt").exists());

        // Manually delete the file whose OID won't change between trees
        fs::remove_file(repo_root.join("unchanged.txt"))?;
        assert!(!repo_root.join("unchanged.txt").exists());

        checkout_mgr
            .checkout_diff(&commit1_oid, &commit2_oid)
            .await?;

        assert!(
            repo_root.join("unchanged.txt").exists(),
            "manually deleted file with identical OID across trees must be restored"
        );
        assert_eq!(
            fs::read(repo_root.join("unchanged.txt"))?,
            b"unchanged content"
        );
        assert_eq!(fs::read(repo_root.join("changed.txt"))?, b"v2");

        Ok(())
    }

    /// M3: parallel checkout equivalence — a 200-file fixture checked out
    /// with the default (parallel) `MEDIAGIT_CHECKOUT_PARALLELISM` must
    /// produce a byte-identical worktree and identical `CheckoutStats`
    /// compared to `MEDIAGIT_CHECKOUT_PARALLELISM=1` (fully serial).
    /// Exercises `checkout_tree_optimized` (fresh checkout) and
    /// `checkout_diff` (added/modified/deleted/unchanged) under real
    /// concurrency.
    #[tokio::test]
    async fn test_parallel_checkout_equivalence_200_files() -> Result<()> {
        let storage_dir = TempDir::new()?;
        let storage = Arc::new(LocalBackend::new(storage_dir.path()).await?);
        let odb = ObjectDatabase::new(storage, 1000);

        fn author() -> Signature {
            Signature::now("Test".to_string(), "test@example.com".to_string())
        }

        // Tree A: 200 flat files.
        let mut tree_a = Tree::new();
        for i in 0..200 {
            let content = format!("content-{i}");
            let oid = odb.write(ObjectType::Blob, content.as_bytes()).await?;
            tree_a.add_entry(TreeEntry::new(
                format!("file_{i:03}.txt"),
                FileMode::Regular,
                oid,
            ));
        }
        let tree_a_oid = tree_a.write(&odb).await?;
        let commit_a = Commit::new(tree_a_oid, author(), author(), "commit A".to_string());
        let commit_a_oid = commit_a.write(&odb).await?;

        // Tree B: modify every 5th file, delete files 150..170, add 10 new files.
        let mut tree_b = Tree::new();
        for i in 0..200 {
            if (150..170).contains(&i) {
                continue; // deleted in B
            }
            let content = if i % 5 == 0 {
                format!("MODIFIED-{i}")
            } else {
                format!("content-{i}")
            };
            let oid = odb.write(ObjectType::Blob, content.as_bytes()).await?;
            tree_b.add_entry(TreeEntry::new(
                format!("file_{i:03}.txt"),
                FileMode::Regular,
                oid,
            ));
        }
        for i in 0..10 {
            let content = format!("new-{i}");
            let oid = odb.write(ObjectType::Blob, content.as_bytes()).await?;
            tree_b.add_entry(TreeEntry::new(
                format!("extra_{i:03}.txt"),
                FileMode::Regular,
                oid,
            ));
        }
        let tree_b_oid = tree_b.write(&odb).await?;
        let mut commit_b = Commit::new(tree_b_oid, author(), author(), "commit B".to_string());
        commit_b.add_parent(commit_a_oid);
        let commit_b_oid = commit_b.write(&odb).await?;

        // Run 1: default (parallel) parallelism.
        // FIXME: Audit that the environment access only happens in single-threaded code.
        unsafe { std::env::remove_var("MEDIAGIT_CHECKOUT_PARALLELISM") };
        let repo_parallel = TempDir::new()?;
        let mgr_parallel = CheckoutManager::new(&odb, repo_parallel.path());
        mgr_parallel.checkout_commit(&commit_a_oid).await?;
        let stats_parallel = mgr_parallel
            .checkout_diff(&commit_a_oid, &commit_b_oid)
            .await?;

        // Run 2: forced serial (SAFETY: test-only env var scoping; no other
        // test in this process reads MEDIAGIT_CHECKOUT_PARALLELISM concurrently).
        // FIXME: Audit that the environment access only happens in single-threaded code.
        unsafe { std::env::set_var("MEDIAGIT_CHECKOUT_PARALLELISM", "1") };
        let repo_serial = TempDir::new()?;
        let mgr_serial = CheckoutManager::new(&odb, repo_serial.path());
        mgr_serial.checkout_commit(&commit_a_oid).await?;
        let stats_serial = mgr_serial
            .checkout_diff(&commit_a_oid, &commit_b_oid)
            .await?;
        // FIXME: Audit that the environment access only happens in single-threaded code.
        unsafe { std::env::remove_var("MEDIAGIT_CHECKOUT_PARALLELISM") };

        // Stats must match exactly (elapsed_ms excluded — timing, not content).
        assert_eq!(stats_parallel.files_added, stats_serial.files_added);
        assert_eq!(stats_parallel.files_modified, stats_serial.files_modified);
        assert_eq!(stats_parallel.files_deleted, stats_serial.files_deleted);
        assert_eq!(stats_parallel.files_unchanged, stats_serial.files_unchanged);
        // Multiples of 5 in 0..200 = 40, minus the 4 that fall inside the
        // deleted 150..170 range (150,155,160,165) = 36 actually modified.
        assert_eq!(stats_parallel.files_added, 10, "10 new files");
        assert_eq!(
            stats_parallel.files_modified, 36,
            "40 multiples of 5, minus 4 deleted"
        );
        assert_eq!(stats_parallel.files_deleted, 20, "files 150..170");
        assert_eq!(stats_parallel.files_unchanged, 200 - 20 - 36);

        // Worktrees must be byte-identical: same file set, same content.
        fn list_files(root: &Path) -> Result<std::collections::BTreeMap<String, Vec<u8>>> {
            let mut out = std::collections::BTreeMap::new();
            for entry in fs::read_dir(root)? {
                let entry = entry?;
                let name = entry.file_name().to_string_lossy().to_string();
                if name == ".mediagit" {
                    continue;
                }
                out.insert(name, fs::read(entry.path())?);
            }
            Ok(out)
        }
        let files_parallel = list_files(repo_parallel.path())?;
        let files_serial = list_files(repo_serial.path())?;
        assert_eq!(
            files_parallel, files_serial,
            "parallel and serial checkout must produce byte-identical worktrees"
        );

        Ok(())
    }

    /// M3: error propagation — a single bad (unwritten) OID must fail the
    /// whole parallel checkout, not silently succeed with a partial worktree.
    ///
    /// F2 extension: the bad OID is planted among many good entries (so the
    /// batch has plenty of in-flight tasks when `abort_all()` fires), and
    /// after failure every good entry's final path is checked: it must be
    /// either absent or hold its full, correct content — never a partial
    /// write. This exercises the F1 (tmp-file-then-rename) + F2 (drain
    /// JoinSet after abort) invariant together.
    #[tokio::test]
    async fn test_parallel_checkout_fails_on_bad_oid() -> Result<()> {
        let storage_dir = TempDir::new()?;
        let storage = Arc::new(LocalBackend::new(storage_dir.path()).await?);
        let odb = ObjectDatabase::new(storage, 100);

        let author = Signature::now("Test".to_string(), "test@example.com".to_string());

        const NUM_GOOD: usize = 50;
        let mut tree = Tree::new();
        let mut expected: Vec<(String, String)> = Vec::with_capacity(NUM_GOOD);
        for i in 0..NUM_GOOD {
            let content = format!("content-{i}");
            let oid = odb.write(ObjectType::Blob, content.as_bytes()).await?;
            let name = format!("file_{i:03}.txt");
            tree.add_entry(TreeEntry::new(name.clone(), FileMode::Regular, oid));
            expected.push((name, content));
        }
        // One entry points at an OID that was never written to the ODB.
        let bad_oid = Oid::hash(b"this blob was never written");
        tree.add_entry(TreeEntry::new(
            "bad.txt".to_string(),
            FileMode::Regular,
            bad_oid,
        ));

        let tree_oid = tree.write(&odb).await?;
        let commit = Commit::new(tree_oid, author.clone(), author, "bad commit".to_string());
        let commit_oid = commit.write(&odb).await?;

        let temp_dir = TempDir::new()?;
        let checkout_mgr = CheckoutManager::new(&odb, temp_dir.path());

        let result = checkout_mgr.checkout_commit(&commit_oid).await;
        assert!(
            result.is_err(),
            "checkout with an unreadable OID must fail, not silently succeed"
        );

        // No good entry's final path may hold partial content: it's either
        // absent (task never ran or was aborted before rename) or fully
        // correct (read_to_file's tmp+rename is atomic).
        for (name, content) in &expected {
            let full_path = temp_dir.path().join(name);
            if full_path.exists() {
                let on_disk = std::fs::read_to_string(&full_path)
                    .with_context(|| format!("failed to read {}", full_path.display()))?;
                assert_eq!(
                    &on_disk,
                    content,
                    "file {} present but content is not the full expected value",
                    full_path.display()
                );
            }
        }

        // Note: a stale `.mgtmp` sibling *can* remain here — `abort_all()`
        // drops an in-flight task's future mid-`.await`, which skips any
        // Rust-level cleanup code in `read_to_file`. That's the accepted
        // residual per the F1+F2 invariant (see `run_parallel_writes` doc
        // comment): never a partial file at a *final* path, at worst a
        // stale tmp sibling that the next checkout of that file overwrites.

        Ok(())
    }

    // ---- M5: sparse checkout ------------------------------------------

    fn author() -> Signature {
        Signature::now("Test".to_string(), "test@example.com".to_string())
    }

    #[tokio::test]
    async fn test_sparse_cone_mode_excludes_on_fresh_checkout() -> Result<()> {
        let storage_dir = TempDir::new()?;
        let storage = Arc::new(LocalBackend::new(storage_dir.path()).await?);
        let odb = ObjectDatabase::new(storage, 100);

        let blob_in = odb.write(ObjectType::Blob, b"included").await?;
        let blob_out = odb.write(ObjectType::Blob, b"excluded").await?;
        let mut tree = Tree::new();
        tree.add_entry(TreeEntry::new(
            "assets/textures/wood.png".to_string(),
            FileMode::Regular,
            blob_in,
        ));
        tree.add_entry(TreeEntry::new(
            "assets/audio/track.wav".to_string(),
            FileMode::Regular,
            blob_out,
        ));
        let tree_oid = tree.write(&odb).await?;
        let commit = Commit::new(tree_oid, author(), author(), "commit".to_string());
        let commit_oid = commit.write(&odb).await?;

        let repo = TempDir::new()?;
        crate::sparse::SparseFilter::write(
            repo.path(),
            crate::SparseMode::Cone,
            &["assets/textures".to_string()],
        )?;

        let checkout_mgr = CheckoutManager::new(&odb, repo.path());
        checkout_mgr.checkout_commit(&commit_oid).await?;

        assert!(repo.path().join("assets/textures/wood.png").exists());
        assert!(
            !repo.path().join("assets/audio/track.wav").exists(),
            "sparse-excluded file must not be materialized"
        );

        Ok(())
    }

    #[tokio::test]
    async fn test_sparse_pattern_mode_excludes_on_fresh_checkout() -> Result<()> {
        let storage_dir = TempDir::new()?;
        let storage = Arc::new(LocalBackend::new(storage_dir.path()).await?);
        let odb = ObjectDatabase::new(storage, 100);

        let blob_png = odb.write(ObjectType::Blob, b"png data").await?;
        let blob_wav = odb.write(ObjectType::Blob, b"wav data").await?;
        let mut tree = Tree::new();
        tree.add_entry(TreeEntry::new(
            "wood.png".to_string(),
            FileMode::Regular,
            blob_png,
        ));
        tree.add_entry(TreeEntry::new(
            "track.wav".to_string(),
            FileMode::Regular,
            blob_wav,
        ));
        let tree_oid = tree.write(&odb).await?;
        let commit = Commit::new(tree_oid, author(), author(), "commit".to_string());
        let commit_oid = commit.write(&odb).await?;

        let repo = TempDir::new()?;
        crate::sparse::SparseFilter::write(
            repo.path(),
            crate::SparseMode::Pattern,
            &["*.png".to_string()],
        )?;

        let checkout_mgr = CheckoutManager::new(&odb, repo.path());
        checkout_mgr.checkout_commit(&commit_oid).await?;

        assert!(repo.path().join("wood.png").exists());
        assert!(!repo.path().join("track.wav").exists());

        Ok(())
    }

    /// A file outside the sparse cone that already exists on disk (e.g. a
    /// leftover from before sparse was enabled) must never be deleted by
    /// ordinary checkout — only `sparse-checkout set/disable` may remove it.
    #[tokio::test]
    async fn test_sparse_excluded_existing_file_not_deleted_by_checkout() -> Result<()> {
        let storage_dir = TempDir::new()?;
        let storage = Arc::new(LocalBackend::new(storage_dir.path()).await?);
        let odb = ObjectDatabase::new(storage, 100);

        let blob_in = odb.write(ObjectType::Blob, b"included").await?;
        let mut tree = Tree::new();
        tree.add_entry(TreeEntry::new(
            "assets/textures/wood.png".to_string(),
            FileMode::Regular,
            blob_in,
        ));
        let tree_oid = tree.write(&odb).await?;
        let commit = Commit::new(tree_oid, author(), author(), "commit".to_string());
        let commit_oid = commit.write(&odb).await?;

        let repo = TempDir::new()?;
        crate::sparse::SparseFilter::write(
            repo.path(),
            crate::SparseMode::Cone,
            &["assets/textures".to_string()],
        )?;

        // Simulate a leftover excluded file already present on disk.
        let leftover = repo.path().join("assets/audio/track.wav");
        fs::create_dir_all(leftover.parent().unwrap())?;
        fs::write(&leftover, b"leftover")?;

        let checkout_mgr = CheckoutManager::new(&odb, repo.path());
        checkout_mgr.checkout_commit(&commit_oid).await?;

        assert!(
            leftover.exists(),
            "sparse-excluded file present on disk must survive checkout_commit's cleanup pass"
        );
        assert_eq!(fs::read(&leftover)?, b"leftover");

        Ok(())
    }

    /// Branch switch (`checkout_diff`) between two commits must keep
    /// applying the same sparse filter throughout: excluded paths never
    /// appear as adds/modifies/deletes even when they differ between trees.
    #[tokio::test]
    async fn test_sparse_branch_switch_keeps_filter() -> Result<()> {
        let storage_dir = TempDir::new()?;
        let storage = Arc::new(LocalBackend::new(storage_dir.path()).await?);
        let odb = ObjectDatabase::new(storage, 100);

        let blob_in_a = odb.write(ObjectType::Blob, b"in-a").await?;
        let blob_out_a = odb.write(ObjectType::Blob, b"out-a").await?;
        let mut tree_a = Tree::new();
        tree_a.add_entry(TreeEntry::new(
            "assets/textures/wood.png".to_string(),
            FileMode::Regular,
            blob_in_a,
        ));
        tree_a.add_entry(TreeEntry::new(
            "assets/audio/track.wav".to_string(),
            FileMode::Regular,
            blob_out_a,
        ));
        let tree_a_oid = tree_a.write(&odb).await?;
        let commit_a = Commit::new(tree_a_oid, author(), author(), "commit A".to_string());
        let commit_a_oid = commit_a.write(&odb).await?;

        // Commit B changes both the included and the excluded file.
        let blob_in_b = odb.write(ObjectType::Blob, b"in-b").await?;
        let blob_out_b = odb.write(ObjectType::Blob, b"out-b").await?;
        let mut tree_b = Tree::new();
        tree_b.add_entry(TreeEntry::new(
            "assets/textures/wood.png".to_string(),
            FileMode::Regular,
            blob_in_b,
        ));
        tree_b.add_entry(TreeEntry::new(
            "assets/audio/track.wav".to_string(),
            FileMode::Regular,
            blob_out_b,
        ));
        let tree_b_oid = tree_b.write(&odb).await?;
        let mut commit_b = Commit::new(tree_b_oid, author(), author(), "commit B".to_string());
        commit_b.add_parent(commit_a_oid);
        let commit_b_oid = commit_b.write(&odb).await?;

        let repo = TempDir::new()?;
        crate::sparse::SparseFilter::write(
            repo.path(),
            crate::SparseMode::Cone,
            &["assets/textures".to_string()],
        )?;

        let checkout_mgr = CheckoutManager::new(&odb, repo.path());
        checkout_mgr.checkout_commit(&commit_a_oid).await?;
        assert!(repo.path().join("assets/textures/wood.png").exists());
        assert!(!repo.path().join("assets/audio/track.wav").exists());

        let stats = checkout_mgr
            .checkout_diff(&commit_a_oid, &commit_b_oid)
            .await?;

        // Only the included file's change is visible to the diff.
        assert_eq!(stats.files_modified, 1);
        assert_eq!(stats.files_added, 0);
        assert_eq!(stats.files_deleted, 0);
        assert_eq!(
            fs::read(repo.path().join("assets/textures/wood.png"))?,
            b"in-b"
        );
        assert!(
            !repo.path().join("assets/audio/track.wav").exists(),
            "excluded file must stay absent across branch switch"
        );

        Ok(())
    }

    /// Sparse + parallel checkout equivalence: `PARALLELISM=1` vs the
    /// default must agree on which files land on disk under an active
    /// sparse cone (not just on stats, as the plain M3 test already covers).
    #[tokio::test]
    async fn test_sparse_parallel_equivalence() -> Result<()> {
        let storage_dir = TempDir::new()?;
        let storage = Arc::new(LocalBackend::new(storage_dir.path()).await?);
        let odb = ObjectDatabase::new(storage, 1000);

        let mut tree = Tree::new();
        for i in 0..50 {
            let oid = odb
                .write(ObjectType::Blob, format!("included-{i}").as_bytes())
                .await?;
            tree.add_entry(TreeEntry::new(
                format!("included/file_{i:03}.txt"),
                FileMode::Regular,
                oid,
            ));
        }
        for i in 0..50 {
            let oid = odb
                .write(ObjectType::Blob, format!("excluded-{i}").as_bytes())
                .await?;
            tree.add_entry(TreeEntry::new(
                format!("excluded/file_{i:03}.txt"),
                FileMode::Regular,
                oid,
            ));
        }
        let tree_oid = tree.write(&odb).await?;
        let commit = Commit::new(tree_oid, author(), author(), "commit".to_string());
        let commit_oid = commit.write(&odb).await?;

        fn list_files(root: &Path) -> Result<std::collections::BTreeMap<String, Vec<u8>>> {
            let mut out = std::collections::BTreeMap::new();
            fn walk(
                dir: &Path,
                root: &Path,
                out: &mut std::collections::BTreeMap<String, Vec<u8>>,
            ) -> Result<()> {
                for entry in fs::read_dir(dir)? {
                    let entry = entry?;
                    let path = entry.path();
                    if path.file_name().and_then(|n| n.to_str()) == Some(".mediagit") {
                        continue;
                    }
                    if path.is_dir() {
                        walk(&path, root, out)?;
                    } else {
                        let rel = path
                            .strip_prefix(root)
                            .unwrap()
                            .to_string_lossy()
                            .replace('\\', "/");
                        out.insert(rel, fs::read(&path)?);
                    }
                }
                Ok(())
            }
            walk(root, root, &mut out)?;
            Ok(out)
        }

        // FIXME: Audit that the environment access only happens in single-threaded code.
        unsafe { std::env::remove_var("MEDIAGIT_CHECKOUT_PARALLELISM") };
        let repo_parallel = TempDir::new()?;
        crate::sparse::SparseFilter::write(
            repo_parallel.path(),
            crate::SparseMode::Cone,
            &["included".to_string()],
        )?;
        let mgr_parallel = CheckoutManager::new(&odb, repo_parallel.path());
        mgr_parallel.checkout_commit(&commit_oid).await?;

        // FIXME: Audit that the environment access only happens in single-threaded code.
        unsafe { std::env::set_var("MEDIAGIT_CHECKOUT_PARALLELISM", "1") };
        let repo_serial = TempDir::new()?;
        crate::sparse::SparseFilter::write(
            repo_serial.path(),
            crate::SparseMode::Cone,
            &["included".to_string()],
        )?;
        let mgr_serial = CheckoutManager::new(&odb, repo_serial.path());
        mgr_serial.checkout_commit(&commit_oid).await?;
        // FIXME: Audit that the environment access only happens in single-threaded code.
        unsafe { std::env::remove_var("MEDIAGIT_CHECKOUT_PARALLELISM") };

        let files_parallel = list_files(repo_parallel.path())?;
        let files_serial = list_files(repo_serial.path())?;
        assert_eq!(
            files_parallel.len(),
            50,
            "only the included/ cone materializes"
        );
        assert_eq!(
            files_parallel, files_serial,
            "sparse-filtered parallel and serial checkout must produce identical worktrees"
        );

        Ok(())
    }

    /// QA-002: a hand-crafted (legacy-poisoned) tree with a `::stageN`
    /// debris entry must be skipped during checkout rather than
    /// materialized as an illegal colon path (os error 123 on Windows).
    /// The rest of the tree still checks out normally.
    #[tokio::test]
    async fn test_checkout_skips_stage_debris_tree_entry() -> Result<()> {
        let temp_dir = TempDir::new()?;
        let repo_root = temp_dir.path();
        let storage_path = repo_root.join(".mediagit");
        fs::create_dir_all(&storage_path)?;

        let storage = Arc::new(LocalBackend::new(&storage_path).await?);
        let odb = ObjectDatabase::new(storage, 100);

        let good_blob = odb.write(ObjectType::Blob, b"good content").await?;
        let debris_blob = odb.write(ObjectType::Blob, b"debris content").await?;

        let mut tree = Tree::new();
        tree.add_entry(TreeEntry::new(
            "good.bin".to_string(),
            FileMode::Regular,
            good_blob,
        ));
        tree.add_entry(TreeEntry::new(
            "x.bin::stage1".to_string(),
            FileMode::Regular,
            debris_blob,
        ));
        let tree_oid = tree.write(&odb).await?;

        let commit = Commit::new(
            tree_oid,
            Signature::now("Test".to_string(), "test@example.com".to_string()),
            Signature::now("Test".to_string(), "test@example.com".to_string()),
            "poisoned tree".to_string(),
        );
        let commit_oid = commit.write(&odb).await?;

        let checkout_mgr = CheckoutManager::new(&odb, repo_root);
        let files_updated = checkout_mgr.checkout_commit(&commit_oid).await?;

        assert_eq!(files_updated, 1, "only the non-debris entry is written");
        assert!(repo_root.join("good.bin").exists());
        assert!(!repo_root.join("x.bin::stage1").exists());

        Ok(())
    }
}
