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

use super::super::repo::{create_storage_backend, find_repo_root};
use anyhow::{Context, Result};
use clap::Parser;
use console::style;
use mediagit_versioning::{resolve_revision, Commit, ObjectDatabase, Oid, RefDatabase, Tree};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

/// Show commit history
///
/// Display commits in reverse chronological order. The output can be filtered
/// by author, date range, or commit message pattern.
#[derive(Parser, Debug)]
#[command(after_help = "EXAMPLES:
    # Show commit history
    mediagit log

    # Show last 10 commits
    mediagit log -n 10

    # Show commits in one-line format
    mediagit log --oneline

    # Show commits with statistics
    mediagit log --stat

    # Show commits by specific author
    mediagit log --author \"John Doe\"

    # Show commits matching pattern
    mediagit log --grep \"fix bug\"

    # Show commits for specific files
    mediagit log -- path/to/file.psd

    # Show commits in date range
    mediagit log --since \"2024-01-01\" --until \"2024-12-31\"

SEE ALSO:
    mediagit-show(1), mediagit-diff(1), mediagit-reflog(1)")]
pub struct LogCmd {
    /// Revision range (e.g., main..feature, v1.0..v2.0)
    #[arg(value_name = "REVISION")]
    pub revision: Option<String>,

    /// Maximum number of commits to show
    #[arg(short = 'n', long, value_name = "NUM")]
    pub max_count: Option<usize>,

    /// Skip N commits
    #[arg(long, value_name = "NUM")]
    pub skip: Option<usize>,

    /// Show abbreviated commit hash
    #[arg(long)]
    pub oneline: bool,

    /// Show graph representation
    #[arg(long)]
    pub graph: bool,

    /// Show commit statistics
    #[arg(long)]
    pub stat: bool,

    /// Show patches
    #[arg(short = 'p', long)]
    pub patch: bool,

    /// Show commits by author
    #[arg(long, value_name = "PATTERN")]
    pub author: Option<String>,

    /// Show commits with matching message
    #[arg(long, value_name = "PATTERN")]
    pub grep: Option<String>,

    /// Show commits since date
    #[arg(long, value_name = "DATE")]
    pub since: Option<String>,

    /// Show commits until date
    #[arg(long, value_name = "DATE")]
    pub until: Option<String>,

    /// Show only commits affecting these paths
    #[arg(value_name = "PATHS")]
    pub paths: Vec<String>,

    /// Show verbose output
    #[arg(short, long)]
    pub verbose: bool,

    /// Quiet mode
    #[arg(short, long)]
    pub quiet: bool,
}

