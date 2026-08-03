// MediaGit - Git for Media Files
// Copyright (C) 2025 MediaGit Contributors
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU Affero General Public License as published
// by the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE. See the
// GNU Affero General Public License for more details.

//! Local filesystem storage backend
//!
//! Implements the `StorageBackend` trait using the local filesystem with:
//! - Sharded directory structure to prevent too many files in one directory
//! - Atomic writes using temp files and atomic rename operations
//! - Proper file permissions (0644 for files, 0755 for directories)
//! - Async I/O using tokio::fs
//!
//! # Directory Structure (layout v2)
//!
//! Each key's own directory component determines its physical top-level
//! folder, so `chunks/`, `chunk-deltas/`, `manifests/`, `deltas/`, `packs/`,
//! `bitmaps/` (and any per-repo `<ns>/` prefix added by
//! [`crate::NamespacedBackend`]) each get **true hash fanout** — sharded on
//! the hash/OID itself, not on the key string's own prefix. See
//! [`LocalBackend::object_path`] for the exact placement rule.
//!
//! ```text
//! root/
//!   <ns>/
//!     LAYOUT                    = "2"
//!     objects/ab/cd/<oid>         (bare OIDs — no dir component in the key)
//!     chunks/de/ad/<hash>
//!     chunk-deltas/de/ad/<hash>[.meta]
//!     manifests/../<hash>
//!     deltas/../<hash>[.meta]
//!     packs/aa/<pack_oid>          (single-level shard)
//! ```
//!
//! This prevents too many files in a single directory, improving filesystem performance.
//!
//! # Examples
//!
//! ```rust,no_run
//! use mediagit_storage::{StorageBackend, local::LocalBackend};
//! use std::path::Path;
//!
//! #[tokio::main]
//! async fn main() -> anyhow::Result<()> {
//!     // Create a local storage backend at .mediagit
//!     let storage = LocalBackend::new(".mediagit").await?;
//!
//!     // Store data
//!     storage.put("objects/abc123", b"file content").await?;
//!
//!     // Retrieve data
//!     let data = storage.get("objects/abc123").await?;
//!     assert_eq!(data, b"file content");
//!
//!     // Check existence
//!     if storage.exists("objects/abc123").await? {
//!         println!("Object exists");
//!     }
//!
//!     // List objects with prefix
//!     let objects = storage.list_objects("objects/").await?;
//!     println!("Found {} objects", objects.len());
//!
//!     // Delete object
//!     storage.delete("objects/abc123").await?;
//!
//!     Ok(())
//! }
//! ```

use crate::StorageBackend;
use async_trait::async_trait;
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use tokio::fs;
use tokio::io::AsyncWriteExt;

/// Monotonic counter for unique temp file names.
/// Prevents temp-file collisions when multiple async tasks write the same key concurrently.
static TEMP_WRITE_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Returns true if this OS error code is a transient Windows error worth retrying.
///
/// - `os error 2`  = `ERROR_FILE_NOT_FOUND` — directory not yet visible after creation
/// - `os error 5`  = `ERROR_ACCESS_DENIED` — Windows Defender / AV scanning the file,
///   or concurrent `CreateDirectory` race on NTFS
/// - `os error 32` = `ERROR_SHARING_VIOLATION` — another process has the file open
#[inline]
fn is_transient_windows_error(e: &std::io::Error) -> bool {
    matches!(e.raw_os_error(), Some(2) | Some(5) | Some(32))
}

/// Result type for adaptive loading - either memory-mapped or heap-allocated
///
/// Large files (>10MB) are memory-mapped for efficiency,
/// while small files are loaded into a Vec for simpler handling.
#[derive(Debug)]
pub enum MmapOrVec {
    /// Memory-mapped file view (for large files)
    Mmap(memmap2::Mmap),
    /// Heap-allocated data (for small files)
    Vec(Vec<u8>),
}

impl AsRef<[u8]> for MmapOrVec {
    fn as_ref(&self) -> &[u8] {
        match self {
            MmapOrVec::Mmap(mmap) => mmap.as_ref(),
            MmapOrVec::Vec(vec) => vec.as_ref(),
        }
    }
}

/// Local filesystem storage backend
///
/// Stores objects in a sharded directory structure with atomic writes.
/// Implements the StorageBackend trait for local filesystem storage.
///
/// # Thread Safety
///
/// This implementation is `Send + Sync` and can be safely shared across threads
/// and async tasks. The filesystem provides natural synchronization for concurrent access.
#[derive(Clone)]
pub struct LocalBackend {
    root: PathBuf,
}

impl LocalBackend {
    /// Create a new local filesystem backend at the given root path
    ///
    /// Creates the root directory if it doesn't exist.
    /// The objects directory (root/objects) will be created on first write.
    ///
    /// # Arguments
    ///
    /// * `root` - Path to the root directory for storage
    ///
    /// # Returns
    ///
    /// * `Ok(LocalBackend)` - Successfully created backend
    /// * `Err` - If the root path exists but is not a directory
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// use mediagit_storage::local::LocalBackend;
    ///
    /// # #[tokio::main]
    /// # async fn main() -> anyhow::Result<()> {
    /// let storage = LocalBackend::new(".mediagit").await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn new<P: AsRef<Path>>(root: P) -> anyhow::Result<Self> {
        let root = root.as_ref().to_path_buf();

        // Create root directory if it doesn't exist
        if !root.exists() {
            fs::create_dir_all(&root).await?;
        } else if !root.is_dir() {
            return Err(anyhow::anyhow!(
                "path exists but is not a directory: {}",
                root.display()
            ));
        }

