// SPDX-License-Identifier: BUSL-1.1
// Copyright (C) 2025-2026 Aswin Krishnamoorthy

use anyhow::{Context, Result};
use mediagit_versioning::{
    Commit, FileMode, ObjectDatabase, ObjectType, Oid, PackWriter, StreamingPackWriter, Tag, Tree,
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
        // Bounds the CONNECT only -- see connect_timeout_secs. A slow but
        // progressing transfer is governed by read_timeout, not this.
        .connect_timeout(std::time::Duration::from_secs(
            connect_timeout_secs().max(1),
        ))
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
    //
    // A READ timeout is a different thing and is safe here. `.timeout()` caps
    // the TOTAL request; `.read_timeout()` caps the gap BETWEEN bytes. A slow
    // but progressing upload resets it on every byte and never trips it, so the
    // Azure regression above stays fixed. What it does catch is the case the
    // keepalive rationale misses: keepalive proves a peer is ALIVE, not that it
    // is ANSWERING. A peer that holds the connection open and goes silent keeps
    // keepalive satisfied while the client waits forever.
    //
    // Not hypothetical. Captured live 2026-08-21 (20260821-rl6hunt12): a clone
    // that normally takes 0.55s sat for 102s with CPU flat across a 10s sample
    // (0.11s -> 0.14s), all six threads in Wait, holding one Established
    // connection, having completed encryption-key + info/refs and issued nothing
    // since. Same shape in 20260820-ga4 (416s) and 20260820-ga8 (1800s). No
    // deadline covered it: the push deadline wraps only upload_pack /
    // upload_chunked_objects / update_refs, and everything before those ran
    // unbounded.
    //
    // 300s default is far above any legitimate inter-byte gap (the server
    // streams block-by-block, so bytes keep arriving during a long upload) and
    // far below the harness timeouts that were previously the only thing
    // stopping these hangs. MEDIAGIT_CONTROL_READ_TIMEOUT_SECS tunes it; 0
    // disables it and restores the old unbounded behaviour.
    //
    // NOTE: this bounds the STALL, it does not explain it. The underlying cause
    // of the block is still unknown — see
    // [[project-client-prebulk-hang-2026-08-21]]. This turns an indefinite hang
    // into a bounded, reportable error so the next occurrence is diagnosable
    // instead of silent.
    let read_timeout_secs = std::env::var("MEDIAGIT_CONTROL_READ_TIMEOUT_SECS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(300);
    if read_timeout_secs > 0 {
        builder = builder.read_timeout(std::time::Duration::from_secs(read_timeout_secs));
    }
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

/// TCP+TLS connect timeout for both protocol clients.
/// `MEDIAGIT_CONNECT_TIMEOUT_SECS`; `0` restores the OS default.
///
/// WHY. Neither client set one, so a connect to a black-holed peer fell back to
/// the OS default -- on Windows roughly 21s of SYN retries, and unbounded in the
/// worst case. That is not a hypothetical cost here: the MinIO SDK path already
/// carries the measured version of this bug in `minio.rs`, where "a stalled TCP
/// can cost ~30 s/attempt x default-3 retries = ~90 s/op, which on a
/// multi-endpoint push compounded to the 10-15 min outages we saw". It was fixed
/// there and never applied to the reqwest clients that carry the control plane
/// and every presigned transfer.
///
/// This is what makes a retry BUDGET work. A wall-clock budget spends itself on
/// however many attempts fit inside it, so an unbounded connect converts ~20 fast
/// attempts into ~5 slow ones -- the budget looks generous and buys almost
/// nothing. Bounding the connect is the difference between retrying and waiting.
///
/// 10s, not MinIO's 5s: this path includes cross-region TLS handshakes, and
/// `minio.rs` already widened its own window for exactly that reason. Only the
/// CONNECT is bounded -- a slow but progressing transfer is governed by
/// `read_timeout`, so large uploads over a thin link are unaffected.
fn connect_timeout_secs() -> u64 {
    std::env::var("MEDIAGIT_CONNECT_TIMEOUT_SECS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(10)
}

/// Build the data-plane `reqwest::ClientBuilder` shared by every presigned
/// transfer: pack PUTs (`packs.rs`), per-chunk PUTs (`push.rs`, two passes) and
/// presigned GETs (`pull.rs`).
///
/// These four sites each rolled their own builder and had drifted apart:
/// `packs.rs` was missing `tcp_keepalive`, and the download client in `pull.rs`
/// carried no request bound whatsoever, leaving a stalled GET to sit until the
/// ABSOLUTE `MEDIAGIT_PULL_DEADLINE_SECS` (3600s) killed the entire phase.
///
/// Returns a BUILDER, not a Client, deliberately. The one setting the four sites
/// genuinely disagree on is `.timeout()`, and they are right to: a total request
/// ceiling is fine for an upload of a bounded pack/chunk, and wrong for a
/// download of an arbitrarily large object. Folding a shared `.timeout(300)` in
/// here would silently cap every download at five minutes. Each call site keeps
/// that decision; everything else is settled here, once.
///
/// HTTP/1.1 on purpose (all four sites already did this): parallel TCP sockets
/// beat h2 multiplexing for large bodies, since parallel congestion windows
/// beat one. No credentials are ever set — a presigned URL carries its own
/// authorization in the query string, and `self.client`'s `x-api-key` default
/// header must never reach a bucket.
pub fn data_plane_client_builder() -> reqwest::ClientBuilder {
    crate::ensure_crypto_provider();
    let mut builder = reqwest::Client::builder()
        .pool_idle_timeout(std::time::Duration::from_secs(60))
        .pool_max_idle_per_host(http_pool_max())
        // Bounds the CONNECT only -- see connect_timeout_secs. Large presigned
        // transfers are governed by read_timeout below, not this.
        .connect_timeout(std::time::Duration::from_secs(
            connect_timeout_secs().max(1),
        ))
        .tcp_keepalive(std::time::Duration::from_secs(45))
        .tcp_nodelay(true)
        .http1_only();

    // Same distinction as the control plane, one layer over: `.timeout()` caps
    // the TOTAL request, `.read_timeout()` caps the gap BETWEEN bytes. Only the
    // second is safe on a data plane that legitimately moves multi-hundred-MB
    // objects — a slow but progressing transfer resets it on every byte.
    //
    // THAT SENTENCE IS TRUE OF DOWNLOADS AND FALSE OF UPLOADS, and the
    // difference cost a whole class of pushes. See
    // `data_plane_upload_client_builder` — on an upload the client is WRITING,
    // the bucket correctly sends nothing until the body completes, so no read
    // ever arrives to reset the timer and this becomes a hard total deadline on
    // the upload itself. Upload sites must use that constructor, not this one.
    //
    // What it catches is what `tcp_keepalive` cannot: keepalive proves a peer is
    // ALIVE, not that it is ANSWERING. A bucket that accepts the connection and
    // then goes quiet keeps keepalive satisfied indefinitely.
    //
    // 300s is far above any real inter-byte gap on a transfer that is actually
    // moving, and far below the phase deadlines that were previously the only
    // backstop. MEDIAGIT_DATA_READ_TIMEOUT_SECS tunes it; 0 restores the old
    // unbounded behaviour.
    let read_timeout_secs = data_plane_read_timeout_secs();
    if read_timeout_secs > 0 {
        builder = builder.read_timeout(std::time::Duration::from_secs(read_timeout_secs));
    }
    builder
}

/// Inter-byte read timeout for data-plane transfers, in seconds. 0 disables.
pub fn data_plane_read_timeout_secs() -> u64 {
    std::env::var("MEDIAGIT_DATA_READ_TIMEOUT_SECS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(300)
}

/// Slowest upload rate still considered ALIVE, in bytes per second.
///
/// Only used to turn a body size into a deadline, so it wants to sit below the
/// worst link the product is expected to work on, not near the typical one.
/// 20260427-E0 measured a real Azure push at 0.24-0.37 MB/s AGGREGATE; split
/// across the default 8 concurrent packs that is ~30 KiB/s per pack. 16 KiB/s
/// keeps ~2x margin under that, and still bounds a genuinely dead socket.
///
/// THE COST, stated plainly: at a 64 MiB pack cap this yields a 4096s bound, so
/// a genuinely silent peer on the UPLOAD path is now caught in ~68 minutes
/// instead of 5. That is a real weakening of what the flat read timeout was
/// added for, and it is the right trade only because the two failures are not
/// comparable. Killing a healthy pack is not a delayed error, it is a WRONG one:
/// it demotes the whole push to the per-chunk path and cost a measured 2.5x
/// (918.66s -> 368.27s on the same 2 GB). A dead socket, meanwhile, is still
/// bounded — by this ceiling, by the per-pack retry budget above it, and by the
/// absolute phase deadline above that.
///
/// The structural way to get both back is a smaller pack: at a 16 MiB cap this
/// bound falls to 1024s while the natural upload time falls to ~76s, which is
/// what AWS already enjoys by splitting packs into ~8 MiB MPU parts. Left alone
/// here deliberately — pack size changes what lands in the bucket, and this
/// change should not.
const MIN_UPLOAD_BYTES_PER_SEC: u64 = 16 * 1024;

/// Data-plane client for UPLOADING a body of known, bounded size.
///
/// WHY THIS EXISTS. `read_timeout` bounds the gap between bytes RECEIVED. On a
/// download that is a stall detector; on an upload there is nothing to receive
/// until the body finishes, so the timer never resets and a flat value silently
/// becomes a total deadline on the transfer.
///
/// Measured 2026-09-04, azure, 2 GB as 32 packs of 64 MiB at concurrency 8:
///
///   read_timeout=300   6/32 packs landed, push 918.66s
///   read_timeout=1800  32/32 packs landed, push 368.27s
///
/// In the failing run four packs died at EXACTLY 300.0s, within one second of
/// each other — the signature of a deadline measured from request start, not of
/// four connections independently going quiet. In the passing run the first wave
/// registered at 106..269s, i.e. the old bound left a 10% margin on work that
/// takes `pack_size × concurrency / bandwidth`. That is why ga38/ga39/ga40 read
/// 32, 16 and 0 packs landed and looked like flakiness: a constant bound against
/// a variable cost is a cliff, and ordinary link variation walks either side of
/// it.
///
/// So an upload gets a TOTAL ceiling derived from the bytes it must move —
/// `push.rs` already reasons this way for chunks ("a bounded chunk body, so a
/// TOTAL request ceiling is safe here"). The rule becomes "this upload must
/// sustain at least MIN_UPLOAD_BYTES_PER_SEC", which a dead socket still fails
/// and a slow-but-moving one does not.
///
/// `MEDIAGIT_DATA_READ_TIMEOUT_SECS=0` disables the bound here too, so the one
/// documented escape hatch keeps working.
pub fn data_plane_upload_client_builder(max_body_bytes: u64) -> reqwest::ClientBuilder {
    let builder = data_plane_client_builder();
    let base = data_plane_read_timeout_secs();
    if base == 0 {
        return builder;
    }
    // Never TIGHTER than the flat bound: this only ever buys a large body more
    // room, so a small-body upload keeps exactly the behaviour it has today.
    let need = max_body_bytes / MIN_UPLOAD_BYTES_PER_SEC;
    let bound = std::time::Duration::from_secs(base.max(need));
    // BOTH, and the read one is the whole point. `data_plane_client_builder`
    // has already installed read_timeout(300); leaving it would fire at 300s no
    // matter what total ceiling is set beside it, which is the exact bug this
    // constructor exists to fix. Raising it to `bound` stops it pre-empting the
    // real deadline, and `.timeout(bound)` is then the honest bound: a total
    // ceiling on a body whose size we know.
    builder.read_timeout(bound).timeout(bound)
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
pub(crate) fn jitter_upto(bound_millis: u64) -> u64 {
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
/// that is still honoured -- it asked, and it knows -- but only up to
/// [`rate_limit_max_wait_ms`], because "it knows" stops being true exactly when
/// the limiter is the thing that is broken.
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
    std::time::Duration::from_millis(own.max(asked).min(rate_limit_max_wait_ms()))
}

/// Hard ceiling on a SINGLE rate-limit wait, however long the server asks for.
///
/// Without this the honoured `Retry-After` was unbounded: with
/// `rate_limit_max_retries()` defaulting to 10, a `Retry-After: 900` measured
/// at 965,668ms for one attempt and 47,098,138ms (13.08 HOURS) across the
/// budget. The client sat idle that whole time while the server answered
/// /health in 3ms -- indistinguishable, from outside, from a hang.
///
/// 60s is chosen to be far above any legitimate limiter (a 2 rps bucket refills
/// in ~500ms; even the pathological 20260818-p10check case wanted 900s only
/// because the period was miscomputed) while keeping the worst case bounded at
/// retries x 60s. It also sits above the 45s that a `Retry-After: 30` can reach
/// through jitter, so a server's reasonable ask is still obeyed exactly.
pub(crate) fn rate_limit_max_wait_ms() -> u64 {
    std::env::var("MEDIAGIT_RATE_LIMIT_MAX_WAIT_SECS")
        .ok()
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(60)
        .saturating_mul(1000)
}

/// Seconds a SHORT control-plane request gets to produce response headers
/// before its connection is abandoned and the request retried on a fresh one.
/// 0 disables the bound entirely, restoring the previous behaviour.
fn short_request_deadline_secs() -> u64 {
    std::env::var("MEDIAGIT_SHORT_REQUEST_DEADLINE_SECS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(30)
}

/// How many bounded, fresh-connection attempts before the unbounded fallback.
///
/// WHY 6 AND NOT 2. Two was the original value, and campaign 20260909-ga49
/// measured it as too few — the first hard evidence of this bound being wrong
/// rather than merely untested. `07_auth/A11-auth-grants` failed with a push
/// that ran 720.1s and then reported
/// `Failed to send GET /info/refs: … operation timed out`. Reconstructed from
/// both sides:
///
///   client  two 30s deadlines missed on `/encryption-key`, then two more on
///           `/info/refs`, each logging rt_workers=1, rt_alive_tasks=2,
///           rt_global_queue_depth=0 — timers firing exactly on schedule, so
///           the runtime was healthy, not starved.
///   server  `accepted` climbing 4→9 while `routed` stayed frozen at 13 and
///           `idle_s` ran to 699. Connections were accepted and handed to
///           axum; not one produced a request the router ever saw.
///
/// The arithmetic closes it: 30 + 30 + 300 (the `read_timeout` finally killing
/// the unbounded attempt) = 360s per request, twice = 720s. Exactly the
/// measured push.
///
/// So the stall outlived BOTH bounded attempts, and the unbounded fallback then
/// inherited a third stalled connection and rode it all the way to the read
/// timeout. The fallback is a safety net for a genuinely slow server; it is the
/// worst possible thing to spend on a stalled one. More fresh-connection
/// attempts before reaching it is the fix, and `/encryption-key` recovering on
/// a later attempt in the same push is direct evidence that a fresh connection
/// does eventually get served.
///
/// Worst case grows from 360s to 480s for a request that fails anyway; in
/// exchange a stall gets six chances across three minutes instead of two across
/// one. `MEDIAGIT_SHORT_REQUEST_ATTEMPTS` tunes it; 0 is clamped to 1, since
/// zero bounded attempts would silently restore the unbounded-only behaviour
/// this whole function exists to prevent.
///
/// NOTE THIS IS MITIGATION, NOT A CURE. Why a connection accepted by axum never
/// yields a request to the router is still unknown — and ga49 REFUTES the
/// explanation recorded below and in `send_short_control_request`, which had it
/// completing into the kernel backlog with the acceptor never returning it. Here
/// `accepted` moved. The acceptor was fine.
fn short_request_attempts() -> u32 {
    std::env::var("MEDIAGIT_SHORT_REQUEST_ATTEMPTS")
        .ok()
        .and_then(|v| v.parse::<u32>().ok())
        .unwrap_or(6)
        .max(1)
}

/// Send a short, bodyless control request under a deadline, retrying on a
/// FRESH connection if no response headers arrive in time.
///
/// THE BUG THIS EXISTS FOR. A clone intermittently hangs on its very first
/// request. Captured twice with the server's own counters (2026-08-31): the
/// client's TCP connect completes, `Established` to the server, CPU flat; the
/// server's `accepted` counter never moves while its heartbeat keeps ticking.
/// For THAT signature the reading was that the connection completes into the
/// kernel's listen backlog and the acceptor never returns it.
///
/// THERE IS A SECOND SIGNATURE, AND THE BACKLOG READING DOES NOT COVER IT.
/// 20260909-ga49 hit the same symptom from this call site with `accepted`
/// CLIMBING 4→9 while `routed` stayed frozen at 13 and `idle_s` ran to 699. The
/// acceptor was demonstrably working; the connection was accepted and handed to
/// axum, and no request ever reached the router. Client-side runtime metrics
/// were healthy at the same moment (`rt_workers=1, rt_alive_tasks=2,
/// rt_global_queue_depth=0`) with the deadline timers firing exactly on
/// schedule, so this was not executor starvation either.
///
/// So: two distinct signatures, `accepted` frozen vs `accepted` moving. Whether
/// they are one fault presenting differently is UNKNOWN. Do not quote the
/// backlog explanation as the cause of a stall without first checking which of
/// the two the server's counters actually show — that is what they are for.
///
/// WHY RETRYING IS THE FIX AND NOT A PAPER-OVER. Abandoning the request drops
/// the future, which drops hyper's connection, so the retry necessarily dials a
/// NEW connection rather than reusing the abandoned one. If the acceptor is
/// healthy and merely lost this connection, the retry succeeds and the hang is
/// gone. If instead the acceptor is wedged process-wide, the retry fails too --
/// but it fails in seconds with a named error rather than stalling silently,
/// which is a strict improvement either way. Which of those two it is has not
/// been established: 2 hangs in ~2500 clones, and neither reproduced on demand.
///
/// WHY NOT JUST LOWER `read_timeout`. That knob is global to the control plane
/// and its 300s value is load-bearing: during a several-hundred-MB PUT the
/// client reads nothing for minutes while the server ships blocks to cloud, so
/// a short read timeout would kill healthy uploads -- the exact regression that
/// value was chosen to avoid. This bound is applied only to requests that carry
/// no body and return a tiny response, where a stall cannot be legitimate.
///
/// THE LAST ATTEMPT IS DELIBERATELY UNBOUNDED. A server that is genuinely slow
/// -- a cold cloud backend taking longer than the deadline to list refs -- must
/// not be newly broken by this. So the final attempt runs exactly as before,
/// still covered by `read_timeout`. Anything that worked before still works;
/// the only change is that a stall now gets two fast retries first.
pub(crate) async fn send_short_control_request<F, Fut>(
    what: &str,
    make: F,
) -> anyhow::Result<reqwest::Response>
where
    F: Fn() -> Fut,
    Fut: std::future::Future<Output = reqwest::Result<reqwest::Response>>,
{
    send_short_control_request_with_deadline(what, short_request_deadline_secs(), make).await
}

/// The body of [`send_short_control_request`], with the deadline passed in.
///
/// Split out purely so the tests can drive it with a 1s deadline instead of
/// mutating `MEDIAGIT_SHORT_REQUEST_DEADLINE_SECS`. That env var is read
/// per-call and process-global, so a test that set it would race every other
/// test in the binary -- and a flaky gate for a bug this rare is worse than no
/// gate at all.
async fn send_short_control_request_with_deadline<F, Fut>(
    what: &str,
    secs: u64,
    make: F,
) -> anyhow::Result<reqwest::Response>
where
    F: Fn() -> Fut,
    Fut: std::future::Future<Output = reqwest::Result<reqwest::Response>>,
{
    if secs == 0 {
        return Ok(send_with_rate_limit_retry(make).await?);
    }
    let bounded_attempts = short_request_attempts();
    for attempt in 1..=bounded_attempts {
        match tokio::time::timeout(
            std::time::Duration::from_secs(secs),
            send_with_rate_limit_retry(&make),
        )
        .await
        {
            Ok(res) => return Ok(res?),
            Err(_) => {
                // Runtime state at the moment of the stall. 20260901-ga38 pinned
                // every retry to a server-side accept with second precision: the
                // server ACCEPTS each fresh connection and the request never
                // reaches its router, while the client sits at flat CPU with all
                // threads in Wait. Two very different faults produce that, and
                // nothing on record can tell them apart:
                //
                //   queue backed up / tasks piling up -> the runtime is starved or
                //       deadlocked, and the connection future is simply never
                //       polled (something is blocking an async worker)
                //   queue empty, task count normal    -> the runtime is healthy and
                //       the stall is inside the connection itself
                //
                // Logged only when a request has ALREADY missed its deadline, so
                // this costs nothing on a healthy run. WARN, not info: main.rs
                // pins the CLI filter to `warn` unless --verbose or MEDIAGIT_LOG is
                // set, and an info line here would be invisible in exactly the
                // campaign runs that catch this (the same trap that made
                // pack_builder's retry logging useless until it was raised).
                //
                // Only the metrics stable without `tokio_unstable` are read.
                let (workers, alive, queued) = match tokio::runtime::Handle::try_current() {
                    Ok(h) => {
                        let m = h.metrics();
                        (
                            m.num_workers() as i64,
                            m.num_alive_tasks() as i64,
                            m.global_queue_depth() as i64,
                        )
                    }
                    Err(_) => (-1, -1, -1),
                };
                tracing::warn!(
                    request = what,
                    attempt,
                    bounded_attempts,
                    deadline_s = secs,
                    rt_workers = workers,
                    rt_alive_tasks = alive,
                    rt_global_queue_depth = queued,
                    "no response headers within deadline; abandoning this connection \
                     and retrying on a fresh one (see MEDIAGIT_SHORT_REQUEST_DEADLINE_SECS)"
                )
            }
        }
    }
    Ok(send_with_rate_limit_retry(&make).await?)
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
    /// `download_chunked_objects`. Takes precedence over the internal default,
    /// but is still overridden by the `MEDIAGIT_DOWNLOAD_CONCURRENCY`
    /// env var when that is set. Note the internal default it displaces is
    /// **32** on this path (`pull.rs::pull_streaming`) and **24** on the pack
    /// range-GET path (`packs.rs`) — the split is deliberate and measured; see
    /// the comment at that `packs.rs` call site. Pass a value derived from
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

        let response =
            send_short_control_request("GET /info/refs", || self.client.get(&url).send())
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

        let response =
            send_short_control_request("GET /info/refs", || self.client.get(&url).send())
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
        self.upload_pack(pack_data).await
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
/// Where an MPU part's bytes come from.
///
/// X2: a 64 MiB cloud pack used to be read into RAM in full before MPU even
/// started, and at `MEDIAGIT_PACK_UPLOAD_CONCURRENCY` = 8 that is a ~512 MiB
/// floor which is precisely what capped that knob at 8 against a cap of 64.
/// MPU parts are natural range reads, so the pack never needs to be resident:
/// `File` reads one part at a time and peak residency becomes `part_size`, not
/// the whole object.
///
/// `Memory` is unchanged behaviour for callers that legitimately already hold
/// the bytes (the per-chunk path, where the chunk was just produced in memory).
pub(crate) enum MpuSource<'a> {
    Memory(&'a [u8]),
    File(&'a std::path::Path),
}

impl MpuSource<'_> {
    /// Read exactly one part.
    ///
    /// The file is re-opened per part rather than held: a single shared handle
    /// would need a cursor shared across the part loop, and re-opening is
    /// negligible against a multi-MiB network PUT. Same reasoning as
    /// `push.rs`'s `upload_pack_file`, which re-opens inside its retry closure.
    async fn part(&self, start: u64, len: usize) -> std::io::Result<Vec<u8>> {
        match self {
            MpuSource::Memory(bytes) => {
                let s = start as usize;
                Ok(bytes[s..s + len].to_vec())
            }
            MpuSource::File(path) => {
                use tokio::io::{AsyncReadExt, AsyncSeekExt};
                let mut f = tokio::fs::File::open(path).await?;
                f.seek(std::io::SeekFrom::Start(start)).await?;
                let mut buf = vec![0u8; len];
                f.read_exact(&mut buf).await?;
                Ok(buf)
            }
        }
    }
}

pub(crate) async fn upload_chunk_mpu(
    api_client: &reqwest::Client,
    direct_client: &reqwest::Client,
    base_url: &str,
    chunk_hex: &str,
    chunk_data: &[u8],
) -> bool {
    upload_object_mpu(
        api_client,
        direct_client,
        base_url,
        "chunks",
        chunk_hex,
        MpuSource::Memory(chunk_data),
        chunk_data.len() as u64,
    )
    .await
}

/// Upload one object via server-orchestrated MPU under `family` (`chunks` or
/// `packs`).
///
/// `family` selects the route set, NOT a request field: an old server simply
/// 404s on `/packs/mpu/start` and this returns false, so the caller falls back
/// to the single-PUT path it already had. A `kind` field would instead be
/// silently ignored by an old server, which would initiate the upload against
/// `chunks/<pack_oid>` -- the wrong prefix, and a `complete_pack` head() failure
/// on an upload that reported success.
///
/// The wire field stays `chunk_id` for both so the server's request types and
/// hex validation are shared; only the object-store prefix differs.
pub(crate) async fn upload_object_mpu(
    api_client: &reqwest::Client,
    direct_client: &reqwest::Client,
    base_url: &str,
    family: &str,
    chunk_hex: &str,
    source: MpuSource<'_>,
    total_len: u64,
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
    let start_url = format!("{}/{}/mpu/start", base_url, family);
    let resp = match send_with_rate_limit_retry(|| {
        api_client
            .post(&start_url)
            .json(&StartReq {
                chunk_id: chunk_hex,
                chunk_size: total_len,
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
        let total = total_len as usize;
        let start = (part.part_number as usize - 1) * part_size;
        let end = (start + part_size).min(total);
        if start >= total {
            break;
        }

        // Read this part once, outside the attempt loop: retries re-send the
        // same bytes, and re-reading per attempt would add syscalls without
        // changing what is sent. Peak residency here is one part, not the whole
        // object -- the entire point of X2.
        //
        // `Bytes`, not `Vec<u8>`, because the send below clones the body on
        // EVERY attempt including the first, and a Vec clone is a full copy.
        // That doubling was visible in the numbers: peak working set at
        // MEDIAGIT_PACK_UPLOAD_CONCURRENCY=8 rose 31.5 MB per extra in-flight
        // pack against a 16 MiB part size -- almost exactly 2x the part, not
        // 1x. Same reasoning and same fix as the single-PUT fallback in
        // `pack_builder.rs`, which already uses `Bytes` for this (B5).
        let part_data: bytes::Bytes = match source.part(start as u64, end - start).await {
            Ok(b) => b.into(),
            Err(e) => {
                // Decline MPU rather than fail the upload: the caller's
                // single-PUT fallback still has a correct path to the bucket,
                // and "degrades, never fails" is this function's contract.
                tracing::debug!(
                    chunk = %chunk_hex,
                    part = part.part_number,
                    err = %e,
                    "MPU part read failed; using single-PUT"
                );
                return false;
            }
        };

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

            // Content-Length is explicit and load-bearing. Without it a body of
            // unknown length becomes `Transfer-Encoding: chunked`, which a
            // presigned S3 PUT rejects with 501 -- and 501 is exactly the code
            // this path treats as "backend has no MPU, fall back to single PUT".
            // Getting it wrong would not error; it would silently demote every
            // pack to the slower path while the logs looked healthy.
            let result = direct_client
                .put(&part.url)
                .header(reqwest::header::CONTENT_LENGTH, part_data.len())
                .body(part_data.clone())
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
                                    .post(format!("{}/{}/mpu/abort", base_url, family))
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
                        .post(format!("{}/{}/mpu/abort", base_url, family))
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
    //
    // RETRIED, unlike everything above it, because by this point every part is
    // ALREADY IN THE BUCKET. Only the commit is outstanding, and giving up here
    // throws away the whole successful upload to re-send the entire body down
    // the single-PUT path -- degrading to the LESS resilient route at the exact
    // moment the link is misbehaving, which is backwards.
    //
    // 20260907-ga42, aws S5: `packs/mpu/complete` returned 500 twenty-three
    // times out of twenty-nine while the server's own aws-sdk-s3 call failed
    // with `dispatch failure` (a transport error, 33-40s latency) during a
    // ~4-minute connectivity fault to s3.ap-south-1. Six of those completions
    // DID succeed, so the fault was intermittent, not fatal. Each 500 sent a
    // fully-uploaded 64 MiB pack back to single-PUT, and two of those then lost
    // their own retry budget to connect timeouts -- packsOffered=32
    // packsCompleted=30, the only failing gate in the run.
    //
    // A retry here is cheap in a way a body retry is not: the request carries
    // only the part list, so it costs one small round trip rather than 64 MiB.
    // That asymmetry is why this loop is bounded by attempts and not by the
    // wall-clock budget the pack PUT uses.
    //
    // Only TRANSIENT statuses are retried. A 4xx means the part list or upload
    // id is wrong, and re-sending an identical request cannot fix that.
    const MPU_COMPLETE_MAX_ATTEMPTS: u32 = 5;
    let complete_url = format!("{}/{}/mpu/complete", base_url, family);
    let mut complete_resp = None;
    for attempt in 0..MPU_COMPLETE_MAX_ATTEMPTS {
        if attempt > 0 {
            // Same equal-jitter shape as the pack PUT backoff, capped low: the
            // server is still reachable (it answered 500), so this is waiting
            // out ITS upstream, not a dead peer.
            let ceiling = (500u64 << (attempt - 1)).min(8_000);
            let wait = ceiling / 2 + jitter_upto(ceiling / 2);
            tokio::time::sleep(std::time::Duration::from_millis(wait)).await;
        }
        match send_with_rate_limit_retry(|| {
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
            Ok(r) => {
                let status = r.status();
                if status.is_success() {
                    complete_resp = Some(r);
                    break;
                }
                // 408/429/5xx are worth another go; anything else is our own
                // request being wrong and will stay wrong.
                let transient =
                    status.as_u16() == 408 || status.as_u16() == 429 || status.is_server_error();
                if !transient || attempt + 1 == MPU_COMPLETE_MAX_ATTEMPTS {
                    tracing::debug!(
                        chunk = %chunk_hex,
                        status = status.as_u16(),
                        attempts = attempt + 1,
                        transient,
                        "MPU complete gave up; falling back to single-PUT"
                    );
                    return false;
                }
                tracing::warn!(
                    chunk = %chunk_hex,
                    status = status.as_u16(),
                    attempt = attempt + 1,
                    "MPU complete transient; retrying (parts are already uploaded, \
                     falling back would re-send the whole body)"
                );
            }
            Err(e) => {
                if attempt + 1 == MPU_COMPLETE_MAX_ATTEMPTS {
                    tracing::debug!(
                        chunk = %chunk_hex,
                        err = %e,
                        attempts = attempt + 1,
                        "MPU complete request failed; falling back"
                    );
                    return false;
                }
                tracing::warn!(
                    chunk = %chunk_hex,
                    err = %e,
                    attempt = attempt + 1,
                    "MPU complete transport failure; retrying"
                );
            }
        }
    }
    let Some(complete_resp) = complete_resp else {
        return false;
    };
    debug_assert!(complete_resp.status().is_success());

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

    /// An absurd `Retry-After` must be CAPPED, not obeyed.
    ///
    /// `rate_limit_backoff` used to honour the header without any upper bound,
    /// which defeated the very invariant its own doc comment claims -- "a push
    /// cannot stall indefinitely behind a misconfigured limiter". With
    /// `rate_limit_max_retries()` defaulting to 10, a server sending
    /// `Retry-After: 900` could park a client for ~3.75 HOURS while the server
    /// itself stayed healthy.
    ///
    /// Not hypothetical: `security.rs` records the server emitting ~900s values
    /// ("the client honoured one and slept 22 minutes"), and 20260820-ga6's
    /// RL6-client-recovers-from-429 hung a clone for 30 minutes against a
    /// server that answered /health in 3ms throughout.
    #[test]
    fn an_absurd_retry_after_is_capped() {
        let h = HeaderValue::from_static("900");
        for _ in 0..50 {
            let d = rate_limit_backoff(0, Some(&h)).as_millis() as u64;
            assert!(
                d <= 60_000,
                "a 900s Retry-After must be capped at the 60s ceiling, got {d}ms"
            );
        }
    }

    /// The cap must bound the WHOLE retry budget, not just one attempt.
    /// Ten attempts at the ceiling is the worst case a caller can face.
    #[test]
    fn total_retry_budget_is_bounded() {
        let h = HeaderValue::from_static("3600");
        let worst: u64 = (0..super::rate_limit_max_retries())
            .map(|a| rate_limit_backoff(a, Some(&h)).as_millis() as u64)
            .sum();
        assert!(
            worst <= 10 * 60_000,
            "worst-case total sleep must stay bounded, got {worst}ms"
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
                    "attempt {attempt}: {d}ms is under half the {ceiling}ms ceiling \
                        - the backoff can collapse to near zero again"
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

/// Gates for the short-control-request deadline, the fix for the intermittent
/// clone hang in which the server accepts a connection into the kernel backlog
/// and never returns it from `accept()`.
///
/// Both directions are asserted, because this guard has two ways to be wrong
/// and only one of them looks like a failure:
///   1. it must RESCUE a stalled connection by retrying on a fresh one
///   2. it must NOT break a server that is merely slow
///
/// A guard tested only in direction 1 would happily ship a client that gives up
/// on every slow cloud backend, and the test suite would stay green.
#[cfg(test)]
mod short_request_deadline_tests {
    use super::send_short_control_request_with_deadline;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    const OK_BODY: &[u8] =
        b"HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: 2\r\n\r\n{}";

    /// Accept connections; stall the first `stall_first` of them forever, then
    /// answer the rest after `delay`. Returns the bound address.
    ///
    /// The stalled sockets are deliberately LEAKED into the spawned task rather
    /// than dropped: dropping would send FIN and the client would see a clean
    /// connection close, which is a different failure from the one under test.
    /// The real bug leaves the client waiting on a socket nobody answers.
    async fn stalling_server(stall_first: usize, delay: std::time::Duration) -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let mut held = Vec::new();
            let mut seen = 0usize;
            loop {
                let Ok((mut sock, _)) = listener.accept().await else {
                    return;
                };
                seen += 1;
                if seen <= stall_first {
                    held.push(sock); // never answered, never closed
                    continue;
                }
                tokio::spawn(async move {
                    // Drain the request BEFORE answering. Closing a socket that
                    // still has unread bytes in its receive buffer sends an RST
                    // instead of a FIN, and the RST discards the response the
                    // client was about to read -- surfacing as WSAECONNRESET
                    // (10054) and looking exactly like a server bug. Read to the
                    // end of the request headers first.
                    let mut buf = Vec::new();
                    let mut byte = [0u8; 1];
                    while !buf.ends_with(b"\r\n\r\n") {
                        match sock.read(&mut byte).await {
                            Ok(0) | Err(_) => return,
                            Ok(_) => buf.push(byte[0]),
                        }
                    }
                    tokio::time::sleep(delay).await;
                    let _ = sock.write_all(OK_BODY).await;
                    let _ = sock.flush().await;
                    // Let the client consume the response before the socket is
                    // dropped; a drop here would race the read on loopback.
                    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
                });
            }
        });
        format!("http://{addr}")
    }

    /// Direction 1: a connection that is accepted and never answered must be
    /// abandoned and retried on a fresh connection, and the request must
    /// ultimately succeed. Without the fix this call never returns.
    #[tokio::test]
    async fn stalled_connection_is_abandoned_and_retried() {
        let url = stalling_server(1, std::time::Duration::ZERO).await;
        crate::ensure_crypto_provider();
        let client = reqwest::Client::new();
        let resp = tokio::time::timeout(
            std::time::Duration::from_secs(20),
            send_short_control_request_with_deadline("GET /test", 1, || client.get(&url).send()),
        )
        .await
        .expect("must not hang: the whole point of the deadline")
        .expect("the retry on a fresh connection must succeed");
        assert_eq!(resp.status(), 200);
    }

    /// The ga49 shape: a stall that outlives MORE THAN TWO connections.
    ///
    /// Three connections are accepted and never answered. Under the old
    /// `BOUNDED_ATTEMPTS = 2` both bounded attempts are consumed by stalls and
    /// the UNBOUNDED fallback then inherits the third stalled connection — and
    /// waits on it forever, which in production meant riding the 300s
    /// `read_timeout` to a failed push (`07_auth/A11-auth-grants`, 720.1s).
    ///
    /// The count is hardcoded at 3, deliberately not derived from
    /// `short_request_attempts()`: an expectation that tracks the constant
    /// cannot fail when the constant is wrong, which is the whole defect here.
    /// Red-verified 2026-09-09 — with `MEDIAGIT_SHORT_REQUEST_ATTEMPTS=2` this
    /// test hangs until its own timeout and fails; at the default 6 it passes.
    #[tokio::test]
    async fn stall_outlasting_two_attempts_still_recovers() {
        let url = stalling_server(3, std::time::Duration::ZERO).await;
        crate::ensure_crypto_provider();
        let client = reqwest::Client::new();
        let resp = tokio::time::timeout(
            std::time::Duration::from_secs(30),
            send_short_control_request_with_deadline("GET /test", 1, || client.get(&url).send()),
        )
        .await
        .expect(
            "a stall longer than two connections must not reach the unbounded \
             attempt -- that is the ga49 hang",
        )
        .expect("a later fresh-connection attempt must succeed");
        assert_eq!(resp.status(), 200);
    }

    /// Direction 2: a server that is merely SLOW must still be served, not
    /// broken by the new bound. Every connection here answers, but only after
    /// twice the deadline -- so both bounded attempts time out and the final
    /// unbounded attempt has to carry it. If that fallback were removed this
    /// test fails, which is exactly the regression it exists to catch.
    #[tokio::test]
    async fn slow_server_still_succeeds_via_the_unbounded_attempt() {
        let url = stalling_server(0, std::time::Duration::from_secs(2)).await;
        crate::ensure_crypto_provider();
        let client = reqwest::Client::new();
        let resp = tokio::time::timeout(
            std::time::Duration::from_secs(30),
            send_short_control_request_with_deadline("GET /slow", 1, || client.get(&url).send()),
        )
        .await
        .expect("a slow server must not be turned into a hang")
        .expect("a slow server must still succeed");
        assert_eq!(resp.status(), 200);
    }

    /// Proof that the two gates above are not vacuous.
    ///
    /// `secs = 0` disables the bound, which IS the pre-fix behaviour. Against
    /// the identical stalling server the request must then never return. If
    /// this ever passes, something else is rescuing the stall and the two tests
    /// above stop being evidence that the deadline does anything -- the exact
    /// way a guard ends up permanently green while protecting nothing.
    #[tokio::test]
    async fn without_the_deadline_the_same_stall_never_returns() {
        let url = stalling_server(1, std::time::Duration::ZERO).await;
        crate::ensure_crypto_provider();
        let client = reqwest::Client::new();
        let outcome = tokio::time::timeout(
            std::time::Duration::from_secs(3),
            send_short_control_request_with_deadline("GET /test", 0, || client.get(&url).send()),
        )
        .await;
        assert!(
            outcome.is_err(),
            "with the bound disabled the stall must persist; it returned instead, \
             so the deadline is not what makes the other tests pass"
        );
    }
}

/// `upload_object_mpu` must not throw away a fully-uploaded MPU because the
/// final commit hit a transient.
///
/// By the time `mpu/complete` is called every part is ALREADY in the bucket.
/// Returning false there sends the whole body back down the single-PUT path --
/// the less resilient route, chosen at the exact moment the link is misbehaving.
///
/// 20260907-ga42, aws S5: `packs/mpu/complete` returned 500 twenty-three times
/// of twenty-nine, while the server's own aws-sdk-s3 call failed with
/// `dispatch failure` during a ~4-minute connectivity fault to s3.ap-south-1.
/// SIX of those completions succeeded, so the fault was intermittent. Each 500
/// demoted a finished 64 MiB pack to single-PUT, and two then lost their retry
/// budget to connect timeouts: packsOffered=32 packsCompleted=30.
#[cfg(test)]
mod mpu_complete_retry_tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    /// Stub standing in for the control plane AND the bucket on one address.
    ///
    /// `complete_failures` completions are answered `fail_status` before the
    /// first 204, so one knob covers "transient then success" and "permanent".
    async fn serve(
        listener: TcpListener,
        addr: String,
        complete_failures: usize,
        fail_status: &'static str,
        completes: Arc<AtomicUsize>,
    ) {
        loop {
            let Ok((mut sock, _)) = listener.accept().await else {
                return;
            };
            let addr = addr.clone();
            let completes = Arc::clone(&completes);
            tokio::spawn(async move {
                let mut buf = Vec::new();
                let mut tmp = [0u8; 8192];
                loop {
                    let Ok(n) = sock.read(&mut tmp).await else {
                        return;
                    };
                    if n == 0 {
                        return;
                    }
                    buf.extend_from_slice(&tmp[..n]);
                    if buf.windows(4).any(|w| w == b"\r\n\r\n") {
                        break;
                    }
                }
                let head = String::from_utf8_lossy(&buf).to_string();
                let first = head.lines().next().unwrap_or("").to_string();
                let reply = |status: &str, body: String, extra: &str| {
                    format!(
                        "HTTP/1.1 {status}\r\ncontent-type: application/json\r\n{extra}content-length: {}\r\nConnection: close\r\n\r\n{body}",
                        body.len()
                    )
                };

                let resp = if first.contains("/packs/mpu/start") {
                    // One part, pointed back at this same stub.
                    let body = format!(
                        r#"{{"upload_id":"up-1","part_size":8388608,"parts":[{{"part_number":1,"url":"http://{addr}/bucket/part1"}}]}}"#
                    );
                    reply("200 OK", body, "")
                } else if first.starts_with("PUT") && first.contains("/bucket/part1") {
                    // A part upload must return an ETag or the client cannot
                    // build the completion request at all.
                    reply("200 OK", "{}".into(), "ETag: \"etag-1\"\r\n")
                } else if first.contains("/packs/mpu/complete") {
                    let n = completes.fetch_add(1, Ordering::SeqCst);
                    if n < complete_failures {
                        reply(fail_status, "{}".into(), "")
                    } else {
                        reply("204 No Content", String::new(), "")
                    }
                } else {
                    reply("404 Not Found", "{}".into(), "")
                };
                let _ = sock.write_all(resp.as_bytes()).await;
                let _ = sock.flush().await;
            });
        }
    }

    async fn run(complete_failures: usize, fail_status: &'static str) -> (bool, usize) {
        crate::ensure_crypto_provider();
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let addr = listener.local_addr().expect("addr").to_string();
        let completes = Arc::new(AtomicUsize::new(0));
        let server = tokio::spawn(serve(
            listener,
            addr.clone(),
            complete_failures,
            fail_status,
            Arc::clone(&completes),
        ));

        let api = reqwest::Client::builder().build().unwrap();
        let direct = reqwest::Client::builder().build().unwrap();
        let ok = super::upload_object_mpu(
            &api,
            &direct,
            &format!("http://{addr}/repo"),
            "packs",
            &"aa".repeat(32),
            super::MpuSource::Memory(&[7u8; 1024]),
            1024,
        )
        .await;
        server.abort();
        (ok, completes.load(Ordering::SeqCst))
    }

    /// THE ga42 CASE. Two transient 500s then success: the MPU must be COMMITTED,
    /// not abandoned. This assertion fails against the previous code, which
    /// returned false on the first non-2xx.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_transient_complete_is_retried_not_abandoned() {
        let (ok, attempts) = run(2, "500 Internal Server Error").await;
        assert!(
            ok,
            "two transient 500s on mpu/complete must be retried -- every part is \
             already in the bucket, and falling back re-sends the whole body down \
             the single-PUT path"
        );
        assert_eq!(
            attempts, 3,
            "expected 2 failed completions + 1 success; got {attempts}"
        );
    }

    /// The other half. A 4xx means our own request is wrong -- the part list or
    /// the upload id -- and re-sending an identical request cannot fix it. It
    /// must NOT be retried, or a permanently broken commit burns five round
    /// trips before falling back.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_permanent_complete_failure_is_not_retried() {
        let (ok, attempts) = run(usize::MAX, "400 Bad Request").await;
        assert!(!ok, "a permanent completion failure must fall back");
        assert_eq!(
            attempts, 1,
            "a 4xx must not be retried; got {attempts} attempts"
        );
    }
}

/// X2: `MpuSource::File` must yield byte-identical parts to `MpuSource::Memory`.
///
/// The whole point of reading parts as ranges is that nobody notices. The risk
/// is arithmetic: a wrong offset silently uploads the wrong bytes (the object
/// still "uploads", and the corruption surfaces much later as a failed
/// verification), and the final part is short whenever the object is not an
/// exact multiple of the part size -- which, for a 64 MiB pack against an
/// 8 MiB part size, is the common case rather than an edge case.
#[cfg(test)]
mod mpu_source_tests {
    use super::MpuSource;

    /// Deterministic, position-dependent bytes: a swapped or overlapping range
    /// changes the content, which a constant fill would hide.
    fn payload(len: usize) -> Vec<u8> {
        (0..len).map(|i| (i % 251) as u8).collect()
    }

    async fn assert_parts_match(total: usize, part_size: usize) {
        let data = payload(total);
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("pack.tmp");
        tokio::fs::write(&path, &data).await.unwrap();

        let from_file = MpuSource::File(&path);
        let from_mem = MpuSource::Memory(&data);

        let mut reassembled = Vec::with_capacity(total);
        let mut start = 0usize;
        while start < total {
            let len = part_size.min(total - start);
            let f = from_file.part(start as u64, len).await.unwrap();
            let m = from_mem.part(start as u64, len).await.unwrap();
            assert_eq!(f.len(), len, "file part length at offset {start}");
            assert_eq!(f, m, "file/memory mismatch at offset {start} len {len}");
            reassembled.extend_from_slice(&f);
            start += len;
        }

        // The decisive assertion: the parts put the original object back
        // together. Per-part equality alone would still pass if every part were
        // read from the same offset.
        //
        // Reported as the first divergent index rather than `assert_eq!` on the
        // vectors: a mismatch on a multi-KiB object otherwise prints both buffers
        // in full, which buries the one number that identifies the bug.
        assert_eq!(
            reassembled.len(),
            data.len(),
            "reassembled length differs (total={total}, part={part_size})"
        );
        if let Some(i) = (0..data.len()).find(|&i| reassembled[i] != data[i]) {
            panic!(
                "parts did not reassemble to the original object                  (total={total}, part={part_size}): first difference at byte {i},                  expected {expected} got {got} -- an offset this far in means the                  part starting at {part_start} was read from the wrong place",
                expected = data[i],
                got = reassembled[i],
                part_start = i - (i % part_size),
            );
        }
    }

    #[tokio::test]
    async fn exact_multiple_of_part_size() {
        assert_parts_match(4096, 1024).await;
    }

    #[tokio::test]
    async fn final_part_is_short() {
        // 4097 = four full parts plus a 1-byte tail; the off-by-one case.
        assert_parts_match(4097, 1024).await;
    }

    #[tokio::test]
    async fn single_part_smaller_than_part_size() {
        assert_parts_match(100, 1024).await;
    }

    #[tokio::test]
    async fn reading_past_the_end_is_an_error_not_a_short_read() {
        // read_exact must fail rather than hand back a truncated part, which
        // would upload silently-wrong bytes.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("short.tmp");
        tokio::fs::write(&path, &payload(10)).await.unwrap();
        assert!(MpuSource::File(&path).part(0, 64).await.is_err());
    }
}
