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
use crate::progress::{OperationStats, ProgressTracker};
use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use mediagit_versioning::{Oid, Ref, RefDatabase, Reflog, ReflogEntry};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::Instant;

/// Manage branches
///
/// Create, list, rename, and delete branches. Branches are lightweight references
/// to commits that allow parallel development workflows.
#[derive(Parser, Debug)]
// BUG-006: bare `branch` and `branch <name>` are translated in main.rs
// preprocess_args — list default with no positional, `create` default when
// a name is supplied (git-style sugar).
#[command(after_help = "USAGE NOTE:
    `branch` requires a subcommand. `mediagit branch <name>` is not valid —
    use `mediagit branch create <name>` to make a branch.

EXAMPLES:
    # List all local branches
    mediagit branch list

    # List all branches with verbose output
    mediagit branch list -v

    # Create a new branch
    mediagit branch create feature-branch

    # Create branch from specific commit
    mediagit branch create hotfix abc123

    # Switch to a branch
    mediagit branch switch main

    # Create and switch to new branch
    mediagit branch switch -c feature-branch

    # Rename current branch
    mediagit branch rename new-name

    # Rename specific branch
    mediagit branch rename old-name new-name

    # Delete a branch
    mediagit branch delete feature-branch

    # Show branch information
    mediagit branch show

SEE ALSO:
    mediagit-checkout(1), mediagit-merge(1), mediagit-tag(1)")]
pub struct BranchCmd {
    #[command(subcommand)]
    pub subcommand: BranchSubcommand,
}

#[derive(Subcommand, Debug)]
pub enum BranchSubcommand {
    /// List branches
    #[command(alias = "ls")]
    List(ListOpts),

    /// Create a new branch
    Create(CreateOpts),

    /// Switch to a branch
    #[command(alias = "checkout", alias = "co")]
    Switch(SwitchOpts),

    /// Delete a branch
    #[command(alias = "rm")]
    Delete(DeleteOpts),

    /// Protect a branch
    Protect(ProtectOpts),

    /// Rename a branch
    #[command(alias = "move", alias = "mv")]
    Rename(RenameOpts),

    /// Show branch information
    Show(ShowOpts),

    /// Merge a branch
    Merge(MergeOpts),
}

/// List branches
#[derive(Parser, Debug)]
pub struct ListOpts {
    /// List remote branches
    #[arg(short, long)]
    pub remote: bool,

    /// List all branches (local and remote)
    #[arg(short = 'a', long)]
    pub all: bool,

    /// Show verbose output
    #[arg(short, long)]
    pub verbose: bool,

    /// Quiet mode
    #[arg(short, long)]
    pub quiet: bool,

    /// Sort branches
    #[arg(long, value_name = "KEY")]
    pub sort: Option<String>,
}

/// Create a new branch
#[derive(Parser, Debug)]
pub struct CreateOpts {
    /// Branch name
    #[arg(value_name = "NAME")]
    pub name: String,

    /// Start point (defaults to HEAD)
    #[arg(value_name = "START_POINT")]
    pub start_point: Option<String>,

    /// Set upstream branch
    #[arg(short = 'u', long, value_name = "UPSTREAM")]
    pub set_upstream: Option<String>,

    /// Track a remote branch
    #[arg(long)]
    pub track: bool,

    /// Don't track
    #[arg(long)]
    pub no_track: bool,

    /// Quiet mode
    #[arg(short, long)]
    pub quiet: bool,
}

/// Switch to a branch
#[derive(Parser, Debug)]
pub struct SwitchOpts {
    /// Branch name
    #[arg(value_name = "BRANCH")]
    pub branch: String,

    /// Create and switch to new branch
    #[arg(short, long)]
    pub create: bool,

    /// When used with --create, set up upstream tracking. If `branch` looks
    /// like `<remote>/<name>` (e.g. `origin/feat-a`), the new local branch
    /// is named `<name>`, started from the `<remote>/<name>` tracking ref
    /// instead of HEAD, and set to track it.
    #[arg(long)]
    pub track: bool,

    /// Force switch even if local changes
    #[arg(short = 'f', long)]
    pub force: bool,

    /// Don't check out files
    #[arg(long)]
    pub no_guess: bool,

    /// Quiet mode
    #[arg(short, long)]
    pub quiet: bool,
}

/// Delete a branch
#[derive(Parser, Debug)]
pub struct DeleteOpts {
    /// Branch names to delete
    #[arg(value_name = "BRANCHES", required = true)]
    pub branches: Vec<String>,

    /// Force delete (ignore merge status)
    #[arg(short = 'D', long)]
    pub force: bool,

