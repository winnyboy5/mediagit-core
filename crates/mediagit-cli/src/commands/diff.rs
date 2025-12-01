use anyhow::{Context, Result};
use clap::Parser;
use console::style;
use mediagit_storage::LocalBackend;
use mediagit_versioning::{Commit, ObjectDatabase, Oid, RefDatabase};
use std::sync::Arc;

/// Show changes between commits
#[derive(Parser, Debug)]
pub struct DiffCmd {
    /// First revision to compare
    #[arg(value_name = "REVISION1")]
    pub from: Option<String>,

    /// Second revision to compare
    #[arg(value_name = "REVISION2")]
    pub to: Option<String>,

    /// Compare with working directory
    #[arg(long)]
    pub cached: bool,

    /// Show word-level changes
    #[arg(long)]
    pub word_diff: bool,

    /// Show statistics
    #[arg(long)]
    pub stat: bool,

    /// Show summary
    #[arg(long)]
    pub summary: bool,

    /// Number of context lines
    #[arg(short = 'U', long, value_name = "NUM")]
    pub unified: Option<usize>,

    /// Paths to diff
    #[arg(value_name = "PATHS")]
    pub paths: Vec<String>,

    /// Quiet mode
    #[arg(short, long)]
    pub quiet: bool,
}

impl DiffCmd {
    pub async fn execute(&self) -> Result<()> {
        if self.quiet {
            return Ok(());
        }

        let repo_root = self.find_repo_root()?;
        let storage_path = repo_root.join(".mediagit/objects");
        let storage: Arc<dyn mediagit_storage::StorageBackend> =
            Arc::new(LocalBackend::new(&storage_path).await?);
        let refdb = RefDatabase::new(storage.clone());
        let odb = ObjectDatabase::new(storage, 1000);

        // Resolve commits
        let (from_oid, to_oid) = self.resolve_commits(&refdb).await?;

        // Read commits
        let from_data = odb.read(&from_oid).await?;
        let from_commit = Commit::deserialize(&from_data)
            .context(format!("Failed to deserialize commit {}", from_oid))?;

        let to_data = odb.read(&to_oid).await?;
        let to_commit = Commit::deserialize(&to_data)
            .context(format!("Failed to deserialize commit {}", to_oid))?;

        // Display diff header
        println!("{} Comparing commits:", style("📊").cyan().bold());
        println!("  From: {} ({})", from_oid, from_commit.message.lines().next().unwrap_or(""));
        println!("  To:   {} ({})", to_oid, to_commit.message.lines().next().unwrap_or(""));
        println!();

        // Basic tree comparison
        if from_commit.tree == to_commit.tree {
            println!("{}", style("No changes between commits").dim());
        } else {
            println!("{}", style("Trees differ:").bold());
            println!("  From tree: {}", from_commit.tree);
            println!("  To tree:   {}", to_commit.tree);
            println!();
            println!("{}", style("Full diff functionality requires tree traversal and comparison").dim());
            println!("{}", style("This feature will be enhanced in a future release").dim());
        }

        Ok(())
    }

    async fn resolve_commits(&self, refdb: &RefDatabase) -> Result<(Oid, Oid)> {
        let from_oid = if let Some(from) = &self.from {
            self.resolve_revision(refdb, from).await?
        } else {
            // Use HEAD~1 (parent of HEAD)
            let head = refdb.read("HEAD").await?;
            let head_oid = match head.oid {
                Some(oid) => oid,
                None => {
                    if let Some(target) = head.target {
                        let target_ref = refdb.read(&target).await?;
                        target_ref.oid.ok_or_else(|| anyhow::anyhow!("No commits yet"))?
                    } else {
                        anyhow::bail!("No commits yet");
                    }
                }
            };
            head_oid
        };

        let to_oid = if let Some(to) = &self.to {
            self.resolve_revision(refdb, to).await?
        } else {
            // Use HEAD
            let head = refdb.read("HEAD").await?;
            match head.oid {
                Some(oid) => oid,
                None => {
                    if let Some(target) = head.target {
                        let target_ref = refdb.read(&target).await?;
                        target_ref.oid.ok_or_else(|| anyhow::anyhow!("No commits yet"))?
                    } else {
                        anyhow::bail!("No commits yet");
                    }
                }
            }
        };

        Ok((from_oid, to_oid))
    }

    async fn resolve_revision(&self, refdb: &RefDatabase, revision: &str) -> Result<Oid> {
        // Try as direct OID
        if let Ok(oid) = Oid::from_hex(revision) {
            return Ok(oid);
        }

        // Try as reference
        let ref_result = refdb.read(revision).await;
        match ref_result {
            Ok(r) => r.oid.ok_or_else(|| anyhow::anyhow!("Reference {} has no commit", revision)),
            Err(_) => {
                // Try with refs/heads prefix
                let with_prefix = format!("refs/heads/{}", revision);
                let ref_result = refdb.read(&with_prefix).await;
                match ref_result {
                    Ok(r) => r.oid.ok_or_else(|| anyhow::anyhow!("Reference {} has no commit", revision)),
                    Err(_) => anyhow::bail!("Cannot resolve revision: {}", revision),
                }
            }
        }
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
