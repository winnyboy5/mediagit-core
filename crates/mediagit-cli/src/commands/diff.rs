// SPDX-License-Identifier: BUSL-1.1
// Copyright (C) 2025-2026 Aswin Krishnamoorthy

use super::super::repo::{create_storage_backend, find_repo_root};
use anyhow::{Context, Result};
use clap::Parser;
use console::style;
use mediagit_versioning::{
    Commit, Index, ObjectDatabase, Oid, RefDatabase, Tag, Tree, TreeDiffer, resolve_revision,
};
use serde::Serialize;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::ignore_rules::IgnoreMatcher;
use crate::output;

/// `--machine` data for `diff`.
#[derive(Debug, Clone, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[cfg_attr(test, ts(export, export_to = "../../../ui/src/api/generated/"))]
pub struct DiffReport {
    /// `commits` (`diff <a> <b>`), `working` (`diff`, working tree vs HEAD) or
    /// `cached` (`diff --cached`, index vs HEAD).
    pub mode: String,
    /// Source commit; `None` only when there is no left-hand commit to name.
    pub from: Option<String>,
    /// Target commit; `None` for `working` and `cached`.
    pub to: Option<String>,
    pub files: Vec<DiffFileReport>,
}

#[derive(Debug, Clone, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[cfg_attr(test, ts(export, export_to = "../../../ui/src/api/generated/"))]
pub struct DiffFileReport {
    pub path: String,
    /// `added`, `modified` or `deleted`.
    pub status: String,
    #[cfg_attr(test, ts(type = "number | null"))]
    pub old_size: Option<u64>,
    #[cfg_attr(test, ts(type = "number | null"))]
    pub new_size: Option<u64>,
    /// Either side looks binary (NUL byte in the first 8000 bytes, or too
    /// large to sniff). No text hunks are produced.
    pub binary: bool,
}

const BINARY_SNIFF: usize = 8000;
/// Blobs above this are reported binary without being read.
// ponytail: size cutoff instead of streaming a prefix; add a ranged read if text files >8 MiB matter.
const BINARY_READ_LIMIT: u64 = 8 * 1024 * 1024;

/// (size, binary) of a stored blob; `(None, false)` when absent.
async fn oid_side(odb: &ObjectDatabase, oid: Option<&Oid>) -> (Option<u64>, bool) {
    let Some(oid) = oid else {
        return (None, false);
    };
    let size = odb.get_object_size(oid).await.ok().map(|s| s as u64);
    let binary = match size {
        Some(s) if s <= BINARY_READ_LIMIT => match odb.read(oid).await {
            Ok(b) => b.iter().take(BINARY_SNIFF).any(|&c| c == 0),
            Err(_) => false,
        },
        Some(_) => true,
        None => false,
    };
    (size, binary)
}

/// (size, binary) of a working-tree file; `(None, false)` when unreadable.
fn file_side(path: &Path) -> (Option<u64>, bool) {
    use std::io::Read;
    let size = std::fs::metadata(path).ok().map(|m| m.len());
    let mut buf = [0u8; BINARY_SNIFF];
    let binary = match std::fs::File::open(path).and_then(|mut f| f.read(&mut buf)) {
        Ok(n) => buf[..n].contains(&0),
        Err(_) => false,
    };
    (size, binary)
}

/// A stored `old`/`new` pair becomes one report row.
async fn stored_file(
    odb: &ObjectDatabase,
    path: String,
    status: &str,
    old: Option<&Oid>,
    new: Option<&Oid>,
) -> DiffFileReport {
    let (old_size, old_bin) = oid_side(odb, old).await;
    let (new_size, new_bin) = oid_side(odb, new).await;
    DiffFileReport {
        path,
        status: status.into(),
        old_size,
        new_size,
        binary: old_bin || new_bin,
    }
}

/// Show changes between commits
///
/// Display differences between commits, commit and working tree, or between
/// the index and working tree.
#[derive(Parser, Debug)]
#[command(after_help = "EXAMPLES:
    # Show changes between two commits
    mediagit diff abc123 def456

    # Show changes from HEAD to working directory
    mediagit diff HEAD

    # Show staged changes
    mediagit diff --cached

    # Show changes with statistics
    mediagit diff --stat abc123 def456

    # Show changes for specific files
    mediagit diff -- path/to/file.psd

    # Compare with previous commit
    mediagit diff HEAD~1 HEAD

