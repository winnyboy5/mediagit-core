//! Clone a remote repository.
//!
//! The `clone` command creates a copy of an existing remote repository.

use anyhow::{Context, Result};
use clap::Parser;
use console::style;
use crate::progress::{ProgressTracker, OperationStats};
use mediagit_storage::LocalBackend;
use mediagit_versioning::{
    CheckoutManager, ObjectDatabase, RefDatabase,
};
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
            anyhow::bail!(
                "Destination path '{}' already exists",
                target_dir.display()
            );
        }

        // Create progress tracker and stats (matching pull.rs pattern)
        let mut stats = OperationStats::new();
        let progress = ProgressTracker::new(self.quiet);

        // Step 1: Create directory
        let init_spinner = progress.spinner("Creating directory...");
        std::fs::create_dir_all(&target_dir)
            .context("Failed to create target directory")?;

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
        let storage: Arc<dyn mediagit_storage::StorageBackend> =
            Arc::new(LocalBackend::new(&storage_path).await?);
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
            println!("  Remote ref: {} -> {}", remote_ref.name, &remote_ref.oid[..8]);
        }

        // Step 6: Pull objects
        let download_pb = progress.download_bar("Receiving objects");
        let (pack_data, chunked_oids) = client.pull(&odb, &remote_ref_name).await?;
        let pack_size = pack_data.len() as u64;
        download_pb.set_length(pack_size);
        download_pb.set_position(pack_size);
        stats.bytes_downloaded = pack_size;
        download_pb.finish_with_message("Download complete");

        if self.verbose {
            println!("  Received {} bytes pack, {} chunked objects", pack_size, chunked_oids.len());
        }

        // Step 7: Unpack objects (non-chunked)
        let pack_reader = mediagit_versioning::PackReader::new(pack_data)?;
        let objects = pack_reader.list_objects();
        let object_count = objects.len();
        stats.objects_received = object_count as u64;

        let unpack_pb = progress.object_bar("Unpacking objects", object_count as u64);
        for (idx, oid) in objects.iter().enumerate() {
            // Use get_object_with_type to preserve the correct object type
            let (obj_type, obj_data) = pack_reader.get_object_with_type(oid)?;
            let written_oid = odb.write(obj_type, &obj_data).await?;
            unpack_pb.set_position((idx + 1) as u64);

            // Verify OID matches - mismatch indicates data corruption
            if written_oid != *oid {
                anyhow::bail!(
                    "Data integrity error: OID mismatch for object {}: expected {}, computed {}. \
                     This indicates data corruption during transfer.",
                    &oid.to_hex()[..12],
                    oid.to_hex(),
                    written_oid.to_hex()
                );
            }
        }
        unpack_pb.finish_with_message("Unpack complete");

        if self.verbose {
            println!("  Unpacked {} objects", object_count);
        }

        // Step 7b: Download chunked objects (large files)
        if !chunked_oids.is_empty() {
            let chunk_pb = progress.object_bar("Downloading large files", chunked_oids.len() as u64);

            let chunks_downloaded = client.download_chunked_objects(&odb, &chunked_oids, |_current, _total, _msg| {
                // Progress tracking handled by chunk_pb
            }).await?;

            chunk_pb.finish_with_message("Download complete");

            if self.verbose {
                println!("  Downloaded {} chunks for {} large files", chunks_downloaded, chunked_oids.len());
            }
        }

        // Step 8: Update refs
        let remote_oid = mediagit_versioning::Oid::from_hex(&remote_ref.oid)
            .map_err(|e| anyhow::anyhow!("Invalid remote OID: {}", e))?;
        let ref_update = mediagit_versioning::Ref::new_direct(remote_ref_name.clone(), remote_oid);
        refdb.write(&ref_update).await?;

        // Step 8b: Download objects for all remote branches and create tracking refs
        for ref_info in &remote_refs.refs {
            if ref_info.name.starts_with("refs/heads/") {
                // Use unwrap_or to safely handle refs that don't have the expected prefix
                let branch_name = ref_info.name.strip_prefix("refs/heads/")
                    .unwrap_or(&ref_info.name);
                let tracking_ref_name = format!("refs/remotes/origin/{}", branch_name);

                // Skip the branch we already downloaded (the default checkout branch)
                if ref_info.name == remote_ref_name {
                    // Just create the tracking ref for the default branch (objects already downloaded)
                    if let Ok(tracking_oid) = mediagit_versioning::Oid::from_hex(&ref_info.oid) {
                        let tracking_ref = mediagit_versioning::Ref::new_direct(
                            tracking_ref_name.clone(),
                            tracking_oid,
                        );
                        refdb.write(&tracking_ref).await?;

                        if self.verbose {
                            println!("  Created tracking ref: {} -> {}", tracking_ref_name, &ref_info.oid[..8]);
                        }
                    }
                    continue;
                }

                // For other branches, download objects first
                if self.verbose {
                    println!("  Downloading objects for branch: {}", branch_name);
                }

                // Download objects for this branch
                let (branch_pack_data, branch_chunked_oids) = client.pull(&odb, &ref_info.name).await?;
                let branch_pack_size = branch_pack_data.len();

                if self.verbose {
                    println!("    Received {} bytes pack, {} chunked objects", branch_pack_size, branch_chunked_oids.len());
                }

                // Unpack objects for this branch
                if branch_pack_size > 0 {
                    let branch_pack_reader = mediagit_versioning::PackReader::new(branch_pack_data)?;
                    let branch_objects = branch_pack_reader.list_objects();

                    for oid in branch_objects.iter() {
                        let (obj_type, obj_data) = branch_pack_reader.get_object_with_type(oid)?;
                        let written_oid = odb.write(obj_type, &obj_data).await?;

                        // Verify OID matches
                        if written_oid != *oid {
                            anyhow::bail!(
                                "Data integrity error: OID mismatch for object {}: expected {}, computed {}",
                                &oid.to_hex()[..12],
                                oid.to_hex(),
                                written_oid.to_hex()
                            );
                        }
                    }

                    if self.verbose {
                        println!("    Unpacked {} objects", branch_objects.len());
                    }
                }

                // Download chunked objects for this branch (large files)
                if !branch_chunked_oids.is_empty() {
                    let chunks_downloaded = client.download_chunked_objects(&odb, &branch_chunked_oids, |_current, _total, _msg| {
                        // Silent progress for additional branches
                    }).await?;

                    if self.verbose {
                        println!("    Downloaded {} chunks for {} large files", chunks_downloaded, branch_chunked_oids.len());
                    }
                }

                // Now create the tracking ref with objects available
                if let Ok(tracking_oid) = mediagit_versioning::Oid::from_hex(&ref_info.oid) {
                    let tracking_ref = mediagit_versioning::Ref::new_direct(
                        tracking_ref_name.clone(),
                        tracking_oid,
                    );
                    refdb.write(&tracking_ref).await?;

                    if self.verbose {
                        println!("  Created tracking ref: {} -> {}", tracking_ref_name, &ref_info.oid[..8]);
                    }
                }
            }
        }


        // Step 9: Checkout working directory
        let checkout_pb = progress.file_bar("Checking out", 0);
        let checkout_mgr = CheckoutManager::new(&odb, &target_dir);
        let files_count = checkout_mgr.checkout_commit(&remote_oid).await?;
        checkout_pb.set_length(files_count as u64);
        checkout_pb.set_position(files_count as u64);
        checkout_pb.finish_with_message("Checkout complete");
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
