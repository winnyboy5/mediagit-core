// SPDX-License-Identifier: BUSL-1.1
// Copyright (C) 2025-2026 Aswin Krishnamoorthy

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
            let checkout_start = std::time::Instant::now();
            let files_count = checkout_mgr.checkout_fresh(&oid).await?;
            mediagit_protocol::bench::emit_checkout_summary(
                checkout_start,
                files_count as u64,
                false,
            );
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

        // C5: an interrupted clone leaves a directory behind. Adopt it and
        // resume — but ONLY if this clone is what created it, which the
        // in-progress marker is the proof of. Anything else still bails.
        let resuming = match read_clone_marker(&target_dir) {
            Some(marker) => {
                if marker.url != self.url || marker.branch != branch {
                    anyhow::bail!(
                        "Destination path '{}' holds an interrupted clone of a DIFFERENT \
                         source ({} @ {}).\n  Remove it, or re-run with the same URL and \
                         branch to resume it.",
                        target_dir.display(),
                        marker.url,
                        marker.branch
                    );
                }
                if !self.quiet {
                    println!(
                        "{} Resuming interrupted clone in '{}' (already-downloaded objects \
                         are skipped)",
                        style("↻").cyan().bold(),
                        target_dir.display()
                    );
                }
                true
            }
            None => {
                if target_dir.exists() {
                    anyhow::bail!("Destination path '{}' already exists", target_dir.display());
                }
                false
            }
        };

        // Create progress tracker and stats (matching pull.rs pattern)
        let mut stats = OperationStats::for_operation("clone");
        let progress = ProgressTracker::new(self.quiet);

        // Step 1: Create directory
        let init_spinner = progress.spinner("Creating directory...");
        std::fs::create_dir_all(&target_dir).context("Failed to create target directory")?;

        // C5: has anything worth keeping landed yet?
        //
        // Set just before bulk transfer starts — i.e. after the URL, the
        // credentials and the repo's existence have all been validated by
        // `get_refs`. A failure BEFORE this point is a setup failure with no
        // downloaded data behind it, and wiping is still the right answer
        // (a bad URL should not leave a stub directory the next attempt then
        // refuses). A failure AFTER it may have gigabytes behind it.
        let data_phase = std::sync::atomic::AtomicBool::new(false);

        // Steps 2-9 can all fail (network, refs, checkout) after target_dir
        // has been created. Wrap them so a failure with nothing behind it
        // cleans up the partial clone directory before the error propagates
        // (NOTE-RM-3). We always own target_dir here — either the exists()
        // check above bailed, or the marker proves this command created it.
        let clone_result: Result<()> = async {
        // Step 2: Initialize repository
        init_spinner.set_message("Initializing repository...");
        let storage_path = target_dir.join(".mediagit");
        // On resume the control directory is already built. Re-running Step 3
        // in particular would mint a NEW repo_id and namespace into
        // config.toml, orphaning every object already downloaded under the old
        // one — so the whole of setup is skipped, not just the parts that
        // would error.
        if !resuming {
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

        // C5: from here on, a leftover directory is a resumable clone rather
        // than debris. Written after config.toml so an adopted directory is
        // always one that got at least as far as being a valid repository.
        write_clone_marker(&storage_path, &self.url, branch)?;
        } // end `if !resuming`

        // Step 4: Initialize storage and fetch. `create_storage_backend`
        // performs the LAYOUT marker check/write itself (one of the two
        // production wrap-points), keyed off the repo_id just written above.
        init_spinner.set_message("Connecting to remote...");

        // F3: a `clone` run *inside* an encrypted repository would seal the new
        // repository's objects under the outer repo's process key while the new
        // one gets no key file of its own — permanently unopenable.
        //
        // The guard normally rides along inside `create_storage_backend`, but
        // that now happens after the escrow round trip (the ODB must capture
        // the cloned repo's key at construction). Run it here so the refusal
        // still comes before any network call: a guard the user only reaches
        // after a connection error is not a guard.
        // PHASE MARKERS (`MEDIAGIT_LOG=debug`), from here to the first byte of
        // bulk transfer.
        //
        // Clone has hung four times with the process alive, CPU flat across two
        // samples, every thread in Wait, and NOTHING in the log after the last
        // completed request. Twice the server had seen zero requests; twice it
        // had seen encryption-key + info/refs and nothing since. A single fixed
        // code location cannot explain both, and there is currently no evidence
        // that distinguishes them - the log is silent through this entire
        // stretch, so every investigation so far has had to guess which step
        // blocked.
        //
        // These markers make the next occurrence name the step it died in. That
        // is precisely how the A7 backend-outage hang - open for two weeks and
        // two campaigns - was solved in one run. `debug!` costs nothing when the
        // level is off, so this stays out of normal output.
        //
        // To arm them for a hang hunt, ALWAYS keep the global level in the
        // filter - measured on one clone, 8 markers either way:
        //
        //   MEDIAGIT_LOG=warn,mediagit::commands::clone=debug ..  28 stderr lines
        //   MEDIAGIT_LOG=debug ................................. 253 stderr lines
        //
        // A campaign runs thousands of clones, so that 9x matters.
        //
        // The leading `warn,` is NOT optional, and omitting it is not a style
        // choice - it silently breaks other things. An EnvFilter built from
        // target-only directives disables every target that does not match, so
        // `MEDIAGIT_LOG=mediagit::commands::clone=debug` suppresses the whole
        // rest of the tree, including `mediagit_protocol`'s
        // "rate limited (429); backing off before retry" warning.
        //
        // That is not hypothetical: arming exactly that filter across campaign
        // ga21 made 07_ratelimit's RL6 drill fail. RL6 greps the client's output
        // for "429" to prove throttling actually happened before it will credit
        // recovery - a deliberately anti-vacuous check - so silencing the
        // warning made a healthy limiter look like it never fired. The
        // instrument changed what it was measuring.
        //
        // Measured, not assumed: RUST_LOG="tower_http=debug" yields 0 WARN lines
        // from other targets; RUST_LOG="warn,tower_http=debug" yields 3.
        tracing::debug!(phase = "key-armed", "clone: local key/scope resolved");
        crate::encryption::install_armed_key()?;
        mediagit_compression::ensure_key_scope(&target_dir)?;

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
        tracing::debug!(phase = "resolving-credentials", "clone: resolving credentials");
        let (mut credentials, cred_source) =
            crate::repo::resolve_credentials_tiered(&target_dir, &clone_config, "origin");
        let mut client = mediagit_protocol::ProtocolClient::new(self.url.clone())
            .with_credentials(credentials.clone());

        // Step 5: Get remote refs
        init_spinner.set_message("Fetching remote refs...");
        // First authenticated call of this command — a cached keychain
        // credential (e.g. from a prior `auth login`) may have expired; on a
        // 401, invalidate it and retry once with the next tier (I11).
        tracing::debug!(phase = "get-refs", "clone: requesting info/refs");
        let remote_refs = match client.get_refs().await {
            Ok(r) => r,
            Err(e)
                if crate::repo::invalidate_on_unauthorized(
                    &clone_config,
                    "origin",
                    cred_source,
                    &e,
                ) =>
            {
                credentials =
                    crate::repo::resolve_credentials(&target_dir, &clone_config, "origin");
                client = mediagit_protocol::ProtocolClient::new(self.url.clone())
                    .with_credentials(credentials.clone());
                client.get_refs().await?
            }
            Err(e) => return Err(e),
        };
        crate::repo::remember_credentials(&clone_config, "origin", &credentials);
        init_spinner.finish_with_message("Connected");

        // DC-7/D4: if the remote holds an encryption key for this repository,
        // adopt it before a single object is written.
        //
        // Everything about to be downloaded is sealed under it, and the ODB
        // built below captures the key at construction — so this has to happen
        // first or the clone writes objects it cannot read and reports success.
        // The local master comes from the same precedence `key init` uses
        // (keyfile, keychain, then a keychain entry provisioned on the spot),
        // so a clone never prompts.
        //
        // 404 means the remote does not do escrow, which is every remote that
        // is not serving encrypted repositories: nothing to adopt, carry on.
        tracing::debug!(phase = "get-encryption-key", "clone: refs received; asking for escrowed key");
        if let mediagit_protocol::client::escrow::EscrowedKey::Present(key) =
            client.get_encryption_key().await?
        {
            let source = crate::encryption::adopt_repo_key(&target_dir, &key)?;
            let repo_key = mediagit_security::encryption::EncryptionKey::from_bytes(key.to_vec())
                .map_err(|e| anyhow::anyhow!("the remote's encryption key is unusable: {e}"))?;
            mediagit_compression::set_process_key(&target_dir, repo_key)
                .context("installing the cloned repository's at-rest encryption key")?;
            if !self.quiet {
                println!(
                    "{} This repository is encrypted; its key is now held by {}",
                    style("🔑").cyan(),
                    style(source.describe()).yellow()
                );
            }
        }

        tracing::debug!(phase = "create-storage", "clone: building storage backend");
        let storage = create_storage_backend(&target_dir).await?;
        let odb = Arc::new(ObjectDatabase::with_smart_compression(
            Arc::clone(&storage),
            1000,
        ));

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
            // The freshly-written clone config already carries a top-level
            // `cdc_seed = 0` line; replace its value in place. Prepending a
            // second `cdc_seed` key produced invalid TOML (duplicate key) and
            // broke every clone of a seeded repo.
            let mut replaced = false;
            let mut lines: Vec<String> = existing
                .lines()
                .map(|l| {
                    if !replaced && l.trim_start().starts_with("cdc_seed") && l.contains('=') {
                        replaced = true;
                        format!("cdc_seed = {seed}")
                    } else {
                        l.to_string()
                    }
                })
                .collect();
            if !replaced {
                lines.insert(0, format!("cdc_seed = {seed}"));
            }
            std::fs::write(&config_path, lines.join("\n") + "\n")?;
        }

        tracing::debug!(phase = "cdc-seed-written", "clone: config written; selecting ref");
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
        tracing::debug!(phase = "pull-streaming", "clone: starting bulk transfer");
        // Past setup: `get_refs` above has already proved the URL, the
        // credentials and the repo. Anything that fails from here has data
        // behind it worth resuming from.
        data_phase.store(true, std::sync::atomic::Ordering::SeqCst);
        let chunked_oids = client
            .pull_streaming(&odb, &remote_ref_name, vec![])
            .await?;
        tracing::debug!(phase = "pull-complete", "clone: bulk transfer finished");
        download_pb.finish_with_message("Received objects");

        if self.verbose {
            println!(
                "  Received objects via streaming, {} chunked objects",
                chunked_oids.len()
            );
        }

        let remote_oid = mediagit_versioning::Oid::from_hex(&remote_ref.oid)
            .map_err(|e| anyhow::anyhow!("Invalid remote OID: {}", e))?;

        // C4: overlap the working-tree writes we can already do with the chunk
        // downloads we still have to wait for.
        //
        // `pull_streaming` has just written every non-chunked object to the
        // ODB, so every small blob in the tree is ALREADY complete — only
        // chunked media is still in flight. Checkout used to sit behind a hard
        // barrier waiting for all of it anyway.
        //
        // `MEDIAGIT_CLONE_OVERLAP=0` restores the old serial path. That is both
        // the revert switch and the parity oracle the clone-parity test diffs
        // against, so it must stay reachable.
        let checkout_mgr = CheckoutManager::new(&odb, &target_dir);
        let overlap_enabled = std::env::var("MEDIAGIT_CLONE_OVERLAP").as_deref() != Ok("0");
        let split = if overlap_enabled {
            let deferred_oids: std::collections::HashSet<_> =
                chunked_oids.iter().copied().collect();
            Some(checkout_mgr.plan_fresh(&remote_oid, &deferred_oids).await?)
        } else {
            None
        };
        let (ready_entries, deferred_entries) = match split {
            Some(plan) => (Some(plan.ready), Some(plan.deferred)),
            None => (None, None),
        };
        let mut files_written = 0usize;

        // Step 7: Download chunked objects (large files)
        if !chunked_oids.is_empty() {
            // Total bytes seeded from manifests in Phase 1 via first on_progress call.
            let chunk_pb = progress.download_bar("Downloading large files", 0);

            let chunk_pb_ref = chunk_pb.clone();
            let download = client.download_chunked_objects(
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
            );

            let (chunks_downloaded, chunk_bytes) = if let Some(ready) = ready_entries {
                let (downloaded, wrote) =
                    tokio::try_join!(download, checkout_mgr.write_entries(ready))?;
                files_written += wrote;
                downloaded
            } else {
                download.await?
            };

            chunk_pb.finish_with_message(format!("Downloaded {} chunks", chunks_downloaded));
            stats.objects_received += chunks_downloaded as u64;
            // RP-2: clone reported no download figure either.
            stats.bytes_downloaded += chunk_bytes;

            if self.verbose {
                println!(
                    "  Downloaded {} chunks for {} large files",
                    chunks_downloaded,
                    chunked_oids.len()
                );
            }
        } else if let Some(ready) = ready_entries {
            // Nothing to overlap with, but the split is still the cheaper
            // path: these entries are written here and skipped at Step 9.
            files_written += checkout_mgr.write_entries(ready).await?;
        }

        // Step 8: Update refs
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
            }
        }

        // Write tag refs (refs/tags/*) and restore annotated tag .meta
        // sidecars (refs/tag-meta/*) received from the server. Shared with
        // `fetch` — see commands::fetch::fetch_tags. Pass the default branch
        // tip as the have-set: a tag pointing at (or behind) it needs no
        // extra download; anything else fetch_tags pulls on its own.
        let clone_have = vec![remote_oid.to_hex()];
        super::fetch::fetch_tags(
            &refdb,
            &odb,
            &storage_path,
            &remote_refs.refs,
            &client,
            &clone_have,
            self.verbose,
        )
        .await?;

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
        // Timed from HERE, not from before the download: this is the wait the
        // user actually still has after the last byte lands. With overlap off
        // it is the whole checkout, matching the C0 baseline; with it on it is
        // the residual tail, which is the number that should collapse.
        let checkout_start = std::time::Instant::now();
        // With overlap on, only the chunked entries are left — the rest were
        // written above, under the download. With it off this is the original
        // whole-tree checkout.
        let files_count = match deferred_entries {
            Some(deferred) => files_written + checkout_mgr.write_entries(deferred).await?,
            None => checkout_mgr.checkout_fresh(&remote_oid).await?,
        };
        mediagit_protocol::bench::emit_checkout_summary(checkout_start, files_count as u64, overlap_enabled);
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

        // C5: split the wipe rule. Deleting on ANY error is what made clone
        // unresumable by construction — an 11 GB clone dying at 90% deleted the
        // very ODB the retry would have skipped work against.
        match (
            clone_result.is_ok(),
            data_phase.load(std::sync::atomic::Ordering::SeqCst),
        ) {
            (true, _) => {
                // Success: the directory is a finished repository, not a clone
                // in progress. Clearing the marker is what stops a later clone
                // into the same path from silently adopting it.
                clear_clone_marker(&target_dir);
            }
            (false, false) => {
                // Setup failure — bad URL, auth, no such repo. Nothing was
                // downloaded, so leaving a stub the next attempt refuses would
                // be strictly worse than cleaning up. Unchanged behaviour.
                let _ = std::fs::remove_dir_all(&target_dir);
                println!("cleaned up partial clone at {}", target_dir.display());
            }
            (false, true) => {
                // Data landed. Keep it: the marker makes this directory
                // adoptable, and re-running skips everything already in the
                // ODB.
                println!(
                    "kept partial clone at {} — re-run the same `clone` command to resume",
                    target_dir.display()
                );
            }
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

/// What an interrupted clone left behind, read back from its marker file.
#[derive(Debug, PartialEq, Eq)]
struct CloneMarker {
    url: String,
    branch: String,
}

/// Name of the in-progress marker, inside the repo's `.mediagit` directory.
///
/// C5: this file is the ONLY thing that makes a leftover directory adoptable.
/// Clone must never resume into a directory it did not create — a user's
/// `~/projects/foo` that happens to share a name with the repo must still be
/// refused — and only `clone` ever writes this name.
const CLONE_MARKER: &str = "CLONE_IN_PROGRESS";

fn clone_marker_path(target_dir: &Path) -> PathBuf {
    target_dir.join(".mediagit").join(CLONE_MARKER)
}

fn write_clone_marker(storage_path: &Path, url: &str, branch: &str) -> Result<()> {
    std::fs::write(
        storage_path.join(CLONE_MARKER),
        format!("url={url}\nbranch={branch}\n"),
    )
    .context("Failed to write clone in-progress marker")
}

/// `None` means "not a resumable clone" — no marker, or one we cannot parse.
///
/// An unreadable or malformed marker degrades to a clean refusal (the caller's
/// "already exists" bail), never to adopting a directory on a guess. Same
/// principle as the pack-verification fallbacks: missing data downgrades the
/// path, it does not relax the check.
fn read_clone_marker(target_dir: &Path) -> Option<CloneMarker> {
    let text = std::fs::read_to_string(clone_marker_path(target_dir)).ok()?;
    let mut url = None;
    let mut branch = None;
    for line in text.lines() {
        if let Some(v) = line.strip_prefix("url=") {
            url = Some(v.to_string());
        } else if let Some(v) = line.strip_prefix("branch=") {
            branch = Some(v.to_string());
        }
    }
    Some(CloneMarker {
        url: url?,
        branch: branch?,
    })
}

fn clear_clone_marker(target_dir: &Path) {
    let _ = std::fs::remove_file(clone_marker_path(target_dir));
}

#[cfg(test)]
mod clone_marker_tests {
    use super::*;

    fn seed(dir: &Path, url: &str, branch: &str) {
        let storage = dir.join(".mediagit");
        std::fs::create_dir_all(&storage).unwrap();
        write_clone_marker(&storage, url, branch).unwrap();
    }

    /// The round trip clone's resume decision is made on.
    #[test]
    fn marker_round_trips() {
        let tmp = tempfile::tempdir().unwrap();
        seed(tmp.path(), "http://host:3000/repo", "main");
        assert_eq!(
            read_clone_marker(tmp.path()),
            Some(CloneMarker {
                url: "http://host:3000/repo".to_string(),
                branch: "main".to_string(),
            })
        );
    }

    /// A directory clone did not create must NOT be adoptable. This is the
    /// invariant that keeps C5 from turning "clone into an existing path" from
    /// a refusal into a silent merge.
    #[test]
    fn a_directory_without_a_marker_is_never_adopted() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(tmp.path().join(".mediagit")).unwrap();
        std::fs::write(tmp.path().join("my-notes.txt"), b"not a clone").unwrap();
        assert_eq!(read_clone_marker(tmp.path()), None);
    }

    /// Success clears it, so a LATER clone into the same path gets the
    /// "already exists" refusal rather than adopting a finished repository and
    /// checking out over the user's working tree.
    #[test]
    fn clearing_the_marker_makes_the_directory_unadoptable_again() {
        let tmp = tempfile::tempdir().unwrap();
        seed(tmp.path(), "http://host:3000/repo", "main");
        assert!(read_clone_marker(tmp.path()).is_some());
        clear_clone_marker(tmp.path());
        assert_eq!(read_clone_marker(tmp.path()), None);
    }

    /// A truncated marker (killed mid-write) must fail closed to "not
    /// resumable", not resume against a half-known source.
    #[test]
    fn a_malformed_marker_fails_closed() {
        let tmp = tempfile::tempdir().unwrap();
        let storage = tmp.path().join(".mediagit");
        std::fs::create_dir_all(&storage).unwrap();
        std::fs::write(storage.join(CLONE_MARKER), b"url=http://host:3000/re").unwrap();
        assert_eq!(
            read_clone_marker(tmp.path()),
            None,
            "a marker missing its branch must not be treated as resumable"
        );
    }
}
