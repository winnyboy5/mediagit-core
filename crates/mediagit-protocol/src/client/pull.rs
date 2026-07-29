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

impl ProtocolClient {
    /// Pull objects from remote and return pack data with chunked object OIDs
    ///
    /// # Arguments
    /// * `odb` - Local object database
    /// * `remote_ref` - Remote ref to pull
    /// * `local_oids` - List of OIDs we already have locally (for incremental pull)
    ///
    /// Pass local commit OIDs to avoid downloading objects we already have.
    /// If empty, all objects reachable from the remote ref will be downloaded.
    ///
    /// Returns (pack_data, chunked_oids)
    #[deprecated(note = "Use pull_streaming() instead for memory-efficient downloads")]
    #[allow(deprecated)]
    pub async fn pull_with_have(
        &self,
        _odb: &ObjectDatabase,
        remote_ref: &str,
        local_oids: Vec<String>,
    ) -> Result<(Vec<u8>, Vec<Oid>)> {
        // Get remote refs
        let remote_refs = self.get_refs().await?;

        // Find the ref we want
        let ref_info = remote_refs
            .refs
            .iter()
            .find(|r| r.name == remote_ref)
            .ok_or_else(|| anyhow::anyhow!("Remote ref '{}' not found", remote_ref))?;

        // Request objects we don't have
        let want = vec![ref_info.oid.clone()];
        let have = local_oids; // OIDs we already have locally

        self.download_pack(want, have).await
    }
    /// Pull objects from a remote ref (backwards compatible, downloads all objects)
    ///
    /// For incremental pulls, use `pull_streaming` instead.
    #[deprecated(note = "Use pull_streaming() instead for memory-efficient downloads")]
    #[allow(deprecated)]
    pub async fn pull(
        &self,
        odb: &ObjectDatabase,
        remote_ref: &str,
    ) -> Result<(Vec<u8>, Vec<Oid>)> {
        self.pull_with_have(odb, remote_ref, Vec::new()).await
    }
    /// Download a pack file from the server
    ///
    /// Returns (pack_data, chunked_oids) - chunked objects need separate transfer
    #[deprecated(note = "Use download_pack_streaming() instead for memory-efficient downloads")]
    pub async fn download_pack(
        &self,
        want: Vec<String>,
        have: Vec<String>,
    ) -> Result<(Vec<u8>, Vec<Oid>)> {
        // First, send want request
        let want_url = format!("{}/objects/want", self.base_url);
        tracing::debug!("POST {}", want_url);

        let want_req = WantRequest { want, have };

        let response = self
            .client
            .post(&want_url)
            .json(&want_req)
            .send()
            .await
            .context("Failed to send want request")?;

        if !response.status().is_success() {
            anyhow::bail!(
                "POST /objects/want failed with status: {}",
                response.status()
            );
        }

        // Parse the response to get the request_id (required for GET /objects/pack)
        let want_response: WantResponse = response
            .json()
            .await
            .context("Failed to parse want response")?;

        // Then download the pack with the request_id header
        let pack_url = format!("{}/objects/pack", self.base_url);
        tracing::debug!(
            "GET {} (request_id: {})",
            pack_url,
            want_response.request_id
        );

        let response = self
            .client
            .get(&pack_url)
            .header("X-Request-ID", &want_response.request_id)
            .send()
            .await
            .context("Failed to download pack file")?;

        if !response.status().is_success() {
            // Include the server's message. An incomplete-closure refusal is
            // operator-actionable ("run fsck on the server"), and a bare
            // status code strands the user with "500 Internal Server Error"
            // for a condition the server can name precisely.
            let status = response.status();
            let detail = response.text().await.unwrap_or_default();
            let detail = detail.trim();
            if detail.is_empty() {
                anyhow::bail!("GET /objects/pack failed with status: {}", status);
            }
            anyhow::bail!("GET /objects/pack failed ({}): {}", status, detail);
        }

        // Parse X-Chunked-Objects header for large files that need separate transfer
        let chunked_oids: Vec<Oid> = response
            .headers()
            .get("X-Chunked-Objects")
            .and_then(|h| h.to_str().ok())
            .map(|s| {
                s.split(',')
                    .filter_map(|oid_str| Oid::from_hex(oid_str.trim()).ok())
                    .collect()
            })
            .unwrap_or_default();

        if !chunked_oids.is_empty() {
            tracing::info!(
                count = chunked_oids.len(),
                "Received {} chunked objects for separate download",
                chunked_oids.len()
            );
        }

        let pack_data = response.bytes().await.context("Failed to read pack data")?;

        Ok((pack_data.to_vec(), chunked_oids))
    }

