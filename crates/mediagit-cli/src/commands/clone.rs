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

//! Clone a remote repository.
//!
//! The `clone` command creates a copy of an existing remote repository.

use crate::progress::{OperationStats, ProgressTracker};
use crate::repo::create_storage_backend;
use anyhow::{Context, Result};
use clap::Parser;
use console::style;
use mediagit_versioning::{CheckoutManager, ObjectDatabase, RefDatabase};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

/// Clone a repository into a new directory
///
/// Creates a new directory, initializes a MediaGit repository, configures
/// the remote, and pulls all content from the remote repository.
#[derive(Parser, Debug)]
#[command(after_help = "EXAMPLES:
    # Clone a repository
    mediagit clone http://server:3000/my-project

    # Clone into a specific directory
    mediagit clone http://server:3000/my-project my-local-copy

    # Clone with progress info
    mediagit clone --verbose http://server:3000/my-project

SEE ALSO:
    mediagit-init(1), mediagit-pull(1), mediagit-remote(1)")]
pub struct CloneCmd {
    /// Remote repository URL
    #[arg(value_name = "URL")]
    pub url: String,

    /// Directory to clone into (defaults to repository name from URL)
    #[arg(value_name = "DIRECTORY")]
    pub directory: Option<String>,

    /// Branch to checkout (defaults to main)
    #[arg(short, long, value_name = "BRANCH")]
    pub branch: Option<String>,

    /// Quiet mode
    #[arg(short, long)]
    pub quiet: bool,

    /// Verbose mode
    #[arg(short, long)]
    pub verbose: bool,
}

impl CloneCmd {
    pub async fn execute(&self) -> Result<()> {
        // MediaGit's remote clone protocol speaks HTTP(S) only; anything
        // else is treated as a local repository path (a directory
        // containing `.mediagit`).
        if self.url.starts_with("http://") || self.url.starts_with("https://") {
            self.execute_remote().await
        } else {
            self.execute_local().await
        }
    }

