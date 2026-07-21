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

//! Server-enforced file locking (Tracks B1-B3).
//!
//! Locks are keyed by repo-relative path and persisted per-repo at
//! `<repo>/.mediagit/locks.jsonl` (first line `{"v":1}` header, one
//! `LockRecord` JSON object per following line). The in-memory map on
//! `AppState` is the hot-path source of truth; the JSONL file exists purely
//! so locks survive a server restart. Mutations always take the write lock
//! and rewrite the whole file (tmp+rename) — lock sets are small
//! (human-driven "I'm working on this asset" locks), so no append log is
//! needed.

use axum::http::StatusCode;
use mediagit_versioning::{Commit, ObjectDatabase, Oid, Tree, TreeDiffer};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::Arc;

use crate::state::AppState;

/// A single file lock record, persisted to `.mediagit/locks.jsonl`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LockRecord {
    pub path: String,
    /// User id (or client-supplied identity string on a no-auth server).
    pub owner: String,
    pub lock_id: String,
    pub created_at: u64,
}

/// Outcome of attempting to create a lock.
pub enum CreateLockOutcome {
    Created(LockRecord),
    /// Path was already locked; carries the existing record so callers can
    /// build a 409 response naming the current owner.
    AlreadyLocked(LockRecord),
}

/// First line of `locks.jsonl` — a version marker for future format changes.
#[derive(Serialize, Deserialize)]
struct LockFileHeader {
    v: u32,
}

/// Current on-disk version for `locks.jsonl` headers (`{"v":N}`). A header
/// with a different version is a hard error at load — never silently
/// accepted (format-freeze policy).
const LOCKS_CURRENT_VERSION: u32 = 1;

fn locks_file_path(repo_path: &Path) -> std::path::PathBuf {
    repo_path.join(".mediagit").join("locks.jsonl")
}

/// Normalize a lock path to tree-path form so enforcement comparisons match:
/// backslashes to forward slashes, strip leading `./` and `/`. A Windows
/// client locking `assets\a.psd` must block a push touching `assets/a.psd`.
pub(crate) fn normalize_lock_path(path: &str) -> String {
    let p = path.trim().replace('\\', "/");
    p.trim_start_matches("./")
        .trim_start_matches('/')
        .to_string()
}

fn now_unix() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

/// Derive a lock id without a uuid dependency: hex BLAKE3 of
/// path+owner+timestamp. A collision would require the same path, owner and
/// creation second, and the path is already checked against the existing
/// lock map before this is called (double-lock is rejected with 409 first).
fn generate_lock_id(path: &str, owner: &str, created_at: u64) -> String {
    let input = format!("{}:{}:{}", path, owner, created_at);
    blake3::hash(input.as_bytes()).to_hex().to_string()
}

async fn load_locks_file(path: &Path) -> anyhow::Result<HashMap<String, LockRecord>> {
    let content = match tokio::fs::read_to_string(path).await {
        Ok(c) => c,
        Err(_) => return Ok(HashMap::new()),
    };
    let mut map = HashMap::new();
    for (i, line) in content.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        if i == 0 {
            let header: LockFileHeader = serde_json::from_str(line).map_err(|e| {
                anyhow::anyhow!("corrupt locks file {}: bad header: {}", path.display(), e)
            })?;
            if header.v != LOCKS_CURRENT_VERSION {
                anyhow::bail!(
                    "unsupported locks file version {} in {}, this build supports v{}",
                    header.v,
                    path.display(),
                    LOCKS_CURRENT_VERSION
                );
            }
            continue;
        }
        if let Ok(rec) = serde_json::from_str::<LockRecord>(line) {
            map.insert(rec.path.clone(), rec);
        }
    }
    Ok(map)
}

