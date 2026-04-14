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

//! Object-graph reachability walker.
//!
//! Computes the set of OIDs reachable from a set of root commits, walking
//! commits → trees → blobs. Supports a `stop_at` cutoff set: any OID in
//! `stop_at` is neither visited nor recursed into, which is exactly what the
//! pack-negotiation path needs ("collect everything reachable from `want`
//! that is not already reachable from `have`").
//!
//! Unknown or unreadable OIDs are skipped silently — callers often feed in
//! stale haves from clients and must not fail the whole walk on one bad OID.

use crate::{Commit, ObjectDatabase, Oid, Tree};
use std::collections::{HashSet, VecDeque};

/// Walk the full object closure reachable from `roots`, stopping at any OID
/// present in `stop_at`.
///
/// Returns the set of visited OIDs (not including anything in `stop_at`).
/// Traverses commits → (parents, tree) and trees → (entries). Blobs are leaf
/// nodes. Chunked blobs are detected via [`ObjectDatabase::is_chunked`] and
/// added to the result without reading their data — matching the existing
/// pack-walker behavior in `mediagit-server`.
///
/// # Leniency
///
/// OIDs that cannot be read from the ODB are skipped silently. This is
/// intentional: the server receives `have` OIDs from remote clients and must
/// tolerate stale or unknown values without erroring the entire fetch.
pub async fn walk_reachable<I>(
    odb: &ObjectDatabase,
    roots: I,
    stop_at: &HashSet<Oid>,
) -> anyhow::Result<HashSet<Oid>>
where
    I: IntoIterator<Item = Oid>,
{
    let mut visited: HashSet<Oid> = HashSet::new();
    let mut queue: VecDeque<Oid> = VecDeque::new();

    for oid in roots {
        if stop_at.contains(&oid) || !visited.insert(oid) {
            continue;
        }
        queue.push_back(oid);
    }

    while let Some(oid) = queue.pop_front() {
        // Chunked blobs: never recurse, never read — the manifest lookup is
        // cheap and avoids reassembling multi-GB files just to mark them.
        if odb.is_chunked(&oid).await.unwrap_or(false) {
            continue;
        }

        // Lenient read: unknown OIDs (stale haves, corrupted state) are
        // simply not expanded. They remain in `visited` so the caller sees
        // they were "reached".
        let data = match odb.read(&oid).await {
            Ok(data) => data,
            Err(_) => continue,
        };

        // Try commit, then tree, then fall through as blob (leaf).
        if let Ok(commit) = Commit::deserialize(&data) {
            enqueue(commit.tree, stop_at, &mut visited, &mut queue);
            for parent in commit.parents {
                enqueue(parent, stop_at, &mut visited, &mut queue);
            }
            continue;
        }

        if let Ok(tree) = Tree::deserialize(&data) {
            for entry in tree.iter() {
                enqueue(entry.oid, stop_at, &mut visited, &mut queue);
            }
            continue;
        }

        // Blob: leaf node, already inserted when enqueued.
    }

    Ok(visited)
}

