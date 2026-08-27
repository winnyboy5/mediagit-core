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

use std::sync::Arc;
/// Lightweight throughput instrumentation for push/pull operations.
/// Gated by `MEDIAGIT_BENCH=1`; zero overhead when disabled.
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

/// Schema version for the `[bench]` summary line. Increment when fields change.
///
/// 4: `op=add` gained the per-phase breakdown
/// (`oid_ms`/`cdc_ms`/`hash_ms`/`send_block_ms`/`compress_ms`/`write_ms`/`autogc_ms`).
/// 5: new `op=checkout` line. Working-tree write time was previously folded into
/// the caller's wall time with no record of its own, so the cost of the
/// download->checkout barrier could not be measured at all.
const BENCH_SCHEMA_VERSION: u8 = 5;

/// Collect all `MEDIAGIT_*` env vars as a sorted `KEY=val,...` string.
fn collect_knobs() -> String {
    let mut pairs: Vec<(String, String)> = std::env::vars()
        .filter(|(k, _)| k.starts_with("MEDIAGIT_"))
        .collect();
    pairs.sort_by(|a, b| a.0.cmp(&b.0));
    pairs
        .iter()
        .map(|(k, v)| format!("{}={}", k, v))
        .collect::<Vec<_>>()
        .join(",")
}

pub fn enabled() -> bool {
    std::env::var("MEDIAGIT_BENCH").as_deref() == Ok("1")
}

/// Accumulates byte/chunk counts across all stream passes for one push/pull.
pub struct BenchSession {
    op: &'static str,
    chunks: AtomicU64,
    bytes: AtomicU64,
    /// Sum of per-batch elapsed wall-ns (≈ combined active network time).
    phase_ns: AtomicU64,
    /// Time from first manifest fetch start to first byte of chunk data received (B9).
    manifest_to_first_byte_ns: AtomicU64,
    wall_start: Instant,
    concurrency: usize,
    // F11: Track-F pack-mode metrics
    presign_urls: AtomicU64,
    pack_count: AtomicU64,
    pack_bytes: AtomicU64,
    range_gets: AtomicU64,
    coalesced_gets: AtomicU64,
}

impl BenchSession {
    fn new(op: &'static str, concurrency: usize) -> Arc<Self> {
        Arc::new(Self {
            op,
            chunks: AtomicU64::new(0),
            bytes: AtomicU64::new(0),
            phase_ns: AtomicU64::new(0),
            manifest_to_first_byte_ns: AtomicU64::new(0),
            wall_start: Instant::now(),
            concurrency,
            presign_urls: AtomicU64::new(0),
            pack_count: AtomicU64::new(0),
            pack_bytes: AtomicU64::new(0),
            range_gets: AtomicU64::new(0),
            coalesced_gets: AtomicU64::new(0),
        })
    }

    /// Record presigned URL requests issued (one per pack upload or download batch).
    pub fn record_presign_urls(&self, n: u64) {
        self.presign_urls.fetch_add(n, Ordering::Relaxed);
    }

    /// Record one completed pack upload or download.
    pub fn record_pack(&self, byte_len: u64) {
        self.pack_count.fetch_add(1, Ordering::Relaxed);
        self.pack_bytes.fetch_add(byte_len, Ordering::Relaxed);
    }

    /// Record one Range-GET request. `coalesced=true` when the request covers
    /// more than one logical chunk (i.e. ranges were merged).
    pub fn record_range_get(&self, coalesced: bool) {
        self.range_gets.fetch_add(1, Ordering::Relaxed);
        if coalesced {
            self.coalesced_gets.fetch_add(1, Ordering::Relaxed);
        }
    }

    /// Record time from first manifest fetch start to first chunk byte received.
    /// Call once per pull/clone operation when the first chunk body byte arrives.
    pub fn record_manifest_to_first_byte(&self, elapsed: Duration) {
        self.manifest_to_first_byte_ns
            .store(elapsed.as_nanos() as u64, Ordering::Relaxed);
    }

    /// Record one completed stream batch: N chunks, B bytes, D elapsed.
    pub fn record_batch(&self, chunks: u64, bytes: u64, elapsed: Duration) {
        self.chunks.fetch_add(chunks, Ordering::Relaxed);
        self.bytes.fetch_add(bytes, Ordering::Relaxed);
        self.phase_ns
            .fetch_add(elapsed.as_nanos() as u64, Ordering::Relaxed);
    }

