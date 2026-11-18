// SPDX-License-Identifier: BUSL-1.1
// Copyright (C) 2025-2026 Aswin Krishnamoorthy

use super::super::output::{self, ConflictStop};
use super::super::repo::{create_storage_backend, find_repo_root};
use anyhow::{Context, Result};
use clap::Parser;
use console::style;
use mediagit_versioning::{
    CheckoutManager, Commit, Index, MergeEngine, ObjectDatabase, Oid, Ref, RefDatabase, Tree,
    apply_merge_to_workdir,
};
use serde::Serialize;
use std::path::PathBuf;
use std::sync::Arc;

/// Apply changes from existing commits
#[derive(Parser, Debug)]
pub struct CherryPickCmd {
    /// Commit hash(es) to cherry-pick
    /// (not required by clap: `--continue`, `--abort` and `--skip` take none)
    #[arg(value_name = "COMMITS")]
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

    /// Use original commit message
    #[arg(short = 'x', long)]
    pub append_message: bool,

    /// Quiet mode
    #[arg(short, long)]
    pub quiet: bool,
}

/// `--machine` data for `cherry-pick` (start, `--continue`, `--skip`, `--abort`).
#[derive(Debug, Clone, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[cfg_attr(test, ts(export, export_to = "../../../ui/src/api/generated/"))]
pub struct CherryPickReport {
    /// `start`, `continue`, `skip` or `abort`.
    pub action: String,
    /// `picked`, `not_committed`, `completed`, `aborted` or `conflict` (the
    /// command then also exits 1; resolve, `add`, then `--continue`).
    pub status: String,
    /// New commits created by this run.
    pub commits: Vec<String>,
    /// `conflict`: the commit being picked and the unresolved paths.
    pub conflict_commit: Option<String>,
    pub conflicts: Vec<String>,
}

struct Outcome {
    status: &'static str,
    commits: Vec<String>,
}

fn outcome(status: &'static str, commits: Vec<String>) -> Result<Outcome> {
    Ok(Outcome { status, commits })
}

impl CherryPickCmd {
    fn quiet(&self) -> bool {
        self.quiet || output::machine_mode()
    }

    pub async fn execute(&self) -> Result<()> {
        let action = if self.abort {
            "abort"
        } else if self.continue_pick {
            "continue"
        } else if self.skip {
            "skip"
        } else {
            "start"
        };
        match self.run().await {
            Ok(o) if output::machine_mode() => output::emit_ok(&CherryPickReport {
                action: action.into(),
                status: o.status.into(),
                commits: o.commits,
                conflict_commit: None,
                conflicts: Vec::new(),
            }),
            Ok(_) => Ok(()),
            Err(e) if output::machine_mode() => match e.downcast_ref::<ConflictStop>() {
                Some(c) => {
                    output::emit_ok(&CherryPickReport {
                        action: action.into(),
                        status: "conflict".into(),
                        commits: Vec::new(),
                        conflict_commit: c.commit.clone(),
                        conflicts: c.paths.clone(),
                    })?;
                    std::process::exit(1);
                }
                None => Err(e),
            },
            Err(e) => Err(e),
        }
    }

    async fn run(&self) -> Result<Outcome> {
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

        if self.commits.is_empty() {
            anyhow::bail!(
                "missing <COMMITS>\n  usage: mediagit cherry-pick <COMMITS>...\n  \
                 (--continue, --abort and --skip take no commits)"
            );
        }

        // Start new cherry-pick operation
        self.start_cherrypick(&repo_root).await
    }

