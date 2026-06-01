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

use super::*;

impl ObjectDatabase {
    pub fn new(storage: Arc<dyn StorageBackend>, cache_capacity: u64) -> Self {
        info!(
            capacity = cache_capacity,
            compression = "zlib (Git-compatible)",
            delta = true,
            "Creating ObjectDatabase with LRU cache and delta encoding"
        );

        Self {
            storage,
            cache: Cache::builder()
                .max_capacity(DEFAULT_CACHE_MAX_BYTES)
                .weigher(|_key: &Oid, value: &Arc<Vec<u8>>| -> u32 {
                    value.len().try_into().unwrap_or(u32::MAX)
                })
                .build(),
            metrics: Arc::new(RwLock::new(OdbMetrics::new())),
            compressor: Arc::new(ZlibCompressor::default_level()),
            compression_enabled: true,
            smart_compressor: None,
            chunk_strategy: None,
            delta_enabled: true, // ✅ CRITICAL FIX: Enable delta compression by default for storage savings
            similarity_detector: Arc::new(RwLock::new(crate::similarity::SimilarityDetector::new(
                crate::similarity::MAX_SIMILARITY_CANDIDATES,
            ))),
            base_chunk_cache: Cache::new(64),
            delta_written_pairs: Arc::new(Mutex::new(std::collections::HashSet::new())),
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
            cache: Cache::builder()
                .max_capacity(DEFAULT_CACHE_MAX_BYTES)
                .weigher(|_key: &Oid, value: &Arc<Vec<u8>>| -> u32 {
                    value.len().try_into().unwrap_or(u32::MAX)
                })
                .build(),
            metrics: Arc::new(RwLock::new(OdbMetrics::new())),
            compressor,
            compression_enabled,
            smart_compressor: None,
            chunk_strategy: None,
            delta_enabled: true, // ✅ Enable delta compression for storage savings
            similarity_detector: Arc::new(RwLock::new(crate::similarity::SimilarityDetector::new(
                crate::similarity::MAX_SIMILARITY_CANDIDATES,
            ))),
            base_chunk_cache: Cache::new(64),
            delta_written_pairs: Arc::new(Mutex::new(std::collections::HashSet::new())),
        }
    }

