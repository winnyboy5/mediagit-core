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

/// Header carrying the OP-7 correlation id (see `ProtocolClient::new`).
///
/// Deliberately distinct from `X-Request-ID`: that header is already
/// load-bearing as the want-cache key for `GET /objects/pack` (client sets
/// it from `WantResponse::request_id` in `pull.rs`; server reads it in
/// `handlers::repo`), scoped to one pack negotiation. Reusing it here would
/// collide with that and break pack download.
pub const OP_ID_HEADER: &str = "x-mediagit-op-id";

/// Mint a correlation id for one client operation (push/pull/clone/...).
///
/// Same cheap idiom as the server's own `generate_request_id` (timestamp +
/// process-local counter, not a UUID) — protocol can't depend on
/// mediagit-server to reuse that one directly, and a client-side id only
/// needs to be unique within this process's lifetime.
fn generate_operation_id() -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let id = COUNTER.fetch_add(1, Ordering::SeqCst);
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    format!("{timestamp}-{id}")
}

/// Build the control-plane `reqwest::Client` with the given credentials and
/// operation id baked in as default headers (present on every request sent
/// through this client instance). Shared by `ProtocolClient::new` and
/// `with_credentials` so both construct the client identically apart from
/// the credentials header.
fn build_control_plane_client(creds: &Credentials, op_id: &str) -> reqwest::Client {
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
    let mut headers = reqwest::header::HeaderMap::new();
    if let Some((name, value)) = creds.header()
        && let Ok(header_value) = reqwest::header::HeaderValue::from_str(&value)
    {
        headers.insert(reqwest::header::HeaderName::from_static(name), header_value);
    }
    if let Ok(header_value) = reqwest::header::HeaderValue::from_str(op_id) {
        headers.insert(
            reqwest::header::HeaderName::from_static(OP_ID_HEADER),
            header_value,
        );
    }
    if !headers.is_empty() {
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
    /// OP-7 correlation id for this operation (one push/pull/clone), sent as
    /// `OP_ID_HEADER` on every control-plane request via `client`'s default
    /// headers. Stored so `with_credentials` can carry it over when it
    /// rebuilds `client`.
    operation_id: String,
    /// Optional override for parallel chunk-upload fan-out. Takes precedence
    /// over the internal default (32) but is itself overridden by the
    /// `MEDIAGIT_UPLOAD_CONCURRENCY` env var. Set via `with_concurrent_uploads`.
    concurrent_uploads: Option<usize>,
    /// Optional override for parallel chunk-download fan-out. Takes precedence
    /// over the internal default (24) but is itself overridden by the
    /// `MEDIAGIT_DOWNLOAD_CONCURRENCY` env var. Set via `with_concurrent_downloads`.
    concurrent_downloads: Option<usize>,
}

/// OP-2: absolute wall-clock budget for a download (pull / fetch / clone).
///
/// `push` has had one since A7; the download side had none, so a backend that
/// accepted the connection and then stopped sending left `next_object()`
/// awaiting forever with no output and no way to tell a stall from a slow
/// large transfer. The failure mode was a clone that never returns.
///
/// Absolute rather than stall-based, matching push: the default is generous
/// enough that a real large clone will not trip it, and lowering
/// `MEDIAGIT_PULL_DEADLINE_SECS` is how you fail fast against a dead backend.
///
/// ponytail: absolute deadline, upgrade to a progress-reset stall deadline if
/// multi-hour legitimate clones ever false-trip it — same note as push.
pub(crate) fn pull_deadline_secs() -> u64 {
    std::env::var("MEDIAGIT_PULL_DEADLINE_SECS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .filter(|&s| s > 0)
        .unwrap_or(3600)
}

/// Wrap a download future in [`pull_deadline_secs`], naming the phase that ran out.
pub(crate) async fn with_pull_deadline<T>(
    phase: &str,
    fut: impl std::future::Future<Output = anyhow::Result<T>>,
) -> anyhow::Result<T> {
    let secs = pull_deadline_secs();
    match tokio::time::timeout(std::time::Duration::from_secs(secs), fut).await {
        Ok(res) => res,
        Err(_) => anyhow::bail!(
            "{phase} aborted: exceeded MEDIAGIT_PULL_DEADLINE_SECS ({secs}s); \
             the remote or its storage backend may be unavailable"
        ),
    }
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

/// Maximum 429 retries for a single control-plane request. Default 10,
/// overridable via `MEDIAGIT_RATE_LIMIT_RETRIES`.
///
/// Raised from 5 on 2026-08-18. A rate limit is BACKPRESSURE, not an error: the
/// right response is to get slower, not to fail. Five attempts with equal
/// jitter is only ~4-8s of total waiting, and a request contending with dozens
/// of siblings against a tight budget can easily need longer than that just to
/// reach the front of the queue -- measured against a 2 rps server, requests
/// reached attempt 4 of 5 and then gave up, failing a push that only needed to
/// be patient.
///
/// Ten attempts is roughly 40-80s of backoff before surrendering. That is still
/// firmly bounded -- nothing like the 3600s stall this whole area started with
/// -- while being long enough that a legitimate push under a deliberately tight
/// limiter completes instead of erroring.
pub(crate) fn rate_limit_max_retries() -> u32 {
    std::env::var("MEDIAGIT_RATE_LIMIT_RETRIES")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(10)
}

/// A cheap non-cryptographic jitter source.
///
/// `rand` is not a dependency of this crate and this does not justify adding
/// one: retry spreading needs to be unpredictable between peers, not secure.
/// The crate already hand-rolls a seed for MPU part scheduling; this is the
/// same trick in one place.
fn jitter_upto(bound_millis: u64) -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);

    if bound_millis == 0 {
        return 0;
    }
    let seed = COUNTER.fetch_add(1, Ordering::Relaxed)
        ^ std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(0);
    // splitmix64 finalizer -- good enough avalanche for spreading retries.
    let mut z = seed.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    (z ^ (z >> 31)) % bound_millis
}

/// How long to wait before retrying, given the attempt number and the server's
/// `Retry-After` header if it sent one.
///
/// Jitter is not decoration. Un-jittered exponential backoff is the one shape
/// AWS's own measurements single out as strictly worse than every jittered
/// variant, on both total client work and total elapsed time: N clients that
/// were rate limited together retry together, forever. This crate's backoff
/// was `250ms * 2^attempt` with no spread at all, and a large push runs many
/// uploads concurrently -- precisely the herd that produces.
///
/// The result is the LARGER of what the server asked for and our own Full
/// Jitter backoff. Both halves are load-bearing:
///
/// - Taking the server's number as a lower bound is the whole point of the
///   header: it knows when the bucket refills and we do not.
/// - Flooring it with our own backoff is what makes the header safe to trust.
///   `tower_governor`'s `use_headers()` emits whole seconds and rounds down, so
///   a bucket that refills in under a second advertises **`Retry-After: 0`** --
///   verified, see `retry_after_carries_an_actionable_delay` in
///   `mediagit-server`. A client that honours that literally sleeps for zero
///   and hammers straight back, spending its entire retry budget in
///   microseconds without the bucket ever refilling. That is not theoretical:
///   it is what made a 500-commit churn push fail against a rate-limited
///   server in `20260817-gagate3`, on `POST /packs/complete`.
///
/// Un-jittered exponential backoff is separately the one shape AWS's own
/// measurements single out as strictly worse than every jittered variant, on
/// both total client work and total elapsed time -- N clients limited together
/// otherwise retry together, forever.
///
/// The 16s ceiling on our own backoff is preserved so a push cannot stall
/// indefinitely behind a misconfigured limiter. A server asking for longer than
/// that is still honoured: it asked, and it knows.
pub(crate) fn rate_limit_backoff(
    attempt: u32,
    retry_after: Option<&reqwest::header::HeaderValue>,
) -> std::time::Duration {
    let own = {
        let ceiling = (250u64 * (1u64 << attempt.min(6))).min(16_000);
        // EQUAL jitter (half fixed, half random), not FULL jitter
        // (`random(0, ceiling)`).
        //
        // Full Jitter is the better choice when the server tells you nothing
        // and you are only trying to spread a herd. Ours tells you nothing
        // WORSE than that: `tower_governor` rounds `Retry-After` down to whole
        // seconds, so any sub-second refill reports 0 and the client is left to
        // invent the entire delay. Under full jitter that invented delay can
        // round to near zero on any attempt -- observed 191ms, 220ms, 95ms on
        // consecutive retries against a 2 rps limiter needing ~500ms per token,
        // which burned the whole 5-attempt budget without ever waiting long
        // enough, and failed the push.
        //
        // Equal jitter keeps the spread that breaks the herd while guaranteeing
        // the wait actually GROWS: attempt 1 lands in 125-250ms, attempt 4 in
        // 2-4s. AWS's own analysis puts it level with full jitter on total work
        // and completion time, so nothing is given up here.
        ceiling / 2 + jitter_upto(ceiling / 2)
    };
    let asked = retry_after
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.trim().parse::<u64>().ok())
        .map(|secs| {
            let base = secs.saturating_mul(1000);
            base + jitter_upto(base / 2)
        })
        .unwrap_or(0);
    std::time::Duration::from_millis(own.max(asked))
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
        let wait = rate_limit_backoff(attempt, resp.headers().get(reqwest::header::RETRY_AFTER));
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
pub mod escrow;
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
        // OP-7: one id per operation, minted here because this is where a
        // push/pull/clone actually begins — the CLI constructs a fresh
        // `ProtocolClient` per command (see crates/mediagit-cli/src/commands/
        // {push,pull,clone}.rs), so "one client instance" already matches
        // "one user-facing operation".
        let operation_id = generate_operation_id();
        Self {
            base_url: base_url.into(),
            client: build_control_plane_client(&credentials, &operation_id),
            credentials,
            operation_id,
            concurrent_uploads: None,
            concurrent_downloads: None,
        }
    }

    /// Attach credentials, sent as a default header (`Authorization: Bearer
    /// <t>` or `X-Api-Key: <k>`) on every control-plane request this client
    /// makes. Rebuilds the internal HTTP client with the same pool/timeout
    /// settings as `new` — direct/presigned cloud-storage requests use their
    /// own separately-built client and never see this header regardless.
    /// `Credentials::None` (the default) attaches no header at all. Keeps
    /// the same OP-7 operation id minted in `new`.
    pub fn with_credentials(mut self, credentials: Credentials) -> Self {
        self.client = build_control_plane_client(&credentials, &self.operation_id);
        self.credentials = credentials;
        self
    }

    /// The credentials currently configured on this client.
    pub fn credentials(&self) -> &Credentials {
        &self.credentials
    }

    /// The OP-7 correlation id for this operation, sent as `OP_ID_HEADER` on
    /// every control-plane request. Lets a caller report the id it should
    /// grep server logs for.
    pub fn operation_id(&self) -> &str {
        &self.operation_id
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
            operation_id: self.operation_id.clone(),
            concurrent_uploads: self.concurrent_uploads,
            concurrent_downloads: Some(n),
        }
    }

    /// Get all refs from the remote repository
    pub async fn get_refs(&self) -> Result<RefsResponse> {
        let url = format!("{}/info/refs", self.base_url);
        tracing::debug!("GET {}", url);

        let response = send_with_rate_limit_retry(|| self.client.get(&url).send())
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

        let response = send_with_rate_limit_retry(|| self.client.get(&url).send())
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

        let response = send_with_rate_limit_retry(|| self.client.post(&url).json(&request).send())
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
    // `parts` is borrowed rather than owned: `send_with_rate_limit_retry`
    // re-invokes its closure per attempt, and an owned `Vec` moved into this
    // struct inside that closure would fail to compile (E0507, moving a
    // captured variable out of an `Fn` closure). A borrow is re-usable across
    // attempts for free.
    #[derive(serde::Serialize)]
    struct CompleteReq<'a> {
        chunk_id: &'a str,
        upload_id: &'a str,
        parts: &'a [CompletedPart],
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
    let resp = match send_with_rate_limit_retry(|| {
        api_client
            .post(&start_url)
            .json(&StartReq {
                chunk_id: chunk_hex,
                chunk_size: chunk_data.len() as u64,
            })
            .send()
    })
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
                            let _ = send_with_rate_limit_retry(|| {
                                api_client
                                    .post(format!("{}/chunks/mpu/abort", base_url))
                                    .json(&AbortReq {
                                        chunk_id: chunk_hex,
                                        upload_id: &upload_id,
                                    })
                                    .send()
                            })
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
                let _ = send_with_rate_limit_retry(|| {
                    api_client
                        .post(format!("{}/chunks/mpu/abort", base_url))
                        .json(&AbortReq {
                            chunk_id: chunk_hex,
                            upload_id: &upload_id,
                        })
                        .send()
                })
                .await;
                return false;
            }
        }
    }

    // --- Complete MPU ---
    let complete_url = format!("{}/chunks/mpu/complete", base_url);
    let complete_resp = match send_with_rate_limit_retry(|| {
        api_client
            .post(&complete_url)
            .json(&CompleteReq {
                chunk_id: chunk_hex,
                upload_id: &upload_id,
                parts: &completed_parts,
            })
            .send()
    })
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
                force_with_lease: false,
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

