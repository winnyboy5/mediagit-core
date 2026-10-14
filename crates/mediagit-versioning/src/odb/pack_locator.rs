// SPDX-License-Identifier: BUSL-1.1
// Copyright (C) 2025-2026 Aswin Krishnamoorthy

//! Compact in-memory index of where every packed object lives, so a packed read
//! is one ranged read instead of a whole-pack fetch + checksum per object.
//!
//! Memory: a flat vector sorted by oid, 48 bytes per entry exactly (32 oid +
//! 16 location) with no hash-table slack, looked up by binary search. Pack keys
//! are interned, so an entry never carries a string.

use crate::Oid;
use std::collections::HashMap;
use std::sync::Arc;

/// Where one packed entry lives: pack key plus the entry's absolute byte range
/// (header + payload), taken from the pack's own index. Built on demand.
#[derive(Clone)]
pub(super) struct PackLocation {
    pub(super) pack_key: Arc<str>,
    pub(super) offset: u64,
    pub(super) len: u32,
}

/// Stored form: the pack key is interned (`pack` indexes `PackLocator::packs`).
/// `len` is a `u32` because a pack entry is a single chunk/object (packs are
/// capped far below 4 GiB).
#[derive(Clone, Copy)]
struct Loc {
    pack: u32,
    len: u32,
    offset: u64,
}

/// `Oid -> location` over every indexed pack. `packs` also records which pack
/// keys have been indexed (packs are immutable, so a refresh only needs to
/// index keys not yet seen).
///
/// Call [`finish`](Self::finish) after a batch of `insert`s; lookups before that
/// do not see the new entries.
#[derive(Default)]
pub(super) struct PackLocator {
    packs: Vec<Arc<str>>,
    pack_ids: HashMap<Arc<str>, u32>,
    /// Sorted by oid; the first location of each oid. `entries[..sorted]` is
    /// sorted, the tail is awaiting `finish`.
    entries: Vec<(Oid, Loc)>,
    sorted: usize,
    /// Extra locations, only for the (rare) oids packed in more than one pack.
    duplicates: HashMap<Oid, Vec<Loc>>,
}

impl PackLocator {
    pub(super) fn has_pack(&self, pack_key: &str) -> bool {
        self.pack_ids.contains_key(pack_key)
    }

    pub(super) fn pack_count(&self) -> usize {
        self.packs.len()
    }

    /// Register a pack key and return its interned id.
    pub(super) fn add_pack(&mut self, pack_key: &str) -> u32 {
        let key: Arc<str> = Arc::from(pack_key);
        let id = self.packs.len() as u32;
        self.packs.push(key.clone());
        self.pack_ids.insert(key, id);
        id
    }

    pub(super) fn insert(&mut self, oid: Oid, pack: u32, offset: u64, len: u32) {
        self.entries.push((oid, Loc { pack, len, offset }));
    }

    /// Sort the pending entries in, move oids that appear more than once into
    /// the side map, and release spare capacity.
    pub(super) fn finish(&mut self) {
        if self.sorted == self.entries.len() {
            return;
        }
        self.entries
            .sort_unstable_by_key(|(oid, loc)| (*oid, loc.pack, loc.offset));
        // Compact in place (no second allocation): keep the first of each oid.
        let mut w = 0;
        for r in 0..self.entries.len() {
            let (oid, loc) = self.entries[r];
            if w > 0 && self.entries[w - 1].0 == oid {
                self.duplicates.entry(oid).or_default().push(loc);
            } else {
                self.entries[w] = (oid, loc);
                w += 1;
            }
        }
        self.entries.truncate(w);
        self.entries.shrink_to_fit();
        self.sorted = self.entries.len();
    }

    fn find(&self, oid: &Oid) -> Option<Loc> {
        let sorted = &self.entries[..self.sorted];
        sorted
            .binary_search_by(|(o, _)| o.cmp(oid))
            .ok()
            .map(|i| sorted[i].1)
    }

    pub(super) fn contains(&self, oid: &Oid) -> bool {
        self.find(oid).is_some()
    }

    pub(super) fn iter(&self) -> impl Iterator<Item = &Oid> {
        self.entries[..self.sorted].iter().map(|(oid, _)| oid)
    }

