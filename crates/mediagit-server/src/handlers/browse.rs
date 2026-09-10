// SPDX-License-Identifier: BUSL-1.1
// Copyright (C) 2025-2026 Aswin Krishnamoorthy

use super::*;

// ============================================================================
// Raw File Serving Endpoints — HTTP "Download Raw" equivalent
// ============================================================================

/// Query parameters shared by file and tree endpoints
#[derive(serde::Deserialize)]
pub struct RefQueryParams {
    #[serde(rename = "ref", default = "default_ref_head")]
    ref_name: String,
}

/// JSON shape for a single entry in a tree listing response
#[derive(serde::Serialize)]
pub struct TreeEntryResponse {
    pub(super) name: String,
    pub(super) mode: String,
    pub(super) oid: String,
    #[serde(rename = "type")]
    pub(super) entry_type: String,
}

/// JSON response body for `GET /{repo}/tree[/{path}]`
#[derive(serde::Serialize)]
pub struct TreeListResponse {
    #[serde(rename = "ref")]
    pub(super) ref_name: String,
    pub(super) commit: String,
    pub(super) path: String,
    pub(super) entries: Vec<TreeEntryResponse>,
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