    /// Emit `[bench]` summary line to stderr.  Call once at end of operation.
    ///
    /// `util_pct` interpretation:
    /// - ≥ 80 % → saturated (network or CPU is the cap)
    /// - 30-80 % → partially utilised (latency or serialisation overhead)
    /// - < 30 % → mostly idle (single-object, or short repo — overhead dominated)
    pub fn summary(&self) {
        let wall_s = self.wall_start.elapsed().as_secs_f64();
        let chunks = self.chunks.load(Ordering::Relaxed);
        let bytes = self.bytes.load(Ordering::Relaxed);
        let phase_ns = self.phase_ns.load(Ordering::Relaxed);
        let active_s = phase_ns as f64 / 1e9;
        let bps_wall = if wall_s > 0.0 {
            bytes as f64 / wall_s
        } else {
            0.0
        };
        let avg_mb = if chunks > 0 {
            bytes as f64 / chunks as f64 / (1024.0 * 1024.0)
        } else {
            0.0
        };
        // Fraction of concurrency × wall time that was active.
        // > 1.0 impossible in theory; values near 1.0 = well-saturated.
        let util_pct = if wall_s > 0.0 && self.concurrency > 0 {
            (active_s / (wall_s * self.concurrency as f64)).min(1.0) * 100.0
        } else {
            0.0
        };
        let manifest_to_first_byte_ms = {
            let ns = self.manifest_to_first_byte_ns.load(Ordering::Relaxed);
            if ns > 0 {
                format!("{:.1}", ns as f64 / 1e6)
            } else {
                "n/a".to_string()
            }
        };
        let presign_urls = self.presign_urls.load(Ordering::Relaxed);
        let pack_count = self.pack_count.load(Ordering::Relaxed);
        let pack_bytes = self.pack_bytes.load(Ordering::Relaxed);
        let range_gets = self.range_gets.load(Ordering::Relaxed);
        let coalesced_gets = self.coalesced_gets.load(Ordering::Relaxed);
        let pack_bytes_avg_mb = if pack_count > 0 {
            pack_bytes as f64 / pack_count as f64 / (1024.0 * 1024.0)
        } else {
            0.0
        };
        let range_coalesce_ratio = if range_gets > 0 {
            coalesced_gets as f64 / range_gets as f64
        } else {
            0.0
        };
        let knobs = collect_knobs();
        eprintln!(
            "[bench] bench_schema_version={schema} op={op} chunks={chunks} total_bytes={bytes} \
             wall={wall:.2}s active_sum={active:.2}s \
             avg_chunk_mb={avg:.2} throughput_mbs={mbs:.2} \
             util_pct={util:.0}% manifest_to_first_byte_ms={m2fb} \
             presign_urls={presign_urls} pack_count={pack_count} \
             pack_bytes_avg_mb={pack_bytes_avg:.2} range_gets={range_gets} \
             range_coalesce_ratio={coalesce_ratio:.2} \
             knobs={knobs}",
            schema = BENCH_SCHEMA_VERSION,
            op = self.op,
            chunks = chunks,
            bytes = bytes,
            wall = wall_s,
            active = active_s,
            avg = avg_mb,
            mbs = bps_wall / (1024.0 * 1024.0),
            util = util_pct,
            m2fb = manifest_to_first_byte_ms,
            presign_urls = presign_urls,
            pack_count = pack_count,
            pack_bytes_avg = pack_bytes_avg_mb,
            range_gets = range_gets,
            coalesce_ratio = range_coalesce_ratio,
            knobs = if knobs.is_empty() {
                "none".to_string()
            } else {
                knobs
            },
        );
    }
}

/// Returns a new `BenchSession` when `MEDIAGIT_BENCH=1`, else `None`.
pub fn maybe_start(op: &'static str, concurrency: usize) -> Option<Arc<BenchSession>> {
    if enabled() {
        Some(BenchSession::new(op, concurrency))
    } else {
        None
    }
}

/// Emit a `[bench] op=commit` summary line.
///
/// Call once at the end of a successful `commit` with the wall-clock start time
/// and the number of tree entries written. No-op unless `MEDIAGIT_BENCH=1`.
///
/// Deliberately does NOT report a throughput figure. `commit` writes tree and
/// commit objects, not the blob bytes `add` already stored, so a bytes/second
/// number here would describe nothing real — and this file's own history is that
/// gating a derived metric alongside its source double-counts every regression
/// (`hash_mbs` was dropped from `08_perf`'s gate set for exactly that).
///
/// Added 2026-07-30: `08_perf` had measured and written commit timings since it
/// was created, but `commit` emitted no `[bench]` line, so the harness had no
/// record to compare and silently skipped every one of them. Half that phase's
/// workload was gated against nothing.
pub fn emit_commit_summary(wall_start: std::time::Instant, files: u64) {
    if !enabled() {
        return;
    }
    let wall_s = wall_start.elapsed().as_secs_f64();
    let knobs = collect_knobs();
    eprintln!(
        "[bench] bench_schema_version={schema} op=commit files={files} \
         wall={wall:.2}s knobs={knobs}",
        schema = BENCH_SCHEMA_VERSION,
        files = files,
        wall = wall_s,
        knobs = if knobs.is_empty() {
            "none".to_string()
        } else {
            knobs
        },
    );
}