    fn resolve(&self, loc: Loc) -> PackLocation {
        PackLocation {
            pack_key: self.packs[loc.pack as usize].clone(),
            offset: loc.offset,
            len: loc.len,
        }
    }

    /// Every location `oid` is packed at (usually one), primary first.
    pub(super) fn locations(&self, oid: &Oid) -> Vec<PackLocation> {
        let Some(first) = self.find(oid) else {
            return Vec::new();
        };
        let mut out = vec![self.resolve(first)];
        if let Some(more) = self.duplicates.get(oid) {
            out.extend(more.iter().map(|l| self.resolve(*l)));
        }
        out
    }

    /// The location of `oid` inside one specific pack (delta bases live there).
    pub(super) fn location_in(&self, oid: &Oid, pack_key: &str) -> Option<PackLocation> {
        let id = *self.pack_ids.get(pack_key)?;
        let first = self.find(oid)?;
        std::iter::once(first)
            .chain(self.duplicates.get(oid).into_iter().flatten().copied())
            .find(|l| l.pack == id)
            .map(|l| self.resolve(l))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entry_is_forty_eight_bytes() {
        assert_eq!(std::mem::size_of::<Loc>(), 16);
        assert_eq!(std::mem::size_of::<(Oid, Loc)>(), 48);
    }

    #[test]
    fn duplicates_and_pack_scoped_lookup() {
        let mut l = PackLocator::default();
        let a = l.add_pack("packs/a");
        let b = l.add_pack("packs/b");
        let oid = Oid::hash(b"x");
        let other = Oid::hash(b"y");
        l.insert(oid, b, 40, 100);
        l.insert(other, a, 99, 1);
        l.insert(oid, a, 13, 100);
        l.finish();
        let locs = l.locations(&oid);
        assert_eq!(locs.len(), 2);
        assert_eq!(&*locs[0].pack_key, "packs/a", "lowest pack id is primary");
        assert_eq!(l.location_in(&oid, "packs/b").unwrap().offset, 40);
        assert!(l.location_in(&oid, "packs/missing").is_none());
        assert!(l.has_pack("packs/a") && !l.has_pack("packs/z"));
        assert!(l.contains(&other) && !l.contains(&Oid::hash(b"z")));

        // A later batch merges into the sorted vector and keeps earlier entries.
        let c = l.add_pack("packs/c");
        l.insert(Oid::hash(b"z"), c, 7, 3);
        assert!(!l.contains(&Oid::hash(b"z")), "not visible before finish");
        l.finish();
        assert!(l.contains(&Oid::hash(b"z")) && l.contains(&oid) && l.contains(&other));
        assert_eq!(l.locations(&oid).len(), 2);
    }

    /// Heap cost of the locator for realistic chunk counts. Exact for the
    /// entry vector (capacity is shrunk to length); packs are noise.
    #[test]
    fn locator_memory_per_entry() {
        for n in [100_000usize, 1_000_000] {
            let mut l = PackLocator::default();
            let packs: Vec<u32> = (0..(n / 1024 + 1))
                .map(|i| l.add_pack(&format!("packs/{i:064x}")))
                .collect();
            for i in 0..n {
                let oid = Oid::hash(&(i as u64).to_le_bytes());
                l.insert(oid, packs[i / 1024], 13 + (i % 1024) as u64 * 4096, 4096);
            }
            l.finish();
            assert!(l.duplicates.is_empty());
            assert_eq!(l.entries.capacity(), l.entries.len(), "no spare capacity");
            let bytes = l.entries.capacity() * std::mem::size_of::<(Oid, Loc)>()
                + l.packs.len() * (16 + 64 + 40);
            let per_entry = bytes as f64 / n as f64;
            println!(
                "locator n={n}: {:.1} MiB total, {per_entry:.1} B/entry",
                bytes as f64 / 1048576.0
            );
            assert!(per_entry < 52.0, "locator too large: {per_entry} B/entry");
            // Every entry is findable.
            for i in (0..n).step_by(9973) {
                assert!(l.contains(&Oid::hash(&(i as u64).to_le_bytes())));
            }
        }
    }
}
