// Copyright (C) 2026  winnyboy5
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU Affero General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU Affero General Public License for more details.
//
// You should have received a copy of the GNU Affero General Public License
// along with this program.  If not, see <https://www.gnu.org/licenses/>.
// Copyright (C) 2025 MediaGit Contributors
// SPDX-License-Identifier: AGPL-3.0-or-later

//! LRU cache benchmarks

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion};
use mediagit_storage::cache::LruCache;
use std::sync::Arc;

fn bench_cache_get(c: &mut Criterion) {
    let rt = tokio::runtime::Runtime::new().unwrap();

    let mut group = c.benchmark_group("cache_get");

    for size in [100, 1_000, 10_000].iter() {
        group.bench_with_input(BenchmarkId::from_parameter(size), size, |b, &size| {
            let cache = Arc::new(LruCache::new(size * 1024)); // 1KB per entry

            // Pre-populate cache
            rt.block_on(async {
                for i in 0..size {
                    cache.put(format!("key_{}", i), vec![0u8; 1024]).await;
                }
            });

            b.to_async(&rt).iter(|| async {
                let key = format!("key_{}", black_box(42));
                black_box(cache.get(&key).await)
            });
        });
    }

    group.finish();
}

fn bench_cache_put(c: &mut Criterion) {
    let rt = tokio::runtime::Runtime::new().unwrap();

    let mut group = c.benchmark_group("cache_put");

    for size in [100, 1_000, 10_000].iter() {
        group.bench_with_input(BenchmarkId::from_parameter(size), size, |b, &size| {
            let cache = Arc::new(LruCache::new(size * 1024));
            let counter = Arc::new(std::sync::atomic::AtomicU64::new(0));

            b.to_async(&rt).iter(|| {
                let counter = counter.clone();
                let cache = cache.clone();
                async move {
                    let idx = counter.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    let key = format!("key_{}", black_box(idx));
                    cache.put(black_box(key), black_box(vec![0u8; 1024])).await;
                }
            });
        });
    }

    group.finish();
}

fn bench_cache_eviction(c: &mut Criterion) {
    let rt = tokio::runtime::Runtime::new().unwrap();

    c.bench_function("cache_eviction_100_entries", |b| {
        let cache = Arc::new(LruCache::new(100 * 1024)); // 100KB cache
        let counter = Arc::new(std::sync::atomic::AtomicU64::new(0));

        b.to_async(&rt).iter(|| {
            let counter = counter.clone();
            let cache = cache.clone();
            async move {
                // This will trigger evictions after 100 entries
                let idx = counter.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                let key = format!("key_{}", black_box(idx));
                cache.put(black_box(key), black_box(vec![0u8; 1024])).await;
            }
        });
    });
}

fn bench_concurrent_access(c: &mut Criterion) {
    let rt = tokio::runtime::Runtime::new().unwrap();

    let mut group = c.benchmark_group("concurrent_access");

    for threads in [2, 4, 8].iter() {
        group.bench_with_input(
            BenchmarkId::from_parameter(threads),
            threads,
            |b, &threads| {
                let cache = Arc::new(LruCache::new(10_000 * 1024));

                // Pre-populate
                rt.block_on(async {
                    for i in 0..1000 {
                        cache.put(format!("key_{}", i), vec![0u8; 1024]).await;
                    }
                });

                b.to_async(&rt).iter(|| async {
                    let mut handles = vec![];

                    for _ in 0..threads {
                        let cache = cache.clone();
                        handles.push(tokio::spawn(async move {
                            for i in 0..100 {
                                let key = format!("key_{}", black_box(i));
                                black_box(cache.get(&key).await);
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

fn bench_cache_stats(c: &mut Criterion) {
    let rt = tokio::runtime::Runtime::new().unwrap();

    c.bench_function("cache_stats", |b| {
        let cache = Arc::new(LruCache::new(1024 * 1024));

        rt.block_on(async {
            for i in 0..1000 {
                cache.put(format!("key_{}", i), vec![0u8; 100]).await;
            }
        });

        b.to_async(&rt)
            .iter(|| async { black_box(cache.stats().await) });
    });
}

criterion_group!(
    benches,
    bench_cache_get,
    bench_cache_put,
    bench_cache_eviction,
    bench_concurrent_access,
    bench_cache_stats
);
criterion_main!(benches);
