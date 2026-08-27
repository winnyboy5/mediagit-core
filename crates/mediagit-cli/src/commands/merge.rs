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
    CheckoutManager, Commit, Index, MergeEngine, MergeStrategy, ObjectDatabase, ObjectType, Oid,
    Ref, RefDatabase, Reflog, ReflogEntry, Signature, apply_merge_to_workdir, resolve_revision,
};
use std::sync::Arc;

/// Merge branches
///
/// Join two or more development histories together. By default, creates a
/// merge commit combining the changes from both branches.
#[derive(Parser, Debug)]
#[command(after_help = "EXAMPLES:
    # Merge feature branch into current branch
    mediagit merge feature-branch

    # Merge with custom message
    mediagit merge feature-branch -m \"Merge feature X\"

    # Force merge commit (no fast-forward)
    mediagit merge --no-ff feature-branch

    # Fast-forward only merge
    mediagit merge --ff-only feature-branch

    # Merge with specific strategy
    mediagit merge -s recursive feature-branch

    # Abort merge after conflicts
    mediagit merge --abort

    # Continue merge after resolving conflicts
    mediagit merge --continue

SEE ALSO:
    mediagit-branch(1), mediagit-rebase(1), mediagit-cherry-pick(1)")]
pub struct MergeCmd {
    /// Branch to merge
    #[arg(value_name = "BRANCH", required = false)]
    pub branch: Option<String>,

    /// Merge message
    #[arg(short, long, value_name = "MESSAGE")]
    pub message: Option<String>,

    /// Create a merge commit even if fast-forward is possible
    #[arg(long)]
    pub no_ff: bool,

    /// Perform fast-forward only merge
    #[arg(long)]
    pub ff_only: bool,

    /// Squash commits before merging
    #[arg(long)]
    pub squash: bool,

    /// Merge strategy
    #[arg(short = 's', long, value_name = "STRATEGY")]
    pub strategy: Option<String>,

    /// Merge strategy option
    #[arg(short = 'X', long, value_name = "OPTION")]
    pub strategy_option: Option<String>,

    /// Don't commit merge result
    #[arg(long)]
    pub no_commit: bool,

    /// Abort merge
    #[arg(long)]
    pub abort: bool,

    /// Continue after resolving conflicts
    #[arg(long = "continue", alias = "continue-merge", hide = true)]
    pub continue_merge: bool,

    /// Quiet mode
    #[arg(short, long)]
    pub quiet: bool,

    /// Verbose mode
    #[arg(short, long)]
    pub verbose: bool,
}

