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
pub mod s3;

use async_trait::async_trait;
use std::fmt::Debug;
use tokio::io::{AsyncRead, AsyncReadExt};

#[cfg(feature = "azure")]
pub use azure::AzureBackend;
pub use b2_spaces::B2SpacesBackend;
pub use error::{StorageError, StorageResult};
#[cfg(feature = "gcs")]
pub use gcs::GcsBackend;
pub use local::LocalBackend;
pub use minio::MinIOBackend;
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
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
