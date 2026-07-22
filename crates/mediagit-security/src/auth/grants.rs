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

//! Per-repo authorization grants (H2).
//!
//! A grant is `{user_id, repo, level}`: the maximum access `user_id` has on
//! `repo`. This sits alongside the global [`crate::auth::user::Role`] system —
//! a user's flat `permissions` list (from their role) still governs when no
//! grants exist at all (see `check_permission` in `mediagit-server`), but once
//! grants are recorded they can scope a user's access to specific repos.
//!
//! Persisted as `grants.jsonl` via the same helpers as `users.jsonl` /
//! `api_keys.jsonl` (see [`super::persist`]).

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::RwLock;

use super::{AuthResult, persist};

/// Access level granted to a user on a repo. Ordered `Read < Write < Admin`
/// so a grant at a given level satisfies any requirement at or below it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Level {
    Read,
    Write,
    Admin,
}

/// A single persisted grant record.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct Grant {
    user_id: String,
    repo: String,
    level: Level,
}

/// Per-repo grants store, backed in-memory with optional JSONL persistence
/// (mirrors [`super::credentials::CredentialsStore`] / [`super::apikey::ApiKeyAuth`]).
///
/// The in-memory map uses `std::sync::RwLock` rather than `tokio::sync::RwLock`:
/// lookups are pure in-memory and never held across an `.await`, so a
/// synchronous lock lets callers (like `mediagit-server`'s `check_permission`)
/// query grants without becoming `async` themselves.
pub struct GrantsStore {
    grants: RwLock<HashMap<(String, String), Level>>,

    /// Path to `grants.jsonl` when persistence is enabled; `None` for a
    /// purely in-memory store.
    store_path: Option<PathBuf>,
}

impl GrantsStore {
    /// Create a new grants store (in-memory only, no persistence).
    pub fn new() -> Self {
        Self {
            grants: RwLock::new(HashMap::new()),
            store_path: None,
        }
    }

    /// Load grants persisted at `store_dir/grants.jsonl`, or start fresh if
    /// the file doesn't exist yet (first boot). An existing-but-corrupt file
    /// is a hard error. Set `MEDIAGIT_AUTH_PERSIST=0` to force in-memory
    /// behavior even when a directory is given.
    pub fn load_or_new(store_dir: &Path) -> AuthResult<Self> {
        if !persist::persist_enabled() {
            return Ok(Self::new());
        }

        let path = store_dir.join("grants.jsonl");
        let records: Vec<Grant> = persist::load_jsonl(&path)?;

        let mut grants = HashMap::new();
        for g in records {
            grants.insert((g.user_id, g.repo), g.level);
        }

        Ok(Self {
            grants: RwLock::new(grants),
            store_path: Some(path),
        })
    }

    /// Persist the current in-memory state to `grants.jsonl`. A no-op when
    /// this store was constructed with [`GrantsStore::new`] (no store path).
    async fn persist(&self) -> AuthResult<()> {
        let Some(path) = &self.store_path else {
            return Ok(());
        };
        let records = self.snapshot();
        persist::save_jsonl(path, &records).await
    }

    fn snapshot(&self) -> Vec<Grant> {
        let grants = self.grants.read().expect("grants lock poisoned");
        grants
            .iter()
            .map(|((user_id, repo), level)| Grant {
                user_id: user_id.clone(),
                repo: repo.clone(),
                level: *level,
            })
            .collect()
    }

    /// Grant `user_id` `level` access on `repo`, replacing any existing
    /// grant for that pair.
    pub async fn grant(&self, user_id: &str, repo: &str, level: Level) -> AuthResult<()> {
        {
            let mut grants = self.grants.write().expect("grants lock poisoned");
            grants.insert((user_id.to_string(), repo.to_string()), level);
        }
        self.persist().await
    }

    /// Revoke `user_id`'s grant on `repo`, if any.
    pub async fn revoke(&self, user_id: &str, repo: &str) -> AuthResult<()> {
        {
            let mut grants = self.grants.write().expect("grants lock poisoned");
            grants.remove(&(user_id.to_string(), repo.to_string()));
        }
        self.persist().await
    }

    /// Remove every grant held by `user_id` (H3: called when the user
    /// account itself is deleted, so stale grants for a nonexistent user
    /// don't linger in `grants.jsonl`). A no-op — but still persists once,
    /// matching every other mutator — if the user held no grants.
    pub async fn remove_user(&self, user_id: &str) -> AuthResult<()> {
        {
            let mut grants = self.grants.write().expect("grants lock poisoned");
            grants.retain(|(u, _), _| u != user_id);
        }
        self.persist().await
    }

    /// The level granted to `user_id` on `repo`, if any.
    pub fn get(&self, user_id: &str, repo: &str) -> Option<Level> {
        self.grants
            .read()
            .expect("grants lock poisoned")
            .get(&(user_id.to_string(), repo.to_string()))
            .copied()
    }

    /// All `(repo, level)` grants held by `user_id`.
    pub fn list_for_user(&self, user_id: &str) -> Vec<(String, Level)> {
        self.grants
            .read()
            .expect("grants lock poisoned")
            .iter()
            .filter(|((u, _), _)| u == user_id)
            .map(|((_, repo), level)| (repo.clone(), *level))
            .collect()
    }

