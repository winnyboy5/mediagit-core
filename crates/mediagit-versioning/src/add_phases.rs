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

//! Where `add` actually spends its time.
//!
//! `[bench] op=add` reported `wall`, `hash_mbs`, `files` and `total_bytes` and
//! nothing else, so every claim about `add` being slow — PERF-V10-PSD has been
//! open since v10 — rested on a single number that could not distinguish
//! "hashing is slow" from "the writes are slow" from "auto-gc ran". `wall` even
//! included auto-gc silently.
//!
//! Two specific suspicions this exists to settle, both of which the whole-wall
//! number cannot:
//!
//! - **The file is walked twice.** `Oid::from_file_mmap_parallel` mmaps the
//!   whole file for the dedup OID, then `StreamCDC` opens and reads it again.
//!   If [`Phase::Oid`] is a material fraction, that second walk is removable.
//! - **The producer is serial.** FastCDC boundary detection and per-chunk
//!   BLAKE3 run on one blocking thread feeding N workers. If [`Phase::Cdc`] and
//!   [`Phase::Hash`] dominate, worker concurrency cannot help — but if
//!   [`Phase::SendBlock`] dominates instead, the producer is *waiting on the
//!   workers* and the premise is backwards. That distinction is the point.
//!
//! Off unless `MEDIAGIT_BENCH=1`: the flag is read once into a `OnceLock`, and
//! when it is off every `record` is a load and a branch, with no clock read.
//! Timing is opt-in because [`Phase::Compress`] and [`Phase::Write`] are
//! per-chunk and an unconditional `Instant::now()` pair there would be
//! instrumentation the measurement has to subtract back out.
//!
//! Counters are process-global and additive. `add` is one process doing one
//! job, so there is nothing to attribute them to; [`reset`] exists for tests.

use std::sync::OnceLock;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

/// A stage of `add`, in roughly the order bytes flow through it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    /// Whole-file dedup OID (`Oid::from_file_mmap_parallel`) — the first walk.
    Oid,
    /// FastCDC boundary detection, which also does the second read of the file.
    Cdc,
    /// Per-chunk BLAKE3, on the same serial thread as `Cdc`.
    Hash,
    /// Producer blocked on the bounded channel — i.e. time spent waiting for
    /// the workers. High here means the consumers are the bottleneck.
    SendBlock,
    /// Per-chunk dedup existence lookups (two `storage.exists` per chunk).
    Dedup,
    /// Chunk-delta encoding: base resolution, chain walk, delta write.
    Delta,
    /// Waiting to acquire the global `delta_written_pairs` mutex. Measured
    /// apart from the work because that lock is held across storage I/O (a
    /// chain re-walk and the `.meta` put), so N workers serialise on it — and
    /// a single `delta_ms` cannot tell "computing" from "waiting in line".
    DeltaLock,
    /// `resolve_delta_base` chain walks. Twice per chunk — once before the
    /// lock, once under it — each doing storage lookups per hop.
    DeltaResolve,
    /// `DeltaEncoder::encode` itself: the actual delta computation.
    DeltaEncode,
    /// Compressing the encoded delta before storing it.
    DeltaCompress,
    /// The `.meta` sidecar put plus the delta binary put. Named so that
    /// "the writes are slow" can be told apart from "the task was waiting for
    /// a runtime thread": these counters are per-task elapsed wall time, so
    /// scheduling delay shows up as the gap between `delta_ms` and the sum of
    /// its sub-phases, not inside any of them.
    DeltaWrite,
    /// Fetching the base chunk and decompressing it, before `DeltaEncode` can
    /// run. This was the one region inside [`Phase::Delta`] with no counter,
    /// and once the `delta_written_pairs` lock was removed it became the
    /// dominant cost: 964 s of a 1,390 s `delta_ms` on a 357 MB PSD, ~69%,
    /// visible only as the gap between `delta_ms` and its sub-phases. A
    /// measured gap is not a diagnosis — name it so the next claim about it
    /// starts from a number.
    DeltaBaseFetch,
    /// Per-chunk compression in the worker pool.
    Compress,
    /// Per-chunk `storage.put`.
    Write,
    /// Auto-gc, which `wall` used to hide.
    AutoGc,
}

impl Phase {
    const COUNT: usize = 15;

    fn index(self) -> usize {
        match self {
            Phase::Oid => 0,
            Phase::Cdc => 1,
            Phase::Hash => 2,
            Phase::SendBlock => 3,
            Phase::Dedup => 4,
            Phase::Delta => 5,
            Phase::DeltaLock => 6,
            Phase::DeltaResolve => 7,
            Phase::DeltaEncode => 8,
            Phase::DeltaCompress => 9,
            Phase::DeltaBaseFetch => 13,
            Phase::DeltaWrite => 14,
            Phase::Compress => 10,
            Phase::Write => 11,
            Phase::AutoGc => 12,
        }
    }

