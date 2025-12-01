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
use mediagit_storage::LocalBackend;
use mediagit_versioning::{ObjectDatabase, Oid, ObjectType, PackReader, PackWriter, Ref, RefDatabase};
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

/// GET /:repo/info/refs - List all refs in the repository
pub async fn get_refs(
    Path(repo): Path<String>,
    State(state): State<Arc<AppState>>,
    auth_user: Option<Extension<AuthUser>>,
) -> Result<Json<RefsResponse>, StatusCode> {
    tracing::info!("GET /{}/info/refs", repo);

    // Check permission: repo:read required
    check_permission(auth_user.as_deref(), "repo:read", state.is_auth_enabled())?;

    let repo_path = state.repos_dir.join(&repo);
    if !repo_path.exists() {
        tracing::warn!("Repository not found: {}", repo);
        return Err(StatusCode::NOT_FOUND);
    }

    // Initialize storage and refdb
    let storage = LocalBackend::new(&repo_path)
        .await
        .map_err(|e| {
            tracing::error!("Failed to initialize storage: {}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    let storage_arc: Arc<dyn mediagit_storage::StorageBackend> = Arc::new(storage);
    let refdb = RefDatabase::new(Arc::clone(&storage_arc));

    // List all refs - read specific known refs (simplified)
    // TODO: implement proper ref listing
    let known_refs = vec!["HEAD", "refs/heads/main"];
    let mut ref_infos = Vec::new();

    for ref_name in known_refs {
        if let Ok(r) = refdb.read(ref_name).await {
            ref_infos.push(RefInfo {
                name: r.name,
                oid: r.oid.map(|o| o.to_hex()).unwrap_or_default(),
                target: r.target,
            });
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
    let storage = LocalBackend::new(&repo_path)
        .await
        .map_err(|e| {
            tracing::error!("Failed to initialize storage: {}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    let odb = ObjectDatabase::new(Arc::new(storage), 1000);

    // Unpack the pack file
    let pack_reader = PackReader::new(body.to_vec()).map_err(|e| {
        tracing::error!("Failed to read pack file: {}", e);
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    for oid in pack_reader.list_objects() {
        let obj_data = pack_reader.get_object(&oid).map_err(|e| {
            tracing::error!("Failed to get object {}: {}", oid, e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?;
        // Write object to ODB (assuming Blob type for now)
        odb.write(ObjectType::Blob, &obj_data)
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
    let storage = LocalBackend::new(&repo_path)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let odb = ObjectDatabase::new(Arc::new(storage), 1000);

    // Generate pack file with wanted objects
    let mut pack_writer = PackWriter::new();

    for oid_str in &want_list {
        let oid = Oid::from_hex(oid_str)
            .map_err(|_| StatusCode::BAD_REQUEST)?;

        let obj_data = odb
            .read(&oid)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

        // TODO: Need to determine object type properly
        // For now, assume Blob type (suitable for media files)
        let object_type = ObjectType::Blob;

        // Add to pack (returns offset as u64, not Result)
        let _offset = pack_writer.add_object(oid, object_type, &obj_data);
    }

    // Finalize pack (returns Vec<u8> directly, not Result)
    let pack_data = pack_writer.finalize();

    tracing::info!("Sending pack file ({} bytes)", pack_data.len());

    Ok((
        StatusCode::OK,
        [("Content-Type", "application/octet-stream")],
        pack_data,
    ))
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
    let storage = LocalBackend::new(&repo_path)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let refdb = RefDatabase::new(Arc::new(storage));

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
