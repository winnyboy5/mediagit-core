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

use super::super::repo::{collect_local_have, create_storage_backend, find_repo_root};
use super::rebase::RebaseCmd;
use crate::progress::{OperationStats, ProgressTracker};
use anyhow::{Context, Result};
use chrono::Utc;
use clap::Parser;
use console::style;
use mediagit_versioning::{CheckoutManager, Commit, MergeStrategy, RefDatabase, Signature};
use std::sync::Arc;
use std::time::Instant;

/// Fetch and integrate remote changes
///
/// Fetches changes from a remote repository and integrates them into the
/// current branch. By default, this performs a fetch followed by a merge.
#[derive(Parser, Debug)]
#[command(after_help = "EXAMPLES:
    # Pull changes from origin into current branch
    mediagit pull

    # Pull specific branch from origin
    mediagit pull origin main

    # Pull and rebase instead of merge
    mediagit pull --rebase

    # Preview what would be pulled
    mediagit pull --dry-run

SEE ALSO:
    mediagit-push(1), mediagit-fetch(1), mediagit-merge(1), mediagit-rebase(1)")]
pub struct PullCmd {
    /// Remote name (defaults to origin)
    #[arg(value_name = "REMOTE")]
    pub remote: Option<String>,

    /// Branch to pull (defaults to tracking branch)
    #[arg(value_name = "BRANCH")]
    pub branch: Option<String>,

    /// Rebase instead of merge
    #[arg(short = 'r', long)]
    pub rebase: bool,

    /// Merge strategy (hidden - MediaGit uses binary-aware merge for media files)
    #[arg(short = 's', long, value_name = "STRATEGY", hide = true)]
    pub strategy: Option<String>,

    /// Merge option (hidden - not applicable to binary media files)
    #[arg(short = 'X', long, value_name = "OPTION", hide = true)]
    pub strategy_option: Option<String>,

    /// Perform validation without pulling
    #[arg(long)]
    pub dry_run: bool,

    /// Quit if conflicts occur
    #[arg(long)]
    pub no_commit: bool,

    /// Abort pull
    #[arg(long)]
    pub abort: bool,

    /// Continue after resolving conflicts
    #[arg(long = "continue", alias = "continue-pull", hide = true)]
    pub continue_pull: bool,

    /// Quiet mode
    #[arg(short, long)]
    pub quiet: bool,

    /// Verbose mode
    #[arg(short, long)]
    pub verbose: bool,
}

