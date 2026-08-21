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

/// Does this presigned URL's signed header set already carry `content-length`?
///
/// SigV4 signs `content-length` — it is NOT in aws-sigv4's `excluded_headers`
/// (only Authorization, User-Agent, X-Ray-Trace-Id and Transfer-Encoding are) —
/// so whenever the server presigns with a concrete length, `content-length`
/// lands in `required_headers` and forms part of `SignedHeaders`.
///
/// Adding our own on top is not a harmless overwrite:
/// `reqwest::RequestBuilder::header` calls `HeaderMap::append`, not `insert`, so
/// it emits a SECOND `content-length` line. A duplicated signed header does not
/// canonicalize back to the single value that was signed, and S3/MinIO answer
/// `SignatureDoesNotMatch` — 976 and 748 of them in campaigns 20260803-scale and
/// 20260804-scale-verify2, all on this per-chunk fallback path. The pack path
/// (`pack_builder.rs`) never had the bug because it only ever replays
/// `required_headers`, and it logged none.
///
/// `tests/presigned_put_headers.rs` pins this at the socket: hyper does not
/// collapse the duplicate, so the second instance really does reach the peer.
///
/// The server still legitimately presigns unbound URLs (`content_length == 0`),
/// which carry no signed `content-length`; those DO need one supplied.
fn signs_content_length(required_headers: &[[String; 2]]) -> bool {
    required_headers
        .iter()
        .any(|h| h[0].eq_ignore_ascii_case("content-length"))
}

impl ProtocolClient {
    /// Push local objects and update remote refs
    ///
    /// # Arguments
    /// * `odb` - Local object database
    /// * `updates` - List of ref updates to apply
    /// * `force` - Force update even if not fast-forward
    ///
    /// Returns the ref update response and push statistics
    pub async fn push(
        &self,
        odb: &ObjectDatabase,
        updates: Vec<RefUpdate>,
        force: bool,
    ) -> Result<(RefUpdateResponse, PushStats)> {
        let mut stats = PushStats::default();

        // Collect commit OIDs from ref updates (what we want to push)
        let mut commit_oids = Vec::new();
        // Collect "have" OIDs - objects remote already has (to exclude from push)
        let mut have_oids = Vec::new();

        for update in &updates {
            let oid = Oid::from_hex(&update.new_oid)
                .context(format!("Invalid OID in update: {}", update.new_oid))?;
            commit_oids.push(oid);

            // If remote has an existing OID, add it to "have" list
            if let Some(old_oid) = &update.old_oid
                && let Ok(oid) = Oid::from_hex(old_oid)
            {
                have_oids.push(oid);
            }
        }

        // Collect only NEW objects (not reachable from remote's current state)
        if !commit_oids.is_empty() {
            let objects = self
                .collect_reachable_objects(odb, commit_oids, have_oids)
                .await?;

            stats.objects_count = objects.len();
            stats.commits_count = objects
                .iter()
                .filter(|(_, t)| matches!(t, ObjectType::Commit))
                .count();
            stats.trees_count = objects
                .iter()
                .filter(|(_, t)| matches!(t, ObjectType::Tree))
                .count();
            stats.blobs_count = objects
                .iter()
                .filter(|(_, t)| matches!(t, ObjectType::Blob))
                .count();

            tracing::info!(
                "Collected {} objects for push ({} commits, {} trees, {} blobs)",
                stats.objects_count,
                stats.commits_count,
                stats.trees_count,
                stats.blobs_count
            );

            // Only upload if there are new objects
            if !objects.is_empty() {
                // Generate and upload pack file with new objects only
                let (pack_data, chunked_oids) = self.generate_pack(odb, objects).await?;
                stats.bytes_uploaded = pack_data.len();
                self.upload_pack(&pack_data).await?;

                // Upload chunked objects (large files) if any
                if !chunked_oids.is_empty() {
                    let (_chunks, chunk_bytes) = self
                        .upload_chunked_objects(odb, &chunked_oids, |_, _| {})
                        .await?;
                    // RP-1: `+=`, not `=` — the metadata pack above is real
                    // upload too, just a tiny fraction of it.
                    stats.bytes_uploaded += chunk_bytes as usize;
                }
            } else {
                tracing::info!("No new objects to push - remote already has all objects");
            }
        }

        // Update refs
        let request = RefUpdateRequest {
            updates,
            force,
            force_with_lease: false,
        };
        let response = self.update_refs(request).await?;
        Ok((response, stats))
    }

    /// Push local objects and update remote refs with progress tracking
    ///
    /// # Arguments
    /// * `odb` - Local object database
    /// * `updates` - List of ref updates to apply
    /// * `force` - Force update even if not fast-forward
    /// * `on_progress` - Callback function for progress updates
    ///
    /// Returns the ref update response and push statistics
    /// `force_with_lease` is a third mode, not a synonym for `force`: it
    /// waives the server's ancestry requirement while keeping the `old_oid`
    /// compare-and-swap, so a rewritten history can be pushed but a
    /// concurrent update by someone else is still refused.
    pub async fn push_with_progress<F>(
        &self,
        odb: &ObjectDatabase,
        updates: Vec<RefUpdate>,
        force: bool,
        force_with_lease: bool,
        on_progress: F,
    ) -> Result<(RefUpdateResponse, PushStats)>
    where
        F: Fn(PushProgress),
    {
        let mut stats = PushStats::default();

        // Collect commit OIDs from ref updates (what we want to push)
        let mut commit_oids = Vec::new();
        let mut have_oids = Vec::new();

        for update in &updates {
            let oid = Oid::from_hex(&update.new_oid)
                .context(format!("Invalid OID in update: {}", update.new_oid))?;
            commit_oids.push(oid);

            if let Some(old_oid) = &update.old_oid
                && let Ok(oid) = Oid::from_hex(old_oid)
            {
                have_oids.push(oid);
            }
        }

        // Phase 1: Collect objects with progress
        on_progress(PushProgress {
            phase: PushPhase::Collecting,
            current: 0,
            total: 0,
            message: "Collecting objects...".to_string(),
        });

        let objects = if !commit_oids.is_empty() {
            self.collect_reachable_objects(odb, commit_oids, have_oids)
                .await?
        } else {
            Vec::new()
        };

        stats.objects_count = objects.len();
        stats.commits_count = objects
            .iter()
            .filter(|(_, t)| matches!(t, ObjectType::Commit))
            .count();
        stats.trees_count = objects
            .iter()
            .filter(|(_, t)| matches!(t, ObjectType::Tree))
            .count();
        stats.blobs_count = objects
            .iter()
            .filter(|(_, t)| matches!(t, ObjectType::Blob))
            .count();

        on_progress(PushProgress {
            phase: PushPhase::Collecting,
            current: stats.objects_count as u64,
            total: stats.objects_count as u64,
            message: format!("Found {} objects", stats.objects_count),
        });

        // A7: bound the total wall time spent on network uploads so a mid-push
        // backend outage fails fast with a clear error instead of hanging on
        // retries forever. Absolute deadline (not stall-based): the default is
        // generous enough that a real large push won't trip it; lower
        // MEDIAGIT_PUSH_DEADLINE_SECS to fail faster on a dead backend.
        // ponytail: absolute deadline, upgrade to a progress-reset stall
        // deadline if multi-hour legit pushes ever false-trip it.
        let push_deadline_secs = std::env::var("MEDIAGIT_PUSH_DEADLINE_SECS")
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            .filter(|&s| s > 0)
            .unwrap_or(3600);
        let push_deadline =
            tokio::time::Instant::now() + std::time::Duration::from_secs(push_deadline_secs);
        let deadline_err = || {
            anyhow::anyhow!(
                "push aborted: exceeded MEDIAGIT_PUSH_DEADLINE_SECS ({push_deadline_secs}s) \
                 uploading to remote; the storage backend may be unavailable"
            )
        };

        if !objects.is_empty() {
            // Phase 2: Generate pack with progress
            let total_objects = objects.len() as u64;
            on_progress(PushProgress {
                phase: PushPhase::Packing,
                current: 0,
                total: total_objects,
                message: "Generating pack...".to_string(),
            });

            let (pack_data, chunked_oids) = self.generate_pack(odb, objects).await?;
            stats.bytes_uploaded = pack_data.len();

            on_progress(PushProgress {
                phase: PushPhase::Packing,
                current: total_objects,
                total: total_objects,
                message: format!(
                    "Packed {} objects ({} bytes)",
                    total_objects,
                    pack_data.len()
                ),
            });

            // Phase 3: Upload pack with progress
            on_progress(PushProgress {
                phase: PushPhase::Uploading,
                current: 0,
                total: pack_data.len() as u64,
                message: "Uploading pack...".to_string(),
            });

            tokio::time::timeout_at(push_deadline, self.upload_pack(&pack_data))
                .await
                .map_err(|_| deadline_err())??;

            on_progress(PushProgress {
                phase: PushPhase::Uploading,
                current: pack_data.len() as u64,
                total: pack_data.len() as u64,
                message: "Pack upload complete".to_string(),
            });

            // Phase 4: Upload chunked objects (large files)
            if !chunked_oids.is_empty() {
                let upload =
                    self.upload_chunked_objects(odb, &chunked_oids, |bytes_done, bytes_total| {
                        on_progress(PushProgress {
                            phase: PushPhase::Uploading,
                            current: bytes_done,
                            total: bytes_total,
                            message: String::new(),
                        });
                    });
                // RP-1: the `??` discarded the upload's own byte count, so
                // `bytes_uploaded` kept only the metadata pack size assigned
                // above and the summary under-reported by orders of magnitude.
                let (_chunks, chunk_bytes) = tokio::time::timeout_at(push_deadline, upload)
                    .await
                    .map_err(|_| deadline_err())??;
                stats.bytes_uploaded += chunk_bytes as usize;
            }
        } else {
            tracing::info!("No new objects to push");
        }

        // Update refs
        let request = RefUpdateRequest {
            updates,
            force,
            force_with_lease,
        };
        let response = tokio::time::timeout_at(push_deadline, self.update_refs(request))
            .await
            .map_err(|_| deadline_err())??;
        Ok((response, stats))
    }

    /// Upload a pack file to the server
    pub(crate) async fn upload_pack(&self, pack_data: &[u8]) -> Result<()> {
        let url = format!("{}/objects/pack", self.base_url);
        tracing::debug!("POST {} ({} bytes)", url, pack_data.len());

        let response = crate::client::send_with_rate_limit_retry(|| {
            self.client
                .post(&url)
                .header("Content-Type", "application/octet-stream")
                .body(pack_data.to_vec())
                .send()
        })
        .await
        .context("Failed to upload pack file")?;

        let status = response.status();
        if !status.is_success() {
            // A rejection here is almost always authorization, and "403
            // Forbidden" on its own does not tell the user what to do about it.
            let hint = match status.as_u16() {
                401 => "\n  Not authenticated. Run `mediagit auth login <server>`.",
                403 => {
                    "\n  Authenticated, but this account cannot push to this repository. \
                     An admin must grant it write access."
                }
                _ => "",
            };
            anyhow::bail!("POST /objects/pack failed with status: {status}{hint}");
        }

        Ok(())
    }

