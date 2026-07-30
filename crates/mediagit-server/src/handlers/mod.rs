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
    Extension, Json,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};
use bytes::Bytes;
use futures::stream::StreamExt;
use mediagit_compression::{Compressor, SmartCompressor};
use mediagit_metrics::types::OperationType as MetricOp;
use mediagit_protocol::{
    RefInfo, RefUpdateRequest, RefUpdateResponse, RefUpdateResult, RefsResponse, WantRequest,
    WantResponse,
};
use mediagit_security::auth::{AuthUser, GrantLevel, GrantsStore};
use mediagit_storage::{
    AzureBackend, GcsBackend, GcsConfig, LocalBackend, MinIOBackend, StorageBackend,
};
use mediagit_versioning::{
    Commit, FileMode, LcaFinder, ObjectDatabase, ObjectType, Oid, Ref, RefDatabase, Reflog,
    ReflogEntry, StreamingPackWriter, Tag, Tree, TreeEntry, resolve_revision,
};
use std::path::Path as StdPath;
use std::sync::Arc;
use tokio::io::duplex;
use tokio_util::io::ReaderStream;

use crate::state::{AppState, PackLoc};

pub(crate) mod admin;
pub(crate) mod browse;
pub(crate) mod chunks;
pub(crate) mod locks;
pub(crate) mod repo;
pub(crate) mod transfer;

pub use admin::*;
pub use browse::*;
pub use chunks::*;
pub use locks::*;
pub use repo::*;
pub use transfer::*;

/// Helper function to check if user has required permission.
///
/// Order of checks (H2):
/// 1. Auth disabled -> allow everything (unchanged pre-H2 behavior).
/// 2. No authenticated user -> reject.
/// 3. Admin role (flat `user:manage` permission, unique to `Role::Admin`)
///    always allowed, regardless of per-repo grants.
/// 4. `MEDIAGIT_GRANTS_ENFORCE=0`, or no grants recorded **for this repo**
///    ([`GrantsStore::repo_has_grants`]) -> fall back to the flat role check
///    exactly as before H2. AU-4: this was previously keyed on whether the
///    store held *any* grant, so configuring one repo silently switched every
///    other repo's authorization mode. Set `MEDIAGIT_GRANTS_ENFORCE=strict`
///    to enforce on every repo including ungranted ones.
/// 5. Otherwise, per-repo grant lookup: the user's grant level for `repo`
///    must be at or above the level implied by `required_permission`
///    (`read ⊂ write ⊂ admin`). A permission string that isn't
///    repo-scoped (e.g. `user:manage`) isn't covered by grants and falls
///    back to the flat check.
fn check_permission(
    auth_user: Option<&AuthUser>,
    required_permission: &str,
    auth_enabled: bool,
    grants: &GrantsStore,
    repo: &str,
) -> Result<(), StatusCode> {
    // If auth is disabled, allow all requests
    if !auth_enabled {
        return Ok(());
    }

    // If auth is enabled but no user found, reject
    let user = auth_user.ok_or(StatusCode::UNAUTHORIZED)?;

    // Admin role always allowed, regardless of per-repo grants.
    if user.permissions.contains(&"user:manage".to_string()) {
        return Ok(());
    }

    let flat_check = || {
        if user.permissions.contains(&required_permission.to_string()) {
            Ok(())
        } else {
            deny(&user.user_id, repo, required_permission);
            Err(StatusCode::FORBIDDEN)
        }
    };

    // AU-4: decide enforcement **per repo**, not globally.
    //
    // This asked `!grants.is_empty()` — whether the store held any grant at
    // all — so the first grant an operator recorded to onboard one tenant
    // flipped every *other* repository from flat-role to grant-based
    // authorization at the same instant, locking out every user who had no
    // explicit grant there. A routine onboarding step had server-wide blast
    // radius, and nothing in the API hinted at it.
    //
    // Scoped to the repo under access, the backward-compat intent still holds
    // — a repo with no grants recorded behaves exactly like the pre-H2 flat
    // check — but configuring one repo no longer reconfigures the rest.
    // `MEDIAGIT_GRANTS_ENFORCE`:
    //   "0"      — off everywhere; flat roles only (unchanged).
    //   "strict" — on for every repo, including those with no grants recorded,
    //              so an ungranted repo denies rather than falling back. This
    //              is the fail-closed posture the old global behaviour gave by
    //              accident; it is now something an operator opts into
    //              deliberately instead of triggering by recording a grant.
    //   otherwise — per-repo (default).
    let enforce = std::env::var("MEDIAGIT_GRANTS_ENFORCE");
    let grants_enforced = match enforce.as_deref() {
        Ok("0") => false,
        Ok("strict") => true,
        _ => grants.repo_has_grants(repo),
    };
    if !grants_enforced {
        return flat_check();
    }

    let required_level = match required_permission {
        "repo:read" => GrantLevel::Read,
        "repo:write" => GrantLevel::Write,
        "repo:admin" => GrantLevel::Admin,
        _ => return flat_check(),
    };

    match grants.get(&user.user_id, repo) {
        Some(level) if level >= required_level => Ok(()),
        _ => {
            // The grant-based denial is the other half of DC-8: both refusal
            // paths must emit the event, or the audit stream shows denials only
            // on ungranted repos and goes quiet on exactly the repos an
            // operator configured tenancy for.
            deny(&user.user_id, repo, required_permission);
            Err(StatusCode::FORBIDDEN)
        }
    }
}

