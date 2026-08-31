// SPDX-License-Identifier: BUSL-1.1
// Copyright (C) 2025-2026 Aswin Krishnamoorthy

#![allow(missing_docs)]
//! Axum REST API server for MediaGit repositories.
//!
//! Provides HTTP endpoints for push, pull, clone, and repository management.
//! Includes rate limiting and authentication middleware. CORS is off by
//! default and only added when `cors_allowed_origins` is set in the server
//! config (see `apply_cors_layer`).
//!
//! # Middleware Stack (applied in order)
//!
//! 1. `TraceLayer` — request/response logging via `tracing`
//! 2. `RateLimitLayer` — per-IP rate limiting via `governor`
//! 3. `AuthLayer` — JWT or API key authentication (skipped for `/health` and `/auth/*`)
//! 4. `DefaultBodyLimit` — 2 GiB cap on request bodies (1 MiB on `/auth/*`,
//!    `/{repo}/refs/update`, and `/{repo}/locks*`)
//!
//! # Quick Start
//!
//! ```no_run
//! use mediagit_server::{create_router, AppState};
//! use std::sync::Arc;
//! use std::path::PathBuf;
//!
//! let state = Arc::new(AppState::new(PathBuf::from("/data/repos")));
//! let app = create_router(state);
//! // Serve with: axum::serve(listener, app).await
//! ```

// Library exports for mediagit-server
// This allows integration tests to use server components

pub mod auth_routes;
pub mod config;
pub mod encryption;
pub mod handlers;
pub mod instance_lock;
pub mod locks;
pub mod security;
pub mod state;

pub use auth_routes::create_auth_router;
pub use config::ServerConfig;
pub use security::RateLimitConfig;
pub use security::validate_repo_name;
pub use state::AppState;

use axum::{
    Json, Router,
    extract::DefaultBodyLimit,
    http::StatusCode,
    middleware,
    response::IntoResponse,
    routing::{delete, get, post, put},
};
use std::sync::Arc;
use tower_http::trace::TraceLayer;

use mediagit_security::auth::auth_middleware;

/// Health check handler — always returns 200 OK with version info.
/// This route is intentionally placed **outside** auth/rate-limit middleware
/// so that container orchestrators (k8s, Docker Compose, AWS ELB) can probe
/// the server without needing credentials.
async fn health_handler() -> impl IntoResponse {
    (
        StatusCode::OK,
        Json(serde_json::json!({
            "status": "ok",
            "version": env!("CARGO_PKG_VERSION"),
            "service": "mediagit-server"
        })),
    )
}

/// OP-7: build the per-request tracing span, adding the client's
/// correlation id (if any) as an `op_id` field so it lands on every log line
/// emitted while handling the request -- including storage-layer errors,
/// which is where diagnosing today's incidents actually needed it. Before
/// this, the client error and the server log shared no join key but the
/// timestamp, which doesn't scale to a concurrent multi-GB transfer.
///
/// Same shape as `tower_http::trace::DefaultMakeSpan`'s default (name
/// "request", DEBUG, method/uri/version) plus `op_id`, so this isn't a
/// second logging mechanism, just that one extended.
///
/// The header (`mediagit_protocol::client::OP_ID_HEADER`, NOT
/// `X-Request-ID` -- that one is the load-bearing want-cache key for
/// `GET /objects/pack`, see `handlers::repo`) is optional: curl, health
/// checks, and older clients that never send it still get a span, just with
/// a server-minted id that can't be joined back to a client-side log.
fn make_request_span(request: &axum::extract::Request) -> tracing::Span {
    let op_id = request
        .headers()
        .get(mediagit_protocol::client::OP_ID_HEADER)
        .and_then(|v| v.to_str().ok())
        .map(str::to_string)
        .unwrap_or_else(state::generate_request_id);
    tracing::debug_span!(
        "request",
        method = %request.method(),
        uri = %request.uri(),
        version = ?request.version(),
        op_id = %op_id,
    )
}