    /// Collect all NEW objects reachable from given commit OIDs
    ///
    /// Performs depth-first graph traversal to collect commits, trees, and blobs.
    /// Excludes objects reachable from `have_oids` (objects remote already has).
    /// Returns vec of (OID, ObjectType) tuples for NEW objects only.
    async fn collect_reachable_objects(
        &self,
        odb: &ObjectDatabase,
        commit_oids: Vec<Oid>,
        have_oids: Vec<Oid>,
    ) -> Result<Vec<(Oid, ObjectType)>> {
        let mut visited = HashSet::new();
        let mut result = Vec::new();
        let mut queue = VecDeque::new();

        // OPTIMIZATION: First, mark all objects reachable from "have" commits as already visited
        // This prevents us from collecting objects the remote already has
        if !have_oids.is_empty() {
            let mut have_queue = VecDeque::new();
            for oid in have_oids {
                if visited.insert(oid) {
                    let obj_type = detect_object_type(odb, &oid).await;
                    have_queue.push_back((oid, obj_type));
                }
            }

            // Walk the "have" graph to mark all reachable objects as visited
            while let Some((oid, obj_type)) = have_queue.pop_front() {
                // Don't add to result - we're just marking as visited
                // OPTIMIZATION: Skip reading blobs entirely. Blobs are leaf nodes
                // with no child references to traverse. Reading them is wasteful,
                // especially for chunked objects (large video/audio files) where
                // odb.read() must reassemble the entire file from chunks.
                if obj_type == ObjectType::Blob {
                    continue;
                }

                if let Ok(obj_data) = odb.read(&oid).await {
                    match obj_type {
                        ObjectType::Commit => {
                            if let Ok(commit) =
                                mediagit_versioning::format::deserialize::<Commit>(&obj_data)
                            {
                                if visited.insert(commit.tree) {
                                    have_queue.push_back((commit.tree, ObjectType::Tree));
                                }
                                for parent_oid in commit.parents {
                                    if visited.insert(parent_oid) {
                                        have_queue.push_back((parent_oid, ObjectType::Commit));
                                    }
                                }
                            }
                        }
                        ObjectType::Tree => {
                            if let Ok(tree) =
                                mediagit_versioning::format::deserialize::<Tree>(&obj_data)
                            {
                                for entry in tree.entries.values() {
                                    if visited.insert(entry.oid) {
                                        let entry_type = match entry.mode {
                                            FileMode::Directory => ObjectType::Tree,
                                            _ => ObjectType::Blob,
                                        };
                                        have_queue.push_back((entry.oid, entry_type));
                                    }
                                }
                            }
                        }
                        ObjectType::Tag => {
                            if let Ok(tag) = Tag::deserialize(&obj_data)
                                && visited.insert(tag.target)
                            {
                                have_queue.push_back((tag.target, tag.target_type));
                            }
                        }
                        // Blob is filtered above; this arm satisfies exhaustiveness.
                        ObjectType::Blob => {}
                    }
                }
            }

            tracing::debug!("Marked {} objects as already on remote", visited.len());
        }

        // Now collect only NEW objects (not in visited set). Ref-update
        // targets are usually commits, but can also be annotated Tag
        // objects (e.g. `push --tags`), so the type must be detected rather
        // than assumed.
        for oid in commit_oids {
            if visited.insert(oid) {
                let obj_type = detect_object_type(odb, &oid).await;
                queue.push_back((oid, obj_type));
            }
        }

        while let Some((oid, obj_type)) = queue.pop_front() {
            // Add to result (this is a NEW object)
            result.push((oid, obj_type));

            // Only read object data for commits and trees (need to traverse refs)
            // Blobs are leaf nodes - no need to read their contents here
            match obj_type {
                ObjectType::Commit => {
                    let obj_data = odb
                        .read(&oid)
                        .await
                        .context(format!("Failed to read commit {}", oid))?;

                    // Deserialize commit to extract tree and parent refs
                    let commit: Commit = mediagit_versioning::format::deserialize(&obj_data)
                        .context(format!("Failed to deserialize commit {}", oid))?;

                    // Add tree OID
                    if visited.insert(commit.tree) {
                        queue.push_back((commit.tree, ObjectType::Tree));
                    }

                    // Add parent commit OIDs
                    for parent_oid in commit.parents {
                        if visited.insert(parent_oid) {
                            queue.push_back((parent_oid, ObjectType::Commit));
                        }
                    }
                }
                ObjectType::Tree => {
                    let obj_data = odb
                        .read(&oid)
                        .await
                        .context(format!("Failed to read tree {}", oid))?;

                    // Deserialize tree to extract blob/subtree refs
                    let tree: Tree = mediagit_versioning::format::deserialize(&obj_data)
                        .context(format!("Failed to deserialize tree {}", oid))?;

                    for entry in tree.entries.values() {
                        if visited.insert(entry.oid) {
                            // Determine type based on FileMode
                            let entry_type = match entry.mode {
                                FileMode::Directory => ObjectType::Tree,
                                _ => ObjectType::Blob, // Regular, Executable, Symlink
                            };
                            queue.push_back((entry.oid, entry_type));
                        }
                    }
                }
                ObjectType::Tag => {
                    let obj_data = odb
                        .read(&oid)
                        .await
                        .context(format!("Failed to read tag {}", oid))?;

                    let tag: Tag = mediagit_versioning::format::deserialize(&obj_data)
                        .context(format!("Failed to deserialize tag {}", oid))?;

                    if visited.insert(tag.target) {
                        queue.push_back((tag.target, tag.target_type));
                    }
                }
                ObjectType::Blob => {
                    // Blobs are leaf nodes - no references to follow
                    // Don't read blob content here as it could be huge (20GB chunked files)
                }
            }
        }

        Ok(result)
    }

    /// Generate a pack file containing specified objects with their types
    ///
    /// Uses incremental pack generation to minimize memory usage.
    /// Note: Chunked blobs (large files stored as chunks) are SKIPPED in packs.
    /// They should be transferred separately via manifest + chunks.
    ///
    /// Returns: (pack_data, chunked_object_oids)
    async fn generate_pack(
        &self,
        odb: &ObjectDatabase,
        objects: Vec<(Oid, ObjectType)>,
    ) -> Result<(Vec<u8>, Vec<Oid>)> {
        // First, filter out chunked objects
        let mut chunked_objects: Vec<Oid> = Vec::new();
        let mut non_chunked: Vec<(Oid, ObjectType)> = Vec::new();

        for (oid, object_type) in objects {
            if object_type == ObjectType::Blob && odb.is_chunked(&oid).await.unwrap_or(false) {
                tracing::debug!(oid = %oid, "Skipping chunked blob in pack generation");
                chunked_objects.push(oid);
            } else {
                non_chunked.push((oid, object_type));
            }
        }

        if !chunked_objects.is_empty() {
            tracing::info!(
                count = chunked_objects.len(),
                "Chunked objects to transfer separately"
            );
        }

        // Use standard PackWriter but process objects incrementally
        // Each object is read, added to pack, then data is dropped before next read
        // This avoids holding all object data in memory simultaneously
        let mut pack_writer = PackWriter::new();

        for (oid, obj_type) in non_chunked {
            // Read single object
            let obj_data = odb
                .read(&oid)
                .await
                .context(format!("Failed to read object {}", oid))?;

            // Add to pack (internally compressed/processed)
            pack_writer.add_object(oid, obj_type, &obj_data);

            // obj_data is dropped here, freeing memory before next iteration
        }

        // Finalize pack
        let pack_data = pack_writer.finalize();
        Ok((pack_data, chunked_objects))
    }

    // ========================================================================
    // Chunk Transfer Methods - For efficient large file push
    // ========================================================================

    /// Upload a single chunk-delta to the remote server.
    ///
    /// Writes via `PUT /:repo/chunk-deltas/:chunk_id` with the compressed
    /// delta payload as the body and the base chunk OID as the
    /// `X-Mediagit-Delta-Base` header. Called by `upload_chunked_objects`
    /// in its delta pass, after the base chunk has been confirmed present
    /// on the remote — otherwise the server would hold an orphaned delta
    /// that cannot be reconstructed.
    pub async fn upload_chunk_delta(
        &self,
        chunk_id: &Oid,
        base_id: &Oid,
        compressed_delta_bytes: Vec<u8>,
    ) -> Result<()> {
        let url = format!("{}/chunk-deltas/{}", self.base_url, chunk_id.to_hex());
        // Bytes outside the closure, .clone() inside — send_with_rate_limit_retry
        // re-invokes `make` per attempt, and `Fn` cannot move an owned Vec out.
        let compressed_delta_bytes: bytes::Bytes = compressed_delta_bytes.into();
        let response = crate::client::send_with_rate_limit_retry(|| {
            self.client
                .put(&url)
                .header("x-mediagit-delta-base", base_id.to_hex())
                .body(compressed_delta_bytes.clone())
                .send()
        })
        .await
        .context(format!("Failed to PUT /chunk-deltas/{}", chunk_id))?;

        if !response.status().is_success() {
            anyhow::bail!(
                "PUT /chunk-deltas/{} failed with status: {}",
                chunk_id,
                response.status()
            );
        }

        Ok(())
    }

    /// Check which chunks exist on the remote server
    ///
    /// Returns list of chunk IDs that are MISSING (need to be uploaded)
    /// Upload a manifest to the remote server
    async fn upload_manifest(&self, oid: &Oid, data: &[u8]) -> Result<()> {
        let url = format!("{}/manifests/{}", self.base_url, oid.to_hex());

        let response = crate::client::send_with_rate_limit_retry(|| {
            self.client.put(&url).body(data.to_vec()).send()
        })
        .await
        .context(format!("Failed to PUT /manifests/{}", oid))?;

        if !response.status().is_success() {
            anyhow::bail!(
                "PUT /manifests/{} failed with status: {}",
                oid,
                response.status()
            );
        }

        Ok(())
    }