impl PullCmd {
    pub async fn execute(&self) -> Result<()> {
        let start_time = Instant::now();
        let mut stats = OperationStats::for_operation("pull");
        let progress = ProgressTracker::new(self.quiet);

        let remote = self.remote.as_deref().unwrap_or("origin");

        // Validate repository
        let repo_root = find_repo_root()?;
        let storage_path = repo_root.join(".mediagit");
        let storage = create_storage_backend(&repo_root).await?;
        let refdb = RefDatabase::new(&storage_path);

        if self.dry_run && !self.quiet {
            println!("{} Running in dry-run mode", style("ℹ").blue());
        }

        if !self.quiet {
            println!(
                "{} Preparing to pull from {}...",
                style("📥").cyan().bold(),
                style(remote).yellow()
            );
        }

        // Validate local repository state and read HEAD once
        let head = refdb.read("HEAD").await.context("Failed to read HEAD")?;

        if self.verbose {
            println!("  Remote: {}", remote);
            if let Some(branch) = &self.branch {
                println!("  Branch: {}", branch);
            }
            if self.rebase {
                println!("  Strategy: rebase");
            } else {
                println!("  Strategy: merge");
            }
        }

        // Load config to get remote URL
        let config = mediagit_config::Config::load(&repo_root).await?;
        let remote_url = config
            .resolve_remote_url(remote)
            .map_err(|e| anyhow::anyhow!("{}", e))?;

        if self.verbose {
            println!("  Remote URL: {}", remote_url);
        }

        // Initialize protocol client. Honour [performance] upload_concurrency
        // from the repo config so users can tune parallel chunk fan-out
        // without setting MEDIAGIT_UPLOAD_CONCURRENCY in the env.
        let (mut credentials, cred_source) =
            crate::repo::resolve_credentials_tiered(&repo_root, &config, remote);
        let build_client = |creds: mediagit_protocol::Credentials| {
            let mut c =
                mediagit_protocol::ProtocolClient::new(remote_url.clone()).with_credentials(creds);
            if let Some(n) = config.performance.upload_concurrency {
                c = c.with_concurrent_uploads(n);
            }
            if let Some(n) = config.performance.download_concurrency {
                c = c.with_concurrent_downloads(n);
            }
            c
        };
        let mut client = build_client(credentials.clone());

        // Initialize ODB with smart compression for consistent read/write
        let odb = Arc::new(mediagit_versioning::ObjectDatabase::with_smart_compression(
            Arc::clone(&storage),
            1000,
        ));

        // Determine remote ref to pull
        // Clone head.target early since we need it later for branch comparison
        let current_head_target = head.target.clone();
        let remote_ref = if let Some(branch) = &self.branch {
            // Normalize branch name to handle both short and full ref paths
            mediagit_versioning::normalize_ref_name(branch)
        } else {
            // Default: pull tracking branch for current HEAD (reuse head from above)
            current_head_target
                .clone()
                .ok_or_else(|| anyhow::anyhow!("HEAD is detached, please specify a branch"))?
        };

        if self.verbose {
            println!("  Pulling ref: {}", remote_ref);
        }

        // Get current local ref state BEFORE downloading
        let local_ref = refdb.read(&remote_ref).await.ok();

        // ================================================================
        // STEP 1: Fetch ALL remote branch refs and update tracking refs
        // This ensures new branches pushed by other collaborators are visible
        // NOTE: This runs BEFORE the "already up to date" check so users
        // always see new remote branches even when current branch is synced
        // ================================================================
        // First authenticated call of this command — a cached keychain
        // credential may have expired; on a 401, invalidate it and retry
        // once with the next tier (I11).
        let all_remote_refs = match client.get_refs().await {
            Ok(r) => r,
            Err(e) if crate::repo::invalidate_on_unauthorized(&config, remote, cred_source, &e) => {
                credentials = crate::repo::resolve_credentials(&repo_root, &config, remote);
                client = build_client(credentials.clone());
                client.get_refs().await?
            }
            Err(e) => return Err(e),
        };
        crate::repo::remember_credentials(&config, remote, &credentials);
        let remote_branches: Vec<_> = all_remote_refs
            .refs
            .iter()
            .filter(|r| r.name.starts_with("refs/heads/"))
            .collect();

        // Get remote OID for current branch (for sync check below)
        let remote_oid_check = all_remote_refs
            .refs
            .iter()
            .find(|r| r.name == remote_ref)
            .map(|r| r.oid.clone());

        if !self.dry_run {
            // Create remotes directory for tracking refs
            let remotes_dir = storage_path.join("refs").join("remotes").join(remote);
            std::fs::create_dir_all(&remotes_dir)?;

            let mut tracking_refs_updated = 0;
            for branch_ref in &remote_branches {
                let branch_name = branch_ref
                    .name
                    .strip_prefix("refs/heads/")
                    .unwrap_or(&branch_ref.name);
                let tracking_ref_name = format!("refs/remotes/{}/{}", remote, branch_name);

                // Parse remote OID and update tracking ref
                if let Ok(branch_oid) = mediagit_versioning::Oid::from_hex(&branch_ref.oid) {
                    // Create parent directories for nested branches (e.g., feature/auth)
                    let tracking_path = storage_path
                        .join("refs")
                        .join("remotes")
                        .join(remote)
                        .join(branch_name);
                    if let Some(parent) = tracking_path.parent() {
                        std::fs::create_dir_all(parent)?;
                    }

                    let tracking_ref =
                        mediagit_versioning::Ref::new_direct(tracking_ref_name.clone(), branch_oid);
                    refdb.write(&tracking_ref).await?;
                    tracking_refs_updated += 1;

                    if self.verbose {
                        println!(
                            "  {} {} -> {}",
                            style("→").cyan(),
                            tracking_ref_name,
                            &branch_ref.oid[..8.min(branch_ref.oid.len())]
                        );
                    }
                }
            }

            if !self.quiet && tracking_refs_updated > 0 {
                println!(
                    "{} Fetched {} remote tracking refs",
                    style("✓").green(),
                    tracking_refs_updated
                );
            }
        }

        // Check if current branch is already synchronized (avoid redundant object download)
        if let (Some(local), Some(remote)) = (
            local_ref.as_ref().and_then(|r| r.oid.as_ref()),
            remote_oid_check.as_ref(),
        ) {
            let local_oid_str = local.to_hex();
            if &local_oid_str == remote {
                // BUG-008 guard: matching OIDs are insufficient if the object
                // isn't actually in the local ODB (can happen when a branch
                // was fetched into a tracking ref but its objects were never
                // downloaded — clone ships only default-branch objects).
                let have_object = odb.exists(local).await.unwrap_or(false);
                if have_object {
                    if !self.quiet {
                        println!("{} Already up to date", style("✓").green());
                        println!("  {} {}", style("→").cyan(), &remote[..8]);
                    }
                    return Ok(());
                }
                // Fall through to pull — we know the OID but don't have the
                // object, so we need to download it.
            }
        }

        if !self.dry_run {
            // ================================================================
            // STEP 2: Pull the specific branch's objects
            // ================================================================
            // Build the "have" list from ALL local ref state for incremental
            // pull. Sending every local tip (branches, tags, remote tracking
            // refs) lets the server prune anything we already have from the
            // pack walk — not just the current branch's tip.
            let local_have = collect_local_have(&refdb, &odb).await;

            // Pull using streaming protocol (memory-efficient for large files)
            // Pass local OIDs to avoid downloading objects we already have
            // Use spinner: total bytes unknown, pull_streaming has no progress callback
            let download_pb = progress.spinner("Receiving objects...");

            // Use streaming pull - objects are written directly to ODB as they're received
            let chunked_oids = client.pull_streaming(&odb, &remote_ref, local_have).await?;

            if !self.quiet {
                if chunked_oids.is_empty() {
                    println!(
                        "{} Received and unpacked objects (streaming)",
                        style("↓").cyan()
                    );
                } else {
                    println!(
                        "{} Received pack (streaming) + {} chunked objects",
                        style("↓").cyan(),
                        chunked_oids.len()
                    );
                }
            }
            download_pb.finish_with_message("Received objects");

            // Download chunked objects (large files)
            if !chunked_oids.is_empty() {
                // Total bytes seeded from manifests in Phase 1 via first on_progress call.
                let chunk_pb = progress.download_bar("Downloading large files", 0);

                let chunk_pb_ref = chunk_pb.clone();
                let mut last_bytes_done = 0u64;
                let chunks_downloaded = client
                    .download_chunked_objects(
                        &odb,
                        &chunked_oids,
                        move |bytes_done, bytes_total, msg| {
                            if chunk_pb_ref.length() != Some(bytes_total) {
                                chunk_pb_ref.set_length(bytes_total);
                                chunk_pb_ref.reset_eta();
                            }
                            // Reset ETA on large jumps (end-of-object correction) so
                            // the 5s inter-object delta-check stall doesn't produce "eta 231y".
                            if bytes_done.saturating_sub(last_bytes_done) > 1_048_576 {
                                chunk_pb_ref.reset_eta();
                            }
                            last_bytes_done = bytes_done;
                            chunk_pb_ref.set_position(bytes_done);
                            chunk_pb_ref.set_message(msg.to_string());
                        },
                    )
                    .await?;

                chunk_pb.finish_with_message(format!("Downloaded {} chunks", chunks_downloaded));

                stats.objects_received += chunks_downloaded as u64;

                if !self.quiet {
                    println!(
                        "{} Downloaded {} chunks for {} large files",
                        style("✓").green(),
                        chunks_downloaded,
                        chunked_oids.len()
                    );
                }
            }

            // Reuse remote OID fetched at the top of the function (L180)
            // instead of making a second get_refs() HTTP call.
            let remote_oid = remote_oid_check
                .clone()
                .ok_or_else(|| anyhow::anyhow!("Remote ref '{}' not found", remote_ref))?;

            // Update local ref to match remote
            let remote_oid_parsed = mediagit_versioning::Oid::from_hex(&remote_oid)
                .map_err(|e| anyhow::anyhow!("Invalid remote OID: {}", e))?;

            // Update remote tracking ref first (refs/remotes/<remote>/<branch>)
            if remote_ref.starts_with("refs/heads/") {
                // Safe: we just checked for the prefix above
                let branch_name = remote_ref
                    .strip_prefix("refs/heads/")
                    .unwrap_or(&remote_ref);
                let tracking_ref_name = format!("refs/remotes/{}/{}", remote, branch_name);

                // Create remotes directory if needed
                let remotes_dir = storage_path.join("refs").join("remotes").join(remote);
                std::fs::create_dir_all(&remotes_dir)?;

                let tracking_ref = mediagit_versioning::Ref::new_direct(
                    tracking_ref_name.clone(),
                    remote_oid_parsed,
                );
                refdb.write(&tracking_ref).await?;

                if self.verbose {
                    println!(
                        "  Updated tracking ref: {} -> {}",
                        tracking_ref_name,
                        &remote_oid[..8]
                    );
                }
            }

            // Integrate changes (merge or rebase) - ONLY if pulling the current branch
            // Check if we're pulling the current branch or a different one
            let is_pulling_current_branch = match &current_head_target {
                Some(target) => target == &remote_ref,
                None => false, // Detached HEAD - don't auto-merge
            };

            // Only write the local branch ref directly here when it's NOT the
            // current branch. For the current branch, the local ref must only
            // move as a RESULT of integration (fast-forward/rebase/merge)
            // below, never before it -- writing it here would silently
            // clobber a divergent local commit (BUG-RM-1).
            if !is_pulling_current_branch {
                let ref_update =
                    mediagit_versioning::Ref::new_direct(remote_ref.clone(), remote_oid_parsed);
                refdb.write(&ref_update).await?;

                if !self.quiet {
                    println!(
                        "{} Updated {} to {}",
                        style("✓").green(),
                        remote_ref,
                        &remote_oid[..8]
                    );
                }

                // Pulled a different branch - just update refs, don't merge into current
                let branch_short = remote_ref
                    .strip_prefix("refs/heads/")
                    .unwrap_or(&remote_ref);
                if !self.quiet {
                    println!(
                        "{} Fetched branch '{}' (use: mediagit branch switch {})",
                        style("✓").green(),
                        branch_short,
                        branch_short
                    );
                }
            } else if self.rebase {
                // Rebase integration using the RebaseCmd
                let head = refdb.read("HEAD").await?;
                // HEAD is normally symbolic on a checked-out branch (oid: None,
                // target: Some("refs/heads/<branch>")). Resolve the real head
                // OID in that case instead of treating it as "no local commits"
                // (BUG-RM-1: that wrongly took the fast-forward path and
                // discarded divergent local commits).
                let head_oid = match head.oid {
                    Some(oid) => oid,
                    None => refdb.resolve("HEAD").await?,
                };

                let lca_finder = mediagit_versioning::LcaFinder::new(Arc::clone(&odb));
                if lca_finder
                    .is_ancestor(&head_oid, &remote_oid_parsed)
                    .await?
                {
                    // Local branch has no divergent commits -- plain fast-forward
                    fast_forward_to(
                        &refdb,
                        &odb,
                        &repo_root,
                        &head,
                        &remote_oid_parsed,
                        &remote_oid,
                        self.quiet,
                        self.verbose,
                    )
                    .await?;
                } else {
                    // Get upstream ref name (e.g., "origin/main" or just "main")
                    let upstream_name = if remote_ref.starts_with("refs/heads/") {
                        // Use remote tracking ref as upstream
                        let branch_name = remote_ref
                            .strip_prefix("refs/heads/")
                            .unwrap_or(&remote_ref);
                        format!("{}/{}", remote, branch_name)
                    } else {
                        remote_oid.clone()
                    };

                    if self.verbose {
                        let head_hex = head_oid.to_hex();
                        println!("  Rebasing {} onto {}", &head_hex[..8], &remote_oid[..8]);
                    }

                    // Create and execute rebase command
                    let rebase_cmd = RebaseCmd {
                        upstream: upstream_name,
                        branch: None, // Rebase current branch
                        rebase_merges: false,
                        keep_empty: false,
                        autosquash: false,
                        abort: false,
                        continue_rebase: false,
                        skip: false,
                        quiet: self.quiet,
                        verbose: self.verbose,
                    };

                    rebase_cmd.execute().await?;

                    if !self.quiet {
                        println!("{} Rebased successfully", style("✓").green().bold());
                    }
                }
            } else {
                // Merge integration - only for CURRENT branch
                let head = refdb.read("HEAD").await?;
                // See rebase branch above: resolve symbolic HEAD to its real
                // OID instead of treating it as "no local commits" (BUG-RM-1).
                let head_oid = match head.oid {
                    Some(oid) => oid,
                    None => refdb.resolve("HEAD").await?,
                };

                let lca_finder = mediagit_versioning::LcaFinder::new(Arc::clone(&odb));
                if lca_finder
                    .is_ancestor(&head_oid, &remote_oid_parsed)
                    .await?
                {
                    // Local branch has no divergent commits -- plain fast-forward
                    fast_forward_to(
                        &refdb,
                        &odb,
                        &repo_root,
                        &head,
                        &remote_oid_parsed,
                        &remote_oid,
                        self.quiet,
                        self.verbose,
                    )
                    .await?;
                } else {
                    // WT-3: refuse before the merge writes anything. The merge
                    // tree does not exist yet, so collisions cannot be checked
                    // here — `target: None` limits this to uncommitted edits to
                    // tracked files, which is the hazard a merge introduces.
                    crate::worktree_guard::AtRisk::check(&repo_root, &odb, Some(&head_oid), None)
                        .await?
                        .ensure_clean("merge")?;

                    let merge_engine = mediagit_versioning::MergeEngine::new(Arc::clone(&odb));

                    if self.verbose {
                        let head_hex = head_oid.to_hex();
                        println!("  Merging {} into {}", &remote_oid[..8], &head_hex[..8]);
                    }

                    let merge_result = merge_engine
                        .merge(&head_oid, &remote_oid_parsed, MergeStrategy::Recursive)
                        .await?;

                    // Create merge commit
                    if let Some(tree_oid) = merge_result.tree_oid {
                        let author = Signature::new(
                            "MediaGit User".to_string(),
                            "user@mediagit.local".to_string(),
                            Utc::now(),
                        );
                        // Create merge commit with both parents (local HEAD and remote)
                        let branch_name = head.target.as_deref().unwrap_or("HEAD");
                        let merge_commit = Commit::with_parents(
                            tree_oid,
                            vec![head_oid, remote_oid_parsed],
                            author.clone(),
                            author,
                            format!("Merge remote branch into {}", branch_name),
                        );
                        let commit_oid = merge_commit.write(&odb).await?;

                        // Update HEAD to merge commit
                        if let Some(target) = &head.target {
                            // HEAD is symbolic, update the target branch
                            let target_ref =
                                mediagit_versioning::Ref::new_direct(target.clone(), commit_oid);
                            refdb.write(&target_ref).await?;
                        } else {
                            // HEAD is detached, update HEAD directly
                            let head_ref = mediagit_versioning::Ref::new_direct(
                                "HEAD".to_string(),
                                commit_oid,
                            );
                            refdb.write(&head_ref).await?;
                        }

                        // Checkout working directory to match merge result,
                        // bounded to tracked paths so untracked work survives
                        // the rewrite (WT-1).
                        let tracked =
                            crate::worktree_guard::tracked_paths(&repo_root, &odb, Some(&head_oid))
                                .await?;
                        let checkout_mgr =
                            CheckoutManager::new(&odb, &repo_root).with_tracked_paths(tracked);
                        let files_count = checkout_mgr.checkout_commit(&commit_oid).await?;
                        if self.verbose {
                            println!("  Checked out {} files", files_count);
                        }

                        if !self.quiet {
                            let commit_hex = commit_oid.to_hex();
                            println!(
                                "{} Merged successfully to {}",
                                style("✓").green().bold(),
                                &commit_hex[..8]
                            );
                        }
                    } else {
                        anyhow::bail!("Merge failed: no tree result");
                    }
                }
            }
        } else if !self.quiet {
            println!("{} Dry run complete (no changes made)", style("ℹ").blue());
        }

        // Print operation summary
        stats.duration_ms = start_time.elapsed().as_millis() as u64;
        if !self.quiet && !self.dry_run {
            println!("\n{} {}", style("📊").cyan(), stats.summary());
        }

        // Save stats for later retrieval by stats command
        if !self.dry_run
            && let Err(e) = stats.save(&storage_path)
        {
            tracing::warn!("Failed to save operation stats: {}", e);
        }

        // Best-effort auto-gc: reclaims stale objects from partial fetches
        // and trim points changed by the pull. Skipped on dry-run.
        if !self.dry_run {
            let _ =
                crate::auto_gc::maybe_run(&repo_root, crate::auto_gc::TriggerMode::PostPull).await;
        }

        Ok(())
    }
}