impl MergeCmd {
    pub async fn execute(&self) -> Result<()> {
        // Validate that branch and --continue are mutually exclusive
        if self.continue_merge && self.branch.is_some() {
            anyhow::bail!("--continue takes no branch argument");
        }

        // Handle abort/continue first
        if self.abort {
            return self.abort_merge().await;
        }
        if self.continue_merge {
            return self.continue_merge_process().await;
        }

        // Ensure branch is provided (it's optional in the struct to allow --continue)
        let branch = self
            .branch
            .as_ref()
            .context("branch argument is required")?;

        // Find repository root
        let repo_root = find_repo_root()?;
        let storage_path = repo_root.join(".mediagit");
        let storage = create_storage_backend(&repo_root).await?;
        let refdb = RefDatabase::new(&storage_path);
        let odb = Arc::new(ObjectDatabase::with_smart_compression(storage, 1000));

        // Resolve branch to OID
        let their_oid = resolve_revision(branch, &refdb, &odb).await?;

        // Get current HEAD commit
        let head = refdb.read("HEAD").await?;
        let head_target = head.target.clone();
        let our_oid = match head.oid {
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

        // WT-3: merge had no pre-flight dirty check at all. Refuse before any
        // ref or working-tree write, so a refusal leaves nothing half done.
        crate::worktree_guard::AtRisk::check(&repo_root, &odb, Some(&our_oid), Some(&their_oid))
            .await?
            .ensure_clean("merge")?;

        // WT-1: bound what the post-merge checkouts below may delete.
        let tracked =
            crate::worktree_guard::tracked_paths(&repo_root, &odb, Some(&our_oid)).await?;

        // -X/--strategy-option is accepted by clap for git muscle-memory, but it
        // has never been implemented: the field was declared, parsed, and then
        // read nowhere in this function. `merge -X ours` therefore changed
        // nothing while looking like it had worked.
        //
        // A silent no-op is the worst outcome for a CONFLICT-RESOLUTION flag
        // specifically - the user believes they steered which side won, and
        // only finds out from the merged content. Refusing is the established
        // pattern here: `commit -a` does exactly this rather than pretend
        // (commit.rs:105-114).
        //
        // `-s/--strategy` is the real knob and IS honoured (ours/theirs/
        // recursive, below), so the error points there.
        reject_strategy_option(self.strategy_option.as_deref())?;

        // Parse merge strategy
        let strategy = match self.strategy.as_deref() {
            Some("ours") => MergeStrategy::Ours,
            Some("theirs") => MergeStrategy::Theirs,
            Some("recursive") | None => MergeStrategy::Recursive,
            Some(s) => anyhow::bail!("Unknown merge strategy: {}", s),
        };

        if !self.quiet {
            println!(
                "{} Merging {} into {}...",
                style("🔀").cyan().bold(),
                style(branch).yellow(),
                style("HEAD").cyan()
            );
            println!("{} Analyzing commit history...", style("🔍").cyan());
        }

        // Create merge engine and perform merge
        let engine = MergeEngine::new(odb.clone());

        if !self.quiet {
            println!("{} Computing merge...", style("⚙️ ").cyan());
        }

        let result = engine.merge(&our_oid, &their_oid, strategy).await?;

        // Handle merge result
        if let Some(ff_info) = &result.fast_forward {
            if ff_info.is_fast_forward && self.squash {
                // Squash merge on FF-eligible branch: create a single-parent commit
                // using their tree rather than fast-forwarding, so the branch history
                // is collapsed into one commit.
                let their_commit = Commit::read(&odb, &their_oid).await?;
                let config = mediagit_config::Config::load(&repo_root)
                    .await
                    .unwrap_or_default();
                let author_name = std::env::var("MEDIAGIT_AUTHOR_NAME").unwrap_or_else(|_| {
                    config.author.name.clone().unwrap_or_else(|| {
                        std::env::var("USER").unwrap_or_else(|_| "Unknown".to_string())
                    })
                });
                let author_email = std::env::var("MEDIAGIT_AUTHOR_EMAIL").unwrap_or_else(|_| {
                    config
                        .author
                        .email
                        .clone()
                        .unwrap_or_else(|| "unknown@localhost".to_string())
                });
                let signature = Signature::now(author_name, author_email);
                let message = self
                    .message
                    .clone()
                    .unwrap_or_else(|| format!("Squash merge branch '{}' into HEAD", branch));
                let squash_commit = Commit {
                    tree: their_commit.tree,
                    parents: vec![our_oid],
                    author: signature.clone(),
                    committer: signature,
                    message,
                };
                let commit_data = squash_commit.serialize()?;
                let commit_oid = odb.write(ObjectType::Commit, &commit_data).await?;
                if let Some(ref target) = head_target {
                    let new_ref = Ref::new_direct(target.clone(), commit_oid);
                    refdb.write(&new_ref).await?;
                } else {
                    let new_ref = Ref::new_direct("HEAD".to_string(), commit_oid);
                    refdb.write(&new_ref).await?;
                }
                let checkout_mgr =
                    CheckoutManager::new(&odb, &repo_root).with_tracked_paths(tracked);
                checkout_mgr
                    .checkout_commit(&commit_oid)
                    .await
                    .context("Failed to update working directory after squash merge")?;
                let reflog = Reflog::new(&storage_path);
                let entry = ReflogEntry::now(
                    our_oid,
                    commit_oid,
                    "user",
                    "user@mediagit",
                    &format!("merge {}: squash merge (ff)", branch),
                );
                let _ = reflog.append("HEAD", &entry).await;
                if !self.quiet {
                    println!(
                        "{} Squash merge committed: {}",
                        style("✓").green().bold(),
                        &commit_oid.to_string()[..7]
                    );
                }
                return Ok(());
            }
            if ff_info.is_fast_forward {
                if self.ff_only || !self.no_ff {
                    // Fast-forward merge
                    if !self.quiet {
                        println!(
                            "{} Fast-forwarding {} -> {}",
                            style("✓").green(),
                            &ff_info.from.to_string()[..7],
                            &ff_info.to.to_string()[..7]
                        );
                    }

                    // Update HEAD to point to their commit
                    if let Some(ref target) = head_target {
                        let new_ref = Ref::new_direct(target.clone(), their_oid);
                        refdb.write(&new_ref).await?;
                    } else {
                        let new_ref = Ref::new_direct("HEAD".to_string(), their_oid);
                        refdb.write(&new_ref).await?;
                    }

                    // Update working directory to match the merged commit (ISS-008 fix)
                    let checkout_mgr =
                        CheckoutManager::new(&odb, &repo_root).with_tracked_paths(tracked);
                    checkout_mgr
                        .checkout_commit(&their_oid)
                        .await
                        .context("Failed to update working directory after fast-forward merge")?;

                    // Record reflog entry
                    let reflog = Reflog::new(&storage_path);
                    let reflog_msg = format!("merge {}: fast-forward", branch);
                    let entry =
                        ReflogEntry::now(our_oid, their_oid, "user", "user@mediagit", &reflog_msg);
                    let _ = reflog.append("HEAD", &entry).await;

                    return Ok(());
                } else if self.ff_only {
                    anyhow::bail!("Fast-forward only requested but not possible");
                }
            }
        }

        // Check for conflicts
        if !result.conflicts.is_empty() {
            // Load the ours/theirs trees so apply_merge_to_workdir can write clean paths
            let ours_commit = mediagit_versioning::Commit::read(&odb, &our_oid).await?;
            let theirs_commit = mediagit_versioning::Commit::read(&odb, &their_oid).await?;
            let ours_tree = mediagit_versioning::Tree::read(&odb, &ours_commit.tree).await?;
            let theirs_tree = mediagit_versioning::Tree::read(&odb, &theirs_commit.tree).await?;

            let mut index = Index::load(&repo_root)?;

            apply_merge_to_workdir(
                &result,
                &ours_tree,
                &theirs_tree,
                &odb,
                &repo_root,
                &mut index,
                their_oid,
                our_oid,
            )
            .await
            .context("Failed to apply merge to working directory")?;

            index.save(&repo_root)?;

            println!(
                "{} Merge conflicts detected in {} file(s):",
                style("⚠").yellow().bold(),
                result.conflicts.len()
            );
            for conflict in &result.conflicts {
                println!("  {} {}", style("conflict:").red(), conflict.path);
                if self.verbose {
                    println!("    Type: {:?}", conflict.conflict_type);
                }
            }
            println!(
                "\n{} Conflict markers written. Resolve conflicts, 'add' them, then run 'mediagit merge --continue'",
                style("→").cyan()
            );
            std::process::exit(1);
        }

        // No conflicts - create merge commit
        if !self.no_commit {
            let tree_oid = result.tree_oid.context("No merged tree created")?;

            // Create commit signature
            // Priority: MEDIAGIT_AUTHOR_* env vars > config.toml [author] > $USER > defaults
            let config = mediagit_config::Config::load(&repo_root)
                .await
                .unwrap_or_default();
            let author_name = std::env::var("MEDIAGIT_AUTHOR_NAME").unwrap_or_else(|_| {
                config.author.name.clone().unwrap_or_else(|| {
                    std::env::var("USER").unwrap_or_else(|_| "Unknown".to_string())
                })
            });
            let author_email = std::env::var("MEDIAGIT_AUTHOR_EMAIL").unwrap_or_else(|_| {
                config
                    .author
                    .email
                    .clone()
                    .unwrap_or_else(|| "unknown@localhost".to_string())
            });

            let signature = Signature::now(author_name, author_email);

            let message = self.message.clone().unwrap_or_else(|| {
                if self.squash {
                    format!("Squash merge branch '{}' into HEAD", branch)
                } else {
                    format!("Merge branch '{}' into HEAD", branch)
                }
            });

            // Verify both parent commits exist in the object database
            // This prevents creating merge commits with invalid parent references
            odb.read(&our_oid).await.context(format!(
                "Parent commit {} (ours) not found in object database",
                our_oid
            ))?;
            odb.read(&their_oid).await.context(format!(
                "Parent commit {} (theirs) not found in object database",
                their_oid
            ))?;

            // Squash merge: collapse their branch into a single commit with one parent.
            let parents = if self.squash {
                vec![our_oid]
            } else {
                vec![our_oid, their_oid]
            };

            let merge_commit = Commit {
                tree: tree_oid,
                parents,
                author: signature.clone(),
                committer: signature,
                message,
            };

            let commit_data = merge_commit.serialize()?;
            let commit_oid = odb.write(ObjectType::Commit, &commit_data).await?;

            // Update HEAD
            if let Some(ref target) = head_target {
                let new_ref = Ref::new_direct(target.clone(), commit_oid);
                refdb.write(&new_ref).await?;
            } else {
                let new_ref = Ref::new_direct("HEAD".to_string(), commit_oid);
                refdb.write(&new_ref).await?;
            }

            // Update working directory to match the merged commit (ISS-008 fix)
            let checkout_mgr = CheckoutManager::new(&odb, &repo_root).with_tracked_paths(tracked);
            checkout_mgr
                .checkout_commit(&commit_oid)
                .await
                .context("Failed to update working directory after merge commit")?;

            // Record reflog entry
            let reflog = Reflog::new(&storage_path);
            let reflog_msg = if self.squash {
                format!("merge {}: squash merge", branch)
            } else {
                format!("merge {}: merge commit", branch)
            };
            let entry = ReflogEntry::now(our_oid, commit_oid, "user", "user@mediagit", &reflog_msg);
            let _ = reflog.append("HEAD", &entry).await;

            if !self.quiet {
                let label = if self.squash {
                    "Squash merge committed"
                } else {
                    "Merge committed"
                };
                println!(
                    "{} {}: {}",
                    style("✓").green().bold(),
                    label,
                    &commit_oid.to_string()[..7]
                );
            }
        } else if !self.quiet {
            println!("{} Merge successful (not committed)", style("✓").green());
        }

        Ok(())
    }

    async fn abort_merge(&self) -> Result<()> {
        if !self.quiet {
            println!("{} Aborting merge...", style("✗").red());
        }

        let repo_root = find_repo_root()?;
        let mediagit_dir = repo_root.join(".mediagit");

        // Clean up merge state files
        let merge_head = mediagit_dir.join("MERGE_HEAD");
        let merge_msg = mediagit_dir.join("MERGE_MSG");
        let merge_mode = mediagit_dir.join("MERGE_MODE");
        let orig_head = mediagit_dir.join("ORIG_HEAD");

        // A merge was actually in progress only if any of this state exists.
        // Guards the "no merge in progress" case: without it, aborting when
        // there's nothing to abort would still forcibly reset the working
        // tree and index, destroying unrelated staged/working changes.
        let had_merge_state =
            merge_head.exists() || merge_msg.exists() || merge_mode.exists() || orig_head.exists();

        if !had_merge_state {
            anyhow::bail!("There is no merge to abort (no merge in progress)");
        }

        let mut cleaned = 0;

        if merge_head.exists() {
            std::fs::remove_file(&merge_head).context("Failed to remove MERGE_HEAD")?;
            cleaned += 1;
        }

        if merge_msg.exists() {
            std::fs::remove_file(&merge_msg).context("Failed to remove MERGE_MSG")?;
            cleaned += 1;
        }

        if merge_mode.exists() {
            std::fs::remove_file(&merge_mode).context("Failed to remove MERGE_MODE")?;
            cleaned += 1;
        }

        if had_merge_state {
            // Restore the working tree to the pre-merge commit and clear the
            // index. An empty index means "clean" (a normal commit clears it
            // too); leaving apply_merge_to_workdir's staged conflict entries
            // behind made `status` report everything as staged after an
            // abort, and left conflict-marker files sitting in the working
            // tree.
            let pre_merge_oid = if orig_head.exists() {
                let content =
                    std::fs::read_to_string(&orig_head).context("Failed to read ORIG_HEAD")?;
                Some(Oid::from_hex(content.trim())?)
            } else {
                let refdb = RefDatabase::new(&mediagit_dir);
                refdb.resolve("HEAD").await.ok()
            };

            if let Some(pre_merge_oid) = pre_merge_oid {
                let storage = create_storage_backend(&repo_root).await?;
                let odb = Arc::new(ObjectDatabase::with_smart_compression(storage, 1000));
                // WT-1: an abort restores the pre-merge state; it must not
                // take untracked files with it. Tracked = pre-merge tree plus
                // whatever apply_merge_to_workdir staged.
                let tracked =
                    crate::worktree_guard::tracked_paths(&repo_root, &odb, Some(&pre_merge_oid))
                        .await?;
                let checkout_mgr =
                    CheckoutManager::new(&odb, &repo_root).with_tracked_paths(tracked);
                checkout_mgr
                    .checkout_commit(&pre_merge_oid)
                    .await
                    .context("Failed to restore working directory on merge abort")?;
            }

            let mut index = Index::load(&repo_root)?;
            index.clear();
            index.save(&repo_root)?;

            if orig_head.exists() {
                std::fs::remove_file(&orig_head).context("Failed to remove ORIG_HEAD")?;
                cleaned += 1;
            }
        }

        if !self.quiet {
            println!(
                "{} Merge aborted. Cleaned up {} state file(s).",
                style("✓").green(),
                cleaned
            );
        }

        Ok(())
    }

    async fn continue_merge_process(&self) -> Result<()> {
        if !self.quiet {
            println!("{} Continuing merge...", style("→").cyan());
        }

        let repo_root = find_repo_root()?;
        let mediagit_dir = repo_root.join(".mediagit");

        // Check if merge is in progress
        let merge_head_path = mediagit_dir.join("MERGE_HEAD");
        if !merge_head_path.exists() {
            anyhow::bail!("No merge in progress");
        }

        // Read MERGE_HEAD to get the OID being merged
        let merge_head_content =
            std::fs::read_to_string(&merge_head_path).context("Failed to read MERGE_HEAD")?;
        let merge_oid = mediagit_versioning::Oid::from_hex(merge_head_content.trim())?;

        // Read MERGE_MSG if it exists
        let merge_msg_path = mediagit_dir.join("MERGE_MSG");
        let message = if merge_msg_path.exists() {
            std::fs::read_to_string(&merge_msg_path).context("Failed to read MERGE_MSG")?
        } else {
            format!("Merge commit {}", merge_oid.to_hex())
        };

        // Create commit with resolved changes
        if !self.quiet {
            println!("{} Creating merge commit...", style("→").cyan());
        }

        // Load index and create tree
        let mut index = mediagit_versioning::Index::load(&repo_root)?;
        if index.is_empty() {
            anyhow::bail!("No changes staged. Use 'add' to stage resolved files.");
        }

        let storage = create_storage_backend(&repo_root).await?;
        let odb = mediagit_versioning::ObjectDatabase::with_smart_compression(storage, 1000);

        // Legacy on-disk indexes from before the merge fix may still carry
        // ::stageN debris entries; skip them from the tree and purge them so
        // they don't linger.
        let debris_paths: Vec<std::path::PathBuf> = index
            .entries()
            .filter(|e| mediagit_versioning::is_stage_debris_key(&e.path.to_string_lossy()))
            .map(|e| e.path.clone())
            .collect();
        for path in &debris_paths {
            index.remove_entry(path);
        }
        if !debris_paths.is_empty() {
            index.save(&repo_root)?;
        }

        let mut tree = mediagit_versioning::Tree::new();
        for entry in index.entries() {
            tree.add_entry(mediagit_versioning::TreeEntry::new(
                entry.path.to_string_lossy().to_string(),
                mediagit_versioning::FileMode::Regular,
                entry.oid,
            ));
        }
        let tree_oid = tree.write(&odb).await?;

        // Get current HEAD
        let refdb = mediagit_versioning::RefDatabase::new(&mediagit_dir);
        let current_oid = refdb.resolve("HEAD").await?;

        // Create merge commit with two parents
        let signature = mediagit_versioning::Signature {
            name: "MediaGit User".to_string(),
            email: "user@mediagit.local".to_string(),
            timestamp: chrono::Utc::now(),
        };

        let commit = mediagit_versioning::Commit {
            tree: tree_oid,
            parents: vec![current_oid, merge_oid],
            author: signature.clone(),
            committer: signature,
            message,
        };

        let commit_oid = commit.write(&odb).await?;

        // Update HEAD. HEAD is normally a SYMBOLIC ref pointing at
        // refs/heads/<branch>. Resolve HEAD's target branch and write the merge
        // commit there directly, mirroring how a normal `commit` updates the
        // branch ref (commit.rs uses Ref::new_direct + write, NOT update()).
        // update(force=false) rejects ANY change (it is not a real
        // fast-forward check), which would wrongly fail merge completion.
        // Detached HEAD (a direct ref) still updates "HEAD" directly.
        let head_ref = refdb.read("HEAD").await?;
        match head_ref.target {
            Some(branch) => {
                refdb.write(&Ref::new_direct(branch, commit_oid)).await?;
            }
            None => {
                refdb
                    .write(&Ref::new_direct("HEAD".to_string(), commit_oid))
                    .await?;
            }
        }

        // Record reflog
        let reflog = Reflog::new(&mediagit_dir);
        let reflog_msg = "merge: continue (resolved conflicts)".to_string();
        let entry = ReflogEntry::now(
            current_oid,
            commit_oid,
            "user",
            "user@mediagit",
            &reflog_msg,
        );
        let _ = reflog.append("HEAD", &entry).await;

        // Clean up merge state after successful commit
        if merge_head_path.exists() {
            std::fs::remove_file(&merge_head_path).ok();
        }
        if merge_msg_path.exists() {
            std::fs::remove_file(&merge_msg_path).ok();
        }
        let merge_mode = mediagit_dir.join("MERGE_MODE");
        if merge_mode.exists() {
            std::fs::remove_file(&merge_mode).ok();
        }

        if !self.quiet {
            println!("{} Merge continued successfully", style("✓").green());
            println!("  Created merge commit: {}", commit_oid.to_hex());
        }

        Ok(())
    }
}

/// Refuse `-X/--strategy-option` rather than accepting and ignoring it.
///
/// Extracted from `execute` purely so it is assertable: `execute` needs a real
/// repository, so the one behaviour worth pinning - that the flag REFUSES
/// rather than silently doing nothing - could not be tested inline. Same reason
/// `clamp_cap` and `startup_probe_timeout_secs` are separate functions.
fn reject_strategy_option(opt: Option<&str>) -> anyhow::Result<()> {
    match opt {
        None => Ok(()),
        Some(o) => Err(anyhow::anyhow!(
            "merge -X/--strategy-option is not supported in MediaGit (got '{o}').
             It was accepted but never applied, so passing it changed nothing.
             Use 'mediagit merge -s ours|theirs|recursive' to choose a strategy."
        )),
    }
}

#[cfg(test)]
#[allow(unsafe_code)] // edition-2024: test-only env::set_var/remove_var requires unsafe
mod tests {
    use super::reject_strategy_option;

