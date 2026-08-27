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
use mediagit_versioning::{
    CheckoutManager, Commit, Index, MergeEngine, ObjectDatabase, Oid, Ref, RefDatabase, Tree,
    apply_merge_to_workdir,
};
use std::path::PathBuf;
use std::sync::Arc;

/// Apply changes from existing commits
#[derive(Parser, Debug)]
pub struct CherryPickCmd {
    /// Commit hash(es) to cherry-pick
    #[arg(value_name = "COMMITS", required = true)]
    pub commits: Vec<String>,

    /// Continue cherry-pick after resolving conflicts
    #[arg(long = "continue", alias = "continue-pick", hide = true)]
    pub continue_pick: bool,

    /// Abort cherry-pick operation
    #[arg(long)]
    pub abort: bool,

    /// Skip current commit and continue
    #[arg(long)]
    pub skip: bool,

    /// Don't automatically commit
    #[arg(short = 'n', long)]
    pub no_commit: bool,

    /// Edit commit message before committing
    #[arg(short, long)]
    pub edit: bool,

    /// Use original commit message
    #[arg(short = 'x', long)]
    pub append_message: bool,

    /// Quiet mode
    #[arg(short, long)]
    pub quiet: bool,
}

impl CherryPickCmd {
    pub async fn execute(&self) -> Result<()> {
        let repo_root = find_repo_root()?;

        // Handle special operations
        if self.abort {
            return self.abort_cherrypick(&repo_root).await;
        }

        if self.continue_pick {
            return self.continue_cherrypick(&repo_root).await;
        }

        if self.skip {
            return self.skip_cherrypick(&repo_root).await;
        }

        // Start new cherry-pick operation
        self.start_cherrypick(&repo_root).await
    }

    async fn start_cherrypick(&self, repo_root: &PathBuf) -> Result<()> {
        let mediagit_dir = repo_root.join(".mediagit");
        let storage = create_storage_backend(repo_root).await?;
        let odb = Arc::new(ObjectDatabase::with_smart_compression(
            storage.clone(),
            1000,
        ));
        let refdb = RefDatabase::new(&mediagit_dir);

        // Get current HEAD
        let current_oid = refdb
            .resolve("HEAD")
            .await
            .context("Failed to resolve HEAD")?;

        // WT-3: cherry-pick had no pre-flight dirty check at all. Only the
        // modified half is knowable up front (the target tree is the result
        // of a merge computed per commit); untracked collisions are handled
        // by `apply_commit`.
        crate::worktree_guard::AtRisk::check(repo_root, &odb, Some(&current_oid), None)
            .await?
            .ensure_clean("cherry-pick")?;

        if !self.quiet {
            println!(
                "{} Starting cherry-pick on branch at {}",
                style("→").cyan(),
                style(current_oid.to_hex()).yellow()
            );
        }

        // Process each commit to cherry-pick
        let mut picked_commits = Vec::new();
        for commit_ref in &self.commits {
            let commit_oid = self
                .resolve_commit(&refdb, repo_root, commit_ref)
                .await
                .context(format!("Failed to resolve commit: {}", commit_ref))?;

            if !self.quiet {
                println!(
                    "{} Applying commit {}",
                    style("→").cyan(),
                    style(commit_oid.to_hex()).yellow()
                );
            }

            match self
                .apply_commit(&odb, &refdb, repo_root, &commit_oid)
                .await
            {
                Ok(merged_tree_oid) => {
                    picked_commits.push(commit_oid);

                    if !self.no_commit {
                        // Create commit automatically, using the merged tree
                        // produced by the 3-way merge above (not an index
                        // rebuild, which would drop every file not touched
                        // by this commit).
                        self.create_cherry_pick_commit(
                            odb.as_ref(),
                            &refdb,
                            repo_root,
                            &commit_oid,
                            Some(merged_tree_oid),
                        )
                        .await?;
                    }
                }
                Err(e) => {
                    // Save cherry-pick state for continuation
                    self.save_cherrypick_state(repo_root, &self.commits, &picked_commits)
                        .await?;

                    println!("{} Cherry-pick failed with conflicts", style("✗").red());
                    println!("{} {}", style("Error:").red(), e);
                    println!();
                    println!("Resolve conflicts, then run:");
                    println!(
                        "  {} to continue",
                        style("mediagit cherry-pick --continue").yellow()
                    );
                    println!(
                        "  {} to abort",
                        style("mediagit cherry-pick --abort").yellow()
                    );
                    println!(
                        "  {} to skip this commit",
                        style("mediagit cherry-pick --skip").yellow()
                    );

                    return Err(e);
                }
            }
        }

        if !self.quiet {
            println!(
                "{} Successfully cherry-picked {} commit(s)",
                style("✓").green(),
                picked_commits.len()
            );
        }

        Ok(())
    }

