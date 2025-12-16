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

//! Object Database (ODB) - Content-addressable storage with SHA-256 addressing
//!
//! The ODB provides:
//! - **Content-addressable storage**: Objects are identified by SHA-256 hash of their content
//! - **Automatic deduplication**: Identical content is stored only once
//! - **LRU caching**: Frequently accessed objects are cached in memory
//! - **Observable metrics**: Track cache performance and deduplication efficiency

use crate::{ObjectType, Oid, OdbMetrics};
use mediagit_compression::{Compressor, ZlibCompressor};
use mediagit_storage::StorageBackend;
use moka::future::Cache;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{debug, info, warn};

/// Object Database with content-addressable storage
///
/// The ObjectDatabase provides Git-compatible content-addressable storage
/// with automatic deduplication and LRU caching for performance.
///
/// # Architecture
///
/// - **Storage**: Pluggable backend via `StorageBackend` trait
/// - **Caching**: Moka LRU cache for hot objects
/// - **Addressing**: SHA-256 hash for content addressing
/// - **Organization**: Git-like object paths: `objects/{first2hex}/{remaining62hex}`
///
/// # Examples
///
/// ```no_run
/// use mediagit_versioning::{ObjectDatabase, ObjectType};
/// use mediagit_storage::LocalBackend;
/// use std::sync::Arc;
///
/// #[tokio::main]
/// async fn main() -> anyhow::Result<()> {
///     let storage = Arc::new(LocalBackend::new("/tmp/test-odb")?);
///     let odb = ObjectDatabase::new(storage, 1000);
///
///     // Write an object
///     let data = b"Hello, World!";
///     let oid = odb.write(ObjectType::Blob, data).await?;
///
///     // Read it back
///     let retrieved = odb.read(&oid).await?;
///     assert_eq!(retrieved, data);
///
///     // Metrics show deduplication
///     let metrics = odb.metrics().await;
///     println!("Dedup ratio: {:.1}%", metrics.dedup_ratio() * 100.0);
///
///     Ok(())
/// }
/// ```
pub struct ObjectDatabase {
    /// Underlying storage backend
    storage: Arc<dyn StorageBackend>,

    /// LRU cache for frequently accessed objects
    cache: Cache<Oid, Arc<Vec<u8>>>,

    /// Metrics tracking
    metrics: Arc<RwLock<OdbMetrics>>,

    /// Compression engine (Git-compatible zlib by default)
    compressor: Arc<dyn Compressor>,

    /// Enable/disable compression (default: true)
    compression_enabled: bool,
}

impl ObjectDatabase {
    /// Create a new ObjectDatabase with the given storage backend and cache size
    ///
    /// Uses Git-compatible zlib compression by default.
    ///
    /// # Arguments
    ///
    /// * `storage` - Storage backend implementation
    /// * `cache_capacity` - Maximum number of objects to cache in memory
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use mediagit_versioning::ObjectDatabase;
    /// use mediagit_storage::LocalBackend;
    /// use std::sync::Arc;
    ///
    /// let storage = Arc::new(LocalBackend::new("/tmp/odb").unwrap());
    /// let odb = ObjectDatabase::new(storage, 1000);
    /// ```
    pub fn new(storage: Arc<dyn StorageBackend>, cache_capacity: u64) -> Self {
        info!(
            capacity = cache_capacity,
            compression = "zlib (Git-compatible)",
            "Creating ObjectDatabase with LRU cache"
        );

        Self {
            storage,
            cache: Cache::new(cache_capacity),
            metrics: Arc::new(RwLock::new(OdbMetrics::new())),
            compressor: Arc::new(ZlibCompressor::default_level()),
            compression_enabled: true,
        }
    }

    /// Create a new ObjectDatabase with custom compression settings
    ///
    /// # Arguments
    ///
    /// * `storage` - Storage backend implementation
    /// * `cache_capacity` - Maximum number of objects to cache in memory
    /// * `compressor` - Custom compression implementation
    /// * `compression_enabled` - Enable/disable compression
    pub fn with_compression(
        storage: Arc<dyn StorageBackend>,
        cache_capacity: u64,
        compressor: Arc<dyn Compressor>,
        compression_enabled: bool,
    ) -> Self {
        info!(
            capacity = cache_capacity,
            compression_enabled = compression_enabled,
            "Creating ObjectDatabase with custom compression"
        );

        Self {
            storage,
            cache: Cache::new(cache_capacity),
            metrics: Arc::new(RwLock::new(OdbMetrics::new())),
            compressor,
            compression_enabled,
        }
    }

