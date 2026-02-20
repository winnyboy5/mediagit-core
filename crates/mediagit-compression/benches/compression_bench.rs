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

#![allow(clippy::unwrap_used)]
//! Compression benchmarks comparing Zstd vs Brotli vs Adaptive

use criterion::{black_box, criterion_group, criterion_main, Criterion};
use mediagit_compression::{
    adaptive::AdaptiveCompressor, BrotliCompressor, Compressor, CompressionLevel, ZstdCompressor,
    PerObjectTypeCompressor, CompressionProfile, ObjectType, TypeAwareCompressor,
};

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

fn benchmark_adaptive(c: &mut Criterion) {
    let mut group = c.benchmark_group("adaptive_vs_static");

    // Adaptive compressor
    let adaptive = AdaptiveCompressor::new();

    // Static compressors at default level
    let zstd = ZstdCompressor::new(CompressionLevel::Default);
    let brotli = BrotliCompressor::new(CompressionLevel::Default);

    // Test 1: Tiny text (optimal: Brotli Best)
    let tiny_text = black_box(generate_test_data(512, "text"));
    group.bench_function("adaptive_tiny_text", |b| {
        b.iter(|| adaptive.compress(&tiny_text))
    });
    group.bench_function("static_zstd_tiny_text", |b| {
        b.iter(|| zstd.compress(&tiny_text))
    });
    group.bench_function("static_brotli_tiny_text", |b| {
        b.iter(|| brotli.compress(&tiny_text))
    });

    // Test 2: Small JSON (optimal: Brotli Best)
    let small_json = black_box(generate_test_data(50 * 1024, "json"));
    group.bench_function("adaptive_small_json", |b| {
        b.iter(|| adaptive.compress(&small_json))
    });
    group.bench_function("static_zstd_small_json", |b| {
        b.iter(|| zstd.compress(&small_json))
    });
    group.bench_function("static_brotli_small_json", |b| {
        b.iter(|| brotli.compress(&small_json))
    });

    // Test 3: Large text (optimal: Zstd Fast)
    let large_text = black_box(generate_test_data(50 * 1024 * 1024, "text"));
    group.bench_function("adaptive_large_text", |b| {
        b.iter(|| adaptive.compress(&large_text))
    });
    group.bench_function("static_zstd_large_text", |b| {
        b.iter(|| zstd.compress(&large_text))
    });

    // Test 4: Random data (optimal: Store)
    let random = black_box(generate_test_data(10 * 1024, "random"));
    group.bench_function("adaptive_random", |b| {
        b.iter(|| adaptive.compress(&random))
    });
    group.bench_function("static_zstd_random", |b| {
        b.iter(|| zstd.compress(&random))
    });

    // Test 5: Mixed workload (show overall benefit)
    group.bench_function("adaptive_mixed_workload", |b| {
        let tiny = generate_test_data(512, "text");
        let small = generate_test_data(50 * 1024, "json");
        let medium = generate_test_data(1024 * 1024, "text");
        let random = generate_test_data(10 * 1024, "random");

        b.iter(|| {
            adaptive.compress(&tiny).unwrap();
            adaptive.compress(&small).unwrap();
            adaptive.compress(&medium).unwrap();
            adaptive.compress(&random).unwrap();
        })
    });

    group.bench_function("static_zstd_mixed_workload", |b| {
        let tiny = generate_test_data(512, "text");
        let small = generate_test_data(50 * 1024, "json");
        let medium = generate_test_data(1024 * 1024, "text");
        let random = generate_test_data(10 * 1024, "random");

        b.iter(|| {
            zstd.compress(&tiny).unwrap();
            zstd.compress(&small).unwrap();
            zstd.compress(&medium).unwrap();
            zstd.compress(&random).unwrap();
        })
    });

    group.finish();
}

fn benchmark_per_type_compressor(c: &mut Criterion) {
    let mut group = c.benchmark_group("per_type_compressor");

    // Compressors with different profiles
    let balanced = PerObjectTypeCompressor::new();
    let speed = PerObjectTypeCompressor::with_profile(CompressionProfile::Speed);
    let max_compression = PerObjectTypeCompressor::with_profile(CompressionProfile::MaxCompression);

    // Test data for different object types
    let text_data = black_box(generate_test_data(10 * 1024, "text"));
    let json_data = black_box(generate_test_data(10 * 1024, "json"));
    let random_data = black_box(generate_test_data(10 * 1024, "random"));

    // Balanced profile
    group.bench_function("balanced_text", |b| {
        b.iter(|| balanced.compress_typed(&text_data, ObjectType::Text))
    });

    group.bench_function("balanced_json", |b| {
        b.iter(|| balanced.compress_typed(&json_data, ObjectType::Json))
    });

    // Speed profile
    group.bench_function("speed_text", |b| {
        b.iter(|| speed.compress_typed(&text_data, ObjectType::Text))
    });

    group.bench_function("speed_json", |b| {
        b.iter(|| speed.compress_typed(&json_data, ObjectType::Json))
    });

    // Max compression profile
    group.bench_function("max_compression_text", |b| {
        b.iter(|| max_compression.compress_typed(&text_data, ObjectType::Text))
    });

    group.bench_function("max_compression_json", |b| {
        b.iter(|| max_compression.compress_typed(&json_data, ObjectType::Json))
    });

    // Already compressed types (should store)
    group.bench_function("balanced_jpeg_store", |b| {
        b.iter(|| balanced.compress_typed(&random_data, ObjectType::Jpeg))
    });

    // Git objects (should use Zlib)
    group.bench_function("balanced_git_blob", |b| {
        b.iter(|| balanced.compress_typed(&text_data, ObjectType::GitBlob))
    });

    group.finish();
}

criterion_group!(
    benches,
    benchmark_compression,
    benchmark_decompression,
    benchmark_levels,
    benchmark_data_types,
    benchmark_adaptive,
    benchmark_per_type_compressor
);
criterion_main!(benches);
