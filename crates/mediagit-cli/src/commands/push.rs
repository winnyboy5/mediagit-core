use anyhow::{Context, Result};
use clap::Parser;
use console::style;
use mediagit_storage::LocalBackend;
use mediagit_versioning::{RefDatabase};
use std::sync::Arc;

/// Update remote references and send objects
#[derive(Parser, Debug)]
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
                "{} Preparing to push to {}...",
                style("📤").cyan().bold(),
                style(remote).yellow()
            );
        }

        // Validate local refs exist
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

        // Initialize ODB with 1000 object cache capacity
        let odb =
            mediagit_versioning::ObjectDatabase::new(Arc::clone(&storage), 1000);

        // Determine which refs to push
        let ref_to_push = if self.refspec.is_empty() {
            // Default: push current branch
            let head = refdb.read("HEAD").await?;
            head.target.ok_or_else(|| {
                anyhow::anyhow!("HEAD is detached, please specify a refspec")
            })?
        } else {
            // Use first refspec (simplified - should parse properly)
            self.refspec[0].clone()
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
        let update = mediagit_protocol::RefUpdate {
            name: ref_to_push.clone(),
            old_oid: remote_oid.clone(),
            new_oid: local_oid_str.clone(),
        };

        if !self.quiet {
            if let Some(ref remote) = remote_oid {
                println!(
                    "{} Updating {} -> {}",
                    style("→").cyan(),
                    &remote[..8],
                    &local_oid_str[..8]
                );
            } else {
                println!(
                    "{} Creating {} at {}",
                    style("*").green(),
                    ref_to_push,
                    &local_oid_str[..8]
                );
            }
        }

        if !self.dry_run {
            // Push using protocol client
            let result = client.push(&odb, vec![update], self.force).await?;

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
                println!("{} Push successful!", style("✓").green().bold());
                for res in result.results {
                    if res.success {
                        println!("  {} {}", style("✓").green(), res.ref_name);
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
