// SPDX-License-Identifier: BUSL-1.1
// Copyright (C) 2025-2026 Aswin Krishnamoorthy

use anyhow::{Context, Result};
use mediagit_versioning::{CloudPackResult, ObjectType, Oid, PackKind, StreamingPackWriter};
use std::path::PathBuf;

/// How long to keep retrying ONE pack PUT through transport and transient-status
/// failures, in seconds. `MEDIAGIT_PACK_PUT_RETRY_BUDGET_SECS`; `0` restores the
/// old attempt-count-only behaviour.
///
/// WHY A TIME BUDGET AND NOT AN ATTEMPT COUNT. The previous bound was five
/// attempts with an equal-jitter backoff whose ceiling doubled from 250ms, so
/// the four waits summed to 1.9-3.75s. That is the total amount of link trouble
/// a pack upload could survive: under four seconds. 20260901-ga38 measured both
/// halves of that -- the waits directly (`wait_ms` 130-227) and the consequence:
/// five `pack push FAILED after retries`, with the gcs arm registering 16 of 32
/// packs and falling back to 97 per-chunk proxy PUTs. A link that wobbles for
/// ten seconds -- ordinary on Wi-Fi, routine on a saturated uplink -- exhausts
/// the budget and drops the push off the cloud-pack fast path.
///
/// NOT evidenced by ga39's azure arm, despite the superficially similar
/// 32-vs-16: there all 16 packs registered, ChunkPuts was 0 and the push exited
/// 0. That was the existing retry SUCCEEDING via a re-signed URL, and the gate
/// misreporting it -- fixed separately in `Get-QaPackFastPathCounts`. Cited here
/// because mistaking a working retry for a failing one is how a budget gets
/// tuned against noise.
///
/// An outage is measured in seconds-to-minutes, so the bound that matters is
/// wall-clock, not a count. 120s rides out a typical reconnect while still
/// failing in bounded time against a backend that is genuinely gone.
///
/// Retrying is safe to do at length here: the PUT targets a content-addressed
/// key with the same pack bytes, so re-sending is idempotent. On a healthy link
/// nothing changes at all -- the first attempt succeeds and none of this runs.
fn pack_put_retry_budget_secs() -> u64 {
    std::env::var("MEDIAGIT_PACK_PUT_RETRY_BUDGET_SECS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(120)
}

/// Equal-jitter backoff for pack PUT retries, capped so the wait actually
/// reaches a useful size.
///
/// Half fixed so the wait grows, half random so concurrent pack uploads do not
/// retry in lockstep -- which matters here, since ga11's GCS failures arrived as
/// a simultaneous burst.
///
/// The 30s ceiling only raises the old 16s one; it is NOT where the bug was.
/// The old schedule never got near its own ceiling because it stopped at five
/// attempts -- waits of 0.125-0.25, 0.25-0.5, 0.5-1 and 1-2s, so 1.9-3.75s in
/// total. The budget in `pack_put_should_give_up` is the actual fix.
fn pack_put_backoff_ms(attempt: u32) -> u64 {
    const CEILING_MAX_MS: u64 = 30_000;
    let ceiling = (250u64.saturating_mul(1u64 << attempt.min(10))).min(CEILING_MAX_MS);
    ceiling / 2 + crate::client::jitter_upto(ceiling / 2)
}

/// Whether a pack PUT has run out of room to retry.
///
/// Extracted as a free function because it IS the fix: the old bound was a bare
/// `attempt + 1 == 5`, which let a pack upload survive only 1.9-3.75s of link
/// trouble regardless of how long the outage actually lasted. Keeping it inline
/// would have left the one load-bearing rule untestable, and the backoff curve
/// -- which barely changed -- as the only thing under test.
///
/// `budget` of zero restores the historical five-attempt behaviour exactly, so
/// `MEDIAGIT_PACK_PUT_RETRY_BUDGET_SECS=0` is a true revert switch.
fn pack_put_should_give_up(
    attempt: u32,
    elapsed: std::time::Duration,
    wait_ms: u64,
    budget: std::time::Duration,
    max_attempts: u32,
) -> bool {
    if attempt + 1 >= max_attempts {
        return true;
    }
    if budget.is_zero() {
        return attempt + 1 >= 5;
    }
    elapsed + std::time::Duration::from_millis(wait_ms) > budget
}

fn bytes_to_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{:02x}", b)).collect()
}

