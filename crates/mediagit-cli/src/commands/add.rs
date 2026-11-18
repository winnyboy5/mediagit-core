// SPDX-License-Identifier: BUSL-1.1
// Copyright (C) 2025-2026 Aswin Krishnamoorthy

//! Stage file contents for commit.
//!
//! The `add` command stages changes to files for inclusion in the next commit.

use super::super::progress::ProgressTracker;
use super::super::repo::{create_storage_backend, find_repo_root};
use anyhow::{Context, Result};
use clap::Parser;
use mediagit_versioning::{
    ChunkStrategy, Commit, Index, IndexEntry, ObjectDatabase, ObjectType, Oid, RefDatabase, Tree,
};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

/// Stage files for commit (reusable by other commands like commit --include)
///
/// This function performs the same staging logic as the add command, allowing
/// other commands to stage files without reimplementing the chunking/hashing/delta logic.
pub async fn stage_files_for_commit(paths: &[String], repo_root: &std::path::Path) -> Result<u64> {
    if paths.is_empty() {
        return Ok(0);
    }

    let repo_root =
        dunce::canonicalize(repo_root).unwrap_or_else(|_| std::path::PathBuf::from(repo_root));

    // Initialize storage backend
    let storage_path = repo_root.join(".mediagit");
    let storage = create_storage_backend(&repo_root).await?;
    let odb =
        ObjectDatabase::with_optimizations(storage, 1000, Some(ChunkStrategy::MediaAware), true, 0);

    // Load index
    let mut index = Index::load(&repo_root)?;

    // Load the base tree for unchanged detection
    let head_files = Arc::new(base_files(&repo_root, &storage_path, &odb).await?);

    let index_files: Arc<IndexStat> = {
        let mut map = std::collections::HashMap::new();
        for entry in index.entries() {
            map.insert(entry.path.clone(), (entry.size, entry.mtime, entry.oid));
        }
        Arc::new(map)
    };

    // A staged deletion is never "unchanged": a present file there is re-staged.
    let unresolved: HashSet<PathBuf> = index
        .unresolved_paths()
        .into_iter()
        .chain(index.deleted_paths().cloned())
        .collect();
    let stat = std::sync::Mutex::new(mediagit_versioning::StatCache::load(&repo_root));

    // Stage each path sequentially
    let mut added_count = 0u64;
    for path_str in paths {
        // nosemgrep: rust.actix.path-traversal.tainted-path.tainted-path -- local CLI arg
        let path = std::path::Path::new(path_str);
        if path.is_file() {
            // Never skip an existing file just because it cannot be
            // canonicalized (BUG-009): fall back to the plain absolute path.
            let abs_path = dunce::canonicalize(path).or_else(|_| std::path::absolute(path))?;
            match AddCmd::process_single_file(
                &abs_path,
                &repo_root,
                &odb,
                &head_files,
                &index_files,
                &unresolved,
                &stat,
                true,
                None,
            )
            .await
            {
                Ok((Some(file_result), _)) => {
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
                Ok((None, _)) => {} // unchanged, skip
                Err(e) => {
                    crate::phash_index::flush(&repo_root).await;
                    return Err(e);
                }
            }
        }
    }
    crate::phash_index::flush(&repo_root).await;

    // Save index
    index.save(&repo_root).context("Failed to save index")?;
    let _ = stat.lock().unwrap().save();

    Ok(added_count)
}

/// Files of the tree the next commit builds on, for "unchanged" detection and
/// deletions. That is HEAD, except during a rebase: `--continue` commits on
/// top of the rebase's new parent while HEAD still names the branch being
/// moved, so comparing with HEAD skipped edits that differ from the new parent.
async fn base_files(
    repo_root: &Path,
    storage_path: &Path,
    odb: &ObjectDatabase,
) -> Result<HashMap<PathBuf, Oid>> {
    let base = if super::rebase_state::RebaseState::in_progress(repo_root) {
        Some(super::rebase_state::RebaseState::load(repo_root)?.new_parent)
    } else {
        RefDatabase::new(storage_path).resolve("HEAD").await.ok()
    };
    let mut files = HashMap::new();
    if let Some(base_oid) = base
        && let Ok(commit_data) = odb.read(&base_oid).await
        && let Ok(commit) = mediagit_versioning::format::deserialize_exact::<Commit>(&commit_data)
        && let Ok(tree_data) = odb.read(&commit.tree).await
        && let Ok(tree) = mediagit_versioning::format::deserialize_exact::<Tree>(&tree_data)
    {
        for entry in tree.iter() {
            files.insert(PathBuf::from(&entry.name), entry.oid);
        }
    }
    Ok(files)
}

/// Per index path: size, mtime (stat cache) and staged oid.
type IndexStat = HashMap<PathBuf, (u64, Option<u64>, Oid)>;

/// What the next commit would hold for `path` today: the staged entry if any,
/// else the base tree's. "Unchanged" means equal to this, not just to the base:
/// an index entry that differs from the base must be overwritten.
fn staged_or_base(
    index_files: &IndexStat,
    base: &HashMap<PathBuf, Oid>,
    path: &Path,
) -> Option<Oid> {
    index_files
        .get(path)
        .map(|e| e.2)
        .or_else(|| base.get(path).copied())
}

/// `--machine` data for `add`.
#[derive(Debug, Clone, serde::Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[cfg_attr(test, ts(export, export_to = "../../../ui/src/api/generated/"))]
pub struct AddReport {
    /// New files staged.
    #[cfg_attr(test, ts(type = "number"))]
    pub added: u64,
    /// Files already in HEAD that were re-staged with changes.
    #[cfg_attr(test, ts(type = "number"))]
    pub modified: u64,
    /// Deletions staged.
    #[cfg_attr(test, ts(type = "number"))]
    pub deleted: u64,
    /// Total size of the staged files.
    #[cfg_attr(test, ts(type = "number"))]
    pub bytes: u64,
    /// Unsafe (absolute or `..`) paths found in the staging area or the last
    /// version and replaced or removed (BUG-008). Empty on a healthy repository.
    pub repaired_unsafe_paths: Vec<String>,
}

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

    /// Show what would be staged
    #[arg(long)]
    pub dry_run: bool,

    /// Force add even if listed in .mediagitignore
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

/// Default global in-flight byte budget for parallel file processing (512 MiB).
/// Override via `MEDIAGIT_ADD_MAX_INFLIGHT_BYTES`.
const DEFAULT_ADD_MAX_INFLIGHT_BYTES: u64 = 512 * 1024 * 1024;

fn add_max_inflight_bytes() -> u64 {
    match std::env::var("MEDIAGIT_ADD_MAX_INFLIGHT_BYTES") {
        Ok(v) => v.parse::<u64>().unwrap_or_else(|_| {
            tracing::warn!(
                "MEDIAGIT_ADD_MAX_INFLIGHT_BYTES='{}' is not a valid u64, using default {}",
                v,
                DEFAULT_ADD_MAX_INFLIGHT_BYTES
            );
            DEFAULT_ADD_MAX_INFLIGHT_BYTES
        }),
        Err(_) => DEFAULT_ADD_MAX_INFLIGHT_BYTES,
    }
}

/// A file `add` cannot record under a repo-relative path (BUG-008/009).
/// Carried as an `anyhow` error so the add loops can name it, carry on with
/// the other files, and still exit non-zero at the end.
#[derive(Debug)]
struct Untrackable(String);

impl std::fmt::Display for Untrackable {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Untrackable {}

/// Windows drops a trailing dot or space when it opens a name, so such a file
/// cannot be read back by its own name and checkout would recreate it under a
/// different one. Returns the reason in plain words.
/// Which names a folder really holds, read once per folder (BUG-011).
///
/// `Path::exists` asks the filesystem, which on Windows and default macOS
/// answers yes for `art/Hero.txt` when the file is now called `art/hero.txt`.
/// Deleted-or-not must be decided by the exact stored name, folder by folder.
#[derive(Default)]
struct ExactNames {
    folders:
        std::collections::HashMap<PathBuf, Option<std::collections::HashSet<std::ffi::OsString>>>,
}

/// Only these mean "the folder is not there". Any other read error (access
/// denied, too many open files, a dropped network drive) says nothing about
/// the files in it, so it must never be read as "they were deleted".
fn folder_is_absent(e: &std::io::Error) -> bool {
    use std::io::ErrorKind::{NotADirectory, NotFound};
    // 267 is ERROR_DIRECTORY: a file stands where the folder was.
    matches!(e.kind(), NotFound | NotADirectory) || e.raw_os_error() == Some(267)
}

impl ExactNames {
    /// True when every component of `rel` is present under `root` with
    /// exactly this spelling. A missing folder holds nothing; a folder that
    /// cannot be read is an error naming it.
    fn exists_exact(&mut self, root: &Path, rel: &Path) -> Result<bool> {
        let mut dir = root.to_path_buf();
        for part in rel.components() {
            let name = part.as_os_str();
            if !self.folders.contains_key(&dir) {
                let listing = match std::fs::read_dir(&dir) {
                    Ok(rd) => Some(
                        rd.map(|e| e.map(|e| e.file_name()))
                            .collect::<std::io::Result<_>>()
                            .map_err(|e| unreadable_folder(&dir, &e))?,
                    ),
                    Err(e) if folder_is_absent(&e) => None,
                    Err(e) => return Err(unreadable_folder(&dir, &e)),
                };
                self.folders.insert(dir.clone(), listing);
            }
            let known = self.folders[&dir]
                .as_ref()
                .is_some_and(|n| n.contains(name));
            if !known {
                return Ok(false);
            }
            dir.push(name);
        }
        Ok(true)
    }
}

fn unreadable_folder(dir: &Path, e: &std::io::Error) -> anyhow::Error {
    anyhow::anyhow!(
        "Cannot read the folder '{}' ({e}), so `add` cannot tell which of its files were \
         deleted. Nothing was staged. Fix the folder's permissions and run `add` again.",
        dir.display()
    )
}

fn windows_unstorable_name(path: &Path) -> Option<&'static str> {
    let name = path.file_name()?.to_string_lossy();
    (name.ends_with('.') || name.ends_with(' '))
        .then_some("Windows cannot keep a name that ends in a dot or a space; rename the file")
}

/// What `add` did about an unsafe path found in the staging area or the last
/// version: the file is re-added under its own name if it is still in the
/// working folder (found by matching the tail of the bad path), else the path
/// is simply removed from the next version.
fn describe_unsafe_repair(bad: &str, repo_root: &Path) -> String {
    let parts: Vec<&str> = bad
        .split(['/', '\\'])
        .filter(|p| !p.is_empty() && *p != "..")
        .collect();
    for i in 0..parts.len() {
        let tail = parts[i..].join("/");
        if repo_root.join(&tail).is_file() {
            return format!("Replaced unsafe path {bad} with {tail}");
        }
    }
    format!(
        "Removed unsafe path {bad} from the next version (its file is not in the working folder)"
    )
}

/// If `e` is an [`Untrackable`], warn (unless quiet), remember it and return
/// true so the caller moves on to the next file.
fn note_untrackable(e: &anyhow::Error, list: &mut Vec<String>, quiet: bool) -> bool {
    let Some(u) = e.downcast_ref::<Untrackable>() else {
        return false;
    };
    if !quiet {
        crate::output::warning(&format!("Cannot track {u}"));
    }
    list.push(u.to_string());
    true
}

/// Result from processing a single file in parallel
pub struct FileResult {
    pub relative_path: PathBuf,
    pub oid: Oid,
    pub file_size: u64,
    pub mode: u32,
    pub mtime: Option<u64>,
}

impl AddCmd {
    pub async fn execute(&self) -> Result<()> {
        use crate::output;

        // Validate: either --all, --update, or paths must be provided
        if !self.all && !self.update && self.paths.is_empty() {
            anyhow::bail!(
                "Nothing specified, nothing added.\nUse 'mediagit add <file>...' or 'mediagit add --all' to stage files."
            );
        }

        // Find repository root and canonicalize to match the canonicalized file
        // paths produced by collect_files_recursive (which uses dunce::canonicalize).
        // Without this, strip_prefix fails on Windows when the working directory
        // path differs from the canonicalized form (e.g. 8.3 short names), causing
        // files to be stored with absolute paths in the index/tree.
        let repo_root = dunce::canonicalize(find_repo_root()?)
            .unwrap_or_else(|_| find_repo_root().expect("repo root"));

        if self.dry_run {
            output::info("Running in dry-run mode");
        }

        if !self.quiet && !self.dry_run {
            output::progress("Staging files...");
        }

        let _add_wall = std::time::Instant::now();

        // Initialize storage backend from config (supports S3, Azure, GCS, filesystem)
        let storage_path = repo_root.join(".mediagit");
        let storage = create_storage_backend(&repo_root).await?;

        // MEDIAGIT_DELTA_ENABLED was read into `RepoConfig` (config.rs:149) and
        // never consulted here, so `add` — the one command whose cost it
        // governs — ignored it entirely. That is not a cosmetic gap: chunk-delta
        // is 96% of `add`'s wall on a large PSD (measured 95.79s with, 0.59s
        // with --no-delta), so an A/B run with the env var set produced two
        // identical measurements while appearing to be a controlled comparison.
        // Same dead-knob class as MEDIAGIT_CHUNK_WRITE_CONCURRENCY on the
        // streaming path and GCS_UPLOAD_CONCURRENCY on the presigned path.
        //
        // Either switch turns delta OFF; neither can force it back ON past the
        // other. An env var that overrode an explicit `--no-delta` would be a
        // surprise in the dangerous direction.
        let env_delta_off = std::env::var("MEDIAGIT_DELTA_ENABLED").as_deref() == Ok("0")
            || std::env::var("MEDIAGIT_DELTA_ENABLED").as_deref() == Ok("false");
        let delta_enabled = !self.no_delta && !env_delta_off;
        let chunk_strategy = if self.no_chunking {
            None
        } else {
            Some(ChunkStrategy::MediaAware)
        };

        // Repo's persisted CDC seed (0 for repos without the field / legacy repos).
        // `MEDIAGIT_CDC_SEED` env var overrides this inside the chunker itself.
        //
        // Not `.unwrap_or(0)`: falling back to 0 on an unreadable config would
        // chunk this repo's content on DIFFERENT boundaries than every object
        // already in it, silently destroying dedup against its own history. An
        // absent config.toml already yields the default (seed 0) through
        // `Config::load` itself, so an `Err` here means the file exists and is
        // broken — which is a stop, not a shrug.
        let cdc_seed = mediagit_config::Config::load(&repo_root).await?.cdc_seed;

        let odb = ObjectDatabase::with_optimizations(
            storage,
            1000,
            chunk_strategy,
            delta_enabled,
            cdc_seed,
        );

        if !self.quiet && self.verbose {
            output::info("Auto-chunking enabled for large files");
        }

        // Load the index
        let mut index = Index::load(&repo_root)?;

        // BUG-008 way out: an index entry left by an older release (absolute or
        // `..` path) is dropped here; the file, if it still exists, is re-added
        // below under its repo-relative path.
        let unsafe_staged: Vec<PathBuf> = index
            .entries()
            .map(|e| e.path.clone())
            .filter(|p| mediagit_versioning::check_tracked_path(&p.to_string_lossy()).is_err())
            .collect();
        // Data-affecting, so named even under --quiet; --machine gets the list
        // in `repaired_unsafe_paths`.
        let mut repaired: std::collections::BTreeMap<String, String> =
            std::collections::BTreeMap::new();
        for p in &unsafe_staged {
            let note = describe_unsafe_repair(&p.to_string_lossy(), &repo_root);
            output::warning(&note);
            repaired.insert(p.to_string_lossy().into_owned(), note);
            index.remove_entry(p);
        }

        // Build index lookup for stat-cache change detection (size + mtime)
        let index_files: Arc<IndexStat> = {
            let mut map = HashMap::new();
            for entry in index.entries() {
                map.insert(entry.path.clone(), (entry.size, entry.mtime, entry.oid));
            }
            Arc::new(map)
        };

        // BUG-029: an unresolved path named directly as a file is always
        // re-staged: naming it is the decision. A folder or `.` walk is not,
        // so it never settles a clash whose bytes happen to equal ours.
        let all_unresolved: HashSet<PathBuf> = index.unresolved_paths().into_iter().collect();
        let unresolved: Arc<HashSet<PathBuf>> = Arc::new(
            self.paths
                .iter()
                .map(Path::new)
                .filter(|p| p.is_file())
                .filter_map(|p| dunce::canonicalize(p).ok())
                .filter_map(|p| crate::repo::repo_relative(&repo_root, &p).ok())
                .filter(|p| all_unresolved.contains(p))
                // A staged deletion is never "unchanged": a present file there
                // is re-staged, however it was named.
                .chain(index.deleted_paths().cloned())
                .collect(),
        );

        // The tree the next commit builds on, to identify already-tracked files
        let head_files = Arc::new(base_files(&repo_root, &storage_path, &odb).await?);

        // BUG-039: the index is empty after a commit, so size+mtime against it
        // cannot skip anything. The stat cache (the one `status` fills) can.
        let stat = Arc::new(std::sync::Mutex::new(mediagit_versioning::StatCache::load(
            &repo_root,
        )));

        // Tracked paths = union of HEAD tree paths and current index paths.
        // Used by `-u`/`--update` to restrict staging to already-tracked
        // files (never untracked ones) — see expand_paths.
        let tracked_paths: std::collections::HashSet<PathBuf> = head_files
            .keys()
            .cloned()
            .chain(index_files.keys().cloned())
            .collect();

        // Expand paths (globs, directories) into file list
        let (files_to_add, refused) = self.expand_paths(&repo_root, &tracked_paths)?;
        // Files the walk refused up front (Windows-unstorable names, symlinks
        // that leave the repository): named now, counted into the exit status.
        let mut untrackable: Vec<String> = Vec::new();
        for msg in refused {
            if !self.quiet {
                output::warning(&format!("Cannot track {msg}"));
            }
            untrackable.push(msg);
        }

        // Calculate total bytes for progress bar. Statting every file up
        // front can take a while on large trees with no feedback yet, so
        // show a spinner for this scan phase (separate from the byte/file
        // progress bar created below).
        let (total_files, total_bytes) = if !self.quiet && !files_to_add.is_empty() {
            let scan_tracker = ProgressTracker::new(self.quiet);
            let scan_spinner =
                scan_tracker.spinner(&format!("scanning {} files…", files_to_add.len()));
            let mut bytes = 0u64;
            for f in &files_to_add {
                if let Ok(meta) = std::fs::metadata(f) {
                    bytes += meta.len();
                }
            }
            scan_spinner.finish_and_clear();
            (files_to_add.len() as u64, bytes)
        } else {
            (files_to_add.len() as u64, 0)
        };

        // Create progress bar
        // The tracker must outlive the work: dropping it stops the `--machine`
        // progress sampler.
        let tracker = ProgressTracker::new(self.quiet);
        let (progress_bar, items_bar) = if !self.dry_run && total_files > 0 {
            let pb = tracker.add_bar(&format!("0/{} files", total_files), total_bytes);
            let ib = tracker.object_bar("files", total_files);
            (Some(Arc::new(pb)), Some(Arc::new(ib)))
        } else {
            (None, None)
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
        let mut modified_count = 0u64;
        let mut staged_bytes = 0u64;
        let mut skipped_count = 0u64;

        if !self.dry_run && !files_to_add.is_empty() {
            if use_parallel {
                // --- PARALLEL FILE PROCESSING ---
                let max_concurrent = self.jobs.unwrap_or_else(|| num_cpus::get().min(8));
                let semaphore = Arc::new(tokio::sync::Semaphore::new(max_concurrent));

                // Global in-flight byte budget: the file-count semaphore above
                // admits up to `max_concurrent` files regardless of size, so
                // e.g. 8 concurrent 300MB in-memory reads can spike RAM well
                // past what any single-file threshold suggests. Permits are
                // granted in 1 MiB units; a file larger than the whole budget
                // is capped to `total_byte_permits` so it still runs (alone)
                // instead of deadlocking.
                const BYTES_PER_PERMIT: u64 = 1024 * 1024;
                let total_byte_permits = (add_max_inflight_bytes() / BYTES_PER_PERMIT)
                    .max(1)
                    .min(u32::MAX as u64) as u32;
                let byte_semaphore =
                    Arc::new(tokio::sync::Semaphore::new(total_byte_permits as usize));

                let mut file_tasks = tokio::task::JoinSet::new();
                let skipped = Arc::new(AtomicU64::new(0));

                #[allow(clippy::unnecessary_to_owned)]
                for file_path in files_to_add.iter().cloned() {
                    let sem = semaphore.clone();
                    let byte_semaphore = byte_semaphore.clone();
                    let odb = odb.clone();
                    let head_files = head_files.clone();
                    let index_files = index_files.clone();
                    let stat = stat.clone();
                    let unresolved = unresolved.clone();
                    let repo_root = repo_root.clone();
                    let progress_bytes = progress_bytes.clone();
                    let progress_files = progress_files.clone();
                    let progress_bar = progress_bar.clone();
                    let items_bar = items_bar.clone();
                    let skipped = skipped.clone();

                    file_tasks.spawn(async move {
                        let _permit = sem
                            .acquire()
                            .await
                            .map_err(|_| anyhow::anyhow!("Semaphore closed"))?;

                        let file_size = tokio::fs::metadata(&file_path)
                            .await
                            .map(|m| m.len())
                            .unwrap_or(0);
                        let needed_permits = (file_size.div_ceil(BYTES_PER_PERMIT).max(1) as u32)
                            .min(total_byte_permits);
                        let _byte_permit = byte_semaphore
                            .acquire_many_owned(needed_permits)
                            .await
                            .map_err(|_| anyhow::anyhow!("Byte semaphore closed"))?;

                        // Per-chunk/file byte callback: updates progress_bytes + bar position
                        // Called incrementally for large streaming files (per chunk),
                        // once for small in-memory files, and once for skipped files.
                        let on_bytes: Option<Arc<dyn Fn(u64) + Send + Sync>> =
                            if let Some(ref pb) = progress_bar {
                                let pb = pb.clone();
                                let pb_bytes = progress_bytes.clone();
                                Some(Arc::new(move |n: u64| {
                                    let total = pb_bytes.fetch_add(n, Ordering::Relaxed) + n;
                                    pb.set_position(total);
                                }))
                            } else {
                                None
                            };

                        let result = Self::process_single_file(
                            &file_path,
                            &repo_root,
                            &odb,
                            &head_files,
                            &index_files,
                            &unresolved,
                            &stat,
                            delta_enabled,
                            on_bytes,
                        )
                        .await;

                        match result {
                            Ok((Some(file_result), _)) => {
                                let done = progress_files.fetch_add(1, Ordering::Relaxed) + 1;
                                if let Some(ref ib) = items_bar {
                                    ib.set_position(done);
                                }
                                if let Some(ref pb) = progress_bar {
                                    pb.set_message(format!("{}/{} files", done, total_files));
                                }
                                Ok(Some(file_result))
                            }
                            Ok((None, _)) => {
                                skipped.fetch_add(1, Ordering::Relaxed);
                                let done = progress_files.fetch_add(1, Ordering::Relaxed) + 1;
                                if let Some(ref ib) = items_bar {
                                    ib.set_position(done);
                                }
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
                            staged_bytes += file_result.file_size;
                            if head_files.contains_key(&file_result.relative_path) {
                                modified_count += 1;
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
                        }
                        Ok(Ok(None)) => {
                            // Skipped
                        }
                        Ok(Err(e)) => {
                            if note_untrackable(&e, &mut untrackable, self.quiet) {
                                continue;
                            }
                            if !self.force {
                                // The stored images' pHashes outlive a failed run.
                                crate::phash_index::flush(&repo_root).await;
                                return Err(e);
                            }
                            if !self.quiet {
                                output::warning(&format!("Error staging file: {}", e));
                            }
                        }
                        Err(e) => {
                            if !self.force {
                                crate::phash_index::flush(&repo_root).await;
                                return Err(anyhow::anyhow!("Task panicked: {}", e));
                            }
                        }
                    }
                }

                skipped_count = skipped.load(Ordering::Relaxed);
            } else {
                // --- SEQUENTIAL FILE PROCESSING (fallback / --no-parallel) ---
                // Create callback once; it updates progress_bytes + bar position for every
                // file (including skipped) and every chunk of large streaming files.
                let on_bytes_seq: Option<Arc<dyn Fn(u64) + Send + Sync>> =
                    if let Some(ref pb) = progress_bar {
                        let pb = pb.clone();
                        let pb_bytes = progress_bytes.clone();
                        Some(Arc::new(move |n: u64| {
                            let total = pb_bytes.fetch_add(n, Ordering::Relaxed) + n;
                            pb.set_position(total);
                        }))
                    } else {
                        None
                    };

                for file_path in &files_to_add {
                    let result = Self::process_single_file(
                        file_path,
                        &repo_root,
                        &odb,
                        &head_files,
                        &index_files,
                        &unresolved,
                        &stat,
                        delta_enabled,
                        on_bytes_seq.clone(),
                    )
                    .await;

                    match result {
                        Ok((Some(file_result), _)) => {
                            if self.verbose {
                                output::detail(
                                    "added",
                                    &format!("{} ({})", file_path.display(), file_result.oid),
                                );
                            }

                            staged_bytes += file_result.file_size;
                            if head_files.contains_key(&file_result.relative_path) {
                                modified_count += 1;
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

                            let done = progress_files.fetch_add(1, Ordering::Relaxed) + 1;
                            if let Some(ref ib) = items_bar {
                                ib.set_position(done);
                            }
                            if let Some(ref pb) = progress_bar {
                                pb.set_message(format!("{}/{} files", done, total_files));
                            }
                        }
                        Ok((None, _)) => {
                            let done = progress_files.fetch_add(1, Ordering::Relaxed) + 1;
                            if let Some(ref ib) = items_bar {
                                ib.set_position(done);
                            }
                            if let Some(ref pb) = progress_bar {
                                pb.set_message(format!("{}/{} files", done, total_files));
                            }
                            skipped_count += 1;
                            if self.verbose {
                                output::detail(
                                    "skipped (unchanged)",
                                    &file_path.display().to_string(),
                                );
                            }
                        }
                        Err(e) => {
                            if note_untrackable(&e, &mut untrackable, self.quiet) {
                                continue;
                            }
                            if !self.force {
                                // The stored images' pHashes outlive a failed run.
                                crate::phash_index::flush(&repo_root).await;
                                return Err(e);
                            }
                            if !self.quiet {
                                output::warning(&format!("Error staging file: {}", e));
                            }
                        }
                    }
                }
            }
            crate::phash_index::flush(&repo_root).await;
        } else if self.dry_run {
            added_count = files_to_add.len() as u64;
        }

        // Detect deleted files: files in HEAD but not in working directory.
        // Git semantics: deletions are staged only when the pathspec covers
        // them — `add -A`/`-u`/`add .` sweep the repo, while `add <file>`
        // must not stage unrelated deletions.
        let mut deleted_count = 0;

        // '/'-normalized once so the per-HEAD-path check below is a set lookup,
        // not a scan of every working file (BUG-039).
        let working_files: std::collections::HashSet<String> = files_to_add
            .iter()
            .filter_map(|p| crate::repo::repo_relative(&repo_root, p).ok())
            .map(|p| p.to_string_lossy().replace('\\', "/"))
            .collect();

        // Pathspec prefixes (repo-relative, '/'-normalized) that scope which
        // deletions may be staged when neither -A nor -u is given.
        let deletion_scopes: Vec<String> = if self.all || self.update {
            vec![String::new()] // whole repo
        } else {
            self.paths
                .iter()
                .map(|p| {
                    let joined = if Path::new(p).is_absolute() {
                        PathBuf::from(p)
                    } else {
                        std::env::current_dir().unwrap_or_default().join(p)
                    };
                    // The repository folder itself is a valid scope: everything.
                    crate::repo::repo_scope(&repo_root, &joined).unwrap_or_else(|| {
                        let s = p.replace('\\', "/");
                        if s == "." { String::new() } else { s }
                    })
                })
                .collect()
        };

        // Sparse-excluded paths are absent by design, not deleted (BUG-041).
        let sparse = mediagit_versioning::SparseFilter::load(&repo_root)
            .context("Failed to load sparse-checkout patterns")?;

        let mut on_disk = ExactNames::default();
        // A clashing path the base lacks (we deleted, they edited) holds a
        // provisional staged copy; deleting it from disk is a decision too.
        let unresolved_new = all_unresolved
            .iter()
            .filter(|p| !head_files.contains_key(*p));
        // F2: so is a staged path the base lacks (the pick added it cleanly).
        let staged_new = index_files.keys().filter(|p| !head_files.contains_key(*p));
        for head_path in head_files
            .as_ref()
            .keys()
            .chain(unresolved_new)
            .chain(staged_new)
        {
            // BUG-008 way out: the last commit holds an absolute or `..` path.
            // Stage its removal whatever the pathspec or --ignore-removal; the
            // file itself, if present, was re-added above under its real path.
            if mediagit_versioning::check_tracked_path(&head_path.to_string_lossy()).is_err() {
                let key = head_path.to_string_lossy().into_owned();
                if let std::collections::btree_map::Entry::Vacant(slot) = repaired.entry(key) {
                    let note = describe_unsafe_repair(slot.key(), &repo_root);
                    output::warning(&note);
                    slot.insert(note);
                }
                if !self.dry_run {
                    index.mark_deleted(head_path.clone());
                }
                deleted_count += 1;
                continue;
            }
            if !sparse.is_included(head_path) {
                continue;
            }
            let head_path_normalized =
                PathBuf::from(head_path.to_string_lossy().replace('\\', "/"));

            let head_str = head_path_normalized.to_string_lossy().to_string();
            let in_scope = deletion_scopes.iter().any(|scope| {
                scope.is_empty()
                    || head_str == *scope
                    || head_str.starts_with(&format!("{}/", scope.trim_end_matches('/')))
            });
            if !in_scope {
                continue;
            }

            let exists_in_working_dir = working_files.contains(&head_str);

            // Stage deletion unless --ignore-removal is set
            if !exists_in_working_dir
                && !self.ignore_removal
                && !on_disk.exists_exact(&repo_root, head_path)?
            {
                if !self.dry_run {
                    index.mark_deleted(head_path.clone());
                    if self.verbose {
                        output::detail("deleted", &head_path.display().to_string());
                    }
                }
                deleted_count += 1;
            }
        }

        // Finish progress bar
        if let Some(pb) = progress_bar {
            pb.finish_and_clear();
        }
        if let Some(ib) = items_bar {
            ib.finish_and_clear();
        }
        drop(tracker);

        // Save the index
        if !self.dry_run {
            index.save(&repo_root).context("Failed to save index")?;
            let _ = stat.lock().unwrap().save();
        }

        if !self.quiet {
            if added_count > 0 {
                let msg = if self.dry_run {
                    format!("Would stage {} file(s)", added_count)
                } else {
                    format!("Staged {} file(s)", added_count)
                };
                output::success(&msg);
            }
            if deleted_count > 0 {
                let msg = if self.dry_run {
                    format!("Would stage {} deletion(s)", deleted_count)
                } else {
                    format!("Staged {} deletion(s)", deleted_count)
                };
                output::success(&msg);
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

        if !untrackable.is_empty() {
            anyhow::bail!(
                "{} file(s) could not be tracked and were not staged:\n  {}",
                untrackable.len(),
                untrackable.join("\n  ")
            );
        }

        // Best-effort auto-gc after add. Re-staging a file before commit
        // orphans the previous version's chunks immediately; the threshold
        // gate (50 MiB or 100 orphans) makes trivial adds a no-op.
        if !self.dry_run && added_count > 0 {
            // Timed separately because `wall` below includes it. An `add` that
            // happened to trip the gc threshold looked like a slow `add`, which
            // is a plausible way for a perf claim to be wrong for a whole cycle.
            let gc_timer = std::time::Instant::now();
            let _ =
                crate::auto_gc::maybe_run(&repo_root, crate::auto_gc::TriggerMode::PostAdd).await;
            mediagit_versioning::add_phases::record(
                mediagit_versioning::add_phases::Phase::AutoGc,
                gc_timer.elapsed(),
            );
        }

        mediagit_protocol::bench::emit_add_summary(_add_wall, total_bytes, added_count);

        if output::machine_mode() {
            return output::emit_ok(&AddReport {
                added: added_count - modified_count,
                modified: modified_count,
                deleted: deleted_count,
                bytes: staged_bytes,
                repaired_unsafe_paths: repaired.values().cloned().collect(),
            });
        }

        Ok(())
    }

    /// Process a single file: hash, check HEAD, write to ODB
    ///
    /// Returns `Ok(Some(FileResult))` if file was staged,
    /// `Ok(None)` if file was skipped (unchanged from HEAD),
    /// `Err` on failure.
    #[allow(clippy::too_many_arguments)] // one shared per-file worker; a params struct would be churn
    pub async fn process_single_file(
        file_path: &Path,
        repo_root: &Path,
        odb: &ObjectDatabase,
        head_files: &HashMap<PathBuf, Oid>,
        index_files: &IndexStat,
        unresolved: &HashSet<PathBuf>,
        stat: &std::sync::Mutex<mediagit_versioning::StatCache>,
        delta_enabled: bool,
        on_bytes: Option<Arc<dyn Fn(u64) + Send + Sync>>,
    ) -> Result<(Option<FileResult>, u64)> {
        let metadata = tokio::fs::metadata(file_path).await.map_err(|e| {
            anyhow::Error::new(Untrackable(format!(
                "{}: cannot read file metadata ({e})",
                file_path.display()
            )))
        })?;

        let file_size = metadata.len();
        // Aligns with should_use_chunking() minimum.
        // Files ≥ STREAMING_THRESHOLD go through write_chunked_from_file() which uses
        // StreamCDC for most formats.  Only container formats with dedicated
        // structure-aware parsers (MP4, MKV, AVI, GLB, FBX, …) use mmap + format-aware
        // chunking; all others stay on StreamCDC for maximum delta-compressibility.
        use super::utils::STREAMING_THRESHOLD;

        // BUG-008: one helper, an error instead of falling back to the
        // absolute path.
        let relative_path = if file_path.is_absolute() {
            crate::repo::repo_relative(repo_root, file_path)
        } else {
            crate::repo::normalize_path(file_path, repo_root)
        }
        .map_err(|e| anyhow::Error::new(Untrackable(format!("{e:#}"))))?;

        // Stat-cache check: skip if file hasn't changed since last staging
        let file_mtime = metadata
            .modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_secs());

        // BUG-029: naming a conflicted path is the decision, even when its
        // bytes equal HEAD's, so it never takes the "unchanged" exits below.
        let conflicted = unresolved.contains(&relative_path);

        if !conflicted
            && let Some(&(idx_size, Some(idx_mtime), _)) = index_files.get(&relative_path)
            && idx_size == file_size
            && let Some(current_mtime) = file_mtime
            && idx_mtime == current_mtime
        {
            if let Some(ref cb) = on_bytes {
                cb(file_size);
            }
            return Ok((None, file_size)); // Unchanged since last staging
        }

        // BUG-039: the stat cache answers "what does this file hash to" without
        // reading it (size, mtime, file id, change time and the racy margin all
        // have to match). Equal to what the next commit holds = unchanged.
        let rel_key = relative_path.to_string_lossy().into_owned();
        let sym_meta = tokio::fs::symlink_metadata(file_path).await.ok();
        if !conflicted
            && let Some(m) = &sym_meta
            && let Some(cached) = stat.lock().unwrap().lookup(&rel_key, m)
            && staged_or_base(index_files, head_files, &relative_path) == Some(cached)
        {
            if let Some(ref cb) = on_bytes {
                cb(file_size);
            }
            return Ok((None, file_size));
        }

        let filename = file_path.file_name().and_then(|n| n.to_str()).unwrap_or("");

        // Seed similarity detector from previous version (manifest for chunked, blob for small)
        let mut positional = None;
        if let Some(head_oid) = head_files.get(&relative_path) {
            match odb.get_chunk_manifest(head_oid).await {
                Ok(Some(old_manifest)) => {
                    let _ = odb.seed_similarity_from_manifest(&old_manifest).await;
                    if mediagit_versioning::tensor_delta_enabled()
                        && filename.to_lowercase().ends_with(".safetensors")
                    {
                        positional = odb.positional_base_from_manifest(&old_manifest).await;
                    }
                }
                _ => {
                    if delta_enabled {
                        // Non-chunked file: seed from the previous full blob so write_with_delta
                        // can find a similar base across invocations.  We seed regardless of
                        // whether the current file will ultimately use delta, because we haven't
                        // read the file yet and the actual delta decision is made later.
                        let _ = odb.seed_similarity_from_blob(head_oid, filename).await;
                    }
                }
            }
        }

        // Choose streaming vs in-memory based on file size
        let (_content_oid, oid) = if file_size >= STREAMING_THRESHOLD {
            // STREAMING PATH: Files >= 5MB — format-aware chunking via mmap (parallel internally)
            // MEDIAGIT_HASH_PARALLEL: default ON. Set to "0" to force sequential.
            // Falls back to sequential hash automatically when mmap is unavailable
            // (e.g. network FS, locked file, 32-bit address exhaustion).
            // PERF-V10-PSD: this is the FIRST of two walks over the file — the
            // whole-file dedup OID here, then StreamCDC reads it again to chunk
            // it. The second walk is the work itself; this one is the candidate
            // for removal (the streaming pass already hashes every chunk). It
            // is only worth removing if it is a material fraction, hence the
            // measurement before the optimisation. Covers both branches, so a
            // machine where mmap falls back to sequential is still measured.
            let _oid_timer = std::time::Instant::now();
            let content_oid = if std::env::var("MEDIAGIT_HASH_PARALLEL").as_deref() != Ok("0") {
                let path_owned = file_path.to_path_buf();
                match tokio::task::spawn_blocking(move || Oid::from_file_mmap_parallel(&path_owned))
                    .await
                    .map_err(|e| anyhow::anyhow!("hash parallel join: {}", e))
                {
                    Ok(Ok(oid)) => oid,
                    _ => Oid::from_file_async(file_path)
                        .await
                        .context(format!("Failed to hash file: {}", file_path.display()))?,
                }
            } else {
                Oid::from_file_async(file_path)
                    .await
                    .context(format!("Failed to hash file: {}", file_path.display()))?
            };
            mediagit_versioning::add_phases::record(
                mediagit_versioning::add_phases::Phase::Oid,
                _oid_timer.elapsed(),
            );

            if let Some(m) = &sym_meta {
                stat.lock().unwrap().record(&rel_key, m, content_oid);
            }

            // Check if unchanged from HEAD
            if !conflicted
                && staged_or_base(index_files, head_files, &relative_path) == Some(content_oid)
            {
                if let Some(ref cb) = on_bytes {
                    cb(file_size);
                }
                return Ok((None, file_size));
            }

            let oid = odb
                .write_chunked_from_file_positional(
                    file_path,
                    filename,
                    on_bytes.clone(),
                    Some(content_oid),
                    positional,
                )
                .await
                .context("Failed to write chunked object (streaming)")?;

            (content_oid, oid)
        } else {
            // IN-MEMORY PATH: Files < 5MB — read fully into memory, then hash/delta/write
            // nosemgrep: rust.actix.path-traversal.tainted-path.tainted-path -- local CLI arg (add walk)
            let content = tokio::fs::read(file_path)
                .await
                .context(format!("Failed to read file: {}", file_path.display()))?;

            let content_oid = Oid::hash(&content);

            if let Some(m) = &sym_meta {
                stat.lock().unwrap().record(&rel_key, m, content_oid);
            }

            // Check if unchanged from HEAD
            if !conflicted
                && staged_or_base(index_files, head_files, &relative_path) == Some(content_oid)
            {
                if let Some(ref cb) = on_bytes {
                    cb(file_size);
                }
                return Ok((None, file_size));
            }

            // P4a: pHash-guided delta-base nomination for images. Advisory
            // only — nominates which prior object to TRY first; the 80%
            // delta gate in write_delta_against_base still decides.
            // should_use_delta() always skips jpg/png/webp (poor byte-level
            // delta candidates), so without this, images never even attempt
            // delta. MEDIAGIT_PHASH=0 restores that exact prior behavior.
            // Already stored: nothing to nominate or record, skip the decode.
            let phash_lookup = if delta_enabled
                && crate::phash_index::is_hashable_image(filename)
                && !odb.exists(&content_oid).await.unwrap_or(false)
            {
                crate::phash_index::compute_and_nominate(repo_root, &content).await
            } else {
                None
            };

            let phash_delta_oid =
                if let Some(base_oid) = phash_lookup.as_ref().and_then(|l| l.nominated_base) {
                    odb.write_delta_against_base(ObjectType::Blob, &content, filename, base_oid)
                        .await
                        .context("Failed pHash-guided delta attempt")?
                } else {
                    None
                };

            // Use parallel chunking for large files, sequential for small
            let oid = if let Some(delta_oid) = phash_delta_oid {
                delta_oid
            } else if Self::should_use_chunking(content.len(), filename) {
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

            // Record this file's pHash for future nominations (content-addressed
            // storage means `oid` == `content_oid` regardless of which branch above wrote it).
            if let Some(lookup) = phash_lookup {
                crate::phash_index::record(repo_root, lookup.hash, oid).await;
            }

            // Report bytes for non-streaming staged files (streaming path uses per-chunk callback)
            if let Some(ref cb) = on_bytes {
                cb(file_size);
            }

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

        Ok((
            Some(FileResult {
                relative_path,
                oid,
                file_size,
                mode,
                mtime: file_mtime,
            }),
            file_size,
        ))
    }

    /// Check if path is outside .mediagit directory
    fn is_outside_mediagit(path: &Path, mediagit_dir: &Path) -> bool {
        // BUG-009: a path that cannot be canonicalized (trailing dot, ...) was
        // reported "inside .mediagit" and silently dropped from the walk. Judge
        // it lexically instead.
        match dunce::canonicalize(path) {
            Ok(abs_path) => !abs_path.starts_with(mediagit_dir),
            Err(_) => !path.starts_with(mediagit_dir),
        }
    }

    /// Decide how a walked or named file enters the add list: the canonical
    /// path, or a plain-words reason it cannot be tracked (BUG-008/009).
    /// Refusing here, with the path the user knows, beats failing later on
    /// the canonical target.
    fn leaves_repo(path: &Path, repo_root: &Path) -> bool {
        let Ok(canon) = dunce::canonicalize(path) else {
            return false;
        };
        crate::repo::repo_relative(repo_root, &canon).is_err()
            && std::path::absolute(path)
                .is_ok_and(|lexical| crate::repo::repo_relative(repo_root, &lexical).is_ok())
    }

    fn link_message(path: &Path) -> String {
        format!(
            "{}: this is a symbolic link that points outside the repository. \
             Add a rule for it to .mediagitignore to stop this warning",
            path.display()
        )
    }

    fn resolve_entry(path: &Path, repo_root: &Path) -> std::result::Result<PathBuf, String> {
        if cfg!(windows)
            && let Some(why) = windows_unstorable_name(path)
        {
            return Err(format!("{}: {why}", path.display()));
        }
        let Ok(canon) = dunce::canonicalize(path) else {
            return Ok(path.to_path_buf());
        };
        if Self::leaves_repo(path, repo_root) {
            return Err(Self::link_message(path));
        }
        Ok(canon)
    }

    /// Resolve `path` to a repo-relative path for `.mediagitignore` matching.
    /// `strip_prefix(repo_root)` alone fails when `path` is relative (e.g. a
    /// walk started from `add .`), which silently skipped the ignore check
    /// (BUG-GD-1). Canonicalizing first makes the match relative-path-safe.
    fn ignore_relative_path(path: &Path, repo_root: &Path) -> Option<PathBuf> {
        let abs = dunce::canonicalize(path).or_else(|_| std::path::absolute(path));
        crate::repo::repo_relative(repo_root, &abs.ok()?).ok()
    }

    /// Expand paths (globs, directories) into a list of files to add.
    /// `tracked_paths` (repo-relative) is consulted only by the `-u`/`--update`
    /// branch, to restrict candidates to already-tracked files.
    fn expand_paths(
        &self,
        repo_root: &Path,
        tracked_paths: &std::collections::HashSet<PathBuf>,
    ) -> Result<(Vec<PathBuf>, Vec<String>)> {
        use crate::ignore_rules::IgnoreMatcher;
        use crate::output;

        let mut files = Vec::new();
        let mut refused: Vec<String> = Vec::new();
        let mediagit_dir = dunce::canonicalize(repo_root.join(".mediagit"))
            .unwrap_or_else(|_| repo_root.join(".mediagit"));

        // Build ignore matcher unless --force is set.
        // A missing .mediagitignore produces a no-op matcher (no error).
        let matcher = if !self.force {
            match IgnoreMatcher::new(repo_root) {
                Ok(m) => Some(m),
                Err(e) => {
                    if !self.quiet {
                        output::warning(&format!("Could not load .mediagitignore: {}", e));
                    }
                    None
                }
            }
        } else {
            None // --force bypasses .mediagitignore
        };

        // If --all is set and no paths given, add entire repo root
        if self.all && self.paths.is_empty() {
            self.collect_files_recursive(
                repo_root,
                repo_root,
                &mediagit_dir,
                &matcher,
                &mut files,
                &mut refused,
            )?;
            return Ok((files, refused));
        }

        // If --update (-u) is set, collect only already-tracked files that
        // still exist in the working dir (never untracked files).
        if self.update && self.paths.is_empty() {
            let mut candidates = Vec::new();
            self.collect_files_recursive(
                repo_root,
                repo_root,
                &mediagit_dir,
                &matcher,
                &mut candidates,
                &mut refused,
            )?;
            for path in candidates {
                if let Ok(rel) = crate::repo::repo_relative(repo_root, &path)
                    && tracked_paths.contains(&rel)
                {
                    files.push(path);
                }
            }
            return Ok((files, refused));
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
                                        // Check .mediagitignore for explicit glob results
                                        if let Some(ref m) = matcher
                                            && let Some(rel) =
                                                Self::ignore_relative_path(&p, repo_root)
                                            && m.is_ignored(&rel, false)
                                        {
                                            if self.verbose {
                                                output::detail(
                                                    "ignored (.mediagitignore)",
                                                    &p.display().to_string(),
                                                );
                                            }
                                            continue;
                                        }
                                        match Self::resolve_entry(&p, repo_root) {
                                            Ok(f) => files.push(f),
                                            Err(m) => refused.push(m),
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

            if !path.exists() {
                if !self.force && !self.quiet {
                    output::warning(&format!("Path does not exist: {}", path_str));
                }
                continue;
            }

            if path.is_file() && Self::is_outside_mediagit(path, &mediagit_dir) {
                // Check .mediagitignore for explicitly-named files
                if let Some(ref m) = matcher
                    && let Some(rel) = Self::ignore_relative_path(path, repo_root)
                    && m.is_ignored(&rel, false)
                {
                    if !self.quiet {
                        output::warning(&format!(
                            "'{}' is ignored by .mediagitignore — use --force to override",
                            path_str
                        ));
                    }
                    continue;
                }
                match Self::resolve_entry(path, repo_root) {
                    Ok(f) => files.push(f),
                    Err(m) => refused.push(m),
                }
            } else if path.is_dir() {
                self.collect_files_recursive(
                    path,
                    repo_root,
                    &mediagit_dir,
                    &matcher,
                    &mut files,
                    &mut refused,
                )?;
            }
        }

        Ok((files, refused))
    }

    /// Recursively collect all files from a directory, respecting .mediagitignore.
    #[allow(clippy::only_used_in_recursion)]
    fn collect_files_recursive(
        &self,
        dir: &Path,
        repo_root: &Path,
        mediagit_dir: &Path,
        matcher: &Option<crate::ignore_rules::IgnoreMatcher>,
        files: &mut Vec<PathBuf>,
        refused: &mut Vec<String>,
    ) -> Result<()> {
        use crate::output;

        if !Self::is_outside_mediagit(dir, mediagit_dir) {
            return Ok(());
        }

        // nosemgrep: rust.actix.path-traversal.tainted-path.tainted-path -- local CLI arg (add walk)
        let entries = std::fs::read_dir(dir)
            .context(format!("Failed to read directory: {}", dir.display()))?;

        // BUG-039: canonicalizing every file costs several syscalls each. A
        // plain file (not a link) inside an already-canonical folder has the
        // canonical path `canon_dir/name`, so the folder is resolved once.
        let canon_dir = dunce::canonicalize(dir).ok();

        for entry in entries {
            let entry = entry?;
            let path = entry.path();
            // `file_type` comes from the directory listing itself, so it still
            // answers for names a path lookup rewrites (trailing dot, BUG-009).
            let ft = entry.file_type()?;

            let fast = (ft.is_file()
                && !(cfg!(windows) && windows_unstorable_name(&path).is_some()))
            .then(|| canon_dir.as_ref().map(|d| d.join(entry.file_name())))
            .flatten()
            .filter(|c| crate::repo::repo_relative(repo_root, c).is_ok());

            let outside = match &fast {
                Some(c) => !c.starts_with(mediagit_dir),
                None => Self::is_outside_mediagit(&path, mediagit_dir),
            };
            if !outside {
                continue;
            }

            // Check .mediagitignore before descending into dirs or staging files
            let rel_for_ignore = match &fast {
                Some(c) => crate::repo::repo_relative(repo_root, c).ok(),
                None => Self::ignore_relative_path(&path, repo_root),
            };
            if let Some(m) = matcher
                && let Some(rel) = rel_for_ignore
            {
                let is_dir = path.is_dir();
                if m.is_ignored(&rel, is_dir) {
                    if self.verbose {
                        output::detail("ignored (.mediagitignore)", &path.display().to_string());
                    }
                    continue; // skip file OR prune entire directory
                }
            }

            if let Some(c) = fast {
                files.push(c);
            } else if path.is_file() || ft.is_file() {
                match Self::resolve_entry(&path, repo_root) {
                    Ok(f) => files.push(f),
                    Err(m) => refused.push(m),
                }
            } else if path.is_dir() || ft.is_dir() {
                // A folder Windows cannot address would fail to open below.
                if cfg!(windows)
                    && let Some(why) = windows_unstorable_name(&path)
                {
                    refused.push(format!("{}: {why}", path.display()));
                    continue;
                }
                // A linked folder is refused once, not once per file inside.
                if Self::leaves_repo(&path, repo_root) {
                    refused.push(Self::link_message(&path));
                    continue;
                }
                self.collect_files_recursive(
                    &path,
                    repo_root,
                    mediagit_dir,
                    matcher,
                    files,
                    refused,
                )?;
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
            "parquet" | "arrow" | "feather" | "orc" | "avro" | "hdf5" | "h5" | "npy" | "npz"
            | "tfrecords" | "petastorm" => true,

            // === ML MODELS: Good dedup for checkpoint files (5MB) ===
            "pt" | "pth" | "safetensors" | "ckpt" | "pb" | "onnx" | "gguf" | "ggml" | "tflite"
            | "keras" | "bin" => true,

            // === VIDEO: Structure-aware chunking (5MB) ===
            "mp4" | "mov" | "avi" | "mkv" | "webm" | "flv" | "wmv" | "mpg" | "mpeg" | "m4v"
            | "3gp" => true,

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
            "glb" | "gltf" | "obj" | "fbx" | "blend" | "usd" | "usda" | "usdc" => {
                size > MIN_SIZE_10MB
            }

            // === ARCHIVES: Uncompressed archives benefit from chunking (5MB) ===
            "tar" | "cpio" | "iso" | "dmg" => true,

            // === PRE-COMPRESSED: Never chunk (no benefit) ===
            "jpg" | "jpeg" | "png" | "webp" | "gif" | "avif" | "heic" | "mp3" | "aac" | "ogg"
            | "opus" | "zip" | "gz" | "bz2" | "xz" | "7z" | "rar" | "tar.gz" | "tgz" => false,

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
            "txt" | "md" | "json" | "xml" | "html" | "css" | "js" | "ts" | "py" | "rs" | "go"
            | "java" | "c" | "cpp" | "h" | "hpp" => true,
            // Uncompressed raster/audio: Good delta candidates
            "psd" | "tif" | "tiff" | "bmp" | "wav" | "aiff" => true,
            // Video (raw/uncompressed): Good delta candidates
            "avi" | "mov" => true,
            // Compressed video: Moderate benefit (only for very large files)
            "mp4" | "mkv" | "flv" | "wmv" => data.len() > 100 * 1024 * 1024,
            // Compressed images: Poor delta candidates (skip)
            "jpg" | "jpeg" | "png" | "webp" | "gif" => false,
            // Archives: No delta benefit (skip)
            "zip" | "gz" | "bz2" | "7z" | "rar" => false,
            // PDF-container formats (AI, InDesign, PDF):
            // These embed DEFLATE-compressed streams internally; any content change
            // reshuffles the compressed bytes, reducing byte-level similarity.
            // For small files (<50 MB) the delta attempt yields negligible savings and
            // wastes CPU — skip. For large files (≥50 MB) even partial similarity
            // (e.g., unchanged embedded images) can save tens of MB, so attempt delta
            // and let the 80% benefit gate reject it if the delta is not worthwhile.
            "ai" | "ait" | "indd" | "idml" | "indt" | "pdf" => data.len() > 50 * 1024 * 1024,
            // 3D text-based formats: Excellent delta candidates
            "obj" | "gltf" | "dae" | "ply" | "stl" | "stp" | "step" => true,
            // 3D binary formats: Moderate delta candidates
            "glb" | "fbx" | "blend" | "abc" | "usd" | "usda" | "usdc" => data.len() > 1024 * 1024,
            // Vector/PostScript text formats: Good delta candidates
            "eps" | "svg" => true,
            // Unknown: Conservative approach (allow if large)
            _ => data.len() > 50 * 1024 * 1024,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// I3: only "not there" means absent; any other read error is not a
    /// verdict about the files in the folder.
    #[test]
    fn only_a_missing_folder_counts_as_absent() {
        use std::io::{Error, ErrorKind};
        assert!(folder_is_absent(&Error::from(ErrorKind::NotFound)));
        assert!(folder_is_absent(&Error::from(ErrorKind::NotADirectory)));
        for kind in [
            ErrorKind::PermissionDenied,
            ErrorKind::Interrupted,
            ErrorKind::TimedOut,
            ErrorKind::Other,
        ] {
            assert!(!folder_is_absent(&Error::from(kind)), "{kind:?}");
        }
    }

    /// An unreadable folder fails naming the folder instead of reporting its
    /// files deleted. Unix only: the test strips the folder's permissions.
    #[cfg(unix)]
    #[test]
    fn unreadable_folder_is_an_error_naming_it() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = tempfile::tempdir().unwrap();
        let sub = tmp.path().join("locked");
        std::fs::create_dir(&sub).unwrap();
        std::fs::write(sub.join("a.txt"), "x").unwrap();
        std::fs::set_permissions(&sub, std::fs::Permissions::from_mode(0o000)).unwrap();
        let readable = std::fs::read_dir(&sub).is_ok(); // running as root
        let res = ExactNames::default().exists_exact(tmp.path(), Path::new("locked/a.txt"));
        std::fs::set_permissions(&sub, std::fs::Permissions::from_mode(0o755)).unwrap();
        if !readable {
            let msg = res.unwrap_err().to_string();
            assert!(msg.contains("locked"), "{msg}");
        }
    }

    /// BUG-011: the stored spelling decides, on any filesystem, for the file
    /// and for every folder above it; each folder is read once.
    #[test]
    fn exists_exact_compares_every_component_exactly_and_reads_each_folder_once() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir(tmp.path().join("art")).unwrap();
        std::fs::write(tmp.path().join("art/hero.txt"), "x").unwrap();
        let mut names = ExactNames::default();
        assert!(
            names
                .exists_exact(tmp.path(), Path::new("art/hero.txt"))
                .unwrap()
        );
        assert!(
            names
                .exists_exact(tmp.path(), Path::new("art/hero.txt"))
                .unwrap()
        );
        assert!(
            !names
                .exists_exact(tmp.path(), Path::new("art/Hero.txt"))
                .unwrap()
        );
        assert!(
            !names
                .exists_exact(tmp.path(), Path::new("Art/hero.txt"))
                .unwrap()
        );
        assert!(
            !names
                .exists_exact(tmp.path(), Path::new("art/missing.txt"))
                .unwrap()
        );
        assert!(
            !names
                .exists_exact(tmp.path(), Path::new("nodir/x"))
                .unwrap()
        );
        assert_eq!(names.folders.len(), 2, "root and art, each read once");
    }

    #[test]
    fn windows_unstorable_name_flags_trailing_dot_and_space_only() {
        for bad in ["d/shot.v1.", "d/name ", "shot.", "x/y/z "] {
            assert!(
                windows_unstorable_name(Path::new(bad)).is_some(),
                "{bad} must be flagged"
            );
        }
        for ok in [
            "d/shot.v1",
            "d/a b.txt",
            "d/.hidden",
            "d/trailing space .txt",
            "d/dir.ext/file",
        ] {
            assert!(
                windows_unstorable_name(Path::new(ok)).is_none(),
                "{ok} must pass"
            );
        }
    }
}