/// Fast-forward HEAD to the given OID: update the ref (symbolic or direct),
/// checkout the working directory, and print status messages.
#[allow(clippy::too_many_arguments)]
async fn fast_forward_to(
    refdb: &RefDatabase,
    odb: &std::sync::Arc<mediagit_versioning::ObjectDatabase>,
    repo_root: &std::path::Path,
    head: &mediagit_versioning::Ref,
    oid: &mediagit_versioning::Oid,
    oid_hex: &str,
    quiet: bool,
    verbose: bool,
) -> Result<()> {
    // WT-1/WT-3: refuse *before* moving the ref. A fast-forward rewrites the
    // working tree exactly like `merge`/`switch` do, and this is the ordinary
    // outcome of a routine `pull` — so it was the most reachable path by which
    // uncommitted edits were overwritten and untracked files deleted.
    let head_oid = refdb.resolve("HEAD").await.ok();
    crate::worktree_guard::AtRisk::check(repo_root, odb, head_oid.as_ref(), Some(oid))
        .await?
        .ensure_clean("pull")?;

    // Update the right ref — symbolic HEAD updates the target branch, detached
    // HEAD updates HEAD directly.
    if let Some(target) = &head.target {
        let target_ref = mediagit_versioning::Ref::new_direct(target.clone(), *oid);
        refdb.write(&target_ref).await?;
    } else {
        let head_ref = mediagit_versioning::Ref::new_direct("HEAD".to_string(), *oid);
        refdb.write(&head_ref).await?;
    }

    // Checkout working directory to match new HEAD. Bounded to tracked paths so
    // untracked work is never collateral, even though the guard above already
    // refused on collisions (WT-1).
    let tracked = crate::worktree_guard::tracked_paths(repo_root, odb, head_oid.as_ref()).await?;
    let checkout_mgr = CheckoutManager::new(odb, repo_root).with_tracked_paths(tracked);
    let files_count = checkout_mgr.checkout_commit(oid).await?;
    if verbose {
        println!("  Checked out {} files", files_count);
    }

    if !quiet {
        println!(
            "{} Fast-forwarded to {}",
            style("✓").green().bold(),
            &oid_hex[..8]
        );
    }

    Ok(())
}
