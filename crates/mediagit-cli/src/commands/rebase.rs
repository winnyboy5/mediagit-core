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

use anyhow::{Context, Result};
use clap::Parser;
use console::style;
use mediagit_versioning::{
    CheckoutManager, Commit, LcaFinder, MergeEngine, MergeStrategy, ObjectDatabase, ObjectType,
    Oid, Ref, RefDatabase, Signature, Tree,
};
use std::collections::HashSet;
use std::sync::Arc;

use super::super::repo::{create_storage_backend, find_repo_root};
use super::rebase_state::RebaseState;

/// Rebase commits
#[derive(Parser, Debug)]
pub struct RebaseCmd {
    /// Upstream branch to rebase onto
    #[arg(value_name = "UPSTREAM")]
    pub upstream: String,

    /// Branch to rebase (defaults to current)
    #[arg(value_name = "BRANCH")]
    pub branch: Option<String>,

    /// Rebase merge commits (not yet implemented)
    #[arg(short = 'm', long, hide = true)]
    pub rebase_merges: bool,

    /// Keep empty commits
    #[arg(long)]
    pub keep_empty: bool,

    /// Autosquash - automatically apply fixup/squash (not yet implemented)
    #[arg(long, hide = true)]
    pub autosquash: bool,

    /// Abort rebase
    #[arg(long)]
    pub abort: bool,

    /// Continue after resolving conflicts
    #[arg(long)]
    pub continue_rebase: bool,

    /// Skip current commit
    #[arg(long)]
    pub skip: bool,

    /// Quiet mode
    #[arg(short, long)]
    pub quiet: bool,

    /// Verbose mode
    #[arg(short, long)]
    pub verbose: bool,
}

impl RebaseCmd {
    pub async fn execute(&self) -> Result<()> {
        let repo_root = find_repo_root()?;

        // Handle special operations
        if self.abort {
            return self.abort_rebase(&repo_root).await;
        }
        if self.continue_rebase {
            return self.continue_rebase_process(&repo_root).await;
        }
        if self.skip {
            return self.skip_commit(&repo_root).await;
        }

        // Check if rebase already in progress
        if RebaseState::in_progress(&repo_root) {
            anyhow::bail!("A rebase is already in progress. Use --continue, --skip, or --abort.");
        }

        // Merge rebases not yet supported
        if self.rebase_merges {
            anyhow::bail!("Rebase with merge commits not yet implemented.");
        }

        let storage_path = repo_root.join(".mediagit");
        let storage = create_storage_backend(&repo_root).await?;
        let refdb = RefDatabase::new(&storage_path);
        let odb = Arc::new(ObjectDatabase::with_smart_compression(storage, 1000));

        // Resolve upstream branch
        let upstream_oid = self.resolve_branch(&refdb, &self.upstream).await?;

        // Get current HEAD
        let head = refdb.read("HEAD").await?;
        let head_target = head.target.clone();
        let current_oid = match head.oid {
            Some(oid) => oid,
            None => {
                if let Some(ref target) = head_target {
                    let target_ref = refdb.read(target).await?;
                    target_ref.oid.context("HEAD has no commit yet")?
                } else {
                    anyhow::bail!("HEAD has no commit yet");
                }
            }
        };

        if !self.quiet {
            println!(
                "{} Rebasing onto {}...",
                style("🔄").cyan().bold(),
                style(&self.upstream).yellow()
            );
        }

        // Find merge base (common ancestor)
        let lca_finder = LcaFinder::new(odb.clone());
        let merge_bases = lca_finder
            .find_merge_base(&current_oid, &upstream_oid)
            .await?;

        if merge_bases.is_empty() {
            anyhow::bail!("No common ancestor found");
        }

        let base_oid = merge_bases[0];

        if self.verbose {
            println!("  Merge base: {}", &base_oid.to_string()[..7]);
        }

        // Check if already up to date
        if lca_finder.is_ancestor(&upstream_oid, &current_oid).await? {
            if !self.quiet {
                println!("{} Already up to date", style("✓").green());
            }
            return Ok(());
        }

        // Collect commits to rebase (from base to current)
        let commits_to_rebase = self.collect_commits(&odb, &base_oid, &current_oid).await?;

        if commits_to_rebase.is_empty() {
            if !self.quiet {
                println!("{} No commits to rebase", style("ℹ").blue());
            }
            return Ok(());
        }

        if !self.quiet {
            println!("  Rebasing {} commit(s)...", commits_to_rebase.len());
        }

        // Collect commit OIDs for state tracking
        let commit_oids: Vec<Oid> = {
            let mut oids = Vec::new();
            let mut current = current_oid;
            let mut visited = HashSet::new();

            loop {
                if visited.contains(&current) || current == base_oid {
                    break;
                }
                visited.insert(current);

                let data = odb.read(&current).await?;
                let commit = Commit::deserialize(&data)?;
                oids.push(current);

                if let Some(parent) = commit.parents.first() {
                    current = *parent;
                } else {
                    break;
                }
            }
            oids.reverse();
            oids
        };

        // Create and save initial rebase state
        let mut state = RebaseState::new(
            current_oid,
            head_target.clone(),
            upstream_oid,
            commit_oids.clone(),
        );
        state.save(&repo_root)?;

        // Rebase commits one by one
        let result = self
            .apply_commits(&repo_root, &odb, &refdb, &mut state, &commits_to_rebase)
            .await;

        match result {
            Ok(new_head) => {
                // Update HEAD to point to new commit chain
                if let Some(ref target) = head_target {
                    let new_ref = Ref::new_direct(target.clone(), new_head);
                    refdb.write(&new_ref).await?;
                } else {
                    let new_ref = Ref::new_direct("HEAD".to_string(), new_head);
                    refdb.write(&new_ref).await?;
                }

                // Sync the working directory to the newly rebased tree
                let checkout_mgr = CheckoutManager::new(&odb, &repo_root);
                checkout_mgr
                    .checkout_commit(&new_head)
                    .await
                    .context("Failed to update working directory after rebase")?;

                // Clear rebase state on success
                RebaseState::clear(&repo_root)?;

                if !self.quiet {
                    println!(
                        "{} Successfully rebased {} commit(s)",
                        style("✓").green().bold(),
                        commits_to_rebase.len()
                    );
                }
                Ok(())
            }
            Err(e) => {
                // State is preserved for continue/abort
                Err(e)
            }
        }
    }

