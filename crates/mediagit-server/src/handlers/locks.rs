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

use super::*;
use crate::locks::{self, LockRecord};

// ============================================================================
// Server-enforced file locking (B2) — HTTP surface over `crate::locks`
// ============================================================================

#[derive(serde::Deserialize)]
pub struct CreateLockRequest {
    pub path: String,
    #[serde(default)]
    pub owner: Option<String>,
}

#[derive(serde::Serialize)]
pub struct LockResponse {
    pub lock_id: String,
    pub path: String,
    pub owner: String,
    pub created_at: u64,
}

impl From<LockRecord> for LockResponse {
    fn from(r: LockRecord) -> Self {
        Self {
            lock_id: r.lock_id,
            path: r.path,
            owner: r.owner,
            created_at: r.created_at,
        }
    }
}

#[derive(serde::Serialize)]
pub struct LockConflictResponse {
    pub error: String,
    pub path: String,
    pub owner: String,
    pub lock_id: String,
}

#[derive(serde::Serialize)]
pub struct ListLocksResponse {
    pub locks: Vec<LockResponse>,
}

#[derive(serde::Deserialize)]
pub struct DeleteLockQuery {
    #[serde(default)]
    pub force: Option<String>,
}

/// Resolve the identity to attribute a lock action to: the authenticated
/// user when auth is enabled, otherwise the client-supplied `owner` string
/// (required in that case — a no-auth server has no other notion of who's
/// asking).
fn resolve_owner(
    auth_user: Option<&AuthUser>,
    supplied: Option<String>,
) -> Result<String, StatusCode> {
    if let Some(user) = auth_user {
        return Ok(user.user_id.clone());
    }
    supplied
        .filter(|s| !s.trim().is_empty())
        .ok_or(StatusCode::BAD_REQUEST)
}

/// POST /:repo/locks — acquire a lock on `path`.
pub async fn create_lock(
    Path(repo): Path<String>,
    State(state): State<Arc<AppState>>,
    auth_user: Option<Extension<AuthUser>>,
    Json(req): Json<CreateLockRequest>,
) -> Result<Response, StatusCode> {
    check_permission(
        auth_user.as_deref(),
        "repo:write",
        state.is_auth_enabled(),
        &state.grants,
        &repo,
    )?;

    let repo_path = state.repos_dir.join(&repo);
    if !repo_path.exists() {
        return Err(StatusCode::NOT_FOUND);
    }
    if req.path.trim().is_empty() {
        return Err(StatusCode::BAD_REQUEST);
    }
    let owner = resolve_owner(auth_user.as_deref(), req.owner)?;

    match locks::create_lock(&state, &repo, &repo_path, req.path.clone(), owner).await? {
        locks::CreateLockOutcome::Created(record) => {
            Ok((StatusCode::CREATED, Json(LockResponse::from(record))).into_response())
        }
        locks::CreateLockOutcome::AlreadyLocked(existing) => Ok((
            StatusCode::CONFLICT,
            Json(LockConflictResponse {
                error: format!(
                    "'{}' is already locked by {}",
                    existing.path, existing.owner
                ),
                path: existing.path,
                owner: existing.owner,
                lock_id: existing.lock_id,
            }),
        )
            .into_response()),
    }
}

/// GET /:repo/locks — list active locks.
pub async fn list_locks(
    Path(repo): Path<String>,
    State(state): State<Arc<AppState>>,
    auth_user: Option<Extension<AuthUser>>,
) -> Result<Json<ListLocksResponse>, StatusCode> {
    check_permission(
        auth_user.as_deref(),
        "repo:read",
        state.is_auth_enabled(),
        &state.grants,
        &repo,
    )?;

    let repo_path = state.repos_dir.join(&repo);
    if !repo_path.exists() {
        return Err(StatusCode::NOT_FOUND);
    }

    let map = locks::get_repo_locks(&state, &repo, &repo_path).await?;
    let mut list: Vec<LockResponse> = map.into_values().map(LockResponse::from).collect();
    list.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(Json(ListLocksResponse { locks: list }))
}

/// DELETE /:repo/locks/:lock_id?force=1 — release a lock.
///
/// Without `force`, the requester must be the lock's owner (compared against
/// `AuthUser.user_id`; a no-auth server has no identity to compare and so
/// must always pass `force=1`, which itself requires `repo:admin`).
pub async fn delete_lock(
    Path((repo, lock_id)): Path<(String, String)>,
    State(state): State<Arc<AppState>>,
    auth_user: Option<Extension<AuthUser>>,
    axum::extract::Query(query): axum::extract::Query<DeleteLockQuery>,
) -> Result<StatusCode, StatusCode> {
    check_permission(
        auth_user.as_deref(),
        "repo:write",
        state.is_auth_enabled(),
        &state.grants,
        &repo,
    )?;

    let repo_path = state.repos_dir.join(&repo);
    if !repo_path.exists() {
        return Err(StatusCode::NOT_FOUND);
    }

    let force = query.force.as_deref() == Some("1");
    if force {
        check_permission(
            auth_user.as_deref(),
            "repo:admin",
            state.is_auth_enabled(),
            &state.grants,
            &repo,
        )?;
    }

    let record = locks::find_lock_by_id(&state, &repo, &repo_path, &lock_id).await?;
    let record = record.ok_or(StatusCode::NOT_FOUND)?;

    if !force {
        let requester = auth_user.as_ref().map(|u| u.user_id.as_str());
        let is_owner = requester.map(|id| id == record.owner).unwrap_or(false);
        if !is_owner {
            return Err(StatusCode::FORBIDDEN);
        }
    }

    locks::delete_lock(&state, &repo, &repo_path, &lock_id).await?;
    Ok(StatusCode::NO_CONTENT)
}
