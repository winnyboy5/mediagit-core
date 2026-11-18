// SPDX-License-Identifier: BUSL-1.1
// Copyright (C) 2025-2026 Aswin Krishnamoorthy

//! Revert commits by creating new commits that undo changes.

use anyhow::{Context, Result};
use clap::Parser;
use console::style;
use serde::Serialize;
use std::path::Path;
use std::sync::Arc;
use tokio::fs;

use mediagit_versioning::{
    CheckoutManager, Commit, Index, MergeEngine, MergeStrategy, ObjectDatabase, Oid, RefDatabase,
    Reflog, ReflogEntry, Signature, Tree,
};

use super::super::output;
use super::super::repo::{create_storage_backend, find_repo_root};

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
    #[arg(id = "continue", long = "continue", alias = "continue-revert", hide = true, conflicts_with_all = ["abort", "skip", "commits"])]
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

/// `--machine` data for `revert`, `--continue`, `--abort` and `--skip`.
#[derive(Debug, Clone, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[cfg_attr(test, ts(export, export_to = "../../../ui/src/api/generated/"))]
pub struct RevertReport {
    /// `revert`, `continue`, `abort` or `skip`.
    pub action: String,
    /// `reverted`, `not_committed`, `conflict` (the command then also exits 1),
    /// `aborted`, `skipped` or `completed`.
    pub status: String,
    /// New revert commits created by this run.
    pub commits: Vec<String>,
    /// `conflict`: paths left unresolved.
    pub conflicts: Vec<String>,
}

fn revert_report(action: &str, status: &str, commits: Vec<String>) -> RevertReport {
    RevertReport {
        action: action.into(),
        status: status.into(),
        commits,
        conflicts: Vec::new(),
    }
}

/// A revert stopped on conflicts; carries the paths for the machine report.
#[derive(Debug)]
struct ConflictStop(Vec<String>);

impl std::fmt::Display for ConflictStop {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Revert resulted in conflict - resolve and continue")
    }
}

impl std::error::Error for ConflictStop {}

#[derive(Debug)]
struct RevertState {
    commits: Vec<String>,
    current_index: usize,
    original_head: Oid,
    /// X3: false when `commits[current_index]` has not run yet (the rest was
    /// refused), so `--continue` has no conflict to commit first.
    pending: bool,
}

impl std::fmt::Display for RevertState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}\n{}{}\n{}",
            self.original_head.to_hex(),
            self.current_index,
            if self.pending { "" } else { " next" },
            self.commits.join("\n")
        )
    }
}

impl RevertState {
    fn from_string(s: &str) -> Result<Self> {
        let lines: Vec<&str> = s.lines().collect();
        if lines.len() < 3 {
            anyhow::bail!("Invalid revert state file");
        }
        let (index, pending) = match lines[1].strip_suffix(" next") {
            Some(i) => (i, false),
            None => (lines[1], true),
        };

        Ok(Self {
            original_head: Oid::from_hex(lines[0])?,
            current_index: index.parse()?,
            commits: lines[2..].iter().map(|s| s.to_string()).collect(),
            pending,
        })
    }

    async fn save(&self, state_file: &Path) -> Result<()> {
        fs::write(state_file, self.to_string()).await?;
        Ok(())
    }
}

impl RevertCmd {
    fn quiet(&self) -> bool {
        self.quiet || output::machine_mode()
    }

