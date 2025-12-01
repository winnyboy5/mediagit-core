use anyhow::{Context, Result};
use mediagit_versioning::{ObjectDatabase, ObjectType, Oid, PackWriter};

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
        let url = format!("{}/info/refs", self.base_url);
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
        // Collect OIDs we need to send (simplified - send all objects for new refs)
        let mut oids_to_send = Vec::new();
        for update in &updates {
            oids_to_send.push(update.new_oid.clone());
        }

        // Generate pack file with objects
        if !oids_to_send.is_empty() {
            let pack_data = self.generate_pack(odb, oids_to_send).await?;
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

    /// Generate a pack file containing specified objects
    async fn generate_pack(&self, odb: &ObjectDatabase, oids: Vec<String>) -> Result<Vec<u8>> {
        let mut pack_writer = PackWriter::new();

        for oid_str in oids {
            // Parse OID from hex string
            let oid = Oid::from_hex(&oid_str)
                .map_err(|e| anyhow::anyhow!("Invalid OID {}: {}", oid_str, e))?;

            // Read object data from ODB
            let obj_data = odb
                .read(&oid)
                .await
                .context(format!("Failed to read object {}", oid_str))?;

            // TODO: Need to determine object type properly
            // For now, assume Blob type (suitable for media files)
            // In the future, we should either:
            // 1. Add metadata storage to track object types
            // 2. Add read_with_type() method to ObjectDatabase
            // 3. Parse object data to determine type
            let object_type = ObjectType::Blob;

            // Add to pack (returns offset as u64, not Result)
            let _offset = pack_writer.add_object(oid, object_type, &obj_data);
        }

        // Finalize pack (returns Vec<u8> directly, not Result)
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