/// I4: wrap `router` with a `CorsLayer` restricted to `origins` (exact
/// match), or return it unchanged if `origins` is `None`/empty — today's
/// behavior (no CORS layer, no CORS headers) is preserved when CORS isn't
/// configured. Applied as the outermost layer by the caller so CORS
/// preflight (`OPTIONS`) requests are answered before auth/path-validation
/// middleware would otherwise reject them.
pub fn apply_cors_layer(router: Router, origins: Option<&[String]>) -> Router {
    use axum::http::{HeaderName, Method};
    use tower_http::cors::CorsLayer;

    let Some(origins) = origins.filter(|o| !o.is_empty()) else {
        return router;
    };

    let allowed_origins: Vec<_> = origins.iter().filter_map(|o| o.parse().ok()).collect();

    let cors = CorsLayer::new()
        .allow_origin(allowed_origins)
        .allow_methods([
            Method::GET,
            Method::POST,
            Method::PUT,
            Method::DELETE,
            Method::OPTIONS,
        ])
        .allow_headers([
            HeaderName::from_static("authorization"),
            HeaderName::from_static("content-type"),
        ]);

    router.layer(cors)
}

/// I3: 1 MiB request body cap for endpoints that only ever carry small JSON
/// payloads (ref updates, lock records) — as opposed to the 2 GiB default
/// that exists to accommodate large media chunk/pack uploads. Built as a
/// standalone router with its own `DefaultBodyLimit` layer and merged into
/// the main router, since axum's `DefaultBodyLimit` is a router-wide layer,
/// not something settable per-`.route()` call; the layer closest to the
/// handler wins, so this stays in effect even though the outer 2 GiB layer
/// is also applied to the merged whole.
fn small_body_routes(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/{repo}/refs/update", post(handlers::update_refs))
        .route(
            "/{repo}/locks",
            get(handlers::list_locks).post(handlers::create_lock),
        )
        .route("/{repo}/locks/{lock_id}", delete(handlers::delete_lock))
        .layer(DefaultBodyLimit::max(1024 * 1024))
        .with_state(state)
}

/// Create the axum router with all endpoints
pub use security::SharedRateLimiter;

/// Create the router without rate limiting.
pub fn create_router(state: Arc<AppState>) -> Router {
    build_router(state, None)
}

/// Create a router that shares an existing rate limiter.
///
/// SV-1: the HTTPS listener used to call `create_router`, so switching TLS on
/// silently switched rate limiting *off* for the port actually exposed to the
/// internet — while the log still said "Rate limiting ENABLED". Sharing the
/// limiter rather than building a second one matters: two independent
/// governors would hand an attacker twice the budget for splitting traffic
/// across the two ports.
pub fn create_router_sharing_rate_limit(
    state: Arc<AppState>,
    limiter: SharedRateLimiter,
) -> Router {
    build_router(state, Some(limiter))
}

