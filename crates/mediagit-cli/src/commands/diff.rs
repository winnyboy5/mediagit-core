// Copyright (C) 2026  winnyboy5
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU Affero General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU Affero General Public License for more details.
//
// You should have received a copy of the GNU Affero General Public License
// along with this program.  If not, see <https://www.gnu.org/licenses/>.
use anyhow::{Context, Result};
use clap::Parser;
use console::style;
use mediagit_versioning::{resolve_revision, Commit, Index, ObjectDatabase, Oid, RefDatabase, Tree};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use super::super::repo::{find_repo_root, create_storage_backend};

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

    /// Show word-level changes
    #[arg(long)]
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
        if self.quiet {
            return Ok(());
        }

        let repo_root = find_repo_root()?;
        let storage_path = repo_root.join(".mediagit");
        let storage = create_storage_backend(&repo_root).await?;
        let refdb = RefDatabase::new(&storage_path);
        let odb = ObjectDatabase::with_smart_compression(storage, 1000);

        // If no revisions specified and not --cached, compare working tree vs HEAD
        if self.from.is_none() && self.to.is_none() && !self.cached {
            return self.diff_working_tree(&repo_root, &refdb, &odb).await;
        }

        // If --cached with no revisions, compare index vs HEAD
        if self.from.is_none() && self.to.is_none() && self.cached {
            return self.diff_cached(&repo_root, &refdb, &odb).await;
        }

        // Resolve commits using revision parser (supports HEAD~N)
        let (from_oid, to_oid) = self.resolve_commits(&refdb, &odb).await?;

        // Read commits
        let from_data = odb.read(&from_oid).await?;
        let from_commit = Commit::deserialize(&from_data)
            .context(format!("Failed to deserialize commit {}", from_oid))?;

        let to_data = odb.read(&to_oid).await?;
        let to_commit = Commit::deserialize(&to_data)
            .context(format!("Failed to deserialize commit {}", to_oid))?;

        // Display diff header
        println!("{} Comparing commits:", style("📊").cyan().bold());
        println!("  From: {} ({})", from_oid, from_commit.message.lines().next().unwrap_or(""));
        println!("  To:   {} ({})", to_oid, to_commit.message.lines().next().unwrap_or(""));
        println!();

        // Basic tree comparison
        if from_commit.tree == to_commit.tree {
            println!("{}", style("No changes between commits").dim());
        } else {
            println!("{}", style("Trees differ:").bold());
            println!("  From tree: {}", from_commit.tree);
            println!("  To tree:   {}", to_commit.tree);
            println!();
            println!("{}", style("Full diff functionality requires tree traversal and comparison").dim());
            println!("{}", style("This feature will be enhanced in a future release").dim());
        }

        Ok(())
    }

    /// Compare working tree against HEAD commit
    async fn diff_working_tree(&self, repo_root: &Path, refdb: &RefDatabase, odb: &ObjectDatabase) -> Result<()> {
        // Resolve HEAD
        let head_oid = resolve_revision("HEAD", refdb, odb).await
            .context("No commits yet — nothing to diff")?;

        // Read HEAD commit and its tree
        let head_data = odb.read(&head_oid).await?;
        let head_commit = Commit::deserialize(&head_data)
            .context("Failed to deserialize HEAD commit")?;

        let tree_data = odb.read(&head_commit.tree).await?;
        let tree = Tree::deserialize(&tree_data)
            .context("Failed to deserialize HEAD tree")?;

        // Build HEAD file map
        let mut head_files: HashMap<PathBuf, Oid> = HashMap::new();
        for entry in tree.iter() {
            head_files.insert(PathBuf::from(&entry.name), entry.oid);
        }

        // Scan working directory
        let working_files = self.scan_working_directory(repo_root)?;

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
                let working_oid = if let Ok(content) = std::fs::read(&full_path) {
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

        // Display results
        let short_head = &head_oid.to_hex()[..7];
        println!("{} Diff: working tree vs HEAD ({})", style("📊").cyan().bold(), style(short_head).yellow());
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
    async fn diff_cached(&self, repo_root: &Path, refdb: &RefDatabase, odb: &ObjectDatabase) -> Result<()> {
        // Resolve HEAD
        let head_oid = resolve_revision("HEAD", refdb, odb).await
            .context("No commits yet — nothing to diff")?;

        // Read HEAD commit and its tree
        let head_data = odb.read(&head_oid).await?;
        let head_commit = Commit::deserialize(&head_data)
            .context("Failed to deserialize HEAD commit")?;

        let tree_data = odb.read(&head_commit.tree).await?;
        let tree = Tree::deserialize(&tree_data)
            .context("Failed to deserialize HEAD tree")?;

        // Build HEAD file map
        let mut head_files: HashMap<PathBuf, Oid> = HashMap::new();
        for entry in tree.iter() {
            head_files.insert(PathBuf::from(&entry.name), entry.oid);
        }

        // Load index
        let index = Index::load(repo_root)?;

        if index.is_empty() {
            println!("{} No staged changes (index is empty)", style("ℹ").blue());
            return Ok(());
        }

        let short_head = &head_oid.to_hex()[..7];
        println!("{} Diff: index vs HEAD ({})", style("📊").cyan().bold(), style(short_head).yellow());
        println!();

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

    async fn resolve_commits(&self, refdb: &RefDatabase, odb: &ObjectDatabase) -> Result<(Oid, Oid)> {
        let from_oid = if let Some(from) = &self.from {
            resolve_revision(from, refdb, odb).await
                .context(format!("Cannot resolve from revision: {}", from))?
        } else {
            // Default: Use HEAD
            resolve_revision("HEAD", refdb, odb).await
                .context("No commits yet")?
        };

        let to_oid = if let Some(to) = &self.to {
            resolve_revision(to, refdb, odb).await
                .context(format!("Cannot resolve to revision: {}", to))?
        } else {
            // Default: Use HEAD
            resolve_revision("HEAD", refdb, odb).await
                .context("No commits yet")?
        };

        Ok((from_oid, to_oid))
    }

    /// Scan working directory for files (excluding .mediagit)
    fn scan_working_directory(&self, repo_root: &Path) -> Result<HashSet<PathBuf>> {
        let mut files = HashSet::new();
        self.scan_directory_recursive(repo_root, repo_root, &mut files)?;
        Ok(files)
    }

    fn scan_directory_recursive(&self, repo_root: &Path, current_dir: &Path, files: &mut HashSet<PathBuf>) -> Result<()> {
        for entry in std::fs::read_dir(current_dir)? {
            let entry = entry?;
            let path = entry.path();

            // Skip .mediagit directory
            if path.file_name().and_then(|n| n.to_str()) == Some(".mediagit") {
                continue;
            }

            if path.is_file() {
                if let Ok(rel_path) = path.strip_prefix(repo_root) {
                    files.insert(rel_path.to_path_buf());
                }
            } else if path.is_dir() {
                self.scan_directory_recursive(repo_root, &path, files)?;
            }
        }
        Ok(())
    }
}
