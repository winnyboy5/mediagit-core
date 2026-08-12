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

/// `MEDIAGIT_REPACK_CHUNKS` — gate for consolidating loose chunks into
/// Track-F-style cloud packs during `gc --repack` (GA I9). Default enabled;
/// `0` reproduces the pre-I9 behavior exactly (loose chunks folded into the
/// same monolithic `PackWriter` pack as loose objects, no JSONL manifest).
fn repack_chunks_cloud_enabled() -> bool {
    std::env::var("MEDIAGIT_REPACK_CHUNKS")
        .ok()
        .and_then(|v| v.parse::<u8>().ok())
        .map(|v| v != 0)
        .unwrap_or(true)
}

// ST-4: the caps used by chunk repacking are the same ones `PackBuilder`
// (mediagit-protocol, F1-F11 push path) uses, so a repacked repo's packs look
// like ones produced by a normal push. Both now read the single clamped
// definition in `crate::pack` rather than parsing the env var separately.
use crate::pack::{
    pack_bytes_cap as repack_pack_bytes_cap, pack_chunks_cap as repack_pack_chunks_cap,
};
use mediagit_compression::EncryptionKey;

/// Whether this process holds an at-rest encryption key (DC-7).
fn process_key_is_set() -> bool {
    mediagit_compression::process_key().is_some()
}

