// SPDX-License-Identifier: BUSL-1.1
// Copyright (C) 2025-2026 Aswin Krishnamoorthy

//! Hosted-repo admin surface: list / create / delete repositories under
//! `repos_dir`. Mounted only with auth enabled (see `auth_routes`), gated on
//! the flat `user:manage` permission like the rest of the admin routes.
//!
//! Repo names are validated with `security::validate_repo_name` BEFORE any
//! filesystem access (J6 path-traversal class), and are additionally limited
//! to a single path segment so the flat listing stays consistent.

use super::*;
use axum::extract::Query;
use serde::{Deserialize, Serialize};

/// Cap on directory entries visited when summing a repo's size.
const SIZE_WALK_MAX_ENTRIES: usize = 200_000;

#[derive(Serialize)]
pub struct AdminRepoInfo {
    pub name: String,
    pub size_bytes: u64,
    /// Not computed (would need an ODB walk); always absent for now.
    pub object_count: Option<u64>,
    /// Unix seconds, mtime of the repo directory.
    pub last_modified: u64,
}

#[derive(Deserialize)]
pub struct CreateRepoRequest {
    pub name: String,
}

#[derive(Deserialize)]
pub struct DeleteRepoParams {
    pub confirm: Option<String>,
    #[serde(default)]
    pub force: bool,
}

#[derive(Serialize)]
pub struct DeleteRepoResult {
    pub deleted: bool,
    /// True when the repo's data lives outside the repo directory
    /// (non-filesystem backend) and was therefore NOT removed.
    pub storage_retained: bool,
    pub backend: &'static str,
}

fn backend_kind(cfg: &mediagit_config::Config) -> &'static str {
    use mediagit_config::StorageConfig as S;
    match cfg.storage {
        S::FileSystem(_) => "filesystem",
        S::S3(_) => "s3",
        S::Azure(_) => "azure",
        S::GCS(_) => "gcs",
        S::Multi(_) => "multi",
    }
}

#[derive(Serialize)]
pub struct RepoGrantInfo {
    pub user_id: String,
    /// Empty if the account no longer exists.
    pub username: String,
    pub level: GrantLevel,
}

fn admin_gate(state: &AppState, auth_user: Option<&AuthUser>) -> Result<String, StatusCode> {
    check_permission(
        auth_user,
        "user:manage",
        state.is_auth_enabled(),
        &state.grants,
        "",
    )?;
    Ok(auth_user.map(|u| u.user_id.clone()).unwrap_or_default())
}

/// CON, PRN, AUX, NUL, COM1-9, LPT1-9, with or without an extension. Refused on
/// every platform so a repo created on Linux stays portable.
fn is_windows_device_name(name: &str) -> bool {
    let stem = name.split('.').next().unwrap_or("").to_ascii_uppercase();
    matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || (stem.len() == 4
            && (stem.starts_with("COM") || stem.starts_with("LPT"))
            && matches!(stem.as_bytes()[3], b'1'..=b'9'))
}

fn valid_admin_repo_name(name: &str) -> Result<(), StatusCode> {
    let bad = name.is_empty()
        || name.contains('/')
        || name.starts_with('.')
        || name.ends_with(['.', ' '])
        || is_windows_device_name(name)
        || crate::security::validate_repo_name(name).is_err();
    if bad {
        tracing::warn!(repo = %name, "repo admin refused: invalid repository name");
        return Err(StatusCode::BAD_REQUEST);
    }
    Ok(())
}

fn mtime_secs(path: &StdPath) -> u64 {
    std::fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Sum of file sizes under `root`, visiting at most `SIZE_WALK_MAX_ENTRIES`
/// entries.
// ponytail: bounded dir walk; huge repos report a lower bound. Cache or
// track size incrementally if the GUI needs exact figures.
fn dir_size(root: &StdPath) -> u64 {
    let mut total = 0u64;
    let mut visited = 0usize;
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in rd.flatten() {
            visited += 1;
            if visited > SIZE_WALK_MAX_ENTRIES {
                return total;
            }
            let Ok(ft) = entry.file_type() else { continue };
            if ft.is_dir() {
                stack.push(entry.path());
            } else if ft.is_file() {
                total += entry.metadata().map(|m| m.len()).unwrap_or(0);
            }
        }
    }
    total
}

