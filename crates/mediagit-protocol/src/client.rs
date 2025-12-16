use anyhow::{Context, Result};
use mediagit_versioning::{Commit, FileMode, ObjectDatabase, ObjectType, Oid, PackWriter, Tree};
use std::collections::{HashSet, VecDeque};

use crate::types::{RefUpdate, RefUpdateRequest, RefUpdateResponse, RefsResponse, WantRequest};

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
    pub async fn push(
        &self,
        odb: &ObjectDatabase,
        updates: Vec<RefUpdate>,
        force: bool,
    ) -> Result<RefUpdateResponse> {
        // Collect commit OIDs from ref updates
        let mut commit_oids = Vec::new();
        for update in &updates {
            let oid = Oid::from_hex(&update.new_oid)
                .context(format!("Invalid OID in update: {}", update.new_oid))?;
            commit_oids.push(oid);
        }

        // Collect all reachable objects (commits, trees, blobs)
        if !commit_oids.is_empty() {
            let objects = self.collect_reachable_objects(odb, commit_oids).await?;

            tracing::info!(
                "Collected {} objects for push ({} commits, {} trees, {} blobs)",
                objects.len(),
                objects
                    .iter()
                    .filter(|(_, t)| matches!(t, ObjectType::Commit))
                    .count(),
                objects
                    .iter()
                    .filter(|(_, t)| matches!(t, ObjectType::Tree))
                    .count(),
                objects
                    .iter()
                    .filter(|(_, t)| matches!(t, ObjectType::Blob))
                    .count()
            );

            // Generate and upload pack file with complete object graph
            let pack_data = self.generate_pack(odb, objects).await?;
            self.upload_pack(&pack_data).await?;
        }

        // Update refs
        let request = RefUpdateRequest { updates, force };
        self.update_refs(request).await
    }

    /// Pull objects from remote and return pack data
    ///
    /// # Arguments
    /// * `odb` - Local object database
    /// * `remote_ref` - Remote ref to pull
    pub async fn pull(&self, _odb: &ObjectDatabase, remote_ref: &str) -> Result<Vec<u8>> {
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
    async fn download_pack(&self, want: Vec<String>, have: Vec<String>) -> Result<Vec<u8>> {
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

        let pack_data = response
            .bytes()
            .await
            .context("Failed to read pack data")?;

        Ok(pack_data.to_vec())
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

    /// Collect all objects reachable from given commit OIDs
    ///
    /// Performs depth-first graph traversal to collect commits, trees, and blobs.
    /// Returns vec of (OID, ObjectType) tuples for all reachable objects.
    async fn collect_reachable_objects(
        &self,
        odb: &ObjectDatabase,
        commit_oids: Vec<Oid>,
    ) -> Result<Vec<(Oid, ObjectType)>> {
        let mut visited = HashSet::new();
        let mut result = Vec::new();
        let mut queue = VecDeque::new();

        // Start with commit OIDs
        for oid in commit_oids {
            if visited.insert(oid) {
                queue.push_back((oid, ObjectType::Commit));
            }
        }

        while let Some((oid, obj_type)) = queue.pop_front() {
            // Add to result
            result.push((oid, obj_type));

            // Read object data
            let obj_data = odb
                .read(&oid)
                .await
                .context(format!("Failed to read object {}", oid))?;

            match obj_type {
                ObjectType::Commit => {
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
                }
            }
        }

        Ok(result)
    }

    /// Generate a pack file containing specified objects with their types
    async fn generate_pack(
        &self,
        odb: &ObjectDatabase,
        objects: Vec<(Oid, ObjectType)>,
    ) -> Result<Vec<u8>> {
        let mut pack_writer = PackWriter::new();

        for (oid, object_type) in objects {
            // Read object data from ODB
            let obj_data = odb
                .read(&oid)
                .await
                .context(format!("Failed to read object {}", oid))?;

            // Add to pack with correct type
            let _offset = pack_writer.add_object(oid, object_type, &obj_data);
        }

        // Finalize pack
        let pack_data = pack_writer.finalize();
        Ok(pack_data)
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
