// SPDX-License-Identifier: BUSL-1.1
// Copyright (C) 2025-2026 Aswin Krishnamoorthy

use super::*;

/// Cap on how many chunks `seed_similarity_from_manifest` will sample from a
/// previous manifest. Without a bound, seeding is O(prior chunk count ×
/// delta-chain depth) — every chunk of the previous version's manifest gets
/// a full `get_chunk` reconstruction. Override via
/// `MEDIAGIT_SIMILARITY_SEED_MAX_CHUNKS`.
fn similarity_seed_max_chunks() -> usize {
    match std::env::var("MEDIAGIT_SIMILARITY_SEED_MAX_CHUNKS") {
        Ok(v) => v.parse::<usize>().unwrap_or_else(|_| {
            warn!(
                "MEDIAGIT_SIMILARITY_SEED_MAX_CHUNKS='{}' is not a valid usize, using default 256",
                v
            );
            256
        }),
        Err(_) => 256,
    }
}

/// DC-7: a chunk manifest as it goes to **local** storage.
///
/// A manifest names the file and lists the plaintext hash of every one of its
/// chunks, so leaving it in the clear hands an attacker the filename and a
/// confirmation oracle for content they can guess — most of what at-rest
/// encryption was bought to prevent.
///
/// Sealing lives here, at the storage boundary, and deliberately **not** in
/// `ChunkManifest::to_bytes`: the exact same bytes travel over the wire
/// (`PUT /manifests/{oid}`) and that format must not move. Storage writers
/// seal, storage readers open, everything else is untouched.
///
/// Keyed from the database's own compressor rather than the process-global
/// key, because the server holds a key per repository and has no process key
/// at all -- with the global, a server writing a manifest for a keyed
/// repository wrote it in the clear.
fn seal_manifest<'a>(
    compressor: Option<&SmartCompressor>,
    bytes: &'a [u8],
) -> anyhow::Result<std::borrow::Cow<'a, [u8]>> {
    match compressor {
        Some(c) => c
            .seal_bytes(bytes.to_vec())
            .map(std::borrow::Cow::Owned)
            .map_err(|e| anyhow::anyhow!("Failed to seal chunk manifest: {e}")),
        None => mediagit_compression::seal_at_rest(bytes)
            .map_err(|e| anyhow::anyhow!("Failed to seal chunk manifest: {e}")),
    }
}

/// Seal bytes that arrived from a **remote**, for storage in this database.
///
/// Two rules, both learned the hard way:
///
/// 1. **Never wrap twice.** Since DC-7/D4 the server holds the repository key
///    and hands back exactly what the client uploaded, so on an encrypted repo
///    these arrive already sealed. A second envelope unseals to a first one,
///    which the codec sniffer calls uncompressed and returns as content --
///    caught only by the chunk's hash check, and reported as corruption.
/// 2. **Key from the database, not the process.** `seal_at_rest` reads the
///    process-global key, which the server does not have; a per-repo-keyed
///    database would silently store plaintext through it.
fn seal_from_wire<'a>(
    compressor: Option<&SmartCompressor>,
    data: &'a [u8],
) -> anyhow::Result<std::borrow::Cow<'a, [u8]>> {
    if mediagit_compression::is_sealed(data) {
        return Ok(std::borrow::Cow::Borrowed(data));
    }
    match compressor {
        Some(c) => Ok(std::borrow::Cow::Owned(c.seal_bytes(data.to_vec())?)),
        None => Ok(mediagit_compression::seal_at_rest(data)?),
    }
}

/// Inverse of [`seal_manifest`]. Every local reader of `manifests/<oid>` must
/// come through here — a raw `storage.get` is the "ODB bypass" defect this
/// codebase has shipped six times, and on a keyed repo it now returns
/// ciphertext that `from_bytes` will misparse.
fn open_manifest<'a>(
    compressor: Option<&SmartCompressor>,
    bytes: &'a [u8],
) -> anyhow::Result<std::borrow::Cow<'a, [u8]>> {
    match compressor {
        Some(c) => c
            .open_bytes(bytes)
            .map_err(|e| anyhow::anyhow!("Failed to open chunk manifest: {e}")),
        None => mediagit_compression::open_at_rest(bytes)
            .map_err(|e| anyhow::anyhow!("Failed to open chunk manifest: {e}")),
    }
}

