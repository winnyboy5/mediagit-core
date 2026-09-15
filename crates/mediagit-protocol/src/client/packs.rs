// SPDX-License-Identifier: BUSL-1.1
// Copyright (C) 2025-2026 Aswin Krishnamoorthy

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

/// Decide what the pack phase reports, given how far it got.
///
/// Extracted from `push_full_chunks_via_packs` so the one invariant that keeps a
/// partial pack failure from becoming DATA LOSS can be tested directly.
///
/// The caller in `push.rs` runs its per-chunk fallback only when the pack path
/// did NOT succeed:
///
/// ```text
/// if !full_chunks.is_empty() && !pack_pushed { ...fallback... }
/// ```
///
/// So reporting Ok because "most packs landed" would skip the fallback and leave
/// the failed pack's chunks uploaded NOWHERE, while the push exits 0. Any pack
/// failure must therefore surface as Err, no matter how many packs succeeded -
/// the caller then re-checks what actually exists server-side and uploads only
/// the genuinely missing chunks.
fn finish_pack_phase(
    chunks_done: u32,
    bytes_done: u64,
    pack_failure: Option<anyhow::Error>,
    outcome: Result<()>,
) -> Result<(u32, u64)> {
    outcome?;
    match pack_failure {
        Some(e) => Err(e),
        None => Ok((chunks_done, bytes_done)),
    }
}

#[cfg(test)]
mod pack_phase_tests {
    use super::finish_pack_phase;

    #[test]
    fn a_partial_pack_failure_is_reported_as_failure_not_partial_success() {
        // The trap this exists to guard. Before 20260821, one failed pack aborted
        // every queued pack; now they keep uploading, which makes "lots of packs
        // succeeded AND one failed" a reachable state for the first time.
        //
        // The tempting implementation returns Ok((chunks_done, bytes_done)) here
        // because real work got done. That sets pack_pushed = true in push.rs,
        // skips the per-chunk fallback, and silently drops the failed pack's
        // chunks - a push reporting success with data missing.
        let out = finish_pack_phase(
            9_999,
            8_888,
            Some(anyhow::anyhow!("pack 7 of 32 exhausted its retry budget")),
            Ok(()),
        );
        assert!(
            out.is_err(),
            "a pack failure must surface as Err even when other packs succeeded; \
             returning Ok skips the caller's fallback and loses that pack's chunks"
        );
    }

    #[test]
    fn a_clean_pack_phase_reports_its_totals() {
        // The other half: without this, "always Err" would pass the test above
        // while disabling the fast path entirely.
        let out = finish_pack_phase(42, 1024, None, Ok(())).expect("clean phase must be Ok");
        assert_eq!(
            out,
            (42, 1024),
            "a clean phase must report what it uploaded"
        );
    }

    #[test]
    fn a_hard_error_still_wins_over_the_totals() {
        let out = finish_pack_phase(5, 5, None, Err(anyhow::anyhow!("odb read failed")));
        assert!(
            out.is_err(),
            "an error from the build/upload loop must propagate"
        );
    }
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

        // The bound scales with the pack, because the cost does. A pack BODY is
        // bounded, but the time to upload it is not: it is (pack size / share of
        // the link), and packs upload concurrently, so each one's share shrinks
        // as fan-out grows.
        //
        // Measured in 20260821-s5check. Azure moved 2048 MB in 2072s — ~1 MB/s
        // aggregate — and a 64 MB pack's wall time blew past the 300s ceiling
        // while it was still PROGRESSING. Only the 2 packs that got through
        // early survived; the other 30 were killed mid-flight and the push fell
        // back to the per-chunk path at 0.99 MB/s:
        //
        //   [azure] packsOffered=32 packsCompleted=2 perChunkProxyPUTs=724
        //   cause: "operation timed out" (x3 across azure+aws, x0 HTTP statuses)
        //
        // DROPPING `.timeout()` DID NOT FIX THAT, and this comment used to claim
        // it had. The shared builder's read_timeout took over as the ceiling: it
        // bounds the gap between bytes RECEIVED, and an upload receives nothing
        // until the body completes, so it never resets and lands on the same
        // 300s wall. The same drill reproduced it a fortnight later — azure
        // 6/32, and four packs dying at EXACTLY 300.0s within one second of each
        // other, which is a start-time deadline and not four independent stalls.
        // Raising ONLY that knob to 1800 took the same push from 6/32 in 918.66s
        // to 32/32 in 368.27s.
        //
        // So the fix is the shape, not the number: a total ceiling derived from
        // the bytes, i.e. "this pack must sustain MIN_UPLOAD_BYTES_PER_SEC". A
        // dead socket still fails; a slow-but-moving pack is left alone. Passing
        // the cap (not the actual pack size) keeps every pack on one client, and
        // the cap is the largest body any of them can hold.
        let direct_client =
            super::data_plane_upload_client_builder(mediagit_versioning::pack_bytes_cap())
                .build()
                .unwrap_or_else(|_| reqwest::Client::new());

