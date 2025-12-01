//! Stage file contents for commit.
//!
//! The `add` command stages changes to files for inclusion in the next commit.

use anyhow::{Context, Result};
use clap::Parser;
use mediagit_storage::LocalBackend;
use mediagit_versioning::{ObjectDatabase, ObjectType, Oid};
use std::path::Path;
use std::sync::Arc;

/// Add file contents to the staging area
#[derive(Parser, Debug)]
pub struct AddCmd {
    /// Files or patterns to add
    #[arg(value_name = "PATHS", required = true)]
    pub paths: Vec<String>,

    /// Add all changes
    #[arg(short = 'A', long)]
    pub all: bool,

    /// Interactively choose hunks to add
    #[arg(short, long)]
    pub patch: bool,

    /// Show what would be staged
    #[arg(long)]
    pub dry_run: bool,

    /// Force add even if listed in .gitignore
    #[arg(short, long)]
    pub force: bool,

    /// Ignore removal of files in the index
    #[arg(long)]
    pub ignore_removal: bool,

    /// Update tracked files only
    #[arg(short, long)]
    pub update: bool,

    /// Quiet mode
    #[arg(short, long)]
    pub quiet: bool,

    /// Verbose mode
    #[arg(short, long)]
    pub verbose: bool,
}

impl AddCmd {
    pub async fn execute(&self) -> Result<()> {
        use crate::output;

        // Find repository root
        let repo_root = self.find_repo_root()?;

        if self.dry_run {
            output::info("Running in dry-run mode");
        }

        if !self.quiet && !self.dry_run {
            output::progress("Staging files...");
        }

        // Initialize storage and ODB
        let storage_path = repo_root.join(".mediagit/objects");
        let storage: Arc<dyn mediagit_storage::StorageBackend> =
            Arc::new(LocalBackend::new(&storage_path).await?);
        let odb = ObjectDatabase::new(storage, 1000);

        let mut added_count = 0;

        for path_str in &self.paths {
            let path = Path::new(path_str);

            if !path.exists() {
                if !self.force {
                    output::warning(&format!("Path does not exist: {}", path_str));
                    continue;
                }
            }

            if path.is_file() {
                if !self.dry_run {
                    // Read file content
                    let content = tokio::fs::read(path)
                        .await
                        .context(format!("Failed to read file: {}", path_str))?;

                    // Write to object database
                    let oid = odb
                        .write(ObjectType::Blob, &content)
                        .await
                        .context("Failed to write object")?;

                    if self.verbose {
                        output::detail("added", &format!("{} ({})", path_str, oid));
                    }
                }

                added_count += 1;
            }
        }

        if !self.quiet {
            output::success(&format!("Staged {} file(s)", added_count));
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