    /// Apply commits during rebase, updating state as we go.
    async fn apply_commits(
        &self,
        repo_root: &std::path::Path,
        odb: &Arc<ObjectDatabase>,
        _refdb: &RefDatabase,
        state: &mut RebaseState,
        commits: &[(Oid, Commit)],
    ) -> Result<Oid> {
        let mut new_parent = state.new_parent;
        let merge_engine = MergeEngine::new(odb.clone());

        for (original_oid, original_commit) in commits.iter() {
            // Update state for current commit
            if !state.commits_remaining.is_empty() {
                state.advance();
            }
            state.set_new_parent(new_parent);
            state.save(repo_root)?;

            if self.verbose {
                let (current, total) = state.progress();
                println!(
                    "  [{}/{}] {}",
                    current,
                    total,
                    original_commit.message.lines().next().unwrap_or("")
                );
            }

            // Replay this commit's actual change via a real 3-way merge instead
            // of copying its tree verbatim (which silently drops/overwrites
            // anything the new base has that this commit's snapshot doesn't).
            //   base   = tree of this commit's own parent (what it changed FROM)
            //   ours   = tree of the new parent (what we're replaying onto)
            //   theirs = tree of this commit (what it changed TO)
            let base_tree = match original_commit.parents.first() {
                Some(parent_oid) => {
                    let data = odb.read(parent_oid).await?;
                    Commit::deserialize(&data)?.tree
                }
                None => Tree::new().write(odb).await?,
            };

            let new_parent_data = odb.read(&new_parent).await?;
            let ours_tree = Commit::deserialize(&new_parent_data)?.tree;

            let merge_result = merge_engine
                .merge_trees(
                    &base_tree,
                    &ours_tree,
                    &original_commit.tree,
                    MergeStrategy::Recursive,
                )
                .await?;

            if merge_result.has_conflicts() {
                anyhow::bail!(
                    "rebase stopped: conflict replaying commit {} '{}'",
                    &original_oid.to_hex()[..7],
                    original_commit.message.lines().next().unwrap_or("")
                );
            }

            let merged_tree = merge_result
                .tree_oid
                .context("merge produced no tree during rebase")?;

            // Create new commit with the merged tree and new parent
            let new_commit = Commit {
                tree: merged_tree,
                parents: vec![new_parent],
                author: original_commit.author.clone(),
                committer: Signature::now(
                    original_commit.committer.name.clone(),
                    original_commit.committer.email.clone(),
                ),
                message: original_commit.message.clone(),
            };

            let commit_data = new_commit.serialize()?;
            let commit_oid = odb.write(ObjectType::Commit, &commit_data).await?;

            new_parent = commit_oid;

            // Update state with new parent for next iteration
            state.set_new_parent(new_parent);
            state.current_commit = None; // Mark current as complete
            state.save(repo_root)?;
        }

        Ok(new_parent)
    }

