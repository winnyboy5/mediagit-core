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

use anyhow::{Context, Result};
use mediagit_versioning::{
    Commit, FileMode, ObjectDatabase, ObjectType, Oid, PackWriter, Tag, Tree,
    chunking::ChunkManifest,
};
use std::collections::{HashSet, VecDeque};
use std::sync::{
    Arc,
    atomic::{AtomicU64, AtomicUsize, Ordering},
};

use crate::types::{
    RefUpdate, RefUpdateRequest, RefUpdateResponse, RefsResponse, WantRequest, WantResponse,
};

/// Presigned PUT URL info returned by the server for direct-to-bucket uploads.
#[derive(serde::Deserialize)]
pub(crate) struct PresignedPutInfo {
    url: String,
    #[allow(dead_code)]
    method: String,
    required_headers: Vec<[String; 2]>,
}

/// Presigned GET URL info returned by the server for direct-from-bucket downloads.
#[derive(serde::Deserialize, Clone)]
pub(crate) struct PresignedGetInfo {
    url: String,
    #[allow(dead_code)]
    method: String,
    headers: Vec<(String, String)>,
    #[allow(dead_code)]
    expires_in_secs: u64,
}

/// Location of a chunk within a cloud pack object (F7).
struct PackLocInfo {
    pack_oid: String,
    offset: u64,
    length: u32,
    compressed_hash: Option<String>,
}

/// Result of a `ProtocolClient::repair_remote` chunk-healing pass (BUG-RM-3).
#[derive(Debug, Clone, Default)]
pub struct RepairReport {
    /// Number of chunk ids strong-verified against the remote.
    pub verified: usize,
    /// Chunks the remote reported as invalid and that were successfully
    /// re-uploaded from the local ODB.
    pub repaired: usize,
    /// Chunks the remote reported as invalid but could not be repaired
    /// (missing/unreadable locally, or the re-upload itself failed).
    pub unrepairable: Vec<String>,
}

/// Statistics from a push operation
#[derive(Debug, Clone, Default)]
pub struct PushStats {
    /// Number of objects collected for push
    pub objects_count: usize,
    /// Number of commit objects
    pub commits_count: usize,
    /// Number of tree objects
    pub trees_count: usize,
    /// Number of blob objects
    pub blobs_count: usize,
    /// Bytes uploaded (pack size)
    pub bytes_uploaded: usize,
}

/// Phase of the push operation for progress tracking
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PushPhase {
    /// Collecting reachable objects
    Collecting,
    /// Generating pack file
    Packing,
    /// Uploading pack to server
    Uploading,
}

/// Progress update during push operation
#[derive(Debug, Clone)]
pub struct PushProgress {
    /// Current phase of the push
    pub phase: PushPhase,
    /// Current progress (objects collected, bytes packed, etc.)
    pub current: u64,
    /// Total expected (may be 0 if unknown)
    pub total: u64,
    /// Human-readable message
    pub message: String,
}

/// Client credentials for authenticating requests to a MediaGit server's
/// control-plane endpoints (`self.base_url`-relative: `/info/refs`,
/// `/objects/*`, `/chunks/*`, ...). Never sent to direct/presigned
/// cloud-storage URLs (S3/Azure/GCS/MinIO) — those always go out over a
/// separately-constructed `direct_client` that carries no default headers,
/// so baking credentials into `ProtocolClient`'s own `reqwest::Client`
/// cannot leak them to a third-party bucket.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum Credentials {
    /// No credentials — no `Authorization`/`X-Api-Key` header is sent.
    /// Talking to an authless server with `None` behaves exactly as before
    /// this feature existed (the authless-regression guard).
    #[default]
    None,
    /// JWT bearer token, sent as `Authorization: Bearer <token>`.
    Bearer(String),
    /// API key, sent as `X-Api-Key: <key>`.
    ApiKey(String),
}

impl Credentials {
    fn header(&self) -> Option<(&'static str, String)> {
        match self {
            Credentials::None => None,
            Credentials::Bearer(token) => Some(("authorization", format!("Bearer {token}"))),
            Credentials::ApiKey(key) => Some(("x-api-key", key.clone())),
        }
    }
}

