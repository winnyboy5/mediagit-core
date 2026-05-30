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

use axum::{
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    Extension, Json,
};
use bytes::Bytes;
use futures::stream::StreamExt;
use mediagit_compression::{Compressor, SmartCompressor};
use mediagit_protocol::{
    RefInfo, RefUpdateRequest, RefUpdateResponse, RefUpdateResult, RefsResponse, WantRequest,
    WantResponse,
};
use mediagit_security::auth::AuthUser;
use mediagit_storage::{AzureBackend, GcsBackend, LocalBackend, MinIOBackend, StorageBackend};
use mediagit_versioning::{
    resolve_revision, Commit, LcaFinder, ObjectDatabase, ObjectType, Oid, Ref, RefDatabase, Reflog,
    ReflogEntry, StreamingPackWriter, Tree,
};
use std::path::Path as StdPath;
use std::sync::Arc;
use tokio::io::duplex;
use tokio_util::io::ReaderStream;

use crate::state::{AppState, PackLoc};

/// Helper function to check if user has required permission
fn check_permission(
    auth_user: Option<&AuthUser>,
    required_permission: &str,
    auth_enabled: bool,
) -> Result<(), StatusCode> {
    // If auth is disabled, allow all requests
    if !auth_enabled {
        return Ok(());
    }

    // If auth is enabled but no user found, reject
    let user = auth_user.ok_or(StatusCode::UNAUTHORIZED)?;

    // Check if user has the required permission
    if user.permissions.contains(&required_permission.to_string()) {
        Ok(())
    } else {
        tracing::warn!(
            "User {} lacks permission: {}",
            user.user_id,
            required_permission
        );
        Err(StatusCode::FORBIDDEN)
    }
}

/// Per-handler entry: returns the cached storage backend for this repo,
/// constructing it on first use. Constructing a backend (especially Azure/S3)
/// is expensive — TLS handshake plus a bucket/container existence RTT — so we
/// build it once per repo per server lifetime and reuse the `Arc` from then on.
async fn get_or_init_storage(
    state: &AppState,
    repo_path: &StdPath,
) -> Result<Arc<dyn StorageBackend>, StatusCode> {
    let key = repo_path.to_path_buf();

    // Fast path: cached backend.
    if let Some(backend) = state.storage_backends.read().await.get(&key).cloned() {
        return Ok(backend);
    }

    // Slow path: take the write lock, double-check, build, insert.
    let mut map = state.storage_backends.write().await;
    if let Some(backend) = map.get(&key).cloned() {
        return Ok(backend);
    }
    let backend = build_storage_backend(repo_path).await?;
    map.insert(key, Arc::clone(&backend));
    Ok(backend)
}

/// Per-handler entry: returns a clone of the cached ObjectDatabase for this repo.
/// All clones share the same Arc<delta_written_pairs> HashSet, which is required
/// for the TOCTOU circular-delta-chain prevention guard to function correctly.
/// Without sharing, each concurrent handler has its own HashSet and the guard
/// is ineffective against parallel writers within the same pack upload.
async fn get_or_init_odb(
    state: &AppState,
    repo_path: &StdPath,
) -> Result<ObjectDatabase, StatusCode> {
    let key = repo_path.to_path_buf();

    // Fast path: cached ODB template — clone shares all Arc fields.
    if let Some(odb) = state.odb_cache.read().await.get(&key).cloned() {
        return Ok(odb);
    }

    // Slow path: build storage + ODB, double-checked.
    let mut map = state.odb_cache.write().await;
    if let Some(odb) = map.get(&key).cloned() {
        return Ok(odb);
    }
    let storage = get_or_init_storage(state, repo_path).await?;
    let odb = ObjectDatabase::with_smart_compression(storage, 1000);
    map.insert(key, odb.clone());
    Ok(odb)
}

