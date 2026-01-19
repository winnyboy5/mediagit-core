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

/// Validate a ref name for safety
/// Ref names must not contain special characters that could cause filesystem issues
fn validate_ref_name(name: &str) -> Result<()> {
    // Empty ref names are invalid
    if name.is_empty() {
        anyhow::bail!("Ref name cannot be empty");
    }

    // Check for prohibited characters that could cause filesystem issues
    // Based on git's ref naming rules
    let prohibited_chars = ['\\', ':', '?', '*', '"', '<', '>', '|', '\0'];
    for c in prohibited_chars {
        if name.contains(c) {
            anyhow::bail!("Ref name '{}' contains prohibited character '{}'", name, c);
        }
    }

    // Check for prohibited patterns
    if name.starts_with('.') || name.ends_with('.') {
        anyhow::bail!("Ref name '{}' cannot start or end with '.'", name);
    }
    if name.starts_with('/') || name.ends_with('/') {
        anyhow::bail!("Ref name '{}' cannot start or end with '/'", name);
    }
    if name.contains("..") {
        anyhow::bail!("Ref name '{}' cannot contain '..'", name);
    }
    if name.contains("//") {
        anyhow::bail!("Ref name '{}' cannot contain consecutive '/'", name);
    }
    if name.ends_with(".lock") {
        anyhow::bail!("Ref name '{}' cannot end with '.lock'", name);
    }
    if name.contains("@{") {
        anyhow::bail!("Ref name '{}' cannot contain '@{{'", name);
    }

    Ok(())
}

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
        let refs_to_push: Vec<String> = if self.all {
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
            let ref_name = head.target.ok_or_else(|| {
                anyhow::anyhow!("HEAD is detached, please specify a refspec")
            })?;
            vec![ref_name]
        } else {
            // Use refspecs - normalize to handle both short and full ref names
            self.refspec.iter()
                .map(|r| mediagit_versioning::normalize_ref_name(r))
                .collect()
        };

        // Get remote refs to check current state
        let remote_refs = client.get_refs().await?;

        // Build list of ref updates, skipping those already up-to-date
        let mut updates = Vec::new();
        let mut skipped_uptodate = 0;

        for ref_to_push in &refs_to_push {
            // Validate ref name before pushing
            validate_ref_name(ref_to_push)?;

            // Read local ref OID
            let local_ref = refdb.read(ref_to_push).await?;
            let local_oid = local_ref
                .oid
                .ok_or_else(|| anyhow::anyhow!("Ref '{}' has no OID", ref_to_push))?;

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
            });
        }

        // If all refs are up-to-date, exit early
        if updates.is_empty() {
            if !self.quiet {
                println!("{} All {} refs already up to date", style("✓").green(), skipped_uptodate);
            }
            return Ok(());
        }

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

            // Push all refs with progress callback
            let (result, push_stats) = client.push_with_progress(
                &odb,
                updates.clone(),
                self.force,
                |progress| {
                    if let Some(ref pb) = pb {
                        let (percent, msg) = match progress.phase {
                            PushPhase::Collecting => {
                                if progress.total > 0 {
                                    let pct = (progress.current * 100 / progress.total) as u64;
                                    (pct.min(30), format!("Collecting... {}/{} objects", progress.current, progress.total))
                                } else {
                                    (10, "Collecting objects...".to_string())
                                }
                            }
                            PushPhase::Packing => {
                                if progress.total > 0 {
                                    let pct = 30 + (progress.current * 40 / progress.total) as u64;
                                    (pct.min(70), format!("Packing... {}/{} objects", progress.current, progress.total))
                                } else {
                                    (50, "Packing...".to_string())
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
                                    (90, "Uploading...".to_string())
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
                let errors: Vec<_> = result
                    .results
                    .iter()
                    .filter(|r| !r.success)
                    .filter_map(|r| r.error.as_ref().map(|e| format!("{}: {}", r.ref_name, e)))
                    .collect();
                anyhow::bail!("Push failed: {}", errors.join(", "));
            }

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
                    println!("  {} {} refs already up to date", style("ℹ").blue(), skipped_uptodate);
                }
            }
        } else {
            if !self.quiet {
                println!("{} Would push {} refs:", style("ℹ").blue(), updates.len());
                for update in &updates {
                    if let Some(ref old) = update.old_oid {
                        println!(
                            "  {} {} → {}",
                            update.name,
                            &old[..8],
                            &update.new_oid[..8]
                        );
                    } else {
                        println!(
                            "  {} (new) → {}",
                            update.name,
                            &update.new_oid[..8]
                        );
                    }
                }
                if skipped_uptodate > 0 {
                    println!("  {} {} refs already up to date", style("ℹ").blue(), skipped_uptodate);
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
