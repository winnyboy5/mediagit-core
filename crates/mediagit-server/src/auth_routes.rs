// SPDX-License-Identifier: BUSL-1.1
// Copyright (C) 2025-2026 Aswin Krishnamoorthy

//! Authentication routes for MediaGit server
//!
//! Provides HTTP endpoints for user authentication and management.

use axum::{
    Router,
    extract::DefaultBodyLimit,
    middleware,
    routing::{delete, get, patch, post},
};
use std::sync::Arc;

use mediagit_security::auth::{
    ApiKeyAuth, AuthLayer, AuthService, auth_middleware, login_handler, logout_handler, me_handler,
    refresh_handler, register_handler,
};

use crate::state::AppState;

/// Create authentication router with all auth endpoints
///
/// # Endpoints
/// - POST /auth/register - Register new user
/// - POST /auth/login - Login user
/// - POST /auth/logout - Logout user (client-side token deletion)
/// - POST /auth/refresh - Refresh access token
/// - GET /auth/me - Get current user info (requires authentication)
pub fn create_auth_router(auth_service: Arc<AuthService>) -> Router {
    // Create authentication layer (with dummy API key auth for completeness)
    let api_key_auth = Arc::new(ApiKeyAuth::new());
    let auth_layer = Arc::new(AuthLayer::new(
        Arc::clone(&auth_service.jwt_auth),
        api_key_auth,
    ));

    // Protected routes that require authentication
    let protected = Router::new()
        .route("/auth/me", get(me_handler))
        .layer(middleware::from_fn(move |req, next| {
            let auth_layer = Arc::clone(&auth_layer);
            auth_middleware(auth_layer, req, next)
        }));

    // Public routes
    Router::new()
        .route("/auth/register", post(register_handler))
        .route("/auth/login", post(login_handler))
        .route("/auth/logout", post(logout_handler))
        .route("/auth/refresh", post(refresh_handler))
        .merge(protected)
        // I3: auth payloads are small JSON bodies; cap well below the 2 GiB
        // default used for media chunk/pack uploads.
        .layer(DefaultBodyLimit::max(1024 * 1024))
        .with_state(auth_service)
}

