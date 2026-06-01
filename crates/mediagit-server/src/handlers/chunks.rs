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

// ============================================================================
// Chunk Transfer Endpoints - For efficient large file push
// ============================================================================

/// POST /:repo/chunks/check - Check which chunks exist on remote
///
/// Request body: JSON array of chunk IDs (hex strings)
/// Response: JSON array of MISSING chunk IDs
pub async fn check_chunks_exist(
    Path(repo): Path<String>,
    State(state): State<Arc<AppState>>,
    auth_user: Option<Extension<AuthUser>>,
    Json(chunk_ids): Json<Vec<String>>,
) -> Result<Json<Vec<String>>, StatusCode> {
    // Check write permission
    check_permission(auth_user.as_deref(), "repo:write", state.is_auth_enabled())?;

    tracing::debug!(repo = %repo, chunk_count = chunk_ids.len(), "Checking chunk existence");

    // Resolve repository path
    let repo_path = state.repos_dir.join(&repo);
    if !repo_path.exists() {
        tracing::warn!(repo = %repo, "Repository not found");
        return Err(StatusCode::NOT_FOUND);
    }

    // Create storage backend
    let storage = get_or_init_storage(&state, &repo_path).await?;

    // Lazy-load pack index from JSONL if not yet warm (mirrors locate_chunks).
    {
        let idx = state.pack_index.read().await;
        if !idx.contains_key(&repo) {
            drop(idx);
            load_jsonl_index(&state, &repo, &repo_path).await?;
        }
    }

    // Build set of chunks stored in packs for this repo (pack-index-aware).
    let in_pack_set: std::collections::HashSet<String> = {
        let idx = state.pack_index.read().await;
        idx.get(&repo)
            .map(|m| m.keys().cloned().collect())
            .unwrap_or_default()
    };

    // Check chunks concurrently — up to 50 in-flight existence checks.
    // storage is Arc<dyn StorageBackend> (Send+Sync), cheap to clone.
    //
    // A chunk counts as "present" if any of:
    //   - `chunks/<id>` exists in storage (full chunk)
    //   - `chunk-deltas/<id>.meta` exists (delta sidecar)
    //   - chunk is recorded in the in-memory pack_index (pack-stored)
    let missing: Vec<String> = futures::stream::iter(chunk_ids)
        .map(|chunk_id_hex| {
            let storage = Arc::clone(&storage);
            let in_pack = in_pack_set.contains(&chunk_id_hex);
            async move {
                if in_pack {
                    return None;
                }
                let chunk_key = format!("chunks/{}", chunk_id_hex);
                let delta_meta_key = format!("chunk-deltas/{}.meta", chunk_id_hex);
                let (full, delta) = futures::future::join(
                    storage.exists(&chunk_key),
                    storage.exists(&delta_meta_key),
                )
                .await;
                let exists = matches!(full, Ok(true)) || matches!(delta, Ok(true));
                if exists {
                    None
                } else {
                    if let Err(ref e) = full {
                        tracing::warn!(chunk = %chunk_id_hex, error = %e, "Error checking chunk (full)");
                    }
                    if let Err(ref e) = delta {
                        tracing::warn!(chunk = %chunk_id_hex, error = %e, "Error checking chunk (delta)");
                    }
                    Some(chunk_id_hex)
                }
            }
        })
        .buffer_unordered(50)
        .filter_map(|x| async { x })
        .collect()
        .await;

    tracing::debug!(
        repo = %repo,
        missing_count = missing.len(),
        "Chunk existence check complete"
    );

    Ok(Json(missing))
}

