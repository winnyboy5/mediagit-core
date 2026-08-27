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
    /// AU-5: the repo's durable `repo_id` at the time the grant was made.
    ///
    /// Grants were keyed on repo *name* alone, so deleting a repo and
    /// recreating one with the same name — routine when repo names are
    /// reused — silently handed the new repo every grant the old one had,
    /// including Admin. Binding the grant to the identity that existed when
    /// it was issued means a recreated repo (which gets a fresh id) matches
    /// nothing.
    ///
    /// `None` marks a legacy grant recorded before this field existed. Those
    /// still match on name, so upgrading does not revoke anyone's access; the
    /// binding is captured the next time the grant is re-issued.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    repo_id: Option<String>,
}

/// Per-repo grants store, backed in-memory with optional JSONL persistence
/// (mirrors [`super::credentials::CredentialsStore`] / [`super::apikey::ApiKeyAuth`]).
///
/// The in-memory map uses `std::sync::RwLock` rather than `tokio::sync::RwLock`:
/// lookups are pure in-memory and never held across an `.await`, so a
/// synchronous lock lets callers (like `mediagit-server`'s `check_permission`)
/// query grants without becoming `async` themselves.
/// `(user_id, repo_name)` — the lookup key for a grant.
type GrantKey = (String, String);

/// `(level, repo_id)` — the granted level and the repo identity it was bound
/// to when issued. `None` marks a legacy grant recorded before AU-5.
type GrantValue = (Level, Option<String>);

pub struct GrantsStore {
    grants: RwLock<HashMap<GrantKey, GrantValue>>,

    /// AU-8: serialises snapshot-and-write so concurrent mutations cannot
    /// persist out of order.
    ///
    /// [`Self::snapshot`] materialises an owned `Vec` and releases the map
    /// lock before the write — it must, because `grants` is a
    /// `std::sync::RwLock` and holding that across an `.await` would block
    /// the runtime thread. But that left a window: a `grant` could snapshot,
    /// a concurrent `revoke` could then mutate, snapshot and finish its write
    /// first, and the older `grant` snapshot would land last and **restore
    /// the revoked grant on disk**. Memory stayed correct, so the revoke
    /// looked successful right up until the next restart reloaded the file.
    ///
    /// Taking the snapshot *inside* this mutex means whichever write runs
    /// last also snapshotted last, so the file converges on current state.
    /// `credentials.rs` and `apikey.rs` avoid the same race differently, by
    /// holding their (tokio) read guard across the write.
    persist_lock: tokio::sync::Mutex<()>,

    /// Path to `grants.jsonl` when persistence is enabled; `None` for a
    /// purely in-memory store.
    store_path: Option<PathBuf>,
}

impl GrantsStore {
    /// Create a new grants store (in-memory only, no persistence).
    pub fn new() -> Self {
        Self {
            grants: RwLock::new(HashMap::new()),
            persist_lock: tokio::sync::Mutex::new(()),
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
            grants.insert((g.user_id, g.repo), (g.level, g.repo_id));
        }

        Ok(Self {
            grants: RwLock::new(grants),
            persist_lock: tokio::sync::Mutex::new(()),
            store_path: Some(path),
        })
    }

    /// Persist the current in-memory state to `grants.jsonl`. A no-op when
    /// this store was constructed with [`GrantsStore::new`] (no store path).
    async fn persist(&self) -> AuthResult<()> {
        let Some(path) = &self.store_path else {
            return Ok(());
        };
        // AU-8: snapshot and write under one lock — see `persist_lock`.
        let _serialised = self.persist_lock.lock().await;
        let records = self.snapshot();
        persist::save_jsonl(path, &records).await
    }

    fn snapshot(&self) -> Vec<Grant> {
        // Recover from poison rather than propagate it: grants writes are
        // small, atomic `HashMap` mutations (insert/remove/retain), so a
        // poisoned lock still holds either the pre- or post-mutation state,
        // never a torn one. This is a read-mostly, security-critical path —
        // one panicking auth request must not poison the lock and cascade
        // into every subsequent grant check panicking too.
        let grants = self.grants.read().unwrap_or_else(|e| e.into_inner());
        grants
            .iter()
            .map(|((user_id, repo), (level, repo_id))| Grant {
                user_id: user_id.clone(),
                repo: repo.clone(),
                level: *level,
                repo_id: repo_id.clone(),
            })
            .collect()
    }

