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

//! Roaring-bitmap-backed reachability index (M3, roadmap #2b).
//!
//! Persists a commit's full object closure (everything [`walk_reachable`]
//! would visit from it) as a compact, versioned artifact under the
//! `bitmaps/` logical namespace, so pack negotiation can skip the BFS walk
//! (one ODB read per object) when a valid bitmap exists for the client's
//! `have` tip.
//!
//! # Correctness contract
//!
//! This is **derived data**: a pure speedup, never a correctness
//! dependency. Any miss, staleness, corruption, or format-version mismatch
//! must silently fall back to [`walk_reachable`] — never error the caller.
//! `gc` may prune bitmaps for commits no longer reachable; it must never
//! treat a missing/stale bitmap as a corruption signal.

use crate::{ObjectDatabase, Oid, walk_reachable};
use anyhow::Result;
use roaring::RoaringBitmap;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

/// Bumped whenever the on-disk format changes. Readers reject any other
/// version by falling back to BFS rather than erroring — see the module
/// doc's correctness contract.
const BITMAP_FORMAT_VERSION: u8 = 1;

/// Read `MEDIAGIT_BITMAP` (default ON). Set to `0`/`false`/`off` to disable
/// both generation and consumption — every caller falls back to BFS.
pub fn bitmap_enabled() -> bool {
    match std::env::var("MEDIAGIT_BITMAP") {
        Ok(v) => !matches!(v.to_ascii_lowercase().as_str(), "0" | "false" | "off"),
        Err(_) => true,
    }
}

/// Logical storage key for a commit's reachability bitmap.
pub fn bitmap_key(commit_oid: &Oid) -> String {
    format!("bitmaps/{}.bitmap", commit_oid.to_hex())
}

/// A reachability bitmap: dense u32 ids <-> OIDs for one commit's full
/// object closure (commits/trees/blobs reachable from it, per
/// [`walk_reachable`]).
///
/// The id space is local to this bitmap file (not a global, cross-commit
/// numbering) — simple and sufficient for the single-bitmap lookup this
/// module supports today; multi-bitmap set algebra (a global id space
/// enabling cheap AND/OR across commits) is future scope.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct BitmapPayload {
    /// id -> Oid, index is the dense id.
    ids: Vec<Oid>,
    /// Membership set (always `0..ids.len()` today — every id in `ids` is a
    /// member by construction — but kept as a real roaring bitmap so the
    /// on-disk format doesn't need to change if future generation logic
    /// produces a sparser membership set).
    #[serde(with = "roaring_bytes")]
    bitmap: RoaringBitmap,
}

/// Serde adapter: (de)serialize a `RoaringBitmap` via its own portable
/// binary format (not derived serde), wrapped as a postcard byte string.
mod roaring_bytes {
    use roaring::RoaringBitmap;
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(bitmap: &RoaringBitmap, s: S) -> Result<S::Ok, S::Error> {
        let mut bytes = Vec::with_capacity(bitmap.serialized_size());
        bitmap
            .serialize_into(&mut bytes)
            .map_err(serde::ser::Error::custom)?;
        s.serialize_bytes(&bytes)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<RoaringBitmap, D::Error> {
        let bytes: Vec<u8> = Vec::deserialize(d)?;
        RoaringBitmap::deserialize_from(&bytes[..]).map_err(serde::de::Error::custom)
    }
}

/// A commit's reachability bitmap, ready to serialize/deserialize or query.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReachabilityBitmap {
    payload: BitmapPayload,
}

impl ReachabilityBitmap {
    /// Build from an already-computed OID set (e.g. from [`walk_reachable`]).
    pub fn from_oids(oids: &HashSet<Oid>) -> Self {
        let ids: Vec<Oid> = oids.iter().copied().collect();
        let bitmap: RoaringBitmap = (0..ids.len() as u32).collect();
        Self {
            payload: BitmapPayload { ids, bitmap },
        }
    }

    /// Compute by walking the full reachability closure from `commit_oid`.
    pub async fn generate(odb: &ObjectDatabase, commit_oid: Oid) -> Result<Self> {
        let empty = HashSet::new();
        let oids = walk_reachable(odb, [commit_oid], &empty).await?;
        Ok(Self::from_oids(&oids))
    }

    /// Number of objects in the closure.
    pub fn len(&self) -> usize {
        self.payload.ids.len()
    }

    pub fn is_empty(&self) -> bool {
        self.payload.ids.is_empty()
    }

    /// Materialize the OID set this bitmap represents.
    pub fn to_oid_set(&self) -> HashSet<Oid> {
        self.payload
            .bitmap
            .iter()
            .filter_map(|id| self.payload.ids.get(id as usize).copied())
            .collect()
    }

    /// Serialize: `[version byte][postcard-encoded payload]`.
    pub fn serialize(&self) -> Result<Vec<u8>> {
        let body = crate::format::serialize(&self.payload)?;
        let mut out = Vec::with_capacity(1 + body.len());
        out.push(BITMAP_FORMAT_VERSION);
        out.extend_from_slice(&body);
        Ok(out)
    }

    /// Deserialize. Returns `None` (not an error) on a version mismatch,
    /// truncated header, or malformed payload — callers must fall back to
    /// BFS, never fail, on any of these (see module doc).
    pub fn deserialize(bytes: &[u8]) -> Option<Self> {
        let (&version, body) = bytes.split_first()?;
        if version != BITMAP_FORMAT_VERSION {
            return None;
        }
        let payload: BitmapPayload = crate::format::deserialize(body).ok()?;
        Some(Self { payload })
    }
}

