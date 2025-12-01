// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2025 MediaGit Contributors

//! Compression algorithm performance benchmarks
//!
//! Benchmarks:
//! - Zstd compression/decompression at various levels
//! - Brotli compression/decompression at various levels
//! - Delta encoding for similar files
//! - Compression decision logic
//!
//! Target Performance:
//! - Compression: <100ms for 10MB files

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use mediagit_compression::{
    brotli_compressor::BrotliCompressor, delta::DeltaCompressor, zstd_compressor::ZstdCompressor,
    Compressor,
};

/// Generate test data with specific characteristics
fn generate_test_data(size: usize, pattern: u8) -> Vec<u8> {
    vec![pattern; size]
}

/// Generate slightly modified version for delta encoding
fn generate_similar_data(base: &[u8], change_ratio: f32) -> Vec<u8> {
    let mut modified = base.to_vec();
    let num_changes = (base.len() as f32 * change_ratio) as usize;

    for i in 0..num_changes {
        let pos = (i * base.len() / num_changes) % base.len();
        modified[pos] = modified[pos].wrapping_add(1);
    }

    modified
}

/// Benchmark Zstd compression at various levels
fn bench_zstd_compress(c: &mut Criterion) {
    let mut group = c.benchmark_group("zstd_compress");
    let data = generate_test_data(10_485_760, 42); // 10MB

    group.throughput(Throughput::Bytes(data.len() as u64));

    for level in [1, 3, 9, 19].iter() {
        group.bench_with_input(BenchmarkId::from_parameter(level), level, |b, &level| {
            let compressor = ZstdCompressor::new(level);
            b.iter(|| {
                let compressed = compressor.compress(black_box(&data)).unwrap();
                black_box(compressed);
            });
        });
    }
    group.finish();
}

/// Benchmark Zstd decompression
fn bench_zstd_decompress(c: &mut Criterion) {
    let mut group = c.benchmark_group("zstd_decompress");
    let data = generate_test_data(10_485_760, 42); // 10MB
    let compressor = ZstdCompressor::new(9);
    let compressed = compressor.compress(&data).unwrap();

    group.throughput(Throughput::Bytes(data.len() as u64));
    group.bench_function("level_9", |b| {
        b.iter(|| {
            let decompressed = compressor.decompress(black_box(&compressed)).unwrap();
            black_box(decompressed);
        });
    });
    group.finish();
}

/// Benchmark Brotli compression at various levels
fn bench_brotli_compress(c: &mut Criterion) {
    let mut group = c.benchmark_group("brotli_compress");
    let data = generate_test_data(10_485_760, 42); // 10MB

    group.throughput(Throughput::Bytes(data.len() as u64));

    for level in [1, 6, 11].iter() {
        group.bench_with_input(BenchmarkId::from_parameter(level), level, |b, &level| {
            let compressor = BrotliCompressor::new(*level as u32);
            b.iter(|| {
                let compressed = compressor.compress(black_box(&data)).unwrap();
                black_box(compressed);
            });
        });
    }
    group.finish();
}

/// Benchmark Brotli decompression
fn bench_brotli_decompress(c: &mut Criterion) {
    let mut group = c.benchmark_group("brotli_decompress");
    let data = generate_test_data(10_485_760, 42); // 10MB
    let compressor = BrotliCompressor::new(11);
    let compressed = compressor.compress(&data).unwrap();

    group.throughput(Throughput::Bytes(data.len() as u64));
    group.bench_function("level_11", |b| {
        b.iter(|| {
            let decompressed = compressor.decompress(black_box(&compressed)).unwrap();
            black_box(decompressed);
        });
    });
    group.finish();
}

/// Benchmark delta encoding
fn bench_delta_encode(c: &mut Criterion) {
    let mut group = c.benchmark_group("delta_encode");
    let base_data = generate_test_data(10_485_760, 42); // 10MB

    for change_ratio in [0.01, 0.05, 0.10, 0.20].iter() {
        let modified_data = generate_similar_data(&base_data, *change_ratio);
        group.throughput(Throughput::Bytes(modified_data.len() as u64));

        group.bench_with_input(
            BenchmarkId::from_parameter(format!("{}%", (change_ratio * 100.0) as u32)),
            change_ratio,
            |b, _| {
                let compressor = DeltaCompressor::new();
                b.iter(|| {
                    let delta = compressor
                        .encode_delta(black_box(&base_data), black_box(&modified_data))
                        .unwrap();
                    black_box(delta);
                });
            },
        );
    }
    group.finish();
}

/// Benchmark delta decoding
fn bench_delta_decode(c: &mut Criterion) {
    let mut group = c.benchmark_group("delta_decode");
    let base_data = generate_test_data(10_485_760, 42); // 10MB
    let modified_data = generate_similar_data(&base_data, 0.05); // 5% changes
    let compressor = DeltaCompressor::new();
    let delta = compressor
        .encode_delta(&base_data, &modified_data)
        .unwrap();

    group.throughput(Throughput::Bytes(modified_data.len() as u64));
    group.bench_function("5%_change", |b| {
        b.iter(|| {
            let decoded = compressor
                .decode_delta(black_box(&base_data), black_box(&delta))
                .unwrap();
            black_box(decoded);
        });
    });
    group.finish();
}

/// Benchmark compression ratio comparison
fn bench_compression_ratios(c: &mut Criterion) {
    let mut group = c.benchmark_group("compression_ratios");

    // Text-like data (high compression)
    let text_data = generate_test_data(1_048_576, b'a'); // 1MB repetitive

    // Random-like data (low compression)
    let random_data: Vec<u8> = (0..1_048_576).map(|i| (i % 256) as u8).collect();

    group.bench_function("zstd_text_data", |b| {
        let compressor = ZstdCompressor::new(9);
        b.iter(|| {
            let compressed = compressor.compress(black_box(&text_data)).unwrap();
            black_box(compressed);
        });
    });

    group.bench_function("zstd_random_data", |b| {
        let compressor = ZstdCompressor::new(9);
        b.iter(|| {
            let compressed = compressor.compress(black_box(&random_data)).unwrap();
            black_box(compressed);
        });
    });

    group.finish();
}

criterion_group!(
    benches,
    bench_zstd_compress,
    bench_zstd_decompress,
    bench_brotli_compress,
    bench_brotli_decompress,
    bench_delta_encode,
    bench_delta_decode,
    bench_compression_ratios
);
criterion_main!(benches);
