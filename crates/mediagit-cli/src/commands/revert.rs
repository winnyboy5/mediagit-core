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

//! Revert commits by creating new commits that undo changes.

use anyhow::{Context, Result};
use clap::Parser;
use console::style;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::fs;

use mediagit_versioning::{
    Commit, Index, IndexEntry, MergeEngine, MergeStrategy, ObjectDatabase, Oid, RefDatabase,
    Reflog, ReflogEntry, Signature, Tree, TreeEntry,
};

use super::super::output;
use super::super::repo::{find_repo_root, create_storage_backend};

/// Revert commits by creating inverse commits
#[derive(Parser, Debug)]
#[command(after_help = "EXAMPLES:
    mediagit revert HEAD
    mediagit revert abc1234
    mediagit revert --no-commit HEAD
    mediagit revert --continue
    mediagit revert --abort")]
pub struct RevertCmd {
    /// Commits to revert
    #[arg(value_name = "COMMITS")]
    pub commits: Vec<String>,

    /// Don't commit after reverting
    #[arg(short = 'n', long)]
    pub no_commit: bool,

    /// Custom commit message
    #[arg(short, long)]
    pub message: Option<String>,

    /// Continue after conflicts
    #[arg(id = "continue", long = "continue", conflicts_with_all = ["abort", "skip", "commits"])]
    pub continue_revert: bool,

    /// Abort current revert
    #[arg(long, conflicts_with_all = ["continue", "skip", "commits"])]
    pub abort: bool,

    /// Skip current commit
    #[arg(long, conflicts_with_all = ["continue", "abort", "commits"])]
    pub skip: bool,

    /// Quiet mode
    #[arg(short, long)]
    pub quiet: bool,
}

const REVERT_STATE_FILE: &str = "REVERT_STATE";

#[derive(Debug)]
struct RevertState {
    commits: Vec<String>,
    current_index: usize,
    original_head: Oid,
}

impl RevertState {
    fn to_string(&self) -> String {
        format!(
            "{}\n{}\n{}",
            self.original_head.to_hex(),
            self.current_index,
            self.commits.join("\n")
        )
    }

    fn from_string(s: &str) -> Result<Self> {
        let lines: Vec<&str> = s.lines().collect();
        if lines.len() < 3 {
            anyhow::bail!("Invalid revert state file");
        }

        Ok(Self {
            original_head: Oid::from_hex(lines[0])?,
            current_index: lines[1].parse()?,
            commits: lines[2..].iter().map(|s| s.to_string()).collect(),
        })
    }
}

impl RevertCmd {
    pub async fn execute(&self) -> Result<()> {
        let repo_root = find_repo_root()?;
        let storage_path = repo_root.join(".mediagit");

        if self.continue_revert {
            return self.do_continue(&repo_root, &storage_path).await;
        }
        if self.abort {
            return self.do_abort(&storage_path).await;
        }
        if self.skip {
            return self.do_skip(&storage_path).await;
        }

        if self.commits.is_empty() {
            anyhow::bail!("Must specify at least one commit to revert");
        }

        let state_file = storage_path.join(REVERT_STATE_FILE);
        if state_file.exists() {
            anyhow::bail!(
                "A revert is already in progress.\n\
                 Use 'mediagit revert --continue' or 'mediagit revert --abort'."
            );
        }

        let storage = create_storage_backend(&repo_root).await?;
        let odb = Arc::new(ObjectDatabase::new(storage.clone(), 10000));
        let refs = RefDatabase::new(&storage_path);

        let original_head = refs.resolve("HEAD").await?;

        // Resolve commits
        let mut resolved_commits = Vec::new();
        for commit_spec in &self.commits {
            let oid = self.resolve_commit(&odb, &refs, commit_spec).await?;
            resolved_commits.push(oid);
        }

        // Process each commit
        for (i, commit_oid) in resolved_commits.iter().enumerate() {
            let result = self
                .revert_single_commit(&repo_root, &storage_path, &odb, &refs, commit_oid)
                .await;

            if let Err(e) = result {
                if e.to_string().contains("conflict") {
                    let state = RevertState {
                        commits: resolved_commits.iter().map(|o| o.to_hex()).collect(),
                        current_index: i,
                        original_head: original_head.clone(),
                    };
                    fs::write(&state_file, state.to_string()).await?;

                    println!(
                        "{} Revert stopped due to conflicts.",
                        style("⚠").yellow().bold()
                    );
                    println!("  Resolve conflicts and run 'mediagit revert --continue'");
                    return Ok(());
                }
                return Err(e);
            }
        }

        if !self.quiet {
            println!(
                "\n{} Successfully reverted {} commit(s)",
                style("✓").green().bold(),
                resolved_commits.len()
            );
        }

        Ok(())
    }

