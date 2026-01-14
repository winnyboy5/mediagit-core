use anyhow::{Context, Result};
use chrono::Utc;
use clap::Parser;
use console::style;
use mediagit_storage::LocalBackend;
use mediagit_versioning::{Commit, MergeStrategy, RefDatabase, Signature};
use std::sync::Arc;
use std::time::Instant;
use crate::progress::{ProgressTracker, OperationStats};

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

    # Continue pull after resolving conflicts
    mediagit pull --continue

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

    /// Merge strategy
    #[arg(short = 's', long, value_name = "STRATEGY")]
    pub strategy: Option<String>,

    /// Merge option
    #[arg(short = 'X', long, value_name = "OPTION")]
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
    #[arg(long)]
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
        let mut stats = OperationStats::new();
        let progress = ProgressTracker::new(self.quiet);

        let remote = self.remote.as_deref().unwrap_or("origin");

        // Validate repository
        let repo_root = self.find_repo_root()?;
        let storage_path = repo_root.join(".mediagit");
        let storage: Arc<dyn mediagit_storage::StorageBackend> =
            Arc::new(LocalBackend::new(&storage_path).await?);
        let refdb = RefDatabase::new(&storage_path);

        if self.dry_run {
            if !self.quiet {
                println!("{} Running in dry-run mode", style("ℹ").blue());
            }
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
            .get_remote_url(remote)
            .map_err(|e| anyhow::anyhow!("{}", e))?;

        if self.verbose {
            println!("  Remote URL: {}", remote_url);
        }

        // Initialize protocol client
        let client = mediagit_protocol::ProtocolClient::new(remote_url);

        // Initialize ODB with smart compression for consistent read/write
        let odb = Arc::new(
            mediagit_versioning::ObjectDatabase::with_smart_compression(Arc::clone(&storage), 1000)
        );

        // Determine remote ref to pull
        let remote_ref = if let Some(branch) = &self.branch {
            // Normalize branch name to handle both short and full ref paths
            mediagit_versioning::normalize_ref_name(branch)
        } else {
            // Default: pull tracking branch for current HEAD (reuse head from above)
            head.target.ok_or_else(|| {
                anyhow::anyhow!("HEAD is detached, please specify a branch")
            })?
        };

        if self.verbose {
            println!("  Pulling ref: {}", remote_ref);
        }

        // Get current local ref state BEFORE downloading
        let local_ref = refdb.read(&remote_ref).await.ok();

        // Get remote refs to check current state
        let remote_refs_check = client.get_refs().await?;
        let remote_oid_check = remote_refs_check
            .refs
            .iter()
            .find(|r| r.name == remote_ref)
            .map(|r| r.oid.clone());

        // Check if already synchronized (avoid redundant pull)
        if let (Some(local), Some(remote)) = (local_ref.as_ref().and_then(|r| r.oid.as_ref()), remote_oid_check.as_ref()) {
            let local_oid_str = local.to_hex();
            if &local_oid_str == remote {
                if !self.quiet {
                    println!("{} Already up to date", style("✓").green());
                    println!("  {} {}", style("→").cyan(), &remote[..8]);
                }
                return Ok(());
            }
        }

        if !self.dry_run {
            // Pull using protocol client (downloads pack file)
            let download_pb = progress.download_bar("Receiving objects");
            let (pack_data, chunked_oids) = client.pull(&odb, &remote_ref).await?;
            let pack_size = pack_data.len() as u64;

            download_pb.set_length(pack_size);
            download_pb.set_position(pack_size);
            stats.bytes_downloaded = pack_size;

            if !self.quiet {
                if chunked_oids.is_empty() {
                    println!(
                        "{} Received {} bytes",
                        style("↓").cyan(),
                        pack_size
                    );
                } else {
                    println!(
                        "{} Received {} bytes pack + {} chunked objects",
                        style("↓").cyan(),
                        pack_size,
                        chunked_oids.len()
                    );
                }
            }
            download_pb.finish_with_message("Download complete");

            // Unpack received objects (non-chunked)
            let pack_reader = mediagit_versioning::PackReader::new(pack_data)?;
            let objects = pack_reader.list_objects();
            let object_count = objects.len() as u64;

            let unpack_pb = progress.object_bar("Unpacking objects", object_count);
            for (idx, oid) in objects.iter().enumerate() {
                // Use get_object_with_type to preserve the correct object type
                let (obj_type, obj_data) = pack_reader.get_object_with_type(oid)?;
                odb.write(obj_type, &obj_data).await?;
                unpack_pb.set_position((idx + 1) as u64);
            }
            stats.objects_received = object_count;

            if !self.quiet {
                println!("{} Unpacked objects", style("✓").green());
            }
            unpack_pb.finish_with_message("Unpack complete");

            // Download chunked objects (large files)
            if !chunked_oids.is_empty() {
                let chunk_pb = progress.object_bar("Downloading large files", chunked_oids.len() as u64);
                
                // Download with simple logging callback (no progress bar in closure)
                let chunks_downloaded = client.download_chunked_objects(&odb, &chunked_oids, |_current, _total, _msg| {
                    // Progress tracking handled outside closure
                }).await?;

                chunk_pb.finish_with_message("Download complete");

                if !self.quiet {
                    println!(
                        "{} Downloaded {} chunks for {} large files",
                        style("✓").green(),
                        chunks_downloaded,
                        chunked_oids.len()
                    );
                }
            }

            // Get remote ref OID
            let remote_refs = client.get_refs().await?;
            let remote_oid = remote_refs
                .refs
                .iter()
                .find(|r| r.name == remote_ref)
                .ok_or_else(|| anyhow::anyhow!("Remote ref '{}' not found", remote_ref))?
                .oid
                .clone();

            // Update local ref to match remote
            let remote_oid_parsed = mediagit_versioning::Oid::from_hex(&remote_oid)
                .map_err(|e| anyhow::anyhow!("Invalid remote OID: {}", e))?;
            let ref_update = mediagit_versioning::Ref::new_direct(remote_ref.clone(), remote_oid_parsed);
            refdb.write(&ref_update).await?;

            if !self.quiet {
                println!(
                    "{} Updated {} to {}",
                    style("✓").green(),
                    remote_ref,
                    &remote_oid[..8]
                );
            }

            // Integrate changes (merge or rebase)
            if self.rebase {
                // TODO: Implement rebase integration
                if !self.quiet {
                    println!(
                        "{} Rebase integration not yet implemented",
                        style("⚠").yellow()
                    );
                    println!("  Use merge strategy instead (default)");
                }
            } else {
                // Merge integration
                let head = refdb.read("HEAD").await?;
                if let Some(head_oid) = head.oid {
                    let merge_engine =
                        mediagit_versioning::MergeEngine::new(Arc::clone(&odb));

                    if self.verbose {
                        let head_hex = head_oid.to_hex();
                        println!("  Merging {} into {}", &remote_oid[..8], &head_hex[..8]);
                    }

                    // Parse remote OID
                    let remote_oid_parsed = mediagit_versioning::Oid::from_hex(&remote_oid)
                        .map_err(|e| anyhow::anyhow!("Invalid remote OID: {}", e))?;

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
                            let target_ref = mediagit_versioning::Ref::new_direct(target.clone(), commit_oid);
                            refdb.write(&target_ref).await?;
                        } else {
                            // HEAD is detached, update HEAD directly
                            let head_ref = mediagit_versioning::Ref::new_direct("HEAD".to_string(), commit_oid);
                            refdb.write(&head_ref).await?;
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
                } else {
                    // No local commits, just update HEAD (fast-forward)
                    let remote_oid_parsed = mediagit_versioning::Oid::from_hex(&remote_oid)
                        .map_err(|e| anyhow::anyhow!("Invalid remote OID: {}", e))?;

                    if let Some(target) = &head.target {
                        // HEAD is symbolic, update the target branch
                        let target_ref = mediagit_versioning::Ref::new_direct(target.clone(), remote_oid_parsed);
                        refdb.write(&target_ref).await?;
                    } else {
                        // HEAD is detached, update HEAD directly
                        let head_ref = mediagit_versioning::Ref::new_direct("HEAD".to_string(), remote_oid_parsed);
                        refdb.write(&head_ref).await?;
                    }

                    if !self.quiet {
                        println!(
                            "{} Fast-forwarded to {}",
                            style("✓").green().bold(),
                            &remote_oid[..8]
                        );
                    }
                }
            }
        } else {
            if !self.quiet {
                println!(
                    "{} Dry run complete (no changes made)",
                    style("ℹ").blue()
                );
            }
        }

        // Print operation summary
        stats.duration_ms = start_time.elapsed().as_millis() as u64;
        if !self.quiet && !self.dry_run {
            println!("\n{} {}", style("📊").cyan(), stats.summary());
        }

        Ok(())
    }

    fn find_repo_root(&self) -> Result<std::path::PathBuf> {
        let mut current = std::env::current_dir()?;

        loop {
            if current.join(".mediagit").exists() {
                return Ok(current);
            }

            if !current.pop() {
                anyhow::bail!("Not a mediagit repository");
            }
        }
    }
}