    /// Clone from a local MediaGit repository. Copies the `.mediagit`
    /// control directory verbatim (objects incl. namespace dirs, refs,
    /// config — not the staged index, not the source's working files),
    /// fixes up the storage path + `origin` remote in the copied config,
    /// then materializes the working tree via the same `CheckoutManager`
    /// path the remote clone uses.
    async fn execute_local(&self) -> Result<()> {
        let start_time = Instant::now();

        let source_dir = dunce::canonicalize(&self.url)
            .with_context(|| format!("Local clone source not found: '{}'", self.url))?;
        let source_mediagit = source_dir.join(".mediagit");
        if !source_mediagit.is_dir() {
            anyhow::bail!(
                "unsupported remote URL '{}'.\n\n\
                 `mediagit clone` supports http:// and https:// URLs, or a path to an\n\
                 existing local MediaGit repository (a directory containing `.mediagit`).\n\
                 To serve a local repo over HTTP, run:\n    \
                 mediagit-server -c mediagit-server.toml",
                self.url
            );
        }

        let target_dir = match &self.directory {
            Some(dir) => PathBuf::from(dir),
            None => PathBuf::from(source_dir.file_name().ok_or_else(|| {
                anyhow::anyhow!("Could not determine repository name from '{}'", self.url)
            })?),
        };

        if !self.quiet {
            println!(
                "{} Cloning into '{}'...",
                style("📦").cyan().bold(),
                target_dir.display()
            );
        }
        if target_dir.exists() {
            anyhow::bail!("Destination path '{}' already exists", target_dir.display());
        }
        std::fs::create_dir_all(&target_dir).context("Failed to create target directory")?;
        // Canonicalize so the fixed-up `base_path` we write below is
        // absolute (matching how `init` writes it) — a relative path here
        // gets joined onto `repo_root` a second time by
        // `create_inner_storage_backend`, doubling the prefix.
        let target_dir =
            dunce::canonicalize(&target_dir).context("Failed to resolve target directory")?;

        let clone_result: Result<()> = async {
            let storage_path = target_dir.join(".mediagit");
            copy_dir_skip(&source_mediagit, &storage_path, Path::new("index"))
                .context("Failed to copy .mediagit directory")?;

            // The copied config's filesystem `base_path` is still the
            // SOURCE repo's absolute objects path (baked in at `init`
            // time) — repoint it at the target's own copy, and record
            // `origin` as the source we cloned from.
            let mut config = mediagit_config::Config::load(&target_dir)
                .await
                .context("Failed to load copied config")?;
            if let mediagit_config::StorageConfig::FileSystem(ref mut fs) = config.storage {
                fs.base_path = storage_path.join("objects").display().to_string();
            }
            config.remotes.insert(
                "origin".to_string(),
                mediagit_config::RemoteConfig::new(source_dir.display().to_string()),
            );
            config.save(&target_dir)?;

            let storage = create_storage_backend(&target_dir).await?;
            let odb = Arc::new(ObjectDatabase::with_smart_compression(
                Arc::clone(&storage),
                1000,
            ));
            let refdb = RefDatabase::new(&storage_path);

            // Default branch: explicit --branch, else whatever the copied
            // HEAD already points to (the source repo's own default).
            let branch = match &self.branch {
                Some(b) => b.clone(),
                None => refdb
                    .read("HEAD")
                    .await
                    .ok()
                    .and_then(|h| h.target)
                    .and_then(|t| t.strip_prefix("refs/heads/").map(str::to_string))
                    .unwrap_or_else(|| "main".to_string()),
            };
            let ref_name = format!("refs/heads/{}", branch);
            let oid = refdb
                .read(&ref_name)
                .await
                .ok()
                .and_then(|r| r.oid)
                .ok_or_else(|| {
                    anyhow::anyhow!("Branch '{}' not found in source repository", branch)
                })?;
            refdb.update_symbolic("HEAD", &ref_name).await?;

            let progress = ProgressTracker::new(self.quiet);
            let checkout_pb = progress.spinner("Checking out files...");
            let checkout_mgr = CheckoutManager::new(&odb, &target_dir);
            let files_count = checkout_mgr.checkout_fresh(&oid).await?;
            checkout_pb.finish_with_message(format!("Checked out {} files", files_count));

            if !self.quiet {
                println!(
                    "\n{} Cloned into '{}' ({} files, {:.2}s)",
                    style("✅").green().bold(),
                    target_dir.display(),
                    files_count,
                    start_time.elapsed().as_secs_f64()
                );
            }

            let _ = crate::auto_gc::maybe_run(&target_dir, crate::auto_gc::TriggerMode::PostClone)
                .await;
            Ok(())
        }
        .await;

        if clone_result.is_err() {
            let _ = std::fs::remove_dir_all(&target_dir);
            println!("cleaned up partial clone at {}", target_dir.display());
        }
        clone_result
    }