    /// Delete only if merged
    #[arg(short = 'd', long)]
    pub delete_merged: bool,

    /// Delete remote tracking branches (e.g. origin/feature)
    #[arg(short = 'r', long)]
    pub remote: bool,

    /// Quiet mode
    #[arg(short, long)]
    pub quiet: bool,
}

/// Protect a branch
#[derive(Parser, Debug)]
pub struct ProtectOpts {
    /// Branch name
    #[arg(value_name = "BRANCH")]
    pub branch: String,

    /// Require pull request reviews before merge
    #[arg(long)]
    pub require_reviews: bool,

    /// Unprotect the branch
    #[arg(long)]
    pub unprotect: bool,

    /// Quiet mode
    #[arg(short, long)]
    pub quiet: bool,
}

/// Rename a branch
///
/// With one argument, renames the current branch: `branch rename <new-name>`
/// With two arguments, renames a specific branch:  `branch rename <old-name> <new-name>`
#[derive(Parser, Debug)]
pub struct RenameOpts {
    /// New name (1 arg) or old branch name (2 args)
    #[arg(value_name = "OLD_OR_NEW")]
    pub first_arg: String,

    /// New branch name when renaming a specific branch
    #[arg(value_name = "NEW_NAME")]
    pub second_arg: Option<String>,

    /// Force rename
    #[arg(short = 'f', long)]
    pub force: bool,

    /// Quiet mode
    #[arg(short, long)]
    pub quiet: bool,
}

/// Show branch information
#[derive(Parser, Debug)]
pub struct ShowOpts {
    /// Branch name (current branch if not specified)
    #[arg(value_name = "BRANCH")]
    pub branch: Option<String>,

    /// Show verbose information
    #[arg(short, long)]
    pub verbose: bool,
}

/// Merge a branch
#[derive(Parser, Debug)]
pub struct MergeOpts {
    /// Branch to merge
    #[arg(value_name = "BRANCH", required = true)]
    pub branch: String,

    /// Create a merge commit
    #[arg(long)]
    pub no_ff: bool,

    /// Perform a fast-forward only merge
    #[arg(long)]
    pub ff_only: bool,

    /// Merge message
    #[arg(short, long, value_name = "MESSAGE")]
    pub message: Option<String>,

    /// Quit if merge conflicts occur
    #[arg(long)]
    pub abort: bool,

    /// Continue after resolving conflicts
    #[arg(long)]
    pub continue_merge: bool,

    /// Quiet mode
    #[arg(short, long)]
    pub quiet: bool,
}

impl BranchCmd {
    pub async fn execute(&self) -> Result<()> {
        match &self.subcommand {
            BranchSubcommand::List(opts) => self.list(opts).await,
            BranchSubcommand::Create(opts) => self.create(opts).await,
            BranchSubcommand::Switch(opts) => self.switch(opts).await,
            BranchSubcommand::Delete(opts) => self.delete(opts).await,
            BranchSubcommand::Protect(opts) => self.protect(opts).await,
            BranchSubcommand::Rename(opts) => self.rename(opts).await,
            BranchSubcommand::Show(opts) => self.show(opts).await,
            BranchSubcommand::Merge(opts) => self.merge(opts).await,
        }
    }

