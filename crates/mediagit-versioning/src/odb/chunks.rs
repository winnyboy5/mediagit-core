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

        if let Some((base_id, score)) = similar {
            // Refuse self-loops and cycles. Without this, parallel adds of
            // similar chunks can produce A→B and B→A on disk, which makes
            // both unreadable (see chunk delta chain reconstruction).
            if base_id == chunk.id
                || chunk_delta_chain_contains_impl(&*self.storage, base_id, chunk.id).await
            {
                debug!(
                    chunk_id = %chunk.id,
                    base_id = %base_id,
                    "Refusing chunk delta — would create cycle, falling back to full chunk"
                );
                let mut detector = self.similarity_detector.write().await;
                detector.add_object(chunk_meta);
                return Ok(false);
            }

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
                    // The lock is held through the chain re-walk AND the meta write:
                    // the walk at the top of this fn races with concurrent writers
                    // (three parallel writes can form A→B→C→A with every pre-walk
                    // passing, because no meta is on disk yet). Serializing
                    // [walk + meta write] means whichever write closes a loop sees
                    // the completed chain and refuses.
                    let mut pairs = self.delta_written_pairs.lock().await;
                    if pairs.contains(&(base_id, chunk.id)) {
                        return Ok(false);
                    }
                    if chunk_delta_chain_contains_impl(&*self.storage, base_id, chunk.id).await {
                        drop(pairs);
                        debug!(
                            chunk_id = %chunk.id,
                            base_id = %base_id,
                            "Refusing chunk delta at commit — concurrent writes would close a cycle"
                        );
                        let mut detector = self.similarity_detector.write().await;
                        detector.add_object(chunk_meta);
                        return Ok(false);
                    }
                    pairs.insert((chunk.id, base_id));

                    // Write .meta FIRST — it is the durability anchor for all existence
                    // probes (odb.rs:exists, check_chunk_deltas_exist). Writing meta before
                    // the binary means a crash between the two writes leaves an unreachable
                    // binary (collected by `gc`) rather than a binary with no routing sidecar
                    // (which would cause clone 404 when the probe correctly returns the id
                    // but the download handler sees no meta and returns NOT_FOUND).
                    let meta_key = format!("chunk-deltas/{}.meta", chunk.id.to_hex());
                    let meta_data = format!("base:{}", base_id.to_hex());
                    if let Err(e) = self.storage.put(&meta_key, meta_data.as_bytes()).await {
                        if !self.storage.exists(&meta_key).await.unwrap_or(false) {
                            return Err(anyhow::anyhow!("Failed to store chunk delta meta: {}", e));
                        }
                    }
                    drop(pairs);

                    if let Err(e) = self.storage.put(&delta_key, &compressed_delta).await {
                        // Best-effort cleanup: remove the .meta we already committed so the
                        // chunk is not permanently misrouted. If the delete also fails, gc
                        // will collect the orphaned sidecar on next run.
                        let _ = self.storage.delete(&meta_key).await;
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
                        if let Some(result) = smart_comp.compress_by_codec(&chunk.data, codec_hint)
                        {
                            result.map_err(|e| {
                                anyhow::anyhow!(
                                    "Failed to compress chunk {} (codec): {}",
                                    chunk_key,
                                    e
                                )
                            })?
                        } else {
                            // Unknown codec → fall back to file-level strategy
                            let chunk_comp_type = if !filename.is_empty() {
                                CompressionObjectType::from_path(filename)
                            } else {
                                CompressionObjectType::Unknown
                            };
                            smart_comp
                                .compress_typed_with_size(&chunk.data, chunk_comp_type)
                                .map_err(|e| {
                                    anyhow::anyhow!("Failed to compress chunk {}: {}", chunk_key, e)
                                })?
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
        let manifest_data = crate::format::serialize(&manifest).map_err(|e| {
            anyhow::anyhow!("Failed to serialize chunk manifest for {}: {}", oid, e)
        })?;
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
                    if let Some(base_id) = base_oid_opt {
                        // Cycle prevention: refuse self-loop and any base whose
                        // existing on-disk chain leads back to this chunk. Without
                        // this guard, two parallel encoders processing similar
                        // chunks can produce mutually-referencing deltas (A→B and
                        // B→A) that fail to reconstruct on read.
                        let cycle_risk = base_id == chunk.id
                            || chunk_delta_chain_contains_impl(&*storage, base_id, chunk.id).await;
                        if cycle_risk {
                            debug!(
                                chunk_id = %chunk.id,
                                base_id = %base_id,
                                "Parallel: refusing chunk delta to prevent cycle"
                            );
                        } else {
                            let base_key = format!("chunks/{}", base_id.to_hex());
                            // Check decompressed base chunk cache before hitting storage
                            let base_data_arc =
                                if let Some(cached) = base_chunk_cache.get(&base_id).await {
                                    Some(cached)
                                } else if let Ok(base_compressed) = storage.get(&base_key).await {
                                    let decompressed = if let Some(ref smart) = smart_comp {
                                        decompress_typed_blocking(smart.clone(), base_compressed)
                                            .await
                                            .ok()
                                    } else {
                                        decompress_blocking(compressor.clone(), base_compressed)
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
                                } else {
                                    None
                                };

                            if let Some(base_data) = base_data_arc {
                                let delta = DeltaEncoder::encode(&base_data, &chunk.data);
                                let delta_bytes = delta.to_bytes();
                                let delta_ratio =
                                    delta_bytes.len() as f64 / chunk.data.len() as f64;

                                let threshold =
                                    delta_ratio_threshold(chunk.codec_hint, chunk.chunk_type);
                                if delta_ratio < threshold {
                                    let delta_key = format!("chunk-deltas/{}", chunk.id.to_hex());
                                    let compressed_delta = if let Some(ref smart) = smart_comp {
                                        smart
                                            .compress_typed(
                                                &delta_bytes,
                                                CompressionObjectType::Unknown,
                                            )
                                            .map_err(|e| anyhow::anyhow!("Compress delta: {}", e))?
                                    } else {
                                        compressor
                                            .compress(&delta_bytes)
                                            .map_err(|e| anyhow::anyhow!("Compress delta: {}", e))?
                                    };

                                    // TOCTOU guard FIRST: check+register before any I/O.
                                    // Lock held through the chain re-walk AND the meta
                                    // write: the pre-walk above races with concurrent
                                    // writers (three parallel writes can form A→B→C→A
                                    // with every pre-walk passing, since no meta is on
                                    // disk yet). Serializing [walk + meta write] means
                                    // whichever write closes a loop sees the completed
                                    // chain and refuses. Only the small meta put happens
                                    // under the lock; the delta binary put stays outside.
                                    let mut pairs = delta_pairs.lock().await;
                                    let should_write = if pairs.contains(&(base_id, chunk.id)) {
                                        debug!(
                                            chunk_id = %chunk.id,
                                            base_id = %base_id,
                                            "Parallel: TOCTOU delta race — reverse pair committed; skipping write"
                                        );
                                        false
                                    } else if chunk_delta_chain_contains_impl(
                                        &*storage, base_id, chunk.id,
                                    )
                                    .await
                                    {
                                        debug!(
                                            chunk_id = %chunk.id,
                                            base_id = %base_id,
                                            "Parallel: refusing chunk delta at commit — concurrent writes would close a cycle"
                                        );
                                        false
                                    } else {
                                        pairs.insert((chunk.id, base_id));
                                        true
                                    };

                                    if should_write {
                                        // Write .meta FIRST (durability anchor — see the
                                        // sequential path above), still under the lock so
                                        // concurrent chain walks observe it atomically.
                                        let meta_key =
                                            format!("chunk-deltas/{}.meta", chunk.id.to_hex());
                                        let meta_data = format!("base:{}", base_id.to_hex());
                                        if let Err(e) =
                                            storage.put(&meta_key, meta_data.as_bytes()).await
                                        {
                                            if !storage.exists(&meta_key).await.unwrap_or(false) {
                                                return Err(anyhow::anyhow!(
                                                    "Store delta meta: {}",
                                                    e
                                                ));
                                            }
                                        }
                                        drop(pairs);

                                        // Tolerate concurrent writes: if put fails but chunk exists, treat as dedup
                                        if let Err(e) =
                                            storage.put(&delta_key, &compressed_delta).await
                                        {
                                            if !storage.exists(&delta_key).await.unwrap_or(false) {
                                                // Remove the routing sidecar so the chunk is
                                                // not permanently misrouted to a missing binary.
                                                let _ = storage.delete(&meta_key).await;
                                                return Err(anyhow::anyhow!("Store delta: {}", e));
                                            }
                                        }

                                        debug!(
                                            chunk_id = %chunk.id,
                                            base_id = %base_id,
                                            delta_ratio,
                                            "Parallel: stored chunk as delta"
                                        );
                                        stored_as_delta = true;
                                    } else {
                                        drop(pairs);
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
        let manifest_data = crate::format::serialize(&manifest)
            .map_err(|e| anyhow::anyhow!("Failed to serialize manifest: {}", e))?;
        self.storage
            .put(&manifest_key, &manifest_data)
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
        let num_workers = num_cpus::get().clamp(2, 16);
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
                    let chunk_exists = storage.exists(&chunk_key).await.unwrap_or(false);
                    let delta_exists = storage.exists(&delta_meta_key).await.unwrap_or(false);

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
                    if let Some(base_id) = base_oid_opt {
                        // Cycle prevention (see parallel non-streaming variant
                        // above for full rationale): refuse self-loops and any
                        // base whose chain leads back to this chunk.
                        let cycle_risk = base_id == chunk.id
                            || chunk_delta_chain_contains_impl(&*storage, base_id, chunk.id).await;
                        if cycle_risk {
                            debug!(
                                chunk_id = %chunk.id,
                                base_id = %base_id,
                                "Streaming parallel: refusing chunk delta to prevent cycle"
                            );
                        } else {
                            let base_key = format!("chunks/{}", base_id.to_hex());
                            // Check decompressed base chunk cache before hitting storage
                            let base_data_arc =
                                if let Some(cached) = base_chunk_cache.get(&base_id).await {
                                    Some(cached)
                                } else if let Ok(base_compressed) = storage.get(&base_key).await {
                                    let decompressed = if let Some(ref smart) = smart_comp {
                                        decompress_typed_blocking(smart.clone(), base_compressed)
                                            .await
                                            .ok()
                                    } else {
                                        decompress_blocking(compressor.clone(), base_compressed)
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
                                } else {
                                    None
                                };

                            if let Some(base_data) = base_data_arc {
                                let delta = DeltaEncoder::encode(&base_data, &chunk.data);
                                let delta_bytes = delta.to_bytes();
                                let delta_ratio =
                                    delta_bytes.len() as f64 / chunk.data.len() as f64;

                                let threshold =
                                    delta_ratio_threshold(chunk.codec_hint, chunk.chunk_type);
                                if delta_ratio < threshold {
                                    let delta_key = format!("chunk-deltas/{}", chunk.id.to_hex());
                                    let compressed_delta = if let Some(ref smart) = smart_comp {
                                        smart
                                            .compress_typed(
                                                &delta_bytes,
                                                CompressionObjectType::Unknown,
                                            )
                                            .map_err(|e| anyhow::anyhow!("Compress delta: {}", e))?
                                    } else {
                                        compressor
                                            .compress(&delta_bytes)
                                            .map_err(|e| anyhow::anyhow!("Compress delta: {}", e))?
                                    };

                                    // TOCTOU guard FIRST: check+register before any I/O.
                                    // Lock held through the chain re-walk AND the meta
                                    // write — same cycle-closing race as the other two
                                    // chunk-delta write sites (see the sequential path).
                                    let mut pairs = delta_pairs.lock().await;
                                    let should_write = if pairs.contains(&(base_id, chunk.id)) {
                                        debug!(
                                            chunk_id = %chunk.id,
                                            base_id = %base_id,
                                            "Streaming parallel: TOCTOU delta race — reverse pair committed; skipping write"
                                        );
                                        false
                                    } else if chunk_delta_chain_contains_impl(
                                        &*storage, base_id, chunk.id,
                                    )
                                    .await
                                    {
                                        debug!(
                                            chunk_id = %chunk.id,
                                            base_id = %base_id,
                                            "Streaming parallel: refusing chunk delta at commit — concurrent writes would close a cycle"
                                        );
                                        false
                                    } else {
                                        pairs.insert((chunk.id, base_id));
                                        true
                                    };

                                    if should_write {
                                        // Write .meta FIRST (durability anchor), still
                                        // under the lock so concurrent chain walks
                                        // observe it atomically.
                                        let meta_key =
                                            format!("chunk-deltas/{}.meta", chunk.id.to_hex());
                                        let meta_data = format!("base:{}", base_id.to_hex());
                                        if let Err(e) =
                                            storage.put(&meta_key, meta_data.as_bytes()).await
                                        {
                                            if !storage.exists(&meta_key).await.unwrap_or(false) {
                                                return Err(anyhow::anyhow!(
                                                    "Store delta meta: {}",
                                                    e
                                                ));
                                            }
                                        }
                                        drop(pairs);

                                        // Tolerate concurrent writes: if put fails but chunk exists, treat as dedup
                                        if let Err(e) =
                                            storage.put(&delta_key, &compressed_delta).await
                                        {
                                            if !storage.exists(&delta_key).await.unwrap_or(false) {
                                                // Remove the routing sidecar so the chunk is
                                                // not permanently misrouted to a missing binary.
                                                let _ = storage.delete(&meta_key).await;
                                                return Err(anyhow::anyhow!("Store delta: {}", e));
                                            }
                                        }

                                        debug!(
                                            chunk_id = %chunk.id,
                                            base_id = %base_id,
                                            delta_ratio,
                                            "Streaming parallel: stored chunk as delta"
                                        );
                                        stored_as_delta = true;
                                    } else {
                                        drop(pairs);
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
                    if !stored_as_delta {
                        let codec_hint = to_chunk_codec_hint(chunk.codec_hint, chunk.chunk_type);
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
                                    if let Some(result) =
                                        smart.compress_by_codec(&chunk_data, codec_hint)
                                    {
                                        result.map_err(|e| {
                                            anyhow::anyhow!("Compress chunk (codec): {}", e)
                                        })
                                    } else {
                                        smart
                                            .compress_typed_with_size(&chunk_data, comp_type)
                                            .map_err(|e| anyhow::anyhow!("Compress chunk: {}", e))
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
                            if let Some(result) = smart.compress_by_codec(&chunk.data, codec_hint) {
                                result
                                    .map_err(|e| anyhow::anyhow!("Compress chunk (codec): {}", e))?
                            } else {
                                // Unknown codec → fall back to file-level strategy
                                smart
                                    .compress_typed_with_size(&chunk.data, comp_type)
                                    .map_err(|e| anyhow::anyhow!("Compress chunk: {}", e))?
                            }
                        } else if compression_enabled {
                            compressor
                                .compress(&chunk.data)
                                .map_err(|e| anyhow::anyhow!("Compress chunk: {}", e))?
                        } else {
                            chunk.data.clone()
                        };

                        // Tolerate concurrent writes
                        if let Err(e) = storage.put(&chunk_key, &data_to_store).await {
                            if !storage.exists(&chunk_key).await.unwrap_or(false) {
                                return Err(anyhow::anyhow!("Store chunk: {}", e));
                            }
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

        let manifest_data = crate::format::serialize(&manifest)?;
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
        let manifest: ChunkManifest = crate::format::deserialize(&manifest_data)
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

            // Verify chunk integrity (hash + size)
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
            let manifest: ChunkManifest = crate::format::deserialize(&manifest_data)
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
            let manifest: ChunkManifest = crate::format::deserialize(&manifest_data)
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
        let manifest: crate::chunking::ChunkManifest =
            crate::format::deserialize(&manifest_data)
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
        // Walk the delta chain iteratively (no async recursion) to bound stack
        // usage regardless of chain length and to detect cycles. Each iteration
        // reads a tiny meta record; the heavy work (base read + delta apply)
        // happens after the full chain is known.
        const MAX_CHUNK_DELTA_DEPTH: usize = MAX_DELTA_DEPTH as usize;
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
            if chain.len() > MAX_CHUNK_DELTA_DEPTH {
                anyhow::bail!(
                    "Chunk delta chain too deep (> {}): chain starting at {}",
                    MAX_CHUNK_DELTA_DEPTH,
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

        // Read the base chunk (non-delta) once
        let base_key = format!("chunks/{}", base_id.to_hex());
        let compressed_base = self.storage.get(&base_key).await?;
        let mut current = if let Some(smart_comp) = &self.smart_compressor {
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
        };

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
            let decompressed = self.get_chunk(chunk_id).await.map_err(|e| {
                anyhow::anyhow!("Failed to reconstruct delta chunk {}: {}", chunk_id, e)
            })?;

            // Re-compress for network transfer
            if let Some(smart_comp) = &self.smart_compressor {
                smart_comp
                    .compress_typed(&decompressed, CompressionObjectType::Unknown)
                    .map_err(|e| {
                        anyhow::anyhow!(
                            "Failed to compress reconstructed chunk {}: {}",
                            chunk_id,
                            e
                        )
                    })
            } else {
                self.compressor.compress(&decompressed).map_err(|e| {
                    anyhow::anyhow!("Failed to compress reconstructed chunk {}: {}", chunk_id, e)
                })
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
        self.storage
            .put(&chunk_key, data)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to store chunk {}: {}", chunk_id, e))
    }

    /// Store a compressed chunk from a local temp file (B4 stream-to-disk).
    ///
    /// Delegates to `StorageBackend::put_file`. On `LocalBackend` this is a
    /// zero-copy atomic rename; cloud backends fall back to reading the file
    /// and uploading.
    pub async fn put_compressed_chunk_from_file(
        &self,
        chunk_id: &Oid,
        path: &std::path::Path,
    ) -> anyhow::Result<()> {
        let chunk_key = format!("chunks/{}", chunk_id.to_hex());
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
        let manifest_data = crate::format::serialize(manifest)
            .map_err(|e| anyhow::anyhow!("Failed to serialize manifest: {}", e))?;
        self.storage
            .put(&manifest_key, &manifest_data)
            .await
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
        if chunk_id == base_id
            || chunk_delta_chain_contains_impl(&*self.storage, *base_id, *chunk_id).await
        {
            anyhow::bail!(
                "would create chunk delta cycle: chunk {} base {}",
                chunk_id,
                base_id
            );
        }

        let delta_key = format!("chunk-deltas/{}", chunk_id.to_hex());
        if let Err(e) = self.storage.put(&delta_key, compressed_delta_bytes).await {
            if !self.storage.exists(&delta_key).await.unwrap_or(false) {
                return Err(anyhow::anyhow!("Failed to store chunk delta: {}", e));
            }
        }

        let meta_key = format!("chunk-deltas/{}.meta", chunk_id.to_hex());
        let meta_data = format!("base:{}", base_id.to_hex());
        if let Err(e) = self.storage.put(&meta_key, meta_data.as_bytes()).await {
            if !self.storage.exists(&meta_key).await.unwrap_or(false) {
                return Err(anyhow::anyhow!("Failed to store chunk delta meta: {}", e));
            }
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