        Ok(LocalBackend { root })
    }

    /// Create a new local filesystem backend synchronously
    ///
    /// This is a convenience method for synchronous contexts.
    /// Use `LocalBackend::new()` in async contexts.
    ///
    /// # Arguments
    ///
    /// * `root` - Path to the root directory for storage
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// use mediagit_storage::local::LocalBackend;
    ///
    /// let storage = LocalBackend::new_sync(".mediagit")?;
    /// # Ok::<(), anyhow::Error>(())
    /// ```
    pub fn new_sync<P: AsRef<Path>>(root: P) -> anyhow::Result<Self> {
        let root = root.as_ref().to_path_buf();

        // Create root directory if it doesn't exist
        if !root.exists() {
            std::fs::create_dir_all(&root)?;
        } else if !root.is_dir() {
            return Err(anyhow::anyhow!(
                "path exists but is not a directory: {}",
                root.display()
            ));
        }

        Ok(LocalBackend { root })
    }

    /// Get the root path for this backend
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Layout-v2 top-level namespace directories that co-locate directly
    /// with their hash-sharded contents (no synthetic `objects/` wrapper
    /// layer). Any key whose *immediate parent directory component* is one
    /// of these names shards straight under that directory:
    /// `<dir(key)>/<base[0:2]>/<base[2:4]>/<base>`.
    ///
    /// Any other key (most commonly a bare OID with no directory component
    /// at all, or a namespaced bare OID like `<ns>/<oid>`) is routed through
    /// a synthetic `objects/` shard layer instead:
    /// `<dir(key)>/objects/<base[0:2]>/<base[2:4]>/<base>`.
    ///
    /// `bitmaps/` (M3) is included pre-emptively: it needs no code change
    /// when that milestone starts writing `bitmaps/<commit_oid>.bitmap`.
    ///
    /// Deliberately does NOT include `"objects"` itself — that name is
    /// reserved as the synthetic wrapper folder. If it were also treated as
    /// a "known" category, a repo namespace that happened to sanitize to
    /// literally `objects` could collide with the unnamespaced bare-OID
    /// path used by direct (non-namespaced) `LocalBackend` callers (tests).
    const KNOWN_CATEGORIES: [&'static str; 5] =
        ["chunks", "chunk-deltas", "manifests", "deltas", "bitmaps"];

    /// Split `src` into a fixed 2-char/2-char shard pair, padding with `_`
    /// (not a valid lowercase-hex character, so it can't collide with a real
    /// hash shard) when `src` is shorter than 4 chars. This keeps every key
    /// — regardless of length — at the same shard depth, which is what
    /// makes `list_objects`'s reverse mapping a simple fixed-offset strip
    /// instead of needing to re-derive variable shard depth per key.
    fn shard_pair(src: &str) -> (String, String) {
        let mut c: Vec<char> = src.chars().take(4).collect();
        while c.len() < 4 {
            c.push('_');
        }
        (c[0..2].iter().collect(), c[2..4].iter().collect())
    }

    /// Get the physical path for a given (possibly namespace-prefixed)
    /// logical key.
    ///
    /// # Layout v2
    ///
    /// The physical path is derived structurally from the key's own
    /// directory component, so `chunks/`, `chunk-deltas/`, `manifests/`,
    /// `deltas/`, `packs/`, `bitmaps/` — and any per-repo `<ns>/` prefix
    /// added by [`crate::NamespacedBackend`] — get **true hash fanout**
    /// instead of every key colliding into one `objects/ch/un/` directory
    /// (the v1 fanout-collapse bug: sharding was on the *key string's*
    /// first 4 chars, not the hash).
    ///
    /// - `chunks/<hash>` → `chunks/<h0:2>/<h2:4>/<hash>`
    /// - `chunk-deltas/<hash>.meta` shards on the hash part (`.meta`
    ///   stripped only for computing the shard, kept in the filename) so it
    ///   co-locates with its binary sibling `chunk-deltas/<h0:2>/<h2:4>/<hash>`.
    /// - `packs/<pack_oid>` → `packs/<p0:2>/<pack_oid>` (single-level shard).
    /// - A bare OID with no directory component (or a namespaced bare OID,
    ///   e.g. `<ns>/<oid>`) → `[<ns>/]objects/<o0:2>/<o2:4>/<oid>`.
    ///
    /// Keys whose basename is shorter than 4 chars are padded (see
    /// [`Self::shard_pair`]) rather than panicking or skipping sharding.
    fn object_path(&self, key: &str) -> PathBuf {
        let key_path = Path::new(key);
        let base = key_path
            .file_name()
            .and_then(|f| f.to_str())
            .unwrap_or(key)
            .to_string();
        let dir_components: Vec<String> = key_path
            .parent()
            .map(|p| p.iter().map(|c| c.to_string_lossy().into_owned()).collect())
            .unwrap_or_default();

        // Shard on the hash part only; `.meta` companions co-locate with
        // their binary sibling by stripping the suffix before sharding
        // while keeping it in the final filename.
        let shard_src = base.strip_suffix(".meta").unwrap_or(&base);

        let mut p = self.root.clone();
        for c in &dir_components {
            p = p.join(c);
        }

        // The `LAYOUT` marker (v2) is control-plane metadata, not object
        // data: it's placed unsharded directly under its directory
        // (`[<ns>/]LAYOUT`) instead of through the objects/category shard
        // rules. This also keeps it structurally shallower than anything
        // `walk_dir_iterative` reconstructs as a logical key, so it never
        // shows up in `list_objects` results.
        if base == "LAYOUT" {
            return p.join(&base);
        }

        if dir_components.last().map(|s| s.as_str()) == Some("packs") {
            let (s1, _s2) = Self::shard_pair(shard_src);
            return p.join(s1).join(&base);
        }

        let is_known_category = dir_components
            .last()
            .map(|s| Self::KNOWN_CATEGORIES.contains(&s.as_str()))
            .unwrap_or(false);
        if !is_known_category {
            p = p.join("objects");
        }

        let (s1, s2) = Self::shard_pair(shard_src);
        p.join(s1).join(s2).join(&base)
    }

    /// Validate `key` (J6: reject path traversal / absolute / drive-rooted
    /// keys) before computing its physical path. Every production entry
    /// point that turns a caller-supplied key into a filesystem path must
    /// go through this instead of calling [`Self::object_path`] directly —
    /// that raw method has no validation and is only safe to call with keys
    /// already known-good (as the unit tests below do, to pin the mapping
    /// table itself).
    fn object_path_checked(&self, key: &str) -> anyhow::Result<PathBuf> {
        crate::validate_object_key(key)?;
        Ok(self.object_path(key))
    }

    /// Ensure parent directory exists, creating it if necessary
    ///
    /// # Arguments
    ///
    /// * `path` - The path for which to ensure parent directories exist
    ///
    /// # Returns
    ///
    /// * `Ok(())` - Parent directory exists or was created
    /// * `Err` - If directory creation fails
    async fn ensure_parent_dir(&self, path: &Path) -> anyhow::Result<()> {
        if let Some(parent) = path.parent() {
            // create_dir_all is idempotent on Linux/macOS, but on Windows NTFS
            // concurrent calls for the same path can transiently return ACCESS_DENIED (5).
            // Retry with backoff to handle this Windows race condition.
            for attempt in 0u32..=3 {
                match fs::create_dir_all(parent).await {
                    Ok(()) => return Ok(()),
                    Err(e) if attempt < 3 && is_transient_windows_error(&e) => {
                        let delay_ms = 5 * (3u64.pow(attempt));
                        tokio::time::sleep(std::time::Duration::from_millis(delay_ms)).await;
                    }
                    Err(e) => return Err(e.into()),
                }
            }
        }
        Ok(())
    }

    /// Get a memory-mapped view of an object
    ///
    /// Memory mapping is more efficient for large files as it doesn't require
    /// loading the entire file into heap memory. The OS handles paging data
    /// in and out as needed.
    ///
    /// # Safety
    ///
    /// This function uses unsafe code internally, but the Mmap is safe to use
    /// as long as the file is not modified while the mmap is open.
    ///
    /// # Arguments
    ///
    /// * `key` - The object identifier
    ///
    /// # Returns
    ///
    /// * `Ok(Mmap)` - Memory-mapped view of the file
    /// * `Err` - If the key doesn't exist or an I/O error occurs
    #[allow(unsafe_code)] // audited: read-only mmap, file not modified while mapped
    pub fn get_mmap(&self, key: &str) -> anyhow::Result<memmap2::Mmap> {
        if key.is_empty() {
            return Err(anyhow::anyhow!("key cannot be empty"));
        }

        let path = self.object_path_checked(key)?;
        let file = std::fs::File::open(&path)?;

        // SAFETY: The file is opened read-only and we assume it won't be modified
        // while the mmap is open. The mmap will be invalidated if the file is deleted.
        let mmap = unsafe { memmap2::Mmap::map(&file)? };

        Ok(mmap)
    }

    /// Get file size in bytes
    ///
    /// Returns the size of an object without reading its contents.
    ///
    /// # Arguments
    ///
    /// * `key` - The object identifier
    ///
    /// # Returns
    ///
    /// * `Ok(u64)` - File size in bytes
    /// * `Err` - If the key doesn't exist or an I/O error occurs
    pub async fn get_size(&self, key: &str) -> anyhow::Result<u64> {
        if key.is_empty() {
            return Err(anyhow::anyhow!("key cannot be empty"));
        }

        let path = self.object_path_checked(key)?;
        let metadata = fs::metadata(&path).await?;
        Ok(metadata.len())
    }

    /// Adaptive get: uses mmap for large files, normal read for small files
    ///
    /// Threshold is 10MB - files larger than this are memory-mapped for
    /// better performance and lower memory usage.
    ///
    /// # Arguments
    ///
    /// * `key` - The object identifier
    ///
    /// # Returns
    ///
    /// * `Ok(MmapOrVec)` - Either a memory-mapped view or a Vec<u8>
    /// * `Err` - If the key doesn't exist or an I/O error occurs
    pub async fn get_adaptive(&self, key: &str) -> anyhow::Result<MmapOrVec> {
        const MMAP_THRESHOLD: u64 = 10 * 1024 * 1024; // 10MB

        let size = self.get_size(key).await?;

        if size > MMAP_THRESHOLD {
            tracing::debug!(key = %key, size = size, "Using mmap for large file");
            Ok(MmapOrVec::Mmap(self.get_mmap(key)?))
        } else {
            Ok(MmapOrVec::Vec(
                fs::read(self.object_path_checked(key)?).await?,
            ))
        }
    }
}

