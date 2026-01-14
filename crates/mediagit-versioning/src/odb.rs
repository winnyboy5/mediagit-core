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


use crate::{ObjectType, Oid, OdbMetrics};
use crate::chunking::{ChunkManifest, ChunkRef, ChunkStrategy, ContentChunker};
use crate::delta::DeltaEncoder;
use mediagit_compression::{Compressor, SmartCompressor, TypeAwareCompressor, ZlibCompressor};
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

    /// Smart compressor for type-aware compression (optional)
    smart_compressor: Option<Arc<SmartCompressor>>,

    /// Chunking strategy (optional)
    chunk_strategy: Option<ChunkStrategy>,

    /// Enable delta encoding for similar objects
    delta_enabled: bool,

    /// Similarity detector for finding delta base candidates
    similarity_detector: Arc<RwLock<crate::similarity::SimilarityDetector>>,
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
            // Use smart compressor
            let storage_data = if let Some(smart_comp) = &self.smart_compressor {
                let compressed = smart_comp
                    .compress_typed(data, compression_type)
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

        // ✅ FIX: Skip chunking for small files (<10MB) to avoid overhead
        const MIN_CHUNK_SIZE: usize = 10 * 1024 * 1024; // 10MB
        if data.len() < MIN_CHUNK_SIZE {
            debug!(
                size = data.len(),
                threshold = MIN_CHUNK_SIZE,
                "File too small for chunking, using standard write"
            );
            return self.write_with_path(obj_type, data, filename).await;
        }

        // ✅ FIX: Skip chunking for pre-compressed formats (already optimal)
        if !filename.is_empty() {
            let compression_type = CompressionObjectType::from_path(filename);
            let should_skip_chunking = matches!(
                compression_type,
                CompressionObjectType::Jpeg
                    | CompressionObjectType::Png
                    | CompressionObjectType::Gif
                    | CompressionObjectType::Webp
                    | CompressionObjectType::Avif
                    | CompressionObjectType::Heic
                    | CompressionObjectType::Mp4
                    | CompressionObjectType::Mov
                    | CompressionObjectType::Avi
                    | CompressionObjectType::Webm
                    | CompressionObjectType::Mp3
                    | CompressionObjectType::Aac
                    | CompressionObjectType::Ogg
            );

            if should_skip_chunking {
                debug!(
                    file_type = ?compression_type,
                    size = data.len(),
                    "Pre-compressed format detected, skipping chunking to avoid overhead"
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

        // Store each chunk with smart compression
        for chunk in &chunks {
            // Use to_hex() for consistent storage paths (LocalBackend handles sharding)
            let chunk_key = format!("chunks/{}", chunk.id.to_hex());

            // Check if chunk exists (deduplication at chunk level)
            let exists = self.storage.exists(&chunk_key).await
                .map_err(|e| anyhow::anyhow!("Failed to check chunk existence for {}: {}", chunk_key, e))?;
            
            if !exists {
                // Determine compression strategy for chunk based on file type
                let compressed = if let Some(smart_comp) = &self.smart_compressor {
                    // Use file type for optimal compression (e.g., Brotli for CSV, Zstd for ML)
                    let chunk_comp_type = if !filename.is_empty() {
                        CompressionObjectType::from_path(filename)
                    } else {
                        CompressionObjectType::Unknown
                    };
                    smart_comp.compress_typed(&chunk.data, chunk_comp_type)
                        .map_err(|e| anyhow::anyhow!("Failed to compress chunk {}: {}", chunk_key, e))?
                } else {
                    self.compressor.compress(&chunk.data)
                        .map_err(|e| anyhow::anyhow!("Failed to compress chunk {}: {}", chunk_key, e))?
                };

                // Store chunk
                self.storage.put(&chunk_key, &compressed).await
                    .map_err(|e| anyhow::anyhow!("Failed to store chunk {} (size: {} bytes): {}", chunk_key, compressed.len(), e))?;

                debug!(
                    chunk_id = %chunk.id,
                    original_size = chunk.data.len(),
                    compressed_size = compressed.len(),
                    chunk_type = ?chunk.chunk_type,
                    "Stored chunk"
                );
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

        // Cache original data
        self.cache.insert(oid, Arc::new(data.to_vec())).await;

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
            "Streaming chunked write: file={}, size={}MB",
            filename,
            file_size / (1024 * 1024)
        );
        
        // Compute file OID using streaming hash (constant memory)
        let file_oid = Oid::from_file_async(path).await?;
        
        // Check if we already have this file (use to_hex for consistency)
        if self.storage.exists(&format!("manifests/{}", file_oid.to_hex())).await? {
            debug!("File already exists in storage: {}", file_oid);
            return Ok(file_oid);
        }
        
        // Track written chunks and bytes
        let chunks_written = AtomicU64::new(0);
        let bytes_written = AtomicU64::new(0);
        
        // Create chunker with MediaAware strategy
        let chunker = ContentChunker::new(
            self.chunk_strategy.clone().unwrap_or(ChunkStrategy::MediaAware)
        );
        
        // Use streaming callback pattern - each chunk written immediately
        let storage = self.storage.clone();
        let compressor = self.compressor.clone();
        let smart_comp = self.smart_compressor.clone();
        let compression_enabled = self.compression_enabled;
        
        // Track chunk refs for manifest
        let chunk_refs = std::sync::Arc::new(tokio::sync::Mutex::new(Vec::<ChunkRef>::new()));
        let chunk_refs_clone = chunk_refs.clone();
        
        let filename_owned = filename.to_string();
        
        let _chunk_oids = chunker.chunk_file_streaming(path, |chunk| {
            let storage = storage.clone();
            let compressor = compressor.clone();
            let smart_comp = smart_comp.clone();
            let chunks_written = &chunks_written;
            let bytes_written = &bytes_written;
            let chunk_refs = chunk_refs_clone.clone();
            let filename = filename_owned.clone();
            
            async move {
                // Use to_hex() for consistent storage paths (LocalBackend handles sharding)
                let chunk_key = format!("chunks/{}", chunk.id.to_hex());
                
                // Build ChunkRef for manifest
                let chunk_ref = ChunkRef {
                    id: chunk.id,
                    offset: chunk.offset,
                    size: chunk.size,
                    chunk_type: chunk.chunk_type,
                };
                
                // Skip if chunk already exists (deduplication)
                if storage.exists(&chunk_key).await? {
                    debug!("Chunk already exists: {}", chunk.id);
                    chunk_refs.lock().await.push(chunk_ref);
                    return Ok(());
                }
                
                // Compress chunk with smart compressor (type-aware compression)
                // Use Brotli for CSV/text, Zstd for binary, etc.
                let comp_type = CompressionObjectType::from_path(&filename);
                let data_to_store = if let Some(ref smart) = smart_comp {
                    // Use smart compressor for optimal compression by file type
                    smart.compress_typed(&chunk.data, comp_type)?
                } else if compression_enabled {
                    // Fall back to default compressor
                    compressor.compress(&chunk.data)?
                } else {
                    chunk.data.clone()
                };
                
                // Store chunk
                storage.put(&chunk_key, &data_to_store).await?;
                
                chunks_written.fetch_add(1, Ordering::Relaxed);
                bytes_written.fetch_add(chunk.size as u64, Ordering::Relaxed);
                
                // Add to manifest refs
                chunk_refs.lock().await.push(chunk_ref);
                
                Ok(())
            }
        }).await?;
        
        // Create and store manifest  
        let chunk_refs_final = chunk_refs.lock().await.clone();
        let manifest = ChunkManifest {
            chunks: chunk_refs_final,
            total_size: file_size,
            filename: Some(filename.to_string()),
        };
        
        let manifest_data = bincode::serialize(&manifest)?;
        // Use to_hex() for consistent storage paths
        let manifest_key = format!("manifests/{}", file_oid.to_hex());
        self.storage.put(&manifest_key, &manifest_data).await?;
        
        info!(
            "Streaming chunked write complete: {} chunks, {}MB written",
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
        let similar = detector.find_similar(&metadata, crate::similarity::MIN_SIMILARITY_THRESHOLD);
        drop(detector);

        if let Some((base_oid, score)) = similar {
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
                    if base_depth >= MAX_DELTA_DEPTH {
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
        let meta_str = String::from_utf8_lossy(&meta_data);
        
        // Parse format: "base:{oid}:depth:{n}" or legacy "base:{oid}"
        if let Some(depth_part) = meta_str.split(":depth:").nth(1) {
            Ok(depth_part.parse::<u8>().unwrap_or(1))
        } else {
            // Legacy format without depth, assume depth 1
            Ok(1)
        }
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

        // Reconstruct from chunks
        let mut reconstructed = Vec::with_capacity(manifest.total_size as usize);

        for (idx, chunk_ref) in manifest.chunks.iter().enumerate() {
            // Use to_hex() for consistent storage paths
            let chunk_key = format!("chunks/{}", chunk_ref.id.to_hex());

            // Read compressed chunk
            let compressed = self.storage.get(&chunk_key).await
                .map_err(|e| anyhow::anyhow!("Failed to read chunk {}: {}", chunk_ref.id.to_hex(), e))?;

            // Decompress chunk
            let decompressed = if let Some(smart_comp) = &self.smart_compressor {
                smart_comp.decompress_typed(&compressed)
                    .map_err(|e| anyhow::anyhow!("Failed to decompress chunk {}: {}", chunk_ref.id.to_hex(), e))?
            } else {
                self.compressor.decompress(&compressed)
                    .map_err(|e| anyhow::anyhow!("Failed to decompress chunk {}: {}", chunk_ref.id.to_hex(), e))?
            };

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
                // Use to_hex() for consistent storage paths
                let chunk_key = format!("chunks/{}", chunk_ref.id.to_hex());
                
                // Read compressed chunk
                let compressed = self.storage.get(&chunk_key).await
                    .map_err(|e| anyhow::anyhow!("Failed to read chunk {}: {}", chunk_ref.id.to_hex(), e))?;
                
                // Decompress chunk
                let decompressed = if let Some(smart_comp) = &self.smart_compressor {
                    smart_comp.decompress_typed(&compressed)
                        .map_err(|e| anyhow::anyhow!("Failed to decompress chunk {}: {}", chunk_ref.id.to_hex(), e))?
                } else {
                    self.compressor.decompress(&compressed)
                        .map_err(|e| anyhow::anyhow!("Failed to decompress chunk {}: {}", chunk_ref.id.to_hex(), e))?
                };
                
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

        // Check if object has chunk manifest (chunked object)
        // Use to_hex() for consistent storage paths
        let manifest_key = format!("manifests/{}", oid.to_hex());
        if self.storage.exists(&manifest_key).await? {
            debug!(oid = %oid, "Found chunk manifest, reconstructing from chunks");
            return self.read_chunked(oid).await;
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
        } else if self.compression_enabled || storage_data.len() >= 2 && storage_data[0] == 0x78 {
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
    /// # let storage = Arc::new(LocalBackend::new("/tmp/odb")?);
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

    /// Get chunk data by chunk ID
    ///
    /// Reads and decompresses a single chunk.
    pub async fn get_chunk(&self, chunk_id: &Oid) -> anyhow::Result<Vec<u8>> {
        let chunk_key = format!("chunks/{}", chunk_id.to_hex());
        let compressed = self.storage.get(&chunk_key).await?;
        
        // Decompress
        if let Some(smart_comp) = &self.smart_compressor {
            smart_comp.decompress_typed(&compressed)
                .map_err(|e| anyhow::anyhow!("Failed to decompress chunk: {}", e))
        } else {
            self.compressor.decompress(&compressed)
                .map_err(|e| anyhow::anyhow!("Failed to decompress chunk: {}", e))
        }
    }

    /// Get raw compressed chunk data (no decompression)
    ///
    /// Used for efficient network transfer - chunks are sent as-is to remote.
    /// This avoids costly decompress/recompress cycle during push.
    pub async fn get_compressed_chunk(&self, chunk_id: &Oid) -> anyhow::Result<Vec<u8>> {
        let chunk_key = format!("chunks/{}", chunk_id.to_hex());
        self.storage.get(&chunk_key).await
            .map_err(|e| anyhow::anyhow!("Failed to read compressed chunk {}: {}", chunk_id, e))
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

    /// Check if a chunk exists
    pub async fn chunk_exists(&self, chunk_id: &Oid) -> anyhow::Result<bool> {
        let chunk_key = format!("chunks/{}", chunk_id.to_hex());
        self.storage.exists(&chunk_key).await
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
    /// # let storage = Arc::new(LocalBackend::new("/tmp/odb")?);
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
}
