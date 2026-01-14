use anyhow::{Context, Result};
use mediagit_versioning::{Commit, FileMode, ObjectDatabase, ObjectType, Oid, PackWriter, Tree};
use std::collections::{HashSet, VecDeque};

use crate::types::{RefUpdate, RefUpdateRequest, RefUpdateResponse, RefsResponse, WantRequest};

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
}

impl ProtocolClient {
    /// Create a new protocol client
    ///
    /// # Arguments
    /// * `base_url` - Base URL of the MediaGit server (e.g., "http://localhost:3000/repo")
    pub fn new(base_url: impl Into<String>) -> Self {
        Self {
            base_url: base_url.into(),
            client: reqwest::Client::new(),
        }
    }

    /// Get all refs from the remote repository
    pub async fn get_refs(&self) -> Result<RefsResponse> {
        println!("Fetching refs from {}", self.base_url);
        let url = format!("{}/info/refs", self.base_url);
        println!("Fetching url from {}", url);
        tracing::debug!("GET {}", url);

        let response = self
            .client
            .get(&url)
            .send()
            .await
            .context("Failed to send GET /info/refs")?;

        if !response.status().is_success() {
            anyhow::bail!(
                "GET /info/refs failed with status: {}",
                response.status()
            );
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
            let objects = self.collect_reachable_objects(odb, commit_oids, have_oids).await?;

            stats.objects_count = objects.len();
            stats.commits_count = objects.iter().filter(|(_, t)| matches!(t, ObjectType::Commit)).count();
            stats.trees_count = objects.iter().filter(|(_, t)| matches!(t, ObjectType::Tree)).count();
            stats.blobs_count = objects.iter().filter(|(_, t)| matches!(t, ObjectType::Blob)).count();

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
                    self.upload_chunked_objects(odb, &chunked_oids, |_, _, _| {}).await?;
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
            self.collect_reachable_objects(odb, commit_oids, have_oids).await?
        } else {
            Vec::new()
        };

        stats.objects_count = objects.len();
        stats.commits_count = objects.iter().filter(|(_, t)| matches!(t, ObjectType::Commit)).count();
        stats.trees_count = objects.iter().filter(|(_, t)| matches!(t, ObjectType::Tree)).count();
        stats.blobs_count = objects.iter().filter(|(_, t)| matches!(t, ObjectType::Blob)).count();

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
                message: format!("Packed {} objects ({} bytes)", total_objects, pack_data.len()),
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
                on_progress(PushProgress {
                    phase: PushPhase::Uploading,
                    current: 0,
                    total: chunked_oids.len() as u64,
                    message: format!("Uploading {} chunked files...", chunked_oids.len()),
                });

                let chunks_uploaded = self.upload_chunked_objects(odb, &chunked_oids, |current, total, msg| {
                    tracing::info!("Chunked upload: {}/{} - {}", current, total, msg);
                }).await?;

                on_progress(PushProgress {
                    phase: PushPhase::Uploading,
                    current: chunked_oids.len() as u64,
                    total: chunked_oids.len() as u64,
                    message: format!("Uploaded {} chunked files ({} chunks)", chunked_oids.len(), chunks_uploaded),
                });
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
    /// 
    /// Returns (pack_data, chunked_oids)
    pub async fn pull(&self, _odb: &ObjectDatabase, remote_ref: &str) -> Result<(Vec<u8>, Vec<Oid>)> {
        // Get remote refs
        let remote_refs = self.get_refs().await?;

        // Find the ref we want
        let ref_info = remote_refs
            .refs
            .iter()
            .find(|r| r.name == remote_ref)
            .ok_or_else(|| anyhow::anyhow!("Remote ref '{}' not found", remote_ref))?;

        // Request objects we don't have
        // Simplified: request the commit OID (should walk tree recursively)
        let want = vec![ref_info.oid.clone()];
        let have = Vec::new(); // TODO: compute what we already have

        self.download_pack(want, have).await
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
    pub async fn download_pack(&self, want: Vec<String>, have: Vec<String>) -> Result<(Vec<u8>, Vec<Oid>)> {
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

        // Then download the pack
        let pack_url = format!("{}/objects/pack", self.base_url);
        tracing::debug!("GET {}", pack_url);

        let response = self
            .client
            .get(&pack_url)
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

        let pack_data = response
            .bytes()
            .await
            .context("Failed to read pack data")?;

        Ok((pack_data.to_vec(), chunked_oids))
    }

    /// Update remote refs
    async fn update_refs(&self, request: RefUpdateRequest) -> Result<RefUpdateResponse> {
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
                    have_queue.push_back((oid, ObjectType::Commit));
                }
            }

            // Walk the "have" graph to mark all reachable objects as visited
            while let Some((oid, obj_type)) = have_queue.pop_front() {
                // Don't add to result - we're just marking as visited
                if let Ok(obj_data) = odb.read(&oid).await {
                    match obj_type {
                        ObjectType::Commit => {
                            if let Ok(commit) = bincode::deserialize::<Commit>(&obj_data) {
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
                            if let Ok(tree) = bincode::deserialize::<Tree>(&obj_data) {
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
                        ObjectType::Blob => {}
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
                    let commit: Commit = bincode::deserialize(&obj_data)
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
                    let tree: Tree = bincode::deserialize(&obj_data)
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
    /// Note: Chunked blobs (large files stored as chunks) are SKIPPED in packs.
    /// They should be transferred separately via manifest + chunks.
    /// 
    /// Returns: (pack_data, chunked_object_oids)
    async fn generate_pack(
        &self,
        odb: &ObjectDatabase,
        objects: Vec<(Oid, ObjectType)>,
    ) -> Result<(Vec<u8>, Vec<Oid>)> {
        let mut pack_writer = PackWriter::new();
        let mut chunked_objects: Vec<Oid> = Vec::new();

        for (oid, object_type) in objects {
            // Skip chunked blobs - they're too large to fit in memory
            // They will be transferred separately
            if object_type == ObjectType::Blob && odb.is_chunked(&oid).await.unwrap_or(false) {
                tracing::debug!(oid = %oid, "Skipping chunked blob in pack generation");
                chunked_objects.push(oid);
                continue;
            }
            
            // Read object data from ODB
            let obj_data = odb
                .read(&oid)
                .await
                .context(format!("Failed to read object {}", oid))?;

            // Add to pack with correct type
            let _offset = pack_writer.add_object(oid, object_type, &obj_data);
        }

        if !chunked_objects.is_empty() {
            tracing::info!(
                count = chunked_objects.len(),
                "Chunked objects to transfer separately"
            );
        }

        // Finalize pack
        let pack_data = pack_writer.finalize();
        Ok((pack_data, chunked_objects))
    }

    // ========================================================================
    // Chunk Transfer Methods - For efficient large file push
    // ========================================================================

    /// Check which chunks exist on the remote server
    /// 
    /// Returns list of chunk IDs that are MISSING (need to be uploaded)
    async fn check_chunks_exist(&self, chunk_ids: &[String]) -> Result<Vec<String>> {
        let url = format!("{}/chunks/check", self.base_url);
        tracing::debug!(count = chunk_ids.len(), "Checking chunk existence on remote");

        let response = self
            .client
            .post(&url)
            .json(&chunk_ids)
            .send()
            .await
            .context("Failed to POST /chunks/check")?;

        if !response.status().is_success() {
            anyhow::bail!("POST /chunks/check failed with status: {}", response.status());
        }

        response
            .json::<Vec<String>>()
            .await
            .context("Failed to parse chunks check response")
    }

    /// Upload a single chunk to the remote server
    #[allow(dead_code)]
    async fn upload_single_chunk(&self, chunk_id: &str, data: &[u8]) -> Result<()> {
        let url = format!("{}/chunks/{}", self.base_url, chunk_id);
        
        let response = self
            .client
            .put(&url)
            .body(data.to_vec())
            .send()
            .await
            .context(format!("Failed to PUT /chunks/{}", chunk_id))?;

        if !response.status().is_success() {
            anyhow::bail!("PUT /chunks/{} failed with status: {}", chunk_id, response.status());
        }

        Ok(())
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
            anyhow::bail!("PUT /manifests/{} failed with status: {}", oid, response.status());
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
        F: FnMut(usize, usize, &str),
    {
        use std::sync::Arc;
        use tokio::sync::Semaphore;

        if chunked_oids.is_empty() {
            return Ok(0);
        }

        let mut total_chunks_uploaded = 0;
        let concurrent_uploads = 8;
        let semaphore = Arc::new(Semaphore::new(concurrent_uploads));

        for (obj_idx, oid) in chunked_oids.iter().enumerate() {
            // Get manifest for this object
            let manifest = match odb.get_chunk_manifest(oid).await? {
                Some(m) => m,
                None => {
                    tracing::warn!(oid = %oid, "No manifest found for chunked object");
                    continue;
                }
            };

            let total_chunks = manifest.chunks.len();
            on_progress(obj_idx + 1, chunked_oids.len(), 
                &format!("Object {}/{}: {} chunks", obj_idx + 1, chunked_oids.len(), total_chunks));

            // Get all chunk IDs
            let chunk_ids: Vec<String> = manifest.chunks.iter()
                .map(|c| c.id.to_hex())
                .collect();

            // Check which chunks the remote needs
            let missing_chunks = self.check_chunks_exist(&chunk_ids).await?;
            
            if missing_chunks.is_empty() {
                tracing::debug!(oid = %oid, "All chunks already exist on remote");
            } else {
                tracing::info!(
                    oid = %oid, 
                    missing = missing_chunks.len(),
                    total = total_chunks,
                    "Uploading missing chunks"
                );

                // Upload missing chunks in parallel
                let missing_set: std::collections::HashSet<String> = missing_chunks.into_iter().collect();
                let mut upload_handles = Vec::new();

                for chunk_ref in &manifest.chunks {
                    if missing_set.contains(&chunk_ref.id.to_hex()) {
                        let chunk_id = chunk_ref.id;
                        let sem = Arc::clone(&semaphore);
                        let client = self.client.clone();
                        let base_url = self.base_url.clone();
                        
                        // Get compressed chunk data (no decompression needed)
                        let chunk_data = odb.get_compressed_chunk(&chunk_id).await?;

                        let handle = tokio::spawn(async move {
                            let _permit = sem.acquire().await.unwrap();
                            let url = format!("{}/chunks/{}", base_url, chunk_id.to_hex());
                            
                            client.put(&url)
                                .body(chunk_data)
                                .send()
                                .await
                                .map(|_| ())
                                .map_err(|e| anyhow::anyhow!("Failed to upload chunk {}: {}", chunk_id, e))
                        });

                        upload_handles.push(handle);
                    }
                }

                // Wait for all uploads to complete
                for handle in upload_handles {
                    handle.await??;
                    total_chunks_uploaded += 1;
                }
            }

            // Upload manifest last (ensures all chunks exist first)
            let manifest_data = bincode::serialize(&manifest)
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
    pub async fn download_manifest(&self, oid: &Oid) -> Result<mediagit_versioning::chunking::ChunkManifest> {
        let url = format!("{}/manifests/{}", self.base_url, oid.to_hex());
        
        let response = self
            .client
            .get(&url)
            .send()
            .await
            .context(format!("Failed to GET /manifests/{}", oid))?;

        if !response.status().is_success() {
            anyhow::bail!("GET /manifests/{} failed with status: {}", oid, response.status());
        }

        let data = response.bytes().await?;
        bincode::deserialize(&data).context("Failed to deserialize manifest")
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
            anyhow::bail!("GET /chunks/{} failed with status: {}", chunk_id, response.status());
        }

        Ok(response.bytes().await?.to_vec())
    }

    /// Download all chunks for chunked objects with parallel downloads
    /// 
    /// Uses 8 concurrent downloads for optimal throughput (>100MB/s target)
    pub async fn download_chunked_objects<F>(
        &self,
        odb: &ObjectDatabase,
        chunked_oids: &[Oid],
        mut on_progress: F,
    ) -> Result<usize>
    where
        F: FnMut(usize, usize, &str),
    {
        use std::sync::Arc;
        use tokio::sync::Semaphore;

        if chunked_oids.is_empty() {
            return Ok(0);
        }

        let mut total_chunks_downloaded = 0;
        let concurrent_downloads = 8;
        let semaphore = Arc::new(Semaphore::new(concurrent_downloads));

        for (obj_idx, oid) in chunked_oids.iter().enumerate() {
            // Download manifest first
            let manifest = self.download_manifest(oid).await?;
            let total_chunks = manifest.chunks.len();

            on_progress(obj_idx + 1, chunked_oids.len(), 
                &format!("Object {}/{}: {} chunks", obj_idx + 1, chunked_oids.len(), total_chunks));

            // Check which chunks we already have locally
            let mut missing_chunks: Vec<Oid> = Vec::new();
            for chunk_ref in &manifest.chunks {
                if !odb.chunk_exists(&chunk_ref.id).await.unwrap_or(false) {
                    missing_chunks.push(chunk_ref.id);
                }
            }

            if missing_chunks.is_empty() {
                tracing::debug!(oid = %oid, "All chunks already exist locally");
            } else {
                tracing::info!(
                    oid = %oid,
                    missing = missing_chunks.len(),
                    total = total_chunks,
                    "Downloading missing chunks"
                );

                // Download missing chunks in parallel
                let mut download_handles = Vec::new();

                for chunk_id in missing_chunks {
                    let sem = Arc::clone(&semaphore);
                    let client = self.client.clone();
                    let base_url = self.base_url.clone();

                    let handle = tokio::spawn(async move {
                        let _permit = sem.acquire().await.unwrap();
                        let url = format!("{}/chunks/{}", base_url, chunk_id.to_hex());
                        
                        let response = client.get(&url)
                            .send()
                            .await
                            .map_err(|e| anyhow::anyhow!("Failed to download chunk {}: {}", chunk_id, e))?;
                        
                        if !response.status().is_success() {
                            anyhow::bail!("GET /chunks/{} failed with status: {}", chunk_id, response.status());
                        }
                        
                        let data = response.bytes().await
                            .map_err(|e| anyhow::anyhow!("Failed to read chunk {}: {}", chunk_id, e))?;
                        
                        Ok::<_, anyhow::Error>((chunk_id, data.to_vec()))
                    });

                    download_handles.push(handle);
                }

                // Wait for all downloads and store chunks
                for handle in download_handles {
                    let (chunk_id, chunk_data) = handle.await??;
                    odb.put_compressed_chunk(&chunk_id, &chunk_data).await?;
                    total_chunks_downloaded += 1;
                }
            }

            // Store manifest locally
            odb.put_manifest(oid, &manifest).await?;
            
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