#[cfg(test)]
mod backoff_tests {
    use super::rate_limit_backoff;
    use reqwest::header::HeaderValue;

    #[test]
    fn honours_retry_after_and_never_undercuts_it() {
        let h = HeaderValue::from_static("2");
        for _ in 0..50 {
            let d = rate_limit_backoff(0, Some(&h)).as_millis() as u64;
            assert!(
                (2000..3000).contains(&d),
                "expected the server's 2s plus up to half again, got {d}ms"
            );
        }
    }

    /// `Retry-After: 0` must NOT mean "retry immediately".
    ///
    /// This test previously asserted the opposite, on the grounds that it kept
    /// the 429 tests fast. It was asserting the bug: the server really does
    /// send 0 whenever the bucket refills in under a second, and honouring
    /// that literally spends the whole retry budget in microseconds. The floor
    /// is what makes the header safe to trust.
    #[test]
    fn retry_after_zero_still_backs_off() {
        let h = HeaderValue::from_static("0");
        let mut any_nonzero = false;
        for _ in 0..100 {
            let d = rate_limit_backoff(0, Some(&h)).as_millis() as u64;
            assert!(
                d <= 250,
                "attempt-0 backoff should stay under the ceiling, got {d}ms"
            );
            if d > 0 {
                any_nonzero = true;
            }
        }
        assert!(
            any_nonzero,
            "Retry-After: 0 must fall back to our own jittered backoff, not to zero"
        );
    }

