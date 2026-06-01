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
// Presigned Upload Endpoints — Direct client→backend chunk transfer
// ============================================================================

#[derive(serde::Deserialize)]
pub struct PresignChunkUploadsRequest {
    chunk_ids: Vec<String>,
    #[serde(default)]
    sizes: std::collections::HashMap<String, u64>,
}

#[derive(serde::Deserialize)]
pub struct PresignPackUploadsRequest {
    pack_ids: Vec<String>,
    /// Byte sizes parallel to `pack_ids`; index i is the size for `pack_ids[i]`.
    #[serde(default)]
    sizes: Vec<u64>,
}

#[derive(serde::Serialize)]
pub struct PresignedPutJson {
    url: String,
    method: String,
    /// Each entry is [header-name, header-value].
    required_headers: Vec<[String; 2]>,
}

/// POST /:repo/packs/upload-urls — Mint presigned PUT URLs for a batch of pack objects.
///
/// Request: `{ pack_ids: [hex, ...], sizes: [u64, ...] }`
/// Response: `{ hex: { url, method, required_headers } | null, ... }`
///
/// A `null` entry means the backend does not support presigning; the client must
/// fall back to the server-proxied upload route for that pack.
pub async fn presign_pack_uploads(
    Path(repo): Path<String>,
    State(state): State<Arc<AppState>>,
    auth_user: Option<Extension<AuthUser>>,
    Json(req): Json<PresignPackUploadsRequest>,
) -> Result<Json<std::collections::HashMap<String, Option<PresignedPutJson>>>, StatusCode> {
    check_permission(auth_user.as_deref(), "repo:write", state.is_auth_enabled())?;

    let repo_path = state.repos_dir.join(&repo);
    if !repo_path.exists() {
        return Err(StatusCode::NOT_FOUND);
    }
    let storage = get_or_init_storage(&state, &repo_path).await?;
    let ttl = std::time::Duration::from_secs(state.presigned_url_ttl_secs);

    let presign_concurrency: usize = std::env::var("MEDIAGIT_PRESIGN_CONCURRENCY")
        .ok()
        .and_then(|s| s.parse().ok())
        .filter(|n: &usize| *n > 0)
        .unwrap_or(64);

    let count = req.pack_ids.len();
    let pairs: Vec<(String, u64)> = req
        .pack_ids
        .into_iter()
        .enumerate()
        .map(|(i, id)| {
            let size = req.sizes.get(i).copied().unwrap_or(0);
            (id, size)
        })
        .collect();

    let entries: Vec<(String, Option<PresignedPutJson>)> =
        futures::stream::iter(pairs.into_iter().map(|(pack_id, content_length)| {
            let storage = Arc::clone(&storage);
            let repo = repo.clone();
            async move {
                let key = format!("packs/{}", pack_id);
                let entry = match storage.presign_put(&key, content_length, ttl).await {
                    Ok(Some(p)) => Some(PresignedPutJson {
                        url: p.url,
                        method: p.method,
                        required_headers: p
                            .required_headers
                            .into_iter()
                            .map(|(k, v)| [k, v])
                            .collect(),
                    }),
                    Ok(None) => None,
                    Err(e) => {
                        tracing::warn!(
                            repo = %repo,
                            pack = %pack_id,
                            err = %e,
                            "presign_put failed; client will fall back to proxy upload"
                        );
                        None
                    }
                };
                (pack_id, entry)
            }
        }))
        .buffer_unordered(presign_concurrency)
        .collect()
        .await;

    let urls: std::collections::HashMap<String, Option<PresignedPutJson>> =
        entries.into_iter().collect();

    tracing::info!(
        repo = %repo,
        count = count,
        "Presigned pack upload URLs generated"
    );
    Ok(Json(urls))
}

