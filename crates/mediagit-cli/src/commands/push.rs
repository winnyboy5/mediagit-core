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
use super::utils::validate_ref_name;
use crate::progress::{OperationStats, ProgressTracker};
use anyhow::{Context, Result};
use clap::Parser;
use console::style;
use indicatif::ProgressBar;
use mediagit_protocol::PushPhase;
use mediagit_versioning::{LcaFinder, ObjectType, RefDatabase};
use std::sync::{Arc, Mutex};
use std::time::Instant;

/// Update remote references and send objects
///
/// Pushes local commits to a remote repository, updating the remote
/// references to point to the new commits. This makes your local changes
/// available to others.
///
/// By default, pushes only the current branch to the remote.
#[derive(Parser, Debug)]
#[command(after_help = "EXAMPLES:
    # Push current branch to origin (default)
    mediagit push

    # Push specific branch to origin
    mediagit push origin main

    # Push all branches
    mediagit push --all

    # Push all tags
    mediagit push --tags

    # Push and set upstream tracking
    mediagit push -u origin feature-branch

    # Preview what would be pushed
    mediagit push --dry-run

    # Force push (use with caution!)
    mediagit push --force-with-lease

SEE ALSO:
    mediagit-pull(1), mediagit-fetch(1), mediagit-remote(1)")]
pub struct PushCmd {
    /// Remote name or URL
    #[arg(value_name = "REMOTE")]
    pub remote: Option<String>,

    /// Refspec to push (e.g., main:main, HEAD:refs/for/main)
    #[arg(value_name = "REFSPEC")]
    pub refspec: Vec<String>,

    /// Push all branches
    #[arg(short, long)]
    pub all: bool,

    /// Push all tags
    #[arg(long)]
    pub tags: bool,

    /// Follow tags
    #[arg(long)]
    pub follow_tags: bool,

    /// Perform validation without sending
    #[arg(long)]
    pub dry_run: bool,

    /// Force push (dangerous)
    #[arg(short = 'f', long)]
    pub force: bool,

    /// Force with lease (safer)
    #[arg(long)]
    pub force_with_lease: bool,

    /// Delete remote ref
    #[arg(short = 'd', long)]
    pub delete: bool,

    /// Set upstream branch
    #[arg(short = 'u', long)]
    pub set_upstream: bool,

    /// Push without setting upstream (not recommended for long-lived branches)
    #[arg(long)]
    pub no_track: bool,

    /// Quiet mode
    #[arg(short, long)]
    pub quiet: bool,

    /// Verbose mode
    #[arg(short, long)]
    pub verbose: bool,

    /// Verify remote chunk integrity and force re-upload any chunk the
    /// server reports as corrupted, using the local repo as the source of
    /// truth. Runs even if refs are already up to date (that's the case a
    /// poisoned remote needs). Always runs a full strong verify (BLAKE3
    /// re-hash) — not gated by MEDIAGIT_STRONG_VERIFY.
    #[arg(long)]
    pub repair: bool,
}