/// Build the control-plane `reqwest::Client` with the given credentials
/// baked in as a default header (present on every request sent through this
/// client instance). Shared by `ProtocolClient::new` and `with_credentials`
/// so both construct the client identically apart from the header.
fn build_control_plane_client(creds: &Credentials) -> reqwest::Client {
    let pool_max = http_pool_max();
    crate::ensure_crypto_provider();
    let mut builder = reqwest::Client::builder()
        // HTTP/2 is used for control-plane traffic (manifest, URL-mint,
        // ref negotiation). Multiplexing many small requests on one TLS
        // connection avoids per-request handshake cost. Data-plane
        // presigned PUT/GET uses separate direct_client (HTTP/1.1).
        .pool_max_idle_per_host(pool_max)
        .pool_idle_timeout(std::time::Duration::from_secs(90))
        .tcp_keepalive(std::time::Duration::from_secs(30))
        .tcp_nodelay(true)
        .http2_adaptive_window(true)
        // Larger HTTP/2 windows reduce flow-control stalls when bulk
        // media chunks ride concurrent streams over one connection.
        .http2_initial_stream_window_size(8 * 1024 * 1024)
        .http2_initial_connection_window_size(32 * 1024 * 1024)
        .http2_keep_alive_interval(Some(std::time::Duration::from_secs(20)))
        // The MediaGit control protocol never legitimately redirects; following
        // one would re-send x-api-key cross-host (reqwest strips Authorization
        // on cross-host redirects but NOT custom headers).
        .redirect(reqwest::redirect::Policy::none());
    // No per-request timeout: large chunked-blob PUTs to Azure/S3
    // (single object up to several hundred MB) can legitimately run
    // for minutes — the server side ships block-by-block to cloud.
    // tcp_keepalive (30s) already detects truly dead peers; a hard
    // request ceiling here causes spurious "error sending request"
    // failures on healthy slow uploads. See dev-tests/azure-manual-
    // test for the regression that motivated removing this.
    if let Some((name, value)) = creds.header()
        && let Ok(header_value) = reqwest::header::HeaderValue::from_str(&value)
    {
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert(reqwest::header::HeaderName::from_static(name), header_value);
        builder = builder.default_headers(headers);
    }
    builder.build().unwrap_or_else(|_| reqwest::Client::new())
}

/// HTTP client for the MediaGit protocol
pub struct ProtocolClient {
    base_url: String,
    client: reqwest::Client,
    /// Credentials attached to every control-plane request via `client`'s
    /// default headers (set in `build_control_plane_client`). Stored so
    /// `with_credentials` can rebuild `client` and so callers can inspect
    /// what's configured.
    credentials: Credentials,
    /// Optional override for parallel chunk-upload fan-out. Takes precedence
    /// over the internal default (32) but is itself overridden by the
    /// `MEDIAGIT_UPLOAD_CONCURRENCY` env var. Set via `with_concurrent_uploads`.
    concurrent_uploads: Option<usize>,
    /// Optional override for parallel chunk-download fan-out. Takes precedence
    /// over the internal default (24) but is itself overridden by the
    /// `MEDIAGIT_DOWNLOAD_CONCURRENCY` env var. Set via `with_concurrent_downloads`.
    concurrent_downloads: Option<usize>,
}

/// Single source of truth for `MEDIAGIT_HTTP_POOL_MAX`.
/// All three reqwest clients (control, upload, download) read this one function
/// so a user-set value applies uniformly. Default 64 is safe for all client types;
/// the prior per-client defaults (64/96/160) were inconsistent (B8 fix).
pub(crate) fn http_pool_max() -> usize {
    std::env::var("MEDIAGIT_HTTP_POOL_MAX")
        .ok()
        .and_then(|s| s.parse().ok())
        .filter(|n: &usize| *n > 0)
        .unwrap_or(64)
}

/// Maximum 429 retries for a single control-plane request.
fn rate_limit_max_retries() -> u32 {
    std::env::var("MEDIAGIT_RATE_LIMIT_RETRIES")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(5)
}

/// Send a control-plane request, waiting out HTTP 429 instead of failing.
///
/// A large push issues one control-plane request per chunk on the
/// server-proxy path (`PUT /chunks/<id>`), plus two per chunk on the staged
/// (MPU) path. That legitimately outruns a rate limiter, and without this the
/// push simply fails — which invites the operator to re-run it by hand. That
/// manual retry loop is what masked a real corruption bug in the 2026-07-22
/// `psds` incident: the 429s aborted each attempt before push ever reached the
/// chunk read that was actually broken.
///
/// Honours the server's `Retry-After` header (seconds) when present, since the
/// limiter already emits it (`use_headers()`), and otherwise backs off
/// exponentially. `make` is called afresh for each attempt because a request
/// body is consumed by sending.
pub(crate) async fn send_with_rate_limit_retry<F, Fut>(
    make: F,
) -> reqwest::Result<reqwest::Response>
where
    F: Fn() -> Fut,
    Fut: std::future::Future<Output = reqwest::Result<reqwest::Response>>,
{
    let max = rate_limit_max_retries();
    let mut attempt = 0u32;
    loop {
        let resp = make().await?;
        if resp.status() != reqwest::StatusCode::TOO_MANY_REQUESTS || attempt >= max {
            return Ok(resp);
        }
        let wait = resp
            .headers()
            .get(reqwest::header::RETRY_AFTER)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.trim().parse::<u64>().ok())
            .map(std::time::Duration::from_secs)
            // No header: exponential backoff, capped so a push cannot stall
            // indefinitely behind a misconfigured limiter.
            .unwrap_or_else(|| std::time::Duration::from_millis(250 * (1u64 << attempt.min(6))));
        attempt += 1;
        tracing::warn!(
            attempt,
            max,
            wait_ms = wait.as_millis() as u64,
            "rate limited (429); backing off before retry"
        );
        tokio::time::sleep(wait).await;
    }
}

