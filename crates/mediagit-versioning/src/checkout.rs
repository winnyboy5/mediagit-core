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
//! to match a specific commit's tree structure.

use crate::{Commit, FileMode, ObjectDatabase, Oid, Tree};
use anyhow::{Context, Result};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use tracing::{debug, info};

/// Checkout manager for working directory operations
pub struct CheckoutManager<'a> {
    odb: &'a ObjectDatabase,
    repo_root: PathBuf,
}

impl<'a> CheckoutManager<'a> {
    /// Create a new checkout manager
    pub fn new(odb: &'a ObjectDatabase, repo_root: impl Into<PathBuf>) -> Self {
        Self {
            odb,
            repo_root: repo_root.into(),
        }
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
        let (target_files, files_updated) = self.checkout_tree_optimized(&commit.tree, Path::new("")).await?;
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
                let file_path = self.repo_root.join(&file);
                debug!("Removing file not in target: {}", file.display());

                if file_path.exists() {
                    fs::remove_file(&file_path)
                        .with_context(|| format!("Failed to remove file: {}", file_path.display()))?;
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
                let rel_path = path.strip_prefix(&self.repo_root)
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
        let entries: Vec<_> = fs::read_dir(dir)?
            .filter_map(|e| e.ok())
            .collect();
        
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
                            debug!("Failed to remove empty directory {} after {} attempts: {}", 
                                   dir.display(), attempts, e);
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

    /// Checkout a tree recursively, writing all files to working directory
    fn checkout_tree<'b>(
        &'b self,
        tree_oid: &'b Oid,
        prefix: &'b Path,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<usize>> + 'b>> {
        Box::pin(async move {
            let tree = Tree::read(self.odb, tree_oid).await?;

            let mut files_updated = 0;

            for entry in tree.iter() {
                let entry_path = prefix.join(&entry.name);
                let full_path = self.repo_root.join(&entry_path);

                match entry.mode {
                    FileMode::Regular | FileMode::Executable => {
                        // Ensure parent directory exists
                        if let Some(parent) = full_path.parent() {
                            fs::create_dir_all(parent)
                                .with_context(|| format!("Failed to create directory: {}", parent.display()))?;
                        }

                        // Use streaming write for checkout (constant memory)
                        // This method handles both chunked and non-chunked objects
                        self.odb.read_to_file(&entry.oid, &full_path).await
                            .with_context(|| format!("Failed to checkout file: {}", full_path.display()))?;

                        // Set executable permission if needed
                        #[cfg(unix)]
                        if entry.mode == FileMode::Executable {
                            use std::os::unix::fs::PermissionsExt;
                            let mut perms = fs::metadata(&full_path)?.permissions();
                            perms.set_mode(0o755);
                            fs::set_permissions(&full_path, perms)?;
                        }

                        debug!("Checked out file: {}", entry_path.display());
                        files_updated += 1;
                    }
                    FileMode::Symlink => {
                        // Read symlink target
                        let target_data = self.odb.read(&entry.oid).await?;
                        #[allow(unused_variables)]
                        let target = String::from_utf8(target_data)
                            .context("Symlink target is not valid UTF-8")?;

                        // Ensure parent directory exists
                        if let Some(parent) = full_path.parent() {
                            fs::create_dir_all(parent)?;
                        }

                        // Create symlink
                        #[cfg(unix)]
                        {
                            use std::os::unix::fs::symlink;
                            // Remove existing file/link if present
                            let _ = fs::remove_file(&full_path);
                            symlink(&target, &full_path)
                                .with_context(|| format!("Failed to create symlink: {}", full_path.display()))?;
                        }

                        #[cfg(not(unix))]
                        {
                            debug!("Symlinks not supported on this platform, skipping: {}", entry_path.display());
                        }

                        files_updated += 1;
                    }
                    FileMode::Directory => {
                        // Recursively checkout subdirectory
                        let subdir_count = self.checkout_tree(&entry.oid, &entry_path).await?;
                        files_updated += subdir_count;
                    }
                }
            }

            Ok(files_updated)
        })
    }

    /// Optimized checkout: Single-pass tree traversal that both collects files and writes them
    ///
    /// This eliminates the redundant tree traversal (get_tree_files + checkout_tree).
    /// Returns (file_paths, files_updated) for cleanup and counting.
    #[allow(clippy::type_complexity)]
    fn checkout_tree_optimized<'b>(
        &'b self,
        tree_oid: &'b Oid,
        prefix: &'b Path,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<(HashSet<PathBuf>, usize)>> + 'b>> {
        Box::pin(async move {
            let tree = Tree::read(self.odb, tree_oid).await?;

            let mut file_paths = HashSet::new();
            let mut files_updated = 0;

            for entry in tree.iter() {
                let entry_path = prefix.join(&entry.name);
                let full_path = self.repo_root.join(&entry_path);

                match entry.mode {
                    FileMode::Regular | FileMode::Executable => {
                        // Collect path for cleanup
                        file_paths.insert(entry_path.clone());

                        // OPTIMIZATION: Differential checkout - skip unchanged files
                        // Check if file exists and matches the expected OID
                        let mut skip_write = false;
                        if full_path.exists() {
                            // Quick check: Compare file size first (cheap operation)
                            if let Ok(metadata) = fs::metadata(&full_path) {
                                if let Ok(expected_size) = self.odb.get_object_size(&entry.oid).await {
                                    if metadata.len() == expected_size as u64 {
                                        // Size matches - perform full hash comparison
                                        if let Ok(file_data) = fs::read(&full_path) {
                                            let file_oid = Oid::hash(&file_data);
                                            if file_oid == entry.oid {
                                                // File is unchanged - skip write operation
                                                skip_write = true;
                                                debug!("Skipped unchanged file: {}", entry_path.display());
                                            }
                                        }
                                    }
                                }
                            }
                        }

                        if !skip_write {
                            // Ensure parent directory exists
                            if let Some(parent) = full_path.parent() {
                                fs::create_dir_all(parent)
                                    .with_context(|| format!("Failed to create directory: {}", parent.display()))?;
                            }

                            // Use streaming write for checkout (constant memory)
                            self.odb.read_to_file(&entry.oid, &full_path).await
                                .with_context(|| format!("Failed to checkout file: {}", full_path.display()))?;

                            // Set executable permission if needed
                            #[cfg(unix)]
                            if entry.mode == FileMode::Executable {
                                use std::os::unix::fs::PermissionsExt;
                                let mut perms = fs::metadata(&full_path)?.permissions();
                                perms.set_mode(0o755);
                                fs::set_permissions(&full_path, perms)?;
                            }

                            debug!("Checked out file: {}", entry_path.display());
                            files_updated += 1;
                        }
                    }
                    FileMode::Symlink => {
                        // Collect path for cleanup
                        file_paths.insert(entry_path.clone());

                        // Read symlink target
                        let target_data = self.odb.read(&entry.oid).await?;
                        let target = String::from_utf8(target_data)
                            .context("Symlink target is not valid UTF-8")?;

                        // Ensure parent directory exists
                        if let Some(parent) = full_path.parent() {
                            fs::create_dir_all(parent)?;
                        }

                        // Create symlink
                        #[cfg(unix)]
                        {
                            use std::os::unix::fs::symlink;
                            // Remove existing file/link if present
                            if full_path.exists() || full_path.symlink_metadata().is_ok() {
                                let _ = fs::remove_file(&full_path);
                            }
                            symlink(&target, &full_path)
                                .with_context(|| format!("Failed to create symlink: {}", full_path.display()))?;
                        }

                        #[cfg(not(unix))]
                        {
                            // On non-Unix, write symlink target as a regular file
                            fs::write(&full_path, target.as_bytes())
                                .with_context(|| format!("Failed to write symlink file: {}", full_path.display()))?;
                        }

                        debug!("Checked out symlink: {}", entry_path.display());
                        files_updated += 1;
                    }
                    FileMode::Directory => {
                        // Recursively checkout subdirectory
                        let (subdir_paths, subdir_count) = self.checkout_tree_optimized(&entry.oid, &entry_path).await?;
                        file_paths.extend(subdir_paths);
                        files_updated += subdir_count;
                    }
                }
            }

            Ok((file_paths, files_updated))
        })
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

        // Early exit if same tree
        if from_commit.tree == to_commit.tree {
            info!("Same tree, nothing to do");
            return Ok(CheckoutStats {
                files_added: 0,
                files_modified: 0,
                files_deleted: 0,
                files_unchanged: 0,
                elapsed_ms: start.elapsed().as_millis() as u64,
            });
        }

        // Get file mappings from both trees
        let from_files = self
            .get_tree_files_with_oid(&from_commit.tree, Path::new(""))
            .await?;
        let to_files = self
            .get_tree_files_with_oid(&to_commit.tree, Path::new(""))
            .await?;

        let mut stats = CheckoutStats {
            files_added: 0,
            files_modified: 0,
            files_deleted: 0,
            files_unchanged: 0,
            elapsed_ms: 0,
        };

        // Process files in target tree
        for (path, (to_oid, mode)) in &to_files {
            let full_path = self.repo_root.join(path);

            match from_files.get(path) {
                Some((from_oid, _)) if from_oid == to_oid => {
                    // File unchanged - skip
                    stats.files_unchanged += 1;
                    debug!("Unchanged: {}", path.display());
                }
                Some(_) => {
                    // File modified - update it
                    self.checkout_single_file(&full_path, to_oid, *mode).await?;
                    stats.files_modified += 1;
                    debug!("Modified: {}", path.display());
                }
                None => {
                    // File added - create it
                    self.checkout_single_file(&full_path, to_oid, *mode).await?;
                    stats.files_added += 1;
                    debug!("Added: {}", path.display());
                }
            }
        }

        // Delete files not in target tree
        for path in from_files.keys() {
            if !to_files.contains_key(path) {
                let full_path = self.repo_root.join(path);
                if full_path.exists() {
                    fs::remove_file(&full_path)
                        .with_context(|| format!("Failed to delete: {}", full_path.display()))?;
                    stats.files_deleted += 1;
                    debug!("Deleted: {}", path.display());
                }
            }
        }

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
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<HashMap<PathBuf, (Oid, FileMode)>>> + 'b>>
    {
        Box::pin(async move {
            let tree = Tree::read(self.odb, tree_oid).await?;
            let mut files = HashMap::new();

            for entry in tree.iter() {
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

    /// Checkout a single file from the object database
    async fn checkout_single_file(
        &self,
        full_path: &Path,
        oid: &Oid,
        mode: FileMode,
    ) -> Result<()> {
        // Read blob data
        let blob_data = self
            .odb
            .read(oid)
            .await
            .with_context(|| format!("Failed to read blob: {}", oid))?;

        // Ensure parent directory exists
        if let Some(parent) = full_path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("Failed to create directory: {}", parent.display()))?;
        }

        match mode {
            FileMode::Regular | FileMode::Executable => {
                // Write file
                fs::write(full_path, &blob_data)
                    .with_context(|| format!("Failed to write file: {}", full_path.display()))?;

                // Set executable permission if needed
                #[cfg(unix)]
                if mode == FileMode::Executable {
                    use std::os::unix::fs::PermissionsExt;
                    let mut perms = fs::metadata(full_path)?.permissions();
                    perms.set_mode(0o755);
                    fs::set_permissions(full_path, perms)?;
                }
            }
            FileMode::Symlink => {
                let target = String::from_utf8(blob_data)
                    .context("Symlink target is not valid UTF-8")?;

                #[cfg(unix)]
                {
                    use std::os::unix::fs::symlink;
                    let _ = fs::remove_file(full_path);
                    symlink(&target, full_path)
                        .with_context(|| format!("Failed to create symlink: {}", full_path.display()))?;
                }

                #[cfg(not(unix))]
                {
                    // On Windows, write symlink target as regular file
                    fs::write(full_path, target)?;
                }
            }
            FileMode::Directory => {
                // Directories are handled by recursion, not here
            }
        }

        Ok(())
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
        tree.add_entry(TreeEntry::new("file.txt".to_string(), FileMode::Regular, blob_oid));
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
        tree1.add_entry(TreeEntry::new("unchanged.txt".to_string(), FileMode::Regular, blob1));
        tree1.add_entry(TreeEntry::new("changed.txt".to_string(), FileMode::Regular, blob2));
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
        tree2.add_entry(TreeEntry::new("unchanged.txt".to_string(), FileMode::Regular, blob1)); // Same OID
        tree2.add_entry(TreeEntry::new("changed.txt".to_string(), FileMode::Regular, blob3)); // Different OID
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
        let stats = checkout_mgr.checkout_diff(&commit1_oid, &commit2_oid).await?;

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
        tree1.add_entry(TreeEntry::new("a.txt".to_string(), FileMode::Regular, blob_a));
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
        tree2.add_entry(TreeEntry::new("b.txt".to_string(), FileMode::Regular, blob_b));
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
        let stats = checkout_mgr.checkout_diff(&commit1_oid, &commit2_oid).await?;

        assert_eq!(stats.files_added, 1);   // b.txt added
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
}