// ST-4: caps come from `mediagit_versioning::pack` so this path and
// `gc --repack` cannot disagree about them, and so an out-of-range value is
// clamped in exactly one place. They were previously parsed here and again in
// the ODB, unclamped in both.
use mediagit_versioning::{pack_bytes_cap, pack_chunks_cap};

/// Client-side cloud pack assembler.
///
/// Accumulates chunks and flushes a `CloudPackResult` when the byte cap
/// (`MEDIAGIT_PACK_BYTES`, default 64 MiB) or chunk cap
/// (`MEDIAGIT_PACK_CHUNKS`, default 1024) is reached.
///
/// Call `finish()` after all chunks are added to flush any remainder.
pub struct PackBuilder {
    temp_dir: PathBuf,
    writer: Option<StreamingPackWriter<tokio::fs::File>>,
    current_bytes: u64,
    current_chunks: u32,
    bytes_cap: u64,
    chunks_cap: u32,
}

impl PackBuilder {
    pub fn new(temp_dir: impl Into<PathBuf>) -> Self {
        Self {
            temp_dir: temp_dir.into(),
            writer: None,
            current_bytes: 0,
            current_chunks: 0,
            bytes_cap: pack_bytes_cap(),
            chunks_cap: pack_chunks_cap(),
        }
    }

    pub fn with_caps(temp_dir: impl Into<PathBuf>, bytes_cap: u64, chunks_cap: u32) -> Self {
        Self {
            temp_dir: temp_dir.into(),
            writer: None,
            current_bytes: 0,
            current_chunks: 0,
            bytes_cap,
            chunks_cap,
        }
    }

    /// Add a full (uncompressed) chunk to the current pack.
    ///
    /// Returns `Some(CloudPackResult)` if the pack was sealed (cap hit), or
    /// `None` if still accumulating.
    pub async fn add_chunk(&mut self, oid: Oid, data: &[u8]) -> Result<Option<CloudPackResult>> {
        if self.writer.is_none() {
            let w = StreamingPackWriter::new_open_ended(PackKind::CloudObject, &self.temp_dir)
                .await
                .context("create pack writer")?;
            self.writer = Some(w);
        }

        let w = self.writer.as_mut().unwrap();
        w.write_object(oid, ObjectType::Blob, data)
            .await
            .context("write chunk to pack")?;

        // +5 for the per-object type(1) + size(4) header bytes
        self.current_bytes += data.len() as u64 + 5;
        self.current_chunks += 1;

        if self.current_bytes >= self.bytes_cap || self.current_chunks >= self.chunks_cap {
            Ok(Some(self.seal().await?))
        } else {
            Ok(None)
        }
    }

    /// Force-flush all remaining chunks regardless of count. Returns `None` if
    /// the pack is empty.
    pub async fn finish(&mut self) -> Result<Option<CloudPackResult>> {
        if self.writer.is_none() || self.current_chunks == 0 {
            return Ok(None);
        }
        Ok(Some(self.seal().await?))
    }

    async fn seal(&mut self) -> Result<CloudPackResult> {
        let writer = self.writer.take().expect("seal called with no writer");
        let result = writer.finalize_cloud().await.context("finalize pack")?;
        self.current_bytes = 0;
        self.current_chunks = 0;
        tracing::debug!(
            pack_oid = bytes_to_hex(&result.pack_oid),
            byte_len = result.byte_len,
            chunks = result.index.len(),
            "Pack sealed"
        );
        Ok(result)
    }

    pub fn current_chunks(&self) -> u32 {
        self.current_chunks
    }

    pub fn current_bytes(&self) -> u64 {
        self.current_bytes
    }
}

