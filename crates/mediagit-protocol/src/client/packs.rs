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

/// Result of one concurrent pack upload, carried back through the in-flight
/// queue. A struct rather than a wide tuple because every field feeds a
/// different consumer: progress accounting, `[bench]`, and error rollback.
struct PackUploadOutcome {
    result: Result<()>,
    /// Whether the bytes went out over a presigned PUT rather than the proxy.
    presigned_direct: bool,
    /// Compressed bytes that actually crossed the wire.
    pack_bytes: u64,
    /// Manifest (uncompressed) bytes, the progress-bar numerator's unit.
    manifest_bytes: u64,
    /// Chunks bundled into this pack.
    chunks: u64,
    /// Wall time for upload + register.
    elapsed: std::time::Duration,
}

impl ProtocolClient {
    // -----------------------------------------------------------------------
    // F4: Pack-mode push — bundle full chunks into cloud packs
    // -----------------------------------------------------------------------

    /// Bundle `full_chunks` into cloud packs and upload each via presigned PUT.
    /// Returns `(chunks_uploaded, bytes_uploaded)`.
    #[allow(clippy::type_complexity)]
    pub(crate) async fn push_full_chunks_via_packs(
        &self,
        full_chunks: &[Oid],
        odb: &ObjectDatabase,
        chunk_manifest_sizes: &std::collections::HashMap<Oid, u64>,
        bytes_progress: &Arc<AtomicU64>,
        bench: Option<&Arc<crate::bench::BenchSession>>,
    ) -> Result<(u32, u64)> {
        use crate::pack_builder::{PackBuilder, upload_and_register};
        use futures::stream::{FuturesUnordered, StreamExt};

        // Each in-flight pack is read fully into RAM (~MEDIAGIT_PACK_BYTES) for upload,
        // so keep this modest; =1 restores sequential uploads. Default 8 ≈ 512 MiB ceiling.
        //
        // ST-4: bounded above as well as below. Peak RAM here is
        // MEDIAGIT_PACK_BYTES × this value, so leaving the multiplier
        // unbounded left the product unbounded no matter how the byte cap was
        // clamped. 64 is already far past the point where more concurrency
        // buys throughput on a WAN-bound link.
        const MAX_PACK_UPLOAD_CONCURRENCY: usize = 64;
        let pack_upload_concurrency: usize = std::env::var("MEDIAGIT_PACK_UPLOAD_CONCURRENCY")
            .ok()
            .and_then(|v| v.parse().ok())
            .filter(|&n| n > 0)
            .map(|n: usize| n.min(MAX_PACK_UPLOAD_CONCURRENCY))
            .unwrap_or(8);

        let temp_dir = tempfile::TempDir::new().context("create pack temp dir")?;
        let mut builder = PackBuilder::new(temp_dir.path());

        crate::ensure_crypto_provider();
        let direct_client = reqwest::Client::builder()
            .pool_idle_timeout(std::time::Duration::from_secs(60))
            .pool_max_idle_per_host(http_pool_max())
            .tcp_nodelay(true)
            .timeout(std::time::Duration::from_secs(300))
            .http1_only()
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());

        let mut chunks_done: u32 = 0;
        let mut bytes_done: u64 = 0;
        // Numerator (in manifest/uncompressed bytes — same unit as the denominator)
        // already added to `bytes_progress`. Tracked so we can roll back on error and
        // let the per-chunk fallback re-credit from a clean slate (no double-count).
        let mut credited: u64 = 0;
        let mut current_hashes: Vec<(String, String)> = Vec::new();
        // Σ manifest size of the chunks accumulated into the currently-open pack.
        let mut pending_manifest_bytes: u64 = 0;
        // Boxed so the per-pack and final-pack upload futures (distinct anonymous
        // types) can share one queue.
        let mut inflight: FuturesUnordered<
            std::pin::Pin<Box<dyn std::future::Future<Output = PackUploadOutcome> + Send>>,
        > = FuturesUnordered::new();

