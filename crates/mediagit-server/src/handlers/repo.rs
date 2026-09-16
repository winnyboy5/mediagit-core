// SPDX-License-Identifier: BUSL-1.1
// Copyright (C) 2025-2026 Aswin Krishnamoorthy

use super::*;

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
    check_permission(
        auth_user.as_deref(),
        "repo:read",
        state.is_auth_enabled(),
        &state.grants,
        &repo,
    )?;

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

    // `api-v1` is the frozen wire contract (Phase 9): the response shapes in
    // `mediagit_protocol::types`, pinned by `tests/api_contract.rs`. A client
    // or the 1.0 UI keys off it to tell "this server speaks a protocol I
    // understand" from "this server is newer than me". Additive changes keep
    // this token; anything that removes, renames or retypes a field must
    // introduce `api-v2` rather than redefine v1 under clients' feet.
    let mut capabilities = vec!["pack-v1".to_string(), "api-v1".to_string()];

    // Advertise the repo's CDC seed (if any) so clones inherit matching chunk
    // boundaries. Omitted entirely when the seed is 0 (legacy repos / repos
    // without the field) to keep the capability list unchanged for them.
    let cdc_seed = mediagit_config::Config::load(&repo_path)
        .await
        .map(|c| c.cdc_seed)
        .unwrap_or(0);
    if cdc_seed != 0 {
        capabilities.push(format!("cdc-seed={}", cdc_seed));
    }

    Ok(Json(RefsResponse {
        refs: ref_infos,
        capabilities,
    }))
}

/// Bytes of a rejected request body we will read and discard before giving up.
///
/// Bounded because a rejected 10 GiB push is not worth the bandwidth; past this
/// point the reset is the honest outcome.
const REJECT_DRAIN_LIMIT: usize = 8 * 1024 * 1024;

/// Return a status for a *streaming* request without the client losing it to a
/// connection reset.
///
/// Returning early from a handler drops the unread body. hyper then resets the
/// connection, and a client still writing sees ECONNRESET instead of the status
/// we sent — so a `403` on `POST /objects/pack` reached the user as "An
/// existing connection was forcibly closed by the remote host", an unactionable
/// network error for what is really "you lack push permission". Reading the
/// body first lets the client's write side finish so it can read the response.
///
/// Only `upload_pack` needs this: every other body-taking handler uses the
/// `Bytes` extractor, which buffers the body *before* the handler runs.
async fn reject_streamed(body: axum::body::Body, code: StatusCode) -> StatusCode {
    use futures::stream::StreamExt;
    let mut stream = body.into_data_stream();
    let mut seen = 0usize;
    while let Some(Ok(chunk)) = stream.next().await {
        seen += chunk.len();
        if seen >= REJECT_DRAIN_LIMIT {
            break;
        }
    }
    code
}

/// POST /:repo/objects/pack - Upload a pack file (streaming)
pub async fn upload_pack(
    path: Path<String>,
    state: State<Arc<AppState>>,
    auth_user: Option<Extension<AuthUser>>,
    body: axum::body::Body,
) -> Result<StatusCode, StatusCode> {
    // DC-4: record around the whole handler rather than at the success return.
    // The body has a dozen `?` exits, and instrumenting only the happy path
    // leaves the error counter at zero — the one number an operator alerts on.
    let started = std::time::Instant::now();
    let app = Arc::clone(&state.0);
    let result = upload_pack_inner(path, state, auth_user, body).await;
    app.record_op(MetricOp::Store, started, result.is_ok());
    result
}

async fn upload_pack_inner(
    Path(repo): Path<String>,
    State(state): State<Arc<AppState>>,
    auth_user: Option<Extension<AuthUser>>,
    body: axum::body::Body,
) -> Result<StatusCode, StatusCode> {
    tracing::info!("POST /{}/objects/pack (streaming)", repo);

    // Check permission: repo:write required
    if let Err(code) = check_permission(
        auth_user.as_deref(),
        "repo:write",
        state.is_auth_enabled(),
        &state.grants,
        &repo,
    ) {
        return Err(reject_streamed(body, code).await);
    }

    let repo_path = state.repos_dir.join(&repo);
    if !repo_path.exists() {
        tracing::warn!("Repository not found: {}", repo);
        return Err(reject_streamed(body, StatusCode::NOT_FOUND).await);
    }

    // Initialize ODB (shared per-repo so the delta_written_pairs graph is shared
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
    path: Path<String>,
    state: State<Arc<AppState>>,
    auth_user: Option<Extension<AuthUser>>,
    headers: HeaderMap,
) -> Result<axum::response::Response<axum::body::Body>, StatusCode> {
    // DC-4: see `upload_pack`. Measures time to *build* the response; the pack
    // body streams afterwards, so this is negotiation-and-walk latency, not
    // transfer time. Conflating the two would make a slow link read as a slow
    // server.
    let started = std::time::Instant::now();
    let app = Arc::clone(&state.0);
    let result = download_pack_inner(path, state, auth_user, headers).await;
    app.record_op(MetricOp::Retrieve, started, result.is_ok());
    result
}

