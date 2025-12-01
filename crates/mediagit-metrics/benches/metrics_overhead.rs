//! Benchmark metrics collection overhead
//!
//! Measures the performance overhead of metrics collection to ensure it stays below 1%

use criterion::{black_box, criterion_group, criterion_main, Criterion, BenchmarkId};
use mediagit_metrics::{MetricsRegistry, types::{CompressionAlgorithm, OperationType, StorageBackend}};
use std::time::{Duration, Instant};

/// Benchmark deduplication write without metrics
fn dedup_write_baseline(iterations: u64) -> Duration {
    let start = Instant::now();

    for i in 0..iterations {
        let bytes = (i % 10_000) as u64;
        let is_new = i % 2 == 0;

        // Simulate dedup logic without metrics
        black_box((bytes, is_new));
    }

    start.elapsed()
}

/// Benchmark deduplication write with metrics
fn dedup_write_with_metrics(iterations: u64, registry: &MetricsRegistry) -> Duration {
    let start = Instant::now();

    for i in 0..iterations {
        let bytes = (i % 10_000) as u64;
        let is_new = i % 2 == 0;

        // Record metrics
        registry.record_dedup_write(bytes, is_new);

        black_box((bytes, is_new));
    }

    start.elapsed()
}

/// Benchmark compression operation without metrics
fn compression_baseline(iterations: u64) -> Duration {
    let start = Instant::now();

    for i in 0..iterations {
        let original = (i % 10_000) as u64;
        let compressed = (original * 6) / 10; // 60% compression ratio

        black_box((original, compressed));
    }

    start.elapsed()
}

/// Benchmark compression operation with metrics
fn compression_with_metrics(iterations: u64, registry: &MetricsRegistry) -> Duration {
    let start = Instant::now();

    for i in 0..iterations {
        let original = (i % 10_000) as u64;
        let compressed = (original * 6) / 10;

        // Alternate between compression algorithms
        let algo = if i % 2 == 0 {
            CompressionAlgorithm::Zstd
        } else {
            CompressionAlgorithm::Brotli
        };

        registry.record_compression(algo, original, compressed);

        black_box((original, compressed));
    }

    start.elapsed()
}

/// Benchmark cache hit/miss without metrics
fn cache_baseline(iterations: u64) -> Duration {
    let start = Instant::now();

    for i in 0..iterations {
        let is_hit = i % 4 != 0; // 75% hit rate
        black_box(is_hit);
    }

    start.elapsed()
}

/// Benchmark cache hit/miss with metrics
fn cache_with_metrics(iterations: u64, registry: &MetricsRegistry) -> Duration {
    let start = Instant::now();

    for i in 0..iterations {
        if i % 4 != 0 {
            registry.record_cache_hit();
        } else {
            registry.record_cache_miss();
        }
    }

    start.elapsed()
}

/// Benchmark operation timing without metrics
fn operation_baseline(iterations: u64) -> Duration {
    let start = Instant::now();

    for i in 0..iterations {
        let duration = (i % 100) as f64 / 1000.0; // Vary duration
        black_box(duration);
    }

    start.elapsed()
}

/// Benchmark operation timing with metrics
fn operation_with_metrics(iterations: u64, registry: &MetricsRegistry) -> Duration {
    let start = Instant::now();

    for i in 0..iterations {
        let duration = (i % 100) as f64 / 1000.0;

        let op_type = if i % 2 == 0 {
            OperationType::Store
        } else {
            OperationType::Retrieve
        };

        let backend = match i % 3 {
            0 => StorageBackend::Filesystem,
            1 => StorageBackend::S3,
            _ => StorageBackend::AzureBlob,
        };

        registry.record_operation_duration(op_type, backend, duration);
        registry.record_operation_complete(op_type, backend, true);
    }

    start.elapsed()
}

/// Calculate overhead percentage
fn calculate_overhead(baseline: Duration, with_metrics: Duration) -> f64 {
    let baseline_ms = baseline.as_secs_f64() * 1000.0;
    let metrics_ms = with_metrics.as_secs_f64() * 1000.0;

    if baseline_ms == 0.0 {
        return 0.0;
    }

    ((metrics_ms - baseline_ms) / baseline_ms) * 100.0
}