SEE ALSO:
    mediagit-status(1), mediagit-log(1), mediagit-show(1)")]
pub struct DiffCmd {
    /// First revision to compare
    #[arg(value_name = "REVISION1")]
    pub from: Option<String>,

    /// Second revision to compare
    #[arg(value_name = "REVISION2")]
    pub to: Option<String>,

    /// Compare with working directory
    #[arg(long)]
    pub cached: bool,

    /// Show word-level changes (not yet implemented)
    #[arg(long, hide = true)]
    pub word_diff: bool,

    /// Show statistics
    #[arg(long)]
    pub stat: bool,

    /// Show summary
    #[arg(long)]
    pub summary: bool,

    /// Number of context lines
    #[arg(short = 'U', long, value_name = "NUM")]
    pub unified: Option<usize>,

    /// Paths to diff
    #[arg(value_name = "PATHS")]
    pub paths: Vec<String>,

    /// Quiet mode
    #[arg(short, long)]
    pub quiet: bool,
}

impl DiffCmd {
    pub async fn execute(&self) -> Result<()> {
        // UX-5: declared but never read, so it silently produced an ordinary
        // diff. Refuse rather than pretend (see `rebase --rebase-merges`).
        if self.word_diff {
            anyhow::bail!("--word-diff is not yet implemented.");
        }
        // --machine always answers: quiet would leave stdout empty.
        if self.quiet && !output::machine_mode() {
            return Ok(());
        }

        let repo_root = find_repo_root()?;
        let storage_path = repo_root.join(".mediagit");
        let storage = create_storage_backend(&repo_root).await?;
        let refdb = RefDatabase::new(&storage_path);
        let odb = Arc::new(ObjectDatabase::with_smart_compression(storage, 1000));

        // If no revisions specified and not --cached, compare working tree vs HEAD
        if self.from.is_none() && self.to.is_none() && !self.cached {
            return self.diff_working_tree(&repo_root, &refdb, &odb).await;
        }

        // If --cached with no revisions, compare index vs HEAD
        if self.from.is_none() && self.to.is_none() && self.cached {
            return self.diff_cached(&repo_root, &refdb, &odb).await;
        }

        // Resolve commits using revision parser (supports HEAD~N). Peel
        // annotated tags (which resolve to a tag object, not a commit).
        let (from_oid, to_oid) = self.resolve_commits(&refdb, &odb).await?;
        let from_oid = Self::peel_to_commit(from_oid, &odb).await?;
        let to_oid = Self::peel_to_commit(to_oid, &odb).await?;

        // Read commits
        let from_data = odb.read(&from_oid).await?;
        let from_commit = Commit::deserialize(&from_data)
            .context(format!("Failed to deserialize commit {}", from_oid))?;

        let to_data = odb.read(&to_oid).await?;
        let to_commit = Commit::deserialize(&to_data)
            .context(format!("Failed to deserialize commit {}", to_oid))?;

        if output::machine_mode() {
            let mut files = Vec::new();
            if from_commit.tree != to_commit.tree {
                let diff = TreeDiffer::new(odb.clone())
                    .diff_trees(&from_commit.tree, &to_commit.tree)
                    .await
                    .context("Failed to diff trees")?;
                for e in &diff.added {
                    files
                        .push(stored_file(&odb, e.name.clone(), "added", None, Some(&e.oid)).await);
                }
                for e in &diff.modified {
                    files.push(
                        stored_file(
                            &odb,
                            e.path.clone(),
                            "modified",
                            Some(&e.source.oid),
                            Some(&e.target.oid),
                        )
                        .await,
                    );
                }
                for e in &diff.deleted {
                    files.push(
                        stored_file(&odb, e.name.clone(), "deleted", Some(&e.oid), None).await,
                    );
                }
                files.sort_by(|a, b| a.path.cmp(&b.path));
            }
            return output::emit_ok(&DiffReport {
                mode: "commits".into(),
                from: Some(from_oid.to_hex()),
                to: Some(to_oid.to_hex()),
                files,
            });
        }

        // Display diff header
        println!("{} Comparing commits:", style("📊").cyan().bold());
        println!(
            "  From: {} ({})",
            from_oid,
            from_commit.message.lines().next().unwrap_or("")
        );
        println!(
            "  To:   {} ({})",
            to_oid,
            to_commit.message.lines().next().unwrap_or("")
        );
        println!();

        // Tree diff using TreeDiffer
        if from_commit.tree == to_commit.tree {
            println!("{}", style("No changes between commits").dim());
            return Ok(());
        }

        let differ = TreeDiffer::new(odb.clone());
        let diff = differ
            .diff_trees(&from_commit.tree, &to_commit.tree)
            .await
            .context("Failed to diff trees")?;

        let total = diff.added.len() + diff.deleted.len() + diff.modified.len();
        if total == 0 {
            println!("{}", style("No changes between commits").dim());
            return Ok(());
        }

        for entry in &diff.added {
            println!("  {}      {}", style("added:").green(), entry.name);
        }
        for entry in &diff.modified {
            println!("  {} {}", style("modified:").yellow(), entry.path);
        }
        for entry in &diff.deleted {
            println!("  {}    {}", style("deleted:").red(), entry.name);
        }

        println!();
        if self.stat || self.summary {
            println!(
                "{} {} file(s) changed: {} added, {} modified, {} deleted",
                style("Summary:").bold(),
                total,
                diff.added.len(),
                diff.modified.len(),
                diff.deleted.len()
            );
        }

        Ok(())
    }