    /// Upload all chunks for a single chunked object.
    ///
    /// Returns `(chunks_uploaded, bytes_uploaded, bytes_total_delta)`.
    /// Called by the B2 parallel pipeline in `upload_chunked_objects`.
    ///
    /// Immediately publishes its expected byte count to `bytes_total_progress`
    /// after the chunk-existence check, so the display denominator reflects all
    /// in-flight objects — not only those that have completed.
    async fn push_one_object(
        &self,
        oid: &Oid,
        odb: &ObjectDatabase,
        concurrent_uploads: usize,
        bytes_progress: Arc<AtomicU64>,
        bytes_total_progress: Arc<AtomicU64>,
        bench: Option<&Arc<crate::bench::BenchSession>>,
    ) -> Result<(u32, u64, u64)> {
        use futures::stream::StreamExt;
        let mut chunks_uploaded: u32 = 0;
        let mut upload_bytes: u64 = 0;
        let mut upload_bytes_total: u64 = 0;

        // Get manifest for this object
        let manifest = match odb.get_chunk_manifest(oid).await? {
            Some(m) => m,
            None => {
                tracing::warn!(oid = %oid, "No manifest found for chunked object");
                return Ok((chunks_uploaded, upload_bytes, upload_bytes_total));
            }
        };

        // Get all chunk IDs
        let chunk_ids: Vec<String> = manifest.chunks.iter().map(|c| c.id.to_hex()).collect();

        // Pre-discover local delta info for every manifest chunk. This is
        // a local DB lookup per chunk — cheap compared to a network RTT —
        // and lets us fold the chunk-existence check and the delta-base
        // existence check into ONE POST (down from two sequential RTTs).
        // We cache the delta payloads here so Pass A/C don't re-read them.
        let mut local_delta_lookup: std::collections::HashMap<Oid, (Oid, Vec<u8>)> =
            std::collections::HashMap::new();
        let mut speculative_base_hexes: std::collections::HashSet<String> =
            std::collections::HashSet::new();
        for c in &manifest.chunks {
            if let Some((base_id, delta_bytes)) = odb.get_local_chunk_delta_raw(&c.id).await? {
                speculative_base_hexes.insert(base_id.to_hex());
                local_delta_lookup.insert(c.id, (base_id, delta_bytes));
            }
        }

        // Combined existence check: manifest chunks + speculative delta bases.
        let mut combined_check: Vec<String> = chunk_ids.clone();
        let chunk_id_set: std::collections::HashSet<&String> = chunk_ids.iter().collect();
        for base_hex in &speculative_base_hexes {
            if !chunk_id_set.contains(base_hex) {
                combined_check.push(base_hex.clone());
            }
        }
        let combined_missing: std::collections::HashSet<String> = self
            .check_chunks_exist(&combined_check)
            .await?
            .into_iter()
            .collect();

        // Missing manifest chunks = subset of combined_missing.
        let missing_chunks: Vec<String> = chunk_ids
            .iter()
            .filter(|h| combined_missing.contains(*h))
            .cloned()
            .collect();

        if missing_chunks.is_empty() {
            tracing::debug!(oid = %oid, "All chunks already exist on remote");
        } else {
            tracing::info!(
                oid = %oid,
                missing = missing_chunks.len(),
                "Uploading missing chunks"
            );

            let missing_set: std::collections::HashSet<String> =
                missing_chunks.into_iter().collect();

            // Update total with this object's missing chunk bytes for accurate ETA
            upload_bytes_total += manifest
                .chunks
                .iter()
                .filter(|c| missing_set.contains(&c.id.to_hex()))
                .map(|c| c.size as u64)
                .sum::<u64>();

            // Publish expected bytes immediately (before any uploads start) so the
            // B2 pipeline denominator reflects all in-flight objects, not only
            // completed ones. Without this, the display shows e.g. 27 MiB / 699 B
            // when 8 objects are in-flight but only 1 has returned.
            bytes_total_progress.fetch_add(upload_bytes_total, Ordering::Relaxed);

            // Manifest size per missing chunk (uncompressed logical bytes).
            // Used to unify numerator units with the denominator above —
            // get_compressed_chunk returns on-disk stored bytes which diverge
            // for compressible content and cause the progress bar to overshoot.
            let chunk_manifest_sizes: std::collections::HashMap<Oid, u64> = manifest
                .chunks
                .iter()
                .filter(|c| missing_set.contains(&c.id.to_hex()))
                .map(|c| (c.id, c.size as u64))
                .collect();

            let chunks_to_upload: Vec<Oid> = manifest
                .chunks
                .iter()
                .filter(|c| missing_set.contains(&c.id.to_hex()))
                .map(|c| c.id)
                .collect();

            // Partition into locally-delta chunks (ship as deltas) and
            // plain full chunks. Reuses the pre-computed local_delta_lookup,
            // avoiding a second pass over odb.get_local_chunk_delta_raw.
            let mut full_chunks: Vec<Oid> = Vec::new();
            let mut delta_chunks: Vec<(Oid, Oid, Vec<u8>)> = Vec::new();
            for chunk_id in &chunks_to_upload {
                match local_delta_lookup.remove(chunk_id) {
                    Some((base_id, delta_bytes)) => {
                        delta_chunks.push((*chunk_id, base_id, delta_bytes));
                    }
                    None => full_chunks.push(*chunk_id),
                }
            }

            // A base is considered "landing this push" if it's any chunk in
            // chunks_to_upload — regardless of whether it lands as a full or
            // as a delta. A chunk-delta can be a base for another chunk-delta
            // (depth-2 chain); the server reconstructs the chain from the
            // .meta sidecars. Restricting this set to `full_chunks` would
            // force depth-2 chains to degrade to full uploads, silently
            // shrinking clone-side savings.
            let bases_landing_this_push: std::collections::HashSet<Oid> =
                chunks_to_upload.iter().copied().collect();

            // ── Pass A: full chunks (must land before any delta whose
            // base is in this push) ──────────────────────────────────
            let cloud_packs = std::env::var("MEDIAGIT_CLOUD_PACKS")
                .as_deref()
                .unwrap_or("1")
                != "0";
            let mut pack_had_error = false;
            let pack_pushed = if !full_chunks.is_empty() && cloud_packs {
                match self
                    .push_full_chunks_via_packs(
                        &full_chunks,
                        odb,
                        &chunk_manifest_sizes,
                        &bytes_progress,
                        bench,
                    )
                    .await
                {
                    Ok((n, b)) if n > 0 => {
                        chunks_uploaded += n;
                        upload_bytes += b;
                        // Numerator already credited per-pack inside push_full_chunks_via_packs.
                        true
                    }
                    Ok(_) => {
                        tracing::debug!("pack push: 0 chunks uploaded; using per-chunk fallback");
                        false
                    }
                    Err(e) => {
                        pack_had_error = true;
                        // `?e` not `%e`. `%` renders only the OUTERMOST anyhow
                        // context, so all eight of these in 20260821-ga11 read
                        // "upload_and_register pack" and the status code that
                        // actually explained the failure was discarded — the
                        // one fact needed to diagnose it. `?` prints the chain.
                        //
                        // The consequence is stated too, not just the event: a
                        // fallback is not a neutral retry, it is the whole push
                        // dropping to the per-chunk path. In ga11 that was
                        // 8.69 -> 0.98 MB/s, and nothing told the user why their
                        // push suddenly took 35 minutes.
                        tracing::warn!(
                            err = ?e,
                            "pack push FAILED after retries; falling back to the per-chunk                              upload path for the rest of this push. This is materially                              slower (measured ~9x on 20260821-ga11); the error above is why"
                        );
                        false
                    }
                }
            } else {
                false
            };
            if !full_chunks.is_empty() && !pack_pushed {
                // If the pack path failed mid-way, some packs may have completed
                // and registered their chunks server-side before the error. Re-check
                // so the fallback only uploads chunks that are genuinely still missing,
                // avoiding bandwidth waste and duplicate storage.
                if pack_had_error {
                    let hexes: Vec<String> = full_chunks.iter().map(|c| c.to_hex()).collect();
                    let still_missing: std::collections::HashSet<String> =
                        self.check_chunks_exist(&hexes).await?.into_iter().collect();
                    // push_full_chunks_via_packs rolled back all its numerator credits on error.
                    // Re-credit the bytes for chunks that landed in completed packs so the
                    // denominator (already published at bytes_total_progress) stays balanced
                    // and the progress bar can reach 100%.
                    let already_uploaded_bytes: u64 = full_chunks
                        .iter()
                        .filter(|id| !still_missing.contains(&id.to_hex()))
                        .filter_map(|id| chunk_manifest_sizes.get(id))
                        .sum();
                    if already_uploaded_bytes > 0 {
                        bytes_progress.fetch_add(already_uploaded_bytes, Ordering::Relaxed);
                    }
                    full_chunks.retain(|id| still_missing.contains(&id.to_hex()));
                    tracing::debug!(
                        original = hexes.len(),
                        still_missing = full_chunks.len(),
                        already_credited = already_uploaded_bytes,
                        "re-checked existence after partial pack failure"
                    );
                }

                // Request presigned PUT URLs from the server.  Cloud
                // backends return signed bucket URLs; Local/Mock return
                // null → falls through to the server-proxy PUT path.
                let full_chunk_hexes: Vec<String> =
                    full_chunks.iter().map(|c| c.to_hex()).collect();
                // Bind the presigned URL's Content-Length to the actual compressed
                // on-disk size (what will be PUT), not the manifest's uncompressed
                // size — a mismatch there causes a 403 SignatureDoesNotMatch on
                // compressible content. `None` (delta/repacked chunk) maps to 0,
                // matching the server's "unbound URL" contract.
                let chunk_sizes: std::collections::HashMap<String, u64> = {
                    let lens = futures::future::join_all(
                        full_chunks.iter().map(|id| odb.compressed_chunk_len(id)),
                    )
                    .await;
                    full_chunks
                        .iter()
                        .zip(lens)
                        .map(|(id, len)| (id.to_hex(), len.unwrap_or(0)))
                        .collect()
                };
                let presigned_urls = std::sync::Arc::new(
                    self.request_chunk_upload_urls(&full_chunk_hexes, &chunk_sizes)
                        .await,
                );

                // Per-push counter for permanent-config failures (auth, bucket policy).
                // If ≥3 distinct chunks are rejected with config-level errors the
                // operator is warned; individual chunks still fall back to proxy
                // independently — the batch is never poisoned as a whole.
                let permanent_config_failures = std::sync::Arc::new(AtomicUsize::new(0));

                // Dedicated direct-upload HTTP client with a short pool-idle window.
                // S3 edges close idle keep-alive TCP connections at ~20 s; reusing a
                // stale socket yields a RequestTimeout 400.  A 15 s idle cap ensures
                // every retry opens a fresh socket before S3 can half-close it.
                // The per-request timeout (5 min) caps stalled uploads on lossy WAN;
                // a 64 MiB chunk at 0.2 MiB/s takes ~320 s so this is generous.
                // Data-plane client for presigned PUT uploads.
                // Kept on HTTP/1.1 intentionally: parallel TCP sockets give
                // better raw throughput for large bodies than h2 multiplexing
                // on a single TCP connection (parallel cwnd > one congestion
                // window). Pool size via MEDIAGIT_HTTP_POOL_MAX (see http_pool_max()).
                crate::ensure_crypto_provider();
                let direct_client = reqwest::Client::builder()
                    .pool_idle_timeout(std::time::Duration::from_secs(60))
                    .pool_max_idle_per_host(http_pool_max())
                    .tcp_keepalive(std::time::Duration::from_secs(45))
                    .tcp_nodelay(true)
                    .timeout(std::time::Duration::from_secs(300))
                    .http1_only()
                    .build()
                    .unwrap_or_else(|_| reqwest::Client::new());

                let _pass_a_t = std::time::Instant::now();
                let mut _pass_a_n = 0u64;
                let mut _pass_a_bytes = 0u64;
                let mut stream = futures::stream::iter(full_chunks.clone())
                    .map(|chunk_id| {
                        let client = self.client.clone();
                        let direct_client = direct_client.clone();
                        let base_url = self.base_url.clone();
                        let odb = odb.clone();
                        let presigned = presigned_urls.clone();
                        let perm_fails = permanent_config_failures.clone();
                        async move {
                            // B5: convert to Bytes immediately so retry clones are O(1) refcount bumps.
                            let chunk_data: bytes::Bytes =
                                odb.get_compressed_chunk(&chunk_id).await?.into();
                            let chunk_size = chunk_data.len() as u64;
                            let hex = chunk_id.to_hex();

                            // Phase 2: MPU path — active when MEDIAGIT_STAGED_UPLOAD=1 and
                            // chunk_size >= MEDIAGIT_MPU_THRESHOLD_BYTES (default 16 MiB).
                            // Single-PUT is cheaper for 8–16 MiB chunks (no part overhead).
                            // On any failure falls through to the single-PUT path below.
                            let mut direct_succeeded = false;
                            {
                                let use_mpu = std::env::var("MEDIAGIT_STAGED_UPLOAD")
                                    .as_deref()
                                    != Ok("0");
                                let mpu_threshold: u64 =
                                    std::env::var("MEDIAGIT_MPU_THRESHOLD_BYTES")
                                        .ok()
                                        .and_then(|v| v.parse().ok())
                                        .unwrap_or(16 * 1024 * 1024);
                                if use_mpu
                                    && chunk_size >= mpu_threshold
                                    && upload_chunk_mpu(
                                        &client,
                                        &direct_client,
                                        &base_url,
                                        &hex,
                                        &chunk_data,
                                    )
                                    .await
                                {
                                    direct_succeeded = true;
                                }
                            }
                            if !direct_succeeded
                            && let Some(Some(purl)) = presigned.get(&hex) {
                                let mut current_url = purl.url.clone();
                                let mut current_headers = purl.required_headers.clone();
                                let mut resigned = false;
                                const MAX_ATTEMPTS: u32 = 5;
                                'direct: for attempt in 0..MAX_ATTEMPTS {
                                    if attempt > 0 {
                                        // Exponential backoff: 1 s → 2 s → 4 s → 8 s, cap 30 s.
                                        // Jitter seeded from the chunk hex de-syncs concurrent
                                        // retries to avoid thundering-herd back-pressure.
                                        let base_ms =
                                            (1000u64 << (attempt - 1)).min(30_000);
                                        let seed = (hex
                                            .as_bytes()
                                            .first()
                                            .copied()
                                            .unwrap_or(0)
                                            as u64)
                                            .wrapping_mul(13)
                                            .wrapping_add(attempt as u64 * 7);
                                        let jitter_ms = base_ms * (seed % 25) / 100;
                                        tokio::time::sleep(
                                            tokio::time::Duration::from_millis(
                                                base_ms + jitter_ms,
                                            ),
                                        )
                                        .await;
                                    }

                                    // Only supply content-length when the signature
                                    // does not already commit to one — see
                                    // `signs_content_length`.
                                    let mut req = direct_client.put(&current_url);
                                    if !signs_content_length(&current_headers) {
                                        req = req.header(
                                            reqwest::header::CONTENT_LENGTH,
                                            chunk_data.len(),
                                        );
                                    }
                                    for [k, v] in &current_headers {
                                        req = req.header(k.as_str(), v.as_str());
                                    }

                                    match req.body(chunk_data.clone()).send().await {
                                        Ok(r) if r.status().is_success() => {
                                            direct_succeeded = true;
                                            break 'direct;
                                        }
                                        Ok(r) => {
                                            let status = r.status().as_u16();
                                            let header_code = r
                                                .headers()
                                                .get("x-amz-error-code")
                                                .or_else(|| {
                                                    r.headers().get("x-ms-error-code")
                                                })
                                                .and_then(|v| v.to_str().ok())
                                                .unwrap_or("")
                                                .to_owned();
                                            let content_type = r
                                                .headers()
                                                .get(reqwest::header::CONTENT_TYPE)
                                                .and_then(|v| v.to_str().ok())
                                                .unwrap_or("")
                                                .to_owned();
                                            let request_id = r
                                                .headers()
                                                .get("x-amz-request-id")
                                                .or_else(|| {
                                                    r.headers().get("x-ms-request-id")
                                                })
                                                .or_else(|| {
                                                    r.headers().get("x-goog-generation")
                                                })
                                                .and_then(|v| v.to_str().ok())
                                                .unwrap_or("")
                                                .to_owned();
                                            let body_bytes =
                                                r.bytes().await.unwrap_or_default();
                                            let body_str = std::str::from_utf8(
                                                &body_bytes
                                                    [..body_bytes.len().min(2048)],
                                            )
                                            .unwrap_or("");
                                            let outcome =
                                                crate::error_class::classify_auto(
                                                    status,
                                                    &current_url,
                                                    &content_type,
                                                    &header_code,
                                                    body_str,
                                                );
                                            let is_last = attempt == MAX_ATTEMPTS - 1;
                                            use crate::error_class::TransferOutcome;
                                            match outcome {
                                                TransferOutcome::Transient => {
                                                    tracing::debug!(
                                                        chunk = %hex,
                                                        attempt,
                                                        status,
                                                        code = %header_code,
                                                        "Direct upload transient error; retrying"
                                                    );
                                                    if is_last {
                                                        tracing::warn!(
                                                            chunk = %hex,
                                                            status,
                                                            code = %header_code,
                                                            body = %body_str,
                                                            "Direct upload failed after {} \
                                                             attempts (transient); falling \
                                                             back to proxy for this chunk",
                                                            MAX_ATTEMPTS
                                                        );
                                                        break 'direct;
                                                    }
                                                    // continue retry loop
                                                }
                                                TransferOutcome::RefreshUrl => {
                                                    if !resigned {
                                                        let resign_url = format!(
                                                            "{}/chunks/upload-urls",
                                                            base_url
                                                        );
                                                        let ids = [hex.clone()];
                                                        let mut sz = std::collections::HashMap::new();
                                                        sz.insert(hex.clone(), chunk_size);
                                                        #[derive(serde::Serialize)]
                                                        struct ResignReq<'a> {
                                                            chunk_ids: &'a [String],
                                                            sizes: &'a std::collections::HashMap<String, u64>,
                                                        }
                                                        let refreshed = client
                                                            .post(&resign_url)
                                                            .json(&ResignReq {
                                                                chunk_ids: &ids,
                                                                sizes: &sz,
                                                            })
                                                            .send()
                                                            .await
                                                            .ok()
                                                            .filter(|r| r.status().is_success());
                                                        if let Some(r) = refreshed
                                                            && let Ok(map) = r
                                                                .json::<std::collections::HashMap<
                                                                    String,
                                                                    Option<PresignedPutInfo>,
                                                                >>()
                                                                .await
                                                                && let Some(Some(np)) = map.get(&hex) {
                                                                    current_url = np.url.clone();
                                                                    current_headers = np.required_headers.clone();
                                                                    resigned = true;
                                                                    tracing::debug!(
                                                                        chunk = %hex,
                                                                        attempt,
                                                                        "Presigned URL refreshed; retrying"
                                                                    );
                                                                    continue 'direct;
                                                                }
                                                        tracing::warn!(
                                                            chunk = %hex,
                                                            attempt,
                                                            status,
                                                            code = %header_code,
                                                            "URL resign failed; falling back to proxy"
                                                        );
                                                    } else {
                                                        tracing::warn!(
                                                            chunk = %hex,
                                                            attempt,
                                                            status,
                                                            code = %header_code,
                                                            "URL already refreshed but still \
                                                             expired; falling back to proxy"
                                                        );
                                                    }
                                                    break 'direct;
                                                }
                                                TransferOutcome::PermanentChunk
                                                | TransferOutcome::PermanentChunkAfterDelay(_) => {
                                                    tracing::warn!(
                                                        chunk = %hex,
                                                        attempt,
                                                        status,
                                                        body = %body_str,
                                                        "Direct upload failed (permanent, \
                                                         chunk-level); falling back to proxy"
                                                    );
                                                    break 'direct;
                                                }
                                                TransferOutcome::PermanentConfig => {
                                                    let fails = perm_fails
                                                        .fetch_add(1, Ordering::Relaxed)
                                                        + 1;
                                                    tracing::warn!(
                                                        chunk = %hex,
                                                        attempt,
                                                        status,
                                                        code = %header_code,
                                                        body = %body_str,
                                                        request_id = %request_id,
                                                        permanent_failures = fails,
                                                        body_len = chunk_size,
                                                        "Direct upload rejected \
                                                         (config/auth error); falling back \
                                                         to proxy. See troubleshooting docs \
                                                         for bucket-policy / presigned-URL \
                                                         setup. If the code is \
                                                         SignatureDoesNotMatch, compare \
                                                         body_len against the \
                                                         Content-Length the server signed \
                                                         (compressed_chunk_len at presign \
                                                         time) before suspecting \
                                                         credentials — they diverge if the \
                                                         chunk was repacked between \
                                                         presign and upload."
                                                    );
                                                    if fails >= 3 {
                                                        tracing::warn!(
                                                            "≥3 chunks rejected with \
                                                             auth/config errors — verify \
                                                             bucket policy and credentials"
                                                        );
                                                    }
                                                    break 'direct;
                                                }
                                            }
                                        }
                                        Err(e) => {
                                            // A TIMEOUT is never routine: the per-request
                                            // budget is 300s, so each one burns five
                                            // minutes, and MAX_ATTEMPTS of them can absorb
                                            // ~25 minutes on a single chunk. If a later
                                            // attempt then succeeds we `break 'direct` and
                                            // nothing above debug is ever emitted, so the
                                            // user is told only "Push successful" after a
                                            // 20-minute wait (measured 2026-08-03: an 8 MiB
                                            // push took 1,188s and printed no diagnostic).
                                            // Warn on timeouts; keep fast connect/body
                                            // errors at debug where they belong.
                                            if e.is_timeout() {
                                                tracing::warn!(
                                                    chunk = %hex,
                                                    attempt,
                                                    err = %e,
                                                    "Direct upload timed out after the \
                                                     per-request budget; retrying (each \
                                                     timeout costs the full budget, so a \
                                                     push that looks merely slow is stalling)"
                                                );
                                            } else {
                                                tracing::debug!(
                                                    chunk = %hex,
                                                    attempt,
                                                    err = %e,
                                                    "Direct upload network error; retrying"
                                                );
                                            }
                                            if attempt == MAX_ATTEMPTS - 1 {
                                                tracing::warn!(
                                                    chunk = %hex,
                                                    err = %e,
                                                    is_connect = e.is_connect(),
                                                    is_timeout = e.is_timeout(),
                                                    is_body = e.is_body(),
                                                    "Direct upload network error after {} \
                                                     attempts; falling back to proxy for \
                                                     this chunk",
                                                    MAX_ATTEMPTS
                                                );
                                            }
                                        }
                                    }
                                }
                            } // end if !direct_succeeded

                            if direct_succeeded {
                                return Ok::<(Oid, u64), anyhow::Error>((chunk_id, chunk_size));
                            }

                            // Proxy path: used when no presigned URL was issued, or all
                            // direct attempts for this chunk were exhausted.
                            let url = format!("{}/chunks/{}", base_url, hex);
                            // One control-plane request per chunk: this is the
                            // path that can outrun the server's rate limiter on
                            // a large push, so wait out 429 rather than failing.
                            let resp = crate::client::send_with_rate_limit_retry(|| {
                                client.put(&url).body(chunk_data.clone()).send()
                            })
                            .await
                            .map_err(|e| {
                                anyhow::anyhow!("Failed to upload chunk {}: {}", chunk_id, e)
                            })?;
                            if !resp.status().is_success() {
                                anyhow::bail!(
                                    "PUT /chunks/{} failed with status: {}",
                                    chunk_id,
                                    resp.status()
                                );
                            }
                            Ok::<(Oid, u64), anyhow::Error>((chunk_id, chunk_size))
                        }
                    })
                    .buffer_unordered(concurrent_uploads);

                while let Some(result) = stream.next().await {
                    let (chunk_id, chunk_bytes) = result?;
                    _pass_a_n += 1;
                    _pass_a_bytes += chunk_bytes;
                    chunks_uploaded += 1;
                    upload_bytes += chunk_bytes;
                    bytes_progress.fetch_add(
                        chunk_manifest_sizes
                            .get(&chunk_id)
                            .copied()
                            .unwrap_or(chunk_bytes),
                        Ordering::Relaxed,
                    );
                }

                // Verify every chunk actually landed; retry stragglers via
                // proxy PUT (handles rare presigned PUT silent failures).
                // On verify endpoint failure, treat ALL as missing and
                // retry via proxy PUT — never silently assume success.
                let still_missing = match self.verify_chunk_uploads(&full_chunk_hexes).await {
                    Ok(m) => m,
                    Err(e) => {
                        tracing::warn!(
                            err = %e,
                            count = full_chunk_hexes.len(),
                            "chunk verify failed; retrying all via proxy PUT"
                        );
                        full_chunk_hexes.clone()
                    }
                };
                if !still_missing.is_empty() {
                    tracing::warn!(
                        count = still_missing.len(),
                        "Retrying unconfirmed chunks via proxy PUT"
                    );
                    let retry_ids: Vec<Oid> = still_missing
                        .iter()
                        .filter_map(|h| Oid::from_hex(h).ok())
                        .collect();
                    let _pass_retry_t = std::time::Instant::now();
                    let mut _pass_retry_n = 0u64;
                    let mut _pass_retry_bytes = 0u64;
                    let mut stream = futures::stream::iter(retry_ids)
                        .map(|chunk_id| {
                            let client = self.client.clone();
                            let base_url = self.base_url.clone();
                            let odb = odb.clone();
                            async move {
                                let chunk_data: bytes::Bytes =
                                    odb.get_compressed_chunk(&chunk_id).await?.into();
                                let chunk_size = chunk_data.len() as u64;
                                let url = format!("{}/chunks/{}", base_url, chunk_id.to_hex());
                                let resp = crate::client::send_with_rate_limit_retry(|| {
                                    client.put(&url).body(chunk_data.clone()).send()
                                })
                                .await
                                .map_err(|e| anyhow::anyhow!("Retry chunk {}: {}", chunk_id, e))?;
                                if !resp.status().is_success() {
                                    anyhow::bail!(
                                        "Retry PUT /chunks/{} failed: {}",
                                        chunk_id,
                                        resp.status()
                                    );
                                }
                                Ok::<u64, anyhow::Error>(chunk_size)
                            }
                        })
                        .buffer_unordered(concurrent_uploads);

                    while let Some(result) = stream.next().await {
                        let chunk_bytes = result?;
                        _pass_retry_n += 1;
                        _pass_retry_bytes += chunk_bytes;
                        chunks_uploaded += 1;
                        upload_bytes += chunk_bytes;
                        // Do NOT fetch_add here: these bytes were already
                        // counted in Pass A. Retry is the same logical work.
                    }
                    let _ = (_pass_retry_n, _pass_retry_bytes);
                }
            }