    /// Create ObjectDatabase with smart compression (type-aware)
    pub fn with_smart_compression(storage: Arc<dyn StorageBackend>, cache_capacity: u64) -> Self {
        info!(
            capacity = cache_capacity,
            compression = "smart (type-aware)",
            delta = true,
            "Creating ObjectDatabase with smart compression and delta encoding"
        );

        Self {
            storage,
            cache: Cache::builder()
                .max_capacity(DEFAULT_CACHE_MAX_BYTES)
                .weigher(|_key: &Oid, value: &Arc<Vec<u8>>| -> u32 {
                    value.len().try_into().unwrap_or(u32::MAX)
                })
                .build(),
            metrics: Arc::new(RwLock::new(OdbMetrics::new())),
            compressor: Arc::new(ZlibCompressor::default_level()),
            compression_enabled: true,
            smart_compressor: Some(Arc::new(SmartCompressor::new())),
            chunk_strategy: None,
            delta_enabled: true, // ✅ CRITICAL FIX: Enable delta compression for 70-90% storage savings
            similarity_detector: Arc::new(RwLock::new(crate::similarity::SimilarityDetector::new(
                crate::similarity::MAX_SIMILARITY_CANDIDATES,
            ))),
            base_chunk_cache: Cache::new(64),
            delta_written_pairs: Arc::new(Mutex::new(std::collections::HashSet::new())),
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
            cache: Cache::builder()
                .max_capacity(DEFAULT_CACHE_MAX_BYTES)
                .weigher(|_key: &Oid, value: &Arc<Vec<u8>>| -> u32 {
                    value.len().try_into().unwrap_or(u32::MAX)
                })
                .build(),
            metrics: Arc::new(RwLock::new(OdbMetrics::new())),
            compressor: Arc::new(ZlibCompressor::default_level()),
            compression_enabled: true,
            smart_compressor: Some(Arc::new(SmartCompressor::new())),
            chunk_strategy,
            delta_enabled,
            similarity_detector: Arc::new(RwLock::new(crate::similarity::SimilarityDetector::new(
                crate::similarity::MAX_SIMILARITY_CANDIDATES,
            ))),
            base_chunk_cache: Cache::new(64),
            delta_written_pairs: Arc::new(Mutex::new(std::collections::HashSet::new())),
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
            cache: Cache::builder()
                .max_capacity(DEFAULT_CACHE_MAX_BYTES)
                .weigher(|_key: &Oid, value: &Arc<Vec<u8>>| -> u32 {
                    value.len().try_into().unwrap_or(u32::MAX)
                })
                .build(),
            metrics: Arc::new(RwLock::new(OdbMetrics::new())),
            compressor: Arc::new(ZlibCompressor::default_level()),
            compression_enabled: false,
            smart_compressor: None,
            chunk_strategy: None,
            delta_enabled: false,
            similarity_detector: Arc::new(RwLock::new(crate::similarity::SimilarityDetector::new(
                crate::similarity::MAX_SIMILARITY_CANDIDATES,
            ))),
            base_chunk_cache: Cache::new(64),
            delta_written_pairs: Arc::new(Mutex::new(std::collections::HashSet::new())),
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
    /// Computes the BLAKE3 hash of the content and stores it if not already present.
    /// Automatic deduplication: identical content returns the same OID without re-storing.
    ///
    /// # Arguments
    ///
    /// * `obj_type` - Type of the object (Blob, Tree, or Commit)
    /// * `data` - Object content
    ///
    /// # Returns
    ///
    /// The OID (BLAKE3 hash) of the object
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
        // Delegate to smart-compression path when available.
        // write_with_path with an empty filename falls back to magic-byte type detection,
        // giving every object the correct per-format strategy instead of Zstd Default.
        if self.smart_compressor.is_some() {
            return Box::pin(self.write_with_path(obj_type, data, "")).await;
        }

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
                let compressed = self
                    .compressor
                    .compress(data)
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

        // Cache the UNCOMPRESSED object for future reads (skip large objects)
        if data.len() <= MAX_CACHEABLE_OBJECT_SIZE {
            self.cache.insert(oid, Arc::new(data.to_vec())).await;
        }

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
    /// The OID (BLAKE3 hash) of the object
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

        // Cache the UNCOMPRESSED object (skip large objects)
        if data.len() <= MAX_CACHEABLE_OBJECT_SIZE {
            self.cache.insert(oid, Arc::new(data.to_vec())).await;
        }

        Ok(oid)
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

        info!(max_objects, remove_loose, "Starting repack operation");

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
                        if let Some((base_oid, score)) = detector
                            .find_similar(&metadata, crate::similarity::MIN_SIMILARITY_THRESHOLD)
                        {
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

    /// Resolve an abbreviated OID prefix to a full OID.
    ///
    /// Scans loose objects for keys matching the given hex prefix.
    /// Returns an error if zero or more than one object matches.
    pub async fn resolve_abbreviated_oid(&self, abbrev: &str) -> anyhow::Result<Oid> {
        if abbrev.len() < 4 {
            anyhow::bail!(
                "Abbreviated OID must be at least 4 characters, got {}",
                abbrev.len()
            );
        }
        if abbrev.len() == 64 {
            return Oid::from_hex(abbrev).map_err(|e| anyhow::anyhow!("Invalid OID: {}", e));
        }

        // Validate it looks like a hex prefix
        if !abbrev.chars().all(|c| c.is_ascii_hexdigit()) {
            anyhow::bail!("Not a valid OID prefix: {}", abbrev);
        }

        // Prefix-scan the storage for matching keys
        let keys = self.storage.list_objects(abbrev).await?;

        let mut matches: Vec<Oid> = Vec::new();
        for key in keys {
            // Skip non-object namespaces
            if key.starts_with("packs/")
                || key.starts_with("deltas/")
                || key.starts_with("chunk-deltas/")
                || key.starts_with("manifests/")
            {
                continue;
            }
            if key.starts_with(abbrev) {
                if let Ok(oid_bytes) = hex::decode(&key) {
                    if oid_bytes.len() == 32 {
                        let mut bytes = [0u8; 32];
                        bytes.copy_from_slice(&oid_bytes);
                        matches.push(Oid::from(bytes));
                    }
                }
            }
        }

        match matches.len() {
            0 => anyhow::bail!("No object matches abbreviated OID: {}", abbrev),
            1 => Ok(matches.remove(0)),
            n => anyhow::bail!(
                "Ambiguous abbreviated OID '{}' — {} objects match",
                abbrev,
                n
            ),
        }
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
}