        // Records one completed pack upload against the bench session.
        //
        // `record_batch` matters as much as the pack counters: it is what feeds
        // `throughput_mbs`, and pack mode is the DEFAULT push path. Without it
        // a pack-mode push reports `chunks=0 total_bytes=0 throughput_mbs=0.00`
        // and any throughput comparison silently measures nothing.
        let note_pack = |o: &PackUploadOutcome| {
            if let Some(b) = bench {
                b.record_pack(o.pack_bytes);
                if o.presigned_direct {
                    b.record_presign_urls(1);
                }
                b.record_batch(o.chunks, o.pack_bytes, o.elapsed);
            }
        };

        // Build packs synchronously; upload them concurrently (bounded). Wrapped so a
        // failure mid-stream can roll back the numerator credits before propagating.
        let outcome: Result<()> = async {
            for chunk_id in full_chunks {
                let data = odb
                    .get_compressed_chunk(chunk_id)
                    .await
                    .with_context(|| format!("read chunk {} for pack", chunk_id))?;

                // Record BLAKE3(compressed bytes) for per-slice pull-side verify.
                let comp_hash = Oid::hash(&data).to_hex();
                current_hashes.push((chunk_id.to_hex(), comp_hash));
                pending_manifest_bytes += chunk_manifest_sizes.get(chunk_id).copied().unwrap_or(0);

                if let Some(result) = builder
                    .add_chunk(*chunk_id, &data)
                    .await
                    .with_context(|| format!("pack chunk {}", chunk_id))?
                {
                    // The sealed pack includes the chunk just added (writer flushes on
                    // cap hit), so it covers every chunk accumulated since the last seal.
                    let hashes = std::mem::take(&mut current_hashes);
                    let manifest_bytes = std::mem::take(&mut pending_manifest_bytes);
                    let pack_byte_len = result.byte_len;
                    let pack_chunks = hashes.len() as u64;
                    let direct = direct_client.clone(); // cheap: reqwest::Client is Arc-internal
                    inflight.push(Box::pin(async move {
                        let t = std::time::Instant::now();
                        let r = upload_and_register(
                            result,
                            &self.base_url,
                            &self.client,
                            &direct,
                            &hashes,
                        )
                        .await;
                        let elapsed = t.elapsed();
                        let (result, presigned_direct) = match r {
                            Ok(d) => (Ok(()), d),
                            Err(e) => (Err(e), false),
                        };
                        PackUploadOutcome {
                            result,
                            presigned_direct,
                            pack_bytes: pack_byte_len,
                            manifest_bytes,
                            chunks: pack_chunks,
                            elapsed,
                        }
                    }));

                    // Backpressure: never hold more than N packs in flight.
                    while inflight.len() >= pack_upload_concurrency {
                        if let Some(o) = inflight.next().await {
                            note_pack(&o);
                            o.result.context("upload_and_register pack")?;
                            bytes_done += o.pack_bytes;
                            bytes_progress.fetch_add(o.manifest_bytes, Ordering::Relaxed);
                            credited += o.manifest_bytes;
                        }
                    }
                }
                chunks_done += 1;
            }

            // Flush the final partial pack.
            if let Some(result) = builder.finish().await.context("finish final pack")? {
                let hashes = std::mem::take(&mut current_hashes);
                let manifest_bytes = std::mem::take(&mut pending_manifest_bytes);
                let pack_byte_len = result.byte_len;
                let pack_chunks = hashes.len() as u64;
                let direct = direct_client.clone();
                inflight.push(Box::pin(async move {
                    let t = std::time::Instant::now();
                    let r =
                        upload_and_register(result, &self.base_url, &self.client, &direct, &hashes)
                            .await;
                    let elapsed = t.elapsed();
                    let (result, presigned_direct) = match r {
                        Ok(d) => (Ok(()), d),
                        Err(e) => (Err(e), false),
                    };
                    PackUploadOutcome {
                        result,
                        presigned_direct,
                        pack_bytes: pack_byte_len,
                        manifest_bytes,
                        chunks: pack_chunks,
                        elapsed,
                    }
                }));
            }

            // Drain remaining uploads.
            while let Some(o) = inflight.next().await {
                note_pack(&o);
                o.result.context("upload_and_register pack")?;
                bytes_done += o.pack_bytes;
                bytes_progress.fetch_add(o.manifest_bytes, Ordering::Relaxed);
                credited += o.manifest_bytes;
            }
            Ok(())
        }
        .await;