            // Optional strong verify: decompress + BLAKE3 every chunk server-side.
            // Gated by MEDIAGIT_STRONG_VERIFY=1; endpoint unavailability is non-fatal.
            // Runs in pack mode too — the server consults the pack index on a
            // loose miss, so packed chunks are pack-valid to verify.
            if std::env::var("MEDIAGIT_STRONG_VERIFY").as_deref() == Ok("1")
                && !full_chunks.is_empty()
            {
                let hexes: Vec<String> = full_chunks.iter().map(|c| c.to_hex()).collect();
                tracing::debug!(count = hexes.len(), "Running strong chunk integrity verify");
                match self.strong_verify_chunks(&hexes, false).await {
                    Ok(invalid) if !invalid.is_empty() => {
                        anyhow::bail!(
                            "Strong verify found {} chunk(s) with corrupted content: {:?}",
                            invalid.len(),
                            &invalid[..invalid.len().min(5)]
                        );
                    }
                    Ok(_) => tracing::debug!("Strong verify passed"),
                    Err(e) => {
                        tracing::warn!(err = %e, "Strong verify endpoint unavailable; skipping")
                    }
                }
            }

            // ── Pass B: verify out-of-push bases actually exist server-side;
            // any delta whose base is still missing must degrade to a full
            // upload so the server never ends up with an orphaned delta.
            let out_of_push_base_hexes: Vec<String> = {
                let mut seen = std::collections::HashSet::new();
                delta_chunks
                    .iter()
                    .map(|(_, b, _)| *b)
                    .filter(|b| !bases_landing_this_push.contains(b))
                    .filter(|b| seen.insert(*b))
                    .map(|b| b.to_hex())
                    .collect()
            };

