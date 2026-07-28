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
use mediagit_server::{AppState, create_router};
use std::sync::Arc;
use tempfile::TempDir;
use tower::util::ServiceExt;

/// Admin/write JWTs plus the live `AppState` so tests can inspect store
/// state directly (e.g. `state.grants.get(...)`) alongside HTTP calls.
/// AU-2: the accounts these tokens name must actually exist.
///
/// Permissions are now re-derived from the live user store on every request
/// rather than trusted from the token, so a token naming an unregistered user
/// is rejected as `401` — correctly, since that is indistinguishable from a
/// token for a deleted account. These helpers previously minted tokens out of
/// thin air, which no longer reflects how authentication works.
async fn test_state_with_tokens() -> (Arc<AppState>, String, String) {
    let temp_dir = TempDir::new().unwrap();
    let repos_dir = temp_dir.path().to_path_buf();
    let api_key_auth = Arc::new(ApiKeyAuth::new());
    let jwt_secret = "test-secret-key-for-admin-tests";

    let state = Arc::new(AppState::new_with_auth(repos_dir, jwt_secret, api_key_auth));

    for (id, role) in [
        ("admin-user", mediagit_security::auth::user::Role::Admin),
        ("write-user", mediagit_security::auth::user::Role::Write),
    ] {
        let user = mediagit_security::auth::User::new(
            id.to_string(),
            id.to_string(),
            format!("{id}@example.com"),
            role,
        );
        state
            .auth_service()
            .unwrap()
            .credentials_store
            .register_user(user, "password123")
            .await
            .unwrap();
    }

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

fn patch_json(uri: &str, token: Option<&str>, body: &str) -> Request<Body> {
    let mut b = Request::builder()
        .uri(uri)
        .method("PATCH")
        .header("Content-Type", "application/json");
    if let Some(t) = token {
        b = b.header("Authorization", format!("Bearer {}", t));
    }
    b.body(Body::from(body.to_string())).unwrap()
}

/// Register a real user (with a role) in the live credentials store, distinct
/// from the loose JWT tokens `test_state_with_tokens` mints — needed for any
/// test whose handler looks the target user up by ID (role changes,
/// password changes, key-owner permission lookups, last-admin counting).
async fn register_role_user(
    state: &Arc<AppState>,
    id: &str,
    username: &str,
    role: mediagit_security::auth::user::Role,
) {
    let user = mediagit_security::auth::User::new(
        id.to_string(),
        username.to_string(),
        format!("{username}@example.com"),
        role,
    );
    state
        .auth_service()
        .unwrap()
        .credentials_store
        .register_user(user, "password123")
        .await
        .unwrap();
}

// ---- 401 unauthenticated ----

#[tokio::test]
async fn list_users_401_unauthenticated() {
    let (state, _admin, _write) = test_state_with_tokens().await;
    let app = create_router(state);
    let resp = app.oneshot(get("/auth/users", None)).await.unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn delete_user_401_unauthenticated() {
    let (state, _admin, _write) = test_state_with_tokens().await;
    let app = create_router(state);
    let resp = app
        .oneshot(delete_req("/auth/users/someone", None, None))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn upsert_grant_401_unauthenticated() {
    let (state, _admin, _write) = test_state_with_tokens().await;
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
    let (state, _admin, _write) = test_state_with_tokens().await;
    let app = create_router(state);
    let resp = app.oneshot(get("/auth/keys", None)).await.unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn revoke_key_401_unauthenticated() {
    let (state, _admin, _write) = test_state_with_tokens().await;
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
    let (state, _admin, write) = test_state_with_tokens().await;
    let app = create_router(state);
    let resp = app.oneshot(get("/auth/users", Some(&write))).await.unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn delete_user_403_for_write_role() {
    let (state, _admin, write) = test_state_with_tokens().await;
    let app = create_router(state);
    let resp = app
        .oneshot(delete_req("/auth/users/someone", Some(&write), None))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn upsert_grant_403_for_write_role() {
    let (state, _admin, write) = test_state_with_tokens().await;
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
    let (state, _admin, write) = test_state_with_tokens().await;
    let app = create_router(state);
    let resp = app.oneshot(get("/auth/keys", Some(&write))).await.unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn revoke_key_403_for_write_role() {
    let (state, _admin, write) = test_state_with_tokens().await;
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
    let (state, admin, _write) = test_state_with_tokens().await;
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
    let (state, admin, _write) = test_state_with_tokens().await;

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
    state
        .grants
        .grant("victim", "repoA", mediagit_security::auth::GrantLevel::Read)
        .await
        .unwrap();
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

    assert!(
        state
            .auth_service()
            .unwrap()
            .credentials_store
            .get_user("victim")
            .await
            .is_err()
    );
    assert_eq!(state.grants.get("victim", "repoA"), None);
}

#[tokio::test]
async fn list_users_returns_id_username_role_no_secrets() {
    let (state, admin, _write) = test_state_with_tokens().await;

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
    let resp = app.oneshot(get("/auth/users", Some(&admin))).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    let arr = body.as_array().unwrap();
    // AU-2: the shared helper now registers `admin-user` and `write-user`
    // because their tokens must name real accounts. Assert on the user this
    // test added rather than on the total.
    let alice = arr
        .iter()
        .find(|u| u["id"] == "u1")
        .expect("alice should be listed");
    assert_eq!(alice["username"], "alice");
    assert_eq!(alice["role"], "Read");
    // Never leak a password hash or any credential material.
    assert!(arr[0].get("password_hash").is_none());
    assert!(arr[0].get("email").is_none());
}

#[tokio::test]
async fn revoke_key_removes_auth() {
    let (state, admin, _write) = test_state_with_tokens().await;

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

    // AU-2: an API key is only honoured while its owner exists, so the owner
    // has to be a real account for the pre-revocation request to succeed.
    state
        .auth_service()
        .unwrap()
        .credentials_store
        .register_user(
            mediagit_security::auth::User::new(
                "keyed-user".to_string(),
                "keyed-user".to_string(),
                "keyed@example.com".to_string(),
                mediagit_security::auth::user::Role::Read,
            ),
            "password123",
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
    let (state, admin, _write) = test_state_with_tokens().await;

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
    let resp = app.oneshot(get("/auth/keys", Some(&admin))).await.unwrap();
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

// ---- POST /auth/keys: self-service minting cannot escalate ----

#[tokio::test]
async fn create_key_permission_intersection_cannot_escalate() {
    let (state, _admin, write) = test_state_with_tokens().await;
    register_role_user(
        &state,
        "write-user",
        "writer",
        mediagit_security::auth::user::Role::Write,
    )
    .await;
    let app = create_router(Arc::clone(&state));

    let resp = app
        .oneshot(post_json(
            "/auth/keys",
            Some(&write),
            r#"{"name":"escalate","permissions":["repo:admin","user:manage"]}"#,
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);

    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    let perms = body["permissions"].as_array().unwrap();
    assert!(
        perms.is_empty(),
        "Write user must not be able to mint a repo:admin/user:manage key, got {:?}",
        perms
    );
    assert!(body["key"].is_string());
}

#[tokio::test]
async fn create_key_defaults_to_owner_permissions_when_omitted() {
    let (state, _admin, write) = test_state_with_tokens().await;
    register_role_user(
        &state,
        "write-user",
        "writer",
        mediagit_security::auth::user::Role::Write,
    )
    .await;
    let app = create_router(Arc::clone(&state));

    let resp = app
        .oneshot(post_json(
            "/auth/keys",
            Some(&write),
            r#"{"name":"default"}"#,
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    let perms: Vec<String> = body["permissions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect();
    assert_eq!(
        perms,
        vec!["repo:read".to_string(), "repo:write".to_string()]
    );
}

#[tokio::test]
async fn create_key_for_other_user_requires_admin() {
    let (state, _admin, write) = test_state_with_tokens().await;
    register_role_user(
        &state,
        "write-user",
        "writer",
        mediagit_security::auth::user::Role::Write,
    )
    .await;
    register_role_user(
        &state,
        "victim2",
        "victim2",
        mediagit_security::auth::user::Role::Write,
    )
    .await;
    let app = create_router(Arc::clone(&state));

    let resp = app
        .oneshot(post_json(
            "/auth/keys",
            Some(&write),
            r#"{"name":"x","user_id":"victim2"}"#,
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
}

// ---- DELETE /auth/keys/{id}: ownership-scoped for non-admins ----

#[tokio::test]
async fn revoke_key_own_key_allowed_for_write_role() {
    let (state, _admin, write) = test_state_with_tokens().await;
    let auth_layer = state.auth().unwrap();
    let (_plaintext, api_key) = auth_layer
        .api_key_auth()
        .generate_key("write-user".to_string(), "mine".to_string(), vec![])
        .await
        .unwrap();
    let app = create_router(Arc::clone(&state));

    let resp = app
        .oneshot(delete_req(
            &format!("/auth/keys/{}", api_key.id),
            Some(&write),
            None,
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NO_CONTENT);
}

// ---- PATCH /auth/users/{id}/role ----

#[tokio::test]
async fn set_role_403_for_write_role() {
    let (state, _admin, write) = test_state_with_tokens().await;
    register_role_user(
        &state,
        "target",
        "target",
        mediagit_security::auth::user::Role::Write,
    )
    .await;
    let app = create_router(Arc::clone(&state));

    let resp = app
        .oneshot(patch_json(
            "/auth/users/target/role",
            Some(&write),
            r#"{"role":"Admin"}"#,
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn set_role_last_admin_demotion_refused() {
    // AU-2: the shared helper registers `admin-user` as an Admin (its token
    // must name a real account), so this test walks the count down to one
    // rather than assuming it starts there. That exercises the invariant more
    // thoroughly than the original: the *first* demotion must succeed, and
    // only the one that would leave zero admins is refused.
    let (state, admin, _write) = test_state_with_tokens().await;
    register_role_user(
        &state,
        "second-admin",
        "second",
        mediagit_security::auth::user::Role::Admin,
    )
    .await;
    let app = create_router(Arc::clone(&state));

    // Two admins exist — demoting one is allowed.
    let resp = app
        .clone()
        .oneshot(patch_json(
            "/auth/users/second-admin/role",
            Some(&admin),
            r#"{"role":"Write"}"#,
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    // `admin-user` is now the only admin — demoting it must be refused.
    let resp = app
        .oneshot(patch_json(
            "/auth/users/admin-user/role",
            Some(&admin),
            r#"{"role":"Write"}"#,
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CONFLICT);

    // Role must be unchanged after the refusal.
    assert_eq!(
        state
            .auth_service()
            .unwrap()
            .credentials_store
            .get_user("admin-user")
            .await
            .unwrap()
            .role,
        mediagit_security::auth::user::Role::Admin
    );
}

#[tokio::test]
async fn set_role_demotion_allowed_when_multiple_admins() {
    let (state, admin, _write) = test_state_with_tokens().await;
    register_role_user(
        &state,
        "admin1",
        "admin1",
        mediagit_security::auth::user::Role::Admin,
    )
    .await;
    register_role_user(
        &state,
        "admin2",
        "admin2",
        mediagit_security::auth::user::Role::Admin,
    )
    .await;
    let app = create_router(Arc::clone(&state));

    let resp = app
        .oneshot(patch_json(
            "/auth/users/admin1/role",
            Some(&admin),
            r#"{"role":"Write"}"#,
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(
        state
            .auth_service()
            .unwrap()
            .credentials_store
            .get_user("admin1")
            .await
            .unwrap()
            .role,
        mediagit_security::auth::user::Role::Write
    );
}

// ---- POST /auth/password (self-service) ----

#[tokio::test]
async fn change_password_rejects_wrong_current_password() {
    let (state, _admin, write) = test_state_with_tokens().await;
    register_role_user(
        &state,
        "write-user",
        "writer",
        mediagit_security::auth::user::Role::Write,
    )
    .await;
    let app = create_router(Arc::clone(&state));

    let resp = app
        .oneshot(post_json(
            "/auth/password",
            Some(&write),
            r#"{"current_password":"wrongpw","new_password":"newpassword123"}"#,
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn change_password_succeeds_with_correct_current_password() {
    let (state, _admin, write) = test_state_with_tokens().await;
    register_role_user(
        &state,
        "write-user",
        "writer",
        mediagit_security::auth::user::Role::Write,
    )
    .await;
    let app = create_router(Arc::clone(&state));

    let resp = app
        .oneshot(post_json(
            "/auth/password",
            Some(&write),
            r#"{"current_password":"password123","new_password":"newpassword123"}"#,
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    // Old password no longer authenticates; new one does.
    let auth_service = state.auth_service().unwrap();
    assert!(
        auth_service
            .credentials_store
            .authenticate("writer", "password123")
            .await
            .is_err()
    );
    assert!(
        auth_service
            .credentials_store
            .authenticate("writer", "newpassword123")
            .await
            .is_ok()
    );
}

// ---- Write-role authorization negatives on the remaining admin-only routes ----

#[tokio::test]
async fn write_user_cannot_reset_others_password() {
    let (state, _admin, write) = test_state_with_tokens().await;
    register_role_user(
        &state,
        "victim3",
        "victim3",
        mediagit_security::auth::user::Role::Write,
    )
    .await;
    let app = create_router(Arc::clone(&state));

    let resp = app
        .oneshot(patch_json(
            "/auth/users/victim3/password",
            Some(&write),
            r#"{"new_password":"newpassword123"}"#,
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn write_user_cannot_create_users() {
    let (state, _admin, write) = test_state_with_tokens().await;
    let app = create_router(Arc::clone(&state));

    let resp = app
        .oneshot(post_json(
            "/auth/users",
            Some(&write),
            r#"{"username":"newu","email":"newu@example.com","password":"password123","role":"Write"}"#,
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
}

// ---- Admin happy paths for the new §E routes ----

#[tokio::test]
async fn admin_create_user_succeeds() {
    let (state, admin, _write) = test_state_with_tokens().await;
    let app = create_router(Arc::clone(&state));

    let resp = app
        .oneshot(post_json(
            "/auth/users",
            Some(&admin),
            r#"{"username":"created","email":"created@example.com","password":"password123","role":"Read"}"#,
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);

    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(body["username"], "created");
    assert_eq!(body["role"], "Read");
}

#[tokio::test]
async fn admin_reset_password_recovers_forgotten_password() {
    let (state, admin, _write) = test_state_with_tokens().await;
    register_role_user(
        &state,
        "forgetful",
        "forgetful",
        mediagit_security::auth::user::Role::Write,
    )
    .await;
    let app = create_router(Arc::clone(&state));

    let resp = app
        .oneshot(patch_json(
            "/auth/users/forgetful/password",
            Some(&admin),
            r#"{"new_password":"recoveredpw123"}"#,
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NO_CONTENT);

    assert!(
        state
            .auth_service()
            .unwrap()
            .credentials_store
            .authenticate("forgetful", "recoveredpw123")
            .await
            .is_ok()
    );
}

// ---- GET /auth/whoami ----

#[tokio::test]
async fn whoami_returns_role_and_grants() {
    let (state, _admin, write) = test_state_with_tokens().await;
    register_role_user(
        &state,
        "write-user",
        "writer",
        mediagit_security::auth::user::Role::Write,
    )
    .await;
    state
        .grants
        .grant(
            "write-user",
            "repoA",
            mediagit_security::auth::GrantLevel::Read,
        )
        .await
        .unwrap();
    let app = create_router(Arc::clone(&state));

    let resp = app
        .oneshot(get("/auth/whoami", Some(&write)))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(body["username"], "writer");
    assert_eq!(body["role"], "Write");
    assert_eq!(body["grants"][0]["repo"], "repoA");
    assert_eq!(body["grants"][0]["level"], "read");
}

#[tokio::test]
async fn whoami_401_unauthenticated() {
    let (state, _admin, _write) = test_state_with_tokens().await;
    let app = create_router(state);
    let resp = app.oneshot(get("/auth/whoami", None)).await.unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

/// AU-1: deleting a user must revoke their API keys.
///
/// `delete_user` cleared credentials and grants but never touched the key
/// store — and `ApiKey` carries no expiry, so those keys authenticated
/// indefinitely. Deletion is often *how* a compromised or offboarded account
/// is handled, which made this the gap most likely to be relied upon.
///
/// Note the sibling test `delete_user_cascades_grants` asserts the grant
/// cascade and stops there. The absence of this assertion is why the gap
/// shipped.
#[tokio::test]
async fn delete_user_revokes_api_keys() {
    let (state, admin, _write) = test_state_with_tokens().await;

    let user = mediagit_security::auth::User::new(
        "keyholder".to_string(),
        "keyholder".to_string(),
        "keyholder@example.com".to_string(),
        mediagit_security::auth::user::Role::Write,
    );
    state
        .auth_service()
        .unwrap()
        .credentials_store
        .register_user(user, "password123")
        .await
        .unwrap();

    let keys = state.auth_layer.as_ref().unwrap().api_key_auth();
    keys.generate_key(
        "keyholder".to_string(),
        "ci-token".to_string(),
        vec!["repo:write".to_string()],
    )
    .await
    .unwrap();
    assert_eq!(
        keys.list_user_keys("keyholder").await.len(),
        1,
        "precondition: the user should own one key"
    );

    let app = create_router(Arc::clone(&state));
    let resp = app
        .oneshot(delete_req("/auth/users/keyholder", Some(&admin), None))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NO_CONTENT);

    assert!(
        keys.list_user_keys("keyholder").await.is_empty(),
        "deleted user still owns API keys — they never expire, so this is \
         permanent authenticated access for an account that no longer exists"
    );
}

/// AU-2: a deleted user's token must stop working immediately.
///
/// Permissions came from the token's claims and were never re-checked, so a
/// deleted account kept its access until the JWT expired — up to 24 h. Since
/// deletion is typically how a compromised account is contained, the
/// containment did not actually contain anything.
#[tokio::test]
async fn deleted_user_token_is_rejected_immediately() {
    let (state, admin, write) = test_state_with_tokens().await;

    // The write user's token works while the account exists.
    let app = create_router(Arc::clone(&state));
    let resp = app
        .oneshot(get("/auth/whoami", Some(&write)))
        .await
        .unwrap();
    assert_ne!(
        resp.status(),
        StatusCode::UNAUTHORIZED,
        "precondition: a live user's token should authenticate"
    );

    // Admin deletes them.
    let app = create_router(Arc::clone(&state));
    let resp = app
        .oneshot(delete_req("/auth/users/write-user", Some(&admin), None))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NO_CONTENT);

    // The same, still-unexpired token must now be refused.
    let app = create_router(Arc::clone(&state));
    let resp = app
        .oneshot(get("/auth/whoami", Some(&write)))
        .await
        .unwrap();
    assert_eq!(
        resp.status(),
        StatusCode::UNAUTHORIZED,
        "a deleted user's unexpired token still authenticated — deletion does \
         not revoke access until the token expires"
    );
}

/// AU-2 corollary: demotion takes effect at once, not at token expiry.
#[tokio::test]
async fn demoted_user_loses_admin_rights_immediately() {
    let (state, admin, _write) = test_state_with_tokens().await;
    register_role_user(
        &state,
        "second-admin",
        "second",
        mediagit_security::auth::user::Role::Admin,
    )
    .await;

    // Demote the *token holder* while two admins exist, so the last-admin
    // guard does not intervene.
    let app = create_router(Arc::clone(&state));
    let resp = app
        .oneshot(patch_json(
            "/auth/users/admin-user/role",
            Some(&admin),
            r#"{"role":"Read"}"#,
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    // Their existing admin token must no longer carry admin rights.
    let app = create_router(Arc::clone(&state));
    let resp = app.oneshot(get("/auth/users", Some(&admin))).await.unwrap();
    assert_eq!(
        resp.status(),
        StatusCode::FORBIDDEN,
        "demoted admin retained admin access via a stale token"
    );
}
