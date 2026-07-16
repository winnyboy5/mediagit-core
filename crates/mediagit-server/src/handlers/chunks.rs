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
                // Check pack index first — in-memory, free (D1: this used to run
                // after the chunk-deltas storage GET below, costing an extra
                // storage RTT on every packed-chunk proxy download).
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
                // Not in pack index — check whether this chunk is stored as a delta.
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

// ============================================================================
// D2: Batch pack-chunk proxy — for backends with no presigned GET (GCS+ADC)
// ============================================================================

/// One requested slice in a `POST /:repo/packs/batch-get` request.
///
/// `offset`/`length` are advisory only — the server never trusts them for the
/// actual fetch. They exist so the client's request body is self-describing
/// for debugging; the authoritative offset/length always come from the
/// server's own `pack_index`.
#[derive(serde::Deserialize)]
pub struct BatchGetEntry {
    pub chunk_oid: String,
    #[allow(dead_code)]
    pub offset: u64,
    #[allow(dead_code)]
    pub length: u32,
}

#[derive(serde::Deserialize)]
pub struct BatchGetRequest {
    pub pack_oid: String,
    pub entries: Vec<BatchGetEntry>,
}

/// Process-wide cap on concurrent in-flight `batch-get` requests. Each one
/// streams up to a whole pack object, so unbounded concurrency risks the
/// same TCP/memory exhaustion the GCS upload path hit (see
/// project_gcs_concurrent_upload_fix). Default 4, override via
/// `MEDIAGIT_BATCH_GET_CONCURRENCY`.
fn batch_get_semaphore() -> Arc<tokio::sync::Semaphore> {
    static SEM: std::sync::OnceLock<Arc<tokio::sync::Semaphore>> = std::sync::OnceLock::new();
    Arc::clone(SEM.get_or_init(|| {
        let n: usize = std::env::var("MEDIAGIT_BATCH_GET_CONCURRENCY")
            .ok()
            .and_then(|v| v.parse().ok())
            .filter(|&n| n > 0)
            .unwrap_or(4);
        Arc::new(tokio::sync::Semaphore::new(n))
    }))
}

/// Coalesce adjacent/near (chunk_oid, offset, length) entries — sorted by
/// offset — into byte ranges, same merge rule as the client's
/// `coalesce_chunk_ranges` (mediagit-protocol client/mod.rs): merge when the
/// gap since the current range end is within `max_gap` AND the merged range
/// stays within `max_bytes`.
fn coalesce_ranges(
    entries: &[(String, u64, u32)],
    max_gap: u64,
    max_bytes: u64,
) -> Vec<(u64, u64)> {
    let mut ranges: Vec<(u64, u64)> = Vec::new();
    if entries.is_empty() {
        return ranges;
    }
    let mut cur_start = entries[0].1;
    let mut cur_end = cur_start + entries[0].2 as u64;
    for (_, off, len) in &entries[1..] {
        let entry_end = off + *len as u64;
        let gap = off.saturating_sub(cur_end);
        let merged = entry_end - cur_start;
        if gap <= max_gap && merged <= max_bytes {
            cur_end = cur_end.max(entry_end);
        } else {
            ranges.push((cur_start, cur_end));
            cur_start = *off;
            cur_end = entry_end;
        }
    }
    ranges.push((cur_start, cur_end));
    ranges
}

/// Write one batch-get frame: `[chunk_oid: 32 raw bytes][len: u32 LE][data: len bytes]`.
/// `payload` is the compressed chunk data with the 5-byte pack entry header
/// already stripped — identical to what the presigned Range-GET client path
/// verifies (mediagit-protocol client/packs.rs `pull_chunks_via_packs`).
/// An empty `payload` is a valid "miss" frame (invalid/unindexed entry).
async fn write_batch_frame<W: tokio::io::AsyncWrite + Unpin>(
    w: &mut W,
    chunk_oid_hex: &str,
    payload: &[u8],
) -> anyhow::Result<()> {
    use tokio::io::AsyncWriteExt;
    let oid = Oid::from_hex(chunk_oid_hex)?;
    w.write_all(oid.as_bytes()).await?;
    w.write_all(&(payload.len() as u32).to_le_bytes()).await?;
    if !payload.is_empty() {
        w.write_all(payload).await?;
    }
    Ok(())
}

