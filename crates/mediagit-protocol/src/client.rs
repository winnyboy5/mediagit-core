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

use anyhow::{Context, Result};
use mediagit_versioning::{
    chunking::ChunkManifest, Commit, FileMode, ObjectDatabase, ObjectType, Oid, PackWriter, Tree,
};
use std::collections::{HashSet, VecDeque};

use crate::types::{
    RefUpdate, RefUpdateRequest, RefUpdateResponse, RefsResponse, WantRequest, WantResponse,
};

/// Statistics from a push operation
#[derive(Debug, Clone, Default)]
pub struct PushStats {
    /// Number of objects collected for push
    pub objects_count: usize,
    /// Number of commit objects
    pub commits_count: usize,
    /// Number of tree objects
    pub trees_count: usize,
    /// Number of blob objects
    pub blobs_count: usize,
    /// Bytes uploaded (pack size)
    pub bytes_uploaded: usize,
}

/// Phase of the push operation for progress tracking
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PushPhase {
    /// Collecting reachable objects
    Collecting,
    /// Generating pack file
    Packing,
    /// Uploading pack to server
    Uploading,
}

/// Progress update during push operation
#[derive(Debug, Clone)]
pub struct PushProgress {
    /// Current phase of the push
    pub phase: PushPhase,
    /// Current progress (objects collected, bytes packed, etc.)
    pub current: u64,
    /// Total expected (may be 0 if unknown)
    pub total: u64,
    /// Human-readable message
    pub message: String,
}

/// HTTP client for the MediaGit protocol
pub struct ProtocolClient {
    base_url: String,
    client: reqwest::Client,
    /// Optional override for parallel chunk-upload fan-out. Takes precedence
    /// over the internal default (32) but is itself overridden by the
    /// `MEDIAGIT_UPLOAD_CONCURRENCY` env var. Set via `with_concurrent_uploads`.
    concurrent_uploads: Option<usize>,
}

impl ProtocolClient {
    /// Create a new protocol client
    ///
    /// # Arguments
    /// * `base_url` - Base URL of the MediaGit server (e.g., "http://localhost:3000/repo")
    pub fn new(base_url: impl Into<String>) -> Self {
        Self {
            base_url: base_url.into(),
            concurrent_uploads: None,
            client: reqwest::Client::builder()
                // Pool sized to keep parallel uploaders/downloaders from
                // tearing down + re-handshaking TLS on every burst. 32 is well
                // under the IOCP/kernel-handle threshold on Windows but big
                // enough that pipelined push/pull keeps connections warm.
                .pool_max_idle_per_host(32)
                .pool_idle_timeout(std::time::Duration::from_secs(90))
                .tcp_keepalive(std::time::Duration::from_secs(30))
                .tcp_nodelay(true)
                .http2_adaptive_window(true)
                // Larger HTTP/2 windows reduce flow-control stalls when bulk
                // media chunks ride concurrent streams over one connection.
                .http2_initial_stream_window_size(8 * 1024 * 1024)
                .http2_initial_connection_window_size(32 * 1024 * 1024)
                .http2_keep_alive_interval(Some(std::time::Duration::from_secs(20)))
                // No per-request timeout: large chunked-blob PUTs to Azure/S3
                // (single object up to several hundred MB) can legitimately run
                // for minutes — the server side ships block-by-block to cloud.
                // tcp_keepalive (30s) already detects truly dead peers; a hard
                // request ceiling here causes spurious "error sending request"
                // failures on healthy slow uploads. See dev-tests/azure-manual-
                // test for the regression that motivated removing this.
                .build()
                .unwrap_or_else(|_| reqwest::Client::new()),
        }
    }

    /// Override the parallel chunk-upload fan-out used by
    /// `upload_chunked_objects`. Takes precedence over the internal default
    /// of 32, but is still overridden by the `MEDIAGIT_UPLOAD_CONCURRENCY`
    /// env var when that is set. Pass a value derived from
    /// `[performance] upload_concurrency` in the repo config.
    pub fn with_concurrent_uploads(mut self, n: usize) -> Self {
        self.concurrent_uploads = if n > 0 { Some(n) } else { None };
        self
    }

