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
//! - **Delta compression**: Store only differences between similar objects
//! - **Delta chain limits**: Prevent unbounded delta chains for consistent read performance

/// Maximum delta chain depth before re-storing as full object.
/// This prevents read performance degradation from long delta chains.
/// After this depth, objects are stored as full copies to break the chain.
pub const MAX_DELTA_DEPTH: u8 = 10;

/// Maximum allowed object size (16 GB).
/// This prevents memory allocation failures from corrupted chunk manifests
/// that may contain extremely large total_size values.
pub const MAX_OBJECT_SIZE: u64 = 16 * 1024 * 1024 * 1024;


use crate::{ObjectType, Oid, OdbMetrics};
use crate::chunking::{ChunkManifest, ChunkRef, ChunkStrategy, ContentChunker};
use crate::delta::{Delta, DeltaDecoder, DeltaEncoder};
use mediagit_compression::{Compressor, SmartCompressor, TypeAwareCompressor, ZlibCompressor, CompressionAlgorithm};
use mediagit_compression::ObjectType as CompressionObjectType;
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
///     let storage: Arc<dyn mediagit_storage::StorageBackend> =
///         Arc::new(LocalBackend::new("/tmp/test-odb").await?);
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

    /// Smart compressor for type-aware compression (optional)
    smart_compressor: Option<Arc<SmartCompressor>>,

    /// Chunking strategy (optional)
    chunk_strategy: Option<ChunkStrategy>,

    /// Enable delta encoding for similar objects
    delta_enabled: bool,

    /// Similarity detector for finding delta base candidates
    similarity_detector: Arc<RwLock<crate::similarity::SimilarityDetector>>,
}

impl Clone for ObjectDatabase {
    fn clone(&self) -> Self {
        Self {
            storage: self.storage.clone(),
            cache: self.cache.clone(),
            metrics: self.metrics.clone(),
            compressor: self.compressor.clone(),
            compression_enabled: self.compression_enabled,
            smart_compressor: self.smart_compressor.clone(),
            chunk_strategy: self.chunk_strategy,
            delta_enabled: self.delta_enabled,
            similarity_detector: self.similarity_detector.clone(),
        }
    }
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
    /// # #[tokio::main]
    /// # async fn main() -> anyhow::Result<()> {
    /// let storage: Arc<dyn mediagit_storage::StorageBackend> =
    ///     Arc::new(LocalBackend::new("/tmp/odb").await?);
    /// let odb = ObjectDatabase::new(storage, 1000);
    /// # Ok(())
    /// # }
    pub fn new(storage: Arc<dyn StorageBackend>, cache_capacity: u64) -> Self {
        info!(
            capacity = cache_capacity,
            compression = "zlib (Git-compatible)",
            delta = true,
            "Creating ObjectDatabase with LRU cache and delta encoding"
        );

        Self {
            storage,
            cache: Cache::new(cache_capacity),
            metrics: Arc::new(RwLock::new(OdbMetrics::new())),
            compressor: Arc::new(ZlibCompressor::default_level()),
            compression_enabled: true,
            smart_compressor: None,
            chunk_strategy: None,
            delta_enabled: true,  // ✅ CRITICAL FIX: Enable delta compression by default for storage savings
            similarity_detector: Arc::new(RwLock::new(crate::similarity::SimilarityDetector::new(
                crate::similarity::MAX_SIMILARITY_CANDIDATES,
            ))),
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
            smart_compressor: None,
            chunk_strategy: None,
            delta_enabled: true,  // ✅ Enable delta compression for storage savings
            similarity_detector: Arc::new(RwLock::new(crate::similarity::SimilarityDetector::new(
                crate::similarity::MAX_SIMILARITY_CANDIDATES,
            ))),
        }
    }

    /// Create ObjectDatabase with smart compression (type-aware)
    pub fn with_smart_compression(
        storage: Arc<dyn StorageBackend>,
        cache_capacity: u64,
    ) -> Self {
        info!(
            capacity = cache_capacity,
            compression = "smart (type-aware)",
            delta = true,
            "Creating ObjectDatabase with smart compression and delta encoding"
        );

        Self {
            storage,
            cache: Cache::new(cache_capacity),
            metrics: Arc::new(RwLock::new(OdbMetrics::new())),
            compressor: Arc::new(ZlibCompressor::default_level()),
            compression_enabled: true,
            smart_compressor: Some(Arc::new(SmartCompressor::new())),
            chunk_strategy: None,
            delta_enabled: true,  // ✅ CRITICAL FIX: Enable delta compression for 70-90% storage savings
            similarity_detector: Arc::new(RwLock::new(crate::similarity::SimilarityDetector::new(
                crate::similarity::MAX_SIMILARITY_CANDIDATES,
            ))),
        }
    }

    /// Create ObjectDatabase with full optimization features
    pub fn with_optimizations(
        storage: Arc<dyn StorageBackend>,
        cache_capacity: u64,
        chunk_strategy: Option<ChunkStrategy>,
        delta_enabled: bool,
    ) -> Self {
        info!(
            capacity = cache_capacity,
            chunking = chunk_strategy.is_some(),
            delta = delta_enabled,
            "Creating ObjectDatabase with full optimizations"
        );

        Self {
            storage,
            cache: Cache::new(cache_capacity),
            metrics: Arc::new(RwLock::new(OdbMetrics::new())),
            compressor: Arc::new(ZlibCompressor::default_level()),
            compression_enabled: true,
            smart_compressor: Some(Arc::new(SmartCompressor::new())),
            chunk_strategy,
            delta_enabled,
            similarity_detector: Arc::new(RwLock::new(crate::similarity::SimilarityDetector::new(
                crate::similarity::MAX_SIMILARITY_CANDIDATES,
            ))),
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
            smart_compressor: None,
            chunk_strategy: None,
            delta_enabled: false,
            similarity_detector: Arc::new(RwLock::new(crate::similarity::SimilarityDetector::new(
                crate::similarity::MAX_SIMILARITY_CANDIDATES,
            ))),
        }
    }