/// POST /:repo/packs/batch-get — batch-fetch multiple chunk slices out of one
/// pack object in a single request/response.
///
/// Exists for storage backends with no presigned-GET support (GCS + ADC):
/// without this, every packed chunk pull falls back to one server-proxy RTT
/// per chunk via `GET /chunks/:id`. This collapses a whole pack's worth of
/// chunk pulls into one request.
///
/// Streams `[chunk_oid:32][len:u32 LE][data:len]` frames, one per requested
/// entry, in request order. Entries not found in the server's `pack_index`
/// for the given `pack_oid` come back as zero-length frames — the client
/// treats those as a miss and falls back to the per-chunk path.
pub async fn batch_get_pack_chunks(
    Path(repo): Path<String>,
    State(state): State<Arc<AppState>>,
    auth_user: Option<Extension<AuthUser>>,
    Json(req): Json<BatchGetRequest>,
) -> Result<Response, StatusCode> {
    check_permission(auth_user.as_deref(), "repo:read", state.is_auth_enabled())?;

    let repo_path = state.repos_dir.join(&repo);
    if !repo_path.exists() {
        return Err(StatusCode::NOT_FOUND);
    }
    let storage = get_or_init_storage(&state, &repo_path).await?;

    // Lazy-load pack index from JSONL if not yet warm (mirrors locate_chunks).
    {
        let idx = state.pack_index.read().await;
        if !idx.contains_key(&repo) {
            drop(idx);
            load_jsonl_index(&state, &repo, &repo_path).await?;
        }
    }

    // Validate every requested entry against the server's own pack_index —
    // never trust client-supplied offset/length for the actual fetch. Entries
    // that don't resolve (unknown chunk, or resolve to a different pack) are
    // served as zero-length "miss" frames instead of erroring the request.
    let mut valid: Vec<(String, u64, u32)> = Vec::new();
    let mut invalid: Vec<String> = Vec::new();
    {
        let idx = state.pack_index.read().await;
        let repo_idx = idx.get(&repo);
        for e in &req.entries {
            match repo_idx.and_then(|m| m.get(&e.chunk_oid)) {
                Some(loc) if loc.pack_oid == req.pack_oid && loc.length >= 5 => {
                    valid.push((e.chunk_oid.clone(), loc.offset, loc.length));
                }
                _ => invalid.push(e.chunk_oid.clone()),
            }
        }
    }

    let pack_key = format!("packs/{}", req.pack_oid);
    let pack_size = storage.head(&pack_key).await.map_err(|e| {
        tracing::error!(pack = %req.pack_oid, err = %e, "batch-get: head() failed");
        StatusCode::INTERNAL_SERVER_ERROR
    })?;
    let Some(pack_size) = pack_size else {
        return Err(StatusCode::NOT_FOUND);
    };

    // ponytail: whole-pack read ceiling is the pack size (Track F caps packs
    // at 64 MiB), not a strict 32 MiB bound. Only taken when >=50% of the
    // pack is wanted, so it's rarely worse than the coalesced-range path,
    // which itself buffers at most `MAX_RANGE_BYTES` (8 MiB) at a time.
    let total_wanted: u64 = valid.iter().map(|(_, _, len)| *len as u64).sum();
    let whole_pack = pack_size > 0 && total_wanted.saturating_mul(2) >= pack_size;

    let semaphore = batch_get_semaphore();
    let permit = semaphore
        .acquire_owned()
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    const MAX_GAP_BYTES: u64 = 1_048_576;
    const MAX_RANGE_BYTES: u64 = 8_388_608;

    let (reader, writer) = tokio::io::duplex(256 * 1024);

    tokio::spawn(async move {
        let _permit = permit;
        let mut w = writer;
        let result: anyhow::Result<()> = async {
            for chunk_oid in &invalid {
                write_batch_frame(&mut w, chunk_oid, &[]).await?;
            }
            if valid.is_empty() {
                return Ok(());
            }

            if whole_pack {
                let data = storage.get_with_size_hint(&pack_key, Some(pack_size)).await?;
                for (chunk_oid, offset, length) in &valid {
                    let start = *offset as usize + 5; // skip [type:1][size:4]
                    let end = *offset as usize + *length as usize;
                    match data.get(start..end) {
                        Some(payload) => write_batch_frame(&mut w, chunk_oid, payload).await?,
                        None => {
                            tracing::warn!(chunk = %chunk_oid, pack = %req.pack_oid, "batch-get: pack_index entry out of bounds");
                            write_batch_frame(&mut w, chunk_oid, &[]).await?;
                        }
                    }
                }
            } else {
                let mut sorted = valid.clone();
                sorted.sort_unstable_by_key(|(_, off, _)| *off);
                let ranges = coalesce_ranges(&sorted, MAX_GAP_BYTES, MAX_RANGE_BYTES);
                for (range_start, range_end) in ranges {
                    let buf = storage
                        .get_range(&pack_key, range_start, range_end - range_start)
                        .await?;
                    for (chunk_oid, offset, length) in &sorted {
                        if *offset < range_start || *offset + *length as u64 > range_end {
                            continue;
                        }
                        let rel = (*offset - range_start) as usize + 5; // skip header
                        let rel_end = (*offset - range_start) as usize + *length as usize;
                        match buf.get(rel..rel_end) {
                            Some(payload) => write_batch_frame(&mut w, chunk_oid, payload).await?,
                            None => {
                                tracing::warn!(chunk = %chunk_oid, pack = %req.pack_oid, "batch-get: range slice out of bounds");
                                write_batch_frame(&mut w, chunk_oid, &[]).await?;
                            }
                        }
                    }
                }
            }
            Ok(())
        }
        .await;
        if let Err(e) = result {
            tracing::warn!(pack = %req.pack_oid, err = %e, "batch-get: stream write failed");
        }
    });

    let stream = ReaderStream::new(reader);
    let body = axum::body::Body::from_stream(stream);
    Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", "application/octet-stream")
        .body(body)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)
}

