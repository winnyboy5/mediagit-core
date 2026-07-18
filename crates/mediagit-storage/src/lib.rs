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

#![allow(missing_docs)]
//! Storage abstraction layer for MediaGit
//!
//! This crate provides a unified, asynchronous storage interface that supports multiple backends:
//! - Local filesystem (via `mediagit-local-storage`)
//! - AWS S3
//! - Azure Blob Storage
//! - Google Cloud Storage
//! - MinIO / S3-compatible
//! - Backblaze B2 / DigitalOcean Spaces
//!
//! # Architecture
//!
//! The `StorageBackend` trait defines a minimal but complete interface for object storage
//! operations, allowing implementations to handle various storage systems transparently.
//!
//! ## Core Concepts
//!
//! - **Keys**: Unique identifiers for stored objects (strings, typically hierarchical like file paths)
//! - **Objects**: Arbitrary binary data associated with a key
//! - **Prefixes**: String prefixes used for listing and organization (similar to S3 object prefixes)
//!
//! # Features
//!
//! - **Async-first**: All operations are async using `tokio` for non-blocking I/O
//! - **Thread-safe**: All implementations must be `Send + Sync` for safe concurrent use
//! - **Debuggable**: All implementations must implement `Debug`
//! - **Error handling**: Uses `anyhow::Result` for ergonomic error management
//!
//! # Examples
//!
//! Using the mock backend for testing:
//!
//! ```no_run
//! use mediagit_storage::{StorageBackend, mock::MockBackend};
//!
//! #[tokio::main]
//! async fn main() -> anyhow::Result<()> {
//!     // Create an in-memory backend for testing
//!     let storage = MockBackend::new();
//!
//!     // Store data
//!     storage.put("documents/resume.pdf", b"PDF content").await?;
//!
//!     // Retrieve data
//!     let data = storage.get("documents/resume.pdf").await?;
//!     assert_eq!(data, b"PDF content");
//!
//!     // Check existence
//!     if storage.exists("documents/resume.pdf").await? {
//!         println!("File exists");
//!     }
//!
//!     // List objects with prefix
//!     let documents = storage.list_objects("documents/").await?;
//!     println!("Found {} documents", documents.len());
//!
//!     // Delete object
//!     storage.delete("documents/resume.pdf").await?;
//!
//!     Ok(())
//! }
//! ```
//!
//! # Implementation Guide
//!
//! When implementing `StorageBackend`:
//!
//! 1. Use `#[async_trait]` macro on your impl block
//! 2. Return `anyhow::Result<T>` for all operations
//! 3. Ensure your type implements `Send + Sync + Debug`
//! 4. Handle empty keys gracefully (typically return an error)
//! 5. List operations should return sorted results for consistency
//! 6. Deleting non-existent objects should succeed (idempotent)
//!
//! # Error Handling
//!
//! While the trait uses `anyhow::Result`, consider using the `StorageError` enum
//! in `error.rs` for more structured error information:
//!
//! ```no_run
//! use mediagit_storage::error::{StorageError, StorageResult};
//!
//! fn validate_key(key: &str) -> StorageResult<()> {
//!     if key.is_empty() {
//!         Err(StorageError::invalid_key("key cannot be empty"))
//!     } else {
//!         Ok(())
//!     }
//! }
//! ```

#[cfg(feature = "azure")]
pub mod azure;
pub mod b2_spaces;
pub mod cache;
pub mod error;
#[cfg(feature = "gcs")]
pub mod gcs;
pub(crate) mod http_pool;
pub mod local;
pub mod minio;
pub mod mock;
pub mod namespaced;
pub mod s3;

use async_trait::async_trait;
use std::fmt::Debug;
use tokio::io::{AsyncRead, AsyncReadExt};

#[cfg(feature = "azure")]
pub use azure::AzureBackend;
pub use b2_spaces::B2SpacesBackend;
pub use error::{StorageError, StorageResult};
#[cfg(feature = "gcs")]
pub use gcs::{GcsBackend, GcsConfig};
pub use local::LocalBackend;
pub use minio::MinIOBackend;
pub use namespaced::{generate_repo_id, sanitize_namespace, NamespacedBackend};
pub use s3::S3Backend;