        drop(temp_dir);

        match outcome {
            Ok(()) => Ok((chunks_done, bytes_done)),
            Err(e) => {
                // Roll back our credits so the per-chunk fallback re-credits from zero.
                if credited > 0 {
                    bytes_progress.fetch_sub(credited, Ordering::Relaxed);
                }
                Err(e)
            }
        }
    }

    // -----------------------------------------------------------------------
    // F7: Pack-based pull (locate → presign → Range-GET → ODB)
    // F8: Per-slice BLAKE3 verify
    // -----------------------------------------------------------------------

    /// Locate chunks in the server's pack manifest index.
    ///
    /// `wants_full_repo=true` fetches the entire map in one response (full-clone).
    async fn locate_chunks_in_packs(
        &self,
        chunk_ids: &[String],
        wants_full_repo: bool,
    ) -> Result<std::collections::HashMap<String, PackLocInfo>> {
        #[derive(serde::Serialize)]
        struct Req<'a> {
            chunk_ids: &'a [String],
            wants_full_repo: bool,
        }
        #[derive(serde::Deserialize)]
        struct LocEntry {
            pack_oid: String,
            offset: u64,
            length: u32,
            #[serde(default)]
            compressed_hash: Option<String>,
        }

        let url = format!("{}/chunks/locate", self.base_url);
        let resp = send_with_rate_limit_retry(|| {
            self.client
                .post(&url)
                .json(&Req {
                    chunk_ids,
                    wants_full_repo,
                })
                .send()
        })
        .await
        .context("POST /chunks/locate")?;
        if !resp.status().is_success() {
            anyhow::bail!("POST /chunks/locate returned {}", resp.status());
        }
        let map: std::collections::HashMap<String, LocEntry> =
            resp.json().await.context("parse /chunks/locate")?;
        Ok(map
            .into_iter()
            .map(|(oid, e)| {
                (
                    oid,
                    PackLocInfo {
                        pack_oid: e.pack_oid,
                        offset: e.offset,
                        length: e.length,
                        compressed_hash: e.compressed_hash,
                    },
                )
            })
            .collect())
    }
    /// Request presigned GET URLs for a batch of pack objects.
    async fn request_pack_download_urls(
        &self,
        pack_ids: &[String],
    ) -> std::collections::HashMap<String, Option<PresignedGetInfo>> {
        if pack_ids.is_empty() {
            return std::collections::HashMap::new();
        }
        #[derive(serde::Serialize)]
        struct Req<'a> {
            pack_ids: &'a [String],
        }
        let url = format!("{}/packs/presign-download-urls", self.base_url);
        let result = async {
            let resp = send_with_rate_limit_retry(|| {
                self.client.post(&url).json(&Req { pack_ids }).send()
            })
            .await?;
            if !resp.status().is_success() {
                anyhow::bail!(
                    "POST /packs/presign-download-urls returned {}",
                    resp.status()
                );
            }
            resp.json::<std::collections::HashMap<String, Option<PresignedGetInfo>>>()
                .await
                .context("parse /packs/presign-download-urls")
        }
        .await;
        match result {
            Ok(m) => m,
            Err(e) => {
                tracing::warn!(err = %e, "request_pack_download_urls failed");
                std::collections::HashMap::new()
            }
        }
    }

    /// Pull a set of chunks via pack-mode Range-GET (F7 + F8).
    ///
    /// Returns the set of chunk OIDs successfully written to ODB.
    /// Caller diffs against the requested set to find any that were skipped
    /// (bounds error or hash mismatch) and falls back to per-chunk download.
    pub(crate) async fn pull_chunks_via_packs(
        &self,
        chunk_ids: &[Oid],
        odb: &ObjectDatabase,
        on_progress: Option<std::sync::Arc<dyn Fn(u64) + Send + Sync>>,
        bench: Option<&Arc<crate::bench::BenchSession>>,
    ) -> Result<std::collections::HashSet<Oid>> {
        use futures::{StreamExt, TryStreamExt};

        if chunk_ids.is_empty() {
            return Ok(std::collections::HashSet::new());
        }

        // Integrity model: pack bytes are compressed (matching local ODB storage format).
        // chunk_oid = BLAKE3(uncompressed), so per-slice BLAKE3 would not match.
        // Integrity is covered by: (a) pack_oid = BLAKE3(full pack bytes) verified at push
        // upload time via complete_pack; (b) clone-SHA / fsck tests on the pulled working tree.
        // Per-slice hash verification requires storing compressed hashes in the manifest
        // and is deferred as a future improvement.
        let download_concurrency: usize = std::env::var("MEDIAGIT_DOWNLOAD_CONCURRENCY")
            .ok()
            .and_then(|s| s.parse().ok())
            .filter(|n: &usize| *n > 0)
            .or(self.concurrent_downloads)
            .unwrap_or(24);

        let coalesce_max_gap: u64 = std::env::var("MEDIAGIT_PACK_RANGE_COALESCE_MAX_GAP")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(1_048_576);
        let coalesce_max_bytes: u64 = std::env::var("MEDIAGIT_PACK_RANGE_COALESCE_MAX_BYTES")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(8_388_608);

        let all_hex: Vec<String> = chunk_ids.iter().map(|o| o.to_hex()).collect();
        let wants_full_repo = all_hex.len() > 512;
        let loc_map = self
            .locate_chunks_in_packs(&all_hex, wants_full_repo)
            .await?;
        if loc_map.is_empty() {
            return Ok(std::collections::HashSet::new());
        }

        // Build chunk → compressed_hash lookup for per-slice verify.
        let compressed_hashes: std::collections::HashMap<String, String> = loc_map
            .iter()
            .filter_map(|(hex, loc)| {
                loc.compressed_hash
                    .as_ref()
                    .map(|h| (hex.clone(), h.clone()))
            })
            .collect();
        let compressed_hashes = std::sync::Arc::new(compressed_hashes);

        // Group by pack_oid, sort by offset.
        let mut by_pack: std::collections::HashMap<String, Vec<(String, u64, u32)>> =
            std::collections::HashMap::new();
        for hex in &all_hex {
            if let Some(loc) = loc_map.get(hex) {
                by_pack.entry(loc.pack_oid.clone()).or_default().push((
                    hex.clone(),
                    loc.offset,
                    loc.length,
                ));
            }
        }

        let pack_ids: Vec<String> = by_pack.keys().cloned().collect();
        let presign_map = self.request_pack_download_urls(&pack_ids).await;
        // Count URLs actually issued, not packs asked about: a backend that
        // cannot presign (GCS + ADC) answers with `None` and the fetch quietly
        // takes the server-proxy path instead.
        if let Some(b) = bench {
            b.record_presign_urls(presign_map.values().filter(|o| o.is_some()).count() as u64);
        }

        // No presigned GET (e.g. GCS + ADC): batch-fetch via the server proxy
        // instead of dropping the pack and falling all the way back to one
        // per-chunk HTTP round-trip each. MEDIAGIT_PACK_PROXY_BATCH=0 restores
        // the old drop-and-per-chunk-fallback behavior.
        let use_batch_proxy = std::env::var("MEDIAGIT_PACK_PROXY_BATCH")
            .map(|v| v != "0")
            .unwrap_or(true);

        enum PackFetchTask {
            Presigned {
                chunks: Vec<(String, u64, u32)>,
                url: String,
                pack_oid: String,
            },
            Batch {
                chunks: Vec<(String, u64, u32)>,
                pack_oid: String,
            },
        }

        let tasks: Vec<PackFetchTask> = by_pack
            .into_iter()
            .filter_map(|(pack_oid, mut chunks)| {
                chunks.sort_unstable_by_key(|(_, off, _)| *off);
                match presign_map.get(&pack_oid).and_then(|o| o.as_ref()) {
                    Some(p) => Some(PackFetchTask::Presigned {
                        chunks,
                        url: p.url.clone(),
                        pack_oid,
                    }),
                    None if use_batch_proxy => Some(PackFetchTask::Batch { chunks, pack_oid }),
                    None => None,
                }
            })
            .collect();

        let client = self.client.clone();
        let base_url = self.base_url.clone();
        // Clone odb so it can move into concurrent async tasks (all fields are Arc-wrapped).
        let odb = odb.clone();

        let written_set: std::collections::HashSet<Oid> = futures::stream::iter(tasks)
            .map(|task| {
                let client = client.clone();
                let base_url = base_url.clone();
                let cmg = coalesce_max_gap;
                let cmb = coalesce_max_bytes;
                let comp_hashes = std::sync::Arc::clone(&compressed_hashes);
                let odb = odb.clone();
                let progress = on_progress.clone();
                let bench = bench.cloned();
                async move {
                    let fetch_start = std::time::Instant::now();
                    let out: Vec<(Oid, Vec<u8>)> = match task {
                        PackFetchTask::Presigned {
                            chunks,
                            url,
                            pack_oid,
                        } => {
                            fetch_pack_slices_presigned(
                                &client,
                                &url,
                                &pack_oid,
                                &chunks,
                                cmg,
                                cmb,
                                &comp_hashes,
                                bench.as_ref(),
                            )
                            .await?
                        }
                        PackFetchTask::Batch { chunks, pack_oid } => {
                            fetch_pack_slices_batch(
                                &client,
                                &base_url,
                                &pack_oid,
                                &chunks,
                                &comp_hashes,
                            )
                            .await?
                        }
                    };
                    if let Some(b) = &bench {
                        let fetched: u64 = out.iter().map(|(_, d)| d.len() as u64).sum();
                        b.record_pack(fetched);
                        // Feeds throughput_mbs; pack mode is the default pull
                        // path, so without this a pack-mode clone reports zero
                        // throughput. Timed around the fetch only, excluding
                        // the ODB writes below, to match the push side's
                        // transfer-only accounting.
                        b.record_batch(out.len() as u64, fetched, fetch_start.elapsed());
                    }
                    // Write to ODB and report progress as each chunk arrives, without
                    // buffering all pack results first (eliminates the collect().await pattern).
                    let mut written: Vec<Oid> = Vec::new();
                    for (oid, data) in out {
                        let chunk_size = data.len() as u64;
                        odb.put_compressed_chunk(&oid, &data)
                            .await
                            .with_context(|| format!("write chunk {} to ODB", oid))?;
                        if let Some(ref cb) = progress {
                            cb(chunk_size);
                        }
                        written.push(oid);
                    }
                    Ok::<Vec<Oid>, anyhow::Error>(written)
                }
            })
            .buffer_unordered(download_concurrency)
            .try_fold(
                std::collections::HashSet::<Oid>::new(),
                |mut acc, oids: Vec<Oid>| async move {
                    acc.extend(oids);
                    Ok(acc)
                },
            )
            .await
            .map_err(|e| {
                e.context("pack Range-GET failed; caller should use per-chunk fallback")
            })?;

        if written_set.is_empty() && !loc_map.is_empty() {
            tracing::error!(
                located = loc_map.len(),
                "pack-mode pull: 0/{} chunks passed verify — all slices failed \
                 compressed-hash check or bounds; server JSONL may be stale or \
                 from a different binary. Falling back to per-chunk (slow).",
                loc_map.len()
            );
        }
        Ok(written_set)
    }
}