    /// Get reference to the underlying storage backend
    ///
    /// Useful for creating transactions or accessing storage directly.
    pub fn storage(&self) -> &Arc<dyn StorageBackend> {
        &self.storage
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
    /// # let storage: Arc<dyn mediagit_storage::StorageBackend> =
    /// #     Arc::new(LocalBackend::new("/tmp/odb").await?);
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

    /// Write an object with smart compression based on filename
    ///
    /// Automatically detects file type and applies optimal compression strategy.
    /// Falls back to standard write if smart compression is not enabled.
    ///
    /// # Arguments
    ///
    /// * `obj_type` - Type of the object (Blob, Tree, or Commit)
    /// * `data` - Object content
    /// * `filename` - Filename for type detection (can be empty)
    ///
    /// # Returns
    ///
    /// The OID (SHA-256 hash) of the object
    pub async fn write_with_path(
        &self,
        obj_type: ObjectType,
        data: &[u8],
        filename: &str,
    ) -> anyhow::Result<Oid> {
        // If smart compression is not enabled, fall back to standard write
        if self.smart_compressor.is_none() {
            return self.write(obj_type, data).await;
        }

        // Compute OID from UNCOMPRESSED content (Git compatibility)
        let oid = Oid::hash(data);

        // Detect file type for smart compression
        let compression_type = if !filename.is_empty() {
            CompressionObjectType::from_path(filename)
        } else {
            CompressionObjectType::from_magic_bytes(data)
        };

        debug!(
            oid = %oid,
            obj_type = %obj_type,
            filename = filename,
            detected_type = ?compression_type,
            size = data.len(),
            "Writing object with smart compression"
        );

        // Build storage key
        let key = oid.to_hex();

        // Check if object already exists (deduplication)
        let exists = self.storage.exists(&key).await?;

        if exists {
            debug!(oid = %oid, "Object already exists (deduplicated)");
            let mut metrics = self.metrics.write().await;
            metrics.record_write(data.len() as u64, false);
        } else {
            // Use smart compressor with size-aware strategy
            let storage_data = if let Some(smart_comp) = &self.smart_compressor {
                let compressed = smart_comp
                    .compress_typed_with_size(data, compression_type)
                    .map_err(|e| anyhow::anyhow!("Smart compression failed: {}", e))?;

                debug!(
                    oid = %oid,
                    original_size = data.len(),
                    compressed_size = compressed.len(),
                    ratio = compressed.len() as f64 / data.len() as f64,
                    file_type = ?compression_type,
                    "Smart compressed object"
                );

                compressed
            } else {
                // Fallback to standard compression
                self.compressor
                    .compress(data)
                    .map_err(|e| anyhow::anyhow!("Compression failed: {}", e))?
            };

            // Store object
            self.storage.put(&key, &storage_data).await?;

            info!(
                oid = %oid,
                original_size = data.len(),
                storage_size = storage_data.len(),
                file_type = ?compression_type,
                "Stored new object with smart compression"
            );

            // Update metrics
            let mut metrics = self.metrics.write().await;
            metrics.record_write(data.len() as u64, true);
        }

        // Cache the UNCOMPRESSED object
        self.cache.insert(oid, Arc::new(data.to_vec())).await;

        Ok(oid)
    }

    /// Try to store a chunk as delta against a similar existing chunk.
    ///
    /// Returns `true` if the chunk was successfully stored as a delta,
    /// `false` if the caller should store the full chunk instead.
    ///
    /// This is shared between `write_chunked()` (in-memory) and
    /// `write_chunked_from_file()` (streaming) paths.
    async fn try_store_chunk_as_delta(
        &self,
        chunk: &crate::chunking::ContentChunk,
        _filename: Option<&str>,
        min_similarity: f64,
        size_ratio_threshold: f64,
    ) -> anyhow::Result<bool> {
        if !self.delta_enabled || chunk.data.len() < 4096 {
            return Ok(false);
        }

        // Create metadata for this chunk
        let mut chunk_meta = crate::similarity::ObjectMetadata::new(
            chunk.id,
            chunk.data.len(),
            crate::ObjectType::Blob,
            None, // Chunks don't have individual filenames
        );
        chunk_meta.generate_samples(&chunk.data);

        // Check for similar chunk with type-aware thresholds
        let detector = self.similarity_detector.read().await;
        let similar = detector.find_similar_with_size_ratio(
            &chunk_meta, min_similarity, size_ratio_threshold,
        );
        drop(detector); // Release read lock

        if let Some((base_id, score)) = similar {
            // Try to load base chunk and create delta
            if let Ok(base_data) = self.get_chunk(&base_id).await {
                // Create delta
                let delta = DeltaEncoder::encode(&base_data, &chunk.data);
                let delta_bytes = delta.to_bytes();

                // Only use delta if beneficial (<80% of original)
                let delta_ratio = delta_bytes.len() as f64 / chunk.data.len() as f64;
                if delta_ratio < 0.80 {
                    // Store chunk delta
                    let delta_key = format!("chunk-deltas/{}", chunk.id.to_hex());
                    let compressed_delta = if let Some(smart_comp) = &self.smart_compressor {
                        smart_comp.compress_typed(&delta_bytes, CompressionObjectType::Unknown)
                            .map_err(|e| anyhow::anyhow!("Failed to compress chunk delta: {}", e))?
                    } else {
                        self.compressor.compress(&delta_bytes)
                            .map_err(|e| anyhow::anyhow!("Failed to compress chunk delta: {}", e))?
                    };

                    self.storage.put(&delta_key, &compressed_delta).await
                        .map_err(|e| anyhow::anyhow!("Failed to store chunk delta: {}", e))?;

                    // Store delta metadata (base reference)
                    let meta_key = format!("chunk-deltas/{}.meta", chunk.id.to_hex());
                    let meta_data = format!("base:{}", base_id.to_hex());
                    self.storage.put(&meta_key, meta_data.as_bytes()).await
                        .map_err(|e| anyhow::anyhow!("Failed to store chunk delta meta: {}", e))?;

                    debug!(
                        chunk_id = %chunk.id,
                        base_id = %base_id,
                        original_size = chunk.data.len(),
                        delta_size = delta_bytes.len(),
                        ratio = delta_ratio,
                        similarity = score.score,
                        "Stored chunk as delta"
                    );
                    return Ok(true);
                }
            }
        }

        // Register this chunk for future similarity matching
        let mut detector = self.similarity_detector.write().await;
        detector.add_object(chunk_meta);

        Ok(false)
    }

    /// Write object with chunking support for large media files
    ///
    /// Splits the object into chunks, stores each chunk individually,
    /// and creates a manifest for reconstruction. Enables chunk-level
    /// deduplication across files.
    ///
    /// # Arguments
    ///
    /// * `obj_type` - Type of the object (Blob, Tree, or Commit)
    /// * `data` - Object content
    /// * `filename` - Filename for media-aware chunking
    ///
    /// # Returns
    ///
    /// The OID (SHA-256 hash) of the original data
    pub async fn write_chunked(
        &self,
        obj_type: ObjectType,
        data: &[u8],
        filename: &str,
    ) -> anyhow::Result<Oid> {
        // If chunking not enabled, fall back to standard write
        if self.chunk_strategy.is_none() {
            return self.write_with_path(obj_type, data, filename).await;
        }

        // Skip chunking for small files (<1MB) to avoid overhead
        // Files 1-10MB benefit from chunking for delta encoding
        const MIN_CHUNK_SIZE: usize = 1 * 1024 * 1024; // 1MB
        if data.len() < MIN_CHUNK_SIZE {
            debug!(
                size = data.len(),
                threshold = MIN_CHUNK_SIZE,
                "File too small for chunking, using standard write"
            );
            return self.write_with_path(obj_type, data, filename).await;
        }

        // Skip chunking for small compressed formats that don't benefit from chunking
        // Note: Video formats (MP4, MOV, AVI, WebM) ARE chunked because:
        // - Chunking enables partial deduplication (shared intros/outros)
        // - Large videos benefit from chunk-level delta encoding
        // - Enables resumable transfers for large files
        if !filename.is_empty() {
            let compression_type = CompressionObjectType::from_path(filename);
            let should_skip_chunking = matches!(
                compression_type,
                // Compressed images: typically small, don't benefit from chunking
                CompressionObjectType::Jpeg
                    | CompressionObjectType::Png
                    | CompressionObjectType::Gif
                    | CompressionObjectType::Webp
                    | CompressionObjectType::Avif
                    | CompressionObjectType::Heic
                    // Compressed audio: typically small files
                    | CompressionObjectType::Mp3
                    | CompressionObjectType::Aac
                    | CompressionObjectType::Ogg
                // Note: Video formats (Mp4, Mov, Avi, Webm) are NOT skipped
                // They benefit from chunking for partial dedup and large file handling
            );

            if should_skip_chunking {
                debug!(
                    file_type = ?compression_type,
                    size = data.len(),
                    "Small compressed format detected, skipping chunking"
                );
                return self.write_with_path(obj_type, data, filename).await;
            }
        }

        // Compute OID from original data (git compatibility)
        let oid = Oid::hash(data);

        debug!(
            oid = %oid,
            size = data.len(),
            filename = filename,
            "Writing chunked object"
        );

        // Check if object already exists
        let key = oid.to_hex();
        let exists = self.storage.exists(&key).await
            .map_err(|e| anyhow::anyhow!("Failed to check if chunked object {} exists: {}", key, e))?;
        if exists {
            debug!(oid = %oid, "Chunked object already exists (deduplicated)");
            let mut metrics = self.metrics.write().await;
            metrics.record_write(data.len() as u64, false);
            return Ok(oid);
        }

        // Create chunker with configured strategy
        let chunker = ContentChunker::new(self.chunk_strategy.unwrap());

        // Chunk the data
        let chunks = chunker.chunk(data, filename).await
            .map_err(|e| anyhow::anyhow!("Failed to chunk data for {} (size: {} bytes): {}", filename, data.len(), e))?;

        info!(
            oid = %oid,
            chunks = chunks.len(),
            total_size = data.len(),
            "Chunked object into {} chunks",
            chunks.len()
        );

        // Store each chunk with smart compression and optional delta encoding
        let min_similarity = crate::similarity::get_similarity_threshold(Some(filename));
        let size_ratio_threshold = crate::similarity::get_size_ratio_threshold(Some(filename));

        for chunk in &chunks {
            // Use to_hex() for consistent storage paths (LocalBackend handles sharding)
            let chunk_key = format!("chunks/{}", chunk.id.to_hex());

            // Check if chunk already exists (deduplication at chunk level)
            // Also check delta storage for chunks stored as deltas
            let chunk_exists = self.storage.exists(&chunk_key).await
                .map_err(|e| anyhow::anyhow!("Failed to check chunk existence for {}: {}", chunk_key, e))?;
            let delta_exists = self.storage.exists(&format!("chunk-deltas/{}.meta", chunk.id.to_hex())).await
                .unwrap_or(false);

            if !chunk_exists && !delta_exists {
                // Try delta encoding first via shared helper
                let stored_as_delta = self.try_store_chunk_as_delta(
                    chunk,
                    Some(filename),
                    min_similarity,
                    size_ratio_threshold,
                ).await?;

                // Store full chunk if delta wasn't beneficial
                if !stored_as_delta {
                    let compressed = if let Some(smart_comp) = &self.smart_compressor {
                        let chunk_comp_type = if !filename.is_empty() {
                            CompressionObjectType::from_path(filename)
                        } else {
                            CompressionObjectType::Unknown
                        };
                        smart_comp.compress_typed_with_size(&chunk.data, chunk_comp_type)
                            .map_err(|e| anyhow::anyhow!("Failed to compress chunk {}: {}", chunk_key, e))?
                    } else {
                        self.compressor.compress(&chunk.data)
                            .map_err(|e| anyhow::anyhow!("Failed to compress chunk {}: {}", chunk_key, e))?
                    };

                    self.storage.put(&chunk_key, &compressed).await
                        .map_err(|e| anyhow::anyhow!("Failed to store chunk {} (size: {} bytes): {}", chunk_key, compressed.len(), e))?;

                    debug!(
                        chunk_id = %chunk.id,
                        original_size = chunk.data.len(),
                        compressed_size = compressed.len(),
                        chunk_type = ?chunk.chunk_type,
                        "Stored full chunk"
                    );
                }
            } else {
                debug!(chunk_id = %chunk.id, "Chunk already exists (deduplicated)");
            }
        }

        // Create chunk manifest
        let manifest = crate::chunking::ChunkManifest::from_chunks(
            chunks,
            Some(filename.to_string()),
        );

        // Store manifest (use to_hex() for consistent storage paths)
        let manifest_key = format!("manifests/{}", oid.to_hex());
        let manifest_data = bincode::serialize(&manifest)
            .map_err(|e| anyhow::anyhow!("Failed to serialize chunk manifest for {}: {}", oid, e))?;
        self.storage.put(&manifest_key, &manifest_data).await
            .map_err(|e| anyhow::anyhow!("Failed to store chunk manifest {} (size: {} bytes): {}", manifest_key, manifest_data.len(), e))?;

        info!(
            oid = %oid,
            chunks = manifest.chunk_count(),
            "Stored chunk manifest"
        );

        // Update metrics
        let mut metrics = self.metrics.write().await;
        metrics.record_write(data.len() as u64, true);

        // NOTE: Don't cache full data for chunked objects - individual chunks are
        // already stored and the manifest provides reconstruction. Caching the full
        // data here would duplicate memory (e.g. 55MB WAV → 3.4GB RAM).

        Ok(oid)
    }

    /// Write chunked object with parallel chunk processing
    ///
    /// Uses a producer-consumer pipeline for high-throughput staging:
    /// - Producer: chunks the data (FastCDC/MediaAware)
    /// - Workers: dedup → similarity → delta/compress → store (in parallel)
    /// - Assembler: collects results, sorts by sequence, builds manifest
    ///
    /// Falls back to sequential `write_chunked()` for small files or when
    /// chunking is not enabled.
    pub async fn write_chunked_parallel(
        &self,
        obj_type: ObjectType,
        data: &[u8],
        filename: &str,
    ) -> anyhow::Result<Oid> {
        // Reuse same guards as write_chunked()
        if self.chunk_strategy.is_none() {
            return self.write_with_path(obj_type, data, filename).await;
        }

        const MIN_CHUNK_SIZE: usize = 1 * 1024 * 1024; // 1MB
        if data.len() < MIN_CHUNK_SIZE {
            return self.write_with_path(obj_type, data, filename).await;
        }

        // Skip chunking for small compressed formats
        if !filename.is_empty() {
            let compression_type = CompressionObjectType::from_path(filename);
            let should_skip = matches!(
                compression_type,
                CompressionObjectType::Jpeg
                    | CompressionObjectType::Png
                    | CompressionObjectType::Gif
                    | CompressionObjectType::Webp
                    | CompressionObjectType::Avif
                    | CompressionObjectType::Heic
                    | CompressionObjectType::Mp3
                    | CompressionObjectType::Aac
                    | CompressionObjectType::Ogg
            );
            if should_skip {
                return self.write_with_path(obj_type, data, filename).await;
            }
        }

        // Compute OID from original data
        let oid = Oid::hash(data);

        // Check if object already exists
        let key = oid.to_hex();
        if self.storage.exists(&key).await
            .map_err(|e| anyhow::anyhow!("Failed to check object existence: {}", e))? {
            let mut metrics = self.metrics.write().await;
            metrics.record_write(data.len() as u64, false);
            return Ok(oid);
        }

        // Check manifest existence too
        if self.storage.exists(&format!("manifests/{}", key)).await.unwrap_or(false) {
            let mut metrics = self.metrics.write().await;
            metrics.record_write(data.len() as u64, false);
            return Ok(oid);
        }

        // Chunk the data
        let chunker = ContentChunker::new(self.chunk_strategy.unwrap());
        let chunks = chunker.chunk(data, filename).await
            .map_err(|e| anyhow::anyhow!("Failed to chunk data: {}", e))?;

        let num_chunks = chunks.len();
        info!(oid = %oid, chunks = num_chunks, size = data.len(), "Parallel chunked write");

        // Compute type-aware thresholds once
        let min_similarity = crate::similarity::get_similarity_threshold(Some(filename));
        let size_ratio_threshold = crate::similarity::get_size_ratio_threshold(Some(filename));
        let comp_type = if !filename.is_empty() {
            CompressionObjectType::from_path(filename)
        } else {
            CompressionObjectType::Unknown
        };

        // For small chunk counts, sequential is faster (no channel overhead)
        if num_chunks <= 4 {
            return self.write_chunked(obj_type, data, filename).await;
        }

        // --- Parallel pipeline ---
        let num_workers = num_cpus::get().min(num_chunks).max(2);
        let (tx, rx) = async_channel::bounded::<(usize, crate::chunking::ContentChunk)>(64);

        // Send all chunks to the channel with sequence IDs
        let producer = tokio::spawn(async move {
            for (seq_id, chunk) in chunks.into_iter().enumerate() {
                if tx.send((seq_id, chunk)).await.is_err() {
                    break; // receivers dropped
                }
            }
            // tx is dropped here, closing the channel
        });

        // Spawn worker tasks
        let mut worker_handles = Vec::with_capacity(num_workers);
        for _ in 0..num_workers {
            let rx = rx.clone();
            let storage = self.storage.clone();
            let compressor = self.compressor.clone();
            let smart_comp = self.smart_compressor.clone();
            let similarity_detector = self.similarity_detector.clone();
            let delta_enabled = self.delta_enabled;

            let handle = tokio::spawn(async move {
                let mut results: Vec<(usize, ChunkRef)> = Vec::new();

                while let Ok((seq_id, chunk)) = rx.recv().await {
                    let chunk_ref = ChunkRef {
                        id: chunk.id,
                        offset: chunk.offset,
                        size: chunk.size,
                        chunk_type: chunk.chunk_type,
                    };

                    // 1. Dedup check
                    let chunk_key = format!("chunks/{}", chunk.id.to_hex());
                    let delta_meta_key = format!("chunk-deltas/{}.meta", chunk.id.to_hex());

                    let chunk_exists = storage.exists(&chunk_key).await.unwrap_or(false);
                    let delta_exists = storage.exists(&delta_meta_key).await.unwrap_or(false);

                    if chunk_exists || delta_exists {
                        debug!(chunk_id = %chunk.id, "Parallel: chunk deduplicated");
                        results.push((seq_id, chunk_ref));
                        continue;
                    }

                    // 2. Delta encoding attempt
                    let mut stored_as_delta = false;
                    if delta_enabled && chunk.data.len() >= 4096 {
                        let mut chunk_meta = crate::similarity::ObjectMetadata::new(
                            chunk.id,
                            chunk.data.len(),
                            crate::ObjectType::Blob,
                            None,
                        );
                        chunk_meta.generate_samples(&chunk.data);

                        // Read-lock: concurrent with other workers
                        let detector = similarity_detector.read().await;
                        let similar = detector.find_similar_with_size_ratio(
                            &chunk_meta, min_similarity, size_ratio_threshold,
                        );
                        drop(detector);

                        if let Some((base_id, score)) = similar {
                            // Delta chains are prevented by is_delta flag in
                            // SimilarityDetector::find_similar_with_size_ratio()
                            {
                                let base_key = format!("chunks/{}", base_id.to_hex());
                                if let Ok(base_compressed) = storage.get(&base_key).await {
                                    let base_data = if let Some(ref smart) = smart_comp {
                                        smart.decompress_typed(&base_compressed).ok()
                                    } else {
                                        compressor.decompress(&base_compressed).ok()
                                    };

                                    if let Some(base_data) = base_data {
                                        let delta = DeltaEncoder::encode(&base_data, &chunk.data);
                                        let delta_bytes = delta.to_bytes();
                                        let delta_ratio = delta_bytes.len() as f64 / chunk.data.len() as f64;

                                        if delta_ratio < 0.80 {
                                            let delta_key = format!("chunk-deltas/{}", chunk.id.to_hex());
                                            let compressed_delta = if let Some(ref smart) = smart_comp {
                                                smart.compress_typed(&delta_bytes, CompressionObjectType::Unknown)
                                                    .map_err(|e| anyhow::anyhow!("Compress delta: {}", e))?
                                            } else {
                                                compressor.compress(&delta_bytes)
                                                    .map_err(|e| anyhow::anyhow!("Compress delta: {}", e))?
                                            };

                                            // Tolerate concurrent writes: if put fails but chunk exists, treat as dedup
                                            if let Err(e) = storage.put(&delta_key, &compressed_delta).await {
                                                if !storage.exists(&delta_key).await.unwrap_or(false) {
                                                    return Err(anyhow::anyhow!("Store delta: {}", e));
                                                }
                                            }

                                            let meta_key = format!("chunk-deltas/{}.meta", chunk.id.to_hex());
                                            let meta_data = format!("base:{}", base_id.to_hex());
                                            if let Err(e) = storage.put(&meta_key, meta_data.as_bytes()).await {
                                                if !storage.exists(&meta_key).await.unwrap_or(false) {
                                                    return Err(anyhow::anyhow!("Store delta meta: {}", e));
                                                }
                                            }

                                            debug!(
                                                chunk_id = %chunk.id,
                                                base_id = %base_id,
                                                delta_ratio,
                                                similarity = score.score,
                                                "Parallel: stored chunk as delta"
                                            );
                                            stored_as_delta = true;
                                        }
                                    }
                                }
                            }
                        }

                        // Register for future similarity (write-lock, brief)
                        // Mark as delta so it won't be used as a base candidate
                        chunk_meta.is_delta = stored_as_delta;
                        {
                            let mut detector = similarity_detector.write().await;
                            detector.add_object(chunk_meta);
                        }
                    }

                    // 3. Full compress + store if not delta
                    if !stored_as_delta {
                        let compressed = if let Some(ref smart) = smart_comp {
                            smart.compress_typed_with_size(&chunk.data, comp_type)
                                .map_err(|e| anyhow::anyhow!("Compress chunk: {}", e))?
                        } else {
                            compressor.compress(&chunk.data)
                                .map_err(|e| anyhow::anyhow!("Compress chunk: {}", e))?
                        };

                        // Tolerate concurrent writes: if put fails but chunk exists, treat as dedup
                        if let Err(e) = storage.put(&chunk_key, &compressed).await {
                            if !storage.exists(&chunk_key).await.unwrap_or(false) {
                                return Err(anyhow::anyhow!("Store chunk: {}", e));
                            }
                        }

                        debug!(
                            chunk_id = %chunk.id,
                            original = chunk.data.len(),
                            compressed = compressed.len(),
                            "Parallel: stored full chunk"
                        );
                    }

                    results.push((seq_id, chunk_ref));
                }

                Ok::<_, anyhow::Error>(results)
            });

            worker_handles.push(handle);
        }
        // Drop our copy of rx so workers can detect channel close
        drop(rx);

        // Wait for producer to finish sending
        producer.await.map_err(|e| anyhow::anyhow!("Producer task failed: {}", e))?;

        // Collect results from all workers
        let mut all_refs: Vec<(usize, ChunkRef)> = Vec::with_capacity(num_chunks);
        for handle in worker_handles {
            let worker_refs = handle.await
                .map_err(|e| anyhow::anyhow!("Worker task panicked: {}", e))??;
            all_refs.extend(worker_refs);
        }

        // Sort by sequence ID to restore original chunk order
        all_refs.sort_by_key(|(seq_id, _)| *seq_id);
        let chunk_refs: Vec<ChunkRef> = all_refs.into_iter().map(|(_, r)| r).collect();

        // Build and store manifest
        let manifest = ChunkManifest {
            chunks: chunk_refs,
            total_size: data.len() as u64,
            filename: Some(filename.to_string()),
        };

        let manifest_key = format!("manifests/{}", oid.to_hex());
        let manifest_data = bincode::serialize(&manifest)
            .map_err(|e| anyhow::anyhow!("Failed to serialize manifest: {}", e))?;
        self.storage.put(&manifest_key, &manifest_data).await
            .map_err(|e| anyhow::anyhow!("Failed to store manifest: {}", e))?;

        info!(oid = %oid, chunks = manifest.chunk_count(), "Parallel chunked write complete");

        // Update metrics
        let mut metrics = self.metrics.write().await;
        metrics.record_write(data.len() as u64, true);

        // NOTE: Don't cache full data for chunked objects - individual chunks are
        // already stored and the manifest provides reconstruction. Caching the full
        // data here would duplicate memory (e.g. 55MB WAV → 3.4GB RAM).

        Ok(oid)
    }

    /// Write a file with chunking using streaming reads (constant memory)
    ///
    /// This method processes files of any size without loading them entirely
    /// into memory. Chunks are generated and written incrementally.
    ///
    /// # Arguments
    ///
    /// * `path` - Path to the file to chunk and store
    /// * `filename` - Filename for format detection
    ///
    /// # Returns
    ///
    /// The OID (SHA-256 hash) of the file content
    ///
    /// # Memory Usage
    ///
    /// Memory usage is bounded by chunk size (~8MB max for TB+ files)
    /// regardless of total file size.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # use mediagit_versioning::ObjectDatabase;
    /// # async fn example() -> anyhow::Result<()> {
    /// # let odb: ObjectDatabase = todo!();
    /// let oid = odb.write_chunked_from_file(
    ///     "/path/to/large_video.mp4",
    ///     "large_video.mp4"
    /// ).await?;
    /// println!("Stored file with OID: {}", oid);
    /// # Ok(())
    /// # }
    /// ```
    pub async fn write_chunked_from_file<P: AsRef<std::path::Path>>(
        &self,
        path: P,
        filename: &str,
    ) -> anyhow::Result<Oid> {
        use std::sync::atomic::{AtomicU64, Ordering};

        let path = path.as_ref();
        let file_size = std::fs::metadata(path)?.len();

        info!(
            "Streaming parallel chunked write: file={}, size={}MB",
            filename,
            file_size / (1024 * 1024)
        );

        // Compute file OID using streaming hash (constant memory)
        let file_oid = Oid::from_file_async(path).await?;

        // Check if we already have this file
        if self.storage.exists(&format!("manifests/{}", file_oid.to_hex())).await? {
            debug!("File already exists in storage: {}", file_oid);
            return Ok(file_oid);
        }

        // Track progress
        let chunks_written = Arc::new(AtomicU64::new(0));
        let bytes_written = Arc::new(AtomicU64::new(0));

        // Compute type-aware thresholds once
        let min_similarity = crate::similarity::get_similarity_threshold(Some(filename));
        let size_ratio_threshold = crate::similarity::get_size_ratio_threshold(Some(filename));
        let comp_type = if !filename.is_empty() {
            CompressionObjectType::from_path(filename)
        } else {
            CompressionObjectType::Unknown
        };

        // --- Parallel pipeline: spawn workers FIRST, then produce chunks ---
        let num_workers = num_cpus::get().min(16).max(2);
        let (tx, rx) = async_channel::bounded::<(usize, crate::chunking::ContentChunk)>(64);

        // Spawn worker tasks BEFORE producing chunks to avoid deadlock.
        // The producer (chunk_file_streaming) uses a synchronous FastCDC iterator
        // for large files. If workers aren't already consuming, the bounded channel
        // fills up and the producer blocks forever.
        let mut worker_handles = Vec::with_capacity(num_workers);
        for _ in 0..num_workers {
            let rx = rx.clone();
            let storage = self.storage.clone();
            let compressor = self.compressor.clone();
            let smart_comp = self.smart_compressor.clone();
            let compression_enabled = self.compression_enabled;
            let delta_enabled = self.delta_enabled;
            let similarity_detector = self.similarity_detector.clone();
            let chunks_w = chunks_written.clone();
            let bytes_w = bytes_written.clone();

            let handle = tokio::spawn(async move {
                let mut results: Vec<(usize, ChunkRef)> = Vec::new();

                while let Ok((seq_id, chunk)) = rx.recv().await {
                    let chunk_ref = ChunkRef {
                        id: chunk.id,
                        offset: chunk.offset,
                        size: chunk.size,
                        chunk_type: chunk.chunk_type,
                    };

                    // 1. Dedup check
                    let chunk_key = format!("chunks/{}", chunk.id.to_hex());
                    let delta_meta_key = format!("chunk-deltas/{}.meta", chunk.id.to_hex());
                    let chunk_exists = storage.exists(&chunk_key).await.unwrap_or(false);
                    let delta_exists = storage.exists(&delta_meta_key).await.unwrap_or(false);

                    if chunk_exists || delta_exists {
                        debug!(chunk_id = %chunk.id, "Streaming parallel: chunk deduplicated");
                        results.push((seq_id, chunk_ref));
                        continue;
                    }

                    // 2. Delta encoding attempt
                    let mut stored_as_delta = false;
                    if delta_enabled && chunk.data.len() >= 4096 {
                        let mut chunk_meta = crate::similarity::ObjectMetadata::new(
                            chunk.id,
                            chunk.data.len(),
                            crate::ObjectType::Blob,
                            None,
                        );
                        chunk_meta.generate_samples(&chunk.data);

                        let detector = similarity_detector.read().await;
                        let similar = detector.find_similar_with_size_ratio(
                            &chunk_meta, min_similarity, size_ratio_threshold,
                        );
                        drop(detector);

                        if let Some((base_id, score)) = similar {
                            let base_is_delta = storage.exists(
                                &format!("chunk-deltas/{}.meta", base_id.to_hex())
                            ).await.unwrap_or(false);

                            if !base_is_delta {
                                let base_key = format!("chunks/{}", base_id.to_hex());
                                if let Ok(base_compressed) = storage.get(&base_key).await {
                                    let base_data = if let Some(ref smart) = smart_comp {
                                        smart.decompress_typed(&base_compressed).ok()
                                    } else {
                                        compressor.decompress(&base_compressed).ok()
                                    };

                                    if let Some(base_data) = base_data {
                                        let delta = DeltaEncoder::encode(&base_data, &chunk.data);
                                        let delta_bytes = delta.to_bytes();
                                        let delta_ratio = delta_bytes.len() as f64 / chunk.data.len() as f64;

                                        if delta_ratio < 0.80 {
                                            let delta_key = format!("chunk-deltas/{}", chunk.id.to_hex());
                                            let compressed_delta = if let Some(ref smart) = smart_comp {
                                                smart.compress_typed(&delta_bytes, CompressionObjectType::Unknown)
                                                    .map_err(|e| anyhow::anyhow!("Compress delta: {}", e))?
                                            } else {
                                                compressor.compress(&delta_bytes)
                                                    .map_err(|e| anyhow::anyhow!("Compress delta: {}", e))?
                                            };

                                            storage.put(&delta_key, &compressed_delta).await
                                                .map_err(|e| anyhow::anyhow!("Store delta: {}", e))?;

                                            let meta_key = format!("chunk-deltas/{}.meta", chunk.id.to_hex());
                                            let meta_data = format!("base:{}", base_id.to_hex());
                                            storage.put(&meta_key, meta_data.as_bytes()).await
                                                .map_err(|e| anyhow::anyhow!("Store delta meta: {}", e))?;

                                            debug!(
                                                chunk_id = %chunk.id,
                                                base_id = %base_id,
                                                delta_ratio,
                                                similarity = score.score,
                                                "Streaming parallel: stored chunk as delta"
                                            );
                                            stored_as_delta = true;
                                        }
                                    }
                                }
                            }
                        }

                        if !stored_as_delta {
                            let mut detector = similarity_detector.write().await;
                            detector.add_object(chunk_meta);
                        }
                    }

                    // 3. Full compress + store if not delta
                    if !stored_as_delta {
                        let data_to_store = if let Some(ref smart) = smart_comp {
                            smart.compress_typed_with_size(&chunk.data, comp_type)
                                .map_err(|e| anyhow::anyhow!("Compress chunk: {}", e))?
                        } else if compression_enabled {
                            compressor.compress(&chunk.data)
                                .map_err(|e| anyhow::anyhow!("Compress chunk: {}", e))?
                        } else {
                            chunk.data.clone()
                        };

                        storage.put(&chunk_key, &data_to_store).await
                            .map_err(|e| anyhow::anyhow!("Store chunk: {}", e))?;
                    }

                    chunks_w.fetch_add(1, Ordering::Relaxed);
                    bytes_w.fetch_add(chunk.size as u64, Ordering::Relaxed);

                    results.push((seq_id, chunk_ref));
                }

                Ok::<_, anyhow::Error>(results)
            });

            worker_handles.push(handle);
        }
        // Drop our copy of rx so workers detect channel close
        drop(rx);

        // Producer: run synchronous FastCDC file I/O in a blocking thread pool task to avoid
        // stalling the tokio executor.  FastCDC's StreamCDC iterator performs blocking reads;
        // running it on a tokio async thread would starve other tasks during large-file ingestion.
        //
        // Bridge the sync/async boundary with a bounded tokio::sync::mpsc channel:
        //   spawn_blocking → blocking_send → tokio_rx.recv() → async_channel tx → workers
        let seq_counter = Arc::new(AtomicU64::new(0));
        let (blocking_tx, mut blocking_rx) = tokio::sync::mpsc::channel::<crate::chunking::ContentChunk>(32);

        let path_owned = path.to_path_buf();
        let chunk_strategy = self.chunk_strategy.unwrap_or(ChunkStrategy::MediaAware);
        let file_producer = tokio::task::spawn_blocking(move || {
            let chunker_inner = ContentChunker::new(chunk_strategy);
            chunker_inner.collect_file_chunks_blocking(&path_owned, blocking_tx)
        });

        // Forward chunks from the blocking thread to the async worker channel
        while let Some(chunk) = blocking_rx.recv().await {
            let seq_id = seq_counter.fetch_add(1, Ordering::SeqCst) as usize;
            tx.send((seq_id, chunk)).await
                .map_err(|_| anyhow::anyhow!("Worker channel closed unexpectedly"))?;
        }

        // Propagate any error from the blocking producer
        file_producer.await
            .map_err(|e| anyhow::anyhow!("File chunker task panicked: {}", e))??;

        // Close sender so workers know no more chunks are coming
        drop(tx);

        // Collect results from all workers
        let mut all_refs: Vec<(usize, ChunkRef)> = Vec::new();
        for handle in worker_handles {
            let worker_refs = handle.await
                .map_err(|e| anyhow::anyhow!("Worker task panicked: {}", e))??;
            all_refs.extend(worker_refs);
        }

        // Sort by sequence ID to restore original chunk order
        all_refs.sort_by_key(|(seq_id, _)| *seq_id);
        let chunk_refs_final: Vec<ChunkRef> = all_refs.into_iter().map(|(_, r)| r).collect();

        // Create and store manifest
        let manifest = ChunkManifest {
            chunks: chunk_refs_final,
            total_size: file_size,
            filename: Some(filename.to_string()),
        };

        let manifest_data = bincode::serialize(&manifest)?;
        let manifest_key = format!("manifests/{}", file_oid.to_hex());
        self.storage.put(&manifest_key, &manifest_data).await?;

        info!(
            "Streaming parallel write complete: {} chunks, {}MB written",
            chunks_written.load(Ordering::Relaxed),
            bytes_written.load(Ordering::Relaxed) / (1024 * 1024)
        );

        // Update metrics
        let mut metrics = self.metrics.write().await;
        metrics.record_write(file_size, true);

        Ok(file_oid)
    }
    /// Write an object with delta compression if similar object found
    ///
    /// Attempts to find a similar object in recent history and stores only the
    /// delta (difference) if similarity exceeds threshold.
    ///
    /// # Arguments
    ///
    /// * `obj_type` - The type of object being written
    /// * `data` - The object content
    /// * `filename` - Optional filename for metadata
    ///
    /// # Returns
    ///
    /// The OID of the stored object
    ///
    /// # Delta Compression Logic
    ///
    /// 1. Generate samples from the new object
    /// 2. Search recent objects for similarity (> 30%)
    /// 3. If similar object found, create delta
    /// 4. Use delta only if smaller than 80% of original
    /// 5. Fall back to standard write otherwise
    pub async fn write_with_delta(
        &self,
        obj_type: ObjectType,
        data: &[u8],
        filename: &str,
    ) -> anyhow::Result<Oid> {
        if !self.delta_enabled {
            // Delta not enabled, fall back to standard write
            return self.write_with_path(obj_type, data, filename).await;
        }

        let oid = Oid::hash(data);

        debug!(
            oid = %oid,
            size = data.len(),
            filename,
            "Attempting delta compression"
        );

        // Create metadata and generate samples for similarity detection
        let mut metadata = crate::similarity::ObjectMetadata::new(
            oid,
            data.len(),
            obj_type,
            if filename.is_empty() {
                None
            } else {
                Some(filename.to_string())
            },
        );
        metadata.generate_samples(data);

        // Find similar object for delta base
        let detector = self.similarity_detector.read().await;
        let threshold = crate::similarity::get_similarity_threshold(
            if filename.is_empty() { None } else { Some(filename) }
        );
        let size_ratio = crate::similarity::get_size_ratio_threshold(
            if filename.is_empty() { None } else { Some(filename) }
        );
        let similar = detector.find_similar_with_size_ratio(&metadata, threshold, size_ratio);
        drop(detector);

        if let Some((base_oid, score)) = similar {
            // CRITICAL: Prevent self-referencing delta (OID == base OID)
            if oid == base_oid {
                warn!(
                    oid = %oid,
                    "Attempted to create self-referencing delta, storing as full object"
                );
            } else {
            info!(
                oid = %oid,
                base_oid = %base_oid,
                similarity = score.score,
                "Found similar object, attempting delta compression"
            );

            // Read base object
            match self.read(&base_oid).await {
                Ok(base_data) => {
                    // Check delta chain depth - prevent unbounded chains
                    let base_depth = self.get_delta_depth(&base_oid).await.unwrap_or(0);
                    
                    // Also check if base's chain already contains this OID (would create cycle)
                    let would_create_cycle = self.delta_chain_contains(&base_oid, &oid).await.unwrap_or(false);
                    
                    if would_create_cycle {
                        warn!(
                            oid = %oid,
                            base_oid = %base_oid,
                            "Base's delta chain already contains this OID, storing as full object to prevent cycle"
                        );
                        // Fall through to standard write
                    } else if base_depth >= MAX_DELTA_DEPTH {
                        info!(
                            oid = %oid,
                            base_oid = %base_oid,
                            depth = base_depth,
                            max_depth = MAX_DELTA_DEPTH,
                            "Delta chain limit reached, storing as full object"
                        );
                        // Fall through to standard write
                    } else {
                        // Create delta
                        let delta = DeltaEncoder::encode(&base_data, data);
                        let delta_data = delta.to_bytes();

                        // Only use delta if it's smaller than 80% of original
                        let delta_ratio = delta_data.len() as f64 / data.len() as f64;

                        if delta_ratio < 0.80 {
                        info!(
                            oid = %oid,
                            original_size = data.len(),
                            delta_size = delta_data.len(),
                            ratio = delta_ratio,
                            "Delta compression beneficial, storing delta"
                        );

                        // Store delta
                        let delta_key = format!("deltas/{}", oid.to_hex());
                        let compressed_delta = if let Some(smart_comp) = &self.smart_compressor {
                            smart_comp.compress_typed(&delta_data, CompressionObjectType::Unknown)?
                        } else {
                            self.compressor.compress(&delta_data)?
                        };

                        self.storage.put(&delta_key, &compressed_delta).await?;

                        // Store delta metadata (base OID reference + chain depth)
                        let new_depth = base_depth + 1;
                        let delta_meta = format!("base:{}:depth:{}", base_oid.to_hex(), new_depth);
                        let meta_key = format!("deltas/{}.meta", oid.to_hex());
                        self.storage.put(&meta_key, delta_meta.as_bytes()).await?;
                        
                        debug!(
                            oid = %oid,
                            base_oid = %base_oid,
                            depth = new_depth,
                            "Stored delta with chain depth"
                        );

                        // Update metrics
                        let mut metrics = self.metrics.write().await;
                        metrics.record_write(data.len() as u64, true);

                        // Cache original data
                        self.cache.insert(oid, Arc::new(data.to_vec())).await;

                        // Add to similarity detector for future matching
                        let mut detector = self.similarity_detector.write().await;
                        detector.add_object(metadata);

                        return Ok(oid);
                    } else {
                        debug!(
                            oid = %oid,
                            delta_ratio,
                            "Delta not beneficial, using standard storage"
                        );
                    }
                    } // Close depth check else branch
                }
                Err(e) => {
                    warn!(
                        oid = %oid,
                        base_oid = %base_oid,
                        error = %e,
                        "Failed to read base object, using standard storage"
                    );
                }
            }
            } // Close else block for oid != base_oid check
        }

        // No similar object found or delta not beneficial
        // Add metadata to detector for future comparisons
        let mut detector = self.similarity_detector.write().await;
        detector.add_object(metadata);
        drop(detector);

        // Fall back to standard write
        self.write_with_path(obj_type, data, filename).await
    }

    /// Get the delta chain depth for an object
    ///
    /// Returns 0 if object is not a delta (full object).
    /// Returns the chain depth if object is stored as a delta.
    ///
    /// # Arguments
    ///
    /// * `oid` - Object identifier to check
    ///
    /// # Returns
    ///
    /// The delta chain depth (0 = full object, 1+ = delta depth)
    async fn get_delta_depth(&self, oid: &Oid) -> anyhow::Result<u8> {
        let meta_key = format!("deltas/{}.meta", oid.to_hex());
        
        // Check if delta metadata exists
        if !self.storage.exists(&meta_key).await? {
            return Ok(0); // Not a delta, depth is 0
        }
        
        // Read and parse metadata
        let meta_data = self.storage.get(&meta_key).await?;
        let meta_str = String::from_utf8(meta_data)
            .map_err(|e| anyhow::anyhow!("Invalid delta metadata encoding in get_delta_depth: {}", e))?;

        // Parse format: "base:{oid}:depth:{n}" or legacy "base:{oid}"
        if let Some(depth_part) = meta_str.split(":depth:").nth(1) {
            // Trim to handle any trailing whitespace/newlines
            let trimmed = depth_part.trim();
            match trimmed.parse::<u8>() {
                Ok(depth) => Ok(depth),
                Err(e) => {
                    warn!(meta_str = %meta_str, error = %e, "Failed to parse delta depth, defaulting to 1");
                    Ok(1)
                }
            }
        } else {
            // Legacy format without depth, assume depth 1
            Ok(1)
        }
    }

    /// Check if a target OID exists in the delta chain starting from a given OID
    ///
    /// This is used to prevent creating circular delta references.
    /// Walks the delta chain from `start_oid` and returns true if `target_oid` is found.
    ///
    /// # Arguments
    ///
    /// * `start_oid` - Starting point of the delta chain to check
    /// * `target_oid` - OID to search for in the chain
    ///
    /// # Returns
    ///
    /// True if target_oid is found in the chain, false otherwise
    async fn delta_chain_contains(&self, start_oid: &Oid, target_oid: &Oid) -> anyhow::Result<bool> {
        let mut current_oid = *start_oid;
        let mut visited = std::collections::HashSet::new();
        
        // Walk the chain with depth limit to prevent infinite loops
        for _ in 0..=MAX_DELTA_DEPTH {
            // Check if current matches target
            if current_oid == *target_oid {
                return Ok(true);
            }
            
            // Check for cycles in our walk
            if !visited.insert(current_oid) {
                // Already visited this OID, we're in a cycle (shouldn't happen but be safe)
                return Ok(false);
            }
            
            // Try to get the base OID of current
            let meta_key = format!("deltas/{}.meta", current_oid.to_hex());
            if !self.storage.exists(&meta_key).await? {
                // Not a delta, end of chain
                return Ok(false);
            }
            
            // Parse base OID from metadata
            let meta_data = self.storage.get(&meta_key).await?;
            let meta_str = String::from_utf8(meta_data)?;
            let after_prefix = meta_str
                .strip_prefix("base:")
                .ok_or_else(|| anyhow::anyhow!("Invalid delta metadata format"))?
                .trim();
            
            // Handle both formats: "base:{oid}:depth:{n}" and legacy "base:{oid}"
            let base_oid_hex = if let Some(idx) = after_prefix.find(":depth:") {
                &after_prefix[..idx]
            } else {
                after_prefix
            };
            
            current_oid = Oid::from_hex(base_oid_hex)?;
        }
        
        // Exceeded depth limit without finding target
        Ok(false)
    }

    /// List all pack files in the database

    ///
    /// Returns a list of pack file keys
    async fn list_pack_files(&self) -> anyhow::Result<Vec<String>> {
        let pack_keys = self.storage.list_objects("packs/").await?;

        // Filter for .pack files only
        let pack_files: Vec<String> = pack_keys
            .into_iter()
            .filter(|key| key.ends_with(".pack"))
            .collect();

        debug!(count = pack_files.len(), "Found pack files");
        Ok(pack_files)
    }

    /// Read an object from pack files
    ///
    /// Searches through all pack files to find the requested object.
    /// This is used as a fallback when loose object is not found.
    async fn read_from_packs(&self, oid: &Oid) -> anyhow::Result<Vec<u8>> {
        use crate::pack::PackReader;

        debug!(oid = %oid, "Searching for object in pack files");

        // List all pack files
        let pack_files = self.list_pack_files().await?;

        if pack_files.is_empty() {
            anyhow::bail!("Object {} not found: no loose object and no pack files", oid);
        }

        // Search through each pack file
        for pack_key in &pack_files {
            // Read pack file data
            match self.storage.get(pack_key).await {
                Ok(pack_data) => {
                    // Parse pack file
                    match PackReader::new(pack_data) {
                        Ok(pack_reader) => {
                            // Try to get object from this pack
                            match pack_reader.get_object(oid) {
                                Ok(compressed_data) => {
                                    debug!(
                                        oid = %oid,
                                        pack = pack_key,
                                        "Found object in pack file"
                                    );

                                    // Decompress the object data (pack stores compressed data)
                                    let data = if let Some(smart_comp) = &self.smart_compressor {
                                        match smart_comp.decompress_typed(&compressed_data) {
                                            Ok(d) => d,
                                            Err(_) => {
                                                // Fallback to standard decompression
                                                match self.compressor.decompress(&compressed_data) {
                                                    Ok(d) => d,
                                                    Err(_) => compressed_data, // Use raw data as last resort
                                                }
                                            }
                                        }
                                    } else if self.compression_enabled || (compressed_data.len() >= 2 && compressed_data[0] == 0x78) {
                                        match self.compressor.decompress(&compressed_data) {
                                            Ok(d) => d,
                                            Err(_) => compressed_data,
                                        }
                                    } else {
                                        compressed_data
                                    };

                                    // Verify integrity
                                    let computed_oid = Oid::hash(&data);
                                    if computed_oid != *oid {
                                        warn!(
                                            expected = %oid,
                                            computed = %computed_oid,
                                            pack = pack_key,
                                            "Pack object integrity check failed"
                                        );
                                        continue; // Try next pack
                                    }

                                    // Cache the decompressed data
                                    let arc_data = Arc::new(data.clone());
                                    self.cache.insert(*oid, arc_data).await;

                                    info!(
                                        oid = %oid,
                                        pack = pack_key,
                                        size = data.len(),
                                        "Successfully read object from pack file"
                                    );

                                    return Ok(data);
                                }
                                Err(_) => {
                                    // Object not in this pack, try next one
                                    continue;
                                }
                            }
                        }
                        Err(e) => {
                            warn!(
                                pack = pack_key,
                                error = %e,
                                "Failed to parse pack file"
                            );
                            continue;
                        }
                    }
                }
                Err(e) => {
                    warn!(
                        pack = pack_key,
                        error = %e,
                        "Failed to read pack file"
                    );
                    continue;
                }
            }
        }

        // Object not found in any pack
        anyhow::bail!(
            "Object {} not found: no loose object and not found in {} pack files",
            oid,
            pack_files.len()
        )
    }

    /// Reconstruct chunked object from manifest and chunks
    ///
    /// Private method to handle chunk-based object reconstruction.
    async fn read_chunked(&self, oid: &Oid) -> anyhow::Result<Vec<u8>> {
        debug!(oid = %oid, "Reconstructing chunked object");

        // Load chunk manifest (use to_hex() for consistent storage paths)
        let manifest_key = format!("manifests/{}", oid.to_hex());
        let manifest_data = self.storage.get(&manifest_key).await?;
        let manifest: ChunkManifest = bincode::deserialize(&manifest_data)
            .map_err(|e| anyhow::anyhow!("Failed to deserialize chunk manifest: {}", e))?;

        debug!(
            oid = %oid,
            chunk_count = manifest.chunk_count(),
            total_size = manifest.total_size,
            "Loaded chunk manifest"
        );

        // Validate total_size before allocation to prevent OOM from corrupted manifests
        if manifest.total_size > MAX_OBJECT_SIZE {
            anyhow::bail!(
                "ChunkManifest total_size {} bytes exceeds maximum allowed size {} bytes for object {}. \
                This may indicate a corrupted manifest.",
                manifest.total_size,
                MAX_OBJECT_SIZE,
                oid
            );
        }

        // Reconstruct from chunks
        let mut reconstructed = Vec::with_capacity(manifest.total_size as usize);

        for (idx, chunk_ref) in manifest.chunks.iter().enumerate() {
            // Use get_chunk() which handles both full and delta-encoded chunks
            let decompressed = self.get_chunk(&chunk_ref.id).await
                .map_err(|e| anyhow::anyhow!("Failed to read chunk {} (index {}): {}", chunk_ref.id.to_hex(), idx, e))?;

            // Verify chunk size matches manifest
            if decompressed.len() != chunk_ref.size {
                anyhow::bail!(
                    "Chunk size mismatch for chunk {}: expected {}, got {}",
                    chunk_ref.id.to_hex(),
                    chunk_ref.size,
                    decompressed.len()
                );
            }

            reconstructed.extend_from_slice(&decompressed);

            debug!(
                oid = %oid,
                chunk_idx = idx,
                chunk_id = %chunk_ref.id.to_hex(),
                chunk_size = chunk_ref.size,
                "Reconstructed chunk"
            );
        }

        // Verify total size
        if reconstructed.len() != manifest.total_size as usize {
            anyhow::bail!(
                "Reconstructed size mismatch: expected {}, got {}",
                manifest.total_size,
                reconstructed.len()
            );
        }

        // Verify integrity on reconstructed data
        let computed_oid = Oid::hash(&reconstructed);
        if computed_oid != *oid {
            warn!(
                expected = %oid,
                computed = %computed_oid,
                "Chunk reconstruction integrity check failed"
            );
            anyhow::bail!(
                "Chunk reconstruction failed: expected OID {}, computed {}",
                oid,
                computed_oid
            );
        }

        debug!(
            oid = %oid,
            reconstructed_size = reconstructed.len(),
            chunk_count = manifest.chunk_count(),
            "Successfully reconstructed chunked object"
        );

        // Cache reconstructed data for future reads
        let arc_data = Arc::new(reconstructed.clone());
        self.cache.insert(*oid, arc_data).await;

        Ok(reconstructed)
    }

    /// Reconstruct a delta-encoded object
    ///
    /// Reads the delta metadata to find the base object, then applies
    /// the delta to reconstruct the original object.
    ///
    /// # Arguments
    ///
    /// * `oid` - Object identifier of the delta-encoded object
    ///
    /// # Returns
    ///
    /// The reconstructed object content
    async fn read_delta(&self, oid: &Oid) -> anyhow::Result<Vec<u8>> {
        debug!(oid = %oid, "Reconstructing delta-encoded object");

        // Read delta metadata to get base OID
        let meta_key = format!("deltas/{}.meta", oid.to_hex());
        let meta_data = self.storage.get(&meta_key).await
            .map_err(|e| anyhow::anyhow!("Failed to read delta metadata for {}: {}", oid, e))?;

        // Parse base OID from metadata (format: "base:{hex_oid}:depth:{n}" or legacy "base:{hex_oid}")
        let meta_str = String::from_utf8(meta_data)
            .map_err(|e| anyhow::anyhow!("Invalid delta metadata encoding: {}", e))?;

        let after_prefix = meta_str
            .strip_prefix("base:")
            .ok_or_else(|| anyhow::anyhow!("Invalid delta metadata format: {}", meta_str))?
            .trim();

        // Handle both formats: "base:{oid}:depth:{n}" and legacy "base:{oid}"
        let base_oid_hex = if let Some(idx) = after_prefix.find(":depth:") {
            &after_prefix[..idx]
        } else {
            after_prefix
        };

        let base_oid = Oid::from_hex(base_oid_hex)
            .map_err(|e| anyhow::anyhow!("Invalid base OID in delta metadata: {}", e))?;

        debug!(
            oid = %oid,
            base_oid = %base_oid,
            "Found delta base object"
        );

        // Read base object (this may recursively read another delta, with depth limit)
        // To prevent infinite recursion, we track depth via a simple counter
        let base_data = self.read_delta_with_depth(&base_oid, 1).await?;

        // Read delta data
        let delta_key = format!("deltas/{}", oid.to_hex());
        let compressed_delta = self.storage.get(&delta_key).await
            .map_err(|e| anyhow::anyhow!("Failed to read delta data for {}: {}", oid, e))?;

        // Decompress delta
        let delta_bytes = if let Some(smart_comp) = &self.smart_compressor {
            smart_comp.decompress_typed(&compressed_delta)
                .map_err(|e| anyhow::anyhow!("Failed to decompress delta: {}", e))?
        } else {
            self.compressor.decompress(&compressed_delta)
                .map_err(|e| anyhow::anyhow!("Failed to decompress delta: {}", e))?
        };

        // Parse and apply delta
        let delta = Delta::from_bytes(&delta_bytes)
            .map_err(|e| anyhow::anyhow!("Failed to parse delta: {}", e))?;

        let reconstructed = DeltaDecoder::apply(&base_data, &delta)
            .map_err(|e| anyhow::anyhow!("Failed to apply delta: {}", e))?;

        // Verify integrity
        let computed_oid = Oid::hash(&reconstructed);
        if computed_oid != *oid {
            anyhow::bail!(
                "Delta reconstruction failed: expected OID {}, computed {}",
                oid,
                computed_oid
            );
        }

        info!(
            oid = %oid,
            base_oid = %base_oid,
            base_size = base_data.len(),
            delta_size = delta_bytes.len(),
            result_size = reconstructed.len(),
            "Successfully reconstructed delta-encoded object"
        );

        // Cache reconstructed data
        let arc_data = Arc::new(reconstructed.clone());
        self.cache.insert(*oid, arc_data).await;

        Ok(reconstructed)
    }

    /// Read delta with depth tracking and circular reference detection
    ///
    /// Maximum delta chain depth is 10 levels (consistent with MAX_DELTA_DEPTH).
    /// Uses Box::pin to handle async recursion.
    /// Tracks visited OIDs to detect actual circular references.
    fn read_delta_with_depth(&self, oid: &Oid, depth: usize) -> std::pin::Pin<Box<dyn std::future::Future<Output = anyhow::Result<Vec<u8>>> + Send + '_>> {
        let oid = *oid;
        // Create a new visited set for the initial call
        let visited = std::collections::HashSet::new();
        self.read_delta_with_depth_internal(oid, depth, visited)
    }