/// Helper function to create storage backend based on repository configuration
async fn build_storage_backend(repo_path: &StdPath) -> Result<Arc<dyn StorageBackend>, StatusCode> {
    // Load repository configuration
    let config = mediagit_config::Config::load(repo_path)
        .await
        .map_err(|e| {
            tracing::error!("Failed to load repository config: {}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    // Create storage backend based on configuration
    let storage: Arc<dyn StorageBackend> = match &config.storage {
        mediagit_config::StorageConfig::FileSystem(fs_config) => {
            // Use configured base_path - it can be absolute or relative to repo
            let storage_path = if std::path::Path::new(&fs_config.base_path).is_absolute() {
                std::path::PathBuf::from(&fs_config.base_path)
            } else if fs_config.base_path == "./data" {
                // Default config value - use .mediagit instead
                repo_path.join(".mediagit")
            } else {
                repo_path.join(&fs_config.base_path)
            };
            tracing::debug!(
                "Using filesystem storage backend: {}",
                storage_path.display()
            );
            let storage = LocalBackend::new(&storage_path).await.map_err(|e| {
                tracing::error!("Failed to initialize filesystem backend: {}", e);
                StatusCode::INTERNAL_SERVER_ERROR
            })?;
            Arc::new(storage)
        }
        mediagit_config::StorageConfig::S3(s3_config) => {
            if s3_config.endpoint.is_some() {
                build_minio_compatible_storage(s3_config).await?
            } else {
                build_aws_s3_storage(s3_config).await?
            }
        }
        mediagit_config::StorageConfig::Azure(azure_config) => {
            tracing::info!(
                "Using Azure storage backend: container={} prefix='{}'",
                azure_config.container,
                azure_config.prefix
            );

            // Use connection string if provided, otherwise use account key.
            // Both paths now thread the configured `prefix` through so that
            // multiple repos sharing one container don't collide on identical
            // OIDs. (Pre-fix, prefix was silently ignored on put/get/exists/
            // delete and only honoured on list_objects — see C-BUG-AZURE-PREFIX.)
            let storage = if let Some(conn_str) = &azure_config.connection_string {
                AzureBackend::with_connection_string_and_prefix(
                    &azure_config.container,
                    conn_str,
                    &azure_config.prefix,
                )
                .await
                .map_err(|e| {
                    tracing::error!(
                        "Failed to initialize Azure backend with connection string: {}",
                        e
                    );
                    StatusCode::INTERNAL_SERVER_ERROR
                })?
            } else if let Some(account_key) = &azure_config.account_key {
                AzureBackend::with_account_key_and_prefix(
                    &azure_config.account_name,
                    &azure_config.container,
                    account_key,
                    &azure_config.prefix,
                )
                .await
                .map_err(|e| {
                    tracing::error!("Failed to initialize Azure backend with account key: {}", e);
                    StatusCode::INTERNAL_SERVER_ERROR
                })?
            } else {
                tracing::error!("Azure backend requires either connection_string or account_key");
                return Err(StatusCode::INTERNAL_SERVER_ERROR);
            };
            Arc::new(storage)
        }
        mediagit_config::StorageConfig::GCS(gcs_config) => {
            tracing::info!(
                "Using GCS storage backend: bucket={}, project={}",
                gcs_config.bucket,
                gcs_config.project_id
            );

            // Resolve credentials_path: absolute, ~-prefixed, or relative to repo dir.
            // None means fall back to ADC (GOOGLE_APPLICATION_CREDENTIALS or metadata server).
            let resolved_creds: Option<std::path::PathBuf> =
                gcs_config.credentials_path.as_deref().map(|raw| {
                    if let Some(rest) = raw.strip_prefix('~') {
                        let home = std::env::var("HOME")
                            .or_else(|_| std::env::var("USERPROFILE"))
                            .unwrap_or_default();
                        std::path::PathBuf::from(format!("{}{}", home, rest))
                    } else {
                        let p = std::path::Path::new(raw);
                        if p.is_absolute() {
                            p.to_path_buf()
                        } else {
                            repo_path.join(raw)
                        }
                    }
                });

            let storage = match resolved_creds {
                Some(path) => GcsBackend::new(&gcs_config.project_id, &gcs_config.bucket, &path)
                    .await
                    .map_err(|e| {
                        tracing::error!("Failed to initialize GCS backend: {}", e);
                        StatusCode::INTERNAL_SERVER_ERROR
                    })?,
                None => {
                    GcsBackend::with_default_credentials(&gcs_config.project_id, &gcs_config.bucket)
                        .await
                        .map_err(|e| {
                            tracing::error!(
                                "Failed to initialize GCS backend with default credentials: {}",
                                e
                            );
                            StatusCode::INTERNAL_SERVER_ERROR
                        })?
                }
            };

            Arc::new(storage)
        }
        mediagit_config::StorageConfig::Multi(_) => {
            tracing::error!("Multi-backend storage is not yet implemented");
            return Err(StatusCode::INTERNAL_SERVER_ERROR);
        }
    };

    Ok(storage)
}

/// MinIO / S3-compatible storage (MinIO, DigitalOcean Spaces, Cloudflare R2, etc.).
/// Used when the repo config has an explicit `endpoint` URL.
async fn build_minio_compatible_storage(
    s3_config: &mediagit_config::S3Storage,
) -> Result<Arc<dyn StorageBackend>, StatusCode> {
    let endpoint = s3_config.endpoint.as_deref().unwrap_or_default();
    tracing::info!(
        "Using MinIO/S3-compatible backend: bucket={}, endpoint={}, prefix='{}'",
        s3_config.bucket,
        endpoint,
        s3_config.prefix
    );
    MinIOBackend::new_with_prefix(
        endpoint,
        &s3_config.bucket,
        s3_config.access_key_id.as_deref().unwrap_or(""),
        s3_config.secret_access_key.as_deref().unwrap_or(""),
        &s3_config.prefix,
    )
    .await
    .map(|b| Arc::new(b) as Arc<dyn StorageBackend>)
    .map_err(|e| {
        tracing::error!("Failed to initialize MinIO backend: {:#}", e);
        StatusCode::INTERNAL_SERVER_ERROR
    })
}

/// Native AWS S3 storage.
/// Used when the repo config has no `endpoint` (i.e. real AWS, not an S3-compatible service).
/// Passes the correct region for SigV4 signing and uses virtual-hosted-style addressing.
async fn build_aws_s3_storage(
    s3_config: &mediagit_config::S3Storage,
) -> Result<Arc<dyn StorageBackend>, StatusCode> {
    tracing::info!(
        "Using AWS S3 backend: bucket={}, region={}, prefix='{}'",
        s3_config.bucket,
        s3_config.region,
        s3_config.prefix
    );
    let aws_config = mediagit_storage::minio::MinIOConfig {
        endpoint: format!("https://s3.{}.amazonaws.com", s3_config.region),
        bucket: s3_config.bucket.clone(),
        access_key: s3_config.access_key_id.as_deref().unwrap_or("").to_string(),
        secret_key: s3_config
            .secret_access_key
            .as_deref()
            .unwrap_or("")
            .to_string(),
        prefix: s3_config.prefix.clone(),
        region: s3_config.region.clone(),
        path_style: false,
        ..mediagit_storage::minio::MinIOConfig::default()
    };
    MinIOBackend::with_config(aws_config)
        .await
        .map(|b| Arc::new(b) as Arc<dyn StorageBackend>)
        .map_err(|e| {
            tracing::error!("Failed to initialize AWS S3 backend: {:#}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })
}

/// GET /:repo/info/refs - List all refs in the repository
pub async fn get_refs(
    Path(repo): Path<String>,
    State(state): State<Arc<AppState>>,
    auth_user: Option<Extension<AuthUser>>,
) -> Result<Json<RefsResponse>, StatusCode> {
    tracing::info!("GET /{}/info/refs", repo);

    // Validate repository name to prevent path traversal
    crate::security::validate_repo_name(&repo).map_err(|e| {
        tracing::warn!("Invalid repository name '{}': {}", repo, e);
        StatusCode::BAD_REQUEST
    })?;

    // Check permission: repo:read required
    check_permission(auth_user.as_deref(), "repo:read", state.is_auth_enabled())?;

    let repo_path = state.repos_dir.join(&repo);
    if !repo_path.exists() {
        tracing::warn!("Repository not found: {}", repo);
        return Err(StatusCode::NOT_FOUND);
    }

    // Initialize storage and refdb
    let _storage = get_or_init_storage(&state, &repo_path).await?;
    let refdb = RefDatabase::new(repo_path.join(".mediagit"));

    // List all refs by scanning refs directory
    let refs_dir = repo_path.join(".mediagit/refs");
    let mut ref_infos = Vec::new();

    // Read HEAD
    if let Ok(head) = refdb.read("HEAD").await {
        ref_infos.push(RefInfo {
            name: "HEAD".to_string(),
            oid: head.oid.map(|o| o.to_hex()).unwrap_or_default(),
            target: head.target,
        });
    }

    // Recursively read all refs in refs/heads, refs/tags, refs/remotes, etc.
    if refs_dir.exists() {
        // Use walkdir pattern to recursively traverse all ref directories
        let mut dirs_to_visit = vec![refs_dir.clone()];

        while let Some(current_dir) = dirs_to_visit.pop() {
            if let Ok(entries) = std::fs::read_dir(&current_dir) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    if path.is_dir() {
                        // Add subdirectory to visit (for nested refs like feature/branch or remotes/origin/main)
                        dirs_to_visit.push(path);
                    } else if path.is_file() {
                        // Skip .meta sidecar files (annotated tag metadata)
                        if path.extension().and_then(|e| e.to_str()) == Some("meta") {
                            continue;
                        }
                        // Construct ref name relative to refs_dir
                        // e.g., refs/heads/main, refs/heads/feature/branch, refs/remotes/origin/main
                        if let Ok(relative_path) = path.strip_prefix(&refs_dir) {
                            let ref_name = format!(
                                "refs/{}",
                                relative_path.to_string_lossy().replace('\\', "/")
                            );

                            if let Ok(r) = refdb.read(&ref_name).await {
                                ref_infos.push(RefInfo {
                                    name: ref_name,
                                    oid: r.oid.map(|o| o.to_hex()).unwrap_or_default(),
                                    target: r.target,
                                });
                            }
                        }
                    }
                }
            }
        }
    }

    Ok(Json(RefsResponse {
        refs: ref_infos,
        capabilities: vec!["pack-v1".to_string()],
    }))
}

/// POST /:repo/objects/pack - Upload a pack file (streaming)
pub async fn upload_pack(
    Path(repo): Path<String>,
    State(state): State<Arc<AppState>>,
    auth_user: Option<Extension<AuthUser>>,
    body: axum::body::Body,
) -> Result<StatusCode, StatusCode> {
    tracing::info!("POST /{}/objects/pack (streaming)", repo);

    // Check permission: repo:write required
    check_permission(auth_user.as_deref(), "repo:write", state.is_auth_enabled())?;

    let repo_path = state.repos_dir.join(&repo);
    if !repo_path.exists() {
        tracing::warn!("Repository not found: {}", repo);
        return Err(StatusCode::NOT_FOUND);
    }

    // Initialize ODB (shared per-repo so delta_written_pairs HashSet is shared
    // across concurrent handlers — required for TOCTOU cycle prevention).
    let odb = get_or_init_odb(&state, &repo_path).await?;

    // Convert body to AsyncRead stream
    use futures::stream::TryStreamExt;
    use tokio_util::io::StreamReader;

    let stream = body.into_data_stream().map_err(std::io::Error::other);

    let stream_reader = StreamReader::new(stream);

    // Create streaming pack reader
    let mut reader = mediagit_versioning::StreamingPackReader::new(stream_reader)
        .await
        .map_err(|e| {
            tracing::error!("Failed to create streaming pack reader: {}", e);
            StatusCode::BAD_REQUEST
        })?;

    // Reader is sequential (a pack is one byte stream) but writes can overlap.
    // Bound concurrent ODB writes with a sliding window so a slow backend PUT
    // never blocks the reader from queuing the next object.
    //
    // Priority: env var override > config override (`[performance]
    // pack_workers` in the repo config) > internal default (8).
    let config_pack_workers: Option<usize> = mediagit_config::Config::load(&repo_path)
        .await
        .ok()
        .and_then(|c| c.performance.pack_workers)
        .filter(|n| *n > 0);
    let workers_n: usize = std::env::var("MEDIAGIT_PACK_WORKERS")
        .ok()
        .and_then(|s| s.parse().ok())
        .filter(|n: &usize| *n > 0)
        .or(config_pack_workers)
        .unwrap_or(8);

    tracing::info!(
        "Processing streaming pack upload (concurrent writes: {})",
        workers_n
    );

    use futures::stream::FuturesUnordered;

    let mut in_flight: FuturesUnordered<_> = FuturesUnordered::new();
    let mut object_count: usize = 0;

    // Helper: drain one completed write and bump the counter.
    async fn drain_one(
        in_flight: &mut FuturesUnordered<
            impl std::future::Future<Output = anyhow::Result<(Oid, Oid)>>,
        >,
        object_count: &mut usize,
    ) -> Result<(), StatusCode> {
        if let Some(res) = in_flight.next().await {
            let (expected, stored) = res.map_err(|e| {
                tracing::error!("Failed to write object to ODB: {}", e);
                StatusCode::INTERNAL_SERVER_ERROR
            })?;
            if stored != expected {
                tracing::warn!(
                    expected = %expected,
                    actual = %stored,
                    "OID mismatch during pack upload (object may have different content)"
                );
            }
            *object_count += 1;
            if (*object_count).is_multiple_of(100) {
                tracing::debug!("Processed {} objects", *object_count);
            }
        }
        Ok(())
    }

    loop {
        // Apply back-pressure: only fetch a new object once we have a worker slot.
        while in_flight.len() >= workers_n {
            drain_one(&mut in_flight, &mut object_count).await?;
        }

        match reader.next_object().await {
            Some(Ok((oid, obj_type, data))) => {
                let odb_clone = odb.clone();
                in_flight.push(async move {
                    let stored = odb_clone.write(obj_type, &data).await?;
                    Ok::<_, anyhow::Error>((oid, stored))
                });
            }
            Some(Err(e)) => {
                tracing::error!("Failed to read object from pack stream: {}", e);
                return Err(StatusCode::BAD_REQUEST);
            }
            None => break,
        }
    }

    // Drain remaining writes.
    while !in_flight.is_empty() {
        drain_one(&mut in_flight, &mut object_count).await?;
    }

    tracing::info!(
        "Successfully unpacked {} objects (streaming via ODB)",
        object_count
    );
    Ok(StatusCode::OK)
}

/// GET /:repo/objects/pack - Download a pack file (after POST to /objects/want)
/// Requires X-Request-ID header with the request_id from POST /objects/want response.
pub async fn download_pack(
    Path(repo): Path<String>,
    State(state): State<Arc<AppState>>,
    auth_user: Option<Extension<AuthUser>>,
    headers: HeaderMap,
) -> Result<axum::response::Response<axum::body::Body>, StatusCode> {
    tracing::info!("GET /{}/objects/pack", repo);

    // Check permission: repo:read required
    check_permission(auth_user.as_deref(), "repo:read", state.is_auth_enabled())?;

    // Get request ID from header (required to prevent race conditions)
    let request_id = headers
        .get("X-Request-ID")
        .and_then(|v| v.to_str().ok())
        .ok_or_else(|| {
            tracing::warn!("Missing X-Request-ID header in GET /objects/pack");
            StatusCode::BAD_REQUEST
        })?;

    // Get the wanted objects from state using request_id (prevents race conditions)
    let want_entry = {
        let mut want_cache = state.want_cache.lock().await;
        // Remove from cache after retrieval (one-time use)
        match want_cache.remove(request_id) {
            Some(entry) => {
                // Verify the request is for the same repo
                if entry.repo != repo {
                    tracing::error!(
                        "Request ID {} was for repo '{}' but pack requested for '{}'",
                        request_id,
                        entry.repo,
                        repo
                    );
                    return Err(StatusCode::BAD_REQUEST);
                }
                entry
            }
            None => {
                tracing::warn!("Request ID {} not found or already used", request_id);
                return Err(StatusCode::BAD_REQUEST);
            }
        }
    };
    let want_list = want_entry.want_list;
    let have_list = want_entry.have_list;

    let repo_path = state.repos_dir.join(&repo);
    if !repo_path.exists() {
        return Err(StatusCode::NOT_FOUND);
    }

    // Initialize ODB (shared per-repo for TOCTOU guard effectiveness).
    let odb = get_or_init_odb(&state, &repo_path).await?;

    // Expand the client's `have` set into the full object closure the client
    // is known to already have. Any OID in this set — including entire
    // subtrees and blobs reachable from a parent commit — is pruned from the
    // pack walk below. Unknown haves (stale or forged) are silently skipped
    // by `walk_reachable`, which is the whole point of having it be lenient.
    let have_oids: Vec<Oid> = have_list
        .iter()
        .filter_map(|s| Oid::from_hex(s).ok())
        .collect();
    // Fast path: empty have-set (clone) skips the expensive BFS expansion.
    let stop_at = if have_oids.is_empty() {
        std::collections::HashSet::new()
    } else {
        let empty: std::collections::HashSet<Oid> = std::collections::HashSet::new();
        mediagit_versioning::walk_reachable(&odb, have_oids, &empty)
            .await
            .map_err(|e| {
                tracing::error!("Failed to expand have-closure: {}", e);
                StatusCode::INTERNAL_SERVER_ERROR
            })?
    };
    tracing::info!(
        "Have-closure: {} objects reachable from {} have OIDs",
        stop_at.len(),
        have_list.len()
    );

    // Collect all objects reachable from wanted OIDs via iterative BFS,
    // pruning anything the client already has (via `stop_at`).
    // Strict: reject request if any want OID is malformed.
    let want_oids: Vec<Oid> = want_list
        .iter()
        .map(|s| Oid::from_hex(s).map_err(|_| StatusCode::BAD_REQUEST))
        .collect::<Result<_, _>>()?;
    let objects_to_pack = collect_objects_bfs(&odb, want_oids, &stop_at)
        .await
        .map_err(|e| {
            tracing::error!("Failed to collect objects: {}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    tracing::info!(
        "Collecting {} objects for pack (from {} requested, {} pruned via have)",
        objects_to_pack.len(),
        want_list.len(),
        stop_at.len()
    );

    // Filter out chunked objects — they'll be transferred separately.
    // Also prune any chunked OIDs the client already has (in stop_at)
    // to avoid unnecessary manifest download requests.
    //
    // Probe `is_chunked` in parallel: each probe is one backend HEAD on cloud
    // storage, and the sequential version made pack generation O(N × RTT).
    use futures::stream::StreamExt;
    let probes = futures::stream::iter(objects_to_pack.iter().copied())
        .map(|oid| {
            let odb_ref = &odb;
            async move {
                let chunked = odb_ref.is_chunked(&oid).await.unwrap_or(false);
                (oid, chunked)
            }
        })
        .buffer_unordered(8)
        .collect::<Vec<(Oid, bool)>>()
        .await;

    let mut chunked_objects: Vec<String> = Vec::new();
    let mut non_chunked_objects: Vec<Oid> = Vec::new();
    // Re-bucket in original order for deterministic pack output.
    let probe_lookup: std::collections::HashMap<Oid, bool> = probes.into_iter().collect();
    for oid in &objects_to_pack {
        let chunked = probe_lookup.get(oid).copied().unwrap_or(false);
        if chunked {
            if !stop_at.contains(oid) {
                tracing::debug!(oid = %oid, "Chunked blob — separate transfer");
                chunked_objects.push(oid.to_hex());
            } else {
                tracing::debug!(oid = %oid, "Chunked blob already on client — skipping");
            }
        } else {
            non_chunked_objects.push(*oid);
        }
    }

    tracing::info!(
        "Generating pack ({} objects, {} chunked)",
        non_chunked_objects.len(),
        chunked_objects.len()
    );

    // Use streaming pack generation for O(64KB) memory instead of O(pack_size)
    // This prevents server OOM when generating large packs
    use axum::http::header;
    use axum::response::Response;

    // Create 64KB buffered duplex channel for streaming
    let (writer, reader) = duplex(64 * 1024);

    // Wrap ODB in Arc for sharing with background task
    let odb_arc = Arc::new(odb);
    let odb_clone = odb_arc.clone();
    let objects_to_stream = non_chunked_objects.clone();
    let object_count = objects_to_stream.len() as u32;

    tracing::info!(
        object_count = object_count,
        "Starting streaming pack generation"
    );

    // Spawn background task to write pack to channel
    tokio::spawn(async move {
        let temp_dir = std::env::temp_dir();
        let result: Result<(), anyhow::Error> = async {
            let mut pack_writer = StreamingPackWriter::new(writer, object_count, &temp_dir)
                .await
                .map_err(|e| anyhow::anyhow!("Failed to create streaming pack writer: {}", e))?;

            for oid in objects_to_stream {
                let obj_data = odb_clone.read(&oid).await?;
                let obj_type = detect_object_type(&obj_data).unwrap_or(ObjectType::Blob);
                pack_writer
                    .write_object(oid, obj_type, &obj_data)
                    .await
                    .map_err(|e| anyhow::anyhow!("Failed to write object {}: {}", oid, e))?;
            }

            pack_writer
                .finalize()
                .await
                .map_err(|e| anyhow::anyhow!("Failed to finalize pack: {}", e))?;

            tracing::info!("Streaming pack generation completed successfully");
            Ok(())
        }
        .await;

        if let Err(e) = result {
            tracing::error!(error = %e, "Streaming pack generation failed");
        }
    });

    // Create streaming response body from reader
    let stream = ReaderStream::new(reader);
    let body = axum::body::Body::from_stream(stream);

    // Build response (chunked transfer encoding, no Content-Length)
    let mut response_builder = Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "application/octet-stream");

    if !chunked_objects.is_empty() {
        response_builder = response_builder.header("X-Chunked-Objects", chunked_objects.join(","));
        tracing::info!(
            "Including {} chunked objects in header for separate transfer",
            chunked_objects.len()
        );
    }

    response_builder
        .body(body)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)
}

/// Iteratively collect an object and all reachable children (commits, trees,
/// blobs) using BFS. Objects already in `stop_at` are pruned — neither added
/// nor recursed into — which turns full-history packs into delta packs during
/// incremental fetch.
///
/// Uses `VecDeque`-based BFS instead of recursive `Box::pin` to avoid heap
/// allocations per traversal step in deep histories.
async fn collect_objects_bfs(
    odb: &ObjectDatabase,
    roots: impl IntoIterator<Item = Oid>,
    stop_at: &std::collections::HashSet<Oid>,
) -> Result<Vec<Oid>, anyhow::Error> {
    use futures::stream::StreamExt;

    let mut visited = std::collections::HashSet::new();
    let mut collected: Vec<Oid> = Vec::new();
    let mut frontier: Vec<Oid> = Vec::new();

    for oid in roots {
        if stop_at.contains(&oid) || !visited.insert(oid) {
            continue;
        }
        frontier.push(oid);
    }

    let parallelism: usize = std::env::var("MEDIAGIT_BFS_PARALLELISM")
        .ok()
        .and_then(|s| s.parse().ok())
        .filter(|n: &usize| *n > 0)
        .unwrap_or(16);

    // Level-by-level parallel BFS: at each step we read every object in the
    // current frontier concurrently (capped by `parallelism`), then expand the
    // next frontier sequentially. The previous serial implementation made one
    // Azure GET per object — for a 3-object clone that was 18 s of real
    // round-trip stalls. With parallelism=16 a tree of 100 entries finishes
    // in ~7 batches instead of 100 sequential reads.
    while !frontier.is_empty() {
        let batch: Vec<Oid> = frontier.split_off(0);

        // Probe + read each oid concurrently. is_chunked + read for chunked
        // blobs is short-circuited because chunked manifests don't recurse.
        let mut probe_stream = futures::stream::iter(batch.into_iter().map(|oid| async move {
            let chunked = odb.is_chunked(&oid).await.unwrap_or(false);
            if chunked {
                (oid, true, None)
            } else {
                let read = odb.read(&oid).await.ok();
                (oid, false, read)
            }
        }))
        .buffer_unordered(parallelism);

        // Re-collect results so output order is deterministic-ish (stable
        // wrt the order they entered the frontier — for a clone we sort
        // before pack generation anyway, so out-of-order completion is fine).
        let mut batch_results: Vec<(Oid, bool, Option<Vec<u8>>)> = Vec::new();
        while let Some(item) = probe_stream.next().await {
            batch_results.push(item);
        }

        for (oid, chunked, read) in batch_results {
            if chunked {
                collected.push(oid);
                continue;
            }
            let obj_data = match read {
                Some(d) => d,
                None => {
                    tracing::warn!("Object {} not found", oid);
                    continue;
                }
            };
            collected.push(oid);

            let obj_type = detect_object_type(&obj_data).unwrap_or(ObjectType::Blob);
            match obj_type {
                ObjectType::Commit => {
                    if let Ok(commit) = Commit::deserialize(&obj_data) {
                        if !stop_at.contains(&commit.tree) && visited.insert(commit.tree) {
                            frontier.push(commit.tree);
                        }
                        for parent in commit.parents {
                            if !stop_at.contains(&parent) && visited.insert(parent) {
                                frontier.push(parent);
                            }
                        }
                    }
                }
                ObjectType::Tree => {
                    if let Ok(tree) = Tree::deserialize(&obj_data) {
                        for entry in tree.iter() {
                            if !stop_at.contains(&entry.oid) && visited.insert(entry.oid) {
                                frontier.push(entry.oid);
                            }
                        }
                    }
                }
                ObjectType::Blob => { /* leaf */ }
            }
        }
    }

    Ok(collected)
}

/// POST /:repo/objects/want - Request specific objects
/// Returns a unique request_id that must be used in the X-Request-ID header
/// when calling GET /objects/pack to retrieve the objects.
pub async fn request_objects(
    Path(repo): Path<String>,
    State(state): State<Arc<AppState>>,
    auth_user: Option<Extension<AuthUser>>,
    Json(want_req): Json<WantRequest>,
) -> Result<Json<WantResponse>, StatusCode> {
    tracing::info!(
        "POST /{}/objects/want (want: {}, have: {})",
        repo,
        want_req.want.len(),
        want_req.have.len()
    );

    // Check permission: repo:read required
    check_permission(auth_user.as_deref(), "repo:read", state.is_auth_enabled())?;

    // Generate unique request ID to prevent race conditions between concurrent clients
    let request_id = crate::state::generate_request_id();

    // Store the want list in cache keyed by request_id (not repo name).
    // `have` is retained so download_pack can prune objects already on the
    // client — this is the incremental-fetch path. Empty `have` gives the
    // clone-equivalent full pack.
    {
        let mut want_cache = state.want_cache.lock().await;
        want_cache.insert(request_id.clone(), repo, want_req.want, want_req.have);
    }

    Ok(Json(WantResponse { request_id }))
}

/// POST /:repo/refs/update - Update repository refs
pub async fn update_refs(
    Path(repo): Path<String>,
    State(state): State<Arc<AppState>>,
    auth_user: Option<Extension<AuthUser>>,
    Json(req): Json<RefUpdateRequest>,
) -> Result<Json<RefUpdateResponse>, StatusCode> {
    tracing::info!("POST /{}/refs/update ({} updates)", repo, req.updates.len());

    // Check permission: repo:write required
    check_permission(auth_user.as_deref(), "repo:write", state.is_auth_enabled())?;

    let repo_path = state.repos_dir.join(&repo);
    if !repo_path.exists() {
        return Err(StatusCode::NOT_FOUND);
    }

    // Initialize refdb and ODB (shared per-repo for TOCTOU guard effectiveness).
    let refdb = RefDatabase::new(repo_path.join(".mediagit"));
    let odb = Arc::new(get_or_init_odb(&state, &repo_path).await?);
    let reflog = Reflog::new(repo_path.join(".mediagit"));

    let mut results = Vec::new();
    let mut all_success = true;

    for update in req.updates {
        // Handle ref deletion
        if update.delete {
            // HEAD protection: prevent deleting the currently active branch
            if let Ok(head) = refdb.read("HEAD").await {
                if head.target.as_deref() == Some(&update.name) {
                    tracing::warn!(
                        "Refusing to delete '{}': it is the current HEAD",
                        update.name
                    );
                    results.push(RefUpdateResult {
                        ref_name: update.name.clone(),
                        success: false,
                        error: Some(format!(
                            "refusing to delete the current branch: '{}'",
                            update.name
                        )),
                    });
                    all_success = false;
                    continue;
                }
            }

            // Safety check: verify old_oid matches (if provided)
            if let Some(expected_old) = &update.old_oid {
                if let Ok(current_ref) = refdb.read(&update.name).await {
                    if let Some(current_oid) = &current_ref.oid {
                        let current_oid_str = current_oid.to_hex();
                        if &current_oid_str != expected_old && !req.force {
                            tracing::warn!(
                                "Ref delete rejected for '{}': expected {}, got {}",
                                update.name,
                                expected_old,
                                current_oid_str
                            );
                            results.push(RefUpdateResult {
                                ref_name: update.name.clone(),
                                success: false,
                                error: Some("ref changed since last fetch".to_string()),
                            });
                            all_success = false;
                            continue;
                        }
                    }
                }
            }

            // Verify ref exists before deleting
            match refdb.read(&update.name).await {
                Ok(_) => {}
                Err(_) => {
                    tracing::warn!("Ref '{}' does not exist, cannot delete", update.name);
                    results.push(RefUpdateResult {
                        ref_name: update.name.clone(),
                        success: false,
                        error: Some(format!("ref '{}' does not exist", update.name)),
                    });
                    all_success = false;
                    continue;
                }
            }

            // Delete the ref
            match refdb.delete(&update.name).await {
                Ok(_) => {
                    tracing::info!("Deleted ref '{}'", update.name);
                    results.push(RefUpdateResult {
                        ref_name: update.name,
                        success: true,
                        error: None,
                    });
                }
                Err(e) => {
                    tracing::error!("Failed to delete ref '{}': {}", update.name, e);
                    results.push(RefUpdateResult {
                        ref_name: update.name,
                        success: false,
                        error: Some(e.to_string()),
                    });
                    all_success = false;
                }
            }
            continue;
        }

        // Check if old_oid matches (if provided)
        if let Some(expected_old) = &update.old_oid {
            if let Ok(current_ref) = refdb.read(&update.name).await {
                if let Some(current_oid) = &current_ref.oid {
                    let current_oid_str = current_oid.to_hex();
                    if &current_oid_str != expected_old && !req.force {
                        tracing::warn!(
                            "Ref update rejected: expected {}, got {}",
                            expected_old,
                            current_oid_str
                        );
                        results.push(RefUpdateResult {
                            ref_name: update.name.clone(),
                            success: false,
                            error: Some("non-fast-forward".to_string()),
                        });
                        all_success = false;
                        continue;
                    }
                }
            }
        }

        // Ancestry check: when force=false and the ref already exists, require
        // that the new commit is a descendant of the current tip (fast-forward only).
        if !req.force && !update.delete {
            if let Ok(current_ref) = refdb.read(&update.name).await {
                if let Some(current_oid) = &current_ref.oid {
                    let new_oid_parsed =
                        Oid::from_hex(&update.new_oid).map_err(|_| StatusCode::BAD_REQUEST)?;
                    let lca = LcaFinder::new(Arc::clone(&odb));
                    match lca.is_ancestor(current_oid, &new_oid_parsed).await {
                        Ok(true) => {} // fast-forward: current is ancestor of new — allowed
                        Ok(false) => {
                            tracing::warn!(
                                "Non-fast-forward push rejected for '{}': {} is not ancestor of {}",
                                update.name,
                                current_oid.to_hex(),
                                update.new_oid
                            );
                            results.push(RefUpdateResult {
                                ref_name: update.name.clone(),
                                success: false,
                                error: Some("non-fast-forward".to_string()),
                            });
                            all_success = false;
                            continue;
                        }
                        Err(e) => {
                            tracing::error!("Ancestry check failed for '{}': {}", update.name, e);
                            results.push(RefUpdateResult {
                                ref_name: update.name.clone(),
                                success: false,
                                error: Some(format!("ancestry check failed: {}", e)),
                            });
                            all_success = false;
                            continue;
                        }
                    }
                }
            }
        }

        // Capture the pre-write OID for reflog
        let pre_write_oid = if let Ok(current_ref) = refdb.read(&update.name).await {
            current_ref.oid
        } else {
            None
        };

        // Update the ref
        let new_oid = Oid::from_hex(&update.new_oid).map_err(|_| StatusCode::BAD_REQUEST)?;
        let ref_update = Ref::new_direct(update.name.clone(), new_oid);

        match refdb.write(&ref_update).await {
            Ok(_) => {
                tracing::info!("Updated {} to {}", update.name, update.new_oid);

                // Append reflog entry for every successful ref write
                let old_oid_for_log = pre_write_oid.unwrap_or_else(|| Oid::from_bytes([0u8; 32]));
                let (actor_name, actor_email) = auth_user
                    .as_ref()
                    .map(|u| (u.user_id.clone(), format!("{}@local", u.user_id)))
                    .unwrap_or_else(|| ("server".to_string(), "server@local".to_string()));
                let msg = if req.force {
                    "push (force)".to_string()
                } else {
                    "push".to_string()
                };
                let entry =
                    ReflogEntry::now(old_oid_for_log, new_oid, &actor_name, &actor_email, &msg);
                if let Err(e) = reflog.append(&update.name, &entry).await {
                    tracing::warn!("Failed to write reflog for '{}': {}", update.name, e);
                }

                results.push(RefUpdateResult {
                    ref_name: update.name,
                    success: true,
                    error: None,
                });
            }
            Err(e) => {
                tracing::error!("Failed to update {}: {}", update.name, e);
                results.push(RefUpdateResult {
                    ref_name: update.name,
                    success: false,
                    error: Some(e.to_string()),
                });
                all_success = false;
            }
        }
    }

    Ok(Json(RefUpdateResponse {
        success: all_success,
        results,
    }))
}

/// Helper function to detect object type from raw object data
/// MediaGit stores objects with bincode serialization, so we try to deserialize
/// as Commit or Tree. If neither works, it's a Blob.
fn detect_object_type(data: &[u8]) -> Option<ObjectType> {
    // Try to deserialize as Commit first using its own deserializer
    if Commit::deserialize(data).is_ok() {
        return Some(ObjectType::Commit);
    }

    // Try to deserialize as Tree using its own deserializer
    if Tree::deserialize(data).is_ok() {
        return Some(ObjectType::Tree);
    }

    // If neither, it's a Blob (or at minimum treat it as one)
    Some(ObjectType::Blob)
}

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

    // Check chunks concurrently — up to 50 in-flight existence checks.
    // storage is Arc<dyn StorageBackend> (Send+Sync), cheap to clone.
    //
    // A chunk counts as "present" if either `chunks/<id>` or `chunk-deltas/<id>.meta`
    // exists: the delta sidecar is a valid storage form and the reader path
    // handles both. Otherwise a re-push would rematerialize existing deltas
    // into full chunks and undo the storage savings.
    let missing: Vec<String> = futures::stream::iter(chunk_ids)
        .map(|chunk_id_hex| {
            let storage = Arc::clone(&storage);
            async move {
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

/// Parse a `chunk-deltas/<id>.meta` body of the form `base:<hex>` into the
/// base OID hex. Returns `None` for malformed/empty meta.
fn parse_chunk_delta_meta(meta_bytes: &[u8]) -> Option<String> {
    let s = std::str::from_utf8(meta_bytes).ok()?;
    let trimmed = s.trim();
    let hex = trimmed.strip_prefix("base:")?;
    if hex.len() == 64 && hex.chars().all(|c| c.is_ascii_hexdigit()) {
        Some(hex.to_string())
    } else {
        None
    }
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

/// Header carrying the base chunk OID (hex) for a chunk-delta upload.
pub const DELTA_BASE_HEADER: &str = "x-mediagit-delta-base";

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
        futures::stream::iter(pairs.into_iter().map(|(chunk_id, content_length)| {
            let storage = Arc::clone(&storage);
            let repo = repo.clone();
            async move {
                let key = format!("chunks/{}", chunk_id);
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

    let missing: Vec<String> = futures::stream::iter(req.chunk_ids)
        .map(|chunk_id_hex| {
            let storage = Arc::clone(&storage);
            async move {
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
// Raw File Serving Endpoints — HTTP "Download Raw" equivalent
// ============================================================================

/// Query parameters shared by file and tree endpoints
#[derive(serde::Deserialize)]
pub struct RefQueryParams {
    #[serde(rename = "ref", default = "default_ref_head")]
    ref_name: String,
}

fn default_ref_head() -> String {
    "HEAD".to_string()
}

/// Validate a file path component for security (no path traversal, no absolute paths)
fn validate_file_path(path: &str) -> Result<(), StatusCode> {
    if path.starts_with('/') {
        return Err(StatusCode::BAD_REQUEST);
    }
    for component in path.split('/') {
        if component == ".." || component == "." || component.contains('\0') {
            return Err(StatusCode::BAD_REQUEST);
        }
    }
    Ok(())
}

/// Walk the commit tree to resolve a file path to its blob OID.
async fn resolve_path_to_blob(
    odb: &ObjectDatabase,
    refdb: &RefDatabase,
    ref_str: &str,
    file_path: &str,
) -> Result<Oid, StatusCode> {
    let commit_oid = resolve_revision(ref_str, refdb, odb)
        .await
        .map_err(|_| StatusCode::NOT_FOUND)?;

    let commit_data = odb
        .read(&commit_oid)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let commit =
        Commit::deserialize(&commit_data).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let mut current_oid = commit.tree;
    let components: Vec<&str> = file_path.split('/').filter(|s| !s.is_empty()).collect();
    if components.is_empty() {
        return Err(StatusCode::BAD_REQUEST);
    }

    for (i, component) in components.iter().enumerate() {
        let tree_data = odb
            .read(&current_oid)
            .await
            .map_err(|_| StatusCode::NOT_FOUND)?;
        let tree = Tree::deserialize(&tree_data).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        let entry = tree.entries.get(*component).ok_or(StatusCode::NOT_FOUND)?;

        if i == components.len() - 1 {
            if entry.is_tree() {
                // Path points to a directory, not a file
                return Err(StatusCode::BAD_REQUEST);
            }
            return Ok(entry.oid);
        } else {
            if !entry.is_tree() {
                return Err(StatusCode::NOT_FOUND);
            }
            current_oid = entry.oid;
        }
    }
    Err(StatusCode::NOT_FOUND)
}

/// Walk the commit tree to resolve a directory path to its Tree object.
/// Empty `dir_path` returns the root tree.
async fn resolve_path_to_tree(
    odb: &ObjectDatabase,
    refdb: &RefDatabase,
    ref_str: &str,
    dir_path: &str,
) -> Result<(Oid, Tree), StatusCode> {
    let commit_oid = resolve_revision(ref_str, refdb, odb)
        .await
        .map_err(|_| StatusCode::NOT_FOUND)?;

    let commit_data = odb
        .read(&commit_oid)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let commit =
        Commit::deserialize(&commit_data).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let mut current_oid = commit.tree;
    let components: Vec<&str> = dir_path.split('/').filter(|s| !s.is_empty()).collect();

    for component in &components {
        let tree_data = odb
            .read(&current_oid)
            .await
            .map_err(|_| StatusCode::NOT_FOUND)?;
        let tree = Tree::deserialize(&tree_data).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        let entry = tree.entries.get(*component).ok_or(StatusCode::NOT_FOUND)?;
        if !entry.is_tree() {
            return Err(StatusCode::NOT_FOUND);
        }
        current_oid = entry.oid;
    }

    let tree_data = odb
        .read(&current_oid)
        .await
        .map_err(|_| StatusCode::NOT_FOUND)?;
    let tree = Tree::deserialize(&tree_data).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok((commit_oid, tree))
}

/// JSON shape for a single entry in a tree listing response
#[derive(serde::Serialize)]
pub struct TreeEntryResponse {
    name: String,
    mode: String,
    oid: String,
    #[serde(rename = "type")]
    entry_type: String,
}

/// JSON response body for `GET /{repo}/tree[/{path}]`
#[derive(serde::Serialize)]
pub struct TreeListResponse {
    #[serde(rename = "ref")]
    ref_name: String,
    commit: String,
    path: String,
    entries: Vec<TreeEntryResponse>,
}

/// GET /{repo}/files/{*path}?ref=HEAD
///
/// Download a file from committed state. Streams chunked blobs via O(64KB) duplex
/// channel — memory usage is independent of file size.
pub async fn download_file_by_path(
    Path((repo, file_path)): Path<(String, String)>,
    State(state): State<Arc<AppState>>,
    auth_user: Option<Extension<AuthUser>>,
    axum::extract::Query(params): axum::extract::Query<RefQueryParams>,
) -> Result<axum::response::Response<axum::body::Body>, StatusCode> {
    tracing::info!("GET /{}/files/{} ref={}", repo, file_path, params.ref_name);

    crate::security::validate_repo_name(&repo).map_err(|_| StatusCode::BAD_REQUEST)?;
    validate_file_path(&file_path)?;
    check_permission(auth_user.as_deref(), "repo:read", state.is_auth_enabled())?;

    let repo_path = state.repos_dir.join(&repo);
    if !repo_path.exists() {
        return Err(StatusCode::NOT_FOUND);
    }

    let odb = Arc::new(get_or_init_odb(&state, &repo_path).await?);
    let refdb = RefDatabase::new(repo_path.join(".mediagit"));

    let blob_oid = resolve_path_to_blob(&odb, &refdb, &params.ref_name, &file_path).await?;
    let filename = file_path
        .split('/')
        .next_back()
        .unwrap_or("file")
        .to_string();

    let manifest_opt = odb
        .get_chunk_manifest(&blob_oid)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    use axum::http::header;
    use axum::response::Response;

    if let Some(manifest) = manifest_opt {
        // Chunked blob: stream via duplex channel — O(64KB) memory regardless of file size.
        let total_size = manifest.total_size;
        let (writer, reader) = duplex(64 * 1024);
        let odb_clone = Arc::clone(&odb);

        tokio::spawn(async move {
            use tokio::io::AsyncWriteExt;
            let mut w = writer;
            for chunk_ref in &manifest.chunks {
                match odb_clone.get_chunk(&chunk_ref.id).await {
                    Ok(data) => {
                        if w.write_all(&data).await.is_err() {
                            tracing::warn!("Client disconnected during chunked file download");
                            return;
                        }
                    }
                    Err(e) => {
                        tracing::error!(
                            error = %e,
                            chunk_id = %chunk_ref.id,
                            "Failed to read chunk during file download"
                        );
                        return;
                    }
                }
            }
            // Dropping writer closes the duplex channel, signalling EOF to reader.
        });

        let stream = ReaderStream::new(reader);
        let body = axum::body::Body::from_stream(stream);
        Response::builder()
            .status(StatusCode::OK)
            .header(header::CONTENT_TYPE, "application/octet-stream")
            .header(header::CONTENT_LENGTH, total_size)
            .header(
                header::CONTENT_DISPOSITION,
                format!("attachment; filename=\"{}\"", filename),
            )
            .header("X-MediaGit-OID", blob_oid.to_hex())
            .header("X-MediaGit-Chunked", "true")
            .body(body)
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)
    } else {
        // Non-chunked blob: read fully (fits in memory by definition — not chunked).
        let data = odb
            .read(&blob_oid)
            .await
            .map_err(|_| StatusCode::NOT_FOUND)?;
        let size = data.len();
        let body = axum::body::Body::from(bytes::Bytes::from(data));
        Response::builder()
            .status(StatusCode::OK)
            .header(header::CONTENT_TYPE, "application/octet-stream")
            .header(header::CONTENT_LENGTH, size)
            .header(
                header::CONTENT_DISPOSITION,
                format!("attachment; filename=\"{}\"", filename),
            )
            .header("X-MediaGit-OID", blob_oid.to_hex())
            .header("X-MediaGit-Chunked", "false")
            .body(body)
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)
    }
}

/// Shared logic for tree listing (used by both `list_tree` and `list_tree_root`)
async fn list_tree_impl(
    repo: String,
    dir_path: String,
    state: Arc<AppState>,
    auth_user: Option<Extension<AuthUser>>,
    ref_name: String,
) -> Result<Json<TreeListResponse>, StatusCode> {
    crate::security::validate_repo_name(&repo).map_err(|_| StatusCode::BAD_REQUEST)?;
    if !dir_path.is_empty() {
        validate_file_path(&dir_path)?;
    }
    check_permission(auth_user.as_deref(), "repo:read", state.is_auth_enabled())?;

    let repo_path = state.repos_dir.join(&repo);
    if !repo_path.exists() {
        return Err(StatusCode::NOT_FOUND);
    }

    let odb = get_or_init_odb(&state, &repo_path).await?;
    let refdb = RefDatabase::new(repo_path.join(".mediagit"));

    let (commit_oid, tree) = resolve_path_to_tree(&odb, &refdb, &ref_name, &dir_path).await?;

    let entries = tree
        .iter()
        .map(|entry| TreeEntryResponse {
            name: entry.name.clone(),
            mode: entry.mode.to_string(),
            oid: entry.oid.to_hex(),
            entry_type: if entry.is_tree() {
                "tree".to_string()
            } else {
                "blob".to_string()
            },
        })
        .collect();

    Ok(Json(TreeListResponse {
        ref_name,
        commit: commit_oid.to_hex(),
        path: dir_path,
        entries,
    }))
}

/// GET /{repo}/tree/{*path}?ref=HEAD — List directory contents at path
pub async fn list_tree(
    Path((repo, dir_path)): Path<(String, String)>,
    State(state): State<Arc<AppState>>,
    auth_user: Option<Extension<AuthUser>>,
    axum::extract::Query(params): axum::extract::Query<RefQueryParams>,
) -> Result<Json<TreeListResponse>, StatusCode> {
    tracing::info!("GET /{}/tree/{} ref={}", repo, dir_path, params.ref_name);
    list_tree_impl(repo, dir_path, state, auth_user, params.ref_name).await
}

/// GET /{repo}/tree?ref=HEAD — List root tree contents
pub async fn list_tree_root(
    Path(repo): Path<String>,
    State(state): State<Arc<AppState>>,
    auth_user: Option<Extension<AuthUser>>,
    axum::extract::Query(params): axum::extract::Query<RefQueryParams>,
) -> Result<Json<TreeListResponse>, StatusCode> {
    tracing::info!("GET /{}/tree ref={}", repo, params.ref_name);
    list_tree_impl(repo, String::new(), state, auth_user, params.ref_name).await
}

// ============================================================================
// Pack Manifest Endpoints (F6) — Track-F cloud pack bundling
// ============================================================================

#[derive(serde::Deserialize)]
pub struct ManifestEntry {
    pub chunk_oid: String,
    pub offset: u64,
    pub length: u32,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct PackIndexLine {
    chunk_oid: String,
    pack_oid: String,
    offset: u64,
    length: u32,
}

#[derive(serde::Deserialize)]
pub struct CompletePackRequest {
    pub pack_oid: String,
    pub manifest: Vec<ManifestEntry>,
}

/// POST /{repo}/packs/complete — Register a finished cloud pack and its chunk manifest.
///
/// Server HEADs the pack object before accepting the manifest so a crash between
/// PUT and complete cannot create a manifest pointing at a missing pack.
pub async fn complete_pack(
    Path(repo): Path<String>,
    State(state): State<Arc<AppState>>,
    auth_user: Option<Extension<AuthUser>>,
    Json(req): Json<CompletePackRequest>,
) -> Result<StatusCode, StatusCode> {
    check_permission(auth_user.as_deref(), "repo:write", state.is_auth_enabled())?;

    crate::security::validate_repo_name(&repo).map_err(|_| StatusCode::BAD_REQUEST)?;
    let repo_path = state.repos_dir.join(&repo);
    if !repo_path.exists() {
        return Err(StatusCode::NOT_FOUND);
    }
    let storage = get_or_init_storage(&state, &repo_path).await?;

    let pack_key = format!("packs/{}", req.pack_oid);
    match storage.head(&pack_key).await {
        Ok(Some(_)) => {}
        _ => {
            tracing::warn!(
                repo = %repo,
                pack = %req.pack_oid,
                "complete_pack: pack object missing in storage"
            );
            return Err(StatusCode::CONFLICT);
        }
    }

    // Persist to JSONL under <repo>/.mediagit/packs/<shard>/<pack_oid>.jsonl
    let shard = if req.pack_oid.len() >= 2 {
        &req.pack_oid[..2]
    } else {
        "00"
    };
    let manifest_dir = repo_path.join(".mediagit").join("packs").join(shard);
    tokio::fs::create_dir_all(&manifest_dir)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let manifest_path = manifest_dir.join(format!("{}.jsonl", req.pack_oid));

    let mut jsonl = String::new();
    for entry in &req.manifest {
        let line = PackIndexLine {
            chunk_oid: entry.chunk_oid.clone(),
            pack_oid: req.pack_oid.clone(),
            offset: entry.offset,
            length: entry.length,
        };
        match serde_json::to_string(&line) {
            Ok(s) => {
                jsonl.push_str(&s);
                jsonl.push('\n');
            }
            Err(e) => {
                tracing::error!("Failed to serialize manifest entry: {}", e);
                return Err(StatusCode::INTERNAL_SERVER_ERROR);
            }
        }
    }
    // Hold write lock for both JSONL write and in-memory update so concurrent
    // complete_pack calls don't interleave their appends (F9 concurrency guard).
    {
        let mut idx = state.pack_index.write().await;
        tokio::fs::write(&manifest_path, jsonl.as_bytes())
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        let repo_idx = idx.entry(repo.clone()).or_default();
        for entry in &req.manifest {
            repo_idx.insert(
                entry.chunk_oid.clone(),
                PackLoc {
                    pack_oid: req.pack_oid.clone(),
                    offset: entry.offset,
                    length: entry.length,
                },
            );
        }
    }

    tracing::info!(
        repo = %repo,
        pack = %req.pack_oid,
        chunks = req.manifest.len(),
        "Pack manifest registered"
    );
    Ok(StatusCode::CREATED)
}

/// Scan local JSONL manifest files for a repo and populate pack_index.
async fn load_jsonl_index(
    state: &AppState,
    repo: &str,
    repo_path: &std::path::Path,
) -> Result<(), StatusCode> {
    let packs_dir = repo_path.join(".mediagit").join("packs");
    if !packs_dir.exists() {
        let mut idx = state.pack_index.write().await;
        idx.entry(repo.to_string()).or_default();
        return Ok(());
    }

    let mut repo_entries: std::collections::HashMap<String, PackLoc> =
        std::collections::HashMap::new();

    let mut shard_dir = tokio::fs::read_dir(&packs_dir)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    while let Some(shard) = shard_dir
        .next_entry()
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
    {
        let shard_path = shard.path();
        if !shard_path.is_dir() {
            continue;
        }
        let mut files = tokio::fs::read_dir(&shard_path)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        while let Some(file) = files
            .next_entry()
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        {
            let file_path = file.path();
            if file_path.extension().and_then(|e| e.to_str()) != Some("jsonl") {
                continue;
            }
            let content = tokio::fs::read_to_string(&file_path)
                .await
                .unwrap_or_default();
            for line in content.lines() {
                if line.is_empty() {
                    continue;
                }
                if let Ok(entry) = serde_json::from_str::<PackIndexLine>(line) {
                    if !entry.chunk_oid.is_empty() && !entry.pack_oid.is_empty() {
                        repo_entries.insert(
                            entry.chunk_oid,
                            PackLoc {
                                pack_oid: entry.pack_oid,
                                offset: entry.offset,
                                length: entry.length,
                            },
                        );
                    }
                }
            }
        }
    }

    let mut idx = state.pack_index.write().await;
    idx.insert(repo.to_string(), repo_entries);
    Ok(())
}

#[derive(serde::Deserialize)]
pub struct LocateChunksRequest {
    pub chunk_ids: Vec<String>,
    #[serde(default)]
    pub wants_full_repo: bool,
}

#[derive(serde::Serialize)]
pub struct LocatedChunk {
    pub pack_oid: String,
    pub offset: u64,
    pub length: u32,
}

/// POST /{repo}/chunks/locate — Resolve chunk OIDs to pack locations.
///
/// `wants_full_repo=true` returns the entire chunk→pack map in one response
/// (used by full-clone to eliminate per-chunk round-trips).
pub async fn locate_chunks(
    Path(repo): Path<String>,
    State(state): State<Arc<AppState>>,
    auth_user: Option<Extension<AuthUser>>,
    Json(req): Json<LocateChunksRequest>,
) -> Result<Json<std::collections::HashMap<String, LocatedChunk>>, StatusCode> {
    check_permission(auth_user.as_deref(), "repo:read", state.is_auth_enabled())?;

    crate::security::validate_repo_name(&repo).map_err(|_| StatusCode::BAD_REQUEST)?;
    let repo_path = state.repos_dir.join(&repo);
    if !repo_path.exists() {
        return Err(StatusCode::NOT_FOUND);
    }

    // Lazy-load from JSONL if this repo has no entry in the in-memory index yet.
    {
        let idx = state.pack_index.read().await;
        if !idx.contains_key(&repo) {
            drop(idx);
            load_jsonl_index(&state, &repo, &repo_path).await?;
        }
    }

    let idx = state.pack_index.read().await;
    let repo_idx = match idx.get(&repo) {
        Some(m) => m,
        None => return Ok(Json(std::collections::HashMap::new())),
    };

    let result: std::collections::HashMap<String, LocatedChunk> = if req.wants_full_repo {
        repo_idx
            .iter()
            .map(|(chunk_oid, loc)| {
                (
                    chunk_oid.clone(),
                    LocatedChunk {
                        pack_oid: loc.pack_oid.clone(),
                        offset: loc.offset,
                        length: loc.length,
                    },
                )
            })
            .collect()
    } else {
        req.chunk_ids
            .iter()
            .filter_map(|id| {
                repo_idx.get(id).map(|loc| {
                    (
                        id.clone(),
                        LocatedChunk {
                            pack_oid: loc.pack_oid.clone(),
                            offset: loc.offset,
                            length: loc.length,
                        },
                    )
                })
            })
            .collect()
    };

    Ok(Json(result))
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

#[derive(serde::Serialize)]
pub struct RebuildIndexResponse {
    pub indexed_chunks: usize,
}

/// POST /{repo}/packs/rebuild-index — Rebuild in-memory pack index from local JSONL files.
///
/// Evicts the current in-memory index for the repo and rescans all JSONL manifest
/// files under `<repo>/.mediagit/packs/`. Safe to call multiple times.
pub async fn rebuild_pack_index(
    Path(repo): Path<String>,
    State(state): State<Arc<AppState>>,
    auth_user: Option<Extension<AuthUser>>,
) -> Result<Json<RebuildIndexResponse>, StatusCode> {
    check_permission(auth_user.as_deref(), "repo:write", state.is_auth_enabled())?;

    crate::security::validate_repo_name(&repo).map_err(|_| StatusCode::BAD_REQUEST)?;
    let repo_path = state.repos_dir.join(&repo);
    if !repo_path.exists() {
        return Err(StatusCode::NOT_FOUND);
    }

    // Evict current in-memory state then reload from disk.
    {
        let mut idx = state.pack_index.write().await;
        idx.remove(&repo);
    }
    load_jsonl_index(&state, &repo, &repo_path).await?;

    let count = {
        let idx = state.pack_index.read().await;
        idx.get(&repo).map(|m| m.len()).unwrap_or(0)
    };

    tracing::info!(repo = %repo, chunks = count, "Pack index rebuilt from JSONL");
    Ok(Json(RebuildIndexResponse {
        indexed_chunks: count,
    }))
}
