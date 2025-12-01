// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2025 MediaGit Contributors

//! Object Database (ODB) performance benchmarks
//!
//! Benchmarks critical ODB operations:
//! - Object store/retrieve for various sizes
//! - Deduplication check performance
//! - Cache hit performance
//!
//! Target Performance:
//! - Store/retrieve: <50ms for <100MB
//! - Dedup check: <10ms
//! - Cache hit: <5ms

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use mediagit_storage::{mock::MockBackend, StorageBackend};
use mediagit_versioning::odb::ObjectDatabase;
use std::sync::Arc;
use tempfile::TempDir;
use tokio::runtime::Runtime;

/// Benchmark object store operations for various sizes
fn bench_object_store(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();
    let mut group = c.benchmark_group("odb_store");

    // Test sizes: 1KB, 1MB, 10MB, 100MB
    for size in [1024, 1_048_576, 10_485_760, 104_857_600].iter() {
        group.throughput(Throughput::Bytes(*size as u64));
        group.bench_with_input(BenchmarkId::from_parameter(size), size, |b, &size| {
            let temp_dir = TempDir::new().unwrap();
            let backend = Arc::new(MockBackend::new());
            let odb = ObjectDatabase::new(backend, temp_dir.path().to_path_buf());

            let data = vec![0u8; size];

            b.to_async(&rt).iter(|| async {
                let oid = odb.store(black_box(&data)).await.unwrap();
                black_box(oid);
            });
        });
    }
    group.finish();
}

/// Benchmark object retrieve operations
fn bench_object_retrieve(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();
    let mut group = c.benchmark_group("odb_retrieve");

    for size in [1024, 1_048_576, 10_485_760, 104_857_600].iter() {
        group.throughput(Throughput::Bytes(*size as u64));
        group.bench_with_input(BenchmarkId::from_parameter(size), size, |b, &size| {
            let temp_dir = TempDir::new().unwrap();
            let backend = Arc::new(MockBackend::new());
            let odb = ObjectDatabase::new(backend, temp_dir.path().to_path_buf());

            let data = vec![0u8; size];
            let oid = rt.block_on(odb.store(&data)).unwrap();

            b.to_async(&rt).iter(|| async {
                let retrieved = odb.get(black_box(&oid)).await.unwrap();
                black_box(retrieved);
            });
        });
    }
    group.finish();
}

/// Benchmark deduplication check performance
fn bench_dedup_check(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();
    let temp_dir = TempDir::new().unwrap();
    let backend = Arc::new(MockBackend::new());
    let odb = ObjectDatabase::new(backend, temp_dir.path().to_path_buf());

    // Store initial object
    let data = vec![42u8; 1_048_576]; // 1MB
    let oid = rt.block_on(odb.store(&data)).unwrap();

    c.bench_function("odb_dedup_check", |b| {
        b.to_async(&rt).iter(|| async {
            let exists = odb.exists(black_box(&oid)).await.unwrap();
            black_box(exists);
        });
    });
}

/// Benchmark cache hit performance
fn bench_cache_hit(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();
    let temp_dir = TempDir::new().unwrap();
    let backend = Arc::new(MockBackend::new());
    let odb = ObjectDatabase::new(backend, temp_dir.path().to_path_buf());

    // Store and retrieve to populate cache
    let data = vec![42u8; 1_048_576]; // 1MB
    let oid = rt.block_on(odb.store(&data)).unwrap();
    let _ = rt.block_on(odb.get(&oid)).unwrap(); // Populate cache

    c.bench_function("odb_cache_hit", |b| {
        b.to_async(&rt).iter(|| async {
            let retrieved = odb.get(black_box(&oid)).await.unwrap();
            black_box(retrieved);
        });
    });
}

/// Benchmark concurrent reads
fn bench_concurrent_reads(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();
    let temp_dir = TempDir::new().unwrap();
    let backend = Arc::new(MockBackend::new());
    let odb = Arc::new(ObjectDatabase::new(backend, temp_dir.path().to_path_buf()));

    // Store test object
    let data = vec![42u8; 1_048_576]; // 1MB
    let oid = rt.block_on(odb.store(&data)).unwrap();

    c.bench_function("odb_concurrent_reads_10", |b| {
        b.to_async(&rt).iter(|| async {
            let odb = Arc::clone(&odb);
            let oid_clone = oid.clone();

            let handles: Vec<_> = (0..10)
                .map(|_| {
                    let odb = Arc::clone(&odb);
                    let oid = oid_clone.clone();
                    tokio::spawn(async move { odb.get(&oid).await.unwrap() })
                })
                .collect();

            for handle in handles {
                let _ = handle.await.unwrap();
            }
        });
    });
}

criterion_group!(
    benches,
    bench_object_store,
    bench_object_retrieve,
    bench_dedup_check,
    bench_cache_hit,
    bench_concurrent_reads
);
criterion_main!(benches);
