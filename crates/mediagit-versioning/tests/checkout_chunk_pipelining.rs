// SPDX-License-Identifier: BUSL-1.1
// Copyright (C) 2025-2026 Aswin Krishnamoorthy

//! `read_to_file` must reconstruct a chunked file with OVERLAPPED chunk reads,
//! not one chunk at a time.
//!
//! Clone pays a reconstruction stage that push has no equivalent of, and it ran
//! fully serially — `read -> decompress -> hash -> write`, awaited per chunk —
//! with nothing overlapping it, because it is phased strictly after the download
//! completes. `MEDIAGIT_CHECKOUT_PARALLELISM` does not help: it parallelises
//! across FILES, and a media corpus is typically a few enormous files, so the
//! file axis has nothing to spread.
//!
//! HONEST SCOPE, measured 2026-09-15 on the 16 GB corpus across all three cloud
//! backends: this stage is `op=checkout wall=15.45s / 15.04s / 18.40s` against
//! clones of 1,533s / 2,288s / 1,347s — about **1%**. It was ranked the top
//! lever on an estimate of ~160K chunks for a 10.18 GB file; the bench says
//! 4,054 chunks at `avg_chunk_mb=2.53`, because chunk size is tuned to FILE size
//! (`get_chunk_params`) and never approaches the ~64 KiB that estimate assumed.
//! The clone's real cost is the download, and the download's real competitor is
//! the post-push verification re-reading the same bytes.
//!
//! This test is kept because the pipelining is strictly better and
//! memory-neutral, and because a future edit could silently restore the serial
//! loop — not because it moves the headline number.
//!
//! BOTH halves are asserted here, deliberately:
//!   1. the reconstructed bytes are correct, and
//!   2. reads actually overlap.
//!
//! (1) alone would pass the serial implementation this test exists to prevent
//! regressing to — a correctness-only test cannot see the bug. (2) alone would
//! pass an implementation that reorders chunks and corrupts the file, which is
//! the specific hazard of using `buffer_unordered` instead of `buffered` here.

use mediagit_storage::{LocalBackend, StorageBackend};
use mediagit_versioning::chunking::ChunkStrategy;
use mediagit_versioning::{ObjectDatabase, ObjectType};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use tempfile::TempDir;

/// Wraps a real backend and records the high-water mark of concurrent `get`
/// calls for chunk objects.
///
/// The small delay is what makes the measurement meaningful: without it a
/// pipelined implementation can still complete each read before starting the
/// next, and max-in-flight would read 1 on correct code. With it, serial code
/// pins at exactly 1 and pipelined code climbs.
#[derive(Debug)]
struct ConcurrencyProbe {
    inner: Arc<dyn StorageBackend>,
    in_flight: AtomicUsize,
    max_in_flight: AtomicUsize,
    chunk_gets: AtomicUsize,
}

impl ConcurrencyProbe {
    fn new(inner: Arc<dyn StorageBackend>) -> Self {
        Self {
            inner,
            in_flight: AtomicUsize::new(0),
            max_in_flight: AtomicUsize::new(0),
            chunk_gets: AtomicUsize::new(0),
        }
    }

