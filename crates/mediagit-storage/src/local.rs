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
//! # Directory Structure
//!
//! Objects are stored in a sharded directory layout:
//! ```text
//! root/
//!   objects/
//!     ab/
//!       cd/
//!         abcd1234567890...
//! ```
//!
//! For a key like "abcd1234567890", the path becomes:
//! `root/objects/ab/cd/abcd1234567890`
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
use tokio::fs;
use tokio::io::AsyncWriteExt;

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

    /// Get the path for a given key with sharding
    ///
    /// Sharding layout for objects: `root/objects/AB/CD/key` where:
    /// - AB is the first 2 characters of the key
    /// - CD is the next 2 characters (if key is 4+ chars)
    /// - key is the full key (with "/" encoded as "::")
    ///
    /// Special handling for pack files:
    /// - Keys starting with "packs/" are stored directly without sharding
    /// - Example: "packs/pack-123.pack" → `root/packs/pack-123.pack`
    ///
    /// This allows keys with "/" in them (like "images/photo1.jpg").
    /// Since "/" cannot appear in filenames, we encode it as "::".
    ///
    /// # Arguments
    ///
    /// * `key` - The object key (can contain "/" for hierarchical keys)
    ///
    /// # Returns
    ///
    /// Full path for the object
    ///
    /// # Examples
    ///
    /// For key "abcd1234567890":
    /// - Returns: `root/objects/ab/cd/abcd1234567890`
    ///
    /// For key "images/photo1.jpg":
    /// - Returns: `root/objects/im/ag/images__photo1.jpg`
    ///
    /// For key "packs/pack-123.pack":
    /// - Returns: `root/packs/pack-123.pack`
    fn object_path(&self, key: &str) -> PathBuf {
        // Special case: pack files should not be sharded
        // They are stored directly under root/packs/
        if key.starts_with("packs/") {
            return self.root.join(key);
        }

        // Encode "/" as "__" to allow keys with "/" in filenames
        // Note: We use "__" instead of "::" for Windows compatibility (":" is reserved)
        let encoded_key = key.replace('/', "__");

        if key.len() >= 4 {
            // For keys with 4+ chars: use shard1/shard2/key layout
            let shard1 = &key[0..2];
            let shard2 = &key[2..4];
            self.root
                .join("objects")
                .join(shard1)
                .join(shard2)
                .join(&encoded_key)
        } else if key.len() >= 2 {
            // For keys with 2-3 chars: use shard1/key layout
            let shard1 = &key[0..2];
            self.root.join("objects").join(shard1).join(&encoded_key)
        } else {
            // For single-char keys: just store directly under objects
            self.root.join("objects").join(&encoded_key)
        }
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
            if !parent.exists() {
                fs::create_dir_all(parent).await?;
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
    pub fn get_mmap(&self, key: &str) -> anyhow::Result<memmap2::Mmap> {
        if key.is_empty() {
            return Err(anyhow::anyhow!("key cannot be empty"));
        }

        let path = self.object_path(key);
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

        let path = self.object_path(key);
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
            Ok(MmapOrVec::Vec(fs::read(self.object_path(key)).await?))
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

        let path = self.object_path(key);

        match fs::read(&path).await {
            Ok(data) => Ok(data),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                Err(anyhow::anyhow!("object not found: {}", key))
            }
            Err(e) => Err(e.into()),
        }
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

        let path = self.object_path(key);
        self.ensure_parent_dir(&path).await?;

        // Write to a temporary file first
        let temp_path = path.with_extension("tmp");

        // Remove any stale temp file
        let _ = fs::remove_file(&temp_path).await;

        // Create and write to temp file
        let mut file = fs::File::create(&temp_path).await?;
        file.write_all(data).await?;
        file.sync_all().await?;
        drop(file);

        // Atomically rename temp file to final location
        fs::rename(&temp_path, &path).await?;

        Ok(())
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

        let path = self.object_path(key);
        match fs::try_exists(&path).await {
            Ok(exists) => Ok(exists),
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

        let path = self.object_path(key);

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
    /// Recursively walks the appropriate directory structure:
    /// - For "packs/" prefix: searches root/packs/ directly
    /// - For other prefixes: searches root/objects/ with sharding
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

        // Special case: pack files are stored directly under root/packs/
        if prefix.starts_with("packs/") {
            let packs_dir = self.root.join("packs");

            // If packs directory doesn't exist, return empty list
            if !packs_dir.exists() {
                return Ok(Vec::new());
            }

            // List files directly in packs/ directory
            Self::walk_dir_flat(&packs_dir, prefix, &mut results).await?;
        } else {
            // Regular objects: search in sharded objects/ directory
            let objects_dir = self.root.join("objects");

            // If objects directory doesn't exist, return empty list
            if !objects_dir.exists() {
                return Ok(Vec::new());
            }

            Self::walk_dir_iterative(&objects_dir, &objects_dir, prefix, &mut results).await?;
        }

        results.sort();
        Ok(results)
    }
}

// Helper function for iterative directory traversal
impl LocalBackend {
    /// Walk a flat directory (like packs/) and collect matching keys
    ///
    /// For directories that don't use sharding (like packs/), list files directly
    /// and reconstruct keys by prepending the directory name.
    ///
    /// # Arguments
    ///
    /// * `dir` - The directory to walk (e.g., root/packs/)
    /// * `prefix` - The key prefix to filter by (e.g., "packs/")
    /// * `results` - Vector to collect matching keys
    async fn walk_dir_flat(
        dir: &Path,
        prefix: &str,
        results: &mut Vec<String>,
    ) -> anyhow::Result<()> {
        let mut entries = match fs::read_dir(dir).await {
            Ok(entries) => entries,
            Err(_) => return Ok(()), // Directory doesn't exist or can't be read
        };

        while let Some(entry) = entries.next_entry().await? {
            let path = entry.path();

            if path.is_file() {
                // For packs directory: reconstruct key as "packs/filename"
                if let Some(filename) = path.file_name() {
                    if let Some(filename_str) = filename.to_str() {
                        let key = format!("packs/{}", filename_str);
                        if key.starts_with(prefix) {
                            results.push(key);
                        }
                    }
                }
            }
        }

        Ok(())
    }

    /// Iteratively walk directory tree and collect matching keys
    /// Uses a work queue to avoid recursive async function issues
    ///
    /// Reconstructs keys by removing the shard directories (first 2-4 path components)
    /// that were added during storage.
    async fn walk_dir_iterative(
        objects_dir: &Path,
        objects_base: &Path,
        prefix: &str,
        results: &mut Vec<String>,
    ) -> anyhow::Result<()> {
        let mut work_queue = vec![objects_dir.to_path_buf()];

        while let Some(current_path) = work_queue.pop() {
            let mut entries = match fs::read_dir(&current_path).await {
                Ok(entries) => entries,
                Err(_) => continue, // Skip directories we can't read
            };

            while let Some(entry) = entries.next_entry().await? {
                let path = entry.path();

                if path.is_dir() {
                    // Add to work queue for processing
                    work_queue.push(path);
                } else {
                    // Reconstruct the key by removing shard directories
                    // Path structure: objects/AB/CD/key or objects/AB/key or objects/key
                    // The key is the last component (or components after the shard dirs)

                    if let Ok(relative_path) = path.strip_prefix(objects_base) {
                        let components: Vec<_> = relative_path
                            .components()
                            .map(|c| c.as_os_str().to_string_lossy().to_string())
                            .collect();

                        // Reconstruct the original key by analyzing the path structure
                        // Path structures:
                        // - 1-char key: objects/X -> key is "X"
                        // - 2-3 char key: objects/AB/encoded_key -> key is the filename
                        // - 4+ char key: objects/AB/CD/encoded_key -> key is the filename
                        // Filenames have "/" encoded as "::" which needs to be decoded
                        let mut key = if components.is_empty() {
                            continue;
                        } else if components.len() == 1 {
                            // Single-char key stored at objects/X
                            components[0].clone()
                        } else {
                            // For 2+ components, the key is always in the last component (the filename)
                            // The shard dirs (1st and maybe 2nd component) are just organizational
                            components.last().unwrap().clone()
                        };

                        // Decode "__" back to "/"
                        key = key.replace("__", "/");

                        // Filter by prefix
                        if key.starts_with(prefix) {
                            results.push(key);
                        }
                    }
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
        assert_eq!(&mmap[mmap.len() - 100..], &large_data[large_data.len() - 100..]);
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
}