impl PushCmd {
    pub async fn execute(&self) -> Result<()> {
        let start_time = Instant::now();
        let mut stats = OperationStats::for_operation("push");

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
                "{} Preparing to push to {}...",
                style("📤").cyan().bold(),
                style(remote).yellow()
            );
        }

        // Validate local refs exist and read HEAD once
        let head = refdb.read("HEAD").await.context("Failed to read HEAD")?;

        if head.oid.is_none() && head.target.is_none() {
            anyhow::bail!("Nothing to push - no commits yet");
        }

        if self.verbose {
            println!("  Remote: {}", remote);
            if !self.refspec.is_empty() {
                println!("  Refspecs: {}", self.refspec.join(", "));
            }
            if self.force {
                println!("  {} Force push enabled", style("⚠").yellow());
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
        let credentials = crate::repo::resolve_credentials(&repo_root, &config, remote);
        let mut client = mediagit_protocol::ProtocolClient::new(remote_url)
            .with_credentials(credentials.clone());
        if let Some(n) = config.performance.upload_concurrency {
            client = client.with_concurrent_uploads(n);
        }

        // Initialize ODB with smart compression for consistent read/write
        let odb = Arc::new(mediagit_versioning::ObjectDatabase::with_smart_compression(
            Arc::clone(&storage),
            1000,
        ));

        // ===== Handle push --delete: delete remote refs without uploading objects =====
        if self.delete {
            if self.refspec.is_empty() {
                anyhow::bail!("push --delete requires at least one branch name");
            }

            if !self.quiet {
                println!(
                    "{} Deleting remote branch(es): {}",
                    style("🗑️").red().bold(),
                    self.refspec.join(", ")
                );
            }

            // Get remote refs to find current OIDs for safety
            let remote_refs = client.get_refs().await?;

            let mut updates = Vec::new();
            for ref_name in &self.refspec {
                let full_ref = mediagit_versioning::normalize_ref_name(ref_name);
                validate_ref_name(&full_ref)?;

                // Get current remote OID for safety check
                let remote_oid = remote_refs
                    .refs
                    .iter()
                    .find(|r| r.name == full_ref)
                    .map(|r| r.oid.clone());

                if remote_oid.is_none() {
                    if !self.quiet {
                        println!(
                            "  {} Branch '{}' does not exist on remote",
                            style("⚠").yellow(),
                            ref_name
                        );
                    }
                    continue;
                }

                updates.push(mediagit_protocol::RefUpdate {
                    name: full_ref,
                    old_oid: remote_oid,
                    new_oid: String::new(), // ignored for delete
                    delete: true,
                });
            }

            if updates.is_empty() {
                if !self.quiet {
                    println!("{} No branches to delete", style("ℹ").blue());
                }
                return Ok(());
            }

            // Send delete request directly (no packing/uploading)
            let request = mediagit_protocol::RefUpdateRequest {
                updates: updates.clone(),
                force: self.force,
            };

            let response = client.update_refs(request).await?;
            crate::repo::remember_credentials(&config, remote, &credentials);

            // Report results
            for result in &response.results {
                if result.success {
                    if !self.quiet {
                        let display_name = result
                            .ref_name
                            .strip_prefix("refs/heads/")
                            .unwrap_or(&result.ref_name);
                        println!(
                            "  {} Deleted remote branch '{}'",
                            style("✓").green(),
                            display_name
                        );
                    }

                    // Clean up local remote-tracking ref
                    let tracking_ref = result
                        .ref_name
                        .replace("refs/heads/", &format!("refs/remotes/{}/", remote));
                    if refdb.read(&tracking_ref).await.is_ok() {
                        if let Err(e) = refdb.delete(&tracking_ref).await {
                            tracing::warn!(
                                "Failed to delete local tracking ref {}: {}",
                                tracking_ref,
                                e
                            );
                        } else if self.verbose {
                            println!("  Cleaned up local tracking ref: {}", tracking_ref);
                        }
                    }
                } else if !self.quiet {
                    let display_name = result
                        .ref_name
                        .strip_prefix("refs/heads/")
                        .unwrap_or(&result.ref_name);
                    let error_msg = result.error.as_deref().unwrap_or("unknown error");
                    println!(
                        "  {} Failed to delete '{}': {}",
                        style("✗").red(),
                        display_name,
                        error_msg
                    );
                }
            }

            if !response.success {
                anyhow::bail!("Some branch deletions failed");
            }

            // Hint about garbage collection
            if !self.quiet {
                println!(
                    "\n{} To reclaim storage, run: mediagit gc",
                    style("hint:").cyan()
                );
            }

            stats.duration_ms = start_time.elapsed().as_millis() as u64;
            if let Err(e) = stats.save(&storage_path) {
                tracing::warn!("Failed to save operation stats: {}", e);
            }

            return Ok(());
        }

        // Determine which refs to push
        let mut refs_to_push: Vec<String> = if self.all {
            // Push all local branches
            let branches = refdb.list_branches().await?;
            if branches.is_empty() {
                anyhow::bail!("No branches to push");
            }
            if self.verbose {
                println!("  Pushing {} branches", branches.len());
            }
            branches
        } else if self.refspec.is_empty() {
            // Default: push current branch (reuse head from above)
            let ref_name = head
                .target
                .ok_or_else(|| anyhow::anyhow!("HEAD is detached, please specify a refspec"))?;
            vec![ref_name]
        } else {
            // Use refspecs - normalize to handle both short and full ref names.
            // For bare names (e.g. "v1.0"), normalize_ref_name produces refs/heads/<name>.
            // If that ref doesn't exist locally, fall back to refs/tags/<name> so that
            // `push origin v1.0` works when v1.0 is a tag rather than a branch.
            let mut resolved = Vec::new();
            for r in &self.refspec {
                let candidate = mediagit_versioning::normalize_ref_name(r);
                // If the candidate is a heads ref but doesn't exist, try tags
                let resolved_ref = if candidate.starts_with("refs/heads/")
                    && refdb.read(&candidate).await.is_err()
                {
                    let tag_candidate = format!(
                        "refs/tags/{}",
                        candidate.strip_prefix("refs/heads/").unwrap()
                    );
                    if refdb.read(&tag_candidate).await.is_ok() {
                        tag_candidate
                    } else {
                        // Neither exists — keep the original so the error is reported below
                        candidate
                    }
                } else {
                    candidate
                };
                resolved.push(resolved_ref);
            }
            resolved
        };

        // Get remote refs to check current state (404 = repo not created yet, treated as empty)
        let remote_refs = client.get_refs_or_empty().await?;
        crate::repo::remember_credentials(&config, remote, &credentials);

        // Append tag refs when --tags or --follow-tags is specified
        if self.tags || self.follow_tags {
            let all_tags = refdb.list_tags().await?;
            if self.follow_tags {
                // Only include tags whose target OID is an ancestor of a pushed branch tip
                let lca = LcaFinder::new(Arc::clone(&odb));
                // Collect branch tips from refs_to_push
                let mut branch_tips: Vec<mediagit_versioning::Oid> = Vec::new();
                for branch_ref in &refs_to_push {
                    if let Ok(r) = refdb.read(branch_ref).await {
                        if let Some(oid) = r.oid {
                            branch_tips.push(oid);
                        }
                    }
                }
                for tag_ref in all_tags {
                    if let Ok(r) = refdb.read(&tag_ref).await {
                        if let Some(tag_oid) = r.oid {
                            // Include tag if its target is an ancestor of any branch tip
                            let mut include = false;
                            for tip in &branch_tips {
                                if lca.is_ancestor(&tag_oid, tip).await.unwrap_or(false) {
                                    include = true;
                                    break;
                                }
                            }
                            if include && !refs_to_push.contains(&tag_ref) {
                                refs_to_push.push(tag_ref);
                            }
                        }
                    }
                }
            } else {
                // --tags: include all local tags
                for tag_ref in all_tags {
                    if !refs_to_push.contains(&tag_ref) {
                        refs_to_push.push(tag_ref);
                    }
                }
            }
            if self.verbose {
                let tag_count = refs_to_push
                    .iter()
                    .filter(|r| r.starts_with("refs/tags/"))
                    .count();
                println!("  Including {} tag(s)", tag_count);
            }
        }

        // Build list of ref updates, skipping those already up-to-date
        let mut updates = Vec::new();
        let mut skipped_uptodate = 0;
        // Local OIDs for every ref being pushed, collected regardless of
        // up-to-date status. --repair needs the FULL set (not just refs with
        // new commits) because a poisoned remote chunk is, by definition,
        // one the server already believes it has.
        let mut repair_commit_oids: Vec<mediagit_versioning::Oid> = Vec::new();

        for ref_to_push in &refs_to_push {
            // Validate ref name before pushing
            validate_ref_name(ref_to_push)?;

            // Read local ref OID
            let local_ref = refdb.read(ref_to_push).await?;
            let local_oid = local_ref
                .oid
                .ok_or_else(|| anyhow::anyhow!("Ref '{}' has no OID", ref_to_push))?;

            if self.repair {
                repair_commit_oids.push(local_oid);
            }

            let remote_oid = remote_refs
                .refs
                .iter()
                .find(|r| &r.name == ref_to_push)
                .map(|r| r.oid.clone());

            let local_oid_str = local_oid.to_hex();

            // Check if already up-to-date
            if let Some(ref remote) = remote_oid {
                if remote == &local_oid_str {
                    skipped_uptodate += 1;
                    if self.verbose {
                        println!("  {} already up to date", ref_to_push);
                    }
                    continue;
                }
            }

            updates.push(mediagit_protocol::RefUpdate {
                name: ref_to_push.clone(),
                old_oid: remote_oid,
                new_oid: local_oid_str,
                delete: false,
            });
        }

        // BLOCK: Check for new branches without upstream (Git-like behavior)
        // For non-default branches, require explicit -u or --no-track
        if !self.set_upstream && !self.no_track && self.refspec.is_empty() && !self.all {
            for update in &updates {
                // Only check new branches (no old_oid means it doesn't exist on remote)
                if update.old_oid.is_none() && update.name.starts_with("refs/heads/") {
                    let branch_name = update
                        .name
                        .strip_prefix("refs/heads/")
                        .unwrap_or(&update.name);

                    // Default branches (main/master) are allowed without -u
                    let is_default_branch = branch_name == "main" || branch_name == "master";

                    // Check if upstream is already configured
                    let has_upstream = config.get_branch_upstream(branch_name).is_some();

                    // Block if: new branch + not default + no upstream configured
                    if !is_default_branch && !has_upstream {
                        anyhow::bail!(
                            "The current branch '{}' has no upstream branch.\n\
                            To push the current branch and set the remote as upstream, use:\n\n\
                            \x20   mediagit push -u {} {}\n",
                            branch_name,
                            remote,
                            branch_name
                        );
                    }
                }
            }
        }

        // --repair: strong-verify every chunk reachable from the pushed refs and
        // force re-upload any the server reports as corrupted. Runs even when refs
        // are already up to date - that's exactly the poisoned-remote scenario
        // (BUG-RM-3), since ordinary push dedup never re-checks content once the
        // server claims to already have a chunk.
        if self.repair {
            if !self.quiet {
                println!("{} Verifying remote chunk integrity...", style("🔧").cyan());
            }
            let report = client
                .repair_remote(&odb, repair_commit_oids.clone())
                .await
                .context("Remote chunk repair failed")?;
            if !self.quiet {
                println!(
                    "  {} verified {} chunk(s): {} repaired, {} unrepairable",
                    style("✓").green(),
                    report.verified,
                    report.repaired,
                    report.unrepairable.len()
                );
                if !report.unrepairable.is_empty() {
                    println!(
                        "  {} unrepairable (missing/unreadable locally): {:?}",
                        style("⚠").yellow(),
                        &report.unrepairable[..report.unrepairable.len().min(5)]
                    );
                }
            }
        }

        // If all refs are up-to-date, exit early
        if updates.is_empty() {
            if !self.quiet {
                println!(
                    "{} All {} refs already up to date",
                    style("✓").green(),
                    skipped_uptodate
                );
            }
            return Ok(());
        }

        if !self.dry_run {
            let tracker = ProgressTracker::new(self.quiet);
            // Spinner covers Collecting + Packing phases (no byte count yet)
            let phase_spinner = if !self.quiet {
                Some(tracker.spinner("Collecting objects..."))
            } else {
                None
            };
            // Bytes bar created on first Uploading callback; shared via Arc<Mutex>
            let upload_pb: Arc<Mutex<Option<ProgressBar>>> = Arc::new(Mutex::new(None));
            let upload_pb_cb = Arc::clone(&upload_pb);

            let (result, push_stats) = client
                .push_with_progress(&odb, updates.clone(), self.force, move |progress| {
                    match progress.phase {
                        PushPhase::Collecting => {
                            if let Some(ref sp) = phase_spinner {
                                let msg = if progress.total > 0 {
                                    format!(
                                        "Collecting... {}/{} objects",
                                        progress.current, progress.total
                                    )
                                } else {
                                    "Collecting objects...".to_string()
                                };
                                sp.set_message(msg);
                            }
                        }
                        PushPhase::Packing => {
                            if let Some(ref sp) = phase_spinner {
                                let msg = if progress.total > 0 {
                                    format!(
                                        "Packing... {}/{} objects",
                                        progress.current, progress.total
                                    )
                                } else {
                                    "Generating pack...".to_string()
                                };
                                sp.set_message(msg);
                            }
                        }
                        PushPhase::Uploading => {
                            let mut guard = upload_pb_cb.lock().unwrap_or_else(|e| e.into_inner());
                            if guard.is_none() {
                                // Finish spinner, create bytes progress bar
                                if let Some(ref sp) = phase_spinner {
                                    sp.finish_and_clear();
                                }
                                *guard = Some(tracker.push_bar(progress.total));
                            }
                            if let Some(ref pb) = *guard {
                                // Grow total dynamically as more objects are checked
                                if progress.total > pb.length().unwrap_or(0) {
                                    pb.set_length(progress.total);
                                }
                                // Reset ETA on large jumps (pack seals, object transitions)
                                // so protocol overhead stalls don't produce "eta 231y".
                                let prev = pb.position();
                                if progress.current.saturating_sub(prev) > 1_048_576 {
                                    pb.reset_eta();
                                }
                                pb.set_position(progress.current);
                            }
                        }
                    }
                })
                .await?;

            // Clean up whichever bar is still active
            if let Some(pb) = upload_pb.lock().unwrap_or_else(|e| e.into_inner()).take() {
                pb.finish_with_message("done");
            }

            // Push annotated tag .meta sidecars: write meta content as an ODB blob
            // and record its OID in refs/tag-meta/<name> so clones can retrieve it.
            for ref_pushed in &refs_to_push {
                if let Some(tag_name) = ref_pushed.strip_prefix("refs/tags/") {
                    let meta_path = storage_path
                        .join("refs")
                        .join("tags")
                        .join(format!("{}.meta", tag_name));
                    if meta_path.exists() {
                        if let Ok(meta_bytes) = std::fs::read(&meta_path) {
                            // Store meta as a blob in the ODB
                            match odb.write(ObjectType::Blob, &meta_bytes).await {
                                Ok(meta_oid) => {
                                    // Upload the blob bytes to the server before registering
                                    // the ref — update_refs only records the pointer, it does
                                    // not transfer object data.
                                    if let Ok(meta_raw) = odb.read(&meta_oid).await {
                                        if let Err(e) = client
                                            .upload_loose_object(
                                                meta_oid,
                                                ObjectType::Blob,
                                                &meta_raw,
                                            )
                                            .await
                                        {
                                            tracing::warn!(
                                                "Failed to upload tag meta blob for {}: {}",
                                                tag_name,
                                                e
                                            );
                                        }
                                    }

                                    // Push a ref pointing to this blob OID so server stores it
                                    let meta_ref_name = format!("refs/tag-meta/{}", tag_name);
                                    let remote_meta_oid = remote_refs
                                        .refs
                                        .iter()
                                        .find(|r| r.name == meta_ref_name)
                                        .map(|r| r.oid.clone());
                                    let meta_update = mediagit_protocol::RefUpdate {
                                        name: meta_ref_name,
                                        old_oid: remote_meta_oid,
                                        new_oid: meta_oid.to_hex(),
                                        delete: false,
                                    };
                                    let meta_req = mediagit_protocol::RefUpdateRequest {
                                        updates: vec![meta_update],
                                        force: true,
                                    };
                                    if let Err(e) = client.update_refs(meta_req).await {
                                        tracing::warn!(
                                            "Failed to push tag meta ref for {}: {}",
                                            tag_name,
                                            e
                                        );
                                    }
                                }
                                Err(e) => {
                                    tracing::warn!(
                                        "Failed to store tag meta blob for {}: {}",
                                        tag_name,
                                        e
                                    );
                                }
                            }
                        }
                    }
                }
            }

            // Update operation stats from push stats
            stats.bytes_uploaded = push_stats.bytes_uploaded as u64;
            stats.objects_sent = push_stats.objects_count as u64;

            if !result.success {
                let errors: Vec<_> = result
                    .results
                    .iter()
                    .filter(|r| !r.success)
                    .filter_map(|r| r.error.as_ref().map(|e| format!("{}: {}", r.ref_name, e)))
                    .collect();
                anyhow::bail!("Push failed: {}", errors.join(", "));
            }

            // Track which branches are new (didn't exist on remote before this push)
            let mut new_branches: Vec<String> = Vec::new();

            if !self.quiet {
                println!("{} Push successful!", style("✓").green().bold());

                for res in result.results {
                    if res.success {
                        // Find the corresponding update to show old->new
                        if let Some(update) = updates.iter().find(|u| u.name == res.ref_name) {
                            if let Some(ref old) = update.old_oid {
                                println!(
                                    "  {} {} {} → {}",
                                    style("✓").green(),
                                    res.ref_name,
                                    &old[..8],
                                    &update.new_oid[..8]
                                );
                            } else {
                                // Track new branches for auto-upstream
                                if res.ref_name.starts_with("refs/heads/") {
                                    let branch_name = res
                                        .ref_name
                                        .strip_prefix("refs/heads/")
                                        .unwrap_or(&res.ref_name);
                                    new_branches.push(branch_name.to_string());
                                }
                                println!(
                                    "  {} {} (new) → {}",
                                    style("*").green(),
                                    res.ref_name,
                                    &update.new_oid[..8]
                                );
                            }
                        } else {
                            println!("  {} {}", style("✓").green(), res.ref_name);
                        }
                    }
                }
                if skipped_uptodate > 0 {
                    println!(
                        "  {} {} refs already up to date",
                        style("ℹ").blue(),
                        skipped_uptodate
                    );
                }
            } else {
                // Even in quiet mode, we need to track new branches for upstream
                for update in &updates {
                    if update.old_oid.is_none() && update.name.starts_with("refs/heads/") {
                        let branch_name = update
                            .name
                            .strip_prefix("refs/heads/")
                            .unwrap_or(&update.name);
                        new_branches.push(branch_name.to_string());
                    }
                }
            }

            // Create/update tracking refs for pushed branches (refs/remotes/origin/branch)
            // This ensures `branch list -r` shows pushed branches in the original repo
            for update in &updates {
                if update.name.starts_with("refs/heads/") {
                    let branch_name = update
                        .name
                        .strip_prefix("refs/heads/")
                        .unwrap_or(&update.name);
                    let tracking_ref_name = format!("refs/remotes/{}/{}", remote, branch_name);

                    if let Ok(oid) = mediagit_versioning::Oid::from_hex(&update.new_oid) {
                        let tracking_ref =
                            mediagit_versioning::Ref::new_direct(tracking_ref_name.clone(), oid);
                        if let Err(e) = refdb.write(&tracking_ref).await {
                            if self.verbose {
                                println!(
                                    "  Warning: Failed to update tracking ref {}: {}",
                                    tracking_ref_name, e
                                );
                            }
                        } else if self.verbose {
                            println!("  Updated tracking ref: {}", tracking_ref_name);
                        }
                    }
                }
            }

            // Auto-setup upstream tracking for default branches, or when -u is explicitly used
            // Non-default branches without -u are blocked before push, so they won't reach here
            let should_process_upstream = self.set_upstream || !new_branches.is_empty();

            if should_process_upstream && !refs_to_push.is_empty() {
                let mut config = config; // Make mutable
                let mut any_upstream_set = false;

                for ref_to_push in &refs_to_push {
                    // Extract branch name from full ref (e.g., "refs/heads/main" -> "main")
                    let branch_name = ref_to_push
                        .strip_prefix("refs/heads/")
                        .unwrap_or(ref_to_push);

                    // Check if this is a new branch
                    let is_new_branch = new_branches.contains(&branch_name.to_string());
                    let has_upstream = config.get_branch_upstream(branch_name).is_some();

                    // Determine if this is a default branch (main or master)
                    let is_default_branch = branch_name == "main" || branch_name == "master";

                    // Set upstream if:
                    // 1. -u flag explicitly requested, OR
                    // 2. New DEFAULT branch (main/master) without existing upstream
                    if self.set_upstream || (is_new_branch && !has_upstream && is_default_branch) {
                        config.set_branch_upstream(branch_name, remote, ref_to_push.clone());
                        any_upstream_set = true;

                        if !self.quiet {
                            println!(
                                "{} Branch '{}' set up to track '{}/{}'",
                                style("ℹ").blue(),
                                branch_name,
                                remote,
                                branch_name
                            );
                        }
                    }
                }

                // Save the updated config only if we made changes
                if any_upstream_set {
                    config.save(&repo_root)?;
                }
            }
        } else if !self.quiet {
            println!("{} Would push {} refs:", style("ℹ").blue(), updates.len());
            for update in &updates {
                if let Some(ref old) = update.old_oid {
                    println!("  {} {} → {}", update.name, &old[..8], &update.new_oid[..8]);
                } else {
                    println!("  {} (new) → {}", update.name, &update.new_oid[..8]);
                }
            }
            if skipped_uptodate > 0 {
                println!(
                    "  {} {} refs already up to date",
                    style("ℹ").blue(),
                    skipped_uptodate
                );
            }
            println!("{} Dry run complete (no changes made)", style("ℹ").blue());
        }

        // Print operation summary
        stats.duration_ms = start_time.elapsed().as_millis() as u64;
        if !self.quiet && !self.dry_run {
            println!("{} {}", style("📊").cyan(), stats.summary());
        }

        // Save stats for later retrieval by stats command
        if !self.dry_run {
            if let Err(e) = stats.save(&storage_path) {
                tracing::warn!("Failed to save operation stats: {}", e);
            }
        }

        Ok(())
    }
}