/// Storage backend trait for object storage operations
///
/// This trait defines the minimal interface for object storage systems.
/// Implementations must be async-safe, thread-safe, and handle errors gracefully.
///
/// # Safety Requirements
///
/// All implementations must:
/// - Be `Send` to cross thread boundaries
/// - Be `Sync` for safe concurrent access
/// - Implement `Debug` for observability
/// - Be thread-safe and support concurrent operations
///
/// # Error Handling
///
/// All operations return `anyhow::Result<T>` to allow flexible error context.
/// Operations should return `Err` for:
/// - `get`: Key doesn't exist (use "object not found" message)
/// - `put`: Permission denied, quota exceeded, or I/O errors
/// - `exists`: Typically only I/O or permission errors
/// - `delete`: Typically succeeds even if object doesn't exist (idempotent)
/// - `list_objects`: Permission denied or I/O errors
///
/// # Examples
///
/// See [`mock::MockBackend`] for a complete example implementation.
///
/// ```rust,no_run
/// # use mediagit_storage::{StorageBackend, mock::MockBackend};
/// #[tokio::main]
/// async fn example() -> anyhow::Result<()> {
///     let backend: Box<dyn StorageBackend> = Box::new(MockBackend::new());
///
///     backend.put("my_key", b"my_data").await?;
///     let retrieved = backend.get("my_key").await?;
///     assert_eq!(retrieved, b"my_data");
///
///     Ok(())
/// }
/// ```
/// Metadata returned by [`StorageBackend::presign_put`] for direct-to-backend uploads.
///
/// The client performs `PUT url` with the required headers; the server never
/// sees the chunk bytes. When the backend cannot issue presigned URLs (local
/// filesystem, mock, or cloud backends without key-based credentials),
/// `presign_put` returns `Ok(None)` and the caller falls back to the existing
/// server-proxied PUT route.
#[derive(Debug, Clone)]
pub struct PresignedPut {
    /// The presigned URL the client PUTs bytes to directly.
    pub url: String,
    /// HTTP method string — always "PUT" for all current backends.
    pub method: String,
    /// Headers the client MUST send verbatim (e.g. `x-amz-*` SigV4 headers).
    pub required_headers: Vec<(String, String)>,
    /// Absolute expiry instant; client should not attempt the URL after this.
    pub expires_at: std::time::SystemTime,
}

/// Metadata returned by [`StorageBackend::presign_get`] for direct-from-backend downloads.
///
/// The client performs `GET url` with the required headers; the server never
/// sees the chunk bytes. When the backend cannot issue presigned URLs,
/// `presign_get` returns `Ok(None)` and the caller falls back to the existing
/// server-proxied GET route.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct PresignedDownload {
    /// The presigned URL the client GETs bytes from directly.
    pub url: String,
    /// Headers the client MUST send verbatim with the GET request.
    pub headers: Vec<(String, String)>,
    /// TTL in seconds from the time the URL was minted.
    pub expires_in_secs: u64,
}

/// Result of [`StorageBackend::create_presigned_mpu`]: one presigned `UploadPart`
/// URL per part. The client PUTs each part directly, collects the `ETag` header,
/// then calls the server's `chunks/mpu/complete` endpoint.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct PresignedMpu {
    pub upload_id: String,
    pub parts: Vec<PresignedMpuPart>,
    /// Recommended part size in bytes; the last part may be smaller.
    pub part_size: u64,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct PresignedMpuPart {
    pub part_number: i32,
    pub url: String,
}

/// A completed part the client reports back to finalize a multipart upload.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct MpuCompletedPart {
    pub part_number: i32,
    pub etag: String,
}

#[async_trait]
pub trait StorageBackend: Send + Sync + Debug {
    /// Retrieve an object by its key
    ///
    /// # Arguments
    ///
    /// * `key` - The object identifier (non-empty string)
    ///
    /// # Returns
    ///
    /// * `Ok(Vec<u8>)` - The object data
    /// * `Err` - If the key doesn't exist or an I/O error occurs
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - The key doesn't exist (should use "object not found" in the error message)
    /// - An I/O error occurs
    /// - Permission is denied
    /// - The key is empty
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// # use mediagit_storage::{StorageBackend, mock::MockBackend};
    /// # #[tokio::main]
    /// # async fn main() -> anyhow::Result<()> {
    /// let storage = MockBackend::new();
    /// storage.put("document.pdf", b"content").await?;
    ///
    /// let data = storage.get("document.pdf").await?;
    /// assert_eq!(data, b"content");
    /// # Ok(())
    /// # }
    /// ```
    async fn get(&self, key: &str) -> anyhow::Result<Vec<u8>>;

