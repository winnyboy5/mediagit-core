use anyhow::{Context, Result};
use clap::Parser;
use console::style;
use mediagit_versioning::{resolve_revision, Commit, ObjectDatabase, RefDatabase};
use super::super::repo::{find_repo_root, create_storage_backend};

/// Show object information
#[derive(Parser, Debug)]
pub struct ShowCmd {
    /// Object to show (commit, tag, tree, blob) - defaults to HEAD
    #[arg(value_name = "OBJECT")]
    pub object: Option<String>,

    /// Show patch (not yet implemented)
    #[arg(short = 'p', long, hide = true)]
    pub patch: bool,

    /// Show statistics (not yet implemented)
    #[arg(long, hide = true)]
    pub stat: bool,

    /// Show pretty format (not yet implemented)
    #[arg(long, value_name = "FORMAT", hide = true)]
    pub pretty: Option<String>,

    /// Number of context lines (not yet implemented)
    #[arg(short = 'U', long, value_name = "NUM", hide = true)]
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

        let repo_root = find_repo_root()?;
        let storage_path = repo_root.join(".mediagit");
        let storage = create_storage_backend(&repo_root).await?;
        let refdb = RefDatabase::new(&storage_path);
        let odb = ObjectDatabase::with_smart_compression(storage, 1000);

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


}
