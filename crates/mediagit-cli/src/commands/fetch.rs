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

//! Fetch remote changes without merging.
//!
//! The `fetch` command downloads objects and refs from a remote repository
//! without integrating them into the local branches.

use super::super::repo::{collect_local_have, create_storage_backend, find_repo_root};
use crate::progress::{OperationStats, ProgressTracker};
use anyhow::Result;
use clap::Parser;
use console::style;
use futures::StreamExt;
use mediagit_versioning::{ObjectDatabase, Ref, RefDatabase};
use std::path::Path;
use std::sync::Arc;
use std::time::Instant;

/// Fetch changes from a remote repository
///
/// Downloads objects and refs from a remote repository and updates
/// remote tracking refs (refs/remotes/<remote>/<branch>). Does not
/// modify local branches or working directory.
///
/// By default, fetches only the current branch from the remote.
#[derive(Parser, Debug)]
#[command(after_help = "EXAMPLES:
    # Fetch current branch from origin (default)
    mediagit fetch

    # Fetch from a specific remote
    mediagit fetch upstream

    # Fetch a specific branch
    mediagit fetch origin main

    # Fetch all branches in parallel (up to MEDIAGIT_FETCH_BRANCH_CONCURRENCY=4)
    mediagit fetch --all

SEE ALSO:
    mediagit-pull(1), mediagit-push(1), mediagit-clone(1)")]
pub struct FetchCmd {
    /// Remote name (defaults to origin)
    #[arg(value_name = "REMOTE")]
    pub remote: Option<String>,

    /// Branch to fetch (fetches current branch by default; use --all for all branches)
    #[arg(value_name = "BRANCH")]
    pub branch: Option<String>,

    /// Fetch all branches from the remote
    #[arg(long)]
    pub all: bool,

    /// Prune remote tracking refs that no longer exist on remote
    #[arg(short, long)]
    pub prune: bool,

    /// Quiet mode
    #[arg(short, long)]
    pub quiet: bool,

    /// Verbose mode
    #[arg(short, long)]
    pub verbose: bool,
}