    async fn execute_remote(&self) -> Result<()> {
        let start_time = Instant::now();

        // Determine target directory
        let target_dir = self.get_target_directory()?;
        let branch = self.branch.as_deref().unwrap_or("main");

        if !self.quiet {
            println!(
                "{} Cloning into '{}'...",
                style("📦").cyan().bold(),
                target_dir.display()
            );
        }

        // Check if directory already exists
        if target_dir.exists() {
            anyhow::bail!("Destination path '{}' already exists", target_dir.display());
        }

        // Create progress tracker and stats (matching pull.rs pattern)
        let mut stats = OperationStats::for_operation("clone");
        let progress = ProgressTracker::new(self.quiet);

        // Step 1: Create directory
        let init_spinner = progress.spinner("Creating directory...");
        std::fs::create_dir_all(&target_dir).context("Failed to create target directory")?;

        // Steps 2-9 can all fail (network, refs, checkout) after target_dir
        // has been created. Wrap them so any failure cleans up the partial
        // clone directory before the error propagates (NOTE-RM-3). We always
        // own target_dir here — the exists() check above already bailed if
        // it was there before this clone, so it's safe to remove on failure.
        let clone_result: Result<()> = async {
        // Step 2: Initialize repository
        init_spinner.set_message("Initializing repository...");
        let storage_path = target_dir.join(".mediagit");
        std::fs::create_dir_all(&storage_path)?;
        std::fs::create_dir_all(storage_path.join("objects"))?;
        std::fs::create_dir_all(storage_path.join("refs").join("heads"))?;
        std::fs::create_dir_all(storage_path.join("refs").join("tags"))?;
        std::fs::create_dir_all(storage_path.join("refs").join("remotes").join("origin"))?;

        // Create HEAD pointing to main branch
        let head_content = format!("ref: refs/heads/{}\n", branch);
        std::fs::write(storage_path.join("HEAD"), head_content)?;

        // Step 3: Configure remote
        init_spinner.set_message("Configuring remote...");
        // Layout v2: default namespace = sanitized basename of the clone
        // target directory, matching `init`'s convention.
        let namespace = target_dir
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "repo".to_string());
        let config_content = format!(
            r#"repo_namespace = "{}"
layout_version = {}
repo_id = "{}"

[remotes.origin]
url = "{}"
"#,
            mediagit_storage::sanitize_namespace(&namespace),
            mediagit_config::CURRENT_LAYOUT_VERSION,
            mediagit_storage::generate_repo_id(),
            self.url
        );
        std::fs::write(storage_path.join("config.toml"), config_content)?;

        // Step 4: Initialize storage and fetch. `create_storage_backend`
        // performs the LAYOUT marker check/write itself (one of the two
        // production wrap-points), keyed off the repo_id just written above.
        init_spinner.set_message("Connecting to remote...");
        let storage = create_storage_backend(&target_dir).await?;
        let odb = Arc::new(ObjectDatabase::with_smart_compression(
            Arc::clone(&storage),
            1000,
        ));
        let refdb = RefDatabase::new(&storage_path);

        // Initialize protocol client. config.toml was just written above with
        // an empty `[remotes.origin]` (no token yet — clone predates the
        // repo existing, so per-remote credentials can't be set ahead of
        // time), so credential resolution here effectively falls through to
        // env MEDIAGIT_TOKEN/MEDIAGIT_API_KEY. Also honours env
        // MEDIAGIT_DOWNLOAD_CONCURRENCY to tune download concurrency.
        let clone_config = mediagit_config::Config::load(&target_dir)
            .await
            .unwrap_or_default();
        let client = mediagit_protocol::ProtocolClient::new(self.url.clone()).with_credentials(
            crate::repo::resolve_credentials(&target_dir, &clone_config, "origin"),
        );

        // Step 5: Get remote refs
        init_spinner.set_message("Fetching remote refs...");
        let remote_refs = client.get_refs().await?;
        init_spinner.finish_with_message("Connected");

        // Inherit the remote's CDC seed (if advertised) so this clone produces
        // matching chunk boundaries for better cross-clone dedup. Missing
        // capability just leaves cdc_seed at 0 (legacy) — this only affects
        // dedup ratio, never correctness (chunk storage is content-addressed).
        if let Some(seed) = remote_refs.capabilities.iter().find_map(|c| {
            c.strip_prefix("cdc-seed=")
                .and_then(|v| v.parse::<u64>().ok())
        }) {
            let config_path = storage_path.join("config.toml");
            let existing = std::fs::read_to_string(&config_path).unwrap_or_default();
            std::fs::write(&config_path, format!("cdc_seed = {}\n{}", seed, existing))?;
        }

        let remote_ref_name = format!("refs/heads/{}", branch);
        let remote_ref = remote_refs
            .refs
            .iter()
            .find(|r| r.name == remote_ref_name)
            .ok_or_else(|| anyhow::anyhow!("Remote branch '{}' not found", branch))?;

        if self.verbose {
            println!(
                "  Remote ref: {} -> {}",
                remote_ref.name,
                &remote_ref.oid[..8]
            );
        }

        // Step 6: Pull objects using streaming (memory-efficient)
        // Use spinner: total bytes unknown, pull_streaming has no progress callback
        let download_pb = progress.spinner("Receiving objects...");
        // Use streaming pull to avoid OOM with large files
        let chunked_oids = client
            .pull_streaming(&odb, &remote_ref_name, vec![])
            .await?;
        download_pb.finish_with_message("Received objects");

        if self.verbose {
            println!(
                "  Received objects via streaming, {} chunked objects",
                chunked_oids.len()
            );
        }

        // Step 7: Download chunked objects (large files)
        if !chunked_oids.is_empty() {
            // Total bytes seeded from manifests in Phase 1 via first on_progress call.
            let chunk_pb = progress.download_bar("Downloading large files", 0);

            let chunk_pb_ref = chunk_pb.clone();
            let chunks_downloaded = client
                .download_chunked_objects(
                    &odb,
                    &chunked_oids,
                    move |bytes_done, bytes_total, msg| {
                        if chunk_pb_ref.length() != Some(bytes_total) {
                            chunk_pb_ref.set_length(bytes_total);
                            chunk_pb_ref.reset_eta();
                        }
                        chunk_pb_ref.set_position(bytes_done);
                        chunk_pb_ref.set_message(msg.to_string());
                    },
                )
                .await?;

            chunk_pb.finish_with_message(format!("Downloaded {} chunks", chunks_downloaded));
            stats.objects_received += chunks_downloaded as u64;

            if self.verbose {
                println!(
                    "  Downloaded {} chunks for {} large files",
                    chunks_downloaded,
                    chunked_oids.len()
                );
            }
        }

        // Step 8: Update refs
        let remote_oid = mediagit_versioning::Oid::from_hex(&remote_ref.oid)
            .map_err(|e| anyhow::anyhow!("Invalid remote OID: {}", e))?;
        let ref_update = mediagit_versioning::Ref::new_direct(remote_ref_name.clone(), remote_oid);
        refdb.write(&ref_update).await?;

        // Upstream tracking (M2 plumbing, consumed by `status` in M4): the
        // cloned default branch tracks origin's default branch by construction.
        {
            let mut tracking_config = mediagit_config::Config::load(&target_dir).await?;
            tracking_config.set_branch_upstream(branch, "origin", remote_ref_name.clone());
            tracking_config.save(&target_dir)?;
        }

        // Step 8b: Create tracking refs for all remote branches (LAZY CLONE)
        // We only download objects for the default branch. Other branches' objects
        // will be fetched on-demand when user runs `pull origin branch` or `branch switch`.
        // Also write tag refs (refs/tags/*) received from the server.
        let mut other_branches = Vec::new();
        // Collect tag-meta refs for second pass after ODB objects are available
        let mut tag_meta_refs: Vec<(String, String)> = Vec::new();
        for ref_info in &remote_refs.refs {
            if ref_info.name.starts_with("refs/heads/") {
                let branch_name = ref_info
                    .name
                    .strip_prefix("refs/heads/")
                    .unwrap_or(&ref_info.name);
                let tracking_ref_name = format!("refs/remotes/origin/{}", branch_name);

                // Create tracking ref for this branch (just the reference, not objects)
                if let Ok(tracking_oid) = mediagit_versioning::Oid::from_hex(&ref_info.oid) {
                    let tracking_ref = mediagit_versioning::Ref::new_direct(
                        tracking_ref_name.clone(),
                        tracking_oid,
                    );
                    refdb.write(&tracking_ref).await?;

                    if self.verbose {
                        println!(
                            "  Created tracking ref: {} -> {}",
                            tracking_ref_name,
                            &ref_info.oid[..8]
                        );
                    }

                    // Track other branches for summary
                    if ref_info.name != remote_ref_name {
                        other_branches.push(branch_name.to_string());
                    }
                }
            } else if ref_info.name.starts_with("refs/tags/") {
                // Write tag ref directly
                if let Ok(tag_oid) = mediagit_versioning::Oid::from_hex(&ref_info.oid) {
                    let tag_ref =
                        mediagit_versioning::Ref::new_direct(ref_info.name.clone(), tag_oid);
                    refdb.write(&tag_ref).await?;
                    if self.verbose {
                        println!(
                            "  Created tag ref: {} -> {}",
                            ref_info.name,
                            &ref_info.oid[..8]
                        );
                    }
                }
            } else if let Some(tag_name) = ref_info.name.strip_prefix("refs/tag-meta/") {
                // Record for second pass: blob OID -> .meta file
                tag_meta_refs.push((tag_name.to_string(), ref_info.oid.clone()));
            }
        }

        // Restore annotated tag .meta sidecars from ODB blobs
        for (tag_name, blob_oid_hex) in &tag_meta_refs {
            if let Ok(blob_oid) = mediagit_versioning::Oid::from_hex(blob_oid_hex) {
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
                        } else if self.verbose {
                            println!("  Restored annotated tag meta: {}.meta", tag_name);
                        }
                    }
                    Err(e) => {
                        tracing::warn!("Failed to read tag meta blob {}: {}", blob_oid_hex, e);
                    }
                }
            }
        }

        // Show available branches to user
        if !other_branches.is_empty() && !self.quiet {
            println!(
                "{} {} other branch(es) available: {}",
                style("ℹ").blue(),
                other_branches.len(),
                other_branches.join(", ")
            );
            println!("  Use 'mediagit pull origin <branch>' then 'mediagit branch switch <branch>' to access");
        }

        // Step 9: Checkout working directory
        // Use spinner: file count only known after checkout finishes
        let checkout_pb = progress.spinner("Checking out files...");
        let checkout_mgr = CheckoutManager::new(&odb, &target_dir);
        let files_count = checkout_mgr.checkout_fresh(&remote_oid).await?;
        checkout_pb.finish_with_message(format!("Checked out {} files", files_count));
        stats.files_updated = files_count as u64;

        if self.verbose {
            println!("  Checked out {} files", files_count);
        }

        // Summary with stats
        stats.duration_ms = start_time.elapsed().as_millis() as u64;
        if !self.quiet {
            println!(
                "\n{} Cloned into '{}'",
                style("✅").green().bold(),
                target_dir.display()
            );
            println!("{} {}", style("📊").cyan(), stats.summary());
        }

        // Save stats for later retrieval by stats command
        if let Err(e) = stats.save(&storage_path) {
            tracing::warn!("Failed to save operation stats: {}", e);
        }

        // Best-effort auto-gc: clones can leave stale partial objects from
        // failed transfers; this trims them silently when significant.
        let _ =
            crate::auto_gc::maybe_run(&target_dir, crate::auto_gc::TriggerMode::PostClone).await;

            Ok(())
        }
        .await;

        if clone_result.is_err() {
            let _ = std::fs::remove_dir_all(&target_dir);
            println!("cleaned up partial clone at {}", target_dir.display());
        }
        clone_result
    }

    /// Extract repository name from URL and determine target directory
    fn get_target_directory(&self) -> Result<PathBuf> {
        if let Some(ref dir) = self.directory {
            return Ok(PathBuf::from(dir));
        }

        // Extract name from URL
        // e.g., http://localhost:3000/my-project -> my-project
        // Note: URLs always use forward slashes per RFC 3986, regardless of OS,
        // so rsplit('/') is correct for cross-platform URL parsing.
        let url = self.url.trim_end_matches('/');
        let name = url
            .rsplit('/')
            .next()
            .ok_or_else(|| anyhow::anyhow!("Could not determine repository name from URL"))?;

        if name.is_empty() {
            anyhow::bail!("Could not determine repository name from URL");
        }

        Ok(PathBuf::from(name))
    }
}

/// Recursively copy `src` to `dst`, skipping any entry whose path relative
/// to `src` equals `skip_relative` (used to exclude the source's staged
/// index from a local clone's copied `.mediagit`).
fn copy_dir_skip(src: &Path, dst: &Path, skip_relative: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        if Path::new(&entry.file_name()) == skip_relative {
            continue;
        }
        let dst_path = dst.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_dir_skip(&entry.path(), &dst_path, Path::new(""))?;
        } else {
            std::fs::copy(entry.path(), &dst_path)?;
        }
    }
    Ok(())
}
