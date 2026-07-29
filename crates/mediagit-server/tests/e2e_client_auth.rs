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

//! End-to-end client-auth tests (M2 Step 1): real `ProtocolClient` against a
//! real (in-process, real TCP) server, both with auth enabled and disabled.
//!
//! Covers the three cells the roadmap's M2 gate calls for:
//!   - authed server + correctly-credentialed client -> succeeds
//!   - authed server + wrong/no credentials -> fails cleanly (401)
//!   - authless server + credentialed client -> still works (server ignores
//!     the header) — this is the mirror of the authless-regression guard
//!     tested at the protocol-unit level in mediagit-protocol.

use mediagit_protocol::{Credentials, ProtocolClient};
use mediagit_security::auth::{ApiKeyAuth, JwtAuth};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::net::TcpListener;

async fn start_authless_server(repos_dir: PathBuf) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let state = Arc::new(mediagit_server::AppState::new(repos_dir));
    let app = mediagit_server::create_router(state);
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
    format!("http://{}", addr)
}

/// Returns (base_url, jwt_secret, api_key_auth) so callers can mint tokens/keys.
async fn start_authed_server(repos_dir: PathBuf) -> (String, String, Arc<ApiKeyAuth>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let jwt_secret = "e2e-client-auth-test-secret".to_string();
    let api_key_auth = Arc::new(ApiKeyAuth::new());
    let state = Arc::new(mediagit_server::AppState::new_with_auth(
        repos_dir,
        &jwt_secret,
        api_key_auth.clone(),
    ));

    // AU-2: permissions are re-derived from the live user store per request,
    // so tokens and API keys are only honoured while their account exists.
    // Register the identities these tests authenticate as.
    for (id, role) in [
        ("test-user", mediagit_security::auth::user::Role::Read),
        ("e2e-user", mediagit_security::auth::user::Role::Write),
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
            .register_user(user, "render farm quiet hum")
            .await
            .unwrap();
    }

    let app = mediagit_server::create_router(state);
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
    (format!("http://{}", addr), jwt_secret, api_key_auth)
}

#[tokio::test]
async fn authed_server_accepts_correct_bearer_token() {
    let repos_root = tempfile::TempDir::new().unwrap();
    let (base_url, jwt_secret, _api_key_auth) =
        start_authed_server(repos_root.path().to_path_buf()).await;

    let jwt_auth = JwtAuth::new(&jwt_secret);
    let token = jwt_auth
        .generate_token("test-user", vec!["repo:read".to_string()])
        .unwrap();

    std::fs::create_dir_all(repos_root.path().join("authed-repo")).unwrap();

    let client = ProtocolClient::new(format!("{}/authed-repo", base_url))
        .with_credentials(Credentials::Bearer(token));

    let refs = client.get_refs().await;
    assert!(
        refs.is_ok(),
        "authed client with valid token should succeed: {refs:?}"
    );
}

#[tokio::test]
async fn authed_server_rejects_wrong_token() {
    let repos_root = tempfile::TempDir::new().unwrap();
    let (base_url, _jwt_secret, _api_key_auth) =
        start_authed_server(repos_root.path().to_path_buf()).await;

    std::fs::create_dir_all(repos_root.path().join("authed-repo2")).unwrap();

    let client = ProtocolClient::new(format!("{}/authed-repo2", base_url))
        .with_credentials(Credentials::Bearer("not-a-real-token".to_string()));

    let refs = client.get_refs().await;
    assert!(refs.is_err(), "wrong token must fail cleanly, not succeed");
}

#[tokio::test]
async fn authed_server_rejects_no_credentials() {
    let repos_root = tempfile::TempDir::new().unwrap();
    let (base_url, _jwt_secret, _api_key_auth) =
        start_authed_server(repos_root.path().to_path_buf()).await;

    std::fs::create_dir_all(repos_root.path().join("authed-repo3")).unwrap();

    let client = ProtocolClient::new(format!("{}/authed-repo3", base_url));
    // Credentials::None by default.

    let refs = client.get_refs().await;
    assert!(
        refs.is_err(),
        "no credentials against an authed server must fail cleanly"
    );
}

#[tokio::test]
async fn authed_server_accepts_correct_api_key() {
    let repos_root = tempfile::TempDir::new().unwrap();
    let (base_url, _jwt_secret, api_key_auth) =
        start_authed_server(repos_root.path().to_path_buf()).await;

    let (plaintext_key, _info) = api_key_auth
        .generate_key(
            "test-user".to_string(),
            "e2e test key".to_string(),
            vec!["repo:read".to_string()],
        )
        .await
        .unwrap();

    std::fs::create_dir_all(repos_root.path().join("authed-repo4")).unwrap();

    let client = ProtocolClient::new(format!("{}/authed-repo4", base_url))
        .with_credentials(Credentials::ApiKey(plaintext_key));

    let refs = client.get_refs().await;
    assert!(
        refs.is_ok(),
        "authed client with valid API key should succeed: {refs:?}"
    );
}

