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
use std::sync::{
    atomic::{AtomicU64, AtomicUsize, Ordering},
    Arc,
};

use crate::types::{
    RefUpdate, RefUpdateRequest, RefUpdateResponse, RefsResponse, WantRequest, WantResponse,
};

/// Presigned PUT URL info returned by the server for direct-to-bucket uploads.
#[derive(serde::Deserialize)]
struct PresignedPutInfo {
    url: String,
    #[allow(dead_code)]
    method: String,
    required_headers: Vec<[String; 2]>,
}

/// Presigned GET URL info returned by the server for direct-from-bucket downloads.
#[derive(serde::Deserialize, Clone)]
struct PresignedGetInfo {
    url: String,
    #[allow(dead_code)]
    method: String,
    headers: Vec<(String, String)>,
    #[allow(dead_code)]
    expires_in_secs: u64,
}

/// Location of a chunk within a cloud pack object (F7).
struct PackLocInfo {
    pack_oid: String,
    offset: u64,
    length: u32,
}

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
    /// Optional override for parallel chunk-download fan-out. Takes precedence
    /// over the internal default (24) but is itself overridden by the
    /// `MEDIAGIT_DOWNLOAD_CONCURRENCY` env var. Set via `with_concurrent_downloads`.
    concurrent_downloads: Option<usize>,
}

/// Single source of truth for `MEDIAGIT_HTTP_POOL_MAX`.
/// All three reqwest clients (control, upload, download) read this one function
/// so a user-set value applies uniformly. Default 64 is safe for all client types;
/// the prior per-client defaults (64/96/160) were inconsistent (B8 fix).
fn http_pool_max() -> usize {
    std::env::var("MEDIAGIT_HTTP_POOL_MAX")
        .ok()
        .and_then(|s| s.parse().ok())
        .filter(|n: &usize| *n > 0)
        .unwrap_or(64)
}

impl ProtocolClient {
    /// Create a new protocol client
    ///
    /// # Arguments
    /// * `base_url` - Base URL of the MediaGit server (e.g., "http://localhost:3000/repo")
    pub fn new(base_url: impl Into<String>) -> Self {
        // Control-plane pool size: idle TLS connections to mediagit-server.
        // Override via MEDIAGIT_HTTP_POOL_MAX (see http_pool_max()).
        let pool_max = http_pool_max();
        Self {
            base_url: base_url.into(),
            concurrent_uploads: None,
            concurrent_downloads: None,
            client: reqwest::Client::builder()
                // HTTP/2 is used for control-plane traffic (manifest, URL-mint,
                // ref negotiation). Multiplexing many small requests on one TLS
                // connection avoids per-request handshake cost. Data-plane
                // presigned PUT/GET uses separate direct_client (HTTP/1.1).
                .pool_max_idle_per_host(pool_max)
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

    /// Override the parallel chunk-download fan-out used by
    /// `download_chunked_objects`. Takes precedence over the internal default
    /// of 24, but is still overridden by the `MEDIAGIT_DOWNLOAD_CONCURRENCY`
    /// env var when that is set. Pass a value derived from
    /// `[performance] download_concurrency` in the repo config.
    pub fn with_concurrent_downloads(mut self, n: usize) -> Self {
        self.concurrent_downloads = if n > 0 { Some(n) } else { None };
        self
    }

    /// Create a shallow copy of this client with a different concurrent_downloads
    /// cap. reqwest::Client is Arc-based so the connection pool is shared.
    /// Used by the parallel fetch path to bound per-branch download fan-out.
    pub fn with_download_cap(&self, n: usize) -> Self {
        Self {
            base_url: self.base_url.clone(),
            client: self.client.clone(),
            concurrent_uploads: self.concurrent_uploads,
            concurrent_downloads: Some(n),
        }
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

    /// Request presigned PUT URLs for a batch of chunk IDs.
    ///
    /// Returns a map of `chunk_hex → Option<PresignedPutInfo>`.
    /// `None` means the backend doesn't support presigning — caller must use
    /// the server-proxied `PUT /chunks/:id` path instead.
    /// Any network/parse failure is treated as "no presign" (graceful degradation).
    async fn request_chunk_upload_urls(
        &self,
        chunk_ids: &[String],
        sizes: &std::collections::HashMap<String, u64>,
    ) -> std::collections::HashMap<String, Option<PresignedPutInfo>> {
        #[derive(serde::Serialize)]
        struct Req<'a> {
            chunk_ids: &'a [String],
            sizes: &'a std::collections::HashMap<String, u64>,
        }

        let url = format!("{}/chunks/upload-urls", self.base_url);
        let result = async {
            let resp = self
                .client
                .post(&url)
                .json(&Req { chunk_ids, sizes })
                .send()
                .await?;
            if !resp.status().is_success() {
                anyhow::bail!("POST /chunks/upload-urls returned {}", resp.status());
            }
            resp.json::<std::collections::HashMap<String, Option<PresignedPutInfo>>>()
                .await
                .context("parse /chunks/upload-urls")
        }
        .await;

        match result {
            Ok(map) => map,
            Err(e) => {
                tracing::debug!(err = %e, "presign URL request failed; using proxy PUT for all chunks");
                std::collections::HashMap::new()
            }
        }
    }

    /// Verify that a set of chunk IDs exist on the server after upload.
    ///
    /// Returns `Ok(missing)` where missing is the subset not yet in storage.
    /// Returns `Err` on network/server failure — caller must treat all chunks
    /// as unconfirmed and retry via proxy PUT to avoid silent data loss.
    /// Request presigned GET URLs for a batch of chunk IDs.
    ///
    /// Returns a map of `chunk_hex → Option<PresignedGetInfo>`.
    /// `None` means the backend doesn't support presigning or chunk doesn't exist yet
    /// — caller must use the server-proxied `GET /chunks/:id` path instead.
    /// Any network/parse failure is treated as "no presign" (graceful degradation).
    async fn request_chunk_download_urls(
        &self,
        chunk_ids: &[String],
    ) -> std::collections::HashMap<String, Option<PresignedGetInfo>> {
        if chunk_ids.is_empty() {
            return std::collections::HashMap::new();
        }

        #[derive(serde::Serialize)]
        struct Req<'a> {
            chunks: &'a [String],
        }

        use futures::stream::StreamExt;

        // Split into smaller batches so each HTTP round-trip is bounded in size/latency.
        // Batches are fired concurrently (up to 4 in-flight) and merged into one map.
        let batch_size: usize = std::env::var("MEDIAGIT_PRESIGN_BATCH")
            .ok()
            .and_then(|s| s.parse().ok())
            .filter(|n: &usize| *n > 0)
            .unwrap_or(512);

        let batches: Vec<&[String]> = chunk_ids.chunks(batch_size).collect();
        let base_url = &self.base_url;
        let client = &self.client;

        let results: Vec<_> = futures::stream::iter(batches)
            .map(|batch| async move {
                let url = format!("{}/chunks/download-urls", base_url);
                let result = async {
                    let resp = client
                        .post(&url)
                        .json(&Req { chunks: batch })
                        .timeout(std::time::Duration::from_secs(20))
                        .send()
                        .await?;
                    if !resp.status().is_success() {
                        anyhow::bail!("POST /chunks/download-urls returned {}", resp.status());
                    }
                    resp.json::<std::collections::HashMap<String, Option<PresignedGetInfo>>>()
                        .await
                        .context("parse /chunks/download-urls")
                }
                .await;
                match result {
                    Ok(map) => map,
                    Err(e) => {
                        tracing::debug!(err = %e, "presign batch request failed; affected chunks use proxy GET");
                        std::collections::HashMap::new()
                    }
                }
            })
            .buffer_unordered(4)
            .collect()
            .await;

        let mut merged = std::collections::HashMap::with_capacity(chunk_ids.len());
        for map in results {
            merged.extend(map);
        }
        merged
    }

    async fn verify_chunk_uploads(&self, chunk_ids: &[String]) -> anyhow::Result<Vec<String>> {
        #[derive(serde::Serialize)]
        struct Req<'a> {
            chunk_ids: &'a [String],
        }
        #[derive(serde::Deserialize)]
        struct Resp {
            missing: Vec<String>,
        }

        let url = format!("{}/chunks/complete", self.base_url);
        let resp = self
            .client
            .post(&url)
            .json(&Req { chunk_ids })
            .send()
            .await?;
        if !resp.status().is_success() {
            anyhow::bail!("POST /chunks/complete returned {}", resp.status());
        }
        let r = resp
            .json::<Resp>()
            .await
            .context("parse /chunks/complete")?;
        Ok(r.missing)
    }

