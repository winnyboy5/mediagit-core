//! Authentication routes for MediaGit server
//!
//! Provides HTTP endpoints for user authentication and management.

use axum::{
    routing::{get, post},
    Router,
};
use std::sync::Arc;

use mediagit_security::auth::{
    login_handler, logout_handler, me_handler, refresh_handler, register_handler, AuthService,
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
    Router::new()
        .route("/auth/register", post(register_handler))
        .route("/auth/login", post(login_handler))
        .route("/auth/logout", post(logout_handler))
        .route("/auth/refresh", post(refresh_handler))
        .route("/auth/me", get(me_handler))
        .with_state(auth_service)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        body::Body,
        http::{Request, StatusCode},
    };
    use mediagit_security::{auth::{LoginRequest, RegisterRequest}, Role};
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
