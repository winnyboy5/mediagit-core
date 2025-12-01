use anyhow::{Context, Result};
use chrono::Utc;
use clap::Parser;
use console::style;
use mediagit_storage::LocalBackend;
use mediagit_versioning::{Commit, MergeStrategy, RefDatabase, Signature};
use std::sync::Arc;

/// Fetch and integrate remote changes
#[derive(Parser, Debug)]
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
        let remote = self.remote.as_deref().unwrap_or("origin");

        // Validate repository
        let repo_root = self.find_repo_root()?;
        let storage_path = repo_root.join(".mediagit/objects");
        let storage: Arc<dyn mediagit_storage::StorageBackend> =
            Arc::new(LocalBackend::new(&storage_path).await?);
        let refdb = RefDatabase::new(Arc::clone(&storage));

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

        // Validate local repository state
        let _head = refdb.read("HEAD").await.context("Failed to read HEAD")?;

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

        // Initialize ODB with 1000 object cache capacity
        let odb = Arc::new(
            mediagit_versioning::ObjectDatabase::new(Arc::clone(&storage), 1000)
        );

        // Determine remote ref to pull
        let remote_ref = if let Some(branch) = &self.branch {
            format!("refs/heads/{}", branch)
        } else {
            // Default: pull tracking branch for current HEAD
            let head = refdb.read("HEAD").await?;
            head.target.ok_or_else(|| {
                anyhow::anyhow!("HEAD is detached, please specify a branch")
            })?
        };

        if self.verbose {
            println!("  Pulling ref: {}", remote_ref);
        }

        if !self.dry_run {
            // Pull using protocol client (downloads pack file)
            let pack_data = client.pull(&odb, &remote_ref).await?;

            if !self.quiet {
                println!(
                    "{} Received {} bytes",
                    style("↓").cyan(),
                    pack_data.len()
                );
            }

            // Unpack received objects
            let pack_reader = mediagit_versioning::PackReader::new(pack_data)?;
            for oid in pack_reader.list_objects() {
                let obj_data = pack_reader.get_object(&oid)?;
                // Write object to ODB (assuming Blob type for now)
                odb.write(mediagit_versioning::ObjectType::Blob, &obj_data).await?;
            }

            if !self.quiet {
                println!("{} Unpacked objects", style("✓").green());
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
                        let merge_commit = Commit::new(
                            tree_oid,
                            author.clone(),
                            author,
                            "Merge commit".to_string(),
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