/// A `SmartCompressor` whenever the process is keyed, otherwise `None`.
///
/// At-rest encryption is applied by `SmartCompressor`, and only on the
/// compression path. An `ObjectDatabase` built *without* one therefore writes
/// **plaintext** — silently, into a repository whose owner ran
/// `mediagit key init`. The process-global key (DC-7 D3) makes every
/// `SmartCompressor::new()` site pick the key up automatically, but it can do
/// nothing about a database that has no smart compressor at all.
///
/// No production call site builds one today — every one uses
/// `with_smart_compression` or `with_optimizations` — so this closes a latent
/// trap rather than a live bug. It is guarded here instead of left to
/// convention because the "ODB bypass" class (a path that skips the shared
/// entry point) has recurred six-plus times in this codebase, and this
/// instance would fail in the worst possible direction: quietly, and only for
/// the users who explicitly asked for encryption.
///
/// With no key set — the norm — this returns `None` and every constructor
/// behaves exactly as before, so unencrypted output stays byte-identical.
fn sealing_compressor_if_keyed() -> Option<Arc<SmartCompressor>> {
    process_key_is_set().then(|| Arc::new(SmartCompressor::new()))
}

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
                .max_capacity(super::cache_max_bytes())
                .weigher(|_key: &Oid, value: &Arc<Vec<u8>>| -> u32 {
                    value.len().try_into().unwrap_or(u32::MAX)
                })
                .build(),
            metrics: Arc::new(RwLock::new(OdbMetrics::new())),
            compressor: Arc::new(ZlibCompressor::default_level()),
            compression_enabled: true,
            smart_compressor: sealing_compressor_if_keyed(),
            chunk_strategy: None,
            cdc_seed: 0,
            delta_enabled: true, // ✅ CRITICAL FIX: Enable delta compression by default for storage savings
            similarity_detector: Arc::new(RwLock::new(crate::similarity::SimilarityDetector::new(
                crate::similarity::MAX_SIMILARITY_CANDIDATES,
            ))),
            base_chunk_cache: build_base_chunk_cache(),
            delta_written_pairs: Arc::new(Mutex::new(crate::odb::DeltaGraph::default())),
            pack_membership: Arc::new(RwLock::new(None)),
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
                .max_capacity(super::cache_max_bytes())
                .weigher(|_key: &Oid, value: &Arc<Vec<u8>>| -> u32 {
                    value.len().try_into().unwrap_or(u32::MAX)
                })
                .build(),
            metrics: Arc::new(RwLock::new(OdbMetrics::new())),
            compressor,
            // A caller-supplied `false` cannot be honoured while a process key
            // is set: see `sealing_compressor_if_keyed`, sealing only happens
            // through the compression path.
            compression_enabled: compression_enabled || process_key_is_set(),
            smart_compressor: sealing_compressor_if_keyed(),
            chunk_strategy: None,
            cdc_seed: 0,
            delta_enabled: true, // ✅ Enable delta compression for storage savings
            similarity_detector: Arc::new(RwLock::new(crate::similarity::SimilarityDetector::new(
                crate::similarity::MAX_SIMILARITY_CANDIDATES,
            ))),
            base_chunk_cache: build_base_chunk_cache(),
            delta_written_pairs: Arc::new(Mutex::new(crate::odb::DeltaGraph::default())),
            pack_membership: Arc::new(RwLock::new(None)),
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
                .max_capacity(super::cache_max_bytes())
                .weigher(|_key: &Oid, value: &Arc<Vec<u8>>| -> u32 {
                    value.len().try_into().unwrap_or(u32::MAX)
                })
                .build(),
            metrics: Arc::new(RwLock::new(OdbMetrics::new())),
            compressor: Arc::new(ZlibCompressor::default_level()),
            compression_enabled: true,
            smart_compressor: Some(Arc::new(SmartCompressor::new())),
            chunk_strategy: None,
            cdc_seed: 0,
            delta_enabled: true, // ✅ CRITICAL FIX: Enable delta compression for 70-90% storage savings
            similarity_detector: Arc::new(RwLock::new(crate::similarity::SimilarityDetector::new(
                crate::similarity::MAX_SIMILARITY_CANDIDATES,
            ))),
            base_chunk_cache: build_base_chunk_cache(),
            delta_written_pairs: Arc::new(Mutex::new(crate::odb::DeltaGraph::default())),
            pack_membership: Arc::new(RwLock::new(None)),
        }
    }

    /// Seal every object this database writes under `key`, and open what it
    /// reads back with it.
    ///
    /// This is the server's only way in. The process-global key
    /// (`crate::process_key`) is bound to a single repo root and is therefore
    /// structurally unusable in a process serving many repositories, so the
    /// server carries the key per repo and hands it here instead.
    ///
    /// `None` is a no-op, which is what keeps the unencrypted path byte-for-byte
    /// what it was: an unkeyed compressor writes exactly what it wrote before
    /// DC-7 existed.
    pub fn with_at_rest_key(mut self, key: Option<EncryptionKey>) -> Self {
        if let Some(key) = key {
            self.smart_compressor = Some(Arc::new(SmartCompressor::new().with_key(key)));
            self.compression_enabled = true;
        }
        self
    }

    /// Does this database seal what it writes?
    ///
    /// Used by the server to catch a cached database whose key state no longer
    /// matches the repository on disk — a mismatch there means either plaintext
    /// written to an encrypted repo or unreadable objects, so it is an error
    /// rather than something to silently correct.
    pub fn is_at_rest_encrypted(&self) -> bool {
        self.smart_compressor
            .as_ref()
            .is_some_and(|c| c.is_encrypted())
    }

    /// Create ObjectDatabase with full optimization features
    pub fn with_optimizations(
        storage: Arc<dyn StorageBackend>,
        cache_capacity: u64,
        chunk_strategy: Option<ChunkStrategy>,
        delta_enabled: bool,
        cdc_seed: u64,
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
                .max_capacity(super::cache_max_bytes())
                .weigher(|_key: &Oid, value: &Arc<Vec<u8>>| -> u32 {
                    value.len().try_into().unwrap_or(u32::MAX)
                })
                .build(),
            metrics: Arc::new(RwLock::new(OdbMetrics::new())),
            compressor: Arc::new(ZlibCompressor::default_level()),
            compression_enabled: true,
            smart_compressor: Some(Arc::new(SmartCompressor::new())),
            chunk_strategy,
            cdc_seed,
            delta_enabled,
            similarity_detector: Arc::new(RwLock::new(crate::similarity::SimilarityDetector::new(
                crate::similarity::MAX_SIMILARITY_CANDIDATES,
            ))),
            base_chunk_cache: build_base_chunk_cache(),
            delta_written_pairs: Arc::new(Mutex::new(crate::odb::DeltaGraph::default())),
            pack_membership: Arc::new(RwLock::new(None)),
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
                .max_capacity(super::cache_max_bytes())
                .weigher(|_key: &Oid, value: &Arc<Vec<u8>>| -> u32 {
                    value.len().try_into().unwrap_or(u32::MAX)
                })
                .build(),
            metrics: Arc::new(RwLock::new(OdbMetrics::new())),
            compressor: Arc::new(ZlibCompressor::default_level()),
            // `without_compression` stores raw bytes, which for a keyed
            // process would mean storing PLAINTEXT. Encryption outranks the
            // caller's compression preference.
            compression_enabled: process_key_is_set(),
            smart_compressor: sealing_compressor_if_keyed(),
            chunk_strategy: None,
            cdc_seed: 0,
            delta_enabled: false,
            similarity_detector: Arc::new(RwLock::new(crate::similarity::SimilarityDetector::new(
                crate::similarity::MAX_SIMILARITY_CANDIDATES,
            ))),
            base_chunk_cache: build_base_chunk_cache(),
            delta_written_pairs: Arc::new(Mutex::new(crate::odb::DeltaGraph::default())),
            pack_membership: Arc::new(RwLock::new(None)),
        }
    }

    /// Read a reachability bitmap (`bitmaps/<oid>.bitmap`) by its storage key.
    ///
    /// Raw passthrough: bitmaps live outside the content-addressed object
    /// model (keyed by commit OID, not by a hash of their own bytes), so
    /// they don't go through `exists()`/pack-membership like objects do.
    pub async fn get_bitmap(&self, key: &str) -> anyhow::Result<Vec<u8>> {
        self.storage.get(key).await
    }

    /// Store a reachability bitmap at the given storage key.
    pub async fn put_bitmap(&self, key: &str, data: &[u8]) -> anyhow::Result<()> {
        self.storage.put(key, data).await
    }

    /// Delete a reachability bitmap at the given storage key.
    pub async fn delete_bitmap(&self, key: &str) -> anyhow::Result<()> {
        self.storage.delete(key).await
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
        if self.storage.exists(&manifest_key).await? {
            return Ok(true);
        }

        // Also check pack membership: `gc --repack` bundles loose objects
        // into a pack and (with remove_loose) deletes the loose copy, so a
        // miss above doesn't mean "we don't have it" — it may live only in
        // a pack now. Mirrors `chunk_exists()`'s union semantics.
        self.ensure_pack_membership_loaded().await?;
        let guard = self.pack_membership.read().await;
        Ok(guard.as_ref().is_some_and(|set| set.contains(oid)))
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

    /// Delete a loose object's storage entry and drop it from the cache.
    ///
    /// Repair-path primitive (QA-013): `write()` dedups on `exists()`, so a
    /// corrupt-but-present object can only be replaced by deleting it first.
    /// Only removes the loose `<hex>` key — pack-resident objects are not
    /// touched (pack repair is `gc --repack` territory).
    pub async fn delete_object(&self, oid: &Oid) -> anyhow::Result<()> {
        self.cache.invalidate(oid).await;
        let key = oid.to_hex();
        if self.storage.exists(&key).await? {
            self.storage.delete(&key).await?;
        }
        Ok(())
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

        // List all loose objects (commits/trees/tags/small blobs, bare-hex
        // keys) and loose chunks (content-addressed pieces of chunked/large
        // files under "chunks/"). Most repo bytes in a media-VCS live in
        // chunks, not bare-hex objects — see list_loose_chunks() docs.
        let loose_objects = self.list_loose_objects().await?;
        let loose_chunks = self.list_loose_chunks().await?;
        stats.loose_objects_found = loose_objects.len() + loose_chunks.len();

        if loose_objects.is_empty() && loose_chunks.is_empty() {
            info!("No loose objects to repack");
            return Ok(stats);
        }

        // max_objects caps the combined total; whole objects are prioritized,
        // chunks fill the remainder.
        let (objects_to_pack, chunks_to_pack): (&[Oid], &[Oid]) = if max_objects == 0 {
            (&loose_objects[..], &loose_chunks[..])
        } else if loose_objects.len() >= max_objects {
            (&loose_objects[..max_objects], &[])
        } else {
            let remaining = max_objects - loose_objects.len();
            (
                &loose_objects[..],
                &loose_chunks[..remaining.min(loose_chunks.len())],
            )
        };

        info!(
            total_loose_objects = loose_objects.len(),
            total_loose_chunks = loose_chunks.len(),
            packing_objects = objects_to_pack.len(),
            packing_chunks = chunks_to_pack.len(),
            "Found loose objects"
        );

        // I9: when enabled (default), loose chunks are consolidated into
        // Track-F-style cloud packs (see `repack_chunks_into_cloud_packs`)
        // instead of the monolithic `PackWriter` pack below. `legacy_chunks_to_pack`
        // is empty in that case so the loop just below packs nothing for chunks,
        // leaving `MEDIAGIT_REPACK_CHUNKS=0` as an exact reproduction of the
        // pre-I9 combined-pack behavior.
        let chunks_cloud_mode = repack_chunks_cloud_enabled();
        let legacy_chunks_to_pack: &[Oid] = if chunks_cloud_mode {
            &[]
        } else {
            chunks_to_pack
        };

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

                                    // DC-7: the pack copy REPLACES the loose
                                    // sealed object (deleted below when
                                    // `remove_loose`), so it has to carry the
                                    // same protection. The regular-object
                                    // branch gets this from `compress_typed`;
                                    // a delta never reaches the compressor, so
                                    // without this `gc --repack` would quietly
                                    // and permanently un-encrypt it.
                                    // `PackReader::read_delta_object` opens it
                                    // symmetrically.
                                    let sealed_delta =
                                        mediagit_compression::seal_at_rest(&delta_data)?;
                                    pack_writer.add_delta_object(*oid, base_oid, &sealed_delta);
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

        // Add loose chunks to the pack (legacy path only — empty when
        // `chunks_cloud_mode` is active; see prelude above). Chunks are
        // already compressed on disk and already delta-deduplicated against
        // sibling chunks (see chunk-deltas/), so unlike whole objects we
        // store the existing compressed bytes as-is instead of re-running
        // whole-object delta detection against them.
        let mut legacy_packed_chunk_oids = Vec::new();
        for chunk_id in legacy_chunks_to_pack {
            match self.get_compressed_chunk(chunk_id).await {
                Ok(compressed) => {
                    total_original_size += compressed.len() as u64;
                    pack_writer.add_object(*chunk_id, ObjectType::Blob, &compressed);
                    legacy_packed_chunk_oids.push(*chunk_id);
                }
                Err(e) => {
                    warn!(chunk_id = %chunk_id, error = %e, "Failed to read chunk for packing");
                }
            }
        }

        let legacy_total_packed = packed_oids.len() + legacy_packed_chunk_oids.len();

        if legacy_total_packed > 0 {
            // Finalize pack
            let pack_data = pack_writer.finalize();
            stats.pack_size += pack_data.len() as u64;
            stats.bytes_saved += total_original_size.saturating_sub(pack_data.len() as u64);

            // Generate pack file name with timestamp
            let pack_id = format!("pack-{}", chrono::Utc::now().timestamp());
            let pack_key = format!("packs/{}.pack", pack_id);

            // Store pack file
            self.storage.put(&pack_key, &pack_data).await?;

            // Extend the pack-membership set in place if it's already loaded, so
            // chunk_exists() sees these OIDs immediately without a full pack
            // rescan. If it hasn't been loaded yet, leave it as None — the next
            // chunk_exists() call will lazily build it from all packs including
            // this new one.
            {
                let mut guard = self.pack_membership.write().await;
                if let Some(set) = guard.as_mut() {
                    set.extend(packed_oids.iter().copied());
                    set.extend(legacy_packed_chunk_oids.iter().copied());
                }
            }

            info!(
                pack_id,
                size = pack_data.len(),
                objects = legacy_total_packed,
                deltas = stats.delta_objects,
                "Pack file created"
            );

            // Remove loose objects if requested
            if remove_loose {
                let mut removed = 0;
                for oid in &packed_oids {
                    // Use oid.to_hex() for consistency - LocalBackend handles path sharding
                    let object_key = oid.to_hex();
                    match self.storage.delete(&object_key).await {
                        Err(e) => {
                            warn!(oid = %oid, error = %e, "Failed to remove loose object");
                        }
                        _ => {
                            removed += 1;
                        }
                    }
                }
                for chunk_id in &legacy_packed_chunk_oids {
                    let chunk_key = format!("chunks/{}", chunk_id.to_hex());
                    match self.storage.delete(&chunk_key).await {
                        Err(e) => {
                            warn!(chunk_id = %chunk_id, error = %e, "Failed to remove loose chunk");
                        }
                        _ => {
                            removed += 1;
                        }
                    }
                }
                stats.loose_objects_removed += removed;
                info!(removed, "Removed loose objects");
            }
        }

        // I9: consolidate the remainder of `chunks_to_pack` into cloud packs
        // (no-op when `chunks_cloud_mode` is false — `chunks_to_pack` was
        // already fully drained into `legacy_packed_chunk_oids` above).
        let cloud_packed_chunk_oids = if chunks_cloud_mode && !chunks_to_pack.is_empty() {
            self.repack_chunks_into_cloud_packs(chunks_to_pack, remove_loose, &mut stats)
                .await?
        } else {
            Vec::new()
        };

        stats.objects_packed =
            packed_oids.len() + legacy_packed_chunk_oids.len() + cloud_packed_chunk_oids.len();

        if stats.objects_packed == 0 {
            info!("No objects were successfully packed");
            return Ok(stats);
        }

        info!(
            packed = stats.objects_packed,
            pack_size = stats.pack_size,
            saved = stats.bytes_saved,
            "Repack complete"
        );

        Ok(stats)
    }

    /// Consolidate loose chunks into Track-F-style cloud packs during
    /// `gc --repack` (GA I9, gated by `MEDIAGIT_REPACK_CHUNKS`).
    ///
    /// Uses the same envelope/caps as the push-time `PackBuilder`
    /// (mediagit-protocol): `StreamingPackWriter` with `PackKind::CloudObject`,
    /// capped at `MEDIAGIT_PACK_BYTES` (default 64 MiB) / `MEDIAGIT_PACK_CHUNKS`
    /// (default 1024) per pack, plus a JSONL chunk-index manifest at
    /// `packs/<shard>/<pack_oid>.jsonl` — the exact schema and path
    /// `complete_pack`/`load_jsonl_index` (mediagit-server) expect, so a
    /// repacked repo's chunks are servable the same way freshly-pushed ones
    /// are.
    ///
    /// Abort-safety per pack (see `seal_chunk_cloud_pack`): store pack bytes
    /// -> persist JSONL index -> extend in-memory `pack_membership` -> delete
    /// loose chunks, in that order. A crash before the JSONL write leaves the
    /// loose chunks untouched; a crash after leaves the pack fully readable
    /// via `read_from_packs`/`get_chunk` regardless of whether the loose
    /// copies were removed yet.
    ///
    /// Memory bound: one pack in flight, streamed to a temp file on disk
    /// (never buffered whole in RAM — `finalize_cloud` re-reads it in 64 KiB
    /// chunks to checksum) plus one loose chunk being read/decompressed at a
    /// time.
    async fn repack_chunks_into_cloud_packs(
        &self,
        chunk_ids: &[Oid],
        remove_loose: bool,
        stats: &mut RepackStats,
    ) -> anyhow::Result<Vec<Oid>> {
        use crate::pack::PackKind;
        use crate::streaming_pack::StreamingPackWriter;

        let bytes_cap = repack_pack_bytes_cap();
        let chunks_cap = repack_pack_chunks_cap();
        let temp_dir = std::env::temp_dir();

        let mut packed_chunk_oids: Vec<Oid> = Vec::new();
        let mut writer: Option<StreamingPackWriter<tokio::fs::File>> = None;
        // (chunk_id, compressed_hash_hex, compressed_len) for the pack currently open.
        let mut pending: Vec<(Oid, String, u64)> = Vec::new();
        let mut cur_bytes: u64 = 0;

        for chunk_id in chunk_ids {
            let compressed = match self.get_compressed_chunk(chunk_id).await {
                Ok(c) => c,
                Err(e) => {
                    warn!(chunk_id = %chunk_id, error = %e, "Failed to read chunk for cloud-pack repack");
                    continue;
                }
            };

            if writer.is_none() {
                writer = Some(
                    StreamingPackWriter::new_open_ended(PackKind::CloudObject, &temp_dir).await?,
                );
            }
            let w = writer.as_mut().expect("writer just initialized above");
            w.write_object(*chunk_id, ObjectType::Blob, &compressed)
                .await?;

            let compressed_hash = Oid::hash(&compressed).to_hex();
            cur_bytes += compressed.len() as u64 + 5; // +5 header bytes, matches PackBuilder's accounting
            pending.push((*chunk_id, compressed_hash, compressed.len() as u64));

            if cur_bytes >= bytes_cap || pending.len() as u32 >= chunks_cap {
                let sealed_writer = writer.take().expect("writer present when cap hit");
                self.seal_chunk_cloud_pack(
                    sealed_writer,
                    &mut pending,
                    remove_loose,
                    stats,
                    &mut packed_chunk_oids,
                )
                .await?;
                cur_bytes = 0;
            }
        }

        if let Some(w) = writer.take()
            && !pending.is_empty()
        {
            self.seal_chunk_cloud_pack(
                w,
                &mut pending,
                remove_loose,
                stats,
                &mut packed_chunk_oids,
            )
            .await?;
        }

        Ok(packed_chunk_oids)
    }

    /// Finalize one cloud pack of loose chunks and durably register it.
    /// See `repack_chunks_into_cloud_packs` for the abort-safety ordering.
    async fn seal_chunk_cloud_pack(
        &self,
        writer: crate::streaming_pack::StreamingPackWriter<tokio::fs::File>,
        pending: &mut Vec<(Oid, String, u64)>,
        remove_loose: bool,
        stats: &mut RepackStats,
        packed_chunk_oids: &mut Vec<Oid>,
    ) -> anyhow::Result<()> {
        let result = writer.finalize_cloud().await?;
        let pack_oid_hex = hex::encode(&result.pack_oid);

        // 1. Store the pack bytes first — a crash here leaves at worst an
        //    orphaned temp pack object, never a lost loose chunk (nothing
        //    below this point has run yet).
        let pack_key = format!("packs/{}", pack_oid_hex);
        self.storage.put_file(&pack_key, &result.temp_path).await?;
        let _ = tokio::fs::remove_file(&result.temp_path).await;

        // 2. Persist the JSONL chunk index — durable via `StorageBackend::put`
        //    (tmp+rename on `LocalBackend`; a single atomic PUT on cloud
        //    backends). Field names match `complete_pack`'s `PackIndexLine`
        //    exactly so `load_jsonl_index` (mediagit-server) can pick this up.
        //
        //    Key deliberately has NO manual shard component: `LocalBackend::
        //    object_path()` special-cases a key whose immediate parent is
        //    literally "packs" (`dir_components.last() == Some("packs")`) into
        //    a single shard level derived from the basename itself — the same
        //    rule that shards the pack object's own key just above. Adding a
        //    shard component here defeats that special case (parent becomes
        //    "<shard>" instead of "packs") and falls through to the generic
        //    two-level object shard instead, double-nesting the manifest.
        let pending_meta: std::collections::HashMap<Oid, String> = pending
            .iter()
            .map(|(id, hash, _)| (*id, hash.clone()))
            .collect();

        let mut jsonl = String::new();
        for loc in &result.index {
            let compressed_hash = pending_meta.get(&loc.chunk_oid).map(|h| h.as_str());
            let line = serde_json::json!({
                "chunk_oid": loc.chunk_oid.to_hex(),
                "pack_oid": pack_oid_hex,
                "offset": loc.offset,
                "length": loc.length,
                "compressed_hash": compressed_hash,
            });
            jsonl.push_str(&line.to_string());
            jsonl.push('\n');
        }
        let manifest_key = format!("packs/{}.jsonl", pack_oid_hex);
        self.storage.put(&manifest_key, jsonl.as_bytes()).await?;

        // 3. Update in-memory pack membership before touching loose copies,
        //    so a reader racing this repack never sees a chunk as "gone"
        //    (loose deleted) without it also being visible in the pack index.
        {
            let mut guard = self.pack_membership.write().await;
            if let Some(set) = guard.as_mut() {
                set.extend(pending.iter().map(|(id, _, _)| *id));
            }
        }

        let original_bytes: u64 = pending.iter().map(|(_, _, len)| len).sum();
        stats.pack_size += result.byte_len;
        stats.bytes_saved += original_bytes.saturating_sub(result.byte_len);

        // 4. Only now remove the loose copies — everything needed to read
        //    these chunks back out of the pack is already durable.
        let mut removed = 0usize;
        if remove_loose {
            for (chunk_id, _, _) in pending.iter() {
                let chunk_key = format!("chunks/{}", chunk_id.to_hex());
                match self.storage.delete(&chunk_key).await {
                    Err(e) => {
                        warn!(chunk_id = %chunk_id, error = %e, "Failed to remove loose chunk after cloud-pack repack");
                    }
                    _ => {
                        removed += 1;
                    }
                }
            }
            stats.loose_objects_removed += removed;
        }

        info!(
            pack_oid = pack_oid_hex,
            chunks = pending.len(),
            pack_size = result.byte_len,
            loose_removed = removed,
            "Cloud pack created from loose chunks (repack)"
        );

        packed_chunk_oids.extend(pending.iter().map(|(id, _, _)| *id));
        pending.clear();

        Ok(())
    }

    /// Resolve an abbreviated OID prefix to a full OID.
    ///
    /// Scans loose objects and pack-embedded objects for keys matching the
    /// given hex prefix. Returns an error if zero or more than one object
    /// matches.
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
            if key.starts_with(abbrev)
                && let Ok(oid_bytes) = hex::decode(&key)
                && oid_bytes.len() == 32
            {
                let mut bytes = [0u8; 32];
                bytes.copy_from_slice(&oid_bytes);
                matches.push(Oid::from(bytes));
            }
        }

        // Also match against pack-embedded objects (post-`gc --repack`, the
        // loose copy may be gone, so the storage prefix-scan above misses
        // them — see `exists()`'s pack-membership union for the same gap).
        self.ensure_pack_membership_loaded().await?;
        {
            let guard = self.pack_membership.read().await;
            if let Some(set) = guard.as_ref() {
                for oid in set.iter() {
                    if oid.to_hex().starts_with(abbrev) && !matches.contains(oid) {
                        matches.push(*oid);
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
            if let Ok(oid_bytes) = hex::decode(&key)
                && oid_bytes.len() == 32
            {
                let mut bytes = [0u8; 32];
                bytes.copy_from_slice(&oid_bytes);
                oids.push(Oid::from(bytes));
            }
        }

        debug!(count = oids.len(), "Listed loose objects");
        Ok(oids)
    }

    /// List all loose chunks in the object database
    ///
    /// Scans the `chunks/` namespace and returns OIDs of all loose chunk
    /// objects — the content-addressed pieces that large/chunked files are
    /// split into. These live under `chunks/<hex>` (not a bare-hex key like
    /// commits/trees/tags/small blobs), so `list_loose_objects()` never sees
    /// them: its `hex::decode()` on a `"chunks/<hex>"` key fails because of
    /// the slash, and the entry is silently dropped. Without this, `repack`
    /// only ever bundled the small metadata objects and reported "0 objects
    /// packed" on repos where nearly all bytes live in chunks.
    async fn list_loose_chunks(&self) -> anyhow::Result<Vec<Oid>> {
        let mut oids = Vec::new();

        let keys = self.storage.list_objects("chunks/").await?;
        for key in keys {
            if let Some(hex) = key.strip_prefix("chunks/")
                && let Ok(oid) = Oid::from_hex(hex)
            {
                oids.push(oid);
            }
        }

        debug!(count = oids.len(), "Listed loose chunks");
        Ok(oids)
    }
}