    /// Get all refs from the remote repository
    pub async fn get_refs(&self) -> Result<RefsResponse> {
        let url = format!("{}/info/refs", self.base_url);
        tracing::debug!("GET {}", url);

        let response = self
            .client
            .get(&url)
            .send()
            .await
            .context("Failed to send GET /info/refs")?;

        if !response.status().is_success() {
            anyhow::bail!("GET /info/refs failed with status: {}", response.status());
        }

        response
            .json::<RefsResponse>()
            .await
            .context("Failed to parse refs response")
    }

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
            if let Some(old_oid) = &update.old_oid {
                if let Ok(oid) = Oid::from_hex(old_oid) {
                    have_oids.push(oid);
                }
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
                    self.upload_chunked_objects(odb, &chunked_oids, |_, _| {})
                        .await?;
                }
            } else {
                tracing::info!("No new objects to push - remote already has all objects");
            }
        }

        // Update refs
        let request = RefUpdateRequest { updates, force };
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
    pub async fn push_with_progress<F>(
        &self,
        odb: &ObjectDatabase,
        updates: Vec<RefUpdate>,
        force: bool,
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

            if let Some(old_oid) = &update.old_oid {
                if let Ok(oid) = Oid::from_hex(old_oid) {
                    have_oids.push(oid);
                }
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

            self.upload_pack(&pack_data).await?;

            on_progress(PushProgress {
                phase: PushPhase::Uploading,
                current: pack_data.len() as u64,
                total: pack_data.len() as u64,
                message: "Pack upload complete".to_string(),
            });

            // Phase 4: Upload chunked objects (large files)
            if !chunked_oids.is_empty() {
                self.upload_chunked_objects(odb, &chunked_oids, |bytes_done, bytes_total| {
                    on_progress(PushProgress {
                        phase: PushPhase::Uploading,
                        current: bytes_done,
                        total: bytes_total,
                        message: String::new(),
                    });
                })
                .await?;
            }
        } else {
            tracing::info!("No new objects to push");
        }

        // Update refs
        let request = RefUpdateRequest { updates, force };
        let response = self.update_refs(request).await?;
        Ok((response, stats))
    }

    /// Pull objects from remote and return pack data with chunked object OIDs
    ///
    /// # Arguments
    /// * `odb` - Local object database
    /// * `remote_ref` - Remote ref to pull
    /// * `local_oids` - List of OIDs we already have locally (for incremental pull)
    ///
    /// Pass local commit OIDs to avoid downloading objects we already have.
    /// If empty, all objects reachable from the remote ref will be downloaded.
    ///
    /// Returns (pack_data, chunked_oids)
    #[deprecated(note = "Use pull_streaming() instead for memory-efficient downloads")]
    #[allow(deprecated)]
    pub async fn pull_with_have(
        &self,
        _odb: &ObjectDatabase,
        remote_ref: &str,
        local_oids: Vec<String>,
    ) -> Result<(Vec<u8>, Vec<Oid>)> {
        // Get remote refs
        let remote_refs = self.get_refs().await?;

        // Find the ref we want
        let ref_info = remote_refs
            .refs
            .iter()
            .find(|r| r.name == remote_ref)
            .ok_or_else(|| anyhow::anyhow!("Remote ref '{}' not found", remote_ref))?;

        // Request objects we don't have
        let want = vec![ref_info.oid.clone()];
        let have = local_oids; // OIDs we already have locally

        self.download_pack(want, have).await
    }

    /// Pull objects from a remote ref (backwards compatible, downloads all objects)
    ///
    /// For incremental pulls, use `pull_streaming` instead.
    #[deprecated(note = "Use pull_streaming() instead for memory-efficient downloads")]
    #[allow(deprecated)]
    pub async fn pull(
        &self,
        odb: &ObjectDatabase,
        remote_ref: &str,
    ) -> Result<(Vec<u8>, Vec<Oid>)> {
        self.pull_with_have(odb, remote_ref, Vec::new()).await
    }

    /// Upload a pack file to the server
    async fn upload_pack(&self, pack_data: &[u8]) -> Result<()> {
        let url = format!("{}/objects/pack", self.base_url);
        tracing::debug!("POST {} ({} bytes)", url, pack_data.len());

        let response = self
            .client
            .post(&url)
            .header("Content-Type", "application/octet-stream")
            .body(pack_data.to_vec())
            .send()
            .await
            .context("Failed to upload pack file")?;

        if !response.status().is_success() {
            anyhow::bail!(
                "POST /objects/pack failed with status: {}",
                response.status()
            );
        }

        Ok(())
    }

    /// Download a pack file from the server
    ///
    /// Returns (pack_data, chunked_oids) - chunked objects need separate transfer
    #[deprecated(note = "Use download_pack_streaming() instead for memory-efficient downloads")]
    pub async fn download_pack(
        &self,
        want: Vec<String>,
        have: Vec<String>,
    ) -> Result<(Vec<u8>, Vec<Oid>)> {
        // First, send want request
        let want_url = format!("{}/objects/want", self.base_url);
        tracing::debug!("POST {}", want_url);

        let want_req = WantRequest { want, have };

        let response = self
            .client
            .post(&want_url)
            .json(&want_req)
            .send()
            .await
            .context("Failed to send want request")?;

        if !response.status().is_success() {
            anyhow::bail!(
                "POST /objects/want failed with status: {}",
                response.status()
            );
        }

        // Parse the response to get the request_id (required for GET /objects/pack)
        let want_response: WantResponse = response
            .json()
            .await
            .context("Failed to parse want response")?;

        // Then download the pack with the request_id header
        let pack_url = format!("{}/objects/pack", self.base_url);
        tracing::debug!(
            "GET {} (request_id: {})",
            pack_url,
            want_response.request_id
        );

        let response = self
            .client
            .get(&pack_url)
            .header("X-Request-ID", &want_response.request_id)
            .send()
            .await
            .context("Failed to download pack file")?;

        if !response.status().is_success() {
            anyhow::bail!(
                "GET /objects/pack failed with status: {}",
                response.status()
            );
        }

        // Parse X-Chunked-Objects header for large files that need separate transfer
        let chunked_oids: Vec<Oid> = response
            .headers()
            .get("X-Chunked-Objects")
            .and_then(|h| h.to_str().ok())
            .map(|s| {
                s.split(',')
                    .filter_map(|oid_str| Oid::from_hex(oid_str.trim()).ok())
                    .collect()
            })
            .unwrap_or_default();

        if !chunked_oids.is_empty() {
            tracing::info!(
                count = chunked_oids.len(),
                "Received {} chunked objects for separate download",
                chunked_oids.len()
            );
        }

        let pack_data = response.bytes().await.context("Failed to read pack data")?;

        Ok((pack_data.to_vec(), chunked_oids))
    }

    /// Download pack using streaming (memory-efficient for large files)
    ///
    /// This method processes the pack incrementally without loading it entirely into memory.
    /// Objects are written directly to the ODB as they're received.
    ///
    /// Returns the list of chunked objects that need separate transfer.
    pub async fn download_pack_streaming(
        &self,
        odb: &ObjectDatabase,
        want: Vec<String>,
        have: Vec<String>,
    ) -> Result<Vec<Oid>> {
        // Send want request
        let want_url = format!("{}/objects/want", self.base_url);
        tracing::debug!("POST {} (streaming)", want_url);

        let want_req = WantRequest { want, have };

        let response = self
            .client
            .post(&want_url)
            .json(&want_req)
            .send()
            .await
            .context("Failed to send want request")?;

        if !response.status().is_success() {
            anyhow::bail!(
                "POST /objects/want failed with status: {}",
                response.status()
            );
        }

        let want_response: WantResponse = response
            .json()
            .await
            .context("Failed to parse want response")?;

        // Download pack with streaming
        let pack_url = format!("{}/objects/pack", self.base_url);
        tracing::debug!(
            "GET {} (streaming, request_id: {})",
            pack_url,
            want_response.request_id
        );

        let response = self
            .client
            .get(&pack_url)
            .header("X-Request-ID", &want_response.request_id)
            .send()
            .await
            .context("Failed to download pack file")?;

        if !response.status().is_success() {
            anyhow::bail!(
                "GET /objects/pack failed with status: {}",
                response.status()
            );
        }

        // Parse X-Chunked-Objects header
        let chunked_oids: Vec<Oid> = response
            .headers()
            .get("X-Chunked-Objects")
            .and_then(|h| h.to_str().ok())
            .map(|s| {
                s.split(',')
                    .filter_map(|oid_str| Oid::from_hex(oid_str.trim()).ok())
                    .collect()
            })
            .unwrap_or_default();

        if !chunked_oids.is_empty() {
            tracing::info!(
                count = chunked_oids.len(),
                "Received {} chunked objects for separate download",
                chunked_oids.len()
            );
        }

        // Stream response body and write objects via ODB (ensures proper compression)
        use futures::stream::TryStreamExt;
        use tokio_util::io::StreamReader;

        let stream = response.bytes_stream().map_err(std::io::Error::other);

        let stream_reader = StreamReader::new(stream);

        let mut reader = mediagit_versioning::StreamingPackReader::new(stream_reader)
            .await
            .context("Failed to create streaming pack reader")?;

        tracing::info!("Processing streaming pack download");

        let mut object_count = 0;
        while let Some(result) = reader.next_object().await {
            let (_oid, obj_type, data) =
                result.context("Failed to read object from pack stream")?;

            // Write through ODB to ensure proper compression and storage format.
            // PackTransaction bypassed compression, causing read failures.
            odb.write(obj_type, &data)
                .await
                .context("Failed to write object from pack")?;

            object_count += 1;
            if object_count % 100 == 0 {
                tracing::debug!("Downloaded {} objects", object_count);
            }
        }

        tracing::info!(
            "Successfully downloaded {} objects (streaming)",
            object_count
        );

        Ok(chunked_oids)
    }

    /// Pull using streaming (memory-efficient)
    ///
    /// Returns the list of chunked objects that need separate transfer.
    pub async fn pull_streaming(
        &self,
        odb: &ObjectDatabase,
        remote_ref: &str,
        local_oids: Vec<String>,
    ) -> Result<Vec<Oid>> {
        // Get remote refs
        let remote_refs = self.get_refs().await?;

        // Find the ref we want
        let ref_info = remote_refs
            .refs
            .iter()
            .find(|r| r.name == remote_ref)
            .ok_or_else(|| anyhow::anyhow!("Remote ref '{}' not found", remote_ref))?;

        // Request objects we don't have
        let want = vec![ref_info.oid.clone()];
        let have = local_oids;

        self.download_pack_streaming(odb, want, have).await
    }

    /// Update remote refs
    pub async fn update_refs(&self, request: RefUpdateRequest) -> Result<RefUpdateResponse> {
        let url = format!("{}/refs/update", self.base_url);
        tracing::debug!("POST {}", url);

        let response = self
            .client
            .post(&url)
            .json(&request)
            .send()
            .await
            .context("Failed to update refs")?;

        if !response.status().is_success() {
            anyhow::bail!(
                "POST /refs/update failed with status: {}",
                response.status()
            );
        }

        response
            .json::<RefUpdateResponse>()
            .await
            .context("Failed to parse ref update response")
    }

    /// Upload a single loose object (any type) to the server.
    ///
    /// Wraps the raw bytes in a minimal pack and POSTs it to `/objects/pack`.
    /// Use this for blobs that are not reachable from any commit graph (e.g.
    /// annotated-tag `.meta` sidecars stored as bare blobs).
    pub async fn upload_loose_object(
        &self,
        oid: Oid,
        obj_type: ObjectType,
        data: &[u8],
    ) -> Result<()> {
        let mut pack_writer = PackWriter::new();
        pack_writer.add_object(oid, obj_type, data);
        let pack_data = pack_writer.finalize();
        self.upload_pack(&pack_data).await
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
                    // Detect actual object type by reading and inspecting the object
                    let obj_type = if let Ok(obj_data) = odb.read(&oid).await {
                        // Try to deserialize as each type to detect the actual type
                        if mediagit_versioning::format::deserialize::<Commit>(&obj_data).is_ok() {
                            ObjectType::Commit
                        } else if mediagit_versioning::format::deserialize::<Tree>(&obj_data)
                            .is_ok()
                        {
                            ObjectType::Tree
                        } else {
                            ObjectType::Blob
                        }
                    } else {
                        // Object not found locally - assume Commit for remote objects
                        ObjectType::Commit
                    };
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
                        // Blob is filtered above; this arm satisfies exhaustiveness.
                        _ => {}
                    }
                }
            }

            tracing::debug!("Marked {} objects as already on remote", visited.len());
        }

        // Now collect only NEW objects (not in visited set)
        for oid in commit_oids {
            if visited.insert(oid) {
                queue.push_back((oid, ObjectType::Commit));
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
        let response = self
            .client
            .put(&url)
            .header("x-mediagit-delta-base", base_id.to_hex())
            .body(compressed_delta_bytes)
            .send()
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
    async fn check_chunks_exist(&self, chunk_ids: &[String]) -> Result<Vec<String>> {
        let url = format!("{}/chunks/check", self.base_url);
        tracing::debug!(
            count = chunk_ids.len(),
            "Checking chunk existence on remote"
        );

        let response = self
            .client
            .post(&url)
            .json(&chunk_ids)
            .send()
            .await
            .context("Failed to POST /chunks/check")?;

        if !response.status().is_success() {
            anyhow::bail!(
                "POST /chunks/check failed with status: {}",
                response.status()
            );
        }

        response
            .json::<Vec<String>>()
            .await
            .context("Failed to parse chunks check response")
    }

    /// Upload a manifest to the remote server
    async fn upload_manifest(&self, oid: &Oid, data: &[u8]) -> Result<()> {
        let url = format!("{}/manifests/{}", self.base_url, oid.to_hex());

        let response = self
            .client
            .put(&url)
            .body(data.to_vec())
            .send()
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

    /// Upload all chunks for a chunked object with parallel uploads
    ///
    /// Uses 8 concurrent uploads for optimal throughput (>100MB/s target)
    pub async fn upload_chunked_objects<F>(
        &self,
        odb: &ObjectDatabase,
        chunked_oids: &[Oid],
        mut on_progress: F,
    ) -> Result<usize>
    where
        F: FnMut(u64, u64),
    {
        use futures::stream::StreamExt;

        if chunked_oids.is_empty() {
            return Ok(0);
        }

        let mut total_chunks_uploaded = 0;
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

        let mut bytes_done: u64 = 0;
        let mut bytes_total: u64 = 0;
        on_progress(0, 0);

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
                    let mut stream = futures::stream::iter(full_chunks.clone())
                        .map(|chunk_id| {
                            let client = self.client.clone();
                            let base_url = self.base_url.clone();
                            let odb = odb.clone();
                            async move {
                                let chunk_data = odb.get_compressed_chunk(&chunk_id).await?;
                                let chunk_size = chunk_data.len() as u64;
                                let url = format!("{}/chunks/{}", base_url, chunk_id.to_hex());
                                let resp = client
                                    .put(&url)
                                    .body(chunk_data)
                                    .send()
                                    .await
                                    .map_err(|e| anyhow::anyhow!("Failed to upload chunk {}: {}", chunk_id, e))?;
                                if !resp.status().is_success() {
                                    anyhow::bail!("PUT /chunks/{} failed with status: {}", chunk_id, resp.status());
                                }
                                Ok::<u64, anyhow::Error>(chunk_size)
                            }
                        })
                        .buffer_unordered(concurrent_uploads);

                    while let Some(result) = stream.next().await {
                        let chunk_bytes = result?;
                        total_chunks_uploaded += 1;
                        bytes_done += chunk_bytes;
                        on_progress(bytes_done, bytes_total);
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
                    let mut stream = futures::stream::iter(degraded_ids)
                        .map(|chunk_id| {
                            let client = self.client.clone();
                            let base_url = self.base_url.clone();
                            let odb = odb.clone();
                            async move {
                                let chunk_data = odb.get_compressed_chunk(&chunk_id).await?;
                                let chunk_size = chunk_data.len() as u64;
                                let url = format!("{}/chunks/{}", base_url, chunk_id.to_hex());
                                let resp = client
                                    .put(&url)
                                    .body(chunk_data)
                                    .send()
                                    .await
                                    .map_err(|e| anyhow::anyhow!("Failed to upload chunk {}: {}", chunk_id, e))?;
                                if !resp.status().is_success() {
                                    anyhow::bail!("PUT /chunks/{} failed with status: {}", chunk_id, resp.status());
                                }
                                Ok::<u64, anyhow::Error>(chunk_size)
                            }
                        })
                        .buffer_unordered(concurrent_uploads);

                    while let Some(result) = stream.next().await {
                        let chunk_bytes = result?;
                        total_chunks_uploaded += 1;
                        bytes_done += chunk_bytes;
                        on_progress(bytes_done, bytes_total);
                    }
                }

                // ── Pass C: ship deltas as deltas (verbatim; no rematerialize) ─
                if !uploadable_deltas.is_empty() {
                    let mut stream = futures::stream::iter(uploadable_deltas)
                        .map(|(chunk_id, base_id, delta_bytes)| {
                            let client = self.client.clone();
                            let base_url = self.base_url.clone();
                            async move {
                                let delta_size = delta_bytes.len() as u64;
                                let url =
                                    format!("{}/chunk-deltas/{}", base_url, chunk_id.to_hex());
                                let resp = client
                                    .put(&url)
                                    .header("x-mediagit-delta-base", base_id.to_hex())
                                    .body(delta_bytes)
                                    .send()
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
                        total_chunks_uploaded += 1;
                        bytes_done += chunk_bytes;
                        on_progress(bytes_done, bytes_total);
                    }
                }
            }

            // Upload manifest last (ensures all chunks exist first)
            let manifest_data = mediagit_versioning::format::serialize(&manifest)
                .context("Failed to serialize manifest")?;
            self.upload_manifest(oid, &manifest_data).await?;

            tracing::debug!(oid = %oid, "Manifest uploaded");
        }

        Ok(total_chunks_uploaded)
    }

    // ========================================================================
    // Chunk Download Methods - For efficient large file pull/clone
    // ========================================================================

    /// Download a manifest from the remote server
    pub async fn download_manifest(&self, oid: &Oid) -> Result<ChunkManifest> {
        let url = format!("{}/manifests/{}", self.base_url, oid.to_hex());

        let response = self
            .client
            .get(&url)
            .send()
            .await
            .context(format!("Failed to GET /manifests/{}", oid))?;

        if !response.status().is_success() {
            anyhow::bail!(
                "GET /manifests/{} failed with status: {}",
                oid,
                response.status()
            );
        }

        let data = response.bytes().await?;
        mediagit_versioning::format::deserialize(&data).context("Failed to deserialize manifest")
    }

    /// Download a single chunk from the remote server
    pub async fn download_chunk(&self, chunk_id: &Oid) -> Result<Vec<u8>> {
        let url = format!("{}/chunks/{}", self.base_url, chunk_id.to_hex());

        let response = self
            .client
            .get(&url)
            .send()
            .await
            .context(format!("Failed to GET /chunks/{}", chunk_id))?;

        if !response.status().is_success() {
            anyhow::bail!(
                "GET /chunks/{} failed with status: {}",
                chunk_id,
                response.status()
            );
        }

        Ok(response.bytes().await?.to_vec())
    }

    /// Ask the server which of the given chunk IDs exist as chunk-deltas.
    ///
    /// Returns a map `chunk_id → base_chunk_id` (both as `Oid`). Chunks not
    /// in the response are stored as full chunks (use `download_chunk`).
    ///
    /// On 404 (old server without the endpoint) or any error, returns an
    /// empty map — the caller falls back to full-chunk downloads. This keeps
    /// new clients compatible with old servers.
    async fn check_chunk_deltas(&self, chunk_ids: &[Oid]) -> std::collections::HashMap<Oid, Oid> {
        let mut empty = std::collections::HashMap::new();
        if chunk_ids.is_empty() {
            return empty;
        }

        let url = format!("{}/chunk-deltas/check", self.base_url);
        let payload: Vec<String> = chunk_ids.iter().map(|o| o.to_hex()).collect();

        let response = match self.client.post(&url).json(&payload).send().await {
            Ok(r) => r,
            Err(e) => {
                tracing::debug!(error = %e, "chunk-deltas/check failed (treating as no deltas)");
                return empty;
            }
        };

        if !response.status().is_success() {
            // 404 means old server — silently fall back. Other statuses also
            // fall back (best-effort optimization, never blocks the clone).
            tracing::debug!(
                status = %response.status(),
                "chunk-deltas/check non-success (treating as no deltas)"
            );
            return empty;
        }

        let map: std::collections::HashMap<String, String> = match response.json().await {
            Ok(m) => m,
            Err(e) => {
                tracing::debug!(error = %e, "chunk-deltas/check parse failed");
                return empty;
            }
        };

        for (id_hex, base_hex) in map {
            if let (Ok(id), Ok(base)) = (Oid::from_hex(&id_hex), Oid::from_hex(&base_hex)) {
                empty.insert(id, base);
            }
        }
        empty
    }

    /// Download all chunks for chunked objects with parallel downloads
    ///
    /// Uses 8 concurrent downloads for optimal throughput (>100MB/s target).
    /// Progress callback receives `(chunks_done, total_manifest_chunks, msg)` for
    /// smooth chunk-level ETA (avoids "211y" caused by object-level reporting).
    ///
    /// Delta-aware: for each manifest, asks the server which chunks exist as
    /// chunk-deltas via `POST /chunk-deltas/check`. Chunks present as deltas
    /// are downloaded via `GET /chunk-deltas/<id>` and persisted via
    /// `odb.write_chunk_delta`, preserving the storage savings the server has.
    /// Falls back to full-chunk downloads for any chunk the server doesn't
    /// report as a delta, and for old servers that don't expose the endpoint.
    pub async fn download_chunked_objects<F>(
        &self,
        odb: &ObjectDatabase,
        chunked_oids: &[Oid],
        mut on_progress: F,
    ) -> Result<usize>
    where
        F: FnMut(usize, usize, &str),
    {
        use futures::stream::StreamExt;

        if chunked_oids.is_empty() {
            return Ok(0);
        }

        // Concurrency for parallel chunk downloads. Default 32 matches the
        // upload side. Residential downlink typically has more headroom than
        // upstream so c=64 also helped (clone 92 -> 57s in measurements), but
        // 32 is the safer cross-platform default; high-downstream users can
        // env-override. HTTP/1.1 keep-alive + pool_max_idle_per_host=32 in
        // ProtocolClient::new keeps connections warm.
        let concurrent_downloads: usize = std::env::var("MEDIAGIT_DOWNLOAD_CONCURRENCY")
            .ok()
            .and_then(|s| s.parse().ok())
            .filter(|n: &usize| *n > 0)
            .unwrap_or(32);
        let n_objects = chunked_oids.len();

        // ── Phase 1: manifests (fast — small metadata payloads) ──────────────
        // Download all manifests upfront to know total_chunks before any data
        // transfer. Manifests are tiny (list of chunk IDs + sizes); storing them
        // all is at most ~KB total even for large repos.
        struct ObjectWork {
            oid: Oid,
            manifest: ChunkManifest,
            missing_chunks: Vec<Oid>,
        }

        let mut object_work: Vec<ObjectWork> = Vec::with_capacity(n_objects);
        let mut total_manifest_chunks: usize = 0;

        for oid in chunked_oids.iter() {
            let manifest = self.download_manifest(oid).await?;
            let obj_total = manifest.chunks.len();
            total_manifest_chunks += obj_total;

            let mut missing: Vec<Oid> = Vec::new();
            for chunk_ref in &manifest.chunks {
                if !odb.chunk_exists(&chunk_ref.id).await.unwrap_or(false) {
                    missing.push(chunk_ref.id);
                }
            }

            if missing.is_empty() {
                tracing::debug!(oid = %oid, "All chunks already exist locally");
            } else {
                tracing::info!(
                    oid = %oid,
                    missing = missing.len(),
                    total = obj_total,
                    "Downloading missing chunks"
                );
            }

            object_work.push(ObjectWork {
                oid: *oid,
                manifest,
                missing_chunks: missing,
            });
        }

        // Signal total upfront so the progress bar initialises with correct length.
        on_progress(
            0,
            total_manifest_chunks,
            &format!(
                "{} objects, {} total chunks",
                n_objects, total_manifest_chunks
            ),
        );

        // ── Phase 2: chunk download — streaming write, O(concurrent × chunk) RAM
        // Writing each chunk as it arrives (while let Some) instead of collect()
        // means peak RAM = concurrent_downloads × max_chunk_size (≤64 KB on
        // Windows) rather than accumulating every result before any disk write.
        let mut total_chunks_downloaded = 0usize;
        let mut chunks_done: usize = 0;

        for (
            obj_idx,
            ObjectWork {
                oid,
                manifest,
                missing_chunks,
            },
        ) in object_work.into_iter().enumerate()
        {
            let already_local = manifest.chunks.len() - missing_chunks.len();

            if !missing_chunks.is_empty() {
                // Ask the server which of these chunks are stored as deltas.
                // Best-effort: empty map on old servers / errors.
                let delta_map = self.check_chunk_deltas(&missing_chunks).await;

                // Split into "full chunks" and "delta chunks" based on what the
                // server actually stores. A chunk that the server reports as a
                // delta MUST be downloaded via `/chunk-deltas/<id>` — hitting
                // `/chunks/<id>` for it would 404 because no full copy exists.
                // Delta chains (depth ≥ 2) are fine: all chunks in the chain
                // land locally as deltas in this single pass; reads later
                // follow the chain via the .meta sidecars.
                //
                // Cycle pre-filter still applies: if writing a chunk as a delta
                // would create a cycle with the LOCAL on-disk chain (e.g. a
                // prior partial clone), degrade that one to a full download.
                // If the full endpoint also lacks the object the server is
                // misconfigured — surface the error rather than mask it.
                let mut full_chunks: Vec<Oid> = Vec::new();
                let mut delta_chunks: Vec<Oid> = Vec::new();
                for cid in &missing_chunks {
                    if let Some(&base) = delta_map.get(cid) {
                        let cycle_risk = base == *cid
                            || odb
                                .chunk_delta_chain_contains(&base, cid)
                                .await
                                .unwrap_or(false);
                        if cycle_risk {
                            tracing::debug!(
                                chunk = %cid,
                                base = %base,
                                "Routing to full-chunk download (would create local cycle)"
                            );
                            full_chunks.push(*cid);
                        } else {
                            delta_chunks.push(*cid);
                        }
                    } else {
                        full_chunks.push(*cid);
                    }
                }

                tracing::debug!(
                    full = full_chunks.len(),
                    deltas = delta_chunks.len(),
                    "Split chunk download: full first, then deltas"
                );

                // ── Pass A: full chunks (and any chunks needed as delta bases) ──
                if !full_chunks.is_empty() {
                    let mut stream = futures::stream::iter(full_chunks.into_iter())
                        .map(|chunk_id| {
                            let client = self.client.clone();
                            let base_url = self.base_url.clone();
                            async move {
                                let url = format!("{}/chunks/{}", base_url, chunk_id.to_hex());
                                let response = client.get(&url).send().await.map_err(|e| {
                                    anyhow::anyhow!("Failed to download chunk {}: {}", chunk_id, e)
                                })?;
                                if !response.status().is_success() {
                                    anyhow::bail!(
                                        "GET /chunks/{} failed with status: {}",
                                        chunk_id,
                                        response.status()
                                    );
                                }
                                let data = response.bytes().await.map_err(|e| {
                                    anyhow::anyhow!("Failed to read chunk {}: {}", chunk_id, e)
                                })?;
                                Ok::<_, anyhow::Error>((chunk_id, data.to_vec()))
                            }
                        })
                        .buffer_unordered(concurrent_downloads);

                    while let Some(result) = stream.next().await {
                        let (chunk_id, chunk_data) = result?;
                        odb.put_compressed_chunk(&chunk_id, &chunk_data).await?;
                        total_chunks_downloaded += 1;
                        chunks_done += 1;
                        on_progress(
                            chunks_done,
                            total_manifest_chunks,
                            &format!("Object {}/{}", obj_idx + 1, n_objects),
                        );
                    }
                }

                // ── Pass B: delta chunks (bases are now local) ────────────────
                if !delta_chunks.is_empty() {
                    let delta_map = std::sync::Arc::new(delta_map);
                    let mut stream = futures::stream::iter(delta_chunks.into_iter())
                        .map(|chunk_id| {
                            let client = self.client.clone();
                            let base_url = self.base_url.clone();
                            let delta_map = std::sync::Arc::clone(&delta_map);
                            async move {
                                let url =
                                    format!("{}/chunk-deltas/{}", base_url, chunk_id.to_hex());
                                let response = client.get(&url).send().await.map_err(|e| {
                                    anyhow::anyhow!(
                                        "Failed to download chunk-delta {}: {}",
                                        chunk_id,
                                        e
                                    )
                                })?;
                                if !response.status().is_success() {
                                    anyhow::bail!(
                                        "GET /chunk-deltas/{} failed with status: {}",
                                        chunk_id,
                                        response.status()
                                    );
                                }
                                let data = response.bytes().await.map_err(|e| {
                                    anyhow::anyhow!(
                                        "Failed to read chunk-delta {}: {}",
                                        chunk_id,
                                        e
                                    )
                                })?;
                                let base = delta_map.get(&chunk_id).copied().ok_or_else(|| {
                                    anyhow::anyhow!(
                                        "Internal: chunk {} missing from delta_map",
                                        chunk_id
                                    )
                                })?;
                                Ok::<_, anyhow::Error>((chunk_id, base, data.to_vec()))
                            }
                        })
                        .buffer_unordered(concurrent_downloads);

                    while let Some(result) = stream.next().await {
                        let (chunk_id, base_id, delta_bytes) = result?;
                        if let Err(e) = odb
                            .write_chunk_delta(&chunk_id, &base_id, &delta_bytes)
                            .await
                        {
                            tracing::warn!(
                                chunk = %chunk_id,
                                base = %base_id,
                                error = %e,
                                "Delta write failed (cycle?), falling back to full chunk"
                            );
                            let full = self.download_chunk(&chunk_id).await?;
                            odb.put_compressed_chunk(&chunk_id, &full).await?;
                        }
                        total_chunks_downloaded += 1;
                        chunks_done += 1;
                        on_progress(
                            chunks_done,
                            total_manifest_chunks,
                            &format!("Object {}/{}", obj_idx + 1, n_objects),
                        );
                    }
                }
            }

            // Advance bar for chunks that were already local (dedup / re-clone).
            if already_local > 0 {
                chunks_done += already_local;
                on_progress(
                    chunks_done,
                    total_manifest_chunks,
                    &format!("Object {}/{}", obj_idx + 1, n_objects),
                );
            }

            // Store manifest locally
            odb.put_manifest(&oid, &manifest).await?;

            tracing::debug!(oid = %oid, "Chunked object downloaded");
        }

        Ok(total_chunks_downloaded)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_client_creation() {
        let client = ProtocolClient::new("http://localhost:3000/test-repo");
        assert_eq!(client.base_url, "http://localhost:3000/test-repo");
    }

    // Additional integration tests would require a running server
    // These should be in tests/integration/
}