    /// Create a new ObjectDatabase without compression
    ///
    /// Useful for testing or when compression is handled externally.
    pub fn without_compression(storage: Arc<dyn StorageBackend>, cache_capacity: u64) -> Self {
        info!(
            capacity = cache_capacity,
            "Creating ObjectDatabase without compression"
        );

        Self {
            storage,
            cache: Cache::new(cache_capacity),
            metrics: Arc::new(RwLock::new(OdbMetrics::new())),
            compressor: Arc::new(ZlibCompressor::default_level()),
            compression_enabled: false,
        }
    }

    /// Write an object to the database
    ///
    /// Computes the SHA-256 hash of the content and stores it if not already present.
    /// Automatic deduplication: identical content returns the same OID without re-storing.
    ///
    /// # Arguments
    ///
    /// * `obj_type` - Type of the object (Blob, Tree, or Commit)
    /// * `data` - Object content
    ///
    /// # Returns
    ///
    /// The OID (SHA-256 hash) of the object
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use mediagit_versioning::{ObjectDatabase, ObjectType};
    /// # use mediagit_storage::LocalBackend;
    /// # use std::sync::Arc;
    /// # #[tokio::main]
    /// # async fn main() -> anyhow::Result<()> {
    /// # let storage = Arc::new(LocalBackend::new("/tmp/odb")?);
    /// # let odb = ObjectDatabase::new(storage, 100);
    ///
    /// let data = b"file content";
    /// let oid = odb.write(ObjectType::Blob, data).await?;
    /// println!("Stored object: {}", oid);
    /// # Ok(())
    /// # }
    /// ```
    pub async fn write(&self, obj_type: ObjectType, data: &[u8]) -> anyhow::Result<Oid> {
        // Compute OID from UNCOMPRESSED content (Git compatibility)
        let oid = Oid::hash(data);

        debug!(
            oid = %oid,
            obj_type = %obj_type,
            size = data.len(),
            compressed = self.compression_enabled,
            "Writing object"
        );

        // Build storage key (LocalBackend will handle sharding)
        let key = oid.to_hex();

        // Check if object already exists (deduplication)
        let exists = self.storage.exists(&key).await?;

        if exists {
            debug!(oid = %oid, "Object already exists (deduplicated)");
            // Update metrics for duplicate write
            let mut metrics = self.metrics.write().await;
            metrics.record_write(data.len() as u64, false);
        } else {
            // Compress data if enabled
            let storage_data = if self.compression_enabled {
                let compressed = self.compressor.compress(data)
                    .map_err(|e| anyhow::anyhow!("Compression failed: {}", e))?;

                debug!(
                    oid = %oid,
                    original_size = data.len(),
                    compressed_size = compressed.len(),
                    ratio = compressed.len() as f64 / data.len() as f64,
                    "Compressed object"
                );

                compressed
            } else {
                data.to_vec()
            };

            // Store object (compressed or raw)
            self.storage.put(&key, &storage_data).await?;

            info!(
                oid = %oid,
                original_size = data.len(),
                storage_size = storage_data.len(),
                compressed = self.compression_enabled,
                "Stored new object"
            );

            // Update metrics for new write
            let mut metrics = self.metrics.write().await;
            metrics.record_write(data.len() as u64, true);
        }

        // Cache the UNCOMPRESSED object for future reads
        self.cache.insert(oid, Arc::new(data.to_vec())).await;

        Ok(oid)
    }

