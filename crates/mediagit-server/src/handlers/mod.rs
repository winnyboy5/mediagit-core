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

pub(crate) mod browse;
pub(crate) mod chunks;
pub(crate) mod repo;
pub(crate) mod transfer;

pub use browse::*;
pub use chunks::*;
pub use repo::*;
pub use transfer::*;

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

/// Header carrying the base chunk OID (hex) for a chunk-delta upload.
pub const DELTA_BASE_HEADER: &str = "x-mediagit-delta-base";

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

#[derive(serde::Serialize, serde::Deserialize)]
struct PackIndexLine {
    chunk_oid: String,
    pack_oid: String,
    offset: u64,
    length: u32,
    #[serde(default)]
    compressed_hash: Option<String>,
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
                                compressed_hash: entry.compressed_hash,
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
