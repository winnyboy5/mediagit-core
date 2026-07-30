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
    check_permission(
        auth_user.as_deref(),
        "repo:write",
        state.is_auth_enabled(),
        &state.grants,
        &repo,
    )?;

    if !req.pack_ids.iter().all(|id| is_hex_str(id)) {
        tracing::warn!(repo = %repo, "Rejecting presign_pack_uploads: a pack_id is not hex");
        return Err(StatusCode::BAD_REQUEST);
    }

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
    check_permission(
        auth_user.as_deref(),
        "repo:write",
        state.is_auth_enabled(),
        &state.grants,
        &repo,
    )?;

    if !req.chunk_ids.iter().all(|id| is_hex_str(id)) {
        tracing::warn!(repo = %repo, "Rejecting presign_chunk_uploads: a chunk_id is not hex");
        return Err(StatusCode::BAD_REQUEST);
    }

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
        futures::stream::iter(pairs.into_iter().map(|(chunk_id, content_length)| {
            let storage = Arc::clone(&storage);
            let repo = repo.clone();
            async move {
                let key = format!("chunks/{}", chunk_id);
                // content_length is the compressed length the client will actually
                // PUT (Odb::compressed_chunk_len), not the manifest's uncompressed
                // size. 0 means it couldn't be known cheaply (delta-encoded or
                // gc-repacked chunk) — the URL is intentionally left unbound.
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
    check_permission(
        auth_user.as_deref(),
        "repo:read",
        state.is_auth_enabled(),
        &state.grants,
        &repo,
    )?;

    if !req.chunks.iter().all(|id| is_hex_str(id)) {
        tracing::warn!(repo = %repo, "Rejecting presign_chunk_downloads: a chunk id is not hex");
        return Err(StatusCode::BAD_REQUEST);
    }

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

/// Read one chunk's bytes from storage — a loose object first, falling back
/// to a cloud pack slice on a loose miss — and verify BLAKE3(decompressed)
/// matches `chunk_id_hex` via [`verify_chunk_content`].
///
/// Shared by `complete_chunk_uploads` (server-enforced check after a
/// presigned upload) and `verify_chunk_integrity` (client-triggered strong
/// verify): same read-then-verify rule, one implementation.
///
/// Returns `Ok(bytes_read)` when the chunk is present and valid. Returns
/// `Err(pack_loc)` when the chunk is missing everywhere or its content does
/// not match its id — `pack_loc` is `Some` only when the chunk was found (but
/// invalid) inside a pack, which callers need to evict the right manifest.
async fn read_and_verify_chunk(
    storage: &Arc<dyn StorageBackend>,
    state: &Arc<AppState>,
    repo: &str,
    compressor: &Arc<SmartCompressor>,
    chunk_id_hex: &str,
) -> Result<u64, Option<PackLoc>> {
    let key = format!("chunks/{}", chunk_id_hex);
    let (bytes, pack_loc) = match storage.get(&key).await {
        Ok(data) => (Some(data), None),
        Err(_) => {
            // Loose miss — the chunk may live only inside a cloud pack.
            let loc = {
                let idx = state.pack_index.read().await;
                idx.get(repo).and_then(|m| m.get(chunk_id_hex)).cloned()
            };
            match &loc {
                Some(l) if l.length >= 5 => {
                    let pack_key = format!("packs/{}", l.pack_oid);
                    // Skip 5-byte pack entry header [type:1][size:4].
                    let data_offset = l.offset + 5;
                    let data_len = (l.length as u64) - 5;
                    match storage.get_range(&pack_key, data_offset, data_len).await {
                        Ok(data) => (Some(data), loc),
                        Err(_) => (None, loc),
                    }
                }
                _ => (None, loc),
            }
        }
    };

    let Some(compressed) = bytes else {
        return Err(pack_loc);
    };
    let len = compressed.len() as u64;
    if verify_chunk_content(compressor, chunk_id_hex, Bytes::from(compressed)).await {
        Ok(len)
    } else {
        Err(pack_loc)
    }
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
    check_permission(
        auth_user.as_deref(),
        "repo:write",
        state.is_auth_enabled(),
        &state.grants,
        &repo,
    )?;

    if !req.chunk_ids.iter().all(|id| is_valid_hex_id(id)) {
        tracing::warn!(repo = %repo, "Rejecting complete_chunk_uploads: a chunk_id is not a 64-char hex id");
        return Err(StatusCode::BAD_REQUEST);
    }

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

    // Server-enforced content verification (default ON — see
    // `ServerConfig::verify_content_on_complete`). Presigned uploads go
    // client→bucket directly, so a mere `head()` existence check (the `else`
    // branch below) accepts any bytes under a claimed id. This is the only
    // point the server can catch that: read every completed chunk back,
    // decompress, and compare BLAKE3 to its claimed id via the same
    // `read_and_verify_chunk` helper `verify_chunk_integrity` uses.
    let verify_enabled = state.verify_chunks_on_complete;
    let compressor = Arc::new(SmartCompressor::new());
    let verify_start = std::time::Instant::now();
    let chunk_count = req.chunk_ids.len();
    let verified_bytes = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));

    let missing: Vec<String> = futures::stream::iter(req.chunk_ids)
        .map(|chunk_id_hex| {
            let storage = Arc::clone(&storage);
            let in_pack = in_pack_set.contains(&chunk_id_hex);
            let compressor = Arc::clone(&compressor);
            let state = Arc::clone(&state);
            let repo = repo.clone();
            let verified_bytes = Arc::clone(&verified_bytes);
            async move {
                if in_pack {
                    return None;
                }
                if !verify_enabled {
                    let key = format!("chunks/{}", chunk_id_hex);
                    return match storage.head(&key).await {
                        Ok(Some(n)) if n > 0 => None,
                        _ => Some(chunk_id_hex),
                    };
                }
                match read_and_verify_chunk(&storage, &state, &repo, &compressor, &chunk_id_hex)
                    .await
                {
                    Ok(len) => {
                        verified_bytes.fetch_add(len, std::sync::atomic::Ordering::Relaxed);
                        None
                    }
                    Err(_) => Some(chunk_id_hex),
                }
            }
        })
        .buffer_unordered(50)
        .filter_map(|x| async { x })
        .collect()
        .await;

    if verify_enabled {
        tracing::info!(
            repo = %repo,
            chunk_count = chunk_count,
            verified_bytes = verified_bytes.load(std::sync::atomic::Ordering::Relaxed),
            missing_count = missing.len(),
            elapsed_ms = verify_start.elapsed().as_millis() as u64,
            "Chunk content verification on complete finished"
        );
    } else {
        tracing::debug!(
            repo = %repo,
            missing_count = missing.len(),
            "Chunk upload completion verified (existence-only; content verification disabled)"
        );
    }
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
    /// Evict invalid entries found in a cloud pack from the pack index so
    /// subsequent locate/get calls fall through to a loose re-upload instead
    /// of repeatedly serving corrupt bytes from the pack (QA-013 A1).
    #[serde(default)]
    evict_invalid: bool,
}

#[derive(serde::Serialize)]
pub struct VerifyIntegrityResponse {
    invalid: Vec<String>,
    /// Chunk ids that were invalid, found in a pack, and evicted from the
    /// pack index (only populated when `evict_invalid` was set).
    #[serde(default)]
    evicted: Vec<String>,
}

pub async fn verify_chunk_integrity(
    Path(repo): Path<String>,
    State(state): State<Arc<AppState>>,
    auth_user: Option<Extension<AuthUser>>,
    Json(req): Json<VerifyIntegrityRequest>,
) -> Result<Json<VerifyIntegrityResponse>, StatusCode> {
    check_permission(
        auth_user.as_deref(),
        "repo:read",
        state.is_auth_enabled(),
        &state.grants,
        &repo,
    )?;

    if !req.chunk_ids.iter().all(|id| is_valid_hex_id(id)) {
        tracing::warn!(repo = %repo, "Rejecting verify_chunk_integrity: a chunk_id is not a 64-char hex id");
        return Err(StatusCode::BAD_REQUEST);
    }

    let repo_path = state.repos_dir.join(&repo);
    if !repo_path.exists() {
        return Err(StatusCode::NOT_FOUND);
    }
    let storage = get_or_init_storage(&state, &repo_path).await?;
    let compressor = std::sync::Arc::new(SmartCompressor::new());

    // Lazy-load pack index from JSONL if not yet warm (mirrors locate_chunks).
    // Without this, a verify on a freshly restarted server sees an empty index,
    // misses packed chunks, and never evicts poisoned entries (CHK20).
    {
        let idx = state.pack_index.read().await;
        if !idx.contains_key(&repo) {
            drop(idx);
            crate::handlers::load_jsonl_index(&state, &repo, &repo_path).await?;
        }
    }

    // Each result is (chunk_id, pack_loc) where pack_loc is Some(..) when the
    // chunk was (attempted to be) served from a cloud pack rather than a loose
    // object — needed below to evict the entry from the right pack manifest.
    let results: Vec<(String, Option<PackLoc>)> = futures::stream::iter(req.chunk_ids)
        .map(|chunk_id_hex| {
            let storage = Arc::clone(&storage);
            let compressor = std::sync::Arc::clone(&compressor);
            let state = Arc::clone(&state);
            let repo = repo.clone();
            async move {
                match read_and_verify_chunk(&storage, &state, &repo, &compressor, &chunk_id_hex)
                    .await
                {
                    Ok(_len) => None,
                    Err(pack_loc) => Some((chunk_id_hex, pack_loc)),
                }
            }
        })
        .buffer_unordered(20)
        .filter_map(|x| async { x })
        .collect()
        .await;

    let invalid: Vec<String> = results.iter().map(|(id, _)| id.clone()).collect();

    let evict_enabled = req.evict_invalid
        && std::env::var("MEDIAGIT_REPAIR_EVICT_PACK_ENTRIES")
            .as_deref()
            .unwrap_or("1")
            != "0";

    let mut evicted: Vec<String> = Vec::new();
    if evict_enabled {
        // Group by pack_oid so each pack's manifest is rewritten at most once.
        let mut by_pack: std::collections::HashMap<String, Vec<String>> =
            std::collections::HashMap::new();
        for (id, loc) in results {
            if let Some(loc) = loc {
                by_pack.entry(loc.pack_oid).or_default().push(id);
            }
        }
        for (pack_oid, ids) in by_pack {
            match evict_pack_entries(&state, &repo_path, &repo, &pack_oid, &ids).await {
                Ok(()) => evicted.extend(ids),
                Err(status) => tracing::warn!(
                    repo = %repo,
                    pack = %pack_oid,
                    ?status,
                    "Failed to evict invalid pack entries"
                ),
            }
        }
    }

    tracing::debug!(
        repo = %repo,
        invalid_count = invalid.len(),
        evicted_count = evicted.len(),
        "Chunk integrity verified"
    );
    Ok(Json(VerifyIntegrityResponse { invalid, evicted }))
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
    check_permission(
        auth_user.as_deref(),
        "repo:write",
        state.is_auth_enabled(),
        &state.grants,
        &repo,
    )?;

    if !is_hex_str(&req.chunk_id) {
        tracing::warn!(repo = %repo, chunk_id = %req.chunk_id, "Rejecting mpu_start: chunk_id is not hex");
        return Err(StatusCode::BAD_REQUEST);
    }

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
    check_permission(
        auth_user.as_deref(),
        "repo:write",
        state.is_auth_enabled(),
        &state.grants,
        &repo,
    )?;

    if !is_hex_str(&req.chunk_id) {
        tracing::warn!(repo = %repo, chunk_id = %req.chunk_id, "Rejecting mpu_complete: chunk_id is not hex");
        return Err(StatusCode::BAD_REQUEST);
    }

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
    check_permission(
        auth_user.as_deref(),
        "repo:write",
        state.is_auth_enabled(),
        &state.grants,
        &repo,
    )?;

    if !is_hex_str(&req.chunk_id) {
        tracing::warn!(repo = %repo, chunk_id = %req.chunk_id, "Rejecting mpu_abort: chunk_id is not hex");
        return Err(StatusCode::BAD_REQUEST);
    }

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
    check_permission(
        auth_user.as_deref(),
        "repo:read",
        state.is_auth_enabled(),
        &state.grants,
        &repo,
    )?;

    crate::security::validate_repo_name(&repo).map_err(|_| StatusCode::BAD_REQUEST)?;

    if !req.pack_ids.iter().all(|id| is_hex_str(id)) {
        tracing::warn!(repo = %repo, "Rejecting presign_pack_downloads: a pack_id is not hex");
        return Err(StatusCode::BAD_REQUEST);
    }

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

#[derive(serde::Deserialize)]
pub struct VerifyObjectsRequest {
    oids: Vec<String>,
    /// Delete invalid loose objects so a follow-up re-push isn't skipped by
    /// `odb.write`'s exists()-dedup (QA-013 object repair).
    #[serde(default)]
    evict_invalid: bool,
}

#[derive(serde::Serialize)]
pub struct VerifyObjectsResponse {
    invalid: Vec<String>,
    evicted: Vec<String>,
}

/// POST /:repo/objects/verify-integrity — read + BLAKE3 re-hash whole objects
/// (commits/trees/blobs) through the server's ODB. Complements the chunk-level
/// verify above: `push --repair` needs this for blobs stored un-chunked, which
/// the chunk-manifest walk never sees (CHK20's poisoned object was one).
pub async fn verify_object_integrity(
    Path(repo): Path<String>,
    State(state): State<Arc<AppState>>,
    auth_user: Option<Extension<AuthUser>>,
    Json(req): Json<VerifyObjectsRequest>,
) -> Result<Json<VerifyObjectsResponse>, StatusCode> {
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
    let odb = std::sync::Arc::new(get_or_init_odb(&state, &repo_path).await?);

    let invalid: Vec<String> = futures::stream::iter(req.oids)
        .map(|oid_hex| {
            let odb = std::sync::Arc::clone(&odb);
            async move {
                let Ok(oid) = mediagit_versioning::Oid::from_hex(&oid_hex) else {
                    return Some(oid_hex);
                };
                match odb.read(&oid).await {
                    Ok(data) if blake3::hash(&data).to_hex().to_string() == oid_hex => None,
                    _ => Some(oid_hex),
                }
            }
        })
        .buffer_unordered(20)
        .filter_map(|r| async move { r })
        .collect()
        .await;

    let evict_enabled = req.evict_invalid
        && std::env::var("MEDIAGIT_REPAIR_EVICT_PACK_ENTRIES")
            .as_deref()
            .unwrap_or("1")
            != "0";

    let mut evicted = Vec::new();
    if evict_enabled {
        for oid_hex in &invalid {
            if let Ok(oid) = mediagit_versioning::Oid::from_hex(oid_hex) {
                match odb.delete_object(&oid).await {
                    Ok(()) => evicted.push(oid_hex.clone()),
                    Err(e) => {
                        tracing::warn!(oid = %oid_hex, error = %e, "Failed to evict invalid object")
                    }
                }
            }
        }
    }

    tracing::debug!(
        repo = %repo,
        invalid_count = invalid.len(),
        evicted_count = evicted.len(),
        "Object integrity verified"
    );
    Ok(Json(VerifyObjectsResponse { invalid, evicted }))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Serializes access to `MEDIAGIT_REPAIR_EVICT_PACK_ENTRIES` across the
    /// tests below — env vars are process-global and tests run concurrently
    /// under `#[tokio::test]` in the same binary.
    static EVICT_ENV_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

    /// Writes one packed chunk whose compressed bytes are corrupted — it will
    /// not decompress back to content matching its claimed chunk id — and
    /// registers it in both the in-memory pack_index and the persisted JSONL
    /// manifest, mirroring what `complete_pack` writes for a real pack.
    /// Returns the chunk's claimed (now-invalid) id.
    async fn write_corrupt_pack_entry(
        state: &AppState,
        storage: &LocalBackend,
        repo: &str,
        repo_path: &std::path::Path,
        pack_oid: &str,
    ) -> String {
        let content = b"the quick brown fox jumps over the lazy dog".to_vec();
        let chunk_id = blake3::hash(&content).to_hex().to_string();
        let compressor = SmartCompressor::new();
        let mut compressed = compressor.compress(&content).expect("compress");
        // Corrupt the compressed payload so decompress() no longer round-trips
        // to content whose BLAKE3 matches chunk_id (or fails to decompress).
        let last = compressed.len() - 1;
        compressed[last] ^= 0xFF;

        // [type:1][size:4] pack entry header — server only skips these bytes,
        // content is irrelevant here.
        let mut pack_bytes = vec![0u8; 5];
        pack_bytes.extend_from_slice(&compressed);

        let pack_key = format!("packs/{}", pack_oid);
        storage.put(&pack_key, &pack_bytes).await.expect("put pack");

        let loc = PackLoc {
            pack_oid: pack_oid.to_string(),
            offset: 0,
            length: pack_bytes.len() as u32,
            compressed_hash: None,
        };
        {
            let mut idx = state.pack_index.write().await;
            idx.entry(repo.to_string())
                .or_default()
                .insert(chunk_id.clone(), loc);
        }

        let shard = &pack_oid[..2];
        let manifest_dir = repo_path.join(".mediagit").join("packs").join(shard);
        tokio::fs::create_dir_all(&manifest_dir).await.unwrap();
        let line = PackIndexLine {
            chunk_oid: chunk_id.clone(),
            pack_oid: pack_oid.to_string(),
            offset: 0,
            length: pack_bytes.len() as u32,
            compressed_hash: None,
        };
        let jsonl = format!("{}\n", serde_json::to_string(&line).unwrap());
        tokio::fs::write(manifest_dir.join(format!("{}.jsonl", pack_oid)), jsonl)
            .await
            .unwrap();

        chunk_id
    }

    #[tokio::test]
    async fn verify_detects_invalid_packed_chunk_and_evicts() {
        let _guard = EVICT_ENV_LOCK.lock().await;
        mediagit_test_utils::remove_var("MEDIAGIT_REPAIR_EVICT_PACK_ENTRIES"); // default: enabled

        let tmp = tempfile::tempdir().unwrap();
        let repo = "test-repo".to_string();
        let repo_path = tmp.path().join(&repo);
        tokio::fs::create_dir_all(&repo_path).await.unwrap();
        let storage = LocalBackend::new(repo_path.join(".mediagit"))
            .await
            .expect("local backend");
        let state = Arc::new(AppState::new(tmp.path().to_path_buf()));
        let chunk_id =
            write_corrupt_pack_entry(&state, &storage, &repo, &repo_path, "deadbeef00").await;

        let req = VerifyIntegrityRequest {
            chunk_ids: vec![chunk_id.clone()],
            evict_invalid: true,
        };
        let resp = verify_chunk_integrity(
            Path(repo.clone()),
            State(Arc::clone(&state)),
            None,
            Json(req),
        )
        .await
        .expect("handler ok")
        .0;

        assert_eq!(resp.invalid, vec![chunk_id.clone()]);
        assert_eq!(resp.evicted, vec![chunk_id.clone()]);

        // Evicted from the in-memory index.
        {
            let idx = state.pack_index.read().await;
            assert!(idx.get(&repo).unwrap().get(&chunk_id).is_none());
        }

        // Evicted from the persisted JSONL manifest too.
        let manifest_path = repo_path
            .join(".mediagit")
            .join("packs")
            .join("de")
            .join("deadbeef00.jsonl");
        let content = tokio::fs::read_to_string(&manifest_path).await.unwrap();
        assert!(!content.contains(&chunk_id));
    }

    #[tokio::test]
    async fn verify_reports_only_when_eviction_disabled_by_knob() {
        let _guard = EVICT_ENV_LOCK.lock().await;
        mediagit_test_utils::set_var("MEDIAGIT_REPAIR_EVICT_PACK_ENTRIES", "0");

        let tmp = tempfile::tempdir().unwrap();
        let repo = "test-repo".to_string();
        let repo_path = tmp.path().join(&repo);
        tokio::fs::create_dir_all(&repo_path).await.unwrap();
        let storage = LocalBackend::new(repo_path.join(".mediagit"))
            .await
            .expect("local backend");
        let state = Arc::new(AppState::new(tmp.path().to_path_buf()));
        let chunk_id =
            write_corrupt_pack_entry(&state, &storage, &repo, &repo_path, "cafebabe00").await;

        let req = VerifyIntegrityRequest {
            chunk_ids: vec![chunk_id.clone()],
            evict_invalid: true,
        };
        let resp = verify_chunk_integrity(
            Path(repo.clone()),
            State(Arc::clone(&state)),
            None,
            Json(req),
        )
        .await
        .expect("handler ok")
        .0;

        mediagit_test_utils::remove_var("MEDIAGIT_REPAIR_EVICT_PACK_ENTRIES");

        // Still reported invalid, but report-only: nothing evicted.
        assert_eq!(resp.invalid, vec![chunk_id.clone()]);
        assert!(resp.evicted.is_empty());

        let idx = state.pack_index.read().await;
        assert!(idx.get(&repo).unwrap().get(&chunk_id).is_some());
    }
}

#[cfg(test)]
mod complete_chunk_uploads_content_verification_tests {
    use super::*;

    /// Store a loose chunk at `chunks/<claimed_id>` whose content does NOT
    /// hash to `claimed_id` — simulates the presigned-upload hole this
    /// layer closes: bytes landed directly in the bucket (or were corrupted
    /// out-of-band) without ever passing through server-side content checks.
    async fn write_corrupt_loose_chunk(storage: &dyn StorageBackend, claimed_id: &str) {
        let content = b"real content that does not match claimed_id".to_vec();
        let compressor = SmartCompressor::new();
        let compressed = compressor.compress(&content).expect("compress");
        let key = format!("chunks/{}", claimed_id);
        storage
            .put(&key, &compressed)
            .await
            .expect("put corrupt loose chunk");
    }

    #[tokio::test]
    async fn complete_reports_corrupted_chunk_as_missing_when_verification_enabled() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = "test-repo".to_string();
        let repo_path = tmp.path().join(&repo);
        tokio::fs::create_dir_all(&repo_path).await.unwrap();
        // Opt in explicitly. Content verification defaults to OFF (see
        // `ServerConfig::verify_content_on_complete` for the measured reason), so a
        // test that wants it must ask for it — exactly as an operator must. This
        // assertion used to read "AppState::new must default content verification ON",
        // which was the test depending on an implicit default rather than stating its
        // own precondition.
        let state =
            Arc::new(AppState::new(tmp.path().to_path_buf()).with_verify_chunks_on_complete(true));
        assert!(
            state.verify_chunks_on_complete,
            "this test requires content verification enabled"
        );

        let storage = get_or_init_storage(&state, &repo_path)
            .await
            .expect("storage");
        let claimed_id = blake3::hash(b"a completely different payload")
            .to_hex()
            .to_string();
        write_corrupt_loose_chunk(storage.as_ref(), &claimed_id).await;

        let req = CompleteUploadRequest {
            chunk_ids: vec![claimed_id.clone()],
        };
        let resp = complete_chunk_uploads(Path(repo), State(Arc::clone(&state)), None, Json(req))
            .await
            .expect("handler ok")
            .0;

        assert_eq!(
            resp.missing,
            vec![claimed_id],
            "corrupted chunk must be reported missing so the client re-uploads it"
        );
    }

    #[tokio::test]
    async fn complete_does_not_report_corrupted_chunk_when_verification_disabled() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = "test-repo".to_string();
        let repo_path = tmp.path().join(&repo);
        tokio::fs::create_dir_all(&repo_path).await.unwrap();
        let mut state = AppState::new(tmp.path().to_path_buf());
        state.verify_chunks_on_complete = false;
        let state = Arc::new(state);

        let storage = get_or_init_storage(&state, &repo_path)
            .await
            .expect("storage");
        let claimed_id = blake3::hash(b"a completely different payload")
            .to_hex()
            .to_string();
        write_corrupt_loose_chunk(storage.as_ref(), &claimed_id).await;

        let req = CompleteUploadRequest {
            chunk_ids: vec![claimed_id.clone()],
        };
        let resp = complete_chunk_uploads(Path(repo), State(Arc::clone(&state)), None, Json(req))
            .await
            .expect("handler ok")
            .0;

        assert!(
            resp.missing.is_empty(),
            "with verification disabled the completion check must fall back to \
             existence-only (head), proving the knob actually does something"
        );
    }
}
