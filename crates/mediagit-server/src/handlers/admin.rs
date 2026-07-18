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

//! Minimal admin surface (H3): user, grant, and API-key management.
//!
//! Every handler here is gated on the flat `user:manage` permission (only
//! `Role::Admin` carries it — see `User::permissions`). The `repo` argument
//! `check_permission` normally uses for per-repo grant lookups is irrelevant
//! for `user:manage`: that permission string isn't repo-scoped, so
//! `check_permission` always falls back to the flat role check for it
//! regardless of what `repo` is passed (see `handlers::check_permission`
//! doc comment) — these handlers pass `""`.
//!
//! Grant mutations go through `state.grants` (the same `GrantsStore`
//! instance `check_permission` reads for repo-level enforcement elsewhere),
//! not `AuthService::grants_store` — the two are separate in-memory
//! instances that only agree at boot (both load from the same
//! `grants.jsonl`), so writing through the wrong one would mean a granted
//! user doesn't actually gain access until a server restart.

use super::*;
use mediagit_security::auth::{user::Role, ApiKey, User};
use serde::{Deserialize, Serialize};

/// User info for the admin listing — deliberately excludes the password
/// hash (which isn't even on `User`; it lives on `UserCredentials`) and
/// email, matching exactly what was asked: id, username, role.
#[derive(Serialize)]
pub struct AdminUserInfo {
    pub id: String,
    pub username: String,
    pub role: Role,
}

impl From<User> for AdminUserInfo {
    fn from(u: User) -> Self {
        Self {
            id: u.id,
            username: u.username,
            role: u.role,
        }
    }
}

/// API key info for the admin listing — metadata only, never the plaintext
/// key (which isn't stored anywhere after generation) or its hash.
#[derive(Serialize)]
pub struct AdminKeyInfo {
    pub id: String,
    pub name: String,
    pub user_id: String,
    pub created_at: i64,
}

impl From<ApiKey> for AdminKeyInfo {
    fn from(k: ApiKey) -> Self {
        Self {
            id: k.id,
            name: k.name,
            user_id: k.user_id,
            created_at: k.created_at,
        }
    }
}

#[derive(Deserialize)]
pub struct GrantRequest {
    pub repo: String,
    pub level: GrantLevel,
}

#[derive(Deserialize)]
pub struct GrantRepoRequest {
    pub repo: String,
}

/// GET /auth/users — list all users (id, username, role only).
pub async fn list_users(
    State(state): State<Arc<AppState>>,
    auth_user: Option<Extension<AuthUser>>,
) -> Result<Json<Vec<AdminUserInfo>>, StatusCode> {
    check_permission(auth_user.as_deref(), "user:manage", state.is_auth_enabled(), &state.grants, "")?;
    let auth_service = state.auth_service().ok_or(StatusCode::NOT_FOUND)?;
    let users = auth_service.credentials_store.list_users().await;
    Ok(Json(users.into_iter().map(AdminUserInfo::from).collect()))
}

/// DELETE /auth/users/{id} — remove a user account and any grants they held.
pub async fn delete_user(
    Path(id): Path<String>,
    State(state): State<Arc<AppState>>,
    auth_user: Option<Extension<AuthUser>>,
) -> Result<StatusCode, StatusCode> {
    check_permission(auth_user.as_deref(), "user:manage", state.is_auth_enabled(), &state.grants, "")?;
    let auth_service = state.auth_service().ok_or(StatusCode::NOT_FOUND)?;
    auth_service
        .credentials_store
        .delete_user(&id)
        .await
        .map_err(|_| StatusCode::NOT_FOUND)?;
    state
        .grants
        .remove_user(&id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(StatusCode::NO_CONTENT)
}

/// POST /auth/users/{id}/grants — upsert a per-repo grant for the user.
pub async fn upsert_grant(
    Path(id): Path<String>,
    State(state): State<Arc<AppState>>,
    auth_user: Option<Extension<AuthUser>>,
    Json(req): Json<GrantRequest>,
) -> Result<StatusCode, StatusCode> {
    check_permission(auth_user.as_deref(), "user:manage", state.is_auth_enabled(), &state.grants, "")?;
    state
        .grants
        .grant(&id, &req.repo, req.level)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(StatusCode::NO_CONTENT)
}

/// DELETE /auth/users/{id}/grants — remove a per-repo grant for the user.
pub async fn remove_grant(
    Path(id): Path<String>,
    State(state): State<Arc<AppState>>,
    auth_user: Option<Extension<AuthUser>>,
    Json(req): Json<GrantRepoRequest>,
) -> Result<StatusCode, StatusCode> {
    check_permission(auth_user.as_deref(), "user:manage", state.is_auth_enabled(), &state.grants, "")?;
    state
        .grants
        .revoke(&id, &req.repo)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(StatusCode::NO_CONTENT)
}

/// GET /auth/keys — list all API keys (metadata only, never the secret).
pub async fn list_keys(
    State(state): State<Arc<AppState>>,
    auth_user: Option<Extension<AuthUser>>,
) -> Result<Json<Vec<AdminKeyInfo>>, StatusCode> {
    check_permission(auth_user.as_deref(), "user:manage", state.is_auth_enabled(), &state.grants, "")?;
    let auth_layer = state.auth().ok_or(StatusCode::NOT_FOUND)?;
    let keys = auth_layer.api_key_auth().list_all_keys().await;
    Ok(Json(keys.into_iter().map(AdminKeyInfo::from).collect()))
}

/// DELETE /auth/keys/{id} — revoke an API key.
pub async fn revoke_key(
    Path(id): Path<String>,
    State(state): State<Arc<AppState>>,
    auth_user: Option<Extension<AuthUser>>,
) -> Result<StatusCode, StatusCode> {
    check_permission(auth_user.as_deref(), "user:manage", state.is_auth_enabled(), &state.grants, "")?;
    let auth_layer = state.auth().ok_or(StatusCode::NOT_FOUND)?;
    auth_layer
        .api_key_auth()
        .revoke_key(&id)
        .await
        .map_err(|_| StatusCode::NOT_FOUND)?;
    Ok(StatusCode::NO_CONTENT)
}
