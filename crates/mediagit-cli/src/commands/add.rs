// Copyright (C) 2026  winnyboy5
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU Affero General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU Affero General Public License for more details.
//
// You should have received a copy of the GNU Affero General Public License
// along with this program.  If not, see <https://www.gnu.org/licenses/>.
//! Stage file contents for commit.
//!
//! The `add` command stages changes to files for inclusion in the next commit.

use anyhow::{Context, Result};
use clap::Parser;
use indicatif::{ProgressBar, ProgressStyle};
use mediagit_versioning::{ChunkStrategy, Commit, Index, IndexEntry, ObjectDatabase, ObjectType, Oid, RefDatabase, Tree};
use super::super::repo::{find_repo_root, create_storage_backend};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

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

    # Control parallelism
    mediagit add -j 4 *.psd

SEE ALSO:
    mediagit-status(1), mediagit-commit(1), mediagit-reset(1)")]
pub struct AddCmd {
    /// Files or patterns to add
    #[arg(value_name = "PATHS")]
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

    /// Disable automatic chunking for large files (chunking is ON by default)
    #[arg(long)]
    pub no_chunking: bool,

    /// Disable delta compression (delta is enabled by default for suitable files)
    #[arg(long)]
    pub no_delta: bool,

    /// Disable parallel file processing (process files sequentially)
    #[arg(long)]
    pub no_parallel: bool,

    /// Number of parallel worker threads (default: number of CPU cores, max 8)
    #[arg(short = 'j', long = "jobs")]
    pub jobs: Option<usize>,
}

/// Result from processing a single file in parallel
struct FileResult {
    relative_path: PathBuf,
    oid: Oid,
    file_size: u64,
    mode: u32,
    mtime: Option<u64>,
}

impl AddCmd {
    pub async fn execute(&self) -> Result<()> {
        use crate::output;

        // Validate: either --all or paths must be provided
        if !self.all && self.paths.is_empty() {
            anyhow::bail!("Nothing specified, nothing added.\nUse 'mediagit add <file>...' or 'mediagit add --all' to stage files.");
        }

        // Find repository root
        let repo_root = find_repo_root()?;

        if self.dry_run {
            output::info("Running in dry-run mode");
        }

        if !self.quiet && !self.dry_run {
            output::progress("Staging files...");
        }

        // Initialize storage backend from config (supports S3, Azure, GCS, filesystem)
        let storage_path = repo_root.join(".mediagit");
        let storage = create_storage_backend(&repo_root).await?;

        let delta_enabled = !self.no_delta;

        let odb = ObjectDatabase::with_optimizations(
            storage,
            1000,
            Some(ChunkStrategy::MediaAware),
            delta_enabled
        );

        if !self.quiet && self.verbose {
            output::info("Auto-chunking enabled for large files");
        }

        // Load the index
        let mut index = Index::load(&repo_root)?;

        // Build index lookup for stat-cache change detection (size + mtime)
        let index_files: Arc<HashMap<PathBuf, (u64, Option<u64>)>> = {
            let mut map = HashMap::new();
            for entry in index.entries() {
                map.insert(entry.path.clone(), (entry.size, entry.mtime));
            }
            Arc::new(map)
        };

        // Get HEAD commit tree to identify already-tracked files
        let refdb = RefDatabase::new(&storage_path);
        let head_files: Arc<HashMap<PathBuf, Oid>> = {
            let mut files = HashMap::new();
            if let Ok(head_oid) = refdb.resolve("HEAD").await {
                if let Ok(commit_data) = odb.read(&head_oid).await {
                    if let Ok(commit) = bincode::deserialize::<Commit>(&commit_data) {
                        if let Ok(tree_data) = odb.read(&commit.tree).await {
                            if let Ok(tree) = bincode::deserialize::<Tree>(&tree_data) {
                                for entry in tree.iter() {
                                    files.insert(PathBuf::from(&entry.name), entry.oid);
                                }
                            }
                        }
                    }
                }
            }
            Arc::new(files)
        };

        // Expand paths (globs, directories) into file list
        let files_to_add = self.expand_paths(&repo_root)?;

        // Calculate total bytes for progress bar
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

        // Create progress bar
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
            Some(Arc::new(pb))
        } else {
            None
        };

        // Atomic counters for thread-safe progress
        let progress_bytes = Arc::new(AtomicU64::new(0));
        let progress_files = Arc::new(AtomicU64::new(0));

        let use_parallel = !self.no_parallel && files_to_add.len() > 1;