    pub async fn execute(&self) -> Result<()> {
        let repo_root = find_repo_root()?;
        let storage_path = repo_root.join(".mediagit");

        if self.continue_revert {
            return self.do_continue(&repo_root, &storage_path).await;
        }
        if self.abort {
            return self.do_abort(&repo_root, &storage_path).await;
        }
        if self.skip {
            return self.do_skip(&repo_root, &storage_path).await;
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
        let odb = Arc::new(ObjectDatabase::with_smart_compression(
            storage.clone(),
            10000,
        ));
        let refs = RefDatabase::new(&storage_path);

        let original_head = refs.resolve("HEAD").await?;

        // Resolve commits
        let mut resolved_commits = Vec::new();
        for commit_spec in &self.commits {
            let oid = mediagit_versioning::resolve_revision(commit_spec, &refs, &odb).await?;
            resolved_commits.push(oid);
        }

        // BUG-008: check HEAD, every listed commit and its parent before the
        // first revert is committed.
        let mut involved = vec![original_head];
        for oid in &resolved_commits {
            involved.push(*oid);
            let c = Commit::read(&odb, oid).await?;
            involved.extend(c.parents.first().copied());
        }
        crate::repo::ensure_no_unsafe_paths(&odb, &repo_root, &involved).await?;

        let commits: Vec<String> = resolved_commits.iter().map(|o| o.to_hex()).collect();
        self.run_reverts(
            &repo_root,
            &storage_path,
            &odb,
            &refs,
            &commits,
            0,
            original_head,
            Vec::new(),
            "revert",
        )
        .await
    }

    /// Revert `commits[start..]` in order. A conflict saves the state (this
    /// revert is the current one) and stops; any other refusal leaves no
    /// state. Used by the first run, `--continue` and `--skip` alike, so the
    /// rest of a sequence is always applied the same way.
    #[allow(clippy::too_many_arguments)]
    async fn run_reverts(
        &self,
        repo_root: &Path,
        storage_path: &Path,
        odb: &Arc<ObjectDatabase>,
        refs: &RefDatabase,
        commits: &[String],
        start: usize,
        original_head: Oid,
        mut new_commits: Vec<String>,
        action: &str,
    ) -> Result<()> {
        let state_file = storage_path.join(REVERT_STATE_FILE);
        // X1: a stop after the first revert cannot be continued or skipped
        // without mixing the reverts, so refuse before anything is written.
        if self.no_commit && commits.len().saturating_sub(start) > 1 {
            anyhow::bail!(
                "--no-commit takes one version at a time. Undo them one at a time, \
                 or leave out -n to save each one. Nothing was changed."
            );
        }
        // `--no-commit`: each revert goes on top of the previous result.
        let mut onto = None;
        for (i, spec) in commits.iter().enumerate().skip(start) {
            let commit_oid = Oid::from_hex(spec)?;
            let result = self
                .revert_single_commit(repo_root, storage_path, odb, refs, &commit_oid, onto)
                .await;

            match result {
                Ok(oid) if self.no_commit => onto = Some(oid),
                Ok(oid) => new_commits.push(oid.to_hex()),
                Err(e) => {
                    let stop = e.downcast_ref::<ConflictStop>();
                    // X3: a refusal inside `--continue` / `--skip` (the state
                    // exists) keeps this revert and the rest listed. A first
                    // run's refusal leaves no state.
                    if stop.is_none() && !state_file.exists() {
                        return Err(e);
                    }
                    RevertState {
                        commits: commits.to_vec(),
                        current_index: i,
                        original_head,
                        pending: stop.is_some(),
                    }
                    .save(&state_file)
                    .await?;
                    let Some(stop) = stop else {
                        return Err(e);
                    };

                    if output::machine_mode() {
                        output::emit_ok(&RevertReport {
                            conflicts: stop.0.clone(),
                            ..revert_report(action, "conflict", new_commits)
                        })?;
                        std::process::exit(1);
                    }

                    println!(
                        "{} Revert stopped due to conflicts.",
                        style("⚠").yellow().bold()
                    );
                    println!("  Resolve conflicts and run 'mediagit revert --continue'");
                    // Non-zero exit so CI can detect a conflict-stop instead of
                    // silently reporting success (AUD-1).
                    anyhow::bail!("Revert stopped due to conflicts");
                }
            }
        }

        // X3: the whole sequence ran; a `--continue` / `--skip` state goes now.
        if state_file.exists() {
            fs::remove_file(&state_file).await?;
        }

        if !self.quiet() && start < commits.len() {
            println!(
                "\n{} Successfully reverted {} commit(s)",
                style("✓").green().bold(),
                commits.len() - start
            );
        }

        if output::machine_mode() {
            let status = if self.no_commit {
                "not_committed"
            } else {
                "reverted"
            };
            return output::emit_ok(&revert_report(action, status, new_commits));
        }

        Ok(())
    }

    /// Revert one commit on top of `onto` (HEAD when `None`). Returns the new
    /// revert commit, or under `--no-commit` a temporary commit holding the
    /// staged result for the next revert to build on.
    async fn revert_single_commit(
        &self,
        repo_root: &Path,
        storage_path: &Path,
        odb: &Arc<ObjectDatabase>,
        refs: &RefDatabase,
        commit_oid: &Oid,
        onto: Option<Oid>,
    ) -> Result<Oid> {
        let commit = Commit::read(odb, commit_oid)
            .await
            .with_context(|| "Failed to read commit to revert")?;

        let parent_oid = commit
            .parents
            .first()
            .context("Cannot revert initial commit")?;

        let real_head = refs.resolve("HEAD").await?;
        let head_oid = onto.unwrap_or(real_head);
        let head_commit = Commit::read(odb, &head_oid).await?;

        // WT-3: revert had no pre-flight dirty check at all. The merge result
        // can only materialize paths from ours (all tracked) or from the
        // reverted commit's parent, so checking collisions against the parent
        // is exact.
        crate::repo::ensure_no_unsafe_paths(odb, repo_root, &[head_oid, *parent_oid]).await?;
        crate::worktree_guard::AtRisk::check(repo_root, odb, Some(&head_oid), Some(parent_oid))
            .await?
            .ensure_clean("revert")?;

        // WT-1: bound what the checkouts below may delete.
        let tracked = crate::worktree_guard::tracked_paths(repo_root, odb, Some(&head_oid)).await?;

        if !self.quiet() {
            output::progress(&format!(
                "Reverting {} \"{}\"",
                &commit_oid.to_hex()[..7],
                commit.summary()
            ));
        }

        // Use MergeEngine for explicit 3-way merge with correct trees.
        // For revert, the merge base must be the commit-to-revert's tree
        // (not the auto-computed LCA which would return "already up to date"):
        //   base   = tree of the commit being reverted
        //   ours   = tree of HEAD (current state)
        //   theirs = tree of the commit's parent (the state we want to reach)
        let merge_engine = MergeEngine::new(odb.clone());
        let parent_commit = Commit::read(odb, parent_oid).await?;

        let merge_result = merge_engine
            .merge_trees(
                &commit.tree,        // base: the commit being reverted
                &head_commit.tree,   // ours: current HEAD
                &parent_commit.tree, // theirs: parent of reverted commit
                MergeStrategy::Recursive,
            )
            .await?;

        let message = self.message.clone().unwrap_or_else(|| {
            format!(
                "Revert \"{}\"\n\nThis reverts commit {}.",
                commit.summary(),
                commit_oid.to_hex()
            )
        });

        if merge_result.has_conflicts() {
            // WT-5: materialise the conflict instead of staging HEAD unchanged.
            //
            // This used to write the *unmodified HEAD tree* into the index and
            // bail, with nothing written to the working tree. The user had no
            // marker to resolve, so running `--continue` immediately was the
            // natural next step — and `do_continue` then built a tree from that
            // index (HEAD's own tree) and committed it as "the revert". The
            // command reported success and reverted nothing.
            let ours_tree = Tree::read(odb, &head_commit.tree).await?;
            let mut index = Index::load(repo_root)?;

            mediagit_versioning::apply_merge_to_workdir(
                &merge_result,
                &ours_tree,
                odb,
                repo_root,
                &mut index,
                *parent_oid,
                real_head,
                &message,
            )
            .await
            .context("Failed to write revert conflict to the working directory")?;

            index.save(repo_root)?;

            if !self.quiet() {
                println!(
                    "{} Revert stopped: conflict in {} file(s):",
                    console::style("⚠").yellow().bold(),
                    merge_result.conflicts.len()
                );
                for conflict in &merge_result.conflicts {
                    println!("  {} {}", console::style("conflict:").red(), conflict.path);
                }
                println!(
                    "  Resolve the file(s), {}, then run {}",
                    console::style("mediagit add <file>").cyan(),
                    console::style("mediagit revert --continue").cyan()
                );
            }

            return Err(ConflictStop(
                merge_result
                    .conflicts
                    .iter()
                    .map(|c| c.path.clone())
                    .collect(),
            )
            .into());
        }

        let new_tree_oid = merge_result
            .tree_oid
            .context("Merge did not produce a tree")?;

        if !self.no_commit {
            // Reverting a change that is already undone would record an empty
            // commit and report success.
            if new_tree_oid == head_commit.tree {
                return Err(output::CodedError::new(
                    "empty_commit",
                    format!(
                        "nothing to commit - the change from {} is already reverted on this branch",
                        &commit_oid.to_hex()[..7]
                    ),
                )
                .into());
            }

            let revert_commit = Commit::with_parents(
                new_tree_oid,
                vec![head_oid],
                Signature::now("MediaGit".to_string(), "mediagit@local".to_string()),
                Signature::now("MediaGit".to_string(), "mediagit@local".to_string()),
                message,
            );

            let new_commit_oid = revert_commit.write(odb).await?;

            // Update refs
            let current_branch = self.get_current_branch(storage_path).await?;
            if let Some(ref branch) = current_branch {
                refs.update(&format!("refs/heads/{}", branch), new_commit_oid, true)
                    .await?;
            } else {
                refs.update("HEAD", new_commit_oid, true).await?;
            }

            // Reflog
            let reflog = Reflog::new(storage_path);
            let entry = ReflogEntry::now(
                head_oid,
                new_commit_oid,
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

            // Update working tree to match the reverted state.
            // Without this, the working directory stays out-of-sync with HEAD.
            let checkout_mgr = CheckoutManager::new(odb, repo_root).with_tracked_paths(tracked);
            checkout_mgr
                .checkout_commit(&new_commit_oid)
                .await
                .context("Failed to update working directory after revert")?;

            if !self.quiet() {
                println!(
                    "{} Created revert commit: {}",
                    style("✓").green(),
                    &new_commit_oid.to_hex()[..7]
                );
            }
            Ok(new_commit_oid)
        } else {
            // No commit mode - stage the result as changes on top of HEAD
            // (deletions included) and update the working tree.
            let merged_tree = Tree::read(odb, &new_tree_oid).await?;
            let real_head_tree =
                Tree::read(odb, &Commit::read(odb, &real_head).await?.tree).await?;
            crate::worktree_guard::restage_no_commit(repo_root, &real_head_tree, &merged_tree)?;

            // Update working tree to match the reverted tree.
            // Create a temporary commit so CheckoutManager can traverse the tree.
            let temp_commit = Commit::with_parents(
                new_tree_oid,
                vec![head_oid],
                Signature::now("MediaGit".to_string(), "mediagit@local".to_string()),
                Signature::now("MediaGit".to_string(), "mediagit@local".to_string()),
                "revert (no-commit)".to_string(),
            );
            let temp_oid = temp_commit.write(odb).await?;
            let checkout_mgr = CheckoutManager::new(odb, repo_root).with_tracked_paths(tracked);
            checkout_mgr
                .checkout_commit(&temp_oid)
                .await
                .context("Failed to update working directory after revert")?;

            if !self.quiet() {
                println!(
                    "{} Reverted {} (not committed)",
                    style("✓").green(),
                    &commit_oid.to_hex()[..7]
                );
            }
            Ok(temp_oid)
        }
    }

    async fn do_continue(&self, repo_root: &Path, storage_path: &Path) -> Result<()> {
        let state_file = storage_path.join(REVERT_STATE_FILE);
        if !state_file.exists() {
            anyhow::bail!("No revert in progress");
        }

        // nosemgrep: rust.actix.path-traversal.tainted-path.tainted-path -- repo-internal constant path
        let state_content = fs::read_to_string(&state_file).await?;
        let state = RevertState::from_string(&state_content)?;

        // WT-5/WT-9: refuse while any path is still unacknowledged.
        //
        // Asks the index rather than scanning file contents for `<<<<<<<`.
        // Binary files never receive markers — the resolver checks out one
        // side provisionally instead — so a content scan reported "resolved"
        // for precisely the media files this system versions, and let
        // `--continue` commit a side the user never reviewed.
        //
        // The state file is deliberately NOT removed until after this check:
        // bailing must leave the revert resumable.
        let unresolved = Index::load(repo_root)?.unresolved_paths();
        if !unresolved.is_empty() {
            anyhow::bail!(
                "Cannot continue: {} path(s) still unresolved:\n  {}\n\
                 Review each, then `mediagit add <path>` to accept it \
                 (binary files included — staging is the acknowledgement), \
                 or `mediagit revert --abort`.",
                unresolved.len(),
                unresolved
                    .iter()
                    .map(|p| p.display().to_string())
                    .collect::<Vec<_>>()
                    .join("\n  ")
            );
        }

        let storage = create_storage_backend(repo_root).await?;
        let odb = Arc::new(ObjectDatabase::with_smart_compression(
            storage.clone(),
            10000,
        ));
        let refs = RefDatabase::new(storage_path);

        // Validate first: every id of the sequence must parse.
        for spec in &state.commits {
            Oid::from_hex(spec)?;
        }

        // X3: with no revert pending (the rest was refused), nothing is
        // committed here; the rest starts at `current_index`.
        let mut created = Vec::new();
        let mut next = state.current_index;
        if state.pending {
            let new_commit_oid = self
                .commit_resolution(repo_root, storage_path, &odb, &refs, &state)
                .await?;
            created.push(new_commit_oid.to_hex());
            next += 1;
        }

        // Then the rest of the sequence, listed until it ran: a conflict
        // there saves a new state, a refusal keeps this one.
        RevertState {
            commits: state.commits.clone(),
            current_index: next,
            original_head: state.original_head,
            pending: false,
        }
        .save(&state_file)
        .await?;
        self.run_reverts(
            repo_root,
            storage_path,
            &odb,
            &refs,
            &state.commits,
            next,
            state.original_head,
            created,
            "continue",
        )
        .await
    }

    /// Commit the resolved revert `state.commits[state.current_index]` on
    /// HEAD, then clear its staged entries and the merge files.
    async fn commit_resolution(
        &self,
        repo_root: &Path,
        storage_path: &Path,
        odb: &Arc<ObjectDatabase>,
        refs: &RefDatabase,
        state: &RevertState,
    ) -> Result<Oid> {
        let revert_oid = Oid::from_hex(&state.commits[state.current_index])?;
        let revert_commit = Commit::read(odb, &revert_oid).await?;

        // HEAD's tree with the resolution staged on top, never the index alone.
        let head_oid = refs.resolve("HEAD").await?;
        let mut index = Index::load(repo_root)?;
        let head_tree = Commit::read(odb, &head_oid).await?.tree;
        let tree_oid = mediagit_versioning::tree_with_index(odb, &head_tree, &index).await?;

        // DL-8: a resolution equal to HEAD changes nothing; no empty commit.
        if tree_oid == head_tree {
            return Err(output::CodedError::new(
                "empty_commit",
                format!(
                    "nothing to commit - the resolution leaves the branch as it is (reverting {})",
                    &revert_oid.to_hex()[..7]
                ),
            )
            .into());
        }

        let message = self.message.clone().unwrap_or_else(|| {
            format!(
                "Revert \"{}\"\n\nThis reverts commit {}.",
                revert_commit.summary(),
                revert_oid.to_hex()
            )
        });

        let new_commit = Commit::with_parents(
            tree_oid,
            vec![head_oid],
            Signature::now("MediaGit".to_string(), "mediagit@local".to_string()),
            Signature::now("MediaGit".to_string(), "mediagit@local".to_string()),
            message,
        );

        let new_commit_oid = new_commit.write(odb).await?;

        let current_branch = self.get_current_branch(storage_path).await?;
        if let Some(ref branch) = current_branch {
            refs.update(&format!("refs/heads/{}", branch), new_commit_oid, true)
                .await?;
        } else {
            refs.update("HEAD", new_commit_oid, true).await?;
        }

        // Write, then clear: a failure above leaves the revert continuable.
        // The resolution is committed, so its staged entries go too.
        mediagit_versioning::clear_merge_state(storage_path)?;
        index.clear();
        index.save(repo_root)?;

        if !self.quiet() {
            println!(
                "{} Revert continued: {}",
                style("✓").green().bold(),
                &new_commit_oid.to_hex()[..7]
            );
        }
        Ok(new_commit_oid)
    }

    async fn do_abort(&self, repo_root: &Path, storage_path: &Path) -> Result<()> {
        let state_file = storage_path.join(REVERT_STATE_FILE);
        if !state_file.exists() {
            anyhow::bail!("No revert in progress");
        }

        // nosemgrep: rust.actix.path-traversal.tainted-path.tainted-path -- repo-internal constant path
        let state_content = fs::read_to_string(&state_file).await?;
        let state = RevertState::from_string(&state_content)?;

        let refs = RefDatabase::new(storage_path);
        let storage = create_storage_backend(repo_root).await?;
        let odb = Arc::new(ObjectDatabase::with_smart_compression(storage, 10000));
        // Y1: with no conflict stopped (the rest was refused) an edit on disk
        // is the user's. Refuse before any write rather than wipe it.
        if !state.pending {
            let head = refs.resolve("HEAD").await?;
            crate::worktree_guard::AtRisk::check(
                repo_root,
                &odb,
                Some(&head),
                Some(&state.original_head),
            )
            .await?
            .ensure_clean("revert --abort")?;
        }
        let current_branch = self.get_current_branch(storage_path).await?;
        if let Some(ref branch) = current_branch {
            refs.update(&format!("refs/heads/{}", branch), state.original_head, true)
                .await?;
        } else {
            refs.update("HEAD", state.original_head, true).await?;
        }

        // Restore the working tree to the pre-revert commit and clear the
        // index. An empty index means "clean" (a normal commit clears it
        // too); do_revert_single_commit's save_tree_to_index left the index
        // fully populated on conflict, so without this `status` would show
        // everything staged after an abort.
        // WT-1: an abort must not take untracked files with it.
        let tracked = crate::worktree_guard::tracked_paths_with_staged(
            repo_root,
            &odb,
            Some(&state.original_head),
        )
        .await?;
        let checkout_mgr = CheckoutManager::new(&odb, repo_root).with_tracked_paths(tracked);
        checkout_mgr
            .checkout_commit(&state.original_head)
            .await
            .context("Failed to restore working directory on revert abort")?;

        let mut index = Index::load(repo_root)?;
        index.clear();
        index.save(repo_root)?;

        fs::remove_file(&state_file).await?;
        mediagit_versioning::clear_merge_state(storage_path)?;

        if output::machine_mode() {
            return output::emit_ok(&revert_report("abort", "aborted", Vec::new()));
        }
        println!(
            "{} Revert aborted, HEAD reset to {}",
            style("✓").green().bold(),
            &state.original_head.to_hex()[..7]
        );

        Ok(())
    }

    async fn do_skip(&self, repo_root: &Path, storage_path: &Path) -> Result<()> {
        let state_file = storage_path.join(REVERT_STATE_FILE);
        if !state_file.exists() {
            anyhow::bail!("No revert in progress");
        }

        // nosemgrep: rust.actix.path-traversal.tainted-path.tainted-path -- repo-internal constant path
        let state_content = fs::read_to_string(&state_file).await?;
        let state = RevertState::from_string(&state_content)?;
        let next = state.current_index + 1;

        // Validate first: every id of the sequence must parse.
        for spec in &state.commits {
            Oid::from_hex(spec)?;
        }

        // The skipped revert's conflict is dropped: the paths it touched go
        // back to HEAD, the index and merge files are cleared. HEAD is read
        // first; the state file is only touched once that worked.
        let refs = RefDatabase::new(storage_path);
        let head_oid = refs.resolve("HEAD").await?;
        let storage = create_storage_backend(repo_root).await?;
        let odb = Arc::new(ObjectDatabase::with_smart_compression(storage, 10000));
        // X3: with no revert pending (the rest was refused), the skipped one
        // never ran, so there is nothing to put back.
        if state.pending {
            crate::worktree_guard::drop_skipped_step(repo_root, &odb, &head_oid)
                .await
                .context("Failed to restore working directory on revert skip")?;
        }

        // B1: the skip ends this stop. The rest of the sequence runs now (a
        // conflict there saves a new state); a `--continue` is never left
        // pointing at a revert that was never applied. X3: the rest stays
        // listed until it ran, so a refusal in it can be continued.
        if next < state.commits.len() {
            RevertState {
                commits: state.commits.clone(),
                current_index: next,
                original_head: state.original_head,
                pending: false,
            }
            .save(&state_file)
            .await?;
        } else {
            fs::remove_file(&state_file).await?;
        }

        if next >= state.commits.len() {
            if output::machine_mode() {
                return output::emit_ok(&revert_report("skip", "completed", Vec::new()));
            }
            println!(
                "{} Revert completed (skipped last)",
                style("✓").green().bold()
            );
            return Ok(());
        }

        self.run_reverts(
            repo_root,
            storage_path,
            &odb,
            &refs,
            &state.commits,
            next,
            state.original_head,
            Vec::new(),
            "skip",
        )
        .await
    }

    async fn get_current_branch(&self, storage_path: &Path) -> Result<Option<String>> {
        let head_path = storage_path.join("HEAD");
        if !head_path.exists() {
            return Ok(None);
        }

        // nosemgrep: rust.actix.path-traversal.tainted-path.tainted-path -- repo-internal constant path
        let content = fs::read_to_string(&head_path).await?;
        let content = content.trim();

        if let Some(target) = content.strip_prefix("ref: ")
            && let Some(branch) = target.strip_prefix("refs/heads/")
        {
            return Ok(Some(branch.to_string()));
        }

        Ok(None)
    }
}