pub(crate) mod browse;
pub(crate) mod locks;
pub(crate) mod packs;
pub(crate) mod pull;
pub(crate) mod push;
pub(crate) mod transfer;

pub use locks::LockInfo;

impl ProtocolClient {
    /// Create a new protocol client
    ///
    /// # Arguments
    /// * `base_url` - Base URL of the MediaGit server (e.g., "http://localhost:3000/repo")
    pub fn new(base_url: impl Into<String>) -> Self {
        let credentials = Credentials::None;
        Self {
            base_url: base_url.into(),
            client: build_control_plane_client(&credentials),
            credentials,
            concurrent_uploads: None,
            concurrent_downloads: None,
        }
    }

    /// Attach credentials, sent as a default header (`Authorization: Bearer
    /// <t>` or `X-Api-Key: <k>`) on every control-plane request this client
    /// makes. Rebuilds the internal HTTP client with the same pool/timeout
    /// settings as `new` — direct/presigned cloud-storage requests use their
    /// own separately-built client and never see this header regardless.
    /// `Credentials::None` (the default) attaches no header at all.
    pub fn with_credentials(mut self, credentials: Credentials) -> Self {
        self.client = build_control_plane_client(&credentials);
        self.credentials = credentials;
        self
    }

    /// The credentials currently configured on this client.
    pub fn credentials(&self) -> &Credentials {
        &self.credentials
    }

    /// Override the parallel chunk-upload fan-out used by
    /// `upload_chunked_objects`. Takes precedence over the internal default
    /// of 32, but is still overridden by the `MEDIAGIT_UPLOAD_CONCURRENCY`
    /// env var when that is set. Pass a value derived from
    /// `[performance] upload_concurrency` in the repo config.
    pub fn with_concurrent_uploads(mut self, n: usize) -> Self {
        self.concurrent_uploads = if n > 0 { Some(n) } else { None };
        self
    }

    /// Override the parallel chunk-download fan-out used by
    /// `download_chunked_objects`. Takes precedence over the internal default
    /// of 24, but is still overridden by the `MEDIAGIT_DOWNLOAD_CONCURRENCY`
    /// env var when that is set. Pass a value derived from
    /// `[performance] download_concurrency` in the repo config.
    pub fn with_concurrent_downloads(mut self, n: usize) -> Self {
        self.concurrent_downloads = if n > 0 { Some(n) } else { None };
        self
    }

    /// Create a shallow copy of this client with a different concurrent_downloads
    /// cap. reqwest::Client is Arc-based so the connection pool is shared.
    /// Used by the parallel fetch path to bound per-branch download fan-out.
    pub fn with_download_cap(&self, n: usize) -> Self {
        Self {
            base_url: self.base_url.clone(),
            client: self.client.clone(),
            credentials: self.credentials.clone(),
            concurrent_uploads: self.concurrent_uploads,
            concurrent_downloads: Some(n),
        }
    }

    /// Get all refs from the remote repository
    pub async fn get_refs(&self) -> Result<RefsResponse> {
        let url = format!("{}/info/refs", self.base_url);
        tracing::debug!("GET {}", url);

        let response = self
            .client
            .get(&url)
            .send()
            .await
            .context("Failed to send GET /info/refs")?;

        if !response.status().is_success() {
            anyhow::bail!("GET /info/refs failed with status: {}", response.status());
        }

        response
            .json::<RefsResponse>()
            .await
            .context("Failed to parse refs response")
    }

    /// Get all refs from the remote repository, treating 404 as an empty ref list.
    ///
    /// Only for push: a 404 before the first push means the repo doesn't exist yet
    /// and will be auto-created. Clone/fetch/pull must keep erroring on 404.
    pub async fn get_refs_or_empty(&self) -> Result<RefsResponse> {
        let url = format!("{}/info/refs", self.base_url);
        tracing::debug!("GET {}", url);

        let response = self
            .client
            .get(&url)
            .send()
            .await
            .context("Failed to send GET /info/refs")?;

        if response.status().as_u16() == 404 {
            return Ok(RefsResponse {
                refs: Vec::new(),
                capabilities: Vec::new(),
            });
        }
        if !response.status().is_success() {
            anyhow::bail!("GET /info/refs failed with status: {}", response.status());
        }

        response
            .json::<RefsResponse>()
            .await
            .context("Failed to parse refs response")
    }

    /// Update remote refs
    pub async fn update_refs(&self, request: RefUpdateRequest) -> Result<RefUpdateResponse> {
        let url = format!("{}/refs/update", self.base_url);
        tracing::debug!("POST {}", url);

        let response = self
            .client
            .post(&url)
            .json(&request)
            .send()
            .await
            .context("Failed to update refs")?;

        if !response.status().is_success() {
            anyhow::bail!(
                "POST /refs/update failed with status: {}",
                response.status()
            );
        }

        response
            .json::<RefUpdateResponse>()
            .await
            .context("Failed to parse ref update response")
    }