    /// -X was declared, parsed, and read NOWHERE, so `merge -X ours` silently
    /// changed nothing while looking like it worked. On a conflict-resolution
    /// flag that is the worst failure mode: the user believes they chose which
    /// side won and only finds out from the merged content.
    #[test]
    fn strategy_option_is_refused_not_ignored() {
        let err = reject_strategy_option(Some("ours"))
            .unwrap_err()
            .to_string();
        assert!(err.contains("not supported"), "{err}");
        // The message must name the real knob, or the refusal just blocks the
        // user without telling them what to do instead.
        assert!(err.contains("-s ours|theirs|recursive"), "{err}");
        assert!(err.contains("ours"), "{err}");
    }

    /// The other half. Without this, "always refuse" would satisfy the test
    /// above while breaking every merge that does not pass -X.
    #[test]
    fn absent_strategy_option_is_accepted() {
        assert!(reject_strategy_option(None).is_ok());
    }

    use super::*;
    use crate::commands::utils::test_support::{REPO_ENV_LOCK, init_repo_with_commit};
    use clap::Parser;
    use tempfile::TempDir;

    fn parse(args: &[&str]) -> Result<MergeCmd, clap::Error> {
        let mut full = vec!["merge"];
        full.extend_from_slice(args);
        MergeCmd::try_parse_from(full)
    }