    async fn apply_commit(
        &self,
        odb: &Arc<ObjectDatabase>,
        refdb: &RefDatabase,
        repo_root: &PathBuf,
        commit_oid: &Oid,
    ) -> Result<Oid> {
        // Load the commit
        let commit = Commit::read(odb.as_ref(), commit_oid)
            .await
            .context("Failed to read commit")?;

        // Verify parent exists
        if commit.parents.is_empty() {
            anyhow::bail!("Cannot cherry-pick initial commit");
        }

        // Get current HEAD
        let current_oid = refdb.resolve("HEAD").await?;

        // WT-1: the merge result can only materialize paths from ours (all
        // tracked) or theirs, so checking collisions against the picked commit
        // is exact. Done before any write.
        crate::worktree_guard::AtRisk::check(repo_root, odb, Some(&current_oid), Some(commit_oid))
            .await?
            .ensure_clean("cherry-pick")?;

        // Perform three-way merge: current HEAD vs commit being cherry-picked
        let merger = MergeEngine::new(odb.clone());
        let merge_result = merger
            .merge(
                &current_oid,
                commit_oid,
                mediagit_versioning::MergeStrategy::Recursive,
            )
            .await?;

        if !merge_result.conflicts.is_empty() {
            // Write conflict markers via the shared, binary-aware writer (matches
            // merge.rs's continue-merge path) instead of the old bespoke
            // write_conflicts, which Debug-printed OIDs into files and
            // corrupted binaries.
            let ours_commit = Commit::read(odb.as_ref(), &current_oid).await?;
            let theirs_commit = Commit::read(odb.as_ref(), commit_oid).await?;
            let ours_tree = Tree::read(odb.as_ref(), &ours_commit.tree).await?;
            let theirs_tree = Tree::read(odb.as_ref(), &theirs_commit.tree).await?;

            let mut index = Index::load(repo_root)?;

            apply_merge_to_workdir(
                &merge_result,
                &ours_tree,
                &theirs_tree,
                odb,
                repo_root,
                &mut index,
                *commit_oid,
                current_oid,
            )
            .await?;

            index.save(repo_root)?;

            anyhow::bail!("Merge conflicts detected");
        }

        let tree_oid = merge_result
            .tree_oid
            .context("merge produced no tree during cherry-pick")?;

        // Checkout the merged tree. WT-1: only files tracked at the current
        // HEAD may be deleted.
        let tracked =
            crate::worktree_guard::tracked_paths(repo_root, odb, Some(&current_oid)).await?;
        let checkout_mgr =
            CheckoutManager::new(odb.as_ref(), repo_root).with_tracked_paths(tracked);
        let commit_to_checkout = Commit {
            tree: tree_oid,
            parents: vec![current_oid],
            author: commit.author.clone(),
            committer: commit.committer.clone(),
            message: commit.message.clone(),
        };
        // Write temporary commit to get OID for checkout
        let temp_oid = commit_to_checkout.write(odb.as_ref()).await?;
        checkout_mgr.checkout_commit(&temp_oid).await?;

        Ok(tree_oid)
    }

