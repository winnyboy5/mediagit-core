use anyhow::{Context, Result};
use chrono::Utc;
use clap::Parser;
use console::style;
use mediagit_versioning::{CheckoutManager, Commit, MergeStrategy, RefDatabase, Signature};
use std::sync::Arc;
use std::time::Instant;
use crate::progress::{ProgressTracker, OperationStats};
use super::rebase::RebaseCmd;
use super::super::repo::{find_repo_root, create_storage_backend};

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

    /// Merge strategy (hidden - MediaGit uses binary-aware merge for media files)
    #[arg(short = 's', long, value_name = "STRATEGY", hide = true)]
    pub strategy: Option<String>,

    /// Merge option (hidden - not applicable to binary media files)
    #[arg(short = 'X', long, value_name = "OPTION", hide = true)]
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
        let mut stats = OperationStats::for_operation("pull");
        let progress = ProgressTracker::new(self.quiet);

        let remote = self.remote.as_deref().unwrap_or("origin");

        // Validate repository
        let repo_root = find_repo_root()?;
        let storage_path = repo_root.join(".mediagit");
        let storage = create_storage_backend(&repo_root).await?;
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
        // Clone head.target early since we need it later for branch comparison
        let current_head_target = head.target.clone();
        let remote_ref = if let Some(branch) = &self.branch {
            // Normalize branch name to handle both short and full ref paths
            mediagit_versioning::normalize_ref_name(branch)
        } else {
            // Default: pull tracking branch for current HEAD (reuse head from above)
            current_head_target.clone().ok_or_else(|| {
                anyhow::anyhow!("HEAD is detached, please specify a branch")
            })?
        };

        if self.verbose {
            println!("  Pulling ref: {}", remote_ref);
        }

        // Get current local ref state BEFORE downloading
        let local_ref = refdb.read(&remote_ref).await.ok();

        // ================================================================
        // STEP 1: Fetch ALL remote branch refs and update tracking refs
        // This ensures new branches pushed by other collaborators are visible
        // NOTE: This runs BEFORE the "already up to date" check so users
        // always see new remote branches even when current branch is synced
        // ================================================================
        let all_remote_refs = client.get_refs().await?;
        let remote_branches: Vec<_> = all_remote_refs.refs.iter()
            .filter(|r| r.name.starts_with("refs/heads/"))
            .collect();

        // Get remote OID for current branch (for sync check below)
        let remote_oid_check = all_remote_refs
            .refs
            .iter()
            .find(|r| r.name == remote_ref)
            .map(|r| r.oid.clone());

        if !self.dry_run {
            // Create remotes directory for tracking refs
            let remotes_dir = storage_path.join("refs").join("remotes").join(remote);
            std::fs::create_dir_all(&remotes_dir)?;

            let mut tracking_refs_updated = 0;
            for branch_ref in &remote_branches {
                let branch_name = branch_ref.name.strip_prefix("refs/heads/")
                    .unwrap_or(&branch_ref.name);
                let tracking_ref_name = format!("refs/remotes/{}/{}", remote, branch_name);

                // Parse remote OID and update tracking ref
                if let Ok(branch_oid) = mediagit_versioning::Oid::from_hex(&branch_ref.oid) {
                    // Create parent directories for nested branches (e.g., feature/auth)
                    let tracking_path = storage_path.join("refs").join("remotes").join(remote).join(branch_name);
                    if let Some(parent) = tracking_path.parent() {
                        std::fs::create_dir_all(parent)?;
                    }

                    let tracking_ref = mediagit_versioning::Ref::new_direct(tracking_ref_name.clone(), branch_oid);
                    refdb.write(&tracking_ref).await?;
                    tracking_refs_updated += 1;

                    if self.verbose {
                        println!("  {} {} -> {}", style("→").cyan(), tracking_ref_name, &branch_ref.oid[..8.min(branch_ref.oid.len())]);
                    }
                }
            }

            if !self.quiet && tracking_refs_updated > 0 {
                println!(
                    "{} Fetched {} remote tracking refs",
                    style("✓").green(),
                    tracking_refs_updated
                );
            }
        }

        // Check if current branch is already synchronized (avoid redundant object download)
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
            // ================================================================
            // STEP 2: Pull the specific branch's objects
            // ================================================================
            // Build the "have" list from local ref state for incremental pull
            let local_have: Vec<String> = local_ref
                .as_ref()
                .and_then(|r| r.oid.as_ref())
                .map(|oid| vec![oid.to_hex()])
                .unwrap_or_default();

            // Pull using streaming protocol (memory-efficient for large files)
            // Pass local OIDs to avoid downloading objects we already have
            let download_pb = progress.download_bar("Receiving objects");

            // Use streaming pull - objects are written directly to ODB as they're received
            let chunked_oids = client.pull_streaming(&odb, &remote_ref, local_have).await?;

            if !self.quiet {
                if chunked_oids.is_empty() {
                    println!(
                        "{} Received and unpacked objects (streaming)",
                        style("↓").cyan()
                    );
                } else {
                    println!(
                        "{} Received pack (streaming) + {} chunked objects",
                        style("↓").cyan(),
                        chunked_oids.len()
                    );
                }
            }
            download_pb.finish_with_message("Download complete");

            // Download chunked objects (large files)
            if !chunked_oids.is_empty() {
                let chunk_pb = progress.object_bar("Downloading large files", chunked_oids.len() as u64);

                let chunk_pb_ref = chunk_pb.clone();
                let chunks_downloaded = client.download_chunked_objects(&odb, &chunked_oids, move |current, _total, _msg| {
                    chunk_pb_ref.set_position(current as u64);
                }).await?;

                chunk_pb.finish_with_message("Download complete");

                stats.objects_received += chunks_downloaded as u64;

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

            // Update remote tracking ref first (refs/remotes/<remote>/<branch>)
            if remote_ref.starts_with("refs/heads/") {
                // Safe: we just checked for the prefix above
                let branch_name = remote_ref.strip_prefix("refs/heads/")
                    .unwrap_or(&remote_ref);
                let tracking_ref_name = format!("refs/remotes/{}/{}", remote, branch_name);
                
                // Create remotes directory if needed
                let remotes_dir = storage_path.join("refs").join("remotes").join(remote);
                std::fs::create_dir_all(&remotes_dir)?;
                
                let tracking_ref = mediagit_versioning::Ref::new_direct(tracking_ref_name.clone(), remote_oid_parsed);
                refdb.write(&tracking_ref).await?;
                
                if self.verbose {
                    println!("  Updated tracking ref: {} -> {}", tracking_ref_name, &remote_oid[..8]);
                }
            }

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

            // Integrate changes (merge or rebase) - ONLY if pulling the current branch
            // Check if we're pulling the current branch or a different one
            let is_pulling_current_branch = match &current_head_target {
                Some(target) => target == &remote_ref,
                None => false, // Detached HEAD - don't auto-merge
            };

            if !is_pulling_current_branch {
                // Pulled a different branch - just update refs, don't merge into current
                let branch_short = remote_ref.strip_prefix("refs/heads/")
                    .unwrap_or(&remote_ref);
                if !self.quiet {
                    println!(
                        "{} Fetched branch '{}' (use: mediagit branch switch {})",
                        style("✓").green(),
                        branch_short,
                        branch_short
                    );
                }
            } else if self.rebase {
                // Rebase integration using the RebaseCmd
                let head = refdb.read("HEAD").await?;
                if let Some(head_oid) = head.oid {
                    // Get upstream ref name (e.g., "origin/main" or just "main")
                    let upstream_name = if remote_ref.starts_with("refs/heads/") {
                        // Use remote tracking ref as upstream
                        let branch_name = remote_ref.strip_prefix("refs/heads/")
                            .unwrap_or(&remote_ref);
                        format!("{}/{}", remote, branch_name)
                    } else {
                        remote_oid.clone()
                    };

                    if self.verbose {
                        let head_hex = head_oid.to_hex();
                        println!("  Rebasing {} onto {}", &head_hex[..8], &remote_oid[..8]);
                    }

                    // Create and execute rebase command
                    let rebase_cmd = RebaseCmd {
                        upstream: upstream_name,
                        branch: None, // Rebase current branch
                        interactive: false,
                        rebase_merges: false,
                        keep_empty: false,
                        autosquash: false,
                        abort: false,
                        continue_rebase: false,
                        skip: false,
                        quiet: self.quiet,
                        verbose: self.verbose,
                    };

                    rebase_cmd.execute().await?;

                    if !self.quiet {
                        println!(
                            "{} Rebased successfully",
                            style("✓").green().bold()
                        );
                    }
                } else {
                    // No local commits, just update HEAD (fast-forward)
                    let remote_oid_parsed = mediagit_versioning::Oid::from_hex(&remote_oid)
                        .map_err(|e| anyhow::anyhow!("Invalid remote OID: {}", e))?;

                    if let Some(target) = &head.target {
                        let target_ref = mediagit_versioning::Ref::new_direct(target.clone(), remote_oid_parsed);
                        refdb.write(&target_ref).await?;
                    } else {
                        let head_ref = mediagit_versioning::Ref::new_direct("HEAD".to_string(), remote_oid_parsed);
                        refdb.write(&head_ref).await?;
                    }

                    // Checkout working directory to match new HEAD
                    let checkout_mgr = CheckoutManager::new(&odb, &repo_root);
                    let files_count = checkout_mgr.checkout_commit(&remote_oid_parsed).await?;
                    if self.verbose {
                        println!("  Checked out {} files", files_count);
                    }

                    if !self.quiet {
                        println!(
                            "{} Fast-forwarded to {}",
                            style("✓").green().bold(),
                            &remote_oid[..8]
                        );
                    }
                }
            } else {
                // Merge integration - only for CURRENT branch
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

                        // Checkout working directory to match merge result
                        let checkout_mgr = CheckoutManager::new(&odb, &repo_root);
                        let files_count = checkout_mgr.checkout_commit(&commit_oid).await?;
                        if self.verbose {
                            println!("  Checked out {} files", files_count);
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

                    // Checkout working directory to match new HEAD
                    let checkout_mgr = CheckoutManager::new(&odb, &repo_root);
                    let files_count = checkout_mgr.checkout_commit(&remote_oid_parsed).await?;
                    if self.verbose {
                        println!("  Checked out {} files", files_count);
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
        
        // Save stats for later retrieval by stats command
        if !self.dry_run {
            if let Err(e) = stats.save(&storage_path) {
                tracing::warn!("Failed to save operation stats: {}", e);
            }
        }

        Ok(())
    }

}