    /// Grant `user_id` `level` access on `repo`, replacing any existing
    /// grant for that pair.
    pub async fn grant(&self, user_id: &str, repo: &str, level: Level) -> AuthResult<()> {
        self.grant_bound(user_id, repo, None, level).await
    }

    /// AU-5: grant, binding the record to the repo's durable `repo_id`.
    ///
    /// Pass the id whenever the caller can resolve it. A grant carrying an id
    /// stops matching if the repo is later deleted and recreated under the
    /// same name, because the replacement gets a fresh id. `None` records a
    /// legacy-style name-only grant.
    pub async fn grant_bound(
        &self,
        user_id: &str,
        repo: &str,
        repo_id: Option<&str>,
        level: Level,
    ) -> AuthResult<()> {
        {
            let mut grants = self.grants.write().unwrap_or_else(|e| e.into_inner());
            grants.insert(
                (user_id.to_string(), repo.to_string()),
                (level, repo_id.map(str::to_string)),
            );
        }
        self.persist().await
    }

    /// Revoke `user_id`'s grant on `repo`, if any.
    pub async fn revoke(&self, user_id: &str, repo: &str) -> AuthResult<()> {
        {
            let mut grants = self.grants.write().unwrap_or_else(|e| e.into_inner());
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
            let mut grants = self.grants.write().unwrap_or_else(|e| e.into_inner());
            grants.retain(|(u, _), _| u != user_id);
        }
        self.persist().await
    }

    /// The level granted to `user_id` on `repo`, if any.
    pub fn get(&self, user_id: &str, repo: &str) -> Option<Level> {
        self.get_for_repo_id(user_id, repo, None)
    }

    /// AU-5: the level granted to `user_id` on `repo`, honouring the id the
    /// grant was bound to.
    ///
    /// `current_repo_id` is the repo's id *now*. A grant matches when:
    ///   - it carries no id (legacy, pre-AU-5) — matched on name, so an
    ///     upgrade never revokes existing access; or
    ///   - the caller could not resolve an id (`None`) — the request is about
    ///     to fail its own existence check anyway, so this is not a new hole;
    ///     or
    ///   - the ids agree.
    ///
    /// A grant bound to a *different* id is ignored: that is the recreated
    /// repo case this exists to stop.
    pub fn get_for_repo_id(
        &self,
        user_id: &str,
        repo: &str,
        current_repo_id: Option<&str>,
    ) -> Option<Level> {
        let grants = self.grants.read().unwrap_or_else(|e| e.into_inner());
        let (level, bound) = grants.get(&(user_id.to_string(), repo.to_string()))?;
        match (bound.as_deref(), current_repo_id) {
            (None, _) | (_, None) => Some(*level),
            (Some(a), Some(b)) if a == b => Some(*level),
            _ => None,
        }
    }

    /// All `(repo, level)` grants held by `user_id`.
    pub fn list_for_user(&self, user_id: &str) -> Vec<(String, Level)> {
        self.grants
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .filter(|((u, _), _)| u == user_id)
            .map(|((_, repo), (level, _))| (repo.clone(), *level))
            .collect()
    }

    /// All `(user_id, level)` grants recorded for `repo`.
    pub fn list_for_repo(&self, repo: &str) -> Vec<(String, Level)> {
        self.grants
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .filter(|((_, r), _)| r == repo)
            .map(|((user_id, _), (level, _))| (user_id.clone(), *level))
            .collect()
    }