#[inline]
fn enqueue(
    oid: Oid,
    stop_at: &HashSet<Oid>,
    visited: &mut HashSet<Oid>,
    queue: &mut VecDeque<Oid>,
) {
    if stop_at.contains(&oid) {
        return;
    }
    if visited.insert(oid) {
        queue.push_back(oid);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{FileMode, ObjectType, Signature, TreeEntry};
    use mediagit_storage::{LocalBackend, StorageBackend};
    use std::sync::Arc;
    use tempfile::TempDir;

    async fn make_odb() -> (TempDir, ObjectDatabase) {
        let tmp = TempDir::new().unwrap();
        let storage: Arc<dyn StorageBackend> =
            Arc::new(LocalBackend::new(tmp.path()).await.unwrap());
        let odb = ObjectDatabase::new(storage, 100);
        (tmp, odb)
    }

    // Build a tiny linear history: root blob → tree → commit.
    async fn write_commit(
        odb: &ObjectDatabase,
        content: &[u8],
        filename: &str,
        parents: Vec<Oid>,
    ) -> (Oid, Oid, Oid) {
        let blob = odb.write(ObjectType::Blob, content).await.unwrap();
        let mut tree = Tree::new();
        tree.add_entry(TreeEntry::new(
            filename.to_string(),
            FileMode::Regular,
            blob,
        ));
        let tree_oid = tree.write(odb).await.unwrap();
        let author = Signature::now("t".to_string(), "t@e".to_string());
        let mut commit = Commit::new(tree_oid, author.clone(), author, "msg".to_string());
        commit.parents = parents;
        let commit_oid = commit.write(odb).await.unwrap();
        (commit_oid, tree_oid, blob)
    }

    #[tokio::test]
    async fn walks_full_closure_when_stop_empty() {
        let (_tmp, odb) = make_odb().await;
        let (c1, t1, b1) = write_commit(&odb, b"v1", "a.txt", vec![]).await;

        let visited = walk_reachable(&odb, [c1], &HashSet::new()).await.unwrap();
        assert_eq!(visited.len(), 3);
        assert!(visited.contains(&c1));
        assert!(visited.contains(&t1));
        assert!(visited.contains(&b1));
    }

    #[tokio::test]
    async fn stop_at_prunes_subtree() {
        // c2 -> c1 ; walk from c2 with stop={c1} should visit only c2 + t2 + b2.
        let (_tmp, odb) = make_odb().await;
        let (c1, _t1, _b1) = write_commit(&odb, b"v1", "a.txt", vec![]).await;
        let (c2, t2, b2) = write_commit(&odb, b"v2", "b.txt", vec![c1]).await;

        let mut stop = HashSet::new();
        stop.insert(c1);

        let visited = walk_reachable(&odb, [c2], &stop).await.unwrap();
        assert_eq!(visited.len(), 3, "expected c2+t2+b2, got {:?}", visited);
        assert!(visited.contains(&c2));
        assert!(visited.contains(&t2));
        assert!(visited.contains(&b2));
        assert!(!visited.contains(&c1));
    }

    #[tokio::test]
    async fn stop_at_prunes_shared_blob() {
        // Two commits sharing the same blob (by content hash). If the shared
        // blob is in stop_at, it must be excluded from the result set.
        let (_tmp, odb) = make_odb().await;
        let (c1, _t1, b_shared) = write_commit(&odb, b"shared", "a.txt", vec![]).await;

        let mut stop = HashSet::new();
        stop.insert(b_shared);

        let visited = walk_reachable(&odb, [c1], &stop).await.unwrap();
        assert!(!visited.contains(&b_shared));
        assert!(visited.contains(&c1));
    }

    #[tokio::test]
    async fn unknown_root_is_lenient() {
        let (_tmp, odb) = make_odb().await;
        let fake = Oid::hash(b"definitely-not-in-odb");

        let visited = walk_reachable(&odb, [fake], &HashSet::new()).await.unwrap();
        // The fake OID is still marked as visited (we tried); it just has no
        // children to expand. No error is returned — this is the point.
        assert_eq!(visited.len(), 1);
        assert!(visited.contains(&fake));
    }

    #[tokio::test]
    async fn diamond_history_visits_tree_once() {
        // c3 -> c1, c2 -> c1 ; shared tree and blob should appear exactly once.
        let (_tmp, odb) = make_odb().await;
        let (c1, t1, b1) = write_commit(&odb, b"v1", "a.txt", vec![]).await;

        // c2 branches off c1 with a new file (new tree, new blob).
        let (c2, _t2, _b2) = write_commit(&odb, b"v2", "b.txt", vec![c1]).await;

        // c3 is a merge of c1 and c2 — reuses t1's tree (same content), so
        // traversal must dedupe.
        let author = Signature::now("t".to_string(), "t@e".to_string());
        let mut merge_commit = Commit::new(t1, author.clone(), author, "merge".to_string());
        merge_commit.parents = vec![c1, c2];
        let c3 = merge_commit.write(&odb).await.unwrap();

        let visited = walk_reachable(&odb, [c3], &HashSet::new()).await.unwrap();
        // Expected: {c1, c2, c3, t1, t2, b1, b2} = 7 objects, each once.
        assert_eq!(visited.len(), 7, "got {:?}", visited);
        assert!(visited.contains(&c1));
        assert!(visited.contains(&c2));
        assert!(visited.contains(&c3));
        assert!(visited.contains(&t1));
        assert!(visited.contains(&b1));
    }
}