    #[test]
    fn parse_basic_branch() {
        let cmd = parse(&["feature-branch"]).unwrap();
        assert_eq!(cmd.branch, Some("feature-branch".to_string()));
        assert!(!cmd.no_ff);
        assert!(!cmd.ff_only);
        assert!(!cmd.squash);
        assert!(!cmd.abort);
        assert!(!cmd.continue_merge);
    }

    #[test]
    fn parse_branch_is_optional() {
        let cmd = parse(&[]).unwrap();
        assert_eq!(cmd.branch, None);
    }

    #[test]
    fn parse_all_flags() {
        let cmd = parse(&[
            "feature",
            "-m",
            "custom message",
            "--no-ff",
            "-s",
            "recursive",
            "-X",
            "ours",
            "--no-commit",
            "-q",
            "-v",
        ])
        .unwrap();
        assert_eq!(cmd.branch, Some("feature".to_string()));
        assert_eq!(cmd.message.as_deref(), Some("custom message"));
        assert!(cmd.no_ff);
        assert_eq!(cmd.strategy.as_deref(), Some("recursive"));
        assert_eq!(cmd.strategy_option.as_deref(), Some("ours"));
        assert!(cmd.no_commit);
        assert!(cmd.quiet);
        assert!(cmd.verbose);
    }