/// Single source of truth for the route table and middleware stack.
///
/// Previously `create_router` and `create_router_with_rate_limit` each built
/// their own copy of ~28 routes. Two parallel route tables drift — a route
/// added to one and not the other exists or vanishes depending on whether
/// rate limiting happens to be on — and that duplication is exactly what let
/// the HTTPS branch pick the wrong builder.
/// Connections accepted by the listener, and requests that reached the router.
///
/// THE GAP THESE CLOSE. A clone hung for 240s+ in ga36 (and 300s in ga33) with
/// the client reporting `Failed to send GET /info/refs: operation timed out`,
/// while the server logged NOTHING for it -- not even `TraceLayer`'s
/// "started processing request". The runtime was demonstrably healthy: the
/// heartbeat ticked straight through. So the request died somewhere between the
/// kernel accepting the TCP connection and the router seeing it, and NOTHING in
/// that stretch was instrumented.
///
/// Two campaigns produced two reproductions and neither could name the
/// component, because the only available evidence was an absence. These two
/// counters turn that absence into a reading:
///
///   accepted > served and not moving -> the connection arrived but its request
///                                       never reached the router (HTTP parse,
///                                       TLS, or a stuck connection task)
///   accepted not moving              -> the acceptor itself is stuck; the
///                                       client's connection is sitting in the
///                                       kernel backlog showing ESTABLISHED
///
/// Relaxed ordering throughout: these are diagnostic counters read by a
/// once-per-10s log line, not a synchronisation mechanism.
pub static REQS_ROUTED: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Connections the listener has handed to axum. See `REQS_ROUTED` for why.
pub static CONNS_ACCEPTED: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Wrap a listener so every accepted connection is counted.
///
/// `tap_io` runs the moment axum accepts, before hyper reads a single byte, so
/// this counts connections that never produce a request -- which is precisely
/// the case `REQS_ROUTED` alone cannot tell apart from "never accepted".
pub fn counting_listener(
    listener: tokio::net::TcpListener,
) -> axum::serve::TapIo<
    tokio::net::TcpListener,
    impl FnMut(&mut tokio::net::TcpStream) + Send + 'static,
> {
    use axum::serve::ListenerExt;
    listener.tap_io(|_| {
        CONNS_ACCEPTED.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    })
}

/// Millis since process start at which the last request reached the router.
static LAST_ROUTED_MS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Process start, so `idle_s` is meaningful before the first request.
static START: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();

fn now_ms() -> u64 {
    START
        .get_or_init(std::time::Instant::now)
        .elapsed()
        .as_millis() as u64
}

/// Seconds since a request last reached the router (uptime if none ever has).
///
/// Read by the runtime heartbeat. A stall in which this climbs while
/// `REQS_ROUTED` stands still is positive evidence that nothing is arriving --
/// as opposed to arriving and failing to be logged, which is what an absence of
/// log lines alone cannot distinguish.
pub fn secs_since_last_routed_request() -> u64 {
    (now_ms().saturating_sub(LAST_ROUTED_MS.load(std::sync::atomic::Ordering::Relaxed))) / 1000
}

/// Counts every request that reaches the router, as the OUTERMOST layer.
///
/// Deliberately outside `TraceLayer`: the whole point is to count requests that
/// arrive even when the tracing layer never logs them.
async fn count_routed_requests(
    request: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    REQS_ROUTED.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    LAST_ROUTED_MS.store(now_ms(), std::sync::atomic::Ordering::Relaxed);
    next.run(request).await
}

