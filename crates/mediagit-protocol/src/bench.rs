/// Lightweight throughput instrumentation for push/pull operations.
/// Gated by `MEDIAGIT_BENCH=1`; zero overhead when disabled.
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

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
    wall_start: Instant,
    concurrency: usize,
}

impl BenchSession {
    fn new(op: &'static str, concurrency: usize) -> Arc<Self> {
        Arc::new(Self {
            op,
            chunks: AtomicU64::new(0),
            bytes: AtomicU64::new(0),
            phase_ns: AtomicU64::new(0),
            wall_start: Instant::now(),
            concurrency,
        })
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
        eprintln!(
            "[bench] op={op} chunks={chunks} total_bytes={bytes} \
             wall={wall:.2}s active_sum={active:.2}s \
             avg_chunk_mb={avg:.2} throughput_mbs={mbs:.2} \
             util_pct={util:.0}%",
            op = self.op,
            chunks = chunks,
            bytes = bytes,
            wall = wall_s,
            active = active_s,
            avg = avg_mb,
            mbs = bps_wall / (1024.0 * 1024.0),
            util = util_pct,
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