async fn save_locks_file(path: &Path, locks: &HashMap<String, LockRecord>) -> anyhow::Result<()> {
    let mut content = serde_json::to_string(&LockFileHeader {
        v: LOCKS_CURRENT_VERSION,
    })?;
    content.push('\n');
    for rec in locks.values() {
        content.push_str(&serde_json::to_string(rec)?);
        content.push('\n');
    }
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    let tmp = path.with_extension("jsonl.tmp");
    tokio::fs::write(&tmp, content.as_bytes()).await?;
    tokio::fs::rename(&tmp, path).await?;
    Ok(())
}

/// Returns a clone of this repo's lock map, lazily loading it from disk on
/// first access. Mirrors the `get_or_init_storage` double-checked pattern in
/// `handlers/mod.rs` — a read-locked fast path covers every call after the
/// first for a given repo.
pub async fn get_repo_locks(
    state: &AppState,
    repo: &str,
    repo_path: &Path,
) -> Result<HashMap<String, LockRecord>, StatusCode> {
    if let Some(m) = state.locks.read().await.get(repo).cloned() {
        return Ok(m);
    }
    let mut all = state.locks.write().await;
    if let Some(m) = all.get(repo).cloned() {
        return Ok(m);
    }
    let loaded = load_locks_file(&locks_file_path(repo_path))
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    all.insert(repo.to_string(), loaded.clone());
    Ok(loaded)
}

/// Mutates this repo's lock map under the write lock and persists the result
/// to disk before releasing it, so in-memory state and the on-disk JSONL
/// never diverge.
async fn with_repo_locks_write<F, T>(
    state: &AppState,
    repo: &str,
    repo_path: &Path,
    f: F,
) -> Result<T, StatusCode>
where
    F: FnOnce(&mut HashMap<String, LockRecord>) -> T,
{
    let mut all = state.locks.write().await;
    if !all.contains_key(repo) {
        let loaded = load_locks_file(&locks_file_path(repo_path))
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        all.insert(repo.to_string(), loaded);
    }
    let map = all.get_mut(repo).expect("just inserted above");
    let result = f(map);
    let snapshot = map.clone();
    save_locks_file(&locks_file_path(repo_path), &snapshot)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(result)
}

/// Create a lock on `path` for `owner`. Returns `AlreadyLocked` (carrying the
/// existing record) instead of an error when the path is already locked —
/// the handler turns that into a 409.
pub async fn create_lock(
    state: &AppState,
    repo: &str,
    repo_path: &Path,
    path: String,
    owner: String,
) -> Result<CreateLockOutcome, StatusCode> {
    let path = normalize_lock_path(&path);
    with_repo_locks_write(state, repo, repo_path, move |map| {
        if let Some(existing) = map.get(&path) {
            return CreateLockOutcome::AlreadyLocked(existing.clone());
        }
        let created_at = now_unix();
        let lock_id = generate_lock_id(&path, &owner, created_at);
        let record = LockRecord {
            path: path.clone(),
            owner,
            lock_id,
            created_at,
        };
        map.insert(path, record.clone());
        CreateLockOutcome::Created(record)
    })
    .await
}

/// Look up a lock by id (list is small; a linear scan is fine).
pub async fn find_lock_by_id(
    state: &AppState,
    repo: &str,
    repo_path: &Path,
    lock_id: &str,
) -> Result<Option<LockRecord>, StatusCode> {
    let map = get_repo_locks(state, repo, repo_path).await?;
    Ok(map.values().find(|r| r.lock_id == lock_id).cloned())
}

/// Delete a lock by id. Returns `true` if a lock was removed, `false` if no
/// lock with that id existed.
pub async fn delete_lock(
    state: &AppState,
    repo: &str,
    repo_path: &Path,
    lock_id: &str,
) -> Result<bool, StatusCode> {
    with_repo_locks_write(state, repo, repo_path, move |map| {
        let path = map
            .values()
            .find(|r| r.lock_id == lock_id)
            .map(|r| r.path.clone());
        match path {
            Some(p) => {
                map.remove(&p);
                true
            }
            None => false,
        }
    })
    .await
}

// ============================================================================
// B3 — push enforcement
// ============================================================================