    /// All `(user_id, level)` grants recorded for `repo`.
    pub fn list_for_repo(&self, repo: &str) -> Vec<(String, Level)> {
        self.grants
            .read()
            .expect("grants lock poisoned")
            .iter()
            .filter(|((_, r), _)| r == repo)
            .map(|((user_id, _), level)| (user_id.clone(), *level))
            .collect()
    }

    /// `true` if no grants have been recorded at all. Used by
    /// `mediagit-server`'s `check_permission` to decide whether per-repo
    /// enforcement is active — a zero-grants deployment behaves exactly like
    /// the pre-H2 flat permission check.
    pub fn is_empty(&self) -> bool {
        self.grants.read().expect("grants lock poisoned").is_empty()
    }
}

impl Default for GrantsStore {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
// Tests hold the process-global env lock across awaits to serialize
// env-var access (see persist::ENV_LOCK).
#[allow(clippy::unwrap_used, clippy::await_holding_lock)]
mod tests {
    use super::*;

    #[test]
    fn level_ordering() {
        assert!(Level::Read < Level::Write);
        assert!(Level::Write < Level::Admin);
        assert!(Level::Read < Level::Admin);
    }

    #[tokio::test]
    async fn grant_and_get() {
        let store = GrantsStore::new();
        assert!(store.is_empty());

        store.grant("user1", "repoA", Level::Write).await.unwrap();
        assert!(!store.is_empty());
        assert_eq!(store.get("user1", "repoA"), Some(Level::Write));
        assert_eq!(store.get("user1", "repoB"), None);
        assert_eq!(store.get("user2", "repoA"), None);
    }

    #[tokio::test]
    async fn remove_user_clears_all_their_grants_only() {
        let store = GrantsStore::new();
        store.grant("user1", "repoA", Level::Read).await.unwrap();
        store.grant("user1", "repoB", Level::Write).await.unwrap();
        store.grant("user2", "repoA", Level::Admin).await.unwrap();

        store.remove_user("user1").await.unwrap();

        assert_eq!(store.get("user1", "repoA"), None);
        assert_eq!(store.get("user1", "repoB"), None);
        assert_eq!(store.get("user2", "repoA"), Some(Level::Admin));
    }

    #[tokio::test]
    async fn revoke_removes_grant() {
        let store = GrantsStore::new();
        store.grant("user1", "repoA", Level::Admin).await.unwrap();
        store.revoke("user1", "repoA").await.unwrap();
        assert_eq!(store.get("user1", "repoA"), None);
        assert!(store.is_empty());
    }

    #[tokio::test]
    async fn list_for_user_and_repo() {
        let store = GrantsStore::new();
        store.grant("user1", "repoA", Level::Read).await.unwrap();
        store.grant("user1", "repoB", Level::Write).await.unwrap();
        store.grant("user2", "repoA", Level::Admin).await.unwrap();

        let mut for_user1 = store.list_for_user("user1");
        for_user1.sort_by(|a, b| a.0.cmp(&b.0));
        assert_eq!(
            for_user1,
            vec![
                ("repoA".to_string(), Level::Read),
                ("repoB".to_string(), Level::Write)
            ]
        );

        let mut for_repo_a = store.list_for_repo("repoA");
        for_repo_a.sort_by(|a, b| a.0.cmp(&b.0));
        assert_eq!(
            for_repo_a,
            vec![
                ("user1".to_string(), Level::Read),
                ("user2".to_string(), Level::Admin)
            ]
        );
    }

    // ---- persistence ----

    #[tokio::test]
    async fn persists_and_reloads_across_restart() {
        let _guard = persist::ENV_LOCK.read().unwrap();
        let tmp = tempfile::tempdir().unwrap();
        let store = GrantsStore::load_or_new(tmp.path()).unwrap();
        store.grant("user1", "repoA", Level::Write).await.unwrap();

        assert!(tmp.path().join("grants.jsonl").exists());

        // Fresh store from the same dir simulates a server restart.
        let store2 = GrantsStore::load_or_new(tmp.path()).unwrap();
        assert_eq!(store2.get("user1", "repoA"), Some(Level::Write));
    }

    #[tokio::test]
    async fn corrupt_store_file_hard_errors() {
        let _guard = persist::ENV_LOCK.read().unwrap();
        let tmp = tempfile::tempdir().unwrap();
        tokio::fs::write(
            tmp.path().join("grants.jsonl"),
            b"{\"v\":1}\nnot valid json\n",
        )
        .await
        .unwrap();

        let result = GrantsStore::load_or_new(tmp.path());
        assert!(result.is_err(), "corrupt store file must hard-error");
    }

    #[tokio::test]
    async fn missing_store_file_starts_fresh() {
        let _guard = persist::ENV_LOCK.read().unwrap();
        let tmp = tempfile::tempdir().unwrap();
        let store = GrantsStore::load_or_new(tmp.path()).unwrap();
        assert!(store.is_empty());
    }

    #[tokio::test]
    async fn persist_disabled_writes_no_files() {
        let _guard = persist::ENV_LOCK.write().unwrap();
        mediagit_test_utils::set_var("MEDIAGIT_AUTH_PERSIST", "0");
        let tmp = tempfile::tempdir().unwrap();
        let store = GrantsStore::load_or_new(tmp.path()).unwrap();
        store.grant("user1", "repoA", Level::Write).await.unwrap();
        mediagit_test_utils::remove_var("MEDIAGIT_AUTH_PERSIST");

        assert!(!tmp.path().join("grants.jsonl").exists());
    }
}