        let mut chunks_done: u32 = 0;
        let mut bytes_done: u64 = 0;
        // Numerator (in manifest/uncompressed bytes — same unit as the denominator)
        // already added to `bytes_progress`. Tracked so we can roll back on error and
        // let the per-chunk fallback re-credit from a clean slate (no double-count).
        let mut credited: u64 = 0;
        // A pack that exhausts its retry budget used to `?` straight out of the
        // loop, abandoning every pack still queued. 20260821-ga12 measured the
        // cost on the AWS arm: 4 packs failed, 1 landed, and 27 were NEVER
        // ATTEMPTED - packsOffered=32 packsCompleted=1, 361 chunks proxied.
        //
        // The per-pack retry (ee80699) made each pack survive a flaky link; it
        // did nothing about the all-or-nothing structure above it. One pack's
        // bad luck still cost the whole push its fast path.
        //
        // Now a failure is RECORDED and the remaining packs keep going. The
        // failed pack's chunks are covered by the caller's existing re-check
        // fallback - see the note on the return value below, which is the part
        // that must not be got wrong.
        let mut pack_failure: Option<anyhow::Error> = None;
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
                            if let Err(e) = o.result {
                                // Keep the FIRST error: closest to the root
                                // cause; later ones are usually knock-on.
                                if pack_failure.is_none() {
                                    pack_failure = Some(e.context("upload_and_register pack"));
                                }
                            } else {
                                bytes_done += o.pack_bytes;
                                bytes_progress.fetch_add(o.manifest_bytes, Ordering::Relaxed);
                                credited += o.manifest_bytes;
                            }
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
                if let Err(e) = o.result {
                    if pack_failure.is_none() {
                        pack_failure = Some(e.context("upload_and_register pack"));
                    }
                } else {
                    bytes_done += o.pack_bytes;
                    bytes_progress.fetch_add(o.manifest_bytes, Ordering::Relaxed);
                    credited += o.manifest_bytes;
                }
            }
            Ok(())
        }
        .await;

        drop(temp_dir);

        // A pack failure is reported as Err even though the remaining packs were
        // uploaded, and that is deliberate - it is the whole safety argument.
        //
        // The caller only runs its per-chunk fallback when the pack path did NOT
        // succeed (`if !full_chunks.is_empty() && !pack_pushed`). Returning Ok
        // here because "most packs landed" would set pack_pushed = true, skip the
        // fallback, and leave the FAILED pack's chunks uploaded nowhere - a push
        // that reports success with chunks missing. That is a data-loss bug, and
        // a far worse one than the slowdown this change exists to fix.
        //
        // Err keeps the existing, proven recovery exactly as it was: the caller
        // re-checks which chunks actually exist server-side
        // (`push.rs`: "re-checked existence after partial pack failure"), uploads
        // only the genuinely missing ones, and re-credits the rest. Chunks in the
        // packs that DID land are found present and skipped, so the win is real -
        // the fallback now handles a handful of chunks instead of all of them -
        // while the correctness path is byte-for-byte the one already in service.
        //
        // Credits are rolled back for the same reason: the caller's re-check
        // re-credits what it finds, so leaving ours in place would double-count.
        if credited > 0 && (pack_failure.is_some() || outcome.is_err()) {
            // Roll back our credits so the per-chunk fallback re-credits from zero.
            bytes_progress.fetch_sub(credited, Ordering::Relaxed);
        }
        finish_pack_phase(chunks_done, bytes_done, pack_failure, outcome)
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
        // NOTE: `MEDIAGIT_DOWNLOAD_CONCURRENCY` is read in TWO places with
        // DIFFERENT defaults - 24 here (pack range-GETs) and 32 in
        // `pull.rs::pull_streaming` (per-chunk downloads). Setting the env var
        // makes both agree; leaving it unset does not.
        //
        // MEASURED 2026-08-25, and the answer is leave it alone. 384MB / 302
        // chunks over loopback, `MEDIAGIT_BENCH=1`:
        //
        //   conc=1        wall=4.42s  87 MB/s   util_pct=54%
        //   conc=default  wall=3.67s 105 MB/s   util_pct=2%
        //   conc=32       wall=3.57s 107 MB/s   util_pct=2%
        //
        // The knob is LIVE - conc=1 is measurably slower and util_pct moves
        // 54% -> 2%, so the value really does reach the download loop. But 24 vs
        // 32 is 3.67s vs 3.57s, inside run-to-run noise. Unifying the defaults
        // would be churn with no measurable benefit, so the split stays.
        //
        // The load-bearing number is util_pct=2%: at the default the pipeline is
        // nowhere near saturating even 24 slots, so this path is NOT
        // concurrency-limited. Anyone chasing the clone-vs-push asymmetry should
        // read that as "raising download concurrency will not help" - and note
        // both defaults already exceed the pack UPLOAD concurrency (8, above),
        // so "clone is less parallel than push" is not the explanation either.
        //
        // Caveat on scope: measured over loopback against the filesystem backend.
        // A WAN/cloud backend has entirely different latency, so re-measure there
        // before drawing conclusions about cloud clone throughput.
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
                    // Chunks are written to the ODB AS EACH RANGE ARRIVES, via
                    // the sink, rather than collected per-pack and written
                    // after. Under buffer_unordered this is the difference
                    // between holding one coalesced range and holding a whole
                    // 66 MB pack, per in-flight task — measured 1,594 MB client
                    // peak before. See PackChunkSink.
                    let mut sink = PackChunkSink::new(&odb, progress.as_ref());
                    let fetch_start = std::time::Instant::now();
                    match task {
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
                                &mut sink,
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
                                &mut sink,
                            )
                            .await?
                        }
                    };
                    if let Some(b) = &bench {
                        b.record_pack(sink.bytes);
                        // Feeds throughput_mbs; pack mode is the default pull
                        // path, so without this a pack-mode clone reports zero
                        // throughput. Transfer-only accounting, to match the
                        // push side: the ODB writes now happen INSIDE the fetch,
                        // so their accumulated time is subtracted rather than
                        // silently folded into throughput.
                        let net = fetch_start.elapsed().saturating_sub(sink.write_time);
                        b.record_batch(sink.written.len() as u64, sink.bytes, net);
                    }
                    Ok::<Vec<Oid>, anyhow::Error>(sink.written)
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