        if use_parallel && !self.quiet && self.verbose {
            let worker_count = self.jobs.unwrap_or_else(|| num_cpus::get().min(8));
            output::info(&format!("Parallel mode: {} concurrent files", worker_count));
        }

        let mut added_count = 0u64;
        let mut skipped_count = 0u64;

        if !self.dry_run && !files_to_add.is_empty() {
            if use_parallel {
                // --- PARALLEL FILE PROCESSING ---
                let max_concurrent = self.jobs.unwrap_or_else(|| num_cpus::get().min(8));
                let semaphore = Arc::new(tokio::sync::Semaphore::new(max_concurrent));

                let mut file_tasks = tokio::task::JoinSet::new();
                let skipped = Arc::new(AtomicU64::new(0));

                for file_path in files_to_add.iter().cloned() {
                    let sem = semaphore.clone();
                    let odb = odb.clone();
                    let head_files = head_files.clone();
                    let index_files = index_files.clone();
                    let repo_root = repo_root.clone();
                    let progress_bytes = progress_bytes.clone();
                    let progress_files = progress_files.clone();
                    let progress_bar = progress_bar.clone();
                    let total_files = total_files;
                    let skipped = skipped.clone();
                    let delta_enabled = delta_enabled;

                    file_tasks.spawn(async move {
                        let _permit = sem.acquire().await
                            .map_err(|_| anyhow::anyhow!("Semaphore closed"))?;

                        let result = Self::process_single_file(
                            &file_path,
                            &repo_root,
                            &odb,
                            &head_files,
                            &index_files,
                            delta_enabled,
                        ).await;

                        match result {
                            Ok(Some(file_result)) => {
                                // Update progress
                                progress_bytes.fetch_add(file_result.file_size, Ordering::Relaxed);
                                let done = progress_files.fetch_add(1, Ordering::Relaxed) + 1;
                                if let Some(ref pb) = progress_bar {
                                    pb.set_position(progress_bytes.load(Ordering::Relaxed));
                                    pb.set_message(format!("{}/{} files", done, total_files));
                                }
                                Ok(Some(file_result))
                            }
                            Ok(None) => {
                                // Skipped (unchanged)
                                skipped.fetch_add(1, Ordering::Relaxed);
                                let done = progress_files.fetch_add(1, Ordering::Relaxed) + 1;
                                if let Some(ref pb) = progress_bar {
                                    pb.set_message(format!("{}/{} files", done, total_files));
                                }
                                Ok(None)
                            }
                            Err(e) => Err(e),
                        }
                    });
                }

                // Collect results and update index
                while let Some(result) = file_tasks.join_next().await {
                    match result {
                        Ok(Ok(Some(file_result))) => {
                            let entry = IndexEntry::new(
                                file_result.relative_path,
                                file_result.oid,
                                file_result.mode,
                                file_result.file_size,
                                file_result.mtime,
                            );
                            index.add_entry(entry);
                            added_count += 1;
                        }
                        Ok(Ok(None)) => {
                            // Skipped
                        }
                        Ok(Err(e)) => {
                            if !self.force {
                                return Err(e);
                            }
                            if !self.quiet {
                                output::warning(&format!("Error staging file: {}", e));
                            }
                        }
                        Err(e) => {
                            if !self.force {
                                return Err(anyhow::anyhow!("Task panicked: {}", e));
                            }
                        }
                    }
                }

                skipped_count = skipped.load(Ordering::Relaxed);
            } else {
                // --- SEQUENTIAL FILE PROCESSING (fallback / --no-parallel) ---
                for file_path in &files_to_add {
                    let result = Self::process_single_file(
                        file_path,
                        &repo_root,
                        &odb,
                        &head_files,
                        &index_files,
                        delta_enabled,
                    ).await;

                    match result {
                        Ok(Some(file_result)) => {
                            if self.verbose {
                                output::detail("added", &format!("{} ({})", file_path.display(), file_result.oid));
                            }

                            let entry = IndexEntry::new(
                                file_result.relative_path,
                                file_result.oid,
                                file_result.mode,
                                file_result.file_size,
                                file_result.mtime,
                            );
                            index.add_entry(entry);
                            added_count += 1;

                            progress_bytes.fetch_add(file_result.file_size, Ordering::Relaxed);
                            let done = progress_files.fetch_add(1, Ordering::Relaxed) + 1;
                            if let Some(ref pb) = progress_bar {
                                pb.set_position(progress_bytes.load(Ordering::Relaxed));
                                pb.set_message(format!("{}/{} files", done, total_files));
                            }
                        }
                        Ok(None) => {
                            skipped_count += 1;
                            if self.verbose {
                                output::detail("skipped (unchanged)", &file_path.display().to_string());
                            }
                        }
                        Err(e) => {
                            if !self.force {
                                return Err(e);
                            }
                            if !self.quiet {
                                output::warning(&format!("Error staging file: {}", e));
                            }
                        }
                    }
                }
            }
        } else if self.dry_run {
            added_count = files_to_add.len() as u64;
        }