/// PUT /:repo/chunks/:chunk_id - Upload a single chunk
///
/// Request body: Raw compressed chunk data
pub async fn upload_chunk(
    Path((repo, chunk_id)): Path<(String, String)>,
    State(state): State<Arc<AppState>>,
    auth_user: Option<Extension<AuthUser>>,
    body: Bytes,
) -> Result<StatusCode, StatusCode> {
    // Check write permission
    check_permission(auth_user.as_deref(), "repo:write", state.is_auth_enabled())?;

    let upload_start = std::time::Instant::now();
    tracing::info!(
        repo = %repo,
        chunk_id = %chunk_id,
        size = body.len(),
        "PUT chunk"
    );

    // Resolve repository path
    let repo_path = state.repos_dir.join(&repo);
    if !repo_path.exists() {
        tracing::warn!(repo = %repo, "Repository not found");
        return Err(StatusCode::NOT_FOUND);
    }

    // Create storage backend
    let storage = get_or_init_storage(&state, &repo_path).await?;

    // Store chunk directly (already compressed)
    let chunk_key = format!("chunks/{}", chunk_id);
    storage.put(&chunk_key, &body).await.map_err(|e| {
        tracing::error!(chunk = %chunk_id, error = %e, "Failed to store chunk");
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    tracing::info!(
        chunk = %chunk_id,
        size = body.len(),
        elapsed_ms = upload_start.elapsed().as_millis() as u64,
        "Chunk stored"
    );
    Ok(StatusCode::OK)
}

/// PUT /:repo/packs/:pack_id — Proxy-upload a pack object when presigning is unavailable.
///
/// Stores raw pack bytes at `packs/<pack_id>` so that `/packs/complete` can verify via head().
pub async fn upload_pack_proxy(
    Path((repo, pack_id)): Path<(String, String)>,
    State(state): State<Arc<AppState>>,
    auth_user: Option<Extension<AuthUser>>,
    body: Bytes,
) -> Result<StatusCode, StatusCode> {
    check_permission(auth_user.as_deref(), "repo:write", state.is_auth_enabled())?;

    let repo_path = state.repos_dir.join(&repo);
    if !repo_path.exists() {
        return Err(StatusCode::NOT_FOUND);
    }

    let storage = get_or_init_storage(&state, &repo_path).await?;
    let pack_key = format!("packs/{}", pack_id);
    storage.put(&pack_key, &body).await.map_err(|e| {
        tracing::error!(pack = %pack_id, error = %e, "Failed to store pack via proxy");
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    tracing::debug!(pack = %pack_id, bytes = body.len(), "Pack stored via proxy upload");
    Ok(StatusCode::OK)
}

/// PUT /:repo/manifests/:oid - Upload a chunk manifest
///
/// Request body: Serialized ChunkManifest
pub async fn upload_manifest(
    Path((repo, oid)): Path<(String, String)>,
    State(state): State<Arc<AppState>>,
    auth_user: Option<Extension<AuthUser>>,
    body: Bytes,
) -> Result<StatusCode, StatusCode> {
    // Check write permission
    check_permission(auth_user.as_deref(), "repo:write", state.is_auth_enabled())?;

    tracing::debug!(
        repo = %repo,
        oid = %oid,
        size = body.len(),
        "Uploading manifest"
    );

    // Resolve repository path
    let repo_path = state.repos_dir.join(&repo);
    if !repo_path.exists() {
        tracing::warn!(repo = %repo, "Repository not found");
        return Err(StatusCode::NOT_FOUND);
    }

    // Create storage backend
    let storage = get_or_init_storage(&state, &repo_path).await?;

    // Store manifest
    let manifest_key = format!("manifests/{}", oid);
    storage.put(&manifest_key, &body).await.map_err(|e| {
        tracing::error!(oid = %oid, error = %e, "Failed to store manifest");
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    tracing::debug!(oid = %oid, "Manifest stored successfully");
    Ok(StatusCode::OK)
}

// ============================================================================
// Chunk Download Endpoints - For efficient large file pull/clone
// ============================================================================

/// GET /:repo/chunks/:chunk_id - Download a single chunk
///
/// Returns raw compressed chunk data, or 409 JSON `{"kind":"delta","base_id":"<hex>"}` if
/// the chunk is stored as a delta. Clients receiving 409 should re-request via
/// `GET /:repo/chunk-deltas/:chunk_id` and store via `write_chunk_delta`.
pub async fn download_chunk(
    Path((repo, chunk_id)): Path<(String, String)>,
    State(state): State<Arc<AppState>>,
    auth_user: Option<Extension<AuthUser>>,
) -> Result<Response, StatusCode> {
    check_permission(auth_user.as_deref(), "repo:read", state.is_auth_enabled())?;

    tracing::debug!(repo = %repo, chunk_id = %chunk_id, "Downloading chunk");

    let repo_path = state.repos_dir.join(&repo);
    if !repo_path.exists() {
        tracing::warn!(repo = %repo, "Repository not found");
        return Err(StatusCode::NOT_FOUND);
    }

    let storage = get_or_init_storage(&state, &repo_path).await?;

    let chunk_key = format!("chunks/{}", chunk_id);
    match storage.get(&chunk_key).await {
        Ok(chunk_data) => {
            tracing::debug!(chunk = %chunk_id, size = chunk_data.len(), "Chunk downloaded");
            Ok((
                StatusCode::OK,
                [("Content-Type", "application/octet-stream")],
                chunk_data,
            )
                .into_response())
        }
        Err(e) => {
            let chain = format!("{:#}", e).to_lowercase();
            if chain.contains("nosuchkey")
                || chain.contains("no such key")
                || chain.contains("404")
                || chain.contains("not found")
                || chain.contains("service error")
            {
                // Before returning 404: check whether this chunk is stored as a delta.
                // This catches the case where the client's POST /chunk-deltas/check probe
                // failed silently (timeout / transport error) and the client is requesting
                // a delta-only chunk via the wrong /chunks/ route.
                let meta_key = format!("chunk-deltas/{}.meta", chunk_id);
                if let Ok(meta_bytes) = storage.get(&meta_key).await {
                    let meta_str = String::from_utf8_lossy(&meta_bytes);
                    let base_hex = meta_str
                        .strip_prefix("base:")
                        .unwrap_or("")
                        .trim()
                        .to_string();
                    tracing::warn!(
                        chunk = %chunk_id,
                        base = %base_hex,
                        "Chunk requested as full but stored as delta; returning 409 for client re-route"
                    );
                    return Ok((
                        StatusCode::CONFLICT,
                        [("Content-Type", "application/json")],
                        format!(r#"{{"kind":"delta","base_id":"{}"}}"#, base_hex).into_bytes(),
                    )
                        .into_response());
                }
                // Check pack index — chunk may exist inside a pack blob.
                let pack_loc = {
                    let idx = state.pack_index.read().await;
                    idx.get(&repo).and_then(|m| m.get(&chunk_id)).cloned()
                };
                if let Some(loc) = pack_loc {
                    if loc.length < 5 {
                        tracing::error!(
                            chunk = %chunk_id,
                            pack = %loc.pack_oid,
                            length = loc.length,
                            "Pack index entry has length < 5 — corrupt index"
                        );
                        return Err(StatusCode::INTERNAL_SERVER_ERROR);
                    }
                    let pack_key = format!("packs/{}", loc.pack_oid);
                    // Skip 5-byte pack entry header [type:1][size:4] to get raw chunk data.
                    let data_offset = loc.offset + 5;
                    let data_len = (loc.length as u64) - 5;
                    match storage.get_range(&pack_key, data_offset, data_len).await {
                        Ok(data) => {
                            tracing::debug!(
                                chunk = %chunk_id,
                                pack = %loc.pack_oid,
                                "Served chunk from pack via proxy"
                            );
                            return Ok((
                                StatusCode::OK,
                                [("Content-Type", "application/octet-stream")],
                                data,
                            )
                                .into_response());
                        }
                        Err(e) => {
                            tracing::error!(
                                chunk = %chunk_id,
                                pack = %loc.pack_oid,
                                err = %e,
                                "Failed to Range-GET chunk from pack"
                            );
                            return Err(StatusCode::INTERNAL_SERVER_ERROR);
                        }
                    }
                }
                tracing::warn!(chunk = %chunk_id, key = %chunk_key, "Chunk not found in storage");
                Err(StatusCode::NOT_FOUND)
            } else {
                // dispatch failure, connection refused, timeout — storage backend unreachable.
                // Return 503 so clients distinguish "chunk missing" (404) from "backend down" (503).
                tracing::error!(
                    chunk = %chunk_id,
                    key = %chunk_key,
                    error = %e,
                    cause = %format!("{:#}", e),
                    "Chunk storage error: backend unreachable"
                );
                Err(StatusCode::SERVICE_UNAVAILABLE)
            }
        }
    }
}

/// POST /:repo/chunk-deltas/check - Check which chunks exist as chunk-deltas
///
/// Body: JSON array of chunk IDs (hex strings).
/// Response: JSON map { chunk_id_hex -> base_oid_hex } for chunks that have a
/// `chunk-deltas/<id>.meta` sidecar. Chunks not in the response are either
/// stored as full chunks (use `GET /chunks/<id>`) or absent.
///
/// This sidecar pattern lets clients receive deltas during clone/pull instead
/// of inflated full chunks, without changing the manifest schema (which is
/// postcard-encoded and not field-addition-tolerant).
pub async fn check_chunk_deltas_exist(
    Path(repo): Path<String>,
    State(state): State<Arc<AppState>>,
    auth_user: Option<Extension<AuthUser>>,
    Json(chunk_ids): Json<Vec<String>>,
) -> Result<Json<std::collections::HashMap<String, String>>, StatusCode> {
    check_permission(auth_user.as_deref(), "repo:read", state.is_auth_enabled())?;

    tracing::debug!(repo = %repo, chunk_count = chunk_ids.len(), "Checking chunk-delta availability");

    let repo_path = state.repos_dir.join(&repo);
    if !repo_path.exists() {
        return Err(StatusCode::NOT_FOUND);
    }

    let storage = get_or_init_storage(&state, &repo_path).await?;
    let mut deltas = std::collections::HashMap::new();

    // Fast-path: one LIST call to check whether ANY delta metadata exists.
    // For repos with no deltas (synthetic/random data, most test repos), this returns
    // empty immediately (~50ms), skipping potentially thousands of individual GETs.
    let any_delta_keys = storage
        .list_objects("chunk-deltas/")
        .await
        .unwrap_or_default();
    if any_delta_keys.is_empty() {
        tracing::debug!(repo = %repo, "No chunk-deltas in storage; skipping per-chunk check");
        return Ok(Json(deltas));
    }

    // Check chunk-deltas concurrently — up to 200 in-flight meta reads.
    let delta_results: Vec<_> = futures::stream::iter(chunk_ids)
        .map(|chunk_id_hex| {
            let storage = Arc::clone(&storage);
            async move {
                let meta_key = format!("chunk-deltas/{}.meta", chunk_id_hex);
                if let Ok(meta_bytes) = storage.get(&meta_key).await {
                    if let Some(base_hex) = parse_chunk_delta_meta(&meta_bytes) {
                        return Some((chunk_id_hex, base_hex));
                    }
                }
                None
            }
        })
        .buffer_unordered(200)
        .filter_map(|x| async { x })
        .collect()
        .await;
    for (chunk_id_hex, base_hex) in delta_results {
        deltas.insert(chunk_id_hex, base_hex);
    }

    tracing::debug!(repo = %repo, delta_count = deltas.len(), "Chunk-delta check complete");
    Ok(Json(deltas))
}

/// GET /:repo/chunk-deltas/:chunk_id - Download a single chunk-delta payload.
///
/// Returns the raw compressed delta bytes from `chunk-deltas/<chunk_id>`.
/// The base reference is obtained separately via the check endpoint above
/// (which is invoked once per manifest, not once per chunk).
pub async fn download_chunk_delta(
    Path((repo, chunk_id)): Path<(String, String)>,
    State(state): State<Arc<AppState>>,
    auth_user: Option<Extension<AuthUser>>,
) -> Result<impl IntoResponse, StatusCode> {
    check_permission(auth_user.as_deref(), "repo:read", state.is_auth_enabled())?;

    tracing::debug!(repo = %repo, chunk_id = %chunk_id, "Downloading chunk-delta");

    let repo_path = state.repos_dir.join(&repo);
    if !repo_path.exists() {
        return Err(StatusCode::NOT_FOUND);
    }

    let storage = get_or_init_storage(&state, &repo_path).await?;
    let delta_key = format!("chunk-deltas/{}", chunk_id);
    let delta_data = storage.get(&delta_key).await.map_err(|e| {
        tracing::warn!(chunk = %chunk_id, error = %e, "Chunk-delta not found");
        StatusCode::NOT_FOUND
    })?;

    Ok((
        StatusCode::OK,
        [("Content-Type", "application/octet-stream")],
        delta_data,
    ))
}

/// PUT /:repo/chunk-deltas/:chunk_id - Upload a single chunk-delta payload.
///
/// The delta payload is the request body (raw compressed bytes, same format
/// as the server-side storage). The base chunk OID is carried in the
/// `X-Mediagit-Delta-Base` header.
///
/// This exists so the push path can ship deltas verbatim instead of paying
/// the rematerialize-and-re-upload cost through `/chunks/:id`, which was the
/// regression that caused cloned repos to report near-zero compression
/// savings (the server only ever saw full chunks and had no deltas to serve).
///
/// Writes both `chunk-deltas/<chunk_id>` (payload) and `chunk-deltas/<chunk_id>.meta`
/// (body `base:<hex>`) — same on-disk layout as locally-encoded deltas, so
/// downstream reads go through the existing `get_chunk` delta-reconstruction
/// path without any schema change.
pub async fn upload_chunk_delta(
    Path((repo, chunk_id)): Path<(String, String)>,
    State(state): State<Arc<AppState>>,
    auth_user: Option<Extension<AuthUser>>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<StatusCode, StatusCode> {
    check_permission(auth_user.as_deref(), "repo:write", state.is_auth_enabled())?;

    let base_hex = headers
        .get(DELTA_BASE_HEADER)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.trim().to_string())
        .ok_or(StatusCode::BAD_REQUEST)?;

    if base_hex.len() != 64 || !base_hex.chars().all(|c| c.is_ascii_hexdigit()) {
        tracing::warn!(
            repo = %repo,
            chunk_id = %chunk_id,
            base = %base_hex,
            "Rejecting chunk-delta upload: base header is not a 64-char hex oid"
        );
        return Err(StatusCode::BAD_REQUEST);
    }

    if chunk_id.len() != 64 || !chunk_id.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(StatusCode::BAD_REQUEST);
    }

    if chunk_id == base_hex {
        tracing::warn!(
            repo = %repo,
            chunk_id = %chunk_id,
            "Rejecting chunk-delta self-loop"
        );
        return Err(StatusCode::BAD_REQUEST);
    }

    if body.is_empty() {
        return Err(StatusCode::BAD_REQUEST);
    }

    let repo_path = state.repos_dir.join(&repo);
    if !repo_path.exists() {
        return Err(StatusCode::NOT_FOUND);
    }

    let storage = get_or_init_storage(&state, &repo_path).await?;

    let delta_key = format!("chunk-deltas/{}", chunk_id);
    storage.put(&delta_key, &body).await.map_err(|e| {
        tracing::error!(chunk = %chunk_id, error = %e, "Failed to persist chunk-delta payload");
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    let meta_key = format!("chunk-deltas/{}.meta", chunk_id);
    let meta_body = format!("base:{}", base_hex);
    storage
        .put(&meta_key, meta_body.as_bytes())
        .await
        .map_err(|e| {
            tracing::error!(chunk = %chunk_id, error = %e, "Failed to persist chunk-delta meta");
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    tracing::debug!(
        repo = %repo,
        chunk_id = %chunk_id,
        base = %base_hex,
        bytes = body.len(),
        "Chunk-delta uploaded"
    );

    Ok(StatusCode::CREATED)
}

/// GET /:repo/manifests/:oid - Download a chunk manifest
///
/// Returns serialized ChunkManifest
pub async fn download_manifest(
    Path((repo, oid)): Path<(String, String)>,
    State(state): State<Arc<AppState>>,
    auth_user: Option<Extension<AuthUser>>,
) -> Result<impl IntoResponse, StatusCode> {
    // Check read permission
    check_permission(auth_user.as_deref(), "repo:read", state.is_auth_enabled())?;

    tracing::debug!(repo = %repo, oid = %oid, "Downloading manifest");

    // Resolve repository path
    let repo_path = state.repos_dir.join(&repo);
    if !repo_path.exists() {
        tracing::warn!(repo = %repo, "Repository not found");
        return Err(StatusCode::NOT_FOUND);
    }

    // Create storage backend
    let storage = get_or_init_storage(&state, &repo_path).await?;

    // Read manifest
    let manifest_key = format!("manifests/{}", oid);
    let manifest_data = storage.get(&manifest_key).await.map_err(|e| {
        tracing::warn!(oid = %oid, error = %e, "Manifest not found");
        StatusCode::NOT_FOUND
    })?;

    tracing::debug!(oid = %oid, size = manifest_data.len(), "Manifest downloaded");
    Ok((
        StatusCode::OK,
        [("Content-Type", "application/octet-stream")],
        manifest_data,
    ))
}