impl FetchCmd {
    pub async fn execute(&self) -> Result<()> {
        let start_time = Instant::now();
        let mut stats = OperationStats::for_operation("fetch");
        let progress = ProgressTracker::new(self.quiet);

        let remote = self.remote.as_deref().unwrap_or("origin");

        // Find repository root
        let repo_root = find_repo_root()?;
        let storage_path = repo_root.join(".mediagit");
        let storage = create_storage_backend(&repo_root).await?;
        let refdb = RefDatabase::new(&storage_path);

        if !self.quiet {
            println!(
                "{} Fetching from {}...",
                style("📥").cyan().bold(),
                style(remote).yellow()
            );
        }

        // Load config to get remote URL
        let config = mediagit_config::Config::load(&repo_root).await?;
        let remote_url = config
            .resolve_remote_url(remote)
            .map_err(|e| anyhow::anyhow!("{}", e))?;

        if self.verbose {
            println!("  Remote URL: {}", remote_url);
        }

        // Initialize protocol client and ODB. Honour [performance]
        // upload_concurrency from the repo config so users can tune parallel
        // chunk fan-out without setting MEDIAGIT_UPLOAD_CONCURRENCY in the env.
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
        // Arc-wrap for the parallel --all path (pull_streaming/download_chunked_objects take &self).
        let mut client = Arc::new(build_client(credentials.clone()));
        let odb = Arc::new(ObjectDatabase::with_smart_compression(
            Arc::clone(&storage),
            1000,
        ));

        // Get remote refs
        let fetch_spinner = progress.spinner("Fetching remote refs...");
        // First authenticated call of this command — a cached keychain
        // credential may have expired; on a 401, invalidate it and retry
        // once with the next tier (I11).
        let remote_refs = match client.get_refs().await {
            Ok(r) => r,
            Err(e) if crate::repo::invalidate_on_unauthorized(&config, remote, cred_source, &e) => {
                credentials = crate::repo::resolve_credentials(&repo_root, &config, remote);
                client = Arc::new(build_client(credentials.clone()));
                client.get_refs().await?
            }
            Err(e) => return Err(e),
        };
        crate::repo::remember_credentials(&config, remote, &credentials);
        fetch_spinner.finish_with_message("Remote refs fetched");

        // Compute the full local have-set ONCE for this fetch (moved ahead of
        // the branch-fetch machinery below so tag-fetching can reuse it too).
        // The server expands these OIDs into a full object closure and prunes
        // anything already on the client from the pack walk.
        let local_have = collect_local_have(&refdb, &odb).await;
        if self.verbose {
            println!("  Local have-set: {} refs", local_have.len());
        }

        // Tags auto-fetch like git — tag objects are tiny, so there's no
        // --tags flag to opt in. Runs unconditionally (even if no branches
        // need updating below) so a plain `fetch` always picks up new tags.
        // Shared with `clone` (see fetch_tags below). A tag's target commit
        // (or an annotated tag's .meta blob) may not be reachable from any
        // branch we fetch below, so fetch_tags downloads its own object
        // closure for anything missing from the local ODB.
        let tags_updated = fetch_tags(
            &refdb,
            &odb,
            &storage_path,
            &remote_refs.refs,
            &client,
            &local_have,
            self.verbose,
        )
        .await?;
        if tags_updated > 0 && !self.quiet {
            println!("  {} Fetched {} tag(s)", style("🏷").cyan(), tags_updated);
        }

        // Filter to branches (refs/heads/*)
        let remote_branches: Vec<_> = remote_refs
            .refs
            .iter()
            .filter(|r| r.name.starts_with("refs/heads/"))
            .collect();

        if self.verbose {
            println!("  Found {} remote branches", remote_branches.len());
        }

        // Determine which branches to fetch.
        //
        // Default: current branch only — avoids unintentional multi-TB parallel
        // download on a plain `mediagit fetch` in a large media repo.
        // Use --all to explicitly fetch all branches in parallel.
        let branches_to_fetch: Vec<_> = if let Some(branch) = &self.branch {
            let full_ref = mediagit_versioning::normalize_ref_name(branch);
            remote_branches
                .iter()
                .filter(|r| r.name == full_ref)
                .copied()
                .collect()
        } else if self.all {
            remote_branches.clone()
        } else {
            // Default: resolve HEAD symref → current branch ref name.
            let current_ref = refdb
                .read("HEAD")
                .await
                .ok()
                .and_then(|h| h.target)
                .unwrap_or_else(|| "refs/heads/main".to_string());
            remote_branches
                .iter()
                .filter(|r| r.name == current_ref)
                .copied()
                .collect()
        };

        if branches_to_fetch.is_empty() {
            if let Some(branch) = &self.branch {
                anyhow::bail!("Branch '{}' not found on remote", branch);
            } else {
                if !self.quiet {
                    println!("{} No branches to fetch", style("ℹ").blue());
                }
                return Ok(());
            }
        }

        // Create refs/remotes/<remote>/ directory if needed
        let remotes_dir = storage_path.join("refs").join("remotes").join(remote);
        std::fs::create_dir_all(&remotes_dir)?;

        // Max parallel branch fetches for --all. Default 4.
        // Set MEDIAGIT_FETCH_BRANCH_CONCURRENCY=1 to force sequential.
        let branch_concurrency: usize = std::env::var("MEDIAGIT_FETCH_BRANCH_CONCURRENCY")
            .ok()
            .and_then(|v| {
                v.parse().ok().or_else(|| {
                    tracing::warn!("MEDIAGIT_FETCH_BRANCH_CONCURRENCY='{}' is not a valid usize, using default 4", v);
                    None
                })
            })
            .unwrap_or(4)
            .max(1);

        let mut branches_updated = 0;
        let mut branches_uptodate = 0;

        if self.all && branches_to_fetch.len() > 1 && branch_concurrency > 1 {
            // === Parallel --all path ===
            // Phase 1: determine which branches need fetching (sequential — cheap local I/O).
            // Phase 2: fan-out network fetch across branches concurrently.
            // Phase 3: serialize ref writes (ref-write barrier — preserves ODB consistency).
            //
            // Large-file safety: per-branch chunk downloads are bounded by
            // MEDIAGIT_DOWNLOAD_CONCURRENCY; B4 (MEDIAGIT_STREAM_CHUNK_TO_DISK) prevents
            // RAM blowup when fetching multi-GB branches in parallel.

            struct PendingFetch {
                ref_name: String,
                oid_str: String,
                tracking_ref_name: String,
                branch_name: String,
            }

            let mut pending: Vec<PendingFetch> = Vec::new();
            for branch_ref in &branches_to_fetch {
                let branch_name = branch_ref
                    .name
                    .strip_prefix("refs/heads/")
                    .unwrap_or(&branch_ref.name)
                    .to_string();
                let tracking_ref_name = format!("refs/remotes/{}/{}", remote, branch_name);
                // BUG-008: OID match alone is insufficient — check ODB existence too.
                let needs_update = match refdb.read(&tracking_ref_name).await {
                    Ok(existing) => {
                        let oids_match =
                            existing.oid.map(|o| o.to_hex()) == Some(branch_ref.oid.clone());
                        if oids_match {
                            match mediagit_versioning::Oid::from_hex(&branch_ref.oid) {
                                Ok(oid) => !odb.exists(&oid).await.unwrap_or(false),
                                Err(_) => true,
                            }
                        } else {
                            true
                        }
                    }
                    Err(_) => true,
                };
                if needs_update {
                    pending.push(PendingFetch {
                        ref_name: branch_ref.name.clone(),
                        oid_str: branch_ref.oid.clone(),
                        tracking_ref_name,
                        branch_name,
                    });
                } else {
                    branches_uptodate += 1;
                    if self.verbose {
                        println!("  {} is up to date", branch_name);
                    }
                }
            }

            struct FetchResult {
                tracking_ref_name: String,
                remote_oid: mediagit_versioning::Oid,
                branch_name: String,
                oid_short: String,
            }

            if !self.quiet && !pending.is_empty() {
                println!(
                    "  Fetching {} branch(es) in parallel (concurrency {})",
                    pending.len(),
                    branch_concurrency
                );
            }

            // Cap per-branch download concurrency so total in-flight stays bounded.
            // Without a divisor, branch_concurrency × MEDIAGIT_DOWNLOAD_CONCURRENCY ×
            // MEDIAGIT_RANGE_PARALLEL = 4 × 32 × 4 = 512 simultaneous TCP requests —
            // same pool-exhaustion shape as the old B2 push bug. At defaults: 4 × 8 × 4 = 128.
            // Single-branch path (branch_concurrency=1): cap = max(32/1,8)=32, identical to today.
            // Override: MEDIAGIT_FETCH_DOWNLOAD_CONCURRENCY.
            let base_dl_concurrency: usize = std::env::var("MEDIAGIT_DOWNLOAD_CONCURRENCY")
                .ok()
                .and_then(|s| {
                    s.parse().ok().or_else(|| {
                        tracing::warn!("MEDIAGIT_DOWNLOAD_CONCURRENCY='{}' is not a valid usize, using default 32", s);
                        None
                    })
                })
                .filter(|n: &usize| *n > 0)
                .unwrap_or(32);
            let download_per_branch: usize = std::env::var("MEDIAGIT_FETCH_DOWNLOAD_CONCURRENCY")
                .ok()
                .and_then(|s| {
                    s.parse::<usize>().ok().or_else(|| {
                        tracing::warn!("MEDIAGIT_FETCH_DOWNLOAD_CONCURRENCY='{}' is not a valid usize, using computed default", s);
                        None
                    })
                })
                .filter(|n| *n > 0)
                .unwrap_or_else(|| (base_dl_concurrency / branch_concurrency).max(8));

            let fetch_results: Vec<anyhow::Result<FetchResult>> = futures::stream::iter(pending)
                .map(|p| {
                    let client = Arc::new(client.with_download_cap(download_per_branch));
                    let odb = Arc::clone(&odb);
                    let local_have = local_have.clone();
                    // Box::pin avoids large_futures lint; state machine lives on heap.
                    Box::pin(async move {
                        let chunked_oids =
                            client.pull_streaming(&odb, &p.ref_name, local_have).await?;
                        if !chunked_oids.is_empty() {
                            client
                                .download_chunked_objects(&odb, &chunked_oids, |_, _, _| {})
                                .await?;
                        }
                        let remote_oid = mediagit_versioning::Oid::from_hex(&p.oid_str)
                            .map_err(|e| anyhow::anyhow!("Invalid remote OID: {}", e))?;
                        let oid_short = p.oid_str.get(..8).unwrap_or(&p.oid_str).to_string();
                        Ok::<FetchResult, anyhow::Error>(FetchResult {
                            tracking_ref_name: p.tracking_ref_name,
                            remote_oid,
                            branch_name: p.branch_name,
                            oid_short,
                        })
                    })
                })
                .buffer_unordered(branch_concurrency)
                .collect()
                .await;

            for result in fetch_results {
                let f = result?;
                let tracking_ref = Ref::new_direct(f.tracking_ref_name, f.remote_oid);
                refdb.write(&tracking_ref).await?;
                branches_updated += 1;
                if !self.quiet {
                    println!(
                        "  {} {} -> {}",
                        style("→").cyan(),
                        f.branch_name,
                        f.oid_short
                    );
                }
            }
        } else {
            // === Sequential path: single branch, or MEDIAGIT_FETCH_BRANCH_CONCURRENCY=1 ===
            for branch_ref in &branches_to_fetch {
                let branch_name = branch_ref
                    .name
                    .strip_prefix("refs/heads/")
                    .unwrap_or(&branch_ref.name);
                let tracking_ref_name = format!("refs/remotes/{}/{}", remote, branch_name);

                // Check if tracking ref is already up to date.
                //
                // BUG-008: a matching tracking-ref OID is NOT sufficient — clone
                // pre-populates tracking refs for every advertised branch but
                // only ships objects reachable from the default branch, so
                // `refs/remotes/origin/feat-*` can point at commits that are
                // not yet in the local ODB. Without this check, a subsequent
                // `fetch origin feat-x` would short-circuit on the matching
                // OID and never download the missing objects, leaving the
                // user unable to check out the branch.
                //
                // We use `ObjectDatabase::exists()` (cache-first + storage
                // existence probe) so this adds one cheap I/O per branch
                // without ever loading object content into memory.
                let needs_update = match refdb.read(&tracking_ref_name).await {
                    Ok(existing) => {
                        let oids_match =
                            existing.oid.map(|o| o.to_hex()) == Some(branch_ref.oid.clone());
                        if oids_match {
                            match mediagit_versioning::Oid::from_hex(&branch_ref.oid) {
                                Ok(oid) => !odb.exists(&oid).await.unwrap_or(false),
                                Err(_) => true,
                            }
                        } else {
                            true
                        }
                    }
                    Err(_) => true,
                };

                if !needs_update {
                    branches_uptodate += 1;
                    if self.verbose {
                        println!("  {} is up to date", branch_name);
                    }
                    continue;
                }

                if self.verbose {
                    println!("  Fetching {}...", branch_name);
                }

                let download_pb = progress.spinner(&format!("Fetching {}...", branch_name));
                let chunked_oids = client
                    .pull_streaming(&odb, &branch_ref.name, local_have.clone())
                    .await?;
                download_pb.finish_with_message(format!("Fetched {}", branch_name));

                if !chunked_oids.is_empty() {
                    let (chunks_downloaded, chunk_bytes) = client
                        .download_chunked_objects(&odb, &chunked_oids, |_, _, _| {})
                        .await?;
                    // RP-2: fetch keeps its own OperationStats, so it must
                    // credit the same figure pull and clone do — otherwise the
                    // one command whose whole job is transferring data is the
                    // one reporting none.
                    stats.bytes_downloaded += chunk_bytes;
                    if self.verbose {
                        println!("    Downloaded {} chunks", chunks_downloaded);
                    }
                }

                let remote_oid = mediagit_versioning::Oid::from_hex(&branch_ref.oid)
                    .map_err(|e| anyhow::anyhow!("Invalid remote OID: {}", e))?;
                let tracking_ref = Ref::new_direct(tracking_ref_name.clone(), remote_oid);
                refdb.write(&tracking_ref).await?;

                branches_updated += 1;

                if !self.quiet {
                    println!(
                        "  {} {} -> {}",
                        style("→").cyan(),
                        branch_name,
                        &branch_ref.oid[..8]
                    );
                }
            }
        }

        // Prune stale tracking refs if requested.
        // Always compare against ALL remote branches (not just branches_to_fetch),
        // so pruning works correctly when only fetching a subset of branches.
        if self.prune {
            let stale_count = self
                .prune_stale_refs(&refdb, remote, &remote_branches)
                .await?;
            if stale_count > 0 && !self.quiet {
                println!(
                    "{} Pruned {} stale tracking refs",
                    style("🗑").yellow(),
                    stale_count
                );
            }
        }

        // Summary
        stats.duration_ms = start_time.elapsed().as_millis() as u64;
        if !self.quiet {
            if branches_updated > 0 {
                println!(
                    "\n{} Fetched {} branches ({} already up to date)",
                    style("✅").green().bold(),
                    branches_updated,
                    branches_uptodate
                );
            } else {
                println!("\n{} All branches up to date", style("✓").green());
            }
            println!("{} {}", style("📊").cyan(), stats.summary());
        }

        // Save stats for later retrieval by stats command
        if let Err(e) = stats.save(&storage_path) {
            tracing::warn!("Failed to save operation stats: {}", e);
        }

        Ok(())
    }