    /// Field name used in the `[bench]` line.
    fn label(self) -> &'static str {
        match self {
            Phase::Oid => "oid_ms",
            Phase::Cdc => "cdc_ms",
            Phase::Hash => "hash_ms",
            Phase::SendBlock => "send_block_ms",
            Phase::Dedup => "dedup_ms",
            Phase::Delta => "delta_ms",
            Phase::DeltaLock => "delta_lock_ms",
            Phase::DeltaResolve => "delta_resolve_ms",
            Phase::DeltaEncode => "delta_encode_ms",
            Phase::DeltaCompress => "delta_compress_ms",
            Phase::DeltaBaseFetch => "delta_basefetch_ms",
            Phase::DeltaWrite => "delta_write_ms",
            Phase::Compress => "compress_ms",
            Phase::Write => "write_ms",
            Phase::AutoGc => "autogc_ms",
        }
    }

    const ALL: [Phase; Self::COUNT] = [
        Phase::Oid,
        Phase::Cdc,
        Phase::Hash,
        Phase::SendBlock,
        Phase::Dedup,
        Phase::Delta,
        Phase::DeltaLock,
        Phase::DeltaResolve,
        Phase::DeltaEncode,
        Phase::DeltaCompress,
        Phase::DeltaBaseFetch,
        Phase::DeltaWrite,
        Phase::Compress,
        Phase::Write,
        Phase::AutoGc,
    ];
}

#[allow(clippy::declare_interior_mutable_const)]
const ZERO: AtomicU64 = AtomicU64::new(0);
static NANOS: [AtomicU64; Phase::COUNT] = [ZERO; Phase::COUNT];

/// Whether phase timing is on. Same knob as the rest of `[bench]`, so a bench
/// run reports the breakdown without a second flag to remember.
pub fn enabled() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| std::env::var("MEDIAGIT_BENCH").as_deref() == Ok("1"))
}

/// Add `d` to `phase`. Cheap no-op when timing is off.
pub fn record(phase: Phase, d: Duration) {
    if !enabled() {
        return;
    }
    NANOS[phase.index()].fetch_add(d.as_nanos() as u64, Ordering::Relaxed);
}

/// Time `f` into `phase`.
///
/// Reads the clock only when timing is on, so the disabled path does not pay
/// for two `Instant::now()` calls per chunk.
pub fn time<T>(phase: Phase, f: impl FnOnce() -> T) -> T {
    if !enabled() {
        return f();
    }
    let t = Instant::now();
    let out = f();
    record(phase, t.elapsed());
    out
}

/// `phase_label=milliseconds` pairs for every phase, in flow order.
///
/// Always returns all phases, including zeroed ones. A phase that is missing
/// from the output because it happened to be zero is indistinguishable from a
/// phase that was never instrumented — and telling those apart is the entire
/// value of the breakdown.
pub fn snapshot() -> Vec<(&'static str, f64)> {
    Phase::ALL
        .iter()
        .map(|p| {
            let ns = NANOS[p.index()].load(Ordering::Relaxed);
            (p.label(), ns as f64 / 1_000_000.0)
        })
        .collect()
}

/// Zero every counter. For tests; `add` is one process doing one job.
pub fn reset() {
    for c in &NANOS {
        c.store(0, Ordering::Relaxed);
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn snapshot_reports_every_phase_even_at_zero() {
        let s = snapshot();
        assert_eq!(
            s.len(),
            Phase::COUNT,
            "a phase omitted because it was zero cannot be told apart from a \
             phase nobody instrumented"
        );
        // Flow order, so the line reads like the pipeline.
        assert_eq!(s[0].0, "oid_ms");
        assert_eq!(s.last().unwrap().0, "autogc_ms");
    }

    #[test]
    fn recording_is_inert_unless_the_bench_knob_is_set() {
        // The suite does not set MEDIAGIT_BENCH, so this is the default path:
        // instrumentation that costs nothing and, crucially, reports nothing
        // rather than reporting a misleading partial number.
        assert!(!enabled(), "MEDIAGIT_BENCH must not be set in the test env");
        reset();
        record(Phase::Cdc, Duration::from_secs(5));
        let cdc = snapshot()
            .into_iter()
            .find(|(k, _)| *k == "cdc_ms")
            .unwrap()
            .1;
        assert_eq!(cdc, 0.0, "timing must be off unless MEDIAGIT_BENCH=1");
    }

    #[test]
    fn time_returns_the_closures_value_either_way() {
        // The wrapper is on the hot path; it must be transparent.
        assert_eq!(time(Phase::Hash, || 42), 42);
    }
}