impl ObjectDatabase {
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
            &chunk_meta,
            min_similarity,
            size_ratio_threshold,
        );
        drop(detector); // Release read lock

        if let Some((nominated_base, score)) = similar {
            // One walk answers both questions: would this close a cycle, and
            // how deep is the nominated base already? Deriving depth from a
            // second traversal would double the small-file I/O per chunk.
            //
            // Cycles: without this, parallel adds of similar chunks can produce
            // A→B and B→A on disk, which makes both unreadable.
            //
            // Depth: this path loads its base with `get_chunk`, which happily
            // reconstructs *through* an existing chain — so without a depth
            // guard each similar chunk adds a hop and the chain grows without
            // bound until `get_chunk` refuses it and the data is unreadable.
            let base_id = if nominated_base == chunk.id {
                debug!(
                    chunk_id = %chunk.id,
                    "Refusing chunk delta — self-loop, falling back to full chunk"
                );
                let mut detector = self.similarity_detector.write().await;
                detector.add_object(chunk_meta);
                return Ok(false);
            } else {
                let (resolved, observed) =
                    resolve_delta_base_observing(&*self.storage, nominated_base, chunk.id).await;
                // Memoize the chain this walk just read so the guard below can
                // re-check it without storage I/O inside the lock.
                self.delta_written_pairs
                    .lock()
                    .await
                    .merge_observed(observed);
                match resolved {
                    Some(id) => id,
                    None => {
                        debug!(
                            chunk_id = %chunk.id,
                            base_id = %nominated_base,
                            "Refusing chunk delta — cycle or unresolvable chain, falling back to full chunk"
                        );
                        let mut detector = self.similarity_detector.write().await;
                        detector.add_object(chunk_meta);
                        return Ok(false);
                    }
                }
            };

            // Try to load base chunk and create delta
            if let Ok(base_data) = self.get_chunk(&base_id).await {
                // Create delta
                let delta = DeltaEncoder::encode(&base_data, &chunk.data);
                let delta_bytes = delta.to_bytes();

                // Only use delta if beneficial (codec-aware threshold)
                let delta_ratio = delta_bytes.len() as f64 / chunk.data.len() as f64;
                let threshold = delta_ratio_threshold(chunk.codec_hint, chunk.chunk_type);
                if delta_ratio < threshold {
                    // Store chunk delta
                    let delta_key = format!("chunk-deltas/{}", chunk.id.to_hex());
                    let compressed_delta = if let Some(smart_comp) = &self.smart_compressor {
                        smart_comp
                            .compress_typed(&delta_bytes, CompressionObjectType::Unknown)
                            .map_err(|e| anyhow::anyhow!("Failed to compress chunk delta: {}", e))?
                    } else {
                        self.compressor
                            .compress(&delta_bytes)
                            .map_err(|e| anyhow::anyhow!("Failed to compress chunk delta: {}", e))?
                    };

                    // TOCTOU guard FIRST: check+register before any I/O.  If the
                    // reverse pair (base_id, chunk.id) is already committed, skip all
                    // writes — no orphaned binary on disk.
                    //
                    // The re-check is serialized with the *registration*, not with
                    // the meta write: the walk at the top of this fn races with
                    // concurrent writers (three parallel writes can form A→B→C→A
                    // with every pre-walk passing, because no meta is on disk yet).
                    // Registering under the lock, strictly before the meta reaches
                    // disk, means whichever write closes a loop sees the completed
                    // chain in memory and refuses — see `DeltaGraph`.
                    if !commit_delta_pair(
                        &*self.storage,
                        &self.delta_written_pairs,
                        chunk.id,
                        base_id,
                    )
                    .await
                    {
                        debug!(
                            chunk_id = %chunk.id,
                            base_id = %base_id,
                            "Refusing chunk delta at commit — concurrent writes would close a cycle or exceed max depth"
                        );
                        let mut detector = self.similarity_detector.write().await;
                        detector.add_object(chunk_meta);
                        return Ok(false);
                    }

                    // Write .meta FIRST — it is the durability anchor for all existence
                    // probes (odb.rs:exists, check_chunk_deltas_exist). Writing meta before
                    // the binary means a crash between the two writes leaves an unreachable
                    // binary (collected by `gc`) rather than a binary with no routing sidecar
                    // (which would cause clone 404 when the probe correctly returns the id
                    // but the download handler sees no meta and returns NOT_FOUND).
                    let meta_key = format!("chunk-deltas/{}.meta", chunk.id.to_hex());
                    let meta_data = format!("base:{}", base_id.to_hex());
                    if let Err(e) = self.storage.put(&meta_key, meta_data.as_bytes()).await
                        && !self.storage.exists(&meta_key).await.unwrap_or(false)
                    {
                        // Drop the registration too, or the guard would refuse a
                        // legitimate delta for this chunk later in the same run.
                        rollback_delta_pair(&self.delta_written_pairs, chunk.id, base_id).await;
                        return Err(anyhow::anyhow!("Failed to store chunk delta meta: {}", e));
                    }

                    if let Err(e) = self.storage.put(&delta_key, &compressed_delta).await {
                        // Remove the .meta we already committed so the chunk is not
                        // permanently misrouted. If the delete also fails, gc collects the
                        // orphaned sidecar on a later run.
                        // Roll back ONLY if the sidecar is really gone. The in-memory
                        // graph must stay a superset of the on-disk edges: the depth
                        // guard reads it, so an edge still on disk but missing from
                        // memory makes the guard undercount and admit a chain past
                        // MAX_DELTA_DEPTH. Keeping an edge whose binary never landed is
                        // the safe direction to be wrong in - it only makes the guard
                        // refuse a delta it could have taken.
                        match self.storage.delete(&meta_key).await {
                            Ok(()) => {
                                rollback_delta_pair(&self.delta_written_pairs, chunk.id, base_id)
                                    .await;
                            }
                            Err(del_err) => warn!(
                                chunk_id = %chunk.id,
                                base_id = %base_id,
                                error = %del_err,
                                "failed to remove delta routing sidecar after a failed delta \
                                 write; keeping the in-memory edge so the depth guard stays \
                                 conservative"
                            ),
                        }
                        return Err(anyhow::anyhow!("Failed to store chunk delta binary: {}", e));
                    }
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
    /// The OID (BLAKE3 hash) of the original data
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
        const MIN_CHUNK_SIZE: usize = 1024 * 1024; // 1MB
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
                    | CompressionObjectType::Ogg // Note: Video formats (Mp4, Mov, Avi, Webm) are NOT skipped
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
        let exists = self.storage.exists(&key).await.map_err(|e| {
            anyhow::anyhow!("Failed to check if chunked object {} exists: {}", key, e)
        })?;
        if exists {
            debug!(oid = %oid, "Chunked object already exists (deduplicated)");
            let mut metrics = self.metrics.write().await;
            metrics.record_write(data.len() as u64, false);
            return Ok(oid);
        }

        // Create chunker with configured strategy
        let chunker = ContentChunker::with_seed(self.chunk_strategy.unwrap(), self.cdc_seed);

        // Chunk the data
        let chunks = chunker.chunk(data, filename).await.map_err(|e| {
            anyhow::anyhow!(
                "Failed to chunk data for {} (size: {} bytes): {}",
                filename,
                data.len(),
                e
            )
        })?;

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
            let chunk_exists = self.storage.exists(&chunk_key).await.map_err(|e| {
                anyhow::anyhow!("Failed to check chunk existence for {}: {}", chunk_key, e)
            })?;
            let delta_exists = self
                .storage
                .exists(&format!("chunk-deltas/{}.meta", chunk.id.to_hex()))
                .await
                .unwrap_or(false);

            if !chunk_exists && !delta_exists {
                // Try delta encoding first via shared helper
                let stored_as_delta = self
                    .try_store_chunk_as_delta(
                        chunk,
                        Some(filename),
                        min_similarity,
                        size_ratio_threshold,
                    )
                    .await?;

                // Store full chunk if delta wasn't beneficial
                if !stored_as_delta {
                    // For demuxed container chunks with a known codec, use per-chunk
                    // codec-aware compression (matching the streaming/parallel path).
                    let codec_hint = to_chunk_codec_hint(chunk.codec_hint, chunk.chunk_type);
                    let compressed = if let Some(smart_comp) = &self.smart_compressor {
                        // Try codec-aware compression first
                        match smart_comp.compress_by_codec(&chunk.data, codec_hint) {
                            Some(result) => result.map_err(|e| {
                                anyhow::anyhow!(
                                    "Failed to compress chunk {} (codec): {}",
                                    chunk_key,
                                    e
                                )
                            })?,
                            _ => {
                                // Unknown codec → fall back to file-level strategy
                                let chunk_comp_type = if !filename.is_empty() {
                                    CompressionObjectType::from_path(filename)
                                } else {
                                    CompressionObjectType::Unknown
                                };
                                smart_comp
                                    .compress_typed_with_size(&chunk.data, chunk_comp_type)
                                    .map_err(|e| {
                                        anyhow::anyhow!(
                                            "Failed to compress chunk {}: {}",
                                            chunk_key,
                                            e
                                        )
                                    })?
                            }
                        }
                    } else {
                        self.compressor.compress(&chunk.data).map_err(|e| {
                            anyhow::anyhow!("Failed to compress chunk {}: {}", chunk_key, e)
                        })?
                    };

                    self.storage
                        .put(&chunk_key, &compressed)
                        .await
                        .map_err(|e| {
                            anyhow::anyhow!(
                                "Failed to store chunk {} (size: {} bytes): {}",
                                chunk_key,
                                compressed.len(),
                                e
                            )
                        })?;

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
        let manifest =
            crate::chunking::ChunkManifest::from_chunks(chunks, Some(filename.to_string()));

        // Store manifest (use to_hex() for consistent storage paths)
        let manifest_key = format!("manifests/{}", oid.to_hex());
        let manifest_data = manifest.to_bytes().map_err(|e| {
            anyhow::anyhow!("Failed to serialize chunk manifest for {}: {}", oid, e)
        })?;
        let manifest_data = seal_manifest(self.smart_compressor.as_deref(), &manifest_data)?;
        self.storage
            .put(&manifest_key, &manifest_data)
            .await
            .map_err(|e| {
                anyhow::anyhow!(
                    "Failed to store chunk manifest {} (size: {} bytes): {}",
                    manifest_key,
                    manifest_data.len(),
                    e
                )
            })?;

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

        const MIN_CHUNK_SIZE: usize = 1024 * 1024; // 1MB
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
        if self
            .storage
            .exists(&key)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to check object existence: {}", e))?
        {
            let mut metrics = self.metrics.write().await;
            metrics.record_write(data.len() as u64, false);
            return Ok(oid);
        }

        // Check manifest existence too
        if self
            .storage
            .exists(&format!("manifests/{}", key))
            .await
            .unwrap_or(false)
        {
            let mut metrics = self.metrics.write().await;
            metrics.record_write(data.len() as u64, false);
            return Ok(oid);
        }

        // Chunk the data
        let chunker = ContentChunker::with_seed(self.chunk_strategy.unwrap(), self.cdc_seed);
        let chunks = chunker
            .chunk(data, filename)
            .await
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

        // For small chunk counts, sequential is faster (no channel overhead).
        // Threshold lowered to 2: even two chunks benefit from parallel I/O.
        if num_chunks <= 2 {
            return self.write_chunked(obj_type, data, filename).await;
        }

        // --- Parallel pipeline ---
        //
        // Determinism note: similarity query + detector registration happen in
        // the producer, in strict chunk-sequence order. This makes chunk-delta
        // base selection a deterministic function of (prior-commit state, chunk
        // bytes, chunk order) — independent of worker scheduling. Workers then
        // run the expensive encode/compress/store step in parallel against a
        // pre-selected `base_oid_opt`.
        let num_workers = std::env::var("MEDIAGIT_CHUNK_WRITE_CONCURRENCY")
            .ok()
            .and_then(|s| s.parse::<usize>().ok())
            .unwrap_or_else(num_cpus::get)
            .min(num_chunks)
            .max(2);
        let (tx, rx) =
            async_channel::bounded::<(usize, crate::chunking::ContentChunk, Option<Oid>)>(64);

        // Producer: sequential similarity phase, then hand off to workers.
        let producer_similarity_detector = self.similarity_detector.clone();
        let producer_delta_enabled = self.delta_enabled;
        let producer_base_chunk_cache = self.base_chunk_cache.clone();
        let producer = tokio::spawn(async move {
            for (seq_id, chunk) in chunks.into_iter().enumerate() {
                // Replicate the worker's "should attempt delta" predicate.
                let is_high_entropy_codec = matches!(
                    chunk.codec_hint,
                    crate::chunking::CodecHint::H264
                        | crate::chunking::CodecHint::H265
                        | crate::chunking::CodecHint::VP9
                        | crate::chunking::CodecHint::AV1
                        | crate::chunking::CodecHint::AAC
                        | crate::chunking::CodecHint::Opus
                        | crate::chunking::CodecHint::Vorbis
                        | crate::chunking::CodecHint::MP3
                );
                let is_store_video_chunk =
                    if chunk.codec_hint == crate::chunking::CodecHint::Unknown {
                        chunk.chunk_type == crate::chunking::ChunkType::VideoStream
                            && matches!(
                                comp_type,
                                CompressionObjectType::Mp4
                                    | CompressionObjectType::Mov
                                    | CompressionObjectType::Avi
                                    | CompressionObjectType::Mkv
                                    | CompressionObjectType::Webm
                                    | CompressionObjectType::Flv
                                    | CompressionObjectType::Wmv
                                    | CompressionObjectType::Mpg
                                    | CompressionObjectType::Mxf
                            )
                    } else {
                        is_high_entropy_codec
                    };

                let base_oid_opt: Option<Oid> = if producer_delta_enabled
                    && chunk.data.len() >= 4096
                    && !is_store_video_chunk
                {
                    let mut chunk_meta = crate::similarity::ObjectMetadata::new(
                        chunk.id,
                        chunk.data.len(),
                        crate::ObjectType::Blob,
                        None,
                    );
                    chunk_meta.generate_samples(&chunk.data);

                    let detector = producer_similarity_detector.read().await;
                    let similar = detector.find_similar_with_size_ratio(
                        &chunk_meta,
                        min_similarity,
                        size_ratio_threshold,
                    );
                    drop(detector);

                    let base_oid = similar.map(|(oid, _)| oid);
                    // Optimistic marking: we don't know yet if the worker will
                    // succeed at delta encoding (ratio gate may reject it).
                    // Marking `is_delta = true` pessimistically would shrink the
                    // base pool for later chunks.  Keep it `false` so every
                    // registered chunk remains available as a future base.
                    // Shallow depth-2 chains are bounded and were already
                    // possible in the old concurrent pipeline.
                    chunk_meta.is_delta = false;

                    let mut detector = producer_similarity_detector.write().await;
                    detector.add_object(chunk_meta);
                    drop(detector);

                    base_oid
                } else {
                    None
                };

                // Pre-cache the chunk's raw data so that workers looking for
                // this chunk as a delta base can find it even before it has been
                // compressed and stored to the backend.
                producer_base_chunk_cache
                    .insert(chunk.id, Arc::new(chunk.data.clone()))
                    .await;

                if tx.send((seq_id, chunk, base_oid_opt)).await.is_err() {
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
            let base_chunk_cache = self.base_chunk_cache.clone();
            let delta_pairs = self.delta_written_pairs.clone();

            let handle = tokio::spawn(async move {
                let mut results: Vec<(usize, ChunkRef)> = Vec::new();

                while let Ok((seq_id, chunk, base_oid_opt)) = rx.recv().await {
                    let chunk_ref = ChunkRef {
                        id: chunk.id,
                        offset: chunk.offset,
                        size: chunk.size,
                        chunk_type: chunk.chunk_type,
                        codec_hint: chunk.codec_hint,
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

                    // 2. Delta encoding using the base pre-selected by the
                    //    producer (deterministic: same chunk order every run).
                    let mut stored_as_delta = false;
                    if let Some(nominated_base) = base_oid_opt {
                        // Cycle AND depth prevention in one chain walk.
                        //
                        // Cycles: two parallel encoders processing similar chunks
                        // can otherwise produce mutually-referencing deltas (A→B
                        // and B→A) that fail to reconstruct on read.
                        //
                        // Depth: the producer pre-caches every chunk's raw bytes
                        // (see the pre-cache below), so a base that was itself
                        // stored as a delta is still a cache HIT here. Without a
                        // depth guard each similar chunk adds a hop and the chain
                        // grows past what `get_chunk` will reconstruct.
                        let (resolved, observed) =
                            resolve_delta_base_observing(&*storage, nominated_base, chunk.id).await;
                        // Memoize the chain this walk just read so the guard
                        // below can re-check it without storage I/O under the lock.
                        delta_pairs.lock().await.merge_observed(observed);
                        match resolved {
                            None => {
                                debug!(
                                    chunk_id = %chunk.id,
                                    base_id = %nominated_base,
                                    "Parallel: refusing chunk delta (cycle or unresolvable chain)"
                                );
                            }
                            Some(base_id) => {
                                let base_key = format!("chunks/{}", base_id.to_hex());
                                // Fetching + decompressing the base was the
                                // only unmeasured region inside `delta_ms`.
                                // Once the delta lock was removed it became
                                // the dominant cost (~69% of delta_ms), and it
                                // was invisible except as a subtraction.
                                let _basefetch_timer = std::time::Instant::now();
                                // Check decompressed base chunk cache before hitting storage
                                let base_data_arc = if let Some(cached) =
                                    base_chunk_cache.get(&base_id).await
                                {
                                    Some(cached)
                                } else {
                                    match storage.get(&base_key).await {
                                        Ok(base_compressed) => {
                                            let decompressed = if let Some(ref smart) = smart_comp {
                                                decompress_typed_blocking(
                                                    smart.clone(),
                                                    base_compressed,
                                                )
                                                .await
                                                .ok()
                                            } else {
                                                decompress_blocking(
                                                    compressor.clone(),
                                                    base_compressed,
                                                )
                                                .await
                                                .ok()
                                            };
                                            if let Some(data) = decompressed {
                                                let arc = Arc::new(data);
                                                base_chunk_cache.insert(base_id, arc.clone()).await;
                                                Some(arc)
                                            } else {
                                                None
                                            }
                                        }
                                        _ => None,
                                    }
                                };
                                crate::add_phases::record(
                                    crate::add_phases::Phase::DeltaBaseFetch,
                                    _basefetch_timer.elapsed(),
                                );

                                if let Some(base_data) = base_data_arc {
                                    let delta = DeltaEncoder::encode(&base_data, &chunk.data);
                                    let delta_bytes = delta.to_bytes();
                                    let delta_ratio =
                                        delta_bytes.len() as f64 / chunk.data.len() as f64;

                                    let threshold =
                                        delta_ratio_threshold(chunk.codec_hint, chunk.chunk_type);
                                    if delta_ratio < threshold {
                                        let delta_key =
                                            format!("chunk-deltas/{}", chunk.id.to_hex());
                                        let compressed_delta = if let Some(ref smart) = smart_comp {
                                            smart
                                                .compress_typed(
                                                    &delta_bytes,
                                                    CompressionObjectType::Unknown,
                                                )
                                                .map_err(|e| {
                                                    anyhow::anyhow!("Compress delta: {}", e)
                                                })?
                                        } else {
                                            compressor.compress(&delta_bytes).map_err(|e| {
                                                anyhow::anyhow!("Compress delta: {}", e)
                                            })?
                                        };

                                        // TOCTOU guard FIRST: check+register before any I/O.
                                        // The registration is serialized with the re-check,
                                        // and lands strictly before the meta reaches disk:
                                        // the pre-walk above races with concurrent writers
                                        // (three parallel writes can form A→B→C→A with every
                                        // pre-walk passing, since no meta is on disk yet), so
                                        // whichever write closes a loop sees the completed
                                        // chain in memory and refuses. No storage I/O happens
                                        // under the lock at all.
                                        let should_write = commit_delta_pair(
                                            &*storage,
                                            &delta_pairs,
                                            chunk.id,
                                            base_id,
                                        )
                                        .await;

                                        // See the streaming path for why these
                                        // two puts are timed separately.
                                        let _dwrite_timer = std::time::Instant::now();
                                        if should_write {
                                            // Write .meta FIRST (durability anchor — see the
                                            // sequential path above).
                                            let meta_key =
                                                format!("chunk-deltas/{}.meta", chunk.id.to_hex());
                                            let meta_data = format!("base:{}", base_id.to_hex());
                                            if let Err(e) =
                                                storage.put(&meta_key, meta_data.as_bytes()).await
                                                && !storage.exists(&meta_key).await.unwrap_or(false)
                                            {
                                                rollback_delta_pair(
                                                    &delta_pairs,
                                                    chunk.id,
                                                    base_id,
                                                )
                                                .await;
                                                return Err(anyhow::anyhow!(
                                                    "Store delta meta: {}",
                                                    e
                                                ));
                                            }

                                            // Tolerate concurrent writes: if put fails but chunk exists, treat as dedup
                                            if let Err(e) =
                                                storage.put(&delta_key, &compressed_delta).await
                                                && !storage
                                                    .exists(&delta_key)
                                                    .await
                                                    .unwrap_or(false)
                                            {
                                                // Remove the routing sidecar so the chunk is
                                                // not permanently misrouted to a missing binary.
                                                // Roll back ONLY if the sidecar is really gone. The in-memory
                                                // graph must stay a superset of the on-disk edges: the depth
                                                // guard reads it, so an edge still on disk but missing from
                                                // memory makes the guard undercount and admit a chain past
                                                // MAX_DELTA_DEPTH. Keeping an edge whose binary never landed is
                                                // the safe direction to be wrong in - it only makes the guard
                                                // refuse a delta it could have taken.
                                                match storage.delete(&meta_key).await {
                                                    Ok(()) => {
                                                        rollback_delta_pair(
                                                            &delta_pairs,
                                                            chunk.id,
                                                            base_id,
                                                        )
                                                        .await;
                                                    }
                                                    Err(del_err) => warn!(
                                                        chunk_id = %chunk.id,
                                                        base_id = %base_id,
                                                        error = %del_err,
                                                        "failed to remove delta routing sidecar \
                                                         after a failed delta write; keeping the \
                                                         in-memory edge so the depth guard stays \
                                                         conservative"
                                                    ),
                                                }
                                                return Err(anyhow::anyhow!("Store delta: {}", e));
                                            }

                                            debug!(
                                                chunk_id = %chunk.id,
                                                base_id = %base_id,
                                                delta_ratio,
                                                "Parallel: stored chunk as delta"
                                            );
                                            stored_as_delta = true;
                                            crate::add_phases::record(
                                                crate::add_phases::Phase::DeltaWrite,
                                                _dwrite_timer.elapsed(),
                                            );
                                        } else {
                                            debug!(
                                                chunk_id = %chunk.id,
                                                base_id = %base_id,
                                                "Parallel: refusing chunk delta at commit — concurrent writes would close a cycle or exceed max depth"
                                            );
                                        }
                                    }
                                }
                            }
                        }
                    }

                    // 3. Full compress + store if not delta
                    if !stored_as_delta {
                        let compressed = if let Some(ref smart) = smart_comp {
                            smart
                                .compress_typed_with_size(&chunk.data, comp_type)
                                .map_err(|e| anyhow::anyhow!("Compress chunk: {}", e))?
                        } else {
                            compressor
                                .compress(&chunk.data)
                                .map_err(|e| anyhow::anyhow!("Compress chunk: {}", e))?
                        };

                        // Tolerate concurrent writes: if put fails but chunk exists, treat as dedup
                        if let Err(e) = storage.put(&chunk_key, &compressed).await
                            && !storage.exists(&chunk_key).await.unwrap_or(false)
                        {
                            return Err(anyhow::anyhow!("Store chunk: {}", e));
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
        producer
            .await
            .map_err(|e| anyhow::anyhow!("Producer task failed: {}", e))?;

        // Collect results from all workers
        let mut all_refs: Vec<(usize, ChunkRef)> = Vec::with_capacity(num_chunks);
        for handle in worker_handles {
            let worker_refs = handle
                .await
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
        let manifest_data = manifest
            .to_bytes()
            .map_err(|e| anyhow::anyhow!("Failed to serialize manifest: {}", e))?;
        self.storage
            .put(
                &manifest_key,
                &seal_manifest(self.smart_compressor.as_deref(), &manifest_data)?,
            )
            .await
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
    /// The OID (BLAKE3 hash) of the file content
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
    ///     "large_video.mp4",
    ///     None,
    /// ).await?;
    /// println!("Stored file with OID: {}", oid);
    /// # Ok(())
    /// # }
    /// ```
    pub async fn write_chunked_from_file<P: AsRef<std::path::Path>>(
        &self,
        path: P,
        filename: &str,
        on_progress: Option<Arc<dyn Fn(u64) + Send + Sync>>,
        precomputed_oid: Option<Oid>,
    ) -> anyhow::Result<Oid> {
        use std::sync::atomic::{AtomicU64, Ordering};

        let path = path.as_ref();
        let file_size = std::fs::metadata(path)?.len();

        info!(
            "Streaming parallel chunked write: file={}, size={}MB",
            filename,
            file_size / (1024 * 1024)
        );

        // Use caller-supplied OID when available to avoid a redundant full-file read.
        let file_oid = match precomputed_oid {
            Some(oid) => oid,
            None => Oid::from_file_async(path).await?,
        };

        // Check if we already have this file
        if self
            .storage
            .exists(&format!("manifests/{}", file_oid.to_hex()))
            .await?
        {
            debug!("File already exists in storage: {}", file_oid);
            // Report full file size so progress bar stays accurate
            if let Some(ref cb) = on_progress {
                cb(file_size);
            }
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

        // MEDIAGIT_ADD_COMPRESS_BLOCKING: default ON. Set to "0" to disable.
        // Chunks >= threshold bytes have their compression moved to a spawn_blocking
        // thread so the async worker task stays free for I/O during compression.
        let compress_blocking =
            std::env::var("MEDIAGIT_ADD_COMPRESS_BLOCKING").as_deref() != Ok("0");
        let compress_blocking_threshold: usize =
            std::env::var("MEDIAGIT_ADD_COMPRESS_BLOCKING_THRESHOLD")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(262_144); // 256 KiB — below this, spawn overhead > compression cost

        // --- Parallel pipeline: spawn workers FIRST, then produce chunks ---
        //
        // MEDIAGIT_CHUNK_WRITE_CONCURRENCY was documented as the knob for this
        // and never reached here — the streaming path (the production path for
        // files >= 5 MB) hardcoded the worker count, so setting it did nothing.
        // A dead knob is worse than no knob: it makes a measurement look
        // controlled when it is not, which is how GCS_UPLOAD_CONCURRENCY wasted
        // a cycle on the presigned path. Same clamp as before, so the default
        // is byte-for-byte the old behaviour; only an explicit setting changes it.
        let num_workers = std::env::var("MEDIAGIT_CHUNK_WRITE_CONCURRENCY")
            .ok()
            .and_then(|v| v.trim().parse::<usize>().ok())
            .filter(|n| *n > 0)
            .unwrap_or_else(|| num_cpus::get().clamp(2, 16));
        let (tx, rx) =
            async_channel::bounded::<(usize, crate::chunking::ContentChunk, Option<Oid>)>(64);

        // Spawn worker tasks BEFORE producing chunks to avoid deadlock.
        // The producer (collect_file_chunks_blocking) uses mmap + format-aware chunking
        // for large files. If workers aren't already consuming, the bounded channel
        // fills up and the producer blocks forever.
        let mut worker_handles = Vec::with_capacity(num_workers);
        for _ in 0..num_workers {
            let rx = rx.clone();
            let storage = self.storage.clone();
            let compressor = self.compressor.clone();
            let smart_comp = self.smart_compressor.clone();
            let base_chunk_cache = self.base_chunk_cache.clone();
            let delta_pairs = self.delta_written_pairs.clone();
            let compression_enabled = self.compression_enabled;
            let chunks_w = chunks_written.clone();
            let bytes_w = bytes_written.clone();
            let on_progress = on_progress.clone();

            let handle = tokio::spawn(async move {
                let mut results: Vec<(usize, ChunkRef)> = Vec::new();

                while let Ok((seq_id, chunk, base_oid_opt)) = rx.recv().await {
                    let chunk_ref = ChunkRef {
                        id: chunk.id,
                        offset: chunk.offset,
                        size: chunk.size,
                        chunk_type: chunk.chunk_type,
                        codec_hint: chunk.codec_hint,
                    };

                    // 1. Dedup check
                    let chunk_key = format!("chunks/{}", chunk.id.to_hex());
                    let delta_meta_key = format!("chunk-deltas/{}.meta", chunk.id.to_hex());
                    // Two round trips to storage per chunk, before any work is
                    // done. Cheap on a local ODB, not cheap on a remote one, and
                    // invisible in the wall figure until now.
                    let _dedup_timer = std::time::Instant::now();
                    let chunk_exists = storage.exists(&chunk_key).await.unwrap_or(false);
                    let delta_exists = storage.exists(&delta_meta_key).await.unwrap_or(false);
                    crate::add_phases::record(
                        crate::add_phases::Phase::Dedup,
                        _dedup_timer.elapsed(),
                    );

                    if chunk_exists || delta_exists {
                        debug!(chunk_id = %chunk.id, "Streaming parallel: chunk deduplicated");
                        if let Some(ref cb) = on_progress {
                            cb(chunk.size as u64);
                        }
                        results.push((seq_id, chunk_ref));
                        continue;
                    }

                    // 2. Delta encoding using the base pre-selected by the
                    //    producer (deterministic: same chunk order every run).
                    let mut stored_as_delta = false;
                    // Whole delta attempt: base resolution, the chain walk, the
                    // base fetch/decompress, the encode and the .meta write. It
                    // is the largest single unknown in `add` and was entirely
                    // absent from the breakdown's first version, which left 74%
                    // of a 357 MB PSD add unaccounted for.
                    let _delta_timer = std::time::Instant::now();
                    if let Some(nominated_base) = base_oid_opt {
                        // Cycle AND depth prevention in one chain walk (see the
                        // parallel non-streaming variant above for the full
                        // rationale, including why the producer pre-cache means
                        // a delta-stored base is still a cache hit here).
                        let _resolve_timer = std::time::Instant::now();
                        let (resolved, observed) =
                            resolve_delta_base_observing(&*storage, nominated_base, chunk.id).await;
                        // Memoize the chain this walk just read so the guard
                        // below can re-check it without storage I/O under the
                        // lock — that re-walk was the serialised section.
                        delta_pairs.lock().await.merge_observed(observed);
                        crate::add_phases::record(
                            crate::add_phases::Phase::DeltaResolve,
                            _resolve_timer.elapsed(),
                        );
                        match resolved {
                            None => {
                                debug!(
                                    chunk_id = %chunk.id,
                                    base_id = %nominated_base,
                                    "Streaming parallel: refusing chunk delta (cycle or unresolvable chain)"
                                );
                            }
                            Some(base_id) => {
                                let base_key = format!("chunks/{}", base_id.to_hex());
                                // Fetching + decompressing the base was the
                                // only unmeasured region inside `delta_ms`.
                                // Once the delta lock was removed it became
                                // the dominant cost (~69% of delta_ms), and it
                                // was invisible except as a subtraction.
                                let _basefetch_timer = std::time::Instant::now();
                                // Check decompressed base chunk cache before hitting storage
                                let base_data_arc = if let Some(cached) =
                                    base_chunk_cache.get(&base_id).await
                                {
                                    Some(cached)
                                } else {
                                    match storage.get(&base_key).await {
                                        Ok(base_compressed) => {
                                            let decompressed = if let Some(ref smart) = smart_comp {
                                                decompress_typed_blocking(
                                                    smart.clone(),
                                                    base_compressed,
                                                )
                                                .await
                                                .ok()
                                            } else {
                                                decompress_blocking(
                                                    compressor.clone(),
                                                    base_compressed,
                                                )
                                                .await
                                                .ok()
                                            };
                                            if let Some(data) = decompressed {
                                                let arc = Arc::new(data);
                                                base_chunk_cache.insert(base_id, arc.clone()).await;
                                                Some(arc)
                                            } else {
                                                None
                                            }
                                        }
                                        _ => None,
                                    }
                                };
                                crate::add_phases::record(
                                    crate::add_phases::Phase::DeltaBaseFetch,
                                    _basefetch_timer.elapsed(),
                                );

                                if let Some(base_data) = base_data_arc {
                                    let _encode_timer = std::time::Instant::now();
                                    let delta = DeltaEncoder::encode(&base_data, &chunk.data);
                                    let delta_bytes = delta.to_bytes();
                                    crate::add_phases::record(
                                        crate::add_phases::Phase::DeltaEncode,
                                        _encode_timer.elapsed(),
                                    );
                                    let delta_ratio =
                                        delta_bytes.len() as f64 / chunk.data.len() as f64;

                                    let threshold =
                                        delta_ratio_threshold(chunk.codec_hint, chunk.chunk_type);
                                    if delta_ratio < threshold {
                                        let delta_key =
                                            format!("chunk-deltas/{}", chunk.id.to_hex());
                                        let _dcomp_timer = std::time::Instant::now();
                                        let compressed_delta = if let Some(ref smart) = smart_comp {
                                            smart
                                                .compress_typed(
                                                    &delta_bytes,
                                                    CompressionObjectType::Unknown,
                                                )
                                                .map_err(|e| {
                                                    anyhow::anyhow!("Compress delta: {}", e)
                                                })?
                                        } else {
                                            compressor.compress(&delta_bytes).map_err(|e| {
                                                anyhow::anyhow!("Compress delta: {}", e)
                                            })?
                                        };

                                        // TOCTOU guard FIRST: check+register before any I/O
                                        // — same cycle-closing race as the other two
                                        // chunk-delta write sites (see the sequential path).
                                        crate::add_phases::record(
                                            crate::add_phases::Phase::DeltaCompress,
                                            _dcomp_timer.elapsed(),
                                        );

                                        // Queueing on this mutex WAS the bottleneck: it is
                                        // global and used to be held across a full chain
                                        // re-walk and the meta put, so N workers serialised
                                        // here for a measured 269 ms/chunk. The critical
                                        // section is now memory-only; this still times the
                                        // whole guard so a regression is visible.
                                        let _lock_timer = std::time::Instant::now();
                                        let should_write = commit_delta_pair(
                                            &*storage,
                                            &delta_pairs,
                                            chunk.id,
                                            base_id,
                                        )
                                        .await;
                                        crate::add_phases::record(
                                            crate::add_phases::Phase::DeltaLock,
                                            _lock_timer.elapsed(),
                                        );

                                        // The two puts below were the last
                                        // unmeasured region inside `delta_ms`.
                                        // Splitting them out distinguishes
                                        // "the writes are slow" from "the task
                                        // waited for a runtime thread" — these
                                        // counters are per-task elapsed wall,
                                        // so scheduling delay lands in the gap
                                        // rather than in any named phase.
                                        let _dwrite_timer = std::time::Instant::now();
                                        if should_write {
                                            // Write .meta FIRST (durability anchor).
                                            let meta_key =
                                                format!("chunk-deltas/{}.meta", chunk.id.to_hex());
                                            let meta_data = format!("base:{}", base_id.to_hex());
                                            if let Err(e) =
                                                storage.put(&meta_key, meta_data.as_bytes()).await
                                                && !storage.exists(&meta_key).await.unwrap_or(false)
                                            {
                                                rollback_delta_pair(
                                                    &delta_pairs,
                                                    chunk.id,
                                                    base_id,
                                                )
                                                .await;
                                                return Err(anyhow::anyhow!(
                                                    "Store delta meta: {}",
                                                    e
                                                ));
                                            }

                                            // Tolerate concurrent writes: if put fails but chunk exists, treat as dedup
                                            if let Err(e) =
                                                storage.put(&delta_key, &compressed_delta).await
                                                && !storage
                                                    .exists(&delta_key)
                                                    .await
                                                    .unwrap_or(false)
                                            {
                                                // Remove the routing sidecar so the chunk is
                                                // not permanently misrouted to a missing binary.
                                                // Roll back ONLY if the sidecar is really gone. The in-memory
                                                // graph must stay a superset of the on-disk edges: the depth
                                                // guard reads it, so an edge still on disk but missing from
                                                // memory makes the guard undercount and admit a chain past
                                                // MAX_DELTA_DEPTH. Keeping an edge whose binary never landed is
                                                // the safe direction to be wrong in - it only makes the guard
                                                // refuse a delta it could have taken.
                                                match storage.delete(&meta_key).await {
                                                    Ok(()) => {
                                                        rollback_delta_pair(
                                                            &delta_pairs,
                                                            chunk.id,
                                                            base_id,
                                                        )
                                                        .await;
                                                    }
                                                    Err(del_err) => warn!(
                                                        chunk_id = %chunk.id,
                                                        base_id = %base_id,
                                                        error = %del_err,
                                                        "failed to remove delta routing sidecar \
                                                         after a failed delta write; keeping the \
                                                         in-memory edge so the depth guard stays \
                                                         conservative"
                                                    ),
                                                }
                                                return Err(anyhow::anyhow!("Store delta: {}", e));
                                            }

                                            debug!(
                                                chunk_id = %chunk.id,
                                                base_id = %base_id,
                                                delta_ratio,
                                                "Streaming parallel: stored chunk as delta"
                                            );
                                            stored_as_delta = true;
                                            crate::add_phases::record(
                                                crate::add_phases::Phase::DeltaWrite,
                                                _dwrite_timer.elapsed(),
                                            );
                                        } else {
                                            debug!(
                                                chunk_id = %chunk.id,
                                                base_id = %base_id,
                                                "Streaming parallel: refusing chunk delta at commit — concurrent writes would close a cycle"
                                            );
                                        }
                                    }
                                }
                            }
                        }
                    }

                    // 3. Full compress + store if not delta
                    //
                    // For demuxed video container chunks with a known codec, use
                    // per-chunk codec-aware compression (e.g., Zstd for PCM audio,
                    // Brotli for subtitles, Store for H.264).  Falls back to file-level
                    // strategy when codec is unknown.
                    crate::add_phases::record(
                        crate::add_phases::Phase::Delta,
                        _delta_timer.elapsed(),
                    );

                    if !stored_as_delta {
                        let codec_hint = to_chunk_codec_hint(chunk.codec_hint, chunk.chunk_type);
                        // PERF-V10-PSD: compress and write are the two consumer-side
                        // costs. Measured separately from the producer so a high
                        // `send_block_ms` upstream can be attributed to one of them
                        // rather than guessed at.
                        let _compress_timer = std::time::Instant::now();
                        let data_to_store = if compress_blocking
                            && chunk.data.len() >= compress_blocking_threshold
                        {
                            // Offload CPU-heavy compression to a blocking thread so the async
                            // worker task stays free for I/O while this chunk is compressed.
                            let chunk_data = chunk.data.clone();
                            let smart2 = smart_comp.clone();
                            let compressor2 = compressor.clone();
                            tokio::task::spawn_blocking(move || -> anyhow::Result<Vec<u8>> {
                                if let Some(ref smart) = smart2 {
                                    match smart.compress_by_codec(&chunk_data, codec_hint) {
                                        Some(result) => result.map_err(|e| {
                                            anyhow::anyhow!("Compress chunk (codec): {}", e)
                                        }),
                                        _ => smart
                                            .compress_typed_with_size(&chunk_data, comp_type)
                                            .map_err(|e| anyhow::anyhow!("Compress chunk: {}", e)),
                                    }
                                } else if compression_enabled {
                                    compressor2
                                        .compress(&chunk_data)
                                        .map_err(|e| anyhow::anyhow!("Compress chunk: {}", e))
                                } else {
                                    Ok(chunk_data)
                                }
                            })
                            .await
                            .map_err(|e| anyhow::anyhow!("Compression task panicked: {}", e))??
                        } else if let Some(ref smart) = smart_comp {
                            // Try codec-aware compression first
                            match smart.compress_by_codec(&chunk.data, codec_hint) {
                                Some(result) => result.map_err(|e| {
                                    anyhow::anyhow!("Compress chunk (codec): {}", e)
                                })?,
                                _ => {
                                    // Unknown codec → fall back to file-level strategy
                                    smart
                                        .compress_typed_with_size(&chunk.data, comp_type)
                                        .map_err(|e| anyhow::anyhow!("Compress chunk: {}", e))?
                                }
                            }
                        } else if compression_enabled {
                            compressor
                                .compress(&chunk.data)
                                .map_err(|e| anyhow::anyhow!("Compress chunk: {}", e))?
                        } else {
                            chunk.data.clone()
                        };
                        crate::add_phases::record(
                            crate::add_phases::Phase::Compress,
                            _compress_timer.elapsed(),
                        );

                        // Tolerate concurrent writes
                        let _write_timer = std::time::Instant::now();
                        let put_result = storage.put(&chunk_key, &data_to_store).await;
                        crate::add_phases::record(
                            crate::add_phases::Phase::Write,
                            _write_timer.elapsed(),
                        );
                        if let Err(e) = put_result
                            && !storage.exists(&chunk_key).await.unwrap_or(false)
                        {
                            return Err(anyhow::anyhow!("Store chunk: {}", e));
                        }
                    }

                    chunks_w.fetch_add(1, Ordering::Relaxed);
                    bytes_w.fetch_add(chunk.size as u64, Ordering::Relaxed);
                    if let Some(ref cb) = on_progress {
                        cb(chunk.size as u64);
                    }

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
        let (blocking_tx, mut blocking_rx) =
            tokio::sync::mpsc::channel::<crate::chunking::ContentChunk>(32);

        let path_owned = path.to_path_buf();
        let chunk_strategy = self.chunk_strategy.unwrap_or(ChunkStrategy::MediaAware);
        let cdc_seed = self.cdc_seed;
        let file_producer = tokio::task::spawn_blocking(move || {
            let chunker_inner = ContentChunker::with_seed(chunk_strategy, cdc_seed);
            chunker_inner.collect_file_chunks_blocking(&path_owned, blocking_tx)
        });

        // Forward chunks from the blocking thread to the async worker channel.
        // Run similarity detection HERE (on the single-threaded producer path)
        // so base selection is deterministic — independent of worker scheduling.
        let producer_similarity_detector = self.similarity_detector.clone();
        let producer_delta_enabled = self.delta_enabled;
        let producer_base_chunk_cache = self.base_chunk_cache.clone();
        while let Some(chunk) = blocking_rx.recv().await {
            let seq_id = seq_counter.fetch_add(1, Ordering::SeqCst) as usize;

            // Producer-side similarity: replicate the delta-eligibility predicate,
            // then query the detector for the best base.
            let base_oid_opt = if producer_delta_enabled && chunk.data.len() >= 4096 {
                let is_high_entropy_codec = matches!(
                    chunk.codec_hint,
                    crate::chunking::CodecHint::H264
                        | crate::chunking::CodecHint::H265
                        | crate::chunking::CodecHint::VP9
                        | crate::chunking::CodecHint::AV1
                        | crate::chunking::CodecHint::AAC
                        | crate::chunking::CodecHint::Opus
                        | crate::chunking::CodecHint::Vorbis
                        | crate::chunking::CodecHint::MP3
                );
                let is_store_video_chunk =
                    if chunk.codec_hint == crate::chunking::CodecHint::Unknown {
                        chunk.chunk_type == crate::chunking::ChunkType::VideoStream
                            && matches!(
                                comp_type,
                                CompressionObjectType::Mp4
                                    | CompressionObjectType::Mov
                                    | CompressionObjectType::Avi
                                    | CompressionObjectType::Mkv
                                    | CompressionObjectType::Webm
                                    | CompressionObjectType::Flv
                                    | CompressionObjectType::Wmv
                                    | CompressionObjectType::Mpg
                                    | CompressionObjectType::Mxf
                            )
                    } else {
                        is_high_entropy_codec
                    };

                if !is_store_video_chunk {
                    let mut chunk_meta = crate::similarity::ObjectMetadata::new(
                        chunk.id,
                        chunk.data.len(),
                        crate::ObjectType::Blob,
                        None,
                    );
                    chunk_meta.generate_samples(&chunk.data);

                    let detector = producer_similarity_detector.read().await;
                    let similar = detector.find_similar_with_size_ratio(
                        &chunk_meta,
                        min_similarity,
                        size_ratio_threshold,
                    );
                    drop(detector);

                    let base_oid = similar.map(|(oid, _)| oid);
                    // Optimistic marking: keep is_delta=false so this chunk
                    // remains available as a future base candidate.
                    chunk_meta.is_delta = false;

                    let mut detector = producer_similarity_detector.write().await;
                    detector.add_object(chunk_meta);
                    drop(detector);

                    base_oid
                } else {
                    None
                }
            } else {
                None
            };

            // Pre-cache the chunk's raw data so that workers looking for
            // this chunk as a delta base can find it even before it has been
            // compressed and stored to the backend.
            producer_base_chunk_cache
                .insert(chunk.id, Arc::new(chunk.data.clone()))
                .await;

            tx.send((seq_id, chunk, base_oid_opt))
                .await
                .map_err(|_| anyhow::anyhow!("Worker channel closed unexpectedly"))?;
        }

        // Propagate any error from the blocking producer
        file_producer
            .await
            .map_err(|e| anyhow::anyhow!("File chunker task panicked: {}", e))??;

        // Close sender so workers know no more chunks are coming
        drop(tx);

        // Collect results from all workers
        let mut all_refs: Vec<(usize, ChunkRef)> = Vec::new();
        for handle in worker_handles {
            let worker_refs = handle
                .await
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

        let manifest_data = manifest.to_bytes()?;
        let manifest_key = format!("manifests/{}", file_oid.to_hex());
        self.storage
            .put(
                &manifest_key,
                &seal_manifest(self.smart_compressor.as_deref(), &manifest_data)?,
            )
            .await?;

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

    /// List all pack files in the database
    ///
    /// Returns a list of pack file keys. Matches both legacy `gc --repack`
    /// packs (`packs/<id>.pack`) and Track F cloud packs (`packs/<pack_oid>`,
    /// no extension — the server-side pack registry's JSONL manifests live
    /// on local disk under `.mediagit/packs/`, never in this storage prefix,
    /// so any non-`.pack` key here is a cloud-pack object). Both share the
    /// same on-disk envelope (`PackReader` parses either), so a plain
    /// extension filter previously excluded cloud packs from this search,
    /// making `read_from_packs` unable to find chunk-delta base chunks that
    /// landed only inside a cloud pack.
    async fn list_pack_files(&self) -> anyhow::Result<Vec<String>> {
        let pack_keys = self.storage.list_objects("packs/").await?;

        let pack_files: Vec<String> = pack_keys
            .into_iter()
            .filter(|key| {
                key.ends_with(".pack") || !key.rsplit('/').next().unwrap_or("").contains('.')
            })
            .collect();

        debug!(count = pack_files.len(), "Found pack files");
        Ok(pack_files)
    }

    /// Read an object from pack files
    ///
    /// Searches through all pack files to find the requested object.
    /// This is used as a fallback when loose object is not found.
    pub(super) async fn read_from_packs(&self, oid: &Oid) -> anyhow::Result<Vec<u8>> {
        use crate::pack::PackReader;

        debug!(oid = %oid, "Searching for object in pack files");

        // List all pack files
        let pack_files = self.list_pack_files().await?;

        if pack_files.is_empty() {
            anyhow::bail!(
                "Object {} not found: no loose object and no pack files",
                oid
            );
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
                                        match decompress_typed_blocking(
                                            smart_comp.clone(),
                                            compressed_data.clone(),
                                        )
                                        .await
                                        {
                                            Ok(d) => d,
                                            Err(_) => {
                                                // Fallback to standard decompression
                                                match decompress_blocking(
                                                    self.compressor.clone(),
                                                    compressed_data.clone(),
                                                )
                                                .await
                                                {
                                                    Ok(d) => d,
                                                    Err(_) => compressed_data, // Use raw data as last resort
                                                }
                                            }
                                        }
                                    } else if self.compression_enabled
                                        || (compressed_data.len() >= 2
                                            && compressed_data[0] == 0x78)
                                    {
                                        match decompress_blocking(
                                            self.compressor.clone(),
                                            compressed_data.clone(),
                                        )
                                        .await
                                        {
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

                                    // Cache the decompressed data (skip large objects)
                                    if data.len() <= MAX_CACHEABLE_OBJECT_SIZE {
                                        let arc_data = Arc::new(data.clone());
                                        self.cache.insert(*oid, arc_data).await;
                                    }

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
    pub(super) async fn read_chunked(&self, oid: &Oid) -> anyhow::Result<Vec<u8>> {
        debug!(oid = %oid, "Reconstructing chunked object");

        // Load chunk manifest (use to_hex() for consistent storage paths)
        let manifest_key = format!("manifests/{}", oid.to_hex());
        let manifest_data = self.storage.get(&manifest_key).await?;
        let manifest: ChunkManifest = ChunkManifest::from_bytes(&open_manifest(
            self.smart_compressor.as_deref(),
            &manifest_data,
        )?)
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
            let decompressed = self.get_chunk(&chunk_ref.id).await.map_err(|e| {
                anyhow::anyhow!(
                    "Failed to read chunk {} (index {}): {}",
                    chunk_ref.id.to_hex(),
                    idx,
                    e
                )
            })?;

            // Verify chunk integrity (hash + size).
            //
            // B2: this looks like a redundant hash -- the object is hashed again
            // in full below -- and it is not. `get_chunk` verifies the chunk it
            // reads from storage against `base_id`, which equals `chunk_id` only
            // for a NON-delta chunk. For a delta chunk it checks the base, then
            // applies the chain and returns the result **unverified**
            // (`get_chunk_limited`). So for every delta-encoded chunk this is the
            // only thing standing between a mis-applied delta and an object that
            // reassembles to plausible, wrong bytes.
            //
            // It also names WHICH chunk failed, which the whole-object hash below
            // cannot, and it fails before the reconstruction buffer is grown.
            let computed_chunk_oid = Oid::hash(&decompressed);
            if computed_chunk_oid != chunk_ref.id {
                anyhow::bail!(
                    "Chunk integrity check failed for chunk {} (index {}): expected {}, computed {}",
                    chunk_ref.id.to_hex(),
                    idx,
                    chunk_ref.id,
                    computed_chunk_oid
                );
            }

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

        // Verify integrity on reconstructed data.
        //
        // B2: NOT implied by the per-chunk checks above. Those prove every chunk
        // is the chunk it claims to be; they say nothing about whether the
        // manifest listed the right chunks, in the right order, exactly once.
        // Swap two entries and every per-chunk hash still matches, the total size
        // still matches, and the object is still wrong. This is the only check
        // that catches a manifest-level fault, so it stays despite hashing bytes
        // that were already hashed once as chunks.
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

        // Cache reconstructed data for future reads (skip large objects to
        // avoid pinning multi-GB media files in memory)
        if reconstructed.len() <= MAX_CACHEABLE_OBJECT_SIZE {
            self.cache
                .insert(*oid, Arc::new(reconstructed.clone()))
                .await;
        }

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
            let manifest: ChunkManifest = ChunkManifest::from_bytes(&open_manifest(
                self.smart_compressor.as_deref(),
                &manifest_data,
            )?)
            .map_err(|e| anyhow::anyhow!("Failed to deserialize chunk manifest: {}", e))?;

            // Ensure parent directory exists
            if let Some(parent) = path.parent() {
                tokio::fs::create_dir_all(parent).await?;
            }

            // Open a sibling tmp file for streaming write; only renamed into
            // place (via `finalize_atomic_write`) once every chunk has been
            // verified and written. A crash or error mid-stream leaves at
            // worst a stale `.mgtmp`, never a truncated file at `path`.
            let tmp_path = atomic_tmp_path(path)?;
            let mut file = tokio::fs::File::create(&tmp_path).await?;
            let mut bytes_written = 0u64;

            let write_result: anyhow::Result<()> = async {
                for chunk_ref in &manifest.chunks {
                    // Use get_chunk() which handles both full and delta-encoded chunks
                    let decompressed = self.get_chunk(&chunk_ref.id).await.map_err(|e| {
                        anyhow::anyhow!("Failed to read chunk {}: {}", chunk_ref.id.to_hex(), e)
                    })?;

                    // Verify chunk integrity (hash + size)
                    let computed_chunk_oid = Oid::hash(&decompressed);
                    if computed_chunk_oid != chunk_ref.id {
                        anyhow::bail!(
                            "Chunk integrity check failed for {}: expected {}, computed {}",
                            chunk_ref.id.to_hex(),
                            chunk_ref.id,
                            computed_chunk_oid
                        );
                    }

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
                Ok(())
            }
            .await;

            if let Err(e) = write_result {
                drop(file);
                let _ = tokio::fs::remove_file(&tmp_path).await;
                return Err(e);
            }
            drop(file);

            finalize_atomic_write(&tmp_path, path).await?;

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

            let tmp_path = atomic_tmp_path(path)?;
            if let Err(e) = tokio::fs::write(&tmp_path, &data).await {
                let _ = tokio::fs::remove_file(&tmp_path).await;
                return Err(e.into());
            }
            finalize_atomic_write(&tmp_path, path).await?;
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
            match decompress_typed_blocking(smart_comp.clone(), storage_data.clone()).await {
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
                    match decompress_blocking(self.compressor.clone(), storage_data.clone()).await {
                        Ok(d) => d,
                        Err(_) => storage_data, // Use raw data as last resort
                    }
                }
            }
        } else if self.compression_enabled || (storage_data.len() >= 2 && storage_data[0] == 0x78) {
            // Standard decompression path
            match decompress_blocking(self.compressor.clone(), storage_data.clone()).await {
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

        // Validate decompressed size to prevent OOM from corrupted data
        if data.len() as u64 > MAX_OBJECT_SIZE {
            anyhow::bail!(
                "Decompressed object size {} exceeds maximum {} bytes",
                data.len(),
                MAX_OBJECT_SIZE
            );
        }

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

        // Cache UNCOMPRESSED data for future reads (skip large objects)
        if data.len() <= MAX_CACHEABLE_OBJECT_SIZE {
            let arc_data = Arc::new(data.clone());
            self.cache.insert(*oid, arc_data).await;
        }

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

        // For chunked objects, read the manifest to get size without
        // reconstructing the entire file (avoids loading multi-GB files)
        let manifest_key = format!("manifests/{}", oid.to_hex());
        if self.storage.exists(&manifest_key).await? {
            let manifest_data = self.storage.get(&manifest_key).await?;
            let manifest: ChunkManifest = ChunkManifest::from_bytes(&open_manifest(
                self.smart_compressor.as_deref(),
                &manifest_data,
            )?)
            .map_err(|e| anyhow::anyhow!("Failed to deserialize chunk manifest: {}", e))?;
            return Ok(manifest.total_size as usize);
        }

        // Not cached and not chunked - read the object to get its size
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
    pub async fn get_chunk_manifest(
        &self,
        oid: &Oid,
    ) -> anyhow::Result<Option<crate::chunking::ChunkManifest>> {
        let manifest_key = format!("manifests/{}", oid.to_hex());

        if !self.storage.exists(&manifest_key).await? {
            return Ok(None);
        }

        let manifest_data = self.storage.get(&manifest_key).await?;
        let manifest: crate::chunking::ChunkManifest = crate::chunking::ChunkManifest::from_bytes(
            &open_manifest(self.smart_compressor.as_deref(), &manifest_data)?,
        )
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
        let total = manifest.chunks.len();
        let max_chunks = similarity_seed_max_chunks();
        // Spread the sample evenly across the manifest instead of just
        // taking the first `max_chunks` — a stride keeps coverage
        // representative of the whole file, not just its start.
        let stride = (total / max_chunks.max(1)).max(1);

        let mut seeded = 0;
        let mut skipped_deep = 0;
        for chunk_ref in manifest.chunks.iter().step_by(stride).take(max_chunks) {
            // Reconstructing a deep delta chain just to seed the detector
            // costs more than the delta it might later enable — skip it.
            // Depth 2 measured: costs ≤1.7pp savings on 5-deep wav chains,
            // buys flat seeding time on deep epoch chains (PERF-ML-1).
            if self.chunk_delta_depth(&chunk_ref.id).await > 2 {
                skipped_deep += 1;
                continue;
            }
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
        if seeded > 0 || skipped_deep > 0 {
            info!(
                seeded_chunks = seeded,
                skipped_deep_chunks = skipped_deep,
                total_chunks = total,
                sampled_chunks = total.div_ceil(stride).min(max_chunks),
                "Seeded similarity detector from previous manifest"
            );
        }
        Ok(seeded)
    }

    /// Depth of `chunk_id`'s delta chain (0 = full chunk, no `.meta`).
    /// Only reads the small `chunk-deltas/*.meta` sidecars — never chunk
    /// payloads — so it's cheap to call before deciding whether a full
    /// `get_chunk` reconstruction is worth it.
    async fn chunk_delta_depth(&self, chunk_id: &Oid) -> usize {
        let mut depth = 0usize;
        let mut cur = *chunk_id;
        let mut visited = std::collections::HashSet::new();
        loop {
            if depth > MAX_DELTA_DEPTH as usize || !visited.insert(cur) {
                return depth;
            }
            let meta_key = format!("chunk-deltas/{}.meta", cur.to_hex());
            match self.storage.exists(&meta_key).await {
                Ok(true) => {}
                _ => return depth,
            }
            let bytes = match self.storage.get(&meta_key).await {
                Ok(b) => b,
                Err(_) => return depth,
            };
            let s = String::from_utf8_lossy(&bytes);
            let hex = match s.trim().strip_prefix("base:") {
                Some(h) => h.trim(),
                None => return depth,
            };
            let next = match Oid::from_hex(hex) {
                Ok(o) => o,
                Err(_) => return depth,
            };
            cur = next;
            depth += 1;
        }
    }

    /// Seed the similarity detector from a full blob object (non-chunked files).
    ///
    /// Used to enable cross-invocation delta encoding for files that are too small
    /// to be chunked (< 5 MB). After calling this with the previous version's OID,
    /// the next `write_with_delta` call can find a similar base and produce a delta.
    pub async fn seed_similarity_from_blob(&self, oid: &Oid, filename: &str) -> anyhow::Result<()> {
        // Cheap guard: if this OID is actually a chunked manifest, skip. Otherwise
        // self.read() would reconstruct the entire file into memory (hundreds of
        // MB for large creative assets). Callers should try seed_similarity_from_manifest
        // first; this is defence-in-depth.
        let manifest_key = format!("manifests/{}", oid.to_hex());
        if self.storage.exists(&manifest_key).await.unwrap_or(false) {
            debug!(oid = %oid, "Skipping seed_similarity_from_blob: chunked object");
            return Ok(());
        }
        let data = self.read(oid).await?;
        // Size cap: this path is intended for blobs < 5 MB. Bail on anything larger
        // to avoid burning memory + CPU sampling a huge blob whose size-ratio
        // would preclude a similarity match anyway.
        const MAX_SEED_BYTES: usize = 8 * 1024 * 1024; // 8 MiB
        if data.len() > MAX_SEED_BYTES {
            debug!(
                oid = %oid,
                size = data.len(),
                "Skipping seed_similarity_from_blob: oversized blob"
            );
            return Ok(());
        }
        let mut meta = crate::similarity::ObjectMetadata::new(
            *oid,
            data.len(),
            crate::ObjectType::Blob,
            if filename.is_empty() {
                None
            } else {
                Some(filename.to_string())
            },
        );
        meta.generate_samples(&data);
        self.similarity_detector.write().await.add_object(meta);
        debug!(
            oid = %oid,
            filename,
            "Seeded similarity detector from previous blob"
        );
        Ok(())
    }

    /// Get chunk data by chunk ID
    ///
    /// Reads and decompresses a single chunk, reconstructing from delta if needed.
    /// Supports ALL file types: AI/ML models, creative projects, 3D, text, etc.
    pub async fn get_chunk(&self, chunk_id: &Oid) -> anyhow::Result<Vec<u8>> {
        self.get_chunk_limited(chunk_id, Some(MAX_DELTA_DEPTH as usize))
            .await
    }

    /// Reconstruct a chunk through a delta chain of **any** depth.
    ///
    /// Identical to [`ObjectDatabase::get_chunk`] except that the depth limit
    /// is not applied — still cycle-safe, still iterative. This exists solely
    /// for `fsck --repair`'s chain flattening: a repository written before the
    /// write-side depth guard can hold chains that `get_chunk` refuses, and
    /// refusing them is precisely why it needs repairing. Nothing on a read or
    /// transfer path may call this — they must keep enforcing the limit.
    pub async fn reconstruct_chunk_unbounded(&self, chunk_id: &Oid) -> anyhow::Result<Vec<u8>> {
        self.get_chunk_limited(chunk_id, None).await
    }

    /// Shared implementation of [`Self::get_chunk`] /
    /// [`Self::reconstruct_chunk_unbounded`].
    ///
    /// `max_chain` of `None` disables the depth ceiling; cycle detection is
    /// unconditional either way.
    async fn get_chunk_limited(
        &self,
        chunk_id: &Oid,
        max_chain: Option<usize>,
    ) -> anyhow::Result<Vec<u8>> {
        // Walk the delta chain iteratively (no async recursion) to bound stack
        // usage regardless of chain length and to detect cycles. Each iteration
        // reads a tiny meta record; the heavy work (base read + delta apply)
        // happens after the full chain is known.
        let mut chain: Vec<Oid> = Vec::new(); // leaf-first order
        let mut visited: std::collections::HashSet<Oid> = std::collections::HashSet::new();
        let mut cur = *chunk_id;
        let base_id = loop {
            if !visited.insert(cur) {
                anyhow::bail!(
                    "Circular reference detected in chunk delta chain at {}",
                    cur
                );
            }
            if let Some(max) = max_chain
                && chain.len() > max
            {
                anyhow::bail!(
                    "Chunk delta chain too deep (> {}): chain starting at {}",
                    max,
                    chunk_id
                );
            }
            let meta_key = format!("chunk-deltas/{}.meta", cur.to_hex());
            match self.storage.get(&meta_key).await {
                Ok(meta_bytes) => {
                    let meta_str = String::from_utf8_lossy(&meta_bytes);
                    let base_hex = meta_str
                        .strip_prefix("base:")
                        .ok_or_else(|| anyhow::anyhow!("Invalid chunk delta meta: {}", meta_str))?
                        .trim();
                    let next = Oid::from_hex(base_hex).map_err(|e| {
                        anyhow::anyhow!("Invalid base OID in chunk delta meta: {}", e)
                    })?;
                    chain.push(cur);
                    cur = next;
                }
                Err(_) => {
                    // No meta => `cur` is the terminal (non-delta) base chunk
                    break cur;
                }
            }
        };

        // Read the base chunk (non-delta) once. Loose first; if `gc --repack`
        // has bundled it into a pack and removed the loose copy, fall back to
        // the same pack-routing `read_from_packs` uses for whole objects
        // (chunk IDs are content hashes too, so its integrity check applies
        // unchanged). `read_from_packs` returns already-decompressed data.
        let base_key = format!("chunks/{}", base_id.to_hex());
        let mut current = match self.storage.get(&base_key).await {
            Ok(compressed_base) => {
                if let Some(smart_comp) = &self.smart_compressor {
                    decompress_typed_blocking(smart_comp.clone(), compressed_base)
                        .await
                        .map_err(|e| anyhow::anyhow!("Failed to decompress base chunk: {}", e))?
                } else {
                    let algo = CompressionAlgorithm::detect(&compressed_base);
                    match algo {
                        CompressionAlgorithm::None => compressed_base,
                        _ => {
                            let fallback = compressed_base.clone();
                            decompress_blocking(self.compressor.clone(), compressed_base)
                                .await
                                .unwrap_or(fallback)
                        }
                    }
                }
            }
            Err(_) => self.read_from_packs(&base_id).await.map_err(|e| {
                anyhow::anyhow!(
                    "Failed to read base chunk {}: not found loose or in packs: {}",
                    base_id,
                    e
                )
            })?,
        };

        // ST-1: verify the base chunk before building on it.
        //
        // Chunk IDs are content hashes, so `base_id` is exactly the expected
        // digest — the check costs one BLAKE3 pass and needs no extra state.
        // Nothing verified it before, and two consumers take the result on
        // trust: `mediagit-server`'s `download_file_by_path` streams it
        // straight to an HTTP client, and `get_compressed_chunk` re-compresses
        // it into a pack, where it is stored under the id it was *supposed*
        // to have. A corrupt base therefore propagated silently and could be
        // re-published as authoritative.
        //
        // The loose read above can also fall back to raw bytes when
        // decompression fails (a deliberate allowance for content whose first
        // bytes mimic a codec magic). That fallback is only safe *because*
        // something downstream checks the digest — which, until now, nothing
        // did.
        //
        // `read_from_packs` already performs this check, so the pack path
        // pays for it twice; a wrong-but-verified chunk is worth more than a
        // saved hash.
        let actual = Oid::hash(&current);
        if actual != base_id {
            anyhow::bail!(
                "base chunk {} failed integrity check: computed {}. \
                 The stored bytes are corrupt or were written by an \
                 incompatible codec; reconstructing deltas on top of them \
                 would produce silently wrong data.",
                base_id.to_hex(),
                actual.to_hex()
            );
        }

        // Apply deltas from base->leaf (chain is leaf-first, so reverse)
        for delta_oid in chain.iter().rev() {
            let delta_key = format!("chunk-deltas/{}", delta_oid.to_hex());
            let compressed_delta = self
                .storage
                .get(&delta_key)
                .await
                .map_err(|e| anyhow::anyhow!("Failed to load chunk delta: {}", e))?;
            let delta_bytes = if let Some(smart_comp) = &self.smart_compressor {
                decompress_typed_blocking(smart_comp.clone(), compressed_delta)
                    .await
                    .map_err(|e| anyhow::anyhow!("Failed to decompress chunk delta: {}", e))?
            } else {
                decompress_blocking(self.compressor.clone(), compressed_delta)
                    .await
                    .map_err(|e| anyhow::anyhow!("Failed to decompress chunk delta: {}", e))?
            };
            let delta = Delta::from_bytes(&delta_bytes)
                .map_err(|e| anyhow::anyhow!("Failed to parse chunk delta: {}", e))?;
            current = DeltaDecoder::apply(&current, &delta)
                .map_err(|e| anyhow::anyhow!("Failed to apply chunk delta: {}", e))?;
            if current.len() as u64 > MAX_OBJECT_SIZE {
                anyhow::bail!(
                    "Reconstructed chunk size {} exceeds maximum {} bytes",
                    current.len(),
                    MAX_OBJECT_SIZE
                );
            }
        }

        if !chain.is_empty() {
            tracing::debug!(
                chunk_id = %chunk_id,
                base_id = %base_id,
                chain_depth = chain.len(),
                reconstructed_size = current.len(),
                "Reconstructed chunk from delta chain (iterative)"
            );
            return Ok(current);
        }

        // Not a delta chunk - return what we already read as the raw chunk
        Ok(current)
    }

    /// Store `data` as a full (non-delta) chunk under `chunks/<chunk_id>`.
    ///
    /// Used by `fsck --repair` when flattening an over-deep delta chain. The
    /// caller is responsible for having verified that `Oid::hash(data) ==
    /// chunk_id` — this method does not re-hash, because its only caller has
    /// already done so and the check is what makes the repair safe.
    pub async fn write_full_chunk(&self, chunk_id: &Oid, data: &[u8]) -> anyhow::Result<()> {
        let compressed = if let Some(smart_comp) = &self.smart_compressor {
            smart_comp
                .compress_typed(data, CompressionObjectType::Unknown)
                .map_err(|e| anyhow::anyhow!("Failed to compress chunk {}: {}", chunk_id, e))?
        } else {
            self.compressor
                .compress(data)
                .map_err(|e| anyhow::anyhow!("Failed to compress chunk {}: {}", chunk_id, e))?
        };
        let chunk_key = format!("chunks/{}", chunk_id.to_hex());
        self.storage
            .put(&chunk_key, &compressed)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to store chunk {}: {}", chunk_id, e))?;
        Ok(())
    }

    /// Get raw compressed chunk data for network transfer
    ///
    /// Fast path: reads pre-compressed chunk data directly (no decompress/recompress).
    /// Fallback: if the chunk is stored as a delta, reconstructs it via `get_chunk()`
    /// and re-compresses for transfer. This handles the case where deduplication
    /// stored some chunks as deltas against a base chunk.
    pub async fn get_compressed_chunk(&self, chunk_id: &Oid) -> anyhow::Result<Vec<u8>> {
        let chunk_key = format!("chunks/{}", chunk_id.to_hex());

        // Fast path: raw chunk exists loose
        if let Ok(data) = self.storage.get(&chunk_key).await {
            return Ok(data);
        }

        // Fallback: chunk is delta-encoded, or was bundled into a pack by
        // `gc --repack` (and the loose copy removed) — reconstruct via
        // get_chunk() (delta-chain-aware and pack-aware) and re-compress for
        // transfer.
        tracing::debug!(
            chunk_id = %chunk_id,
            "Chunk not found loose, reconstructing via delta chain or pack"
        );

        let decompressed = self
            .get_chunk(chunk_id)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to reconstruct chunk {}: {}", chunk_id, e))?;

        // Re-compress for network transfer
        if let Some(smart_comp) = &self.smart_compressor {
            smart_comp
                .compress_typed(&decompressed, CompressionObjectType::Unknown)
                .map_err(|e| {
                    anyhow::anyhow!("Failed to compress reconstructed chunk {}: {}", chunk_id, e)
                })
        } else {
            self.compressor.compress(&decompressed).map_err(|e| {
                anyhow::anyhow!("Failed to compress reconstructed chunk {}: {}", chunk_id, e)
            })
        }
    }

    /// Byte length of the chunk exactly as `get_compressed_chunk` would return it,
    /// without reading the bytes — or `None` when it cannot be known cheaply.
    ///
    /// Mirrors the fast path of `get_compressed_chunk`: a loose chunk at
    /// `chunks/<hex>` has its length `head`-ed directly. A delta-encoded or
    /// gc-repacked chunk (no loose copy) returns `None` rather than guessing,
    /// since its transfer length depends on a reconstruct+recompress that
    /// hasn't happened yet.
    pub async fn compressed_chunk_len(&self, chunk_id: &Oid) -> Option<u64> {
        let chunk_key = format!("chunks/{}", chunk_id.to_hex());
        self.storage.head(&chunk_key).await.ok().flatten()
    }

    /// Store raw compressed chunk data (no compression)
    ///
    /// Used when receiving pre-compressed chunks from remote. Decompresses
    /// the payload and verifies it hashes to the declared `chunk_id` BEFORE
    /// persisting anything — a corrupted or tampered chunk from an
    /// untrusted transport must never be admitted into the store under a
    /// hash it doesn't match (QA-006b: corruption admitted here propagates
    /// silently through every later reader). On mismatch the chunk is not
    /// stored at all. The send fast path (`get_compressed_chunk` above) is
    /// unaffected — this only guards the receive/write boundary.
    pub async fn put_compressed_chunk(&self, chunk_id: &Oid, data: &[u8]) -> anyhow::Result<()> {
        // The decompressed bytes are needed ONLY to compute the id below — what
        // gets stored is `data`, the original compressed bytes (see
        // `seal_from_wire` further down). So the SmartCompressor path streams
        // through a hashing sink and never materialises the uncompressed chunk;
        // at 24-32 concurrent downloads that buffer was the dominant client
        // allocation during a clone.
        //
        // Only the SmartCompressor arm is streamed. The `else` arm uses the
        // plain `Compressor`, which has no streaming decoder, and its framing
        // (the Store 0x00 prefix) has been a source of silent corruption before
        // — routing it through a different decoder to save memory on a path
        // clone never takes would be a bad trade. `clone` builds its ODB with
        // `with_smart_compression` (`clone.rs:149,414` -> `core.rs:165`, which
        // sets `Some` unconditionally), so the streaming arm is the clone path.
        let computed = if let Some(smart_comp) = &self.smart_compressor {
            decompress_typed_hash_blocking(smart_comp.clone(), data.to_vec())
                .await
                .map_err(|e| anyhow::anyhow!("Chunk {} failed to decompress: {}", chunk_id, e))?
        } else {
            let decompressed = match CompressionAlgorithm::detect(data) {
                CompressionAlgorithm::None => data.to_vec(),
                _ => decompress_blocking(self.compressor.clone(), data.to_vec())
                    .await
                    .map_err(|e| {
                        anyhow::anyhow!("Chunk {} failed to decompress: {}", chunk_id, e)
                    })?,
            };
            Oid::hash(&decompressed)
        };
        if computed != *chunk_id {
            anyhow::bail!(
                "Chunk integrity check failed for chunk {}: expected {}, computed {} — refusing to store",
                chunk_id,
                chunk_id,
                computed
            );
        }

        let chunk_key = format!("chunks/{}", chunk_id.to_hex());
        // DC-7: seal what came off the wire, or a `pull` into a keyed repo
        // leaves the ODB half encrypted -- `unseal` passes unsealed bytes
        // through, so nothing would report the split and reads would just keep
        // working over plaintext on disk.
        //
        // `_once`, not `seal_at_rest`: this used to assume the wire bytes were
        // unsealed "because the server holds no key". Since D4 the server holds
        // the key and hands back exactly what was uploaded, so on an encrypted
        // repo they arrive sealed and wrapping them again made every cloned
        // chunk fail its hash check. The decompress above has already opened
        // them under this repo's key, so passing them through is verified, not
        // assumed.
        let sealed = seal_from_wire(self.smart_compressor.as_deref(), data)?;
        self.storage
            .put(&chunk_key, &sealed)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to store chunk {}: {}", chunk_id, e))
    }

    /// Store a compressed chunk from a local temp file (B4 stream-to-disk).
    ///
    /// Delegates to `StorageBackend::put_file`. On `LocalBackend` this is a
    /// zero-copy atomic rename; cloud backends fall back to reading the file
    /// and uploading.
    ///
    /// DC-7: a keyed repo forfeits the rename. The temp file holds the
    /// remote's unsealed bytes, and there is no way to seal them without
    /// reading them, so the fast path stays available only to the (unkeyed)
    /// majority — where it behaves exactly as it always has.
    pub async fn put_compressed_chunk_from_file(
        &self,
        chunk_id: &Oid,
        path: &std::path::Path,
    ) -> anyhow::Result<()> {
        // Same integrity contract as `put_compressed_chunk`: bytes that arrived
        // from a remote are not stored under an id until they are shown to hash
        // to it.
        //
        // This used to be missing, and this is the DEFAULT path, not a corner:
        // `MEDIAGIT_STREAM_CHUNK_TO_DISK` defaults to 1 (`pull.rs`), so every
        // per-chunk clone fallback came through here, while the verified
        // in-memory sibling was the branch you had to opt into.
        //
        // The reachable failure is a proxied body that ends EARLY WITHOUT AN
        // ERROR. `Body::from_stream` terminates a chunked response normally
        // when its stream yields `None` — it only aborts on `Some(Err)` — so a
        // short upstream read arrives as a complete, short body. The client's
        // streaming helper then returns `Ok(written)` with no size check (its
        // own comment said "the hash check that would catch it happens later",
        // which was true of the in-memory sibling and not of this path), and a
        // truncated chunk was written under a valid id. It would surface much
        // later, as corruption, to someone who did nothing wrong. This repo has
        // already shipped two short-but-clean read bugs: GCS resumable
        // truncation, and the store-prefix P0 that corrupted ~1 in 8000 objects.
        //
        // Verified by STREAMING the staged file, never by reading it back.
        // B4 staged to disk precisely to stop holding chunks in RAM at 24-32
        // concurrent downloads; a verification that buffers would undo the
        // reason this function exists.
        let computed = if let Some(smart) = &self.smart_compressor {
            decompress_file_hash_blocking(smart.clone(), path.to_path_buf())
                .await
                .map_err(|e| anyhow::anyhow!("Chunk {} failed to decompress: {}", chunk_id, e))?
        } else {
            // No SmartCompressor: mirror `put_compressed_chunk`'s else-arm
            // exactly, including its use of the plain `Compressor`, which has
            // no streaming decoder. `clone` always builds its ODB with smart
            // compression (`clone.rs` -> `core.rs`), so this arm is not the
            // clone path and does not carry its memory constraint.
            let data = tokio::fs::read(path).await.map_err(|e| {
                anyhow::anyhow!("Failed to read staged chunk {}: {}", path.display(), e)
            })?;
            let decompressed = match CompressionAlgorithm::detect(&data) {
                CompressionAlgorithm::None => data,
                _ => decompress_blocking(self.compressor.clone(), data)
                    .await
                    .map_err(|e| {
                        anyhow::anyhow!("Chunk {} failed to decompress: {}", chunk_id, e)
                    })?,
            };
            Oid::hash(&decompressed)
        };
        if computed != *chunk_id {
            // The staged file is the evidence, but it is also corrupt: leaving
            // it behind in the temp dir would let a later run stage over a
            // half-written name. The caller removes it on the success path only.
            let _ = tokio::fs::remove_file(path).await;
            anyhow::bail!(
                "Chunk integrity check failed for staged chunk {}: expected {}, computed {} — refusing to store",
                chunk_id,
                chunk_id,
                computed
            );
        }

        let chunk_key = format!("chunks/{}", chunk_id.to_hex());
        if mediagit_compression::process_key().is_some() {
            let data = tokio::fs::read(path).await.map_err(|e| {
                anyhow::anyhow!("Failed to read staged chunk {}: {}", path.display(), e)
            })?;
            // See `put_compressed_chunk`: bytes staged from a remote may
            // already carry an envelope, and a second one is unreadable.
            let sealed = seal_from_wire(self.smart_compressor.as_deref(), &data)?;
            return self
                .storage
                .put(&chunk_key, &sealed)
                .await
                .map_err(|e| anyhow::anyhow!("Failed to store chunk {}: {}", chunk_id, e));
        }
        self.storage
            .put_file(&chunk_key, path)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to store chunk {}: {}", chunk_id, e))
    }

    /// Store chunk manifest
    pub async fn put_manifest(
        &self,
        oid: &Oid,
        manifest: &crate::chunking::ChunkManifest,
    ) -> anyhow::Result<()> {
        let manifest_key = format!("manifests/{}", oid.to_hex());
        let manifest_data = manifest
            .to_bytes()
            .map_err(|e| anyhow::anyhow!("Failed to serialize manifest: {}", e))?;
        self.storage
            .put(
                &manifest_key,
                &seal_manifest(self.smart_compressor.as_deref(), &manifest_data)?,
            )
            .await
            .map_err(|e| anyhow::anyhow!("Failed to store manifest {}: {}", oid, e))
    }

    /// Register a chunk-delta edge that is about to be written to storage by
    /// something OTHER than this ODB's own delta path.
    ///
    /// The under-lock cycle/depth re-check reads the in-memory `DeltaGraph`
    /// rather than storage, and that is only sound while
    /// **in-memory edges ⊇ on-disk edges**. Every writer of a
    /// `chunk-deltas/<id>.meta` sidecar must therefore register here *before*
    /// the sidecar reaches disk, or the graph will report a node as terminal
    /// when disk says it is a delta — which **undercounts chain depth** and
    /// lets a chain slip past `MAX_DELTA_DEPTH`.
    ///
    /// That is not hypothetical: `upload_chunk_delta` (the push-receive
    /// handler) writes the sidecar directly, and a server process runs it
    /// alongside this ODB's own delta writes. Campaign 20260805-repro-a9
    /// caught the result — `A11-delta-chain-depth maxDepth=11 (limit=10)`,
    /// with `fsck`, push and clone all failing on the resulting repository.
    ///
    /// The instance lock guarantees one *process* per repo; it does not
    /// guarantee one *writer* inside it. Registering here restores that.
    pub async fn register_external_delta_edge(&self, chunk_id: Oid, base_id: Oid) {
        self.delta_written_pairs
            .lock()
            .await
            .register_edge(chunk_id, base_id);
    }

    /// Check if a chunk exists (including delta-encoded chunks and chunks
    /// that only live inside a pack file — see `ensure_pack_membership_loaded`).
    pub async fn chunk_exists(&self, chunk_id: &Oid) -> anyhow::Result<bool> {
        let chunk_key = format!("chunks/{}", chunk_id.to_hex());
        if self.storage.exists(&chunk_key).await? {
            return Ok(true);
        }
        // Also check for delta-encoded chunk
        let delta_key = format!("chunk-deltas/{}.meta", chunk_id.to_hex());
        if self.storage.exists(&delta_key).await? {
            return Ok(true);
        }
        // Packs are immutable once written and the loose copy is deleted on
        // `repack(remove_loose=true)`, so a miss above doesn't mean "new" —
        // it may already be packed.
        self.ensure_pack_membership_loaded().await?;
        let guard = self.pack_membership.read().await;
        Ok(guard.as_ref().is_some_and(|set| set.contains(chunk_id)))
    }

    /// Lazily build the in-memory set of every OID embedded in a pack index.
    /// Reads each pack file once (to parse its trailing index) — cheap
    /// relative to a per-call full-pack scan, and never re-run once loaded
    /// except to extend it (`repack()` does this directly).
    ///
    /// `pub(super)`: also used by `odb::core`'s `exists()` and
    /// `resolve_abbreviated_oid()` for pack-membership union semantics.
    pub(super) async fn ensure_pack_membership_loaded(&self) -> anyhow::Result<()> {
        {
            let guard = self.pack_membership.read().await;
            if guard.is_some() {
                return Ok(());
            }
        }
        let mut guard = self.pack_membership.write().await;
        if guard.is_some() {
            // Another task raced us and already loaded it.
            return Ok(());
        }
        use crate::pack::PackReader;
        let mut set = std::collections::HashSet::new();
        for pack_key in self.list_pack_files().await? {
            if let Ok(pack_data) = self.storage.get(&pack_key).await
                && let Ok(pack_reader) = PackReader::new(pack_data)
            {
                for (oid, _) in pack_reader.index().iter() {
                    set.insert(*oid);
                }
            }
        }
        *guard = Some(set);
        Ok(())
    }

    /// Check whether a chunk-delta is present locally for the given chunk id.
    ///
    /// Used by the clone/fetch path to skip re-downloading delta payloads we
    /// already have. Looks at the `.meta` sidecar (presence of meta implies
    /// delta storage; the payload file is written first, so meta is the
    /// authoritative marker).
    pub async fn chunk_delta_exists(&self, chunk_id: &Oid) -> anyhow::Result<bool> {
        let meta_key = format!("chunk-deltas/{}.meta", chunk_id.to_hex());
        self.storage.exists(&meta_key).await
    }

    /// Persist a chunk-delta received from the wire.
    ///
    /// `compressed_delta_bytes` is the raw payload as the server sent it
    /// (already compressed by the server's storage path). `base_id` is the
    /// chunk this delta is encoded against. Caller is responsible for
    /// ensuring the base chunk is locally present before this delta is
    /// stored — otherwise reads will fail until the base arrives.
    ///
    /// Refuses self-loops and writes that would create a cycle in the local
    /// chunk-delta chain — caller should fall back to fetching the full
    /// chunk via `chunks/<id>` in that case.
    ///
    /// Stores the payload at `chunk-deltas/<chunk_id>` and the metadata
    /// (`base:<hex>`) at `chunk-deltas/<chunk_id>.meta`. Tolerates concurrent
    /// writers: if a put races and the key already exists, treats as dedup.
    pub async fn write_chunk_delta(
        &self,
        chunk_id: &Oid,
        base_id: &Oid,
        compressed_delta_bytes: &[u8],
    ) -> anyhow::Result<()> {
        // Unlike the add-side writers, this path takes its base entirely on
        // trust from a remote, so it must not re-target — a delta payload is
        // bound to the exact base it was encoded against. Refusing with an
        // error is correct here: callers (pull.rs) already fall back to
        // fetching the full chunk when this returns Err.
        // Through `commit_delta_pair`, like every other chunk-delta writer.
        //
        // This used to call `resolve_delta_base` against storage and then write,
        // which broke the graph's stated invariant -- in-memory edges must be a
        // SUPERSET of on-disk ones -- in two ways: the check was not atomic with
        // the write, and the resulting edge was never registered at all. A pull
        // running alongside anything else that writes deltas would leave the
        // in-memory guard believing a node was terminal when disk said it was a
        // delta, which undercounts depth. That is the same shape as the defect
        // fixed in `0fb6f6c`, where an undercounted chain reached 15 against a
        // cap of 10 and left the repository unpushable.
        //
        // `commit_delta_pair` keeps the semantics this path needs: it refuses
        // unless the base re-decides to exactly `base_id`, never re-targets --
        // which matters here because the delta bytes are already encoded
        // against `base_id` and a different base would be invalid.
        if chunk_id == base_id
            || !commit_delta_pair(
                &*self.storage,
                &self.delta_written_pairs,
                *chunk_id,
                *base_id,
            )
            .await
        {
            anyhow::bail!(
                "refusing chunk delta: chunk {} base {} would create a cycle or exceed max chain depth {}",
                chunk_id,
                base_id,
                MAX_DELTA_DEPTH
            );
        }

        // Write .meta FIRST — it is the durability anchor for all existence
        // probes (see the add-side writers). A crash between the two writes
        // then leaves an unreachable binary (collected by `gc`) rather than a
        // binary with no routing sidecar, which would make clone 404 when the
        // probe reports the id but the download handler finds no meta.
        let meta_key = format!("chunk-deltas/{}.meta", chunk_id.to_hex());
        let meta_data = format!("base:{}", base_id.to_hex());
        if let Err(e) = self.storage.put(&meta_key, meta_data.as_bytes()).await
            && !self.storage.exists(&meta_key).await.unwrap_or(false)
        {
            // Drop the registration, or the guard refuses a legitimate delta
            // for this chunk later in the same process.
            rollback_delta_pair(&self.delta_written_pairs, *chunk_id, *base_id).await;
            return Err(anyhow::anyhow!("Failed to store chunk delta meta: {}", e));
        }

        let delta_key = format!("chunk-deltas/{}", chunk_id.to_hex());
        if let Err(e) = self.storage.put(&delta_key, compressed_delta_bytes).await
            && !self.storage.exists(&delta_key).await.unwrap_or(false)
        {
            // Remove the routing sidecar so the chunk is not permanently
            // misrouted to a missing binary.
            // Roll back ONLY if the sidecar is really gone. The in-memory
            // graph must stay a superset of the on-disk edges: the depth
            // guard reads it, so an edge still on disk but missing from
            // memory makes the guard undercount and admit a chain past
            // MAX_DELTA_DEPTH. Keeping an edge whose binary never landed is
            // the safe direction to be wrong in - it only makes the guard
            // refuse a delta it could have taken.
            match self.storage.delete(&meta_key).await {
                Ok(()) => {
                    rollback_delta_pair(&self.delta_written_pairs, *chunk_id, *base_id).await;
                }
                Err(del_err) => warn!(
                    chunk_id = %chunk_id,
                    base_id = %base_id,
                    error = %del_err,
                    "failed to remove delta routing sidecar after a failed delta write; \
                     keeping the in-memory edge so the depth guard stays conservative"
                ),
            }
            return Err(anyhow::anyhow!("Failed to store chunk delta: {}", e));
        }

        Ok(())
    }

    /// Read a locally stored chunk-delta as raw compressed bytes + its base OID,
    /// without rematerializing the full chunk.
    ///
    /// Used by the push path to ship deltas over the wire (via
    /// `PUT /chunk-deltas/:id`) instead of paying the rematerialize-and-upload
    /// cost through `get_compressed_chunk` → `/chunks/:id`, which was the
    /// regression that made cloned repos report near-zero compression savings.
    ///
    /// Returns `Ok(None)` when the chunk is not stored as a delta locally.
    /// Returns `Err` only when the `.meta` sidecar is present but malformed
    /// or the payload file cannot be read — both are corruption signals that
    /// the caller should surface rather than paper over.
    pub async fn get_local_chunk_delta_raw(
        &self,
        chunk_id: &Oid,
    ) -> anyhow::Result<Option<(Oid, Vec<u8>)>> {
        let meta_key = format!("chunk-deltas/{}.meta", chunk_id.to_hex());
        let meta_bytes = match self.storage.get(&meta_key).await {
            Ok(b) => b,
            Err(_) => return Ok(None),
        };
        let meta_str = std::str::from_utf8(&meta_bytes).map_err(|e| {
            anyhow::anyhow!("chunk-delta meta for {} is not valid utf8: {}", chunk_id, e)
        })?;
        let base_hex = meta_str.trim().strip_prefix("base:").ok_or_else(|| {
            anyhow::anyhow!("chunk-delta meta for {} missing 'base:' prefix", chunk_id)
        })?;
        let base_id = Oid::from_hex(base_hex).map_err(|e| {
            anyhow::anyhow!(
                "chunk-delta meta for {} has invalid base hex: {}",
                chunk_id,
                e
            )
        })?;

        let delta_key = format!("chunk-deltas/{}", chunk_id.to_hex());
        let delta_bytes = self.storage.get(&delta_key).await.map_err(|e| {
            anyhow::anyhow!(
                "chunk-delta meta present but payload missing for {}: {}",
                chunk_id,
                e
            )
        })?;
        Ok(Some((base_id, delta_bytes)))
    }
}

/// Compute the sibling `.mgtmp` temp path used by `read_to_file` for an
/// atomic write. Appends to the *full* file name rather than using
/// `Path::with_extension`, which replaces the extension and would collide
/// differently-named files sharing a stem (e.g. `a.psd` and `a.txt` would
/// both become `a.mgtmp`). Same directory as `path`, so the eventual
/// rename is a same-filesystem, atomic operation.
fn atomic_tmp_path(path: &std::path::Path) -> anyhow::Result<std::path::PathBuf> {
    let file_name = path.file_name().ok_or_else(|| {
        anyhow::anyhow!("read_to_file: path has no file name: {}", path.display())
    })?;
    let mut tmp_name = file_name.to_os_string();
    tmp_name.push(".mgtmp");
    Ok(path.with_file_name(tmp_name))
}

/// Rename `tmp_path` into place at `path`. Tolerates a transient
/// Windows rename failure (e.g. destination locked by an AV scan or a
/// concurrent reader) with one remove-destination-and-retry, mirroring the
/// spirit of `LocalBackend`'s CAS rename retry. On any final failure,
/// best-effort removes `tmp_path` before returning the error — a failed
/// `read_to_file` never leaves the tmp file behind.
async fn finalize_atomic_write(
    tmp_path: &std::path::Path,
    path: &std::path::Path,
) -> anyhow::Result<()> {
    if let Err(first_err) = tokio::fs::rename(tmp_path, path).await {
        let _ = tokio::fs::remove_file(path).await;
        if let Err(retry_err) = tokio::fs::rename(tmp_path, path).await {
            let _ = tokio::fs::remove_file(tmp_path).await;
            return Err(anyhow::anyhow!(
                "Failed to rename {} to {}: {} (retry: {})",
                tmp_path.display(),
                path.display(),
                first_err,
                retry_err
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod read_to_file_atomicity_tests {
    use super::*;
    use crate::chunking::ChunkStrategy;
    use mediagit_storage::mock::MockBackend;
    use std::sync::Arc as StdArc;

    /// F1: a chunked `read_to_file` that fails partway (missing chunk) must
    /// leave no partial content at the final path and no stray `.mgtmp`
    /// sibling — the write goes to a tmp file first and is only renamed
    /// into place after every chunk is verified.
    #[tokio::test]
    async fn read_to_file_error_leaves_no_partial_final() {
        let storage = StdArc::new(MockBackend::new());
        let odb = ObjectDatabase::with_optimizations(
            storage.clone(),
            100,
            Some(ChunkStrategy::Fixed { size: 256 * 1024 }),
            false,
            0,
        );

        // 2MB of varied content so it chunks into several pieces.
        let data: Vec<u8> = (0..2 * 1024 * 1024).map(|i| (i % 251) as u8).collect();
        let oid = odb
            .write_chunked(ObjectType::Blob, &data, "big.bin")
            .await
            .expect("write_chunked should succeed");

        // Confirm this actually went through the chunked path.
        let manifest_key = format!("manifests/{}", oid.to_hex());
        assert!(
            storage.exists(&manifest_key).await.unwrap(),
            "test setup expected a chunked object"
        );

        // Delete one chunk so reconstruction fails partway through.
        let manifest_data = storage.get(&manifest_key).await.unwrap();
        let manifest: crate::chunking::ChunkManifest =
            crate::chunking::ChunkManifest::from_bytes(&manifest_data).unwrap();
        let victim_chunk = &manifest.chunks[manifest.chunks.len() / 2];
        storage
            .delete(&format!("chunks/{}", victim_chunk.id.to_hex()))
            .await
            .unwrap();

        let tmp_dir = tempfile::TempDir::new().unwrap();
        let dest = tmp_dir.path().join("existing.bin");
        std::fs::write(&dest, b"OLD CONTENT").unwrap();

        let result = odb.read_to_file(&oid, &dest).await;
        assert!(result.is_err(), "read_to_file must fail: chunk missing");

        // Pre-existing content at the final path must be untouched.
        let on_disk = std::fs::read(&dest).unwrap();
        assert_eq!(
            on_disk, b"OLD CONTENT",
            "final path must retain its old content after a failed read_to_file"
        );

        // No stray .mgtmp sibling.
        for entry in std::fs::read_dir(tmp_dir.path()).unwrap() {
            let path = entry.unwrap().path();
            assert!(
                !path.to_string_lossy().ends_with(".mgtmp"),
                "stray tmp file left behind: {}",
                path.display()
            );
        }
    }

    /// F1: `read_to_file` for a non-chunked object must atomically replace
    /// an existing destination file's content (exercises Windows
    /// rename-replace via the tmp-file-then-rename path).
    #[tokio::test]
    async fn read_to_file_overwrites_existing_dest() {
        let storage = StdArc::new(MockBackend::new());
        let odb = ObjectDatabase::new(storage, 100);

        let data = b"brand new content".to_vec();
        let oid = odb.write(ObjectType::Blob, &data).await.unwrap();

        let tmp_dir = tempfile::TempDir::new().unwrap();
        let dest = tmp_dir.path().join("existing.txt");
        std::fs::write(&dest, b"stale content that is longer than the new content").unwrap();

        let bytes_written = odb.read_to_file(&oid, &dest).await.unwrap();
        assert_eq!(bytes_written, data.len() as u64);

        let on_disk = std::fs::read(&dest).unwrap();
        assert_eq!(on_disk, data);

        // No stray .mgtmp sibling after a successful write.
        for entry in std::fs::read_dir(tmp_dir.path()).unwrap() {
            let path = entry.unwrap().path();
            assert!(
                !path.to_string_lossy().ends_with(".mgtmp"),
                "stray tmp file left behind: {}",
                path.display()
            );
        }
    }
}

/// End-to-end guard for the unbounded chunk-delta chain defect.
///
/// The unit tests around `resolve_delta_base` prove the policy; these drive
/// the real `add` write paths, because the defect was not in the policy (there
/// wasn't one) but in every writer independently forgetting depth while
/// remembering cycles.
#[cfg(test)]
mod chunk_delta_depth_tests {
    use super::*;
    use crate::chunking::ChunkStrategy;
    use mediagit_storage::mock::MockBackend;
    use std::sync::Arc as StdArc;

    /// Deepest `chunk-deltas/` chain currently on disk, cycle-safe.
    async fn max_chain_depth(storage: &StdArc<MockBackend>) -> usize {
        let metas = storage.list_objects("chunk-deltas/").await.unwrap();
        let mut worst = 0usize;
        for key in metas.iter().filter(|k| k.ends_with(".meta")) {
            let hex = key
                .trim_start_matches("chunk-deltas/")
                .trim_end_matches(".meta");
            let Ok(start) = Oid::from_hex(hex) else {
                continue;
            };
            let walk = chunk_delta_chain_walk(&**storage, start, None).await;
            assert!(
                !walk.truncated,
                "chain from {start} is unresolvable (cycle or over-cap) — \
                 every chain an add writes must terminate at a full chunk"
            );
            worst = worst.max(walk.depth);
        }
        worst
    }

    /// A run of incrementally-edited similar payloads is exactly the shape
    /// that produced the 624 MiB `psds` repo that could not be pushed: each
    /// new chunk nominates the previous one, which is itself a delta.
    ///
    /// Before the depth guard this grew without bound until `get_chunk`
    /// refused to reconstruct — so with the guard reverted, this test fails.
    #[tokio::test]
    async fn add_of_many_similar_versions_never_exceeds_max_delta_depth() {
        let storage = StdArc::new(MockBackend::new());
        let odb = ObjectDatabase::with_optimizations(
            storage.clone(),
            10_000_000,
            Some(ChunkStrategy::Fixed { size: 256 * 1024 }),
            true,
            0,
        );

        // 30 versions, each a small edit of the last: >2 chunks apiece so this
        // takes the parallel writer, the path the reported failure came from.
        let mut content: Vec<u8> = (0..4 * 1024 * 1024).map(|i| (i % 251) as u8).collect();
        let mut oids = Vec::new();
        for v in 0..30usize {
            for k in 0..64usize {
                content[v * 997 + k] = ((v * 7 + k) | 0x80) as u8;
            }
            let oid = odb
                .write_chunked_parallel(ObjectType::Blob, &content, "asset.psd")
                .await
                .expect("chunked write must succeed");
            oids.push((oid, content.clone()));
        }

        let delta_count = storage
            .list_objects("chunk-deltas/")
            .await
            .unwrap()
            .iter()
            .filter(|k| k.ends_with(".meta"))
            .count();
        // Guard against a vacuous pass: if no deltas were written at all, the
        // payloads never took the delta path and the depth assertion below
        // would be trivially true. (The first draft of this test sat under
        // MIN_CHUNK_SIZE and silently measured nothing.)
        assert!(
            delta_count > 0,
            "no chunk deltas were written — this test is not exercising the \
             chain-depth path and proves nothing"
        );

        let depth = max_chain_depth(&storage).await;
        assert!(
            depth <= MAX_DELTA_DEPTH as usize,
            "on-disk chunk-delta chain reached depth {depth}, deeper than the \
             {} `get_chunk` will reconstruct — the repository would be \
             unpushable and unclonable (measured {depth} with the guard \
             disabled, so this assertion is load-bearing)",
            MAX_DELTA_DEPTH
        );

        // Depth alone is not enough: every version must still read back
        // byte-identically, or we bounded the chain by losing data.
        for (oid, expected) in &oids {
            let got = odb
                .read(oid)
                .await
                .unwrap_or_else(|e| panic!("version {oid} unreadable after add: {e}"));
            assert_eq!(&got, expected, "version {oid} round-tripped incorrectly");
        }
    }

    /// Same invariant on the sequential writer (`num_chunks <= 2` routes here).
    ///
    /// Unlike the parallel test above, this is a guard rather than a
    /// reproduction: measured with the depth check disabled, this path still
    /// stays shallow, because `try_store_chunk_as_delta` returns as soon as it
    /// stores a delta and so never registers that chunk in the similarity
    /// detector — a later chunk cannot nominate it. That is incidental, not
    /// designed (the detector is seeded from prior manifests elsewhere, at
    /// depth <= 2), so the assertion stays: it pins the invariant against a
    /// future change to registration order.
    #[tokio::test]
    async fn sequential_add_of_similar_small_files_never_exceeds_max_delta_depth() {
        let storage = StdArc::new(MockBackend::new());
        let odb = ObjectDatabase::with_optimizations(
            storage.clone(),
            10_000_000,
            Some(ChunkStrategy::Fixed {
                size: 2 * 1024 * 1024,
            }),
            true,
            0,
        );

        let mut content: Vec<u8> = (0..4 * 1024 * 1024).map(|i| (i % 251) as u8).collect();
        let mut oids = Vec::new();
        for v in 0..30usize {
            for k in 0..32usize {
                content[v * 101 + k] = ((v * 5 + k) | 0x80) as u8;
            }
            let oid = odb
                .write_chunked(ObjectType::Blob, &content, "small.psd")
                .await
                .expect("chunked write must succeed");
            oids.push((oid, content.clone()));
        }

        let delta_count = storage
            .list_objects("chunk-deltas/")
            .await
            .unwrap()
            .iter()
            .filter(|k| k.ends_with(".meta"))
            .count();
        assert!(
            delta_count > 0,
            "no chunk deltas were written — this test is not exercising the \
                chain-depth path and proves nothing"
        );

        let depth = max_chain_depth(&storage).await;
        assert!(
            depth <= MAX_DELTA_DEPTH as usize,
            "sequential path reached chain depth {depth}, above {}",
            MAX_DELTA_DEPTH
        );
        for (oid, expected) in &oids {
            let got = odb.read(oid).await.expect("version must be readable");
            assert_eq!(&got, expected);
        }
    }

    /// ST-1: a corrupt base chunk must stop reconstruction, not be built on.
    ///
    /// `get_chunk_limited` decompressed the base and applied deltas without
    /// ever checking it against `base_id` — even though chunk ids *are*
    /// content hashes, so the expected digest was right there. Two consumers
    /// then trusted the result: the server streams it to HTTP clients, and
    /// `get_compressed_chunk` re-packs it under the id it was supposed to
    /// have, republishing corruption as authoritative.
    #[tokio::test]
    async fn corrupt_base_chunk_fails_instead_of_reconstructing_garbage() {
        let storage = StdArc::new(MockBackend::new());
        let odb = ObjectDatabase::with_optimizations(
            storage.clone(),
            10_000_000,
            Some(ChunkStrategy::Fixed {
                size: 2 * 1024 * 1024,
            }),
            true,
            0,
        );

        // Two similar versions so the second is stored as a delta on the first.
        let mut content: Vec<u8> = (0..4 * 1024 * 1024).map(|i| (i % 251) as u8).collect();
        odb.write_chunked(ObjectType::Blob, &content, "v1.bin")
            .await
            .expect("first write");
        for k in 0..64usize {
            content[1000 + k] = 0xAB;
        }
        let v2 = odb
            .write_chunked(ObjectType::Blob, &content, "v2.bin")
            .await
            .expect("second write");

        // Find a delta and the base it depends on.
        let metas = storage.list_objects("chunk-deltas/").await.unwrap();
        let meta_key = metas
            .iter()
            .find(|k| k.ends_with(".meta"))
            .expect("expected at least one chunk delta");
        let meta = storage.get(meta_key).await.unwrap();
        let meta_txt = String::from_utf8_lossy(&meta);
        let base_hex = meta_txt
            .lines()
            .find_map(|l| l.strip_prefix("base:"))
            .expect("meta should name its base")
            .trim()
            .to_string();

        // Corrupt the base chunk's stored bytes in place.
        let base_key = format!("chunks/{base_hex}");
        let good = storage.get(&base_key).await.expect("base chunk present");
        let mut bad = good.clone();
        let n = bad.len();
        bad[n / 2] ^= 0xFF;
        storage.put(&base_key, &bad).await.unwrap();

        // Read the *delta chunk itself* via `get_chunk`, which is the path
        // `browse.rs` and `get_compressed_chunk` use.
        //
        // Deliberately not `read_chunked`: that verifies every chunk against
        // its manifest id and would catch the corruption on its own, so a
        // test through it passes with or without this guard — it proves the
        // manifest check works, not this one. The exposure is exactly the
        // callers that skip that verification.
        let delta_hex = meta_key
            .trim_start_matches("chunk-deltas/")
            .trim_end_matches(".meta");
        let delta_id = Oid::from_hex(delta_hex).expect("delta id");

        let result = odb.get_chunk(&delta_id).await;
        assert!(
            result.is_err(),
            "get_chunk reconstructed on top of a corrupt base and returned \
                success — those bytes are silently wrong, and \
                get_compressed_chunk would re-pack them under a valid id"
        );

        let _ = v2;
    }
}

/// `compressed_chunk_len` backs the presigned chunk-upload PUT's
/// Content-Length. A wrong answer either 403s a valid upload (undersized) or
/// silently accepts more than intended (oversized) — see the presign_put
/// binding at mediagit-server's transfer.rs.
#[cfg(test)]
mod streamed_put_integrity_tests {
    use super::*;
    use mediagit_storage::mock::MockBackend;
    use std::sync::Arc as StdArc;

    /// Both ways of storing a chunk that arrived from a remote must refuse
    /// bytes that do not hash to the id they are filed under.
    ///
    /// `put_compressed_chunk` decompresses and compares against `chunk_id`
    /// before writing. `put_compressed_chunk_from_file` — the B4 streamed
    /// path, and the DEFAULT one, since `MEDIAGIT_STREAM_CHUNK_TO_DISK`
    /// defaults to 1 — did not, so it wrote whatever was staged.
    ///
    /// The reachable shape is a proxied chunk body that ends early WITHOUT an
    /// error: `Body::from_stream` terminates a chunked response normally when
    /// its stream yields `None`, so a short upstream read reaches the client
    /// as a complete, short body. Its streaming helper returns `Ok(written)`
    /// with no size or hash check — its own doc comment says "the hash check
    /// that would catch it happens later", which was true of the in-memory
    /// sibling and not of this path. A truncated chunk would be stored under
    /// a valid id and only surface later, as corruption, on a different
    /// machine, to someone who did nothing wrong.
    #[tokio::test]
    async fn both_puts_refuse_bytes_that_do_not_match_the_id() {
        let storage = StdArc::new(MockBackend::new());
        let odb = ObjectDatabase::new(storage, 100);

        // An id for one payload, bytes for another. Incompressible-by-detect
        // raw bytes, so `CompressionAlgorithm::detect` reports None and the
        // stored form is the input form — the comparison under test is the
        // hash, not the codec.
        let real = vec![b'x'; 4096];
        let chunk_id = Oid::hash(&real);
        let wrong = vec![b'y'; 4096];
        assert_ne!(Oid::hash(&wrong), chunk_id, "fixture must actually differ");

        let in_memory = odb.put_compressed_chunk(&chunk_id, &wrong).await;
        assert!(
            in_memory.is_err(),
            "in-memory put must reject a chunk whose bytes do not hash to its id"
        );

        let tmp = std::env::temp_dir().join("mg-streamed-put-integrity-test.bin");
        tokio::fs::write(&tmp, &wrong)
            .await
            .expect("stage temp file");
        let streamed = odb.put_compressed_chunk_from_file(&chunk_id, &tmp).await;
        let _ = tokio::fs::remove_file(&tmp).await;
        assert!(
            streamed.is_err(),
            "streamed put must reject the same bytes the in-memory put rejected; \
             it is the default clone-fallback path, so an unverified write here \
             stores corruption under a valid id"
        );
    }
}

#[cfg(test)]
mod compressed_chunk_len_tests {
    use super::*;
    use crate::chunking::ChunkStrategy;
    use mediagit_storage::mock::MockBackend;
    use std::sync::Arc as StdArc;

    /// The compressed length must match what `get_compressed_chunk` actually
    /// returns — not the manifest's uncompressed size. Repeated bytes are
    /// used specifically so zlib compresses them well below the input size;
    /// this test would still pass if `compressed_chunk_len` wrongly returned
    /// the uncompressed length unless the two are asserted distinct first.
    #[tokio::test]
    async fn matches_get_compressed_chunk_for_compressible_data() {
        let storage = StdArc::new(MockBackend::new());
        let odb = ObjectDatabase::new(storage, 100);

        let data: Vec<u8> = vec![b'a'; 64 * 1024];
        let chunk_id = Oid::hash(&data);
        odb.write_full_chunk(&chunk_id, &data)
            .await
            .expect("write_full_chunk should succeed");

        let actual = odb
            .get_compressed_chunk(&chunk_id)
            .await
            .expect("chunk was just written");

        assert_ne!(
            actual.len(),
            data.len(),
            "test setup invalid: compressible data must compress to a \
             different size, or this test can't distinguish compressed \
             from uncompressed length"
        );

        let len = odb
            .compressed_chunk_len(&chunk_id)
            .await
            .expect("loose chunk must report a length");
        assert_eq!(
            len,
            actual.len() as u64,
            "compressed_chunk_len must equal the actual bytes get_compressed_chunk sends"
        );
    }

    // RED-VERIFY (documented, not executed): if `compressed_chunk_len`
    // returned the manifest/uncompressed size instead of `head`-ing the
    // stored object, this test fails — `len` above would equal
    // `data.len()` (65536), not `actual.len()` (much smaller after zlib).

    /// A chunk with no loose copy at `chunks/<hex>` — because it is
    /// delta-encoded — must yield `None` rather than a guessed length.
    #[tokio::test]
    async fn returns_none_for_delta_encoded_chunk() {
        let storage = StdArc::new(MockBackend::new());
        let odb = ObjectDatabase::with_optimizations(
            storage.clone(),
            10_000_000,
            Some(ChunkStrategy::Fixed {
                size: 2 * 1024 * 1024,
            }),
            true,
            0,
        );

        // Two similar versions so the second is stored as a delta on the first
        // (mirrors corrupt_base_chunk_fails_instead_of_reconstructing_garbage,
        // which is a proven-reliable delta-producing setup in this file).
        let mut content: Vec<u8> = (0..4 * 1024 * 1024).map(|i| (i % 251) as u8).collect();
        odb.write_chunked(ObjectType::Blob, &content, "v1.bin")
            .await
            .expect("first write");
        for k in 0..64usize {
            content[1000 + k] = 0xAB;
        }
        odb.write_chunked(ObjectType::Blob, &content, "v2.bin")
            .await
            .expect("second write");

        let metas = storage.list_objects("chunk-deltas/").await.unwrap();
        let meta_key = metas
            .iter()
            .find(|k| k.ends_with(".meta"))
            .expect("expected at least one chunk delta");
        let delta_hex = meta_key
            .trim_start_matches("chunk-deltas/")
            .trim_end_matches(".meta");
        let delta_id = Oid::from_hex(delta_hex).expect("delta id");

        assert_eq!(
            odb.compressed_chunk_len(&delta_id).await,
            None,
            "a delta-encoded chunk has no loose object at chunks/<hex> and \
             must not report a guessed length"
        );
    }

    /// A chunk id that was never written at all is likewise `None`.
    #[tokio::test]
    async fn returns_none_for_absent_chunk() {
        let storage = StdArc::new(MockBackend::new());
        let odb = ObjectDatabase::new(storage, 100);

        let absent_id = Oid::hash(b"never written");
        assert_eq!(odb.compressed_chunk_len(&absent_id).await, None);
    }
}