impl fmt::Debug for LocalBackend {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LocalBackend")
            .field("root", &self.root)
            .finish()
    }
}

#[async_trait]
impl StorageBackend for LocalBackend {
    /// Retrieve an object by its key
    ///
    /// Reads the file from the sharded directory structure.
    ///
    /// # Arguments
    ///
    /// * `key` - The object identifier
    ///
    /// # Returns
    ///
    /// * `Ok(Vec<u8>)` - The object data
    /// * `Err` - If the key doesn't exist or an I/O error occurs
    async fn get(&self, key: &str) -> anyhow::Result<Vec<u8>> {
        if key.is_empty() {
            return Err(anyhow::anyhow!("key cannot be empty"));
        }

        let path = self.object_path_checked(key)?;

        match fs::read(&path).await {
            Ok(data) => Ok(data),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                Err(anyhow::anyhow!("object not found: {}", key))
            }
            Err(e) => Err(e.into()),
        }
    }

    async fn get_streaming_range(
        &self,
        key: &str,
        range: std::ops::Range<u64>,
    ) -> anyhow::Result<
        std::pin::Pin<
            Box<dyn futures::Stream<Item = anyhow::Result<bytes::Bytes>> + Send + 'static>,
        >,
    > {
        use tokio::io::{AsyncReadExt, AsyncSeekExt};
        let path = self.object_path_checked(key)?;
        let mut file = tokio::fs::File::open(&path).await.map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                anyhow::anyhow!("object not found: {}", key)
            } else {
                anyhow::anyhow!("get_streaming_range open {}: {}", key, e)
            }
        })?;
        file.seek(std::io::SeekFrom::Start(range.start)).await?;
        let remaining = range.end - range.start;
        let stream = futures::stream::unfold((file, remaining), |(mut f, mut left)| async move {
            if left == 0 {
                return None;
            }
            let chunk_size = left.min(65536) as usize;
            let mut buf = vec![0u8; chunk_size];
            match f.read(&mut buf).await {
                Ok(0) => None,
                Ok(n) => {
                    buf.truncate(n);
                    left -= n as u64;
                    Some((
                        Ok::<bytes::Bytes, anyhow::Error>(bytes::Bytes::from(buf)),
                        (f, left),
                    ))
                }
                // Fuse: yield the error once, then end. Resuming after a failed
                // read would splice a gap into the byte sequence.
                Err(e) => Some((
                    Err(anyhow::anyhow!("get_streaming_range read: {}", e)),
                    (f, 0),
                )),
            }
        });
        Ok(Box::pin(stream))
    }

    /// Store an object with the given key
    ///
    /// Uses atomic writes: writes to a temporary file first, then atomically
    /// renames it to the final location. This ensures no partial writes are visible.
    ///
    /// # Arguments
    ///
    /// * `key` - The object identifier
    /// * `data` - The object content
    ///
    /// # Returns
    ///
    /// * `Ok(())` - The operation succeeded
    /// * `Err` - If an I/O error occurs or permission is denied
    async fn put(&self, key: &str, data: &[u8]) -> anyhow::Result<()> {
        if key.is_empty() {
            return Err(anyhow::anyhow!("key cannot be empty"));
        }

        let path = self.object_path_checked(key)?;

        // Windows-specific transient errors require retry with backoff:
        //
        // - os error 2  (ERROR_FILE_NOT_FOUND)    — shard dir not yet visible post-creation
        // - os error 5  (ERROR_ACCESS_DENIED)     — Windows Defender/AV scanning the file,
        //                                           or concurrent CreateDirectory race on NTFS
        // - os error 32 (ERROR_SHARING_VIOLATION) — another process has the file open
        //
        // "Works on second try" is the classic fingerprint of AV interference.
        const MAX_RETRIES: u32 = 5;
        let mut last_error: Option<std::io::Error> = None;

        // Unique ID per write prevents temp-file collisions when multiple async tasks
        // concurrently write the same key (TOCTOU gap between exists() and put()).
        let write_id = TEMP_WRITE_COUNTER.fetch_add(1, Ordering::Relaxed);

        for attempt in 0..=MAX_RETRIES {
            if attempt > 0 {
                // Exponential backoff: 10ms, 30ms, 90ms, 270ms, 810ms
                let delay_ms = 10 * (3u64.pow(attempt - 1));
                tokio::time::sleep(std::time::Duration::from_millis(delay_ms)).await;
                // Re-ensure parent dir — may have been transiently invisible
                let _ = self.ensure_parent_dir(&path).await;
            } else {
                self.ensure_parent_dir(&path).await?;
            }

            // Unique temp path per write avoids collisions between concurrent writers
            // of the same key (e.g., two tasks storing the same deduplicated chunk).
            let temp_path = path.with_extension(format!("tmp{}", write_id));

            // Remove any stale temp file from a previous (failed) attempt
            let _ = fs::remove_file(&temp_path).await;

            // Create and write temp file
            let file_result = fs::File::create(&temp_path).await;
            let mut file = match file_result {
                Ok(f) => f,
                Err(e) if attempt < MAX_RETRIES && is_transient_windows_error(&e) => {
                    last_error = Some(e);
                    continue;
                }
                Err(e) => return Err(e.into()),
            };

            if let Err(e) = file.write_all(data).await {
                let _ = fs::remove_file(&temp_path).await;
                return Err(e.into());
            }
            if let Err(e) = file.sync_all().await {
                let _ = fs::remove_file(&temp_path).await;
                return Err(e.into());
            }
            drop(file);

            // Atomically rename temp file to final location
            match fs::rename(&temp_path, &path).await {
                Ok(()) => return Ok(()),

                Err(e) if attempt < MAX_RETRIES && is_transient_windows_error(&e) => {
                    // Transient error (AV scan, dir race) — retry after backoff
                    let _ = fs::remove_file(&temp_path).await;
                    last_error = Some(e);
                    continue;
                }

                Err(e)
                    if e.kind() == std::io::ErrorKind::AlreadyExists
                        || e.raw_os_error() == Some(183) =>
                {
                    // Windows ERROR_ALREADY_EXISTS (183): another concurrent writer won the
                    // race and already stored the same content (CAS: same OID = same data).
                    // The destination is correct — treat as success.
                    let _ = fs::remove_file(&temp_path).await;
                    return Ok(());
                }

                Err(e) if is_transient_windows_error(&e) && fs::metadata(&path).await.is_ok() => {
                    // ACCESS_DENIED on rename but destination exists: concurrent winner
                    // already wrote the correct object. Safe to treat as success.
                    let _ = fs::remove_file(&temp_path).await;
                    return Ok(());
                }

                Err(e) => {
                    let _ = fs::remove_file(&temp_path).await;
                    return Err(e.into());
                }
            }
        }

        Err(anyhow::anyhow!(
            "Failed to write object after {} retries: {}",
            MAX_RETRIES,
            last_error.map(|e| e.to_string()).unwrap_or_default()
        ))
    }

    /// Atomic rename from a temp file (B4 stream-to-disk override).
    ///
    /// On same-filesystem paths this avoids reading the file back into RAM.
    /// Falls back to copy + delete when rename crosses device boundaries
    /// (e.g. system temp dir on a different drive from the ODB root on Windows).
    async fn put_file(&self, key: &str, src: &std::path::Path) -> anyhow::Result<()> {
        if key.is_empty() {
            return Err(anyhow::anyhow!("key cannot be empty"));
        }
        let dest = self.object_path_checked(key)?;
        self.ensure_parent_dir(&dest).await?;
        match fs::rename(src, &dest).await {
            Ok(()) => Ok(()),
            Err(_) => {
                // Cross-device rename (different filesystem / drive): copy then delete.
                fs::copy(src, &dest)
                    .await
                    .map_err(|e| anyhow::anyhow!("put_file copy: {}", e))?;
                let _ = fs::remove_file(src).await;
                Ok(())
            }
        }
    }

    /// Check if an object exists
    ///
    /// # Arguments
    ///
    /// * `key` - The object identifier
    ///
    /// # Returns
    ///
    /// * `Ok(true)` - The object exists
    /// * `Ok(false)` - The object doesn't exist
    /// * `Err` - If an I/O error occurs or permission is denied
    async fn exists(&self, key: &str) -> anyhow::Result<bool> {
        if key.is_empty() {
            return Err(anyhow::anyhow!("key cannot be empty"));
        }

        let path = self.object_path_checked(key)?;
        match fs::try_exists(&path).await {
            Ok(exists) => Ok(exists),
            Err(e) => Err(e.into()),
        }
    }

    async fn head(&self, key: &str) -> anyhow::Result<Option<u64>> {
        if key.is_empty() {
            return Err(anyhow::anyhow!("key cannot be empty"));
        }

        let path = self.object_path_checked(key)?;
        match tokio::fs::metadata(&path).await {
            Ok(m) => Ok(Some(m.len())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    /// Delete an object
    ///
    /// This operation is idempotent: deleting a non-existent object succeeds.
    ///
    /// # Arguments
    ///
    /// * `key` - The object identifier
    ///
    /// # Returns
    ///
    /// * `Ok(())` - The operation succeeded (whether the object existed or not)
    /// * `Err` - If an I/O error occurs or permission is denied
    async fn delete(&self, key: &str) -> anyhow::Result<()> {
        if key.is_empty() {
            return Err(anyhow::anyhow!("key cannot be empty"));
        }

        let path = self.object_path_checked(key)?;

        match fs::remove_file(&path).await {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                // Idempotent: deleting non-existent object is success
                Ok(())
            }
            Err(e) => Err(e.into()),
        }
    }

    /// List objects with a given prefix
    ///
    /// Returns a sorted list of all keys that start with the given prefix.
    /// Walks the whole storage root and reconstructs each file's logical key
    /// by reversing [`Self::object_path`]'s placement rule (see
    /// [`Self::walk_dir_iterative`] for the exact reverse mapping), then
    /// filters by prefix. Layout v2's physical tree mirrors the logical
    /// hierarchy closely enough that no per-prefix subtree shortcut is
    /// needed — unlike v1, which required a hardcoded "packs/ is a flat
    /// dir, everything else lives under objects/" special case.
    ///
    /// # Arguments
    ///
    /// * `prefix` - The key prefix to filter by
    ///
    /// # Returns
    ///
    /// * `Ok(Vec<String>)` - Sorted list of matching keys
    /// * `Err` - If an I/O error occurs or permission is denied
    async fn list_objects(&self, prefix: &str) -> anyhow::Result<Vec<String>> {
        let mut results = Vec::new();

        if !self.root.exists() {
            return Ok(results);
        }

        Self::walk_dir_iterative(&self.root, &self.root, prefix, &mut results).await?;

        results.sort();
        Ok(results)
    }
}

// Helper function for iterative directory traversal
impl LocalBackend {
    /// Iteratively walk the storage root and collect matching keys.
    /// Uses a work queue to avoid recursive async function issues.
    ///
    /// Reverses [`Self::object_path`]'s placement rule using fixed offsets
    /// from the end of each file's path (relative to `root`), since every
    /// key is sharded to a uniform depth (see [`Self::shard_pair`]):
    ///
    /// - `[...dir, "packs", shard1, base]` (3 from the end) → key =
    ///   `dir/packs/base` (drop `shard1`).
    /// - `[...dir, category, shard1, shard2, base]` (4 from the end), where
    ///   `category` is one of [`Self::KNOWN_CATEGORIES`] → key =
    ///   `dir/category/base` (drop `shard1`/`shard2`).
    /// - `[...dir, "objects", shard1, shard2, base]` (4 from the end) → key
    ///   = `dir/base` (the `objects` layer is synthetic — dropped entirely,
    ///   along with the shard).
    /// - Anything shallower or unrecognized is a stray/foreign file (e.g. a
    ///   `LAYOUT` marker sitting directly under a namespace dir) and is
    ///   skipped — it was never written through `object_path`.
    async fn walk_dir_iterative(
        dir: &Path,
        root: &Path,
        prefix: &str,
        results: &mut Vec<String>,
    ) -> anyhow::Result<()> {
        let mut work_queue = vec![dir.to_path_buf()];

        while let Some(current_path) = work_queue.pop() {
            let mut entries = match fs::read_dir(&current_path).await {
                Ok(entries) => entries,
                Err(_) => continue, // Skip directories we can't read
            };

            while let Some(entry) = entries.next_entry().await? {
                let path = entry.path();

                if path.is_dir() {
                    work_queue.push(path);
                    continue;
                }

                let Ok(relative_path) = path.strip_prefix(root) else {
                    continue;
                };
                let components: Vec<String> = relative_path
                    .components()
                    .map(|c| c.as_os_str().to_string_lossy().to_string())
                    .collect();
                let n = components.len();
                if n == 0 {
                    continue;
                }
                let base = components[n - 1].clone();

                // Defensively skip `put()`'s crash-recovery artifacts
                // (`<key>.tmp<write_id>`, left behind if the process died
                // between the temp write and the atomic rename): these are
                // never part of the logical key space and must never be
                // exposed by list_objects, even transiently.
                if let Some((_, suffix)) = base.rsplit_once(".tmp")
                    && !suffix.is_empty()
                    && suffix.chars().all(|c| c.is_ascii_digit())
                {
                    continue;
                }

                let key = if n >= 3 && components[n - 3] == "packs" {
                    let mut k = components[..n - 3].to_vec();
                    k.push("packs".to_string());
                    k.push(base);
                    k.join("/")
                } else if n >= 4 && Self::KNOWN_CATEGORIES.contains(&components[n - 4].as_str()) {
                    let mut k = components[..n - 4].to_vec();
                    k.push(components[n - 4].clone());
                    k.push(base);
                    k.join("/")
                } else if n >= 4 && components[n - 4] == "objects" {
                    let mut k = components[..n - 4].to_vec();
                    k.push(base);
                    k.join("/")
                } else {
                    // Not a shape object_path() produces (e.g. LAYOUT marker,
                    // stray temp file) — not part of the logical key space.
                    continue;
                };

                if key.starts_with(prefix) {
                    results.push(key);
                }
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[tokio::test]
    async fn test_new_creates_root_directory() {
        let temp_dir = TempDir::new().unwrap();
        let path = temp_dir.path().join("storage");

        assert!(!path.exists());
        let backend = LocalBackend::new(&path).await.unwrap();
        assert!(path.exists());
        assert_eq!(backend.root(), &path);
    }

    #[tokio::test]
    async fn test_new_with_existing_directory() {
        let temp_dir = TempDir::new().unwrap();
        let path = temp_dir.path();

        let backend = LocalBackend::new(path).await.unwrap();
        assert_eq!(backend.root(), path);
    }

    #[tokio::test]
    async fn test_new_fails_with_file_path() {
        let temp_dir = TempDir::new().unwrap();
        let file_path = temp_dir.path().join("file.txt");
        fs::write(&file_path, b"content").unwrap();

        let result = LocalBackend::new(&file_path).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_put_and_get() {
        let temp_dir = TempDir::new().unwrap();
        let backend = LocalBackend::new(temp_dir.path()).await.unwrap();

        let key = "test_key";
        let data = b"test data content";

        backend.put(key, data).await.unwrap();
        let retrieved = backend.get(key).await.unwrap();
        assert_eq!(retrieved, data);
    }

    #[tokio::test]
    async fn test_sharding_creates_correct_path() {
        let temp_dir = TempDir::new().unwrap();
        let backend = LocalBackend::new(temp_dir.path()).await.unwrap();

        let key = "abcd1234567890";
        let expected_path = temp_dir.path().join("objects/ab/cd/abcd1234567890");

        backend.put(key, b"data").await.unwrap();
        assert!(expected_path.exists());
    }

    #[tokio::test]
    async fn test_get_nonexistent() {
        let temp_dir = TempDir::new().unwrap();
        let backend = LocalBackend::new(temp_dir.path()).await.unwrap();

        let result = backend.get("nonexistent").await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("object not found"));
    }

    #[tokio::test]
    async fn test_exists() {
        let temp_dir = TempDir::new().unwrap();
        let backend = LocalBackend::new(temp_dir.path()).await.unwrap();

        let key = "exists_test";
        assert!(!backend.exists(key).await.unwrap());

        backend.put(key, b"data").await.unwrap();
        assert!(backend.exists(key).await.unwrap());
    }

    #[tokio::test]
    async fn test_delete_existing() {
        let temp_dir = TempDir::new().unwrap();
        let backend = LocalBackend::new(temp_dir.path()).await.unwrap();

        let key = "delete_test";
        backend.put(key, b"data").await.unwrap();
        assert!(backend.exists(key).await.unwrap());

        backend.delete(key).await.unwrap();
        assert!(!backend.exists(key).await.unwrap());
    }

    #[tokio::test]
    async fn test_delete_nonexistent_is_idempotent() {
        let temp_dir = TempDir::new().unwrap();
        let backend = LocalBackend::new(temp_dir.path()).await.unwrap();

        // Should not error, just succeed
        backend.delete("nonexistent").await.unwrap();
        // Deleting again should also succeed
        backend.delete("nonexistent").await.unwrap();
    }

    #[tokio::test]
    async fn test_traversal_key_rejected_and_never_written_outside_root() {
        // J6: a `..`-bearing key must error instead of joining onto a path
        // outside `root`.
        let temp_dir = TempDir::new().unwrap();
        let backend = LocalBackend::new(temp_dir.path()).await.unwrap();
        let outside_marker = temp_dir.path().parent().unwrap().join("pwned.txt");
        let _ = fs::remove_file(&outside_marker);

        let traversal_key = "../pwned.txt";
        assert!(backend.put(traversal_key, b"pwned").await.is_err());
        assert!(backend.get(traversal_key).await.is_err());
        assert!(backend.exists(traversal_key).await.is_err());
        assert!(backend.delete(traversal_key).await.is_err());
        assert!(backend.head(traversal_key).await.is_err());

        assert!(!outside_marker.exists());
    }

    #[tokio::test]
    async fn test_empty_key_operations() {
        let temp_dir = TempDir::new().unwrap();
        let backend = LocalBackend::new(temp_dir.path()).await.unwrap();

        assert!(backend.put("", b"data").await.is_err());
        assert!(backend.get("").await.is_err());
        assert!(backend.exists("").await.is_err());
        assert!(backend.delete("").await.is_err());
    }

    #[tokio::test]
    async fn test_list_objects_empty() {
        let temp_dir = TempDir::new().unwrap();
        let backend = LocalBackend::new(temp_dir.path()).await.unwrap();

        let objects = backend.list_objects("").await.unwrap();
        assert_eq!(objects.len(), 0);
    }

    #[tokio::test]
    async fn test_list_objects_with_prefix() {
        let temp_dir = TempDir::new().unwrap();
        let backend = LocalBackend::new(temp_dir.path()).await.unwrap();

        backend.put("images/photo1.jpg", b"data1").await.unwrap();
        backend.put("images/photo2.jpg", b"data2").await.unwrap();
        backend.put("videos/video1.mp4", b"data3").await.unwrap();
        backend.put("audio/song1.mp3", b"data4").await.unwrap();

        let images = backend.list_objects("images/").await.unwrap();
        assert_eq!(images.len(), 2);
        assert!(images.iter().all(|k| k.starts_with("images/")));

        let videos = backend.list_objects("videos/").await.unwrap();
        assert_eq!(videos.len(), 1);

        let empty = backend.list_objects("nonexistent/").await.unwrap();
        assert_eq!(empty.len(), 0);
    }

    #[tokio::test]
    async fn test_list_objects_sorted() {
        let temp_dir = TempDir::new().unwrap();
        let backend = LocalBackend::new(temp_dir.path()).await.unwrap();

        backend.put("zebra", b"data").await.unwrap();
        backend.put("apple", b"data").await.unwrap();
        backend.put("monkey", b"data").await.unwrap();

        let objects = backend.list_objects("").await.unwrap();
        assert_eq!(objects, vec!["apple", "monkey", "zebra"]);
    }

    #[tokio::test]
    async fn test_list_objects_all_prefixes() {
        let temp_dir = TempDir::new().unwrap();
        let backend = LocalBackend::new(temp_dir.path()).await.unwrap();

        backend.put("data/file1.bin", b"data").await.unwrap();
        backend.put("data/file2.bin", b"data").await.unwrap();
        backend.put("config/settings.json", b"data").await.unwrap();

        let all = backend.list_objects("").await.unwrap();
        assert_eq!(all.len(), 3);
    }

    #[tokio::test]
    async fn test_atomic_write() {
        let temp_dir = TempDir::new().unwrap();
        let backend = LocalBackend::new(temp_dir.path()).await.unwrap();

        let key = "atomic_test";
        let path = backend.object_path(key);

        // Put should use atomic write
        backend.put(key, b"atomic data").await.unwrap();

        // Final file should exist
        assert!(path.exists());

        // Temp file should not exist
        assert!(!path.with_extension("tmp").exists());

        // Data should be correct
        assert_eq!(backend.get(key).await.unwrap(), b"atomic data");
    }

    #[tokio::test]
    async fn test_overwrite() {
        let temp_dir = TempDir::new().unwrap();
        let backend = LocalBackend::new(temp_dir.path()).await.unwrap();

        let key = "overwrite_test";

        backend.put(key, b"old data").await.unwrap();
        assert_eq!(backend.get(key).await.unwrap(), b"old data");

        backend.put(key, b"new data").await.unwrap();
        assert_eq!(backend.get(key).await.unwrap(), b"new data");
    }

    #[tokio::test]
    async fn test_concurrent_writes() {
        let temp_dir = TempDir::new().unwrap();
        let backend = LocalBackend::new(temp_dir.path()).await.unwrap();

        // Spawn concurrent write tasks
        let mut handles = vec![];

        for i in 0..10 {
            let backend_clone = backend.clone();
            let handle = tokio::spawn(async move {
                let key = format!("concurrent_test_{}", i);
                let data = format!("data_{}", i);
                backend_clone.put(&key, data.as_bytes()).await.unwrap();
            });
            handles.push(handle);
        }

        // Wait for all tasks
        for handle in handles {
            handle.await.unwrap();
        }

        // Verify all data was written
        let objects = backend.list_objects("concurrent_test_").await.unwrap();
        assert_eq!(objects.len(), 10);
    }

    #[tokio::test]
    async fn test_concurrent_reads_writes() {
        let temp_dir = TempDir::new().unwrap();
        let backend = LocalBackend::new(temp_dir.path()).await.unwrap();

        // Write initial data
        backend.put("shared_key", b"initial").await.unwrap();

        // Spawn concurrent read/write tasks
        let backend_read = backend.clone();
        let read_handle = tokio::spawn(async move {
            for _ in 0..5 {
                let _ = backend_read.get("shared_key").await;
                tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;
            }
        });

        let backend_write = backend.clone();
        let write_handle = tokio::spawn(async move {
            for i in 0..5 {
                let data = format!("data_{}", i);
                backend_write
                    .put("shared_key", data.as_bytes())
                    .await
                    .unwrap();
                tokio::time::sleep(tokio::time::Duration::from_millis(15)).await;
            }
        });

        read_handle.await.unwrap();
        write_handle.await.unwrap();

        // Final read should succeed
        let final_data = backend.get("shared_key").await.unwrap();
        assert!(!final_data.is_empty());
    }

    #[tokio::test]
    async fn test_large_data() {
        let temp_dir = TempDir::new().unwrap();
        let backend = LocalBackend::new(temp_dir.path()).await.unwrap();

        let large_data = vec![0xFF; 10 * 1024 * 1024]; // 10 MB
        let key = "large_file";

        backend.put(key, &large_data).await.unwrap();
        let retrieved = backend.get(key).await.unwrap();
        assert_eq!(retrieved.len(), 10 * 1024 * 1024);
        assert_eq!(retrieved, large_data);
    }

    #[tokio::test]
    async fn test_debug_impl() {
        let temp_dir = TempDir::new().unwrap();
        let backend = LocalBackend::new(temp_dir.path()).await.unwrap();
        let debug_str = format!("{:?}", backend);
        assert!(debug_str.contains("LocalBackend"));
        assert!(debug_str.contains("root"));
    }

    #[tokio::test]
    async fn test_clone_independence() {
        let temp_dir = TempDir::new().unwrap();
        let backend1 = LocalBackend::new(temp_dir.path()).await.unwrap();
        let backend2 = backend1.clone();

        // Write with backend1
        backend1.put("key1", b"data1").await.unwrap();

        // Read with backend2 (should see the same data)
        assert_eq!(backend2.get("key1").await.unwrap(), b"data1");

        // Write with backend2
        backend2.put("key2", b"data2").await.unwrap();

        // Read with backend1 (should see the new data)
        assert_eq!(backend1.get("key2").await.unwrap(), b"data2");
    }

    #[tokio::test]
    async fn test_short_keys() {
        let temp_dir = TempDir::new().unwrap();
        let backend = LocalBackend::new(temp_dir.path()).await.unwrap();

        // Test with 1-char key
        backend.put("a", b"data").await.unwrap();
        assert_eq!(backend.get("a").await.unwrap(), b"data");

        // Test with 2-char key
        backend.put("ab", b"data").await.unwrap();
        assert_eq!(backend.get("ab").await.unwrap(), b"data");

        // Test with 3-char key
        backend.put("abc", b"data").await.unwrap();
        assert_eq!(backend.get("abc").await.unwrap(), b"data");
    }

    #[tokio::test]
    async fn test_new_sync() {
        let temp_dir = TempDir::new().unwrap();
        let path = temp_dir.path().join("sync_storage");

        assert!(!path.exists());
        let backend = LocalBackend::new_sync(&path).unwrap();
        assert!(path.exists());
        assert_eq!(backend.root(), &path);
    }

    #[tokio::test]
    async fn test_mmap_read() {
        let temp_dir = TempDir::new().unwrap();
        let backend = LocalBackend::new(temp_dir.path()).await.unwrap();

        // Write test data
        let test_data = b"Memory-mapped test data for reading";
        backend.put("mmap_test", test_data).await.unwrap();

        // Read using mmap
        let mmap = backend.get_mmap("mmap_test").unwrap();
        assert_eq!(mmap.as_ref(), test_data);
    }

    #[tokio::test]
    async fn test_mmap_large_file() {
        let temp_dir = TempDir::new().unwrap();
        let backend = LocalBackend::new(temp_dir.path()).await.unwrap();

        // Write 5MB file
        let large_data = vec![0xABu8; 5 * 1024 * 1024];
        backend.put("large_mmap_test", &large_data).await.unwrap();

        // Read using mmap
        let mmap = backend.get_mmap("large_mmap_test").unwrap();
        assert_eq!(mmap.len(), 5 * 1024 * 1024);
        assert_eq!(&mmap[..100], &large_data[..100]);
        assert_eq!(
            &mmap[mmap.len() - 100..],
            &large_data[large_data.len() - 100..]
        );
    }

    #[tokio::test]
    async fn test_get_size() {
        let temp_dir = TempDir::new().unwrap();
        let backend = LocalBackend::new(temp_dir.path()).await.unwrap();

        let test_data = vec![0u8; 12345];
        backend.put("size_test", &test_data).await.unwrap();

        let size = backend.get_size("size_test").await.unwrap();
        assert_eq!(size, 12345);
    }

    #[tokio::test]
    async fn test_adaptive_loading_small() {
        let temp_dir = TempDir::new().unwrap();
        let backend = LocalBackend::new(temp_dir.path()).await.unwrap();

        // Small file (<10MB) should use Vec
        let small_data = vec![1u8; 1024 * 1024]; // 1MB
        backend.put("small_adaptive", &small_data).await.unwrap();

        let result = backend.get_adaptive("small_adaptive").await.unwrap();
        match &result {
            super::MmapOrVec::Vec(v) => assert_eq!(v.len(), 1024 * 1024),
            super::MmapOrVec::Mmap(_) => panic!("Expected Vec for small file"),
        }
        assert_eq!(result.as_ref(), &small_data[..]);
    }

    #[tokio::test]
    async fn test_adaptive_loading_large() {
        let temp_dir = TempDir::new().unwrap();
        let backend = LocalBackend::new(temp_dir.path()).await.unwrap();

        // Large file (>10MB) should use mmap
        let large_data = vec![2u8; 11 * 1024 * 1024]; // 11MB
        backend.put("large_adaptive", &large_data).await.unwrap();

        let result = backend.get_adaptive("large_adaptive").await.unwrap();
        match &result {
            super::MmapOrVec::Mmap(m) => assert_eq!(m.len(), 11 * 1024 * 1024),
            super::MmapOrVec::Vec(_) => panic!("Expected Mmap for large file"),
        }
        assert_eq!(result.as_ref().len(), large_data.len());
    }

    #[tokio::test]
    async fn test_mmap_or_vec_as_ref() {
        // Test that MmapOrVec::as_ref works correctly for both variants
        let temp_dir = TempDir::new().unwrap();
        let backend = LocalBackend::new(temp_dir.path()).await.unwrap();

        let data = b"test data for as_ref";
        backend.put("asref_test", data).await.unwrap();

        // Small file gives Vec
        let result = backend.get_adaptive("asref_test").await.unwrap();
        let slice: &[u8] = result.as_ref();
        assert_eq!(slice, data);
    }

    // -- Layout v2: object_path shard-mapping table test -------------------

    #[test]
    fn test_object_path_v2_mapping_table() {
        let temp_dir = TempDir::new().unwrap();
        let backend = LocalBackend::new_sync(temp_dir.path()).unwrap();
        let root = temp_dir.path();
        let hash = "deadbeef00112233445566778899aabbccddeeff00112233445566778899aa";

        let cases: Vec<(String, PathBuf)> = vec![
            // Bare OID (no namespace, no dir component) -> objects/<h0:2>/<h2:4>/<hash>
            (
                hash.to_string(),
                root.join("objects").join("de").join("ad").join(hash),
            ),
            // Namespaced bare OID -> <ns>/objects/<h0:2>/<h2:4>/<hash>
            (
                format!("myrepo/{hash}"),
                root.join("myrepo")
                    .join("objects")
                    .join("de")
                    .join("ad")
                    .join(hash),
            ),
            // chunks/<hash> -> chunks/<h0:2>/<h2:4>/<hash>
            (
                format!("chunks/{hash}"),
                root.join("chunks").join("de").join("ad").join(hash),
            ),
            // chunk-deltas/<hash>.meta co-locates with its binary sibling
            (
                format!("chunk-deltas/{hash}.meta"),
                root.join("chunk-deltas")
                    .join("de")
                    .join("ad")
                    .join(format!("{hash}.meta")),
            ),
            (
                format!("chunk-deltas/{hash}"),
                root.join("chunk-deltas").join("de").join("ad").join(hash),
            ),
            // manifests/<hash>
            (
                format!("manifests/{hash}"),
                root.join("manifests").join("de").join("ad").join(hash),
            ),
            // deltas/<hash>.meta
            (
                format!("deltas/{hash}.meta"),
                root.join("deltas")
                    .join("de")
                    .join("ad")
                    .join(format!("{hash}.meta")),
            ),
            // packs/<pack_oid> -> packs/<p0:2>/<pack_oid> (single-level shard)
            (
                format!("packs/{hash}"),
                root.join("packs").join("de").join(hash),
            ),
            // Namespaced packs
            (
                format!("myrepo/packs/{hash}"),
                root.join("myrepo").join("packs").join("de").join(hash),
            ),
            // Short basenames: padded, never panics
            (
                "ab".to_string(),
                root.join("objects").join("ab").join("__").join("ab"),
            ),
            (
                "a".to_string(),
                root.join("objects").join("a_").join("__").join("a"),
            ),
        ];

        for (key, expected) in cases {
            assert_eq!(
                backend.object_path(&key),
                expected,
                "mismatch for key {key:?}"
            );
        }
    }

    #[tokio::test]
    async fn test_list_objects_v2_round_trip_all_namespaces() {
        let temp_dir = TempDir::new().unwrap();
        let backend = LocalBackend::new(temp_dir.path()).await.unwrap();
        let h1 = "1111111111111111111111111111111111111111111111111111111111111111";
        let h2 = "2222222222222222222222222222222222222222222222222222222222222222";
        let h3 = "3333333333333333333333333333333333333333333333333333333333333333";

        let mut keys = vec![
            format!("myrepo/{}", &h1[..64]),
            format!("myrepo/chunks/{}", &h2[..64]),
            format!("myrepo/chunk-deltas/{}", &h3[..64]),
            format!("myrepo/chunk-deltas/{}.meta", &h3[..64]),
            format!("myrepo/manifests/{}", &h2[..64]),
            format!("myrepo/deltas/{}", &h1[..64]),
            format!("myrepo/deltas/{}.meta", &h1[..64]),
            format!("myrepo/packs/{}", &h3[..64]),
        ];
        keys.sort();

        for k in &keys {
            backend.put(k, b"payload").await.unwrap();
        }

        let mut listed = backend.list_objects("").await.unwrap();
        listed.sort();
        assert_eq!(
            listed, keys,
            "round-trip logical key set must match exactly"
        );

        // Prefix filtering still works post-rewrite.
        let chunk_keys = backend.list_objects("myrepo/chunks/").await.unwrap();
        assert_eq!(chunk_keys.len(), 1);
    }

    #[tokio::test]
    async fn test_layout_marker_unsharded_and_excluded_from_listing() {
        let temp_dir = TempDir::new().unwrap();
        let backend = LocalBackend::new(temp_dir.path()).await.unwrap();

        backend.put("myrepo/LAYOUT", b"2").await.unwrap();
        backend.put("myrepo/chunks/abc", b"data").await.unwrap();

        assert_eq!(
            backend.object_path("myrepo/LAYOUT"),
            temp_dir.path().join("myrepo").join("LAYOUT")
        );
        assert_eq!(backend.get("myrepo/LAYOUT").await.unwrap(), b"2");

        // LAYOUT must never appear as a logical object key.
        let listed = backend.list_objects("").await.unwrap();
        assert_eq!(listed, vec!["myrepo/chunks/abc".to_string()]);
    }

    #[tokio::test]
    async fn test_crash_mid_put_recovers_under_layout_v2_paths() {
        // Simulates a crash between the temp-file write and the atomic
        // rename: a stale ".tmpN" file is left sitting next to where a
        // layout-v2 (nested, namespaced) key's final path would be. A
        // subsequent `put()` for the same key must still succeed and
        // produce the correct final content — the atomic tmp+rename
        // pattern must survive object_path()'s deeper v2 paths (previously
        // `root/objects/ab/cd/key`, now e.g. `root/ns/chunks/ab/cd/key`).
        let temp_dir = TempDir::new().unwrap();
        let backend = LocalBackend::new(temp_dir.path()).await.unwrap();
        let key = "myrepo/chunks/deadbeef00112233445566778899aabbccddeeff00112233445566778899aa";

        let final_path = backend.object_path(key);
        fs::create_dir_all(final_path.parent().unwrap()).unwrap();
        let stale_temp = final_path.with_extension("tmp999");
        fs::write(&stale_temp, b"leftover from a crashed writer").unwrap();

        backend.put(key, b"recovered content").await.unwrap();

        assert_eq!(backend.get(key).await.unwrap(), b"recovered content");
        // The crashed writer's stale temp file must not resurface as data.
        let listed = backend.list_objects("").await.unwrap();
        assert_eq!(listed, vec![key.to_string()]);
    }

    #[test]
    fn test_layout_v2_doc_examples_from_roadmap() {
        // Pin the exact examples from docs/ROADMAP-2026-07-07-layoutv2-p1p2.md
        // ("New shard rule") so a future edit to object_path() that silently
        // changes these breaks a test, not just a diagram.
        let temp_dir = TempDir::new().unwrap();
        let backend = LocalBackend::new_sync(temp_dir.path()).unwrap();
        let hash = "deadbeef00112233445566778899aabbccddeeff00112233445566778899aa";
        assert_eq!(
            backend.object_path(&format!("chunks/{hash}")),
            temp_dir
                .path()
                .join("chunks")
                .join("de")
                .join("ad")
                .join(hash)
        );
        assert_eq!(
            backend.object_path(&format!("chunk-deltas/{hash}.meta")),
            temp_dir
                .path()
                .join("chunk-deltas")
                .join("de")
                .join("ad")
                .join(format!("{hash}.meta"))
        );
        assert_eq!(
            backend.object_path(&format!("packs/{hash}")),
            temp_dir.path().join("packs").join("de").join(hash)
        );
    }
}
