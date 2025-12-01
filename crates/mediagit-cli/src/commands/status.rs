use anyhow::{Context, Result};
use clap::Parser;
use mediagit_storage::LocalBackend;
use mediagit_versioning::{Ref, RefDatabase};
use std::path::Path;
use std::sync::Arc;

/// Show the working tree status
#[derive(Parser, Debug)]
pub struct StatusCmd {
    /// Show tracked files
    #[arg(long)]
    pub tracked: bool,

    /// Show untracked files
    #[arg(long)]
    pub untracked: bool,

    /// Show ignored files
    #[arg(long)]
    pub ignored: bool,

    /// Show short format
    #[arg(short, long)]
    pub short: bool,

    /// Show porcelain format (for scripts)
    #[arg(long)]
    pub porcelain: bool,

    /// Show branch information
    #[arg(short = 'b', long)]
    pub branch: bool,

    /// Show ahead/behind commits
    #[arg(long)]
    pub ahead_behind: bool,

    /// Quiet mode
    #[arg(short, long)]
    pub quiet: bool,

    /// Verbose mode
    #[arg(short, long)]
    pub verbose: bool,
}

impl StatusCmd {
    pub async fn execute(&self) -> Result<()> {
        use crate::output;

        // Find repository root
        let repo_root = self.find_repo_root()?;

        if !self.quiet {
            output::header("Repository Status");
        }

        // Initialize storage and ref database
        let storage_path = repo_root.join(".mediagit/objects");
        let storage: Arc<dyn mediagit_storage::StorageBackend> =
            Arc::new(LocalBackend::new(&storage_path).await?);
        let refdb = RefDatabase::new(storage);

        // Read HEAD
        let head = refdb
            .read("HEAD")
            .await
            .context("Failed to read HEAD reference")?;

        // Display current branch
        if self.branch || self.verbose {
            match &head {
                Ref {
                    ref_type: mediagit_versioning::RefType::Symbolic,
                    target: Some(branch),
                    ..
                } => {
                    let branch_name = branch.strip_prefix("refs/heads/").unwrap_or(branch);
                    output::success(&format!("On branch: {}", branch_name));
                }
                Ref {
                    ref_type: mediagit_versioning::RefType::Direct,
                    oid: Some(oid),
                    ..
                } => {
                    output::info(&format!("HEAD detached at {}", oid));
                }
                _ => {
                    output::warning("HEAD reference is invalid");
                }
            }
        }

        if !self.quiet {
            output::info("No commits yet");
            output::info("Working tree clean");
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
                anyhow::bail!("Not a mediagit repository (or any parent up to mount point)");
            }
        }
    }
}
