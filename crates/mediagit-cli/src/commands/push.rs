use anyhow::{Context, Result};
use clap::Parser;
use console::style;
use indicatif::{ProgressBar, ProgressStyle};
use mediagit_protocol::PushPhase;
use mediagit_storage::LocalBackend;
use mediagit_versioning::RefDatabase;
use std::sync::Arc;
use std::time::{Duration, Instant};
use crate::progress::OperationStats;

/// Update remote references and send objects
///
/// Pushes local commits to a remote repository, updating the remote
/// references to point to the new commits. This makes your local changes
/// available to others.
#[derive(Parser, Debug)]
#[command(after_help = "EXAMPLES:
    # Push current branch to origin
    mediagit push

    # Push specific branch to origin
    mediagit push origin main

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

    /// Quiet mode
    #[arg(short, long)]
    pub quiet: bool,

    /// Verbose mode
    #[arg(short, long)]
    pub verbose: bool,
}

impl PushCmd {
    pub async fn execute(&self) -> Result<()> {
        let start_time = Instant::now();
        let mut stats = OperationStats::new();

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
            .get_remote_url(remote)
            .map_err(|e| anyhow::anyhow!("{}", e))?;

        if self.verbose {
            println!("  Remote URL: {}", remote_url);
        }

        // Initialize protocol client
        let client = mediagit_protocol::ProtocolClient::new(remote_url);

        // Initialize ODB with smart compression for consistent read/write
        let odb =
            mediagit_versioning::ObjectDatabase::with_smart_compression(Arc::clone(&storage), 1000);

        // Determine which refs to push
        let ref_to_push = if self.refspec.is_empty() {
            // Default: push current branch (reuse head from above)
            head.target.ok_or_else(|| {
                anyhow::anyhow!("HEAD is detached, please specify a refspec")
            })?
        } else {
            // Use first refspec - normalize to handle both short and full ref names
            mediagit_versioning::normalize_ref_name(&self.refspec[0])
        };

        // Read local ref OID
        let local_ref = refdb.read(&ref_to_push).await?;
        let local_oid = local_ref
            .oid
            .ok_or_else(|| anyhow::anyhow!("Ref '{}' has no OID", ref_to_push))?;

        // Get remote refs to check current state
        let remote_refs = client.get_refs().await?;
        let remote_oid = remote_refs
            .refs
            .iter()
            .find(|r| r.name == ref_to_push)
            .map(|r| r.oid.clone());

        // Create ref update (convert Oid to hex string)
        let local_oid_str = local_oid.to_hex();

        // Check if already up-to-date (avoid redundant push)
        if let Some(ref remote) = remote_oid {
            if remote == &local_oid_str {
                if !self.quiet {
                    println!("{} Already up to date", style("✓").green());
                    println!("  {} {}", style("→").cyan(), &local_oid_str[..8]);
                }
                return Ok(());
            }
        }

        let update = mediagit_protocol::RefUpdate {
            name: ref_to_push.clone(),
            old_oid: remote_oid.clone(),
            new_oid: local_oid_str.clone(),
        };

        if !self.dry_run {
            // Create progress bar for push
            let pb = if !self.quiet {
                let pb = ProgressBar::new(100);
                pb.set_style(
                    ProgressStyle::default_bar()
                        .template("{spinner:.green} [{bar:40.cyan/blue}] {msg}")
                        .unwrap()
                        .progress_chars("█▓░"),
                );
                pb.enable_steady_tick(Duration::from_millis(100));
                Some(pb)
            } else {
                None
            };

            // Push with progress callback
            let (result, push_stats) = client.push_with_progress(
                &odb,
                vec![update],
                self.force,
                |progress| {
                    if let Some(ref pb) = pb {
                        let (percent, msg) = match progress.phase {
                            PushPhase::Collecting => {
                                if progress.total > 0 {
                                    let pct = (progress.current * 100 / progress.total) as u64;
                                    (pct.min(30), format!("Collecting... {}/{} objects", progress.current, progress.total))
                                } else {
                                    (10, format!("Collecting objects..."))
                                }
                            }
                            PushPhase::Packing => {
                                if progress.total > 0 {
                                    let pct = 30 + (progress.current * 40 / progress.total) as u64;
                                    (pct.min(70), format!("Packing... {}/{} objects", progress.current, progress.total))
                                } else {
                                    (50, format!("Packing..."))
                                }
                            }
                            PushPhase::Uploading => {
                                if progress.total > 0 {
                                    let pct = 70 + (progress.current * 30 / progress.total) as u64;
                                    let bytes_str = if progress.total > 1024 * 1024 {
                                        format!("{:.1} MiB", progress.total as f64 / (1024.0 * 1024.0))
                                    } else if progress.total > 1024 {
                                        format!("{:.1} KiB", progress.total as f64 / 1024.0)
                                    } else {
                                        format!("{} B", progress.total)
                                    };
                                    (pct.min(100), format!("Uploading... {}", bytes_str))
                                } else {
                                    (90, format!("Uploading..."))
                                }
                            }
                        };
                        pb.set_position(percent);
                        pb.set_message(msg);
                    }
                },
            ).await?;

            // Finish progress bar
            if let Some(pb) = pb {
                pb.finish_and_clear();
            }

            // Update operation stats from push stats
            stats.bytes_uploaded = push_stats.bytes_uploaded as u64;
            stats.objects_sent = push_stats.objects_count as u64;

            if !result.success {
                anyhow::bail!(
                    "Push failed: {}",
                    result
                        .results
                        .iter()
                        .filter_map(|r| r.error.as_ref())
                        .next()
                        .unwrap_or(&"unknown error".to_string())
                );
            }

            if !self.quiet {
                // Show what was updated
                if let Some(ref remote) = remote_oid {
                    println!(
                        "{} Updated {} → {}",
                        style("→").cyan(),
                        &remote[..8],
                        &local_oid_str[..8]
                    );
                } else {
                    println!(
                        "{} Created {} at {}",
                        style("*").green(),
                        ref_to_push,
                        &local_oid_str[..8]
                    );
                }

                println!("{} Push successful!", style("✓").green().bold());
                for res in result.results {
                    if res.success {
                        println!("  {} {}", style("✓").green(), res.ref_name);
                    }
                }
            }
        } else {
            if !self.quiet {
                if let Some(ref remote) = remote_oid {
                    println!(
                        "{} Would update {} → {}",
                        style("→").cyan(),
                        &remote[..8],
                        &local_oid_str[..8]
                    );
                }
                println!(
                    "{} Dry run complete (no changes made)",
                    style("ℹ").blue()
                );
            }
        }

        // Print operation summary
        stats.duration_ms = start_time.elapsed().as_millis() as u64;
        if !self.quiet && !self.dry_run {
            println!("{} {}", style("📊").cyan(), stats.summary());
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
