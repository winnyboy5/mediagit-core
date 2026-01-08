use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Extension,
    Json,
};
use bytes::Bytes;
use mediagit_protocol::{
    RefInfo, RefUpdateRequest, RefUpdateResponse, RefUpdateResult, RefsResponse,
    WantRequest,
};
use mediagit_security::auth::AuthUser;
use mediagit_storage::{LocalBackend, StorageBackend, S3Backend, MinIOBackend, AzureBackend};
use mediagit_versioning::{Commit, ObjectDatabase, Oid, ObjectType, PackReader, PackWriter, Ref, RefDatabase, Tree};
use std::path::Path as StdPath;
use std::sync::Arc;

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

                // Set AWS credentials from config if provided
                if let Some(key_id) = &s3_config.access_key_id {
                    std::env::set_var("AWS_ACCESS_KEY_ID", key_id);
                }
                if let Some(secret) = &s3_config.secret_access_key {
                    std::env::set_var("AWS_SECRET_ACCESS_KEY", secret);
                }
                std::env::set_var("AWS_REGION", &s3_config.region);

                let mut backend_config = mediagit_storage::s3::S3Config::default();
                backend_config.bucket = s3_config.bucket.clone();

                let storage = S3Backend::with_config(backend_config)
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
        mediagit_config::StorageConfig::GCS(_) | mediagit_config::StorageConfig::Multi(_) => {
            tracing::error!("GCS and Multi-backend storage are not yet implemented");
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

    // Recursively read all refs in refs/heads, refs/tags, etc.
    if refs_dir.exists() {
        if let Ok(entries) = std::fs::read_dir(&refs_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    // Read refs/heads/, refs/tags/, etc.
                    let ref_type = path.file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or("");

                    if let Ok(ref_entries) = std::fs::read_dir(&path) {
                        for ref_entry in ref_entries.flatten() {
                            if ref_entry.path().is_file() {
                                let ref_name = format!(
                                    "refs/{}/{}",
                                    ref_type,
                                    ref_entry.file_name().to_string_lossy()
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
    }

    Ok(Json(RefsResponse {
        refs: ref_infos,
        capabilities: vec!["pack-v1".to_string()],
    }))
}

/// POST /:repo/objects/pack - Upload a pack file
pub async fn upload_pack(
    Path(repo): Path<String>,
    State(state): State<Arc<AppState>>,
    auth_user: Option<Extension<AuthUser>>,
    body: Bytes,
) -> Result<StatusCode, StatusCode> {
    tracing::info!("POST /{}/objects/pack ({} bytes)", repo, body.len());

    // Check permission: repo:write required
    check_permission(auth_user.as_deref(), "repo:write", state.is_auth_enabled())?;

    let repo_path = state.repos_dir.join(&repo);
    if !repo_path.exists() {
        tracing::warn!("Repository not found: {}", repo);
        return Err(StatusCode::NOT_FOUND);
    }

    // Initialize storage and odb
    let storage = create_storage_backend(&repo_path).await?;
    let odb = ObjectDatabase::with_smart_compression(storage, 1000);

    // Unpack the pack file
    let pack_reader = PackReader::new(body.to_vec()).map_err(|e| {
        tracing::error!("Failed to read pack file: {}", e);
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    for oid in pack_reader.list_objects() {
        let (object_type, obj_data) = pack_reader.get_object_with_type(&oid).map_err(|e| {
            tracing::error!("Failed to get object {}: {}", oid, e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?;
        // Write object to ODB with correct type
        odb.write(object_type, &obj_data)
            .await
            .map_err(|e| {
                tracing::error!("Failed to write object {}: {}", oid, e);
                StatusCode::INTERNAL_SERVER_ERROR
            })?;
    }

    tracing::info!("Successfully unpacked {} bytes", body.len());
    Ok(StatusCode::OK)
}

/// GET /:repo/objects/pack - Download a pack file (after POST to /objects/want)
pub async fn download_pack(
    Path(repo): Path<String>,
    State(state): State<Arc<AppState>>,
    auth_user: Option<Extension<AuthUser>>,
) -> Result<impl IntoResponse, StatusCode> {
    tracing::info!("GET /{}/objects/pack", repo);

    // Check permission: repo:read required
    check_permission(auth_user.as_deref(), "repo:read", state.is_auth_enabled())?;

    // Get the wanted objects from state (stored by POST /objects/want)
    let want_list = {
        let want_map = state.want_cache.lock().await;
        want_map
            .get(&repo)
            .cloned()
            .ok_or(StatusCode::BAD_REQUEST)?
    };

    let repo_path = state.repos_dir.join(&repo);
    if !repo_path.exists() {
        return Err(StatusCode::NOT_FOUND);
    }

    // Initialize storage and odb
    let storage = create_storage_backend(&repo_path).await?;
    let odb = ObjectDatabase::with_smart_compression(storage, 1000);

    // Collect all objects recursively (commit -> tree -> blobs)
    let mut objects_to_pack: Vec<Oid> = Vec::new();
    let mut visited: std::collections::HashSet<Oid> = std::collections::HashSet::new();

    // For clones, simply add all requested OIDs plus their children
    // We read the object, identify type, and if commit/tree, add children
    for oid_str in &want_list {
        let oid = Oid::from_hex(oid_str)
            .map_err(|_| StatusCode::BAD_REQUEST)?;
        
        // Read the object data
        let obj_data = match odb.read(&oid).await {
            Ok(data) => data,
            Err(e) => {
                tracing::error!("Object {} not found: {}", oid, e);
                continue;
            }
        };
        
        objects_to_pack.push(oid);
        
        // Try to parse as Commit to get tree+blob children
        match Commit::deserialize(&obj_data) {
            Ok(commit) => {
                tracing::info!("Found commit {} -> tree {}", oid, commit.tree);
                
                // Add tree
                match odb.read(&commit.tree).await {
                    Ok(tree_data) => {
                        objects_to_pack.push(commit.tree);
                        
                        // Parse tree to get blobs
                        if let Ok(tree) = Tree::deserialize(&tree_data) {
                            for entry in tree.iter() {
                                if !objects_to_pack.contains(&entry.oid) {
                                    objects_to_pack.push(entry.oid);
                                }
                            }
                        }
                    }
                    Err(e) => {
                        tracing::warn!("Tree {} not found: {}", commit.tree, e);
                    }
                }
            }
            Err(e) => {
                // Not a commit - might be tree or blob, log the first 20 bytes for debugging
                tracing::debug!("Object {} not a commit (err: {}), data len: {}, first bytes: {:?}", 
                    oid, e, obj_data.len(), &obj_data[..std::cmp::min(20, obj_data.len())]);
            }
        }
    }

    tracing::info!("Collecting {} objects for pack (from {} requested)", objects_to_pack.len(), want_list.len());

    // Generate pack file with all collected objects
    let mut pack_writer = PackWriter::new();

    for oid in &objects_to_pack {
        let obj_data = odb
            .read(oid)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

        // Determine object type from data header
        let object_type = detect_object_type(&obj_data).unwrap_or(ObjectType::Blob);

        // Add to pack
        let _offset = pack_writer.add_object(*oid, object_type, &obj_data);
    }

    // Finalize pack
    let pack_data = pack_writer.finalize();

    tracing::info!("Sending pack file ({} bytes, {} objects)", pack_data.len(), objects_to_pack.len());

    Ok((
        StatusCode::OK,
        [("Content-Type", "application/octet-stream")],
        pack_data,
    ))
}

/// Recursively collect an object and its children (for commits and trees)
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

    // Try to read the object
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
            // Parse commit to get tree OID using Commit's own deserializer
            if let Ok(commit) = Commit::deserialize(&obj_data) {
                // Collect the tree
                Box::pin(collect_objects_recursive(odb, commit.tree, collected, visited)).await?;
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
pub async fn request_objects(
    Path(repo): Path<String>,
    State(state): State<Arc<AppState>>,
    auth_user: Option<Extension<AuthUser>>,
    Json(want_req): Json<WantRequest>,
) -> Result<StatusCode, StatusCode> {
    tracing::info!(
        "POST /{}/objects/want (want: {}, have: {})",
        repo,
        want_req.want.len(),
        want_req.have.len()
    );

    // Check permission: repo:read required
    check_permission(auth_user.as_deref(), "repo:read", state.is_auth_enabled())?;

    // Store the want list in cache for subsequent GET /objects/pack
    {
        let mut want_map = state.want_cache.lock().await;
        want_map.insert(repo, want_req.want);
    }

    Ok(StatusCode::OK)
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