    /// Retrieve a byte range from an object (`offset` inclusive, `len` bytes).
    ///
    /// Default implementation fetches the whole object and slices it.
    /// Backends may override for efficient HTTP Range-GET.
    async fn get_range(&self, key: &str, offset: u64, len: u64) -> anyhow::Result<Vec<u8>> {
        let data = self.get(key).await?;
        let start = offset as usize;
        let end = start + len as usize;
        if end > data.len() {
            anyhow::bail!(
                "get_range: {}..{} out of bounds for key '{}' (object len {})",
                offset,
                end,
                key,
                data.len()
            );
        }
        Ok(data[start..end].to_vec())
    }

    /// Stream an object as a sequence of `Bytes` chunks (B7).
    ///
    /// Default impl fetches the full object via `get` and emits it as a single chunk.
    /// Backends may override with a native streaming implementation for better memory
    /// efficiency on large objects. Gated by `MEDIAGIT_STORAGE_STREAMING=1` (OFF by default).
    async fn get_streaming(
        &self,
        key: &str,
    ) -> anyhow::Result<
        std::pin::Pin<
            Box<dyn futures::Stream<Item = anyhow::Result<bytes::Bytes>> + Send + 'static>,
        >,
    > {
        let data = self.get(key).await?;
        let stream = futures::stream::once(async move {
            Ok::<bytes::Bytes, anyhow::Error>(bytes::Bytes::from(data))
        });
        Ok(Box::pin(stream))
    }

    /// Stream a byte-range of an object (F5 — reserved for Track F cloud-pack Range GETs).
    ///
    /// Default impl returns `Unsupported`. S3/GCS/Azure native impls will be added in Track F.
    async fn get_streaming_range(
        &self,
        _key: &str,
        _range: std::ops::Range<u64>,
    ) -> anyhow::Result<
        std::pin::Pin<
            Box<dyn futures::Stream<Item = anyhow::Result<bytes::Bytes>> + Send + 'static>,
        >,
    > {
        anyhow::bail!("get_streaming_range is not yet implemented for this backend")
    }

    /// Store an object from a local temp file (B4 stream-to-disk).
    ///
    /// Default impl reads the file into memory and calls `put`. `LocalBackend`
    /// overrides with an atomic rename so the data is never re-buffered in RAM.
    /// Gated by `MEDIAGIT_STREAM_CHUNK_TO_DISK=1` in the protocol client.
    async fn put_file(&self, key: &str, src: &std::path::Path) -> anyhow::Result<()> {
        let data = tokio::fs::read(src).await?;
        self.put(key, &data).await
    }

    /// Store an object with the given key
    ///
    /// This operation is idempotent: calling it multiple times with the same key
    /// will overwrite previous data.
    ///
    /// # Arguments
    ///
    /// * `key` - The object identifier (non-empty string)
    /// * `data` - The object content (can be empty)
    ///
    /// # Returns
    ///
    /// * `Ok(())` - The operation succeeded
    /// * `Err` - If an I/O error occurs or permission is denied
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - An I/O error occurs
    /// - Permission is denied
    /// - Storage quota exceeded
    /// - The key is empty
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// # use mediagit_storage::{StorageBackend, mock::MockBackend};
    /// # #[tokio::main]
    /// # async fn main() -> anyhow::Result<()> {
    /// let storage = MockBackend::new();
    ///
    /// let data = vec![0x89, 0x50, 0x4E, 0x47]; // PNG magic bytes
    /// storage.put("image.png", &data).await?;
    /// # Ok(())
    /// # }
    /// ```
    async fn put(&self, key: &str, data: &[u8]) -> anyhow::Result<()>;

    /// Check if an object exists
    ///
    /// # Arguments
    ///
    /// * `key` - The object identifier (non-empty string)
    ///
    /// # Returns
    ///
    /// * `Ok(true)` - The object exists
    /// * `Ok(false)` - The object doesn't exist
    /// * `Err` - If an I/O error occurs or permission is denied
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - An I/O error occurs
    /// - Permission is denied
    /// - The key is empty
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// # use mediagit_storage::{StorageBackend, mock::MockBackend};
    /// # #[tokio::main]
    /// # async fn main() -> anyhow::Result<()> {
    /// let storage = MockBackend::new();
    /// storage.put("file.txt", b"content").await?;
    ///
    /// assert!(storage.exists("file.txt").await?);
    /// assert!(!storage.exists("missing.txt").await?);
    /// # Ok(())
    /// # }
    /// ```
    async fn exists(&self, key: &str) -> anyhow::Result<bool>;