    /// Upload a single loose object (any type) to the server.
    ///
    /// Wraps the raw bytes in a minimal pack and POSTs it to `/objects/pack`.
    /// Use this for blobs that are not reachable from any commit graph (e.g.
    /// annotated-tag `.meta` sidecars stored as bare blobs).
    pub async fn upload_loose_object(
        &self,
        oid: Oid,
        obj_type: ObjectType,
        data: &[u8],
    ) -> Result<()> {
        let mut pack_writer = PackWriter::new();
        pack_writer.add_object(oid, obj_type, data);
        let pack_data = pack_writer.finalize();
        self.upload_pack(&pack_data).await
    }
}

/// Coalesce adjacent/near chunk ranges within a sorted (offset-ascending) chunk list.
///
/// Returns a list of (start, end) byte ranges where `end` is exclusive.
/// Only merges if `gap <= max_gap` AND `merged_size <= max_bytes`.
pub(crate) fn coalesce_chunk_ranges(
    chunks: &[(String, u64, u32)],
    max_gap: u64,
    max_bytes: u64,
) -> Vec<(u64, u64)> {
    let mut ranges: Vec<(u64, u64)> = Vec::new();
    if chunks.is_empty() {
        return ranges;
    }
    let mut cur_start = chunks[0].1;
    let mut cur_end = cur_start + chunks[0].2 as u64;

    for (_, off, len) in &chunks[1..] {
        let chunk_end = off + *len as u64;
        let gap = off.saturating_sub(cur_end);
        let merged = chunk_end - cur_start;
        if gap <= max_gap && merged <= max_bytes {
            cur_end = cur_end.max(chunk_end);
        } else {
            ranges.push((cur_start, cur_end));
            cur_start = *off;
            cur_end = chunk_end;
        }
    }
    ranges.push((cur_start, cur_end));
    ranges
}

/// Download a single chunk directly from a presigned GET URL, verify its hash.
///
/// Returns `Ok(bytes)` on success.
/// Returns `Err` on any failure (network, non-2xx, hash mismatch) — caller falls
/// back to the server-proxied `GET /chunks/:id` path.
pub(crate) async fn download_chunk_direct(
    client: &reqwest::Client,
    url: &str,
    headers: &[(String, String)],
    expected_hex: &str,
    size_hint: u64,
) -> anyhow::Result<Vec<u8>> {
    // Range-parallel GET: for large chunks, fan out N parallel byte-range
    // requests to fill multiple TCP congestion windows simultaneously.
    // All S3/Azure/GCS/MinIO backends honour `Range:` on presigned URLs.
    // Disabled when MEDIAGIT_RANGE_PARALLEL=0 or size_hint is too small.
    let range_parallel: usize = std::env::var("MEDIAGIT_RANGE_PARALLEL")
        .ok()
        .and_then(|s| {
            s.parse().ok().or_else(|| {
                tracing::warn!(
                    "MEDIAGIT_RANGE_PARALLEL='{}' is not a valid usize, using default 4",
                    s
                );
                None
            })
        })
        .unwrap_or(4)
        .clamp(0, 16);
    let range_threshold: u64 = std::env::var("MEDIAGIT_RANGE_PARALLEL_THRESHOLD")
        .ok()
        .and_then(|s| {
            s.parse().ok().or_else(|| {
                tracing::warn!("MEDIAGIT_RANGE_PARALLEL_THRESHOLD='{}' is not a valid u64, using default 4 MiB", s);
                None
            })
        })
        .unwrap_or(4 * 1024 * 1024); // 4 MiB — fires on typical media chunks (512 KB – 32 MB)

    if range_parallel > 1 && size_hint >= range_threshold {
        match download_chunk_ranged(
            client,
            url,
            headers,
            expected_hex,
            size_hint,
            range_parallel,
        )
        .await
        {
            Ok(data) => return Ok(data),
            Err(e) => {
                // Range-GET failed (e.g. server doesn't support it) → fall
                // through to single-stream path below.
                tracing::debug!(
                    err = %e,
                    "range-parallel GET failed, falling back to single-stream"
                );
            }
        }
    }

    let mut req = client.get(url);
    for (k, v) in headers {
        req = req.header(k.as_str(), v.as_str());
    }
    let resp = req.send().await?;
    let status = resp.status().as_u16();
    if !resp.status().is_success() {
        let ct = resp
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_string();
        let hc = resp
            .headers()
            .get("x-amz-error-code")
            .or_else(|| resp.headers().get("x-ms-error-code"))
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_string();
        let body = resp.text().await.unwrap_or_default();
        let outcome = crate::error_class::classify_auto_get(status, url, &ct, &hc, &body);
        if let crate::error_class::TransferOutcome::PermanentChunkAfterDelay(delay) = outcome {
            tokio::time::sleep(delay).await;
            anyhow::bail!("presigned GET 404 (after {delay:?} delay): status={status}");
        }
        anyhow::bail!("presigned GET failed: status={status} outcome={outcome:?}");
    }

    // Stream body into buffer.
    // Note: chunk ID = BLAKE3(uncompressed data), but storage holds compressed bytes.
    // We cannot verify the hash here without decompressing; TLS + ETag guarantee
    // transport integrity. Application-layer integrity is verified on read (decompress).
    use futures::StreamExt;
    let mut stream = resp.bytes_stream();
    let mut buf: Vec<u8> = Vec::new();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk?;
        buf.extend_from_slice(&chunk);
    }
    Ok(buf)
}