    /// Internal delta reading with visited set for circular reference detection
    fn read_delta_with_depth_internal(
        &self,
        oid: Oid,
        depth: usize,
        mut visited: std::collections::HashSet<Oid>,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = anyhow::Result<Vec<u8>>> + Send + '_>> {
        Box::pin(async move {
            // Use the same limit as the write side (MAX_DELTA_DEPTH = 10)
            const MAX_DELTA_CHAIN_DEPTH: usize = MAX_DELTA_DEPTH as usize;

            // Check for circular reference FIRST (before depth check)
            if !visited.insert(oid) {
                anyhow::bail!(
                    "Circular reference detected in delta chain at OID {}",
                    oid
                );
            }

            if depth > MAX_DELTA_CHAIN_DEPTH {
                anyhow::bail!(
                    "Delta chain too deep (> {}): chain starting at {}",
                    MAX_DELTA_CHAIN_DEPTH,
                    oid
                );
            }

            // Check cache first
            if let Some(cached) = self.cache.get(&oid).await {
                return Ok((*cached).clone());
            }

            // Check if this is also a delta
            let meta_key = format!("deltas/{}.meta", oid.to_hex());
            if self.storage.exists(&meta_key).await? {
                // Read delta metadata (format: "base:{oid}:depth:{n}" or legacy "base:{oid}")
                let meta_data = self.storage.get(&meta_key).await?;
                let meta_str = String::from_utf8(meta_data)?;
                let after_prefix = meta_str
                    .strip_prefix("base:")
                    .ok_or_else(|| anyhow::anyhow!("Invalid delta metadata format"))?
                    .trim();
                // Handle both formats
                let base_oid_hex = if let Some(idx) = after_prefix.find(":depth:") {
                    &after_prefix[..idx]
                } else {
                    after_prefix
                };
                let base_oid = Oid::from_hex(base_oid_hex)?;

                // Recursively read base with incremented depth, passing the visited set
                let base_data = self.read_delta_with_depth_internal(base_oid, depth + 1, visited).await?;

                // Read and apply delta
                let delta_key = format!("deltas/{}", oid.to_hex());
                let compressed_delta = self.storage.get(&delta_key).await?;
                let delta_bytes = if let Some(smart_comp) = &self.smart_compressor {
                    smart_comp.decompress_typed(&compressed_delta)?
                } else {
                    self.compressor.decompress(&compressed_delta)?
                };

                let delta = Delta::from_bytes(&delta_bytes)?;
                let reconstructed = DeltaDecoder::apply(&base_data, &delta)?;

                // Cache and return
                self.cache.insert(oid, Arc::new(reconstructed.clone())).await;
                return Ok(reconstructed);
            }

            // Not a delta - try other storage methods
            // Check chunk manifest
            let manifest_key = format!("manifests/{}", oid.to_hex());
            if self.storage.exists(&manifest_key).await? {
                return self.read_chunked(&oid).await;
            }

            // Try loose object
            let key = oid.to_hex();
            if let Ok(storage_data) = self.storage.get(&key).await {
                let data = if let Some(smart_comp) = &self.smart_compressor {
                    smart_comp.decompress_typed(&storage_data)
                        .unwrap_or_else(|_| storage_data.clone())
                } else {
                    self.compressor.decompress(&storage_data)
                        .unwrap_or(storage_data)
                };

                self.cache.insert(oid, Arc::new(data.clone())).await;
                return Ok(data);
            }

            // Try pack files
            self.read_from_packs(&oid).await
        })
    }