    /// Delete an object
    ///
    /// This operation is idempotent: deleting a non-existent object should succeed.
    ///
    /// # Arguments
    ///
    /// * `key` - The object identifier (non-empty string)
    ///
    /// # Returns
    ///
    /// * `Ok(())` - The operation succeeded (whether the object existed or not)
    /// * `Err` - If an I/O error occurs or permission is denied
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - An I/O error occurs
    /// - Permission is denied
    /// - The key is empty
    ///
    /// Note: Most implementations should return `Ok(())` for non-existent keys
    /// to support idempotent deletion.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// # use mediagit_storage::{StorageBackend, mock::MockBackend};
    /// # #[tokio::main]
    /// # async fn main() -> anyhow::Result<()> {
    /// let storage = MockBackend::new();
    /// storage.put("temp.dat", b"temporary").await?;
    ///
    /// storage.delete("temp.dat").await?;
    /// assert!(!storage.exists("temp.dat").await?);
    ///
    /// // Deleting again should succeed (idempotent)
    /// storage.delete("temp.dat").await?;
    /// # Ok(())
    /// # }
    /// ```
    async fn delete(&self, key: &str) -> anyhow::Result<()>;

    /// List objects with a given prefix
    ///
    /// Returns a sorted list of all keys that start with the given prefix.
    /// Useful for organization and bulk operations.
    ///
    /// # Arguments
    ///
    /// * `prefix` - The key prefix to filter by (can be empty to list all)
    ///
    /// # Returns
    ///
    /// * `Ok(Vec<String>)` - Sorted list of matching keys (can be empty)
    /// * `Err` - If an I/O error occurs or permission is denied
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - An I/O error occurs
    /// - Permission is denied
    ///
    /// # Implementation Notes
    ///
    /// - Results should be sorted alphabetically for consistency
    /// - An empty prefix should return all keys
    /// - No keys should return an empty vec, not an error
    /// - Prefix matching should be exact string prefix matching
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// # use mediagit_storage::{StorageBackend, mock::MockBackend};
    /// # #[tokio::main]
    /// # async fn main() -> anyhow::Result<()> {
    /// let storage = MockBackend::new();
    /// storage.put("images/photo1.jpg", b"data").await?;
    /// storage.put("images/photo2.jpg", b"data").await?;
    /// storage.put("videos/video1.mp4", b"data").await?;
    ///
    /// let images = storage.list_objects("images/").await?;
    /// assert_eq!(images.len(), 2);
    ///
    /// let all = storage.list_objects("").await?;
    /// assert_eq!(all.len(), 3);
    /// # Ok(())
    /// # }
    /// ```
    async fn list_objects(&self, prefix: &str) -> anyhow::Result<Vec<String>>;

    /// Retrieve an object with an optional caller-supplied size hint.
    ///
    /// Backends that support parallel range reads (e.g. GCS striped downloads)
    /// may use the hint to issue concurrent ranged requests without an extra
    /// metadata round-trip. Callers that already know the object size — for
    /// example, chunk readers consulting a manifest that carries `size` — should
    /// prefer this method over [`get`] for large objects.
    ///
    /// The default implementation ignores the hint and delegates to [`get`],
    /// so backends that don't benefit from striping (filesystem, MinIO, mock)
    /// remain unchanged. Backends that do override this method MUST be
    /// byte-for-byte equivalent to [`get`] for the same key.
    ///
    /// # Arguments
    ///
    /// * `key`  - The object identifier (non-empty string).
    /// * `size` - Optional total object size in bytes. `None` means the caller
    ///            does not know the size; the backend MUST NOT issue an RPC to
    ///            discover it (that probe was the regression that got the prior
    ///            striped-get implementation reverted — keep the common path
    ///            single-RPC).
    async fn get_with_size_hint(&self, key: &str, _size: Option<u64>) -> anyhow::Result<Vec<u8>> {
        self.get(key).await
    }