#[cfg(test)]
mod batch_get_tests {
    use super::*;
    use mediagit_compression::{Compressor, SmartCompressor};
    /// Writes one packed chunk (valid, decompressible) and registers it in the
    /// in-memory pack_index. Mirrors `write_corrupt_pack_entry` in
    /// handlers/transfer.rs but keeps the payload intact for round-trip tests.
    ///
    /// `storage` must be the same `Arc<dyn StorageBackend>` the handler under
    /// test will resolve via `get_or_init_storage` (i.e. namespace-prefixed
    /// per layout v2) — writing through a bare `LocalBackend` instead lands
    /// keys outside the namespace prefix the handler reads from.
    async fn write_pack_entry(
        state: &AppState,
        storage: &Arc<dyn StorageBackend>,
        repo: &str,
        pack_oid: &str,
        offset: u64,
        content: &[u8],
    ) -> (String, u32) {
        let chunk_id = Oid::hash(content).to_hex();
        let compressor = SmartCompressor::new();
        let compressed = compressor.compress(content).expect("compress");

        let mut entry_bytes = vec![0u8; 5]; // [type:1][size:4] header, unused by reader
        entry_bytes.extend_from_slice(&compressed);
        let length = entry_bytes.len() as u32;

        let pack_key = format!("packs/{}", pack_oid);
        let mut pack_bytes = storage.get(&pack_key).await.unwrap_or_default();
        // Pad to offset if needed (single-entry tests use offset == current len).
        if (pack_bytes.len() as u64) < offset {
            pack_bytes.resize(offset as usize, 0);
        }
        pack_bytes.truncate(offset as usize);
        pack_bytes.extend_from_slice(&entry_bytes);
        storage.put(&pack_key, &pack_bytes).await.expect("put pack");

        let loc = PackLoc {
            pack_oid: pack_oid.to_string(),
            offset,
            length,
            compressed_hash: Some(Oid::hash(&compressed).to_hex()),
        };
        {
            let mut idx = state.pack_index.write().await;
            idx.entry(repo.to_string())
                .or_default()
                .insert(chunk_id.clone(), loc);
        }
        (chunk_id, length)
    }