/// Where a pack fetch puts each verified chunk, and what it reports back.
///
/// EXISTS TO BOUND MEMORY. Both fetch functions used to return
/// `Vec<(Oid, Vec<u8>)>` — every chunk of one pack — and the caller wrote them
/// afterwards. Under `buffer_unordered(N)` that is N WHOLE PACKS resident at
/// once. Measured 2026-09-15 on the 16 GB corpus: 66 MB packs at N=24 gave a
/// client peak of 1,594 MB (gcs) / 1,157 MB (aws, azure) — against a push peak
/// of ~293 MB on the same run, and a figure this project had previously driven
/// from 1,075 MB down to 289 MB.
///
/// Writing each chunk as its range is sliced holds ONE RANGE (<= the coalesce
/// cap, 8 MiB) per task instead of a whole pack, without touching concurrency,
/// so there is no throughput trade.
///
/// The integrity check is unchanged and unmoved: `put_compressed_chunk`
/// decompresses and refuses to store unless BLAKE3 matches the chunk_id the
/// client itself asked for. This runs exactly where the caller's loop used to
/// run it, only earlier in time.
///
/// `write_time` is accumulated so the caller can subtract it and keep `[bench]`
/// throughput meaning transfer-only, which is what it documented before the
/// writes moved inside.
struct PackChunkSink<'a> {
    odb: &'a ObjectDatabase,
    progress: Option<&'a std::sync::Arc<dyn Fn(u64) + Send + Sync>>,
    written: Vec<Oid>,
    bytes: u64,
    write_time: std::time::Duration,
}

impl<'a> PackChunkSink<'a> {
    fn new(
        odb: &'a ObjectDatabase,
        progress: Option<&'a std::sync::Arc<dyn Fn(u64) + Send + Sync>>,
    ) -> Self {
        Self {
            odb,
            progress,
            written: Vec::new(),
            bytes: 0,
            write_time: std::time::Duration::ZERO,
        }
    }