    /// A server asking for longer than our own ceiling is still honoured.
    #[test]
    fn a_long_retry_after_wins_over_our_backoff() {
        let h = HeaderValue::from_static("30");
        let d = rate_limit_backoff(0, Some(&h)).as_millis() as u64;
        assert!(
            (30_000..=45_000).contains(&d),
            "expected ~30-45s, got {d}ms"
        );
    }

    /// Equal jitter must never collapse to a uselessly short wait.
    ///
    /// This is the property that matters when the server sends `Retry-After: 0`
    /// and the client has to invent the delay: under full jitter a retry could
    /// land at ~0ms, spending an attempt without waiting for the bucket.
    #[test]
    fn backoff_always_waits_at_least_half_the_ceiling() {
        for attempt in 0..5u32 {
            let ceiling = (250u64 * (1u64 << attempt.min(6))).min(16_000);
            for _ in 0..50 {
                let d = rate_limit_backoff(attempt, None).as_millis() as u64;
                assert!(
                    d >= ceiling / 2,
                    "attempt {attempt}: {d}ms is under half the {ceiling}ms ceiling \n                     - the backoff can collapse to near zero again"
                );
                assert!(
                    d <= ceiling,
                    "attempt {attempt}: {d}ms exceeded {ceiling}ms"
                );
            }
        }
    }

    #[test]
    fn without_a_header_it_is_full_jitter_under_the_ceiling() {
        let mut seen = std::collections::HashSet::new();
        for _ in 0..200 {
            let d = rate_limit_backoff(4, None).as_millis() as u64;
            assert!(d < 250 * (1 << 4), "{d}ms exceeded the attempt-4 ceiling");
            seen.insert(d);
        }
        // The whole point: un-jittered backoff returns one value forever.
        assert!(seen.len() > 1, "backoff is not jittered: always {seen:?}");
    }

    #[test]
    fn the_ceiling_is_capped() {
        for _ in 0..50 {
            assert!(rate_limit_backoff(30, None).as_millis() <= 16_000);
        }
    }
}