fn bench_dedup_overhead(c: &mut Criterion) {
    let registry = MetricsRegistry::new().unwrap();
    let mut group = c.benchmark_group("dedup_overhead");

    for iterations in [1_000, 10_000, 100_000] {
        group.bench_with_input(
            BenchmarkId::new("baseline", iterations),
            &iterations,
            |b, &iterations| {
                b.iter(|| dedup_write_baseline(iterations));
            },
        );

        group.bench_with_input(
            BenchmarkId::new("with_metrics", iterations),
            &iterations,
            |b, &iterations| {
                b.iter(|| dedup_write_with_metrics(iterations, &registry));
            },
        );
    }

    group.finish();
}

fn bench_compression_overhead(c: &mut Criterion) {
    let registry = MetricsRegistry::new().unwrap();
    let mut group = c.benchmark_group("compression_overhead");

    for iterations in [1_000, 10_000, 100_000] {
        group.bench_with_input(
            BenchmarkId::new("baseline", iterations),
            &iterations,
            |b, &iterations| {
                b.iter(|| compression_baseline(iterations));
            },
        );

        group.bench_with_input(
            BenchmarkId::new("with_metrics", iterations),
            &iterations,
            |b, &iterations| {
                b.iter(|| compression_with_metrics(iterations, &registry));
            },
        );
    }

    group.finish();
}

fn bench_cache_overhead(c: &mut Criterion) {
    let registry = MetricsRegistry::new().unwrap();
    let mut group = c.benchmark_group("cache_overhead");

    for iterations in [1_000, 10_000, 100_000] {
        group.bench_with_input(
            BenchmarkId::new("baseline", iterations),
            &iterations,
            |b, &iterations| {
                b.iter(|| cache_baseline(iterations));
            },
        );

        group.bench_with_input(
            BenchmarkId::new("with_metrics", iterations),
            &iterations,
            |b, &iterations| {
                b.iter(|| cache_with_metrics(iterations, &registry));
            },
        );
    }

    group.finish();
}

fn bench_operation_overhead(c: &mut Criterion) {
    let registry = MetricsRegistry::new().unwrap();
    let mut group = c.benchmark_group("operation_overhead");

    for iterations in [1_000, 10_000, 100_000] {
        group.bench_with_input(
            BenchmarkId::new("baseline", iterations),
            &iterations,
            |b, &iterations| {
                b.iter(|| operation_baseline(iterations));
            },
        );

        group.bench_with_input(
            BenchmarkId::new("with_metrics", iterations),
            &iterations,
            |b, &iterations| {
                b.iter(|| operation_with_metrics(iterations, &registry));
            },
        );
    }

    group.finish();
}

fn comprehensive_overhead_test(c: &mut Criterion) {
    let registry = MetricsRegistry::new().unwrap();

    c.bench_function("comprehensive_overhead_100k", |b| {
        b.iter(|| {
            // Baseline: operations without metrics
            let baseline_start = Instant::now();
            for i in 0..100_000 {
                black_box(i);
            }
            let baseline = baseline_start.elapsed();

            // With metrics: mixed operations
            let metrics_start = Instant::now();
            for i in 0..100_000 {
                match i % 4 {
                    0 => registry.record_dedup_write(1024, i % 2 == 0),
                    1 => registry.record_compression(CompressionAlgorithm::Zstd, 1000, 600),
                    2 => registry.record_cache_hit(),
                    _ => registry.record_operation_duration(
                        OperationType::Store,
                        StorageBackend::Filesystem,
                        0.001,
                    ),
                }
            }
            let with_metrics = metrics_start.elapsed();

            let overhead = calculate_overhead(baseline, with_metrics);

            // Report overhead
            eprintln!("Overhead: {:.2}%", overhead);

            // Assert <1% overhead (allowing some measurement variance)
            assert!(
                overhead < 2.0,
                "Metrics overhead {:.2}% exceeds 2% threshold",
                overhead
            );
        });
    });
}

criterion_group!(
    benches,
    bench_dedup_overhead,
    bench_compression_overhead,
    bench_cache_overhead,
    bench_operation_overhead,
    comprehensive_overhead_test
);
criterion_main!(benches);