fn join_path(prefix: &str, name: &str) -> String {
    if prefix.is_empty() {
        name.to_string()
    } else {
        format!("{}/{}", prefix, name)
    }
}

/// Collect every blob path under `tree_oid` (recursing into subdirectories),
/// used when a directory has no prior counterpart to diff against (added
/// subtree, or the commit being walked is a repo's initial commit).
async fn collect_all_paths(
    odb: &ObjectDatabase,
    tree_oid: Oid,
    prefix: &str,
    out: &mut HashSet<String>,
) -> anyhow::Result<()> {
    let mut stack = vec![(tree_oid, prefix.to_string())];
    while let Some((oid, prefix)) = stack.pop() {
        let tree = Tree::read(odb, &oid).await?;
        for entry in tree.entries.values() {
            let path = join_path(&prefix, &entry.name);
            if entry.is_tree() {
                stack.push((entry.oid, path));
            } else {
                out.insert(path);
            }
        }
    }
    Ok(())
}

/// Diff `old_tree` against `new_tree`, adding every touched path to `out`.
/// Recurses only into subdirectories whose OID actually differs (via an
/// explicit stack rather than recursive `async fn`, since async recursion
/// needs boxing).
async fn diff_tree_paths(
    differ: &TreeDiffer,
    odb: &ObjectDatabase,
    old_tree: Oid,
    new_tree: Oid,
    prefix: &str,
    out: &mut HashSet<String>,
) -> anyhow::Result<()> {
    let mut stack = vec![(old_tree, new_tree, prefix.to_string())];
    while let Some((old_t, new_t, prefix)) = stack.pop() {
        if old_t == new_t {
            continue;
        }
        let diff = differ.diff_trees(&old_t, &new_t).await?;

        for e in diff.added.iter().chain(diff.deleted.iter()) {
            let path = join_path(&prefix, &e.name);
            if e.is_tree() {
                collect_all_paths(odb, e.oid, &path, out).await?;
            }
            out.insert(path);
        }

        for m in &diff.modified {
            let path = join_path(&prefix, &m.path);
            out.insert(path.clone());
            match (m.source.is_tree(), m.target.is_tree()) {
                (true, true) => stack.push((m.source.oid, m.target.oid, path)),
                (false, true) => collect_all_paths(odb, m.target.oid, &path, out).await?,
                (true, false) => collect_all_paths(odb, m.source.oid, &path, out).await?,
                (false, false) => {}
            }
        }
    }
    Ok(())
}

/// Walk commits from `new_oid` back to `old_oid` (exclusive) along the
/// first-parent chain, diffing each commit's tree against its parent's tree.
/// `old_oid` of `None` means a new branch — the walk continues to the root
/// commit, still capped at `max_commits`.
///
/// Returns `(touched_paths, capped)`; `capped = true` means the walk hit
/// `max_commits` before reaching `old_oid` (or the root), so the result is
/// incomplete and callers should fail open.
async fn touched_paths_for_push(
    odb: &Arc<ObjectDatabase>,
    new_oid: Oid,
    old_oid: Option<Oid>,
    max_commits: usize,
) -> anyhow::Result<(HashSet<String>, bool)> {
    let differ = TreeDiffer::new(Arc::clone(odb));
    let mut touched = HashSet::new();
    let mut current = new_oid;
    let mut visited = 0usize;

    loop {
        if let Some(old) = old_oid {
            if current == old {
                break;
            }
        }
        if visited >= max_commits {
            return Ok((touched, true));
        }

        let commit = Commit::read(odb, &current).await?;
        visited += 1;

        match commit.first_parent().copied() {
            Some(parent_oid) => {
                let parent_commit = Commit::read(odb, &parent_oid).await?;
                diff_tree_paths(
                    &differ,
                    odb,
                    parent_commit.tree,
                    commit.tree,
                    "",
                    &mut touched,
                )
                .await?;
                current = parent_oid;
            }
            None => {
                // Root commit: everything in its tree is "touched" (relevant
                // only for the new-branch/no-old_oid case; if old_oid was
                // set, this means the pushed range isn't a descendant of it
                // — force push past an unrelated history — and we treat the
                // whole tree as touched too, which is the safe default).
                collect_all_paths(odb, commit.tree, "", &mut touched).await?;
                break;
            }
        }
    }

    Ok((touched, false))
}

