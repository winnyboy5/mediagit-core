// Copyright (C) 2025 MediaGit Contributors
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Object Database (ODB) performance benchmarks
//!
//! Performance targets:
//! - Object store operations: <50ms for <100MB files
//! - Deduplication check: <10ms
//! - Cache hit: <5ms

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion};
use mediagit_storage::LocalBackend;
use mediagit_versioning::{ObjectDatabase, ObjectType};
use std::sync::Arc;
use tempfile::TempDir;

/// Generate test data of specified size
fn generate_test_data(size: usize) -> Vec<u8> {
    (0..size).map(|i| (i % 256) as u8).collect()
}

/// Setup ODB with temporary storage (async version)
async fn setup_odb_async() -> (ObjectDatabase, TempDir) {
    let temp_dir = TempDir::new().unwrap();
    let storage = Arc::new(LocalBackend::new(temp_dir.path().to_str().unwrap()).await.unwrap());
    let odb = ObjectDatabase::new(storage, 1000);
    (odb, temp_dir)
}

fn bench_odb_write(c: &mut Criterion) {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let mut group = c.benchmark_group("odb_write");

    // Target: <50ms for <100MB files
    for size in [1024, 10 * 1024, 100 * 1024, 1024 * 1024, 10 * 1024 * 1024].iter() {
        let size_name = match size {
            1024 => "1KB",
            10_240 => "10KB",
            102_400 => "100KB",
            1_048_576 => "1MB",
            10_485_760 => "10MB",
            _ => "unknown",
        };

        group.bench_with_input(
            BenchmarkId::new("write", size_name),
            size,
            |b, &size| {
                let data = generate_test_data(size);
                b.to_async(&rt).iter(|| async {
                    let (odb, _temp) = setup_odb_async().await;
                    black_box(odb.write(ObjectType::Blob, &data).await.unwrap())
                });
            },
        );
    }

    group.finish();
}

fn bench_odb_read(c: &mut Criterion) {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let mut group = c.benchmark_group("odb_read");

    for size in [1024, 10 * 1024, 100 * 1024, 1024 * 1024].iter() {
        let size_name = match size {
            1024 => "1KB",
            10_240 => "10KB",
            102_400 => "100KB",
            1_048_576 => "1MB",
            _ => "unknown",
        };

        group.bench_with_input(
            BenchmarkId::new("read", size_name),
            size,
            |b, &size| {
                let (odb, _temp) = rt.block_on(setup_odb_async());
                let data = generate_test_data(size);

                // Pre-write the object
                let oid = rt.block_on(async { odb.write(ObjectType::Blob, &data).await.unwrap() });

                b.to_async(&rt).iter(|| async {
                    black_box(odb.read(&black_box(oid)).await.unwrap())
                });
            },
        );
    }

    group.finish();
}

fn bench_odb_cache_hit(c: &mut Criterion) {
    let rt = tokio::runtime::Runtime::new().unwrap();

    // Target: <5ms for cache hit
    c.bench_function("odb_cache_hit", |b| {
        let (odb, _temp) = rt.block_on(setup_odb_async());
        let data = generate_test_data(1024);

        // Pre-write and read to populate cache
        let oid = rt.block_on(async {
            let oid = odb.write(ObjectType::Blob, &data).await.unwrap();
            // First read to populate cache
            odb.read(&oid).await.unwrap();
            oid
        });

        b.to_async(&rt).iter(|| async {
            // This should hit the cache
            black_box(odb.read(&black_box(oid)).await.unwrap())
        });
    });
}

fn bench_odb_deduplication(c: &mut Criterion) {
    let rt = tokio::runtime::Runtime::new().unwrap();

    // Target: <10ms for deduplication check
    c.bench_function("odb_dedup_check", |b| {
        let (odb, _temp) = rt.block_on(setup_odb_async());
        let data = generate_test_data(10 * 1024); // 10KB

        // Pre-write once
        rt.block_on(async {
            odb.write(ObjectType::Blob, &data).await.unwrap();
        });

        b.to_async(&rt).iter(|| async {
            // This should be deduplicated
            black_box(odb.write(ObjectType::Blob, &data).await.unwrap())
        });
    });
}