/// Whether a Range-GET's HTTP status can be sliced with `rel = off - range_start`.
///
/// A compliant server answers a Range request with 206 (partial content). A 200
/// means it ignored the Range and returned the whole object from offset 0, so
/// the `off - range_start` math only lands correctly when `range_start == 0`
/// (body offset 0 == range_start). Any other status/offset combination would
/// mis-slice — the caller must reject it and fall back to the per-chunk path.
/// ST-3: decide whether a pack slice may be accepted.
///
/// A slice is accepted only when the manifest records a `compressed_hash` for
/// it *and* the bytes match. The previous rule verified only when a hash
/// happened to be present and accepted the slice otherwise — fail-open on an
/// integrity check. `compressed_hash` is `#[serde(default)] Option`, so a
/// manifest from an older binary, or one with a truncated line, silently
/// disabled verification for that chunk while still yielding its bytes.
///
/// Failing closed is cheap: a refused slice falls through to the per-chunk
/// download path, which verifies. The cost is throughput on an unverifiable
/// manifest, never correctness.
///
/// Extracted so the Range-GET and batch-get paths cannot drift apart — they
/// must apply the same rule, and previously each had its own copy of it.
fn slice_verifies(
    hex: &str,
    data: &[u8],
    comp_hashes: &std::collections::HashMap<String, String>,
) -> bool {
    match comp_hashes.get(hex) {
        Some(expected) => {
            let computed = Oid::hash(data).to_hex();
            if &computed != expected {
                tracing::warn!(
                    chunk = %hex,
                    expected = %expected,
                    computed = %computed,
                    "compressed-hash mismatch on pack slice; refusing"
                );
                return false;
            }
            true
        }
        None => {
            tracing::warn!(
                chunk = %hex,
                "pack manifest carries no compressed_hash for this chunk;                  refusing the slice unverified and falling back to per-chunk"
            );
            false
        }
    }
}

