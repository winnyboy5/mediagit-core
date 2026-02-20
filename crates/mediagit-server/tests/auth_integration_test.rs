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
//! Integration tests for authentication system
//!
//! Tests the complete authentication flow including registration, login, and protected endpoints.

use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use mediagit_security::auth::AuthService;
use mediagit_server::create_auth_router;
use serde_json::json;
use std::sync::Arc;
use tower::ServiceExt;

#[tokio::test]
async fn test_complete_auth_flow() {
    // Create auth service
    let auth_service = Arc::new(AuthService::new("test-secret-key"));
    let app = create_auth_router(Arc::clone(&auth_service));

    // 1. Register a new user
    let register_body = json!({
        "username": "testuser",
        "email": "test@example.com",
        "password": "securepassword123",
        "role": "Write"
    });

    let response = app
        .clone()
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

    assert_eq!(response.status(), StatusCode::CREATED);

    // 2. Login with the registered user
    let login_body = json!({
        "identifier": "test@example.com",
        "password": "securepassword123"
    });

    let response = app
        .clone()
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

    // Extract response body to get token
    let body_bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let auth_response: mediagit_security::auth::AuthResponse =
        serde_json::from_slice(&body_bytes).unwrap();

    assert!(!auth_response.tokens.access_token.is_empty());
    assert_eq!(auth_response.user.username, "testuser");
    assert_eq!(auth_response.user.email, "test@example.com");

    // 3. Access protected endpoint (/auth/me) with token
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/auth/me")
                .header(
                    "authorization",
                    format!("Bearer {}", auth_response.tokens.access_token),
                )
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    // 4. Refresh token
    let refresh_body = json!({
        "refresh_token": auth_response.tokens.refresh_token
    });

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/auth/refresh")
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_string(&refresh_body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    // 5. Logout
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/auth/logout")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::NO_CONTENT);
}

#[tokio::test]
async fn test_invalid_credentials() {
    let auth_service = Arc::new(AuthService::new("test-secret-key"));
    let app = create_auth_router(auth_service);

    // Try to login with non-existent user
    let login_body = json!({
        "identifier": "nonexistent@example.com",
        "password": "wrongpassword"
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

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn test_protected_endpoint_without_auth() {
    let auth_service = Arc::new(AuthService::new("test-secret-key"));
    let app = create_auth_router(auth_service);

    // Try to access /auth/me without authentication
    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/auth/me")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn test_duplicate_registration() {
    let auth_service = Arc::new(AuthService::new("test-secret-key"));
    let app = create_auth_router(Arc::clone(&auth_service));

    // Register first user
    let register_body = json!({
        "username": "duplicate",
        "email": "duplicate@example.com",
        "password": "password123",
        "role": "Write"
    });

    let response = app
        .clone()
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

    assert_eq!(response.status(), StatusCode::CREATED);

    // Try to register again with same email
    let response = app
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

    assert_eq!(response.status(), StatusCode::CONFLICT);
}

#[tokio::test]
async fn test_weak_password_rejected() {
    let auth_service = Arc::new(AuthService::new("test-secret-key"));
    let app = create_auth_router(auth_service);

    // Try to register with weak password (less than 8 characters)
    let register_body = json!({
        "username": "weakpass",
        "email": "weak@example.com",
        "password": "short",
        "role": "Write"
    });

    let response = app
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

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}
