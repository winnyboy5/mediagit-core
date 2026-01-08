//! Stage file contents for commit.
//!
//! The `add` command stages changes to files for inclusion in the next commit.

use anyhow::{Context, Result};
use clap::Parser;
use indicatif::{ProgressBar, ProgressStyle};
use mediagit_storage::LocalBackend;
use mediagit_versioning::{ChunkStrategy, Commit, Index, IndexEntry, ObjectDatabase, ObjectType, Oid, RefDatabase, StorageConfig, Tree};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

// Cross-platform path canonicalization that handles Windows \\?\ prefix
// On Windows: returns simplified paths without \\?\ prefix when possible
// On Linux/macOS: compiles to std::fs::canonicalize()

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

    /// Disable delta compression (delta is enabled by default for suitable files)
    #[arg(long)]
    pub no_delta: bool,
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
        // Delta is enabled by default, can be disabled with --no-delta
        let delta_enabled = !self.no_delta && (config.delta_enabled || true);

        // Create ODB with appropriate optimizations (delta always enabled by default)
        let odb = if chunking_enabled {
            if !self.quiet {
                output::info("Chunking enabled for large media files");
            }
            ObjectDatabase::with_optimizations(
                storage,
                1000,
                Some(ChunkStrategy::MediaAware),
                delta_enabled
            )
        } else {
            // Use smart compression with delta enabled by default
            ObjectDatabase::with_optimizations(
                storage,
                1000,
                None,
                delta_enabled
            )
        };

        // Load the index
        let mut index = Index::load(&repo_root)?;

        // Get HEAD commit tree to identify already-tracked files
        // This allows us to skip files that haven't changed since the last commit
        let refdb = RefDatabase::new(&storage_path);
        let mut head_files: HashMap<PathBuf, Oid> = HashMap::new();
        
        if let Ok(head_oid) = refdb.resolve("HEAD").await {
            if let Ok(commit_data) = odb.read(&head_oid).await {
                if let Ok(commit) = bincode::deserialize::<Commit>(&commit_data) {
                    if let Ok(tree_data) = odb.read(&commit.tree).await {
                        if let Ok(tree) = bincode::deserialize::<Tree>(&tree_data) {
                            for entry in tree.iter() {
                                head_files.insert(PathBuf::from(&entry.name), entry.oid);
                            }
                        }
                    }
                }
            }
        }

        // Expand paths (globs, directories) into file list
        let files_to_add = self.expand_paths(&repo_root)?;

        // Note: Don't bail early if files_to_add is empty - we may still have deletions to stage

        // Calculate total bytes for progress bar (only if not quiet and files exist)
        let (total_files, total_bytes) = if !self.quiet && !files_to_add.is_empty() {
            let mut bytes = 0u64;
            for f in &files_to_add {
                if let Ok(meta) = std::fs::metadata(f) {
                    bytes += meta.len();
                }
            }
            (files_to_add.len() as u64, bytes)
        } else {
            (files_to_add.len() as u64, 0)
        };

        // Create progress bar for staging
        let progress_bar = if !self.quiet && !self.dry_run && total_files > 0 {
            let pb = ProgressBar::new(total_bytes);
            pb.set_style(
                ProgressStyle::default_bar()
                    .template("{spinner:.green} [{bar:40.cyan/blue}] {bytes}/{total_bytes} ({percent}%) {msg}")
                    .unwrap()
                    .progress_chars("█▓░"),
            );
            pb.enable_steady_tick(Duration::from_millis(100));
            pb.set_message(format!("0/{} files", total_files));
            Some(pb)
        } else {
            None
        };

        let mut added_count = 0;
        let mut skipped_count = 0;
        let mut processed_bytes = 0u64;

        for file_path in &files_to_add {
            if !self.dry_run {
                // Read file content
                let content = tokio::fs::read(file_path)
                    .await
                    .context(format!("Failed to read file: {}", file_path.display()))?;

                // Get relative path early so we can check against HEAD
                let relative_path = file_path.strip_prefix(&repo_root)
                    .unwrap_or(file_path)
                    .to_path_buf();

                // Compute OID to check if file has changed
                let content_oid = Oid::hash(&content);

                // Check if file is already in HEAD with the same OID (unchanged)
                if let Some(head_oid) = head_files.get(&relative_path) {
                    if *head_oid == content_oid {
                        // File is unchanged from HEAD, skip it
                        skipped_count += 1;
                        if self.verbose {
                            output::detail("skipped (unchanged)", &relative_path.display().to_string());
                        }
                        continue;
                    }
                }

                // Get file metadata
                let metadata = tokio::fs::metadata(file_path)
                    .await
                    .context(format!("Failed to read file metadata: {}", file_path.display()))?;

                // Write to object database with intelligent feature selection
                let filename = file_path.file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("");

                // Intelligent feature selection based on file type and size
                let oid = if chunking_enabled && Self::should_use_chunking(content.len(), filename) {
                    // Chunking recommended for this file type/size
                    if self.verbose {
                        output::detail(
                            "chunking",
                            &format!("{} ({:.2} MB)", file_path.display(), content.len() as f64 / 1_048_576.0)
                        );
                    }
                    odb.write_chunked(ObjectType::Blob, &content, filename)
                        .await
                        .context("Failed to write chunked object")?
                } else if delta_enabled && Self::should_use_delta(filename, &content) {
                    // Delta compression recommended for this file type
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

                added_count += 1;

                // Update progress bar
                processed_bytes += content.len() as u64;
                if let Some(ref pb) = progress_bar {
                    pb.set_position(processed_bytes);
                    pb.set_message(format!("{}/{} files", added_count + skipped_count, total_files));
                }
            } else {
                // Dry run - still count but don't actually add
                added_count += 1;
            }
        }

        // Detect deleted files: files in HEAD but not in working directory
        let mut deleted_count = 0;
        
        // Build a set of existing working directory files for fast lookup
        let working_files: std::collections::HashSet<PathBuf> = files_to_add
            .iter()
            .filter_map(|p| p.strip_prefix(&repo_root).ok())
            .map(|p| p.to_path_buf())
            .collect();

        // Check each file in HEAD - if it doesn't exist in working dir, it's deleted
        for (head_path, _head_oid) in &head_files {
            // Normalize path separators for cross-platform comparison
            let head_path_normalized = PathBuf::from(
                head_path.to_string_lossy().replace('\\', "/")
            );
            
            let exists_in_working_dir = working_files.iter().any(|wp| {
                wp.to_string_lossy().replace('\\', "/") == head_path_normalized.to_string_lossy()
            });

            if !exists_in_working_dir {
                // Check if file actually doesn't exist on disk (not just filtered out)
                let full_path = repo_root.join(head_path);
                if !full_path.exists() {
                    if !self.dry_run {
                        index.mark_deleted(head_path.clone());
                        if self.verbose {
                            output::detail("deleted", &head_path.display().to_string());
                        }
                    }
                    deleted_count += 1;
                }
            }
        }

        // Finish progress bar
        if let Some(pb) = progress_bar {
            pb.finish_and_clear();
        }

        // Save the index
        if !self.dry_run {
            index.save(&repo_root)
                .context("Failed to save index")?;
        }

        if !self.quiet {
            if added_count > 0 {
                output::success(&format!("Staged {} file(s)", added_count));
            }
            if deleted_count > 0 {
                output::success(&format!("Staged {} deletion(s)", deleted_count));
            }
            if skipped_count > 0 && self.verbose {
                output::info(&format!("Skipped {} unchanged file(s)", skipped_count));
            }
            if added_count == 0 && deleted_count == 0 {
                if skipped_count > 0 {
                    output::info("No new or modified files to stage");
                } else if head_files.is_empty() && files_to_add.is_empty() {
                    output::warning("No files to stage");
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

    /// Check if path is outside .mediagit directory
    ///
    /// Returns true if the path is valid and not inside .mediagit.
    /// Uses dunce::canonicalize for cross-platform compatibility:
    /// - On Windows: returns paths without \\?\ prefix for reliable comparison
    /// - On Linux/macOS: equivalent to std::fs::canonicalize
    fn is_outside_mediagit(path: &Path, mediagit_dir: &Path) -> bool {
        if let Ok(abs_path) = dunce::canonicalize(path) {
            !abs_path.starts_with(mediagit_dir)
        } else {
            false
        }
    }

    /// Expand paths (globs, directories) into a list of files to add
    fn expand_paths(&self, repo_root: &Path) -> Result<Vec<PathBuf>> {
        use crate::output;

        let mut files = Vec::new();
        // Use dunce::canonicalize for cross-platform path comparison
        // This handles Windows \\?\ prefix and works correctly on all platforms
        let mediagit_dir = dunce::canonicalize(repo_root.join(".mediagit"))
            .unwrap_or_else(|_| repo_root.join(".mediagit"));

        for path_str in &self.paths {
            let path = Path::new(path_str);

            // Handle glob patterns
            if path_str.contains('*') || path_str.contains('?') {
                match glob::glob(path_str) {
                    Ok(entries) => {
                        for entry in entries {
                            match entry {
                                Ok(p) => {
                                    if p.is_file() && Self::is_outside_mediagit(&p, &mediagit_dir) {
                                        files.push(p);
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
            if path.is_file() && Self::is_outside_mediagit(path, &mediagit_dir) {
                files.push(path.to_path_buf());
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
        // Skip .mediagit directory using helper
        if !Self::is_outside_mediagit(dir, mediagit_dir) {
            return Ok(());
        }

        let entries = std::fs::read_dir(dir)
            .context(format!("Failed to read directory: {}", dir.display()))?;

        for entry in entries {
            let entry = entry?;
            let path = entry.path();

            // Skip .mediagit directory and its contents
            if !Self::is_outside_mediagit(&path, mediagit_dir) {
                continue;
            }

            if path.is_file() {
                // Use dunce::canonicalize for cross-platform path normalization
                // This avoids Windows \\?\ prefix issues while working on all platforms
                if let Ok(abs_path) = dunce::canonicalize(&path) {
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

    /// Determine if file should use chunking based on size AND type
    fn should_use_chunking(size: usize, filename: &str) -> bool {
        const MIN_CHUNK_SIZE: usize = 5 * 1024 * 1024; // 5 MB

        if size < MIN_CHUNK_SIZE {
            return false;
        }

        let ext = std::path::Path::new(filename)
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("");

        match ext.to_lowercase().as_str() {
            // Video formats: Chunk at lower threshold (benefits most)
            "mp4" | "mov" | "avi" | "mkv" | "flv" | "wmv" => size > MIN_CHUNK_SIZE,
            // Large uncompressed formats: Chunk at moderate threshold
            "psd" | "tif" | "tiff" | "bmp" | "wav" | "aiff" => size > 10 * 1024 * 1024,
            // Compressed images: Higher threshold (already compressed)
            "jpg" | "jpeg" | "png" | "webp" => size > 50 * 1024 * 1024,
            // 3D models: Chunk at moderate threshold
            "obj" | "fbx" | "blend" | "gltf" | "glb" => size > 20 * 1024 * 1024,
            // Archives: Don't chunk (already compressed)
            "zip" | "gz" | "bz2" | "7z" | "rar" => false,
            // Unknown: Use generic large file threshold
            _ => size > 50 * 1024 * 1024,
        }
    }

    /// Determine if file is suitable for delta compression
    fn should_use_delta(filename: &str, data: &[u8]) -> bool {
        let ext = std::path::Path::new(filename)
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("");

        match ext.to_lowercase().as_str() {
            // Text-based formats: Excellent delta candidates
            "txt" | "md" | "json" | "xml" | "html" | "css" | "js" | "ts" |
            "py" | "rs" | "go" | "java" | "c" | "cpp" | "h" | "hpp" => true,
            // Uncompressed formats: Good delta candidates
            "psd" | "tif" | "tiff" | "bmp" | "wav" | "aiff" => true,
            // Video (raw/uncompressed): Good delta candidates
            "avi" | "mov" => true,
            // Compressed video: Moderate benefit (only for very large files)
            "mp4" | "mkv" | "flv" | "wmv" => data.len() > 100 * 1024 * 1024,
            // Compressed images: Poor delta candidates (skip)
            "jpg" | "jpeg" | "png" | "webp" | "gif" => false,
            // Archives: No delta benefit (skip)
            "zip" | "gz" | "bz2" | "7z" | "rar" => false,
            // Unknown: Conservative approach (allow if large)
            _ => data.len() > 50 * 1024 * 1024,
        }
    }
}