/// DC-8: record an authorization denial as an audit *event*, not just a log line.
///
/// `mediagit-security`'s audit hooks for scanning (`log_invalid_request`,
/// `log_path_traversal_attempt`, `log_rate_limit_exceeded`) were wired through
/// `audit_middleware`, but the authn/authz ones appeared only in tests — so on a
/// multi-tenant server the single event a security team most needs, "who was
/// refused access to which repository", existed nowhere in the audit stream.
/// There was a `tracing::warn!` here, which is a developer breadcrumb, not a
/// structured record anyone can query.
///
/// The client IP is not threaded in: `check_permission` has ~40 call sites and
/// no request context, and plumbing `ConnectInfo` through all of them to
/// enrich one field is a change out of proportion to it (that extractor has
/// also already caused one 500 in this codebase). The middleware already
/// records the IP for the same request, so the two correlate on timestamp.
fn deny(user_id: &str, repo: &str, required_permission: &str) {
    tracing::warn!("User {} lacks permission: {}", user_id, required_permission);
    mediagit_security::audit::log_access_denied(
        std::net::IpAddr::V4(std::net::Ipv4Addr::UNSPECIFIED),
        Some(user_id.to_string()),
        repo.to_string(),
        required_permission,
    );
}

/// Per-handler entry: returns the cached storage backend for this repo,
/// constructing it on first use. Constructing a backend (especially Azure/S3)
/// is expensive — TLS handshake plus a bucket/container existence RTT — so we
/// build it once per repo per server lifetime and reuse the `Arc` from then on.
pub async fn get_or_init_storage(
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

    // AU-5: this is the first time in this process that we have resolved the
    // repo's durable identity, so it is the moment to discard grants that
    // belonged to a *previous* repo of the same name. Deleting a repo and
    // recreating one with the same name used to hand the newcomer every grant
    // the old one had; a recreated repo gets a fresh id, so those bindings no
    // longer match and are dropped here.
    //
    // Done on storage init rather than inside `check_permission` because the
    // repo's identity is not known at authorization time — `check_permission`
    // runs before the repo path is even resolved — and reading config.toml on
    // every authorization would put file I/O on the chunk-transfer hot path.
    if let Ok(config) = mediagit_config::Config::load(repo_path).await
        && let Ok(repo_id) = resolve_repo_id(repo_path, &config)
        && let Some(name) = repo_path.file_name().and_then(|n| n.to_str())
    {
        let pruned = state.grants.prune_stale_bindings(name, &repo_id).await;
        if pruned > 0 {
            tracing::warn!(
                repo = %name,
                repo_id = %repo_id,
                pruned,
                "discarded grant(s) bound to a previous repo of the same name"
            );
        }
    }

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

/// Determine the effective repo namespace (layout v2) for a served repo:
/// env override wins, then the value persisted in the repo's config.toml,
/// then a sanitized basename of the repo path as a last-resort fallback for
/// repos whose config predates `repo_namespace`. Mirrors the CLI's
/// `resolve_repo_namespace` in `mediagit-cli/src/repo.rs`.
fn resolve_repo_namespace(repo_path: &StdPath, config: &mediagit_config::Config) -> String {
    if let Ok(ns) = std::env::var("MEDIAGIT_REPO_NAMESPACE")
        && !ns.trim().is_empty()
    {
        return mediagit_storage::sanitize_namespace(&ns);
    }
    if let Some(ns) = &config.repo_namespace
        && !ns.trim().is_empty()
    {
        return mediagit_storage::sanitize_namespace(ns);
    }
    let basename = repo_path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "repo".to_string());
    mediagit_storage::sanitize_namespace(&basename)
}

