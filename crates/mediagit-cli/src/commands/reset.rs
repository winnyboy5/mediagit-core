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

//! Reset current HEAD to the specified state.

use anyhow::{Context, Result};
use clap::Parser;
use console::style;
use std::path::{Path, PathBuf};
use tokio::fs;

use mediagit_versioning::{
    CheckoutManager, Commit, Index, IndexEntry, ObjectDatabase, Oid, RefDatabase, Reflog,
    ReflogEntry, Tree,
};
use super::super::repo::create_storage_backend;

use super::super::output;
use super::super::repo::find_repo_root;

/// Reset current HEAD to the specified state
#[derive(Parser, Debug)]
#[command(after_help = "MODES:
    --soft   Only move HEAD
    --mixed  Move HEAD and reset index (default)
    --hard   Move HEAD, reset index, and reset working tree

EXAMPLES:
    mediagit reset --soft HEAD~1
    mediagit reset HEAD~1
    mediagit reset --hard HEAD~1
    mediagit reset file.txt")]
pub struct ResetCmd {
    /// Commit to reset to (defaults to HEAD)
    #[arg(value_name = "COMMIT")]
    pub commit: Option<String>,

    /// Files to unstage
    #[arg(value_name = "PATHS", conflicts_with_all = ["soft", "hard"])]
    pub paths: Vec<String>,

    /// Only reset HEAD
    #[arg(long, conflicts_with = "hard")]
    pub soft: bool,

    /// Reset HEAD and index (default)
    #[arg(long, hide = true)]
    pub mixed: bool,

    /// Reset HEAD, index, and working tree
    #[arg(long, conflicts_with = "soft")]
    pub hard: bool,

    /// Quiet mode
    #[arg(short, long)]
    pub quiet: bool,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum ResetMode {
    Soft,
    Mixed,
    Hard,
}

impl ResetCmd {
    pub async fn execute(&self) -> Result<()> {
        let repo_root = find_repo_root()?;
        let storage_path = repo_root.join(".mediagit");

        if !self.paths.is_empty() {
            return self.reset_paths(&repo_root).await;
        }

        let mode = if self.soft {
            ResetMode::Soft
        } else if self.hard {
            ResetMode::Hard
        } else {
            ResetMode::Mixed
        };

        self.reset_to_commit(&repo_root, &storage_path, mode).await
    }

    async fn reset_to_commit(
        &self,
        repo_root: &Path,
        storage_path: &Path,
        mode: ResetMode,
    ) -> Result<()> {
        let storage = create_storage_backend(repo_root).await?;
        let odb = ObjectDatabase::new(storage.clone(), 10000);
        let refs = RefDatabase::new(storage_path);
        let reflog = Reflog::new(storage_path);

        // Get current HEAD
        let old_oid = refs.resolve("HEAD").await?;

        // Resolve target commit
        let target_spec = self.commit.as_deref().unwrap_or("HEAD");
        let target_oid = self.resolve_target(&odb, &refs, target_spec).await?;

        // Verify commit exists
        let target_commit = Commit::read(&odb, &target_oid)
            .await
            .with_context(|| "Invalid commit reference")?;

        if !self.quiet {
            let mode_str = match mode {
                ResetMode::Soft => "soft",
                ResetMode::Mixed => "mixed",
                ResetMode::Hard => "hard",
            };
            output::progress(&format!(
                "Resetting to {} (--{})",
                &target_oid.to_hex()[..7],
                mode_str
            ));
        }

        // Get current branch from HEAD file
        let current_branch = self.get_current_branch(storage_path).await?;

        // Step 1: Move HEAD
        if let Some(ref branch) = current_branch {
            refs.update(&format!("refs/heads/{}", branch), target_oid.clone(), true)
                .await?;
        } else {
            refs.update("HEAD", target_oid.clone(), true).await?;
        }

        // Record to reflog
        let reflog_entry = ReflogEntry::now(
            old_oid,
            target_oid.clone(),
            "MediaGit",
            "mediagit@local",
            &format!("reset: moving to {}", &target_oid.to_hex()[..7]),
        );
        reflog.append("HEAD", &reflog_entry).await?;
        if let Some(ref branch) = current_branch {
            reflog
                .append(&format!("refs/heads/{}", branch), &reflog_entry)
                .await?;
        }

        // Step 1.5: For soft reset, populate index from OLD HEAD's tree
        // Since MediaGit clears the index after commit, we need to explicitly
        // stage the old commit's tree entries so they appear as staged changes.
        if mode == ResetMode::Soft {
            let old_commit = Commit::read(&odb, &old_oid)
                .await
                .with_context(|| "Failed to read old HEAD commit for soft reset")?;
            self.reset_index(repo_root, &odb, &old_commit).await?;
        }

        // Step 2: Reset index (mixed and hard)
        if mode == ResetMode::Mixed || mode == ResetMode::Hard {
            self.reset_index(repo_root, &odb, &target_commit).await?;
        }

        // Step 3: Reset working tree (hard only)
        if mode == ResetMode::Hard {
            self.reset_working_tree(repo_root, &odb, &target_oid).await?;
        }

        if !self.quiet {
            let short_oid = &target_oid.to_hex()[..7];
            match mode {
                ResetMode::Soft => {
                    println!(
                        "{} HEAD is now at {} (changes remain staged)",
                        style("✓").green(),
                        style(short_oid).yellow()
                    );
                }
                ResetMode::Mixed => {
                    println!(
                        "{} HEAD is now at {} (changes are unstaged)",
                        style("✓").green(),
                        style(short_oid).yellow()
                    );
                }
                ResetMode::Hard => {
                    println!(
                        "{} HEAD is now at {} (working tree updated)",
                        style("✓").green(),
                        style(short_oid).yellow()
                    );
                }
            }
            let summary = target_commit.summary();
            println!("  {} {}", style("→").dim(), summary);
        }

        Ok(())
    }

