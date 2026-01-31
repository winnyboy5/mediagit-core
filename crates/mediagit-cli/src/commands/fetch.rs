//! Fetch remote changes without merging.
//!
//! The `fetch` command downloads objects and refs from a remote repository
//! without integrating them into the local branches.

use anyhow::Result;
use clap::Parser;
use console::style;
use mediagit_storage::LocalBackend;
use mediagit_versioning::{ObjectDatabase, RefDatabase, Ref};
use std::sync::Arc;
use std::time::Instant;
use crate::progress::{ProgressTracker, OperationStats};
use super::super::repo::find_repo_root;

/// Fetch changes from a remote repository
///
/// Downloads objects and refs from a remote repository and updates
/// remote tracking refs (refs/remotes/<remote>/<branch>). Does not
/// modify local branches or working directory.
#[derive(Parser, Debug)]
#[command(after_help = "EXAMPLES:
    # Fetch all branches from origin
    mediagit fetch

    # Fetch from a specific remote
    mediagit fetch upstream

    # Fetch a specific branch
    mediagit fetch origin main

    # Fetch all branches explicitly
    mediagit fetch --all

SEE ALSO:
    mediagit-pull(1), mediagit-push(1), mediagit-clone(1)")]
pub struct FetchCmd {
    /// Remote name (defaults to origin)
    #[arg(value_name = "REMOTE")]
    pub remote: Option<String>,

    /// Branch to fetch (fetches all branches by default)
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
        let mut stats = OperationStats::new();
        let progress = ProgressTracker::new(self.quiet);

        let remote = self.remote.as_deref().unwrap_or("origin");

        // Find repository root
        let repo_root = find_repo_root()?;
        let storage_path = repo_root.join(".mediagit");
        let storage: Arc<dyn mediagit_storage::StorageBackend> =
            Arc::new(LocalBackend::new(&storage_path).await?);
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
            .get_remote_url(remote)
            .map_err(|e| anyhow::anyhow!("{}", e))?;

        if self.verbose {
            println!("  Remote URL: {}", remote_url);
        }

        // Initialize protocol client and ODB
        let client = mediagit_protocol::ProtocolClient::new(remote_url);
        let odb = Arc::new(
            ObjectDatabase::with_smart_compression(Arc::clone(&storage), 1000)
        );

        // Get remote refs
        let fetch_spinner = progress.spinner("Fetching remote refs...");
        let remote_refs = client.get_refs().await?;
        fetch_spinner.finish_with_message("Remote refs fetched");

        // Filter to branches (refs/heads/*)
        let remote_branches: Vec<_> = remote_refs.refs.iter()
            .filter(|r| r.name.starts_with("refs/heads/"))
            .collect();

        if self.verbose {
            println!("  Found {} remote branches", remote_branches.len());
        }

        // Determine which branches to fetch
        let branches_to_fetch: Vec<_> = if let Some(branch) = &self.branch {
            let full_ref = mediagit_versioning::normalize_ref_name(branch);
            remote_branches.iter()
                .filter(|r| r.name == full_ref)
                .copied()
                .collect()
        } else {
            // Fetch all branches
            remote_branches
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

        let mut branches_updated = 0;
        let mut branches_uptodate = 0;

        // Fetch each branch
        for branch_ref in &branches_to_fetch {
            let branch_name = branch_ref.name.strip_prefix("refs/heads/")
                .unwrap_or(&branch_ref.name);
            let tracking_ref_name = format!("refs/remotes/{}/{}", remote, branch_name);

            // Check if tracking ref is already up to date
            let needs_update = match refdb.read(&tracking_ref_name).await {
                Ok(existing) => {
                    existing.oid.map(|o| o.to_hex()) != Some(branch_ref.oid.clone())
                }
                Err(_) => true, // Doesn't exist, needs update
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

            // Get local "have" list for incremental fetch
            let local_have: Vec<String> = refdb.read(&tracking_ref_name).await
                .ok()
                .and_then(|r| r.oid)
                .map(|oid| vec![oid.to_hex()])
                .unwrap_or_default();

            // Download objects for this branch
            let download_pb = progress.download_bar(&format!("Fetching {}", branch_name));
            let (pack_data, chunked_oids) = client.pull_with_have(&odb, &branch_ref.name, local_have).await?;
            let pack_size = pack_data.len() as u64;
            download_pb.set_length(pack_size);
            download_pb.set_position(pack_size);
            stats.bytes_downloaded += pack_size;
            download_pb.finish_with_message("Downloaded");

            // Unpack objects
            if pack_size > 0 {
                let pack_reader = mediagit_versioning::PackReader::new(pack_data)?;
                let objects = pack_reader.list_objects();
                stats.objects_received += objects.len() as u64;

                for oid in objects.iter() {
                    let (obj_type, obj_data) = pack_reader.get_object_with_type(oid)?;
                    odb.write(obj_type, &obj_data).await?;
                }
            }

            // Download chunked objects if any
            if !chunked_oids.is_empty() {
                let chunks_downloaded = client.download_chunked_objects(&odb, &chunked_oids, |_, _, _| {}).await?;
                if self.verbose {
                    println!("    Downloaded {} chunks", chunks_downloaded);
                }
            }

            // Update remote tracking ref
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

        // Prune stale tracking refs if requested
        if self.prune {
            let stale_count = self.prune_stale_refs(&refdb, remote, &branches_to_fetch).await?;
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
                println!(
                    "\n{} All branches up to date",
                    style("✓").green()
                );
            }
            println!("{} {}", style("📊").cyan(), stats.summary());
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
            
            let exists_on_remote = remote_branches.iter()
                .any(|r| r.name == remote_ref_name);
            
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