    async fn create_cherry_pick_commit(
        &self,
        odb: &ObjectDatabase,
        refdb: &RefDatabase,
        repo_root: &std::path::Path,
        original_oid: &Oid,
        merged_tree_oid: Option<Oid>,
    ) -> Result<()> {
        // Load original commit for message
        let original_commit = Commit::read(odb, original_oid).await?;

        // Build commit message
        let mut message = original_commit.message.clone();
        if self.append_message {
            message.push_str(&format!(
                "\n\n(cherry picked from commit {})",
                original_oid.to_hex()
            ));
        }

        // Use the merged tree from the 3-way merge when available. Only fall
        // back to rebuilding from the index (manual conflict resolution via
        // `--continue`, where no merge result exists) when it isn't.
        let tree_oid = match merged_tree_oid {
            Some(oid) => oid,
            None => {
                let index = Index::load(repo_root)?;
                let mut tree = Tree::new();
                for entry in index.entries() {
                    tree.add_entry(mediagit_versioning::TreeEntry::new(
                        entry.path.to_string_lossy().to_string(),
                        mediagit_versioning::FileMode::Regular,
                        entry.oid,
                    ));
                }
                tree.write(odb).await?
            }
        };

        // Get current HEAD as parent
        let current_oid = refdb.resolve("HEAD").await?;

        // Create new commit
        let new_commit = Commit {
            tree: tree_oid,
            parents: vec![current_oid],
            author: original_commit.author.clone(),
            committer: mediagit_versioning::Signature {
                name: original_commit.committer.name.clone(),
                email: original_commit.committer.email.clone(),
                timestamp: chrono::Utc::now(),
            },
            message,
        };

        let commit_oid = new_commit.write(odb).await?;

        // Update HEAD
        let head = refdb.read("HEAD").await?;
        if let Some(target) = head.target {
            let new_ref = Ref::new_direct(target, commit_oid);
            refdb.write(&new_ref).await?;
        }

        if !self.quiet {
            println!(
                "{} Created commit {}",
                style("✓").green(),
                style(commit_oid.to_hex()).yellow()
            );
        }

        Ok(())
    }

    async fn continue_cherrypick(&self, repo_root: &PathBuf) -> Result<()> {
        let state_path = repo_root.join(".mediagit/CHERRY_PICK_STATE");
        if !state_path.exists() {
            anyhow::bail!("No cherry-pick in progress");
        }

        // Load state
        let state_json = std::fs::read_to_string(&state_path)?;
        let state: CherryPickState = serde_json::from_str(&state_json)?;

        // Note: Conflict checking would need additional state tracking
        // For now, assume user has resolved conflicts if they're continuing

        // Create commit for current pick
        let mediagit_dir = repo_root.join(".mediagit");
        let storage = create_storage_backend(repo_root).await?;
        let odb = ObjectDatabase::with_smart_compression(storage.clone(), 1000);
        let refdb = RefDatabase::new(&mediagit_dir);

        if let Some(current) = &state.current_commit {
            let current_oid = Oid::from_hex(current)?;
            self.create_cherry_pick_commit(&odb, &refdb, repo_root, &current_oid, None)
                .await?;
        }

        // Continue with remaining commits
        if state.remaining_commits.is_empty() {
            // Clean up state
            std::fs::remove_file(&state_path)?;

            if !self.quiet {
                println!("{} Cherry-pick complete", style("✓").green());
            }
            return Ok(());
        }

        // Process remaining commits
        let remaining: Vec<String> = state.remaining_commits.clone();
        drop(state); // Release state before recursive call

        let cmd = Self {
            commits: remaining,
            continue_pick: false,
            abort: false,
            skip: false,
            no_commit: self.no_commit,
            edit: self.edit,
            append_message: self.append_message,
            quiet: self.quiet,
        };

        cmd.start_cherrypick(repo_root).await
    }

    async fn skip_cherrypick(&self, repo_root: &PathBuf) -> Result<()> {
        let state_path = repo_root.join(".mediagit/CHERRY_PICK_STATE");
        if !state_path.exists() {
            anyhow::bail!("No cherry-pick in progress");
        }

        // Load state
        let state_json = std::fs::read_to_string(&state_path)?;
        let state: CherryPickState = serde_json::from_str(&state_json)?;

        if !self.quiet {
            println!(
                "{} Skipping commit {}",
                style("→").cyan(),
                state.current_commit.as_deref().unwrap_or("unknown")
            );
        }

        // Continue with remaining commits (skip current)
        if state.remaining_commits.is_empty() {
            std::fs::remove_file(&state_path)?;

            if !self.quiet {
                println!("{} Cherry-pick complete", style("✓").green());
            }
            return Ok(());
        }

        let remaining: Vec<String> = state.remaining_commits.clone();
        drop(state);

        let cmd = Self {
            commits: remaining,
            continue_pick: false,
            abort: false,
            skip: false,
            no_commit: self.no_commit,
            edit: self.edit,
            append_message: self.append_message,
            quiet: self.quiet,
        };

        cmd.start_cherrypick(repo_root).await
    }