    async fn read_all_frames(reader: impl tokio::io::AsyncRead + Unpin) -> Vec<(String, Vec<u8>)> {
        use tokio::io::AsyncReadExt;
        let mut r = reader;
        let mut buf = Vec::new();
        r.read_to_end(&mut buf).await.unwrap();
        let mut out = Vec::new();
        let mut pos = 0usize;
        while pos + 36 <= buf.len() {
            let oid_bytes: [u8; 32] = buf[pos..pos + 32].try_into().unwrap();
            let oid = Oid::from_bytes(oid_bytes);
            pos += 32;
            let len = u32::from_le_bytes(buf[pos..pos + 4].try_into().unwrap()) as usize;
            pos += 4;
            let data = buf[pos..pos + len].to_vec();
            pos += len;
            out.push((oid.to_hex(), data));
        }
        out
    }

    #[tokio::test]
    async fn batch_get_returns_correct_frames_for_two_entries() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = "test-repo".to_string();
        let repo_path = tmp.path().join(&repo);
        tokio::fs::create_dir_all(&repo_path).await.unwrap();
        let state = Arc::new(AppState::new(tmp.path().to_path_buf()));
        let storage = get_or_init_storage(&state, &repo_path)
            .await
            .expect("storage");

        let (id_a, len_a) =
            write_pack_entry(&state, &storage, &repo, "packA", 0, b"hello world").await;
        let (id_b, _) = write_pack_entry(
            &state,
            &storage,
            &repo,
            "packA",
            len_a as u64,
            b"goodbye world",
        )
        .await;

        let req = BatchGetRequest {
            pack_oid: "packA".to_string(),
            entries: vec![
                BatchGetEntry {
                    chunk_oid: id_a.clone(),
                    offset: 0,
                    length: len_a,
                },
                BatchGetEntry {
                    chunk_oid: id_b.clone(),
                    offset: 0,
                    length: 0,
                },
            ],
        };

        let resp = batch_get_pack_chunks(
            Path(repo.clone()),
            State(Arc::clone(&state)),
            None,
            Json(req),
        )
        .await
        .expect("handler ok");
        let body = resp.into_body();
        let reader = tokio_util::io::StreamReader::new(
            body.into_data_stream()
                .map(|r| r.map_err(std::io::Error::other)),
        );
        let frames = read_all_frames(reader).await;

        assert_eq!(frames.len(), 2);
        assert_eq!(frames[0].0, id_a);
        assert!(!frames[0].1.is_empty());
        assert_eq!(frames[1].0, id_b);
        assert!(!frames[1].1.is_empty());

        // Decompress and verify content round-trips.
        let compressor = SmartCompressor::new();
        let decompressed_a = compressor.decompress(&frames[0].1).expect("decompress a");
        assert_eq!(decompressed_a, b"hello world");
    }

    #[tokio::test]
    async fn batch_get_bogus_entry_returns_zero_length_frame() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = "test-repo".to_string();
        let repo_path = tmp.path().join(&repo);
        tokio::fs::create_dir_all(&repo_path).await.unwrap();
        let state = Arc::new(AppState::new(tmp.path().to_path_buf()));
        let storage = get_or_init_storage(&state, &repo_path)
            .await
            .expect("storage");

        let (id_a, len_a) =
            write_pack_entry(&state, &storage, &repo, "packA", 0, b"real chunk").await;

        // Bogus entry: not present in pack_index at all.
        let bogus_id = Oid::hash(b"never registered").to_hex();

        let req = BatchGetRequest {
            pack_oid: "packA".to_string(),
            entries: vec![
                BatchGetEntry {
                    chunk_oid: id_a.clone(),
                    offset: 0,
                    length: len_a,
                },
                BatchGetEntry {
                    chunk_oid: bogus_id.clone(),
                    offset: 9999, // client lies about offset; server must ignore this
                    length: 100,
                },
            ],
        };

        let resp = batch_get_pack_chunks(
            Path(repo.clone()),
            State(Arc::clone(&state)),
            None,
            Json(req),
        )
        .await
        .expect("handler ok");
        let body = resp.into_body();
        let reader = tokio_util::io::StreamReader::new(
            body.into_data_stream()
                .map(|r| r.map_err(std::io::Error::other)),
        );
        let frames = read_all_frames(reader).await;

        assert_eq!(frames.len(), 2);
        // Invalid entries are written first, ahead of valid ones.
        assert_eq!(frames[0].0, bogus_id);
        assert!(frames[0].1.is_empty(), "bogus entry must be zero-length");
        assert_eq!(frames[1].0, id_a);
        assert!(!frames[1].1.is_empty());
    }
}
