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

//! Integration tests for the H3 admin surface: GET/DELETE /auth/users,
//! POST/DELETE /auth/users/{id}/grants, GET/DELETE /auth/keys.
//!
//! Every route is gated on the flat `user:manage` permission (Admin role
//! only) — each gets a 403-for-Write-role and 401-unauthenticated check,
//! plus a happy-path exercise of the actual behavior.

use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use mediagit_security::auth::{ApiKeyAuth, JwtAuth};
use mediagit_server::{create_router, AppState};
use std::sync::Arc;
use tempfile::TempDir;
use tower::util::ServiceExt;

/// Admin/write JWTs plus the live `AppState` so tests can inspect store
/// state directly (e.g. `state.grants.get(...)`) alongside HTTP calls.
fn test_state_with_tokens() -> (Arc<AppState>, String, String) {
    let temp_dir = TempDir::new().unwrap();
    let repos_dir = temp_dir.path().to_path_buf();
    let api_key_auth = Arc::new(ApiKeyAuth::new());
    let jwt_secret = "test-secret-key-for-admin-tests";

    let state = Arc::new(AppState::new_with_auth(
        repos_dir,
        jwt_secret,
        api_key_auth,
    ));

    let jwt_auth = JwtAuth::new(jwt_secret);
    let admin_token = jwt_auth
        .generate_token(
            "admin-user",
            vec![
                "repo:read".to_string(),
                "repo:write".to_string(),
                "repo:admin".to_string(),
                "user:manage".to_string(),
            ],
        )
        .unwrap();
    let write_token = jwt_auth
        .generate_token(
            "write-user",
            vec!["repo:read".to_string(), "repo:write".to_string()],
        )
        .unwrap();

    (state, admin_token, write_token)
}

fn get(uri: &str, token: Option<&str>) -> Request<Body> {
    let mut b = Request::builder().uri(uri).method("GET");
    if let Some(t) = token {
        b = b.header("Authorization", format!("Bearer {}", t));
    }
    b.body(Body::empty()).unwrap()
}

fn delete_req(uri: &str, token: Option<&str>, body: Option<&str>) -> Request<Body> {
    let mut b = Request::builder().uri(uri).method("DELETE");
    if let Some(t) = token {
        b = b.header("Authorization", format!("Bearer {}", t));
    }
    let body = match body {
        Some(json) => {
            b = b.header("Content-Type", "application/json");
            Body::from(json.to_string())
        }
        None => Body::empty(),
    };
    b.body(body).unwrap()
}

fn post_json(uri: &str, token: Option<&str>, body: &str) -> Request<Body> {
    let mut b = Request::builder()
        .uri(uri)
        .method("POST")
        .header("Content-Type", "application/json");
    if let Some(t) = token {
        b = b.header("Authorization", format!("Bearer {}", t));
    }
    b.body(Body::from(body.to_string())).unwrap()
}

// ---- 401 unauthenticated ----

