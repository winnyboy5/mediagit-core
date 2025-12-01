// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2025 MediaGit Contributors

//! Storage backend performance benchmarks
//!
//! Benchmarks:
//! - Local filesystem operations
//! - Mock backend operations
//! - Concurrent storage operations
//!
//! Target Performance:
//! - Local operations: <50ms for <100MB
//! - Concurrent operations: handle 100+ concurrent requests

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use mediagit_storage::{local::LocalBackend, mock::MockBackend, StorageBackend};
use std::sync::Arc;
use tempfile::TempDir;
use tokio::runtime::Runtime;

/// Benchmark local filesystem put operations
fn bench_local_put(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();
    let mut group = c.benchmark_group("local_put");

    for size in [1024, 1_048_576, 10_485_760, 104_857_600].iter() {
        group.throughput(Throughput::Bytes(*size as u64));
        group.bench_with_input(BenchmarkId::from_parameter(size), size, |b, &size| {
            let temp_dir = TempDir::new().unwrap();
            let backend = LocalBackend::new(temp_dir.path().to_path_buf());
            let data = vec![0u8; size];

            b.to_async(&rt).iter(|| async {
                let key = format!("test_object_{}", size);
                backend.put(black_box(&key), black_box(&data)).await.unwrap();
            });
        });
    }
    group.finish();
}

/// Benchmark local filesystem get operations
fn bench_local_get(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();
    let mut group = c.benchmark_group("local_get");

    for size in [1024, 1_048_576, 10_485_760, 104_857_600].iter() {
        group.throughput(Throughput::Bytes(*size as u64));
        group.bench_with_input(BenchmarkId::from_parameter(size), size, |b, &size| {
            let temp_dir = TempDir::new().unwrap();
            let backend = LocalBackend::new(temp_dir.path().to_path_buf());
            let data = vec![0u8; size];
            let key = format!("test_object_{}", size);

            // Pre-populate
            rt.block_on(backend.put(&key, &data)).unwrap();

            b.to_async(&rt).iter(|| async {
                let retrieved = backend.get(black_box(&key)).await.unwrap();
                black_box(retrieved);
            });
        });
    }
    group.finish();
}

/// Benchmark local filesystem exists check
fn bench_local_exists(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();
    let temp_dir = TempDir::new().unwrap();
    let backend = LocalBackend::new(temp_dir.path().to_path_buf());
    let data = vec![42u8; 1024];
    let key = "test_object";

    // Pre-populate
    rt.block_on(backend.put(key, &data)).unwrap();

    c.bench_function("local_exists", |b| {
        b.to_async(&rt).iter(|| async {
            let exists = backend.exists(black_box(key)).await.unwrap();
            black_box(exists);
        });
    });
}

/// Benchmark local filesystem delete operations
fn bench_local_delete(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();

    c.bench_function("local_delete", |b| {
        b.to_async(&rt).iter(|| async {
            let temp_dir = TempDir::new().unwrap();
            let backend = LocalBackend::new(temp_dir.path().to_path_buf());
            let data = vec![42u8; 1024];
            let key = "test_object";

            // Create then delete
            backend.put(key, &data).await.unwrap();
            backend.delete(black_box(key)).await.unwrap();
        });
    });
}

/// Benchmark mock backend operations
fn bench_mock_operations(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();
    let backend = MockBackend::new();
    let data = vec![42u8; 1_048_576]; // 1MB
    let key = "test_object";

    c.bench_function("mock_put", |b| {
        b.to_async(&rt).iter(|| async {
            backend.put(black_box(key), black_box(&data)).await.unwrap();
        });
    });

    // Pre-populate for get benchmark
    rt.block_on(backend.put(key, &data)).unwrap();

    c.bench_function("mock_get", |b| {
        b.to_async(&rt).iter(|| async {
            let retrieved = backend.get(black_box(key)).await.unwrap();
            black_box(retrieved);
        });
    });
}

/// Benchmark concurrent put operations
fn bench_concurrent_puts(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();
    let mut group = c.benchmark_group("concurrent_puts");

    for concurrency in [10, 50, 100].iter() {
        group.bench_with_input(
            BenchmarkId::from_parameter(concurrency),
            concurrency,
            |b, &concurrency| {
                let temp_dir = TempDir::new().unwrap();
                let backend = Arc::new(LocalBackend::new(temp_dir.path().to_path_buf()));
                let data = vec![42u8; 1024]; // 1KB per operation

                b.to_async(&rt).iter(|| async {
                    let handles: Vec<_> = (0..concurrency)
                        .map(|i| {
                            let backend = Arc::clone(&backend);
                            let data = data.clone();
                            let key = format!("object_{}", i);
                            tokio::spawn(async move {
                                backend.put(&key, &data).await.unwrap();
                            })
                        })
                        .collect();

                    for handle in handles {
                        handle.await.unwrap();
                    }
                });
            },
        );
    }
    group.finish();
}

/// Benchmark concurrent get operations
fn bench_concurrent_gets(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();
    let mut group = c.benchmark_group("concurrent_gets");

    for concurrency in [10, 50, 100].iter() {
        group.bench_with_input(
            BenchmarkId::from_parameter(concurrency),
            concurrency,
            |b, &concurrency| {
                let temp_dir = TempDir::new().unwrap();
                let backend = Arc::new(LocalBackend::new(temp_dir.path().to_path_buf()));
                let data = vec![42u8; 1024]; // 1KB

                // Pre-populate objects
                for i in 0..concurrency {
                    let key = format!("object_{}", i);
                    rt.block_on(backend.put(&key, &data)).unwrap();
                }

                b.to_async(&rt).iter(|| async {
                    let handles: Vec<_> = (0..concurrency)
                        .map(|i| {
                            let backend = Arc::clone(&backend);
                            let key = format!("object_{}", i);
                            tokio::spawn(async move {
                                backend.get(&key).await.unwrap();
                            })
                        })
                        .collect();

                    for handle in handles {
                        handle.await.unwrap();
                    }
                });
            },
        );
    }
    group.finish();
}

/// Benchmark list operations with varying object counts
fn bench_list_objects(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();
    let mut group = c.benchmark_group("list_objects");

    for count in [10, 100, 1000].iter() {
        group.bench_with_input(BenchmarkId::from_parameter(count), count, |b, &count| {
            let temp_dir = TempDir::new().unwrap();
            let backend = LocalBackend::new(temp_dir.path().to_path_buf());
            let data = vec![42u8; 100]; // Small objects

            // Pre-populate objects
            for i in 0..count {
                let key = format!("object_{:05}", i);
                rt.block_on(backend.put(&key, &data)).unwrap();
            }

            b.to_async(&rt).iter(|| async {
                let objects = backend
                    .list_objects(black_box("object_"))
                    .await
                    .unwrap();
                black_box(objects);
            });
        });
    }
    group.finish();
}

criterion_group!(
    benches,
    bench_local_put,
    bench_local_get,
    bench_local_exists,
    bench_local_delete,
    bench_mock_operations,
    bench_concurrent_puts,
    bench_concurrent_gets,
    bench_list_objects
);
criterion_main!(benches);