fn build_router(state: Arc<AppState>, rate_limiter: Option<SharedRateLimiter>) -> Router {
    // Create Git protocol routes
    let mut git_router = Router::new()
        .route("/{repo}/info/refs", get(handlers::get_refs))
        .route("/{repo}/objects/want", post(handlers::request_objects))
        .route(
            "/{repo}/objects/pack",
            get(handlers::download_pack).post(handlers::upload_pack),
        )
        // Chunk transfer endpoints for large files (push and pull/clone)
        .route("/{repo}/chunks/check", post(handlers::check_chunks_exist))
        .route(
            "/{repo}/chunks/upload-urls",
            post(handlers::presign_chunk_uploads),
        )
        .route(
            "/{repo}/chunks/download-urls",
            post(handlers::presign_chunk_downloads),
        )
        .route(
            "/{repo}/chunks/complete",
            post(handlers::complete_chunk_uploads),
        )
        .route(
            "/{repo}/chunks/verify-integrity",
            post(handlers::verify_chunk_integrity),
        )
        .route(
            "/{repo}/objects/verify-integrity",
            post(handlers::verify_object_integrity),
        )
        .route("/{repo}/chunks/mpu/start", post(handlers::mpu_start))
        .route("/{repo}/chunks/mpu/complete", post(handlers::mpu_complete))
        .route("/{repo}/chunks/mpu/abort", post(handlers::mpu_abort))
        .route(
            "/{repo}/chunks/{chunk_id}",
            get(handlers::download_chunk).put(handlers::upload_chunk),
        )
        // Chunk-delta sidecar endpoints: lets clients pull deltas during
        // clone/fetch instead of inflated full chunks (preserves storage savings).
        .route(
            "/{repo}/chunk-deltas/check",
            post(handlers::check_chunk_deltas_exist),
        )
        .route(
            "/{repo}/chunk-deltas/{chunk_id}",
            get(handlers::download_chunk_delta).put(handlers::upload_chunk_delta),
        )
        .route(
            "/{repo}/manifests/{oid}",
            get(handlers::download_manifest).put(handlers::upload_manifest),
        )
        // Raw file serving endpoints (read-only, repo:read permission)
        .route(
            "/{repo}/files/{*path}",
            get(handlers::download_file_by_path),
        )
        .route("/{repo}/tree/{*path}", get(handlers::list_tree))
        .route("/{repo}/tree", get(handlers::list_tree_root))
        // Pack manifest endpoints (F6) — Track-F cloud pack bundling
        .route("/{repo}/packs/complete", post(handlers::complete_pack))
        .route(
            "/{repo}/packs/upload-urls",
            post(handlers::presign_pack_uploads),
        )
        .route("/{repo}/packs/{pack_id}", put(handlers::upload_pack_proxy))
        .route("/{repo}/chunks/locate", post(handlers::locate_chunks))
        // DC-7/D4 key escrow. Both 404 when `[encryption]` is off, so a server
        // without the feature is indistinguishable from one that predates it.
        .route(
            "/{repo}/encryption-key",
            put(handlers::put_encryption_key).get(handlers::get_encryption_key),
        )
        .route(
            "/{repo}/packs/presign-download-urls",
            post(handlers::presign_pack_downloads),
        )
        .route(
            "/{repo}/packs/rebuild-index",
            post(handlers::rebuild_pack_index),
        )
        // D2: batch-fetch multiple chunk slices out of one pack in a single
        // request — for backends with no presigned GET (GCS + ADC).
        .route(
            "/{repo}/packs/batch-get",
            post(handlers::batch_get_pack_chunks),
        )
        .with_state(Arc::clone(&state))
        // I3: refs/update + locks routes live in their own 1 MiB-capped
        // router (see `small_body_routes`); merged in before the auth layer
        // below so they still get the same auth/rate-limit/security stack.
        .merge(small_body_routes(Arc::clone(&state)));

    // Apply authentication middleware to Git routes if enabled
    if let Some(auth_layer) = &state.auth_layer {
        let auth_layer = Arc::clone(auth_layer);
        git_router = git_router.layer(middleware::from_fn(move |req, next| {
            auth_middleware(Arc::clone(&auth_layer), req, next)
        }));
    }

    // Merge with auth + admin routes if auth is enabled
    let mut router = if let Some(auth_service) = &state.auth_service {
        let auth_router = create_auth_router(Arc::clone(auth_service));
        let admin_router = auth_routes::create_admin_router(Arc::clone(&state));
        git_router.merge(auth_router).merge(admin_router)
    } else {
        git_router
    };

    // Apply security middleware to all routes
    if let Some(limiter) = rate_limiter {
        // Rate limiting sits inside the body limit and outside the security
        // middleware, matching the order the rate-limited builder used.
        router = router.layer(security::GovernorLayer::new(limiter));
    }
    router = router
        // Body size limit (2GB for large media files)
        .layer(DefaultBodyLimit::max(2 * 1024 * 1024 * 1024))
        .layer(middleware::from_fn(security::audit_middleware))
        .layer(middleware::from_fn(security::security_headers_middleware))
        .layer(middleware::from_fn(security::request_validation_middleware))
        .layer(TraceLayer::new_for_http().make_span_with(make_request_span));

    // Path validation middleware must be applied as the outermost layer
    // to intercept requests before routing
    router = router.layer(middleware::from_fn(security::path_validation_middleware));

    // Outside even path validation, so a request is counted the instant it
    // reaches the router -- before anything can reject, block or fail to log it.
    router = router.layer(middleware::from_fn(count_routed_requests));

    // Health check is merged AFTER all middleware so it bypasses auth + rate-limiting
    router = router.merge(
        Router::new()
            .route("/healthz", get(health_handler))
            .route("/health", get(health_handler)),
    );

    router
}