#[tokio::test]
async fn list_users_401_unauthenticated() {
    let (state, _admin, _write) = test_state_with_tokens();
    let app = create_router(state);
    let resp = app.oneshot(get("/auth/users", None)).await.unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn delete_user_401_unauthenticated() {
    let (state, _admin, _write) = test_state_with_tokens();
    let app = create_router(state);
    let resp = app
        .oneshot(delete_req("/auth/users/someone", None, None))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn upsert_grant_401_unauthenticated() {
    let (state, _admin, _write) = test_state_with_tokens();
    let app = create_router(state);
    let resp = app
        .oneshot(post_json(
            "/auth/users/someone/grants",
            None,
            r#"{"repo":"repoA","level":"write"}"#,
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn list_keys_401_unauthenticated() {
    let (state, _admin, _write) = test_state_with_tokens();
    let app = create_router(state);
    let resp = app.oneshot(get("/auth/keys", None)).await.unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn revoke_key_401_unauthenticated() {
    let (state, _admin, _write) = test_state_with_tokens();
    let app = create_router(state);
    let resp = app
        .oneshot(delete_req("/auth/keys/ak_someid", None, None))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

// ---- 403 for a Write-role (non-admin) user ----

#[tokio::test]
async fn list_users_403_for_write_role() {
    let (state, _admin, write) = test_state_with_tokens();
    let app = create_router(state);
    let resp = app.oneshot(get("/auth/users", Some(&write))).await.unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn delete_user_403_for_write_role() {
    let (state, _admin, write) = test_state_with_tokens();
    let app = create_router(state);
    let resp = app
        .oneshot(delete_req("/auth/users/someone", Some(&write), None))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn upsert_grant_403_for_write_role() {
    let (state, _admin, write) = test_state_with_tokens();
    let app = create_router(state);
    let resp = app
        .oneshot(post_json(
            "/auth/users/someone/grants",
            Some(&write),
            r#"{"repo":"repoA","level":"write"}"#,
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn list_keys_403_for_write_role() {
    let (state, _admin, write) = test_state_with_tokens();
    let app = create_router(state);
    let resp = app.oneshot(get("/auth/keys", Some(&write))).await.unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn revoke_key_403_for_write_role() {
    let (state, _admin, write) = test_state_with_tokens();
    let app = create_router(state);
    let resp = app
        .oneshot(delete_req("/auth/keys/ak_someid", Some(&write), None))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
}

// ---- happy paths ----

#[tokio::test]
async fn grant_upsert_then_removed() {
    let (state, admin, _write) = test_state_with_tokens();
    let app = create_router(Arc::clone(&state));

    let resp = app
        .clone()
        .oneshot(post_json(
            "/auth/users/target-user/grants",
            Some(&admin),
            r#"{"repo":"repoA","level":"write"}"#,
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NO_CONTENT);
    assert_eq!(
        state.grants.get("target-user", "repoA"),
        Some(mediagit_security::auth::GrantLevel::Write)
    );

    let resp = app
        .oneshot(delete_req(
            "/auth/users/target-user/grants",
            Some(&admin),
            Some(r#"{"repo":"repoA"}"#),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NO_CONTENT);
    assert_eq!(state.grants.get("target-user", "repoA"), None);
}

#[tokio::test]
async fn delete_user_cascades_grants() {
    let (state, admin, _write) = test_state_with_tokens();

    let user = mediagit_security::auth::User::new(
        "victim".to_string(),
        "victim".to_string(),
        "victim@example.com".to_string(),
        mediagit_security::auth::user::Role::Write,
    );
    state
        .auth_service()
        .unwrap()
        .credentials_store
        .register_user(user, "password123")
        .await
        .unwrap();
    state.grants.grant("victim", "repoA", mediagit_security::auth::GrantLevel::Read).await.unwrap();
    assert_eq!(
        state.grants.get("victim", "repoA"),
        Some(mediagit_security::auth::GrantLevel::Read)
    );

    let app = create_router(Arc::clone(&state));
    let resp = app
        .oneshot(delete_req("/auth/users/victim", Some(&admin), None))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NO_CONTENT);

    assert!(state
        .auth_service()
        .unwrap()
        .credentials_store
        .get_user("victim")
        .await
        .is_err());
    assert_eq!(state.grants.get("victim", "repoA"), None);
}

#[tokio::test]
async fn list_users_returns_id_username_role_no_secrets() {
    let (state, admin, _write) = test_state_with_tokens();

    let user = mediagit_security::auth::User::new(
        "u1".to_string(),
        "alice".to_string(),
        "alice@example.com".to_string(),
        mediagit_security::auth::user::Role::Read,
    );
    state
        .auth_service()
        .unwrap()
        .credentials_store
        .register_user(user, "password123")
        .await
        .unwrap();

    let app = create_router(Arc::clone(&state));
    let resp = app
        .oneshot(get("/auth/users", Some(&admin)))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    let arr = body.as_array().unwrap();
    assert_eq!(arr.len(), 1);
    assert_eq!(arr[0]["id"], "u1");
    assert_eq!(arr[0]["username"], "alice");
    assert_eq!(arr[0]["role"], "Read");
    // Never leak a password hash or any credential material.
    assert!(arr[0].get("password_hash").is_none());
    assert!(arr[0].get("email").is_none());
}

#[tokio::test]
async fn revoke_key_removes_auth() {
    let (state, admin, _write) = test_state_with_tokens();

    let repo_path = state.repos_dir.join("test-repo");
    std::fs::create_dir_all(&repo_path).unwrap();

    let auth_layer = state.auth().unwrap();
    let (plaintext_key, api_key) = auth_layer
        .api_key_auth()
        .generate_key(
            "keyed-user".to_string(),
            "test-key".to_string(),
            vec!["repo:read".to_string()],
        )
        .await
        .unwrap();

    let app = create_router(Arc::clone(&state));

    // Key works before revocation.
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/test-repo/info/refs")
                .header("X-API-Key", plaintext_key.clone())
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    // Admin revokes it.
    let resp = app
        .clone()
        .oneshot(delete_req(
            &format!("/auth/keys/{}", api_key.id),
            Some(&admin),
            None,
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NO_CONTENT);

    // Key no longer authenticates.
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/test-repo/info/refs")
                .header("X-API-Key", plaintext_key)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn list_keys_returns_metadata_only() {
    let (state, admin, _write) = test_state_with_tokens();

    let auth_layer = state.auth().unwrap();
    auth_layer
        .api_key_auth()
        .generate_key(
            "keyed-user".to_string(),
            "my-key".to_string(),
            vec!["repo:read".to_string()],
        )
        .await
        .unwrap();

    let app = create_router(Arc::clone(&state));
    let resp = app
        .oneshot(get("/auth/keys", Some(&admin)))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    let arr = body.as_array().unwrap();
    assert_eq!(arr.len(), 1);
    assert_eq!(arr[0]["name"], "my-key");
    assert_eq!(arr[0]["user_id"], "keyed-user");
    assert!(arr[0]["id"].is_string());
    assert!(arr[0].get("key_hash").is_none());
}