/// B3: reject a push if any commit between `old_oid` (exclusive) and
/// `new_oid` touches a path locked by someone other than `pusher`.
///
/// Perf: short-circuits before any tree walk when the repo has zero locks.
/// Fail-open (walk abandoned, `tracing::warn!`) when the pushed range
/// exceeds `MEDIAGIT_LOCKS_MAX_COMMITS` (default 1000) — a push whose walk
/// is that expensive shouldn't stall on lock enforcement.
///
/// Identity: `pusher` is `AuthUser.user_id` when auth is enabled. When auth
/// is disabled there is no proven identity for the pusher at this call
/// site (unlike lock creation, which can take an explicit `owner` from the
/// request body) — such a push can never prove it owns a lock, so any
/// touched, locked path rejects the push. Operators who need unauthenticated
/// push-side lock enforcement must front the server with something that
/// injects an `AuthUser` (e.g. a reverse proxy header mapped into auth).
pub async fn check_push_locks(
    state: &AppState,
    repo: &str,
    repo_path: &Path,
    odb: &Arc<ObjectDatabase>,
    old_oid: Option<Oid>,
    new_oid: Oid,
    pusher: Option<&str>,
) -> Result<Option<String>, StatusCode> {
    if std::env::var("MEDIAGIT_LOCKS_ENFORCE").as_deref() == Ok("0") {
        return Ok(None);
    }

    let locks = get_repo_locks(state, repo, repo_path).await?;
    if locks.is_empty() {
        return Ok(None);
    }

    let max_commits: usize = std::env::var("MEDIAGIT_LOCKS_MAX_COMMITS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(1000);

    let (touched, capped) = touched_paths_for_push(odb, new_oid, old_oid, max_commits)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    if capped {
        tracing::warn!(
            "Lock enforcement fail-open for '{}': pushed range exceeds MEDIAGIT_LOCKS_MAX_COMMITS ({})",
            repo,
            max_commits
        );
        return Ok(None);
    }

    for path in &touched {
        if let Some(lock) = locks.get(path) {
            let is_owner = pusher.map(|p| p == lock.owner).unwrap_or(false);
            if !is_owner {
                return Ok(Some(format!(
                    "'{}' is locked by {} (lock {})",
                    path, lock.owner, lock.lock_id
                )));
            }
        }
    }

    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use mediagit_storage::LocalBackend;
    use mediagit_versioning::{FileMode, Signature, Tree, TreeEntry};

    async fn new_state_and_repo(
        tmp: &std::path::Path,
        repo: &str,
    ) -> (AppState, std::path::PathBuf) {
        let repo_path = tmp.join(repo);
        tokio::fs::create_dir_all(&repo_path).await.unwrap();
        let state = AppState::new(tmp.to_path_buf());
        (state, repo_path)
    }

    async fn new_odb(repo_path: &std::path::Path) -> Arc<ObjectDatabase> {
        let storage = LocalBackend::new(repo_path.join(".mediagit"))
            .await
            .expect("local backend");
        Arc::new(ObjectDatabase::with_smart_compression(
            Arc::new(storage),
            100,
        ))
    }

    fn sig() -> Signature {
        Signature::now("Tester".to_string(), "tester@example.com".to_string())
    }

    async fn commit_with_tree(
        odb: &ObjectDatabase,
        parent: Option<Oid>,
        files: Vec<(&str, &[u8])>,
    ) -> Oid {
        let mut tree = Tree::new();
        for (name, content) in files {
            let oid = Oid::hash(content);
            tree.add_entry(TreeEntry::new(name.to_string(), FileMode::Regular, oid));
        }
        let tree_oid = tree.write(odb).await.unwrap();
        let commit = match parent {
            Some(p) => Commit::with_parents(tree_oid, vec![p], sig(), sig(), "msg".to_string()),
            None => Commit::new(tree_oid, sig(), sig(), "msg".to_string()),
        };
        commit.write(odb).await.unwrap()
    }

    // ---- B1: store roundtrip ----

    #[tokio::test]
    async fn create_list_delete_roundtrip() {
        let tmp = tempfile::tempdir().unwrap();
        let (state, repo_path) = new_state_and_repo(tmp.path(), "repo1").await;

        let outcome = create_lock(
            &state,
            "repo1",
            &repo_path,
            "assets/model.fbx".to_string(),
            "alice".to_string(),
        )
        .await
        .unwrap();
        let record = match outcome {
            CreateLockOutcome::Created(r) => r,
            CreateLockOutcome::AlreadyLocked(_) => panic!("expected Created"),
        };
        assert_eq!(record.path, "assets/model.fbx");
        assert_eq!(record.owner, "alice");

        let map = get_repo_locks(&state, "repo1", &repo_path).await.unwrap();
        assert_eq!(map.len(), 1);
        assert!(map.contains_key("assets/model.fbx"));

        let deleted = delete_lock(&state, "repo1", &repo_path, &record.lock_id)
            .await
            .unwrap();
        assert!(deleted);

        let map = get_repo_locks(&state, "repo1", &repo_path).await.unwrap();
        assert!(map.is_empty());
    }

    #[tokio::test]
    async fn persists_across_reload() {
        let tmp = tempfile::tempdir().unwrap();
        let (state, repo_path) = new_state_and_repo(tmp.path(), "repo1").await;

        create_lock(
            &state,
            "repo1",
            &repo_path,
            "a.psd".to_string(),
            "bob".to_string(),
        )
        .await
        .unwrap();

        // Fresh AppState (no in-memory cache) simulates a server restart.
        let state2 = AppState::new(tmp.path().to_path_buf());
        let map = get_repo_locks(&state2, "repo1", &repo_path).await.unwrap();
        assert_eq!(map.len(), 1);
        assert_eq!(map.get("a.psd").unwrap().owner, "bob");

        let content = tokio::fs::read_to_string(locks_file_path(&repo_path))
            .await
            .unwrap();
        let mut lines = content.lines();
        assert_eq!(lines.next().unwrap(), r#"{"v":1}"#);
    }

    #[tokio::test]
    async fn load_rejects_future_version_header() {
        let tmp = tempfile::tempdir().unwrap();
        let repo_path = tmp.path().join("repo1");
        tokio::fs::create_dir_all(&repo_path).await.unwrap();
        let path = locks_file_path(&repo_path);
        tokio::fs::create_dir_all(path.parent().unwrap())
            .await
            .unwrap();
        tokio::fs::write(&path, "{\"v\":2}\n").await.unwrap();

        let result = load_locks_file(&path).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn double_lock_returns_already_locked() {
        let tmp = tempfile::tempdir().unwrap();
        let (state, repo_path) = new_state_and_repo(tmp.path(), "repo1").await;

        create_lock(
            &state,
            "repo1",
            &repo_path,
            "x.png".to_string(),
            "alice".to_string(),
        )
        .await
        .unwrap();

        let outcome = create_lock(
            &state,
            "repo1",
            &repo_path,
            "x.png".to_string(),
            "bob".to_string(),
        )
        .await
        .unwrap();

        match outcome {
            CreateLockOutcome::AlreadyLocked(existing) => assert_eq!(existing.owner, "alice"),
            CreateLockOutcome::Created(_) => panic!("expected AlreadyLocked"),
        }
    }

    // ---- B3: push enforcement ----

    #[tokio::test]
    async fn push_rejected_when_touching_lock_owned_by_other_user() {
        let tmp = tempfile::tempdir().unwrap();
        let (state, repo_path) = new_state_and_repo(tmp.path(), "repo1").await;
        let odb = new_odb(&repo_path).await;

        let base = commit_with_tree(&odb, None, vec![("a.txt", b"1")]).await;
        let next = commit_with_tree(&odb, Some(base), vec![("a.txt", b"2")]).await;

        create_lock(
            &state,
            "repo1",
            &repo_path,
            "a.txt".to_string(),
            "alice".to_string(),
        )
        .await
        .unwrap();

        let err = check_push_locks(
            &state,
            "repo1",
            &repo_path,
            &odb,
            Some(base),
            next,
            Some("bob"),
        )
        .await
        .unwrap();

        assert!(err.is_some());
        assert!(err.unwrap().contains("locked by alice"));
    }

    #[tokio::test]
    async fn push_passes_when_same_owner() {
        let tmp = tempfile::tempdir().unwrap();
        let (state, repo_path) = new_state_and_repo(tmp.path(), "repo1").await;
        let odb = new_odb(&repo_path).await;

        let base = commit_with_tree(&odb, None, vec![("a.txt", b"1")]).await;
        let next = commit_with_tree(&odb, Some(base), vec![("a.txt", b"2")]).await;

        create_lock(
            &state,
            "repo1",
            &repo_path,
            "a.txt".to_string(),
            "alice".to_string(),
        )
        .await
        .unwrap();

        let result = check_push_locks(
            &state,
            "repo1",
            &repo_path,
            &odb,
            Some(base),
            next,
            Some("alice"),
        )
        .await
        .unwrap();

        assert!(result.is_none());
    }

    #[tokio::test]
    async fn push_passes_when_no_locks_touched() {
        let tmp = tempfile::tempdir().unwrap();
        let (state, repo_path) = new_state_and_repo(tmp.path(), "repo1").await;
        let odb = new_odb(&repo_path).await;

        let base = commit_with_tree(&odb, None, vec![("a.txt", b"1"), ("b.txt", b"1")]).await;
        let next = commit_with_tree(&odb, Some(base), vec![("a.txt", b"1"), ("b.txt", b"2")]).await;

        create_lock(
            &state,
            "repo1",
            &repo_path,
            "a.txt".to_string(),
            "alice".to_string(),
        )
        .await
        .unwrap();

        let result = check_push_locks(
            &state,
            "repo1",
            &repo_path,
            &odb,
            Some(base),
            next,
            Some("bob"),
        )
        .await
        .unwrap();

        assert!(result.is_none());
    }

    #[tokio::test]
    async fn fail_open_beyond_max_commits() {
        let tmp = tempfile::tempdir().unwrap();
        let (state, repo_path) = new_state_and_repo(tmp.path(), "repo1").await;
        let odb = new_odb(&repo_path).await;

        let mut tip = commit_with_tree(&odb, None, vec![("a.txt", b"0")]).await;
        for i in 1..5u32 {
            tip =
                commit_with_tree(&odb, Some(tip), vec![("a.txt", i.to_string().as_bytes())]).await;
        }

        create_lock(
            &state,
            "repo1",
            &repo_path,
            "a.txt".to_string(),
            "alice".to_string(),
        )
        .await
        .unwrap();

        // Cap of 2 commits can't reach the (unrelated) old_oid, so the walk
        // must fail open rather than rejecting or erroring.
        let unrelated_old = Oid::hash(b"not-in-history");
        let result = check_push_locks(
            &state,
            "repo1",
            &repo_path,
            &odb,
            Some(unrelated_old),
            tip,
            Some("bob"),
        )
        .await;

        // Directly exercise the capped walk to confirm fail-open, since
        // check_push_locks reads MEDIAGIT_LOCKS_MAX_COMMITS from env (not
        // overridden here to avoid cross-test env races).
        let (_touched, capped) = touched_paths_for_push(&odb, tip, Some(unrelated_old), 2)
            .await
            .unwrap();
        assert!(capped);
        // With the default (large) cap, check_push_locks completes the walk
        // and correctly rejects since it never finds old_oid — the whole
        // history is treated as touched (root-commit fallback).
        assert!(result.unwrap().is_some());
    }
}