    async fn collect_commits(
        &self,
        odb: &Arc<ObjectDatabase>,
        base_oid: &Oid,
        head_oid: &Oid,
    ) -> Result<Vec<(Oid, Commit)>> {
        let mut commits = Vec::new();
        let mut visited = HashSet::new();
        let mut current = *head_oid;

        // Walk back from head to base
        loop {
            if visited.contains(&current) || current == *base_oid {
                break;
            }
            visited.insert(current);

            let data = odb.read(&current).await?;
            let commit = Commit::deserialize(&data)?;

            commits.push((current, commit.clone()));

            // Follow first parent
            if let Some(parent) = commit.parents.first() {
                current = *parent;
            } else {
                break;
            }
        }

        // Reverse to get chronological order
        commits.reverse();
        Ok(commits)
    }

    async fn resolve_branch(&self, refdb: &RefDatabase, branch: &str) -> Result<Oid> {
        // Try as direct OID
        if let Ok(oid) = Oid::from_hex(branch) {
            return Ok(oid);
        }

        // Try as reference
        let ref_result = refdb.read(branch).await;
        match ref_result {
            Ok(r) => r.oid.context(format!("Branch {} has no commit", branch)),
            Err(_) => {
                // Try with refs/heads prefix
                let with_prefix = format!("refs/heads/{}", branch);
                let ref_result = refdb.read(&with_prefix).await;
                match ref_result {
                    Ok(r) => r.oid.context(format!("Branch {} has no commit", branch)),
                    Err(_) => anyhow::bail!("Cannot resolve branch: {}", branch),
                }
            }
        }
    }

    async fn abort_rebase(&self, repo_root: &std::path::Path) -> Result<()> {
        // Check if rebase is in progress
        if !RebaseState::in_progress(repo_root) {
            anyhow::bail!("No rebase in progress");
        }

        let state = RebaseState::load(repo_root)?;

        if !self.quiet {
            println!("{} Aborting rebase...", style("✗").red());
        }

        let storage_path = repo_root.join(".mediagit");
        let refdb = RefDatabase::new(&storage_path);

        // Restore HEAD to original position
        if let Some(ref branch) = state.original_branch {
            // HEAD was on a branch
            let new_ref = Ref::new_direct(branch.clone(), state.original_head);
            refdb.write(&new_ref).await?;
        } else {
            // Detached HEAD
            let new_ref = Ref::new_direct("HEAD".to_string(), state.original_head);
            refdb.write(&new_ref).await?;
        }

        // Clear rebase state
        RebaseState::clear(repo_root)?;

        if !self.quiet {
            println!(
                "{} Rebase aborted. HEAD restored to {}",
                style("✓").green(),
                &state.original_head.to_string()[..7]
            );
        }

        Ok(())
    }

