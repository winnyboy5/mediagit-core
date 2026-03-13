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
use std::path::PathBuf;
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
        let config_content = format!(
            r#"[remotes.origin]
url = "{}"
"#,
            self.url
        );
        std::fs::write(storage_path.join("config.toml"), config_content)?;

        // Step 4: Initialize storage and fetch
        init_spinner.set_message("Connecting to remote...");
        let storage = create_storage_backend(&target_dir).await?;
        let odb = Arc::new(ObjectDatabase::with_smart_compression(
            Arc::clone(&storage),
            1000,
        ));
        let refdb = RefDatabase::new(&storage_path);

        // Initialize protocol client
        let client = mediagit_protocol::ProtocolClient::new(self.url.clone());

        // Step 5: Get remote refs
        init_spinner.set_message("Fetching remote refs...");
        let remote_refs = client.get_refs().await?;
        init_spinner.finish_with_message("Connected");
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
            // Total is unknown until Phase 1 (manifest download) completes;
            // set_length is called on the first progress callback.
            let chunk_pb = progress.object_bar("Downloading large files", 0);

            let chunk_pb_ref = chunk_pb.clone();
            let chunks_downloaded = client
                .download_chunked_objects(&odb, &chunked_oids, move |current, total, msg| {
                    chunk_pb_ref.set_length(total as u64);
                    chunk_pb_ref.set_position(current as u64);
                    chunk_pb_ref.set_message(msg.to_string());
                })
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

        // Step 8b: Create tracking refs for all remote branches (LAZY CLONE)
        // We only download objects for the default branch. Other branches' objects
        // will be fetched on-demand when user runs `pull origin branch` or `branch switch`.
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

        Ok(())
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
