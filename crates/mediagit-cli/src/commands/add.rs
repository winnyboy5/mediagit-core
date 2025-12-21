//! Stage file contents for commit.
//!
//! The `add` command stages changes to files for inclusion in the next commit.

use anyhow::{Context, Result};
use clap::Parser;
use mediagit_storage::LocalBackend;
use mediagit_versioning::{ChunkStrategy, Index, IndexEntry, ObjectDatabase, ObjectType, StorageConfig};
use std::path::{Path, PathBuf};
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

    /// Enable chunking for large media files (experimental)
    #[arg(long)]
    pub chunking: bool,

    /// Enable delta compression for similar files (experimental)
    #[arg(long)]
    pub delta: bool,
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

        // Initialize storage and ODB with smart compression
        let storage_path = repo_root.join(".mediagit");
        let storage: Arc<dyn mediagit_storage::StorageBackend> =
            Arc::new(LocalBackend::new(&storage_path).await?);

        // Check if optimizations are enabled (via flags or environment)
        let config = StorageConfig::from_env();
        let chunking_enabled = self.chunking || config.chunking_enabled;
        let delta_enabled = self.delta || config.delta_enabled;

        // Create ODB with appropriate optimizations
        let odb = if chunking_enabled || delta_enabled {
            if !self.quiet {
                if chunking_enabled {
                    output::info("Chunking enabled for large media files");
                }
                if delta_enabled {
                    output::info("Delta compression enabled for similar files");
                }
            }
            ObjectDatabase::with_optimizations(
                storage,
                1000,
                if chunking_enabled {
                    Some(ChunkStrategy::MediaAware)
                } else {
                    None
                },
                delta_enabled
            )
        } else {
            ObjectDatabase::with_smart_compression(storage, 1000)
        };

        // Load the index
        let mut index = Index::load(&repo_root)?;

        // Expand paths (globs, directories) into file list
        let files_to_add = self.expand_paths(&repo_root)?;

        if files_to_add.is_empty() {
            if !self.quiet {
                output::warning("No files to stage");
            }
            anyhow::bail!("No files were staged");
        }

        let mut added_count = 0;

        for file_path in &files_to_add {
            if !self.dry_run {
                // Read file content
                let content = tokio::fs::read(file_path)
                    .await
                    .context(format!("Failed to read file: {}", file_path.display()))?;

                // Get file metadata
                let metadata = tokio::fs::metadata(file_path)
                    .await
                    .context(format!("Failed to read file metadata: {}", file_path.display()))?;

                // Write to object database with appropriate optimization
                let filename = file_path.file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("");

                // Use chunking for large files (>10MB) when enabled
                const CHUNKING_THRESHOLD: usize = 10 * 1024 * 1024; // 10 MB
                let oid = if chunking_enabled && content.len() > CHUNKING_THRESHOLD {
                    // Large file with chunking enabled
                    if self.verbose {
                        output::detail(
                            "chunking",
                            &format!("{} ({:.2} MB)", file_path.display(), content.len() as f64 / 1_048_576.0)
                        );
                    }
                    odb.write_chunked(ObjectType::Blob, &content, filename)
                        .await
                        .context("Failed to write chunked object")?
                } else if delta_enabled {
                    // Delta compression enabled (for similar files)
                    if self.verbose {
                        output::detail(
                            "delta",
                            &format!("{} ({:.2} MB)", file_path.display(), content.len() as f64 / 1_048_576.0)
                        );
                    }
                    odb.write_with_delta(ObjectType::Blob, &content, filename)
                        .await
                        .context("Failed to write object with delta")?
                } else {
                    // Standard write with smart compression
                    odb.write_with_path(ObjectType::Blob, &content, filename)
                        .await
                        .context("Failed to write object")?
                };

                // Add to index
                let relative_path = file_path.strip_prefix(&repo_root)
                    .unwrap_or(file_path)
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
                    output::detail("added", &format!("{} ({})", file_path.display(), oid));
                }
            }

            added_count += 1;
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

    /// Expand paths (globs, directories) into a list of files to add
    fn expand_paths(&self, repo_root: &Path) -> Result<Vec<PathBuf>> {
        use crate::output;

        let mut files = Vec::new();
        let mediagit_dir = repo_root.join(".mediagit");

        for path_str in &self.paths {
            let path = Path::new(path_str);

            // Handle glob patterns
            if path_str.contains('*') || path_str.contains('?') {
                match glob::glob(path_str) {
                    Ok(entries) => {
                        for entry in entries {
                            match entry {
                                Ok(p) => {
                                    if p.is_file() {
                                        // Check if file is in .mediagit directory
                                        if let Ok(abs_path) = p.canonicalize() {
                                            if !abs_path.starts_with(&mediagit_dir) {
                                                files.push(p);
                                            }
                                        }
                                    }
                                }
                                Err(e) => {
                                    if !self.force {
                                        output::warning(&format!("Glob error: {}", e));
                                    }
                                }
                            }
                        }
                    }
                    Err(e) => {
                        if !self.force {
                            output::warning(&format!("Invalid glob pattern '{}': {}", path_str, e));
                        }
                    }
                }
                continue;
            }

            // Check if path exists
            if !path.exists() {
                if !self.force {
                    output::warning(&format!("Path does not exist: {}", path_str));
                }
                continue;
            }

            // Handle files
            if path.is_file() {
                // Skip .mediagit directory files
                if let Ok(abs_path) = path.canonicalize() {
                    if !abs_path.starts_with(&mediagit_dir) {
                        files.push(path.to_path_buf());
                    }
                }
            }
            // Handle directories - recurse
            else if path.is_dir() {
                self.collect_files_recursive(path, &mediagit_dir, &mut files)?;
            }
        }

        Ok(files)
    }

    /// Recursively collect all files from a directory
    fn collect_files_recursive(
        &self,
        dir: &Path,
        mediagit_dir: &Path,
        files: &mut Vec<PathBuf>,
    ) -> Result<()> {
        // Skip .mediagit directory - use canonicalize for reliable comparison
        if let Ok(abs_dir) = dir.canonicalize() {
            if abs_dir.starts_with(mediagit_dir) || abs_dir == *mediagit_dir {
                return Ok(());
            }
        }

        let entries = std::fs::read_dir(dir)
            .context(format!("Failed to read directory: {}", dir.display()))?;

        for entry in entries {
            let entry = entry?;
            let path = entry.path();

            // Skip .mediagit directory and its contents
            if let Ok(abs_path) = path.canonicalize() {
                if abs_path.starts_with(mediagit_dir) {
                    continue;
                }
            }

            if path.is_file() {
                // Normalize path to avoid ./ prefix issues
                if let Ok(abs_path) = path.canonicalize() {
                    files.push(abs_path);
                } else {
                    files.push(path);
                }
            } else if path.is_dir() {
                self.collect_files_recursive(&path, mediagit_dir, files)?;
            }
        }

        Ok(())
    }
}