    /// Compare working tree against HEAD commit
    async fn diff_working_tree(
        &self,
        repo_root: &Path,
        refdb: &RefDatabase,
        odb: &ObjectDatabase,
    ) -> Result<()> {
        // Resolve HEAD
        let head_oid = resolve_revision("HEAD", refdb, odb)
            .await
            .context("No commits yet — nothing to diff")?;

        // Read HEAD commit and its tree
        let head_data = odb.read(&head_oid).await?;
        let head_commit =
            Commit::deserialize(&head_data).context("Failed to deserialize HEAD commit")?;

        let tree_data = odb.read(&head_commit.tree).await?;
        let tree = Tree::deserialize(&tree_data).context("Failed to deserialize HEAD tree")?;

        // Build HEAD file map
        let mut head_files: HashMap<PathBuf, Oid> = HashMap::new();
        for entry in tree.iter() {
            // nosemgrep: rust.actix.path-traversal.tainted-path.tainted-path -- no fs access (map key)
            head_files.insert(PathBuf::from(&entry.name), entry.oid);
        }

        // Scan working directory, honoring .mediagitignore the same way
        // `status` does (BUG-DES-5: diff previously listed ignored files as
        // "added" while status correctly hid them).
        let matcher = IgnoreMatcher::new(repo_root).ok();
        let working_files = self.scan_working_directory(repo_root, &matcher)?;

        // Detect changes
        let mut modified = Vec::new();
        let mut deleted = Vec::new();
        let mut added = Vec::new();

        // Check for modified and deleted files
        for (path, head_oid) in &head_files {
            let full_path = repo_root.join(path);
            if !working_files.contains(path) {
                deleted.push(path.clone());
            } else {
                // Hash working tree file and compare
                // nosemgrep: rust.actix.path-traversal.tainted-path.tainted-path -- read-only probe of local worktree
                // Large files hash in constant memory (same threshold as status/add).
                let large = std::fs::metadata(&full_path)
                    .is_ok_and(|m| m.len() >= super::utils::STREAMING_THRESHOLD);
                let working_oid = if large {
                    match Oid::from_file(&full_path) {
                        Ok(oid) => oid,
                        Err(_) => continue,
                    }
                } else if let Ok(content) = std::fs::read(&full_path) {
                    Oid::hash(&content)
                } else {
                    continue;
                };
                if working_oid != *head_oid {
                    modified.push(path.clone());
                }
            }
        }

        // Check for new files (in working tree but not in HEAD)
        for path in &working_files {
            if !head_files.contains_key(path) {
                added.push(path.clone());
            }
        }

        if output::machine_mode() {
            let mut files = Vec::new();
            for path in &modified {
                let name = path.to_string_lossy().replace('\\', "/");
                let (new_size, new_bin) = file_side(&repo_root.join(path));
                let (old_size, old_bin) = oid_side(odb, head_files.get(path)).await;
                files.push(DiffFileReport {
                    path: name,
                    status: "modified".into(),
                    old_size,
                    new_size,
                    binary: old_bin || new_bin,
                });
            }
            for path in &added {
                let (new_size, binary) = file_side(&repo_root.join(path));
                files.push(DiffFileReport {
                    path: path.to_string_lossy().replace('\\', "/"),
                    status: "added".into(),
                    old_size: None,
                    new_size,
                    binary,
                });
            }
            for path in &deleted {
                files.push(
                    stored_file(
                        odb,
                        path.to_string_lossy().replace('\\', "/"),
                        "deleted",
                        head_files.get(path),
                        None,
                    )
                    .await,
                );
            }
            files.sort_by(|a, b| a.path.cmp(&b.path));
            return output::emit_ok(&DiffReport {
                mode: "working".into(),
                from: Some(head_oid.to_hex()),
                to: None,
                files,
            });
        }

        // Display results
        let short_head = &head_oid.to_hex()[..7];
        println!(
            "{} Diff: working tree vs HEAD ({})",
            style("📊").cyan().bold(),
            style(short_head).yellow()
        );
        println!();

        if modified.is_empty() && deleted.is_empty() && added.is_empty() {
            println!("{}", style("No changes in working tree").dim());
            return Ok(());
        }

        if !modified.is_empty() {
            for path in &modified {
                println!("  {} {}", style("modified:").yellow(), path.display());
            }
        }
        if !added.is_empty() {
            for path in &added {
                println!("  {}      {}", style("added:").green(), path.display());
            }
        }
        if !deleted.is_empty() {
            for path in &deleted {
                println!("  {}    {}", style("deleted:").red(), path.display());
            }
        }

        println!();

        let total = modified.len() + added.len() + deleted.len();
        if self.stat {
            println!(
                "{} {} file(s) changed: {} modified, {} added, {} deleted",
                style("Summary:").bold(),
                total,
                modified.len(),
                added.len(),
                deleted.len()
            );
        }

        Ok(())
    }

