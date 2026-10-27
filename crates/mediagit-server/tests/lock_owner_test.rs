// SPDX-License-Identifier: BUSL-1.1
// Copyright (C) 2025-2026 Aswin Krishnamoorthy

//! Releasing a lock as its owner on a server without auth.
//!
//! Without auth, `create_lock` records the client-supplied owner name, but
//! `delete_lock` never had an identity to compare against, so every plain
//! unlock was `403` and only `?force=1` worked: nobody could release their own
//! lock as its owner. Unlock now compares the client-supplied `owner` the same
//! way create trusts it, and only when auth is off. With auth on, the supplied
//! owner is ignored (see `admin_routes_test::supplied_owner_is_ignored_with_auth`).

use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use mediagit_server::locks::{CreateLockOutcome, create_lock};
use mediagit_server::{AppState, create_router};
use std::sync::Arc;
use tempfile::TempDir;
use tower::util::ServiceExt;

async fn no_auth_state_with_lock(owner: &str) -> (Arc<AppState>, String, TempDir) {
    let dir = TempDir::new().unwrap();
    std::fs::create_dir_all(dir.path().join("repoA")).unwrap();
    let state = Arc::new(AppState::new(dir.path().to_path_buf()));
    let repo_path = state.repos_dir.join("repoA");
    let CreateLockOutcome::Created(record) = create_lock(
        &state,
        "repoA",
        &repo_path,
        "art/hero.psd".to_string(),
        owner.to_string(),
    )
    .await
    .unwrap() else {
        panic!("lock should be created");
    };
    (state, record.lock_id, dir)
}

async fn delete(state: &Arc<AppState>, uri: &str) -> StatusCode {
    create_router(Arc::clone(state))
        .oneshot(
            Request::builder()
                .uri(uri)
                .method("DELETE")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap()
        .status()
}

#[tokio::test]
async fn owner_can_release_own_lock_without_auth() {
    let (state, id, _dir) = no_auth_state_with_lock("alice").await;
    let uri = format!("/repoA/locks/{id}?owner=alice");
    assert_eq!(delete(&state, &uri).await, StatusCode::NO_CONTENT);
}

#[tokio::test]
async fn other_names_and_missing_owner_are_refused_without_auth() {
    let (state, id, _dir) = no_auth_state_with_lock("alice").await;
    for uri in [
        format!("/repoA/locks/{id}?owner=bob"),
        format!("/repoA/locks/{id}?owner="),
        format!("/repoA/locks/{id}"),
    ] {
        assert_eq!(delete(&state, &uri).await, StatusCode::FORBIDDEN, "{uri}");
    }
    // force still releases it, as before.
    let uri = format!("/repoA/locks/{id}?force=1");
    assert_eq!(delete(&state, &uri).await, StatusCode::NO_CONTENT);
}
