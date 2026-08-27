// SPDX-License-Identifier: BUSL-1.1
// Copyright (C) 2025-2026 Aswin Krishnamoorthy

//! DC-7/D4 key escrow, over the actual router.
//!
//! These go through `create_router` rather than calling the handlers directly,
//! because the things most likely to break are the wiring, not the bodies: a
//! route registered on the wrong method, a permission checked against the wrong
//! grant, a 404 that should have been a 409.

use std::sync::Arc;

use axum::body::{Body, to_bytes};
use axum::http::{Method, Request, StatusCode};
use mediagit_security::encryption::EncryptionKey;
use mediagit_server::{AppState, create_router};
use tempfile::TempDir;
use tower::util::ServiceExt;

const KEY_LEN: usize = 32;

/// A server with encryption on, one repository present, and auth off.
///
/// Auth off is the honest default for these: `check_permission` short-circuits
/// to allow when auth is disabled, so what these tests actually pin is the
/// route wiring and the escrow semantics. The permission *strings* are covered
/// by `admin_routes_test`'s grant machinery, which is where that belongs.
fn app_with_encryption(dir: &TempDir, repo: &str, master_byte: u8) -> axum::Router {
    let repos_dir = dir.path().join("repos");
    std::fs::create_dir_all(repos_dir.join(repo).join(".mediagit")).unwrap();
    let master = EncryptionKey::from_bytes(vec![master_byte; KEY_LEN]).unwrap();
    let state = Arc::new(AppState::new(repos_dir).with_encryption_master(Some(master)));
    create_router(state)
}

/// A server with encryption switched off — the default everywhere.
fn app_without_encryption(dir: &TempDir, repo: &str) -> axum::Router {
    let repos_dir = dir.path().join("repos");
    std::fs::create_dir_all(repos_dir.join(repo).join(".mediagit")).unwrap();
    create_router(Arc::new(AppState::new(repos_dir)))
}