async fn download_pack_inner(
    Path(repo): Path<String>,
    State(state): State<Arc<AppState>>,
    auth_user: Option<Extension<AuthUser>>,
    headers: HeaderMap,
) -> Result<axum::response::Response<axum::body::Body>, StatusCode> {
    tracing::info!("GET /{}/objects/pack", repo);

    // Check permission: repo:read required
    check_permission(
        auth_user.as_deref(),
        "repo:read",
        state.is_auth_enabled(),
        &state.grants,
        &repo,
    )?;

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
    //
    // M3 bitmap short-circuit: for each have OID with a valid, current-
    // version bitmap (`bitmaps/<oid>.bitmap`), use its precomputed closure
    // instead of walking the ODB. Any miss/stale/corrupt/version-mismatch/
    // disabled falls back to BFS for that OID — bitmaps are a pure speedup,
    // never a correctness dependency (see `mediagit_versioning::bitmap`).
    let have_oids: Vec<Oid> = have_list
        .iter()
        .filter_map(|s| Oid::from_hex(s).ok())
        .collect();
    // Fast path: empty have-set (clone) skips the expensive BFS expansion.
    let stop_at = if have_oids.is_empty() {
        std::collections::HashSet::new()
    } else if mediagit_versioning::bitmap_enabled() {
        let mut stop_at = std::collections::HashSet::new();
        let mut bfs_roots = Vec::new();
        let mut bitmap_hits = 0usize;
        for have in &have_oids {
            let key = mediagit_versioning::bitmap_key(have);
            let hit = match odb.get_bitmap(&key).await {
                Ok(bytes) => mediagit_versioning::ReachabilityBitmap::deserialize(&bytes),
                Err(_) => None,
            };
            match hit {
                Some(bitmap) => {
                    bitmap_hits += 1;
                    state
                        .bitmap_hits
                        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    stop_at.extend(bitmap.to_oid_set());
                }
                None => bfs_roots.push(*have),
            }
        }
        if !bfs_roots.is_empty() {
            let empty: std::collections::HashSet<Oid> = std::collections::HashSet::new();
            let bfs_extra = mediagit_versioning::walk_reachable(&odb, bfs_roots, &empty)
                .await
                .map_err(|e| {
                    tracing::error!("Failed to expand have-closure: {}", e);
                    StatusCode::INTERNAL_SERVER_ERROR
                })?;
            stop_at.extend(bfs_extra);
        }
        tracing::info!(
            "Have-closure: {} bitmap hit(s), {} BFS fallback root(s) of {} have OIDs",
            bitmap_hits,
            have_oids.len() - bitmap_hits,
            have_oids.len()
        );
        stop_at
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
    let objects_to_pack = match collect_objects_bfs(&odb, want_oids, &stop_at).await {
        Ok(objects) => objects,
        Err(e) => {
            // Answer with a body rather than a bare 500: an incomplete closure
            // is an operator-actionable condition, and a status code alone
            // leaves the client reporting "500 Internal Server Error" for a
            // repository that needs fsck. Only `client_message()` is surfaced,
            // so backend paths and internal errors stay out of the response.
            tracing::error!("Failed to collect objects: {}", e);
            let body = e.client_message();
            return axum::response::Response::builder()
                .status(StatusCode::INTERNAL_SERVER_ERROR)
                .header("content-type", "text/plain; charset=utf-8")
                .body(axum::body::Body::from(body))
                .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR);
        }
    };

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
    check_permission(
        auth_user.as_deref(),
        "repo:read",
        state.is_auth_enabled(),
        &state.grants,
        &repo,
    )?;

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
    check_permission(
        auth_user.as_deref(),
        "repo:write",
        state.is_auth_enabled(),
        &state.grants,
        &repo,
    )?;

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
            if let Ok(head) = refdb.read("HEAD").await
                && head.target.as_deref() == Some(&update.name)
            {
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

            // Safety check: verify old_oid matches (if provided)
            if let Some(expected_old) = &update.old_oid
                && let Ok(current_ref) = refdb.read(&update.name).await
                && let Some(current_oid) = &current_ref.oid
            {
                let current_oid_str = current_oid.to_hex();
                // The lease survives `force_with_lease` — that mode exists to keep
                // this check while dropping the ancestry one. Plain `force` still
                // bypasses it. Applies to deletes too: "delete it, but only if
                // nobody else moved it" is the same guarantee.
                if &current_oid_str != expected_old && (!req.force || req.force_with_lease) {
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
        if let Some(expected_old) = &update.old_oid
            && let Ok(current_ref) = refdb.read(&update.name).await
            && let Some(current_oid) = &current_ref.oid
        {
            let current_oid_str = current_oid.to_hex();
            // The lease survives `force_with_lease` — that mode exists to keep
            // this check while dropping the ancestry one. Plain `force` still
            // bypasses it. Applies to deletes too: "delete it, but only if
            // nobody else moved it" is the same guarantee.
            if &current_oid_str != expected_old && (!req.force || req.force_with_lease) {
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

        // Ancestry check: when neither force mode is set and the ref already
        // exists, require that the new commit is a descendant of the current
        // tip (fast-forward only). `force_with_lease` waives exactly this and
        // nothing else — the CAS above still applies.
        if !req.force
            && !req.force_with_lease
            && !update.delete
            && let Ok(current_ref) = refdb.read(&update.name).await
            && let Some(current_oid) = &current_ref.oid
        {
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

        // Capture the pre-write OID for reflog
        let pre_write_oid = match refdb.read(&update.name).await {
            Ok(current_ref) => current_ref.oid,
            _ => None,
        };

        let new_oid = Oid::from_hex(&update.new_oid).map_err(|_| StatusCode::BAD_REQUEST)?;

        // B3: server-enforced file locking. Reject the push if any commit
        // between the current tip (pre_write_oid) and new_oid touches a path
        // locked by someone other than the pusher. check_push_locks
        // short-circuits before any tree walk when the repo has zero locks.
        let pusher = auth_user.as_ref().map(|u| u.user_id.as_str());
        match crate::locks::check_push_locks(
            &state,
            &repo,
            &repo_path,
            &odb,
            pre_write_oid,
            new_oid,
            pusher,
        )
        .await
        {
            Ok(Some(lock_error)) => {
                tracing::warn!("Push to '{}' rejected: {}", update.name, lock_error);
                results.push(RefUpdateResult {
                    ref_name: update.name.clone(),
                    success: false,
                    error: Some(lock_error),
                });
                all_success = false;
                continue;
            }
            Ok(None) => {}
            Err(status) => return Err(status),
        }

        // Update the ref
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

                // M3 (#2b): post-receive bitmap generation for the new tip.
                // Derived data — generated in the background so it never adds
                // push latency; a failure here is logged and otherwise
                // ignored (walk_reachable BFS fallback covers the miss).
                if mediagit_versioning::bitmap_enabled() {
                    let odb_for_bitmap = Arc::clone(&odb);
                    let ref_name = update.name.clone();
                    let old_oid = pre_write_oid;
                    tokio::spawn(async move {
                        match mediagit_versioning::ReachabilityBitmap::generate(
                            odb_for_bitmap.as_ref(),
                            new_oid,
                        )
                        .await
                        {
                            Ok(bitmap) => match bitmap.serialize() {
                                Ok(bytes) => {
                                    let key = mediagit_versioning::bitmap_key(&new_oid);
                                    match odb_for_bitmap.put_bitmap(&key, &bytes).await {
                                        Err(e) => {
                                            tracing::warn!(
                                                "Failed to persist bitmap for '{}' ({}): {}",
                                                ref_name,
                                                new_oid,
                                                e
                                            );
                                        }
                                        _ => {
                                            if let Some(old_oid) = old_oid {
                                                // Retention: prune the previous tip's bitmap now that
                                                // the new tip's bitmap is safely persisted. Two refs
                                                // pointing at the same tip share one bitmap key;
                                                // deleting it when one ref moves off it is safe because
                                                // bitmaps are pure speedup — a miss falls back to the
                                                // BFS walk, and gc/next-push regenerates as needed.
                                                // Steady state is ~one bitmap per ref; gc's
                                                // regenerate_and_prune_bitmaps remains the backstop for
                                                // orphans (deleted branches, forced moves).
                                                if old_oid != new_oid {
                                                    let old_key =
                                                        mediagit_versioning::bitmap_key(&old_oid);
                                                    if let Err(e) =
                                                        odb_for_bitmap.delete_bitmap(&old_key).await
                                                    {
                                                        tracing::warn!(
                                                            "Failed to delete previous bitmap for '{}' ({}): {}",
                                                            ref_name,
                                                            old_oid,
                                                            e
                                                        );
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                                Err(e) => tracing::warn!(
                                    "Failed to serialize bitmap for '{}' ({}): {}",
                                    ref_name,
                                    new_oid,
                                    e
                                ),
                            },
                            Err(e) => tracing::warn!(
                                "Failed to generate bitmap for '{}' ({}): {}",
                                ref_name,
                                new_oid,
                                e
                            ),
                        }
                    });
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

// ============================================================================
// Pack Manifest Endpoints (F6) — Track-F cloud pack bundling
// ============================================================================

#[derive(serde::Deserialize, Clone)]
pub struct ManifestEntry {
    pub chunk_oid: String,
    pub offset: u64,
    pub length: u32,
    /// BLAKE3 of the compressed chunk bytes (absent on old manifests).
    #[serde(default)]
    pub compressed_hash: Option<String>,
}

#[derive(serde::Deserialize)]
pub struct CompletePackRequest {
    pub pack_oid: String,
    pub manifest: Vec<ManifestEntry>,
}

/// Pack-manifest parity check (M1): returns the first manifest entry (if
/// any) whose `[offset, offset+length)` range exceeds `pack_size`. Pulled out
/// as a pure function so the boundary logic is unit-testable without an HTTP
/// harness or a live storage backend.
fn first_entry_exceeding_pack_size(
    manifest: &[ManifestEntry],
    pack_size: u64,
) -> Option<&ManifestEntry> {
    manifest
        .iter()
        .find(|entry| entry.offset.saturating_add(entry.length as u64) > pack_size)
}

/// Adapts a `blake3::Hasher` to `std::io::Write` so a streaming decompressor
/// can feed it decompressed bytes directly — no buffer ever holds more than
/// one `decompress_streaming` copy chunk (64 KiB) at a time.
/// Reader that BLAKE3-hashes every byte as it passes through.
///
/// Exists because `decompress_typed` (the whole-buffer path) falls back to the
/// RAW bytes when a decoder errors — `detect()` can misfire on incompressible
/// data that happens to start with a codec magic (raw 0x78 plus a byte that
/// satisfies zlib's header checksum, ~1 in 8000 per the note in
/// `decompress_typed`). Streaming reused `detect()` but NOT that recovery, so
/// such a chunk decoded to an error and was FALSELY QUARANTINED. Caught by a
/// real 1 GB push to S3: `bad_entries=9` on a pack that was entirely valid,
/// with 2006 green unit tests behind it.
///
/// Teeing lets both candidate digests be computed in ONE pass with no
/// buffering: the decompressed hash from the sink, and the raw hash from here.
struct TeeHasher<R: std::io::Read> {
    inner: R,
    hasher: blake3::Hasher,
    /// FUSED. The inner reader is a `SyncIoBridge` over a futures `Stream`, and
    /// polling one of those after it has completed panics inside
    /// `futures-util::stream::unfold`. The post-decode drain below reads until
    /// EOF, so without this flag a successful decode (which already consumed to
    /// EOF) would be polled once more and blow up on a worker thread.
    eof: bool,
    /// Set when the underlying reader returned an I/O error, i.e. the slice was
    /// never fully read. Without this, a transient transport failure produces a
    /// digest over a PARTIAL slice, which is indistinguishable from a genuine
    /// hash mismatch — and was quarantining valid data (measured: 26 valid
    /// chunks evicted across 2 packs on a clean 1 GB S3 push, 2026-08-03).
    io_error: bool,
    /// Bytes actually observed. A backend that ends the body early WITHOUT an
    /// error (a short/truncated range response) is the silent twin of
    /// `io_error`: the digest would cover a prefix and mismatch. Counting lets
    /// the caller tell "read it all" from "read some of it".
    bytes_seen: u64,
}

impl<R: std::io::Read> std::io::Read for TeeHasher<R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if self.eof {
            return Ok(0);
        }
        let n = match self.inner.read(buf) {
            Ok(n) => n,
            Err(e) => {
                // Record that the SLICE could not be fully read, as distinct from
                // "the bytes decoded to the wrong digest". Both abort hashing, but
                // only the latter is evidence of corruption — see
                // `EntryVerification::Unreadable`. Fuse afterwards so a failed
                // stream is not polled again.
                self.io_error = true;
                self.eof = true;
                return Err(e);
            }
        };
        if n == 0 {
            self.eof = true;
        } else {
            self.hasher.update(&buf[..n]);
            self.bytes_seen += n as u64;
        }
        Ok(n)
    }
}

struct HashWriter(blake3::Hasher);

impl std::io::Write for HashWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.update(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// Outcome of verifying ONE pack entry.
///
/// The distinction between [`Self::Corrupt`] and [`Self::Unreadable`] is a
/// data-safety boundary, not a nicety. Quarantining evicts entries from the
/// manifest permanently; doing that because a range GET timed out destroys
/// perfectly good data. "I could not verify this" must never be collapsed into
/// "this is corrupt".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EntryVerification {
    /// Slice was read in full and hashed to its claimed id.
    Verified,
    /// Slice was read IN FULL and hashed to something else. The only state that
    /// justifies quarantine.
    Corrupt,
    /// Slice could not be read to completion (transport error, join failure).
    /// Says nothing about the bytes — the pack simply stays unverified and is
    /// retried later.
    Unreadable,
}

/// Core streaming verification, shared by the first-bad-entry and
/// all-bad-entries views below.
///
/// Single streaming path, no size threshold: each entry is read via
/// `get_streaming_range` and fed through [`SmartCompressor::decompress_streaming`]
/// straight into an incremental BLAKE3 hasher, so peak memory is bounded by a
/// fixed per-entry copy buffer × concurrency — independent of both pack size
/// and individual chunk size. No full pack, entry, or decompressed chunk is
/// ever materialized. This makes the "PACK_BYTES × concurrency" OOM shape
/// (see ST-4 notes on push RAM) structurally unreachable rather than merely
/// avoided below a threshold.
///
/// Returns `(corrupt, unreadable)` index lists, each sorted ascending — not
/// "first future to resolve": concurrency must not make the reported
/// entry/entries depend on network timing.
async fn pack_entries_failing_content_verification(
    storage: &Arc<dyn StorageBackend>,
    compressor: &Arc<SmartCompressor>,
    pack_key: &str,
    manifest: &[ManifestEntry],
) -> (Vec<usize>, Vec<usize>) {
    use futures::stream::{StreamExt, TryStreamExt};
    use tokio_util::io::{StreamReader, SyncIoBridge};

    // One `get_range` PER ENTRY, run concurrently, so N WAN latencies don't
    // serialise. `complete_chunk_uploads` and `verify_chunk_integrity` both
    // already fan out with `buffer_unordered`; this was the odd one out.
    //
    // Concurrency is bounded, not unbounded, and the bound is 16 on every
    // repository -- encrypted or not.
    //
    // The 16 was chosen on the promise that each in-flight verification holds
    // only a fixed-size copy buffer, and `SmartCompressor::decompress_streaming`
    // does abandon streaming for a sealed object, so on an encrypted repository
    // a slot costs one whole object instead. This briefly halved the bound to 8
    // on that ground. Measured, that cost **34% of push wall time** on an
    // 824 MB corpus -- against a 5% budget -- while buying headroom this
    // workload never needed: entries here are chunks, roughly 1 MiB each, so
    // the difference is ~8 MiB of peak. Trading a third of push throughput on
    // the differentiator path for that is not a good trade.
    //
    // `MEDIAGIT_PACK_VERIFY_CONCURRENCY` is the knob for an operator who does
    // hold large non-chunked objects and wants the peak bounded harder.
    //
    // This is the PER-ENTRY axis (range-reads inside one pack) and this name owns
    // it. How many PACKS verify at once is a separate axis with a separate name,
    // `MEDIAGIT_PACK_VERIFY_PACK_CONCURRENCY` (see `pack_verify_semaphore`).
    // Until 2026-08-20 one name drove both, at two different defaults.
    let range_concurrency: usize = std::env::var("MEDIAGIT_PACK_VERIFY_CONCURRENCY")
        .ok()
        .and_then(|v| v.parse().ok())
        .filter(|&n: &usize| n > 0)
        .unwrap_or(16);
    // Futures yield an INDEX, not a `&ManifestEntry`: keeping the borrow out of the
    // async block is what lets these be spawned concurrently without the lifetime
    // fighting `buffer_unordered`'s HRTB inference.
    let owned: Vec<(usize, u64, u64, String)> = manifest
        .iter()
        .enumerate()
        .map(|(i, e)| (i, e.offset, e.length as u64, e.chunk_oid.clone()))
        .collect();

    // (entry index, verdict, ms to open the range, ms to read+hash it)
    let results: Vec<(usize, EntryVerification, u64, u64)> =
        futures::stream::iter(owned.into_iter().map(|(idx, offset, length, chunk_oid)| {
            let storage = Arc::clone(storage);
            let compressor = Arc::clone(compressor);
            let pack_key = pack_key.to_string();
            async move {
                // Same 5-byte pack entry header `[type:1][size:4]` that
                // `read_and_verify_chunk` skips — see its comment for the layout.
                // A too-short entry is a manifest-level defect, not a transport
                // one: the claim itself is impossible, so this IS corruption.
                if length < 5 {
                    return (idx, EntryVerification::Corrupt, 0u64, 0u64);
                }
                // Split timing: `open_ms` is how long the backend took to ANSWER,
                // `read_ms` is how long the bytes took to arrive. Those are two
                // different faults — a storage service throttling request starts
                // looks nothing like a transfer that crawls — and without the
                // split, a slow pack is just one opaque `elapsed_ms`, which is
                // exactly what made the 2026-08-20 stall cost hours to diagnose.
                let entry_start = std::time::Instant::now();
                let stream = match storage
                    .get_streaming_range(&pack_key, (offset + 5)..(offset + length))
                    .await
                {
                    Ok(s) => s,
                    // Could not verify != verified-bad. Reads still gate on this
                    // pack (it stays unverified), but nothing is evicted.
                    Err(e) => {
                        tracing::warn!(
                            pack = %pack_key,
                            entry = idx,
                            err = %e,
                            "pack verification: range read failed; entry left unverified"
                        );
                        return (
                            idx,
                            EntryVerification::Unreadable,
                            entry_start.elapsed().as_millis() as u64,
                            0u64,
                        );
                    }
                };
                let open_ms = entry_start.elapsed().as_millis() as u64;
                let async_reader = StreamReader::new(stream.map_err(std::io::Error::other));
                // Must be constructed here (captures the current Tokio Handle) and
                // then moved into spawn_blocking — never used directly on an async
                // worker thread.
                let sync_reader = SyncIoBridge::new(async_reader);
                let outcome = tokio::task::spawn_blocking(move || {
                    // Compute BOTH candidate digests in one pass: the decompressed
                    // bytes (normal case) and the raw bytes (the fallback
                    // `decompress_typed` applies when a decoder errors on a
                    // misdetected codec). Accepting either mirrors the whole-buffer
                    // path exactly. Still fail-closed: a chunk matching NEITHER is
                    // rejected.
                    let expected_len = length - 5;
                    let mut tee = TeeHasher {
                        inner: sync_reader,
                        hasher: blake3::Hasher::new(),
                        eof: false,
                        io_error: false,
                        bytes_seen: 0,
                    };
                    let mut decompressed = HashWriter(blake3::Hasher::new());
                    let decoded_ok = compressor
                        .decompress_streaming(&mut tee, &mut decompressed)
                        .is_ok();
                    // Drain whatever the decoder did not consume, so the RAW digest
                    // covers the whole slice even when decoding aborted early.
                    let _ = std::io::copy(&mut tee, &mut std::io::sink());

                    // A partially-read slice hashes to garbage. Reporting that as a
                    // mismatch is what evicted valid chunks, so the transport
                    // verdict is checked BEFORE either digest is trusted.
                    //
                    // The two cases are logged apart on purpose. A mid-stream
                    // error is a visible transport failure; a SHORT read with no
                    // error is a silent one (the body simply ends early and the
                    // reader reports clean EOF), and only the byte count catches
                    // it. Telling them apart in the log is what makes the next
                    // occurrence diagnosable.
                    if tee.io_error {
                        tracing::warn!(
                            pack = %pack_key,
                            entry = idx,
                            read = tee.bytes_seen,
                            expected = expected_len,
                            "pack verification: stream error mid-entry; entry unreadable"
                        );
                        return EntryVerification::Unreadable;
                    }
                    if tee.bytes_seen != expected_len {
                        tracing::warn!(
                            pack = %pack_key,
                            entry = idx,
                            read = tee.bytes_seen,
                            expected = expected_len,
                            "pack verification: range body ended early with no error \
                             (silent short read); entry unreadable"
                        );
                        return EntryVerification::Unreadable;
                    }

                    let raw_hex = tee.hasher.finalize().to_hex().to_string();
                    if raw_hex == chunk_oid {
                        return EntryVerification::Verified;
                    }
                    if decoded_ok && decompressed.0.finalize().to_hex().to_string() == chunk_oid {
                        return EntryVerification::Verified;
                    }
                    EntryVerification::Corrupt
                })
                .await
                // A JoinError means the check never produced a verdict — that is
                // an absence of evidence, not evidence of corruption.
                .unwrap_or(EntryVerification::Unreadable);
                let read_ms = (entry_start.elapsed().as_millis() as u64).saturating_sub(open_ms);
                (idx, outcome, open_ms, read_ms)
            }
        }))
        .buffer_unordered(range_concurrency)
        .collect::<Vec<(usize, EntryVerification, u64, u64)>>()
        .await;

    // One summary line per pack. Always emitted, not gated on a threshold: the
    // healthy numbers are what make an unhealthy one legible, and a pack that is
    // slow only relative to its peers cannot be spotted from a warning alone.
    if !results.is_empty() {
        let worst = results.iter().max_by_key(|(_, _, o, r)| o + r);
        let total_open: u64 = results.iter().map(|(_, _, o, _)| *o).sum();
        let total_read: u64 = results.iter().map(|(_, _, _, r)| *r).sum();
        if let Some((widx, _, wopen, wread)) = worst {
            tracing::info!(
                pack = %pack_key,
                entries = results.len(),
                concurrency = range_concurrency,
                sum_open_ms = total_open,
                sum_read_ms = total_read,
                worst_entry = widx,
                worst_open_ms = wopen,
                worst_read_ms = wread,
                "pack verification entry timings"
            );
        }
    }

    let mut corrupt: Vec<usize> = Vec::new();
    let mut unreadable: Vec<usize> = Vec::new();
    for (idx, outcome, _open_ms, _read_ms) in results {
        match outcome {
            EntryVerification::Verified => {}
            EntryVerification::Corrupt => corrupt.push(idx),
            EntryVerification::Unreadable => unreadable.push(idx),
        }
    }
    corrupt.sort_unstable();
    unreadable.sort_unstable();
    (corrupt, unreadable)
}

/// Returns the first (lowest-index) manifest entry whose bytes don't hash to
/// its claimed `chunk_oid`, if any. Test-only since the redesign: production
/// code now always wants every bad entry (see
/// [`all_pack_entries_failing_content_verification`]), but this single-answer
/// view is kept for the deterministic-ordering tests below — see
/// [`pack_entries_failing_content_verification`] for why it's deterministic
/// under concurrency.
#[cfg(test)]
async fn first_pack_entry_failing_content_verification<'a>(
    storage: &Arc<dyn StorageBackend>,
    compressor: &Arc<SmartCompressor>,
    pack_key: &str,
    manifest: &'a [ManifestEntry],
) -> Option<&'a ManifestEntry> {
    let (corrupt, _unreadable) =
        pack_entries_failing_content_verification(storage, compressor, pack_key, manifest).await;
    corrupt.first().and_then(|&i| manifest.get(i))
}

/// First 2 hex chars of a pack oid — the shard directory both its `.jsonl`
/// manifest and its `.pending` marker live under
/// (`<repo>/.mediagit/packs/<shard>/`). Pulled out so the marker path and the
/// manifest path can never compute the shard differently.
fn pack_shard(pack_oid: &str) -> &str {
    if pack_oid.len() >= 2 {
        &pack_oid[..2]
    } else {
        "00"
    }
}

/// Path of a pack's durable "unverified" marker — a sibling of its
/// `<pack_oid>.jsonl` manifest in the same shard directory.
///
/// Its PRESENCE, not any in-memory flag, is the source of truth for "this
/// pack has not passed content verification yet": `state.unverified_packs`
/// is a cache of it, kept in step under the same write lock as `pack_index`
/// (see `complete_pack`). `load_jsonl_index` (`handlers/mod.rs`) filters to
/// `extension() == "jsonl"`, so this sibling is silently invisible to the
/// server's own manifest reader — no format change, no compat break.
fn pending_marker_path(repo_path: &std::path::Path, pack_oid: &str) -> std::path::PathBuf {
    repo_path
        .join(".mediagit")
        .join("packs")
        .join(pack_shard(pack_oid))
        .join(format!("{pack_oid}.pending"))
}

/// Reads and parses a pack's `.jsonl` manifest from disk into the
/// `ManifestEntry` shape `verify_pack_in_background` needs. Shared by the
/// startup sweep (`resume_pack_verification`) and the presign-triggered
/// verifier (D3: `ensure_pack_verified_for_presign` in `handlers/transfer.rs`)
/// — both need to hand the same manifest to the same verifier.
///
/// Returns `None` if the manifest is missing, unreadable, or parses to zero
/// entries — callers must fail closed (leave/treat the pack as unverified)
/// in that case, not treat an empty manifest as "nothing to verify."
pub(crate) async fn read_pack_manifest(
    repo_path: &std::path::Path,
    pack_oid: &str,
) -> Option<Vec<ManifestEntry>> {
    let manifest_path = repo_path
        .join(".mediagit")
        .join("packs")
        .join(pack_shard(pack_oid))
        .join(format!("{pack_oid}.jsonl"));
    let content = tokio::fs::read_to_string(&manifest_path).await.ok()?;
    let manifest: Vec<ManifestEntry> = content
        .lines()
        .filter(|l| !l.is_empty())
        .filter_map(|line| serde_json::from_str::<PackIndexLine>(line).ok())
        .map(|e| ManifestEntry {
            chunk_oid: e.chunk_oid,
            offset: e.offset,
            length: e.length,
            compressed_hash: e.compressed_hash,
        })
        .collect();
    if manifest.is_empty() {
        None
    } else {
        Some(manifest)
    }
}

/// Global (not per-repo) semaphore bounding concurrent pack content-verification
/// read-backs. Serialised by default (`MEDIAGIT_PACK_VERIFY_CONCURRENCY=1`).
///
/// The client uploads packs concurrently, so without this N verifications each
/// pull a whole pack back over the SAME shared WAN link at once and thrash it.
/// Measured on a real 1 GB S3 push: per-pack throughput decayed monotonically
/// as requests piled up (0.057 -> 0.032 MiB/s across 8 in flight) for an
/// aggregate of 0.226 MiB/s, while a pack verified alone hit 1.458 MiB/s — 45x
/// better per pack. Global, not per-repo, because the constraint is the shared
/// link, not the repository.
/// Wall-clock ceiling for ONE attempt at verifying ONE pack.
///
/// Sized from measurement, not taste. Healthy per-pack verification against live
/// GCS on this class of link: mean 12.7s, worst 35.5s over a 512 MB payload; the
/// slowest pack in a healthy 2 GB run was 103s. The pathological case that
/// motivated this was 2628s. 300s therefore sits ~3x above the worst healthy
/// observation and ~9x below the failure, which is the widest gap available —
/// close enough to normal that a real stall is caught within minutes, far enough
/// that ordinary slowness never trips it.
///
/// Exceeding it is not an error and quarantines nothing: the pack is simply left
/// unverified for a later attempt, which is the same outcome the pre-existing
/// "unreadable after retries" path already produces.
fn pack_verify_budget() -> std::time::Duration {
    const DEFAULT_BUDGET_SECS: u64 = 300;
    let secs = std::env::var("MEDIAGIT_PACK_VERIFY_BUDGET_SECS")
        .ok()
        .and_then(|s| s.parse::<u64>().ok())
        .filter(|n| *n > 0)
        .unwrap_or(DEFAULT_BUDGET_SECS);
    std::time::Duration::from_secs(secs)
}

/// Knob for THIS (pack-level) axis is `MEDIAGIT_PACK_VERIFY_PACK_CONCURRENCY`.
///
/// It used to read `MEDIAGIT_PACK_VERIFY_CONCURRENCY` — the same name that
/// `pack_entries_failing_content_verification` reads for a completely different
/// axis (how many range-reads run inside ONE pack), with a different default
/// (16 there, 1 here). One name silently drove both, so an operator tuning
/// per-entry memory also changed pack-level serialisation, and vice versa.
///
/// Keeping the default at 1 is not conservatism, it is measured: raising this
/// to 16 on 2026-08-20 made per-pack verification **4.5x worse** (mean 12.7s ->
/// 56.6s over a 512 MB GCS payload), reproducing the link-thrashing described
/// above. More permits is the intuitive fix for a clone blocked on verification
/// and it is the wrong one.
fn now_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Record that a transfer just used the link. Called from the data-plane
/// handlers only — control-plane chatter (refs, manifests) is cheap and must
/// not hold verification off.
pub(crate) fn note_data_plane_activity(state: &AppState) {
    note_data_plane_activity_for(state, 0);
}

/// Declare a transfer of `packs` packs as starting now.
///
/// WHY A LEASE AND NOT JUST A TIMESTAMP. Stamping "activity happened" works only
/// while the server is IN the data path. On the presigned fast path it is
/// deliberately not: the client mints URLs and then reads straight from the
/// bucket, so the server goes silent for the entire transfer.
///
/// Measured 2026-09-15 on the 16 GB corpus, aws arm: 151
/// `presign-download-urls` requests arrived in FOUR distinct seconds
/// (13:56:46, 13:56:49, 13:59:22, then nothing until 14:21:03). The server saw
/// no data-plane traffic for ~21 of the clone's 23 minutes. A 5s quiet window
/// therefore expired almost immediately and verification ran through the whole
/// clone anyway — 34 packs / 2.22 GiB / 1.63 MB/s of stolen downlink, WORSE
/// than the 22 packs measured before the scheduler existed. The first version
/// of this fix did not work, and only a measurement showed it.
///
/// So a presign request is treated as what it actually is: a DECLARATION that
/// the client is about to pull those packs. The lease is derived from the bytes
/// implied by the pack count (each pack is capped at `pack_bytes_cap`) against
/// a deliberately pessimistic floor rate, and is clamped by the same
/// max-defer cap that bounds everything else here — so the worst case is
/// unchanged and verification still cannot be postponed indefinitely.
///
/// HONEST LIMIT: this is a bound, not knowledge. The server never learns when a
/// pull actually finishes, so the lease can expire mid-clone (verification then
/// resumes and competes) or outlast a cancelled one (verification starts later
/// than it could). Knowing would need either a client-side heartbeat or a
/// "pull complete" signal — a protocol change — or removing the read-back
/// entirely via upload-time provider attestation, which is the real fix.
pub(crate) fn note_data_plane_activity_for(state: &AppState, packs: usize) {
    let bytes = (packs as u64).saturating_mul(mediagit_versioning::pack_bytes_cap());
    note_data_plane_transfer_bytes(state, bytes)
}

/// Declare a transfer of a known byte count as starting now.
///
/// The counterpart to [`note_data_plane_activity_for`] for callers that know
/// the exact size rather than a pack count — `mpu_start_for` is handed
/// `chunk_size` outright, so it need not round up to `pack_bytes_cap`.
///
/// WHY UPLOADS NEED THIS AT ALL. Every caller of the lease used to be a READ
/// path (`presign_pack_downloads`, `download_chunk`, `batch_get_pack_chunks`).
/// A push therefore stamped nothing: `data_plane_activity` stayed at its
/// initial `0`, the wait loop's `last == 0` arm fired, and verification ran
/// through the whole push completely unscheduled — measured 2026-09-16 on the
/// 16 GB aws arm, 64 packs verified DURING the push at concurrency 16, against
/// the same bucket over the same link the push was using. The scheduler was
/// inert on exactly the leg that moves the most bytes.
pub(crate) fn note_data_plane_transfer_bytes(state: &AppState, bytes: u64) {
    note_transfer_on(&state.data_plane_activity, bytes)
}

/// The lease arithmetic, against a bare clock.
///
/// Split from the `AppState` wrapper for the same reason
/// `wait_for_data_plane_quiet_with` is: reaching into the process-wide state
/// from a test couples every test in the binary, and four unrelated
/// `complete_pack` tests already failed once that way.
fn note_transfer_on(activity: &std::sync::atomic::AtomicU64, bytes: u64) {
    let now = now_millis();
    let lease_ms = lease_ms_for_bytes(bytes);
    if lease_ms == 0 {
        // A bare "something happened" stamp — no declared size, so it expresses
        // an instant, not an occupancy.
        activity.fetch_max(now, std::sync::atomic::Ordering::Relaxed);
        return;
    }

    // ACCUMULATE, do not max. Transfers queue on one link, so two concurrent
    // 64 MiB uploads occupy it for twice as long as one — but `fetch_max` of
    // `now + lease` collapses them to a single lease, because both compute the
    // same instant. Measured 2026-09-16 on the 16 GB aws push: 67 MB packs
    // leased ~33 s each while real gaps between MPU starts reached 76 s, so the
    // link read as idle mid-push and verification ran straight through it.
    //
    // Extending from the existing deadline models the queue: each declaration
    // adds its own drain time to whatever is already outstanding. Clamped to
    // `now + max_defer` so an accumulating lease can never postpone
    // verification longer than the cap already allows.
    let ceiling = now.saturating_add(verify_max_defer_secs().saturating_mul(1000));
    let mut cur = activity.load(std::sync::atomic::Ordering::Relaxed);
    loop {
        let base = cur.max(now);
        let next = base.saturating_add(lease_ms).min(ceiling);
        if next <= cur {
            // Already at or beyond the ceiling — nothing to extend.
            return;
        }
        match activity.compare_exchange_weak(
            cur,
            next,
            std::sync::atomic::Ordering::Relaxed,
            std::sync::atomic::Ordering::Relaxed,
        ) {
            Ok(_) => return,
            Err(observed) => cur = observed,
        }
    }
}

/// Record that an upload the server orchestrates has started.
///
/// Pairs with [`note_upload_finished`]. See `AppState::uploads_in_flight` for
/// why uploads get an exact in-flight count while downloads get an estimate.
pub(crate) fn note_upload_started(state: &AppState) {
    state
        .uploads_in_flight
        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
}

/// Record that an orchestrated upload finished (completed OR aborted).
///
/// Saturates at zero: an abort following a completion, or a duplicate retry of
/// either, must not drive the count negative and make a busy link read idle.
pub(crate) fn note_upload_finished(state: &AppState) {
    let _ = state.uploads_in_flight.fetch_update(
        std::sync::atomic::Ordering::Relaxed,
        std::sync::atomic::Ordering::Relaxed,
        |n| Some((n - 1).max(0)),
    );
}

/// Is the link carrying a transfer right now?
///
/// Two independent signals, because they answer the question with different
/// certainty. `uploads_in_flight` is KNOWN — the server is told when an upload
/// starts and when it ends. The lease is INFERRED, and is all that downloads
/// can offer, since a presigned GET never reports back.
/// Returns the REASON the link is busy, or `None` when it is quiet.
///
/// A reason rather than a bool so the decision is evident in the log: "this
/// verification deferred" is far less useful than which of the two signals held
/// it off, and the estimated one is the one that has been wrong before.
fn link_busy_reason(
    activity: &std::sync::atomic::AtomicU64,
    inflight: &std::sync::atomic::AtomicI64,
    quiet_secs: u64,
) -> Option<&'static str> {
    if inflight.load(std::sync::atomic::Ordering::Relaxed) > 0 {
        return Some("upload-in-flight");
    }
    let last = activity.load(std::sync::atomic::Ordering::Relaxed);
    if last == 0 {
        return None;
    }
    if now_millis().saturating_sub(last) < quiet_secs.saturating_mul(1000) {
        return Some("transfer-lease");
    }
    None
}

/// How long a transfer of `bytes` is assumed to occupy the link, in ms.
///
/// Split out so the arithmetic is testable without an `AppState`. Zero bytes
/// means "no declared transfer" and yields no lease, which is what keeps a
/// bare `note_data_plane_activity()` stamp behaving as a plain timestamp.
fn lease_ms_for_bytes(bytes: u64) -> u64 {
    if bytes == 0 {
        return 0;
    }
    let floor_mbps = std::env::var("MEDIAGIT_PACK_VERIFY_ASSUMED_MBPS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .filter(|n| *n > 0)
        .unwrap_or(2);
    let secs = (bytes / (1024 * 1024)) / floor_mbps;
    secs.min(verify_max_defer_secs()).saturating_mul(1000)
}

/// Seconds the data plane must be idle before a pack is verified.
fn verify_quiet_secs() -> u64 {
    std::env::var("MEDIAGIT_PACK_VERIFY_QUIET_SECS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(5)
}

/// Upper bound on deferral, so continuous traffic cannot postpone verification
/// forever. 0 disables waiting entirely (verify immediately, pre-2026-09-15
/// behaviour).
fn verify_max_defer_secs() -> u64 {
    std::env::var("MEDIAGIT_PACK_VERIFY_MAX_DEFER_SECS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(900)
}

/// Emit the effective verification-scheduling policy once per process.
///
/// WHY AT `info`. Both outcomes of this scheduler are otherwise invisible at
/// the default log level: the cap-hit and the waited-and-proceeded lines are
/// `debug!`, and when the policy is working correctly NEITHER fires — so a run
/// that deferred perfectly and a run where the knob never reached the server
/// produce byte-identical logs. Measuring a change to
/// `MEDIAGIT_PACK_VERIFY_MAX_DEFER_SECS` against that is measuring nothing.
/// This line makes the value the server actually used a fact in the log.
///
/// Deliberately NOT cached beyond the log: the callers above re-read the env
/// every time, so the knob stays live even though this line prints once.
fn log_verify_policy_once(quiet: u64, max_defer: u64) {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        tracing::info!(
            quiet_secs = quiet,
            max_defer_secs = max_defer,
            "pack verification scheduling policy in effect \
             (MEDIAGIT_PACK_VERIFY_QUIET_SECS / MEDIAGIT_PACK_VERIFY_MAX_DEFER_SECS)"
        );
    });
}

/// The scheduling mechanism, with its policy passed in.
///
/// Split from the env-reading wrapper so the behaviour can be tested at real
/// timescales without mutating process-global state. Both the policy AND the
/// clock are injected, and the clock matters as much as the policy: an earlier
/// version of these tests stamped the real global, and four unrelated
/// `complete_pack` verification tests running in parallel then saw a busy link
/// and deferred past their own poll bounds. A test that reaches into a global
/// does not just risk being flaky itself — it makes every concurrent test flaky.
async fn wait_for_data_plane_quiet_with(
    repo: &str,
    pack_oid: &str,
    quiet: u64,
    max_defer: u64,
    activity: &std::sync::atomic::AtomicU64,
    inflight: &std::sync::atomic::AtomicI64,
) {
    if quiet == 0 || max_defer == 0 {
        return;
    }
    let started = std::time::Instant::now();
    let mut waited_any = false;
    let mut last_reason = "";
    while let Some(reason) = link_busy_reason(activity, inflight, quiet) {
        last_reason = reason;
        if started.elapsed().as_secs() >= max_defer {
            tracing::info!(
                repo,
                pack_oid,
                deferred_s = started.elapsed().as_secs(),
                reason,
                "pack verification deferred to its cap while transfers stayed active; \
                 proceeding so a busy server cannot postpone it indefinitely"
            );
            return;
        }
        waited_any = true;
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    }
    // At `info`, not `debug`. These two lines and the cap line above are the
    // ONLY evidence that this scheduler did anything, and the harness runs at
    // `info`: with them at `debug` a run that deferred perfectly and a run
    // where the mechanism was inert produced identical logs, which is how the
    // push path stayed unscheduled through two 16 GB measurements.
    if waited_any {
        tracing::info!(
            repo,
            pack_oid,
            deferred_s = started.elapsed().as_secs(),
            reason = last_reason,
            "pack verification waited for the data plane to go quiet"
        );
    }
}

/// The verification scheduler: it must yield the link to transfers, AND it must
/// never be postponed indefinitely by them.
///
/// Measured 2026-09-15 on the 16 GB corpus, counting only what verification
/// moved during each clone's OWN window:
///
///   backend | clone     | verify during clone | combined
///   aws     | 6.70 MB/s | 0.97 MB/s           | 7.67 MB/s
///   gcs     | 4.60 MB/s | 4.57 MB/s           | 9.17 MB/s
///   azure   | 7.81 MB/s | 1.55 MB/s           | 9.36 MB/s
///
/// The combined column is near-constant — that is the link ceiling — and gcs
/// clones at 47% of its own push only because its verifier consumed half the
/// downlink re-reading 10.21 GiB while the clone ran.
#[cfg(test)]
mod data_plane_quiet_tests {
    use super::*;

    /// Each test gets its OWN clock, so none of them touches the process-global
    /// one. That is not tidiness: stamping the real global made four unrelated
    /// `complete_pack` verification tests fail, because they run in parallel in
    /// the same binary and saw a link that looked busy.
    fn busy_now() -> std::sync::atomic::AtomicU64 {
        std::sync::atomic::AtomicU64::new(now_millis())
    }

    /// No upload declared — the counter half of the busy check is inert, so a
    /// test exercises the lease half alone.
    fn idle_uploads() -> std::sync::atomic::AtomicI64 {
        std::sync::atomic::AtomicI64::new(0)
    }

    /// Clearing the quiet gate must not licence work that STARTS much later.
    ///
    /// Reproduces the measured failure: the pack passes the wait while the link
    /// is quiet, queues behind the one permit, and the link goes busy while it
    /// waits. Without the post-permit re-check it would verify straight into a
    /// live transfer, which is what produced verifications running end-to-start
    /// through the whole 16 GB push.
    #[tokio::test]
    async fn a_pack_that_queued_while_quiet_rechecks_before_working() {
        let activity = std::sync::atomic::AtomicU64::new(0);
        let inflight = std::sync::atomic::AtomicI64::new(0); // quiet at first
        let sem = tokio::sync::Semaphore::new(1);

        // Someone else holds the only permit, so our pack must queue.
        let held = sem.acquire().await.unwrap();

        let started = std::time::Instant::now();
        let gate =
            acquire_verify_permit_when_quiet("repo", "pack", 1, 30, &activity, &inflight, &sem);
        tokio::pin!(gate);

        // It clears the quiet gate, then blocks on the permit.
        tokio::select! {
            _ = &mut gate => panic!("took a permit that was never available"),
            _ = tokio::time::sleep(std::time::Duration::from_millis(300)) => {}
        }

        // Link goes busy WHILE our pack is queued, then the permit frees up.
        inflight.store(1, std::sync::atomic::Ordering::Relaxed);
        drop(held);

        // It must NOT proceed now — the link is busy again.
        tokio::select! {
            _ = &mut gate => panic!(
                "verification started into a busy link: the permit was taken on a                  stale quiet check, which is the bug this re-check exists to close"
            ),
            _ = tokio::time::sleep(std::time::Duration::from_millis(700)) => {}
        }

        // Once the link really is idle it proceeds.
        inflight.store(0, std::sync::atomic::Ordering::Relaxed);
        let permit = tokio::time::timeout(std::time::Duration::from_secs(10), gate)
            .await
            .expect("never proceeded even after the link went idle");
        assert!(permit.is_some());
        assert!(
            started.elapsed() >= std::time::Duration::from_millis(900),
            "returned too early to have re-waited"
        );
    }

    /// An in-flight upload holds the link busy on its own, with no lease.
    ///
    /// This is the signal the push path was missing entirely: every lease
    /// caller was a read path, so a push left `data_plane_activity` at 0 and
    /// the wait returned instantly.
    #[tokio::test]
    async fn an_in_flight_upload_alone_defers_verification() {
        let idle_clock = std::sync::atomic::AtomicU64::new(0); // no lease at all
        let uploading = std::sync::atomic::AtomicI64::new(1);

        let started = std::time::Instant::now();
        wait_for_data_plane_quiet_with("repo", "pack", 30, 1, &idle_clock, &uploading).await;
        let waited = started.elapsed();

        assert!(
            waited >= std::time::Duration::from_millis(400),
            "an upload in flight did not defer verification ({waited:?}); the push              leg would verify straight through itself, as it did on 2026-09-16"
        );
        assert!(
            waited < std::time::Duration::from_secs(20),
            "waited {waited:?} past the 1s cap — a leaked in-flight count would              postpone verification forever instead of costing one capped wait"
        );
    }

    /// Finishing the upload releases the link.
    #[tokio::test]
    async fn a_finished_upload_stops_deferring() {
        let idle_clock = std::sync::atomic::AtomicU64::new(0);
        let uploads = std::sync::atomic::AtomicI64::new(1);
        uploads.fetch_sub(1, std::sync::atomic::Ordering::Relaxed);

        let started = std::time::Instant::now();
        wait_for_data_plane_quiet_with("repo", "pack", 30, 900, &idle_clock, &uploads).await;

        assert!(
            started.elapsed() < std::time::Duration::from_millis(400),
            "verification still deferred after the upload finished — the count              is not being released, so the link never reads idle again"
        );
    }

    /// The release saturates at zero.
    ///
    /// An abort after a completion, or a retried complete, must not drive the
    /// count negative — a negative count would make a genuinely busy link read
    /// idle and silently disable the scheduler for the rest of the process.
    #[test]
    fn releasing_more_than_started_cannot_go_negative() {
        let uploads = std::sync::atomic::AtomicI64::new(0);
        for _ in 0..3 {
            let _ = uploads.fetch_update(
                std::sync::atomic::Ordering::Relaxed,
                std::sync::atomic::Ordering::Relaxed,
                |n| Some((n - 1).max(0)),
            );
        }
        assert_eq!(uploads.load(std::sync::atomic::Ordering::Relaxed), 0);
    }

    /// A push declares its size, so an upload leaves a lease behind.
    ///
    /// REGRESSION GUARD. Every lease caller used to be a read path, so a push
    /// stamped nothing at all: `data_plane_activity` stayed `0`, the wait
    /// loop's `last == 0` arm fired, and verification ran through the entire
    /// push unscheduled — 64 packs verified DURING the 16 GB aws push on
    /// 2026-09-16, at concurrency 16, over the link the push was using.
    #[test]
    fn a_declared_upload_leaves_a_lease() {
        let one_pack = 64 * 1024 * 1024;
        let lease = lease_ms_for_bytes(one_pack);
        assert!(
            lease > 0,
            "a 64 MiB upload produced no lease, so verification will treat the              link as idle while the client is still uploading to the bucket"
        );
        // 64 MiB at the 2 MB/s pessimistic floor is ~32 s. Assert the shape,
        // not the constant, so retuning the floor does not fail this.
        assert!(
            lease >= 10_000,
            "lease {lease}ms is shorter than a realistic 64 MiB upload; the 5s              quiet window would expire mid-transfer"
        );
    }

    /// The bare stamp must stay a plain timestamp, or control-plane chatter
    /// would start holding verification off.
    #[test]
    fn an_undeclared_stamp_leases_nothing() {
        assert_eq!(lease_ms_for_bytes(0), 0);
    }

    /// Concurrent declarations must ADD, not collapse.
    ///
    /// Transfers queue on one link, so two concurrent 64 MiB uploads occupy it
    /// for twice as long as one. `fetch_max(now + lease)` collapsed them to a
    /// single lease because both compute the same instant — measured on the
    /// 16 GB aws push as 67 MB packs leasing ~33 s against real gaps of up to
    /// 76 s, i.e. the link reading idle while it was saturated.
    #[test]
    fn concurrent_declarations_extend_the_same_lease() {
        let activity = std::sync::atomic::AtomicU64::new(0);
        let horizon = || activity.load(std::sync::atomic::Ordering::Relaxed);

        note_transfer_on(&activity, 64 * 1024 * 1024);
        let one = horizon();
        note_transfer_on(&activity, 64 * 1024 * 1024);
        let two = horizon();

        assert!(
            two >= one + 10_000,
            "second concurrent declaration extended the lease by only {}ms;              N transfers must reserve N transfers' worth of link, or the              scheduler reads a saturated link as idle",
            two.saturating_sub(one)
        );
    }

    /// Accumulation cannot outrun the cap.
    ///
    /// Without a ceiling, a busy push would push the horizon arbitrarily far
    /// out and verification would never run — trading one bug for a worse one.
    #[test]
    fn accumulation_is_clamped_to_the_defer_cap() {
        let activity = std::sync::atomic::AtomicU64::new(0);
        for _ in 0..500 {
            note_transfer_on(&activity, 64 * 1024 * 1024);
        }

        let cap_ms = verify_max_defer_secs() * 1000;
        let ahead = activity
            .load(std::sync::atomic::Ordering::Relaxed)
            .saturating_sub(now_millis());
        assert!(
            ahead <= cap_ms + 2_000,
            "lease ran {ahead}ms ahead, past the {cap_ms}ms cap: continuous              traffic could postpone verification indefinitely"
        );
    }

    /// Both halves, because either alone is satisfiable by a broken
    /// implementation: "returns immediately" passes a scheduler that never
    /// defers, and "waits" passes one that deadlocks a busy server forever.
    #[tokio::test]
    async fn verification_waits_for_a_busy_link_then_proceeds() {
        // Quiet period long enough that an immediate return is unambiguous;
        // defer cap short enough to keep the test fast.
        let clock = busy_now(); // link busy right now

        let started = std::time::Instant::now();
        wait_for_data_plane_quiet_with("repo", "pack", 30, 1, &clock, &idle_uploads()).await;
        let waited = started.elapsed();

        // HALF 1 — it actually deferred. Without this, a no-op scheduler passes.
        assert!(
            waited >= std::time::Duration::from_millis(400),
            "verification did not wait at all ({waited:?}) despite the data plane \
             being active — it would keep stealing downlink from the clone, which \
             measured 4.57 of 9.17 MB/s on gcs"
        );
        // HALF 2 — it gave up at the cap. Without this, a busy server defers
        // verification forever and packs are never quarantined.
        assert!(
            waited < std::time::Duration::from_secs(20),
            "verification waited {waited:?}, past its 1s cap: continuous traffic can \
             postpone it indefinitely, so corrupt packs would never be found"
        );
    }

    /// A LEASE — an activity timestamp in the FUTURE — must keep the waiter
    /// waiting until it passes.
    ///
    /// This is the mechanism the presign path depends on. Stamping "activity
    /// happened" is useless there: measured on the aws 16 GB arm, 151
    /// presign-download requests arrived within four seconds and then the
    /// server saw nothing for ~21 of the clone's 23 minutes, because presigned
    /// URLs take the server out of the data path by design. Verification ran
    /// straight through the clone and stole MORE downlink than before the
    /// scheduler existed (34 packs vs 22). Expressing "busy until T" is what
    /// makes the declaration usable.
    #[tokio::test]
    async fn a_future_lease_keeps_the_link_busy_until_it_expires() {
        // Busy for ~600ms from now.
        let clock = std::sync::atomic::AtomicU64::new(now_millis() + 600);

        let started = std::time::Instant::now();
        // quiet=0 would disable; use 1s quiet and a cap well past the lease so
        // the LEASE is what releases it, not the cap.
        wait_for_data_plane_quiet_with("repo", "pack", 1, 900, &clock, &idle_uploads()).await;
        let waited = started.elapsed();

        assert!(
            waited >= std::time::Duration::from_millis(400),
            "a future lease did not hold verification off ({waited:?}); on the              presigned fast path this is the ONLY signal that a transfer is running,              so without it verification competes for the whole clone"
        );
        assert!(
            waited < std::time::Duration::from_secs(10),
            "waited {waited:?} — the lease never expired, so verification would be              postponed past the transfer it was yielding to"
        );
    }

    /// An idle link must not be waited on at all — the steady state, and the
    /// one that would quietly add latency to every verification if wrong.
    #[tokio::test]
    async fn an_idle_link_is_not_waited_on() {
        // Last activity far enough in the past to count as quiet.
        let clock = std::sync::atomic::AtomicU64::new(now_millis().saturating_sub(60_000));

        let started = std::time::Instant::now();
        wait_for_data_plane_quiet_with("repo", "pack", 1, 900, &clock, &idle_uploads()).await;
        let waited = started.elapsed();

        assert!(
            waited < std::time::Duration::from_millis(200),
            "waited {waited:?} on an idle link — verification should start immediately \
             when nothing is using the link"
        );
    }

    /// The escape hatch has to work, or an operator who hits an unforeseen
    /// interaction has no way back to the previous behaviour.
    #[tokio::test]
    async fn the_scheduler_can_be_disabled() {
        let clock = busy_now();

        let started = std::time::Instant::now();
        wait_for_data_plane_quiet_with("repo", "pack", 0, 900, &clock, &idle_uploads()).await;
        let waited = started.elapsed();

        assert!(
            waited < std::time::Duration::from_millis(200),
            "MEDIAGIT_PACK_VERIFY_QUIET_SECS=0 did not restore immediate verification \
             ({waited:?}); the documented escape hatch is dead"
        );
    }
}

fn pack_verify_semaphore() -> &'static tokio::sync::Semaphore {
    static VERIFY_SEM: std::sync::OnceLock<tokio::sync::Semaphore> = std::sync::OnceLock::new();
    VERIFY_SEM.get_or_init(|| {
        // Deliberately NOT falling back to the old name: it would keep honouring
        // a setting that measurement says makes this worse, and it is what made
        // the two axes indistinguishable in the first place. An operator who set
        // the old name gets told, once, rather than silently losing the pack-level
        // meaning it used to have.
        if std::env::var_os("MEDIAGIT_PACK_VERIFY_CONCURRENCY").is_some() {
            tracing::warn!(
                "MEDIAGIT_PACK_VERIFY_CONCURRENCY now controls only the per-entry range \
                 concurrency INSIDE one pack. To change how many packs verify at once, set \
                 MEDIAGIT_PACK_VERIFY_PACK_CONCURRENCY (default 1; raising it measured 4.5x \
                 slower per pack)"
            );
        }
        let permits = std::env::var("MEDIAGIT_PACK_VERIFY_PACK_CONCURRENCY")
            .ok()
            .and_then(|s| s.parse::<usize>().ok())
            .filter(|n| *n > 0)
            .unwrap_or(1);
        tracing::info!(
            permits,
            "Pack content verification concurrency (MEDIAGIT_PACK_VERIFY_PACK_CONCURRENCY)"
        );
        tokio::sync::Semaphore::new(permits)
    })
}

/// Get-or-create the shared in-flight cell for (repo, pack_oid) so a push's
/// own background verification and a racing clone's presign-triggered
/// verification (`transfer.rs::ensure_pack_verified_for_presign`) never both
/// run a full read-and-hash pass of the same pack. Whoever gets here first
/// does the real work via `cell.get_or_init`; everyone else awaits that same
/// cell instead of starting a second one.
///
/// Without this, a clone that lands within seconds of the push it's reading
/// back (the common "push then pull to confirm" shape, and exactly what the
/// QA scale drill does) paid for TWO full WAN read-and-hash passes of the
/// pack back to back — measured turning a ~5 min GCS clone into 15-50+ min.
pub(crate) async fn get_or_create_pack_verify_cell(
    state: &Arc<AppState>,
    key: &(String, String),
) -> Arc<tokio::sync::OnceCell<bool>> {
    let mut inflight = state.pack_verify_inflight.lock().await;
    Arc::clone(
        inflight
            .entry(key.clone())
            .or_insert_with(|| Arc::new(tokio::sync::OnceCell::new())),
    )
}

/// Background pack verification (PAC: async writes, sync reads). Runs off the
/// push critical path — see `complete_pack`, which enqueues this instead of
/// verifying inline.
///
/// Verifies EVERY manifest entry (not just the first bad one — a pack with
/// two poisoned chunks must not stay half-quarantined with an unknown-status
/// sibling), then always clears the `.pending` marker and the
/// `unverified_packs` entry, whether the pack came out clean or had entries
/// evicted. Verification is DONE either way once this returns; a partially
/// quarantined pack's surviving entries are legitimately verified-clean.
///
/// Durability does NOT come from this task running to completion — it comes
/// from the `.pending` marker plus the startup sweep (`resume_pack_verification`,
/// invoked from `main.rs`). A dropped task (crash, panic) simply leaves the
/// marker on disk; the sweep finds it on the next boot and re-enqueues. That's
/// why a bare `tokio::spawn` at the call site is safe here even though it
/// silently swallows a task that never runs — the marker is the actual source
/// of truth, and this task is just how the common case resolves it quickly.
///
/// Returns `true` when the pack came out clean (every entry verified), `false`
/// when one or more entries were quarantined. `pub(crate)` (not just called
/// via `tokio::spawn` here) so D3's `ensure_pack_verified_for_presign`
/// (`handlers/transfer.rs`) can await it directly — same verify-and-resolve
/// logic, not a second implementation.
/// Wait for a quiet link, take the verify permit, then RE-CHECK before working.
///
/// TWO GATES, because one is not enough. The wait is a snapshot: a pack that
/// finds the link quiet and then queues behind the single permit can sit there
/// for minutes and begin work into a link that went busy again. Measured
/// 2026-09-16 on the 16 GB aws push — verifications ran seamlessly end-to-start
/// (…10:02→10:27, 10:26→10:49, 10:48→11:09) straight through a push, because
/// they had all cleared the gate during one quiet moment.
///
/// Re-checking BEFORE the permit and waiting AFTER it would serialise the
/// waits — each pack burning its own full cap in turn, `max_defer` x N rather
/// than `max_defer` total — which is the trap the original ordering was built
/// to avoid. Release and re-wait instead, bounded by the same overall cap.
///
/// Split out of [`verify_pack_in_background`] so the ordering is testable
/// without a live `AppState`, a storage backend and a real pack — which is
/// exactly why the one-shot version went unnoticed through two 16 GB runs.
#[allow(clippy::too_many_arguments)]
async fn acquire_verify_permit_when_quiet<'a>(
    repo: &str,
    pack_oid: &str,
    quiet: u64,
    max_defer: u64,
    activity: &std::sync::atomic::AtomicU64,
    inflight: &std::sync::atomic::AtomicI64,
    sem: &'a tokio::sync::Semaphore,
) -> Option<tokio::sync::SemaphorePermit<'a>> {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(max_defer);
    loop {
        wait_for_data_plane_quiet_with(repo, pack_oid, quiet, max_defer, activity, inflight).await;
        let permit = sem.acquire().await.ok();
        if link_busy_reason(activity, inflight, quiet).is_none()
            || std::time::Instant::now() >= deadline
        {
            return permit;
        }
        tracing::info!(
            repo,
            pack_oid,
            "link went busy again while this pack queued for its verify permit; \
             yielding the permit and waiting again"
        );
        drop(permit);
    }
}

pub(crate) async fn verify_pack_in_background(
    state: Arc<AppState>,
    repo_path: std::path::PathBuf,
    repo: String,
    pack_oid: String,
    storage: Arc<dyn StorageBackend>,
    manifest: Vec<ManifestEntry>,
) -> bool {
    // Lane admission lives HERE, not inside the budgeted work. Queueing for the
    // permit is unbounded (it is exactly what the budget exists to bound for
    // *other* packs), so counting it against a budget that has not started yet
    // would conflate "this pack is slow" with "this pack has not begun". Held
    // across the whole call, so the lane is given up on return — including the
    // budget-exceeded return — which is the behaviour the timeout below relies on.
    // BEFORE the permit, not after: there is exactly one permit, so a pack
    // waiting for quiet while holding it would stall every other pack's
    // verification behind a queue that cannot drain.
    // TWO GATES, because one is not enough. The wait below is a snapshot: a
    // pack that finds the link quiet then queues behind the single permit can
    // sit there for minutes and start work into a link that went busy again.
    // Measured 2026-09-16 on the 16 GB aws push — verifications ran seamlessly
    // end-to-start (…10:02→10:27, 10:26→10:49, 10:48→11:09) straight through
    // a push, because they had all cleared the gate during one quiet moment.
    //
    // So: wait, take the permit, then re-check. Re-checking BEFORE the permit
    // and waiting AFTER it would serialise the waits — each pack burning its
    // own full cap in turn, `max_defer` x N rather than `max_defer` total —
    // which is the trap the ordering here was built to avoid in the first
    // place. Release and re-wait instead, bounded by the same overall cap.
    log_verify_policy_once(verify_quiet_secs(), verify_max_defer_secs());
    let _permit = acquire_verify_permit_when_quiet(
        &repo,
        &pack_oid,
        verify_quiet_secs(),
        verify_max_defer_secs(),
        &state.data_plane_activity,
        &state.uploads_in_flight,
        pack_verify_semaphore(),
    )
    .await;
    verify_pack_with_budget(
        state,
        repo_path,
        repo,
        pack_oid,
        storage,
        manifest,
        pack_verify_budget(),
    )
    .await
}

/// Budget taken as an argument rather than read from the environment, so the
/// timeout is testable. `#![forbid(unsafe_code)]` plus edition 2024 make
/// `set_var` an `unsafe` call, so a test cannot drive the real variable — the
/// same constraint that shaped `gcs::parse_io_timeout_secs`. A guard nobody has
/// watched fire is the shape this codebase has been bitten by repeatedly, so the
/// seam exists to let a test watch it fire.
///
/// Assumes the caller already holds the `pack_verify_semaphore` permit
/// (`verify_pack_in_background` does). Acquiring it in here instead put an
/// unbounded queue wait inside the region a caller times, which is what made
/// `trickling_pack_gives_up_its_verify_permit` read 6.18s against a 150ms
/// budget: it was queued behind another test's 2s+4s retry ladder, not running.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn verify_pack_with_budget(
    state: Arc<AppState>,
    repo_path: std::path::PathBuf,
    repo: String,
    pack_oid: String,
    storage: Arc<dyn StorageBackend>,
    manifest: Vec<ManifestEntry>,
    budget: std::time::Duration,
) -> bool {
    let compressor = match crate::handlers::repo_compressor(&state, &repo_path) {
        Ok(c) => Arc::new(c),
        Err(_) => {
            tracing::error!(
                repo = %repo,
                pack = %pack_oid,
                "Cannot verify pack: this repository's at-rest key is unreadable"
            );
            return false;
        }
    };

    let pack_key = format!("packs/{pack_oid}");
    let verify_start = std::time::Instant::now();

    // Unreadable entries are retried IN PROCESS before the pack is parked.
    // Without this, one blip left the pack pending until a restart or a pull
    // happened to trigger the presign verifier — measured on a 1 GB S3 push
    // (2026-08-03): 2 of 16 packs sat unresolved indefinitely. Only the
    // still-unresolved entries are re-read, so a retry costs a few ranges, not
    // another whole-pack pass.
    const VERIFY_ATTEMPTS: usize = 3;
    let mut bad: Vec<ManifestEntry> = Vec::new();
    let mut outstanding: Vec<ManifestEntry> = manifest.clone();
    for attempt in 1..=VERIFY_ATTEMPTS {
        // Bounded by WALL TIME, not by stalling. The storage layer already has a
        // no-progress deadline (e.g. MEDIAGIT_GCS_IO_TIMEOUT_SECS, 120s), and on
        // 2026-08-20 it did not fire once while a pack took 2628s for 21 entries
        // — because the read never stopped, it trickled at ~8 KB/s. A deadline
        // that only catches a dead connection cannot catch a live-but-useless one.
        //
        // Why this matters far beyond one pack: verification holds the single
        // global permit (`pack_verify_semaphore`), so that one pack blocked every
        // other pack's verification for 44 minutes, and `presign_pack_downloads`
        // blocks a clone on exactly that. The clone took 3055s; 2661s of it was
        // one presign call waiting behind this.
        //
        // The fix is to give up the LANE, not to widen it. Widening was measured
        // on 2026-08-20 and is catastrophic: 16 permits made a 512 MB GCS clone
        // 131s -> 2042s (15.5x worse), because concurrent whole-pack read-backs
        // thrash the shared link.
        let attempt_result = tokio::time::timeout(
            budget,
            pack_entries_failing_content_verification(
                &storage,
                &compressor,
                &pack_key,
                &outstanding,
            ),
        )
        .await;
        let (corrupt, unreadable_idx) = match attempt_result {
            Ok(v) => v,
            Err(_) => {
                // Park it exactly like the unreadable-after-retries case below:
                // still in `unverified_packs`, nothing quarantined, a later sweep
                // or presign retries it. Correctness is untouched — an unverified
                // pack still cannot have a URL minted for it, so the client takes
                // the fallback path rather than receiving unvetted bytes.
                tracing::warn!(
                    repo = %repo,
                    pack = %pack_oid,
                    attempt,
                    entries_outstanding = outstanding.len(),
                    budget_secs = budget.as_secs(),
                    elapsed_ms = verify_start.elapsed().as_millis() as u64,
                    "pack verification exceeded its wall-clock budget; releasing the verify \
                     permit so other packs are not blocked behind it. Pack stays unverified \
                     (nothing quarantined). Raise MEDIAGIT_PACK_VERIFY_BUDGET_SECS if this \
                     link is legitimately this slow"
                );
                return false;
            }
        };
        bad.extend(corrupt.iter().filter_map(|&i| outstanding.get(i).cloned()));
        if unreadable_idx.is_empty() {
            outstanding.clear();
            break;
        }
        outstanding = unreadable_idx
            .iter()
            .filter_map(|&i| outstanding.get(i).cloned())
            .collect();
        if attempt < VERIFY_ATTEMPTS {
            tracing::warn!(
                repo = %repo,
                pack = %pack_oid,
                attempt,
                unreadable = outstanding.len(),
                "background pack verification: retrying unreadable entries"
            );
            tokio::time::sleep(std::time::Duration::from_secs(2u64.pow(attempt as u32))).await;
        }
    }
    let unreadable = outstanding.len();
    let clean = bad.is_empty() && unreadable == 0;

    // Entries we still could not read leave the pack's status UNKNOWN.
    // Resolving it either way would be wrong: marking it verified would vouch
    // for bytes we never saw, and quarantining would destroy data whose only
    // sin was a flaky link. Leave the .pending marker and the unverified flag
    // in place so the startup sweep (or the next presign) retries; reads keep
    // gating on it meanwhile, so correctness holds while it is unresolved.
    if unreadable > 0 {
        tracing::warn!(
            repo = %repo,
            pack = %pack_oid,
            unreadable,
            corrupt = bad.len(),
            attempts = VERIFY_ATTEMPTS,
            elapsed_ms = verify_start.elapsed().as_millis() as u64,
            "background pack verification: incomplete after retries (entries unreadable); \
             pack stays unverified for a later sweep, nothing quarantined"
        );
        return false;
    }

    if clean {
        let elapsed = verify_start.elapsed();
        let bytes: u64 = manifest.iter().map(|e| e.length as u64).sum();
        // A pack that verifies far slower than its peers is the difference
        // between a clone downloading direct from the bucket and relaying every
        // byte through this server, because a pack is only presignable once
        // this work finishes. On 2026-08-20 one pack took 2628765 ms for 21
        // entries (~8 KB/s)
        // while its 31 siblings averaged ~10 s, and the ONLY trace of it was a
        // single `elapsed_ms` on this line — no rate, no size, and nothing at
        // all until it finally finished 44 minutes later. Diagnosing it needed
        // hours of log archaeology that a warning here would have replaced.
        //
        // Threshold, not a gate: nothing changes behaviour, it just stops a
        // pathological read from being invisible while it happens.
        const SLOW_VERIFY_WARN_SECS: u64 = 60;
        let mbs = if elapsed.as_secs_f64() > 0.0 {
            (bytes as f64 / (1024.0 * 1024.0)) / elapsed.as_secs_f64()
        } else {
            0.0
        };
        if elapsed.as_secs() >= SLOW_VERIFY_WARN_SECS {
            tracing::warn!(
                repo = %repo,
                pack = %pack_oid,
                entries = manifest.len(),
                bytes,
                elapsed_ms = elapsed.as_millis() as u64,
                mb_per_sec = format!("{mbs:.3}"),
                // Was: "a clone waiting on this pack is blocked for the whole
                // duration". That stopped being true in a816b3c — presign now
                // answers immediately and the client proxies — and a log line
                // that describes behaviour the code no longer has will misdirect
                // the next person reading it at 2am.
                "pack verification was pathologically slow; until it finishes, every clone \
                 of this pack relays through the server instead of downloading direct"
            );
        }
        tracing::info!(
            repo = %repo,
            pack = %pack_oid,
            entries = manifest.len(),
            bytes,
            elapsed_ms = elapsed.as_millis() as u64,
            mb_per_sec = format!("{mbs:.3}"),
            "background pack verification: pack verified clean"
        );
    } else {
        let bad_ids: Vec<String> = bad.iter().map(|e| e.chunk_oid.clone()).collect();
        tracing::error!(
            repo = %repo,
            pack = %pack_oid,
            bad_entries = bad_ids.len(),
            "background pack verification: quarantining corrupted chunk(s)"
        );
        if let Err(status) =
            evict_pack_entries(&state, &repo_path, &repo, &pack_oid, &bad_ids).await
        {
            tracing::error!(
                repo = %repo,
                pack = %pack_oid,
                ?status,
                "background pack verification: failed to evict corrupted entries"
            );
        }
    }

    let marker_path = pending_marker_path(&repo_path, &pack_oid);
    if let Err(e) = tokio::fs::remove_file(&marker_path).await
        && e.kind() != std::io::ErrorKind::NotFound
    {
        tracing::warn!(
            repo = %repo,
            pack = %pack_oid,
            err = %e,
            "background pack verification: failed to remove .pending marker"
        );
    }
    {
        let mut unverified = state.unverified_packs.write().await;
        if let Some(set) = unverified.get_mut(&repo) {
            set.remove(&pack_oid);
        }
    }

    clean
}

/// Startup-sweep hook (Stage C durability): given an orphaned `.pending`
/// marker found under a repo's pack dir, mark that pack unverified and
/// (re-)enqueue background verification. Called once per marker discovered
/// by `main.rs`'s existing startup probe.
///
/// Deliberately returns `()`, not `Result` — this can never `bail!` the way
/// the storage-init probe does. A stuck or unreadable marker is a *data*
/// problem, not a *config* problem; refusing to boot over one would turn
/// recoverable state into an outage. Any failure here is logged and the pack
/// is simply left marked unverified for a future sweep to retry — the read
/// path (Stage D) refuses to serve an unverified pack blind, so correctness
/// holds while it waits.
pub async fn resume_pack_verification(
    state: &Arc<AppState>,
    repo_path: &std::path::Path,
    repo: &str,
    pack_oid: &str,
) {
    // Mark unverified FIRST, unconditionally — even if everything below fails,
    // the in-memory state must reflect what the marker on disk already says.
    {
        let mut unverified = state.unverified_packs.write().await;
        unverified
            .entry(repo.to_string())
            .or_default()
            .insert(pack_oid.to_string());
    }

    let Some(manifest) = read_pack_manifest(repo_path, pack_oid).await else {
        tracing::warn!(
            repo,
            pack_oid,
            "startup sweep: could not read or parse manifest for a pending pack; leaving unverified for a future sweep"
        );
        return;
    };

    let storage = match get_or_init_storage(state, repo_path).await {
        Ok(s) => s,
        Err(status) => {
            tracing::warn!(
                repo,
                pack_oid,
                ?status,
                "startup sweep: storage backend init failed for a pending pack; leaving unverified for a future sweep"
            );
            return;
        }
    };

    // Route through the same in-flight cell every other caller uses
    // (`complete_pack`, `ensure_pack_verified_for_presign`). This spawn used to
    // call `verify_pack_in_background` directly, so a presign arriving while
    // the startup sweep was still working could start a SECOND full
    // read-and-hash pass of the same pack — every entry pulled over the WAN
    // twice, which is the exact duplication this cell exists to prevent.
    // The 1-permit semaphore serialised the two passes but did not stop either
    // doing the work.
    let state2 = Arc::clone(state);
    let repo_path2 = repo_path.to_path_buf();
    let repo2 = repo.to_string();
    let pack_oid2 = pack_oid.to_string();
    tokio::spawn(async move {
        let key = (repo2.clone(), pack_oid2.clone());
        let cell = get_or_create_pack_verify_cell(&state2, &key).await;
        cell.get_or_init(|| {
            verify_pack_in_background(
                Arc::clone(&state2),
                repo_path2,
                repo2,
                pack_oid2,
                storage,
                manifest,
            )
        })
        .await;
        state2.pack_verify_inflight.lock().await.remove(&key);
    });
}

/// POST /{repo}/packs/complete — Register a finished cloud pack and its chunk manifest.
///
/// Server HEADs the pack object before accepting the manifest so a crash between
/// PUT and complete cannot create a manifest pointing at a missing pack.
///
/// Content verification (when `verify_chunks_on_complete` is on) is PAC-async:
/// it does not run on this request. Invariant: **a persisted manifest is
/// either verified, or it carries a `.pending` marker** — never neither. See
/// `pending_marker_path` and `verify_pack_in_background`.
pub async fn complete_pack(
    Path(repo): Path<String>,
    State(state): State<Arc<AppState>>,
    auth_user: Option<Extension<AuthUser>>,
    Json(req): Json<CompletePackRequest>,
) -> Result<StatusCode, StatusCode> {
    // DELIBERATELY NOT stamping data-plane activity here.
    //
    // A push's remaining uploads use the UPLINK; verification re-reads use the
    // DOWNLINK, and the contention that was actually measured on 2026-09-15 was
    // downlink-vs-downlink — verification stealing 4.57 of 9.17 MB/s from a
    // CLONE. There is no measurement showing a push is hurt by concurrent
    // verification, and deferring here would also postpone verification through
    // every push, which is when a freshly uploaded pack is most worth checking.
    //
    // Stamps live on the download side only (presign_pack_downloads,
    // download_chunk, batch_get_pack_chunks). Fix what was measured.
    check_permission(
        auth_user.as_deref(),
        "repo:write",
        state.is_auth_enabled(),
        &state.grants,
        &repo,
    )?;

    crate::security::validate_repo_name(&repo).map_err(|_| StatusCode::BAD_REQUEST)?;
    let repo_path = state.repos_dir.join(&repo);
    if !repo_path.exists() {
        return Err(StatusCode::NOT_FOUND);
    }
    let storage = get_or_init_storage(&state, &repo_path).await?;

    let pack_key = format!("packs/{}", req.pack_oid);
    let pack_size: u64;
    match storage.head(&pack_key).await {
        Ok(Some(size)) => pack_size = size,
        Ok(None) => {
            tracing::warn!(
                repo = %repo,
                pack = %req.pack_oid,
                "complete_pack: pack object missing in storage (409)"
            );
            return Err(StatusCode::CONFLICT);
        }
        Err(e) => {
            // Storage error (transient network, backend hiccup). One retry after
            // a short delay before declaring the pack missing — avoids false 409
            // on momentary backend errors when the PUT succeeded.
            tracing::warn!(
                repo = %repo,
                pack = %req.pack_oid,
                err = %e,
                "complete_pack: HEAD check failed with storage error; retrying once"
            );
            tokio::time::sleep(std::time::Duration::from_millis(300)).await;
            match storage.head(&pack_key).await {
                Ok(Some(size)) => pack_size = size,
                _ => {
                    tracing::error!(
                        repo = %repo,
                        pack = %req.pack_oid,
                        "complete_pack: pack still not found after retry (409)"
                    );
                    return Err(StatusCode::CONFLICT);
                }
            }
        }
    }

    // Pack-manifest parity check: every manifest entry's byte range must fit
    // within the pack object's actual (HEAD-reported) size. A manifest
    // claiming ranges beyond the real pack — from a truncated upload, a
    // client bug, or a malicious request — would silently corrupt every
    // future chunk read through this pack, so it's rejected here rather
    // than accepted and discovered later at read time.
    if let Some(bad) = first_entry_exceeding_pack_size(&req.manifest, pack_size) {
        tracing::error!(
            repo = %repo,
            pack = %req.pack_oid,
            pack_size,
            chunk = %bad.chunk_oid,
            offset = bad.offset,
            length = bad.length,
            "complete_pack: manifest entry range exceeds pack size (parity check failed)"
        );
        return Err(StatusCode::BAD_REQUEST);
    }

    // Content verification (PAC, async): a pack whose bytes don't hash to the
    // chunk ids its manifest claims must never be TRUSTED — but verifying it
    // synchronously here is what made a 1 GB push take 42+ minutes (measured
    // 2026-07-30: 0.226 MiB/s aggregate read-back vs ~7.5 MiB/s upload). So
    // instead of blocking on it, this request writes a durable `.pending`
    // marker BEFORE the manifest, registers the pack as UNVERIFIED, returns,
    // and hands verification to a background task.
    //
    // Ordering is the entire safety property: a crash between the marker
    // write and the manifest write leaves a marker with no manifest (harmless
    // — nothing points at it yet). It must never leave a manifest with no
    // marker, or that pack would be trusted forever with no record that it
    // still needs checking. Gated by the same `verify_chunks_on_complete`
    // knob as the loose-chunk path (one policy, not a second knob an operator
    // could half-disable without realising) — when it's off, none of this
    // runs and behavior is identical to today.
    if state.verify_chunks_on_complete {
        let marker_path = pending_marker_path(&repo_path, &req.pack_oid);
        if let Some(parent) = marker_path.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        }
        let path = marker_path.clone();
        tokio::task::spawn_blocking(move || {
            mediagit_versioning::atomic_write::write_atomic(&path, b"")
        })
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    }

    // Persist to JSONL under <repo>/.mediagit/packs/<shard>/<pack_oid>.jsonl
    let manifest_dir = repo_path
        .join(".mediagit")
        .join("packs")
        .join(pack_shard(&req.pack_oid));
    tokio::fs::create_dir_all(&manifest_dir)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let manifest_path = manifest_dir.join(format!("{}.jsonl", req.pack_oid));

    let manifest_len = req.manifest.len();
    let mut jsonl = String::new();
    for entry in &req.manifest {
        let line = PackIndexLine {
            chunk_oid: entry.chunk_oid.clone(),
            pack_oid: req.pack_oid.clone(),
            offset: entry.offset,
            length: entry.length,
            compressed_hash: entry.compressed_hash.clone(),
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
    // The unverified-set registration happens under this SAME lock so it can
    // never drift from pack_index — the two are updated atomically together.
    {
        let mut idx = state.pack_index.write().await;
        // ST-2: atomic. This JSONL *is* the routing table for every chunk in
        // the pack, so a crash or ENOSPC part-way through a plain write leaves
        // a truncated manifest: chunks silently unroutable, or a half-written
        // final line. The pack bytes are already verified present and
        // size-consistent above, so this write is the last step that can
        // desynchronise registration from the blob it describes.
        // `spawn_blocking` because `write_atomic` fsyncs.
        {
            let path = manifest_path.clone();
            let bytes = jsonl.clone().into_bytes();
            tokio::task::spawn_blocking(move || {
                mediagit_versioning::atomic_write::write_atomic(&path, &bytes)
            })
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        }
        let repo_idx = idx.entry(repo.clone()).or_default();
        for entry in &req.manifest {
            repo_idx.insert(
                entry.chunk_oid.clone(),
                PackLoc {
                    pack_oid: req.pack_oid.clone(),
                    offset: entry.offset,
                    length: entry.length,
                    compressed_hash: entry.compressed_hash.clone(),
                },
            );
        }
        if state.verify_chunks_on_complete {
            let mut unverified = state.unverified_packs.write().await;
            unverified
                .entry(repo.clone())
                .or_default()
                .insert(req.pack_oid.clone());
        }
    }

    if state.verify_chunks_on_complete {
        let state2 = Arc::clone(&state);
        let repo_path2 = repo_path.clone();
        let repo2 = repo.clone();
        let pack_oid2 = req.pack_oid.clone();
        let storage2 = Arc::clone(&storage);
        let manifest2 = req.manifest;
        tokio::spawn(async move {
            let key = (repo2.clone(), pack_oid2.clone());
            let cell = get_or_create_pack_verify_cell(&state2, &key).await;
            cell.get_or_init(|| {
                verify_pack_in_background(
                    Arc::clone(&state2),
                    repo_path2,
                    repo2,
                    pack_oid2,
                    storage2,
                    manifest2,
                )
            })
            .await;
            state2.pack_verify_inflight.lock().await.remove(&key);
        });
    }

    tracing::info!(
        repo = %repo,
        pack = %req.pack_oid,
        chunks = manifest_len,
        "Pack manifest registered"
    );
    Ok(StatusCode::CREATED)
}

/// Remove chunk entries from a pack's persisted JSONL manifest and the
/// in-memory pack_index (QA-013 A1). Used by `verify_chunk_integrity` when a
/// packed chunk's content no longer matches its claimed hash — eviction lets
/// subsequent locate/get calls fall through to a loose re-upload instead of
/// repeatedly serving corrupt bytes from the pack.
///
/// No-op if the manifest file is missing; still drops any stale in-memory
/// entries in that case.
pub(crate) async fn evict_pack_entries(
    state: &AppState,
    repo_path: &std::path::Path,
    repo: &str,
    pack_oid: &str,
    chunk_oids: &[String],
) -> Result<(), StatusCode> {
    let shard = if pack_oid.len() >= 2 {
        &pack_oid[..2]
    } else {
        "00"
    };
    let manifest_path = repo_path
        .join(".mediagit")
        .join("packs")
        .join(shard)
        .join(format!("{}.jsonl", pack_oid));

    // Hold the same write lock complete_pack uses so eviction can't race a
    // concurrent complete_pack append (F9 concurrency guard).
    let mut idx = state.pack_index.write().await;

    let content = match tokio::fs::read_to_string(&manifest_path).await {
        Ok(c) => c,
        Err(_) => {
            if let Some(repo_idx) = idx.get_mut(repo) {
                for id in chunk_oids {
                    repo_idx.remove(id);
                }
            }
            return Ok(());
        }
    };

    let evict_set: std::collections::HashSet<&str> =
        chunk_oids.iter().map(|s| s.as_str()).collect();
    let mut kept = String::new();
    for line in content.lines() {
        if line.is_empty() {
            continue;
        }
        if let Ok(entry) = serde_json::from_str::<PackIndexLine>(line)
            && evict_set.contains(entry.chunk_oid.as_str())
        {
            continue;
        }
        kept.push_str(line);
        kept.push('\n');
    }

    // ST-2: the hand-rolled tmp+rename here used a *predictable, shared*
    // temp name (`<pack>.jsonl.tmp`), so two concurrent prunes of the same
    // pack raced on one file and the last rename won — the lost-update shape
    // fixed elsewhere as VC-3. It also never fsynced, so the rename could be
    // durable while the contents were not. `write_atomic` gives both a
    // process/thread-unique temp name and the fsync.
    {
        let path = manifest_path.clone();
        let bytes = kept.clone().into_bytes();
        tokio::task::spawn_blocking(move || {
            mediagit_versioning::atomic_write::write_atomic(&path, &bytes)
        })
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    }

    if let Some(repo_idx) = idx.get_mut(repo) {
        for id in chunk_oids {
            repo_idx.remove(id);
        }
    }

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
    pub compressed_hash: Option<String>,
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
    check_permission(
        auth_user.as_deref(),
        "repo:read",
        state.is_auth_enabled(),
        &state.grants,
        &repo,
    )?;

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
                        compressed_hash: loc.compressed_hash.clone(),
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
                            compressed_hash: loc.compressed_hash.clone(),
                        },
                    )
                })
            })
            .collect()
    };

    Ok(Json(result))
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
    check_permission(
        auth_user.as_deref(),
        "repo:write",
        state.is_auth_enabled(),
        &state.grants,
        &repo,
    )?;

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

#[cfg(test)]
mod tests {
    use super::*;

    /// ST-2: a pack manifest must never be observable half-written.
    ///
    /// The JSONL is the routing table for every chunk in the pack, so a
    /// truncated one makes chunks unroutable or yields a partial final line.
    /// `write_atomic` publishes by rename, so a concurrent reader sees either
    /// the previous contents or the complete new ones — never a prefix.
    #[test]
    fn pack_manifest_write_is_all_or_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("ab").join("deadbeef.jsonl");

        let big = "x".repeat(512 * 1024);
        mediagit_versioning::atomic_write::write_atomic(&path, big.as_bytes()).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap().len(), big.len());

        // Replacement is also all-or-nothing, and leaves no temp residue that
        // a later prune could mistake for a manifest.
        let small = "y".repeat(16);
        mediagit_versioning::atomic_write::write_atomic(&path, small.as_bytes()).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), small);

        let leftovers: Vec<_> = std::fs::read_dir(path.parent().unwrap())
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().to_string())
            .filter(|n| n != "deadbeef.jsonl")
            .collect();
        assert!(
            leftovers.is_empty(),
            "atomic write must not leave temp files behind, found: {leftovers:?}"
        );
    }

    /// The prune path previously built its temp file as
    /// `<pack>.jsonl.tmp` — one shared, predictable name. Two concurrent
    /// prunes of the same pack therefore wrote the same temp file and the
    /// last rename won, silently discarding the other's result: the VC-3
    /// lost-update shape. Unique temp names are what make this safe.
    #[test]
    fn concurrent_writers_to_one_manifest_do_not_share_a_temp_name() {
        let dir = tempfile::tempdir().unwrap();
        let path = std::sync::Arc::new(dir.path().join("cd").join("feedface.jsonl"));

        let handles: Vec<_> = (0..8)
            .map(|i| {
                let path = std::sync::Arc::clone(&path);
                std::thread::spawn(move || {
                    let body = format!("{i}").repeat(4096);
                    mediagit_versioning::atomic_write::write_atomic(&path, body.as_bytes())
                })
            })
            .collect();
        for h in handles {
            h.join().unwrap().expect("every writer must succeed");
        }

        // Whoever landed last, the file must be exactly one writer's payload
        // — not a blend, and not truncated.
        let final_bytes = std::fs::read_to_string(path.as_path()).unwrap();
        assert_eq!(final_bytes.len(), 4096, "torn or interleaved write");
        let first = final_bytes.chars().next().unwrap();
        assert!(
            final_bytes.chars().all(|c| c == first),
            "manifest contains bytes from more than one writer"
        );
    }

    fn entry(chunk_oid: &str, offset: u64, length: u32) -> ManifestEntry {
        ManifestEntry {
            chunk_oid: chunk_oid.to_string(),
            offset,
            length,
            compressed_hash: None,
        }
    }

    #[test]
    fn parity_check_passes_when_all_entries_fit() {
        let manifest = vec![entry("a", 0, 100), entry("b", 100, 200)];
        assert!(first_entry_exceeding_pack_size(&manifest, 300).is_none());
    }

    #[test]
    fn parity_check_passes_at_exact_boundary() {
        // offset + length == pack_size is valid (exclusive upper bound).
        let manifest = vec![entry("a", 0, 300)];
        assert!(first_entry_exceeding_pack_size(&manifest, 300).is_none());
    }

    #[test]
    fn parity_check_flags_entry_exceeding_pack_size() {
        let manifest = vec![entry("a", 0, 100), entry("b", 100, 201)];
        let bad = first_entry_exceeding_pack_size(&manifest, 300).unwrap();
        assert_eq!(bad.chunk_oid, "b");
    }

    #[test]
    fn parity_check_handles_offset_overflow_without_panicking() {
        let manifest = vec![entry("a", u64::MAX - 1, 100)];
        let bad = first_entry_exceeding_pack_size(&manifest, 300).unwrap();
        assert_eq!(bad.chunk_oid, "a");
    }

    #[test]
    fn parity_check_empty_manifest_passes() {
        assert!(first_entry_exceeding_pack_size(&[], 0).is_none());
    }
}