    /// Read an object from the database
    ///
    /// Checks the cache first, then reads from storage if not cached.
    ///
    /// # Arguments
    ///
    /// * `oid` - Object identifier to read
    ///
    /// # Returns
    ///
    /// The object content as bytes
    ///
    /// # Errors
    ///
    /// Returns an error if the object doesn't exist
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use mediagit_versioning::{ObjectDatabase, ObjectType, Oid};
    /// # use mediagit_storage::LocalBackend;
    /// # use std::sync::Arc;
    /// # #[tokio::main]
    /// # async fn main() -> anyhow::Result<()> {
    /// # let storage = Arc::new(LocalBackend::new("/tmp/odb")?);
    /// # let odb = ObjectDatabase::new(storage, 100);
    /// # let oid = odb.write(ObjectType::Blob, b"data").await?;
    ///
    /// let data = odb.read(&oid).await?;
    /// println!("Read {} bytes", data.len());
    /// # Ok(())
    /// # }
    /// ```
    pub async fn read(&self, oid: &Oid) -> anyhow::Result<Vec<u8>> {
        debug!(oid = %oid, "Reading object");

        // Check cache first
        if let Some(cached) = self.cache.get(oid).await {
            debug!(oid = %oid, "Cache hit");
            let mut metrics = self.metrics.write().await;
            metrics.record_cache_hit();
            return Ok((*cached).clone());
        }

        // Cache miss - read from storage
        debug!(oid = %oid, "Cache miss, reading from storage");
        let mut metrics = self.metrics.write().await;
        metrics.record_cache_miss();
        drop(metrics); // Release lock before I/O

        let key = oid.to_hex();
        let storage_data = self.storage.get(&key).await?;

        // Decompress data if needed (auto-detect compression)
        let data = if self.compression_enabled || storage_data.len() >= 2 && storage_data[0] == 0x78 {
            // Try to decompress - the compressor will handle uncompressed data gracefully
            match self.compressor.decompress(&storage_data) {
                Ok(decompressed) => {
                    debug!(
                        oid = %oid,
                        storage_size = storage_data.len(),
                        decompressed_size = decompressed.len(),
                        "Decompressed object"
                    );
                    decompressed
                }
                Err(e) => {
                    // If decompression fails and data looks uncompressed, use as-is
                    // This provides backward compatibility
                    if !self.compression_enabled {
                        warn!(
                            oid = %oid,
                            error = %e,
                            "Decompression failed, using raw data"
                        );
                        storage_data
                    } else {
                        return Err(anyhow::anyhow!("Decompression failed: {}", e));
                    }
                }
            }
        } else {
            storage_data
        };

        // Verify integrity on UNCOMPRESSED data
        let computed_oid = Oid::hash(&data);
        if computed_oid != *oid {
            warn!(
                expected = %oid,
                computed = %computed_oid,
                "Object integrity check failed"
            );
            anyhow::bail!(
                "Object integrity check failed: expected {}, got {}",
                oid,
                computed_oid
            );
        }

        // Cache UNCOMPRESSED data for future reads
        let arc_data = Arc::new(data.clone());
        self.cache.insert(*oid, arc_data).await;

        Ok(data)
    }

    /// Check if an object exists in the database
    ///
    /// Checks cache first for efficiency, then queries storage.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use mediagit_versioning::{ObjectDatabase, ObjectType};
    /// # use mediagit_storage::LocalBackend;
    /// # use std::sync::Arc;
    /// # #[tokio::main]
    /// # async fn main() -> anyhow::Result<()> {
    /// # let storage = Arc::new(LocalBackend::new("/tmp/odb")?);
    /// # let odb = ObjectDatabase::new(storage, 100);
    /// # let oid = odb.write(ObjectType::Blob, b"data").await?;
    ///
    /// if odb.exists(&oid).await? {
    ///     println!("Object exists");
    /// }
    /// # Ok(())
    /// # }
    /// ```
    pub async fn exists(&self, oid: &Oid) -> anyhow::Result<bool> {
        // Check cache first
        if self.cache.get(oid).await.is_some() {
            return Ok(true);
        }

        // Check storage
        let key = format!("objects/{}", oid.to_path());
        self.storage.exists(&key).await
    }

    /// Verify object integrity
    ///
    /// Reads the object and recomputes its hash to ensure it matches the OID.
    ///
    /// # Arguments
    ///
    /// * `oid` - Object identifier to verify
    ///
    /// # Returns
    ///
    /// `true` if the object exists and its hash matches, `false` otherwise
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use mediagit_versioning::{ObjectDatabase, ObjectType};
    /// # use mediagit_storage::LocalBackend;
    /// # use std::sync::Arc;
    /// # #[tokio::main]
    /// # async fn main() -> anyhow::Result<()> {
    /// # let storage = Arc::new(LocalBackend::new("/tmp/odb")?);
    /// # let odb = ObjectDatabase::new(storage, 100);
    /// # let oid = odb.write(ObjectType::Blob, b"data").await?;
    ///
    /// if odb.verify(&oid).await? {
    ///     println!("Object integrity verified");
    /// }
    /// # Ok(())
    /// # }
    /// ```
    pub async fn verify(&self, oid: &Oid) -> anyhow::Result<bool> {
        match self.read(oid).await {
            Ok(data) => {
                let computed = Oid::hash(&data);
                Ok(computed == *oid)
            }
            Err(_) => Ok(false),
        }
    }