    async fn list(&self, opts: &ListOpts) -> Result<()> {
        use crate::output;
        use console::style;

        let repo_root = find_repo_root()?;
        let storage_path = repo_root.join(".mediagit");
        let _storage = create_storage_backend(&repo_root).await?;
        let refdb = RefDatabase::new(&storage_path);

        // Get current branch for highlighting
        let head = refdb.read("HEAD").await.ok();
        let current_branch = head.and_then(|h| h.target);

        let mut any_branches_found = false;

        // List local branches (unless --remote only)
        if !opts.remote {
            let local_branches = refdb.list("heads").await?;

            if !local_branches.is_empty() {
                any_branches_found = true;

                for branch_name in local_branches {
                    // Normalize path separators for cross-platform compatibility (Windows uses \)
                    let normalized_branch = branch_name.replace('\\', "/");
                    let normalized_current =
                        current_branch.as_ref().map(|cb| cb.replace('\\', "/"));

                    let is_current = normalized_current
                        .as_ref()
                        .map(|cb| cb == &normalized_branch)
                        .unwrap_or(false);

                    let prefix = if is_current { "* " } else { "  " };
                    let display_name = normalized_branch
                        .strip_prefix("refs/heads/")
                        .unwrap_or(&normalized_branch);

                    if opts.verbose {
                        let branch_ref = refdb.read(&branch_name).await.ok();
                        let oid_display = branch_ref
                            .and_then(|r| r.oid)
                            .map(|o| o.to_string()[..8].to_string())
                            .unwrap_or_else(|| "unknown".to_string());
                        if is_current {
                            println!(
                                "{}{} -> {}",
                                style(prefix).green(),
                                style(display_name).green().bold(),
                                oid_display
                            );
                        } else {
                            println!("{}{} -> {}", prefix, display_name, oid_display);
                        }
                    } else if is_current {
                        println!(
                            "{}{}",
                            style(prefix).green(),
                            style(display_name).green().bold()
                        );
                    } else {
                        println!("{}{}", prefix, display_name);
                    }
                }
            }
        }

        // List remote branches (if --remote or --all)
        if opts.remote || opts.all {
            let remote_branches = refdb.list("remotes").await?;

            if !remote_branches.is_empty() {
                any_branches_found = true;

                // Add separator if we printed local branches
                if opts.all && !opts.remote {
                    println!(); // Empty line between local and remote
                }

                for branch_name in remote_branches {
                    // Normalize path separators for cross-platform compatibility
                    let normalized_branch = branch_name.replace('\\', "/");
                    let display_name = normalized_branch
                        .strip_prefix("refs/remotes/")
                        .unwrap_or(&normalized_branch);

                    if opts.verbose {
                        let branch_ref = refdb.read(&branch_name).await.ok();
                        let oid_display = branch_ref
                            .and_then(|r| r.oid)
                            .map(|o| o.to_string()[..8].to_string())
                            .unwrap_or_else(|| "unknown".to_string());
                        println!("  {} -> {}", style(display_name).red(), oid_display);
                    } else {
                        println!("  {}", style(display_name).red());
                    }
                }
            }
        }

        if !any_branches_found && !opts.quiet {
            if opts.remote {
                output::info("No remote branches found");
            } else {
                output::info("No branches found");
            }
        }

        Ok(())
    }

    async fn create(&self, opts: &CreateOpts) -> Result<()> {
        use crate::output;

        let repo_root = find_repo_root()?;
        let storage_path = repo_root.join(".mediagit");
        let storage = create_storage_backend(&repo_root).await?;
        let refdb = RefDatabase::new(&storage_path);
        let odb = mediagit_versioning::ObjectDatabase::with_smart_compression(storage, 1000);

        // Validate branch name
        if opts.name.contains("..") || opts.name.starts_with('/') || opts.name.ends_with('/') {
            anyhow::bail!("Invalid branch name: {}", opts.name);
        }

        let branch_ref_name = format!("refs/heads/{}", opts.name);

        // Check if branch already exists
        if refdb.read(&branch_ref_name).await.is_ok() {
            anyhow::bail!("Branch '{}' already exists", opts.name);
        }

        // Get start point (defaults to HEAD) - resolve symbolic refs.
        //
        // BUG-010: accept Git-compatible shorthand for remote-tracking refs.
        // Try the input verbatim first (so explicit `refs/...` / `HEAD` / OIDs
        // keep working), then fall back to `refs/remotes/<input>` when the
        // user wrote something like `origin/feat-a`.
        let start_oid = if let Some(start_point) = &opts.start_point {
            // Route through the shared resolver so OIDs (full/abbrev), tags, and
            // HEAD~N all work as start points (BUG-VFX-2), then keep the
            // remote-shorthand fallback for `origin/feat` style inputs.
            match mediagit_versioning::resolve_revision(start_point, &refdb, &odb).await {
                Ok(oid) => oid,
                Err(primary_err) => {
                    let looks_like_remote_shorthand = start_point.contains('/')
                        && !start_point.starts_with("refs/")
                        && !start_point.starts_with("HEAD")
                        && start_point.len() != 64;
                    if looks_like_remote_shorthand {
                        let fallback = format!("refs/remotes/{}", start_point);
                        refdb
                            .resolve(&fallback)
                            .await
                            .with_context(|| format!("Invalid start point: {}", start_point))?
                    } else {
                        return Err(primary_err)
                            .context(format!("Invalid start point: {}", start_point));
                    }
                }
            }
        } else {
            // Use HEAD as start point (resolve symbolic ref)
            refdb
                .resolve("HEAD")
                .await
                .context("HEAD has no commit yet")?
        };

        // Create the branch reference
        let branch_ref = Ref::new_direct(branch_ref_name.clone(), start_oid);
        refdb.write(&branch_ref).await?;

        if !opts.quiet {
            output::success(&format!("Created branch '{}' at {}", opts.name, start_oid));
        }

        // Upstream tracking (M2 plumbing, consumed by `status` in M4).
        // Source is `--set-upstream <remote>/<branch>` if given, else the
        // start point when `--track` was requested (matching git's
        // "track what you branched from" convention).
        if !opts.no_track {
            let track_source = opts
                .set_upstream
                .as_deref()
                .or_else(|| opts.track.then_some(opts.start_point.as_deref()).flatten());
            if let Some(source) = track_source {
                match source.split_once('/') {
                    Some((remote, remote_branch)) => {
                        let mut config = mediagit_config::Config::load(&repo_root).await?;
                        config.set_branch_upstream(
                            &opts.name,
                            remote,
                            format!("refs/heads/{}", remote_branch),
                        );
                        config.save(&repo_root)?;
                        if !opts.quiet {
                            output::info(&format!(
                                "Branch '{}' set up to track '{}/{}'",
                                opts.name, remote, remote_branch
                            ));
                        }
                    }
                    None if opts.set_upstream.is_some() => {
                        anyhow::bail!(
                            "--set-upstream expects <remote>/<branch> (got '{}')",
                            source
                        );
                    }
                    None => {
                        // --track given but the start point doesn't look like
                        // <remote>/<branch> (e.g. a bare OID or local ref) —
                        // nothing to track against; not an error.
                        if !opts.quiet {
                            output::warning(&format!(
                                "--track: start point '{}' doesn't look like <remote>/<branch>; no upstream recorded",
                                source
                            ));
                        }
                    }
                }
            }
        }

        Ok(())
    }