    /// Download pack using streaming (memory-efficient for large files)
    ///
    /// This method processes the pack incrementally without loading it entirely into memory.
    /// Objects are written directly to the ODB as they're received.
    ///
    /// Returns the list of chunked objects that need separate transfer.
    pub async fn download_pack_streaming(
        &self,
        odb: &ObjectDatabase,
        want: Vec<String>,
        have: Vec<String>,
    ) -> Result<Vec<Oid>> {
        // Send want request
        let want_url = format!("{}/objects/want", self.base_url);
        tracing::debug!("POST {} (streaming)", want_url);

        let want_req = WantRequest { want, have };

        let response = self
            .client
            .post(&want_url)
            .json(&want_req)
            .send()
            .await
            .context("Failed to send want request")?;

        if !response.status().is_success() {
            anyhow::bail!(
                "POST /objects/want failed with status: {}",
                response.status()
            );
        }

        let want_response: WantResponse = response
            .json()
            .await
            .context("Failed to parse want response")?;

        // Download pack with streaming
        let pack_url = format!("{}/objects/pack", self.base_url);
        tracing::debug!(
            "GET {} (streaming, request_id: {})",
            pack_url,
            want_response.request_id
        );

        let response = self
            .client
            .get(&pack_url)
            .header("X-Request-ID", &want_response.request_id)
            .send()
            .await
            .context("Failed to download pack file")?;

        if !response.status().is_success() {
            // Include the server's message. An incomplete-closure refusal is
            // operator-actionable ("run fsck on the server"), and a bare
            // status code strands the user with "500 Internal Server Error"
            // for a condition the server can name precisely.
            let status = response.status();
            let detail = response.text().await.unwrap_or_default();
            let detail = detail.trim();
            if detail.is_empty() {
                anyhow::bail!("GET /objects/pack failed with status: {}", status);
            }
            anyhow::bail!("GET /objects/pack failed ({}): {}", status, detail);
        }

        // Parse X-Chunked-Objects header
        let chunked_oids: Vec<Oid> = response
            .headers()
            .get("X-Chunked-Objects")
            .and_then(|h| h.to_str().ok())
            .map(|s| {
                s.split(',')
                    .filter_map(|oid_str| Oid::from_hex(oid_str.trim()).ok())
                    .collect()
            })
            .unwrap_or_default();

        if !chunked_oids.is_empty() {
            tracing::info!(
                count = chunked_oids.len(),
                "Received {} chunked objects for separate download",
                chunked_oids.len()
            );
        }

        // Stream response body and write objects via ODB (ensures proper compression)
        use futures::stream::TryStreamExt;
        use tokio_util::io::StreamReader;

        let stream = response.bytes_stream().map_err(std::io::Error::other);

        let stream_reader = StreamReader::new(stream);

        let mut reader = mediagit_versioning::StreamingPackReader::new(stream_reader)
            .await
            .context("Failed to create streaming pack reader")?;

        tracing::info!("Processing streaming pack download");

        let mut object_count = 0;
        while let Some(result) = reader.next_object().await {
            let (_oid, obj_type, data) =
                result.context("Failed to read object from pack stream")?;

            // Write through ODB to ensure proper compression and storage format.
            // PackTransaction bypassed compression, causing read failures.
            odb.write(obj_type, &data)
                .await
                .context("Failed to write object from pack")?;

            object_count += 1;
            if object_count % 100 == 0 {
                tracing::debug!("Downloaded {} objects", object_count);
            }
        }

        tracing::info!(
            "Successfully downloaded {} objects (streaming)",
            object_count
        );

        Ok(chunked_oids)
    }