/// Create the admin router (H3): user, grant, and API-key management
/// endpoints, gated on the flat `user:manage` permission (Admin role) except
/// where noted as self-scoped (any authenticated user).
///
/// # Endpoints
/// - GET    /auth/users              - list users (id, username, role)
/// - POST   /auth/users              - admin creates a user with an explicit role
/// - DELETE /auth/users/{id}         - remove a user (cascades their grants)
/// - PATCH  /auth/users/{id}/role    - change a user's role
/// - PATCH  /auth/users/{id}/password - admin password reset (no current password)
/// - POST   /auth/users/{id}/grants  - upsert a per-repo grant
/// - DELETE /auth/users/{id}/grants  - remove a per-repo grant
/// - GET    /auth/keys               - list all API keys (metadata only)
/// - POST   /auth/keys               - self-scoped: mint a key (admin may target `user_id`)
/// - GET    /auth/keys/mine          - self-scoped: list the caller's own keys
/// - DELETE /auth/keys/{id}          - revoke an API key (caller's own, or any as admin)
/// - POST   /auth/password           - self-scoped: change the caller's own password
/// - GET    /auth/whoami             - self-scoped: identity, role, and per-repo grants
///
/// Unlike [`create_auth_router`], this takes `Arc<AppState>` rather than
/// `Arc<AuthService>`: grant mutations (and grant reads for `/auth/whoami`)
/// must go through `AppState::grants` — the same `GrantsStore` instance
/// `check_permission` reads for repo-level enforcement — not
/// `AuthService::grants_store`, a separate in-memory instance that only
/// agrees with `AppState::grants` at boot.
///
/// Only meaningful when `state.auth_service` (and therefore
/// `state.auth_layer`) is `Some`; callers merge it conditionally right
/// alongside `create_auth_router` (see `lib.rs`).
pub fn create_admin_router(state: Arc<AppState>) -> Router {
    let auth_layer = state
        .auth_layer
        .clone()
        .expect("create_admin_router requires auth to be enabled");

    Router::new()
        .route(
            "/auth/users",
            get(crate::handlers::list_users).post(crate::handlers::create_user),
        )
        .route("/auth/users/{id}", delete(crate::handlers::delete_user))
        .route("/auth/users/{id}/role", patch(crate::handlers::set_role))
        // AU-11: suspend/restore without deleting, so offboarding does not
        // have to destroy the account's history to stop its access.
        .route(
            "/auth/users/{id}/disabled",
            patch(crate::handlers::set_disabled),
        )
        .route(
            "/auth/users/{id}/password",
            patch(crate::handlers::reset_password),
        )
        .route(
            "/auth/users/{id}/grants",
            post(crate::handlers::upsert_grant).delete(crate::handlers::remove_grant),
        )
        .route(
            "/auth/keys",
            get(crate::handlers::list_keys).post(crate::handlers::create_key),
        )
        .route("/auth/keys/mine", get(crate::handlers::list_my_keys))
        .route("/auth/keys/{id}", delete(crate::handlers::revoke_key))
        .route("/auth/password", post(crate::handlers::change_password))
        .route("/auth/whoami", get(crate::handlers::whoami))
        .layer(middleware::from_fn(move |req, next| {
            auth_middleware(Arc::clone(&auth_layer), req, next)
        }))
        // Admin payloads are small JSON bodies too (H3), same cap as the
        // rest of /auth/*.
        .layer(DefaultBodyLimit::max(1024 * 1024))
        .with_state(state)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        body::Body,
        http::{Request, StatusCode},
    };
    use serde_json::json;
    use tower::ServiceExt;

    #[tokio::test]
    async fn test_register_endpoint() {
        let auth_service = Arc::new(AuthService::new("test-secret"));
        let app = create_auth_router(auth_service);

        let request_body = json!({
            "username": "testuser",
            "email": "test@example.com",
            "password": "render farm quiet hum"
        });

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/auth/register")
                    .header("content-type", "application/json")
                    .body(Body::from(serde_json::to_string(&request_body).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::CREATED);
    }

    /// AU-3: `allow_open_registration` must survive the trip from
    /// `ServerConfig` to the endpoint.
    ///
    /// It did not. Every `AuthService` constructor hardcoded `true`, and
    /// nothing in `mediagit-server` ever assigned the parsed config value onto
    /// the service — so `allow_open_registration = false` in server.toml, and
    /// the `init` wizard that writes exactly that, were both inert. The
    /// endpoint stayed open while the config, the wizard and the QA gate all
    /// reported success.
    ///
    /// Both halves are asserted, through the real `new_with_full_auth` path a
    /// server actually uses: a config that says closed must produce 403, and a
    /// config that says open must still produce 201. Asserting only the first
    /// would pass against a service that rejects everyone; asserting only the
    /// second is what the old `test_register_endpoint` already did and is what
    /// let this go unnoticed.
    async fn register_status_for(allow_open_registration: bool) -> StatusCode {
        let repos = tempfile::tempdir().unwrap();
        let auth_dir = tempfile::tempdir().unwrap();

        let state = AppState::new_with_full_auth(
            repos.path().to_path_buf(),
            "test-secret",
            auth_dir.path(),
            allow_open_registration,
        )
        .unwrap();

        let auth_service = Arc::clone(state.auth_service().expect("full auth state"));
        let app = create_auth_router(auth_service);

        let body = json!({
            "username": "testuser",
            "email": "test@example.com",
            "password": "render farm quiet hum"
        });

        app.oneshot(
            Request::builder()
                .method("POST")
                .uri("/auth/register")
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_string(&body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap()
        .status()
    }

    #[tokio::test]
    async fn closed_registration_config_actually_closes_the_endpoint() {
        assert_eq!(
            register_status_for(false).await,
            StatusCode::FORBIDDEN,
            "allow_open_registration = false must reject anonymous registration"
        );
    }

    #[tokio::test]
    async fn open_registration_config_still_permits_registration() {
        assert_eq!(
            register_status_for(true).await,
            StatusCode::CREATED,
            "allow_open_registration = true must still admit anonymous registration"
        );
    }

    #[tokio::test]
    async fn test_login_endpoint() {
        let auth_service = Arc::new(AuthService::new("test-secret"));
        let app = create_auth_router(Arc::clone(&auth_service));

        // Register user first
        let register_body = json!({
            "username": "testuser",
            "email": "test@example.com",
            "password": "render farm quiet hum"
        });

        app.clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/auth/register")
                    .header("content-type", "application/json")
                    .body(Body::from(serde_json::to_string(&register_body).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();

        // Now login
        let login_body = json!({
            "identifier": "test@example.com",
            "password": "render farm quiet hum"
        });

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/auth/login")
                    .header("content-type", "application/json")
                    .body(Body::from(serde_json::to_string(&login_body).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
    }
}