    async fn abort_cherrypick(&self, repo_root: &PathBuf) -> Result<()> {
        let state_path = repo_root.join(".mediagit/CHERRY_PICK_STATE");
        if !state_path.exists() {
            anyhow::bail!("No cherry-pick in progress");
        }

        // Load state to get original HEAD
        let state_json = std::fs::read_to_string(&state_path)?;
        let state: CherryPickState = serde_json::from_str(&state_json)?;

        if let Some(original_head) = &state.original_head {
            // Reset to original HEAD
            let mediagit_dir = repo_root.join(".mediagit");
            let storage = create_storage_backend(repo_root).await?;
            let odb = ObjectDatabase::with_smart_compression(storage.clone(), 1000);
            let refdb = RefDatabase::new(&mediagit_dir);

            let original_oid = Oid::from_hex(original_head)?;

            // Update HEAD reference
            let head = refdb.read("HEAD").await?;
            if let Some(target) = head.target {
                let reset_ref = Ref::new_direct(target, original_oid);
                refdb.write(&reset_ref).await?;
            }

            // Restore working directory. WT-1: an abort must not take
            // untracked files with it.
            let tracked =
                crate::worktree_guard::tracked_paths(repo_root, &odb, Some(&original_oid)).await?;
            let checkout_mgr = mediagit_versioning::CheckoutManager::new(&odb, repo_root)
                .with_tracked_paths(tracked);
            checkout_mgr.checkout_commit(&original_oid).await?;
        }

        // Clean up state
        std::fs::remove_file(&state_path)?;

        if !self.quiet {
            println!("{} Cherry-pick aborted", style("✓").green());
        }

        Ok(())
    }

    async fn save_cherrypick_state(
        &self,
        repo_root: &std::path::Path,
        all_commits: &[String],
        picked_commits: &[Oid],
    ) -> Result<()> {
        let mediagit_dir = repo_root.join(".mediagit");
        let refdb = RefDatabase::new(&mediagit_dir);

        let original_head = refdb.resolve("HEAD").await?.to_hex();

        let current_commit = all_commits.get(picked_commits.len()).cloned();
        let remaining_commits = all_commits[picked_commits.len() + 1..].to_vec();

        let state = CherryPickState {
            original_head: Some(original_head),
            current_commit,
            remaining_commits,
        };

        let state_json = serde_json::to_string_pretty(&state)?;
        std::fs::write(mediagit_dir.join("CHERRY_PICK_STATE"), state_json)?;

        Ok(())
    }

    async fn resolve_commit(
        &self,
        refdb: &RefDatabase,
        repo_root: &std::path::Path,
        commit_ref: &str,
    ) -> Result<Oid> {
        // Try full 64-char hex OID first
        if let Ok(oid) = Oid::from_hex(commit_ref) {
            return Ok(oid);
        }

        // Try as branch reference
        let branch_ref = format!("refs/heads/{}", commit_ref);
        if refdb.exists(&branch_ref).await? {
            return refdb.resolve(&branch_ref).await;
        }

        // Try as tag reference
        let tag_ref = format!("refs/tags/{}", commit_ref);
        if refdb.exists(&tag_ref).await? {
            return refdb.resolve(&tag_ref).await;
        }

        // Try short hash prefix matching (e.g., 7-char hashes from `log --oneline`)
        // via the central resolver, which also matches pack-embedded objects
        // (post-`gc --repack`, not just loose ones).
        let looks_like_hex = commit_ref.len() >= 4
            && commit_ref.len() < 64
            && commit_ref.chars().all(|c| c.is_ascii_hexdigit());
        if looks_like_hex && let Ok(storage) = create_storage_backend(repo_root).await {
            let odb = ObjectDatabase::with_smart_compression(storage, 1000);
            if let Ok(oid) = odb.resolve_abbreviated_oid(commit_ref).await {
                return Ok(oid);
            }
        }

        // Try resolving directly via refdb
        refdb
            .resolve(commit_ref)
            .await
            .context(format!("Cannot resolve commit reference: {}", commit_ref))
    }
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
struct CherryPickState {
    original_head: Option<String>,
    current_commit: Option<String>,
    remaining_commits: Vec<String>,
}

#[cfg(test)]
#[allow(unsafe_code)] // edition-2024: test-only env::set_var/remove_var requires unsafe
mod tests {
    use super::*;
    use crate::commands::utils::test_support::{REPO_ENV_LOCK, init_repo_with_commit};
    use clap::Parser;
    use tempfile::TempDir;