/// POST /:repo/chunks/upload-urls — Mint presigned PUT URLs for a batch of chunk objects.
///
/// Request: `{ chunk_ids: [hex, ...], sizes: {hex: u64, ...} }`
/// Response: `{ hex: { url, method, required_headers } | null, ... }`
pub async fn presign_chunk_uploads(
    Path(repo): Path<String>,
    State(state): State<Arc<AppState>>,
    auth_user: Option<Extension<AuthUser>>,
    Json(req): Json<PresignChunkUploadsRequest>,
) -> Result<Json<std::collections::HashMap<String, Option<PresignedPutJson>>>, StatusCode> {
    check_permission(auth_user.as_deref(), "repo:write", state.is_auth_enabled())?;
    let repo_path = state.repos_dir.join(&repo);
    if !repo_path.exists() {
        return Err(StatusCode::NOT_FOUND);
    }
    let storage = get_or_init_storage(&state, &repo_path).await?;
    let ttl = std::time::Duration::from_secs(state.presigned_url_ttl_secs);
    let presign_concurrency: usize = std::env::var("MEDIAGIT_PRESIGN_CONCURRENCY")
        .ok()
        .and_then(|s| s.parse().ok())
        .filter(|n: &usize| *n > 0)
        .unwrap_or(64);

    let count = req.chunk_ids.len();
    let pairs: Vec<(String, u64)> = req
        .chunk_ids
        .into_iter()
        .map(|id| {
            let size = req.sizes.get(&id).copied().unwrap_or(0);
            (id, size)
        })
        .collect();

    let entries: Vec<(String, Option<PresignedPutJson>)> =
        futures::stream::iter(pairs.into_iter().map(|(chunk_id, _content_length)| {
            let storage = Arc::clone(&storage);
            let repo = repo.clone();
            async move {
                let key = format!("chunks/{}", chunk_id);
                // Pass 0 — chunk compressed size is unknown at presign time, and binding
                // the uncompressed manifest size would cause 403 SignatureDoesNotMatch
                // when compressed bytes are PUT for compressible content.
                let entry = match storage.presign_put(&key, 0, ttl).await {
                    Ok(Some(p)) => Some(PresignedPutJson {
                        url: p.url,
                        method: p.method,
                        required_headers: p
                            .required_headers
                            .into_iter()
                            .map(|(k, v)| [k, v])
                            .collect(),
                    }),
                    Ok(None) => None,
                    Err(e) => {
                        tracing::warn!(
                            repo = %repo,
                            chunk = %chunk_id,
                            err = %e,
                            "presign_put failed; client will fall back to proxy upload"
                        );
                        None
                    }
                };
                (chunk_id, entry)
            }
        }))
        .buffer_unordered(presign_concurrency)
        .collect()
        .await;

    let urls: std::collections::HashMap<String, Option<PresignedPutJson>> =
        entries.into_iter().collect();

    tracing::info!(
        repo = %repo,
        count = count,
        "Presigned chunk upload URLs generated"
    );
    Ok(Json(urls))
}

#[derive(serde::Deserialize)]
pub struct PresignDownloadUrlsRequest {
    pub chunks: Vec<String>,
}

#[derive(serde::Serialize)]
pub struct PresignedGetJson {
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub method: String,
    pub expires_in_secs: u64,
}

/// POST /:repo/chunks/download-urls — Mint presigned GET URLs for a batch of chunks.
///
/// Request: `{ chunks: [hex, ...] }`
/// Response: `{ hex: { url, headers, method, expires_in_secs } | null, ... }`
///
/// A `null` entry means either the backend does not support presigning or the
/// chunk does not exist yet; the client must fall back to `GET /chunks/:id`.
pub async fn presign_chunk_downloads(
    Path(repo): Path<String>,
    State(state): State<Arc<AppState>>,
    auth_user: Option<Extension<AuthUser>>,
    Json(req): Json<PresignDownloadUrlsRequest>,
) -> Result<Json<std::collections::HashMap<String, Option<PresignedGetJson>>>, StatusCode> {
    check_permission(auth_user.as_deref(), "repo:read", state.is_auth_enabled())?;

    let repo_path = state.repos_dir.join(&repo);
    if !repo_path.exists() {
        return Err(StatusCode::NOT_FOUND);
    }
    let storage = get_or_init_storage(&state, &repo_path).await?;
    let ttl = std::time::Duration::from_secs(state.presigned_url_ttl_secs);

    // Presigning is a local crypto operation — no network calls needed.
    // Skip the per-chunk exists() check (would cost one S3 HEAD per chunk = O(n) latency).
    // Manifest invariant: chunks listed in a manifest were written before the push completed.
    // If a presigned URL 404s the client falls back to proxy GET automatically.
    //
    // Run all presign_get calls concurrently — even "local" AWS SDK signing routes through the
    // async identity resolver and can cost 20–50 ms each; sequential over 3000+ chunks = minutes.
    let presign_concurrency: usize = std::env::var("MEDIAGIT_PRESIGN_CONCURRENCY")
        .ok()
        .and_then(|s| s.parse().ok())
        .filter(|n: &usize| *n > 0)
        .unwrap_or(64);

    let count = req.chunks.len();
    let entries: Vec<(String, Option<PresignedGetJson>)> =
        futures::stream::iter(req.chunks.into_iter().map(|chunk_id| {
            let storage = Arc::clone(&storage);
            let repo = repo.clone();
            async move {
                let key = format!("chunks/{}", chunk_id);
                let entry = match storage.presign_get(&key, ttl).await {
                    Ok(Some(p)) => Some(PresignedGetJson {
                        url: p.url,
                        headers: p.headers,
                        method: "GET".to_string(),
                        expires_in_secs: p.expires_in_secs,
                    }),
                    Ok(None) => None,
                    Err(e) => {
                        tracing::warn!(
                            repo = %repo,
                            chunk = %chunk_id,
                            err = %e,
                            "presign_get failed; client will fall back to proxy download"
                        );
                        None
                    }
                };
                (chunk_id, entry)
            }
        }))
        .buffer_unordered(presign_concurrency)
        .collect()
        .await;

    let result: std::collections::HashMap<String, Option<PresignedGetJson>> =
        entries.into_iter().collect();

    tracing::debug!(
        repo = %repo,
        count = count,
        "Presigned chunk download URLs generated"
    );
    Ok(Json(result))
}