fn bench_odb_exists(c: &mut Criterion) {
    let rt = tokio::runtime::Runtime::new().unwrap();

    c.bench_function("odb_exists_check", |b| {
        let (odb, _temp) = rt.block_on(setup_odb_async());
        let data = generate_test_data(1024);

        let oid = rt.block_on(async { odb.write(ObjectType::Blob, &data).await.unwrap() });

        b.to_async(&rt).iter(|| async {
            black_box(odb.exists(&black_box(oid)).await.unwrap())
        });
    });
}

fn bench_odb_batch_write(c: &mut Criterion) {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let mut group = c.benchmark_group("odb_batch_write");

    for count in [10, 100].iter() {
        group.bench_with_input(
            BenchmarkId::new("batch", count),
            count,
            |b, &count| {
                let data = generate_test_data(10 * 1024); // 10KB per object

                b.to_async(&rt).iter(|| async {
                    let (odb, _temp) = setup_odb_async().await;
                    for i in 0u32..count {
                        let mut unique_data = data.clone();
                        unique_data.extend_from_slice(&i.to_le_bytes());
                        black_box(odb.write(ObjectType::Blob, &unique_data).await.unwrap());
                    }
                });
            },
        );
    }

    group.finish();
}

fn bench_odb_concurrent_read(c: &mut Criterion) {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let mut group = c.benchmark_group("odb_concurrent");

    for threads in [2, 4, 8].iter() {
        group.bench_with_input(
            BenchmarkId::new("concurrent_read", threads),
            threads,
            |b, &threads| {
                let (odb, _temp) = rt.block_on(setup_odb_async());
                let odb = Arc::new(odb); // Wrap in Arc for concurrent access
                let data = generate_test_data(10 * 1024);

                // Pre-write objects
                let oids = rt.block_on(async {
                    let mut oids = Vec::new();
                    for i in 0u32..100 {
                        let mut unique_data = data.clone();
                        unique_data.extend_from_slice(&i.to_le_bytes());
                        let oid = odb.write(ObjectType::Blob, &unique_data).await.unwrap();
                        oids.push(oid);
                    }
                    oids
                });

                b.to_async(&rt).iter(|| async {
                    let mut handles = Vec::new();

                    for _ in 0..threads {
                        let odb = odb.clone();
                        let oids = oids.clone();
                        handles.push(tokio::spawn(async move {
                            for oid in oids.iter().take(10) {
                                black_box(odb.read(oid).await.unwrap());
                            }
                        }));
                    }

                    for handle in handles {
                        handle.await.unwrap();
                    }
                });
            },
        );
    }

    group.finish();
}

fn bench_odb_metrics(c: &mut Criterion) {
    let rt = tokio::runtime::Runtime::new().unwrap();

    c.bench_function("odb_metrics_collection", |b| {
        let (odb, _temp) = rt.block_on(setup_odb_async());
        let data = generate_test_data(1024);

        // Pre-populate with some data
        rt.block_on(async {
            for i in 0u32..10 {
                let mut unique_data = data.clone();
                unique_data.extend_from_slice(&i.to_le_bytes());
                odb.write(ObjectType::Blob, &unique_data).await.unwrap();
            }
        });

        b.to_async(&rt)
            .iter(|| async { black_box(odb.metrics().await) });
    });
}

criterion_group!(
    benches,
    bench_odb_write,
    bench_odb_read,
    bench_odb_cache_hit,
    bench_odb_deduplication,
    bench_odb_exists,
    bench_odb_batch_write,
    bench_odb_concurrent_read,
    bench_odb_metrics
);
criterion_main!(benches);