    async fn start_cherrypick(&self, repo_root: &PathBuf) -> Result<Outcome> {
        // X1: a stop after the first version cannot be continued or skipped
        // without mixing the versions, so refuse before anything is written.
        if self.no_commit && self.commits.len() > 1 {
            anyhow::bail!(
                "--no-commit takes one version at a time. Pick them one at a time, \
                 or leave out -n to save each one. Nothing was changed."
            );
        }
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
        crate::repo::ensure_no_unsafe_paths(&odb, repo_root, &[current_oid]).await?;
        crate::worktree_guard::AtRisk::check(repo_root, &odb, Some(&current_oid), None)
            .await?
            .ensure_clean("cherry-pick")?;

        if !self.quiet() {
            println!(
                "{} Starting cherry-pick on branch at {}",
                style("→").cyan(),
                style(current_oid.to_hex()).yellow()
            );
        }

        // BUG-008: check every listed commit before the first one is applied,
        // so a refusal cannot come after earlier picks are already committed.
        let mut listed = vec![current_oid];
        for commit_ref in &self.commits {
            let oid = self
                .resolve_commit(&refdb, repo_root, commit_ref)
                .await
                .context(format!("Failed to resolve commit: {}", commit_ref))?;
            // J3: refusals are found here, before any pick, so they never
            // leave a "stopped" state behind.
            if Commit::read(odb.as_ref(), &oid).await?.parents.is_empty() {
                anyhow::bail!(
                    "Cannot cherry-pick {}: it is the first version, with nothing \
                     before it to compare. Nothing was changed.",
                    &oid.to_hex()[..7]
                );
            }
            crate::worktree_guard::AtRisk::check(repo_root, &odb, Some(&current_oid), Some(&oid))
                .await?
                .ensure_clean("cherry-pick")?;
            listed.push(oid);
        }
        // The state file stores full ids: a branch name or short id would not
        // parse in `--continue`.
        let resolved: Vec<String> = listed[1..].iter().map(|o| o.to_hex()).collect();
        crate::repo::ensure_no_unsafe_paths(&odb, repo_root, &listed).await?;

        // Process each commit to cherry-pick
        let mut picked_commits = Vec::new();
        let mut created = Vec::new();
        // J2: under `--no-commit` each pick goes on top of the previous result.
        let mut onto = None;
        // Y2: a run resumed by `--continue` / `--skip` has its rest on file.
        let resuming = repo_root.join(".mediagit/CHERRY_PICK_STATE").exists();
        for (idx, &commit_oid) in listed[1..].iter().enumerate() {
            if !self.quiet() {
                println!(
                    "{} Applying commit {}",
                    style("→").cyan(),
                    style(commit_oid.to_hex()).yellow()
                );
            }

            match self
                .apply_commit(&odb, &refdb, repo_root, &commit_oid, onto)
                .await
            {
                Ok((merged_tree_oid, result_oid)) => {
                    picked_commits.push(commit_oid);
                    if self.no_commit {
                        onto = Some(result_oid);
                    }

                    if !self.no_commit {
                        // Create commit automatically, using the merged tree
                        // produced by the 3-way merge above (not an index
                        // rebuild, which would drop every file not touched
                        // by this commit).
                        let new_oid = match self
                            .create_cherry_pick_commit(
                                odb.as_ref(),
                                &refdb,
                                repo_root,
                                &commit_oid,
                                Some(merged_tree_oid),
                            )
                            .await
                        {
                            Ok(oid) => oid,
                            Err(e) => {
                                // E.g. `empty_commit` mid-sequence: this pick is
                                // the current one, so `--skip` moves on to the
                                // next. With nothing left after it there is
                                // nothing to continue, so leave no state.
                                if idx + 1 < self.commits.len() {
                                    self.save_cherrypick_state(repo_root, &resolved, idx)
                                        .await?;
                                } else {
                                    remove_state(repo_root)?;
                                }
                                return Err(e);
                            }
                        };
                        created.push(new_oid.to_hex());
                        // Y2: this pick is done, so it leaves the list now; a
                        // failure in the next one must not make it run twice.
                        if resuming {
                            write_rest_state(repo_root, new_oid, &resolved[idx + 1..])?;
                        }
                    }
                }
                Err(e) => {
                    // J3: only a conflict is a stop to resume. Any other
                    // refusal leaves no state.
                    if e.downcast_ref::<ConflictStop>().is_none() {
                        return Err(e);
                    }
                    self.save_cherrypick_state(repo_root, &resolved, idx)
                        .await?;

                    if !output::machine_mode() {
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
                    }

                    return Err(e);
                }
            }
        }

        // X3: the rest of a `--continue` / `--skip` ran; its state goes now.
        remove_state(repo_root)?;

        if !self.quiet() {
            println!(
                "{} Successfully cherry-picked {} commit(s)",
                style("✓").green(),
                picked_commits.len()
            );
        }

        outcome(
            if self.no_commit {
                "not_committed"
            } else {
                "picked"
            },
            created,
        )
    }

