// Library exports for mediagit-server
// This allows integration tests to use server components

pub mod config;
pub mod handlers;
pub mod security;
pub mod state;
pub mod auth_routes;

pub use config::ServerConfig;
pub use security::validate_repo_name;
pub use security::RateLimitConfig;
pub use state::AppState;
pub use auth_routes::create_auth_router;

use axum::{
    extract::DefaultBodyLimit,
    middleware,
    routing::{get, post, put},
    Router,
};
use std::sync::Arc;
use tower_http::trace::TraceLayer;

use mediagit_security::auth::auth_middleware;

/// Create the axum router with all endpoints
pub fn create_router(state: Arc<AppState>) -> Router {
    // Create Git protocol routes
    let mut git_router = Router::new()
        .route("/:repo/info/refs", get(handlers::get_refs))
        .route("/:repo/refs/update", post(handlers::update_refs))
        .route("/:repo/objects/want", post(handlers::request_objects))
        .route("/:repo/objects/pack", get(handlers::download_pack))
        .route("/:repo/objects/pack", post(handlers::upload_pack))
        // Chunk transfer endpoints for large files (push)
        .route("/:repo/chunks/check", post(handlers::check_chunks_exist))
        .route("/:repo/chunks/:chunk_id", put(handlers::upload_chunk))
        .route("/:repo/manifests/:oid", put(handlers::upload_manifest))
        // Chunk download endpoints for large files (pull/clone)
        .route("/:repo/chunks/:chunk_id", get(handlers::download_chunk))
        .route("/:repo/manifests/:oid", get(handlers::download_manifest))
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
        .route("/:repo/info/refs", get(handlers::get_refs))
        .route("/:repo/refs/update", post(handlers::update_refs))
        .route("/:repo/objects/want", post(handlers::request_objects))
        .route("/:repo/objects/pack", get(handlers::download_pack))
        .route("/:repo/objects/pack", post(handlers::upload_pack))
        // Chunk transfer endpoints for large files (push)
        .route("/:repo/chunks/check", post(handlers::check_chunks_exist))
        .route("/:repo/chunks/:chunk_id", put(handlers::upload_chunk))
        .route("/:repo/manifests/:oid", put(handlers::upload_manifest))
        // Chunk download endpoints for large files (pull/clone)
        .route("/:repo/chunks/:chunk_id", get(handlers::download_chunk))
        .route("/:repo/manifests/:oid", get(handlers::download_manifest))
        .with_state(Arc::clone(&state));

    // Apply middleware layers
    router = router
        // Body size limit (2GB for large media files)
        .layer(DefaultBodyLimit::max(2 * 1024 * 1024 * 1024))
        // Rate limiting (applied first, before other middleware)
        .layer(GovernorLayer {
            config: governor_config,
        })
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

    (router, cleanup_task)
}