/// Authless-regression guard, server side: a client that happens to carry
/// credentials (e.g. env vars set globally on a dev machine) must not be
/// rejected or otherwise behave differently against an authless server —
/// the server has no auth middleware installed at all when auth is
/// disabled, so the extra header is simply ignored.
#[tokio::test]
async fn authless_server_works_with_credentialed_client() {
    let repos_root = tempfile::TempDir::new().unwrap();
    let base_url = start_authless_server(repos_root.path().to_path_buf()).await;

    std::fs::create_dir_all(repos_root.path().join("authless-repo")).unwrap();

    let client = ProtocolClient::new(format!("{}/authless-repo", base_url))
        .with_credentials(Credentials::Bearer("some-token-nobody-checks".to_string()));

    let refs = client.get_refs().await;
    assert!(
        refs.is_ok(),
        "authless server must ignore a client-sent token, not reject it: {refs:?}"
    );
}

/// Authless-regression guard, no-credentials case: old client behavior
/// against an authless server is completely unchanged.
#[tokio::test]
async fn authless_server_works_with_no_credentials() {
    let repos_root = tempfile::TempDir::new().unwrap();
    let base_url = start_authless_server(repos_root.path().to_path_buf()).await;

    std::fs::create_dir_all(repos_root.path().join("authless-repo2")).unwrap();

    let client = ProtocolClient::new(format!("{}/authless-repo2", base_url));
    let refs = client.get_refs().await;
    assert!(
        refs.is_ok(),
        "authless server + no credentials should work exactly as before: {refs:?}"
    );
}

/// A bare `reqwest` client, for the two tests below that assert on the HTTP
/// status directly rather than through `ProtocolClient`.
///
/// The provider install is what `ProtocolClient` does on construction; without
/// it, whether these tests pass depends on whether some other test in this
/// binary happened to build a `ProtocolClient` first.
fn raw_client() -> reqwest::Client {
    let _ = rustls::crypto::ring::default_provider().install_default();
    reqwest::Client::new()
}

/// A push refused for lack of permission must arrive as a readable 403.
///
/// `upload_pack` streams the body, so returning early from the handler dropped
/// it unread; hyper then reset the connection and the client — still writing —
/// saw "An existing connection was forcibly closed by the remote host" instead
/// of the status. QA phase 07 reported that as an unexplained network failure
/// on every read-role push. The body must be large enough that the client is
/// still writing when the server decides, or the reset never happens.
#[tokio::test]
async fn read_role_push_is_refused_with_a_status_not_a_connection_reset() {
    let repos_root = tempfile::TempDir::new().unwrap();
    let (base_url, jwt_secret, _api_key_auth) =
        start_authed_server(repos_root.path().to_path_buf()).await;

    // test-user is Role::Read, so repo:write is refused.
    let token = JwtAuth::new(&jwt_secret)
        .generate_token("test-user", vec!["repo:read".to_string()])
        .unwrap();

    std::fs::create_dir_all(repos_root.path().join("refused-repo")).unwrap();

    let response = raw_client()
        .post(format!("{base_url}/refused-repo/objects/pack"))
        .header("Authorization", format!("Bearer {token}"))
        .header("Content-Type", "application/octet-stream")
        .body(vec![0u8; 4 * 1024 * 1024])
        .send()
        .await
        .expect("the refusal must come back as an HTTP response, not a transport error");

    assert_eq!(
        response.status(),
        reqwest::StatusCode::FORBIDDEN,
        "a read-role account must be told it lacks push permission"
    );
}

/// Same guard for the repo-not-found path, which returns before reading the
/// body for the same reason.
#[tokio::test]
async fn push_to_missing_repo_is_refused_with_a_status_not_a_connection_reset() {
    let repos_root = tempfile::TempDir::new().unwrap();
    let base_url = start_authless_server(repos_root.path().to_path_buf()).await;

    let response = raw_client()
        .post(format!("{base_url}/no-such-repo/objects/pack"))
        .header("Content-Type", "application/octet-stream")
        .body(vec![0u8; 4 * 1024 * 1024])
        .send()
        .await
        .expect("404 must come back as an HTTP response, not a transport error");

    assert_eq!(response.status(), reqwest::StatusCode::NOT_FOUND);
}