    async fn track<T>(&self, key: &str, fut: impl std::future::Future<Output = T>) -> T {
        let tracked = key.starts_with("chunks/");
        if tracked {
            self.chunk_gets.fetch_add(1, Ordering::SeqCst);
            let now = self.in_flight.fetch_add(1, Ordering::SeqCst) + 1;
            self.max_in_flight.fetch_max(now, Ordering::SeqCst);
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
        let out = fut.await;
        if tracked {
            self.in_flight.fetch_sub(1, Ordering::SeqCst);
        }
        out
    }
}

#[async_trait::async_trait]
impl StorageBackend for ConcurrencyProbe {
    async fn get(&self, key: &str) -> anyhow::Result<Vec<u8>> {
        self.track(key, self.inner.get(key)).await
    }
    async fn get_range(&self, key: &str, offset: u64, len: u64) -> anyhow::Result<Vec<u8>> {
        self.track(key, self.inner.get_range(key, offset, len))
            .await
    }
    async fn put(&self, key: &str, data: &[u8]) -> anyhow::Result<()> {
        self.inner.put(key, data).await
    }
    async fn exists(&self, key: &str) -> anyhow::Result<bool> {
        self.inner.exists(key).await
    }
    async fn delete(&self, key: &str) -> anyhow::Result<()> {
        self.inner.delete(key).await
    }
    async fn list_objects(&self, prefix: &str) -> anyhow::Result<Vec<String>> {
        self.inner.list_objects(prefix).await
    }
    async fn head(&self, key: &str) -> anyhow::Result<Option<u64>> {
        self.inner.head(key).await
    }
}

/// Incompressible, so chunking cannot collapse it into a single chunk and the
/// file genuinely reconstructs from many.
fn pseudo_random_bytes(len: usize, seed: u64) -> Vec<u8> {
    let mut out = Vec::with_capacity(len);
    let mut x = seed | 1;
    while out.len() < len {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        out.extend_from_slice(&x.to_le_bytes());
    }
    out.truncate(len);
    out
}

#[tokio::test]
async fn read_to_file_overlaps_chunk_reads_and_still_reconstructs_exactly() {
    let tmp = TempDir::new().unwrap();
    let local = Arc::new(
        LocalBackend::new(tmp.path().to_str().unwrap())
            .await
            .unwrap(),
    );
    let probe = Arc::new(ConcurrencyProbe::new(local.clone()));
    let storage: Arc<dyn StorageBackend> = probe.clone();

    // Fixed 64 KiB chunks over 2 MiB => 32 chunks: enough for overlap to be
    // unambiguous, small enough to stay a fast unit test.
    let odb = ObjectDatabase::with_optimizations(
        storage,
        100,
        Some(ChunkStrategy::Fixed { size: 64 * 1024 }),
        false,
        0,
    );

    let original = pseudo_random_bytes(2 * 1024 * 1024, 0x5EED);
    let oid = odb
        .write_chunked(ObjectType::Blob, &original, "payload.bin")
        .await
        .expect("write_chunked");

    // Only reads performed by read_to_file should count.
    probe.max_in_flight.store(0, Ordering::SeqCst);
    probe.in_flight.store(0, Ordering::SeqCst);
    probe.chunk_gets.store(0, Ordering::SeqCst);

    let out_path = tmp.path().join("reconstructed.bin");
    let written = odb
        .read_to_file(&oid, &out_path)
        .await
        .expect("read_to_file");

    // (1) correctness — byte-for-byte, in order.
    let round_tripped = tokio::fs::read(&out_path).await.expect("read back");
    assert_eq!(
        written as usize,
        original.len(),
        "read_to_file reported the wrong byte count"
    );
    assert_eq!(
        round_tripped.len(),
        original.len(),
        "reconstructed file is the wrong length"
    );
    assert!(
        round_tripped == original,
        "reconstructed file differs from the original — chunks were assembled out of \
         order or corrupted. `buffered` preserves manifest order; `buffer_unordered` \
         would not, and this is the assertion that catches that substitution."
    );

    let chunk_gets = probe.chunk_gets.load(Ordering::SeqCst);
    let max_in_flight = probe.max_in_flight.load(Ordering::SeqCst);

    assert!(
        chunk_gets > 4,
        "expected the payload to reconstruct from many chunks, saw {chunk_gets} chunk \
         reads — the fixture stopped exercising the multi-chunk path"
    );

    // (2) the efficiency property — the reason this change exists.
    assert!(
        max_in_flight > 1,
        "chunk reads never overlapped (max in-flight = {max_in_flight} across \
         {chunk_gets} reads): read_to_file is reconstructing one chunk at a time \
         again. Clone pays this stage in full, after the download, on files that \
         file-level checkout parallelism cannot help with."
    );

    // (3) and it must stay BOUNDED IN BYTES. Unbounded read-ahead would hold an
    // unbounded number of decompressed chunks at once, and the bound has to be
    // a byte budget rather than a chunk count because chunk size is tuned to
    // file size (1 MB avg under 100 MB, up to 8 MB avg past 100 GB). A count
    // that costs 512 KiB on a small file costs 128 MB on a large one, and it
    // multiplies with MEDIAGIT_CHECKOUT_PARALLELISM across files — which is how
    // the client peak RSS this project drove from 1,075 MB to 289 MB would
    // quietly regress.
    //
    // 16 MiB budget / 64 KiB chunks = 256, clamped to the ceiling of 32.
    assert!(
        max_in_flight <= 32,
        "read-ahead ran {max_in_flight} concurrent chunk reads, above the hard \
         ceiling of 32"
    );
    let budget = 16 * 1024 * 1024u64;
    let in_flight_bytes = max_in_flight as u64 * 64 * 1024;
    assert!(
        in_flight_bytes <= budget,
        "read-ahead held ~{in_flight_bytes} bytes in flight against a {budget}-byte \
         budget — memory is scaling with chunk count instead of with the budget"
    );
}

/// The budget must actually convert to a count, and must do so from the
/// manifest's real average chunk size — not a fixed guess.
///
/// This is the half that a runtime test cannot show cheaply: reconstructing a
/// file with 8 MB chunks to prove the count drops would mean moving hundreds of
/// MB through a unit test. The arithmetic is the thing that was wrong before
/// (a fixed count of 8, documented as "~512 KiB", actually costing ~32 MB on
/// this corpus), so the arithmetic is what gets pinned.
#[test]
fn read_ahead_count_falls_as_chunks_get_bigger() {
    // Same helper the production path uses, via the crate's public surface.
    let small = mediagit_versioning::checkout_chunk_prefetch_for_test(64 * 1024 * 100, 100); // 64 KiB chunks
    let large = mediagit_versioning::checkout_chunk_prefetch_for_test(8 * 1024 * 1024 * 100, 100); // 8 MiB chunks

    assert!(
        small > large,
        "read-ahead count must shrink as chunks grow ({small} vs {large}); a fixed \
         count is what made memory scale with file size"
    );
    assert!(
        large >= 2,
        "count collapsed to {large} — below 2 there is no pipelining left and this \
         is the serial loop again"
    );
    // 16 MiB budget / 8 MiB chunks = 2, so large files hold ~16 MiB in flight.
    assert!(
        large as u64 * 8 * 1024 * 1024 <= 16 * 1024 * 1024,
        "8 MiB chunks at count {large} exceed the 16 MiB per-file budget"
    );
}