    async fn revert_single_commit(
        &self,
        repo_root: &Path,
        storage_path: &Path,
        odb: &Arc<ObjectDatabase>,
        refs: &RefDatabase,
        commit_oid: &Oid,
    ) -> Result<()> {
        let commit = Commit::read(odb, commit_oid)
            .await
            .with_context(|| "Failed to read commit to revert")?;

        let parent_oid = commit
            .parents
            .first()
            .context("Cannot revert initial commit")?;

        let head_oid = refs.resolve("HEAD").await?;
        let head_commit = Commit::read(odb, &head_oid).await?;

        if !self.quiet {
            output::progress(&format!(
                "Reverting {} \"{}\"",
                &commit_oid.to_hex()[..7],
                commit.summary()
            ));
        }

        // Use MergeEngine for 3-way merge
        let merge_engine = MergeEngine::new(odb.clone());

        // We need to merge: base=commit, ours=HEAD, theirs=parent
        // This applies the inverse of the commit
        let merge_result = merge_engine
            .merge(&head_oid, parent_oid, MergeStrategy::Recursive)
            .await?;

        if merge_result.has_conflicts() {
            // Conflicts - save the HEAD tree to index for resolution
            let head_tree = Tree::read(odb, &head_commit.tree).await?;
            self.save_tree_to_index(repo_root, &head_tree)?;
            anyhow::bail!("Revert resulted in conflict - resolve and continue");
        }

        let new_tree_oid = merge_result
            .tree_oid
            .context("Merge did not produce a tree")?;

        if !self.no_commit {
            let message = self.message.clone().unwrap_or_else(|| {
                format!(
                    "Revert \"{}\"\n\nThis reverts commit {}.",
                    commit.summary(),
                    commit_oid.to_hex()
                )
            });

            let revert_commit = Commit::with_parents(
                new_tree_oid.clone(),
                vec![head_oid.clone()],
                Signature::now("MediaGit".to_string(), "mediagit@local".to_string()),
                Signature::now("MediaGit".to_string(), "mediagit@local".to_string()),
                message,
            );

            let new_commit_oid = revert_commit.write(odb).await?;

            // Update refs
            let current_branch = self.get_current_branch(storage_path).await?;
            if let Some(ref branch) = current_branch {
                refs.update(
                    &format!("refs/heads/{}", branch),
                    new_commit_oid.clone(),
                    true,
                )
                .await?;
            } else {
                refs.update("HEAD", new_commit_oid.clone(), true).await?;
            }

            // Reflog
            let reflog = Reflog::new(storage_path);
            let entry = ReflogEntry::now(
                head_oid.clone(),
                new_commit_oid.clone(),
                "MediaGit",
                "mediagit@local",
                &format!("revert: {}", &commit_oid.to_hex()[..7]),
            );
            reflog.append("HEAD", &entry).await?;
            if let Some(ref branch) = current_branch {
                reflog
                    .append(&format!("refs/heads/{}", branch), &entry)
                    .await?;
            }

            if !self.quiet {
                println!(
                    "{} Created revert commit: {}",
                    style("✓").green(),
                    &new_commit_oid.to_hex()[..7]
                );
            }
        } else {
            // No commit mode - save merged tree to index
            let merged_tree = Tree::read(odb, &new_tree_oid).await?;
            self.save_tree_to_index(repo_root, &merged_tree)?;
            if !self.quiet {
                println!(
                    "{} Reverted {} (not committed)",
                    style("✓").green(),
                    &commit_oid.to_hex()[..7]
                );
            }
        }

        Ok(())
    }

    fn save_tree_to_index(&self, repo_root: &Path, tree: &Tree) -> Result<()> {
        let mut index = Index::new();
        for entry in tree.iter() {
            if !entry.is_tree() {
                let idx_entry = IndexEntry::new(
                    PathBuf::from(&entry.name),
                    entry.oid.clone(),
                    entry.mode.as_u32(),
                    0,
                    None,
                );
                index.add_entry(idx_entry);
            }
        }
        index.save(repo_root)?;
        Ok(())
    }