#[derive(serde::Deserialize)]
pub struct CompleteUploadRequest {
    chunk_ids: Vec<String>,
}

#[derive(serde::Serialize)]
pub struct CompleteUploadResponse {
    missing: Vec<String>,
}

/// POST /:repo/chunks/complete — Verify a batch of presigned chunk uploads landed in storage.
///
/// Request: `{ chunk_ids: [hex, ...] }`
/// Response: `{ missing: [hex, ...] }` — chunks the client must retry via `PUT /chunks/:id`.
pub async fn complete_chunk_uploads(
    Path(repo): Path<String>,
    State(state): State<Arc<AppState>>,
    auth_user: Option<Extension<AuthUser>>,
    Json(req): Json<CompleteUploadRequest>,
) -> Result<Json<CompleteUploadResponse>, StatusCode> {
    check_permission(auth_user.as_deref(), "repo:write", state.is_auth_enabled())?;

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

    let in_pack_set: std::collections::HashSet<String> = {
        let idx = state.pack_index.read().await;
        idx.get(&repo)
            .map(|m| m.keys().cloned().collect())
            .unwrap_or_default()
    };

    let missing: Vec<String> = futures::stream::iter(req.chunk_ids)
        .map(|chunk_id_hex| {
            let storage = Arc::clone(&storage);
            let in_pack = in_pack_set.contains(&chunk_id_hex);
            async move {
                if in_pack {
                    return None;
                }
                let key = format!("chunks/{}", chunk_id_hex);
                match storage.head(&key).await {
                    Ok(Some(n)) if n > 0 => None,
                    _ => Some(chunk_id_hex),
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
        "Chunk upload completion verified"
    );
    Ok(Json(CompleteUploadResponse { missing }))
}

/// POST /:repo/chunks/verify-integrity — strong per-chunk BLAKE3 verification (opt-in).
///
/// Reads each chunk from storage, decompresses it, and asserts BLAKE3(uncompressed) == chunk_id.
/// Only called by clients that set `MEDIAGIT_STRONG_VERIFY=1`.
/// Returns `{ invalid: [hex, ...] }` for any chunk whose content does not match its claimed id.
#[derive(serde::Deserialize)]
pub struct VerifyIntegrityRequest {
    chunk_ids: Vec<String>,
}

#[derive(serde::Serialize)]
pub struct VerifyIntegrityResponse {
    invalid: Vec<String>,
}

pub async fn verify_chunk_integrity(
    Path(repo): Path<String>,
    State(state): State<Arc<AppState>>,
    auth_user: Option<Extension<AuthUser>>,
    Json(req): Json<VerifyIntegrityRequest>,
) -> Result<Json<VerifyIntegrityResponse>, StatusCode> {
    check_permission(auth_user.as_deref(), "repo:read", state.is_auth_enabled())?;

    let repo_path = state.repos_dir.join(&repo);
    if !repo_path.exists() {
        return Err(StatusCode::NOT_FOUND);
    }
    let storage = get_or_init_storage(&state, &repo_path).await?;
    let compressor = std::sync::Arc::new(SmartCompressor::new());

    let invalid: Vec<String> = futures::stream::iter(req.chunk_ids)
        .map(|chunk_id_hex| {
            let storage = Arc::clone(&storage);
            let compressor = std::sync::Arc::clone(&compressor);
            async move {
                let key = format!("chunks/{}", chunk_id_hex);
                let compressed = match storage.get(&key).await {
                    Ok(data) => data,
                    Err(_) => return Some(chunk_id_hex),
                };
                let decompressed =
                    match tokio::task::spawn_blocking(move || compressor.decompress(&compressed))
                        .await
                    {
                        Ok(Ok(data)) => data,
                        _ => return Some(chunk_id_hex),
                    };
                let hash_hex = blake3::hash(&decompressed).to_hex().to_string();
                if hash_hex == chunk_id_hex {
                    None
                } else {
                    Some(chunk_id_hex)
                }
            }
        })
        .buffer_unordered(20)
        .filter_map(|x| async { x })
        .collect()
        .await;

    tracing::debug!(
        repo = %repo,
        invalid_count = invalid.len(),
        "Chunk integrity verified"
    );
    Ok(Json(VerifyIntegrityResponse { invalid }))
}

// ============================================================================
// MPU Endpoints — Presigned multipart upload orchestration (S3 / MinIO)
// ============================================================================

#[derive(serde::Deserialize)]
pub struct MpuStartRequest {
    chunk_id: String,
    chunk_size: u64,
}

#[derive(serde::Serialize)]
pub struct MpuStartResponse {
    upload_id: String,
    parts: Vec<MpuPartUrl>,
    part_size: u64,
}

#[derive(serde::Serialize)]
pub struct MpuPartUrl {
    part_number: i32,
    url: String,
}

#[derive(serde::Deserialize)]
pub struct MpuCompleteRequest {
    chunk_id: String,
    upload_id: String,
    parts: Vec<MpuCompletedPartJson>,
}

#[derive(serde::Deserialize)]
pub struct MpuCompletedPartJson {
    part_number: i32,
    etag: String,
}

#[derive(serde::Deserialize)]
pub struct MpuAbortRequest {
    chunk_id: String,
    upload_id: String,
}

/// POST /:repo/chunks/mpu/start — Initiate a presigned multipart upload for one chunk.
///
/// Request: `{ chunk_id: hex, chunk_size: u64 }`
/// Response: `{ upload_id, parts: [{ part_number, url }], part_size }` — or 501 when the backend
/// does not support MPU (client should fall back to single-PUT presigned URL).
pub async fn mpu_start(
    Path(repo): Path<String>,
    State(state): State<Arc<AppState>>,
    auth_user: Option<Extension<AuthUser>>,
    Json(req): Json<MpuStartRequest>,
) -> Result<Json<MpuStartResponse>, StatusCode> {
    check_permission(auth_user.as_deref(), "repo:write", state.is_auth_enabled())?;

    let repo_path = state.repos_dir.join(&repo);
    if !repo_path.exists() {
        return Err(StatusCode::NOT_FOUND);
    }
    let storage = get_or_init_storage(&state, &repo_path).await?;
    let ttl = std::time::Duration::from_secs(state.presigned_url_ttl_secs);
    let key = format!("chunks/{}", req.chunk_id);

    match storage
        .create_presigned_mpu(&key, req.chunk_size, ttl)
        .await
    {
        Ok(Some(mpu)) => {
            tracing::info!(
                repo = %repo,
                chunk = %req.chunk_id,
                parts = mpu.parts.len(),
                "MPU initiated"
            );
            Ok(Json(MpuStartResponse {
                upload_id: mpu.upload_id,
                parts: mpu
                    .parts
                    .into_iter()
                    .map(|p| MpuPartUrl {
                        part_number: p.part_number,
                        url: p.url,
                    })
                    .collect(),
                part_size: mpu.part_size,
            }))
        }
        Ok(None) => {
            tracing::debug!(
                repo = %repo,
                chunk = %req.chunk_id,
                "Backend does not support MPU"
            );
            Err(StatusCode::NOT_IMPLEMENTED)
        }
        Err(e) => {
            let msg = e.to_string().to_lowercase();
            let status = if msg.contains("dispatch failure")
                || msg.contains("connection refused")
                || msg.contains("timeout")
            {
                // Storage backend unreachable; client should fall back to single-PUT presigned URL.
                StatusCode::SERVICE_UNAVAILABLE
            } else {
                StatusCode::INTERNAL_SERVER_ERROR
            };
            tracing::warn!(repo = %repo, chunk = %req.chunk_id, err = %e, %status, "mpu_start failed");
            Err(status)
        }
    }
}

/// POST /:repo/chunks/mpu/complete — Finalize a presigned multipart upload.
///
/// Request: `{ chunk_id: hex, upload_id, parts: [{ part_number, etag }] }`
/// Response: 204 No Content on success.
pub async fn mpu_complete(
    Path(repo): Path<String>,
    State(state): State<Arc<AppState>>,
    auth_user: Option<Extension<AuthUser>>,
    Json(req): Json<MpuCompleteRequest>,
) -> Result<impl IntoResponse, StatusCode> {
    check_permission(auth_user.as_deref(), "repo:write", state.is_auth_enabled())?;

    let repo_path = state.repos_dir.join(&repo);
    if !repo_path.exists() {
        return Err(StatusCode::NOT_FOUND);
    }
    let storage = get_or_init_storage(&state, &repo_path).await?;
    let key = format!("chunks/{}", req.chunk_id);

    let parts = req
        .parts
        .into_iter()
        .map(|p| mediagit_storage::MpuCompletedPart {
            part_number: p.part_number,
            etag: p.etag,
        })
        .collect();

    storage
        .complete_presigned_mpu(&key, &req.upload_id, parts)
        .await
        .map_err(|e| {
            tracing::warn!(repo = %repo, chunk = %req.chunk_id, err = %e, "mpu_complete failed");
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    tracing::debug!(repo = %repo, chunk = %req.chunk_id, "MPU completed");
    Ok(StatusCode::NO_CONTENT)
}

/// POST /:repo/chunks/mpu/abort — Abort a presigned multipart upload, freeing uncommitted parts.
///
/// Request: `{ chunk_id: hex, upload_id }`
/// Response: 204 No Content (idempotent).
pub async fn mpu_abort(
    Path(repo): Path<String>,
    State(state): State<Arc<AppState>>,
    auth_user: Option<Extension<AuthUser>>,
    Json(req): Json<MpuAbortRequest>,
) -> Result<impl IntoResponse, StatusCode> {
    check_permission(auth_user.as_deref(), "repo:write", state.is_auth_enabled())?;

    let repo_path = state.repos_dir.join(&repo);
    if !repo_path.exists() {
        return Err(StatusCode::NOT_FOUND);
    }
    let storage = get_or_init_storage(&state, &repo_path).await?;
    let key = format!("chunks/{}", req.chunk_id);

    let _ = storage.abort_presigned_mpu(&key, &req.upload_id).await;

    tracing::debug!(repo = %repo, chunk = %req.chunk_id, "MPU aborted");
    Ok(StatusCode::NO_CONTENT)
}

#[derive(serde::Deserialize)]
pub struct PresignPackDownloadRequest {
    pub pack_ids: Vec<String>,
}

/// POST /{repo}/packs/presign-download-urls — Mint presigned GET URLs for pack objects.
///
/// Returns one URL per pack_id. Client uses these for Range-GET reconstruction.
pub async fn presign_pack_downloads(
    Path(repo): Path<String>,
    State(state): State<Arc<AppState>>,
    auth_user: Option<Extension<AuthUser>>,
    Json(req): Json<PresignPackDownloadRequest>,
) -> Result<Json<std::collections::HashMap<String, Option<PresignedGetJson>>>, StatusCode> {
    check_permission(auth_user.as_deref(), "repo:read", state.is_auth_enabled())?;

    crate::security::validate_repo_name(&repo).map_err(|_| StatusCode::BAD_REQUEST)?;
    let repo_path = state.repos_dir.join(&repo);
    if !repo_path.exists() {
        return Err(StatusCode::NOT_FOUND);
    }
    let storage = get_or_init_storage(&state, &repo_path).await?;
    let ttl = std::time::Duration::from_secs(state.presigned_url_ttl_secs);

    let presign_concurrency: usize = std::env::var("MEDIAGIT_PRESIGN_CONCURRENCY")
        .ok()
        .and_then(|s| s.parse().ok())
        .filter(|n: &usize| *n > 0)
        .unwrap_or(64);

    let entries: Vec<(String, Option<PresignedGetJson>)> =
        futures::stream::iter(req.pack_ids.into_iter().map(|pack_id| {
            let storage = Arc::clone(&storage);
            let repo = repo.clone();
            async move {
                let key = format!("packs/{}", pack_id);
                let entry = match storage.presign_get(&key, ttl).await {
                    Ok(Some(p)) => Some(PresignedGetJson {
                        url: p.url,
                        headers: p.headers,
                        method: "GET".to_string(),
                        expires_in_secs: p.expires_in_secs,
                    }),
                    Ok(None) => None,
                    Err(e) => {
                        tracing::warn!(
                            repo = %repo,
                            pack = %pack_id,
                            err = %e,
                            "presign_get failed for pack download"
                        );
                        None
                    }
                };
                (pack_id, entry)
            }
        }))
        .buffer_unordered(presign_concurrency)
        .collect()
        .await;

    Ok(Json(entries.into_iter().collect()))
}