/// Create the axum router with rate limiting
///
/// This function creates a router with rate limiting enabled. The rate limiter
/// uses IP-based rate limiting (via SmartIpKeyExtractor) and should be used
/// in production environments.
///
/// # Rate Limit Headers
///
/// When rate limiting is enabled, the following headers are included in responses:
/// - `x-ratelimit-limit`: Total request quota
/// - `x-ratelimit-remaining`: Remaining requests in current window
/// - `x-ratelimit-after`: Seconds until quota reset (when limit exceeded)
/// - `retry-after`: Same as x-ratelimit-after (standard header)
///
/// # Background Cleanup
///
/// The rate limiter stores state for each IP address. To prevent memory leaks,
/// you should spawn the cleanup task returned by `build_with_cleanup()`:
///
/// ```no_run
/// use mediagit_server::{AppState, RateLimitConfig, create_router_with_rate_limit};
/// use std::sync::Arc;
/// use std::path::PathBuf;
///
/// # async fn example() {
/// let state = Arc::new(AppState::new(PathBuf::from("/tmp/repos")));
/// let rate_config = RateLimitConfig::default();
/// let (router, cleanup) = create_router_with_rate_limit(state, rate_config);
///
/// // Spawn cleanup task in background
/// std::thread::spawn(cleanup);
/// # }
/// ```
pub fn create_router_with_rate_limit(
    state: Arc<AppState>,
    rate_limit_config: RateLimitConfig,
) -> (Router, impl FnOnce() + Send + 'static) {
    let (router, cleanup, _limiter) = create_rate_limited_router(state, rate_limit_config);
    (router, cleanup)
}

/// As `create_router_with_rate_limit`, but also hands back the limiter so a
/// second listener (HTTPS) can share it via `create_router_sharing_rate_limit`.
pub fn create_rate_limited_router(
    state: Arc<AppState>,
    rate_limit_config: RateLimitConfig,
) -> (Router, impl FnOnce() + Send + 'static, SharedRateLimiter) {
    // One builder, in `security.rs`. This used to inline its own copy --
    // same shape, but keyed by `SmartIpKeyExtractor` instead of
    // `IdentityOrIpKeyExtractor`, and with a second cleanup thread. That copy
    // is why the per-identity keying `RateLimitConfig` documents was never
    // actually in effect: everyone behind one NAT or one CI runner pool shared
    // a single budget.
    let (governor_config, cleanup_task) = rate_limit_config.build_with_cleanup();

    let router = build_router(state, Some(Arc::clone(&governor_config)));
    (router, cleanup_task, governor_config)
}

/// The routed-request counter is diagnostic, so it has exactly one job: be
/// accurate about whether a request reached the router. Both directions matter.
///
/// This exists because ga33 and ga36 each burned a campaign on a hang whose only
/// evidence was an ABSENCE of log lines -- and an absence cannot distinguish
/// "no request arrived" from "a request arrived and logging failed". A counter
/// that silently stopped incrementing would recreate exactly that ambiguity
/// while looking like an answer, so it is asserted in both directions here.
#[cfg(test)]
mod routed_counter_tests {
    use super::*;
    use axum::body::Body;
    use axum::http::{Request as HttpRequest, StatusCode};
    use tower::ServiceExt;

    fn routed() -> u64 {
        REQS_ROUTED.load(std::sync::atomic::Ordering::Relaxed)
    }