fn range_status_trusted(status: u16, range_start: u64) -> bool {
    status == 206 || (status == 200 && range_start == 0)
}

/// Fetch requested (chunk_oid, offset, length) slices out of one pack via
/// presigned Range-GET requests directly against cloud storage.
///
/// Returns the (Oid, compressed_bytes) pairs that passed bounds + per-slice
/// compressed-hash verification. Entries that fail either check are silently
/// skipped (logged), not errored — the caller falls back to the per-chunk
/// path for anything missing from the returned set. A response that cannot be
/// trusted whole (non-206 for a non-zero-offset range, or a truncated body)
/// errors instead, so the caller re-fetches the range via per-chunk fallback.
#[allow(clippy::too_many_arguments)]
async fn fetch_pack_slices_presigned(
    client: &reqwest::Client,
    url: &str,
    pack_oid: &str,
    chunks: &[(String, u64, u32)],
    coalesce_max_gap: u64,
    coalesce_max_bytes: u64,
    comp_hashes: &std::collections::HashMap<String, String>,
    bench: Option<&Arc<crate::bench::BenchSession>>,
) -> Result<Vec<(Oid, Vec<u8>)>> {
    let ranges = coalesce_chunk_ranges(chunks, coalesce_max_gap, coalesce_max_bytes);
    let mut out: Vec<(Oid, Vec<u8>)> = Vec::new();

    for (range_start, range_end) in ranges {
        // A request is "coalesced" when this one Range covers more than one
        // logical chunk — the ratio of these is what tells us whether range
        // merging is actually earning its keep.
        if let Some(b) = bench {
            let covered = chunks
                .iter()
                .filter(|(_, off, len)| *off >= range_start && *off + *len as u64 <= range_end)
                .count();
            b.record_range_get(covered > 1);
        }
        let hdr = format!("bytes={}-{}", range_start, range_end.saturating_sub(1));
        // Presigned direct-to-bucket GET — not routed through
        // send_with_rate_limit_retry, which only applies to the server's own
        // 429s and Retry-After semantics.
        let resp = client
            .get(url)
            .header("Range", &hdr)
            .send()
            .await
            .with_context(|| format!("Range-GET {} range {}", pack_oid, hdr))?;

        let status = resp.status().as_u16();
        if !range_status_trusted(status, range_start) {
            anyhow::bail!(
                "Range-GET returned {} (want 206) for pack {} range {}",
                status,
                pack_oid,
                hdr
            );
        }

        let body = resp.bytes().await.context("read Range-GET body")?;

        // The requested range maps 1:1 onto real chunk offsets, so a compliant
        // body is at least `range_end - range_start` long. A shorter body means a
        // truncated/partial response (a backend degrading under load returns
        // short reads) — a chunk near the tail would then mis-slice or silently
        // shrink. Bail to the per-chunk fallback rather than trust it.
        let expected_len = (range_end - range_start) as usize;
        if body.len() < expected_len {
            anyhow::bail!(
                "Range-GET short body for pack {} range {}: got {} of {} bytes",
                pack_oid,
                hdr,
                body.len(),
                expected_len
            );
        }

        for (hex, off, len) in chunks {
            if *off < range_start || *off + *len as u64 > range_end {
                continue;
            }
            let rel = (*off - range_start) as usize;
            let slice_end = rel + *len as usize;
            if slice_end > body.len() || *len < 5 {
                tracing::warn!(chunk = %hex, "pack slice bounds error");
                continue;
            }
            // Pack object layout: type(1) + size(4) + data
            let data = &body[rel + 5..slice_end];

            let oid = match Oid::from_hex(hex) {
                Ok(o) => o,
                Err(_) => continue,
            };

            if !slice_verifies(hex, data, comp_hashes) {
                continue;
            }

            out.push((oid, data.to_vec()));
        }
    }
    Ok(out)
}