            // Reuse the combined existence result from the single fused
            // POST above instead of issuing a second /chunks/check round-trip.
            // `combined_missing` already covers every speculative base hex
            // we discovered before Pass A.
            let still_missing_bases: std::collections::HashSet<Oid> = out_of_push_base_hexes
                .iter()
                .filter(|h| combined_missing.contains(*h))
                .filter_map(|h| Oid::from_hex(h).ok())
                .collect();

            let (uploadable_deltas, degraded_to_full): (Vec<_>, Vec<_>) = delta_chunks
                .into_iter()
                .partition(|(_, base, _)| !still_missing_bases.contains(base));

            if !degraded_to_full.is_empty() {
                tracing::warn!(
                    oid = %oid,
                    count = degraded_to_full.len(),
                    "Delta base not on server; degrading to full-chunk upload"
                );
                let degraded_ids: Vec<Oid> =
                    degraded_to_full.into_iter().map(|(c, _, _)| c).collect();
                let _pass_deg_t = std::time::Instant::now();
                let mut _pass_deg_n = 0u64;
                let mut _pass_deg_bytes = 0u64;
                let mut stream = futures::stream::iter(degraded_ids)
                    .map(|chunk_id| {
                        let client = self.client.clone();
                        let base_url = self.base_url.clone();
                        let odb = odb.clone();
                        async move {
                            let chunk_data: bytes::Bytes =
                                odb.get_compressed_chunk(&chunk_id).await?.into();
                            let chunk_size = chunk_data.len() as u64;
                            let url = format!("{}/chunks/{}", base_url, chunk_id.to_hex());
                            let resp = crate::client::send_with_rate_limit_retry(|| {
                                client.put(&url).body(chunk_data.clone()).send()
                            })
                            .await
                            .map_err(|e| {
                                anyhow::anyhow!("Failed to upload chunk {}: {}", chunk_id, e)
                            })?;
                            if !resp.status().is_success() {
                                anyhow::bail!(
                                    "PUT /chunks/{} failed with status: {}",
                                    chunk_id,
                                    resp.status()
                                );
                            }
                            Ok::<(Oid, u64), anyhow::Error>((chunk_id, chunk_size))
                        }
                    })
                    .buffer_unordered(concurrent_uploads);

                while let Some(result) = stream.next().await {
                    let (chunk_id, chunk_bytes) = result?;
                    _pass_deg_n += 1;
                    _pass_deg_bytes += chunk_bytes;
                    chunks_uploaded += 1;
                    upload_bytes += chunk_bytes;
                    bytes_progress.fetch_add(
                        chunk_manifest_sizes
                            .get(&chunk_id)
                            .copied()
                            .unwrap_or(chunk_bytes),
                        Ordering::Relaxed,
                    );
                }
                let _ = (_pass_deg_n, _pass_deg_bytes);
            }