    /// Store an object by streaming bytes from `reader`.
    ///
    /// Backends that support true streaming uploads (e.g. GCS resumable
    /// sessions, S3 multipart) MAY override this to pipe the reader directly
    /// to the wire without buffering the whole payload in memory. The default
    /// implementation drains the reader into a `Vec<u8>` (capacity hinted by
    /// `len` when non-zero) and delegates to [`put`], so non-streaming
    /// backends remain correct without code changes.
    ///
    /// `len` is an advisory upper bound used purely for buffer pre-sizing —
    /// callers MAY pass `0` when the size is unknown. The reader is the
    /// authoritative source of bytes; `len` is never trusted to truncate or
    /// pad data.
    ///
    /// # Arguments
    ///
    /// * `key`    - The object identifier (non-empty string).
    /// * `reader` - A boxed `AsyncRead` supplying the object body. Boxed +
    ///              `Unpin` to keep the trait object-safe; concrete callers
    ///              wrap their reader once at the boundary.
    /// * `len`    - Advisory total length in bytes; `0` if unknown.
    async fn put_streaming(
        &self,
        key: &str,
        mut reader: Box<dyn AsyncRead + Send + Unpin>,
        len: u64,
    ) -> anyhow::Result<()> {
        // Pre-size the buffer when the caller supplied a useful hint to avoid
        // grow-by-doubling allocations on big uploads.
        let mut buf = if len > 0 {
            Vec::with_capacity(len as usize)
        } else {
            Vec::new()
        };
        reader.read_to_end(&mut buf).await?;
        self.put(key, &buf).await
    }

    /// Generate a presigned PUT URL for `key` valid for `ttl`.
    ///
    /// Returns `Ok(Some(PresignedPut))` when the backend can sign a URL so the
    /// client may upload chunk bytes directly to the bucket, bypassing the
    /// server entirely. Returns `Ok(None)` when presigning is not supported
    /// (local filesystem, mock, or non-key-authenticated cloud backends) —
    /// callers MUST fall back to the existing `PUT /chunks/:id` proxy route.
    ///
    /// Errors are restricted to SDK-level failures; transient errors also
    /// return `Ok(None)` to trigger the same fallback rather than aborting
    /// the upload.
    async fn presign_put(
        &self,
        _key: &str,
        _content_length: u64,
        _ttl: std::time::Duration,
    ) -> anyhow::Result<Option<PresignedPut>> {
        Ok(None)
    }

    /// Returns `Ok(Some(PresignedDownload))` when the backend can sign a URL so the
    /// client may download chunk bytes directly from the bucket, bypassing the
    /// server entirely. Returns `Ok(None)` when presigning is not supported —
    /// callers MUST fall back to the existing `GET /chunks/:id` proxy route.
    async fn presign_get(
        &self,
        _key: &str,
        _ttl: std::time::Duration,
    ) -> anyhow::Result<Option<PresignedDownload>> {
        Ok(None)
    }

    /// Initiate a server-side multipart upload and return presigned `UploadPart` URLs.
    ///
    /// Returns `Ok(Some(PresignedMpu))` on S3/MinIO where the SDK exposes
    /// `upload_part().presigned()`. Returns `Ok(None)` for backends that do not
    /// support client-side MPU (local, mock, azure, gcs).
    async fn create_presigned_mpu(
        &self,
        _key: &str,
        _total_size: u64,
        _ttl: std::time::Duration,
    ) -> anyhow::Result<Option<PresignedMpu>> {
        Ok(None)
    }

    /// Complete a presigned multipart upload by submitting the collected ETags.
    /// Only called after a successful `create_presigned_mpu`.
    async fn complete_presigned_mpu(
        &self,
        _key: &str,
        _upload_id: &str,
        _parts: Vec<MpuCompletedPart>,
    ) -> anyhow::Result<()> {
        anyhow::bail!("MPU not supported by this backend")
    }

    /// Abort a presigned multipart upload, releasing uncommitted parts.
    /// Best-effort; returns `Ok(())` for backends that do not support MPU.
    async fn abort_presigned_mpu(&self, _key: &str, _upload_id: &str) -> anyhow::Result<()> {
        Ok(())
    }

