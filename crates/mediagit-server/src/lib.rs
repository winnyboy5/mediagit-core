// Library exports for mediagit-server
// This allows integration tests to use server components

pub mod config;
pub mod handlers;
pub mod state;

pub use state::AppState;
pub use config::ServerConfig;

use axum::{
    middleware,
    routing::{get, post},
    Router,
};
use std::sync::Arc;
use tower_http::trace::TraceLayer;

use mediagit_security::auth::auth_middleware;

/// Create the axum router with all endpoints
pub fn create_router(state: Arc<AppState>) -> Router {
    let mut router = Router::new()
        .route("/:repo/info/refs", get(handlers::get_refs))
        .route("/:repo/refs/update", post(handlers::update_refs))
        .route("/:repo/objects/want", post(handlers::request_objects))
        .route("/:repo/objects/pack", get(handlers::download_pack))
        .route("/:repo/objects/pack", post(handlers::upload_pack))
        .layer(TraceLayer::new_for_http());

    // Add authentication middleware if enabled
    if let Some(auth_layer) = &state.auth_layer {
        let auth_layer = Arc::clone(auth_layer);
        router = router.layer(middleware::from_fn(move |req, next| {
            auth_middleware(Arc::clone(&auth_layer), req, next)
        }));
    }

    router.with_state(state)
}