            // ── Pass C: ship deltas as deltas (verbatim; no rematerialize) ─
            if !uploadable_deltas.is_empty() {
                let _pass_c_t = std::time::Instant::now();
                let mut _pass_c_n = 0u64;
                let mut _pass_c_bytes = 0u64;
                let mut stream = futures::stream::iter(uploadable_deltas)
                    .map(|(chunk_id, base_id, delta_bytes)| {
                        let client = self.client.clone();
                        let base_url = self.base_url.clone();
                        async move {
                            let delta_bytes: bytes::Bytes = delta_bytes.into();
                            let delta_size = delta_bytes.len() as u64;
                            let url = format!("{}/chunk-deltas/{}", base_url, chunk_id.to_hex());
                            let resp = crate::client::send_with_rate_limit_retry(|| {
                                client
                                    .put(&url)
                                    .header("x-mediagit-delta-base", base_id.to_hex())
                                    .body(delta_bytes.clone())
                                    .send()
                            })
                            .await
                            .map_err(|e| {
                                anyhow::anyhow!("Failed to upload chunk-delta {}: {}", chunk_id, e)
                            })?;
                            if !resp.status().is_success() {
                                anyhow::bail!(
                                    "PUT /chunk-deltas/{} failed with status: {}",
                                    chunk_id,
                                    resp.status()
                                );
                            }
                            Ok::<(Oid, u64), anyhow::Error>((chunk_id, delta_size))
                        }
                    })
                    .buffer_unordered(concurrent_uploads);

                while let Some(result) = stream.next().await {
                    let (chunk_id, chunk_bytes) = result?;
                    _pass_c_n += 1;
                    _pass_c_bytes += chunk_bytes;
                    chunks_uploaded += 1;
                    upload_bytes += chunk_bytes;
                    // Use manifest c.size so numerator matches the denominator
                    // reservation (upload_bytes_total counts manifest sizes).
                    bytes_progress.fetch_add(
                        chunk_manifest_sizes
                            .get(&chunk_id)
                            .copied()
                            .unwrap_or(chunk_bytes),
                        Ordering::Relaxed,
                    );
                }
                let _ = (_pass_c_n, _pass_c_bytes);
            }
        }

        // Upload manifest last (ensures all chunks exist first)
        let manifest_data = manifest
            .to_bytes()
            .context("Failed to serialize manifest")?;
        self.upload_manifest(oid, &manifest_data).await?;

        tracing::debug!(oid = %oid, "Manifest uploaded");

        Ok((chunks_uploaded, upload_bytes, upload_bytes_total))
    }

    /// Upload all chunks for a chunked object with parallel uploads
    ///
    /// Uses 8 concurrent uploads for optimal throughput (>100MB/s target)
    pub async fn upload_chunked_objects<F>(
        &self,
        odb: &ObjectDatabase,
        chunked_oids: &[Oid],
        mut on_progress: F,
    ) -> Result<(usize, u64)>
    where
        F: FnMut(u64, u64),
    {
        use futures::stream::StreamExt;

        if chunked_oids.is_empty() {
            return Ok((0, 0));
        }

        let mut total_chunks_uploaded = 0;
        // RP-1: chunk payload is the overwhelming majority of a media push, and
        // it was never counted. `bytes_uploaded` got the metadata pack's size
        // and nothing else, which is how a 15.53 GiB push reported "↑ 2.71 KiB".
        // Counted in wire bytes (compressed chunk / pack bytes) to match the
        // per-chunk and pack paths, and because a rate derived from logical
        // bytes can exceed link capacity — the same unit error behind the
        // "747 MiB/s" reading.
        let mut total_bytes_uploaded: u64 = 0;
        // Concurrency for parallel chunk uploads. buffer_unordered keeps at
        // most N futures active. Default 32 measured 37% faster than 16 on a
        // 2-Mbps upstream to Azure West EU (561s -> 353s for 150 MB cold
        // push). c=64 regressed to 399s on the same link — too many parallel
        // TLS handshakes / TCP slow-starts contend. 32 is the sweet spot.
        // Memory cost: ~chunk_size × N peak buffered. At ~4 MB/chunk × 32 =
        // ~128 MB transient peak per push session.
        // Override via env when bandwidth or backend tolerates more.
        // Priority: env var override > config override (set via
        // `with_concurrent_uploads`) > internal default (32).
        let concurrent_uploads: usize = std::env::var("MEDIAGIT_UPLOAD_CONCURRENCY")
            .ok()
            .and_then(|s| s.parse().ok())
            .filter(|n: &usize| *n > 0)
            .or(self.concurrent_uploads)
            .unwrap_or(32);

        // B2: cross-object push pipeline. Default ON — parity matrix 2/2 on MinIO/AWS/Azure (2026-05-22).
        // Set MEDIAGIT_PUSH_PIPELINE=0 to revert to sequential per-object upload.
        let push_pipeline = std::env::var("MEDIAGIT_PUSH_PIPELINE")
            .as_deref()
            .unwrap_or("1")
            == "1";
        let push_object_concurrency: usize = std::env::var("MEDIAGIT_PUSH_OBJECT_CONCURRENCY")
            .ok()
            .and_then(|s| s.parse().ok())
            .filter(|n: &usize| *n > 0)
            .unwrap_or(8);

        // Target 64 total in-flight S3 PUTs (AWS S3 ap-south-1 WAN sweet spot,
        // see project_throughput_aws_wanbound). Formula is monotone in
        // push_object_concurrency — as outer concurrency grows, per-object shrinks,
        // total stays ≈ TOTAL_IN_FLIGHT_TARGET. .min(concurrent_uploads) honours the
        // user's upload-concurrency intent; .max(4) prevents serialization at absurd
        // outer concurrency values. Override with MEDIAGIT_PUSH_CHUNK_CONCURRENCY.
        const TOTAL_IN_FLIGHT_TARGET: usize = 64;
        let per_obj_concurrent = if push_pipeline && push_object_concurrency > 1 {
            std::env::var("MEDIAGIT_PUSH_CHUNK_CONCURRENCY")
                .ok()
                .and_then(|s| s.parse::<usize>().ok())
                .filter(|n| *n > 0)
                .unwrap_or_else(|| {
                    (TOTAL_IN_FLIGHT_TARGET / push_object_concurrency)
                        .max(4)
                        .min(concurrent_uploads)
                })
        } else {
            concurrent_uploads
        };

        let mut bytes_done: u64 = 0;
        let mut bytes_total: u64 = 0;
        on_progress(0, 0);
        let _upload_bench = crate::bench::maybe_start("upload", concurrent_uploads);

        if push_pipeline {
            use futures::stream::StreamExt;
            // bytes_progress: per-chunk numerator, updated as each chunk upload completes.
            // bytes_total_progress: per-object denominator, published immediately after each
            // object's chunk-existence check so the display reflects all in-flight objects,
            // not only completed ones. Without this separate atomic, bytes_total only updates
            // when push_one_object returns, causing 8-in-flight pipeline to show e.g.
            // "27 MiB / 699 B" because only 1 of 8 objects has completed.
            let bytes_progress = Arc::new(AtomicU64::new(0));
            let bytes_total_progress = Arc::new(AtomicU64::new(0));
            let mut obj_stream = futures::stream::iter(chunked_oids.iter())
                .map(|oid| {
                    let odb = odb.clone();
                    let bp = bytes_progress.clone();
                    let btp = bytes_total_progress.clone();
                    let bench = _upload_bench.clone();
                    async move {
                        self.push_one_object(oid, &odb, per_obj_concurrent, bp, btp, bench.as_ref())
                            .await
                    }
                })
                .buffer_unordered(push_object_concurrency);

            // Refresh the progress display every 500 ms even when no object has
            // completed yet.  Without this, a slow WAN push with many chunks per
            // object shows 0 B/s for minutes even though uploads are in-flight.
            let mut ticker = tokio::time::interval(std::time::Duration::from_millis(500));
            ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                tokio::select! {
                    biased;
                    result = obj_stream.next() => {
                        match result {
                            None => break,
                            Some(r) => {
                                let (chunks, bytes_up, _bytes_total_delta) = r?;
                                total_chunks_uploaded += chunks as usize;
                                total_bytes_uploaded += bytes_up;
                                let done = bytes_progress.load(Ordering::Relaxed);
                                let total = bytes_total_progress.load(Ordering::Relaxed);
                                on_progress(done, total);
                                debug_assert!(
                                    done <= total,
                                    "bytes_progress overshot bytes_total_progress — unit mismatch or double-count regression"
                                );
                            }
                        }
                    }
                    _ = ticker.tick() => {
                        on_progress(
                            bytes_progress.load(Ordering::Relaxed),
                            bytes_total_progress.load(Ordering::Relaxed),
                        );
                    }
                }
            }
            if let Some(b) = &_upload_bench {
                b.summary();
            }
            return Ok((total_chunks_uploaded, total_bytes_uploaded));
        }

        for oid in chunked_oids.iter() {
            // Get manifest for this object
            let manifest = match odb.get_chunk_manifest(oid).await? {
                Some(m) => m,
                None => {
                    tracing::warn!(oid = %oid, "No manifest found for chunked object");
                    continue;
                }
            };

            // Get all chunk IDs
            let chunk_ids: Vec<String> = manifest.chunks.iter().map(|c| c.id.to_hex()).collect();

            // Pre-discover local delta info for every manifest chunk. This is
            // a local DB lookup per chunk — cheap compared to a network RTT —
            // and lets us fold the chunk-existence check and the delta-base
            // existence check into ONE POST (down from two sequential RTTs).
            // We cache the delta payloads here so Pass A/C don't re-read them.
            let mut local_delta_lookup: std::collections::HashMap<Oid, (Oid, Vec<u8>)> =
                std::collections::HashMap::new();
            let mut speculative_base_hexes: std::collections::HashSet<String> =
                std::collections::HashSet::new();
            for c in &manifest.chunks {
                if let Some((base_id, delta_bytes)) = odb.get_local_chunk_delta_raw(&c.id).await? {
                    speculative_base_hexes.insert(base_id.to_hex());
                    local_delta_lookup.insert(c.id, (base_id, delta_bytes));
                }
            }

            // Combined existence check: manifest chunks + speculative delta bases.
            let mut combined_check: Vec<String> = chunk_ids.clone();
            let chunk_id_set: std::collections::HashSet<&String> = chunk_ids.iter().collect();
            for base_hex in &speculative_base_hexes {
                if !chunk_id_set.contains(base_hex) {
                    combined_check.push(base_hex.clone());
                }
            }
            let combined_missing: std::collections::HashSet<String> = self
                .check_chunks_exist(&combined_check)
                .await?
                .into_iter()
                .collect();

            // Missing manifest chunks = subset of combined_missing.
            let missing_chunks: Vec<String> = chunk_ids
                .iter()
                .filter(|h| combined_missing.contains(*h))
                .cloned()
                .collect();

            if missing_chunks.is_empty() {
                tracing::debug!(oid = %oid, "All chunks already exist on remote");
            } else {
                tracing::info!(
                    oid = %oid,
                    missing = missing_chunks.len(),
                    "Uploading missing chunks"
                );

                let missing_set: std::collections::HashSet<String> =
                    missing_chunks.into_iter().collect();

                // Update total with this object's missing chunk bytes for accurate ETA
                bytes_total += manifest
                    .chunks
                    .iter()
                    .filter(|c| missing_set.contains(&c.id.to_hex()))
                    .map(|c| c.size as u64)
                    .sum::<u64>();
                on_progress(bytes_done, bytes_total);

                let chunks_to_upload: Vec<Oid> = manifest
                    .chunks
                    .iter()
                    .filter(|c| missing_set.contains(&c.id.to_hex()))
                    .map(|c| c.id)
                    .collect();

                // Partition into locally-delta chunks (ship as deltas) and
                // plain full chunks. Reuses the pre-computed local_delta_lookup,
                // avoiding a second pass over odb.get_local_chunk_delta_raw.
                let mut full_chunks: Vec<Oid> = Vec::new();
                let mut delta_chunks: Vec<(Oid, Oid, Vec<u8>)> = Vec::new();
                for chunk_id in &chunks_to_upload {
                    match local_delta_lookup.remove(chunk_id) {
                        Some((base_id, delta_bytes)) => {
                            delta_chunks.push((*chunk_id, base_id, delta_bytes));
                        }
                        None => full_chunks.push(*chunk_id),
                    }
                }

                // A base is considered "landing this push" if it's any chunk in
                // chunks_to_upload — regardless of whether it lands as a full or
                // as a delta. A chunk-delta can be a base for another chunk-delta
                // (depth-2 chain); the server reconstructs the chain from the
                // .meta sidecars. Restricting this set to `full_chunks` would
                // force depth-2 chains to degrade to full uploads, silently
                // shrinking clone-side savings.
                let bases_landing_this_push: std::collections::HashSet<Oid> =
                    chunks_to_upload.iter().copied().collect();

                // ── Pass A: full chunks (must land before any delta whose
                // base is in this push) ──────────────────────────────────
                if !full_chunks.is_empty() {
                    // Request presigned PUT URLs from the server.  Cloud
                    // backends return signed bucket URLs; Local/Mock return
                    // null → falls through to the server-proxy PUT path.
                    let full_chunk_hexes: Vec<String> =
                        full_chunks.iter().map(|c| c.to_hex()).collect();
                    // Bind the presigned URL's Content-Length to the actual compressed
                    // on-disk size (what will be PUT), not the manifest's uncompressed
                    // size — a mismatch there causes a 403 SignatureDoesNotMatch on
                    // compressible content. `None` (delta/repacked chunk) maps to 0,
                    // matching the server's "unbound URL" contract.
                    let chunk_sizes: std::collections::HashMap<String, u64> = {
                        let lens = futures::future::join_all(
                            full_chunks.iter().map(|id| odb.compressed_chunk_len(id)),
                        )
                        .await;
                        full_chunks
                            .iter()
                            .zip(lens)
                            .map(|(id, len)| (id.to_hex(), len.unwrap_or(0)))
                            .collect()
                    };
                    let presigned_urls = std::sync::Arc::new(
                        self.request_chunk_upload_urls(&full_chunk_hexes, &chunk_sizes)
                            .await,
                    );

                    // Per-push counter for permanent-config failures (auth, bucket policy).
                    // If ≥3 distinct chunks are rejected with config-level errors the
                    // operator is warned; individual chunks still fall back to proxy
                    // independently — the batch is never poisoned as a whole.
                    let permanent_config_failures = std::sync::Arc::new(AtomicUsize::new(0));

                    // Dedicated direct-upload HTTP client with a short pool-idle window.
                    // S3 edges close idle keep-alive TCP connections at ~20 s; reusing a
                    // stale socket yields a RequestTimeout 400.  A 15 s idle cap ensures
                    // every retry opens a fresh socket before S3 can half-close it.
                    // The per-request timeout (5 min) caps stalled uploads on lossy WAN;
                    // a 64 MiB chunk at 0.2 MiB/s takes ~320 s so this is generous.
                    // Data-plane client for presigned PUT uploads.
                    // Kept on HTTP/1.1 intentionally: parallel TCP sockets give
                    // better raw throughput for large bodies than h2 multiplexing
                    // on a single TCP connection (parallel cwnd > one congestion
                    // window). Pool size via MEDIAGIT_HTTP_POOL_MAX (see http_pool_max()).
                    crate::ensure_crypto_provider();
                    let direct_client = reqwest::Client::builder()
                        .pool_idle_timeout(std::time::Duration::from_secs(60))
                        .pool_max_idle_per_host(http_pool_max())
                        .tcp_keepalive(std::time::Duration::from_secs(45))
                        .tcp_nodelay(true)
                        .timeout(std::time::Duration::from_secs(300))
                        .http1_only()
                        .build()
                        .unwrap_or_else(|_| reqwest::Client::new());

                    let _pass_a_t = std::time::Instant::now();
                    let mut _pass_a_n = 0u64;
                    let mut _pass_a_bytes = 0u64;
                    let mut stream = futures::stream::iter(full_chunks.clone())
                        .map(|chunk_id| {
                            let client = self.client.clone();
                            let direct_client = direct_client.clone();
                            let base_url = self.base_url.clone();
                            let odb = odb.clone();
                            let presigned = presigned_urls.clone();
                            let perm_fails = permanent_config_failures.clone();
                            async move {
                                // B5: convert to Bytes immediately so retry clones are O(1) refcount bumps.
                                let chunk_data: bytes::Bytes =
                                    odb.get_compressed_chunk(&chunk_id).await?.into();
                                let chunk_size = chunk_data.len() as u64;
                                let hex = chunk_id.to_hex();

                                // Phase 2: MPU path — active when MEDIAGIT_STAGED_UPLOAD=1 and
                                // chunk_size >= MEDIAGIT_MPU_THRESHOLD_BYTES (default 16 MiB).
                                // Single-PUT is cheaper for 8–16 MiB chunks (no part overhead).
                                // On any failure falls through to the single-PUT path below.
                                let mut direct_succeeded = false;
                                {
                                    let use_mpu = std::env::var("MEDIAGIT_STAGED_UPLOAD")
                                        .as_deref()
                                        != Ok("0");
                                    let mpu_threshold: u64 =
                                        std::env::var("MEDIAGIT_MPU_THRESHOLD_BYTES")
                                            .ok()
                                            .and_then(|v| v.parse().ok())
                                            .unwrap_or(16 * 1024 * 1024);
                                    if use_mpu
                                        && chunk_size >= mpu_threshold
                                        && upload_chunk_mpu(
                                            &client,
                                            &direct_client,
                                            &base_url,
                                            &hex,
                                            &chunk_data,
                                        )
                                        .await
                                    {
                                        direct_succeeded = true;
                                    }
                                }
                                if !direct_succeeded
                                && let Some(Some(purl)) = presigned.get(&hex) {
                                    let mut current_url = purl.url.clone();
                                    let mut current_headers = purl.required_headers.clone();
                                    let mut resigned = false;
                                    const MAX_ATTEMPTS: u32 = 5;
                                    'direct: for attempt in 0..MAX_ATTEMPTS {
                                        if attempt > 0 {
                                            // Exponential backoff: 1 s → 2 s → 4 s → 8 s, cap 30 s.
                                            // Jitter seeded from the chunk hex de-syncs concurrent
                                            // retries to avoid thundering-herd back-pressure.
                                            let base_ms =
                                                (1000u64 << (attempt - 1)).min(30_000);
                                            let seed = (hex
                                                .as_bytes()
                                                .first()
                                                .copied()
                                                .unwrap_or(0)
                                                as u64)
                                                .wrapping_mul(13)
                                                .wrapping_add(attempt as u64 * 7);
                                            let jitter_ms = base_ms * (seed % 25) / 100;
                                            tokio::time::sleep(
                                                tokio::time::Duration::from_millis(
                                                    base_ms + jitter_ms,
                                                ),
                                            )
                                            .await;
                                        }

                                        // Only supply content-length when the
                                        // signature does not already commit to one
                                        // — see `signs_content_length`.
                                        let mut req = direct_client.put(&current_url);
                                        if !signs_content_length(&current_headers) {
                                            req = req.header(
                                                reqwest::header::CONTENT_LENGTH,
                                                chunk_data.len(),
                                            );
                                        }
                                        for [k, v] in &current_headers {
                                            req = req.header(k.as_str(), v.as_str());
                                        }

                                        match req.body(chunk_data.clone()).send().await {
                                            Ok(r) if r.status().is_success() => {
                                                direct_succeeded = true;
                                                break 'direct;
                                            }
                                            Ok(r) => {
                                                let status = r.status().as_u16();
                                                let header_code = r
                                                    .headers()
                                                    .get("x-amz-error-code")
                                                    .or_else(|| {
                                                        r.headers().get("x-ms-error-code")
                                                    })
                                                    .and_then(|v| v.to_str().ok())
                                                    .unwrap_or("")
                                                    .to_owned();
                                                let content_type = r
                                                    .headers()
                                                    .get(reqwest::header::CONTENT_TYPE)
                                                    .and_then(|v| v.to_str().ok())
                                                    .unwrap_or("")
                                                    .to_owned();
                                                let request_id = r
                                                    .headers()
                                                    .get("x-amz-request-id")
                                                    .or_else(|| {
                                                        r.headers().get("x-ms-request-id")
                                                    })
                                                    .or_else(|| {
                                                        r.headers().get("x-goog-generation")
                                                    })
                                                    .and_then(|v| v.to_str().ok())
                                                    .unwrap_or("")
                                                    .to_owned();
                                                let body_bytes =
                                                    r.bytes().await.unwrap_or_default();
                                                let body_str = std::str::from_utf8(
                                                    &body_bytes
                                                        [..body_bytes.len().min(2048)],
                                                )
                                                .unwrap_or("");
                                                let outcome =
                                                    crate::error_class::classify_auto(
                                                        status,
                                                        &current_url,
                                                        &content_type,
                                                        &header_code,
                                                        body_str,
                                                    );
                                                let is_last = attempt == MAX_ATTEMPTS - 1;
                                                use crate::error_class::TransferOutcome;
                                                match outcome {
                                                    TransferOutcome::Transient => {
                                                        tracing::debug!(
                                                            chunk = %hex,
                                                            attempt,
                                                            status,
                                                            code = %header_code,
                                                            "Direct upload transient error; retrying"
                                                        );
                                                        if is_last {
                                                            tracing::warn!(
                                                                chunk = %hex,
                                                                status,
                                                                code = %header_code,
                                                                body = %body_str,
                                                                "Direct upload failed after {} \
                                                                 attempts (transient); falling \
                                                                 back to proxy for this chunk",
                                                                MAX_ATTEMPTS
                                                            );
                                                            break 'direct;
                                                        }
                                                        // continue retry loop
                                                    }
                                                    TransferOutcome::RefreshUrl => {
                                                        if !resigned {
                                                            let resign_url = format!(
                                                                "{}/chunks/upload-urls",
                                                                base_url
                                                            );
                                                            let ids = [hex.clone()];
                                                            let mut sz = std::collections::HashMap::new();
                                                            sz.insert(hex.clone(), chunk_size);
                                                            #[derive(serde::Serialize)]
                                                            struct ResignReq<'a> {
                                                                chunk_ids: &'a [String],
                                                                sizes: &'a std::collections::HashMap<String, u64>,
                                                            }
                                                            let refreshed = client
                                                                .post(&resign_url)
                                                                .json(&ResignReq {
                                                                    chunk_ids: &ids,
                                                                    sizes: &sz,
                                                                })
                                                                .send()
                                                                .await
                                                                .ok()
                                                                .filter(|r| r.status().is_success());
                                                            if let Some(r) = refreshed
                                                                && let Ok(map) = r
                                                                    .json::<std::collections::HashMap<
                                                                        String,
                                                                        Option<PresignedPutInfo>,
                                                                    >>()
                                                                    .await
                                                                    && let Some(Some(np)) = map.get(&hex) {
                                                                        current_url = np.url.clone();
                                                                        current_headers = np.required_headers.clone();
                                                                        resigned = true;
                                                                        tracing::debug!(
                                                                            chunk = %hex,
                                                                            attempt,
                                                                            "Presigned URL refreshed; retrying"
                                                                        );
                                                                        continue 'direct;
                                                                    }
                                                            tracing::warn!(
                                                                chunk = %hex,
                                                                attempt,
                                                                status,
                                                                code = %header_code,
                                                                "URL resign failed; falling back to proxy"
                                                            );
                                                        } else {
                                                            tracing::warn!(
                                                                chunk = %hex,
                                                                attempt,
                                                                status,
                                                                code = %header_code,
                                                                "URL already refreshed but still \
                                                                 expired; falling back to proxy"
                                                            );
                                                        }
                                                        break 'direct;
                                                    }
                                                    TransferOutcome::PermanentChunk
                                                    | TransferOutcome::PermanentChunkAfterDelay(_) => {
                                                        tracing::warn!(
                                                            chunk = %hex,
                                                            attempt,
                                                            status,
                                                            body = %body_str,
                                                            "Direct upload failed (permanent, \
                                                             chunk-level); falling back to proxy"
                                                        );
                                                        break 'direct;
                                                    }
                                                    TransferOutcome::PermanentConfig => {
                                                        let fails = perm_fails
                                                            .fetch_add(1, Ordering::Relaxed)
                                                            + 1;
                                                        tracing::warn!(
                                                            chunk = %hex,
                                                            attempt,
                                                            status,
                                                            code = %header_code,
                                                            body = %body_str,
                                                            request_id = %request_id,
                                                            permanent_failures = fails,
                                                            "Direct upload rejected \
                                                             (config/auth error); falling back \
                                                             to proxy. See troubleshooting docs \
                                                             for bucket-policy / presigned-URL \
                                                             setup."
                                                        );
                                                        if fails >= 3 {
                                                            tracing::warn!(
                                                                "≥3 chunks rejected with \
                                                                 auth/config errors — verify \
                                                                 bucket policy and credentials"
                                                            );
                                                        }
                                                        break 'direct;
                                                    }
                                                }
                                            }
                                            Err(e) => {
                                                // Same silent-stall gap as the first direct
                                                // -upload loop above — see its comment. A
                                                // timeout costs the full per-request budget,
                                                // so it must not sit at debug.
                                                if e.is_timeout() {
                                                    tracing::warn!(
                                                        chunk = %hex,
                                                        attempt,
                                                        err = %e,
                                                        "Direct upload timed out after the \
                                                         per-request budget; retrying (each \
                                                         timeout costs the full budget, so a \
                                                         push that looks merely slow is \
                                                         stalling)"
                                                    );
                                                } else {
                                                    tracing::debug!(
                                                        chunk = %hex,
                                                        attempt,
                                                        err = %e,
                                                        "Direct upload network error; retrying"
                                                    );
                                                }
                                                if attempt == MAX_ATTEMPTS - 1 {
                                                    tracing::warn!(
                                                        chunk = %hex,
                                                        err = %e,
                                                        is_connect = e.is_connect(),
                                                        is_timeout = e.is_timeout(),
                                                        is_body = e.is_body(),
                                                        "Direct upload network error after {} \
                                                         attempts; falling back to proxy for \
                                                         this chunk",
                                                        MAX_ATTEMPTS
                                                    );
                                                }
                                            }
                                        }
                                    }
                                } // end if !direct_succeeded

                                if direct_succeeded {
                                    return Ok::<u64, anyhow::Error>(chunk_size);
                                }

                                // Proxy path: used when no presigned URL was issued, or all
                                // direct attempts for this chunk were exhausted. One
                                // control-plane request per chunk, so wait out 429
                                // rather than failing (mirrors the pipelined path).
                                let url = format!("{}/chunks/{}", base_url, hex);
                                let resp = crate::client::send_with_rate_limit_retry(|| {
                                    client.put(&url).body(chunk_data.clone()).send()
                                })
                                .await
                                .map_err(|e| {
                                    anyhow::anyhow!(
                                        "Failed to upload chunk {}: {}",
                                        chunk_id,
                                        e
                                    )
                                })?;
                                if !resp.status().is_success() {
                                    anyhow::bail!(
                                        "PUT /chunks/{} failed with status: {}",
                                        chunk_id,
                                        resp.status()
                                    );
                                }
                                Ok::<u64, anyhow::Error>(chunk_size)
                            }
                        })
                        .buffer_unordered(concurrent_uploads);

                    while let Some(result) = stream.next().await {
                        let chunk_bytes = result?;
                        _pass_a_n += 1;
                        _pass_a_bytes += chunk_bytes;
                        total_chunks_uploaded += 1;
                        bytes_done += chunk_bytes;
                        on_progress(bytes_done, bytes_total);
                    }
                    if let Some(b) = &_upload_bench {
                        b.record_batch(_pass_a_n, _pass_a_bytes, _pass_a_t.elapsed());
                    }

                    // Verify every chunk actually landed; retry stragglers via
                    // proxy PUT (handles rare presigned PUT silent failures).
                    // On verify endpoint failure, treat ALL as missing and
                    // retry via proxy PUT — never silently assume success.
                    let still_missing = match self.verify_chunk_uploads(&full_chunk_hexes).await {
                        Ok(m) => m,
                        Err(e) => {
                            tracing::warn!(
                                err = %e,
                                count = full_chunk_hexes.len(),
                                "chunk verify failed; retrying all via proxy PUT"
                            );
                            full_chunk_hexes.clone()
                        }
                    };
                    if !still_missing.is_empty() {
                        tracing::warn!(
                            count = still_missing.len(),
                            "Retrying unconfirmed chunks via proxy PUT"
                        );
                        let retry_ids: Vec<Oid> = still_missing
                            .iter()
                            .filter_map(|h| Oid::from_hex(h).ok())
                            .collect();
                        let _pass_retry_t = std::time::Instant::now();
                        let mut _pass_retry_n = 0u64;
                        let mut _pass_retry_bytes = 0u64;
                        let mut stream = futures::stream::iter(retry_ids)
                            .map(|chunk_id| {
                                let client = self.client.clone();
                                let base_url = self.base_url.clone();
                                let odb = odb.clone();
                                async move {
                                    let chunk_data: bytes::Bytes =
                                        odb.get_compressed_chunk(&chunk_id).await?.into();
                                    let chunk_size = chunk_data.len() as u64;
                                    let url = format!("{}/chunks/{}", base_url, chunk_id.to_hex());
                                    let resp = crate::client::send_with_rate_limit_retry(|| {
                                        client.put(&url).body(chunk_data.clone()).send()
                                    })
                                    .await
                                    .map_err(|e| {
                                        anyhow::anyhow!("Retry chunk {}: {}", chunk_id, e)
                                    })?;
                                    if !resp.status().is_success() {
                                        anyhow::bail!(
                                            "Retry PUT /chunks/{} failed: {}",
                                            chunk_id,
                                            resp.status()
                                        );
                                    }
                                    Ok::<u64, anyhow::Error>(chunk_size)
                                }
                            })
                            .buffer_unordered(concurrent_uploads);

                        while let Some(result) = stream.next().await {
                            let chunk_bytes = result?;
                            _pass_retry_n += 1;
                            _pass_retry_bytes += chunk_bytes;
                            total_chunks_uploaded += 1;
                            bytes_done += chunk_bytes;
                            on_progress(bytes_done, bytes_total);
                        }
                        if let Some(b) = &_upload_bench {
                            b.record_batch(
                                _pass_retry_n,
                                _pass_retry_bytes,
                                _pass_retry_t.elapsed(),
                            );
                        }
                    }
                }

                // ── Pass B: verify out-of-push bases actually exist server-side;
                // any delta whose base is still missing must degrade to a full
                // upload so the server never ends up with an orphaned delta.
                let out_of_push_base_hexes: Vec<String> = {
                    let mut seen = std::collections::HashSet::new();
                    delta_chunks
                        .iter()
                        .map(|(_, b, _)| *b)
                        .filter(|b| !bases_landing_this_push.contains(b))
                        .filter(|b| seen.insert(*b))
                        .map(|b| b.to_hex())
                        .collect()
                };

                // Reuse the combined existence result from the single fused
                // POST above instead of issuing a second /chunks/check round-trip.
                // `combined_missing` already covers every speculative base hex
                // we discovered before Pass A.
                let still_missing_bases: std::collections::HashSet<Oid> = out_of_push_base_hexes
                    .iter()
                    .filter(|h| combined_missing.contains(*h))
                    .filter_map(|h| Oid::from_hex(h).ok())
                    .collect();

                let (uploadable_deltas, degraded_to_full): (Vec<_>, Vec<_>) = delta_chunks
                    .into_iter()
                    .partition(|(_, base, _)| !still_missing_bases.contains(base));

                if !degraded_to_full.is_empty() {
                    tracing::warn!(
                        oid = %oid,
                        count = degraded_to_full.len(),
                        "Delta base not on server; degrading to full-chunk upload"
                    );
                    let degraded_ids: Vec<Oid> =
                        degraded_to_full.into_iter().map(|(c, _, _)| c).collect();
                    let _pass_deg_t = std::time::Instant::now();
                    let mut _pass_deg_n = 0u64;
                    let mut _pass_deg_bytes = 0u64;
                    let mut stream = futures::stream::iter(degraded_ids)
                        .map(|chunk_id| {
                            let client = self.client.clone();
                            let base_url = self.base_url.clone();
                            let odb = odb.clone();
                            async move {
                                let chunk_data: bytes::Bytes =
                                    odb.get_compressed_chunk(&chunk_id).await?.into();
                                let chunk_size = chunk_data.len() as u64;
                                let url = format!("{}/chunks/{}", base_url, chunk_id.to_hex());
                                let resp = crate::client::send_with_rate_limit_retry(|| {
                                    client.put(&url).body(chunk_data.clone()).send()
                                })
                                .await
                                .map_err(|e| {
                                    anyhow::anyhow!("Failed to upload chunk {}: {}", chunk_id, e)
                                })?;
                                if !resp.status().is_success() {
                                    anyhow::bail!(
                                        "PUT /chunks/{} failed with status: {}",
                                        chunk_id,
                                        resp.status()
                                    );
                                }
                                Ok::<u64, anyhow::Error>(chunk_size)
                            }
                        })
                        .buffer_unordered(concurrent_uploads);

                    while let Some(result) = stream.next().await {
                        let chunk_bytes = result?;
                        _pass_deg_n += 1;
                        _pass_deg_bytes += chunk_bytes;
                        total_chunks_uploaded += 1;
                        bytes_done += chunk_bytes;
                        on_progress(bytes_done, bytes_total);
                    }
                    if let Some(b) = &_upload_bench {
                        b.record_batch(_pass_deg_n, _pass_deg_bytes, _pass_deg_t.elapsed());
                    }
                }

                // ── Pass C: ship deltas as deltas (verbatim; no rematerialize) ─
                if !uploadable_deltas.is_empty() {
                    let _pass_c_t = std::time::Instant::now();
                    let mut _pass_c_n = 0u64;
                    let mut _pass_c_bytes = 0u64;
                    let mut stream = futures::stream::iter(uploadable_deltas)
                        .map(|(chunk_id, base_id, delta_bytes)| {
                            let client = self.client.clone();
                            let base_url = self.base_url.clone();
                            async move {
                                let delta_bytes: bytes::Bytes = delta_bytes.into();
                                let delta_size = delta_bytes.len() as u64;
                                let url =
                                    format!("{}/chunk-deltas/{}", base_url, chunk_id.to_hex());
                                let resp = crate::client::send_with_rate_limit_retry(|| {
                                    client
                                        .put(&url)
                                        .header("x-mediagit-delta-base", base_id.to_hex())
                                        .body(delta_bytes.clone())
                                        .send()
                                })
                                .await
                                .map_err(|e| {
                                    anyhow::anyhow!(
                                        "Failed to upload chunk-delta {}: {}",
                                        chunk_id,
                                        e
                                    )
                                })?;
                                if !resp.status().is_success() {
                                    anyhow::bail!(
                                        "PUT /chunk-deltas/{} failed with status: {}",
                                        chunk_id,
                                        resp.status()
                                    );
                                }
                                Ok::<u64, anyhow::Error>(delta_size)
                            }
                        })
                        .buffer_unordered(concurrent_uploads);

                    while let Some(result) = stream.next().await {
                        let chunk_bytes = result?;
                        _pass_c_n += 1;
                        _pass_c_bytes += chunk_bytes;
                        total_chunks_uploaded += 1;
                        bytes_done += chunk_bytes;
                        on_progress(bytes_done, bytes_total);
                    }
                    if let Some(b) = &_upload_bench {
                        b.record_batch(_pass_c_n, _pass_c_bytes, _pass_c_t.elapsed());
                    }
                }
            }

            // Upload manifest last (ensures all chunks exist first)
            let manifest_data = manifest
                .to_bytes()
                .context("Failed to serialize manifest")?;
            self.upload_manifest(oid, &manifest_data).await?;

            tracing::debug!(oid = %oid, "Manifest uploaded");
        }

        if let Some(b) = &_upload_bench {
            b.summary();
        }
        // Sequential/per-chunk path: `bytes_done` accumulates
        // `get_compressed_chunk(..).len()`, i.e. the same wire unit the pack
        // path reports, so the two paths stay comparable.
        Ok((total_chunks_uploaded, bytes_done))
    }

    /// Force-heal remote chunk storage (BUG-RM-3: one corrupt chunk object
    /// permanently bricks a remote, because push dedup and pack-index checks
    /// both treat "server already has it" as sufficient and never re-check
    /// content).
    ///
    /// Walks the FULL object closure reachable from `commit_oids` — no
    /// "have" diffing against the remote's current refs, since a poisoned
    /// chunk is by definition one the server already believes it has (that's
    /// exactly what makes it invisible to ordinary push). Every chunk id
    /// referenced by any chunked blob in the closure is strong-verified via
    /// `POST /chunks/verify-integrity` (BLAKE3 re-hash, always run — never
    /// gated behind `MEDIAGIT_STRONG_VERIFY`). Any chunk the server reports
    /// invalid is re-uploaded unconditionally via `PUT /chunks/:id`, which
    /// the server always overwrites with no existence check (see
    /// `mediagit-server::handlers::chunks::upload_chunk`) — so this bypasses
    /// the "already present" dedup that `/chunks/check` and the pack index
    /// would otherwise apply.
    pub async fn repair_remote(
        &self,
        odb: &ObjectDatabase,
        commit_oids: Vec<Oid>,
    ) -> Result<RepairReport> {
        if commit_oids.is_empty() {
            return Ok(RepairReport::default());
        }

        let objects = self
            .collect_reachable_objects(odb, commit_oids, Vec::new())
            .await?;

        let mut chunk_hexes: Vec<String> = Vec::new();
        let mut seen: HashSet<String> = HashSet::new();
        for (oid, obj_type) in &objects {
            if *obj_type == ObjectType::Blob
                && odb.is_chunked(oid).await.unwrap_or(false)
                && let Some(manifest) = odb.get_chunk_manifest(oid).await?
            {
                for c in &manifest.chunks {
                    let hex = c.id.to_hex();
                    if seen.insert(hex.clone()) {
                        chunk_hexes.push(hex);
                    }
                }
            }
        }

        let mut repaired = 0usize;
        let mut unrepairable = Vec::new();

        // Phase 1: whole objects (commits/trees/un-chunked blobs). The chunk
        // walk below never sees these — CHK20's poisoned blob was one.
        let object_hexes: Vec<String> = objects.iter().map(|(oid, _)| oid.to_hex()).collect();
        let invalid_objects = self.strong_verify_objects(&object_hexes, true).await?;
        if !invalid_objects.is_empty() {
            let invalid_set: HashSet<&str> = invalid_objects.iter().map(|s| s.as_str()).collect();
            let to_reupload: Vec<(Oid, ObjectType)> = objects
                .iter()
                .filter(|(oid, _)| invalid_set.contains(oid.to_hex().as_str()))
                .cloned()
                .collect();
            let n = to_reupload.len();
            let (pack_data, _) = self.generate_pack(odb, to_reupload).await?;
            match self.upload_pack(&pack_data).await {
                Ok(()) => repaired += n,
                Err(_) => unrepairable.extend(invalid_objects.iter().cloned()),
            }
        }

        if chunk_hexes.is_empty() {
            return Ok(RepairReport {
                verified: object_hexes.len(),
                repaired,
                unrepairable,
            });
        }

        let invalid = self.strong_verify_chunks(&chunk_hexes, true).await?;
        let verified = object_hexes.len() + chunk_hexes.len();

        for hex in invalid {
            let oid = match Oid::from_hex(&hex) {
                Ok(o) => o,
                Err(_) => {
                    unrepairable.push(hex);
                    continue;
                }
            };
            let data = match odb.get_compressed_chunk(&oid).await {
                Ok(d) => d,
                Err(_) => {
                    unrepairable.push(hex);
                    continue;
                }
            };
            let url = format!("{}/chunks/{}", self.base_url, hex);
            match self.client.put(&url).body(data).send().await {
                Ok(r) if r.status().is_success() => repaired += 1,
                _ => unrepairable.push(hex),
            }
        }

        Ok(RepairReport {
            verified,
            repaired,
            unrepairable,
        })
    }
}