impl LogCmd {
    pub async fn execute(&self) -> Result<()> {
        if self.quiet {
            return Ok(());
        }

        let repo_root = find_repo_root()?;
        let storage_path = repo_root.join(".mediagit");
        let storage = create_storage_backend(&repo_root).await?;
        let refdb = RefDatabase::new(&storage_path);
        let odb = ObjectDatabase::with_smart_compression(storage, 1000);

        // Get starting commit OID
        let start_oid = if let Some(revision) = &self.revision {
            resolve_revision(revision, &refdb, &odb)
                .await
                .with_context(|| format!("Invalid revision: {}", revision))?
        } else {
            // Use HEAD
            match refdb.read("HEAD").await {
                Ok(head) => {
                    match head.oid {
                        Some(oid) => oid,
                        None => {
                            // HEAD might be symbolic, resolve it
                            if let Some(target) = head.target {
                                match refdb.read(&target).await {
                                    Ok(target_ref) => match target_ref.oid {
                                        Some(oid) => oid,
                                        None => {
                                            println!("{}", style("No commits yet").dim());
                                            return Ok(());
                                        }
                                    },
                                    Err(_) => {
                                        // Branch doesn't exist yet (e.g., refs/heads/main on fresh repo)
                                        println!("{}", style("No commits yet").dim());
                                        return Ok(());
                                    }
                                }
                            } else {
                                println!("{}", style("No commits yet").dim());
                                return Ok(());
                            }
                        }
                    }
                }
                Err(_) => {
                    // HEAD doesn't exist yet
                    println!("{}", style("No commits yet").dim());
                    return Ok(());
                }
            }
        };

        // Traverse commit history
        let mut commits_to_show = Vec::new();
        let mut visited = HashSet::new();
        let mut stack = vec![start_oid];

        while let Some(oid) = stack.pop() {
            if visited.contains(&oid) {
                continue;
            }
            visited.insert(oid);

            // Read commit object
            let data = odb.read(&oid).await?;
            let commit = Commit::deserialize(&data)
                .with_context(|| format!("Failed to deserialize commit {}", oid))?;

            // Apply filters
            if let Some(author_pattern) = &self.author {
                if !commit.author.name.contains(author_pattern)
                    && !commit.author.email.contains(author_pattern)
                {
                    // Add parents to stack even if this commit is filtered
                    for parent in &commit.parents {
                        if !visited.contains(parent) {
                            stack.push(*parent);
                        }
                    }
                    continue;
                }
            }

            if let Some(grep_pattern) = &self.grep {
                if !commit.message.contains(grep_pattern) {
                    // Add parents to stack even if this commit is filtered
                    for parent in &commit.parents {
                        if !visited.contains(parent) {
                            stack.push(*parent);
                        }
                    }
                    continue;
                }
            }

            commits_to_show.push((oid, commit.clone()));

            // Add parents to stack
            for parent in &commit.parents {
                if !visited.contains(parent) {
                    stack.push(*parent);
                }
            }

            // Check if we've reached the limit
            if let Some(max_count) = self.max_count {
                if commits_to_show.len() >= max_count + self.skip.unwrap_or(0) {
                    break;
                }
            }
        }

        // Apply skip
        if let Some(skip) = self.skip {
            if skip < commits_to_show.len() {
                commits_to_show.drain(0..skip);
            } else {
                commits_to_show.clear();
            }
        }

        // Display commits
        if commits_to_show.is_empty() {
            println!("{}", style("No commits to show").dim());
            return Ok(());
        }

        for (oid, commit) in commits_to_show {
            if self.oneline {
                // One-line format
                let short_oid = &oid.to_string()[..7];
                let short_msg = commit.message.lines().next().unwrap_or("");
                println!("{} {}", style(short_oid).yellow(), short_msg);
            } else {
                // Full format
                println!(
                    "{} {}",
                    style("commit").yellow().bold(),
                    style(oid).yellow()
                );
                println!("Author: {} <{}>", commit.author.name, commit.author.email);
                println!("Date:   {}", commit.author.timestamp);
                println!();
                for line in commit.message.lines() {
                    println!("    {}", line);
                }
                println!();
            }

            // --stat: show file change statistics
            if self.stat {
                // Get current commit's tree files
                let current_tree_files = Self::get_tree_file_list(&odb, &commit.tree)
                    .await
                    .unwrap_or_default();

                // Get parent's tree files (empty if no parent / root commit)
                let parent_tree_files = if let Some(parent_oid) = commit.parents.first() {
                    if let Ok(parent_data) = odb.read(parent_oid).await {
                        if let Ok(parent_commit) = Commit::deserialize(&parent_data) {
                            Self::get_tree_file_list(&odb, &parent_commit.tree)
                                .await
                                .unwrap_or_default()
                        } else {
                            HashMap::new()
                        }
                    } else {
                        HashMap::new()
                    }
                } else {
                    HashMap::new()
                };

                let mut added = Vec::new();
                let mut modified = Vec::new();
                let mut deleted = Vec::new();

                // Files in current but not in parent = added
                // Files in both but different OID = modified
                for (path, current_oid) in &current_tree_files {
                    match parent_tree_files.get(path) {
                        Some(parent_oid) if parent_oid != current_oid => {
                            modified.push(path.clone());
                        }
                        None => {
                            added.push(path.clone());
                        }
                        _ => {} // unchanged
                    }
                }

                // Files in parent but not in current = deleted
                for path in parent_tree_files.keys() {
                    if !current_tree_files.contains_key(path) {
                        deleted.push(path.clone());
                    }
                }

                let total_changes = added.len() + modified.len() + deleted.len();
                if total_changes > 0 {
                    for path in &added {
                        println!(" {} | {}", path.display(), style("new file").green());
                    }
                    for path in &modified {
                        println!(" {} | {}", path.display(), style("modified").yellow());
                    }
                    for path in &deleted {
                        println!(" {} | {}", path.display(), style("deleted").red());
                    }
                    println!(
                        " {} file(s) changed, {} added, {} modified, {} deleted",
                        total_changes,
                        added.len(),
                        modified.len(),
                        deleted.len()
                    );
                    println!();
                }
            }
        }

        Ok(())
    }

    /// Helper to get a flat map of file paths to OIDs from a tree
    async fn get_tree_file_list(
        odb: &ObjectDatabase,
        tree_oid: &Oid,
    ) -> Result<HashMap<PathBuf, Oid>> {
        let mut files = HashMap::new();
        Self::walk_tree(odb, tree_oid, &PathBuf::new(), &mut files).await?;
        Ok(files)
    }

    /// Recursively walk a tree, collecting file entries
    fn walk_tree<'a>(
        odb: &'a ObjectDatabase,
        tree_oid: &'a Oid,
        prefix: &'a Path,
        files: &'a mut HashMap<PathBuf, Oid>,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<()>> + 'a>> {
        Box::pin(async move {
            let tree_data = odb.read(tree_oid).await?;
            let tree: Tree = mediagit_versioning::format::deserialize(&tree_data)?;

            for entry in tree.iter() {
                let entry_path = prefix.join(&entry.name);
                match entry.mode {
                    mediagit_versioning::FileMode::Directory => {
                        Self::walk_tree(odb, &entry.oid, &entry_path, files).await?;
                    }
                    _ => {
                        files.insert(entry_path, entry.oid);
                    }
                }
            }
            Ok(())
        })
    }
}