    async fn reset_paths(&self, repo_root: &Path) -> Result<()> {
        let mut index = Index::load(repo_root)?;
        let mut unstaged_count = 0;

        for path in &self.paths {
            if path == "." {
                index.clear();
                unstaged_count = 1;
                if !self.quiet {
                    println!("{} Unstaged all files", style("✓").green());
                }
            } else {
                let path_buf = PathBuf::from(path.replace('\\', "/"));
                if index.remove_entry(&path_buf).is_some() {
                    unstaged_count += 1;
                    if !self.quiet {
                        println!("{} Unstaged: {}", style("✓").green(), path);
                    }
                } else if !self.quiet {
                    println!("{} Not staged: {}", style("ℹ").cyan(), path);
                }
            }
        }

        index.save(repo_root)?;

        if !self.quiet && unstaged_count > 0 {
            println!(
                "\n{} Unstaged {} file(s)",
                style("✓").green().bold(),
                unstaged_count
            );
        }

        Ok(())
    }

    async fn reset_index(
        &self,
        repo_root: &Path,
        odb: &ObjectDatabase,
        commit: &Commit,
    ) -> Result<()> {
        let tree = Tree::read(odb, &commit.tree)
            .await
            .with_context(|| "Failed to read commit tree")?;

        let mut index = Index::new();
        self.add_tree_to_index(odb, &tree, PathBuf::new(), &mut index)
            .await?;

        index.save(repo_root)?;
        Ok(())
    }

    fn add_tree_to_index<'a>(
        &'a self,
        odb: &'a ObjectDatabase,
        tree: &'a Tree,
        prefix: PathBuf,
        index: &'a mut Index,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<()>> + Send + 'a>> {
        Box::pin(async move {
            for entry in tree.iter() {
                let path = if prefix.as_os_str().is_empty() {
                    PathBuf::from(&entry.name)
                } else {
                    prefix.join(&entry.name)
                };

                if entry.is_tree() {
                    let subtree = Tree::read(odb, &entry.oid).await?;
                    self.add_tree_to_index(odb, &subtree, path, index).await?;
                } else {
                    let idx_entry = IndexEntry::new(path, entry.oid.clone(), entry.mode.as_u32(), 0, None);
                    index.add_entry(idx_entry);
                }
            }
            Ok(())
        })
    }

    async fn reset_working_tree(
        &self,
        repo_root: &Path,
        odb: &ObjectDatabase,
        commit_oid: &Oid,
    ) -> Result<()> {
        let checkout_manager = CheckoutManager::new(odb, repo_root);
        checkout_manager
            .checkout_commit(commit_oid)
            .await
            .with_context(|| "Failed to checkout tree")?;
        Ok(())
    }

    async fn get_current_branch(&self, storage_path: &Path) -> Result<Option<String>> {
        let head_path = storage_path.join("HEAD");
        if !head_path.exists() {
            return Ok(None);
        }

        let content = fs::read_to_string(&head_path).await?;
        let content = content.trim();

        if let Some(target) = content.strip_prefix("ref: ") {
            if let Some(branch) = target.strip_prefix("refs/heads/") {
                return Ok(Some(branch.to_string()));
            }
        }

        Ok(None)
    }

    async fn resolve_target(
        &self,
        odb: &ObjectDatabase,
        refs: &RefDatabase,
        spec: &str,
    ) -> Result<Oid> {
        // Handle HEAD~N
        if spec.contains('~') {
            let parts: Vec<&str> = spec.splitn(2, '~').collect();
            let base_ref = parts[0];
            let count: usize = parts.get(1).and_then(|s| s.parse().ok()).unwrap_or(1);

            let base_oid = if base_ref == "HEAD" {
                refs.resolve("HEAD").await?
            } else {
                refs.resolve(base_ref).await?
            };

            let mut current = base_oid;
            for _ in 0..count {
                let commit = Commit::read(odb, &current).await?;
                current = commit
                    .parents
                    .first()
                    .cloned()
                    .context("No parent commit")?;
            }
            return Ok(current);
        }

        // Handle HEAD^
        if spec.contains('^') {
            let parts: Vec<&str> = spec.splitn(2, '^').collect();
            let base_ref = parts[0];

            let base_oid = if base_ref == "HEAD" {
                refs.resolve("HEAD").await?
            } else {
                refs.resolve(base_ref).await?
            };

            let commit = Commit::read(odb, &base_oid).await?;
            return commit
                .parents
                .first()
                .cloned()
                .context("No parent commit");
        }

        // Try as ref
        if let Ok(oid) = refs.resolve(spec).await {
            return Ok(oid);
        }

        // Try as branch
        if let Ok(oid) = refs.resolve(&format!("refs/heads/{}", spec)).await {
            return Ok(oid);
        }

        // Try as OID
        Oid::from_hex(spec).with_context(|| format!("Unknown revision: {}", spec))
    }
}