/// Fan out N parallel `Range:` GETs and reassemble in order.
/// Reassembles bytes from range GETs; no inline hash check here.
/// Verification is deferred to read-time decompression — chunk_id is
/// BLAKE3(uncompressed) but stored bytes are compressed.
pub(crate) async fn download_chunk_ranged(
    client: &reqwest::Client,
    url: &str,
    headers: &[(String, String)],
    _expected_hex: &str,
    _size_hint: u64,
    n_ranges: usize,
) -> anyhow::Result<Vec<u8>> {
    use futures::future::try_join_all;

    // Probe request: `Range: bytes=0-0` to discover actual compressed Content-Length.
    // Unlike HEAD, presigned GET URLs always accept Range requests on AWS/GCS/MinIO/Azure.
    // The 206 response contains `Content-Range: bytes 0-0/<total>` giving the true
    // compressed object size. We need this because _size_hint is the *uncompressed* chunk
    // size — using it for Range striping truncates the last byte (the SmartCompressor tag).
    let actual_size = {
        let mut req = client.get(url).header("Range", "bytes=0-0");
        for (k, v) in headers {
            req = req.header(k.as_str(), v.as_str());
        }
        let resp = req.send().await?;
        if resp.status().as_u16() == 206 {
            resp.headers()
                .get("content-range")
                .and_then(|v| v.to_str().ok())
                .and_then(|s| s.strip_prefix("bytes "))
                .and_then(|s| s.split('/').nth(1))
                .and_then(|n| n.parse::<u64>().ok())
                .ok_or_else(|| {
                    anyhow::anyhow!(
                        "206 missing valid Content-Range; falling back to single-stream"
                    )
                })?
        } else {
            anyhow::bail!(
                "range probe returned {}; falling back to single-stream",
                resp.status()
            );
        }
    };

    let stripe = actual_size / n_ranges as u64;
    let futs: Vec<_> = (0..n_ranges)
        .map(|i| {
            let start = i as u64 * stripe;
            let end = if i + 1 == n_ranges {
                actual_size - 1
            } else {
                (i as u64 + 1) * stripe - 1
            };
            let mut req = client.get(url);
            for (k, v) in headers {
                req = req.header(k.as_str(), v.as_str());
            }
            req = req.header("Range", format!("bytes={start}-{end}"));
            async move {
                let resp = req.send().await?;
                if resp.status().as_u16() == 200 {
                    // Server returned full body instead of 206 — not range-capable.
                    anyhow::bail!("server returned 200 instead of 206 for Range request");
                }
                anyhow::ensure!(
                    resp.status().as_u16() == 206,
                    "range GET returned unexpected status {}",
                    resp.status()
                );
                let data = resp.bytes().await?.to_vec();
                anyhow::Ok((i, data))
            }
        })
        .collect();

    let mut parts: Vec<(usize, Vec<u8>)> = try_join_all(futs).await?;
    parts.sort_unstable_by_key(|(i, _)| *i);

    let mut buf = Vec::with_capacity(actual_size as usize);
    for (_, data) in &parts {
        buf.extend_from_slice(data);
    }
    Ok(buf)
}