/// Emit a `[bench] op=checkout` summary line.
///
/// Call once at the end of a checkout with the wall-clock start time and the
/// number of files written. No-op unless `MEDIAGIT_BENCH=1`.
///
/// Exists because clone's phase-4 (write the working tree) had no record of its
/// own: it ran inside the CLI, after the protocol crate's `BenchSession` for the
/// download had already ended, so its cost was invisible. Clone re-reads and
/// re-decompresses every chunk here, and nothing measured it.
///
/// A separate line rather than a `BenchSession` field because the session is
/// created and dropped inside `client/pull.rs`, which the CLI has no handle on —
/// a field would have had no reachable caller.
///
/// Deliberately reports no throughput figure, for the same reason `op=commit`
/// does not: these bytes were already counted by the clone that fetched them, so
/// a second MB/s here would double-count.
/// `overlap` records whether the C4 download/checkout overlap was on. It is
/// reported explicitly rather than left to `knobs=`, because the knob defaults
/// ON and so is ABSENT from the environment in the interesting case — an A/B
/// that has to read "on" from a missing variable is one transcription slip away
/// from being backwards.
pub fn emit_checkout_summary(wall_start: std::time::Instant, files: u64, overlap: bool) {
    if !enabled() {
        return;
    }
    let wall_s = wall_start.elapsed().as_secs_f64();
    let knobs = collect_knobs();
    eprintln!(
        "[bench] bench_schema_version={schema} op=checkout files={files} \
         overlap={overlap} wall={wall:.2}s knobs={knobs}",
        schema = BENCH_SCHEMA_VERSION,
        files = files,
        overlap = if overlap { "on" } else { "off" },
        wall = wall_s,
        knobs = if knobs.is_empty() {
            "none".to_string()
        } else {
            knobs
        },
    );
}

/// Emit a `[bench] op=add` summary line (A9).
///
/// Call once at the end of the `add` command with the wall-clock start time,
/// the total bytes hashed (sum of all staged file sizes), and the file count.
/// No-op when `MEDIAGIT_BENCH` is not set to `"1"`.
pub fn emit_add_summary(wall_start: std::time::Instant, total_bytes: u64, files: u64) {
    if !enabled() {
        return;
    }
    let wall_s = wall_start.elapsed().as_secs_f64();
    let mbs = if wall_s > 0.0 {
        total_bytes as f64 / wall_s / (1024.0 * 1024.0)
    } else {
        0.0
    };
    let knobs = collect_knobs();
    // PERF-V10-PSD: the per-phase breakdown. `wall` alone cannot say whether
    // `add` is bound by the whole-file OID, by serial CDC, by hashing, by the
    // workers, or by auto-gc — which `wall` also silently included. Every phase
    // is printed even at zero, so "this phase costs nothing" stays
    // distinguishable from "this phase was never instrumented".
    //
    // These do NOT sum to `wall`: the producer and the worker pool overlap by
    // design, so compress_ms + write_ms are wall-clock across N workers while
    // cdc_ms/hash_ms are one serial thread. Read them as attribution, not as a
    // partition — a breakdown that appeared to add up would be the misleading
    // version.
    let phases = mediagit_versioning::add_phases::snapshot()
        .into_iter()
        .map(|(k, ms)| format!("{k}={ms:.1}"))
        .collect::<Vec<_>>()
        .join(" ");
    eprintln!(
        "[bench] bench_schema_version={schema} op=add files={files} \
         total_bytes={bytes} wall={wall:.2}s hash_mbs={mbs:.2} {phases} knobs={knobs}",
        schema = BENCH_SCHEMA_VERSION,
        files = files,
        bytes = total_bytes,
        wall = wall_s,
        mbs = mbs,
        phases = phases,
        knobs = if knobs.is_empty() {
            "none".to_string()
        } else {
            knobs
        },
    );
}