    #[test]
    fn parse_abort_and_continue_flags() {
        let cmd = parse(&["--abort"]).unwrap();
        assert!(cmd.abort);

        let cmd = parse(&["--continue-merge"]).unwrap();
        assert!(cmd.continue_merge);
    }

    #[test]
    fn parse_continue_flag_new_spelling() {
        let cmd = parse(&["--continue"]).unwrap();
        assert!(cmd.continue_merge);
    }

    #[test]
    fn parse_continue_flag_old_spelling() {
        let cmd = parse(&["--continue-merge"]).unwrap();
        assert!(cmd.continue_merge);
    }

    /// Guards `MEDIAGIT_REPO` (see `REPO_ENV_LOCK` docs) across the `.await`
    /// points in `execute()` below — mirrors `tag.rs`'s `SIGN_ENV_LOCK` pattern.
    #[allow(clippy::await_holding_lock)]
    async fn execute_in(repo_path: &std::path::Path, cmd: &MergeCmd) -> Result<()> {
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
        let cmd = parse(&["feature-branch"]).unwrap();
        let err = execute_in(temp.path(), &cmd).await.unwrap_err();
        assert!(err.to_string().contains("Not a mediagit repository"));
    }

    #[tokio::test]
    async fn execute_unknown_branch_is_error() {
        let temp = TempDir::new().unwrap();
        init_repo_with_commit(temp.path()).await;

        let cmd = parse(&["does-not-exist"]).unwrap();
        let err = execute_in(temp.path(), &cmd).await.unwrap_err();
        assert!(err.to_string().contains("Cannot resolve revision"));
    }

