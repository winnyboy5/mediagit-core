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

//! Compression benchmarks comparing Zstd vs Brotli

use criterion::{black_box, criterion_group, criterion_main, Criterion};
use mediagit_compression::{BrotliCompressor, Compressor, CompressionLevel, ZstdCompressor};

/// Generate test data with specified pattern
fn generate_test_data(size: usize, pattern: &str) -> Vec<u8> {
    match pattern {
        "text" => {
            // Repetitive text data (highly compressible)
            let text = "The quick brown fox jumps over the lazy dog. ".as_bytes();
            (0..size)
                .map(|i| text[i % text.len()])
                .collect()
        }
        "json" => {
            // JSON-like structured data
            let json = r#"{"id":123,"name":"MediaGit","size":1024,"hash":"abc123def456"}"#.as_bytes();
            (0..size)
                .map(|i| json[i % json.len()])
                .collect()
        }
        "random" => {
            // Random data (incompressible)
            (0..size).map(|i| ((i ^ 0xAA) & 0xFF) as u8).collect()
        }
        "media_metadata" => {
            // Simulated media metadata (mixed compressibility)
            let metadata = concat!(
                "DURATION=3600\nFORMAT=mp4\nBITRATE=5000\n",
                "CODEC=h264\nRESOLUTION=1920x1080\n",
                "METADATA={'key':'value'}\n"
            )
            .as_bytes();
            (0..size)
                .map(|i| metadata[i % metadata.len()])
                .collect()
        }
        _ => vec![0u8; size],
    }
}

fn benchmark_compression(c: &mut Criterion) {
    let mut group = c.benchmark_group("compression");

    // Test 1KB data
    let data_1kb = black_box(generate_test_data(1024, "text"));

    group.bench_function("zstd_compress_1kb_text_default", |b| {
        let compressor = ZstdCompressor::new(CompressionLevel::Default);
        b.iter(|| compressor.compress(&data_1kb))
    });

    group.bench_function("brotli_compress_1kb_text_default", |b| {
        let compressor = BrotliCompressor::new(CompressionLevel::Default);
        b.iter(|| compressor.compress(&data_1kb))
    });

    // Test 10KB data
    let data_10kb = black_box(generate_test_data(10 * 1024, "text"));

    group.bench_function("zstd_compress_10kb_text_default", |b| {
        let compressor = ZstdCompressor::new(CompressionLevel::Default);
        b.iter(|| compressor.compress(&data_10kb))
    });

    group.bench_function("brotli_compress_10kb_text_default", |b| {
        let compressor = BrotliCompressor::new(CompressionLevel::Default);
        b.iter(|| compressor.compress(&data_10kb))
    });

    // Test 100KB data
    let data_100kb = black_box(generate_test_data(100 * 1024, "text"));

    group.bench_function("zstd_compress_100kb_text_default", |b| {
        let compressor = ZstdCompressor::new(CompressionLevel::Default);
        b.iter(|| compressor.compress(&data_100kb))
    });

    group.bench_function("brotli_compress_100kb_text_default", |b| {
        let compressor = BrotliCompressor::new(CompressionLevel::Default);
        b.iter(|| compressor.compress(&data_100kb))
    });

    group.finish();
}

fn benchmark_decompression(c: &mut Criterion) {
    let mut group = c.benchmark_group("decompression");

    // Prepare compressed data
    let data_100kb = black_box(generate_test_data(100 * 1024, "text"));

    let zstd_compressor = ZstdCompressor::new(CompressionLevel::Default);
    let compressed_zstd = zstd_compressor.compress(&data_100kb).unwrap();

    let brotli_compressor = BrotliCompressor::new(CompressionLevel::Default);
    let compressed_brotli = brotli_compressor.compress(&data_100kb).unwrap();

    group.bench_function("zstd_decompress_100kb", |b| {
        let compressor = ZstdCompressor::new(CompressionLevel::Default);
        b.iter(|| compressor.decompress(black_box(&compressed_zstd)))
    });

    group.bench_function("brotli_decompress_100kb", |b| {
        let compressor = BrotliCompressor::new(CompressionLevel::Default);
        b.iter(|| compressor.decompress(black_box(&compressed_brotli)))
    });

    group.finish();
}

fn benchmark_levels(c: &mut Criterion) {
    let mut group = c.benchmark_group("compression_levels");

    let data = black_box(generate_test_data(10 * 1024, "text"));

    group.bench_function("zstd_fast", |b| {
        let compressor = ZstdCompressor::new(CompressionLevel::Fast);
        b.iter(|| compressor.compress(&data))
    });

    group.bench_function("zstd_default", |b| {
        let compressor = ZstdCompressor::new(CompressionLevel::Default);
        b.iter(|| compressor.compress(&data))
    });

    group.bench_function("zstd_best", |b| {
        let compressor = ZstdCompressor::new(CompressionLevel::Best);
        b.iter(|| compressor.compress(&data))
    });

    group.bench_function("brotli_fast", |b| {
        let compressor = BrotliCompressor::new(CompressionLevel::Fast);
        b.iter(|| compressor.compress(&data))
    });

    group.bench_function("brotli_default", |b| {
        let compressor = BrotliCompressor::new(CompressionLevel::Default);
        b.iter(|| compressor.compress(&data))
    });

    group.bench_function("brotli_best", |b| {
        let compressor = BrotliCompressor::new(CompressionLevel::Best);
        b.iter(|| compressor.compress(&data))
    });

    group.finish();
}

fn benchmark_data_types(c: &mut Criterion) {
    let mut group = c.benchmark_group("data_types");

    let sizes = vec![
        ("1kb", 1024),
        ("100kb", 100 * 1024),
    ];

    for (name, size) in sizes {
        let text_data = black_box(generate_test_data(size, "text"));
        let json_data = black_box(generate_test_data(size, "json"));
        let random_data = black_box(generate_test_data(size, "random"));

        let zstd = ZstdCompressor::new(CompressionLevel::Default);

        group.bench_function(format!("zstd_text_{}", name), |b| {
            b.iter(|| zstd.compress(&text_data))
        });

        group.bench_function(format!("zstd_json_{}", name), |b| {
            b.iter(|| zstd.compress(&json_data))
        });

        group.bench_function(format!("zstd_random_{}", name), |b| {
            b.iter(|| zstd.compress(&random_data))
        });
    }

    group.finish();
}

criterion_group!(
    benches,
    benchmark_compression,
    benchmark_decompression,
    benchmark_levels,
    benchmark_data_types
);
criterion_main!(benches);