        // Detect deleted files: files in HEAD but not in working directory
        let mut deleted_count = 0;

        let working_files: std::collections::HashSet<PathBuf> = files_to_add
            .iter()
            .filter_map(|p| p.strip_prefix(&repo_root).ok())
            .map(|p| p.to_path_buf())
            .collect();

        for (head_path, _head_oid) in head_files.as_ref() {
            let head_path_normalized = PathBuf::from(
                head_path.to_string_lossy().replace('\\', "/")
            );

            let exists_in_working_dir = working_files.iter().any(|wp| {
                wp.to_string_lossy().replace('\\', "/") == head_path_normalized.to_string_lossy()
            });

            if !exists_in_working_dir {
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
                    if !self.paths.is_empty() && !self.all {
                        anyhow::bail!("No files were staged");
                    }
                    output::warning("No files to stage");
                }
            }
        }

        Ok(())
    }

    /// Process a single file: hash, check HEAD, write to ODB
    ///
    /// Returns `Ok(Some(FileResult))` if file was staged,
    /// `Ok(None)` if file was skipped (unchanged from HEAD),
    /// `Err` on failure.
    async fn process_single_file(
        file_path: &Path,
        repo_root: &Path,
        odb: &ObjectDatabase,
        head_files: &HashMap<PathBuf, Oid>,
        index_files: &HashMap<PathBuf, (u64, Option<u64>)>,
        delta_enabled: bool,
    ) -> Result<Option<FileResult>> {
        let metadata = tokio::fs::metadata(file_path)
            .await
            .context(format!("Failed to read file metadata: {}", file_path.display()))?;

        let file_size = metadata.len();
        const STREAMING_THRESHOLD: u64 = 100 * 1024 * 1024; // 100MB

        let relative_path = file_path.strip_prefix(repo_root)
            .unwrap_or(file_path)
            .to_path_buf();

        // Stat-cache check: skip if file hasn't changed since last staging
        let file_mtime = metadata.modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_secs());

        if let Some(&(idx_size, Some(idx_mtime))) = index_files.get(&relative_path) {
            if idx_size == file_size {
                if let Some(current_mtime) = file_mtime {
                    if idx_mtime == current_mtime {
                        return Ok(None); // Unchanged since last staging
                    }
                }
            }
        }

        let filename = file_path.file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("");

        // Seed similarity detector from previous version's manifest
        if let Some(head_oid) = head_files.get(&relative_path) {
            if let Ok(Some(old_manifest)) = odb.get_chunk_manifest(head_oid).await {
                let _ = odb.seed_similarity_from_manifest(&old_manifest).await;
            }
        }

        // Choose streaming vs in-memory based on file size
        let (_content_oid, oid) = if file_size >= STREAMING_THRESHOLD {
            // STREAMING PATH: Files >= 100MB (parallel internally)
            let content_oid = Oid::from_file_async(file_path).await
                .context(format!("Failed to hash file: {}", file_path.display()))?;

            // Check if unchanged from HEAD
            if let Some(head_oid) = head_files.get(&relative_path) {
                if *head_oid == content_oid {
                    return Ok(None);
                }
            }

            let oid = odb.write_chunked_from_file(file_path, filename)
                .await
                .context("Failed to write chunked object (streaming)")?;

            (content_oid, oid)
        } else {
            // IN-MEMORY PATH: Files < 100MB
            let content = tokio::fs::read(file_path)
                .await
                .context(format!("Failed to read file: {}", file_path.display()))?;

            let content_oid = Oid::hash(&content);

            // Check if unchanged from HEAD
            if let Some(head_oid) = head_files.get(&relative_path) {
                if *head_oid == content_oid {
                    return Ok(None);
                }
            }

            // Use parallel chunking for large files, sequential for small
            let oid = if Self::should_use_chunking(content.len(), filename) {
                odb.write_chunked_parallel(ObjectType::Blob, &content, filename)
                    .await
                    .context("Failed to write chunked object")?
            } else if delta_enabled && Self::should_use_delta(filename, &content) {
                odb.write_with_delta(ObjectType::Blob, &content, filename)
                    .await
                    .context("Failed to write object with delta")?
            } else {
                odb.write_with_path(ObjectType::Blob, &content, filename)
                    .await
                    .context("Failed to write object")?
            };

            (content_oid, oid)
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

        Ok(Some(FileResult {
            relative_path,
            oid,
            file_size,
            mode,
            mtime: file_mtime,
        }))
    }

    /// Check if path is outside .mediagit directory
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
        let mediagit_dir = dunce::canonicalize(repo_root.join(".mediagit"))
            .unwrap_or_else(|_| repo_root.join(".mediagit"));

        // If --all is set and no paths given, add entire repo root
        if self.all && self.paths.is_empty() {
            self.collect_files_recursive(repo_root, &mediagit_dir, &mut files)?;
            return Ok(files);
        }

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

            if !path.exists() {
                if !self.force {
                    output::warning(&format!("Path does not exist: {}", path_str));
                }
                continue;
            }

            if path.is_file() && Self::is_outside_mediagit(path, &mediagit_dir) {
                files.push(path.to_path_buf());
            } else if path.is_dir() {
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
        if !Self::is_outside_mediagit(dir, mediagit_dir) {
            return Ok(());
        }

        let entries = std::fs::read_dir(dir)
            .context(format!("Failed to read directory: {}", dir.display()))?;

        for entry in entries {
            let entry = entry?;
            let path = entry.path();

            if !Self::is_outside_mediagit(&path, mediagit_dir) {
                continue;
            }

            if path.is_file() {
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
        const MIN_SIZE_5MB: usize = 5 * 1024 * 1024;
        const MIN_SIZE_10MB: usize = 10 * 1024 * 1024;

        if size < MIN_SIZE_5MB {
            return false;
        }

        let ext = std::path::Path::new(filename)
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("");

        match ext.to_lowercase().as_str() {
            // === TEXT/DATA: Excellent CDC dedup (5MB threshold) ===
            "csv" | "json" | "jsonl" | "txt" | "xml" | "yaml" | "yml" | "toml" => true,

            // === ML DATA: Excellent dedup for incremental datasets (5MB) ===
            "parquet" | "arrow" | "feather" | "orc" | "avro" |
            "hdf5" | "h5" | "npy" | "npz" | "tfrecords" | "petastorm" => true,

            // === ML MODELS: Good dedup for checkpoint files (5MB) ===
            "pt" | "pth" | "safetensors" | "ckpt" | "pb" | "onnx" |
            "gguf" | "ggml" | "tflite" | "keras" | "bin" => true,

            // === VIDEO: Structure-aware chunking (5MB) ===
            "mp4" | "mov" | "avi" | "mkv" | "webm" | "flv" | "wmv" | "mpg" | "mpeg" | "m4v" | "3gp" => true,

            // === UNCOMPRESSED IMAGES: Rolling CDC (5MB) ===
            "psd" | "tif" | "tiff" | "bmp" | "exr" | "hdr" | "raw" => true,

            // === PDF-BASED CREATIVE FILES: CDC chunking (5MB) ===
            "ai" | "ait" | "indd" | "idml" | "indt" | "eps" | "pdf" => true,

            // === CREATIVE PROJECT FILES: CDC chunking (10MB) ===
            "aep" | "prproj" | "drp" | "fcpbundle" => size > MIN_SIZE_10MB,

            // === OFFICE DOCUMENTS: CDC chunking (5MB) ===
            "docx" | "xlsx" | "pptx" | "odt" | "ods" | "odp" => true,

            // === LOSSLESS AUDIO: Good dedup (10MB) ===
            "wav" | "flac" | "aiff" | "alac" => size > MIN_SIZE_10MB,

            // === 3D MODELS: Good dedup (10MB) ===
            "glb" | "gltf" | "obj" | "fbx" | "blend" | "usd" | "usda" | "usdc" => size > MIN_SIZE_10MB,

            // === ARCHIVES: Uncompressed archives benefit from chunking (5MB) ===
            "tar" | "cpio" | "iso" | "dmg" => true,

            // === PRE-COMPRESSED: Never chunk (no benefit) ===
            "jpg" | "jpeg" | "png" | "webp" | "gif" | "avif" | "heic" |
            "mp3" | "aac" | "ogg" | "opus" |
            "zip" | "gz" | "bz2" | "xz" | "7z" | "rar" | "tar.gz" | "tgz" => false,

            // === UNKNOWN: Conservative threshold (10MB) ===
            _ => size > MIN_SIZE_10MB,
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
