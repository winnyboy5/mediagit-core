use anyhow::{Context, Result};
use clap::Parser;
use console::style;
use mediagit_storage::LocalBackend;
use mediagit_versioning::{resolve_revision, Commit, ObjectDatabase, RefDatabase};
use std::sync::Arc;

/// Show object information
#[derive(Parser, Debug)]
pub struct ShowCmd {
    /// Object to show (commit, tag, tree, blob) - defaults to HEAD
    #[arg(value_name = "OBJECT")]
    pub object: Option<String>,

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
        let storage_path = repo_root.join(".mediagit");
        let storage: Arc<dyn mediagit_storage::StorageBackend> =
            Arc::new(LocalBackend::new(&storage_path).await?);
        let refdb = RefDatabase::new(&storage_path);
        let odb = ObjectDatabase::new(storage, 1000);

        // Resolve object ID using revision parser (supports HEAD~N)
        let object_str = self.object.as_deref().unwrap_or("HEAD");
        let oid = resolve_revision(object_str, &refdb, &odb).await
            .context(format!("Cannot resolve object: {}", object_str))?;

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