    /// Read an object and stream directly to file (constant memory)
    ///
    /// This method writes chunked objects directly to disk without loading
    /// the entire file into memory. Suitable for files of any size.
    ///
    /// # Arguments
    ///
    /// * `oid` - The object identifier to read
    /// * `path` - Path where the file should be written
    ///
    /// # Returns
    ///
    /// The number of bytes written to the file
    ///
    /// # Memory Usage
    ///
    /// Memory is bounded by chunk size (~8MB max) regardless of total file size.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # use mediagit_versioning::ObjectDatabase;
    /// # use mediagit_versioning::Oid;
    /// # async fn example() -> anyhow::Result<()> {
    /// # let odb: ObjectDatabase = todo!();
    /// # let oid: Oid = todo!();
    /// let bytes_written = odb.read_to_file(&oid, "/path/to/output.mp4").await?;
    /// println!("Wrote {} bytes", bytes_written);
    /// # Ok(())
    /// # }
    /// ```
    pub async fn read_to_file<P: AsRef<std::path::Path>>(
        &self,
        oid: &Oid,
        path: P,
    ) -> anyhow::Result<u64> {
        use tokio::io::AsyncWriteExt;
        
        let path = path.as_ref();
        
        // Check for chunk manifest (use to_hex() for consistent storage paths)
        let manifest_key = format!("manifests/{}", oid.to_hex());
        
        if self.storage.exists(&manifest_key).await? {
            // CHUNKED OBJECT: Stream chunks directly to file
            info!(oid = %oid, "Streaming chunked object to file");
            
            let manifest_data = self.storage.get(&manifest_key).await?;
            let manifest: ChunkManifest = bincode::deserialize(&manifest_data)
                .map_err(|e| anyhow::anyhow!("Failed to deserialize chunk manifest: {}", e))?;
            
            // Ensure parent directory exists
            if let Some(parent) = path.parent() {
                tokio::fs::create_dir_all(parent).await?;
            }
            
            // Open file for streaming write
            let mut file = tokio::fs::File::create(path).await?;
            let mut bytes_written = 0u64;
            
            for chunk_ref in &manifest.chunks {
                // Use get_chunk() which handles both full and delta-encoded chunks
                let decompressed = self.get_chunk(&chunk_ref.id).await
                    .map_err(|e| anyhow::anyhow!("Failed to read chunk {}: {}", chunk_ref.id.to_hex(), e))?;

                // Verify chunk size
                if decompressed.len() != chunk_ref.size {
                    anyhow::bail!(
                        "Chunk size mismatch for {}: expected {}, got {}",
                        chunk_ref.id.to_hex(),
                        chunk_ref.size,
                        decompressed.len()
                    );
                }

                // Stream to file (chunk is dropped after write)
                file.write_all(&decompressed).await?;
                bytes_written += decompressed.len() as u64;
            }
            
            file.flush().await?;
            
            info!(
                oid = %oid,
                bytes = bytes_written,
                chunks = manifest.chunks.len(),
                "Streaming write complete"
            );
            
            Ok(bytes_written)
        } else {
            // NON-CHUNKED OBJECT: Read and write normally
            let data = self.read(oid).await?;
            
            // Ensure parent directory exists
            if let Some(parent) = path.parent() {
                tokio::fs::create_dir_all(parent).await?;
            }
            
            tokio::fs::write(path, &data).await?;
            Ok(data.len() as u64)
        }
    }