    /// One test, not two: `REQS_ROUTED` is a process-global counter by design
    /// (it is a server-wide diagnostic), so two parallel tests asserting exact
    /// deltas against it race each other. Splitting them produced exactly that
    /// -- an off-by-one that was the harness, not the code.
    ///
    /// Covers, in order:
    ///   1. a served request increments the counter
    ///   2. a REJECTED request increments it too -- the counter answers "did
    ///      anything reach us", not "did anything succeed". Were rejects
    ///      uncounted, a server being flooded with bad paths would look
    ///      identical to one receiving nothing at all.
    ///   3. the idle clock resets when a request lands and never runs backwards
    #[tokio::test]
    async fn routed_counter_and_idle_clock_are_accurate_in_both_directions() {
        let tmp = tempfile::tempdir().unwrap();
        let state = std::sync::Arc::new(AppState::new(tmp.path().to_path_buf()));

        // NOT /healthz. Health routes are merged AFTER the middleware stack so
        // they bypass auth and rate limiting -- and therefore this counter too.
        // That is load-bearing rather than incidental: the QA harness polls
        // /healthz continuously, so if health checks reset the idle clock the
        // heartbeat would report a healthy 0 straight through a total stall.
        let before = routed();
        let app = build_router(std::sync::Arc::clone(&state), None);
        let _ = app
            .oneshot(
                HttpRequest::builder()
                    .uri("/some-repo/info/refs")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let after_ok = routed();
        assert_eq!(
            after_ok,
            before + 1,
            "a request that reached the router must be counted"
        );

        // A repo name containing `..` is rejected by path_validation_middleware,
        // which sits INSIDE the counter, so the reject must still be counted.
        //
        // The path must still MATCH a route. `/../etc/info/refs` does not, and
        // an unmatched path is served by the router's fallback, which sits
        // outside the layer stack and is therefore never counted -- an earlier
        // draft of this test used it and failed for that reason, not because the
        // counter was wrong.
        let app2 = build_router(std::sync::Arc::clone(&state), None);
        let resp2 = app2
            .oneshot(
                HttpRequest::builder()
                    .uri("/bad..repo/info/refs")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_ne!(
            resp2.status(),
            StatusCode::OK,
            "a traversal attempt must not be served"
        );
        assert_eq!(
            routed(),
            after_ok + 1,
            "a REJECTED request still reached the router and must be counted"
        );

        // The idle clock must reset on arrival -- a frozen clock would be worse
        // than none, reporting a healthy 0 through a total stall.
        assert_eq!(
            secs_since_last_routed_request(),
            0,
            "idle clock must reset when a request reaches the router"
        );
        let a = secs_since_last_routed_request();
        let b = secs_since_last_routed_request();
        assert!(b >= a, "idle clock must not run backwards");

        // `accepted` vs `routed` is the whole point of having two counters, so
        // the discriminating case is the one asserted: a connection that is
        // accepted and then sends NOTHING. That is the shape of the hang --
        // ESTABLISHED on the client, silent on the server -- and it must move
        // `accepted` while leaving `routed` alone. A counter that only moved
        // alongside `routed` would be decoration; it could never tell the two
        // failure modes apart, which is the only reason it exists.
        let accepted_before = CONNS_ACCEPTED.load(std::sync::atomic::Ordering::Relaxed);
        let routed_before = routed();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let mut tapped = counting_listener(listener);
        let acceptor = tokio::spawn(async move {
            use axum::serve::Listener;
            let _ = tapped.accept().await;
        });
        // Held open until the accept completes: dropping it early would let the
        // connection be torn down before `tap_io` ever ran.
        let _conn = tokio::net::TcpStream::connect(addr).await.unwrap();
        acceptor.await.unwrap();
        assert_eq!(
            CONNS_ACCEPTED.load(std::sync::atomic::Ordering::Relaxed),
            accepted_before + 1,
            "an accepted connection must be counted before any byte is read"
        );
        assert_eq!(
            routed(),
            routed_before,
            "a connection that sent no request must NOT be counted as routed"
        );
    }
}