    /// Compare index (staged changes) against HEAD
    async fn diff_cached(
        &self,
        repo_root: &Path,
        refdb: &RefDatabase,
        odb: &ObjectDatabase,
    ) -> Result<()> {
        // Resolve HEAD
        let head_oid = resolve_revision("HEAD", refdb, odb)
            .await
            .context("No commits yet — nothing to diff")?;

        // Read HEAD commit and its tree
        let head_data = odb.read(&head_oid).await?;
        let head_commit =
            Commit::deserialize(&head_data).context("Failed to deserialize HEAD commit")?;

        let tree_data = odb.read(&head_commit.tree).await?;
        let tree = Tree::deserialize(&tree_data).context("Failed to deserialize HEAD tree")?;

        // Build HEAD file map
        let mut head_files: HashMap<PathBuf, Oid> = HashMap::new();
        for entry in tree.iter() {
            // nosemgrep: rust.actix.path-traversal.tainted-path.tainted-path -- no fs access (map key)
            head_files.insert(PathBuf::from(&entry.name), entry.oid);
        }

        // Load index
        let index = Index::load(repo_root)?;

        let machine = output::machine_mode();
        if index.is_empty() && !machine {
            println!("{} No staged changes (index is empty)", style("ℹ").blue());
            return Ok(());
        }

        let short_head = &head_oid.to_hex()[..7];
        if !machine {
            println!(
                "{} Diff: index vs HEAD ({})",
                style("📊").cyan().bold(),
                style(short_head).yellow()
            );
            println!();
        }

        let mut staged_new = Vec::new();
        let mut staged_modified = Vec::new();

        for entry in index.entries() {
            if let Some(head_oid) = head_files.get(&entry.path) {
                if entry.oid != *head_oid {
                    staged_modified.push(entry.path.clone());
                }
            } else {
                staged_new.push(entry.path.clone());
            }
        }

        if machine {
            let staged: HashMap<&PathBuf, &Oid> =
                index.entries().map(|e| (&e.path, &e.oid)).collect();
            let mut files = Vec::new();
            for (paths, status) in [(&staged_modified, "modified"), (&staged_new, "added")] {
                for path in paths {
                    files.push(
                        stored_file(
                            odb,
                            path.to_string_lossy().replace('\\', "/"),
                            status,
                            head_files.get(path),
                            staged.get(path).copied(),
                        )
                        .await,
                    );
                }
            }
            files.sort_by(|a, b| a.path.cmp(&b.path));
            return output::emit_ok(&DiffReport {
                mode: "cached".into(),
                from: Some(head_oid.to_hex()),
                to: None,
                files,
            });
        }

        if staged_new.is_empty() && staged_modified.is_empty() {
            println!("{}", style("No staged changes vs HEAD").dim());
            return Ok(());
        }

        for path in &staged_modified {
            println!("  {} {}", style("modified:").yellow(), path.display());
        }
        for path in &staged_new {
            println!("  {}  {}", style("new file:").green(), path.display());
        }

        println!();
        Ok(())
    }