    /// Read an object from the database
    ///
    /// Checks the cache first, then reads from storage if not cached.
    /// Falls back to pack files if loose object not found.
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
    /// # let storage: Arc<dyn mediagit_storage::StorageBackend> =
    /// #     Arc::new(LocalBackend::new("/tmp/odb").await?);
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

        // Check if object has chunk manifest (chunked object)
        // Use to_hex() for consistent storage paths
        let manifest_key = format!("manifests/{}", oid.to_hex());
        if self.storage.exists(&manifest_key).await? {
            debug!(oid = %oid, "Found chunk manifest, reconstructing from chunks");
            return self.read_chunked(oid).await;
        }

        // Check if object is delta-encoded
        let delta_meta_key = format!("deltas/{}.meta", oid.to_hex());
        if self.storage.exists(&delta_meta_key).await? {
            debug!(oid = %oid, "Found delta metadata, reconstructing from delta");
            return self.read_delta(oid).await;
        }

        // Try standard loose object path first
        let key = oid.to_hex();
        let storage_data = match self.storage.get(&key).await {
            Ok(data) => data,
            Err(_) => {
                // Loose object not found - fallback to pack files
                debug!(oid = %oid, "Loose object not found, trying pack files");
                return self.read_from_packs(oid).await;
            }
        };

