//! Stage file contents for commit.
//!
//! The `add` command stages changes to files for inclusion in the next commit.

use anyhow::{Context, Result};
use clap::Parser;
use mediagit_storage::LocalBackend;
use mediagit_versioning::{Index, IndexEntry, ObjectDatabase, ObjectType};
use std::path::Path;
use std::sync::Arc;

/// Add file contents to the staging area
///
/// Stages changes to files for inclusion in the next commit. This command
/// updates the index with the current content found in the working tree.
#[derive(Parser, Debug)]
#[command(after_help = "EXAMPLES:
    # Stage a single file
    mediagit add photo.psd

    # Stage multiple files
    mediagit add image1.jpg image2.png video.mp4

    # Stage all modified and new files
    mediagit add --all

    # Preview what would be staged
    mediagit add --dry-run *.psd

SEE ALSO:
    mediagit-status(1), mediagit-commit(1), mediagit-reset(1)")]
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
        let storage_path = repo_root.join(".mediagit");
        let storage: Arc<dyn mediagit_storage::StorageBackend> =
            Arc::new(LocalBackend::new(&storage_path).await?);
        let odb = ObjectDatabase::new(storage, 1000);

        // Load the index
        let mut index = Index::load(&repo_root)?;

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

                    // Get file metadata
                    let metadata = tokio::fs::metadata(path)
                        .await
                        .context(format!("Failed to read file metadata: {}", path_str))?;

                    // Write to object database
                    let oid = odb
                        .write(ObjectType::Blob, &content)
                        .await
                        .context("Failed to write object")?;

                    // Add to index
                    let relative_path = path.strip_prefix(&repo_root)
                        .unwrap_or(path)
                        .to_path_buf();

                    let mode = if cfg!(unix) {
                        #[cfg(unix)]
                        {
                            use std::os::unix::fs::PermissionsExt;
                            metadata.permissions().mode()
                        }
                        #[cfg(not(unix))]
                        0o100644
                    } else {
                        0o100644
                    };

                    let entry = IndexEntry::new(
                        relative_path.clone(),
                        oid,
                        mode,
                        metadata.len()
                    );
                    index.add_entry(entry);

                    if self.verbose {
                        output::detail("added", &format!("{} ({})", path_str, oid));
                    }
                }

                added_count += 1;
            }
        }

        // Save the index
        if !self.dry_run {
            index.save(&repo_root)
                .context("Failed to save index")?;
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