    /// Follow a Tag object to its target, repeating until a Commit is
    /// reached (branches/OIDs already point at a commit and return
    /// immediately).
    async fn peel_to_commit(mut oid: Oid, odb: &ObjectDatabase) -> Result<Oid> {
        loop {
            let data = odb
                .read(&oid)
                .await
                .context(format!("Failed to read object {}", oid))?;
            if Commit::deserialize(&data).is_ok() {
                return Ok(oid);
            }
            match Tag::deserialize(&data) {
                Ok(tag) => oid = tag.target,
                Err(_) => anyhow::bail!("Object {} is not a commit or tag", oid),
            }
        }
    }

    async fn resolve_commits(
        &self,
        refdb: &RefDatabase,
        odb: &ObjectDatabase,
    ) -> Result<(Oid, Oid)> {
        let from_oid = if let Some(from) = &self.from {
            resolve_revision(from, refdb, odb)
                .await
                .context(format!("Cannot resolve from revision: {}", from))?
        } else {
            // Default: Use HEAD
            resolve_revision("HEAD", refdb, odb)
                .await
                .context("No commits yet")?
        };

        let to_oid = if let Some(to) = &self.to {
            resolve_revision(to, refdb, odb)
                .await
                .context(format!("Cannot resolve to revision: {}", to))?
        } else {
            // Default: Use HEAD
            resolve_revision("HEAD", refdb, odb)
                .await
                .context("No commits yet")?
        };

        Ok((from_oid, to_oid))
    }

    /// Scan working directory for files (excluding .mediagit and anything
    /// matched by .mediagitignore)
    fn scan_working_directory(
        &self,
        repo_root: &Path,
        matcher: &Option<IgnoreMatcher>,
    ) -> Result<HashSet<PathBuf>> {
        let mut files = HashSet::new();
        self.scan_directory_recursive(repo_root, repo_root, matcher, &mut files)?;
        Ok(files)
    }

    #[allow(clippy::only_used_in_recursion)]
    fn scan_directory_recursive(
        &self,
        repo_root: &Path,
        current_dir: &Path,
        matcher: &Option<IgnoreMatcher>,
        files: &mut HashSet<PathBuf>,
    ) -> Result<()> {
        // nosemgrep: rust.actix.path-traversal.tainted-path.tainted-path -- local worktree walk
        for entry in std::fs::read_dir(current_dir)? {
            let entry = entry?;
            let path = entry.path();

            // Skip .mediagit directory
            if path.file_name().and_then(|n| n.to_str()) == Some(".mediagit") {
                continue;
            }

            // Check .mediagitignore (same rule status uses)
            if let Some(m) = matcher
                && let Ok(rel) = path.strip_prefix(repo_root)
            {
                let is_dir = path.is_dir();
                if m.is_ignored(rel, is_dir) {
                    continue; // skip file OR prune entire directory
                }
            }

            if path.is_file() {
                if let Ok(rel_path) = path.strip_prefix(repo_root) {
                    let normalized = PathBuf::from(rel_path.to_string_lossy().replace('\\', "/"));
                    files.insert(normalized);
                }
            } else if path.is_dir() {
                self.scan_directory_recursive(repo_root, &path, matcher, files)?;
            }
        }
        Ok(())
    }
}