    /// Get current metrics
    ///
    /// Returns a snapshot of current performance and deduplication metrics.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use mediagit_versioning::ObjectDatabase;
    /// # use mediagit_storage::LocalBackend;
    /// # use std::sync::Arc;
    /// # #[tokio::main]
    /// # async fn main() -> anyhow::Result<()> {
    /// # let storage = Arc::new(LocalBackend::new("/tmp/odb")?);
    /// # let odb = ObjectDatabase::new(storage, 100);
    ///
    /// let metrics = odb.metrics().await;
    /// println!("Cache hit rate: {:.1}%", metrics.hit_rate() * 100.0);
    /// println!("Dedup ratio: {:.1}%", metrics.dedup_ratio() * 100.0);
    /// # Ok(())
    /// # }
    /// ```
    pub async fn metrics(&self) -> OdbMetrics {
        self.metrics.read().await.clone()
    }

    /// Invalidate cache entry
    ///
    /// Removes an object from the cache. Useful for testing or
    /// when you want to force a fresh read from storage.
    pub async fn invalidate_cache(&self, oid: &Oid) {
        self.cache.invalidate(oid).await;
    }

    /// Clear all cached objects
    ///
    /// Removes all entries from the cache.
    pub async fn clear_cache(&self) {
        self.cache.invalidate_all();
        // Run pending maintenance tasks
        self.cache.run_pending_tasks().await;
    }