    /// Pull using streaming (memory-efficient)
    ///
    /// Returns the list of chunked objects that need separate transfer.
    pub async fn pull_streaming(
        &self,
        odb: &ObjectDatabase,
        remote_ref: &str,
        local_oids: Vec<String>,
    ) -> Result<Vec<Oid>> {
        // Get remote refs
        let remote_refs = self.get_refs().await?;

        // Find the ref we want
        let ref_info = remote_refs
            .refs
            .iter()
            .find(|r| r.name == remote_ref)
            .ok_or_else(|| anyhow::anyhow!("Remote ref '{}' not found", remote_ref))?;

        // Request objects we don't have
        let want = vec![ref_info.oid.clone()];
        let have = local_oids;

        self.download_pack_streaming(odb, want, have).await
    }
    /// Download a manifest from the remote server
    pub async fn download_manifest(&self, oid: &Oid) -> Result<ChunkManifest> {
        let url = format!("{}/manifests/{}", self.base_url, oid.to_hex());

        let response = self
            .client
            .get(&url)
            .send()
            .await
            .context(format!("Failed to GET /manifests/{}", oid))?;

        if !response.status().is_success() {
            anyhow::bail!(
                "GET /manifests/{} failed with status: {}",
                oid,
                response.status()
            );
        }

        let data = response.bytes().await?;
        mediagit_versioning::ChunkManifest::from_bytes(&data)
            .context("Failed to deserialize manifest")
    }
    /// Download a single chunk from the remote server
    pub async fn download_chunk(&self, chunk_id: &Oid) -> Result<Vec<u8>> {
        let url = format!("{}/chunks/{}", self.base_url, chunk_id.to_hex());

        let response = self
            .client
            .get(&url)
            .send()
            .await
            .context(format!("Failed to GET /chunks/{}", chunk_id))?;

        if response.status() == reqwest::StatusCode::SERVICE_UNAVAILABLE {
            anyhow::bail!(
                "GET /chunks/{} failed: server storage backend unreachable (503) — verify the storage service (MinIO/S3/Azure) is running and accessible to the server",
                chunk_id
            );
        }
        if !response.status().is_success() {
            anyhow::bail!(
                "GET /chunks/{} failed with status: {}",
                chunk_id,
                response.status()
            );
        }

        Ok(response.bytes().await?.to_vec())
    }
    /// Download all chunks for chunked objects with parallel downloads
    ///
    /// Uses 8 concurrent downloads for optimal throughput (>100MB/s target).
    /// Progress callback receives `(chunks_done, total_manifest_chunks, msg)` for
    /// smooth chunk-level ETA (avoids "211y" caused by object-level reporting).
    ///
    /// Delta-aware: for each manifest, asks the server which chunks exist as
    /// chunk-deltas via `POST /chunk-deltas/check`. Chunks present as deltas
    /// are downloaded via `GET /chunk-deltas/<id>` and persisted via
    /// `odb.write_chunk_delta`, preserving the storage savings the server has.
    /// Falls back to full-chunk downloads for any chunk the server doesn't
    /// report as a delta, and for old servers that don't expose the endpoint.
    pub async fn download_chunked_objects<F>(
        &self,
        odb: &ObjectDatabase,
        chunked_oids: &[Oid],
        mut on_progress: F,
    ) -> Result<(usize, u64)>
    where
        F: FnMut(u64, u64, &str),
    {
        use futures::stream::StreamExt;

        if chunked_oids.is_empty() {
            return Ok((0, 0));
        }

        // Concurrency for parallel chunk downloads. Default 24 paired with
        // MEDIAGIT_RANGE_PARALLEL=4 gives 96 effective TCP streams — enough
        // headroom without opening more sockets than the pool can keep warm.
        // Env var > builder > internal default.
        let concurrent_downloads: usize = std::env::var("MEDIAGIT_DOWNLOAD_CONCURRENCY")
            .ok()
            .and_then(|s| s.parse().ok())
            .filter(|n: &usize| *n > 0)
            .or(self.concurrent_downloads)
            .unwrap_or(32);
        let n_objects = chunked_oids.len();
        let _download_bench = crate::bench::maybe_start("download", concurrent_downloads);

        // Gate: set MEDIAGIT_DOWNLOAD_DIRECT_DISABLE=1 to bypass presigned GET entirely.
        let direct_download_enabled =
            std::env::var("MEDIAGIT_DOWNLOAD_DIRECT_DISABLE").as_deref() != Ok("1");

        // Data-plane client for presigned GET downloads.
        // HTTP/1.1: parallel TCP sockets beat h2 multiplexing for large bodies.
        // Pool size via MEDIAGIT_HTTP_POOL_MAX (see http_pool_max()).
        crate::ensure_crypto_provider();
        let direct_client = reqwest::Client::builder()
            .pool_idle_timeout(std::time::Duration::from_secs(60))
            .pool_max_idle_per_host(http_pool_max())
            .tcp_keepalive(std::time::Duration::from_secs(45))
            .tcp_nodelay(true)
            .http1_only()
            .build()
            // Fallback must be credential-free: self.client carries auth
            // default_headers, which must never reach presigned URLs.
            .unwrap_or_else(|_| reqwest::Client::new());

        // ── Phase 1: manifests (fast — small metadata payloads) ──────────────
        // Download all manifests upfront to know total_chunks before any data
        // transfer. Manifests are tiny (list of chunk IDs + sizes); storing them
        // all is at most ~KB total even for large repos.
        struct ObjectWork {
            oid: Oid,
            manifest: ChunkManifest,
            missing_chunks: Vec<Oid>,
        }

        // Knobs: set MEDIAGIT_PULL_PIPELINE=0 to disable concurrent manifest fetch.
        // MEDIAGIT_PULL_MANIFEST_CONCURRENCY controls max in-flight manifest requests (default 8).
        let pull_pipeline = std::env::var("MEDIAGIT_PULL_PIPELINE")
            .as_deref()
            .unwrap_or("1")
            != "0";
        let manifest_concurrency: usize = std::env::var("MEDIAGIT_PULL_MANIFEST_CONCURRENCY")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(8)
            .max(1);

        let manifest_start = std::time::Instant::now();

        let object_work: Vec<ObjectWork> = if pull_pipeline && n_objects > 1 {
            // Concurrent manifest fetch + chunk existence check.
            // buffer_unordered only requires Send (not 'static), so &ObjectDatabase
            // is safe to capture as long as ObjectDatabase: Sync.
            let http_client = self.client.clone();
            let base_url = self.base_url.clone();
            use futures::StreamExt;
            let results: Vec<Result<ObjectWork>> = futures::stream::iter(chunked_oids.iter())
                .map(|oid| {
                    let http_client = http_client.clone();
                    let base_url = base_url.clone();
                    async move {
                        // 1. Fetch manifest (HTTP — inlined download_manifest body)
                        let url = format!("{}/manifests/{}", base_url, oid.to_hex());
                        let resp = http_client
                            .get(&url)
                            .send()
                            .await
                            .with_context(|| format!("Failed to GET /manifests/{}", oid))?;
                        if !resp.status().is_success() {
                            anyhow::bail!(
                                "GET /manifests/{} failed with status: {}",
                                oid,
                                resp.status()
                            );
                        }
                        let data = resp.bytes().await?;
                        let manifest = ChunkManifest::from_bytes(&data)
                            .context("Failed to deserialize manifest")?;

                        // 2. Check which chunks are already local (parallel filesystem stats).
                        let obj_total = manifest.chunks.len();
                        let missing: Vec<Oid> =
                            futures::stream::iter(manifest.chunks.iter().map(|c| c.id))
                                .map(|id| async move {
                                    if odb.chunk_exists(&id).await.unwrap_or(false) {
                                        None
                                    } else {
                                        Some(id)
                                    }
                                })
                                .buffer_unordered(64)
                                .filter_map(|x| async move { x })
                                .collect()
                                .await;

                        if missing.is_empty() {
                            tracing::debug!(oid = %oid, "All chunks already exist locally");
                        } else {
                            tracing::info!(
                                oid = %oid,
                                missing = missing.len(),
                                total = obj_total,
                                "Downloading missing chunks"
                            );
                        }

                        Ok::<ObjectWork, anyhow::Error>(ObjectWork {
                            oid: *oid,
                            manifest,
                            missing_chunks: missing,
                        })
                    }
                })
                .buffer_unordered(manifest_concurrency)
                .collect()
                .await;
            results.into_iter().collect::<Result<Vec<ObjectWork>>>()?
        } else {
            // Sequential fallback: MEDIAGIT_PULL_PIPELINE=0 or single object.
            let mut work = Vec::with_capacity(n_objects);
            for oid in chunked_oids.iter() {
                let manifest = self.download_manifest(oid).await?;
                let obj_total = manifest.chunks.len();
                // Check which chunks are already local — run in parallel to avoid
                // O(n) sequential filesystem stats on Windows (3ms × 4000 = 12s per object).
                let missing: Vec<Oid> = futures::stream::iter(manifest.chunks.iter().map(|c| c.id))
                    .map(|id| async move {
                        if odb.chunk_exists(&id).await.unwrap_or(false) {
                            None
                        } else {
                            Some(id)
                        }
                    })
                    .buffer_unordered(64)
                    .filter_map(|x| async move { x })
                    .collect()
                    .await;
                if missing.is_empty() {
                    tracing::debug!(oid = %oid, "All chunks already exist locally");
                } else {
                    tracing::info!(
                        oid = %oid,
                        missing = missing.len(),
                        total = obj_total,
                        "Downloading missing chunks"
                    );
                }
                work.push(ObjectWork {
                    oid: *oid,
                    manifest,
                    missing_chunks: missing,
                });
            }
            work
        };

        // Compute totals from collected manifests.
        let total_manifest_chunks: usize =
            object_work.iter().map(|w| w.manifest.chunks.len()).sum();
        let total_manifest_bytes: u64 = object_work
            .iter()
            .flat_map(|w| w.manifest.chunks.iter().map(|c| c.size as u64))
            .sum();

        // Signal total upfront so the progress bar initialises with correct length.
        on_progress(
            0,
            total_manifest_bytes,
            &format!(
                "{} objects, {} total chunks",
                n_objects, total_manifest_chunks
            ),
        );

        if let Some(b) = &_download_bench {
            b.record_manifest_to_first_byte(manifest_start.elapsed());
        }

        // ── Phase 2: chunk download — streaming write, O(concurrent × chunk) RAM
        // Writing each chunk as it arrives (while let Some) instead of collect()
        // means peak RAM = concurrent_downloads × max_chunk_size (≤64 KB on
        // Windows) rather than accumulating every result before any disk write.
        let mut total_chunks_downloaded = 0usize;
        // RP-2: `bytes_downloaded` was declared and displayed but never
        // assigned anywhere, so pull/clone reported no download figure at all.
        // Counted in wire bytes (what actually crossed the network) to match
        // the push side and to keep any derived rate under link capacity.
        let mut total_net_bytes = 0u64;
        let mut bytes_done: u64 = 0;

        for (
            obj_idx,
            ObjectWork {
                oid,
                manifest,
                missing_chunks,
            },
        ) in object_work.into_iter().enumerate()
        {
            let already_local = manifest.chunks.len() - missing_chunks.len();
            // Size hints for range-parallel GET (W5): chunk hex → uncompressed bytes.
            let chunk_size_map: std::sync::Arc<std::collections::HashMap<String, u64>> =
                std::sync::Arc::new(
                    manifest
                        .chunks
                        .iter()
                        .map(|c| (c.id.to_hex(), c.size as u64))
                        .collect(),
                );

            if !missing_chunks.is_empty() {
                // Ask the server which of these chunks are stored as deltas.
                // Best-effort: empty map on old servers / errors.
                let delta_map = self.check_chunk_deltas(&missing_chunks).await;

                // Split into "full chunks" and "delta chunks" based on what the
                // server actually stores. A chunk that the server reports as a
                // delta MUST be downloaded via `/chunk-deltas/<id>` — hitting
                // `/chunks/<id>` for it would 404 because no full copy exists.
                // Delta chains (depth ≥ 2) are fine: all chunks in the chain
                // land locally as deltas in this single pass; reads later
                // follow the chain via the .meta sidecars.
                //
                // Cycle pre-filter still applies: if writing a chunk as a delta
                // would create a cycle with the LOCAL on-disk chain (e.g. a
                // prior partial clone), degrade that one to a full download.
                // If the full endpoint also lacks the object the server is
                // misconfigured — surface the error rather than mask it.
                let mut full_chunks: Vec<Oid> = Vec::new();
                let mut delta_chunks: Vec<Oid> = Vec::new();
                for cid in &missing_chunks {
                    if let Some(&base) = delta_map.get(cid) {
                        let cycle_risk = base == *cid
                            || odb
                                .chunk_delta_chain_contains(&base, cid)
                                .await
                                .unwrap_or(false);
                        if cycle_risk {
                            tracing::debug!(
                                chunk = %cid,
                                base = %base,
                                "Routing to full-chunk download (would create local cycle)"
                            );
                            full_chunks.push(*cid);
                        } else {
                            delta_chunks.push(*cid);
                        }
                    } else {
                        full_chunks.push(*cid);
                    }
                }

                tracing::debug!(
                    full = full_chunks.len(),
                    deltas = delta_chunks.len(),
                    "Split chunk download: full first, then deltas"
                );

                // F7: pack-mode pull path — locate → presign → Range-GET.
                // Falls back to legacy per-chunk path if pack mode returns 0 chunks or errors.
                let cloud_packs_pull = std::env::var("MEDIAGIT_CLOUD_PACKS")
                    .as_deref()
                    .unwrap_or("1")
                    != "0";
                // F7: pack-mode pull returns the set of chunks written.
                // Remaining (not in written_set) fall through to per-chunk path.
                //
                // Build a thread-safe progress callback: pack downloads run concurrently so
                // bytes_done is accumulated via AtomicU64 and synced back after pack completes.
                let pack_bytes = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
                let pack_bytes_clone = pack_bytes.clone();
                let pack_total = total_manifest_bytes;
                let pack_progress_cb: std::sync::Arc<dyn Fn(u64) + Send + Sync> =
                    std::sync::Arc::new(move |chunk_bytes: u64| {
                        pack_bytes_clone
                            .fetch_add(chunk_bytes, std::sync::atomic::Ordering::Relaxed);
                    });
                let pack_written: std::collections::HashSet<mediagit_versioning::Oid> =
                    if cloud_packs_pull && !full_chunks.is_empty() {
                        // Drive the pack future while emitting progress ticks every 500 ms so
                        // the CLI bar advances during the (potentially long) GCS Range-GET phase.
                        // Intermediate credits use compressed bytes (pack_bytes) — an underestimate
                        // vs the uncompressed denominator — so bytes_done is corrected at the end.
                        let mut pack_fut = std::pin::pin!(self.pull_chunks_via_packs(
                            &full_chunks,
                            odb,
                            Some(pack_progress_cb),
                            _download_bench.as_ref()
                        ));
                        let mut tick = tokio::time::interval(std::time::Duration::from_millis(500));
                        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
                        let pack_result = loop {
                            tokio::select! {
                                result = &mut pack_fut => { break result; }
                                _ = tick.tick() => {
                                    let in_flight =
                                        pack_bytes.load(std::sync::atomic::Ordering::Relaxed);
                                    if in_flight > 0 {
                                        on_progress(
                                            bytes_done.saturating_add(in_flight),
                                            pack_total,
                                            "Downloading via packs...",
                                        );
                                    }
                                }
                            }
                        };
                        match pack_result {
                            Ok(written) => {
                                tracing::debug!(chunks = written.len(), "pack-mode pull complete");
                                written
                            }
                            Err(e) => {
                                tracing::warn!(
                                    err = %e,
                                    "pack-mode pull failed; falling back to per-chunk"
                                );
                                std::collections::HashSet::new()
                            }
                        }
                    } else {
                        std::collections::HashSet::new()
                    };
                // Credit uncompressed bytes matching the denominator unit.
                // pack_bytes holds compressed data.len() — wrong unit; use chunk_size_map
                // (ChunkRef.size = uncompressed) so bytes_done reaches total_manifest_bytes.
                // Only credit on success; on partial error fallback covers all full_chunks.
                if !pack_written.is_empty() {
                    bytes_done += pack_written
                        .iter()
                        .map(|oid| chunk_size_map.get(&oid.to_hex()).copied().unwrap_or(0))
                        .sum::<u64>();
                    on_progress(bytes_done, pack_total, "Pack download complete");
                }
                // Chunks not written by pack-mode pull: fall through to per-chunk download.
                let chunks_for_fallback: Vec<Oid> = full_chunks
                    .into_iter()
                    .filter(|c| !pack_written.contains(c))
                    .collect();
                let fallback_hex: Vec<String> =
                    chunks_for_fallback.iter().map(|c| c.to_hex()).collect();

                if direct_download_enabled && !fallback_hex.is_empty() {
                    on_progress(
                        bytes_done,
                        total_manifest_bytes,
                        &format!("Preparing {} download URLs...", fallback_hex.len()),
                    );
                }
                let download_urls = if direct_download_enabled && !fallback_hex.is_empty() {
                    self.request_chunk_download_urls(&fallback_hex).await
                } else {
                    std::collections::HashMap::new()
                };

                // ── Pass A: full chunks not yet written (pack miss or pack disabled) ──
                if !chunks_for_fallback.is_empty() {
                    let download_urls = std::sync::Arc::new(download_urls);
                    let _dl_pass_a_t = std::time::Instant::now();
                    let mut _dl_pass_a_n = 0u64;
                    let mut _dl_pass_a_bytes = 0u64;
                    // B4: stream downloaded bytes directly to a temp file to reduce peak RAM.
                    // Default ON — Windows stress + MinIO/AWS/Azure 148/148 PASS (2026-05-22).
                    // Set MEDIAGIT_STREAM_CHUNK_TO_DISK=0 to revert to RAM buffering.
                    let stream_to_disk = std::env::var("MEDIAGIT_STREAM_CHUNK_TO_DISK")
                        .as_deref()
                        .unwrap_or("1")
                        == "1";
                    let mut stream = futures::stream::iter(chunks_for_fallback)
                        .map(|chunk_id| {
                            let client = self.client.clone();
                            let direct_client = direct_client.clone();
                            let base_url = self.base_url.clone();
                            let download_urls = std::sync::Arc::clone(&download_urls);
                            let chunk_size_map = std::sync::Arc::clone(&chunk_size_map);
                            let odb = odb.clone();
                            async move {
                                let hex = chunk_id.to_hex();
                                let size_hint = chunk_size_map.get(&hex).copied().unwrap_or(0);
                                // Try presigned direct download first; fall back to proxy on any error.
                                if let Some(Some(presigned)) = download_urls.get(&hex) {
                                    match download_chunk_direct(
                                        &direct_client,
                                        &presigned.url,
                                        &presigned.headers,
                                        &hex,
                                        size_hint,
                                    )
                                    .await
                                    {
                                        Ok(data) => {
                                            let net = data.len() as u64;
                                            odb.put_compressed_chunk(&chunk_id, &data).await?;
                                            return Ok::<_, anyhow::Error>((chunk_id, net));
                                        }
                                        Err(e) => {
                                            tracing::debug!(
                                                chunk = %hex,
                                                err = %e,
                                                "Direct download failed; falling back to proxy GET"
                                            );
                                        }
                                    }
                                }
                                // Proxy GET fallback
                                let url = format!("{}/chunks/{}", base_url, hex);
                                let response = client.get(&url).send().await.map_err(|e| {
                                    anyhow::anyhow!("Failed to download chunk {}: {}", chunk_id, e)
                                })?;
                                // F3: server returns 409 when the chunk is delta-only.
                                // This happens when our POST /chunk-deltas/check probe failed
                                // silently and we ended up in the wrong (full-chunk) pass.
                                // Re-route: download via /chunk-deltas/<id> and store as delta.
                                if response.status() == reqwest::StatusCode::CONFLICT {
                                    let body: serde_json::Value =
                                        response.json().await.unwrap_or_default();
                                    let base_hex =
                                        body.get("base_id").and_then(|v| v.as_str()).unwrap_or("");
                                    if let Ok(base_id) = Oid::from_hex(base_hex) {
                                        let delta_url =
                                            format!("{}/chunk-deltas/{}", base_url, hex);
                                        match client.get(&delta_url).send().await {
                                            Ok(dr) if dr.status().is_success() => {
                                                let delta_bytes = dr.bytes().await?.to_vec();
                                                let net = delta_bytes.len() as u64;
                                                odb.write_chunk_delta(
                                                    &chunk_id,
                                                    &base_id,
                                                    &delta_bytes,
                                                )
                                                .await?;
                                                return Ok::<_, anyhow::Error>((chunk_id, net));
                                            }
                                            _ => {}
                                        }
                                    }
                                    anyhow::bail!(
                                        "GET /chunks/{} returned 409 but delta reclassification failed",
                                        chunk_id
                                    );
                                }
                                if !response.status().is_success() {
                                    anyhow::bail!(
                                        "GET /chunks/{} failed with status: {}",
                                        chunk_id,
                                        response.status()
                                    );
                                }
                                if stream_to_disk {
                                    // B4: stream proxy response to temp file to reduce peak RAM.
                                    use futures::StreamExt as _;
                                    use tokio::io::AsyncWriteExt as _;
                                    let temp_path =
                                        std::env::temp_dir().join(format!("mg-chunk-{}", hex));
                                    let mut f = tokio::fs::File::create(&temp_path)
                                        .await
                                        .map_err(|e| anyhow::anyhow!("B4 temp create: {}", e))?;
                                    let mut written = 0u64;
                                    let mut byte_stream = response.bytes_stream();
                                    while let Some(ch) = byte_stream.next().await {
                                        let b = ch.map_err(|e| {
                                            anyhow::anyhow!("Download stream error: {}", e)
                                        })?;
                                        f.write_all(&b).await.map_err(|e| {
                                            anyhow::anyhow!("Write temp chunk: {}", e)
                                        })?;
                                        written += b.len() as u64;
                                    }
                                    f.flush().await?;
                                    drop(f);
                                    odb.put_compressed_chunk_from_file(&chunk_id, &temp_path)
                                        .await?;
                                    let _ = tokio::fs::remove_file(&temp_path).await;
                                    Ok::<_, anyhow::Error>((chunk_id, written))
                                } else {
                                    let data = response.bytes().await.map_err(|e| {
                                        anyhow::anyhow!("Failed to read chunk {}: {}", chunk_id, e)
                                    })?;
                                    let net = data.len() as u64;
                                    odb.put_compressed_chunk(&chunk_id, &data).await?;
                                    Ok::<_, anyhow::Error>((chunk_id, net))
                                }
                            }
                        })
                        .buffer_unordered(concurrent_downloads);

                    while let Some(result) = stream.next().await {
                        let (chunk_id, net_bytes) = result?;
                        _dl_pass_a_n += 1;
                        _dl_pass_a_bytes += net_bytes;
                        total_net_bytes += net_bytes;
                        // ODB store happens inside the closure (B4 refactor).
                        total_chunks_downloaded += 1;
                        bytes_done += chunk_size_map.get(&chunk_id.to_hex()).copied().unwrap_or(0);
                        on_progress(
                            bytes_done,
                            total_manifest_bytes,
                            &format!("Object {}/{}", obj_idx + 1, n_objects),
                        );
                    }
                    if let Some(b) = &_download_bench {
                        b.record_batch(_dl_pass_a_n, _dl_pass_a_bytes, _dl_pass_a_t.elapsed());
                    }
                }

                // ── Pass B: delta chunks (bases are now local) ────────────────
                if !delta_chunks.is_empty() {
                    let delta_map = std::sync::Arc::new(delta_map);
                    let _dl_pass_b_t = std::time::Instant::now();
                    let mut _dl_pass_b_n = 0u64;
                    let mut _dl_pass_b_bytes = 0u64;
                    let mut stream = futures::stream::iter(delta_chunks)
                        .map(|chunk_id| {
                            let client = self.client.clone();
                            let base_url = self.base_url.clone();
                            let delta_map = std::sync::Arc::clone(&delta_map);
                            async move {
                                let url =
                                    format!("{}/chunk-deltas/{}", base_url, chunk_id.to_hex());
                                let response = client.get(&url).send().await.map_err(|e| {
                                    anyhow::anyhow!(
                                        "Failed to download chunk-delta {}: {}",
                                        chunk_id,
                                        e
                                    )
                                })?;
                                if !response.status().is_success() {
                                    anyhow::bail!(
                                        "GET /chunk-deltas/{} failed with status: {}",
                                        chunk_id,
                                        response.status()
                                    );
                                }
                                let data = response.bytes().await.map_err(|e| {
                                    anyhow::anyhow!(
                                        "Failed to read chunk-delta {}: {}",
                                        chunk_id,
                                        e
                                    )
                                })?;
                                let base = delta_map.get(&chunk_id).copied().ok_or_else(|| {
                                    anyhow::anyhow!(
                                        "Internal: chunk {} missing from delta_map",
                                        chunk_id
                                    )
                                })?;
                                Ok::<_, anyhow::Error>((chunk_id, base, data.to_vec()))
                            }
                        })
                        .buffer_unordered(concurrent_downloads);

                    while let Some(result) = stream.next().await {
                        let (chunk_id, base_id, delta_bytes) = result?;
                        _dl_pass_b_n += 1;
                        _dl_pass_b_bytes += delta_bytes.len() as u64;
                        total_net_bytes += delta_bytes.len() as u64;
                        if let Err(e) = odb
                            .write_chunk_delta(&chunk_id, &base_id, &delta_bytes)
                            .await
                        {
                            tracing::warn!(
                                chunk = %chunk_id,
                                base = %base_id,
                                error = %e,
                                "Delta write failed (cycle?), falling back to full chunk"
                            );
                            let full = self.download_chunk(&chunk_id).await?;
                            odb.put_compressed_chunk(&chunk_id, &full).await?;
                        }
                        total_chunks_downloaded += 1;
                        bytes_done += chunk_size_map.get(&chunk_id.to_hex()).copied().unwrap_or(0);
                        on_progress(
                            bytes_done,
                            total_manifest_bytes,
                            &format!("Object {}/{}", obj_idx + 1, n_objects),
                        );
                    }
                    if let Some(b) = &_download_bench {
                        b.record_batch(_dl_pass_b_n, _dl_pass_b_bytes, _dl_pass_b_t.elapsed());
                    }
                }
            }

            // Advance bar for chunks that were already local (dedup / re-clone).
            if already_local > 0 {
                let already_bytes: u64 = manifest
                    .chunks
                    .iter()
                    .filter(|c| !missing_chunks.iter().any(|m| m == &c.id))
                    .map(|c| c.size as u64)
                    .sum();
                bytes_done += already_bytes;
                on_progress(
                    bytes_done,
                    total_manifest_bytes,
                    &format!("Object {}/{}", obj_idx + 1, n_objects),
                );
            }

            // Store manifest locally
            odb.put_manifest(&oid, &manifest).await?;

            tracing::debug!(oid = %oid, "Chunked object downloaded");
        }

        if let Some(b) = &_download_bench {
            b.summary();
        }
        Ok((total_chunks_downloaded, total_net_bytes))
    }
}