    async fn continue_rebase_process(&self, repo_root: &std::path::Path) -> Result<()> {
        // Check if rebase is in progress
        if !RebaseState::in_progress(repo_root) {
            anyhow::bail!("No rebase in progress");
        }

        let mut state = RebaseState::load(repo_root)?;

        // Check for unresolved conflicts
        if state.has_conflicts() {
            anyhow::bail!(
                "Cannot continue: unresolved conflicts in:\n  {}",
                state
                    .conflict_files
                    .iter()
                    .map(|p| p.display().to_string())
                    .collect::<Vec<_>>()
                    .join("\n  ")
            );
        }

        if !self.quiet {
            println!("{} Continuing rebase...", style("→").cyan());
        }

        let storage_path = repo_root.join(".mediagit");
        let storage = create_storage_backend(repo_root).await?;
        let refdb = RefDatabase::new(&storage_path);
        let odb = Arc::new(ObjectDatabase::with_smart_compression(storage, 1000));

        // Collect remaining commits to apply
        let remaining_commits = self.load_remaining_commits(&odb, &state).await?;

        if remaining_commits.is_empty() {
            // No more commits, finalize
            if let Some(ref branch) = state.original_branch {
                let new_ref = Ref::new_direct(branch.clone(), state.new_parent);
                refdb.write(&new_ref).await?;
            } else {
                let new_ref = Ref::new_direct("HEAD".to_string(), state.new_parent);
                refdb.write(&new_ref).await?;
            }

            let checkout_mgr = CheckoutManager::new(&odb, repo_root);
            checkout_mgr
                .checkout_commit(&state.new_parent)
                .await
                .context("Failed to update working directory after rebase")?;

            RebaseState::clear(repo_root)?;

            if !self.quiet {
                println!("{} Rebase complete", style("✓").green().bold());
            }
            return Ok(());
        }

        // Continue applying remaining commits
        let result = self
            .apply_commits(repo_root, &odb, &refdb, &mut state, &remaining_commits)
            .await;

        match result {
            Ok(new_head) => {
                // Update HEAD
                if let Some(ref branch) = state.original_branch {
                    let new_ref = Ref::new_direct(branch.clone(), new_head);
                    refdb.write(&new_ref).await?;
                } else {
                    let new_ref = Ref::new_direct("HEAD".to_string(), new_head);
                    refdb.write(&new_ref).await?;
                }

                let checkout_mgr = CheckoutManager::new(&odb, repo_root);
                checkout_mgr
                    .checkout_commit(&new_head)
                    .await
                    .context("Failed to update working directory after rebase")?;

                RebaseState::clear(repo_root)?;

                if !self.quiet {
                    println!(
                        "{} Successfully rebased remaining commit(s)",
                        style("✓").green().bold()
                    );
                }
                Ok(())
            }
            Err(e) => Err(e),
        }
    }

    async fn skip_commit(&self, repo_root: &std::path::Path) -> Result<()> {
        // Check if rebase is in progress
        if !RebaseState::in_progress(repo_root) {
            anyhow::bail!("No rebase in progress");
        }

        let mut state = RebaseState::load(repo_root)?;

        if state.current_commit.is_none() && state.commits_remaining.is_empty() {
            anyhow::bail!("No commit to skip");
        }

        if !self.quiet {
            if let Some(current) = state.current_commit {
                println!(
                    "{} Skipping commit {}...",
                    style("→").cyan(),
                    &current.to_string()[..7]
                );
            } else {
                println!("{} Skipping commit...", style("→").cyan());
            }
        }

        // Skip current commit
        state.skip_current();
        state.save(repo_root)?;

        // Continue with remaining
        self.continue_rebase_process(repo_root).await
    }

