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
    CheckoutManager, Commit, Index, LcaFinder, MergeEngine, MergeStrategy, ObjectDatabase,
    ObjectType, Oid, Ref, RefDatabase, Signature, Tree, TreeEntry, apply_merge_to_workdir,
    resolve_revision,
};
use std::collections::HashSet;
use std::sync::Arc;

use super::super::repo::{create_storage_backend, find_repo_root};
use super::rebase_state::RebaseState;

/// Rebase commits
#[derive(Parser, Debug)]
pub struct RebaseCmd {
    /// Upstream branch to rebase onto
    /// FOUND-3: optional because the mode flags take no upstream —
    /// `rebase --continue` / `--abort` / `--skip` resume or discard an
    /// operation already described by `rebase-apply/state.json`. As a required
    /// positional it made every one of them impossible to invoke without
    /// supplying an argument they then ignore.
    #[arg(value_name = "UPSTREAM")]
    pub upstream: Option<String>,

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
    #[arg(long = "continue", alias = "continue-rebase", hide = true)]
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

        let upstream = self.upstream.as_deref().ok_or_else(|| {
            anyhow::anyhow!(
                "missing <UPSTREAM>
  usage: mediagit rebase <UPSTREAM> [BRANCH]
                   (--continue, --abort and --skip take no upstream)"
            )
        })?;

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
        let upstream_oid = resolve_revision(upstream, &refdb, &odb).await?;

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

        // WT-3: rebase had no pre-flight dirty check at all. Refuse before
        // any ref or working-tree write.
        crate::worktree_guard::AtRisk::check(
            &repo_root,
            &odb,
            Some(&current_oid),
            Some(&upstream_oid),
        )
        .await?
        .ensure_clean("rebase")?;

        // WT-1: bound what the checkouts below may delete.
        let tracked =
            crate::worktree_guard::tracked_paths(&repo_root, &odb, Some(&current_oid)).await?;