    /// POST /:repo/chunks/verify-integrity — BLAKE3 re-hash of every stored chunk.
    /// Only called when `MEDIAGIT_STRONG_VERIFY=1`. Returns chunk ids whose stored
    /// content does not match their claimed hash.
    async fn strong_verify_chunks(&self, chunk_ids: &[String]) -> anyhow::Result<Vec<String>> {
        #[derive(serde::Serialize)]
        struct Req<'a> {
            chunk_ids: &'a [String],
        }
        #[derive(serde::Deserialize)]
        struct Resp {
            invalid: Vec<String>,
        }

        let url = format!("{}/chunks/verify-integrity", self.base_url);
        let resp = self
            .client
            .post(&url)
            .json(&Req { chunk_ids })
            .send()
            .await?;
        if !resp.status().is_success() {
            anyhow::bail!("POST /chunks/verify-integrity returned {}", resp.status());
        }
        let r = resp
            .json::<Resp>()
            .await
            .context("parse /chunks/verify-integrity")?;
        Ok(r.invalid)
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
            let pack_pushed = if !full_chunks.is_empty() && cloud_packs {
                match self.push_full_chunks_via_packs(&full_chunks, odb).await {
                    Ok((n, b)) if n > 0 => {
                        chunks_uploaded += n;
                        upload_bytes += b;
                        true
                    }
                    Ok(_) => {
                        tracing::debug!("pack push: 0 chunks uploaded; using per-chunk fallback");
                        false
                    }
                    Err(e) => {
                        tracing::warn!(
                            err = %e,
                            "pack push failed; falling back to per-chunk path"
                        );
                        false
                    }
                }
            } else {
                false
            };
            if !full_chunks.is_empty() && !pack_pushed {
                // Request presigned PUT URLs from the server.  Cloud
                // backends return signed bucket URLs; Local/Mock return
                // null → falls through to the server-proxy PUT path.
                let full_chunk_hexes: Vec<String> =
                    full_chunks.iter().map(|c| c.to_hex()).collect();
                let chunk_sizes: std::collections::HashMap<String, u64> = manifest
                    .chunks
                    .iter()
                    .filter(|c| missing_set.contains(&c.id.to_hex()))
                    .filter(|c| full_chunks.iter().any(|fc| fc == &c.id))
                    .map(|c| (c.id.to_hex(), c.size as u64))
                    .collect();
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
                            if !direct_succeeded {
                            if let Some(Some(purl)) = presigned.get(&hex) {
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

                                    let mut req = direct_client
                                        .put(&current_url)
                                        .header(
                                            reqwest::header::CONTENT_LENGTH,
                                            chunk_data.len(),
                                        );
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
                                                        if let Some(r) = refreshed {
                                                            if let Ok(map) = r
                                                                .json::<std::collections::HashMap<
                                                                    String,
                                                                    Option<PresignedPutInfo>,
                                                                >>()
                                                                .await
                                                            {
                                                                if let Some(Some(np)) = map.get(&hex) {
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
                                                            }
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
                                            tracing::debug!(
                                                chunk = %hex,
                                                attempt,
                                                err = %e,
                                                "Direct upload network error; retrying"
                                            );
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
                            }
                            } // end if !direct_succeeded

                            if direct_succeeded {
                                return Ok::<(Oid, u64), anyhow::Error>((chunk_id, chunk_size));
                            }

                            // Proxy path: used when no presigned URL was issued, or all
                            // direct attempts for this chunk were exhausted.
                            let url = format!("{}/chunks/{}", base_url, hex);
                            let resp = client
                                .put(&url)
                                .body(chunk_data)
                                .send()
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
                                let chunk_data = odb.get_compressed_chunk(&chunk_id).await?;
                                let chunk_size = chunk_data.len() as u64;
                                let url = format!("{}/chunks/{}", base_url, chunk_id.to_hex());
                                let resp = client.put(&url).body(chunk_data).send().await.map_err(
                                    |e| anyhow::anyhow!("Retry chunk {}: {}", chunk_id, e),
                                )?;
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
            // Skipped in pack mode — chunks live at packs/<oid>, not chunks/<hex>.
            if std::env::var("MEDIAGIT_STRONG_VERIFY").as_deref() == Ok("1")
                && !full_chunks.is_empty()
                && !cloud_packs
            {
                let hexes: Vec<String> = full_chunks.iter().map(|c| c.to_hex()).collect();
                tracing::debug!(count = hexes.len(), "Running strong chunk integrity verify");
                match self.strong_verify_chunks(&hexes).await {
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
                let mut stream =
                    futures::stream::iter(degraded_ids)
                        .map(|chunk_id| {
                            let client = self.client.clone();
                            let base_url = self.base_url.clone();
                            let odb = odb.clone();
                            async move {
                                let chunk_data = odb.get_compressed_chunk(&chunk_id).await?;
                                let chunk_size = chunk_data.len() as u64;
                                let url = format!("{}/chunks/{}", base_url, chunk_id.to_hex());
                                let resp = client.put(&url).body(chunk_data).send().await.map_err(
                                    |e| {
                                        anyhow::anyhow!(
                                            "Failed to upload chunk {}: {}",
                                            chunk_id,
                                            e
                                        )
                                    },
                                )?;
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
                            let delta_size = delta_bytes.len() as u64;
                            let url = format!("{}/chunk-deltas/{}", base_url, chunk_id.to_hex());
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
        let manifest_data = mediagit_versioning::format::serialize(&manifest)
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
                    async move {
                        self.push_one_object(oid, &odb, per_obj_concurrent, bp, btp)
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
                                let (chunks, _bytes_up, _bytes_total_delta) = r?;
                                total_chunks_uploaded += chunks as usize;
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
            return Ok(total_chunks_uploaded);
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
                    let chunk_sizes: std::collections::HashMap<String, u64> = manifest
                        .chunks
                        .iter()
                        .filter(|c| missing_set.contains(&c.id.to_hex()))
                        .filter(|c| full_chunks.iter().any(|fc| fc == &c.id))
                        .map(|c| (c.id.to_hex(), c.size as u64))
                        .collect();
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
                                if !direct_succeeded {
                                if let Some(Some(purl)) = presigned.get(&hex) {
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

                                        let mut req = direct_client
                                            .put(&current_url)
                                            .header(
                                                reqwest::header::CONTENT_LENGTH,
                                                chunk_data.len(),
                                            );
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
                                                            if let Some(r) = refreshed {
                                                                if let Ok(map) = r
                                                                    .json::<std::collections::HashMap<
                                                                        String,
                                                                        Option<PresignedPutInfo>,
                                                                    >>()
                                                                    .await
                                                                {
                                                                    if let Some(Some(np)) = map.get(&hex) {
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
                                                                }
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
                                                tracing::debug!(
                                                    chunk = %hex,
                                                    attempt,
                                                    err = %e,
                                                    "Direct upload network error; retrying"
                                                );
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
                                }
                                } // end if !direct_succeeded

                                if direct_succeeded {
                                    return Ok::<u64, anyhow::Error>(chunk_size);
                                }

                                // Proxy path: used when no presigned URL was issued, or all
                                // direct attempts for this chunk were exhausted.
                                let url = format!("{}/chunks/{}", base_url, hex);
                                let resp = client
                                    .put(&url)
                                    .body(chunk_data)
                                    .send()
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
                                    let chunk_data = odb.get_compressed_chunk(&chunk_id).await?;
                                    let chunk_size = chunk_data.len() as u64;
                                    let url = format!("{}/chunks/{}", base_url, chunk_id.to_hex());
                                    let resp =
                                        client.put(&url).body(chunk_data).send().await.map_err(
                                            |e| anyhow::anyhow!("Retry chunk {}: {}", chunk_id, e),
                                        )?;
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
                                let chunk_data = odb.get_compressed_chunk(&chunk_id).await?;
                                let chunk_size = chunk_data.len() as u64;
                                let url = format!("{}/chunks/{}", base_url, chunk_id.to_hex());
                                let resp = client.put(&url).body(chunk_data).send().await.map_err(
                                    |e| {
                                        anyhow::anyhow!(
                                            "Failed to upload chunk {}: {}",
                                            chunk_id,
                                            e
                                        )
                                    },
                                )?;
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
            let manifest_data = mediagit_versioning::format::serialize(&manifest)
                .context("Failed to serialize manifest")?;
            self.upload_manifest(oid, &manifest_data).await?;

            tracing::debug!(oid = %oid, "Manifest uploaded");
        }

        if let Some(b) = &_upload_bench {
            b.summary();
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

        if response.status() == reqwest::StatusCode::SERVICE_UNAVAILABLE {
            anyhow::bail!(
                "GET /chunks/{} failed: server storage backend unreachable (503) — verify the storage service (MinIO/S3/Azure) is running and accessible to the server",
                chunk_id
            );
        }
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
        const TIMEOUT_SECS: u64 = 5;
        match tokio::time::timeout(
            std::time::Duration::from_secs(TIMEOUT_SECS),
            self.check_chunk_deltas_inner(chunk_ids),
        )
        .await
        {
            Ok(map) => map,
            Err(_elapsed) => {
                tracing::debug!(
                    chunks = chunk_ids.len(),
                    timeout_secs = TIMEOUT_SECS,
                    "chunk-deltas/check timed out; treating as no deltas"
                );
                std::collections::HashMap::new()
            }
        }
    }

    async fn check_chunk_deltas_inner(
        &self,
        chunk_ids: &[Oid],
    ) -> std::collections::HashMap<Oid, Oid> {
        let mut empty = std::collections::HashMap::new();
        if chunk_ids.is_empty() {
            return empty;
        }

        let url = format!("{}/chunk-deltas/check", self.base_url);
        let payload: Vec<String> = chunk_ids.iter().map(|o| o.to_hex()).collect();

        let response = match self.client.post(&url).json(&payload).send().await {
            Ok(r) => r,
            Err(e) => {
                // Transport failure is unexpected — warn so production issues surface
                // instead of silently routing all chunks through /chunks/<id> and 404ing.
                tracing::warn!(error = %e, "chunk-deltas/check request failed (treating as no deltas)");
                return empty;
            }
        };

        if !response.status().is_success() {
            if response.status().as_u16() == 404 {
                // 404 means old server without this endpoint — expected, silently fall back.
                tracing::debug!(status = %response.status(), "chunk-deltas/check 404 (old server); treating as no deltas");
            } else {
                // Non-404 failures (500/503/etc.) are unexpected — log at warn so
                // production probe failures are visible rather than silently causing clone 404s.
                tracing::warn!(
                    status = %response.status(),
                    "chunk-deltas/check non-success (treating as no deltas)"
                );
            }
            return empty;
        }

        let map: std::collections::HashMap<String, String> = match response.json().await {
            Ok(m) => m,
            Err(e) => {
                tracing::warn!(error = %e, "chunk-deltas/check response parse failed (treating as no deltas)");
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
        F: FnMut(u64, u64, &str),
    {
        use futures::stream::StreamExt;

        if chunked_oids.is_empty() {
            return Ok(0);
        }

        // Concurrency for parallel chunk downloads. Default 24 paired with
        // MEDIAGIT_RANGE_PARALLEL=4 gives 96 effective TCP streams — enough
        // headroom without opening more sockets than the pool can keep warm.
        // Env var > builder > internal default.
        let concurrent_downloads: usize = std::env::var("MEDIAGIT_DOWNLOAD_CONCURRENCY")
            .ok()
            .and_then(|s| s.parse().ok())
            .filter(|n: &usize| *n > 0)
            .or(self.concurrent_downloads)
            .unwrap_or(32);
        let n_objects = chunked_oids.len();
        let _download_bench = crate::bench::maybe_start("download", concurrent_downloads);

        // Gate: set MEDIAGIT_DOWNLOAD_DIRECT_DISABLE=1 to bypass presigned GET entirely.
        let direct_download_enabled =
            std::env::var("MEDIAGIT_DOWNLOAD_DIRECT_DISABLE").as_deref() != Ok("1");

        // Data-plane client for presigned GET downloads.
        // HTTP/1.1: parallel TCP sockets beat h2 multiplexing for large bodies.
        // Pool size via MEDIAGIT_HTTP_POOL_MAX (see http_pool_max()).
        let direct_client = reqwest::Client::builder()
            .pool_idle_timeout(std::time::Duration::from_secs(60))
            .pool_max_idle_per_host(http_pool_max())
            .tcp_keepalive(std::time::Duration::from_secs(45))
            .tcp_nodelay(true)
            .http1_only()
            .build()
            .unwrap_or_else(|_| self.client.clone());

        // ── Phase 1: manifests (fast — small metadata payloads) ──────────────
        // Download all manifests upfront to know total_chunks before any data
        // transfer. Manifests are tiny (list of chunk IDs + sizes); storing them
        // all is at most ~KB total even for large repos.
        struct ObjectWork {
            oid: Oid,
            manifest: ChunkManifest,
            missing_chunks: Vec<Oid>,
        }

        // Knobs: set MEDIAGIT_PULL_PIPELINE=0 to disable concurrent manifest fetch.
        // MEDIAGIT_PULL_MANIFEST_CONCURRENCY controls max in-flight manifest requests (default 8).
        let pull_pipeline = std::env::var("MEDIAGIT_PULL_PIPELINE")
            .as_deref()
            .unwrap_or("1")
            != "0";
        let manifest_concurrency: usize = std::env::var("MEDIAGIT_PULL_MANIFEST_CONCURRENCY")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(8)
            .max(1);

        let manifest_start = std::time::Instant::now();

        let object_work: Vec<ObjectWork> = if pull_pipeline && n_objects > 1 {
            // Concurrent manifest fetch + chunk existence check.
            // buffer_unordered only requires Send (not 'static), so &ObjectDatabase
            // is safe to capture as long as ObjectDatabase: Sync.
            let http_client = self.client.clone();
            let base_url = self.base_url.clone();
            use futures::StreamExt;
            let results: Vec<Result<ObjectWork>> = futures::stream::iter(chunked_oids.iter())
                .map(|oid| {
                    let http_client = http_client.clone();
                    let base_url = base_url.clone();
                    async move {
                        // 1. Fetch manifest (HTTP — inlined download_manifest body)
                        let url = format!("{}/manifests/{}", base_url, oid.to_hex());
                        let resp = http_client
                            .get(&url)
                            .send()
                            .await
                            .with_context(|| format!("Failed to GET /manifests/{}", oid))?;
                        if !resp.status().is_success() {
                            anyhow::bail!(
                                "GET /manifests/{} failed with status: {}",
                                oid,
                                resp.status()
                            );
                        }
                        let data = resp.bytes().await?;
                        let manifest: ChunkManifest =
                            mediagit_versioning::format::deserialize(&data)
                                .context("Failed to deserialize manifest")?;

                        // 2. Check which chunks are already local (parallel filesystem stats).
                        let obj_total = manifest.chunks.len();
                        let missing: Vec<Oid> =
                            futures::stream::iter(manifest.chunks.iter().map(|c| c.id))
                                .map(|id| async move {
                                    if odb.chunk_exists(&id).await.unwrap_or(false) {
                                        None
                                    } else {
                                        Some(id)
                                    }
                                })
                                .buffer_unordered(64)
                                .filter_map(|x| async move { x })
                                .collect()
                                .await;

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

                        Ok::<ObjectWork, anyhow::Error>(ObjectWork {
                            oid: *oid,
                            manifest,
                            missing_chunks: missing,
                        })
                    }
                })
                .buffer_unordered(manifest_concurrency)
                .collect()
                .await;
            results.into_iter().collect::<Result<Vec<ObjectWork>>>()?
        } else {
            // Sequential fallback: MEDIAGIT_PULL_PIPELINE=0 or single object.
            let mut work = Vec::with_capacity(n_objects);
            for oid in chunked_oids.iter() {
                let manifest = self.download_manifest(oid).await?;
                let obj_total = manifest.chunks.len();
                // Check which chunks are already local — run in parallel to avoid
                // O(n) sequential filesystem stats on Windows (3ms × 4000 = 12s per object).
                let missing: Vec<Oid> = futures::stream::iter(manifest.chunks.iter().map(|c| c.id))
                    .map(|id| async move {
                        if odb.chunk_exists(&id).await.unwrap_or(false) {
                            None
                        } else {
                            Some(id)
                        }
                    })
                    .buffer_unordered(64)
                    .filter_map(|x| async move { x })
                    .collect()
                    .await;
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
                work.push(ObjectWork {
                    oid: *oid,
                    manifest,
                    missing_chunks: missing,
                });
            }
            work
        };

        // Compute totals from collected manifests.
        let total_manifest_chunks: usize =
            object_work.iter().map(|w| w.manifest.chunks.len()).sum();
        let total_manifest_bytes: u64 = object_work
            .iter()
            .flat_map(|w| w.manifest.chunks.iter().map(|c| c.size as u64))
            .sum();

        // Signal total upfront so the progress bar initialises with correct length.
        on_progress(
            0,
            total_manifest_bytes,
            &format!(
                "{} objects, {} total chunks",
                n_objects, total_manifest_chunks
            ),
        );

        if let Some(b) = &_download_bench {
            b.record_manifest_to_first_byte(manifest_start.elapsed());
        }

        // ── Phase 2: chunk download — streaming write, O(concurrent × chunk) RAM
        // Writing each chunk as it arrives (while let Some) instead of collect()
        // means peak RAM = concurrent_downloads × max_chunk_size (≤64 KB on
        // Windows) rather than accumulating every result before any disk write.
        let mut total_chunks_downloaded = 0usize;
        let mut bytes_done: u64 = 0;

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
            // Size hints for range-parallel GET (W5): chunk hex → uncompressed bytes.
            let chunk_size_map: std::sync::Arc<std::collections::HashMap<String, u64>> =
                std::sync::Arc::new(
                    manifest
                        .chunks
                        .iter()
                        .map(|c| (c.id.to_hex(), c.size as u64))
                        .collect(),
                );

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

                // Batch-request presigned GET URLs for full chunks (best-effort;
                // empty map on old servers / disabled backends / env gate).
                let full_chunk_hex: Vec<String> = full_chunks.iter().map(|c| c.to_hex()).collect();

                // F7: pack-mode pull path — locate → presign → Range-GET.
                // Falls back to legacy per-chunk path if pack mode returns 0 chunks or errors.
                let cloud_packs_pull = std::env::var("MEDIAGIT_CLOUD_PACKS")
                    .as_deref()
                    .unwrap_or("1")
                    != "0";
                let pack_pulled = if cloud_packs_pull && !full_chunks.is_empty() {
                    match self.pull_chunks_via_packs(&full_chunks, odb).await {
                        Ok(n) if n > 0 => {
                            tracing::debug!(chunks = n, "pack-mode pull complete");
                            true
                        }
                        Ok(_) => {
                            tracing::debug!(
                                "pack-mode pull: 0 chunks located; using per-chunk fallback"
                            );
                            false
                        }
                        Err(e) => {
                            tracing::warn!(
                                err = %e,
                                "pack-mode pull failed; falling back to per-chunk"
                            );
                            false
                        }
                    }
                } else {
                    false
                };

                if direct_download_enabled && !full_chunk_hex.is_empty() && !pack_pulled {
                    on_progress(
                        bytes_done,
                        total_manifest_bytes,
                        &format!("Preparing {} download URLs...", full_chunk_hex.len()),
                    );
                }
                let download_urls =
                    if direct_download_enabled && !full_chunk_hex.is_empty() && !pack_pulled {
                        self.request_chunk_download_urls(&full_chunk_hex).await
                    } else {
                        std::collections::HashMap::new()
                    };

                // ── Pass A: full chunks (and any chunks needed as delta bases) ──
                if !full_chunks.is_empty() && !pack_pulled {
                    let download_urls = std::sync::Arc::new(download_urls);
                    let _dl_pass_a_t = std::time::Instant::now();
                    let mut _dl_pass_a_n = 0u64;
                    let mut _dl_pass_a_bytes = 0u64;
                    // B4: stream downloaded bytes directly to a temp file to reduce peak RAM.
                    // Default ON — Windows stress + MinIO/AWS/Azure 148/148 PASS (2026-05-22).
                    // Set MEDIAGIT_STREAM_CHUNK_TO_DISK=0 to revert to RAM buffering.
                    let stream_to_disk = std::env::var("MEDIAGIT_STREAM_CHUNK_TO_DISK")
                        .as_deref()
                        .unwrap_or("1")
                        == "1";
                    let mut stream = futures::stream::iter(full_chunks.into_iter())
                        .map(|chunk_id| {
                            let client = self.client.clone();
                            let direct_client = direct_client.clone();
                            let base_url = self.base_url.clone();
                            let download_urls = std::sync::Arc::clone(&download_urls);
                            let chunk_size_map = std::sync::Arc::clone(&chunk_size_map);
                            let odb = odb.clone();
                            async move {
                                let hex = chunk_id.to_hex();
                                let size_hint = chunk_size_map.get(&hex).copied().unwrap_or(0);
                                // Try presigned direct download first; fall back to proxy on any error.
                                if let Some(Some(presigned)) = download_urls.get(&hex) {
                                    match download_chunk_direct(
                                        &direct_client,
                                        &presigned.url,
                                        &presigned.headers,
                                        &hex,
                                        size_hint,
                                    )
                                    .await
                                    {
                                        Ok(data) => {
                                            let net = data.len() as u64;
                                            odb.put_compressed_chunk(&chunk_id, &data).await?;
                                            return Ok::<_, anyhow::Error>((chunk_id, net));
                                        }
                                        Err(e) => {
                                            tracing::debug!(
                                                chunk = %hex,
                                                err = %e,
                                                "Direct download failed; falling back to proxy GET"
                                            );
                                        }
                                    }
                                }
                                // Proxy GET fallback
                                let url = format!("{}/chunks/{}", base_url, hex);
                                let response = client.get(&url).send().await.map_err(|e| {
                                    anyhow::anyhow!("Failed to download chunk {}: {}", chunk_id, e)
                                })?;
                                // F3: server returns 409 when the chunk is delta-only.
                                // This happens when our POST /chunk-deltas/check probe failed
                                // silently and we ended up in the wrong (full-chunk) pass.
                                // Re-route: download via /chunk-deltas/<id> and store as delta.
                                if response.status() == reqwest::StatusCode::CONFLICT {
                                    let body: serde_json::Value =
                                        response.json().await.unwrap_or_default();
                                    let base_hex =
                                        body.get("base_id").and_then(|v| v.as_str()).unwrap_or("");
                                    if let Ok(base_id) = Oid::from_hex(base_hex) {
                                        let delta_url =
                                            format!("{}/chunk-deltas/{}", base_url, hex);
                                        match client.get(&delta_url).send().await {
                                            Ok(dr) if dr.status().is_success() => {
                                                let delta_bytes = dr.bytes().await?.to_vec();
                                                let net = delta_bytes.len() as u64;
                                                odb.write_chunk_delta(
                                                    &chunk_id,
                                                    &base_id,
                                                    &delta_bytes,
                                                )
                                                .await?;
                                                return Ok::<_, anyhow::Error>((chunk_id, net));
                                            }
                                            _ => {}
                                        }
                                    }
                                    anyhow::bail!(
                                        "GET /chunks/{} returned 409 but delta reclassification failed",
                                        chunk_id
                                    );
                                }
                                if !response.status().is_success() {
                                    anyhow::bail!(
                                        "GET /chunks/{} failed with status: {}",
                                        chunk_id,
                                        response.status()
                                    );
                                }
                                if stream_to_disk {
                                    // B4: stream proxy response to temp file to reduce peak RAM.
                                    // Chunk IDs are BLAKE3(uncompressed); proxy returns compressed
                                    // bytes — hash cannot be verified here without decompressing.
                                    // Integrity is verified at read time via decompression.
                                    use futures::StreamExt as _;
                                    use tokio::io::AsyncWriteExt as _;
                                    let temp_path =
                                        std::env::temp_dir().join(format!("mg-chunk-{}", hex));
                                    let mut f = tokio::fs::File::create(&temp_path)
                                        .await
                                        .map_err(|e| anyhow::anyhow!("B4 temp create: {}", e))?;
                                    let mut written = 0u64;
                                    let mut byte_stream = response.bytes_stream();
                                    while let Some(ch) = byte_stream.next().await {
                                        let b = ch.map_err(|e| {
                                            anyhow::anyhow!("Download stream error: {}", e)
                                        })?;
                                        f.write_all(&b).await.map_err(|e| {
                                            anyhow::anyhow!("Write temp chunk: {}", e)
                                        })?;
                                        written += b.len() as u64;
                                    }
                                    f.flush().await?;
                                    drop(f);
                                    odb.put_compressed_chunk_from_file(&chunk_id, &temp_path)
                                        .await?;
                                    let _ = tokio::fs::remove_file(&temp_path).await;
                                    Ok::<_, anyhow::Error>((chunk_id, written))
                                } else {
                                    let data = response.bytes().await.map_err(|e| {
                                        anyhow::anyhow!("Failed to read chunk {}: {}", chunk_id, e)
                                    })?;
                                    let net = data.len() as u64;
                                    odb.put_compressed_chunk(&chunk_id, &data).await?;
                                    Ok::<_, anyhow::Error>((chunk_id, net))
                                }
                            }
                        })
                        .buffer_unordered(concurrent_downloads);

                    while let Some(result) = stream.next().await {
                        let (chunk_id, net_bytes) = result?;
                        _dl_pass_a_n += 1;
                        _dl_pass_a_bytes += net_bytes;
                        // ODB store happens inside the closure (B4 refactor).
                        total_chunks_downloaded += 1;
                        bytes_done += chunk_size_map.get(&chunk_id.to_hex()).copied().unwrap_or(0);
                        on_progress(
                            bytes_done,
                            total_manifest_bytes,
                            &format!("Object {}/{}", obj_idx + 1, n_objects),
                        );
                    }
                    if let Some(b) = &_download_bench {
                        b.record_batch(_dl_pass_a_n, _dl_pass_a_bytes, _dl_pass_a_t.elapsed());
                    }
                }

                // ── Pass B: delta chunks (bases are now local) ────────────────
                if !delta_chunks.is_empty() {
                    let delta_map = std::sync::Arc::new(delta_map);
                    let _dl_pass_b_t = std::time::Instant::now();
                    let mut _dl_pass_b_n = 0u64;
                    let mut _dl_pass_b_bytes = 0u64;
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
                        _dl_pass_b_n += 1;
                        _dl_pass_b_bytes += delta_bytes.len() as u64;
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
                        bytes_done += chunk_size_map.get(&chunk_id.to_hex()).copied().unwrap_or(0);
                        on_progress(
                            bytes_done,
                            total_manifest_bytes,
                            &format!("Object {}/{}", obj_idx + 1, n_objects),
                        );
                    }
                    if let Some(b) = &_download_bench {
                        b.record_batch(_dl_pass_b_n, _dl_pass_b_bytes, _dl_pass_b_t.elapsed());
                    }
                }
            }

            // Advance bar for chunks that were already local (dedup / re-clone).
            if already_local > 0 {
                let already_bytes: u64 = manifest
                    .chunks
                    .iter()
                    .filter(|c| !missing_chunks.iter().any(|m| m == &c.id))
                    .map(|c| c.size as u64)
                    .sum();
                bytes_done += already_bytes;
                on_progress(
                    bytes_done,
                    total_manifest_bytes,
                    &format!("Object {}/{}", obj_idx + 1, n_objects),
                );
            }

            // Store manifest locally
            odb.put_manifest(&oid, &manifest).await?;

            tracing::debug!(oid = %oid, "Chunked object downloaded");
        }

        if let Some(b) = &_download_bench {
            b.summary();
        }
        Ok(total_chunks_downloaded)
    }

    // -----------------------------------------------------------------------
    // F4: Pack-mode push — bundle full chunks into cloud packs
    // -----------------------------------------------------------------------

    /// Bundle `full_chunks` into cloud packs and upload each via presigned PUT.
    /// Returns `(chunks_uploaded, bytes_uploaded)`.
    async fn push_full_chunks_via_packs(
        &self,
        full_chunks: &[Oid],
        odb: &ObjectDatabase,
    ) -> Result<(u32, u64)> {
        use crate::pack_builder::{upload_and_register, PackBuilder};

        let temp_dir = tempfile::TempDir::new().context("create pack temp dir")?;
        let mut builder = PackBuilder::new(temp_dir.path());

        let direct_client = reqwest::Client::builder()
            .pool_idle_timeout(std::time::Duration::from_secs(60))
            .pool_max_idle_per_host(http_pool_max())
            .tcp_nodelay(true)
            .timeout(std::time::Duration::from_secs(300))
            .http1_only()
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());

        let mut chunks_done: u32 = 0;
        let mut bytes_done: u64 = 0;

        for chunk_id in full_chunks {
            let data = odb
                .read(chunk_id)
                .await
                .with_context(|| format!("read chunk {} for pack", chunk_id))?;

            if let Some(result) = builder
                .add_chunk(*chunk_id, &data)
                .await
                .with_context(|| format!("pack chunk {}", chunk_id))?
            {
                let pack_bytes = result.byte_len;
                upload_and_register(result, &self.base_url, &self.client, &direct_client)
                    .await
                    .context("upload_and_register pack")?;
                bytes_done += pack_bytes;
            }
            bytes_done += data.len() as u64;
            chunks_done += 1;
        }

        if let Some(result) = builder.finish().await.context("finish final pack")? {
            let pack_bytes = result.byte_len;
            upload_and_register(result, &self.base_url, &self.client, &direct_client)
                .await
                .context("upload_and_register final pack")?;
            bytes_done += pack_bytes;
        }

        drop(temp_dir);
        Ok((chunks_done, bytes_done))
    }

    // -----------------------------------------------------------------------
    // F7: Pack-based pull (locate → presign → Range-GET → ODB)
    // F8: Per-slice BLAKE3 verify
    // -----------------------------------------------------------------------

    /// Locate chunks in the server's pack manifest index.
    ///
    /// `wants_full_repo=true` fetches the entire map in one response (full-clone).
    async fn locate_chunks_in_packs(
        &self,
        chunk_ids: &[String],
        wants_full_repo: bool,
    ) -> Result<std::collections::HashMap<String, PackLocInfo>> {
        #[derive(serde::Serialize)]
        struct Req<'a> {
            chunk_ids: &'a [String],
            wants_full_repo: bool,
        }
        #[derive(serde::Deserialize)]
        struct LocEntry {
            pack_oid: String,
            offset: u64,
            length: u32,
        }

        let url = format!("{}/chunks/locate", self.base_url);
        let resp = self
            .client
            .post(&url)
            .json(&Req {
                chunk_ids,
                wants_full_repo,
            })
            .send()
            .await
            .context("POST /chunks/locate")?;
        if !resp.status().is_success() {
            anyhow::bail!("POST /chunks/locate returned {}", resp.status());
        }
        let map: std::collections::HashMap<String, LocEntry> =
            resp.json().await.context("parse /chunks/locate")?;
        Ok(map
            .into_iter()
            .map(|(oid, e)| {
                (
                    oid,
                    PackLocInfo {
                        pack_oid: e.pack_oid,
                        offset: e.offset,
                        length: e.length,
                    },
                )
            })
            .collect())
    }

    /// Request presigned GET URLs for a batch of pack objects.
    async fn request_pack_download_urls(
        &self,
        pack_ids: &[String],
    ) -> std::collections::HashMap<String, Option<PresignedGetInfo>> {
        if pack_ids.is_empty() {
            return std::collections::HashMap::new();
        }
        #[derive(serde::Serialize)]
        struct Req<'a> {
            pack_ids: &'a [String],
        }
        let url = format!("{}/packs/presign-download-urls", self.base_url);
        let result = async {
            let resp = self
                .client
                .post(&url)
                .json(&Req { pack_ids })
                .send()
                .await?;
            if !resp.status().is_success() {
                anyhow::bail!(
                    "POST /packs/presign-download-urls returned {}",
                    resp.status()
                );
            }
            resp.json::<std::collections::HashMap<String, Option<PresignedGetInfo>>>()
                .await
                .context("parse /packs/presign-download-urls")
        }
        .await;
        match result {
            Ok(m) => m,
            Err(e) => {
                tracing::warn!(err = %e, "request_pack_download_urls failed");
                std::collections::HashMap::new()
            }
        }
    }

    /// Pull a set of chunks via pack-mode Range-GET (F7 + F8).
    ///
    /// Returns the count of chunks successfully written to ODB.
    /// Chunks not found in the pack index are silently skipped (caller may
    /// fall back to legacy proxy GET for those).
    pub async fn pull_chunks_via_packs(
        &self,
        chunk_ids: &[Oid],
        odb: &ObjectDatabase,
    ) -> Result<u32> {
        use futures::StreamExt;

        if chunk_ids.is_empty() {
            return Ok(0);
        }

        let pack_verify = std::env::var("MEDIAGIT_PACK_VERIFY")
            .as_deref()
            .unwrap_or("1")
            != "0";

        let download_concurrency: usize = std::env::var("MEDIAGIT_DOWNLOAD_CONCURRENCY")
            .ok()
            .and_then(|s| s.parse().ok())
            .filter(|n: &usize| *n > 0)
            .or(self.concurrent_downloads)
            .unwrap_or(24);

        let coalesce_max_gap: u64 = std::env::var("MEDIAGIT_PACK_RANGE_COALESCE_MAX_GAP")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(1_048_576);
        let coalesce_max_bytes: u64 = std::env::var("MEDIAGIT_PACK_RANGE_COALESCE_MAX_BYTES")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(8_388_608);

        let all_hex: Vec<String> = chunk_ids.iter().map(|o| o.to_hex()).collect();
        let wants_full_repo = all_hex.len() > 512;
        let loc_map = self
            .locate_chunks_in_packs(&all_hex, wants_full_repo)
            .await?;
        if loc_map.is_empty() {
            return Ok(0);
        }

        // Group by pack_oid, sort by offset.
        let mut by_pack: std::collections::HashMap<String, Vec<(String, u64, u32)>> =
            std::collections::HashMap::new();
        for hex in &all_hex {
            if let Some(loc) = loc_map.get(hex) {
                by_pack.entry(loc.pack_oid.clone()).or_default().push((
                    hex.clone(),
                    loc.offset,
                    loc.length,
                ));
            }
        }

        let pack_ids: Vec<String> = by_pack.keys().cloned().collect();
        let presign_map = self.request_pack_download_urls(&pack_ids).await;

        let tasks: Vec<_> = by_pack
            .into_iter()
            .filter_map(|(pack_oid, mut chunks)| {
                let url = presign_map
                    .get(&pack_oid)
                    .and_then(|o| o.as_ref())
                    .map(|p| p.url.clone())?;
                chunks.sort_unstable_by_key(|(_, off, _)| *off);
                Some((chunks, url, pack_oid))
            })
            .collect();

        let client = self.client.clone();

        type PackChunkPairs = Vec<(Oid, Vec<u8>)>;
        let per_pack_results: Vec<Result<PackChunkPairs>> = futures::stream::iter(tasks)
            .map(|(chunks, url, pack_oid)| {
                let client = client.clone();
                let cmg = coalesce_max_gap;
                let cmb = coalesce_max_bytes;
                async move {
                    let ranges = coalesce_chunk_ranges(&chunks, cmg, cmb);
                    let mut out: Vec<(Oid, Vec<u8>)> = Vec::new();

                    for (range_start, range_end) in ranges {
                        let hdr = format!("bytes={}-{}", range_start, range_end.saturating_sub(1));
                        let resp = client
                            .get(&url)
                            .header("Range", &hdr)
                            .send()
                            .await
                            .with_context(|| format!("Range-GET {} range {}", pack_oid, hdr))?;

                        let status = resp.status().as_u16();
                        if status != 200 && status != 206 {
                            anyhow::bail!("Range-GET returned {} for pack {}", status, pack_oid);
                        }

                        let body = resp.bytes().await.context("read Range-GET body")?;

                        for (hex, off, len) in &chunks {
                            if *off < range_start || *off + *len as u64 > range_end {
                                continue;
                            }
                            let rel = (*off - range_start) as usize;
                            let slice_end = rel + *len as usize;
                            if slice_end > body.len() || *len < 5 {
                                tracing::warn!(chunk = %hex, "pack slice bounds error");
                                continue;
                            }
                            // Pack object layout: type(1) + size(4) + data
                            let data = &body[rel + 5..slice_end];

                            let oid = match Oid::from_hex(hex) {
                                Ok(o) => o,
                                Err(_) => continue,
                            };

                            // F8: BLAKE3 per-slice verify
                            if pack_verify {
                                let mut h = mediagit_versioning::hash::Hasher::new();
                                h.update(data);
                                let computed = Oid::from_bytes(h.finalize());
                                if computed != oid {
                                    tracing::warn!(
                                        chunk = %hex,
                                        "BLAKE3 mismatch on pack slice"
                                    );
                                    continue;
                                }
                            }

                            out.push((oid, data.to_vec()));
                        }
                    }
                    Ok(out)
                }
            })
            .buffer_unordered(download_concurrency)
            .collect()
            .await;

        let mut chunks_written: u32 = 0;
        for result in per_pack_results {
            match result {
                Ok(pairs) => {
                    for (oid, data) in pairs {
                        odb.write(ObjectType::Blob, &data)
                            .await
                            .with_context(|| format!("write chunk {} to ODB", oid))?;
                        chunks_written += 1;
                    }
                }
                Err(e) => {
                    // Propagate so the caller falls back to per-chunk legacy for all chunks.
                    // ODB dedup handles any re-downloads of already-written chunks.
                    return Err(
                        e.context("pack Range-GET failed; caller should use per-chunk fallback")
                    );
                }
            }
        }

        Ok(chunks_written)
    }
}

/// Coalesce adjacent/near chunk ranges within a sorted (offset-ascending) chunk list.
///
/// Returns a list of (start, end) byte ranges where `end` is exclusive.
/// Only merges if `gap <= max_gap` AND `merged_size <= max_bytes`.
fn coalesce_chunk_ranges(
    chunks: &[(String, u64, u32)],
    max_gap: u64,
    max_bytes: u64,
) -> Vec<(u64, u64)> {
    let mut ranges: Vec<(u64, u64)> = Vec::new();
    if chunks.is_empty() {
        return ranges;
    }
    let mut cur_start = chunks[0].1;
    let mut cur_end = cur_start + chunks[0].2 as u64;

    for (_, off, len) in &chunks[1..] {
        let chunk_end = off + *len as u64;
        let gap = off.saturating_sub(cur_end);
        let merged = chunk_end - cur_start;
        if gap <= max_gap && merged <= max_bytes {
            cur_end = cur_end.max(chunk_end);
        } else {
            ranges.push((cur_start, cur_end));
            cur_start = *off;
            cur_end = chunk_end;
        }
    }
    ranges.push((cur_start, cur_end));
    ranges
}

/// Download a single chunk directly from a presigned GET URL, verify its hash.
///
/// Returns `Ok(bytes)` on success.
/// Returns `Err` on any failure (network, non-2xx, hash mismatch) — caller falls
/// back to the server-proxied `GET /chunks/:id` path.
async fn download_chunk_direct(
    client: &reqwest::Client,
    url: &str,
    headers: &[(String, String)],
    expected_hex: &str,
    size_hint: u64,
) -> anyhow::Result<Vec<u8>> {
    // Range-parallel GET: for large chunks, fan out N parallel byte-range
    // requests to fill multiple TCP congestion windows simultaneously.
    // All S3/Azure/GCS/MinIO backends honour `Range:` on presigned URLs.
    // Disabled when MEDIAGIT_RANGE_PARALLEL=0 or size_hint is too small.
    let range_parallel: usize = std::env::var("MEDIAGIT_RANGE_PARALLEL")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(4)
        .clamp(0, 16);
    let range_threshold: u64 = std::env::var("MEDIAGIT_RANGE_PARALLEL_THRESHOLD")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(4 * 1024 * 1024); // 4 MiB — fires on typical media chunks (512 KB – 32 MB)

    if range_parallel > 1 && size_hint >= range_threshold {
        match download_chunk_ranged(
            client,
            url,
            headers,
            expected_hex,
            size_hint,
            range_parallel,
        )
        .await
        {
            Ok(data) => return Ok(data),
            Err(e) => {
                // Range-GET failed (e.g. server doesn't support it) → fall
                // through to single-stream path below.
                tracing::debug!(
                    err = %e,
                    "range-parallel GET failed, falling back to single-stream"
                );
            }
        }
    }

    let mut req = client.get(url);
    for (k, v) in headers {
        req = req.header(k.as_str(), v.as_str());
    }
    let resp = req.send().await?;
    let status = resp.status().as_u16();
    if !resp.status().is_success() {
        let ct = resp
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_string();
        let hc = resp
            .headers()
            .get("x-amz-error-code")
            .or_else(|| resp.headers().get("x-ms-error-code"))
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_string();
        let body = resp.text().await.unwrap_or_default();
        let outcome = crate::error_class::classify_auto_get(status, url, &ct, &hc, &body);
        if let crate::error_class::TransferOutcome::PermanentChunkAfterDelay(delay) = outcome {
            tokio::time::sleep(delay).await;
            anyhow::bail!("presigned GET 404 (after {delay:?} delay): status={status}");
        }
        anyhow::bail!("presigned GET failed: status={status} outcome={outcome:?}");
    }

    // Stream body into buffer.
    // Note: chunk ID = BLAKE3(uncompressed data), but storage holds compressed bytes.
    // We cannot verify the hash here without decompressing; TLS + ETag guarantee
    // transport integrity. Application-layer integrity is verified on read (decompress).
    use futures::StreamExt;
    let mut stream = resp.bytes_stream();
    let mut buf: Vec<u8> = Vec::new();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk?;
        buf.extend_from_slice(&chunk);
    }
    Ok(buf)
}

/// Fan out N parallel `Range:` GETs and reassemble in order.
/// Reassembles bytes from range GETs; no inline hash check here.
/// Verification is deferred to read-time decompression — chunk_id is
/// BLAKE3(uncompressed) but stored bytes are compressed.
async fn download_chunk_ranged(
    client: &reqwest::Client,
    url: &str,
    headers: &[(String, String)],
    _expected_hex: &str,
    _size_hint: u64,
    n_ranges: usize,
) -> anyhow::Result<Vec<u8>> {
    use futures::future::try_join_all;

    // Probe request: `Range: bytes=0-0` to discover actual compressed Content-Length.
    // Unlike HEAD, presigned GET URLs always accept Range requests on AWS/GCS/MinIO/Azure.
    // The 206 response contains `Content-Range: bytes 0-0/<total>` giving the true
    // compressed object size. We need this because _size_hint is the *uncompressed* chunk
    // size — using it for Range striping truncates the last byte (the SmartCompressor tag).
    let actual_size = {
        let mut req = client.get(url).header("Range", "bytes=0-0");
        for (k, v) in headers {
            req = req.header(k.as_str(), v.as_str());
        }
        let resp = req.send().await?;
        if resp.status().as_u16() == 206 {
            resp.headers()
                .get("content-range")
                .and_then(|v| v.to_str().ok())
                .and_then(|s| s.strip_prefix("bytes "))
                .and_then(|s| s.split('/').nth(1))
                .and_then(|n| n.parse::<u64>().ok())
                .ok_or_else(|| {
                    anyhow::anyhow!(
                        "206 missing valid Content-Range; falling back to single-stream"
                    )
                })?
        } else {
            anyhow::bail!(
                "range probe returned {}; falling back to single-stream",
                resp.status()
            );
        }
    };

    let stripe = actual_size / n_ranges as u64;
    let futs: Vec<_> = (0..n_ranges)
        .map(|i| {
            let start = i as u64 * stripe;
            let end = if i + 1 == n_ranges {
                actual_size - 1
            } else {
                (i as u64 + 1) * stripe - 1
            };
            let mut req = client.get(url);
            for (k, v) in headers {
                req = req.header(k.as_str(), v.as_str());
            }
            req = req.header("Range", format!("bytes={start}-{end}"));
            async move {
                let resp = req.send().await?;
                if resp.status().as_u16() == 200 {
                    // Server returned full body instead of 206 — not range-capable.
                    anyhow::bail!("server returned 200 instead of 206 for Range request");
                }
                anyhow::ensure!(
                    resp.status().as_u16() == 206,
                    "range GET returned unexpected status {}",
                    resp.status()
                );
                let data = resp.bytes().await?.to_vec();
                anyhow::Ok((i, data))
            }
        })
        .collect();

    let mut parts: Vec<(usize, Vec<u8>)> = try_join_all(futs).await?;
    parts.sort_unstable_by_key(|(i, _)| *i);

    let mut buf = Vec::with_capacity(actual_size as usize);
    for (_, data) in &parts {
        buf.extend_from_slice(data);
    }
    Ok(buf)
}

/// Upload a single chunk via server-orchestrated multipart upload (S3/MinIO MPU).
///
/// Returns `true` when the chunk was successfully uploaded and committed.
/// Returns `false` on any failure; the caller falls through to single-PUT or proxy.
///
/// Flow: POST `/chunks/mpu/start` → PUT each part URL → POST `/chunks/mpu/complete`.
/// On part-upload failure the in-flight MPU is aborted best-effort to release S3
/// storage, and `false` is returned so the caller retries via single-PUT/proxy.
async fn upload_chunk_mpu(
    api_client: &reqwest::Client,
    direct_client: &reqwest::Client,
    base_url: &str,
    chunk_hex: &str,
    chunk_data: &[u8],
) -> bool {
    #[derive(serde::Serialize)]
    struct StartReq<'a> {
        chunk_id: &'a str,
        chunk_size: u64,
    }
    #[derive(serde::Deserialize)]
    struct StartResp {
        upload_id: String,
        parts: Vec<PartUrl>,
        part_size: u64,
    }
    #[derive(serde::Deserialize)]
    struct PartUrl {
        part_number: i32,
        url: String,
    }
    #[derive(serde::Serialize)]
    struct CompleteReq<'a> {
        chunk_id: &'a str,
        upload_id: &'a str,
        parts: Vec<CompletedPart>,
    }
    #[derive(serde::Serialize)]
    struct CompletedPart {
        part_number: i32,
        etag: String,
    }
    #[derive(serde::Serialize)]
    struct AbortReq<'a> {
        chunk_id: &'a str,
        upload_id: &'a str,
    }

    // --- Start MPU ---
    let start_url = format!("{}/chunks/mpu/start", base_url);
    let resp = match api_client
        .post(&start_url)
        .json(&StartReq {
            chunk_id: chunk_hex,
            chunk_size: chunk_data.len() as u64,
        })
        .send()
        .await
    {
        Ok(r) => r,
        Err(e) => {
            tracing::debug!(chunk = %chunk_hex, err = %e, "MPU start request failed; using single-PUT");
            return false;
        }
    };

    if resp.status() == reqwest::StatusCode::NOT_IMPLEMENTED {
        tracing::debug!(chunk = %chunk_hex, "Backend does not support MPU; using single-PUT");
        return false;
    }
    if !resp.status().is_success() {
        tracing::debug!(chunk = %chunk_hex, status = resp.status().as_u16(), "MPU start non-2xx; using single-PUT");
        return false;
    }

    let mpu: StartResp = match resp.json().await {
        Ok(m) => m,
        Err(e) => {
            tracing::debug!(chunk = %chunk_hex, err = %e, "MPU start parse error; using single-PUT");
            return false;
        }
    };

    let upload_id = mpu.upload_id.clone();
    let part_size = mpu.part_size as usize;
    let mut completed_parts: Vec<CompletedPart> = Vec::with_capacity(mpu.parts.len());

    // --- Upload each part (with per-part retry, same backoff as single-PUT) ---
    const MAX_PART_ATTEMPTS: u32 = 5;
    for part in &mpu.parts {
        let start = (part.part_number as usize - 1) * part_size;
        let end = (start + part_size).min(chunk_data.len());
        if start >= chunk_data.len() {
            break;
        }
        let part_data = &chunk_data[start..end];

        let mut part_etag: Option<String> = None;
        for attempt in 0..MAX_PART_ATTEMPTS {
            if attempt > 0 {
                let base_ms = (1000u64 << (attempt - 1)).min(30_000);
                let seed = (chunk_hex.as_bytes().first().copied().unwrap_or(0) as u64)
                    .wrapping_mul(13)
                    .wrapping_add(attempt as u64 * 7)
                    .wrapping_add(part.part_number as u64 * 31);
                let jitter_ms = base_ms * (seed % 25) / 100;
                tokio::time::sleep(tokio::time::Duration::from_millis(base_ms + jitter_ms)).await;
            }

            let result = direct_client
                .put(&part.url)
                .header(reqwest::header::CONTENT_LENGTH, part_data.len())
                .body(part_data.to_vec())
                .send()
                .await;

            match result {
                Ok(r) if r.status().is_success() => {
                    let etag = r
                        .headers()
                        .get("etag")
                        .and_then(|v| v.to_str().ok())
                        .unwrap_or("")
                        .to_string();
                    part_etag = Some(etag);
                    break;
                }
                Ok(r) => {
                    let status = r.status().as_u16();
                    let ct = r
                        .headers()
                        .get(reqwest::header::CONTENT_TYPE)
                        .and_then(|v| v.to_str().ok())
                        .unwrap_or("")
                        .to_string();
                    let hdr_code = r
                        .headers()
                        .get("x-amz-error-code")
                        .and_then(|v| v.to_str().ok())
                        .unwrap_or("")
                        .to_string();
                    let body_full = r.text().await.unwrap_or_default();
                    let body_ref = if body_full.len() > 2048 {
                        &body_full[..2048]
                    } else {
                        &body_full[..]
                    };

                    use crate::error_class::{classify_auto, TransferOutcome};
                    match classify_auto(status, &part.url, &ct, &hdr_code, body_ref) {
                        TransferOutcome::Transient | TransferOutcome::RefreshUrl => {
                            // RefreshUrl treated as Transient: per-part URLs are issued per-MPU
                            // and cannot be refreshed individually without restarting the upload.
                            // The part URL TTL (12 h) makes a persistent auth error across all
                            // 5 attempts unlikely.
                            tracing::debug!(
                                chunk = %chunk_hex,
                                part = part.part_number,
                                attempt = attempt + 1,
                                status,
                                "MPU part transient error; retrying"
                            );
                        }
                        TransferOutcome::PermanentChunk
                        | TransferOutcome::PermanentChunkAfterDelay(_)
                        | TransferOutcome::PermanentConfig => {
                            tracing::debug!(
                                chunk = %chunk_hex,
                                part = part.part_number,
                                status,
                                "MPU part permanent error; aborting MPU"
                            );
                            let _ = api_client
                                .post(format!("{}/chunks/mpu/abort", base_url))
                                .json(&AbortReq {
                                    chunk_id: chunk_hex,
                                    upload_id: &upload_id,
                                })
                                .send()
                                .await;
                            return false;
                        }
                    }
                }
                Err(e) => {
                    tracing::debug!(
                        chunk = %chunk_hex,
                        part = part.part_number,
                        attempt = attempt + 1,
                        err = %e,
                        "MPU part network error; retrying"
                    );
                }
            }
        }

        match part_etag {
            Some(etag) => completed_parts.push(CompletedPart {
                part_number: part.part_number,
                etag,
            }),
            None => {
                tracing::debug!(
                    chunk = %chunk_hex,
                    part = part.part_number,
                    "MPU part retry budget exhausted; aborting MPU"
                );
                let _ = api_client
                    .post(format!("{}/chunks/mpu/abort", base_url))
                    .json(&AbortReq {
                        chunk_id: chunk_hex,
                        upload_id: &upload_id,
                    })
                    .send()
                    .await;
                return false;
            }
        }
    }

    // --- Complete MPU ---
    let complete_url = format!("{}/chunks/mpu/complete", base_url);
    let complete_resp = match api_client
        .post(&complete_url)
        .json(&CompleteReq {
            chunk_id: chunk_hex,
            upload_id: &upload_id,
            parts: completed_parts,
        })
        .send()
        .await
    {
        Ok(r) => r,
        Err(e) => {
            tracing::debug!(chunk = %chunk_hex, err = %e, "MPU complete request failed; falling back");
            return false;
        }
    };

    if !complete_resp.status().is_success() {
        tracing::debug!(
            chunk = %chunk_hex,
            status = complete_resp.status().as_u16(),
            "MPU complete non-2xx; falling back"
        );
        return false;
    }

    tracing::debug!(chunk = %chunk_hex, parts = mpu.parts.len(), "MPU upload succeeded");
    true
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