    fn parse(args: &[&str]) -> Result<CherryPickCmd, clap::Error> {
        let mut full = vec!["cherry-pick"];
        full.extend_from_slice(args);
        CherryPickCmd::try_parse_from(full)
    }

    #[test]
    fn parse_basic_commit() {
        let cmd = parse(&["abc123"]).unwrap();
        assert_eq!(cmd.commits, vec!["abc123".to_string()]);
        assert!(!cmd.abort);
        assert!(!cmd.no_commit);
    }

    #[test]
    fn parse_multiple_commits() {
        let cmd = parse(&["c1", "c2", "c3"]).unwrap();
        assert_eq!(cmd.commits, vec!["c1", "c2", "c3"]);
    }

    #[test]
    fn parse_missing_commits_is_error() {
        assert!(parse(&[]).is_err());
    }

    #[test]
    fn parse_all_flags() {
        let cmd = parse(&["c1", "-n", "-e", "-x", "-q"]).unwrap();
        assert!(cmd.no_commit);
        assert!(cmd.edit);
        assert!(cmd.append_message);
        assert!(cmd.quiet);
    }

    #[test]
    fn parse_abort_continue_skip() {
        assert!(parse(&["c1", "--abort"]).unwrap().abort);
        assert!(parse(&["c1", "--continue-pick"]).unwrap().continue_pick);
        assert!(parse(&["c1", "--skip"]).unwrap().skip);
    }

    /// Guards `MEDIAGIT_REPO` across the `.await` points in `execute()`
    /// (see `REPO_ENV_LOCK` docs).
    #[allow(clippy::await_holding_lock)]
    async fn execute_in(repo_path: &std::path::Path, cmd: &CherryPickCmd) -> Result<()> {
        let _guard = REPO_ENV_LOCK.lock().unwrap();
        // FIXME: Audit that the environment access only happens in single-threaded code.
        unsafe { std::env::set_var("MEDIAGIT_REPO", repo_path) };
        let result = cmd.execute().await;
        // FIXME: Audit that the environment access only happens in single-threaded code.
        unsafe { std::env::remove_var("MEDIAGIT_REPO") };
        result
    }

    #[tokio::test]
    async fn execute_no_repo_is_error() {
        let temp = TempDir::new().unwrap();
        let cmd = parse(&["abc123"]).unwrap();
        let err = execute_in(temp.path(), &cmd).await.unwrap_err();
        assert!(err.to_string().contains("Not a mediagit repository"));
    }

    #[tokio::test]
    async fn execute_unresolvable_commit_is_error() {
        let temp = TempDir::new().unwrap();
        init_repo_with_commit(temp.path()).await;

        let cmd = parse(&["does-not-exist"]).unwrap();
        let err = execute_in(temp.path(), &cmd).await.unwrap_err();
        assert!(err.to_string().contains("Failed to resolve commit"));
    }

    #[tokio::test]
    async fn continue_without_cherrypick_in_progress_is_error() {
        let temp = TempDir::new().unwrap();
        init_repo_with_commit(temp.path()).await;

        let cmd = parse(&["c1", "--continue-pick"]).unwrap();
        let err = execute_in(temp.path(), &cmd).await.unwrap_err();
        assert!(err.to_string().contains("No cherry-pick in progress"));
    }

    #[tokio::test]
    async fn abort_without_cherrypick_in_progress_is_error() {
        let temp = TempDir::new().unwrap();
        init_repo_with_commit(temp.path()).await;

        let cmd = parse(&["c1", "--abort"]).unwrap();
        let err = execute_in(temp.path(), &cmd).await.unwrap_err();
        assert!(err.to_string().contains("No cherry-pick in progress"));
    }

    #[tokio::test]
    async fn skip_without_cherrypick_in_progress_is_error() {
        let temp = TempDir::new().unwrap();
        init_repo_with_commit(temp.path()).await;

        let cmd = parse(&["c1", "--skip"]).unwrap();
        let err = execute_in(temp.path(), &cmd).await.unwrap_err();
        assert!(err.to_string().contains("No cherry-pick in progress"));
    }
}
