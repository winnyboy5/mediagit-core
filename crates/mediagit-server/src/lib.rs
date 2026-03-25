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

#![allow(missing_docs)]
//! Axum REST API server for MediaGit repositories.
//!
//! Provides HTTP endpoints for push, pull, clone, and repository management.
//! Includes rate limiting, authentication middleware, and CORS support.
//!
//! # Middleware Stack (applied in order)
//!
//! 1. `TraceLayer` — request/response logging via `tracing`
//! 2. `RateLimitLayer` — per-IP rate limiting via `governor`
//! 3. `AuthLayer` — JWT or API key authentication (skipped for `/health` and `/auth/*`)
//! 4. `DefaultBodyLimit` — 10 GiB cap on request bodies
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
pub mod handlers;
pub mod security;
pub mod state;

pub use auth_routes::create_auth_router;
pub use config::ServerConfig;
pub use security::validate_repo_name;
pub use security::RateLimitConfig;
pub use state::AppState;

use axum::{
    extract::DefaultBodyLimit,
    http::StatusCode,
    middleware,
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
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

/// Create the axum router with all endpoints
pub fn create_router(state: Arc<AppState>) -> Router {
    // Create Git protocol routes
    let mut git_router = Router::new()
        .route("/{repo}/info/refs", get(handlers::get_refs))
        .route("/{repo}/refs/update", post(handlers::update_refs))
        .route("/{repo}/objects/want", post(handlers::request_objects))
        .route(
            "/{repo}/objects/pack",
            get(handlers::download_pack).post(handlers::upload_pack),
        )
        // Chunk transfer endpoints for large files (push and pull/clone)
        .route("/{repo}/chunks/check", post(handlers::check_chunks_exist))
        .route(
            "/{repo}/chunks/{chunk_id}",
            get(handlers::download_chunk).put(handlers::upload_chunk),
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
        .with_state(Arc::clone(&state));

    // Apply authentication middleware to Git routes if enabled
    if let Some(auth_layer) = &state.auth_layer {
        let auth_layer = Arc::clone(auth_layer);
        git_router = git_router.layer(middleware::from_fn(move |req, next| {
            auth_middleware(Arc::clone(&auth_layer), req, next)
        }));
    }

    // Merge with auth routes if auth is enabled
    let mut router = if let Some(auth_service) = &state.auth_service {
        let auth_router = create_auth_router(Arc::clone(auth_service));
        git_router.merge(auth_router)
    } else {
        git_router
    };

    // Apply security middleware to all routes
    router = router
        // Body size limit (2GB for large media files)
        .layer(DefaultBodyLimit::max(2 * 1024 * 1024 * 1024))
        .layer(middleware::from_fn(security::audit_middleware))
        .layer(middleware::from_fn(security::security_headers_middleware))
        .layer(middleware::from_fn(security::request_validation_middleware))
        .layer(TraceLayer::new_for_http());

    // Path validation middleware must be applied as the outermost layer
    // to intercept requests before routing
    router = router.layer(middleware::from_fn(security::path_validation_middleware));

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
    use security::{GovernorConfigBuilder, GovernorLayer, SmartIpKeyExtractor};

    // Build rate limiting configuration
    let governor_config = Arc::new(
        GovernorConfigBuilder::default()
            .per_second(rate_limit_config.requests_per_second)
            .burst_size(rate_limit_config.burst_size)
            .use_headers()
            .key_extractor(SmartIpKeyExtractor)
            .finish()
            .expect("Failed to build rate limiter config"),
    );

    // Create cleanup task
    let limiter = governor_config.limiter().clone();
    let cleanup_task = move || {
        use std::time::Duration;
        let interval = Duration::from_secs(60);
        loop {
            std::thread::sleep(interval);
            let size = limiter.len();
            if size > 0 {
                tracing::debug!("Rate limiter storage size: {}, cleaning up...", size);
                limiter.retain_recent();
            }
        }
    };

    let mut router = Router::new()
        .route("/{repo}/info/refs", get(handlers::get_refs))
        .route("/{repo}/refs/update", post(handlers::update_refs))
        .route("/{repo}/objects/want", post(handlers::request_objects))
        .route(
            "/{repo}/objects/pack",
            get(handlers::download_pack).post(handlers::upload_pack),
        )
        // Chunk transfer endpoints for large files (push and pull/clone)
        .route("/{repo}/chunks/check", post(handlers::check_chunks_exist))
        .route(
            "/{repo}/chunks/{chunk_id}",
            get(handlers::download_chunk).put(handlers::upload_chunk),
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
        .with_state(Arc::clone(&state));

    // Apply middleware layers
    router = router
        // Body size limit (2GB for large media files)
        .layer(DefaultBodyLimit::max(2 * 1024 * 1024 * 1024))
        // Rate limiting (applied first, before other middleware)
        .layer(GovernorLayer::new(governor_config))
        // Security middleware
        .layer(middleware::from_fn(security::audit_middleware))
        .layer(middleware::from_fn(security::security_headers_middleware))
        .layer(middleware::from_fn(security::request_validation_middleware))
        .layer(TraceLayer::new_for_http());

    // Add authentication middleware if enabled
    if let Some(auth_layer) = &state.auth_layer {
        let auth_layer = Arc::clone(auth_layer);
        router = router.layer(middleware::from_fn(move |req, next| {
            auth_middleware(Arc::clone(&auth_layer), req, next)
        }));
    }

    // Path validation middleware must be applied as the outermost layer
    // to intercept requests before routing
    router = router.layer(middleware::from_fn(security::path_validation_middleware));

    // Health check is merged AFTER all middleware so it bypasses auth + rate-limiting
    router = router.merge(
        Router::new()
            .route("/healthz", get(health_handler))
            .route("/health", get(health_handler)),
    );

    (router, cleanup_task)
}