/// Fetch requested (chunk_oid, offset, length) slices out of one pack via
/// the server-proxied `POST /packs/batch-get` endpoint (D2) — used when no
/// presigned GET URL is available (e.g. GCS + ADC), so pulling a pack's
/// worth of chunks costs one request instead of one per chunk.
///
/// Frame format matches `batch_get_pack_chunks` on the server:
/// `[chunk_oid: 32 raw bytes][len: u32 LE][data: len bytes]`, one frame per
/// requested entry in request order. A zero-length frame is a miss (entry
/// not found in the server's pack index) and is skipped here, falling
/// through to the per-chunk path same as a presigned-slice verify failure.
///
/// On HTTP 404 (server predates this endpoint) returns `Ok(vec![])` rather
/// than erroring, so only this pack's chunks fall back to per-chunk —
/// other packs in the same pull are unaffected.
async fn fetch_pack_slices_batch(
    client: &reqwest::Client,
    base_url: &str,
    pack_oid: &str,
    chunks: &[(String, u64, u32)],
    comp_hashes: &std::collections::HashMap<String, String>,
) -> Result<Vec<(Oid, Vec<u8>)>> {
    #[derive(serde::Serialize, Clone)]
    struct Entry<'a> {
        chunk_oid: &'a str,
        offset: u64,
        length: u32,
    }
    #[derive(serde::Serialize)]
    struct Req<'a> {
        pack_oid: &'a str,
        entries: Vec<Entry<'a>>,
    }

    let entries: Vec<Entry> = chunks
        .iter()
        .map(|(hex, off, len)| Entry {
            chunk_oid: hex,
            offset: *off,
            length: *len,
        })
        .collect();

    let url = format!("{}/packs/batch-get", base_url);
    let resp = send_with_rate_limit_retry(|| {
        client
            .post(&url)
            .json(&Req {
                pack_oid,
                entries: entries.clone(),
            })
            .send()
    })
    .await
    .with_context(|| format!("POST /packs/batch-get for pack {}", pack_oid))?;

    if resp.status() == reqwest::StatusCode::NOT_FOUND {
        tracing::debug!(pack = %pack_oid, "batch-get 404 (old server); pack falls back to per-chunk");
        return Ok(Vec::new());
    }
    if !resp.status().is_success() {
        anyhow::bail!("batch-get returned {} for pack {}", resp.status(), pack_oid);
    }

    let body = resp.bytes().await.context("read batch-get body")?;
    let frames = parse_batch_get_frames(&body, pack_oid);

    let mut out: Vec<(Oid, Vec<u8>)> = Vec::new();
    for (oid, data) in frames {
        if data.is_empty() {
            // Miss frame — entry not in server's pack index. Falls through
            // to per-chunk fallback for this one chunk.
            continue;
        }
        let hex = oid.to_hex();
        if !slice_verifies(&hex, &data, comp_hashes) {
            continue;
        }

        out.push((oid, data));
    }
    Ok(out)
}