fn has_any_file(root: &StdPath) -> bool {
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in rd.flatten() {
            match entry.file_type() {
                Ok(ft) if ft.is_dir() => stack.push(entry.path()),
                Ok(_) => return true,
                Err(_) => {}
            }
        }
    }
    false
}

fn repo_info(name: &str, path: &StdPath) -> AdminRepoInfo {
    AdminRepoInfo {
        name: name.to_string(),
        size_bytes: dir_size(path),
        object_count: None,
        last_modified: mtime_secs(path),
    }
}

/// GET /admin/repos
pub async fn list_repos(
    State(state): State<Arc<AppState>>,
    auth_user: Option<Extension<AuthUser>>,
) -> Result<Json<Vec<AdminRepoInfo>>, StatusCode> {
    admin_gate(&state, auth_user.as_deref())?;
    let dir = state.repos_dir.clone();
    let repos = tokio::task::spawn_blocking(move || {
        let mut out = Vec::new();
        if let Ok(rd) = std::fs::read_dir(&dir) {
            for entry in rd.flatten() {
                let name = entry.file_name().to_string_lossy().into_owned();
                if entry.file_type().map(|t| t.is_dir()).unwrap_or(false)
                    && valid_admin_repo_name(&name).is_ok()
                {
                    out.push(repo_info(&name, &entry.path()));
                }
            }
        }
        out.sort_by(|a, b| a.name.cmp(&b.name));
        out
    })
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(Json(repos))
}

/// POST /admin/repos {name} — create an empty repository.
///
/// The server never created repos on push (handlers 404 on a missing dir), so
/// this is the creation path: the repo dir + `.mediagit`, then the same first
/// storage init every handler performs (persists `repo_id` in config.toml).
pub async fn create_repo(
    State(state): State<Arc<AppState>>,
    auth_user: Option<Extension<AuthUser>>,
    Json(req): Json<CreateRepoRequest>,
) -> Result<(StatusCode, Json<AdminRepoInfo>), StatusCode> {
    let who = admin_gate(&state, auth_user.as_deref())?;
    valid_admin_repo_name(&req.name)?;

    tokio::fs::create_dir_all(&state.repos_dir)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let repo_path = state.repos_dir.join(&req.name);
    // create_dir (not _all) so two racing creates cannot both win.
    match tokio::fs::create_dir(&repo_path).await {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
            return Err(StatusCode::CONFLICT);
        }
        Err(_) => return Err(StatusCode::INTERNAL_SERVER_ERROR),
    }
    let init = async {
        tokio::fs::create_dir_all(repo_path.join(".mediagit"))
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        if let Some(storage) = state.default_storage.clone() {
            let cfg = mediagit_config::Config {
                storage,
                ..Default::default()
            };
            let dir = repo_path.clone();
            tokio::task::spawn_blocking(move || cfg.save(dir))
                .await
                .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
                .map_err(|e| {
                    tracing::error!("writing repo storage config: {e}");
                    StatusCode::INTERNAL_SERVER_ERROR
                })?;
        }
        get_or_init_storage(&state, &repo_path).await.map(|_| ())
    }
    .await;
    if let Err(code) = init {
        // Do not leave a half-made repo that would 409 every retry.
        let _ = tokio::fs::remove_dir_all(&repo_path).await;
        return Err(code);
    }
    tracing::info!(admin = %who, repo = %req.name, "audit: repository created");
    let name = req.name.clone();
    let info = tokio::task::spawn_blocking(move || repo_info(&name, &repo_path))
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok((StatusCode::CREATED, Json(info)))
}

