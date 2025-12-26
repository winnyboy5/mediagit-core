//! Performance benchmarks for shallow clone operations
//!
//! Measures performance across various shallow clone depths and repository sizes
//! to establish baselines and validate production readiness.

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use mediagit_storage::mock::MockBackend;
use mediagit_versioning::{Commit, CommitWalker, ObjectDatabase, Oid, ShallowDatabase, Signature, Tree};
use std::sync::Arc;
use tempfile::TempDir;

/// Create a linear commit chain for benchmarking
async fn create_commit_chain(odb: &ObjectDatabase, depth: usize) -> Vec<Oid> {
    let sig = Signature::now("Benchmark User".to_string(), "bench@test.com".to_string());
    let mut commits = Vec::with_capacity(depth);

    for i in 0..depth {
        let tree = Tree::new();
        let tree_oid = tree.write(odb).await.unwrap();

        let message = format!("Commit {}", i + 1);
        let mut commit = Commit::new(tree_oid, sig.clone(), sig.clone(), message);

        if let Some(parent_oid) = commits.last() {
            commit.add_parent(*parent_oid);
        }

        let commit_oid = commit.write(odb).await.unwrap();
        commits.push(commit_oid);
    }

    commits
}

/// Benchmark shallow clone with various depths on 100-commit repository
fn bench_shallow_clone_depths(c: &mut Criterion) {
    let mut group = c.benchmark_group("shallow_clone_depths");

    let runtime = tokio::runtime::Runtime::new().unwrap();

    // Create 100-commit repository
    let storage = Arc::new(MockBackend::new());
    let odb = Arc::new(ObjectDatabase::new(storage, 1000));
    let commits = runtime.block_on(create_commit_chain(&odb, 100));
    let head = *commits.last().unwrap();

    // Benchmark various depths
    for depth in [0, 1, 2, 5, 10, 20, 50, 99].iter() {
        group.throughput(Throughput::Elements(*depth as u64 + 1));

        group.bench_with_input(
            BenchmarkId::new("depth", depth),
            depth,
            |b, &depth| {
                b.iter(|| {
                    runtime.block_on(async {
                        let mut walker = CommitWalker::new(odb.clone());
                        let result = walker.walk_shallow(&head, depth).await.unwrap();
                        black_box(result)
                    })
                });
            },
        );
    }

    group.finish();
}

/// Benchmark shallow clone on repositories of various sizes
fn bench_shallow_clone_repo_sizes(c: &mut Criterion) {
    let mut group = c.benchmark_group("shallow_clone_repo_sizes");
    group.sample_size(20); // Fewer samples for large repos

    let runtime = tokio::runtime::Runtime::new().unwrap();

    // Test various repository sizes
    for repo_size in [10, 50, 100, 500, 1000].iter() {
        let storage = Arc::new(MockBackend::new());
        let odb = Arc::new(ObjectDatabase::new(storage, 1000));
        let commits = runtime.block_on(create_commit_chain(&odb, *repo_size));
        let head = *commits.last().unwrap();

        group.throughput(Throughput::Elements(*repo_size as u64));

        group.bench_with_input(
            BenchmarkId::new("commits", repo_size),
            repo_size,
            |b, _repo_size| {
                b.iter(|| {
                    runtime.block_on(async {
                        let mut walker = CommitWalker::new(odb.clone());
                        // Shallow clone depth=1 (CI/CD use case)
                        let result = walker.walk_shallow(&head, 0).await.unwrap();
                        black_box(result)
                    })
                });
            },
        );
    }

    group.finish();
}