    /// Return the byte length of an object without downloading it.
    ///
    /// Returns `Ok(Some(len))` when the object exists, `Ok(None)` when it
    /// does not. Propagates other errors (permission denied, I/O failure, etc.).
    async fn head(&self, key: &str) -> anyhow::Result<Option<u64>>;
}

/// Prepend `prefix` to `key`, separated by `/`.
///
/// An absent or empty prefix is a no-op so callers are not forced to
/// handle the `None` case in hot paths.
///
/// ```
/// use mediagit_storage::prefixed_key;
/// assert_eq!(prefixed_key(&None, "chunks/abc"), "chunks/abc");
/// assert_eq!(prefixed_key(&Some(String::new()), "chunks/abc"), "chunks/abc");
/// assert_eq!(prefixed_key(&Some("t1".into()), "chunks/abc"), "t1/chunks/abc");
/// assert_eq!(prefixed_key(&Some("t1/".into()), "chunks/abc"), "t1/chunks/abc");
/// ```
pub fn prefixed_key(prefix: &Option<String>, key: &str) -> String {
    match prefix.as_deref().filter(|p| !p.is_empty()) {
        Some(p) => format!("{}/{}", p.trim_end_matches('/'), key),
        None => key.to_string(),
    }
}

/// Reject storage keys/prefixes that could escape the intended storage root
/// via path traversal, absolute paths, or Windows drive/UNC prefixes.
///
/// This is the single choke point every [`StorageBackend`] implementation
/// and wrapper (notably [`NamespacedBackend`] and [`local::LocalBackend`])
/// must call before turning a caller-supplied key into a filesystem path or
/// remote object key. Without it, a user-controlled id containing `..`
/// reaches [`local::LocalBackend`]'s path join unrejected and can write or
/// read outside the repo's storage root (or, once namespaced, outside the
/// per-repo namespace — a cross-tenant escape).
///
/// Deliberately does NOT reject an empty string: `list_objects("")` (list
/// everything under a namespace) is a legitimate call with an empty prefix,
/// and the individual backends already enforce "key cannot be empty" for
/// `get`/`put`/`exists`/`delete`/`head` on their own.
///
/// Handles both `/`- and `\`-based traversal — this runs on Windows, where a
/// literal `..\` in a key is just as dangerous as `../`.
pub fn validate_object_key(key: &str) -> anyhow::Result<()> {
    // Normalize backslashes to forward slashes before parsing with `Path` so
    // `..\win`-style traversal is caught the same way on every platform,
    // not just Windows (where `\` is already a native separator).
    let normalized = key.replace('\\', "/");
    let path = std::path::Path::new(&normalized);

    if path.is_absolute() {
        anyhow::bail!("invalid storage key '{key}': absolute paths are not allowed");
    }

    for component in path.components() {
        match component {
            std::path::Component::ParentDir => {
                anyhow::bail!("invalid storage key '{key}': path traversal ('..') is not allowed");
            }
            std::path::Component::RootDir | std::path::Component::Prefix(_) => {
                anyhow::bail!(
                    "invalid storage key '{key}': absolute or drive-rooted paths are not allowed"
                );
            }
            _ => {}
        }
    }

    Ok(())
}

/// The reserved key used for the layout-version marker written at the
/// storage root (under the namespace, once wrapped in
/// [`NamespacedBackend`]). Not part of the logical object key space —
/// `LocalBackend` places it unsharded and excludes it from `list_objects`.
pub const LAYOUT_MARKER_KEY: &str = "LAYOUT";