    #[tokio::test]
    async fn execute_branch_resolves_via_refs_remotes() {
        // QA-004: "merge origin/x" must resolve refs/remotes tracking refs,
        // not just refs/heads.
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

    #[tokio::test]
    async fn execute_unknown_strategy_is_error() {
        let temp = TempDir::new().unwrap();
        init_repo_with_commit(temp.path()).await;

        // A branch that resolves (main, i.e. HEAD itself) but an invalid -s value.
        let cmd = parse(&["main", "-s", "not-a-real-strategy"]).unwrap();
        let err = execute_in(temp.path(), &cmd).await.unwrap_err();
        assert!(err.to_string().contains("Unknown merge strategy"));
    }

    #[tokio::test]
    async fn continue_merge_without_merge_in_progress_is_error() {
        let temp = TempDir::new().unwrap();
        init_repo_with_commit(temp.path()).await;

        let cmd = parse(&["--continue-merge"]).unwrap();
        let err = execute_in(temp.path(), &cmd).await.unwrap_err();
        assert!(err.to_string().contains("No merge in progress"));
    }

    #[tokio::test]
    async fn abort_merge_with_no_state_is_error() {
        let temp = TempDir::new().unwrap();
        init_repo_with_commit(temp.path()).await;

        let cmd = parse(&["--abort"]).unwrap();
        // git semantics: aborting when no merge is in progress is an error.
        let err = execute_in(temp.path(), &cmd).await.unwrap_err();
        assert!(err.to_string().contains("no merge in progress"));
    }

    #[tokio::test]
    async fn continue_merge_with_branch_is_error() {
        let temp = TempDir::new().unwrap();
        init_repo_with_commit(temp.path()).await;

        // --continue with a branch should error
        let cmd = parse(&["--continue", "feature"]).unwrap();
        let err = execute_in(temp.path(), &cmd).await.unwrap_err();
        assert!(err.to_string().contains("--continue takes no branch"));
    }
}