#[cfg(test)]
mod complete_pack_content_verification_tests {
    use super::*;

    struct PackFixture {
        pack_bytes: Vec<u8>,
        manifest: Vec<ManifestEntry>,
    }

    /// Builds a well-formed two-chunk pack: each entry gets the real 5-byte
    /// `[type:1][size:4]` header (content irrelevant — the server only skips
    /// it) followed by its compressed bytes, and `chunk_oid` is the real
    /// BLAKE3 of the uncompressed content — what a correct client produces.
    fn build_valid_pack() -> PackFixture {
        let compressor = SmartCompressor::new();
        let contents: [&[u8]; 2] = [
            b"the quick brown fox jumps over the lazy dog",
            b"a second, different chunk of content in the same pack",
        ];
        let mut pack_bytes = Vec::new();
        let mut manifest = Vec::new();
        for content in contents {
            let chunk_id = blake3::hash(content).to_hex().to_string();
            let compressed = compressor.compress(content).expect("compress");
            let offset = pack_bytes.len() as u64;
            pack_bytes.extend_from_slice(&[0u8; 5]);
            pack_bytes.extend_from_slice(&compressed);
            let length = (5 + compressed.len()) as u32;
            manifest.push(ManifestEntry {
                chunk_oid: chunk_id,
                offset,
                length,
                compressed_hash: None,
            });
        }
        PackFixture {
            pack_bytes,
            manifest,
        }
    }