/// Check (or, on a fresh/empty store, write) the `LAYOUT` version marker.
///
/// Called by both production storage factories (CLI `create_storage_backend`,
/// server `build_storage_backend`) right after wrapping the backend in
/// [`NamespacedBackend`], so every code path that opens a repo's storage
/// enforces this invariant.
///
/// `repo_id` identifies *this* repository (distinct from the namespace,
/// which defaults to a sanitized directory basename and can collide across
/// independently-created repos pointed at the same storage root/bucket).
/// The marker records it as `"<version> <repo_id>"` so a second, unrelated
/// repo that happens to compute the same namespace is refused instead of
/// silently merging into the first repo's key space (data loss via gc's
/// orphan sweep otherwise).
///
/// - Marker absent AND the namespace has no other keys yet → this is a
///   brand-new store (typically `init`/`clone`, which usually write the
///   marker explicitly themselves — this is the fallback for any other
///   caller that reaches an empty store first): write `expected_version` +
///   `repo_id`.
/// - Marker absent AND the namespace already has data → pre-layout-v2 data
///   with no marker. MediaGit is beta with no migration path, so this is a
///   hard error pointing at re-init/re-clone rather than silently applying
///   v2 physical-path rules to v1-shaped data.
/// - Marker present but its version doesn't match `expected_version` → hard
///   error (same re-init/re-clone guidance).
/// - Marker present, version matches, but carries no `repo_id` → written by
///   pre-namespace-collision-fix code this same beta cycle (single-owner was
///   the only shipped behavior at the time). Adopt it: write `repo_id` into
///   the marker and proceed.
/// - Marker present with a *different* `repo_id` → namespace collision: two
///   independently-created repos computed the same namespace against this
///   storage root/bucket. Hard error naming both the namespace and the
///   conflicting repo_id, with a hint to set the top-level `repo_namespace`
///   key in config.toml (BUG-RM-2: NOT nested under `[storage]` — that key
///   is silently ignored) or `MEDIAGIT_REPO_NAMESPACE`.
/// - Marker present with a matching `repo_id` → no-op.
pub async fn check_or_write_layout_marker(
    storage: &dyn StorageBackend,
    expected_version: u32,
    repo_id: &str,
) -> anyhow::Result<()> {
    match storage.get(LAYOUT_MARKER_KEY).await {
        Ok(data) => {
            let text = String::from_utf8_lossy(&data).trim().to_string();
            let mut parts = text.splitn(2, ' ');
            let found_version = parts.next().unwrap_or("");
            let found_repo_id = parts.next().map(|s| s.trim()).filter(|s| !s.is_empty());

            if found_version != expected_version.to_string() {
                anyhow::bail!(
                    "storage layout version mismatch: found '{found_version}', expected '{expected_version}'. \
                     MediaGit is in beta and does not migrate layouts automatically — \
                     re-init or re-clone this repository."
                );
            }

            match found_repo_id {
                None => {
                    // Pre-namespace-collision-fix marker from this same beta
                    // cycle: adopt it (single-owner was the only shipped
                    // behavior when it was written).
                    storage
                        .put(
                            LAYOUT_MARKER_KEY,
                            format!("{expected_version} {repo_id}").as_bytes(),
                        )
                        .await?;
                    Ok(())
                }
                Some(found) if found == repo_id => Ok(()),
                Some(found) => {
                    anyhow::bail!(
                        "storage namespace collision: this storage location is already owned by \
                         repo_id '{found}', but this repository is '{repo_id}'. Two independently \
                         created repositories appear to share the same namespace on this storage \
                         root/bucket — continuing would risk one repo's `gc` deleting the other's \
                         objects. Set a distinct top-level `repo_namespace` key in config.toml \
                         (not nested under `[storage]`) or the MEDIAGIT_REPO_NAMESPACE \
                         environment variable for one of them."
                    );
                }
            }
        }
        Err(_) => {
            let has_data = !storage
                .list_objects("")
                .await
                .unwrap_or_default()
                .is_empty();
            if has_data {
                anyhow::bail!(
                    "storage has existing data but no LAYOUT marker (pre-layout-v2). \
                     MediaGit is in beta and does not migrate layouts automatically — \
                     re-init or re-clone this repository."
                );
            }
            storage
                .put(
                    LAYOUT_MARKER_KEY,
                    format!("{expected_version} {repo_id}").as_bytes(),
                )
                .await?;
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mock::MockBackend;

    #[tokio::test]
    async fn layout_marker_written_on_fresh_empty_store() {
        let storage = MockBackend::new();
        check_or_write_layout_marker(&storage, 2, "repo-a")
            .await
            .unwrap();
        assert_eq!(storage.get(LAYOUT_MARKER_KEY).await.unwrap(), b"2 repo-a");
    }

    #[tokio::test]
    async fn layout_marker_matching_version_and_repo_id_is_noop() {
        let storage = MockBackend::new();
        storage.put(LAYOUT_MARKER_KEY, b"2 repo-a").await.unwrap();
        check_or_write_layout_marker(&storage, 2, "repo-a")
            .await
            .unwrap();
        assert_eq!(storage.get(LAYOUT_MARKER_KEY).await.unwrap(), b"2 repo-a");
    }

    #[tokio::test]
    async fn layout_marker_mismatch_is_hard_error() {
        let storage = MockBackend::new();
        storage.put(LAYOUT_MARKER_KEY, b"1 repo-a").await.unwrap();
        let err = check_or_write_layout_marker(&storage, 2, "repo-a")
            .await
            .unwrap_err();
        assert!(err.to_string().contains("layout version mismatch"));
        assert!(err.to_string().contains("re-init or re-clone"));
    }

    #[tokio::test]
    async fn layout_marker_missing_with_existing_data_is_hard_error() {
        let storage = MockBackend::new();
        // Pre-existing data, but no LAYOUT marker: pre-v2 layout.
        storage.put("chunks/abc", b"data").await.unwrap();
        let err = check_or_write_layout_marker(&storage, 2, "repo-a")
            .await
            .unwrap_err();
        assert!(err.to_string().contains("no LAYOUT marker"));
    }

    #[tokio::test]
    async fn layout_marker_without_repo_id_is_adopted() {
        // Marker written by pre-namespace-collision-fix code this cycle:
        // version only, no repo_id.
        let storage = MockBackend::new();
        storage.put(LAYOUT_MARKER_KEY, b"2").await.unwrap();
        check_or_write_layout_marker(&storage, 2, "repo-a")
            .await
            .unwrap();
        assert_eq!(storage.get(LAYOUT_MARKER_KEY).await.unwrap(), b"2 repo-a");
        // Second open with the same repo_id is a no-op.
        check_or_write_layout_marker(&storage, 2, "repo-a")
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn layout_marker_different_repo_id_is_collision_error() {
        let storage = MockBackend::new();
        check_or_write_layout_marker(&storage, 2, "repo-a")
            .await
            .unwrap();
        let err = check_or_write_layout_marker(&storage, 2, "repo-b")
            .await
            .unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("namespace collision"));
        assert!(msg.contains("repo-a"));
        assert!(msg.contains("repo-b"));
        assert!(msg.contains("MEDIAGIT_REPO_NAMESPACE"));
    }

    #[test]
    fn storage_trait_compiles() {
        // Compile-time verification that the trait is properly defined
        // This test ensures the trait definition is syntactically correct
    }

    #[test]
    fn trait_is_object_safe() {
        // Verify the trait can be used as a trait object
        fn _check_object_safe(_: &dyn StorageBackend) {}
    }

    #[test]
    fn validate_object_key_rejects_traversal_and_absolute_paths() {
        assert!(validate_object_key("../x").is_err());
        assert!(validate_object_key("chunks/../../etc").is_err());
        assert!(validate_object_key("/abs/path").is_err());
        assert!(validate_object_key("..\\win").is_err());
        // Already-decoded form of `chunks/..%2f` after axum's percent-decoding.
        assert!(validate_object_key("chunks/../").is_err());
        assert!(validate_object_key("C:\\x").is_err());
    }

    #[test]
    fn validate_object_key_accepts_legit_keys() {
        let hex = "deadbeef00112233445566778899aabbccddeeff00112233445566778899aa";
        assert!(validate_object_key(&format!("chunks/{hex}")).is_ok());
        assert!(validate_object_key(&format!("packs/{hex}")).is_ok());
        assert!(validate_object_key(&format!("manifests/{hex}")).is_ok());
        assert!(validate_object_key(&format!("chunk-deltas/{hex}.meta")).is_ok());
        assert!(validate_object_key(&format!("myrepo/chunks/{hex}")).is_ok());
        assert!(validate_object_key(LAYOUT_MARKER_KEY).is_ok());
        assert!(validate_object_key(&format!("myrepo/{LAYOUT_MARKER_KEY}")).is_ok());
        // Empty is allowed (list_objects("") lists everything).
        assert!(validate_object_key("").is_ok());
    }

    #[test]
    fn prefixed_key_empty_is_identity() {
        assert_eq!(prefixed_key(&None, "chunks/abc"), "chunks/abc");
        assert_eq!(
            prefixed_key(&Some(String::new()), "chunks/abc"),
            "chunks/abc"
        );
        assert_eq!(
            prefixed_key(&Some("t1".into()), "chunks/abc"),
            "t1/chunks/abc"
        );
        assert_eq!(
            prefixed_key(&Some("t1/".into()), "chunks/abc"),
            "t1/chunks/abc"
        );
    }
}