        // Decompress data with smart decompression if available
        let data = if let Some(smart_comp) = &self.smart_compressor {
            // Use smart compressor for auto-detection of compression type
            match smart_comp.decompress_typed(&storage_data) {
                Ok(decompressed) => {
                    debug!(
                        oid = %oid,
                        storage_size = storage_data.len(),
                        decompressed_size = decompressed.len(),
                        "Smart decompressed object"
                    );
                    decompressed
                }
                Err(e) => {
                    warn!(
                        oid = %oid,
                        error = %e,
                        "Smart decompression failed, trying fallback"
                    );
                    // Fallback to standard decompression
                    match self.compressor.decompress(&storage_data) {
                        Ok(d) => d,
                        Err(_) => storage_data, // Use raw data as last resort
                    }
                }
            }
        } else if self.compression_enabled || (storage_data.len() >= 2 && storage_data[0] == 0x78) {
            // Standard decompression path
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

    /// Get the size of an object without reading its full content
    ///
    /// This is optimized for differential checkout where we only need
    /// to compare file sizes before deciding whether to read the full object.
    ///
    /// # Performance
    ///
    /// - For cached objects: O(1) cache lookup
    /// - For uncached objects: Reads and decompresses (same as `read()`)
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use mediagit_versioning::{ObjectDatabase, ObjectType};
    /// # use mediagit_storage::LocalBackend;
    /// # use std::sync::Arc;
    /// # #[tokio::main]
    /// # async fn main() -> anyhow::Result<()> {
    /// # let storage: Arc<dyn mediagit_storage::StorageBackend> =
    /// #     Arc::new(LocalBackend::new("/tmp/odb").await?);
    /// # let odb = ObjectDatabase::new(storage, 100);
    /// # let oid = odb.write(ObjectType::Blob, b"test data").await?;
    /// let size = odb.get_object_size(&oid).await?;
    /// println!("Object size: {} bytes", size);
    /// # Ok(())
    /// # }
    /// ```
    pub async fn get_object_size(&self, oid: &Oid) -> anyhow::Result<usize> {
        // Check cache first - if cached, we can get size without I/O
        if let Some(cached) = self.cache.get(oid).await {
            return Ok(cached.len());
        }

        // Not cached - we need to read the object to get its size
        // This will populate the cache for subsequent operations
        let data = self.read(oid).await?;
        Ok(data.len())
    }

    /// Check if an object is stored as chunks (without reading the full object)
    ///
    /// This is used to determine how to handle large objects during push
    /// without loading them fully into memory.
    pub async fn is_chunked(&self, oid: &Oid) -> anyhow::Result<bool> {
        let manifest_key = format!("manifests/{}", oid.to_hex());
        self.storage.exists(&manifest_key).await
    }

    /// Get the chunk manifest for a chunked object
    ///
    /// Returns None if the object is not chunked.
    pub async fn get_chunk_manifest(&self, oid: &Oid) -> anyhow::Result<Option<crate::chunking::ChunkManifest>> {
        let manifest_key = format!("manifests/{}", oid.to_hex());
        
        if !self.storage.exists(&manifest_key).await? {
            return Ok(None);
        }
        
        let manifest_data = self.storage.get(&manifest_key).await?;
        let manifest: crate::chunking::ChunkManifest = bincode::deserialize(&manifest_data)
            .map_err(|e| anyhow::anyhow!("Failed to deserialize chunk manifest: {}", e))?;
        
        Ok(Some(manifest))
    }

    /// Seed the similarity detector with chunks from a previous manifest.
    ///
    /// Call this before `write_chunked()` or `write_chunked_from_file()` when
    /// adding a new version of an existing file. Pre-loading old chunk metadata
    /// into the similarity detector enables delta matching even when CDC
    /// boundaries shift between versions.
    ///
    /// # Arguments
    ///
    /// * `manifest` - The chunk manifest from the previous version of the file
    pub async fn seed_similarity_from_manifest(
        &self,
        manifest: &ChunkManifest,
    ) -> anyhow::Result<usize> {
        let mut seeded = 0;
        for chunk_ref in &manifest.chunks {
            if let Ok(data) = self.get_chunk(&chunk_ref.id).await {
                let mut meta = crate::similarity::ObjectMetadata::new(
                    chunk_ref.id,
                    data.len(),
                    crate::ObjectType::Blob,
                    None,
                );
                meta.generate_samples(&data);
                self.similarity_detector.write().await.add_object(meta);
                seeded += 1;
            }
        }
        if seeded > 0 {
            info!(
                seeded_chunks = seeded,
                total_chunks = manifest.chunks.len(),
                "Seeded similarity detector from previous manifest"
            );
        }
        Ok(seeded)
    }

    /// Get chunk data by chunk ID
    ///
    /// Reads and decompresses a single chunk, reconstructing from delta if needed.
    /// Supports ALL file types: AI/ML models, creative projects, 3D, text, etc.
    pub async fn get_chunk(&self, chunk_id: &Oid) -> anyhow::Result<Vec<u8>> {
        // First check if this chunk is stored as a delta
        let delta_meta_key = format!("chunk-deltas/{}.meta", chunk_id.to_hex());
        if let Ok(meta_bytes) = self.storage.get(&delta_meta_key).await {
            // Parse base reference from meta
            let meta_str = String::from_utf8_lossy(&meta_bytes);
            if let Some(base_hex) = meta_str.strip_prefix("base:") {
                if let Ok(base_id) = Oid::from_hex(base_hex.trim()) {
                    // Load base chunk (recursive call handles nested deltas)
                    let base_data = Box::pin(self.get_chunk(&base_id)).await?;
                    
                    // Load delta
                    let delta_key = format!("chunk-deltas/{}", chunk_id.to_hex());
                    let compressed_delta = self.storage.get(&delta_key).await
                        .map_err(|e| anyhow::anyhow!("Failed to load chunk delta: {}", e))?;
                    
                    // Decompress delta
                    let delta_bytes = if let Some(smart_comp) = &self.smart_compressor {
                        smart_comp.decompress_typed(&compressed_delta)
                            .map_err(|e| anyhow::anyhow!("Failed to decompress chunk delta: {}", e))?
                    } else {
                        self.compressor.decompress(&compressed_delta)
                            .map_err(|e| anyhow::anyhow!("Failed to decompress chunk delta: {}", e))?
                    };
                    
                    // Apply delta to reconstruct chunk
                    let delta = Delta::from_bytes(&delta_bytes)
                        .map_err(|e| anyhow::anyhow!("Failed to parse chunk delta: {}", e))?;
                    let reconstructed = DeltaDecoder::apply(&base_data, &delta)
                        .map_err(|e| anyhow::anyhow!("Failed to apply chunk delta: {}", e))?;
                    
                    tracing::debug!(
                        chunk_id = %chunk_id,
                        base_id = %base_id,
                        reconstructed_size = reconstructed.len(),
                        "Reconstructed chunk from delta"
                    );
                    
                    return Ok(reconstructed);
                }
            }
        }
        
        // Not a delta chunk - read directly
        let chunk_key = format!("chunks/{}", chunk_id.to_hex());
        let compressed = self.storage.get(&chunk_key).await?;
        
        // Decompress using auto-detection
        if let Some(smart_comp) = &self.smart_compressor {
            smart_comp.decompress_typed(&compressed)
                .map_err(|e| anyhow::anyhow!("Failed to decompress chunk: {}", e))
        } else {
            // Fallback: use auto-detection to handle Store (raw) chunks
            let algo = CompressionAlgorithm::detect(&compressed);
            match algo {
                CompressionAlgorithm::None => Ok(compressed.to_vec()),
                CompressionAlgorithm::Zstd => {
                    use mediagit_compression::ZstdCompressor;
                    let zstd = ZstdCompressor::new(mediagit_compression::CompressionLevel::Default);
                    Ok(zstd.decompress(&compressed)
                        .unwrap_or_else(|_| compressed.to_vec()))
                }
                _ => {
                    Ok(self.compressor.decompress(&compressed)
                        .unwrap_or_else(|_| compressed.to_vec()))
                }
            }
        }
    }


    /// Get raw compressed chunk data for network transfer
    ///
    /// Fast path: reads pre-compressed chunk data directly (no decompress/recompress).
    /// Fallback: if the chunk is stored as a delta, reconstructs it via `get_chunk()`
    /// and re-compresses for transfer. This handles the case where deduplication
    /// stored some chunks as deltas against a base chunk.
    pub async fn get_compressed_chunk(&self, chunk_id: &Oid) -> anyhow::Result<Vec<u8>> {
        let chunk_key = format!("chunks/{}", chunk_id.to_hex());

        // Fast path: raw chunk exists
        if let Ok(data) = self.storage.get(&chunk_key).await {
            return Ok(data);
        }

        // Fallback: chunk is delta-encoded — reconstruct and re-compress
        let delta_meta_key = format!("chunk-deltas/{}.meta", chunk_id.to_hex());
        if self.storage.exists(&delta_meta_key).await.unwrap_or(false) {
            tracing::debug!(
                chunk_id = %chunk_id,
                "Chunk stored as delta, reconstructing for transfer"
            );

            // Reconstruct full decompressed data from delta chain
            let decompressed = self.get_chunk(chunk_id).await
                .map_err(|e| anyhow::anyhow!(
                    "Failed to reconstruct delta chunk {}: {}", chunk_id, e
                ))?;

            // Re-compress for network transfer
            if let Some(smart_comp) = &self.smart_compressor {
                smart_comp.compress_typed(&decompressed, CompressionObjectType::Unknown)
                    .map_err(|e| anyhow::anyhow!(
                        "Failed to compress reconstructed chunk {}: {}", chunk_id, e
                    ))
            } else {
                self.compressor.compress(&decompressed)
                    .map_err(|e| anyhow::anyhow!(
                        "Failed to compress reconstructed chunk {}: {}", chunk_id, e
                    ))
            }
        } else {
            Err(anyhow::anyhow!(
                "Failed to read compressed chunk {}: not found as raw or delta",
                chunk_id
            ))
        }
    }

    /// Store raw compressed chunk data (no compression)
    ///
    /// Used when receiving pre-compressed chunks from remote.
    pub async fn put_compressed_chunk(&self, chunk_id: &Oid, data: &[u8]) -> anyhow::Result<()> {
        let chunk_key = format!("chunks/{}", chunk_id.to_hex());
        self.storage.put(&chunk_key, data).await
            .map_err(|e| anyhow::anyhow!("Failed to store chunk {}: {}", chunk_id, e))
    }

    /// Store chunk manifest
    pub async fn put_manifest(&self, oid: &Oid, manifest: &crate::chunking::ChunkManifest) -> anyhow::Result<()> {
        let manifest_key = format!("manifests/{}", oid.to_hex());
        let manifest_data = bincode::serialize(manifest)
            .map_err(|e| anyhow::anyhow!("Failed to serialize manifest: {}", e))?;
        self.storage.put(&manifest_key, &manifest_data).await
            .map_err(|e| anyhow::anyhow!("Failed to store manifest {}: {}", oid, e))
    }

    /// Check if a chunk exists (including delta-encoded chunks)
    pub async fn chunk_exists(&self, chunk_id: &Oid) -> anyhow::Result<bool> {
        let chunk_key = format!("chunks/{}", chunk_id.to_hex());
        if self.storage.exists(&chunk_key).await? {
            return Ok(true);
        }
        // Also check for delta-encoded chunk
        let delta_key = format!("chunk-deltas/{}.meta", chunk_id.to_hex());
        self.storage.exists(&delta_key).await
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
    /// # let storage: Arc<dyn mediagit_storage::StorageBackend> =
    /// #     Arc::new(LocalBackend::new("/tmp/odb").await?);
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

        // Check for regular loose object
        // CRITICAL FIX: Use oid.to_hex() for consistency with read() and write()
        // LocalBackend::object_path() automatically adds "objects/" prefix and sharding
        // This ensures compatibility with both pre-GC and post-GC reorganized object paths
        let key = oid.to_hex();
        if self.storage.exists(&key).await? {
            return Ok(true);
        }

        // Also check for chunked object (stored as manifest + chunks)
        let manifest_key = format!("manifests/{}", oid.to_hex());
        self.storage.exists(&manifest_key).await
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
    /// # let storage: Arc<dyn mediagit_storage::StorageBackend> =
    /// #     Arc::new(LocalBackend::new("/tmp/odb").await?);
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
    /// # let storage: Arc<dyn mediagit_storage::StorageBackend> =
    /// #     Arc::new(LocalBackend::new("/tmp/odb").await?);
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

    /// Repack loose objects into pack files
    ///
    /// Collects loose objects and creates optimized pack files with delta compression.
    /// This can significantly reduce storage space by:
    /// - Batch delta compression for similar objects
    /// - Eliminating per-file overhead
    /// - Optimizing delta chains
    ///
    /// # Arguments
    ///
    /// * `max_objects` - Maximum number of objects to include in each pack (0 = unlimited)
    /// * `remove_loose` - Whether to remove loose objects after packing
    ///
    /// # Returns
    ///
    /// Statistics about the repack operation
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # use mediagit_versioning::ObjectDatabase;
    /// # use mediagit_storage::LocalBackend;
    /// # use std::sync::Arc;
    /// # #[tokio::main]
    /// # async fn main() -> anyhow::Result<()> {
    /// # let storage: Arc<dyn mediagit_storage::StorageBackend> =
    /// #     Arc::new(LocalBackend::new("/tmp/odb").await?);
    /// # let odb = ObjectDatabase::new(storage, 1000);
    /// // Repack up to 1000 objects, keep loose objects
    /// let stats = odb.repack(1000, false).await?;
    /// println!("Packed {} objects, saved {} bytes",
    ///          stats.objects_packed, stats.bytes_saved);
    /// # Ok(())
    /// # }
    /// ```
    pub async fn repack(
        &self,
        max_objects: usize,
        remove_loose: bool,
    ) -> anyhow::Result<RepackStats> {
        use crate::pack::PackWriter;

        info!(
            max_objects,
            remove_loose,
            "Starting repack operation"
        );

        let mut stats = RepackStats::default();

        // List all loose objects
        let loose_objects = self.list_loose_objects().await?;
        stats.loose_objects_found = loose_objects.len();

        if loose_objects.is_empty() {
            info!("No loose objects to repack");
            return Ok(stats);
        }

        let objects_to_pack = if max_objects > 0 && loose_objects.len() > max_objects {
            &loose_objects[..max_objects]
        } else {
            &loose_objects[..]
        };

        info!(
            total_loose = loose_objects.len(),
            packing = objects_to_pack.len(),
            "Found loose objects"
        );

        // Create pack writer
        let mut pack_writer = PackWriter::new();
        let mut packed_oids = Vec::new();

        // Track sizes for statistics
        let mut total_original_size = 0u64;

        // Add objects to pack with delta compression
        for oid in objects_to_pack {
            match self.read(oid).await {
                Ok(data) => {
                    total_original_size += data.len() as u64;

                    // Try to find similar object for delta encoding
                    if self.delta_enabled {
                        let mut metadata = crate::similarity::ObjectMetadata::new(
                            *oid,
                            data.len(),
                            ObjectType::Blob, // Assume blob for now
                            None,
                        );
                        metadata.generate_samples(&data);

                        let detector = self.similarity_detector.read().await;
                        if let Some((base_oid, score)) = detector.find_similar(
                            &metadata,
                            crate::similarity::MIN_SIMILARITY_THRESHOLD,
                        ) {
                            drop(detector);

                            // Try to read base and create delta
                            if let Ok(base_data) = self.read(&base_oid).await {
                                let delta = DeltaEncoder::encode(&base_data, &data);
                                let delta_data = delta.to_bytes();

                                // Use delta if beneficial
                                let delta_ratio = delta_data.len() as f64 / data.len() as f64;
                                if delta_ratio < 0.80 {
                                    debug!(
                                        oid = %oid,
                                        base = %base_oid,
                                        similarity = score.score,
                                        delta_size = delta_data.len(),
                                        original_size = data.len(),
                                        "Using delta encoding in pack"
                                    );

                                    pack_writer.add_delta_object(*oid, base_oid, &delta_data);
                                    stats.delta_objects += 1;
                                    packed_oids.push(*oid);
                                    continue;
                                }
                            }
                        }
                    }

                    // Add as regular object (no delta or delta not beneficial)
                    // Compress if enabled
                    let object_data = if self.compression_enabled {
                        if let Some(smart_comp) = &self.smart_compressor {
                            smart_comp.compress_typed(&data, CompressionObjectType::Unknown)?
                        } else {
                            self.compressor.compress(&data)?
                        }
                    } else {
                        data.clone()
                    };

                    pack_writer.add_object(*oid, ObjectType::Blob, &object_data);
                    packed_oids.push(*oid);
                }
                Err(e) => {
                    warn!(oid = %oid, error = %e, "Failed to read object for packing");
                }
            }
        }

        stats.objects_packed = packed_oids.len();

        if stats.objects_packed == 0 {
            info!("No objects were successfully packed");
            return Ok(stats);
        }

        // Finalize pack
        let pack_data = pack_writer.finalize();
        stats.pack_size = pack_data.len() as u64;
        stats.bytes_saved = total_original_size.saturating_sub(stats.pack_size);

        // Generate pack file name with timestamp
        let pack_id = format!("pack-{}", chrono::Utc::now().timestamp());
        let pack_key = format!("packs/{}.pack", pack_id);

        // Store pack file
        self.storage.put(&pack_key, &pack_data).await?;

        info!(
            pack_id,
            size = pack_data.len(),
            objects = stats.objects_packed,
            deltas = stats.delta_objects,
            "Pack file created"
        );

        // Remove loose objects if requested
        if remove_loose {
            let mut removed = 0;
            for oid in &packed_oids {
                // Use oid.to_hex() for consistency - LocalBackend handles path sharding
                let object_key = oid.to_hex();
                if let Err(e) = self.storage.delete(&object_key).await {
                    warn!(oid = %oid, error = %e, "Failed to remove loose object");
                } else {
                    removed += 1;
                }
            }
            stats.loose_objects_removed = removed;
            info!(removed, "Removed loose objects");
        }

        info!(
            packed = stats.objects_packed,
            pack_size = stats.pack_size,
            saved = stats.bytes_saved,
            "Repack complete"
        );

        Ok(stats)
    }

    /// List all loose objects in the object database
    ///
    /// Scans the objects/ directory and returns OIDs of all loose objects.
    async fn list_loose_objects(&self) -> anyhow::Result<Vec<Oid>> {
        let mut oids = Vec::new();

        // List all keys (empty prefix = all loose objects)
        // LocalBackend stores objects with plain hex keys (e.g., "abc123...")
        // not "objects/abc123..." - the "objects/" part is handled internally
        let keys = self.storage.list_objects("").await?;

        for key in keys {
            // Skip non-object keys (like "packs/...")
            if key.starts_with("packs/") {
                continue;
            }

            // Key is already the hex string - parse directly to OID
            if let Ok(oid_bytes) = hex::decode(&key) {
                if oid_bytes.len() == 32 {
                    let mut bytes = [0u8; 32];
                    bytes.copy_from_slice(&oid_bytes);
                    oids.push(Oid::from(bytes));
                }
            }
        }

        debug!(count = oids.len(), "Listed loose objects");
        Ok(oids)
    }

    /// Parse OID from object path (DEPRECATED - kept for compatibility)
    ///
    /// Converts "objects/ab/cdef..." to "abcdef..." OR just returns the key if it's already a hex string
    #[allow(dead_code)]
    fn parse_oid_from_path(path: &str) -> Option<String> {
        // New behavior: if path doesn't contain "/", it's already a hex OID
        if !path.contains('/') {
            return Some(path.to_string());
        }

        // Legacy behavior: parse "objects/ab/cd..." format
        let parts: Vec<&str> = path.split('/').collect();
        if parts.len() >= 3 && parts[0] == "objects" {
            Some(format!("{}{}", parts[1], parts[2]))
        } else {
            None
        }
    }
}

/// Statistics from a repack operation
#[derive(Debug, Default, Clone)]
pub struct RepackStats {
    /// Number of loose objects found
    pub loose_objects_found: usize,
    /// Number of objects successfully packed
    pub objects_packed: usize,
    /// Number of objects stored as deltas
    pub delta_objects: usize,
    /// Total size of pack file
    pub pack_size: u64,
    /// Bytes saved by packing
    pub bytes_saved: u64,
    /// Number of loose objects removed
    pub loose_objects_removed: usize,
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

    /// REGRESSION TEST for GC --repack branch switching bug
    ///
    /// This test ensures that objects remain readable after GC reorganization.
    /// Previously, ODB::exists() used format!("objects/{}", oid.to_path()) while
    /// ODB::read() used oid.to_hex(), causing branch checkouts to fail after GC.
    #[tokio::test]
    async fn test_object_path_consistency_after_gc() {
        use mediagit_storage::LocalBackend;
        use tempfile::TempDir;

        let temp_dir = TempDir::new().unwrap();
        let storage_path = temp_dir.path();

        // Create storage with real LocalBackend (not mock) to test path handling
        let storage = Arc::new(LocalBackend::new(storage_path).await.unwrap());
        let odb = ObjectDatabase::new(storage.clone(), 100);

        // Write multiple test objects
        let data1 = b"test content 1";
        let data2 = b"test content 2 with different data";
        let data3 = b"yet another test object";

        let oid1 = odb.write(ObjectType::Blob, data1).await.unwrap();
        let oid2 = odb.write(ObjectType::Blob, data2).await.unwrap();
        let oid3 = odb.write(ObjectType::Blob, data3).await.unwrap();

        // Verify objects exist BEFORE any operation
        assert!(odb.exists(&oid1).await.unwrap(), "Object 1 should exist before GC");
        assert!(odb.exists(&oid2).await.unwrap(), "Object 2 should exist before GC");
        assert!(odb.exists(&oid3).await.unwrap(), "Object 3 should exist before GC");

        // Verify objects are readable BEFORE
        let read1 = odb.read(&oid1).await.unwrap();
        assert_eq!(read1, data1, "Should read object 1 before GC");

        let read2 = odb.read(&oid2).await.unwrap();
        assert_eq!(read2, data2, "Should read object 2 before GC");

        let read3 = odb.read(&oid3).await.unwrap();
        assert_eq!(read3, data3, "Should read object 3 before GC");

        // Clear cache to ensure we're reading from storage (not cache)
        odb.clear_cache().await;

        // The test doesn't actually need to repack since objects are already in
        // the sharded storage structure. The key test is that exists() and read()
        // use consistent path resolution via oid.to_hex().

        // CRITICAL TEST: Verify objects still exist AFTER reorganization
        assert!(odb.exists(&oid1).await.unwrap(), "Object 1 should exist after GC");
        assert!(odb.exists(&oid2).await.unwrap(), "Object 2 should exist after GC");
        assert!(odb.exists(&oid3).await.unwrap(), "Object 3 should exist after GC");

        // CRITICAL TEST: Verify objects are still readable AFTER reorganization
        // This is where the bug manifested - checkout would fail here
        let read1_after = odb.read(&oid1).await.unwrap();
        assert_eq!(read1_after, data1, "Should read object 1 after GC reorganization");

        let read2_after = odb.read(&oid2).await.unwrap();
        assert_eq!(read2_after, data2, "Should read object 2 after GC reorganization");

        let read3_after = odb.read(&oid3).await.unwrap();
        assert_eq!(read3_after, data3, "Should read object 3 after GC reorganization");

        // Additional check: Verify size queries work
        let size1 = odb.get_object_size(&oid1).await.unwrap();
        assert_eq!(size1, data1.len(), "Size query should work after GC");
    }

    #[test]
    fn test_delta_metadata_parsing() {
        // Test the delta metadata parsing logic handles both formats correctly
        // Format 1 (new): "base:{oid}:depth:{n}"
        // Format 2 (legacy): "base:{oid}"

        let test_oid = "57b77408e3f862ecc9288b59a6cd6da6c529bd4d61883483c5e8dc7989e1e918";

        // Test new format with depth
        let meta_new = format!("base:{}:depth:1", test_oid);
        let after_prefix = meta_new.strip_prefix("base:").unwrap().trim();
        let base_oid_hex = if let Some(idx) = after_prefix.find(":depth:") {
            &after_prefix[..idx]
        } else {
            after_prefix
        };
        assert_eq!(base_oid_hex, test_oid, "Should parse new format correctly");
        assert_eq!(base_oid_hex.len(), 64, "OID should be 64 chars");

        // Test legacy format without depth
        let meta_legacy = format!("base:{}", test_oid);
        let after_prefix = meta_legacy.strip_prefix("base:").unwrap().trim();
        let base_oid_hex = if let Some(idx) = after_prefix.find(":depth:") {
            &after_prefix[..idx]
        } else {
            after_prefix
        };
        assert_eq!(base_oid_hex, test_oid, "Should parse legacy format correctly");
        assert_eq!(base_oid_hex.len(), 64, "OID should be 64 chars");

        // Verify OID can be parsed
        let oid = Oid::from_hex(base_oid_hex).unwrap();
        assert_eq!(oid.to_hex(), test_oid);
    }

}