    /// Store one verified slice. `data` is borrowed and not retained, so the
    /// range body it points into can be dropped as soon as the range is done.
    async fn accept(&mut self, oid: Oid, data: &[u8]) -> Result<()> {
        let started = std::time::Instant::now();
        self.odb
            .put_compressed_chunk(&oid, data)
            .await
            .with_context(|| format!("write chunk {} to ODB", oid))?;
        self.write_time += started.elapsed();
        self.bytes += data.len() as u64;
        if let Some(cb) = self.progress {
            cb(data.len() as u64);
        }
        self.written.push(oid);
        Ok(())
    }
}

/// Wall-clock budget for retrying ONE pack Range-GET, via
/// `MEDIAGIT_PACK_RANGE_GET_RETRY_BUDGET_SECS` (default 120s).
///
/// Deliberately a separate name from `MEDIAGIT_PACK_PUT_RETRY_BUDGET_SECS`:
/// they bound opposite directions of transfer, and this codebase has already
/// paid for one env var silently driving two axes. Same default, because the
/// same reasoning applies — long enough to ride out a WAN blip, short enough
/// that a genuinely dead range reaches the per-chunk fallback while the clone
/// still has somewhere to go.
fn pack_range_get_retry_budget() -> std::time::Duration {
    let secs = std::env::var("MEDIAGIT_PACK_RANGE_GET_RETRY_BUDGET_SECS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .filter(|&n| n > 0)
        .unwrap_or(120);
    std::time::Duration::from_secs(secs)
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
                "pack manifest carries no compressed_hash for this chunk; \
                    refusing the slice unverified and falling back to per-chunk"
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
    sink: &mut PackChunkSink<'_>,
) -> Result<()> {
    let ranges = coalesce_chunk_ranges(chunks, coalesce_max_gap, coalesce_max_bytes);

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

        // The requested range maps 1:1 onto real chunk offsets, so a compliant
        // body is at least `range_end - range_start` long. A shorter body means a
        // truncated/partial response (a backend degrading under load returns
        // short reads) — a chunk near the tail would then mis-slice or silently
        // shrink, so a short body is a failed attempt, never trusted.
        let expected_len = (range_end - range_start) as usize;

        // RETRIED, since 2026-09-15. This had NO retry at all: one transport
        // error, one non-206, or one short body bailed the whole range, and
        // `pull_chunks_via_packs`'s `try_fold` then short-circuited the batch —
        // dropping the clone onto the per-chunk proxy path, which on the 16 GB
        // corpus is 4,073 requests against 155. A single blip on a WAN link
        // therefore cost a ~26x slowdown. The upload side already learned this
        // (MEDIAGIT_PACK_PUT_RETRY_BUDGET_SECS); the download side had nothing.
        //
        // Budget is elapsed-time, not an attempt count, and is measured from the
        // first attempt so a slow failing attempt spends its own time — the same
        // shape as the PUT budget, and deliberately NOT a fixed per-attempt
        // timer, which is a cliff against bandwidth-dependent work.
        //
        // Permanent outcomes still bail immediately: falling back to per-chunk
        // is correct for a genuinely absent or forbidden object, and retrying it
        // would burn the budget to reach the same place. `RefreshUrl` (an
        // expired signature) also bails, because this function holds no server
        // handle to re-presign with — the per-chunk fallback re-presigns
        // naturally. That is a real limitation, recorded rather than hidden.
        //
        // Presigned direct-to-bucket GET — not routed through
        // send_with_rate_limit_retry, which only applies to the server's own
        // 429s and Retry-After semantics.
        let started = std::time::Instant::now();
        let budget = pack_range_get_retry_budget();
        let mut attempt: u32 = 0;
        let body = loop {
            attempt += 1;
            let why: String = match client.get(url).header("Range", &hdr).send().await {
                Ok(resp) => {
                    let status = resp.status().as_u16();
                    if !range_status_trusted(status, range_start) {
                        let text = resp.text().await.unwrap_or_default();
                        match crate::error_class::classify_auto_get(status, url, "", "", &text) {
                            crate::error_class::TransferOutcome::Transient => {
                                format!("status {status} (want 206)")
                            }
                            other => anyhow::bail!(
                                "Range-GET returned {} (want 206) for pack {} range {} — \
                                 not retryable ({:?}); falling back to per-chunk",
                                status,
                                pack_oid,
                                hdr,
                                other
                            ),
                        }
                    } else {
                        match resp.bytes().await {
                            Ok(b) if b.len() >= expected_len => break b,
                            Ok(b) => {
                                format!("short body: got {} of {} bytes", b.len(), expected_len)
                            }
                            Err(e) => format!("read body: {e}"),
                        }
                    }
                }
                Err(e) => format!("transport: {e}"),
            };

            let elapsed = started.elapsed();
            if elapsed >= budget {
                anyhow::bail!(
                    "Range-GET for pack {} range {} failed after {} attempt(s) in {:?} \
                     (budget {:?}, MEDIAGIT_PACK_RANGE_GET_RETRY_BUDGET_SECS): {}",
                    pack_oid,
                    hdr,
                    attempt,
                    elapsed,
                    budget,
                    why
                );
            }
            tracing::debug!(
                pack = %pack_oid, range = %hdr, attempt, elapsed_ms = elapsed.as_millis() as u64,
                why = %why, "pack Range-GET attempt failed; retrying"
            );
            // 200ms, 400, 800, 1600, 3200, then flat — capped so a long budget
            // does not turn into a few very late attempts.
            let shift = attempt.min(5) - 1;
            tokio::time::sleep(std::time::Duration::from_millis(200u64 << shift)).await;
        };

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

            // Written HERE, not accumulated: `data` borrows `body`, so the range
            // body is the only pack bytes resident and it is dropped at the end
            // of this iteration. See PackChunkSink for the measurement.
            sink.accept(oid, data).await?;
        }
    }
    Ok(())
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
/// On HTTP 404 (server predates this endpoint) returns `Ok(())` having written
/// nothing, so only this pack's chunks fall back to per-chunk — other packs in
/// the same pull are unaffected.
///
/// MEMORY, stated honestly: writing through the sink removes the accumulated
/// copy of the pack, but this path still buffers the whole RESPONSE body plus
/// the frames parsed out of it, because batch-get answers one pack in one
/// response — that is the endpoint's shape, not something this function can
/// stream around. So it improves from roughly 3x pack size to 2x, where the
/// presigned path (the default, and the one measured at 1,594 MB) drops to one
/// coalesced range. This path is the GCS-ADC fallback and was not exercised in
/// the 16 GB runs (`presign_urls == pack_count` on all three backends).
async fn fetch_pack_slices_batch(
    client: &reqwest::Client,
    base_url: &str,
    pack_oid: &str,
    chunks: &[(String, u64, u32)],
    comp_hashes: &std::collections::HashMap<String, String>,
    sink: &mut PackChunkSink<'_>,
) -> Result<()> {
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
        return Ok(());
    }
    if !resp.status().is_success() {
        anyhow::bail!("batch-get returned {} for pack {}", resp.status(), pack_oid);
    }

    let body = resp.bytes().await.context("read batch-get body")?;
    let frames = parse_batch_get_frames(&body, pack_oid);

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

        sink.accept(oid, &data).await?;
    }
    Ok(())
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
            "a manifest with no compressed_hash left the slice unverified and \
                accepted it — fail-open on an integrity check"
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

    /// A pack's chunks must reach the ODB AS EACH RANGE ARRIVES, not be
    /// collected and written after the whole pack is fetched.
    ///
    /// This is the gate for the memory fix, and it is deliberately not a
    /// correctness test: the accumulating version this replaces produced
    /// byte-identical results, so nothing about the stored chunks can
    /// distinguish the two. What distinguishes them is TIMING — with
    /// accumulation, nothing is written until every range is done.
    ///
    /// The payload is split across two ranges (the coalesce byte cap is set
    /// below one entry's size so the two entries cannot merge). The mock bucket
    /// checks, at the moment it is asked for the SECOND range, whether the
    /// first chunk is already in the ODB. Under streaming it is; under
    /// accumulation it cannot be.
    ///
    /// Why it matters: under `buffer_unordered(24)`, accumulation means 24
    /// whole packs resident. Measured 2026-09-15 on the 16 GB corpus at 66 MB
    /// packs — 1,594 MB client peak on gcs, 1,157 MB on aws and azure, against
    /// a ~293 MB push peak in the same run.
    #[tokio::test]
    async fn pack_chunks_are_written_as_ranges_arrive_not_after_the_whole_pack() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        crate::ensure_crypto_provider();

        // Two entries, each `type(1) + size(4) + data`, laid end to end.
        let d1 = b"first-chunk-stored-bytes".to_vec();
        let d2 = b"second-chunk-stored-bytes".to_vec();
        let oid1 = Oid::hash(&d1);
        let oid2 = Oid::hash(&d2);
        let e1: u32 = (5 + d1.len()) as u32;
        let e2: u32 = (5 + d2.len()) as u32;
        let mut pack = vec![0u8];
        pack.extend_from_slice(&(d1.len() as u32).to_le_bytes());
        pack.extend_from_slice(&d1);
        pack.push(0u8);
        pack.extend_from_slice(&(d2.len() as u32).to_le_bytes());
        pack.extend_from_slice(&d2);

        let chunks = vec![(oid1.to_hex(), 0u64, e1), (oid2.to_hex(), e1 as u64, e2)];
        let mut comp = std::collections::HashMap::new();
        comp.insert(oid1.to_hex(), Oid::hash(&d1).to_hex());
        comp.insert(oid2.to_hex(), Oid::hash(&d2).to_hex());

        let (_tmp, odb) = test_odb().await;

        // Serves byte ranges out of `pack`, and on the second request records
        // whether chunk 1 has already landed in the ODB.
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let seen_first_before_second =
            std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let flag = seen_first_before_second.clone();
        let probe_odb = odb.clone();
        let pack_for_server = pack.clone();
        let srv = tokio::spawn(async move {
            let mut n = 0usize;
            loop {
                let Ok((mut sock, _)) = listener.accept().await else {
                    return;
                };
                n += 1;
                let mut buf = [0u8; 4096];
                let read = sock.read(&mut buf).await.unwrap_or(0);
                let head = String::from_utf8_lossy(&buf[..read]).to_string();

                if n == 2 {
                    // THE ASSERTION, made from the server side: by the time the
                    // client asks for the second range, the first range's chunk
                    // must already be stored.
                    if probe_odb.chunk_exists(&oid1).await.unwrap_or(false) {
                        flag.store(true, std::sync::atomic::Ordering::SeqCst);
                    }
                }

                // Parse "Range: bytes=START-END" (inclusive end).
                let (mut start, mut end) = (0usize, pack_for_server.len() - 1);
                if let Some(idx) = head.to_lowercase().find("range: bytes=") {
                    let tail = &head[idx + "range: bytes=".len()..];
                    let spec: String = tail
                        .chars()
                        .take_while(|c| *c != '\r' && *c != '\n')
                        .collect();
                    if let Some((a, b)) = spec.split_once('-') {
                        start = a.trim().parse().unwrap_or(0);
                        end = b.trim().parse().unwrap_or(pack_for_server.len() - 1);
                    }
                }
                let end = end.min(pack_for_server.len().saturating_sub(1));
                let body = &pack_for_server[start..=end];
                let mut resp = format!(
                    "HTTP/1.1 206 Partial Content\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                )
                .into_bytes();
                resp.extend_from_slice(body);
                let _ = sock.write_all(&resp).await;
                let _ = sock.flush().await;
            }
        });

        let mut sink = PackChunkSink::new(&odb, None);
        let client = reqwest::Client::new();
        // max_gap 0 and a byte cap below one entry: the two entries cannot be
        // coalesced, so this is genuinely two Range-GETs.
        fetch_pack_slices_presigned(
            &client,
            &format!("http://{addr}/pack"),
            "packtest",
            &chunks,
            0,
            (e1 as u64).saturating_sub(1),
            &comp,
            None,
            &mut sink,
        )
        .await
        .expect("two-range fetch must succeed");

        srv.abort();

        assert_eq!(
            sink.written.len(),
            2,
            "expected both chunks stored, got {:?}",
            sink.written.len()
        );
        assert!(
            seen_first_before_second.load(std::sync::atomic::Ordering::SeqCst),
            "the first chunk was NOT in the ODB when the second range was requested — \
             the fetch is accumulating the whole pack before writing, which is what \
             put 24 concurrent packs (1,594 MB) in memory on a 16 GB clone"
        );
    }

    /// A pack Range-GET must RIDE OUT a transient failure rather than bailing
    /// the whole range on the first one.
    ///
    /// Before 2026-09-15 this path had no retry at all: one transport error,
    /// one non-206, or one short body bailed, and `pull_chunks_via_packs`'s
    /// `try_fold` short-circuited the batch, dropping the clone onto the
    /// per-chunk proxy path — 4,073 requests against 155 packs on the 16 GB
    /// corpus. A single WAN blip therefore bought a ~26x slowdown.
    ///
    /// Serves 503 twice, then the real bytes. Asserts the slice comes back AND
    /// that it took more than one request, so this cannot pass against a server
    /// that never failed in the first place.
    #[tokio::test]
    async fn pack_range_get_rides_out_a_transient_failure() {
        // Required before building a reqwest::Client: the rustls provider is
        // process-global and installed by whichever test runs first, so without
        // this the test passes in a full run and panics when run filtered.
        crate::ensure_crypto_provider();
        let (url, hits, _srv) = spawn_range_server(RangeServerMode::FailThenServe(2)).await;

        let (chunks, comp_hashes, want_oid, want_data) = single_chunk_fixture();
        let (_tmp, odb) = test_odb().await;
        let mut sink = PackChunkSink::new(&odb, None);
        let client = reqwest::Client::new();
        fetch_pack_slices_presigned(
            &client,
            &url,
            "packtest",
            &chunks,
            1 << 20,
            8 << 20,
            &comp_hashes,
            None,
            &mut sink,
        )
        .await
        .expect("transient failures must be retried, not fatal");

        assert_eq!(sink.written.len(), 1, "expected the slice after retrying");
        assert_eq!(sink.written[0], want_oid);
        // The ODB accepted it, which means put_compressed_chunk's BLAKE3 check
        // passed against the requested chunk_id — a stronger statement than
        // comparing the returned buffer used to be.
        assert_eq!(
            sink.bytes,
            want_data.len() as u64,
            "retried slice stored the wrong number of bytes"
        );
        let seen = hits.load(std::sync::atomic::Ordering::SeqCst);
        assert!(
            seen >= 3,
            "server saw only {seen} request(s) — the transient failures were not \
             retried, so this test would also pass against a server that never failed"
        );
    }

    /// The other half: a PERMANENT status must NOT be retried. Burning the
    /// budget to reach the same answer only delays the per-chunk fallback, which
    /// is the correct destination for a genuinely absent object.
    #[tokio::test]
    async fn pack_range_get_does_not_retry_a_permanent_status() {
        crate::ensure_crypto_provider();
        let (url, hits, _srv) = spawn_range_server(RangeServerMode::AlwaysStatus(404)).await;

        let (chunks, comp_hashes, _, _) = single_chunk_fixture();
        let (_tmp, odb) = test_odb().await;
        let mut sink = PackChunkSink::new(&odb, None);
        let client = reqwest::Client::new();
        let started = std::time::Instant::now();
        let res = fetch_pack_slices_presigned(
            &client,
            &url,
            "packtest",
            &chunks,
            1 << 20,
            8 << 20,
            &comp_hashes,
            None,
            &mut sink,
        )
        .await;

        assert!(
            res.is_err(),
            "a 404 range must surface as an error, not as an empty success"
        );
        assert_eq!(
            hits.load(std::sync::atomic::Ordering::SeqCst),
            1,
            "a permanent status was retried; it must bail straight to the per-chunk fallback"
        );
        assert!(
            started.elapsed() < std::time::Duration::from_secs(5),
            "permanent failure took {:?} — it burned retry budget it should not have",
            started.elapsed()
        );
    }

    /// A pack whose bytes do not match the manifest must be REFUSED by the
    /// fetch path, not returned to the caller.
    ///
    /// `slice_verifies` has three unit tests above, but they call it directly.
    /// They prove the function is correct; they prove NOTHING about whether
    /// `fetch_pack_slices_presigned` actually calls it — the same "unit-tested
    /// guard that is never invoked" shape that was found in
    /// `presign_pack_downloads` on 2026-09-15, where sabotaging the call site
    /// left four unit tests green while the gate did nothing.
    ///
    /// This matters more since presigned URLs are now minted for UNVERIFIED
    /// packs: the client's own refusal is what makes that safe. Returning an
    /// empty set is the correct outcome — the caller's set-diff then refetches
    /// those chunks via the per-chunk path, which verifies independently.
    #[tokio::test]
    async fn a_corrupted_pack_slice_is_refused_by_the_fetch_path() {
        crate::ensure_crypto_provider();
        let (url, _hits, _srv) = spawn_range_server(RangeServerMode::ServeCorrupted).await;

        let (chunks, comp_hashes, _oid, _want_data) = single_chunk_fixture();
        let (_tmp, odb) = test_odb().await;
        let mut sink = PackChunkSink::new(&odb, None);
        let client = reqwest::Client::new();
        fetch_pack_slices_presigned(
            &client,
            &url,
            "packtest",
            &chunks,
            1 << 20,
            8 << 20,
            &comp_hashes,
            None,
            &mut sink,
        )
        .await
        .expect("a failed hash check is a skip, not a hard error — the caller falls back");

        assert!(
            sink.written.is_empty(),
            "fetch_pack_slices_presigned accepted {} slice(s) whose bytes do not match \
             the manifest hash. slice_verifies is not wired into this path, so corrupt \
             pack bytes would reach put_compressed_chunk — and a presigned URL is now \
             minted for unverified packs precisely because this refusal exists.",
            sink.written.len()
        );
        assert_eq!(
            sink.bytes, 0,
            "corrupt bytes were written to the ODB rather than refused"
        );
    }

    enum RangeServerMode {
        /// Answer 503 this many times, then serve the real bytes.
        FailThenServe(usize),
        /// Always answer this status.
        AlwaysStatus(u16),
        /// Serve 206 with bytes that do NOT match the manifest's hash.
        ServeCorrupted,
    }

    /// `(chunks, comp_hashes, expected_oid, expected_stored_bytes)` — the four
    /// things every range-fetch test needs to drive the call and check it.
    type ChunkFixture = (
        Vec<(String, u64, u32)>,
        std::collections::HashMap<String, String>,
        Oid,
        Vec<u8>,
    );

    /// A temp-backed ObjectDatabase for the sink to write into.
    ///
    /// Unkeyed, so `smart_compressor` is `None` and `put_compressed_chunk`
    /// takes its plain arm: it hashes the stored bytes directly, which is why
    /// the fixture below uses `Oid::hash(&data)` as the chunk id. A real ODB
    /// rather than a stand-in, so these tests exercise the actual
    /// write-and-verify path the production sink uses.
    async fn test_odb() -> (tempfile::TempDir, ObjectDatabase) {
        let tmp = tempfile::TempDir::new().expect("tempdir");
        let backend = std::sync::Arc::new(
            mediagit_storage::LocalBackend::new(tmp.path().to_str().unwrap())
                .await
                .expect("local backend"),
        );
        let odb = ObjectDatabase::new(backend, 16);
        (tmp, odb)
    }

    /// One pack entry laid out as `type(1) + size(4) + data`, at offset 0.
    fn single_chunk_fixture() -> ChunkFixture {
        let data = b"the-stored-chunk-bytes".to_vec();
        // Must be the hash of the STORED bytes: that is what the ODB recomputes
        // on write, and what slice_verifies compares against.
        let oid = Oid::hash(&data);
        let hex = oid.to_hex();
        let entry_len = (5 + data.len()) as u32;
        let mut m = std::collections::HashMap::new();
        // slice_verifies compares BLAKE3 of the STORED bytes.
        m.insert(hex.clone(), Oid::hash(&data).to_hex());
        (vec![(hex, 0u64, entry_len)], m, oid, data)
    }

    fn pack_body() -> Vec<u8> {
        let (_, _, _, data) = single_chunk_fixture();
        let mut body = vec![0u8];
        body.extend_from_slice(&(data.len() as u32).to_le_bytes());
        body.extend_from_slice(&data);
        body
    }

    /// Minimal HTTP/1.1 server answering one Range-GET per connection. Raw TCP
    /// rather than a framework: this crate has no test HTTP server, and the
    /// status line needs to be controlled exactly.
    #[allow(clippy::type_complexity)]
    async fn spawn_range_server(
        mode: RangeServerMode,
    ) -> (
        String,
        std::sync::Arc<std::sync::atomic::AtomicUsize>,
        tokio::task::JoinHandle<()>,
    ) {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let hits = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let hits2 = hits.clone();
        let handle = tokio::spawn(async move {
            loop {
                let Ok((mut sock, _)) = listener.accept().await else {
                    return;
                };
                let n = hits2.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
                let mut buf = [0u8; 4096];
                let _ = sock.read(&mut buf).await;
                let resp: Vec<u8> = match mode {
                    RangeServerMode::AlwaysStatus(code) => format!(
                        "HTTP/1.1 {code} Err\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                    )
                    .into_bytes(),
                    RangeServerMode::FailThenServe(fail_n) if n <= fail_n => {
                        "HTTP/1.1 503 Service Unavailable\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                            .to_string()
                            .into_bytes()
                    }
                    RangeServerMode::FailThenServe(_) | RangeServerMode::ServeCorrupted => {
                        let mut body = pack_body();
                        if matches!(mode, RangeServerMode::ServeCorrupted) {
                            // Flip a payload byte: same length, same framing,
                            // wrong content — exactly what a mis-assembled
                            // multipart object or a bit-flip looks like.
                            *body.last_mut().unwrap() ^= 0xFF;
                        }
                        let mut head = format!(
                            "HTTP/1.1 206 Partial Content\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                            body.len()
                        )
                        .into_bytes();
                        head.extend_from_slice(&body);
                        head
                    }
                };
                let _ = sock.write_all(&resp).await;
                let _ = sock.flush().await;
            }
        });
        (format!("http://{addr}/pack"), hits, handle)
    }
}