    /// Load remaining commits from state
    async fn load_remaining_commits(
        &self,
        odb: &Arc<ObjectDatabase>,
        state: &RebaseState,
    ) -> Result<Vec<(Oid, Commit)>> {
        let mut commits = Vec::new();

        for oid in &state.commits_remaining {
            let data = odb.read(oid).await?;
            let commit = Commit::deserialize(&data)?;
            commits.push((*oid, commit));
        }

        Ok(commits)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::utils::test_support::{init_repo_with_commit, REPO_ENV_LOCK};
    use clap::Parser;
    use tempfile::TempDir;

    fn parse(args: &[&str]) -> Result<RebaseCmd, clap::Error> {
        let mut full = vec!["rebase"];
        full.extend_from_slice(args);
        RebaseCmd::try_parse_from(full)
    }

    #[test]
    fn parse_basic_upstream() {
        let cmd = parse(&["main"]).unwrap();
        assert_eq!(cmd.upstream, "main");
        assert!(cmd.branch.is_none());
        assert!(!cmd.abort);
    }

    #[test]
    fn parse_missing_upstream_is_error() {
        assert!(parse(&[]).is_err());
    }

    #[test]
    fn parse_upstream_and_branch() {
        let cmd = parse(&["main", "feature"]).unwrap();
        assert_eq!(cmd.upstream, "main");
        assert_eq!(cmd.branch.as_deref(), Some("feature"));
    }

    #[test]
    fn parse_all_flags() {
        let cmd = parse(&[
            "main",
            "--keep-empty",
            "--abort",
            "--continue-rebase",
            "--skip",
            "-q",
            "-v",
        ])
        .unwrap();
        assert!(cmd.keep_empty);
        assert!(cmd.abort);
        assert!(cmd.continue_rebase);
        assert!(cmd.skip);
        assert!(cmd.quiet);
        assert!(cmd.verbose);
    }

    /// Guards `MEDIAGIT_REPO` across the `.await` points in `execute()`
    /// (see `REPO_ENV_LOCK` docs).
    #[allow(clippy::await_holding_lock)]
    async fn execute_in(repo_path: &std::path::Path, cmd: &RebaseCmd) -> Result<()> {
        let _guard = REPO_ENV_LOCK.lock().unwrap();
        std::env::set_var("MEDIAGIT_REPO", repo_path);
        let result = cmd.execute().await;
        std::env::remove_var("MEDIAGIT_REPO");
        result
    }

    #[tokio::test]
    async fn execute_no_repo_is_error() {
        let temp = TempDir::new().unwrap();
        let cmd = parse(&["main"]).unwrap();
        let err = execute_in(temp.path(), &cmd).await.unwrap_err();
        assert!(err.to_string().contains("Not a mediagit repository"));
    }

    #[tokio::test]
    async fn execute_unknown_upstream_is_error() {
        let temp = TempDir::new().unwrap();
        init_repo_with_commit(temp.path()).await;

        let cmd = parse(&["does-not-exist"]).unwrap();
        let err = execute_in(temp.path(), &cmd).await.unwrap_err();
        assert!(err.to_string().contains("Cannot resolve branch"));
    }

    #[test]
    fn interactive_flag_is_rejected_at_parse_time() {
        // -i/--interactive was removed from clap (unbuilt feature, pre-GA)
        assert!(parse(&["main", "-i"]).is_err());
        assert!(parse(&["main", "--interactive"]).is_err());
    }

    #[tokio::test]
    async fn execute_rebase_merges_is_not_implemented_error() {
        let temp = TempDir::new().unwrap();
        init_repo_with_commit(temp.path()).await;

        let cmd = parse(&["main", "--rebase-merges"]).unwrap();
        let err = execute_in(temp.path(), &cmd).await.unwrap_err();
        assert!(err
            .to_string()
            .contains("Rebase with merge commits not yet implemented"));
    }

    #[tokio::test]
    async fn abort_without_rebase_in_progress_is_error() {
        let temp = TempDir::new().unwrap();
        init_repo_with_commit(temp.path()).await;

        let cmd = parse(&["main", "--abort"]).unwrap();
        let err = execute_in(temp.path(), &cmd).await.unwrap_err();
        assert!(err.to_string().contains("No rebase in progress"));
    }

    #[tokio::test]
    async fn skip_without_rebase_in_progress_is_error() {
        let temp = TempDir::new().unwrap();
        init_repo_with_commit(temp.path()).await;

        let cmd = parse(&["main", "--skip"]).unwrap();
        let err = execute_in(temp.path(), &cmd).await.unwrap_err();
        assert!(err.to_string().contains("No rebase in progress"));
    }
}
