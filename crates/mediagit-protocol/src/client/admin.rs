// SPDX-License-Identifier: BUSL-1.1
// Copyright (C) 2025-2026 Aswin Krishnamoorthy

use super::*;

// ============================================================================
// Hosted-repo admin client — mirrors `/admin/repos` in
// `crates/mediagit-server/src/handlers/repos_admin.rs`.
//
// Unlike the per-repo methods, these need a client built from the server
// ORIGIN only (`ProtocolClient::new("http://host:port")`), not a repo URL,
// with admin credentials attached via `with_credentials`.
// ============================================================================

/// One hosted repository, as listed by `GET /admin/repos`.
#[derive(serde::Deserialize, Debug, Clone)]
pub struct RepoSummary {
    pub name: String,
    pub size_bytes: u64,
    #[serde(default)]
    pub object_count: Option<u64>,
    /// Unix seconds.
    pub last_modified: u64,
}

/// Result of `delete_repo`. `storage_retained` is true when the backend is
/// not local filesystem: the repo was unregistered but its bucket data kept.
#[derive(serde::Deserialize, Debug, Clone)]
pub struct DeleteRepoResult {
    pub deleted: bool,
    pub storage_retained: bool,
    pub backend: String,
}

/// One per-repo grant held by a user. `level` is "read", "write" or "admin".
#[derive(serde::Deserialize, Debug, Clone)]
pub struct UserGrant {
    pub repo: String,
    pub level: String,
}

/// One user's grant on a repo; `username` is empty if the account is gone.
#[derive(serde::Deserialize, Debug, Clone)]
pub struct RepoGrant {
    pub user_id: String,
    pub username: String,
    pub level: String,
}

impl ProtocolClient {
    /// GET /admin/repos — list hosted repositories (admin only).
    pub async fn list_repos(&self) -> Result<Vec<RepoSummary>> {
        let url = format!("{}/admin/repos", self.base_url);
        let resp = self
            .client
            .get(&url)
            .send()
            .await
            .context("Failed to GET /admin/repos")?;
        if !resp.status().is_success() {
            anyhow::bail!("GET /admin/repos failed with status: {}", resp.status());
        }
        resp.json()
            .await
            .context("Failed to parse /admin/repos response")
    }

    /// POST /admin/repos — create an empty hosted repository (admin only).
    /// A 409 means the name is already taken.
    pub async fn create_repo(&self, name: &str) -> Result<RepoSummary> {
        let url = format!("{}/admin/repos", self.base_url);
        let resp = self
            .client
            .post(&url)
            .json(&serde_json::json!({ "name": name }))
            .send()
            .await
            .context("Failed to POST /admin/repos")?;
        if resp.status().as_u16() == 409 {
            anyhow::bail!("repository '{}' already exists", name);
        }
        if !resp.status().is_success() {
            anyhow::bail!("POST /admin/repos failed with status: {}", resp.status());
        }
        resp.json()
            .await
            .context("Failed to parse create-repo response")
    }

    /// DELETE /admin/repos/{name}?confirm={name}[&force=true] (admin only).
    /// The confirmation is always sent; callers decide whether to prompt first.
    /// 409 means the repo is non-empty (needs `force`) or has active transfers.
    pub async fn delete_repo(&self, name: &str, force: bool) -> Result<DeleteRepoResult> {
        let url = format!("{}/admin/repos/{}", self.base_url, name);
        let mut req = self.client.delete(&url).query(&[("confirm", name)]);
        if force {
            req = req.query(&[("force", "true")]);
        }
        let resp = req
            .send()
            .await
            .context("Failed to DELETE /admin/repos/:name")?;
        if resp.status().as_u16() == 409 {
            anyhow::bail!(
                "repository '{}' is not empty or has active transfers (use force to override a non-empty repo)",
                name
            );
        }
        if !resp.status().is_success() {
            anyhow::bail!(
                "DELETE /admin/repos/{} failed with status: {}",
                name,
                resp.status()
            );
        }
        resp.json()
            .await
            .context("Failed to parse delete-repo response")
    }

    /// GET /auth/users/{id}/grants — a user's per-repo grants (admin only).
    pub async fn list_user_grants(&self, user_id: &str) -> Result<Vec<UserGrant>> {
        let url = format!("{}/auth/users/{}/grants", self.base_url, user_id);
        let resp = self
            .client
            .get(&url)
            .send()
            .await
            .context("Failed to GET /auth/users/:id/grants")?;
        if !resp.status().is_success() {
            anyhow::bail!("GET {} failed with status: {}", url, resp.status());
        }
        resp.json().await.context("Failed to parse user grants")
    }

    /// GET /admin/repos/{name}/grants — who has access to a repo (admin only).
    pub async fn list_repo_grants(&self, name: &str) -> Result<Vec<RepoGrant>> {
        let url = format!("{}/admin/repos/{}/grants", self.base_url, name);
        let resp = self
            .client
            .get(&url)
            .send()
            .await
            .context("Failed to GET /admin/repos/:name/grants")?;
        if !resp.status().is_success() {
            anyhow::bail!("GET {} failed with status: {}", url, resp.status());
        }
        resp.json().await.context("Failed to parse repo grants")
    }
}