    /// AU-5: drop grants for `repo` that were bound to a *different*
    /// `current_repo_id`, returning how many were removed.
    ///
    /// Called when a repo's identity is resolved. A repo deleted and
    /// recreated under the same name gets a fresh id, so every grant still
    /// carrying the old one refers to a repository that no longer exists and
    /// must not apply to its replacement. Legacy grants with no binding are
    /// left alone — they predate this field and removing them would revoke
    /// access on upgrade.
    pub async fn prune_stale_bindings(&self, repo: &str, current_repo_id: &str) -> usize {
        let removed = {
            let mut grants = self.grants.write().unwrap_or_else(|e| e.into_inner());
            let before = grants.len();
            grants.retain(|(_, r), (_, bound)| {
                r != repo || bound.as_deref().is_none_or(|b| b == current_repo_id)
            });
            before - grants.len()
        };
        if removed > 0 {
            let _ = self.persist().await;
        }
        removed
    }

    /// AU-4: whether any grant is recorded for `repo`.
    ///
    /// Enforcement is decided **per repository**, not globally. Gating it on
    /// "does the whole store have any grants" meant the first grant an
    /// operator recorded — a routine onboarding step for one tenant —
    /// silently switched every other repo from flat-role to grant-based
    /// authorization, locking out every user who had no explicit grant.
    /// Scoping the question to the repo under access keeps that blast radius
    /// to the repo actually being configured.
    pub fn repo_has_grants(&self, repo: &str) -> bool {
        self.grants
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .keys()
            .any(|(_, r)| r == repo)
    }

