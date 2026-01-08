//! Clone a remote repository.
//!
//! The `clone` command creates a copy of an existing remote repository.

use anyhow::{Context, Result};
use clap::Parser;
use console::style;
use indicatif::{ProgressBar, ProgressStyle};
use mediagit_storage::LocalBackend;
use mediagit_versioning::{
    CheckoutManager, ObjectDatabase, RefDatabase,
};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

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

        // Create progress bar
        let progress = if !self.quiet {
            let pb = ProgressBar::new_spinner();
            pb.set_style(
                ProgressStyle::default_spinner()
                    .template("{spinner:.green} {msg}")
                    .unwrap(),
            );
            pb.enable_steady_tick(Duration::from_millis(100));
            Some(pb)
        } else {
            None
        };

        // Step 1: Create directory
        if let Some(ref pb) = progress {
            pb.set_message("Creating directory...");
        }
        std::fs::create_dir_all(&target_dir)
            .context("Failed to create target directory")?;

        // Step 2: Initialize repository
        if let Some(ref pb) = progress {
            pb.set_message("Initializing repository...");
        }
        let storage_path = target_dir.join(".mediagit");
        std::fs::create_dir_all(&storage_path)?;
        std::fs::create_dir_all(storage_path.join("objects"))?;
        std::fs::create_dir_all(storage_path.join("refs").join("heads"))?;
        std::fs::create_dir_all(storage_path.join("refs").join("tags"))?;

        // Create HEAD pointing to main branch
        let head_content = format!("ref: refs/heads/{}\n", branch);
        std::fs::write(storage_path.join("HEAD"), head_content)?;

        // Step 3: Configure remote
        if let Some(ref pb) = progress {
            pb.set_message("Configuring remote...");
        }
        let config_content = format!(
            r#"[remotes.origin]
url = "{}"
"#,
            self.url
        );
        std::fs::write(storage_path.join("config.toml"), config_content)?;

        // Step 4: Initialize storage and fetch
        if let Some(ref pb) = progress {
            pb.set_message("Connecting to remote...");
        }
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
        if let Some(ref pb) = progress {
            pb.set_message("Fetching remote refs...");
        }
        let remote_refs = client.get_refs().await?;
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
        if let Some(ref pb) = progress {
            pb.set_message("Receiving objects...");
        }
        let pack_data = client.pull(&odb, &remote_ref_name).await?;
        let pack_size = pack_data.len();

        if self.verbose {
            println!("  Received {} bytes", pack_size);
        }

        // Step 7: Unpack objects
        if let Some(ref pb) = progress {
            pb.set_message("Unpacking objects...");
        }
        let pack_reader = mediagit_versioning::PackReader::new(pack_data)?;
        let objects = pack_reader.list_objects();
        let object_count = objects.len();

        for oid in &objects {
            // Use get_object_with_type to preserve the correct object type
            let (obj_type, obj_data) = pack_reader.get_object_with_type(oid)?;
            let written_oid = odb.write(obj_type, &obj_data).await?;
            
            // Verify OID matches (for debugging)
            if self.verbose && written_oid != *oid {
                println!("  Warning: OID mismatch for {}: expected {}, got {}", 
                    &oid.to_hex()[..8], &oid.to_hex()[..8], &written_oid.to_hex()[..8]);
            }
        }

        if self.verbose {
            println!("  Unpacked {} objects", object_count);
        }

        // Step 8: Update refs
        if let Some(ref pb) = progress {
            pb.set_message("Updating refs...");
        }
        let remote_oid = mediagit_versioning::Oid::from_hex(&remote_ref.oid)
            .map_err(|e| anyhow::anyhow!("Invalid remote OID: {}", e))?;
        let ref_update = mediagit_versioning::Ref::new_direct(remote_ref_name.clone(), remote_oid);
        refdb.write(&ref_update).await?;

        // Step 9: Checkout working directory
        if let Some(ref pb) = progress {
            pb.set_message("Checking out files...");
        }
        let checkout_mgr = CheckoutManager::new(&odb, &target_dir);
        let files_count = checkout_mgr.checkout_commit(&remote_oid).await?;

        if self.verbose {
            println!("  Checked out {} files", files_count);
        }

        // Finish progress
        if let Some(pb) = progress {
            pb.finish_and_clear();
        }

        // Summary
        let duration = start_time.elapsed();
        if !self.quiet {
            println!(
                "{} Cloned into '{}' ({} objects, {} files) in {:.1}s",
                style("✅").green().bold(),
                target_dir.display(),
                object_count,
                files_count,
                duration.as_secs_f64()
            );
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