    /// Prune remote tracking refs that no longer exist on remote
    async fn prune_stale_refs(
        &self,
        refdb: &RefDatabase,
        remote: &str,
        remote_branches: &[&mediagit_protocol::RefInfo],
    ) -> Result<usize> {
        let mut pruned = 0;

        // List all local tracking refs for this remote
        let tracking_refs = refdb.list(&format!("remotes/{}", remote)).await?;

        // Find refs that don't exist on remote
        for tracking_ref in tracking_refs {
            let branch_name = tracking_ref
                .strip_prefix(&format!("refs/remotes/{}/", remote))
                .unwrap_or(&tracking_ref);
            let remote_ref_name = format!("refs/heads/{}", branch_name);

            let exists_on_remote = remote_branches.iter().any(|r| r.name == remote_ref_name);

            if !exists_on_remote {
                if self.verbose {
                    println!("  Pruning stale ref: {}", tracking_ref);
                }
                refdb.delete(&tracking_ref).await?;
                pruned += 1;
            }
        }

        Ok(pruned)
    }
}

/// Write tag refs (`refs/tags/*`) and restore annotated tag `.meta` sidecars
/// (`refs/tag-meta/*`) advertised by the remote into the local ref database.
///
/// Shared by `clone` and `fetch` — tags auto-fetch like git (tag objects are
/// tiny), so there is no `--tags` flag. Idempotent: re-running with an
/// unchanged remote just rewrites the same OIDs/content.
///
/// A tag's target commit isn't necessarily reachable from any branch we
/// fetch elsewhere (e.g. a tag on an unmerged or deleted branch), and an
/// annotated tag's `.meta` blob is never reachable from a commit tree at
/// all — so both are fetched here via their own `want` request whenever
/// they're missing from the local ODB, reusing the same pack machinery
/// `clone`/branch-fetch use.
///
/// Returns the number of `refs/tags/*` refs written.
pub(crate) async fn fetch_tags(
    refdb: &RefDatabase,
    odb: &ObjectDatabase,
    storage_path: &Path,
    remote_refs: &[mediagit_protocol::RefInfo],
    client: &mediagit_protocol::ProtocolClient,
    local_have: &[String],
    verbose: bool,
) -> Result<usize> {
    let mut tag_count = 0;
    // Collect tag-meta refs for a second pass after tag refs are written.
    let mut tag_meta_refs: Vec<(String, String)> = Vec::new();

    for ref_info in remote_refs {
        if ref_info.name.starts_with("refs/tags/") {
            if let Ok(tag_oid) = mediagit_versioning::Oid::from_hex(&ref_info.oid) {
                if !odb.exists(&tag_oid).await.unwrap_or(false) {
                    match client
                        .download_pack_streaming(
                            odb,
                            vec![ref_info.oid.clone()],
                            local_have.to_vec(),
                        )
                        .await
                    {
                        Ok(chunked) if !chunked.is_empty() => {
                            if let Err(e) = client
                                .download_chunked_objects(odb, &chunked, |_, _, _| {})
                                .await
                            {
                                tracing::warn!(
                                    "Failed to download chunked objects for tag {}: {}",
                                    ref_info.name,
                                    e
                                );
                                continue;
                            }
                        }
                        Ok(_) => {}
                        Err(e) => {
                            tracing::warn!(
                                "Failed to fetch objects for tag {}: {}",
                                ref_info.name,
                                e
                            );
                            continue;
                        }
                    }
                }
                let tag_ref = Ref::new_direct(ref_info.name.clone(), tag_oid);
                refdb.write(&tag_ref).await?;
                tag_count += 1;
                if verbose {
                    println!(
                        "  Created tag ref: {} -> {}",
                        ref_info.name,
                        &ref_info.oid[..8]
                    );
                }
            }
        } else if let Some(tag_name) = ref_info.name.strip_prefix("refs/tag-meta/") {
            tag_meta_refs.push((tag_name.to_string(), ref_info.oid.clone()));
        }
    }

    // Restore annotated tag .meta sidecars from ODB blobs.
    for (tag_name, blob_oid_hex) in &tag_meta_refs {
        if let Ok(blob_oid) = mediagit_versioning::Oid::from_hex(blob_oid_hex) {
            // The meta blob is never reachable from a commit tree — it's a
            // floating object referenced only by refs/tag-meta/<name> — so
            // fetch it explicitly rather than assuming pull_streaming above
            // already pulled it in.
            if !odb.exists(&blob_oid).await.unwrap_or(false)
                && let Err(e) = client
                    .download_pack_streaming(odb, vec![blob_oid_hex.clone()], vec![])
                    .await
            {
                tracing::warn!("Failed to download tag meta blob for {}: {}", tag_name, e);
                continue;
            }
            match odb.read(&blob_oid).await {
                Ok(meta_bytes) => {
                    let meta_dir = storage_path.join("refs").join("tags");
                    if let Err(e) = tokio::fs::create_dir_all(&meta_dir).await {
                        tracing::warn!("Failed to create tags dir: {}", e);
                        continue;
                    }
                    let meta_path = meta_dir.join(format!("{}.meta", tag_name));
                    if let Err(e) = tokio::fs::write(&meta_path, &meta_bytes).await {
                        tracing::warn!("Failed to write tag meta for {}: {}", tag_name, e);
                    } else if verbose {
                        println!("  Restored annotated tag meta: {}.meta", tag_name);
                    }
                }
                Err(e) => {
                    tracing::warn!("Failed to read tag meta blob {}: {}", blob_oid_hex, e);
                }
            }
        }
    }

    Ok(tag_count)
}