#[cfg(test)]
#[allow(unsafe_code)] // edition-2024: test-only env::set_var/remove_var requires unsafe
mod tests {
    use super::*;
    use crate::{Commit, FileMode, ObjectType, Signature, Tree, TreeEntry};
    use mediagit_storage::LocalBackend;
    use std::sync::Arc;
    use tempfile::TempDir;

    async fn make_odb() -> (TempDir, ObjectDatabase) {
        let tmp = TempDir::new().unwrap();
        let storage: Arc<dyn mediagit_storage::StorageBackend> =
            Arc::new(LocalBackend::new(tmp.path()).await.unwrap());
        (tmp, ObjectDatabase::new(storage, 100))
    }

    #[test]
    fn round_trip_preserves_oid_set() {
        let oids: HashSet<Oid> = (0..50)
            .map(|i| Oid::hash(format!("obj-{i}").as_bytes()))
            .collect();
        let bitmap = ReachabilityBitmap::from_oids(&oids);
        let bytes = bitmap.serialize().unwrap();

        let decoded = ReachabilityBitmap::deserialize(&bytes).expect("valid bitmap must decode");
        assert_eq!(decoded.len(), oids.len());
        assert_eq!(decoded.to_oid_set(), oids);
    }

    #[test]
    fn empty_bytes_fall_back() {
        assert!(ReachabilityBitmap::deserialize(&[]).is_none());
    }

    #[test]
    fn wrong_version_byte_falls_back_not_errors() {
        let oids: HashSet<Oid> = HashSet::from([Oid::hash(b"a")]);
        let bitmap = ReachabilityBitmap::from_oids(&oids);
        let mut bytes = bitmap.serialize().unwrap();
        bytes[0] = 0xFF; // corrupt the version byte
        assert!(
            ReachabilityBitmap::deserialize(&bytes).is_none(),
            "unknown version must decode to None (fallback), not panic or Err"
        );
    }

    #[test]
    fn corrupt_payload_falls_back_not_errors() {
        let oids: HashSet<Oid> = HashSet::from([Oid::hash(b"a"), Oid::hash(b"b")]);
        let bitmap = ReachabilityBitmap::from_oids(&oids);
        let mut bytes = bitmap.serialize().unwrap();
        // Truncate the payload — still has a valid version byte but garbage body.
        bytes.truncate(bytes.len() / 2);
        assert!(
            ReachabilityBitmap::deserialize(&bytes).is_none(),
            "truncated/corrupt payload must decode to None (fallback), not panic or Err"
        );
    }

    #[tokio::test]
    async fn bitmap_matches_bfs_closure_on_synthetic_graph() {
        // Build a ~1K-commit linear+branching history and compare the
        // generated bitmap's closure against walk_reachable's BFS result.
        let (_tmp, odb) = make_odb().await;
        let author = Signature::now("t".to_string(), "t@e".to_string());

        let mut parent: Option<Oid> = None;
        let mut tip = Oid::hash(b"unused");
        for i in 0..1000 {
            let blob = odb
                .write(ObjectType::Blob, format!("v{i}").as_bytes())
                .await
                .unwrap();
            let mut tree = Tree::new();
            tree.add_entry(TreeEntry::new(
                format!("f{}.txt", i % 20), // reuse names so trees/blobs alias across history
                FileMode::Regular,
                blob,
            ));
            let tree_oid = tree.write(&odb).await.unwrap();
            let mut commit = Commit::new(tree_oid, author.clone(), author.clone(), format!("c{i}"));
            if let Some(p) = parent {
                commit.parents = vec![p];
            }
            let commit_oid = commit.write(&odb).await.unwrap();
            parent = Some(commit_oid);
            tip = commit_oid;
        }

        let bfs_closure = walk_reachable(&odb, [tip], &HashSet::new()).await.unwrap();
        let bitmap = ReachabilityBitmap::generate(&odb, tip).await.unwrap();

        assert_eq!(bitmap.to_oid_set(), bfs_closure);
        assert!(
            bitmap.len() > 1000,
            "closure must include trees/blobs, not just commits"
        );
    }

    #[test]
    fn bitmap_enabled_defaults_on_and_respects_knob() {
        // SAFETY: test-only env var scoping; no other test in this process
        // reads MEDIAGIT_BITMAP concurrently within this crate's suite.
        // FIXME: Audit that the environment access only happens in single-threaded code.
        unsafe { std::env::remove_var("MEDIAGIT_BITMAP") };
        assert!(bitmap_enabled(), "default must be ON");

        // FIXME: Audit that the environment access only happens in single-threaded code.
        unsafe { std::env::set_var("MEDIAGIT_BITMAP", "0") };
        assert!(!bitmap_enabled());

        // FIXME: Audit that the environment access only happens in single-threaded code.
        unsafe { std::env::set_var("MEDIAGIT_BITMAP", "false") };
        assert!(!bitmap_enabled());

        // FIXME: Audit that the environment access only happens in single-threaded code.
        unsafe { std::env::set_var("MEDIAGIT_BITMAP", "1") };
        assert!(bitmap_enabled());

        // FIXME: Audit that the environment access only happens in single-threaded code.
        unsafe { std::env::remove_var("MEDIAGIT_BITMAP") };
    }

    #[test]
    fn bitmap_key_is_namespaced_under_bitmaps() {
        let oid = Oid::hash(b"commit");
        assert_eq!(bitmap_key(&oid), format!("bitmaps/{}.bitmap", oid.to_hex()));
    }
}