    async fn do_continue(&self, repo_root: &Path, storage_path: &Path) -> Result<()> {
        let state_file = storage_path.join(REVERT_STATE_FILE);
        if !state_file.exists() {
            anyhow::bail!("No revert in progress");
        }

        let state_content = fs::read_to_string(&state_file).await?;
        let state = RevertState::from_string(&state_content)?;
        fs::remove_file(&state_file).await?;

        let storage = create_storage_backend(repo_root).await?;
        let odb = Arc::new(ObjectDatabase::new(storage.clone(), 10000));
        let refs = RefDatabase::new(storage_path);

        let index = Index::load(repo_root)?;
        let tree = self.build_tree_from_index(&index);
        let tree_oid = tree.write(&odb).await?;

        let head_oid = refs.resolve("HEAD").await?;
        let revert_oid = Oid::from_hex(&state.commits[state.current_index])?;
        let revert_commit = Commit::read(&odb, &revert_oid).await?;

        let message = self.message.clone().unwrap_or_else(|| {
            format!(
                "Revert \"{}\"\n\nThis reverts commit {}.",
                revert_commit.summary(),
                revert_oid.to_hex()
            )
        });

        let new_commit = Commit::with_parents(
            tree_oid,
            vec![head_oid.clone()],
            Signature::now("MediaGit".to_string(), "mediagit@local".to_string()),
            Signature::now("MediaGit".to_string(), "mediagit@local".to_string()),
            message,
        );

        let new_commit_oid = new_commit.write(&odb).await?;

        let current_branch = self.get_current_branch(storage_path).await?;
        if let Some(ref branch) = current_branch {
            refs.update(&format!("refs/heads/{}", branch), new_commit_oid.clone(), true)
                .await?;
        } else {
            refs.update("HEAD", new_commit_oid.clone(), true).await?;
        }

        println!(
            "{} Revert continued: {}",
            style("✓").green().bold(),
            &new_commit_oid.to_hex()[..7]
        );

        Ok(())
    }

    async fn do_abort(&self, storage_path: &Path) -> Result<()> {
        let state_file = storage_path.join(REVERT_STATE_FILE);
        if !state_file.exists() {
            anyhow::bail!("No revert in progress");
        }

        let state_content = fs::read_to_string(&state_file).await?;
        let state = RevertState::from_string(&state_content)?;

        let refs = RefDatabase::new(storage_path);
        let current_branch = self.get_current_branch(storage_path).await?;
        if let Some(ref branch) = current_branch {
            refs.update(
                &format!("refs/heads/{}", branch),
                state.original_head.clone(),
                true,
            )
            .await?;
        } else {
            refs.update("HEAD", state.original_head.clone(), true).await?;
        }

        fs::remove_file(&state_file).await?;

        println!(
            "{} Revert aborted, HEAD reset to {}",
            style("✓").green().bold(),
            &state.original_head.to_hex()[..7]
        );

        Ok(())
    }

    async fn do_skip(&self, storage_path: &Path) -> Result<()> {
        let state_file = storage_path.join(REVERT_STATE_FILE);
        if !state_file.exists() {
            anyhow::bail!("No revert in progress");
        }

        let state_content = fs::read_to_string(&state_file).await?;
        let mut state = RevertState::from_string(&state_content)?;

        state.current_index += 1;

        if state.current_index >= state.commits.len() {
            fs::remove_file(&state_file).await?;
            println!(
                "{} Revert completed (skipped last)",
                style("✓").green().bold()
            );
        } else {
            fs::write(&state_file, state.to_string()).await?;
            println!(
                "{} Skipped, {} commit(s) remaining",
                style("✓").green(),
                state.commits.len() - state.current_index
            );
        }

        Ok(())
    }

    fn build_tree_from_index(&self, index: &Index) -> Tree {
        let mut tree = Tree::new();
        for entry in index.entries() {
            tree.add_entry(TreeEntry::new(
                entry.path.to_string_lossy().to_string(),
                mediagit_versioning::FileMode::from_u32(entry.mode)
                    .unwrap_or(mediagit_versioning::FileMode::Regular),
                entry.oid.clone(),
            ));
        }
        tree
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

    async fn resolve_commit(
        &self,
        odb: &Arc<ObjectDatabase>,
        refs: &RefDatabase,
        spec: &str,
    ) -> Result<Oid> {
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
                    .context("No parent")?;
            }
            return Ok(current);
        }

        if let Ok(oid) = refs.resolve(spec).await {
            return Ok(oid);
        }

        if let Ok(oid) = refs.resolve(&format!("refs/heads/{}", spec)).await {
            return Ok(oid);
        }

        Oid::from_hex(spec).with_context(|| format!("Unknown revision: {}", spec))
    }
}