/// Upload a finished pack to cloud storage via a presigned PUT URL and
/// register its manifest with the server.
///
/// This is the network-side of F4: the PackBuilder handles disk assembly,
/// `upload_and_register` handles transport and server registration.
/// `base_url` must already include the repo segment (e.g. `http://server/my-repo`).
///
/// Returns `true` when the pack bytes went out over a presigned PUT straight
/// to the bucket, `false` when they were proxied through the server. Callers
/// use this to distinguish the two in `[bench]` output — a presign request
/// that silently degraded to the proxy otherwise looks identical to a
/// successful direct upload.
pub async fn upload_and_register(
    result: CloudPackResult,
    base_url: &str,
    http_client: &reqwest::Client,
    direct_client: &reqwest::Client,
    compressed_hashes: &[(String, String)],
) -> Result<bool> {
    let pack_oid_hex = bytes_to_hex(&result.pack_oid);
    let byte_len = result.byte_len;
    let mut presigned_direct = false;

    // 1. Request presigned PUT URL for packs/<pack_oid>
    let presign_url = format!("{}/packs/upload-urls", base_url);
    let presign_body = serde_json::json!({
        "pack_ids": [pack_oid_hex],
        "sizes": [byte_len],
    });
    let presign_resp = crate::client::send_with_rate_limit_retry(|| {
        http_client.post(&presign_url).json(&presign_body).send()
    })
    .await
    .context("POST /packs/upload-urls")?;

    if !presign_resp.status().is_success() {
        anyhow::bail!("POST /packs/upload-urls returned {}", presign_resp.status());
    }

    let presign_map: std::collections::HashMap<String, Option<serde_json::Value>> = presign_resp
        .json()
        .await
        .context("parse /packs/upload-urls response")?;

    // 2. Upload pack bytes
    // B5: Bytes so the proxy-fallback retry closure below clones a refcount
    // bump, not the whole pack.
    let pack_data: bytes::Bytes = tokio::fs::read(&result.temp_path)
        .await
        .context("read pack temp file")?
        .into();

    if let Some(Some(purl)) = presign_map.get(&pack_oid_hex) {
        let put_url = purl["url"].as_str().unwrap_or("").to_string();
        // Presigned direct-to-bucket PUT — not server-bound, so not routed
        // through send_with_rate_limit_retry: a 429 here comes from the BUCKET,
        // not the server's limiter, and needs backend-specific classification.
        //
        // This comment used to claim the 429 was "handled by the caller's own
        // retry loop". There is no such loop. The caller
        // (`client/push.rs:783-812`) only catches the error and abandons the
        // cloud-pack path for the ENTIRE push, so ONE transient status on ONE
        // pack dropped everything to per-chunk upload:
        //
        //   20260821-ga8   96 upload-urls -> 97 packs/complete,     0 per-chunk,  8.69 MB/s
        //   20260821-ga11  96 upload-urls ->  0 packs/complete, 2,284 per-chunk,  0.98 MB/s
        //
        // Azure and GCS both threw transients in ga11 (4 each; GCS's arrived as
        // a burst inside one second, the shape of throttling). Neither was
        // retried even once.
        //
        // Reuses `error_class::classify_auto`. Permanent* still bails
        // immediately — re-sending a whole pack body against a 403 cannot
        // succeed however long the budget is.
        // Hard backstop only. The real bound is the wall-clock budget below --
        // see `pack_put_retry_budget_secs` for why an attempt count alone was
        // the wrong shape. This stays so a pathological zero-latency failure
        // loop cannot spin unboundedly inside the budget.
        const MAX_PACK_PUT_ATTEMPTS: u32 = 24;
        let retry_budget = std::time::Duration::from_secs(pack_put_retry_budget_secs());
        let put_started = std::time::Instant::now();
        // `true` when there is no time left to sleep and try again, so both the
        // transport and the transient-status arms give up on the same rule.
        let out_of_budget = |attempt: u32, wait_ms: u64| -> bool {
            pack_put_should_give_up(
                attempt,
                put_started.elapsed(),
                wait_ms,
                retry_budget,
                MAX_PACK_PUT_ATTEMPTS,
            )
        };
        let mut last_status = String::new();
        let mut sent = false;
        for attempt in 0..MAX_PACK_PUT_ATTEMPTS {
            // Rebuilt per attempt: `send()` consumes the builder, and `body` is
            // a `Bytes` clone (refcount bump, not a copy of the pack) per B5.
            let mut req = direct_client.put(&put_url).body(pack_data.clone());
            if let Some(headers) = purl["required_headers"].as_array() {
                for h in headers {
                    if let (Some(name), Some(val)) = (
                        h.get(0).and_then(|v| v.as_str()),
                        h.get(1).and_then(|v| v.as_str()),
                    ) {
                        req = req.header(name, val);
                    }
                }
            }
            // A TRANSPORT failure never produces a response, so it can never
            // reach the status classifier below. Retried here, or the whole
            // status-based loop is unreachable on exactly the errors a WAN link
            // actually produces.
            //
            // 20260821-s5check measured this: every pack failure across the AWS
            // and Azure arms was a transport error and NOT ONE was an HTTP
            // status — "connection closed before message completed" (x1) and
            // "operation timed out" (x3). A 503 got five attempts; a dropped
            // connection got zero, and the push abandoned the fast path.
            //
            // I argued the other way when writing the status loop: not retrying
            // a timeout avoids re-sending a whole pack body five times. That
            // reasoning ignored which failure is COMMON. Every other uploader
            // here already retries transport errors — the per-chunk MPU path,
            // the control plane, and the server's own aws-sdk-s3, which logged
            // "Failed after 5 retries" in the same run.
            //
            // Every send() error is retried, not a hand-picked subset: no
            // response arrived, so there is nothing to classify as permanent,
            // and the retry BUDGET already caps the cost of being wrong.
            let resp = match req.send().await {
                Ok(r) => r,
                Err(e) => {
                    let wait = pack_put_backoff_ms(attempt);
                    if out_of_budget(attempt, wait) {
                        let spent = put_started.elapsed().as_secs_f64();
                        return Err(anyhow::Error::new(e))
                            .context("presigned PUT of pack")
                            .with_context(|| {
                                format!(
                                    "after {} transport attempts over {spent:.1}s \
                                     (MEDIAGIT_PACK_PUT_RETRY_BUDGET_SECS)",
                                    attempt + 1
                                )
                            });
                    }
                    // WARN, and the level is load-bearing: this went
                    // debug -> info -> warn, and only the last one works.
                    //
                    // main.rs pins the CLI filter to "warn" unless --verbose or
                    // MEDIAGIT_LOG is set, so an info line is invisible in every
                    // default run. 20260821-ga12 proved it: the AWS arm retried
                    // and EXHAUSTED its budget, and the only trace left in a full
                    // campaign log was the phrase "after 5 transport attempts"
                    // buried in the final error - the retries themselves logged
                    // nothing. Moving debug -> info without checking the filter
                    // floor above it changed precisely nothing.
                    //
                    // Being wrong the other way is cheap: this fires at most 4
                    // times per pack and only when the link is already
                    // misbehaving. `pack push FAILED` is already warn! on this
                    // same path and the QA harness parses around it fine.
                    tracing::warn!(
                        attempt = attempt + 1,
                        max_attempts = MAX_PACK_PUT_ATTEMPTS,
                        err = ?e,
                        wait_ms = wait,
                        "pack PUT transport failure; retrying (the push is NOT degraded \
                         unless a later 'pack push FAILED' says so)"
                    );
                    tokio::time::sleep(std::time::Duration::from_millis(wait)).await;
                    continue;
                }
            };
            let status = resp.status();
            if status.is_success() {
                sent = true;
                break;
            }
            last_status = status.to_string();

            let ct = resp
                .headers()
                .get(reqwest::header::CONTENT_TYPE)
                .and_then(|v| v.to_str().ok())
                .unwrap_or("")
                .to_string();
            let hdr_code = resp
                .headers()
                .get("x-amz-error-code")
                .or_else(|| resp.headers().get("x-ms-error-code"))
                .and_then(|v| v.to_str().ok())
                .unwrap_or("")
                .to_string();
            let body_full = resp.text().await.unwrap_or_default();
            let body_ref = if body_full.len() > 2048 {
                &body_full[..2048]
            } else {
                &body_full[..]
            };

            use crate::error_class::{TransferOutcome, classify_auto};
            match classify_auto(status.as_u16(), &put_url, &ct, &hdr_code, body_ref) {
                TransferOutcome::Transient | TransferOutcome::RefreshUrl => {
                    let wait = pack_put_backoff_ms(attempt);
                    if out_of_budget(attempt, wait) {
                        let spent = put_started.elapsed().as_secs_f64();
                        anyhow::bail!(
                            "presigned PUT returned {status} after {} attempts over \
                             {spent:.1}s (MEDIAGIT_PACK_PUT_RETRY_BUDGET_SECS)",
                            attempt + 1
                        );
                    }
                    tracing::debug!(
                        pack = %pack_oid_hex,
                        attempt = attempt + 1,
                        %status,
                        "pack PUT transient error; retrying"
                    );
                    tokio::time::sleep(std::time::Duration::from_millis(wait)).await;
                }
                TransferOutcome::PermanentChunk
                | TransferOutcome::PermanentChunkAfterDelay(_)
                | TransferOutcome::PermanentConfig => {
                    anyhow::bail!("presigned PUT returned {status} (permanent)");
                }
            }
        }
        if !sent {
            anyhow::bail!("presigned PUT returned {last_status}");
        }
        presigned_direct = true;
        tracing::debug!(pack = %pack_oid_hex, bytes = byte_len, "Pack uploaded via presigned URL");
    } else {
        // Proxy fallback: PUT to /packs/<oid> so complete_pack's head("packs/<oid>") succeeds
        let proxy_url = format!("{}/packs/{}", base_url, pack_oid_hex);
        let resp = crate::client::send_with_rate_limit_retry(|| {
            http_client.put(&proxy_url).body(pack_data.clone()).send()
        })
        .await
        .context("proxy PUT of pack")?;
        if !resp.status().is_success() {
            anyhow::bail!("proxy PUT returned {}", resp.status());
        }
        tracing::debug!(pack = %pack_oid_hex, bytes = byte_len, "Pack uploaded via proxy");
    }

    // 3. POST /packs/complete with manifest
    let manifest: Vec<serde_json::Value> = result
        .index
        .iter()
        .map(|loc| {
            let chunk_hex = loc.chunk_oid.to_hex();
            let comp_hash = compressed_hashes
                .iter()
                .find(|(h, _)| h == &chunk_hex)
                .map(|(_, hash)| hash.clone());
            serde_json::json!({
                "chunk_oid": chunk_hex,
                "offset": loc.offset,
                "length": loc.length,
                "compressed_hash": comp_hash,
            })
        })
        .collect();

    let complete_url = format!("{}/packs/complete", base_url);
    let complete_body = serde_json::json!({
        "pack_oid": pack_oid_hex,
        "manifest": manifest,
    });
    let complete_resp = crate::client::send_with_rate_limit_retry(|| {
        http_client.post(&complete_url).json(&complete_body).send()
    })
    .await
    .context("POST /packs/complete")?;

    if !complete_resp.status().is_success() {
        anyhow::bail!("POST /packs/complete returned {}", complete_resp.status());
    }

    // 4. Clean up temp file
    let _ = tokio::fs::remove_file(&result.temp_path).await;

    tracing::info!(
        pack = %pack_oid_hex,
        bytes = byte_len,
        chunks = manifest.len(),
        presigned_direct,
        "Pack registered with server"
    );
    Ok(presigned_direct)
}

