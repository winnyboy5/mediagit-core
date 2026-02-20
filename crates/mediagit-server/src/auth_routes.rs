// Copyright (C) 2026  winnyboy5
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU Affero General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU Affero General Public License for more details.
//
// You should have received a copy of the GNU Affero General Public License
// along with this program.  If not, see <https://www.gnu.org/licenses/>.
//! Authentication routes for MediaGit server
//!
//! Provides HTTP endpoints for user authentication and management.

use axum::{
    middleware,
    routing::{get, post},
    Router,
};
use std::sync::Arc;

use mediagit_security::auth::{
    auth_middleware, login_handler, logout_handler, me_handler, refresh_handler,
    register_handler, ApiKeyAuth, AuthLayer, AuthService,
};

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
        .with_state(auth_service)
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
            "password": "password123",
            "role": "Write"
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

    #[tokio::test]
    async fn test_login_endpoint() {
        let auth_service = Arc::new(AuthService::new("test-secret"));
        let app = create_auth_router(Arc::clone(&auth_service));

        // Register user first
        let register_body = json!({
            "username": "testuser",
            "email": "test@example.com",
            "password": "password123",
            "role": "Write"
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
            "password": "password123"
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
