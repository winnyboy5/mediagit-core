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

//! DC-7/D4: the two endpoints a client uses to escrow its repository key with
//! the server, and to get it back.
//!
//! # Why this exists
//!
//! On push the client PUTs object bytes straight to the object store over
//! presigned URLs — bytes the server never sees. For an encrypted repository
//! that means the server ends up holding objects it cannot verify, register or
//! ever hand back correctly, which is why `mediagit push` refused outright
//! until this existed.
//!
//! # Authorisation
//!
//! Fetching the key requires `repo:read`, the same grant that lets a caller
//! read the repository at all. That is deliberate and it is the whole security
//! boundary of the feature: the threat model is a compromised **object store**,
//! not a compromised server or a compromised grant. Making key access a
//! separate permission would produce a reader who can pull every object and
//! decrypt none of them, which is not a useful state for anyone.
//!
//! Uploading requires `repo:write`.

use axum::Json;
use axum::body::Bytes;
use axum::extract::{Extension, Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use mediagit_security::auth::AuthUser;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use crate::encryption;
use crate::handlers::check_permission;
use crate::state::AppState;

/// A repository key in transit.
///
/// Hex rather than raw bytes so the payload survives any proxy that decides a
/// body is text, and so a mis-sent body fails to parse instead of being stored
/// as a key nobody can reproduce.
#[derive(Debug, Serialize, Deserialize)]
pub struct RepoKeyPayload {
    /// The 32-byte repository key, hex-encoded.
    pub key: String,
}

/// Resolve the master key, or report that this server does not do escrow.
///
/// A server with encryption switched off answers 404 — the same answer an
/// older server gives for an unknown route. The client cannot tell the two
/// apart, and does not need to: in both cases the honest message is "this
/// remote does not support encrypted repositories".
fn master_or_not_found(
    state: &AppState,
) -> Result<&mediagit_security::encryption::EncryptionKey, StatusCode> {
    state
        .encryption_master
        .as_ref()
        .ok_or(StatusCode::NOT_FOUND)
}

/// Locate a repository on disk, 404 if it is not there.
fn repo_path_or_not_found(state: &AppState, repo: &str) -> Result<std::path::PathBuf, StatusCode> {
    let path = state.repos_dir.join(repo);
    if !path.exists() {
        return Err(StatusCode::NOT_FOUND);
    }
    Ok(path)
}

/// `PUT /:repo/encryption-key` — escrow this repository's key.
///
/// Idempotent on the same key, so a client that cannot tell whether its first
/// attempt landed may safely retry. A *different* key is `409 Conflict` and
/// changes nothing: every object already stored is sealed under the key on
/// disk, nothing records which key an object used, and replacing it would
/// orphan the repository silently and completely.
pub async fn put_encryption_key(
    Path(repo): Path<String>,
    State(state): State<Arc<AppState>>,
    auth_user: Option<Extension<AuthUser>>,
    body: Bytes,
) -> Result<impl IntoResponse, StatusCode> {
    check_permission(
        auth_user.as_deref(),
        "repo:write",
        state.is_auth_enabled(),
        &state.grants,
        &repo,
    )?;

    let master = master_or_not_found(&state)?;
    let repo_path = repo_path_or_not_found(&state, &repo)?;

    let payload: RepoKeyPayload = serde_json::from_slice(&body).map_err(|e| {
        tracing::warn!(repo = %repo, error = %e, "Malformed escrow payload");
        StatusCode::BAD_REQUEST
    })?;
    let key_bytes = hex::decode(payload.key.trim()).map_err(|_| {
        tracing::warn!(repo = %repo, "Escrowed key is not valid hex");
        StatusCode::BAD_REQUEST
    })?;

    match encryption::store_repo_key(&repo_path, master, &key_bytes) {
        Ok(()) => {
            tracing::info!(repo = %repo, "Repository key escrowed");
            Ok(StatusCode::NO_CONTENT)
        }
        Err(e) => {
            let msg = e.to_string();
            if msg.contains("already has a different escrowed key") {
                // Not an internal error and not the client's fault in any way
                // it can retry out of: the server is refusing to destroy data.
                tracing::warn!(repo = %repo, "Refused to replace an escrowed key");
                return Err(StatusCode::CONFLICT);
            }
            if msg.contains("must be") {
                return Err(StatusCode::BAD_REQUEST);
            }
            tracing::error!(repo = %repo, error = %msg, "Failed to escrow repository key");
            Err(StatusCode::INTERNAL_SERVER_ERROR)
        }
    }
}

/// `GET /:repo/encryption-key` — fetch this repository's escrowed key.
///
/// Requires `repo:read`. 404 when the repository is not encrypted, which is
/// also what an unencrypted repository looks like from the client's side —
/// there is nothing to hand back and nothing to say.
pub async fn get_encryption_key(
    Path(repo): Path<String>,
    State(state): State<Arc<AppState>>,
    auth_user: Option<Extension<AuthUser>>,
) -> Result<impl IntoResponse, StatusCode> {
    check_permission(
        auth_user.as_deref(),
        "repo:read",
        state.is_auth_enabled(),
        &state.grants,
        &repo,
    )?;

    let master = master_or_not_found(&state)?;
    let repo_path = repo_path_or_not_found(&state, &repo)?;

    let key = encryption::load_repo_key_bytes(&repo_path, master)
        .map_err(|e| {
            tracing::error!(repo = %repo, error = %e, "Failed to unwrap escrowed key");
            StatusCode::INTERNAL_SERVER_ERROR
        })?
        .ok_or(StatusCode::NOT_FOUND)?;

    tracing::info!(repo = %repo, "Repository key released to an authorised client");
    Ok(Json(RepoKeyPayload {
        key: hex::encode(&*key),
    }))
}