    /// Same pack, but the *second* entry's compressed bytes are corrupted
    /// (last byte flipped) so BLAKE3(decompressed) no longer matches its
    /// claimed `chunk_oid` — a poisoned chunk shipped inside an otherwise
    /// well-formed pack. Mirrors `write_corrupt_pack_entry` in transfer.rs.
    fn build_pack_with_corrupted_entry() -> PackFixture {
        let mut fixture = build_valid_pack();
        let bad = &fixture.manifest[1];
        let data_end = (bad.offset + bad.length as u64) as usize;
        let last = data_end - 1;
        fixture.pack_bytes[last] ^= 0xFF;
        fixture
    }

    async fn setup(repo: &str) -> (tempfile::TempDir, Arc<AppState>, std::path::PathBuf) {
        let tmp = tempfile::tempdir().unwrap();
        let repo_path = tmp.path().join(repo);
        tokio::fs::create_dir_all(&repo_path).await.unwrap();
        // Verification defaults to OFF (see `ServerConfig::verify_content_on_complete`
        // for the measured reason), so this fixture opts in explicitly — these tests
        // exist to exercise verification, and a test that silently rode an implicit
        // default would stop testing anything the day the default moved. Which is
        // precisely what happened when it did.
        let state =
            Arc::new(AppState::new(tmp.path().to_path_buf()).with_verify_chunks_on_complete(true));
        (tmp, state, repo_path)
    }