    async fn switch(&self, opts: &SwitchOpts) -> Result<()> {
        use crate::output;
        use mediagit_versioning::{CheckoutManager, Index, ObjectDatabase};

        let start_time = Instant::now();
        let mut stats = OperationStats::for_operation("switch");
        let progress = ProgressTracker::new(opts.quiet);

        let repo_root = find_repo_root()?;
        let storage_path = repo_root.join(".mediagit");
        let storage = create_storage_backend(&repo_root).await?;
        let refdb = RefDatabase::new(&storage_path);

        // Strip refs/heads/ prefix if already present
        let stripped = opts
            .branch
            .strip_prefix("refs/heads/")
            .unwrap_or(&opts.branch);

        // `--track` shorthand: `branch switch -c --track origin/feat-a`
        // creates a local branch named "feat-a" (not "origin/feat-a")
        // tracking origin/feat-a, started from that remote-tracking ref
        // instead of HEAD — mirrors `branch create`'s BUG-010 shorthand.
        let track_shorthand = if opts.create && opts.track {
            stripped.split_once('/')
        } else {
            None
        };
        let branch_name = track_shorthand.map(|(_, name)| name).unwrap_or(stripped);
        let branch_ref_name = format!("refs/heads/{}", branch_name);

        // OPTIMIZATION: Get current commit BEFORE updating HEAD
        // This enables differential checkout (only update changed files)
        let current_commit_oid = refdb.resolve("HEAD").await.ok();

        // If create flag is set, create the branch first
        if opts.create {
            // Check if branch already exists
            if refdb.read(&branch_ref_name).await.is_ok() {
                anyhow::bail!("Branch '{}' already exists", branch_name);
            }

            let start_oid = if let Some((remote, remote_branch)) = track_shorthand {
                let remote_tracking_ref = format!("refs/remotes/{}/{}", remote, remote_branch);
                refdb.resolve(&remote_tracking_ref).await.with_context(|| {
                    format!(
                        "--track: remote-tracking ref '{}' not found",
                        remote_tracking_ref
                    )
                })?
            } else {
                // Get current HEAD for start point (resolve symbolic ref)
                refdb
                    .resolve("HEAD")
                    .await
                    .context("HEAD has no commit yet")?
            };

            // Create the branch reference
            let branch_ref = Ref::new_direct(branch_ref_name.clone(), start_oid);
            refdb.write(&branch_ref).await?;

            if !opts.quiet {
                output::success(&format!("Created branch '{}'", branch_name));
            }

            // Upstream tracking (M2 plumbing, consumed by `status` in M4).
            if let Some((remote, remote_branch)) = track_shorthand {
                let mut config = mediagit_config::Config::load(&repo_root).await?;
                config.set_branch_upstream(
                    branch_name,
                    remote,
                    format!("refs/heads/{}", remote_branch),
                );
                config.save(&repo_root)?;
                if !opts.quiet {
                    output::info(&format!(
                        "Branch '{}' set up to track '{}/{}'",
                        branch_name, remote, remote_branch
                    ));
                }
            } else if opts.track && !opts.quiet {
                output::warning(&format!(
                    "--track: '{}' doesn't look like <remote>/<branch>; no upstream recorded",
                    opts.branch
                ));
            }
        } else {
            // Verify branch exists
            refdb
                .read(&branch_ref_name)
                .await
                .context(format!("Branch '{}' not found", opts.branch))?;
        }

        // Get the commit that the target branch points to
        let target_commit_oid = refdb.resolve(&branch_ref_name).await.context(format!(
            "Failed to resolve branch '{}' to a commit",
            opts.branch
        ))?;

        let odb = ObjectDatabase::with_smart_compression(storage.clone(), 1000);

        // BUG-CLI-B1: refuse to clobber uncommitted changes to tracked files.
        // Checked before HEAD is updated / the working tree is touched.
        if !opts.force {
            if let Some(current_oid) = current_commit_oid {
                if Self::has_uncommitted_changes(&repo_root, &odb, &current_oid).await? {
                    anyhow::bail!(
                        "working tree has uncommitted changes; commit/stash or use --force"
                    );
                }
            }

            // QA-001: refuse to clobber untracked files that collide with a
            // path tracked by the target branch. The dirty-check above only
            // covers files tracked by the *current* HEAD; an untracked file
            // is invisible to it and would otherwise be silently overwritten.
            let collisions = Self::untracked_collision_paths(
                &repo_root,
                &odb,
                current_commit_oid.as_ref(),
                &target_commit_oid,
            )
            .await?;
            if !collisions.is_empty() {
                let list = collisions
                    .iter()
                    .map(|p| p.display().to_string())
                    .collect::<Vec<_>>()
                    .join(", ");
                anyhow::bail!(
                    "switch would overwrite untracked file(s): {}; commit, stash, or use -f",
                    list
                );
            }
        }

        // Update HEAD to point to the branch
        let head = Ref::new_symbolic("HEAD".to_string(), branch_ref_name.clone());
        refdb.write(&head).await?;

        // Update working directory to match the target branch's commit
        let checkout_mgr = CheckoutManager::new(&odb, &repo_root);

        let checkout_pb = progress.spinner("Updating working directory");

        // OPTIMIZATION: Use differential checkout when we have a current commit
        // This only updates files that actually changed between commits
        let files_updated = if let Some(ref current_oid) = current_commit_oid {
            // Differential checkout: only update changed files
            let checkout_stats = checkout_mgr
                .checkout_diff(current_oid, &target_commit_oid)
                .await
                .context("Failed to update working directory")?;
            checkout_stats.files_changed()
        } else {
            // No current commit (initial checkout) - do full checkout
            checkout_mgr
                .checkout_commit(&target_commit_oid)
                .await
                .context("Failed to update working directory")?
        };

        checkout_pb.finish_with_message("Working directory updated");

        stats.files_updated = files_updated as u64;

        // Record reflog entry for branch switch
        let reflog = Reflog::new(&storage_path);
        let old_oid = current_commit_oid.unwrap_or_else(|| Oid::from_bytes([0u8; 32]));
        let reflog_msg = format!(
            "checkout: moving from {} to {}",
            old_oid.to_hex().get(..8).unwrap_or("00000000"),
            branch_name
        );
        let entry = ReflogEntry::now(
            old_oid,
            target_commit_oid,
            "user",
            "user@mediagit",
            &reflog_msg,
        );
        let _ = reflog.append("HEAD", &entry).await;

        // Clear the index (staging area) when switching branches
        // This ensures a clean state on the new branch
        let mut index = Index::load(&repo_root)?;
        index.clear();
        index.save(&repo_root)?;

        if !opts.quiet {
            output::success(&format!("Switched to branch '{}'", branch_name));
            if files_updated > 0 {
                output::info(&format!(
                    "Updated {} file(s) in working directory",
                    files_updated
                ));
            }
        }

        // Print operation summary
        stats.duration_ms = start_time.elapsed().as_millis() as u64;
        if !opts.quiet && stats.files_updated > 0 {
            println!("\n📊 {}", stats.summary());
        }

        // Save stats for later retrieval by stats command
        if let Err(e) = stats.save(&storage_path) {
            tracing::warn!("Failed to save operation stats: {}", e);
        }

        Ok(())
    }