/// Parse a batch-get response body into `(chunk_oid, data)` frames.
///
/// Frame format: `[chunk_oid: 32 raw bytes][len: u32 LE][data: len bytes]`,
/// repeated. A frame with `data` empty is a valid "miss" marker (caller
/// decides how to treat it). Stops (without erroring) on a truncated
/// trailing frame — the whole body is untrusted network input.
fn parse_batch_get_frames(body: &[u8], pack_oid: &str) -> Vec<(Oid, Vec<u8>)> {
    let mut out = Vec::new();
    let mut pos = 0usize;
    while pos + 36 <= body.len() {
        let oid_bytes: [u8; 32] = body[pos..pos + 32]
            .try_into()
            .expect("slice is exactly 32 bytes");
        let oid = Oid::from_bytes(oid_bytes);
        pos += 32;
        let len = u32::from_le_bytes(
            body[pos..pos + 4]
                .try_into()
                .expect("slice is exactly 4 bytes"),
        ) as usize;
        pos += 4;
        if pos + len > body.len() {
            tracing::warn!(pack = %pack_oid, "batch-get response truncated mid-frame");
            break;
        }
        let data = body[pos..pos + len].to_vec();
        pos += len;
        out.push((oid, data));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(oid: &Oid, data: &[u8]) -> Vec<u8> {
        let mut buf = Vec::new();
        buf.extend_from_slice(oid.as_bytes());
        buf.extend_from_slice(&(data.len() as u32).to_le_bytes());
        buf.extend_from_slice(data);
        buf
    }

    #[test]
    fn range_status_trust_rejects_ignored_range() {
        // 206 partial is always fine.
        assert!(range_status_trusted(206, 0));
        assert!(range_status_trusted(206, 4096));
        // A 200 (range ignored -> whole object from offset 0) is only safe when
        // the range started at 0; at any non-zero offset it mis-slices.
        assert!(range_status_trusted(200, 0));
        assert!(!range_status_trusted(200, 1));
        assert!(!range_status_trusted(200, 4096));
        // Anything else is a hard reject.
        assert!(!range_status_trusted(416, 0));
        assert!(!range_status_trusted(500, 0));
    }

    #[test]
    fn parses_multiple_frames_including_a_miss() {
        let oid_a = Oid::hash(b"chunk a");
        let oid_b = Oid::hash(b"chunk b");
        let mut body = Vec::new();
        body.extend_from_slice(&frame(&oid_a, b"payload-a"));
        body.extend_from_slice(&frame(&oid_b, &[])); // miss frame

        let frames = parse_batch_get_frames(&body, "pack1");

        assert_eq!(frames.len(), 2);
        assert_eq!(frames[0].0, oid_a);
        assert_eq!(frames[0].1, b"payload-a");
        assert_eq!(frames[1].0, oid_b);
        assert!(frames[1].1.is_empty());
    }

    #[test]
    fn stops_cleanly_on_truncated_trailing_frame() {
        let oid_a = Oid::hash(b"chunk a");
        let mut body = frame(&oid_a, b"full payload");
        // Append a truncated second frame: valid header, claims 100 bytes,
        // but the body ends after only a few.
        let oid_b = Oid::hash(b"chunk b");
        body.extend_from_slice(oid_b.as_bytes());
        body.extend_from_slice(&100u32.to_le_bytes());
        body.extend_from_slice(b"short");

        let frames = parse_batch_get_frames(&body, "pack1");

        // Only the complete first frame is returned; no panic on the truncated tail.
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].0, oid_a);
        assert_eq!(frames[0].1, b"full payload");
    }

    #[test]
    fn empty_body_parses_to_no_frames() {
        assert!(parse_batch_get_frames(&[], "pack1").is_empty());
    }

    /// ST-3: an unverifiable slice must be refused, not trusted.
    #[test]
    fn slice_without_recorded_hash_is_refused() {
        let data = b"chunk bytes";
        let hex = Oid::hash(data).to_hex();
        let empty = std::collections::HashMap::new();

        assert!(
            !slice_verifies(&hex, data, &empty),
            "a manifest with no compressed_hash left the slice unverified and              accepted it — fail-open on an integrity check"
        );
    }

    #[test]
    fn slice_with_matching_hash_is_accepted() {
        let data = b"chunk bytes";
        let hex = Oid::hash(data).to_hex();
        let mut m = std::collections::HashMap::new();
        // The manifest records BLAKE3 of the compressed bytes as they are
        // stored; here the "compressed" bytes are the payload itself.
        m.insert(hex.clone(), Oid::hash(data).to_hex());

        assert!(slice_verifies(&hex, data, &m));
    }

    #[test]
    fn slice_with_wrong_hash_is_refused() {
        let data = b"chunk bytes";
        let hex = Oid::hash(data).to_hex();
        let mut m = std::collections::HashMap::new();
        m.insert(hex.clone(), Oid::hash(b"different").to_hex());

        assert!(
            !slice_verifies(&hex, data, &m),
            "a slice whose bytes do not match the manifest was accepted"
        );
    }
}