/// Upload a single chunk via server-orchestrated multipart upload (S3/MinIO MPU).
///
/// Returns `true` when the chunk was successfully uploaded and committed.
/// Returns `false` on any failure; the caller falls through to single-PUT or proxy.
///
/// Flow: POST `/chunks/mpu/start` → PUT each part URL → POST `/chunks/mpu/complete`.
/// On part-upload failure the in-flight MPU is aborted best-effort to release S3
/// storage, and `false` is returned so the caller retries via single-PUT/proxy.
pub(crate) async fn upload_chunk_mpu(
    api_client: &reqwest::Client,
    direct_client: &reqwest::Client,
    base_url: &str,
    chunk_hex: &str,
    chunk_data: &[u8],
) -> bool {
    #[derive(serde::Serialize)]
    struct StartReq<'a> {
        chunk_id: &'a str,
        chunk_size: u64,
    }
    #[derive(serde::Deserialize)]
    struct StartResp {
        upload_id: String,
        parts: Vec<PartUrl>,
        part_size: u64,
    }
    #[derive(serde::Deserialize)]
    struct PartUrl {
        part_number: i32,
        url: String,
    }
    #[derive(serde::Serialize)]
    struct CompleteReq<'a> {
        chunk_id: &'a str,
        upload_id: &'a str,
        parts: Vec<CompletedPart>,
    }
    #[derive(serde::Serialize)]
    struct CompletedPart {
        part_number: i32,
        etag: String,
    }
    #[derive(serde::Serialize)]
    struct AbortReq<'a> {
        chunk_id: &'a str,
        upload_id: &'a str,
    }

    // --- Start MPU ---
    let start_url = format!("{}/chunks/mpu/start", base_url);
    let resp = match api_client
        .post(&start_url)
        .json(&StartReq {
            chunk_id: chunk_hex,
            chunk_size: chunk_data.len() as u64,
        })
        .send()
        .await
    {
        Ok(r) => r,
        Err(e) => {
            tracing::debug!(chunk = %chunk_hex, err = %e, "MPU start request failed; using single-PUT");
            return false;
        }
    };

    if resp.status() == reqwest::StatusCode::NOT_IMPLEMENTED {
        tracing::debug!(chunk = %chunk_hex, "Backend does not support MPU; using single-PUT");
        return false;
    }
    if !resp.status().is_success() {
        tracing::debug!(chunk = %chunk_hex, status = resp.status().as_u16(), "MPU start non-2xx; using single-PUT");
        return false;
    }

    let mpu: StartResp = match resp.json().await {
        Ok(m) => m,
        Err(e) => {
            tracing::debug!(chunk = %chunk_hex, err = %e, "MPU start parse error; using single-PUT");
            return false;
        }
    };

    let upload_id = mpu.upload_id.clone();
    let part_size = mpu.part_size as usize;
    let mut completed_parts: Vec<CompletedPart> = Vec::with_capacity(mpu.parts.len());

    // --- Upload each part (with per-part retry, same backoff as single-PUT) ---
    const MAX_PART_ATTEMPTS: u32 = 5;
    for part in &mpu.parts {
        let start = (part.part_number as usize - 1) * part_size;
        let end = (start + part_size).min(chunk_data.len());
        if start >= chunk_data.len() {
            break;
        }
        let part_data = &chunk_data[start..end];

        let mut part_etag: Option<String> = None;
        for attempt in 0..MAX_PART_ATTEMPTS {
            if attempt > 0 {
                let base_ms = (1000u64 << (attempt - 1)).min(30_000);
                let seed = (chunk_hex.as_bytes().first().copied().unwrap_or(0) as u64)
                    .wrapping_mul(13)
                    .wrapping_add(attempt as u64 * 7)
                    .wrapping_add(part.part_number as u64 * 31);
                let jitter_ms = base_ms * (seed % 25) / 100;
                tokio::time::sleep(tokio::time::Duration::from_millis(base_ms + jitter_ms)).await;
            }

            let result = direct_client
                .put(&part.url)
                .header(reqwest::header::CONTENT_LENGTH, part_data.len())
                .body(part_data.to_vec())
                .send()
                .await;

            match result {
                Ok(r) if r.status().is_success() => {
                    let etag = r
                        .headers()
                        .get("etag")
                        .and_then(|v| v.to_str().ok())
                        .unwrap_or("")
                        .to_string();
                    part_etag = Some(etag);
                    break;
                }
                Ok(r) => {
                    let status = r.status().as_u16();
                    let ct = r
                        .headers()
                        .get(reqwest::header::CONTENT_TYPE)
                        .and_then(|v| v.to_str().ok())
                        .unwrap_or("")
                        .to_string();
                    let hdr_code = r
                        .headers()
                        .get("x-amz-error-code")
                        .and_then(|v| v.to_str().ok())
                        .unwrap_or("")
                        .to_string();
                    let body_full = r.text().await.unwrap_or_default();
                    let body_ref = if body_full.len() > 2048 {
                        &body_full[..2048]
                    } else {
                        &body_full[..]
                    };

                    use crate::error_class::{TransferOutcome, classify_auto};
                    match classify_auto(status, &part.url, &ct, &hdr_code, body_ref) {
                        TransferOutcome::Transient | TransferOutcome::RefreshUrl => {
                            // RefreshUrl treated as Transient: per-part URLs are issued per-MPU
                            // and cannot be refreshed individually without restarting the upload.
                            // The part URL TTL (12 h) makes a persistent auth error across all
                            // 5 attempts unlikely.
                            tracing::debug!(
                                chunk = %chunk_hex,
                                part = part.part_number,
                                attempt = attempt + 1,
                                status,
                                "MPU part transient error; retrying"
                            );
                        }
                        TransferOutcome::PermanentChunk
                        | TransferOutcome::PermanentChunkAfterDelay(_)
                        | TransferOutcome::PermanentConfig => {
                            tracing::debug!(
                                chunk = %chunk_hex,
                                part = part.part_number,
                                status,
                                "MPU part permanent error; aborting MPU"
                            );
                            let _ = api_client
                                .post(format!("{}/chunks/mpu/abort", base_url))
                                .json(&AbortReq {
                                    chunk_id: chunk_hex,
                                    upload_id: &upload_id,
                                })
                                .send()
                                .await;
                            return false;
                        }
                    }
                }
                Err(e) => {
                    tracing::debug!(
                        chunk = %chunk_hex,
                        part = part.part_number,
                        attempt = attempt + 1,
                        err = %e,
                        "MPU part network error; retrying"
                    );
                }
            }
        }

        match part_etag {
            Some(etag) => completed_parts.push(CompletedPart {
                part_number: part.part_number,
                etag,
            }),
            None => {
                tracing::debug!(
                    chunk = %chunk_hex,
                    part = part.part_number,
                    "MPU part retry budget exhausted; aborting MPU"
                );
                let _ = api_client
                    .post(format!("{}/chunks/mpu/abort", base_url))
                    .json(&AbortReq {
                        chunk_id: chunk_hex,
                        upload_id: &upload_id,
                    })
                    .send()
                    .await;
                return false;
            }
        }
    }

    // --- Complete MPU ---
    let complete_url = format!("{}/chunks/mpu/complete", base_url);
    let complete_resp = match api_client
        .post(&complete_url)
        .json(&CompleteReq {
            chunk_id: chunk_hex,
            upload_id: &upload_id,
            parts: completed_parts,
        })
        .send()
        .await
    {
        Ok(r) => r,
        Err(e) => {
            tracing::debug!(chunk = %chunk_hex, err = %e, "MPU complete request failed; falling back");
            return false;
        }
    };

    if !complete_resp.status().is_success() {
        tracing::debug!(
            chunk = %chunk_hex,
            status = complete_resp.status().as_u16(),
            "MPU complete non-2xx; falling back"
        );
        return false;
    }

    tracing::debug!(chunk = %chunk_hex, parts = mpu.parts.len(), "MPU upload succeeded");
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_client_creation() {
        let client = ProtocolClient::new("http://localhost:3000/test-repo");
        assert_eq!(client.base_url, "http://localhost:3000/test-repo");
    }

    #[test]
    fn new_client_has_no_credentials() {
        let client = ProtocolClient::new("http://localhost:3000/test-repo");
        assert_eq!(client.credentials(), &Credentials::None);
    }

    #[test]
    fn with_credentials_updates_stored_credentials() {
        let client = ProtocolClient::new("http://localhost:3000/test-repo")
            .with_credentials(Credentials::Bearer("tok".to_string()));
        assert_eq!(
            client.credentials(),
            &Credentials::Bearer("tok".to_string())
        );
    }

    // ------------------------------------------------------------------
    // Header-attachment tests: a minimal raw-HTTP TCP responder captures
    // whatever headers arrive on the wire, so these prove the header is
    // actually sent (or actually absent), not just stored in a struct
    // field. `build_control_plane_client` bakes the header into
    // `self.client`'s default headers, so it rides every request that
    // client makes uniformly (get_refs, update_refs, download_chunk, ...)
    // — these tests exercise a representative sample of control-plane
    // call sites, not an exhaustive list.
    // ------------------------------------------------------------------

    /// Start a raw TCP responder that captures the request headers of the
    /// first connection into `captured` (lowercased names) and replies with
    /// a fixed 200 JSON body sufficient for `RefsResponse`.
    async fn start_header_capturing_server() -> (
        String,
        std::sync::Arc<tokio::sync::Mutex<Option<Vec<(String, String)>>>>,
    ) {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        use tokio::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let captured = std::sync::Arc::new(tokio::sync::Mutex::new(None));
        let captured_clone = captured.clone();

        tokio::spawn(async move {
            if let Ok((mut socket, _)) = listener.accept().await {
                let mut buf = Vec::new();
                let mut chunk = [0u8; 4096];
                // Read until we see the blank-line header terminator.
                loop {
                    let n = socket.read(&mut chunk).await.unwrap_or(0);
                    if n == 0 {
                        break;
                    }
                    buf.extend_from_slice(&chunk[..n]);
                    if buf.windows(4).any(|w| w == b"\r\n\r\n") {
                        break;
                    }
                }
                let text = String::from_utf8_lossy(&buf);
                let headers: Vec<(String, String)> = text
                    .lines()
                    .skip(1) // request line
                    .take_while(|l| !l.is_empty())
                    .filter_map(|l| {
                        let (k, v) = l.split_once(':')?;
                        Some((k.trim().to_lowercase(), v.trim().to_string()))
                    })
                    .collect();
                *captured_clone.lock().await = Some(headers);

                let body = r#"{"refs":[],"capabilities":[]}"#;
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                let _ = socket.write_all(response.as_bytes()).await;
                let _ = socket.shutdown().await;
            }
        });

        (format!("http://{addr}/test-repo"), captured)
    }

    #[tokio::test]
    async fn bearer_credentials_attach_authorization_header() {
        let (url, captured) = start_header_capturing_server().await;
        let client =
            ProtocolClient::new(url).with_credentials(Credentials::Bearer("s3cr3t".to_string()));
        let _ = client.get_refs().await;
        let headers = captured.lock().await.clone().expect("request was received");
        assert_eq!(
            headers
                .iter()
                .find(|(k, _)| k == "authorization")
                .map(|(_, v)| v.as_str()),
            Some("Bearer s3cr3t")
        );
        assert!(!headers.iter().any(|(k, _)| k == "x-api-key"));
    }

    #[tokio::test]
    async fn api_key_credentials_attach_x_api_key_header() {
        let (url, captured) = start_header_capturing_server().await;
        let client =
            ProtocolClient::new(url).with_credentials(Credentials::ApiKey("mykey".to_string()));
        let _ = client.get_refs().await;
        let headers = captured.lock().await.clone().expect("request was received");
        assert_eq!(
            headers
                .iter()
                .find(|(k, _)| k == "x-api-key")
                .map(|(_, v)| v.as_str()),
            Some("mykey")
        );
        assert!(!headers.iter().any(|(k, _)| k == "authorization"));
    }

    /// Authless regression guard: no credentials configured → no
    /// Authorization/X-Api-Key header at all, exactly as before this
    /// feature existed. An old/authless server sees nothing new.
    #[tokio::test]
    async fn no_credentials_attaches_no_auth_header() {
        let (url, captured) = start_header_capturing_server().await;
        let client = ProtocolClient::new(url); // Credentials::None by default
        let _ = client.get_refs().await;
        let headers = captured.lock().await.clone().expect("request was received");
        assert!(!headers.iter().any(|(k, _)| k == "authorization"));
        assert!(!headers.iter().any(|(k, _)| k == "x-api-key"));
    }

    #[tokio::test]
    async fn bearer_credentials_attach_header_on_update_refs_too() {
        // Second representative control-plane call site (POST, not GET),
        // confirming the header rides via default_headers regardless of
        // which ProtocolClient method is used.
        let (url, captured) = start_header_capturing_server().await;
        let client =
            ProtocolClient::new(url).with_credentials(Credentials::Bearer("tok2".to_string()));
        let _ = client
            .update_refs(crate::types::RefUpdateRequest {
                updates: vec![],
                force: false,
            })
            .await;
        let headers = captured.lock().await.clone().expect("request was received");
        assert_eq!(
            headers
                .iter()
                .find(|(k, _)| k == "authorization")
                .map(|(_, v)| v.as_str()),
            Some("Bearer tok2")
        );
    }

    // Additional integration tests would require a running server
    // These should be in tests/integration/

    /// Build a synthetic response without a server.
    fn resp(status: u16, retry_after: Option<&str>) -> reqwest::Response {
        let mut b = http::Response::builder().status(status);
        if let Some(ra) = retry_after {
            b = b.header(reqwest::header::RETRY_AFTER, ra);
        }
        reqwest::Response::from(b.body("").unwrap())
    }

    #[tokio::test]
    async fn rate_limit_retry_waits_out_429_then_succeeds() {
        let calls = std::sync::Arc::new(AtomicUsize::new(0));
        let c = calls.clone();
        let out = send_with_rate_limit_retry(|| {
            let c = c.clone();
            async move {
                let n = c.fetch_add(1, Ordering::SeqCst);
                // "Retry-After: 0" keeps the test fast while still exercising
                // the header path rather than the exponential fallback.
                Ok(if n < 2 {
                    resp(429, Some("0"))
                } else {
                    resp(200, None)
                })
            }
        })
        .await
        .unwrap();

        assert_eq!(out.status(), 200);
        assert_eq!(
            calls.load(Ordering::SeqCst),
            3,
            "must retry past both 429s rather than surfacing the first one"
        );
    }

    #[tokio::test]
    async fn rate_limit_retry_gives_up_and_returns_the_429() {
        // A permanently rate-limited server must not hang the push forever:
        // the helper returns the 429 so the caller reports a real failure.
        let calls = std::sync::Arc::new(AtomicUsize::new(0));
        let c = calls.clone();
        let out = send_with_rate_limit_retry(|| {
            let c = c.clone();
            async move {
                c.fetch_add(1, Ordering::SeqCst);
                Ok(resp(429, Some("0")))
            }
        })
        .await
        .unwrap();

        assert_eq!(out.status(), 429);
        assert_eq!(
            calls.load(Ordering::SeqCst),
            rate_limit_max_retries() as usize + 1,
            "one initial attempt plus the configured retry budget"
        );
    }

    #[tokio::test]
    async fn rate_limit_retry_does_not_retry_non_429() {
        let calls = std::sync::Arc::new(AtomicUsize::new(0));
        let c = calls.clone();
        let out = send_with_rate_limit_retry(|| {
            let c = c.clone();
            async move {
                c.fetch_add(1, Ordering::SeqCst);
                Ok(resp(500, None))
            }
        })
        .await
        .unwrap();

        assert_eq!(out.status(), 500);
        assert_eq!(
            calls.load(Ordering::SeqCst),
            1,
            "only 429 is a rate-limit signal; other statuses are the caller's to handle"
        );
    }
}