    /// BUG-CLI-B1: detect uncommitted changes to tracked files before a
    /// branch switch would silently overwrite them. Mirrors `status`'s
    /// modified-file detection (HEAD tree vs working-directory hash), but
    /// treats any tracked file whose working content differs from HEAD as
    /// uncommitted — staged or not, a switch would blow it away either way.
    async fn has_uncommitted_changes(
        repo_root: &std::path::Path,
        odb: &mediagit_versioning::ObjectDatabase,
        head_oid: &Oid,
    ) -> Result<bool> {
        let commit_data = odb.read(head_oid).await?;
        let commit =
            mediagit_versioning::format::deserialize::<mediagit_versioning::Commit>(&commit_data)?;
        let tree_data = odb.read(&commit.tree).await?;
        let tree =
            mediagit_versioning::format::deserialize::<mediagit_versioning::Tree>(&tree_data)?;

        for entry in tree.iter() {
            let full_path = repo_root.join(&entry.name);
            let working_oid = match std::fs::metadata(&full_path) {
                Ok(metadata) if metadata.len() >= super::utils::STREAMING_THRESHOLD => {
                    match Oid::from_file(&full_path) {
                        Ok(oid) => oid,
                        Err(_) => continue,
                    }
                }
                Ok(_) => match std::fs::read(&full_path) {
                    Ok(content) => Oid::hash(&content),
                    Err(_) => continue,
                },
                // File missing from the working tree — not this guard's concern.
                Err(_) => continue,
            };
            if working_oid != entry.oid {
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// QA-001: paths that are untracked in the working directory but would
    /// be materialized by checking out `target_commit_oid` — i.e. a switch
    /// would silently overwrite them. Mirrors `status`'s untracked-file
    /// definition (working dir scan minus current-HEAD tree minus index
    /// minus ignored), intersected with the target tree's paths.
    async fn untracked_collision_paths(
        repo_root: &std::path::Path,
        odb: &mediagit_versioning::ObjectDatabase,
        current_commit_oid: Option<&Oid>,
        target_commit_oid: &Oid,
    ) -> Result<Vec<PathBuf>> {
        use crate::ignore_rules::IgnoreMatcher;
        use mediagit_versioning::Index;

        // Paths tracked by the branch we're switching away from — never
        // "untracked", even though the index is cleared after every switch.
        let mut head_files: HashSet<PathBuf> = HashSet::new();
        if let Some(oid) = current_commit_oid {
            let commit_data = odb.read(oid).await?;
            let commit = mediagit_versioning::format::deserialize::<mediagit_versioning::Commit>(
                &commit_data,
            )?;
            Self::collect_tree_paths(odb, &commit.tree, Path::new(""), &mut head_files).await?;
        }

        let index = Index::load(repo_root)?;
        let index_files: HashSet<PathBuf> =
            index.entries().map(|entry| entry.path.clone()).collect();

        let mut ignored_files: HashSet<PathBuf> = HashSet::new();
        let matcher = IgnoreMatcher::new(repo_root).ok();
        // Single status-equivalent scan of the working directory (perf budget).
        let status_cmd = super::status::StatusCmd {
            tracked: false,
            untracked: false,
            ignored: false,
            short: false,
            porcelain: false,
            branch: false,
            quiet: true,
            verbose: false,
            json: false,
        };
        let working_files =
            status_cmd.scan_working_directory(repo_root, &matcher, &mut ignored_files)?;

        let untracked: HashSet<PathBuf> = working_files
            .into_iter()
            .filter(|path| {
                !head_files.contains(path)
                    && !index_files.contains(path)
                    && !ignored_files.contains(path)
            })
            .collect();

        if untracked.is_empty() {
            return Ok(Vec::new());
        }

        let target_commit_data = odb.read(target_commit_oid).await?;
        let target_commit = mediagit_versioning::format::deserialize::<mediagit_versioning::Commit>(
            &target_commit_data,
        )?;
        let mut target_files: HashSet<PathBuf> = HashSet::new();
        Self::collect_tree_paths(odb, &target_commit.tree, Path::new(""), &mut target_files)
            .await?;

        let mut collisions: Vec<PathBuf> = untracked.intersection(&target_files).cloned().collect();
        collisions.sort();
        Ok(collisions)
    }

    /// Recursively collect every file path in a tree (directories expanded),
    /// relative to the tree root. Skips stage-debris entries (legacy
    /// merge-conflict artifacts) the same way checkout does.
    fn collect_tree_paths<'a>(
        odb: &'a mediagit_versioning::ObjectDatabase,
        tree_oid: &'a Oid,
        prefix: &'a std::path::Path,
        paths: &'a mut HashSet<PathBuf>,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<()>> + 'a>> {
        Box::pin(async move {
            let tree_data = odb.read(tree_oid).await?;
            let tree =
                mediagit_versioning::format::deserialize::<mediagit_versioning::Tree>(&tree_data)?;

            for entry in tree.iter() {
                if mediagit_versioning::is_stage_debris_key(&entry.name) {
                    continue;
                }
                let entry_path = prefix.join(&entry.name);
                match entry.mode {
                    mediagit_versioning::FileMode::Directory => {
                        Self::collect_tree_paths(odb, &entry.oid, &entry_path, paths).await?;
                    }
                    _ => {
                        paths.insert(entry_path);
                    }
                }
            }
            Ok(())
        })
    }

    async fn delete(&self, opts: &DeleteOpts) -> Result<()> {
        use crate::output;

        let repo_root = find_repo_root()?;
        let storage_path = repo_root.join(".mediagit");
        let _storage = create_storage_backend(&repo_root).await?;
        let refdb = RefDatabase::new(&storage_path);

        let mut deleted_count = 0;

        // Handle remote tracking branch deletion (e.g. origin/feature)
        if opts.remote {
            for branch_name in &opts.branches {
                // Parse "remote/branch" by splitting on first '/'
                let (remote_name, branch_part) = match branch_name.split_once('/') {
                    Some((r, b)) => (r, b),
                    None => {
                        if !opts.quiet {
                            output::warning(&format!(
                                "Invalid remote branch name '{}' (expected format: remote/branch)",
                                branch_name
                            ));
                        }
                        continue;
                    }
                };

                let remote_ref_name = format!("refs/remotes/{}/{}", remote_name, branch_part);

                // Verify ref exists and delete
                match refdb.read(&remote_ref_name).await {
                    Ok(_) => {
                        refdb.delete(&remote_ref_name).await?;
                        deleted_count += 1;

                        if !opts.quiet {
                            output::success(&format!(
                                "Deleted remote-tracking branch '{}'",
                                branch_name
                            ));
                        }
                    }
                    Err(_) => {
                        if !opts.quiet {
                            output::warning(&format!(
                                "Remote-tracking branch '{}' not found",
                                branch_name
                            ));
                        }
                    }
                }
            }

            if !opts.quiet && deleted_count == 0 {
                output::info("No remote-tracking branches were deleted");
            }

            return Ok(());
        }

        // Local branch deletion
        // Load config to check branch protection
        let config = mediagit_config::Config::load(&repo_root).await?;

        // Get current branch to prevent deletion
        let head = refdb.read("HEAD").await?;
        let current_branch = head.target;

        for branch_name in &opts.branches {
            let branch_ref_name = format!("refs/heads/{}", branch_name);

            // Check if trying to delete current branch
            if Some(&branch_ref_name) == current_branch.as_ref() {
                if !opts.quiet {
                    output::warning(&format!("Cannot delete current branch '{}'", branch_name));
                }
                continue;
            }

            // Check branch protection
            if let Some(protection) = config.get_branch_protection(branch_name) {
                if protection.prevent_deletion && !opts.force {
                    if !opts.quiet {
                        output::warning(&format!(
                            "Branch '{}' is protected (use --force to override)",
                            branch_name
                        ));
                    }
                    continue;
                }
            }

            // Verify branch exists
            match refdb.read(&branch_ref_name).await {
                Ok(_) => {
                    // Delete the branch reference
                    refdb.delete(&branch_ref_name).await?;
                    deleted_count += 1;

                    if !opts.quiet {
                        output::success(&format!("Deleted branch '{}'", branch_name));
                    }
                }
                Err(_) => {
                    if !opts.quiet {
                        output::warning(&format!("Branch '{}' not found", branch_name));
                    }
                }
            }
        }

        if !opts.quiet && deleted_count == 0 {
            output::info("No branches were deleted");
        }

        Ok(())
    }
    async fn protect(&self, opts: &ProtectOpts) -> Result<()> {
        use crate::output;
        use mediagit_config::BranchProtection;

        let repo_root = find_repo_root()?;
        let mut config = mediagit_config::Config::load(&repo_root).await?;

        // Normalize branch name
        let branch_name = opts
            .branch
            .strip_prefix("refs/heads/")
            .unwrap_or(&opts.branch)
            .to_string();

        if opts.unprotect {
            // Remove protection
            if config.unprotect_branch(&branch_name).is_some() {
                config.save(&repo_root)?;
                if !opts.quiet {
                    output::success(&format!("Removed protection from branch '{}'", branch_name));
                }
            } else if !opts.quiet {
                output::warning(&format!("Branch '{}' was not protected", branch_name));
            }
        } else {
            // Add protection
            let protection = if opts.require_reviews {
                BranchProtection::with_reviews(1)
            } else {
                BranchProtection::default_protection()
            };

            config.protect_branch_with(&branch_name, protection);
            config.save(&repo_root)?;

            if !opts.quiet {
                let mut rules = vec!["prevent force-push", "prevent deletion"];
                if opts.require_reviews {
                    rules.push("require reviews");
                }
                output::success(&format!(
                    "Protected branch '{}' ({})",
                    branch_name,
                    rules.join(", ")
                ));
            }
        }

        Ok(())
    }

    async fn rename(&self, opts: &RenameOpts) -> Result<()> {
        use crate::output;

        let repo_root = find_repo_root()?;
        let storage_path = repo_root.join(".mediagit");
        let _storage = create_storage_backend(&repo_root).await?;
        let refdb = RefDatabase::new(&storage_path);

        // Determine old and new branch names.
        // With one arg: rename current branch to first_arg.
        // With two args: rename first_arg to second_arg (like git branch -m old new).
        let (old_branch, new_branch) = if let Some(ref new_name) = opts.second_arg {
            (opts.first_arg.clone(), new_name.clone())
        } else {
            // Only one arg: rename current branch
            let head = refdb.read("HEAD").await?;
            let current = match head.target {
                Some(target) => target
                    .strip_prefix("refs/heads/")
                    .unwrap_or(&target)
                    .to_string(),
                None => anyhow::bail!("HEAD is not pointing to a branch"),
            };
            (current, opts.first_arg.clone())
        };

        let old_ref_name = format!("refs/heads/{}", old_branch);
        let new_ref_name = format!("refs/heads/{}", new_branch);

        // Validate new branch name
        if new_branch.contains("..") || new_branch.starts_with('/') || new_branch.ends_with('/') {
            anyhow::bail!("Invalid branch name: {}", new_branch);
        }

        // Check if old branch exists
        let old_ref = refdb
            .read(&old_ref_name)
            .await
            .context(format!("Branch '{}' not found", old_branch))?;

        // Check if new branch already exists (unless force)
        if !opts.force && refdb.read(&new_ref_name).await.is_ok() {
            anyhow::bail!(
                "Branch '{}' already exists. Use --force to overwrite.",
                new_branch
            );
        }

        // Get the OID from the old branch
        let branch_oid = old_ref
            .oid
            .ok_or_else(|| anyhow::anyhow!("Branch has no commit"))?;

        // Create new branch reference
        let new_ref = Ref::new_direct(new_ref_name.clone(), branch_oid);
        refdb.write(&new_ref).await?;

        // Update HEAD if renaming current branch
        let head = refdb.read("HEAD").await?;
        if head.target.as_ref() == Some(&old_ref_name) {
            let new_head = Ref::new_symbolic("HEAD".to_string(), new_ref_name.clone());
            refdb.write(&new_head).await?;
        }

        // Delete old branch reference
        refdb.delete(&old_ref_name).await?;

        if !opts.quiet {
            output::success(&format!(
                "Renamed branch '{}' to '{}'",
                old_branch, new_branch
            ));
        }

        Ok(())
    }

    async fn show(&self, opts: &ShowOpts) -> Result<()> {
        use crate::output;

        let repo_root = find_repo_root()?;
        let storage_path = repo_root.join(".mediagit");
        let _storage = create_storage_backend(&repo_root).await?;
        let refdb = RefDatabase::new(&storage_path);

        // Determine which branch to show
        let branch_name = if let Some(name) = &opts.branch {
            name.clone()
        } else {
            // Get current branch from HEAD
            let head = refdb.read("HEAD").await?;
            match head.target {
                Some(target) => target
                    .strip_prefix("refs/heads/")
                    .unwrap_or(&target)
                    .to_string(),
                None => anyhow::bail!("HEAD is not pointing to a branch"),
            }
        };

        let branch_ref_name = format!("refs/heads/{}", branch_name);

        // Get branch reference
        let branch_ref = refdb
            .read(&branch_ref_name)
            .await
            .context(format!("Branch '{}' not found", branch_name))?;

        output::header(&format!("Branch: {}", branch_name));

        if let Some(oid) = branch_ref.oid {
            output::detail("Commit", &oid.to_string());
        } else {
            output::info("No commits yet");
        }

        Ok(())
    }

    async fn merge(&self, _opts: &MergeOpts) -> Result<()> {
        // NOTE: Branch merge implementation pending (delegates to mediagit merge command)
        // Requires: conflict check, merge execution, commit creation
        anyhow::bail!("Branch merge not yet implemented (use 'mediagit merge' instead)")
    }
}