fn put_key(repo: &str, key_hex: &str) -> Request<Body> {
    Request::builder()
        .method(Method::PUT)
        .uri(format!("/{repo}/encryption-key"))
        .header("content-type", "application/json")
        .body(Body::from(format!(r#"{{"key":"{key_hex}"}}"#)))
        .unwrap()
}

fn get_key(repo: &str) -> Request<Body> {
    Request::builder()
        .method(Method::GET)
        .uri(format!("/{repo}/encryption-key"))
        .body(Body::empty())
        .unwrap()
}

#[tokio::test]
async fn a_key_escrowed_then_fetched_comes_back_identical() {
    let dir = TempDir::new().unwrap();
    let key_hex = hex::encode([0x5cu8; KEY_LEN]);

    let app = app_with_encryption(&dir, "demo", 0x11);
    let resp = app.oneshot(put_key("demo", &key_hex)).await.unwrap();
    assert_eq!(resp.status(), StatusCode::NO_CONTENT);

    let app = app_with_encryption(&dir, "demo", 0x11);
    let resp = app.oneshot(get_key("demo")).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let body = to_bytes(resp.into_body(), 4096).await.unwrap();
    let payload: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(
        payload["key"].as_str().unwrap(),
        key_hex,
        "the key handed back must be the key escrowed, byte for byte"
    );
}

#[tokio::test]
async fn re_escrowing_the_same_key_succeeds_but_a_different_one_conflicts() {
    let dir = TempDir::new().unwrap();
    let first = hex::encode([0x01u8; KEY_LEN]);
    let second = hex::encode([0x02u8; KEY_LEN]);

    let app = app_with_encryption(&dir, "demo", 0x22);
    assert_eq!(
        app.oneshot(put_key("demo", &first)).await.unwrap().status(),
        StatusCode::NO_CONTENT
    );

    // A client that cannot tell whether its upload landed must be able to retry.
    let app = app_with_encryption(&dir, "demo", 0x22);
    assert_eq!(
        app.oneshot(put_key("demo", &first)).await.unwrap().status(),
        StatusCode::NO_CONTENT
    );

    // A different key would orphan every object already sealed under the first.
    let app = app_with_encryption(&dir, "demo", 0x22);
    assert_eq!(
        app.oneshot(put_key("demo", &second))
            .await
            .unwrap()
            .status(),
        StatusCode::CONFLICT
    );

    // And the original survived the refusal.
    let app = app_with_encryption(&dir, "demo", 0x22);
    let resp = app.oneshot(get_key("demo")).await.unwrap();
    let body = to_bytes(resp.into_body(), 4096).await.unwrap();
    let payload: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(payload["key"].as_str().unwrap(), first);
}

#[tokio::test]
async fn a_server_with_encryption_off_has_no_escrow_endpoint() {
    let dir = TempDir::new().unwrap();
    let key_hex = hex::encode([0x09u8; KEY_LEN]);

    // 404, not 500 and not a silent success: the client must be able to tell
    // "this remote does not do encrypted repositories" and say so, and an old
    // server without the route at all answers exactly the same way.
    let app = app_without_encryption(&dir, "demo");
    assert_eq!(
        app.oneshot(put_key("demo", &key_hex))
            .await
            .unwrap()
            .status(),
        StatusCode::NOT_FOUND
    );

    let app = app_without_encryption(&dir, "demo");
    assert_eq!(
        app.oneshot(get_key("demo")).await.unwrap().status(),
        StatusCode::NOT_FOUND
    );
}

#[tokio::test]
async fn an_unencrypted_repository_has_no_key_to_fetch() {
    let dir = TempDir::new().unwrap();
    let app = app_with_encryption(&dir, "demo", 0x33);
    assert_eq!(
        app.oneshot(get_key("demo")).await.unwrap().status(),
        StatusCode::NOT_FOUND
    );
}

#[tokio::test]
async fn an_unknown_repository_is_not_found() {
    let dir = TempDir::new().unwrap();
    let key_hex = hex::encode([0x44u8; KEY_LEN]);

    let app = app_with_encryption(&dir, "demo", 0x44);
    assert_eq!(
        app.oneshot(put_key("nope", &key_hex))
            .await
            .unwrap()
            .status(),
        StatusCode::NOT_FOUND
    );
}

#[tokio::test]
async fn a_malformed_key_is_rejected_not_stored() {
    let dir = TempDir::new().unwrap();

    // Not hex.
    let app = app_with_encryption(&dir, "demo", 0x55);
    assert_eq!(
        app.oneshot(put_key("demo", "zzzz")).await.unwrap().status(),
        StatusCode::BAD_REQUEST
    );

    // Valid hex, wrong length. This one matters: a short key stored without
    // complaint would be used to seal objects and could never be reproduced.
    let app = app_with_encryption(&dir, "demo", 0x55);
    assert_eq!(
        app.oneshot(put_key("demo", "aabbcc"))
            .await
            .unwrap()
            .status(),
        StatusCode::BAD_REQUEST
    );

    // Nothing was stored by either attempt.
    let app = app_with_encryption(&dir, "demo", 0x55);
    assert_eq!(
        app.oneshot(get_key("demo")).await.unwrap().status(),
        StatusCode::NOT_FOUND
    );
}

#[tokio::test]
async fn a_key_escrowed_under_one_master_is_unreadable_under_another() {
    let dir = TempDir::new().unwrap();
    let key_hex = hex::encode([0x66u8; KEY_LEN]);

    let app = app_with_encryption(&dir, "demo", 0xaa);
    assert_eq!(
        app.oneshot(put_key("demo", &key_hex))
            .await
            .unwrap()
            .status(),
        StatusCode::NO_CONTENT
    );

    // Same repository directory, different server master key — as if the
    // operator rotated or restored the wrong one. This must fail loudly rather
    // than hand back 32 bytes of garbage that would "decrypt" every object.
    let app = app_with_encryption(&dir, "demo", 0xbb);
    let status = app.oneshot(get_key("demo")).await.unwrap().status();
    assert_eq!(
        status,
        StatusCode::INTERNAL_SERVER_ERROR,
        "a wrong server master must fail closed, got {status}"
    );
}