/// Detect an object's type by reading it and trying each deserializer in
/// turn (Commit, Tree, Tag; else Blob) — same ordering rationale as
/// `mediagit_versioning::reachability`'s sniff chain. Falls back to
/// `ObjectType::Commit` if the object can't be read at all, matching this
/// module's pre-existing behavior for stale/unknown "have" OIDs from the
/// remote (an over-broad guess here only means the traversal below reads
/// the object and finds it truly isn't a commit, not a correctness issue).
async fn detect_object_type(odb: &ObjectDatabase, oid: &Oid) -> ObjectType {
    let Ok(obj_data) = odb.read(oid).await else {
        return ObjectType::Commit;
    };
    if Commit::deserialize(&obj_data).is_ok() {
        ObjectType::Commit
    } else if Tree::deserialize(&obj_data).is_ok() {
        ObjectType::Tree
    } else if Tag::deserialize(&obj_data).is_ok() {
        ObjectType::Tag
    } else {
        ObjectType::Blob
    }
}

#[cfg(test)]
mod tests {
    use super::signs_content_length;

    #[test]
    fn detects_server_signed_content_length() {
        let headers = [["content-length".to_string(), "1234".to_string()]];
        assert!(signs_content_length(&headers));
    }

    /// HTTP header names are case-insensitive and neither the SDK nor the wire
    /// guarantees a casing. Matching only the lowercase spelling would let a
    /// `Content-Length` through and re-introduce the duplicate.
    #[test]
    fn header_match_is_case_insensitive() {
        let headers = [["Content-Length".to_string(), "1234".to_string()]];
        assert!(signs_content_length(&headers));
    }

    /// Unbound presigned URLs (server passed `content_length == 0`) sign no
    /// length, so the client must still supply one — returning true here would
    /// send a body with no content-length at all.
    #[test]
    fn unbound_url_still_needs_an_explicit_length() {
        let headers = [["x-amz-checksum-crc32".to_string(), "abcd".to_string()]];
        assert!(!signs_content_length(&headers));
        assert!(!signs_content_length(&[]));
    }
}