    #[tokio::test]
    async fn accepts_valid_pack_and_persists_manifest() {
        let repo = "test-repo".to_string();
        let (_tmp, state, repo_path) = setup(&repo).await;
        let storage = get_or_init_storage(&state, &repo_path)
            .await
            .expect("storage");
        let fixture = build_valid_pack();
        let pack_oid = "aabbccddee";
        storage
            .put(&format!("packs/{pack_oid}"), &fixture.pack_bytes)
            .await
            .expect("put pack");

        let req = CompletePackRequest {
            pack_oid: pack_oid.to_string(),
            manifest: fixture.manifest,
        };
        let status = complete_pack(Path(repo), State(Arc::clone(&state)), None, Json(req))
            .await
            .expect("valid pack must be accepted");
        assert_eq!(status, StatusCode::CREATED);

        let manifest_path = repo_path
            .join(".mediagit")
            .join("packs")
            .join("aa")
            .join(format!("{pack_oid}.jsonl"));
        assert!(
            manifest_path.exists(),
            "manifest must be persisted for a pack whose content verifies"
        );
    }

    /// Polls until a pack leaves `state.unverified_packs`, i.e. until the
    /// background verifier spawned by `complete_pack` (or resumed by
    /// `resume_pack_verification`) has finished with it. Bounded so a real
    /// regression (verification never runs, or never clears the set) fails
    /// the test instead of hanging forever.
    async fn wait_until_pack_verified(state: &AppState, repo: &str, pack_oid: &str) {
        for _ in 0..200 {
            {
                let unverified = state.unverified_packs.read().await;
                if !unverified.get(repo).is_some_and(|s| s.contains(pack_oid)) {
                    return;
                }
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        panic!("background pack verification did not complete within 2s");
    }

    /// PAC design: `complete_pack` must return before verification has run —
    /// this is what earns back push throughput. Immediately after it returns,
    /// the pack must already be durably marked unverified: the `.pending`
    /// marker on disk AND the in-memory `unverified_packs` entry.
    #[tokio::test]
    async fn complete_pack_returns_before_verifying_and_marks_unverified() {
        let repo = "test-repo".to_string();
        let (_tmp, state, repo_path) = setup(&repo).await;
        let storage = get_or_init_storage(&state, &repo_path)
            .await
            .expect("storage");
        let fixture = build_valid_pack();
        let pack_oid = "55667788cc";
        storage
            .put(&format!("packs/{pack_oid}"), &fixture.pack_bytes)
            .await
            .expect("put pack");

        let req = CompletePackRequest {
            pack_oid: pack_oid.to_string(),
            manifest: fixture.manifest,
        };
        let status = complete_pack(
            Path(repo.clone()),
            State(Arc::clone(&state)),
            None,
            Json(req),
        )
        .await
        .expect("push must return promptly, without waiting for verification");
        assert_eq!(status, StatusCode::CREATED);

        // No `.await` between complete_pack returning and these checks, so the
        // spawned background task cannot have run yet — this proves the state
        // complete_pack itself left behind, not a race with the worker.
        let marker_path = pending_marker_path(&repo_path, pack_oid);
        assert!(
            marker_path.exists(),
            "pending marker must exist as soon as complete_pack returns"
        );
        let unverified = state.unverified_packs.read().await;
        assert!(
            unverified.get(&repo).is_some_and(|s| s.contains(pack_oid)),
            "pack must be in the unverified set as soon as complete_pack returns"
        );
    }

    /// Regression test for a FALSE QUARANTINE found only by a real 1 GB push to
    /// S3 (`bad_entries=9` on an entirely valid pack, with 2006 green tests
    /// behind it).
    ///
    /// `decompress_typed` falls back to the RAW bytes when a decoder errors,
    /// because `detect()` misfires on incompressible data that happens to begin
    /// with a codec magic — raw `0x78` followed by a byte satisfying zlib's
    /// header checksum. The streaming path reused `detect()` but not that
    /// recovery, so such a chunk decoded to an error and was rejected as
    /// corrupt. Under PAC that means quarantining good data.
    ///
    /// The fixture is a chunk stored WITHOUT the Store prefix whose first two
    /// bytes are a valid-looking zlib header — exactly the shape that fools
    /// detection — with a chunk_oid over the raw bytes. Verification must accept
    /// it.
    #[tokio::test]
    async fn accepts_a_chunk_whose_raw_bytes_look_like_a_codec_header() {
        let repo = "test-repo".to_string();
        let (_tmp, state, repo_path) = setup(&repo).await;
        let storage = get_or_init_storage(&state, &repo_path)
            .await
            .expect("storage");

        // 0x78 0x9C is a textbook zlib header (0x789C % 31 == 0), so detect()
        // returns Zlib — but the remainder is not a zlib stream, so the decoder
        // errors and only the raw-bytes fallback can identify this chunk.
        let mut body = vec![0x78u8, 0x9C];
        body.extend_from_slice(b"raw bytes that merely look compressed, and are not");
        let chunk_oid = blake3::hash(&body).to_hex().to_string();

        let mut pack_bytes = Vec::new();
        pack_bytes.extend_from_slice(&[0u8; 5]); // 5-byte entry header
        pack_bytes.extend_from_slice(&body);
        let manifest = vec![ManifestEntry {
            chunk_oid: chunk_oid.clone(),
            offset: 0,
            length: (5 + body.len()) as u32,
            compressed_hash: None,
        }];

        let pack_oid = "ee".repeat(32);
        storage
            .put(&format!("packs/{pack_oid}"), &pack_bytes)
            .await
            .expect("put pack");

        let compressor = Arc::new(SmartCompressor::new());
        let bad = first_pack_entry_failing_content_verification(
            &storage,
            &compressor,
            &format!("packs/{pack_oid}"),
            &manifest,
        )
        .await;

        assert!(
            bad.is_none(),
            "a chunk whose RAW bytes hash to its id was rejected because detect() \
                misread them as zlib — the streaming path is missing the fallback that \
                decompress_typed applies, and would quarantine valid data"
        );
    }

    /// The ordering that makes the whole design crash-safe: the `.pending`
    /// marker must land on disk strictly BEFORE the manifest.
    ///
    /// Asserted by INDUCING A MARKER-WRITE FAILURE, not by racing a poller
    /// against the two writes. The earlier version of this test span-polled for
    /// a window where the marker existed and the manifest did not; that window
    /// is real but its observability depends on the scheduler, so a CORRECT
    /// implementation could fail the test under load. An intermittent red on
    /// correct code is worse than no test — this suite has already lost a long
    /// investigation to one non-deterministic failure.
    ///
    /// Here a directory is planted at the marker path so `write_atomic` cannot
    /// create the file. `complete_pack` must then fail BEFORE writing the
    /// manifest. If the ordering were reversed the manifest would exist despite
    /// the marker failing — so the absence of the manifest is positive proof of
    /// the ordering, and it is deterministic.
    #[tokio::test]
    async fn marker_write_failure_prevents_manifest_from_being_written() {
        let repo = "test-repo".to_string();
        let (_tmp, state, repo_path) = setup(&repo).await;
        let storage = get_or_init_storage(&state, &repo_path)
            .await
            .expect("storage");
        let fixture = build_valid_pack();
        let pack_oid = "abcdef0123";
        storage
            .put(&format!("packs/{pack_oid}"), &fixture.pack_bytes)
            .await
            .expect("put pack");

        // Plant a directory exactly where the marker file must go: write_atomic
        // renames onto this path, which cannot succeed against a directory.
        let marker_path = pending_marker_path(&repo_path, pack_oid);
        tokio::fs::create_dir_all(&marker_path)
            .await
            .expect("plant blocking directory at marker path");

        let manifest_path = repo_path
            .join(".mediagit")
            .join("packs")
            .join(pack_shard(pack_oid))
            .join(format!("{pack_oid}.jsonl"));
        assert!(
            !manifest_path.exists(),
            "test precondition: manifest must not exist yet"
        );

        let req = CompletePackRequest {
            pack_oid: pack_oid.to_string(),
            manifest: fixture.manifest.clone(),
        };
        let result = complete_pack(
            Path(repo.clone()),
            State(Arc::clone(&state)),
            None,
            Json(req),
        )
        .await;

        assert!(
            result.is_err(),
            "complete_pack must fail when the .pending marker cannot be written"
        );
        assert!(
            !manifest_path.exists(),
            "manifest was written despite the marker failing — the marker is NOT being \
                written first, and a crash in that gap would leave a pack trusted forever"
        );
        let idx = state.pack_index.read().await;
        assert!(
            idx.get(&repo).is_none_or(|m| {
                fixture
                    .manifest
                    .iter()
                    .all(|e| !m.contains_key(&e.chunk_oid))
            }),
            "no chunk may be routable after a failed registration"
        );
    }

    /// Background verification, success path: once it runs, the marker is
    /// removed and the pack leaves `unverified_packs`.
    #[tokio::test]
    async fn background_verification_success_clears_marker_and_unverified_set() {
        let repo = "test-repo".to_string();
        let (_tmp, state, repo_path) = setup(&repo).await;
        let storage = get_or_init_storage(&state, &repo_path)
            .await
            .expect("storage");
        let fixture = build_valid_pack();
        let pack_oid = "11223344bb";
        storage
            .put(&format!("packs/{pack_oid}"), &fixture.pack_bytes)
            .await
            .expect("put pack");

        let req = CompletePackRequest {
            pack_oid: pack_oid.to_string(),
            manifest: fixture.manifest.clone(),
        };
        let status = complete_pack(
            Path(repo.clone()),
            State(Arc::clone(&state)),
            None,
            Json(req),
        )
        .await
        .expect("valid pack must be accepted");
        assert_eq!(status, StatusCode::CREATED);

        let marker_path = pending_marker_path(&repo_path, pack_oid);
        assert!(
            marker_path.exists(),
            "marker must exist right after complete_pack, before background verify runs"
        );

        wait_until_pack_verified(&state, &repo, pack_oid).await;

        assert!(
            !marker_path.exists(),
            "marker must be removed once background verification completes cleanly"
        );
        let idx = state.pack_index.read().await;
        let repo_idx = idx.get(&repo).expect("repo entry must exist");
        for entry in &fixture.manifest {
            assert!(
                repo_idx.contains_key(&entry.chunk_oid),
                "valid entries must remain routable after verification"
            );
        }
    }

    /// Background verification, failure path: with TWO corrupted entries in
    /// one pack, BOTH must be evicted — not just the first found — and the
    /// pack must still leave the unverified set once resolved.
    #[tokio::test]
    async fn background_verification_evicts_all_corrupted_entries_not_just_first() {
        let repo = "test-repo".to_string();
        let (_tmp, state, repo_path) = setup(&repo).await;
        let storage = get_or_init_storage(&state, &repo_path)
            .await
            .expect("storage");
        let fixture = build_pack_with_two_corrupted_entries();
        let bad_ids: Vec<String> = [1usize, 2usize]
            .iter()
            .map(|&i| fixture.manifest[i].chunk_oid.clone())
            .collect();
        let good_id = fixture.manifest[0].chunk_oid.clone();
        let pack_oid = "99aabbccdd";
        storage
            .put(&format!("packs/{pack_oid}"), &fixture.pack_bytes)
            .await
            .expect("put pack");

        let req = CompletePackRequest {
            pack_oid: pack_oid.to_string(),
            manifest: fixture.manifest,
        };
        let status = complete_pack(
            Path(repo.clone()),
            State(Arc::clone(&state)),
            None,
            Json(req),
        )
        .await
        .expect("push must not block on verification, even for a pack that turns out bad");
        assert_eq!(status, StatusCode::CREATED);

        wait_until_pack_verified(&state, &repo, pack_oid).await;

        let idx = state.pack_index.read().await;
        let repo_idx = idx.get(&repo).expect("repo entry must exist");
        for bad_id in &bad_ids {
            assert!(
                !repo_idx.contains_key(bad_id),
                "corrupted entry must be evicted from pack_index: {bad_id}"
            );
        }
        assert!(
            repo_idx.contains_key(&good_id),
            "the valid entry must remain routable"
        );

        let marker_path = pending_marker_path(&repo_path, pack_oid);
        assert!(
            !marker_path.exists(),
            "marker must be cleared once background verification finishes, even on failure"
        );
    }

    /// P0 regression: a pack whose bytes cannot be READ IN FULL must never be
    /// quarantined. "I could not verify this" is not "this is corrupt", and
    /// conflating them evicts valid data permanently.
    ///
    /// This is not hypothetical. On a clean 1 GB push to real S3 (2026-08-03)
    /// transient range-read failures evicted 26 valid chunks across 2 packs
    /// while the push reported success; the bytes were later proven byte-exact
    /// against the source fixture. The old code returned the entry index for
    /// BOTH a read error and a hash mismatch, so eviction followed either way.
    ///
    /// Reproduced deterministically as "the pack verified fine on push, then
    /// the object could not be fully read on a later retry" — a crash-resume
    /// where the link is flaky. Truncating the stored object makes the range
    /// read end early, so the digest would cover a prefix. Required behaviour:
    /// nothing evicted, pack stays unverified, `.pending` marker survives.
    #[tokio::test]
    async fn unreadable_pack_is_left_unverified_and_never_quarantined() {
        let repo = "test-repo".to_string();
        let (_tmp, state, repo_path) = setup(&repo).await;
        let storage = get_or_init_storage(&state, &repo_path)
            .await
            .expect("storage");
        let fixture = build_valid_pack();
        let all_ids: Vec<String> = fixture
            .manifest
            .iter()
            .map(|e| e.chunk_oid.clone())
            .collect();
        let pack_oid = "aa11bb22cc";
        let pack_key = format!("packs/{pack_oid}");
        let full_bytes = fixture.pack_bytes.clone();
        storage.put(&pack_key, &full_bytes).await.expect("put pack");

        let req = CompletePackRequest {
            pack_oid: pack_oid.to_string(),
            manifest: fixture.manifest,
        };
        let status = complete_pack(
            Path(repo.clone()),
            State(Arc::clone(&state)),
            None,
            Json(req),
        )
        .await
        .expect("complete_pack must still return promptly");
        assert_eq!(status, StatusCode::CREATED);
        wait_until_pack_verified(&state, &repo, pack_oid).await;

        // Now the object becomes unreadable-in-full, and a sweep retries it.
        // Every byte still present is genuine; there is simply not all of it.
        storage
            .put(&pack_key, &full_bytes[..full_bytes.len() / 2])
            .await
            .expect("truncate pack");
        let marker_path = pending_marker_path(&repo_path, pack_oid);
        mediagit_versioning::atomic_write::write_atomic(&marker_path, b"")
            .expect("re-arm pending marker");

        resume_pack_verification(&state, &repo_path, &repo, pack_oid).await;
        // Deliberately NOT wait_until_pack_verified: staying unverified IS the
        // expected outcome here, so waiting for it to clear would hang.
        //
        // Must outlast the in-process retry ladder (2s + 4s backoff) so this
        // asserts the TERMINAL state. A shorter wait would observe verification
        // still in flight, and would pass even if the final verdict were to
        // quarantine — which is the exact bug under test. The truncation is
        // permanent, so all attempts fail deterministically.
        tokio::time::sleep(std::time::Duration::from_secs(9)).await;

        let idx = state.pack_index.read().await;
        let repo_idx = idx.get(&repo).expect("repo entry must exist");
        for id in &all_ids {
            assert!(
                repo_idx.contains_key(id),
                "unreadable != corrupt: entry {id} must NOT be evicted just because \
                 its bytes could not be read in full"
            );
        }
        drop(idx);

        let unverified = state.unverified_packs.read().await;
        assert!(
            unverified.get(&repo).is_some_and(|s| s.contains(pack_oid)),
            "a pack that could not be verified must stay UNVERIFIED, so reads keep gating on it"
        );
        drop(unverified);

        assert!(
            marker_path.exists(),
            "the .pending marker must survive so a later sweep retries the check"
        );
    }

    /// Startup sweep, recovery path: an orphaned `.pending` marker with no
    /// corresponding in-memory state (simulating a fresh `AppState` after a
    /// restart) must be picked up, marked unverified, and driven to
    /// completion — exactly like a pack that just went through `complete_pack`.
    #[tokio::test]
    async fn resume_pack_verification_marks_orphaned_marker_unverified_and_completes() {
        let repo = "test-repo".to_string();
        let (_tmp, state, repo_path) = setup(&repo).await;
        let storage = get_or_init_storage(&state, &repo_path)
            .await
            .expect("storage");
        let fixture = build_valid_pack();
        let pack_oid = "ddeeff0011";
        storage
            .put(&format!("packs/{pack_oid}"), &fixture.pack_bytes)
            .await
            .expect("put pack");

        // Write marker + manifest directly, bypassing complete_pack, so
        // `state.unverified_packs` starts genuinely empty for this pack —
        // exactly what a fresh AppState after a restart would see.
        let manifest_dir = repo_path.join(".mediagit").join("packs").join("dd");
        tokio::fs::create_dir_all(&manifest_dir).await.unwrap();
        let mut jsonl = String::new();
        for e in &fixture.manifest {
            jsonl.push_str(&format!(
                "{{\"chunk_oid\":\"{}\",\"pack_oid\":\"{pack_oid}\",\"offset\":{},\"length\":{}}}\n",
                e.chunk_oid, e.offset, e.length
            ));
        }
        tokio::fs::write(manifest_dir.join(format!("{pack_oid}.jsonl")), jsonl)
            .await
            .unwrap();
        let marker_path = pending_marker_path(&repo_path, pack_oid);
        tokio::fs::write(&marker_path, b"").await.unwrap();

        assert!(
            state.unverified_packs.read().await.get(&repo).is_none(),
            "precondition: fresh AppState has no in-memory record of this pack"
        );

        resume_pack_verification(&state, &repo_path, &repo, pack_oid).await;

        // Marked unverified synchronously by resume_pack_verification itself,
        // before its own background spawn has necessarily run.
        assert!(
            state
                .unverified_packs
                .read()
                .await
                .get(&repo)
                .is_some_and(|s| s.contains(pack_oid)),
            "orphaned marker must mark the pack unverified"
        );

        wait_until_pack_verified(&state, &repo, pack_oid).await;
        assert!(
            !marker_path.exists(),
            "orphaned marker must be cleared once the resumed verification completes"
        );
    }

    /// Startup sweep, failure semantics: a marker whose manifest is missing
    /// or unreadable must be logged and left unverified for a future sweep —
    /// it must NEVER propagate an error or panic (the storage-init probe
    /// `bail!`s on a bad config; a bad `.pending` marker is a data problem,
    /// not a config problem, and must not turn into a boot failure).
    #[tokio::test]
    async fn resume_pack_verification_survives_unreadable_manifest_without_bailing() {
        let repo = "test-repo".to_string();
        let (_tmp, state, repo_path) = setup(&repo).await;
        let pack_oid = "feedface01";
        // Marker exists, but there is no manifest at all — e.g. a crash
        // between the marker write and the manifest write.
        let marker_path = pending_marker_path(&repo_path, pack_oid);
        if let Some(parent) = marker_path.parent() {
            tokio::fs::create_dir_all(parent).await.unwrap();
        }
        tokio::fs::write(&marker_path, b"").await.unwrap();

        // Must not panic (a bare .await here would abort the test on panic).
        resume_pack_verification(&state, &repo_path, &repo, pack_oid).await;

        assert!(
            state
                .unverified_packs
                .read()
                .await
                .get(&repo)
                .is_some_and(|s| s.contains(pack_oid)),
            "an unreadable/missing manifest must still leave the pack marked unverified"
        );
        assert!(
            marker_path.exists(),
            "marker must be left in place (not falsely cleared) when the manifest could not be read"
        );
    }

    #[tokio::test]
    async fn accepts_corrupted_pack_when_verification_disabled() {
        let repo = "test-repo".to_string();
        let tmp = tempfile::tempdir().unwrap();
        let repo_path = tmp.path().join(&repo);
        tokio::fs::create_dir_all(&repo_path).await.unwrap();
        let mut state = AppState::new(tmp.path().to_path_buf());
        state.verify_chunks_on_complete = false;
        let state = Arc::new(state);

        let storage = get_or_init_storage(&state, &repo_path)
            .await
            .expect("storage");
        let fixture = build_pack_with_corrupted_entry();
        let pack_oid = "01020304ab";
        storage
            .put(&format!("packs/{pack_oid}"), &fixture.pack_bytes)
            .await
            .expect("put pack");

        let req = CompletePackRequest {
            pack_oid: pack_oid.to_string(),
            manifest: fixture.manifest,
        };
        let status = complete_pack(Path(repo), State(Arc::clone(&state)), None, Json(req))
            .await
            .expect("verification disabled: even a corrupted pack must be accepted");
        assert_eq!(status, StatusCode::CREATED);

        // Proves the knob actually does something (not just an ignored field).
        let manifest_path = repo_path
            .join(".mediagit")
            .join("packs")
            .join("01")
            .join(format!("{pack_oid}.jsonl"));
        assert!(manifest_path.exists());
    }

    /// Same pack as [`build_valid_pack`], but with three chunks, and the
    /// *last two* entries' compressed bytes corrupted (last byte flipped) —
    /// used to prove `.min()` picks the lower-index failure deterministically.
    fn build_pack_with_two_corrupted_entries() -> PackFixture {
        let compressor = SmartCompressor::new();
        let contents: [&[u8]; 3] = [
            b"the quick brown fox jumps over the lazy dog",
            b"a second, different chunk of content in the same pack",
            b"a third chunk, also different, rounding out the pack",
        ];
        let mut pack_bytes = Vec::new();
        let mut manifest = Vec::new();
        for content in contents {
            let chunk_id = blake3::hash(content).to_hex().to_string();
            let compressed = compressor.compress(content).expect("compress");
            let offset = pack_bytes.len() as u64;
            pack_bytes.extend_from_slice(&[0u8; 5]);
            pack_bytes.extend_from_slice(&compressed);
            let length = (5 + compressed.len()) as u32;
            manifest.push(ManifestEntry {
                chunk_oid: chunk_id,
                offset,
                length,
                compressed_hash: None,
            });
        }
        for bad_idx in [1usize, 2usize] {
            let bad = &manifest[bad_idx];
            let last = (bad.offset + bad.length as u64) as usize - 1;
            pack_bytes[last] ^= 0xFF;
        }
        PackFixture {
            pack_bytes,
            manifest,
        }
    }

    /// Exercises the streaming verification path directly (the HTTP-level
    /// tests above only prove `complete_pack`'s end-to-end behavior).
    #[tokio::test]
    async fn streaming_verify_matches_expected_verdict_valid_and_corrupted() {
        let repo = "test-repo".to_string();
        let (_tmp, _state, repo_path) = setup(&repo).await;
        let storage = LocalBackend::new(repo_path.join(".mediagit"))
            .await
            .expect("local backend");
        let storage: Arc<dyn StorageBackend> = Arc::new(storage);
        let compressor = Arc::new(SmartCompressor::new());

        let valid = build_valid_pack();
        storage
            .put("packs/valid-via-stream", &valid.pack_bytes)
            .await
            .expect("put pack");
        let bad = first_pack_entry_failing_content_verification(
            &storage,
            &compressor,
            "packs/valid-via-stream",
            &valid.manifest,
        )
        .await;
        assert!(bad.is_none(), "a valid pack must verify clean");

        let corrupted = build_pack_with_corrupted_entry();
        storage
            .put("packs/corrupted-via-stream", &corrupted.pack_bytes)
            .await
            .expect("put pack");
        let bad = first_pack_entry_failing_content_verification(
            &storage,
            &compressor,
            "packs/corrupted-via-stream",
            &corrupted.manifest,
        )
        .await;
        assert_eq!(
            bad.map(|e| e.chunk_oid.clone()),
            Some(corrupted.manifest[1].chunk_oid.clone()),
            "streaming verification must identify the corrupted entry"
        );
    }

    /// Fail-closed: an entry whose declared length can't even hold the 5-byte
    /// pack-entry header must be rejected without touching storage.
    #[tokio::test]
    async fn entry_shorter_than_header_is_rejected() {
        let repo = "test-repo".to_string();
        let (_tmp, _state, repo_path) = setup(&repo).await;
        let storage = LocalBackend::new(repo_path.join(".mediagit"))
            .await
            .expect("local backend");
        let storage: Arc<dyn StorageBackend> = Arc::new(storage);
        let compressor = Arc::new(SmartCompressor::new());

        let manifest = vec![ManifestEntry {
            chunk_oid: "a".repeat(64),
            offset: 0,
            length: 4, // < 5-byte header
            compressed_hash: None,
        }];
        let bad = first_pack_entry_failing_content_verification(
            &storage,
            &compressor,
            "packs/does-not-matter",
            &manifest,
        )
        .await;
        assert_eq!(
            bad.map(|e| e.chunk_oid.clone()),
            Some("a".repeat(64)),
            "an entry shorter than the pack-entry header must be rejected, not skipped"
        );
    }

    /// Fail-closed, but not fail-destructive: a storage read error (here, the
    /// pack object doesn't exist) must NOT count as verified — and must also
    /// not count as corrupt.
    ///
    /// This test previously asserted the entry came back in the single "bad"
    /// list, which is precisely the conflation that evicted 26 valid chunks on
    /// a clean S3 push (2026-08-03). The fail-closed intent is unchanged and
    /// still asserted; what changed is that "unreadable" is now its own verdict
    /// so quarantine cannot follow from it.
    #[tokio::test]
    async fn range_read_error_is_unreadable_not_corrupt() {
        let repo = "test-repo".to_string();
        let (_tmp, _state, repo_path) = setup(&repo).await;
        let storage = LocalBackend::new(repo_path.join(".mediagit"))
            .await
            .expect("local backend");
        let storage: Arc<dyn StorageBackend> = Arc::new(storage);
        let compressor = Arc::new(SmartCompressor::new());

        let manifest = vec![ManifestEntry {
            chunk_oid: "b".repeat(64),
            offset: 0,
            length: 10,
            compressed_hash: None,
        }];
        let (corrupt, unreadable) = pack_entries_failing_content_verification(
            &storage,
            &compressor,
            "packs/never-uploaded",
            &manifest,
        )
        .await;
        assert_eq!(
            unreadable,
            vec![0],
            "a range read error must be reported as unreadable"
        );
        assert!(
            corrupt.is_empty(),
            "a range read error says nothing about the bytes; treating it as corruption \
             evicts valid data"
        );
    }

    /// Determinism: with two corrupted entries, the LOWER-index one is always
    /// reported, regardless of which concurrent read finishes first — proven
    /// by repeating the check, not by inspecting the `.min()` call site.
    #[tokio::test]
    async fn lower_index_corrupted_entry_reported_deterministically() {
        let repo = "test-repo".to_string();
        let (_tmp, _state, repo_path) = setup(&repo).await;
        let storage = LocalBackend::new(repo_path.join(".mediagit"))
            .await
            .expect("local backend");
        let storage: Arc<dyn StorageBackend> = Arc::new(storage);
        let compressor = Arc::new(SmartCompressor::new());

        let fixture = build_pack_with_two_corrupted_entries();
        storage
            .put("packs/two-corrupted", &fixture.pack_bytes)
            .await
            .expect("put pack");

        for _ in 0..20 {
            let bad = first_pack_entry_failing_content_verification(
                &storage,
                &compressor,
                "packs/two-corrupted",
                &fixture.manifest,
            )
            .await;
            assert_eq!(
                bad.map(|e| e.chunk_oid.clone()),
                Some(fixture.manifest[1].chunk_oid.clone()),
                "the lower-index corrupted entry must be reported every time"
            );
        }
    }
}

#[cfg(test)]
mod pack_verify_budget_tests {
    use super::*;

    /// Delegates everything except the one call verification makes, which it
    /// delays. Models the failure actually observed on 2026-08-20: the read was
    /// never dead, so no no-progress deadline could fire — it simply trickled
    /// (21 entries, 2628s, ~8 KB/s) while holding the only verify permit.
    #[derive(Debug)]
    struct TrickleBackend {
        inner: Arc<dyn StorageBackend>,
        delay: std::time::Duration,
    }

    #[async_trait::async_trait]
    impl StorageBackend for TrickleBackend {
        async fn get(&self, key: &str) -> anyhow::Result<Vec<u8>> {
            self.inner.get(key).await
        }
        async fn put(&self, key: &str, data: &[u8]) -> anyhow::Result<()> {
            self.inner.put(key, data).await
        }
        async fn exists(&self, key: &str) -> anyhow::Result<bool> {
            self.inner.exists(key).await
        }
        async fn delete(&self, key: &str) -> anyhow::Result<()> {
            self.inner.delete(key).await
        }
        async fn list_objects(&self, prefix: &str) -> anyhow::Result<Vec<String>> {
            self.inner.list_objects(prefix).await
        }
        async fn head(&self, key: &str) -> anyhow::Result<Option<u64>> {
            self.inner.head(key).await
        }
        async fn get_streaming_range(
            &self,
            key: &str,
            range: std::ops::Range<u64>,
        ) -> anyhow::Result<
            std::pin::Pin<
                Box<dyn futures::Stream<Item = anyhow::Result<bytes::Bytes>> + Send + 'static>,
            >,
        > {
            tokio::time::sleep(self.delay).await;
            self.inner.get_streaming_range(key, range).await
        }
    }

    fn build_pack() -> (Vec<u8>, Vec<ManifestEntry>) {
        let compressor = SmartCompressor::new();
        let contents: [&[u8]; 2] = [b"budget chunk one", b"budget chunk two, different"];
        let mut pack_bytes = Vec::new();
        let mut manifest = Vec::new();
        for content in contents {
            let chunk_id = blake3::hash(content).to_hex().to_string();
            let compressed = compressor.compress(content).expect("compress");
            let offset = pack_bytes.len() as u64;
            pack_bytes.extend_from_slice(&[0u8; 5]);
            pack_bytes.extend_from_slice(&compressed);
            manifest.push(ManifestEntry {
                chunk_oid: chunk_id,
                offset,
                length: (5 + compressed.len()) as u32,
                compressed_hash: None,
            });
        }
        (pack_bytes, manifest)
    }

    struct Outcome {
        clean: bool,
        elapsed: std::time::Duration,
        pack_still_intact: bool,
    }

    /// Calls `verify_pack_with_budget` directly, NOT `verify_pack_in_background`,
    /// so `elapsed` spans the budgeted work and nothing else. The permit for the
    /// process-global, 1-permit `pack_verify_semaphore` is taken by
    /// `verify_pack_in_background`, one level up: routing through it would put an
    /// unbounded queue wait inside this clock, and every other test in this binary
    /// that verifies a pack is in that queue.
    ///
    /// That queue is what broke CI twice. 2026-08-26, macos-latest/aarch64: 5.89s
    /// against a 150ms budget; then 6.18s on Windows, which is precisely
    /// `unreadable_pack_is_left_unverified_and_never_quarantined`'s 2s + 4s retry
    /// ladder holding the permit. Both times the timeout HAD fired — both are far
    /// below the backend's own 30s delay — so the product was fine and the
    /// measurement was not. A `tokio::sync::Mutex` around these two tests was the
    /// first attempt and was too narrow: it serialised this pair against each
    /// other, not against the rest of the binary. Raising the threshold instead
    /// would blunt the assertion and leave the flake.
    async fn verify_with(delay: std::time::Duration, budget: std::time::Duration) -> Outcome {
        let repo = "budget-repo".to_string();
        let tmp = tempfile::tempdir().unwrap();
        let repo_path = tmp.path().join(&repo);
        tokio::fs::create_dir_all(&repo_path).await.unwrap();
        let state =
            Arc::new(AppState::new(tmp.path().to_path_buf()).with_verify_chunks_on_complete(true));
        let inner = get_or_init_storage(&state, &repo_path)
            .await
            .expect("storage");

        let (pack_bytes, manifest) = build_pack();
        let pack_oid = "beefcafe01".to_string();
        let pack_key = format!("packs/{pack_oid}");
        inner.put(&pack_key, &pack_bytes).await.expect("put pack");

        let slow: Arc<dyn StorageBackend> = Arc::new(TrickleBackend {
            inner: Arc::clone(&inner),
            delay,
        });

        let started = std::time::Instant::now();
        let clean = verify_pack_with_budget(
            Arc::clone(&state),
            repo_path.clone(),
            repo.clone(),
            pack_oid.clone(),
            slow,
            manifest,
            budget,
        )
        .await;
        let elapsed = started.elapsed();

        // Untouched bytes are the observable proof that a timeout parked the pack
        // rather than treating it as corrupt: the quarantine path would have
        // evicted entries via `evict_pack_entries`.
        let pack_still_intact = inner
            .get(&pack_key)
            .await
            .map(|b| b == pack_bytes)
            .unwrap_or(false);

        Outcome {
            clean,
            elapsed,
            pack_still_intact,
        }
    }

    /// The half that must FIRE.
    ///
    /// The backend trickles for 30s; the budget is 150ms. The load-bearing
    /// assertion is ELAPSED: returning `false` alone would also happen if
    /// verification failed for some unrelated reason, and a test that cannot
    /// tell those apart is not testing the timeout. Returning in well under the
    /// backend's own delay is what proves the budget, and nothing else, ended it.
    /// multi_thread: the budget is enforced by a `tokio::time::timeout` timer,
    /// and on the default current-thread runtime that timer cannot fire while
    /// the verification future is doing CPU-bound decompress/hash work without
    /// yielding. A second worker means a late timer can never be mistaken for a
    /// budget that failed to trip.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn trickling_pack_gives_up_its_verify_permit() {
        let out = verify_with(
            std::time::Duration::from_secs(30),
            std::time::Duration::from_millis(150),
        )
        .await;

        assert!(
            !out.clean,
            "a pack that blew its wall-clock budget must not be reported clean"
        );
        assert!(
            out.elapsed < std::time::Duration::from_secs(5),
            "budget must end the attempt early; took {:?}, but the backend's own delay is 30s              — that means the timeout did not fire and this test would pass for the wrong reason",
            out.elapsed
        );
        assert!(
            out.pack_still_intact,
            "a budget breach must park the pack, not quarantine it: the pack bytes must survive"
        );
    }

    /// The half that must stay QUIET. Healthy per-pack verification measured
    /// ~12.7s mean against live GCS and the default budget is 300s, so a guard
    /// that tripped on normal work would be worse than no guard at all.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn healthy_pack_verifies_well_inside_its_budget() {
        let out = verify_with(
            std::time::Duration::from_millis(0),
            std::time::Duration::from_secs(30),
        )
        .await;
        assert!(
            out.clean,
            "a fast, valid pack must verify clean and must NOT be tripped by the budget"
        );
        assert!(out.pack_still_intact, "a clean pack must not be evicted");
    }

    #[test]
    fn budget_defaults_to_300s() {
        // 300s is ~3x the worst healthy per-pack verification observed against
        // live GCS (103s) and ~9x below the 2628s pathological case.
        assert_eq!(pack_verify_budget().as_secs(), 300);
    }
}