    /// Get cache entry count
    ///
    /// Returns the number of objects currently in the cache.
    pub async fn cache_entry_count(&self) -> u64 {
        self.cache.entry_count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mediagit_storage::mock::MockBackend;

    #[tokio::test]
    async fn test_write_and_read() {
        let storage = Arc::new(MockBackend::new());
        let odb = ObjectDatabase::new(storage, 100);

        let data = b"test content";
        let oid = odb.write(ObjectType::Blob, data).await.unwrap();

        let retrieved = odb.read(&oid).await.unwrap();
        assert_eq!(retrieved, data);
    }

    #[tokio::test]
    async fn test_deduplication() {
        let storage = Arc::new(MockBackend::new());
        let odb = ObjectDatabase::new(storage, 100);

        let data = b"duplicate content";

        // Write same content twice
        let oid1 = odb.write(ObjectType::Blob, data).await.unwrap();
        let oid2 = odb.write(ObjectType::Blob, data).await.unwrap();

        // Should return same OID
        assert_eq!(oid1, oid2);

        // Metrics should show deduplication
        let metrics = odb.metrics().await;
        assert_eq!(metrics.unique_objects, 1);
        assert_eq!(metrics.total_writes, 2);
        assert_eq!(metrics.bytes_written, data.len() as u64 * 2);
        assert_eq!(metrics.bytes_stored, data.len() as u64);
        assert_eq!(metrics.dedup_ratio(), 0.5); // 50% deduplicated
    }

    #[tokio::test]
    async fn test_cache_hit() {
        let storage = Arc::new(MockBackend::new());
        let odb = ObjectDatabase::new(storage, 100);

        let data = b"cached data";
        let oid = odb.write(ObjectType::Blob, data).await.unwrap();

        // First read - cache miss
        let _ = odb.read(&oid).await.unwrap();

        // Clear internal state and read again - should be cache hit
        let _ = odb.read(&oid).await.unwrap();

        let metrics = odb.metrics().await;
        assert!(metrics.cache_hits > 0);
    }

    #[tokio::test]
    async fn test_exists() {
        let storage = Arc::new(MockBackend::new());
        let odb = ObjectDatabase::new(storage, 100);

        let data = b"exists test";
        let oid = odb.write(ObjectType::Blob, data).await.unwrap();

        assert!(odb.exists(&oid).await.unwrap());

        let non_existent = Oid::hash(b"does not exist");
        assert!(!odb.exists(&non_existent).await.unwrap());
    }

    #[tokio::test]
    async fn test_verify() {
        let storage = Arc::new(MockBackend::new());
        let odb = ObjectDatabase::new(storage, 100);

        let data = b"verify test";
        let oid = odb.write(ObjectType::Blob, data).await.unwrap();

        assert!(odb.verify(&oid).await.unwrap());

        let non_existent = Oid::hash(b"does not exist");
        assert!(!odb.verify(&non_existent).await.unwrap());
    }

    #[tokio::test]
    async fn test_cache_operations() {
        let storage = Arc::new(MockBackend::new());
        let odb = ObjectDatabase::new(storage, 100);

        let data = b"cache test";
        let oid = odb.write(ObjectType::Blob, data).await.unwrap();

        // Run pending tasks to ensure cache is updated
        odb.cache.run_pending_tasks().await;

        // Should be in cache after write
        assert_eq!(odb.cache_entry_count().await, 1);

        // Invalidate specific entry
        odb.invalidate_cache(&oid).await;

        // Clear all
        odb.clear_cache().await;
    }

    #[tokio::test]
    async fn test_compression_enabled() {
        let storage = Arc::new(MockBackend::new());
        let odb = ObjectDatabase::new(storage.clone(), 100);

        // Write large compressible data
        let data = b"This is test data that compresses well. ".repeat(100);
        let oid = odb.write(ObjectType::Blob, &data).await.unwrap();

        // Read it back
        let retrieved = odb.read(&oid).await.unwrap();
        assert_eq!(retrieved, data);

        // Verify data is compressed in storage
        let key = oid.to_hex();
        let stored_data = storage.get(&key).await.unwrap();

        // Stored data should be smaller than original (compressed)
        assert!(stored_data.len() < data.len());

        // Stored data should have zlib header (0x78)
        assert_eq!(stored_data[0], 0x78);
    }

    #[tokio::test]
    async fn test_compression_disabled() {
        let storage = Arc::new(MockBackend::new());
        let odb = ObjectDatabase::without_compression(storage.clone(), 100);

        let data = b"test data without compression";
        let oid = odb.write(ObjectType::Blob, data).await.unwrap();

        // Read it back
        let retrieved = odb.read(&oid).await.unwrap();
        assert_eq!(retrieved, data);

        // Verify data is NOT compressed in storage
        let key = oid.to_hex();
        let stored_data = storage.get(&key).await.unwrap();

        // Stored data should be same as original (uncompressed)
        assert_eq!(stored_data, data);
    }

    #[tokio::test]
    async fn test_backward_compatibility() {
        let storage = Arc::new(MockBackend::new());

        // First, write uncompressed data (simulating old version)
        let odb_old = ObjectDatabase::without_compression(storage.clone(), 100);
        let data = b"old uncompressed data";
        let oid = odb_old.write(ObjectType::Blob, data).await.unwrap();

        // Now read with compression-enabled ODB (simulating new version)
        let odb_new = ObjectDatabase::new(storage, 100);
        let retrieved = odb_new.read(&oid).await.unwrap();

        // Should read successfully
        assert_eq!(retrieved, data);
    }

    #[tokio::test]
    async fn test_compression_ratio() {
        let storage = Arc::new(MockBackend::new());
        let odb = ObjectDatabase::new(storage.clone(), 100);

        // Highly compressible data (repeated pattern)
        let data = vec![0x42u8; 10000];
        let oid = odb.write(ObjectType::Blob, &data).await.unwrap();

        // Get stored data
        let key = oid.to_hex();
        let stored_data = storage.get(&key).await.unwrap();

        // Compression ratio should be significant (>90% reduction for repeated data)
        let ratio = stored_data.len() as f64 / data.len() as f64;
        assert!(ratio < 0.1, "Expected high compression ratio, got {}", ratio);

        // Verify integrity
        let retrieved = odb.read(&oid).await.unwrap();
        assert_eq!(retrieved, data);
    }

    #[tokio::test]
    async fn test_custom_compressor() {
        use mediagit_compression::{ZstdCompressor, CompressionLevel};

        let storage = Arc::new(MockBackend::new());
        let compressor = Arc::new(ZstdCompressor::new(CompressionLevel::Best));
        let odb = ObjectDatabase::with_compression(storage, 100, compressor, true);

        let data = b"test data with zstd compression";
        let oid = odb.write(ObjectType::Blob, data).await.unwrap();

        let retrieved = odb.read(&oid).await.unwrap();
        assert_eq!(retrieved, data);
    }

    #[tokio::test]
    async fn test_empty_data_compression() {
        let storage = Arc::new(MockBackend::new());
        let odb = ObjectDatabase::new(storage, 100);

        let data = b"";
        let oid = odb.write(ObjectType::Blob, data).await.unwrap();

        let retrieved = odb.read(&oid).await.unwrap();
        assert_eq!(retrieved, data);
    }

    #[tokio::test]
    async fn test_large_file_compression() {
        let storage = Arc::new(MockBackend::new());
        let odb = ObjectDatabase::new(storage, 100);

        // Simulate a 1MB file with some compressibility
        let data = (0..10000)
            .flat_map(|i| format!("Line {} content\n", i).into_bytes())
            .collect::<Vec<u8>>();

        let oid = odb.write(ObjectType::Blob, &data).await.unwrap();
        let retrieved = odb.read(&oid).await.unwrap();

        assert_eq!(retrieved, data);
    }
}
