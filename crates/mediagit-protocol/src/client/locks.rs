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

// ============================================================================
// File-locking client (B4) — mirrors the server's HTTP surface over
// `crate::locks` (see `crates/mediagit-server/src/handlers/locks.rs`).
// ============================================================================

/// Information about a lock, returned by `create_lock` and `list_locks`.
/// Field names mirror the server's `LockResponse` DTO exactly.
#[derive(serde::Deserialize, Debug, Clone)]
pub struct LockInfo {
    pub lock_id: String,
    pub path: String,
    pub owner: String,
    pub created_at: u64,
}

impl ProtocolClient {
    /// POST /:repo/locks — acquire a lock on `path`.
    ///
    /// `owner` is required when talking to a no-auth server (it has no
    /// other identity to attribute the lock to); an authenticated server
    /// derives the owner from the token instead and ignores this field.
    ///
    /// On 409 (already locked), returns an error naming the current owner
    /// and lock id so the caller doesn't need to re-parse anything.
    pub async fn create_lock(&self, path: &str, owner: Option<String>) -> Result<LockInfo> {
        #[derive(serde::Serialize)]
        struct Req<'a> {
            path: &'a str,
            owner: Option<String>,
        }
        #[derive(serde::Deserialize)]
        struct Conflict {
            #[allow(dead_code)]
            error: String,
            owner: String,
            lock_id: String,
        }

        let url = format!("{}/locks", self.base_url);
        let resp = self
            .client
            .post(&url)
            .json(&Req { path, owner })
            .send()
            .await
            .context("Failed to POST /locks")?;

        if resp.status().as_u16() == 409 {
            let conflict: Conflict = resp
                .json()
                .await
                .context("Failed to parse lock conflict response")?;
            anyhow::bail!(
                "'{}' is already locked by {} (lock {})",
                path,
                conflict.owner,
                conflict.lock_id
            );
        }
        if !resp.status().is_success() {
            anyhow::bail!("POST /locks failed with status: {}", resp.status());
        }
        resp.json::<LockInfo>()
            .await
            .context("Failed to parse lock response")
    }

    /// GET /:repo/locks — list active locks (server sorts by path).
    pub async fn list_locks(&self) -> Result<Vec<LockInfo>> {
        #[derive(serde::Deserialize)]
        struct Resp {
            locks: Vec<LockInfo>,
        }

        let url = format!("{}/locks", self.base_url);
        let resp = self
            .client
            .get(&url)
            .send()
            .await
            .context("Failed to GET /locks")?;
        if !resp.status().is_success() {
            anyhow::bail!("GET /locks failed with status: {}", resp.status());
        }
        Ok(resp
            .json::<Resp>()
            .await
            .context("Failed to parse /locks response")?
            .locks)
    }

    /// DELETE /:repo/locks/:lock_id?force=1 — release a lock.
    ///
    /// `force` requires `repo:admin` on the server and bypasses the owner
    /// check; a no-auth server has no identity to compare against and so
    /// must always be called with `force = true`.
    pub async fn delete_lock(&self, lock_id: &str, force: bool) -> Result<()> {
        let url = format!("{}/locks/{}", self.base_url, lock_id);
        let mut req = self.client.delete(&url);
        if force {
            req = req.query(&[("force", "1")]);
        }
        let resp = req
            .send()
            .await
            .context("Failed to DELETE /locks/:lock_id")?;
        if !resp.status().is_success() {
            anyhow::bail!(
                "DELETE /locks/{} failed with status: {}",
                lock_id,
                resp.status()
            );
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lock_info_deserializes_server_response_shape() {
        let json = r#"{"lock_id":"abc123","path":"assets/foo.psd","owner":"alice","created_at":1700000000}"#;
        let info: LockInfo = serde_json::from_str(json).unwrap();
        assert_eq!(info.lock_id, "abc123");
        assert_eq!(info.path, "assets/foo.psd");
        assert_eq!(info.owner, "alice");
        assert_eq!(info.created_at, 1700000000);
    }

    #[test]
    fn list_locks_response_shape_parses() {
        #[derive(serde::Deserialize)]
        struct Resp {
            locks: Vec<LockInfo>,
        }
        let json = r#"{"locks":[{"lock_id":"a","path":"p1","owner":"bob","created_at":1},{"lock_id":"b","path":"p2","owner":"carol","created_at":2}]}"#;
        let resp: Resp = serde_json::from_str(json).unwrap();
        assert_eq!(resp.locks.len(), 2);
        assert_eq!(resp.locks[1].owner, "carol");
        assert_eq!(resp.locks[1].path, "p2");
    }
}