/// Benchmark shallow boundary storage operations
fn bench_shallow_boundary_storage(c: &mut Criterion) {
    let mut group = c.benchmark_group("shallow_boundary_storage");

    let runtime = tokio::runtime::Runtime::new().unwrap();

    // Create various boundary set sizes
    for boundary_count in [1, 10, 50, 100, 500].iter() {
        let temp = TempDir::new().unwrap();
        let shallow_db = ShallowDatabase::new(temp.path());

        let mut boundaries = std::collections::HashSet::new();
        for i in 0..*boundary_count {
            let oid = Oid::hash(format!("boundary_{}", i).as_bytes());
            boundaries.insert(oid);
        }

        group.throughput(Throughput::Elements(*boundary_count as u64));

        // Write benchmark
        group.bench_with_input(
            BenchmarkId::new("write", boundary_count),
            boundary_count,
            |b, _count| {
                b.iter(|| {
                    shallow_db.write_boundaries(&boundaries).unwrap();
                    black_box(())
                });
            },
        );

        // Read benchmark
        shallow_db.write_boundaries(&boundaries).unwrap();
        group.bench_with_input(
            BenchmarkId::new("read", boundary_count),
            boundary_count,
            |b, _count| {
                b.iter(|| {
                    let loaded = shallow_db.read_boundaries().unwrap();
                    black_box(loaded)
                });
            },
        );
    }

    group.finish();
}

/// Benchmark CommitWalker reset operation
fn bench_walker_reset(c: &mut Criterion) {
    let mut group = c.benchmark_group("walker_reset");

    let runtime = tokio::runtime::Runtime::new().unwrap();

    let storage = Arc::new(MockBackend::new());
    let odb = Arc::new(ObjectDatabase::new(storage, 1000));
    let commits = runtime.block_on(create_commit_chain(&odb, 100));
    let head = *commits.last().unwrap();

    group.bench_function("reset_after_walk", |b| {
        b.iter(|| {
            runtime.block_on(async {
                let mut walker = CommitWalker::new(odb.clone());
                walker.walk_shallow(&head, 10).await.unwrap();
                walker.reset();
                black_box(())
            })
        });
    });

    group.finish();
}

/// Benchmark memory usage patterns
fn bench_memory_patterns(c: &mut Criterion) {
    let mut group = c.benchmark_group("memory_patterns");
    group.sample_size(20);

    let runtime = tokio::runtime::Runtime::new().unwrap();

    // Test memory efficiency with large repositories
    for repo_size in [100, 500, 1000].iter() {
        let storage = Arc::new(MockBackend::new());
        let odb = Arc::new(ObjectDatabase::new(storage, 1000));
        let commits = runtime.block_on(create_commit_chain(&odb, *repo_size));
        let head = *commits.last().unwrap();

        group.bench_with_input(
            BenchmarkId::new("full_walk", repo_size),
            repo_size,
            |b, _size| {
                b.iter(|| {
                    runtime.block_on(async {
                        let mut walker = CommitWalker::new(odb.clone());
                        // Full walk (no depth limit - walk all commits)
                        let result = walker.walk_shallow(&head, *repo_size).await.unwrap();
                        black_box(result)
                    })
                });
            },
        );
    }

    group.finish();
}

/// Benchmark CI/CD use case specifically
fn bench_ci_cd_workflow(c: &mut Criterion) {
    let mut group = c.benchmark_group("ci_cd_workflow");
    group.sample_size(50);

    let runtime = tokio::runtime::Runtime::new().unwrap();

    // Simulate realistic CI/CD scenario
    let storage = Arc::new(MockBackend::new());
    let odb = Arc::new(ObjectDatabase::new(storage, 1000));
    let commits = runtime.block_on(create_commit_chain(&odb, 1000));
    let head = *commits.last().unwrap();

    let temp = TempDir::new().unwrap();
    let shallow_db = ShallowDatabase::new(temp.path());

    group.bench_function("complete_ci_cd_shallow_clone", |b| {
        b.iter(|| {
            runtime.block_on(async {
                // Shallow clone depth=1
                let mut walker = CommitWalker::new(odb.clone());
                let result = walker.walk_shallow(&head, 0).await.unwrap();

                // Store boundaries
                shallow_db.write_boundaries(&result.shallow_boundaries).unwrap();

                // Verify shallow
                assert!(shallow_db.is_shallow());

                black_box(result)
            })
        });
    });

    group.finish();
}

criterion_group!(
    benches,
    bench_shallow_clone_depths,
    bench_shallow_clone_repo_sizes,
    bench_shallow_boundary_storage,
    bench_walker_reset,
    bench_memory_patterns,
    bench_ci_cd_workflow
);

criterion_main!(benches);