#[cfg(test)]
mod pack_put_retry_tests {
    use super::{pack_put_backoff_ms, pack_put_should_give_up};
    use std::time::Duration;

    const MAX: u32 = 24;
    const BUDGET: Duration = Duration::from_secs(120);

    /// THE REGRESSION, stated as a test. The old rule was `attempt + 1 == 5`,
    /// so a fifth transport failure ended the pack upload no matter how little
    /// time had passed -- 20260901-ga39 lost 16 of 16 azure packs that way.
    /// Four seconds into a 120s budget there is plainly room to keep going.
    ///
    /// This assertion FAILS against the old bound, which is the point: the
    /// backoff-curve tests below pass either way and prove nothing on their own.
    #[test]
    fn a_fifth_failure_early_in_the_budget_keeps_retrying() {
        assert!(
            !pack_put_should_give_up(4, Duration::from_secs(4), 2_000, BUDGET, MAX),
            "five quick failures must not end a pack upload with 116s of budget left"
        );
    }

    /// The other half: the budget must actually stop. A guard that only ever
    /// said "keep going" would pass the test above and hang on a dead backend.
    #[test]
    fn an_exhausted_budget_gives_up() {
        assert!(
            pack_put_should_give_up(6, Duration::from_secs(119), 30_000, BUDGET, MAX),
            "a wait that would overrun the budget must end the retry loop"
        );
    }

    /// The attempt backstop still binds, so a zero-latency failure loop cannot
    /// spin forever inside the budget.
    #[test]
    fn the_attempt_backstop_still_binds() {
        assert!(
            pack_put_should_give_up(MAX - 1, Duration::ZERO, 0, BUDGET, MAX),
            "the hard attempt cap must hold even with budget remaining"
        );
    }

    /// `MEDIAGIT_PACK_PUT_RETRY_BUDGET_SECS=0` is a true revert switch.
    #[test]
    fn a_zero_budget_restores_the_old_five_attempt_rule() {
        assert!(!pack_put_should_give_up(
            3,
            Duration::ZERO,
            0,
            Duration::ZERO,
            MAX
        ));
        assert!(pack_put_should_give_up(
            4,
            Duration::ZERO,
            0,
            Duration::ZERO,
            MAX
        ));
    }

    #[test]
    fn backoff_grows_and_is_capped() {
        assert!(pack_put_backoff_ms(0) < pack_put_backoff_ms(6));
        for attempt in 0..40 {
            assert!(
                pack_put_backoff_ms(attempt) <= 30_000,
                "attempt {attempt} exceeded the 30s ceiling"
            );
        }
    }
}