    /// `true` if no grants have been recorded at all. Retained for the
    /// startup diagnostic in `mediagit-server`; authorization decisions use
    /// [`Self::repo_has_grants`] instead (AU-4).
    pub fn is_empty(&self) -> bool {
        self.grants
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .is_empty()
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

    #[test]
    fn poisoned_lock_recovers_instead_of_panicking() {
        let store = GrantsStore::new();
        store.grants.write().unwrap().insert(
            ("user1".to_string(), "repoA".to_string()),
            (Level::Read, None),
        );

        std::thread::scope(|s| {
            let handle = s.spawn(|| {
                let _guard = store.grants.write().unwrap();
                panic!("simulated panic while holding the write lock");
            });
            assert!(handle.join().is_err(), "spawned thread should panic");
        });
        assert!(store.grants.is_poisoned());

        // Every read/write site must recover from the poison rather than
        // propagate it.
        assert_eq!(store.get("user1", "repoA"), Some(Level::Read));
        assert!(!store.is_empty());
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

    /// AU-8: a revoke must survive a concurrent grant.
    ///
    /// `snapshot()` releases the map lock before writing, so an older
    /// snapshot could land last and restore a revoked grant on disk. Memory
    /// stayed correct, which is what made it hard to notice — the revoke
    /// looked successful until the next restart reloaded the file.
    #[tokio::test]
    async fn concurrent_mutations_persist_final_state() {
        // Other tests set MEDIAGIT_AUTH_PERSIST=0 process-globally, which
        // would silently make this store in-memory and defeat the reload
        // assertion. Take the shared read guard so we cannot overlap them.
        let _guard = persist::ENV_LOCK.read().unwrap();
        let dir = tempfile::tempdir().unwrap();
        let store = std::sync::Arc::new(GrantsStore::load_or_new(dir.path()).unwrap());

        // Churn: many concurrent grant/revoke pairs on distinct keys, plus a
        // single key that ends revoked.
        store.grant("victim", "repoX", Level::Admin).await.unwrap();

        let mut tasks = Vec::new();
        for i in 0..16 {
            let s = std::sync::Arc::clone(&store);
            tasks.push(tokio::spawn(async move {
                s.grant(&format!("u{i}"), "repoY", Level::Read)
                    .await
                    .unwrap();
            }));
        }
        let s = std::sync::Arc::clone(&store);
        tasks.push(tokio::spawn(async move {
            s.revoke("victim", "repoX").await.unwrap();
        }));
        for t in tasks {
            t.await.unwrap();
        }

        // In-memory truth.
        assert!(store.get("victim", "repoX").is_none());

        // Reload from disk — the revoke must not have been resurrected by a
        // slower concurrent write.
        let reloaded = GrantsStore::load_or_new(dir.path()).unwrap();
        assert!(
            reloaded.get("victim", "repoX").is_none(),
            "a concurrent grant overwrote the revoke on disk; it would come \
                back on restart"
        );
        for i in 0..16 {
            assert_eq!(
                reloaded.get(&format!("u{i}"), "repoY"),
                Some(Level::Read),
                "concurrent grant u{i} was lost on disk"
            );
        }
    }

    /// AU-5: a repo recreated under a reused name must not inherit the old
    /// repo's grants.
    ///
    /// Grants keyed on name alone meant deleting a repo and creating another
    /// with the same name silently handed the newcomer every grant the old
    /// one had — up to and including Admin. Repo-name reuse is routine, so
    /// this needed no unusual operator action to hit.
    #[tokio::test]
    async fn grant_does_not_survive_repo_recreation() {
        let store = GrantsStore::new();
        store
            .grant_bound(
                "alice",
                "shared-name",
                Some("repo-id-original"),
                Level::Admin,
            )
            .await
            .unwrap();

        // Same repo, same identity: the grant applies.
        assert_eq!(
            store.get_for_repo_id("alice", "shared-name", Some("repo-id-original")),
            Some(Level::Admin)
        );

        // Repo deleted and recreated: same name, fresh identity. The old
        // grant must not carry over.
        assert_eq!(
            store.get_for_repo_id("alice", "shared-name", Some("repo-id-replacement")),
            None,
            "a recreated repo inherited the deleted repo's grant"
        );
    }

    /// Upgrading must not revoke anyone: grants recorded before AU-5 carry no
    /// id and keep matching on name.
    #[tokio::test]
    async fn legacy_unbound_grants_still_match() {
        let store = GrantsStore::new();
        store
            .grant("bob", "legacy-repo", Level::Write)
            .await
            .unwrap();

        assert_eq!(
            store.get_for_repo_id("bob", "legacy-repo", Some("any-id")),
            Some(Level::Write),
            "a pre-AU-5 grant should still apply after upgrade"
        );
    }

    /// The binding must survive a reload — it is the persisted field that
    /// does the work, not just in-memory state.
    #[tokio::test]
    async fn repo_id_binding_round_trips_through_disk() {
        let _guard = persist::ENV_LOCK.read().unwrap();
        let dir = tempfile::tempdir().unwrap();
        {
            let store = GrantsStore::load_or_new(dir.path()).unwrap();
            store
                .grant_bound("carol", "r", Some("id-A"), Level::Read)
                .await
                .unwrap();
        }
        let reloaded = GrantsStore::load_or_new(dir.path()).unwrap();
        assert_eq!(
            reloaded.get_for_repo_id("carol", "r", Some("id-A")),
            Some(Level::Read)
        );
        assert_eq!(
            reloaded.get_for_repo_id("carol", "r", Some("id-B")),
            None,
            "binding was lost across persist/reload"
        );
    }

    /// AU-5: pruning drops only the stale bindings.
    ///
    /// It must not touch grants for other repos, nor legacy unbound grants —
    /// removing those would revoke access on upgrade, which is exactly the
    /// outcome the `Option` binding exists to avoid.
    #[tokio::test]
    async fn prune_removes_only_stale_bindings() {
        let store = GrantsStore::new();
        store
            .grant_bound("a", "shared", Some("old-id"), Level::Admin)
            .await
            .unwrap();
        store
            .grant_bound("b", "shared", Some("new-id"), Level::Read)
            .await
            .unwrap();
        store.grant("c", "shared", Level::Write).await.unwrap(); // legacy
        store
            .grant_bound("d", "other", Some("old-id"), Level::Admin)
            .await
            .unwrap();

        let pruned = store.prune_stale_bindings("shared", "new-id").await;
        assert_eq!(pruned, 1, "only the stale binding should go");

        assert_eq!(store.get("a", "shared"), None, "stale grant survived");
        assert_eq!(store.get("b", "shared"), Some(Level::Read));
        assert_eq!(
            store.get("c", "shared"),
            Some(Level::Write),
            "a legacy unbound grant was revoked by the upgrade"
        );
        assert_eq!(
            store.get("d", "other"),
            Some(Level::Admin),
            "pruning one repo affected another"
        );
    }
}
