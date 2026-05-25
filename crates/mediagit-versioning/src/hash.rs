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

#[doc = "Sole hash entry point. New algos extend here."]
pub struct Hasher(blake3::Hasher);

impl Hasher {
    pub fn new() -> Self {
        Hasher(blake3::Hasher::new())
    }

    pub fn update(&mut self, data: &[u8]) -> &mut Self {
        self.0.update(data);
        self
    }

    /// Tree-parallel update via rayon. Produces byte-identical output to sequential `update`
    /// (BLAKE3 tree-hash spec guarantees this). Only valid for a single contiguous slice such
    /// as an mmap — do not mix with sequential `update` calls on the same hasher instance.
    pub fn update_rayon(&mut self, data: &[u8]) -> &mut Self {
        self.0.update_rayon(data);
        self
    }

    pub fn finalize(self) -> [u8; 32] {
        *self.0.finalize().as_bytes()
    }
}

impl Default for Hasher {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_blake3_directly() {
        let data = b"hello mediagit";
        let mut h = Hasher::new();
        h.update(data);
        let result = h.finalize();
        assert_eq!(result, *blake3::hash(data).as_bytes());
    }

    #[test]
    fn incremental_matches_single_pass() {
        let mut h = Hasher::new();
        h.update(b"hello ");
        h.update(b"mediagit");
        let incremental = h.finalize();

        let mut h2 = Hasher::new();
        h2.update(b"hello mediagit");
        let single = h2.finalize();
        assert_eq!(incremental, single);
    }

    /// R4 — BLAKE3 funnel roundtrip property test.
    ///
    /// Verifies that `hash::Hasher` is a transparent shim over `blake3::hash`,
    /// that `Oid::hash` routes through the same funnel, and that the 64-char hex
    /// representation roundtrips through `from_hex`.
    ///
    /// Marked `#[ignore]` because 10k iterations take ~200 ms; run explicitly with
    /// `cargo test -p mediagit-versioning -- --include-ignored hash_funnel_roundtrip`.
    #[test]
    #[ignore = "slow: 10k BLAKE3 roundtrips — run with --include-ignored"]
    fn hash_funnel_roundtrip() {
        use crate::oid::Oid;
        // Deterministic XorShift PRNG — reproducible without a rand dep.
        let mut state = 0xDEAD_BEEF_CAFE_BABEu64;
        let next_byte = |s: &mut u64| -> u8 {
            *s ^= *s << 13;
            *s ^= *s >> 7;
            *s ^= *s << 17;
            (*s & 0xFF) as u8
        };

        for i in 0..10_000usize {
            let len = (i % 4096) + 1; // 1..=4096 bytes, cycling
            let data: Vec<u8> = (0..len).map(|_| next_byte(&mut state)).collect();

            // 1. Hasher shim output matches blake3::hash directly.
            let mut h = Hasher::new();
            h.update(&data);
            let digest = h.finalize();
            assert_eq!(
                digest,
                *blake3::hash(&data).as_bytes(),
                "iter {i}: Hasher diverged from blake3::hash"
            );

            // 2. Oid::hash routes through the same funnel (verify via hex).
            let oid = Oid::hash(&data);
            assert_eq!(
                oid.to_hex(),
                hex::encode(digest),
                "iter {i}: Oid::hash hex diverged from Hasher"
            );

            // 3. to_hex / from_hex roundtrip preserves identity.
            let hex_str = oid.to_hex();
            assert_eq!(hex_str.len(), 64, "iter {i}: hex must be 64 chars");
            let roundtripped =
                Oid::from_hex(&hex_str).expect("from_hex must not fail on valid hex");
            assert_eq!(oid, roundtripped, "iter {i}: from_hex(to_hex(oid)) ≠ oid");
        }
    }
}