/// Resolve this served repository's identity (namespace-collision guard,
/// M2). Mirrors the CLI's `resolve_repo_id` in `mediagit-cli/src/repo.rs`:
/// returns `config.repo_id` if present, otherwise generates one and writes
/// it back to the repo's `config.toml` immediately so it's stable across
/// subsequent requests instead of being regenerated (and thus mismatching
/// the marker) on every call.
fn resolve_repo_id(
    repo_path: &StdPath,
    config: &mediagit_config::Config,
) -> Result<String, StatusCode> {
    if let Some(id) = &config.repo_id
        && !id.trim().is_empty()
    {
        return Ok(id.clone());
    }
    let id = mediagit_storage::generate_repo_id();
    let mut updated = config.clone();
    updated.repo_id = Some(id.clone());
    updated.save(repo_path).map_err(|e| {
        tracing::error!(
            "Failed to persist newly generated repo_id to config.toml: {}",
            e
        );
        StatusCode::INTERNAL_SERVER_ERROR
    })?;
    Ok(id)
}

/// Helper function to create storage backend based on repository configuration.
///
/// Layout v2: always wraps the backend in
/// [`mediagit_storage::NamespacedBackend`] — one of exactly two production
/// construction sites (the other is the CLI's `create_storage_backend`).
async fn build_storage_backend(repo_path: &StdPath) -> Result<Arc<dyn StorageBackend>, StatusCode> {
    // Load repository configuration
    let config = mediagit_config::Config::load(repo_path)
        .await
        .map_err(|e| {
            tracing::error!("Failed to load repository config: {}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    let ns = resolve_repo_namespace(repo_path, &config);
    let repo_id = resolve_repo_id(repo_path, &config)?;

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
            // Credential choice is the config enum's job now; this match is
            // total, so a new auth variant is a compile error here rather than
            // a runtime "requires either ..." 500.
            use mediagit_config::AzureAuth;
            let Some(auth) = &azure_config.auth else {
                tracing::error!(
                    "Azure backend config is missing its `auth` block (pre-v3 flat format?) - see CONFIGURATION.md for the replacement"
                );
                return Err(StatusCode::INTERNAL_SERVER_ERROR);
            };
            let storage = match auth {
                AzureAuth::ConnectionString { value } => {
                    AzureBackend::with_connection_string_and_prefix(
                        &azure_config.container,
                        value,
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
                }
                AzureAuth::AccountKey {
                    account_name,
                    account_key,
                } => AzureBackend::with_account_key_and_prefix(
                    account_name,
                    &azure_config.container,
                    account_key,
                    &azure_config.prefix,
                )
                .await
                .map_err(|e| {
                    tracing::error!("Failed to initialize Azure backend with account key: {}", e);
                    StatusCode::INTERNAL_SERVER_ERROR
                })?,
                AzureAuth::Sas {
                    account_name,
                    token,
                } => AzureBackend::with_sas_token_and_prefix(
                    account_name,
                    &azure_config.container,
                    token,
                    &azure_config.prefix,
                )
                .await
                .map_err(|e| {
                    tracing::error!("Failed to initialize Azure backend with SAS token: {}", e);
                    StatusCode::INTERNAL_SERVER_ERROR
                })?,
                AzureAuth::Emulator => AzureBackend::with_connection_string_and_prefix(
                    &azure_config.container,
                    mediagit_config::AZURITE_DEV_CONNECTION_STRING,
                    &azure_config.prefix,
                )
                .await
                .map_err(|e| {
                    tracing::error!("Failed to initialize Azure backend for emulator: {}", e);
                    StatusCode::INTERNAL_SERVER_ERROR
                })?,
            };
            Arc::new(storage)
        }
        mediagit_config::StorageConfig::GCS(gcs_config) => {
            tracing::info!(
                "Using GCS storage backend: bucket={}, project={}, prefix='{}'",
                gcs_config.bucket,
                gcs_config.project_id,
                gcs_config.prefix
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

            // Thread the configured `prefix` through GcsConfig, same as the CLI
            // path (mediagit-cli/src/repo.rs) and the S3/Azure branches above —
            // pre-fix this was silently dropped and repo data landed at bucket
            // root (C-BUG-GCS-PREFIX).
            let gcs_backend_config = gcs_config_with_prefix(gcs_config);

            let storage = match resolved_creds {
                Some(path) => GcsBackend::with_config(gcs_backend_config, &path)
                    .await
                    .map_err(|e| {
                        tracing::error!("Failed to initialize GCS backend: {}", e);
                        StatusCode::INTERNAL_SERVER_ERROR
                    })?,
                None => GcsBackend::with_default_credentials_and_config(gcs_backend_config)
                    .await
                    .map_err(|e| {
                        tracing::error!(
                            "Failed to initialize GCS backend with default credentials: {}",
                            e
                        );
                        StatusCode::INTERNAL_SERVER_ERROR
                    })?,
            };

            Arc::new(storage)
        }
        mediagit_config::StorageConfig::Multi(_) => {
            tracing::error!("Multi-backend storage is not yet implemented");
            return Err(StatusCode::INTERNAL_SERVER_ERROR);
        }
    };

    let namespaced = mediagit_storage::NamespacedBackend::new(storage, ns).map_err(|e| {
        tracing::error!("Failed to construct namespaced storage backend: {}", e);
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    mediagit_storage::check_or_write_layout_marker(
        &namespaced,
        mediagit_config::CURRENT_LAYOUT_VERSION,
        &repo_id,
    )
    .await
    .map_err(|e| {
        tracing::error!("Layout version check failed: {}", e);
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    Ok(Arc::new(namespaced))
}

/// Build a `GcsConfig` with the repo's configured `prefix` applied.
///
/// Pulled out of `build_storage_backend`'s match arm so the prefix wiring is
/// unit-testable without a network round-trip (constructing a `GcsBackend`
/// requires real credentials/connectivity).
fn gcs_config_with_prefix(gcs_config: &mediagit_config::GCSStorage) -> GcsConfig {
    let mut config = GcsConfig::new(&gcs_config.project_id, &gcs_config.bucket);
    if !gcs_config.prefix.is_empty() {
        config.prefix = Some(gcs_config.prefix.clone());
    }
    config
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
/// Why a want-side walk could not produce a complete closure.
///
/// Typed rather than `anyhow` so the handler can surface the actionable case
/// to the client without risking internal detail (paths, backend errors)
/// leaking into a response body.
#[derive(Debug)]
pub enum CollectError {
    /// A reachable object could not be read. The closure is incomplete, so no
    /// pack can honestly be produced.
    Unreadable(Oid),
    /// Anything else; surfaced to the client as a bare status.
    Other(anyhow::Error),
}

impl CollectError {
    /// Message safe to return to a client: names only the object id, which is
    /// a content hash of data the caller is already authorized to read.
    pub fn client_message(&self) -> String {
        match self {
            Self::Unreadable(oid) => format!(
                "repository is missing objects required to serve this request: {oid}                  is unreadable or absent. The server cannot produce a complete pack;                  run `mediagit fsck` on the server repository.",
            ),
            Self::Other(_) => "failed to collect objects".to_string(),
        }
    }
}

impl std::fmt::Display for CollectError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unreadable(oid) => write!(f, "object {oid} unreadable during want-side walk"),
            Self::Other(e) => write!(f, "{e}"),
        }
    }
}

async fn collect_objects_bfs(
    odb: &ObjectDatabase,
    roots: impl IntoIterator<Item = Oid>,
    stop_at: &std::collections::HashSet<Oid>,
) -> Result<Vec<Oid>, CollectError> {
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
                    // The client asked for this closure. Dropping an
                    // unreadable object here removed it from the pack *and*
                    // abandoned its entire subtree, then answered 200 — the
                    // client streams to the object count in the pack header,
                    // so a short pack is indistinguishable from a complete
                    // one. The damage surfaced much later as "Object <oid>
                    // not found: no loose object and no pack files", in a
                    // repository that had reported a successful clone.
                    //
                    // Leniency belongs on the *have* side (`walk_reachable`),
                    // where a client may legitimately name objects that do
                    // not exist. On the want side it manufactures corruption.
                    return Err(CollectError::Unreadable(oid));
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
                ObjectType::Tag => {
                    if let Ok(tag) = Tag::deserialize(&obj_data)
                        && !stop_at.contains(&tag.target)
                        && visited.insert(tag.target)
                    {
                        frontier.push(tag.target);
                    }
                }
                ObjectType::Blob => { /* leaf */ }
            }
        }
    }

    Ok(collected)
}

/// Helper function to detect object type from raw object data
/// MediaGit stores objects with postcard serialization, so we try to
/// deserialize as Commit, Tree, then Tag (in that order — see
/// `mediagit_versioning::reachability`'s module docs for why this ordering
/// is safe). If none work, it's a Blob.
fn detect_object_type(data: &[u8]) -> Option<ObjectType> {
    // Try to deserialize as Commit first using its own deserializer
    if Commit::deserialize(data).is_ok() {
        return Some(ObjectType::Commit);
    }

    // Try to deserialize as Tree using its own deserializer
    if Tree::deserialize(data).is_ok() {
        return Some(ObjectType::Tree);
    }

    // Try to deserialize as Tag using its own deserializer
    if Tag::deserialize(data).is_ok() {
        return Some(ObjectType::Tag);
    }

    // If none, it's a Blob (or at minimum treat it as one)
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

/// True if `s` is a 64-char lowercase-hex BLAKE3 id — the only shape a
/// legitimate chunk_id/pack_id/oid ever takes.
///
/// J6 (path-traversal fix): validated at the HTTP boundary, before any
/// `format!("chunks/{}", id)`-style storage key is built from a caller
/// path/body param, so a `..`-bearing id fails fast with 400 instead of
/// reaching the storage layer (which independently rejects it too, but
/// that surfaces as a 500 and does the filesystem/key work first).
fn is_valid_hex_id(s: &str) -> bool {
    s.len() == 64 && s.chars().all(|c| c.is_ascii_hexdigit())
}

/// Looser sibling of [`is_valid_hex_id`]: any non-empty all-hex string,
/// without the exact-64-char requirement.
///
/// Used on the presign/MPU endpoints, which never read or write chunk
/// *content* under the id (they only mint a signed URL or start/finish a
/// multipart upload) and whose existing test suite exercises them with
/// shortened placeholder ids (e.g. `"aabbcc"`) rather than full BLAKE3 hex.
/// Still closes the J6 hole: every character it accepts is a hex digit, so
/// `.`, `/`, and `\` (the traversal alphabet) can never appear.
fn is_hex_str(s: &str) -> bool {
    !s.is_empty() && s.chars().all(|c| c.is_ascii_hexdigit())
}

/// The one rule for "does this chunk match its claimed id": decompress
/// `compressed` and compare BLAKE3(decompressed) to `chunk_id_hex`.
///
/// Shared by `chunks::upload_chunk` (proxy upload path) and
/// `transfer::read_and_verify_chunk` (presigned-completion + strong-verify
/// paths) so this check exists in exactly one place — a second, divergent
/// copy is how this codebase got its recurring "ODB bypass" bug class.
/// Returns `false` on either a decompression failure or a hash mismatch.
pub(crate) async fn verify_chunk_content(
    compressor: &Arc<SmartCompressor>,
    chunk_id_hex: &str,
    compressed: Bytes,
) -> bool {
    let compressor = Arc::clone(compressor);
    let decompressed =
        match tokio::task::spawn_blocking(move || compressor.decompress(&compressed)).await {
            Ok(Ok(data)) => data,
            _ => return false,
        };
    blake3::hash(&decompressed).to_hex().to_string() == chunk_id_hex
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

/// Normalize a `/`-joined path for flat-tree lookups: collapses empty
/// segments (leading/trailing/duplicate slashes) so `"a//b/"` and `"a/b"`
/// key the same tree entry.
fn normalize_flat_path(path: &str) -> String {
    path.split('/')
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("/")
}

/// Resolve a file path to its blob OID.
///
/// Commits build a single-level (flat) tree keyed by full relative path
/// (see `commit.rs`); no nested `Directory` entries are ever produced, so
/// this is a direct key lookup rather than a per-component subtree walk.
// ponytail: flat-tree lookup. Upgrade path: nested trees, if ever adopted.
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

    let tree_data = odb
        .read(&commit.tree)
        .await
        .map_err(|_| StatusCode::NOT_FOUND)?;
    let tree = Tree::deserialize(&tree_data).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let key = normalize_flat_path(file_path);
    if key.is_empty() {
        return Err(StatusCode::BAD_REQUEST);
    }
    let entry = tree
        .entries
        .get(key.as_str())
        .ok_or(StatusCode::NOT_FOUND)?;
    if entry.is_tree() {
        // Path points to a directory, not a file
        return Err(StatusCode::BAD_REQUEST);
    }
    Ok(entry.oid)
}

/// Resolve a directory path to a synthesized Tree listing its immediate
/// children. Empty `dir_path` lists the root.
///
/// Commits build a single-level (flat) tree keyed by full relative path
/// (see `commit.rs`), so there is no real subtree object to walk to for a
/// "directory" — instead this scans the BTreeMap's sorted key range
/// starting at the path prefix and stops as soon as a key no longer starts
/// with it (cheap prefix scan, not a full-table scan), synthesizing one
/// entry per immediate child: a plain segment is a file entry (copied
/// as-is), a segment followed by `/` collapses to a deduplicated directory
/// entry.
// ponytail: flat-tree prefix scan. Upgrade path: nested trees, if ever
// adopted — this whole function goes away in favor of a plain tree read.
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

    let root_data = odb
        .read(&commit.tree)
        .await
        .map_err(|_| StatusCode::NOT_FOUND)?;
    let root_tree = Tree::deserialize(&root_data).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let prefix = normalize_flat_path(dir_path);
    let scan_prefix = if prefix.is_empty() {
        String::new()
    } else {
        format!("{}/", prefix)
    };

    let mut listing = Tree::new();
    let mut seen_dirs = std::collections::HashSet::new();
    let mut any_match = prefix.is_empty();
    for (key, entry) in root_tree.entries.range(scan_prefix.clone()..) {
        let Some(rest) = key.strip_prefix(scan_prefix.as_str()) else {
            break;
        };
        any_match = true;
        match rest.split_once('/') {
            Some((dir, _)) => {
                if seen_dirs.insert(dir.to_string()) {
                    let dir_full_path = format!("{}{}", scan_prefix, dir);
                    listing.add_entry(TreeEntry::new(
                        dir.to_string(),
                        FileMode::Directory,
                        Oid::hash(dir_full_path.as_bytes()),
                    ));
                }
            }
            None => {
                listing.add_entry(TreeEntry::new(rest.to_string(), entry.mode, entry.oid));
            }
        }
    }
    if !any_match {
        return Err(StatusCode::NOT_FOUND);
    }

    Ok((commit_oid, listing))
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
                if let Ok(entry) = serde_json::from_str::<PackIndexLine>(line)
                    && !entry.chunk_oid.is_empty()
                    && !entry.pack_oid.is_empty()
                {
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

    let mut idx = state.pack_index.write().await;
    idx.insert(repo.to_string(), repo_entries);
    Ok(())
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use mediagit_security::auth::middleware::AuthMethod;

    /// `MEDIAGIT_GRANTS_ENFORCE` is a process-wide env var (like
    /// `MEDIAGIT_AUTH_PERSIST` in `persist.rs`). Only one test below
    /// mutates it; every other test relies on it being unset, so mutators
    /// take the write side and everyone else takes the read side to avoid
    /// observing a torn value under `cargo test`'s multi-threaded runner.
    static GRANTS_ENV_LOCK: std::sync::RwLock<()> = std::sync::RwLock::new(());

    fn user(permissions: &[&str]) -> AuthUser {
        AuthUser {
            user_id: "user1".to_string(),
            permissions: permissions.iter().map(|p| p.to_string()).collect(),
            auth_method: AuthMethod::Jwt,
        }
    }

    #[test]
    fn auth_disabled_allows_everyone() {
        let _guard = GRANTS_ENV_LOCK.read().unwrap();
        let grants = GrantsStore::new();
        assert!(check_permission(None, "repo:read", false, &grants, "repoA").is_ok());
    }

    #[test]
    fn no_user_rejected_when_auth_enabled() {
        let _guard = GRANTS_ENV_LOCK.read().unwrap();
        let grants = GrantsStore::new();
        assert_eq!(
            check_permission(None, "repo:read", true, &grants, "repoA").unwrap_err(),
            StatusCode::UNAUTHORIZED
        );
    }

    #[test]
    fn zero_grants_backward_compat_uses_flat_role() {
        let _guard = GRANTS_ENV_LOCK.read().unwrap();
        // Empty store -> pre-H2 behavior: flat role permissions decide,
        // regardless of which repo is being accessed.
        let grants = GrantsStore::new();
        let reader = user(&["repo:read"]);
        assert!(check_permission(Some(&reader), "repo:read", true, &grants, "repoA").is_ok());
        assert!(check_permission(Some(&reader), "repo:write", true, &grants, "repoA").is_err());
    }

    #[tokio::test]
    // Deliberately holds the env lock across awaits (see GRANTS_ENV_LOCK).
    #[allow(clippy::await_holding_lock)]
    async fn admin_role_bypasses_grants_entirely() {
        let _guard = GRANTS_ENV_LOCK.read().unwrap();
        let grants = GrantsStore::new();
        grants
            .grant("other", "repoA", GrantLevel::Read)
            .await
            .unwrap();
        let admin = user(&["repo:read", "repo:write", "repo:admin", "user:manage"]);

        // Admin has no grant recorded at all for repoB, yet still passes.
        assert!(check_permission(Some(&admin), "repo:admin", true, &grants, "repoA").is_ok());
        assert!(check_permission(Some(&admin), "repo:write", true, &grants, "repoB").is_ok());
    }

    #[tokio::test]
    // Deliberately holds the env lock across awaits (see GRANTS_ENV_LOCK).
    #[allow(clippy::await_holding_lock)]
    async fn per_repo_grant_allow_deny_matrix() {
        let _guard = GRANTS_ENV_LOCK.read().unwrap();
        let grants = GrantsStore::new();
        // Flat role says read-only, but the per-repo grant says write —
        // once grants are active (store non-empty) the grant wins.
        let requester = user(&["repo:read"]);
        grants
            .grant("user1", "repoA", GrantLevel::Write)
            .await
            .unwrap();

        assert!(check_permission(Some(&requester), "repo:read", true, &grants, "repoA").is_ok());
        assert!(check_permission(Some(&requester), "repo:write", true, &grants, "repoA").is_ok());
        assert!(check_permission(Some(&requester), "repo:admin", true, &grants, "repoA").is_err());

        // AU-4: repoB has no grants recorded, so it is governed by the flat
        // role — a grant on repoA no longer changes repoB's authorization
        // mode. This assertion previously expected denial, encoding the
        // footgun: recording one grant to onboard one tenant silently locked
        // every other user out of every other repo.
        assert!(check_permission(Some(&requester), "repo:read", true, &grants, "repoB").is_ok());
    }

    #[tokio::test]
    // Deliberately holds the env lock across awaits (see GRANTS_ENV_LOCK).
    #[allow(clippy::await_holding_lock)]
    async fn grants_enforce_strict_denies_ungranted_repos() {
        // AU-4: operators who want the fail-closed posture the old global
        // behaviour gave by accident can now ask for it explicitly, rather
        // than triggering it by recording an unrelated grant.
        let _guard = GRANTS_ENV_LOCK.write().unwrap();
        mediagit_test_utils::set_var("MEDIAGIT_GRANTS_ENFORCE", "strict");

        let grants = GrantsStore::new();
        grants
            .grant("user1", "repoA", GrantLevel::Read)
            .await
            .unwrap();
        let requester = user(&["repo:read"]);

        let granted = check_permission(Some(&requester), "repo:read", true, &grants, "repoA");
        let ungranted = check_permission(Some(&requester), "repo:read", true, &grants, "repoB");

        mediagit_test_utils::remove_var("MEDIAGIT_GRANTS_ENFORCE");
        assert!(granted.is_ok(), "granted repo should be allowed");
        assert!(
            ungranted.is_err(),
            "strict mode must deny a repo with no grants instead of falling              back to the flat role"
        );
    }

    #[tokio::test]
    // Deliberately holds the env lock across awaits (see GRANTS_ENV_LOCK).
    #[allow(clippy::await_holding_lock)]
    async fn grants_enforce_opt_out_falls_back_to_flat_role() {
        let _guard = GRANTS_ENV_LOCK.write().unwrap();
        mediagit_test_utils::set_var("MEDIAGIT_GRANTS_ENFORCE", "0");

        let grants = GrantsStore::new();
        grants
            .grant("user1", "repoA", GrantLevel::Read)
            .await
            .unwrap();
        let requester = user(&["repo:read", "repo:write"]);

        // Grant only covers Read, but MEDIAGIT_GRANTS_ENFORCE=0 disables
        // per-repo enforcement entirely, so the flat role (which has
        // "repo:write") is used instead.
        let result = check_permission(Some(&requester), "repo:write", true, &grants, "repoA");

        mediagit_test_utils::remove_var("MEDIAGIT_GRANTS_ENFORCE");
        assert!(result.is_ok());
    }

    #[tokio::test]
    // Deliberately holds the env lock across awaits (see GRANTS_ENV_LOCK).
    #[allow(clippy::await_holding_lock)]
    async fn concurrent_grant_mutations_are_safe() {
        let _guard = GRANTS_ENV_LOCK.read().unwrap();
        let grants = Arc::new(GrantsStore::new());
        let mut tasks = Vec::new();
        for i in 0..20 {
            let grants = Arc::clone(&grants);
            tasks.push(tokio::spawn(async move {
                let user_id = format!("user{}", i % 5);
                grants
                    .grant(&user_id, "repoA", GrantLevel::Write)
                    .await
                    .unwrap();
            }));
        }
        for t in tasks {
            t.await.unwrap();
        }
        for i in 0..5 {
            let user_id = format!("user{}", i);
            assert_eq!(grants.get(&user_id, "repoA"), Some(GrantLevel::Write));
        }
    }

    fn gcs_storage_config(prefix: &str) -> mediagit_config::GCSStorage {
        mediagit_config::GCSStorage {
            bucket: "bucket".to_string(),
            project_id: "project".to_string(),
            credentials_path: None,
            prefix: prefix.to_string(),
        }
    }

    #[test]
    fn gcs_backend_config_threads_configured_prefix() {
        let cfg = gcs_config_with_prefix(&gcs_storage_config("myrepo"));
        assert_eq!(cfg.prefix.as_deref(), Some("myrepo"));
        assert_eq!(cfg.project_id, "project");
        assert_eq!(cfg.bucket_name, "bucket");
    }

    #[test]
    fn gcs_backend_config_empty_prefix_stays_none() {
        let cfg = gcs_config_with_prefix(&gcs_storage_config(""));
        assert_eq!(cfg.prefix, None);
    }
}
