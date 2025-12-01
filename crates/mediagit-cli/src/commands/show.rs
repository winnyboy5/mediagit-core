use anyhow::{Context, Result};
use clap::Parser;
use console::style;
use mediagit_storage::LocalBackend;
use mediagit_versioning::{Commit, ObjectDatabase, Oid, RefDatabase};
use std::sync::Arc;

/// Show object information
#[derive(Parser, Debug)]
pub struct ShowCmd {
    /// Object to show (commit, tag, tree, blob)
    #[arg(value_name = "OBJECT")]
    pub object: String,

    /// Show patch
    #[arg(short = 'p', long)]
    pub patch: bool,

    /// Show statistics
    #[arg(long)]
    pub stat: bool,

    /// Show pretty format
    #[arg(long, value_name = "FORMAT")]
    pub pretty: Option<String>,

    /// Number of context lines
    #[arg(short = 'U', long, value_name = "NUM")]
    pub unified: Option<usize>,

    /// Quiet mode
    #[arg(short, long)]
    pub quiet: bool,

    /// Verbose mode
    #[arg(short, long)]
    pub verbose: bool,
}

impl ShowCmd {
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

        // Resolve object ID
        let oid = self.resolve_object(&refdb).await?;

        // Read object
        let data = odb.read(&oid).await
            .context(format!("Failed to read object {}", oid))?;

        // Try to deserialize as commit
        match Commit::deserialize(&data) {
            Ok(commit) => {
                println!("{} {}", style("commit").yellow().bold(), style(&oid).yellow());
                println!("Author: {} <{}>", commit.author.name, commit.author.email);
                println!("Date:   {}", commit.author.timestamp);
                println!();
                for line in commit.message.lines() {
                    println!("    {}", line);
                }
                println!();

                if self.verbose {
                    println!("Tree: {}", commit.tree);
                    if !commit.parents.is_empty() {
                        println!("Parents:");
                        for parent in &commit.parents {
                            println!("  {}", parent);
                        }
                    }
                }
            }
            Err(_) => {
                // Not a commit, show raw object info
                println!("{} {}", style("object").cyan().bold(), style(&oid).yellow());
                println!("Size: {} bytes", data.len());

                if self.verbose {
                    // Show hex dump of first 256 bytes
                    let display_len = data.len().min(256);
                    println!("\nFirst {} bytes (hex):", display_len);
                    for (i, chunk) in data[..display_len].chunks(16).enumerate() {
                        print!("{:08x}  ", i * 16);
                        for byte in chunk {
                            print!("{:02x} ", byte);
                        }
                        println!();
                    }
                    if data.len() > 256 {
                        println!("... ({} more bytes)", data.len() - 256);
                    }
                }
            }
        }

        Ok(())
    }

    async fn resolve_object(&self, refdb: &RefDatabase) -> Result<Oid> {
        // Try as direct OID
        if let Ok(oid) = Oid::from_hex(&self.object) {
            return Ok(oid);
        }

        // Try as reference
        let ref_result = refdb.read(&self.object).await;
        match ref_result {
            Ok(r) => r.oid.ok_or_else(|| anyhow::anyhow!("Reference {} has no commit", self.object)),
            Err(_) => {
                // Try with refs/heads prefix
                let with_prefix = format!("refs/heads/{}", self.object);
                let ref_result = refdb.read(&with_prefix).await;
                match ref_result {
                    Ok(r) => r.oid.ok_or_else(|| anyhow::anyhow!("Reference {} has no commit", self.object)),
                    Err(_) => anyhow::bail!("Cannot resolve object: {}", self.object),
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