    /// Pick `commit_oid` on top of `onto` (HEAD when `None`). Returns the
    /// merged tree and a commit holding it (for the next `--no-commit` pick).
    async fn apply_commit(
        &self,
        odb: &Arc<ObjectDatabase>,
        refdb: &RefDatabase,
        repo_root: &PathBuf,
        commit_oid: &Oid,
        onto: Option<Oid>,
    ) -> Result<(Oid, Oid)> {
        // Load the commit
        let commit = Commit::read(odb.as_ref(), commit_oid)
            .await
            .context("Failed to read commit")?;

        // Verify parent exists
        if commit.parents.is_empty() {
            anyhow::bail!("Cannot cherry-pick initial commit");
        }

        let head_oid = refdb.resolve("HEAD").await?;
        let current_oid = onto.unwrap_or(head_oid);

        // WT-1: the merge result can only materialize paths from ours (all
        // tracked) or theirs, so checking collisions against the picked commit
        // is exact. Done before any write.
        crate::worktree_guard::AtRisk::check(repo_root, odb, Some(&current_oid), Some(commit_oid))
            .await?
            .ensure_clean("cherry-pick")?;
        crate::repo::ensure_no_unsafe_paths(odb, repo_root, &[*commit_oid]).await?;

        // BUG-028: base = the picked version's parent, so only that version's
        // own change is applied (the branch merge-base would drag in every
        // earlier version of the source branch).
        let parent_commit = Commit::read(odb.as_ref(), &commit.parents[0]).await?;
        let current_commit = Commit::read(odb.as_ref(), &current_oid).await?;
        let merger = MergeEngine::new(odb.clone());
        let merge_result = merger
            .merge_trees(
                &parent_commit.tree,
                &current_commit.tree,
                &commit.tree,
                mediagit_versioning::MergeStrategy::Recursive,
            )
            .await?;

        if !merge_result.conflicts.is_empty() {
            // Write conflict markers via the shared, binary-aware writer (matches
            // merge.rs's continue-merge path) instead of the old bespoke
            // write_conflicts, which Debug-printed OIDs into files and
            // corrupted binaries.
            let ours_tree = Tree::read(odb.as_ref(), &current_commit.tree).await?;

            let mut index = Index::load(repo_root)?;

            apply_merge_to_workdir(
                &merge_result,
                &ours_tree,
                odb,
                repo_root,
                &mut index,
                *commit_oid,
                head_oid,
            )
            .await?;

            index.save(repo_root)?;

            return Err(ConflictStop {
                message: "Merge conflicts detected".into(),
                paths: merge_result
                    .conflicts
                    .iter()
                    .map(|c| c.path.clone())
                    .collect(),
                commit: Some(commit_oid.to_hex()),
            }
            .into());
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

        // `--no-commit` leaves the result staged as changes on top of HEAD,
        // deletions included (the commit path rebuilds nothing: the new
        // commit is what the index should match).
        if self.no_commit {
            let head_tree = Tree::read(
                odb.as_ref(),
                &Commit::read(odb.as_ref(), &head_oid).await?.tree,
            )
            .await?;
            let merged = Tree::read(odb.as_ref(), &tree_oid).await?;
            let mut index = Index::new();
            mediagit_versioning::stage_tree(&mut index, &head_tree, &merged);
            index.save(repo_root)?;
        }

        Ok((tree_oid, temp_oid))
    }

    async fn create_cherry_pick_commit(
        &self,
        odb: &ObjectDatabase,
        refdb: &RefDatabase,
        repo_root: &std::path::Path,
        original_oid: &Oid,
        merged_tree_oid: Option<Oid>,
    ) -> Result<Oid> {
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

        // Get current HEAD as parent
        let current_oid = refdb.resolve("HEAD").await?;

        // Use the merged tree from the 3-way merge when available. After a
        // conflict (`--continue`) it is HEAD's tree with the resolution
        // staged on top — never the index alone.
        let tree_oid = match merged_tree_oid {
            Some(oid) => oid,
            None => {
                let index = Index::load(repo_root)?;
                let head_tree = Commit::read(odb, &current_oid).await?.tree;
                mediagit_versioning::tree_with_index(odb, &head_tree, &index).await?
            }
        };

        // A pick that changes nothing (the change is already on this branch)
        // must not manufacture an empty commit and call it success.
        if Commit::read(odb, &current_oid).await?.tree == tree_oid {
            return Err(output::CodedError::new(
                "empty_commit",
                format!(
                    "nothing to commit - the change from {} is already on this branch",
                    &original_oid.to_hex()[..7]
                ),
            )
            .into());
        }

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

        if !self.quiet() {
            println!(
                "{} Created commit {}",
                style("✓").green(),
                style(commit_oid.to_hex()).yellow()
            );
        }

        Ok(commit_oid)
    }

    async fn continue_cherrypick(&self, repo_root: &PathBuf) -> Result<Outcome> {
        let state_path = repo_root.join(".mediagit/CHERRY_PICK_STATE");
        if !state_path.exists() {
            anyhow::bail!("No cherry-pick in progress");
        }

        // Load state
        // nosemgrep: rust.actix.path-traversal.tainted-path.tainted-path -- repo-internal constant path
        let state_json = std::fs::read_to_string(&state_path)?;
        let state: CherryPickState = serde_json::from_str(&state_json)?;

        // Refuse while any path is still unacknowledged (same rule as rebase
        // and revert): staging a path is the acknowledgement. The state file
        // stays, so the cherry-pick remains resumable.
        let unresolved = Index::load(repo_root)?.unresolved_paths();
        if !unresolved.is_empty() {
            anyhow::bail!(
                "Cannot continue: {} path(s) still unresolved:\n  {}\n\
                 Review each, then `mediagit add <path>` to accept it \
                 (binary files included — staging is the acknowledgement), \
                 or `mediagit cherry-pick --abort`.",
                unresolved.len(),
                unresolved
                    .iter()
                    .map(|p| p.display().to_string())
                    .collect::<Vec<_>>()
                    .join("\n  ")
            );
        }

        // Create commit for current pick
        let mediagit_dir = repo_root.join(".mediagit");
        let storage = create_storage_backend(repo_root).await?;
        let odb = ObjectDatabase::with_smart_compression(storage.clone(), 1000);
        let refdb = RefDatabase::new(&mediagit_dir);

        let mut created = Vec::new();
        if let Some(current) = &state.current_commit {
            let current_oid = Oid::from_hex(current)?;
            let new_oid = self
                .create_cherry_pick_commit(&odb, &refdb, repo_root, &current_oid, None)
                .await?;
            created.push(new_oid.to_hex());

            // The conflicted pick is done; the next one rewrites these if it
            // conflicts. Its staged entries are committed, so they go too.
            mediagit_versioning::clear_merge_state(&mediagit_dir)?;
            let mut index = Index::load(repo_root)?;
            index.clear();
            index.save(repo_root)?;
        }

        // Continue with remaining commits
        if state.remaining_commits.is_empty() {
            std::fs::remove_file(&state_path)?;
            if !self.quiet() {
                println!("{} Cherry-pick complete", style("✓").green());
            }
            return outcome("completed", created);
        }
        // X3: no pick is pending now, but the rest stays listed until it ran,
        // so a refusal in it can be continued after the cause is fixed.
        write_rest_state(
            repo_root,
            refdb.resolve("HEAD").await?,
            &state.remaining_commits,
        )?;

        // Process remaining commits
        let remaining: Vec<String> = state.remaining_commits.clone();
        drop(state); // Release state before recursive call

        let cmd = Self {
            commits: remaining,
            continue_pick: false,
            abort: false,
            skip: false,
            no_commit: self.no_commit,
            append_message: self.append_message,
            quiet: self.quiet,
        };

        let mut rest = cmd.start_cherrypick(repo_root).await?;
        created.append(&mut rest.commits);
        outcome(rest.status, created)
    }

    async fn skip_cherrypick(&self, repo_root: &PathBuf) -> Result<Outcome> {
        let state_path = repo_root.join(".mediagit/CHERRY_PICK_STATE");
        if !state_path.exists() {
            anyhow::bail!("No cherry-pick in progress");
        }

        // Load state
        // nosemgrep: rust.actix.path-traversal.tainted-path.tainted-path -- repo-internal constant path
        let state_json = std::fs::read_to_string(&state_path)?;
        let mut state: CherryPickState = serde_json::from_str(&state_json)?;
        // X3: with no pick pending (the rest was refused), skip the version
        // that was refused: the first one left.
        let pending = state.current_commit.is_some();
        if !pending && !state.remaining_commits.is_empty() {
            state.current_commit = Some(state.remaining_commits.remove(0));
        }

        if !self.quiet() {
            println!(
                "{} Skipping commit {}",
                style("→").cyan(),
                state.current_commit.as_deref().unwrap_or("unknown")
            );
        }

        // Validate first: every remaining pick must resolve and be safe, so a
        // refusal happens before anything is changed.
        let mediagit_dir = repo_root.join(".mediagit");
        let storage = create_storage_backend(repo_root).await?;
        let odb = Arc::new(ObjectDatabase::with_smart_compression(storage, 1000));
        let refdb = RefDatabase::new(&mediagit_dir);
        let head_oid = refdb.resolve("HEAD").await?;
        let mut listed = vec![head_oid];
        for commit_ref in &state.remaining_commits {
            listed.push(
                self.resolve_commit(&refdb, repo_root, commit_ref)
                    .await
                    .context(format!("Failed to resolve commit: {}", commit_ref))?,
            );
        }
        crate::repo::ensure_no_unsafe_paths(&odb, repo_root, &listed).await?;

        // Then drop the skipped pick's conflict: the paths it touched go back
        // to HEAD (other edits stay), index cleared, merge files removed. The
        // stop is over; the rest saves a new state only if it conflicts.
        if pending {
            crate::worktree_guard::drop_skipped_step(repo_root, &odb, &head_oid).await?;
        }

        // Continue with remaining commits (skip current)
        if state.remaining_commits.is_empty() {
            std::fs::remove_file(&state_path)?;
            if !self.quiet() {
                println!("{} Cherry-pick complete", style("✓").green());
            }
            return outcome("completed", Vec::new());
        }

        // X3: listed until it ran (see `continue_cherrypick`).
        write_rest_state(repo_root, head_oid, &state.remaining_commits)?;
        let remaining: Vec<String> = state.remaining_commits.clone();
        drop(state);

        let cmd = Self {
            commits: remaining,
            continue_pick: false,
            abort: false,
            skip: false,
            no_commit: self.no_commit,
            append_message: self.append_message,
            quiet: self.quiet,
        };

        cmd.start_cherrypick(repo_root).await
    }

    async fn abort_cherrypick(&self, repo_root: &PathBuf) -> Result<Outcome> {
        let state_path = repo_root.join(".mediagit/CHERRY_PICK_STATE");
        if !state_path.exists() {
            anyhow::bail!("No cherry-pick in progress");
        }

        // Load state to get original HEAD
        // nosemgrep: rust.actix.path-traversal.tainted-path.tainted-path -- repo-internal constant path
        let state_json = std::fs::read_to_string(&state_path)?;
        let state: CherryPickState = serde_json::from_str(&state_json)?;

        // Y1: no pick is stopped (the rest was refused), so HEAD already is the
        // original and any edit on disk is the user's. Only the state goes.
        if let (Some(original_head), Some(_)) = (&state.original_head, &state.current_commit) {
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

            // The index still stages the conflict; an empty one means clean.
            let mut index = Index::load(repo_root)?;
            index.clear();
            index.save(repo_root)?;
        }

        // Clean up state
        std::fs::remove_file(&state_path)?;
        mediagit_versioning::clear_merge_state(&repo_root.join(".mediagit"))?;

        if !self.quiet() {
            println!("{} Cherry-pick aborted", style("✓").green());
        }

        outcome("aborted", Vec::new())
    }

    async fn save_cherrypick_state(
        &self,
        repo_root: &std::path::Path,
        all_commits: &[String],
        current_idx: usize,
    ) -> Result<()> {
        let mediagit_dir = repo_root.join(".mediagit");
        let refdb = RefDatabase::new(&mediagit_dir);

        let original_head = refdb.resolve("HEAD").await?.to_hex();

        let current_commit = all_commits.get(current_idx).cloned();
        let remaining_commits = all_commits[current_idx + 1..].to_vec();

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

/// A stop with no pick pending: `remaining` still to run on top of `head`.
fn write_rest_state(repo_root: &std::path::Path, head: Oid, remaining: &[String]) -> Result<()> {
    let state = CherryPickState {
        original_head: Some(head.to_hex()),
        current_commit: None,
        remaining_commits: remaining.to_vec(),
    };
    std::fs::write(
        repo_root.join(".mediagit/CHERRY_PICK_STATE"),
        serde_json::to_string_pretty(&state)?,
    )?;
    Ok(())
}

fn remove_state(repo_root: &std::path::Path) -> Result<()> {
    match std::fs::remove_file(repo_root.join(".mediagit/CHERRY_PICK_STATE")) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e.into()),
        _ => Ok(()),
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
    fn parse_missing_commits_is_deferred_to_runtime() {
        // Not a clap error: `--continue`, `--abort` and `--skip` take no
        // commits (execute() rejects a bare `cherry-pick`).
        assert!(parse(&[]).unwrap().commits.is_empty());
        assert!(parse(&["--abort"]).unwrap().abort);
    }

    #[test]
    fn parse_all_flags() {
        let cmd = parse(&["c1", "-n", "-x", "-q"]).unwrap();
        assert!(cmd.no_commit);
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
