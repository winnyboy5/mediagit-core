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
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Show object information
#[derive(Parser, Debug)]
pub struct ShowCmd {
    /// Object to show (commit, tag, tree, blob) - defaults to HEAD
    #[arg(value_name = "OBJECT")]
    pub object: Option<String>,

    /// Show patch (not yet implemented)
    #[arg(short = 'p', long, hide = true)]
    pub patch: bool,

    /// Show file change statistics
    #[arg(long)]
    pub stat: bool,

    /// Show pretty format (not yet implemented)
    #[arg(long, value_name = "FORMAT", hide = true)]
    pub pretty: Option<String>,

    /// Number of context lines (not yet implemented)
    #[arg(short = 'U', long, value_name = "NUM", hide = true)]
    pub unified: Option<usize>,

    /// Quiet mode
    #[arg(short, long)]
    pub quiet: bool,

    /// Verbose mode
    #[arg(short, long)]
    pub verbose: bool,
}

impl ShowCmd {
    pub async fn execute(&self) -> Result<()> {
        if self.quiet {
            return Ok(());
        }

        let repo_root = find_repo_root()?;
        let storage_path = repo_root.join(".mediagit");
        let storage = create_storage_backend(&repo_root).await?;
        let refdb = RefDatabase::new(&storage_path);
        let odb = ObjectDatabase::with_smart_compression(storage, 1000);

        // Resolve object ID using revision parser (supports HEAD~N)
        let object_str = self.object.as_deref().unwrap_or("HEAD");
        let oid = resolve_revision(object_str, &refdb, &odb)
            .await
            .context(format!("Cannot resolve object: {}", object_str))?;

        // Read object
        let data = odb
            .read(&oid)
            .await
            .context(format!("Failed to read object {}", oid))?;

        // Try to deserialize as commit
        match Commit::deserialize(&data) {
            Ok(commit) => {
                println!(
                    "{} {}",
                    style("commit").yellow().bold(),
                    style(&oid).yellow()
                );
                println!("Author: {} <{}>", commit.author.name, commit.author.email);
                println!("Date:   {}", commit.author.timestamp);
                println!();
                for line in commit.message.lines() {
                    println!("    {}", line);
                }
                println!();

                if self.verbose {
                    println!("Tree: {}", commit.tree);
                    if !commit.parents.is_empty() {
                        println!("Parents:");
                        for parent in &commit.parents {
                            println!("  {}", parent);
                        }
                    }
                    println!();
                }

                // Show file changes (always for show, or when --stat explicitly)
                let current_tree_files = Self::get_tree_file_list(&odb, &commit.tree)
                    .await
                    .unwrap_or_default();

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

                for (path, current_oid) in &current_tree_files {
                    match parent_tree_files.get(path) {
                        Some(parent_oid) if parent_oid != current_oid => {
                            modified.push(path.clone());
                        }
                        None => {
                            added.push(path.clone());
                        }
                        _ => {}
                    }
                }

                for path in parent_tree_files.keys() {
                    if !current_tree_files.contains_key(path) {
                        deleted.push(path.clone());
                    }
                }

                let total_changes = added.len() + modified.len() + deleted.len();
                if total_changes > 0 {
                    println!("---");
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
                } else if commit.parents.is_empty() {
                    // Root commit: show all files
                    println!("---");
                    for path in current_tree_files.keys() {
                        println!(" {} | {}", path.display(), style("new file").green());
                    }
                    println!(" {} file(s) in initial commit", current_tree_files.len());
                    println!();
                }
            }
            Err(_) => {
                // Not a commit, show raw object info
                println!("{} {}", style("object").cyan().bold(), style(&oid).yellow());
                println!("Size: {} bytes", data.len());

                if self.verbose {
                    // Show hex dump of first 256 bytes
                    let display_len = data.len().min(256);
                    println!("\nFirst {} bytes (hex):", display_len);
                    for (i, chunk) in data[..display_len].chunks(16).enumerate() {
                        print!("{:08x}  ", i * 16);
                        for byte in chunk {
                            print!("{:02x} ", byte);
                        }
                        println!();
                    }
                    if data.len() > 256 {
                        println!("... ({} more bytes)", data.len() - 256);
                    }
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
