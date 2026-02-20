// Copyright (C) 2026  winnyboy5
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU Affero General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU Affero General Public License for more details.
//
// You should have received a copy of the GNU Affero General Public License
// along with this program.  If not, see <https://www.gnu.org/licenses/>.
use axum::{
    extract::{Path, State},
    http::{StatusCode, HeaderMap},
    response::IntoResponse,
    Extension,
    Json,
};
use bytes::Bytes;
use mediagit_protocol::{
    RefInfo, RefUpdateRequest, RefUpdateResponse, RefUpdateResult, RefsResponse,
    WantRequest, WantResponse,
};
use mediagit_security::auth::AuthUser;
use mediagit_storage::{LocalBackend, StorageBackend, MinIOBackend, AzureBackend, GcsBackend};
use mediagit_versioning::{Commit, ObjectDatabase, Oid, ObjectType, Ref, RefDatabase, Tree, StreamingPackWriter};
use std::path::Path as StdPath;
use std::sync::Arc;
use tokio::io::duplex;
use tokio_util::io::ReaderStream;

use crate::state::AppState;

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

/// Helper function to create storage backend based on repository configuration
async fn create_storage_backend(
    repo_path: &StdPath,
) -> Result<Arc<dyn StorageBackend>, StatusCode> {
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
            tracing::debug!("Using filesystem storage backend: {}", storage_path.display());
            let storage = LocalBackend::new(&storage_path)
                .await
                .map_err(|e| {
                    tracing::error!("Failed to initialize filesystem backend: {}", e);
                    StatusCode::INTERNAL_SERVER_ERROR
                })?;
            Arc::new(storage)
        }
        mediagit_config::StorageConfig::S3(s3_config) => {
            // Use MinIOBackend for custom endpoints (MinIO, DigitalOcean Spaces, etc.)
            // Use S3Backend for AWS S3
            if let Some(endpoint) = &s3_config.endpoint {
                tracing::info!(
                    "Using MinIO/S3-compatible backend: bucket={}, endpoint={}",
                    s3_config.bucket,
                    endpoint
                );

                let storage = MinIOBackend::new(
                    endpoint,
                    &s3_config.bucket,
                    s3_config.access_key_id.as_deref().unwrap_or(""),
                    s3_config.secret_access_key.as_deref().unwrap_or(""),
                )
                .await
                .map_err(|e| {
                    tracing::error!("Failed to initialize MinIO backend: {}", e);
                    StatusCode::INTERNAL_SERVER_ERROR
                })?;
                Arc::new(storage)
            } else {
                tracing::info!("Using AWS S3 storage backend: bucket={}", s3_config.bucket);

                // For AWS S3 without custom endpoint, we use MinIOBackend with the AWS S3 endpoint
                // This avoids setting global environment variables which could cause race conditions
                // in concurrent requests.
                let aws_endpoint = format!("https://s3.{}.amazonaws.com", s3_config.region);

                let storage = MinIOBackend::new(
                    &aws_endpoint,
                    &s3_config.bucket,
                    s3_config.access_key_id.as_deref().unwrap_or(""),
                    s3_config.secret_access_key.as_deref().unwrap_or(""),
                )
                .await
                .map_err(|e| {
                    tracing::error!("Failed to initialize S3 backend: {}", e);
                    StatusCode::INTERNAL_SERVER_ERROR
                })?;
                Arc::new(storage)
            }
        }
        mediagit_config::StorageConfig::Azure(azure_config) => {
            tracing::info!("Using Azure storage backend: container={}", azure_config.container);

            // Use connection string if provided, otherwise use account key
            let storage = if let Some(conn_str) = &azure_config.connection_string {
                AzureBackend::with_connection_string(&azure_config.container, conn_str)
                    .await
                    .map_err(|e| {
                        tracing::error!("Failed to initialize Azure backend with connection string: {}", e);
                        StatusCode::INTERNAL_SERVER_ERROR
                    })?
            } else if let Some(account_key) = &azure_config.account_key {
                AzureBackend::with_account_key(
                    &azure_config.account_name,
                    &azure_config.container,
                    account_key,
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

            // Determine credentials path - from config or default application credentials
            let credentials_path = gcs_config
                .credentials_path
                .as_deref()
                .or_else(|| std::env::var("GOOGLE_APPLICATION_CREDENTIALS").ok().as_deref().map(|_| ""))
                .unwrap_or("");

            let storage = if credentials_path.is_empty() {
                // Use default credentials (ADC) - for GKE, Cloud Run, etc.
                GcsBackend::with_default_credentials(&gcs_config.project_id, &gcs_config.bucket)
                    .await
                    .map_err(|e| {
                        tracing::error!("Failed to initialize GCS backend with default credentials: {}", e);
                        StatusCode::INTERNAL_SERVER_ERROR
                    })?
            } else {
                // Use service account JSON file
                GcsBackend::new(&gcs_config.project_id, &gcs_config.bucket, credentials_path)
                    .await
                    .map_err(|e| {
                        tracing::error!("Failed to initialize GCS backend: {}", e);
                        StatusCode::INTERNAL_SERVER_ERROR
                    })?
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
    let _storage = create_storage_backend(&repo_path).await?;
    let refdb = RefDatabase::new(&repo_path.join(".mediagit"));

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
                            let ref_name = format!("refs/{}", relative_path.to_string_lossy().replace('\\', "/"));

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

    // Initialize storage and ODB for proper compression and storage
    let storage = create_storage_backend(&repo_path).await?;
    let odb = ObjectDatabase::with_smart_compression(storage, 1000);

    // Convert body to AsyncRead stream
    use futures::stream::TryStreamExt;
    use tokio_util::io::StreamReader;

    let stream = body
        .into_data_stream()
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e));

    let stream_reader = StreamReader::new(stream);

    // Create streaming pack reader
    let mut reader = mediagit_versioning::StreamingPackReader::new(stream_reader)
        .await
        .map_err(|e| {
            tracing::error!("Failed to create streaming pack reader: {}", e);
            StatusCode::BAD_REQUEST
        })?;

    tracing::info!("Processing streaming pack upload");

    // Process objects incrementally using ODB (proper compression + storage paths)
    let mut object_count = 0;
    while let Some(result) = reader.next_object().await {
        let (oid, obj_type, data) = result.map_err(|e| {
            tracing::error!("Failed to read object from pack stream: {}", e);
            StatusCode::BAD_REQUEST
        })?;

        // Write through ODB which handles compression and correct storage paths
        let stored_oid = odb.write(obj_type, &data)
            .await
            .map_err(|e| {
                tracing::error!("Failed to write object {} to ODB: {}", oid, e);
                StatusCode::INTERNAL_SERVER_ERROR
            })?;

        if stored_oid != oid {
            tracing::warn!(
                expected = %oid,
                actual = %stored_oid,
                "OID mismatch during pack upload (object may have different content)"
            );
        }

        object_count += 1;
        if object_count % 100 == 0 {
            tracing::debug!("Processed {} objects", object_count);
        }
    }

    tracing::info!("Successfully unpacked {} objects (streaming via ODB)", object_count);
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
                        request_id, entry.repo, repo
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

    let repo_path = state.repos_dir.join(&repo);
    if !repo_path.exists() {
        return Err(StatusCode::NOT_FOUND);
    }

    // Initialize storage and odb
    let storage = create_storage_backend(&repo_path).await?;
    let odb = ObjectDatabase::with_smart_compression(storage, 1000);

    // Collect all objects recursively (commit -> tree -> blobs)
    // Use HashSet for O(1) contains checks, Vec for maintaining insertion order
    let mut objects_to_pack: Vec<Oid> = Vec::new();
    let mut seen_objects: std::collections::HashSet<Oid> = std::collections::HashSet::new();

    // Recursively collect all objects reachable from wanted OIDs
    // This properly handles nested trees (subdirectories) and parent commits (history)
    for oid_str in &want_list {
        let oid = Oid::from_hex(oid_str)
            .map_err(|_| StatusCode::BAD_REQUEST)?;

        // Use recursive collection to get all commits, trees, and blobs
        collect_objects_recursive(&odb, oid, &mut objects_to_pack, &mut seen_objects)
            .await
            .map_err(|e| {
                tracing::error!("Failed to collect objects from {}: {}", oid, e);
                StatusCode::INTERNAL_SERVER_ERROR
            })?;
    }

    tracing::info!("Collecting {} objects for pack (from {} requested)", objects_to_pack.len(), want_list.len());

    // Filter out chunked objects - they'll be transferred separately
    let mut chunked_objects: Vec<String> = Vec::new();
    let mut non_chunked_objects: Vec<Oid> = Vec::new();

    for oid in &objects_to_pack {
        if odb.is_chunked(oid).await.unwrap_or(false) {
            tracing::debug!(oid = %oid, "Skipping chunked blob in pack generation");
            chunked_objects.push(oid.to_hex());
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
    use axum::response::Response;
    use axum::http::header;

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
            let mut pack_writer = StreamingPackWriter::new(
                writer,
                object_count,
                &temp_dir,
            ).await
                .map_err(|e| anyhow::anyhow!("Failed to create streaming pack writer: {}", e))?;

            for oid in objects_to_stream {
                let obj_data = odb_clone.read(&oid).await?;
                let obj_type = detect_object_type(&obj_data).unwrap_or(ObjectType::Blob);
                pack_writer.write_object(oid, obj_type, &obj_data).await
                    .map_err(|e| anyhow::anyhow!("Failed to write object {}: {}", oid, e))?;
            }

            pack_writer.finalize().await
                .map_err(|e| anyhow::anyhow!("Failed to finalize pack: {}", e))?;
            
            tracing::info!("Streaming pack generation completed successfully");
            Ok(())
        }.await;

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

    response_builder.body(body)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)
}

/// Recursively collect an object and its children (for commits and trees).
/// Used by download_pack to ensure all nested objects are included in packs.
async fn collect_objects_recursive(
    odb: &ObjectDatabase,
    oid: Oid,
    collected: &mut Vec<Oid>,
    visited: &mut std::collections::HashSet<Oid>,
) -> Result<(), anyhow::Error> {
    // Skip if already visited
    if visited.contains(&oid) {
        return Ok(());
    }
    visited.insert(oid);

    // Check if this is a chunked object BEFORE reading - avoids massive memory allocation
    // Chunked objects will be streamed separately, not included in pack
    if odb.is_chunked(&oid).await.unwrap_or(false) {
        tracing::debug!(oid = %oid, "Object is chunked - adding to collection without reading content");
        collected.push(oid);
        return Ok(()); // Chunked blobs have no children to recurse into
    }

    // Try to read the object - only for non-chunked objects
    let obj_data = match odb.read(&oid).await {
        Ok(data) => data,
        Err(e) => {
            tracing::warn!("Object {} not found: {}", oid, e);
            return Ok(()); // Skip missing objects
        }
    };

    // Add this object to collection
    collected.push(oid);

    // Determine type and recursively collect children
    let obj_type = detect_object_type(&obj_data).unwrap_or(ObjectType::Blob);
    
    match obj_type {
        ObjectType::Commit => {
            // Parse commit to get tree OID and parent commits using Commit's own deserializer
            if let Ok(commit) = Commit::deserialize(&obj_data) {
                // Collect the tree
                Box::pin(collect_objects_recursive(odb, commit.tree, collected, visited)).await?;

                // Collect parent commits (REQUIRED for complete history)
                for parent in &commit.parents {
                    Box::pin(collect_objects_recursive(odb, *parent, collected, visited)).await?;
                }
            }
        }
        ObjectType::Tree => {
            // Parse tree to get entry OIDs using Tree's own deserializer
            if let Ok(tree) = Tree::deserialize(&obj_data) {
                for entry in tree.iter() {
                    Box::pin(collect_objects_recursive(odb, entry.oid, collected, visited)).await?;
                }
            }
        }
        ObjectType::Blob => {
            // Blobs have no children
        }
    }

    Ok(())
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

    // Store the want list in cache keyed by request_id (not repo name)
    {
        let mut want_cache = state.want_cache.lock().await;
        want_cache.insert(request_id.clone(), repo, want_req.want);
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

    // Initialize storage and refdb
    let _storage = create_storage_backend(&repo_path).await?;
    let refdb = RefDatabase::new(&repo_path.join(".mediagit"));

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
                            error: Some("not fast-forward".to_string()),
                        });
                        all_success = false;
                        continue;
                    }
                }
            }
        }

        // Update the ref
        let new_oid = Oid::from_hex(&update.new_oid)
            .map_err(|_| StatusCode::BAD_REQUEST)?;
        let ref_update = Ref::new_direct(update.name.clone(), new_oid);

        match refdb.write(&ref_update).await {
            Ok(_) => {
                tracing::info!("Updated {} to {}", update.name, update.new_oid);
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
    check_permission(
        auth_user.as_deref(),
        "repo:write",
        state.is_auth_enabled(),
    )?;

    tracing::debug!(repo = %repo, chunk_count = chunk_ids.len(), "Checking chunk existence");

    // Resolve repository path
    let repo_path = state.repos_dir.join(&repo);
    if !repo_path.exists() {
        tracing::warn!(repo = %repo, "Repository not found");
        return Err(StatusCode::NOT_FOUND);
    }

    // Create storage backend
    let storage = create_storage_backend(&repo_path).await?;

    // Check each chunk and collect missing ones
    let mut missing = Vec::new();
    for chunk_id_hex in chunk_ids {
        let chunk_key = format!("chunks/{}", chunk_id_hex);
        match storage.exists(&chunk_key).await {
            Ok(exists) => {
                if !exists {
                    missing.push(chunk_id_hex);
                }
            }
            Err(e) => {
                tracing::warn!(chunk = %chunk_id_hex, error = %e, "Error checking chunk");
                // Assume missing on error
                missing.push(chunk_id_hex);
            }
        }
    }

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
    check_permission(
        auth_user.as_deref(),
        "repo:write",
        state.is_auth_enabled(),
    )?;

    tracing::debug!(
        repo = %repo,
        chunk_id = %chunk_id,
        size = body.len(),
        "Uploading chunk"
    );

    // Resolve repository path
    let repo_path = state.repos_dir.join(&repo);
    if !repo_path.exists() {
        tracing::warn!(repo = %repo, "Repository not found");
        return Err(StatusCode::NOT_FOUND);
    }

    // Create storage backend
    let storage = create_storage_backend(&repo_path).await?;

    // Store chunk directly (already compressed)
    let chunk_key = format!("chunks/{}", chunk_id);
    storage.put(&chunk_key, &body).await.map_err(|e| {
        tracing::error!(chunk = %chunk_id, error = %e, "Failed to store chunk");
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    tracing::debug!(chunk = %chunk_id, "Chunk stored successfully");
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
    check_permission(
        auth_user.as_deref(),
        "repo:write",
        state.is_auth_enabled(),
    )?;

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
    let storage = create_storage_backend(&repo_path).await?;

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
/// Returns raw compressed chunk data
pub async fn download_chunk(
    Path((repo, chunk_id)): Path<(String, String)>,
    State(state): State<Arc<AppState>>,
    auth_user: Option<Extension<AuthUser>>,
) -> Result<impl IntoResponse, StatusCode> {
    // Check read permission
    check_permission(
        auth_user.as_deref(),
        "repo:read",
        state.is_auth_enabled(),
    )?;

    tracing::debug!(repo = %repo, chunk_id = %chunk_id, "Downloading chunk");

    // Resolve repository path
    let repo_path = state.repos_dir.join(&repo);
    if !repo_path.exists() {
        tracing::warn!(repo = %repo, "Repository not found");
        return Err(StatusCode::NOT_FOUND);
    }

    // Create storage backend
    let storage = create_storage_backend(&repo_path).await?;

    // Read compressed chunk directly (no decompression)
    let chunk_key = format!("chunks/{}", chunk_id);
    let chunk_data = storage.get(&chunk_key).await.map_err(|e| {
        tracing::warn!(chunk = %chunk_id, error = %e, "Chunk not found");
        StatusCode::NOT_FOUND
    })?;

    tracing::debug!(chunk = %chunk_id, size = chunk_data.len(), "Chunk downloaded");
    Ok((
        StatusCode::OK,
        [("Content-Type", "application/octet-stream")],
        chunk_data,
    ))
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
    check_permission(
        auth_user.as_deref(),
        "repo:read",
        state.is_auth_enabled(),
    )?;

    tracing::debug!(repo = %repo, oid = %oid, "Downloading manifest");

    // Resolve repository path
    let repo_path = state.repos_dir.join(&repo);
    if !repo_path.exists() {
        tracing::warn!(repo = %repo, "Repository not found");
        return Err(StatusCode::NOT_FOUND);
    }

    // Create storage backend
    let storage = create_storage_backend(&repo_path).await?;

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