        if !self.quiet {
            println!(
                "{} Rebasing onto {}...",
                style("🔄").cyan().bold(),
                style(upstream).yellow()
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
                let checkout_mgr =
                    CheckoutManager::new(&odb, &repo_root).with_tracked_paths(tracked);
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
                // WT-4: materialise the conflict and record it before stopping.
                //
                // This used to bail immediately. Two things went wrong as a
                // result. Nothing was written to the working tree, so the user
                // had no markers to resolve — `--continue` could only guess.
                // And `state.advance()` above had already removed this commit
                // from `commits_remaining` *and* persisted that, so
                // `--continue` resumed from the next commit and silently
                // discarded this one while reporting success.
                //
                // `advance()` does record it in `current_commit`, so the
                // information was there all along; it simply was not written
                // down as a conflict, and `continue_rebase_process` only ever
                // read `commits_remaining`.
                let ours_tree_obj = Tree::read(odb, &ours_tree).await?;
                let theirs_tree_obj = Tree::read(odb, &original_commit.tree).await?;
                let mut index = Index::load(repo_root)?;

                apply_merge_to_workdir(
                    &merge_result,
                    &ours_tree_obj,
                    &theirs_tree_obj,
                    odb,
                    repo_root,
                    &mut index,
                    *original_oid,
                    state.original_head,
                )
                .await
                .context("Failed to write rebase conflict to the working directory")?;

                index.save(repo_root)?;

                let conflict_paths: Vec<std::path::PathBuf> = merge_result
                    .conflicts
                    .iter()
                    .map(|c| std::path::PathBuf::from(&c.path))
                    .collect();
                state.set_conflicts(conflict_paths);
                state.set_new_parent(new_parent);
                state.save(repo_root)?;

                println!(
                    "{} Rebase stopped: conflict replaying {} '{}'",
                    style("⚠").yellow().bold(),
                    &original_oid.to_hex()[..7],
                    original_commit.message.lines().next().unwrap_or("")
                );
                for conflict in &merge_result.conflicts {
                    println!("  {} {}", style("conflict:").red(), conflict.path);
                }
                println!(
                    "  Resolve the file(s), {}, then run {}",
                    style("mediagit add <file>").cyan(),
                    style("mediagit rebase --continue").cyan()
                );
                println!(
                    "  Or abandon the rebase with {}",
                    style("mediagit rebase --abort").cyan()
                );

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

        // WT-4/WT-9: refuse while any path is still unacknowledged.
        //
        // This asks the index, not the file contents. An earlier version
        // scanned for `<<<<<<<` markers, which is a text-only signal: a
        // conflicting PSD never gets markers, because inlining them would
        // corrupt it — the resolver checks out one side provisionally
        // instead. Marker-scanning therefore waved through exactly the file
        // types this system exists to version, letting `--continue` commit a
        // side the user never saw.
        let unresolved = Index::load(repo_root)?.unresolved_paths();
        if !unresolved.is_empty() {
            anyhow::bail!(
                "Cannot continue: {} path(s) still unresolved:\n  {}\n\
                 Review each, then `mediagit add <path>` to accept it \
                 (this works for binary files too — staging is the \
                 acknowledgement, whether or not you edited anything).",
                unresolved.len(),
                unresolved
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

        // WT-1: bound what the checkouts below may delete to files tracked at
        // the pre-rebase HEAD (plus anything staged during conflict
        // resolution) — untracked work is not the rebase's to remove.
        let tracked =
            crate::worktree_guard::tracked_paths(repo_root, &odb, Some(&state.original_head))
                .await?;

        // WT-4: finish the commit that conflicted before moving on.
        //
        // `state.current_commit` is the commit we stopped on. Previously this
        // function went straight to `commits_remaining` — which `advance()`
        // had already removed it from — so the user's resolved work was
        // dropped and the rebase reported success. Commit the resolution
        // under the original commit's identity and message, then continue.
        if let Some(conflicted_oid) = state.current_commit {
            let original = Commit::read(&odb, &conflicted_oid).await?;

            let index = Index::load(repo_root)?;
            let mut tree = Tree::new();
            for entry in index.entries() {
                let path_str = entry.path.to_string_lossy();
                if mediagit_versioning::is_stage_debris_key(&path_str) {
                    continue;
                }
                tree.add_entry(TreeEntry::new(
                    path_str.to_string(),
                    mediagit_versioning::FileMode::from_u32(entry.mode)
                        .unwrap_or(mediagit_versioning::FileMode::Regular),
                    entry.oid,
                ));
            }
            let tree_oid = tree.write(&odb).await?;

            let resolved = Commit {
                tree: tree_oid,
                parents: vec![state.new_parent],
                author: original.author.clone(),
                committer: Signature::now(
                    original.committer.name.clone(),
                    original.committer.email.clone(),
                ),
                message: original.message.clone(),
            };
            let resolved_oid = odb
                .write(ObjectType::Commit, &resolved.serialize()?)
                .await?;

            state.set_new_parent(resolved_oid);
            state.current_commit = None;
            state.set_conflicts(Vec::new());
            state.save(repo_root)?;

            if !self.quiet {
                println!(
                    "  {} resolved {} '{}'",
                    style("✓").green(),
                    &conflicted_oid.to_hex()[..7],
                    original.message.lines().next().unwrap_or("")
                );
            }
        }

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

            let checkout_mgr = CheckoutManager::new(&odb, repo_root).with_tracked_paths(tracked);
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

                let checkout_mgr =
                    CheckoutManager::new(&odb, repo_root).with_tracked_paths(tracked);
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
#[allow(unsafe_code)] // edition-2024: test-only env::set_var/remove_var requires unsafe
mod tests {
    use super::*;
    use crate::commands::utils::test_support::{REPO_ENV_LOCK, init_repo_with_commit};
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
        assert_eq!(cmd.upstream.as_deref(), Some("main"));
        assert!(cmd.branch.is_none());
        assert!(!cmd.abort);
    }

    /// FOUND-3: `upstream` is no longer a required positional, so a bare
    /// `rebase` now *parses* and is rejected at execution instead. That move
    /// is the whole point — as a required arg it made `rebase --continue`,
    /// `--abort` and `--skip` impossible to invoke, since clap demanded an
    /// upstream those modes ignore.
    #[test]
    fn parse_missing_upstream_is_accepted_and_deferred_to_runtime() {
        let cmd = parse(&[]).expect("bare `rebase` must parse; the error belongs at run time");
        assert!(
            cmd.upstream.is_none(),
            "no upstream given, so it must be None for execute() to reject"
        );
        assert!(!cmd.abort && !cmd.continue_rebase && !cmd.skip);
    }

    /// The mode flags must parse standalone — the regression FOUND-3 describes.
    #[test]
    fn parse_mode_flags_without_upstream() {
        for flag in ["--continue", "--abort", "--skip"] {
            let cmd = parse(&[flag])
                .unwrap_or_else(|e| panic!("`rebase {flag}` must parse without an upstream: {e}"));
            assert!(cmd.upstream.is_none());
        }
    }

    #[test]
    fn parse_upstream_and_branch() {
        let cmd = parse(&["main", "feature"]).unwrap();
        assert_eq!(cmd.upstream.as_deref(), Some("main"));
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
        assert!(err.to_string().contains("Cannot resolve revision"));
    }

    #[tokio::test]
    async fn execute_upstream_resolves_via_refs_remotes() {
        // QA-004: `pull -r` passes a tracking ref like "origin/main" as the
        // upstream, which only exists under refs/remotes. Rebase must resolve
        // it instead of failing with "Cannot resolve revision".
        let temp = TempDir::new().unwrap();
        let head_oid = init_repo_with_commit(temp.path()).await;

        let refdb = RefDatabase::new(temp.path().join(".mediagit"));
        refdb
            .write(&Ref::new_direct(
                "refs/remotes/origin/main".to_string(),
                head_oid,
            ))
            .await
            .unwrap();

        let cmd = parse(&["origin/main"]).unwrap();
        let result = execute_in(temp.path(), &cmd).await;
        assert!(
            result.is_ok(),
            "expected resolution to succeed: {:?}",
            result
        );
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
        assert!(
            err.to_string()
                .contains("Rebase with merge commits not yet implemented")
        );
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