/// DELETE /admin/repos/{name}?confirm=<name>[&force=true]
///
/// 400 if `confirm != name`; 404 if absent; 409 if the repo has refs or file
/// locks and `force` is not set, or if it has in-memory active transfers
/// (always, force or not).
// ponytail: no per-repo write guard exists in the server, so delete cannot
// be made atomic with a push: a push that passes these checks and starts
// right after still races the removal, and in-flight MPUs live in the
// storage backend (not tracked in memory) so they are not seen. Add a
// per-repo RwLock taken by write handlers if this must be airtight.
// Only the repo directory is removed; the response's `storage_retained`
// says when the backend is not filesystem (bucket data is kept).
pub async fn delete_repo(
    Path(name): Path<String>,
    Query(params): Query<DeleteRepoParams>,
    State(state): State<Arc<AppState>>,
    auth_user: Option<Extension<AuthUser>>,
) -> Result<Json<DeleteRepoResult>, StatusCode> {
    let who = admin_gate(&state, auth_user.as_deref())?;
    valid_admin_repo_name(&name)?;
    if params.confirm.as_deref() != Some(name.as_str()) {
        return Err(StatusCode::BAD_REQUEST);
    }
    let repo_path = state.repos_dir.join(&name);
    if !repo_path.is_dir() {
        return Err(StatusCode::NOT_FOUND);
    }

    // Active transfers: never overridable by force.
    let wants = state.want_cache.lock().await.mentions_repo(&name);
    let unverified = state
        .unverified_packs
        .read()
        .await
        .get(&name)
        .is_some_and(|s| !s.is_empty());
    if wants || unverified {
        tracing::warn!(admin = %who, repo = %name, "audit: repo delete refused: active transfers");
        return Err(StatusCode::CONFLICT);
    }

    if !params.force {
        let locks = crate::locks::get_repo_locks(&state, &name, &repo_path).await?;
        let rp = repo_path.join(".mediagit/refs");
        let has_refs = tokio::task::spawn_blocking(move || has_any_file(&rp))
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        if has_refs || !locks.is_empty() {
            return Err(StatusCode::CONFLICT);
        }
    }

    let backend = mediagit_config::Config::load(&repo_path)
        .await
        .map(|c| backend_kind(&c))
        .unwrap_or("filesystem");

    // Evict every per-repo cache first so nothing serves a deleted repo.
    state.storage_backends.write().await.remove(&repo_path);
    state.odb_cache.write().await.remove(&repo_path);
    state.pack_index.write().await.remove(&name);
    state.unverified_packs.write().await.remove(&name);
    state.locks.write().await.remove(&name);

    tokio::fs::remove_dir_all(&repo_path).await.map_err(|e| {
        tracing::error!(repo = %name, error = %e, "repo delete failed");
        StatusCode::INTERNAL_SERVER_ERROR
    })?;
    tracing::warn!(admin = %who, repo = %name, force = params.force, "audit: repository deleted");
    Ok(Json(DeleteRepoResult {
        deleted: true,
        storage_retained: backend != "filesystem",
        backend,
    }))
}

/// GET /admin/repos/{name}/grants — who has access to a repo, sorted by user id.
pub async fn list_repo_grants(
    Path(name): Path<String>,
    State(state): State<Arc<AppState>>,
    auth_user: Option<Extension<AuthUser>>,
) -> Result<Json<Vec<RepoGrantInfo>>, StatusCode> {
    admin_gate(&state, auth_user.as_deref())?;
    valid_admin_repo_name(&name)?;
    if !state.repos_dir.join(&name).is_dir() {
        return Err(StatusCode::NOT_FOUND);
    }
    let mut grants = state.grants.list_for_repo(&name);
    grants.sort_by(|a, b| a.0.cmp(&b.0));
    let mut out = Vec::with_capacity(grants.len());
    for (user_id, level) in grants {
        let username = match state.auth_service() {
            Some(svc) => svc
                .credentials_store
                .get_user(&user_id)
                .await
                .map(|u| u.username)
                .unwrap_or_default(),
            None => String::new(),
        };
        out.push(RepoGrantInfo {
            user_id,
            username,
            level,
        });
    }
    Ok(Json(out))
}
