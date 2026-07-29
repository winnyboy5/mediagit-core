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

    // Advertise the repo's CDC seed (if any) so clones inherit matching chunk
    // boundaries. Omitted entirely when the seed is 0 (legacy repos / repos
    // without the field) to keep the capability list unchanged for them.
    let mut capabilities = vec!["pack-v1".to_string()];
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

/// POST /:repo/objects/pack - Upload a pack file (streaming)
pub async fn upload_pack(
    Path(repo): Path<String>,
    State(state): State<Arc<AppState>>,
    auth_user: Option<Extension<AuthUser>>,
    body: axum::body::Body,
) -> Result<StatusCode, StatusCode> {
    tracing::info!("POST /{}/objects/pack (streaming)", repo);

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

#[derive(serde::Deserialize)]
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
    }

    tracing::info!(
        repo = %repo,
        pack = %req.pack_oid,
        chunks = req.manifest.len(),
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
