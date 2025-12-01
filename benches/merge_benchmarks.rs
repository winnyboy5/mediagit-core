// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2025 MediaGit Contributors

//! Merge and versioning performance benchmarks
//!
//! Benchmarks:
//! - LCA (Lowest Common Ancestor) detection
//! - 3-way merge algorithm
//! - Conflict detection
//! - Branch switching
//!
//! Target Performance:
//! - LCA detection: <50ms
//! - Branch switch: <100ms

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion};
use mediagit_versioning::{
    branch::BranchManager,
    commit::{Commit, CommitMetadata},
    lca::find_lowest_common_ancestor,
    merge::{MergeStrategy, Merger},
    oid::ObjectId,
};
use std::collections::HashMap;
use tempfile::TempDir;
use tokio::runtime::Runtime;

/// Create a linear commit history
fn create_linear_history(count: usize) -> HashMap<ObjectId, Commit> {
    let mut commits = HashMap::new();
    let mut prev_oid = None;

    for i in 0..count {
        let oid = ObjectId::from_hex(&format!("{:064x}", i)).unwrap();
        let metadata = CommitMetadata {
            author: "Test Author".to_string(),
            email: "test@example.com".to_string(),
            message: format!("Commit {}", i),
            timestamp: 1_000_000 + i as i64,
        };

        let commit = Commit {
            oid: oid.clone(),
            tree_oid: ObjectId::from_hex(&format!("{:064x}", i + 10000)).unwrap(),
            parents: prev_oid.iter().cloned().collect(),
            metadata,
        };

        commits.insert(oid.clone(), commit);
        prev_oid = Some(oid);
    }

    commits
}

/// Create a branching history with merge
fn create_branching_history(depth: usize) -> HashMap<ObjectId, Commit> {
    let mut commits = HashMap::new();

    // Create main branch
    let mut main_oid = ObjectId::from_hex(&format!("{:064x}", 0)).unwrap();
    for i in 0..depth {
        let oid = ObjectId::from_hex(&format!("{:064x}", i)).unwrap();
        let metadata = CommitMetadata {
            author: "Test Author".to_string(),
            email: "test@example.com".to_string(),
            message: format!("Main commit {}", i),
            timestamp: 1_000_000 + i as i64,
        };

        let commit = Commit {
            oid: oid.clone(),
            tree_oid: ObjectId::from_hex(&format!("{:064x}", i + 10000)).unwrap(),
            parents: if i == 0 { vec![] } else { vec![main_oid.clone()] },
            metadata,
        };

        commits.insert(oid.clone(), commit);
        main_oid = oid;
    }

    // Create feature branch at midpoint
    let branch_point = depth / 2;
    let branch_start = ObjectId::from_hex(&format!("{:064x}", branch_point)).unwrap();
    let mut feature_oid = branch_start.clone();

    for i in 0..depth / 2 {
        let oid = ObjectId::from_hex(&format!("{:064x}", i + 50000)).unwrap();
        let metadata = CommitMetadata {
            author: "Test Author".to_string(),
            email: "test@example.com".to_string(),
            message: format!("Feature commit {}", i),
            timestamp: 1_000_000 + branch_point as i64 + i as i64,
        };

        let commit = Commit {
            oid: oid.clone(),
            tree_oid: ObjectId::from_hex(&format!("{:064x}", i + 60000)).unwrap(),
            parents: vec![feature_oid],
            metadata,
        };

        commits.insert(oid.clone(), commit);
        feature_oid = oid;
    }

    commits
}

/// Benchmark LCA detection with linear history
fn bench_lca_linear(c: &mut Criterion) {
    let mut group = c.benchmark_group("lca_linear_history");

    for size in [10, 50, 100, 500].iter() {
        group.bench_with_input(BenchmarkId::from_parameter(size), size, |b, &size| {
            let commits = create_linear_history(size);
            let oid1 = ObjectId::from_hex(&format!("{:064x}", size / 2)).unwrap();
            let oid2 = ObjectId::from_hex(&format!("{:064x}", size - 1)).unwrap();

            b.iter(|| {
                let lca = find_lowest_common_ancestor(
                    black_box(&oid1),
                    black_box(&oid2),
                    black_box(&commits),
                )
                .unwrap();
                black_box(lca);
            });
        });
    }
    group.finish();
}

/// Benchmark LCA detection with branching history
fn bench_lca_branching(c: &mut Criterion) {
    let mut group = c.benchmark_group("lca_branching_history");

    for size in [10, 50, 100, 500].iter() {
        group.bench_with_input(BenchmarkId::from_parameter(size), size, |b, &size| {
            let commits = create_branching_history(size);
            let main_tip = ObjectId::from_hex(&format!("{:064x}", size - 1)).unwrap();
            let feature_tip = ObjectId::from_hex(&format!("{:064x}", size / 2 - 1 + 50000)).unwrap();

            b.iter(|| {
                let lca = find_lowest_common_ancestor(
                    black_box(&main_tip),
                    black_box(&feature_tip),
                    black_box(&commits),
                )
                .unwrap();
                black_box(lca);
            });
        });
    }
    group.finish();
}

/// Benchmark 3-way merge with no conflicts
fn bench_merge_no_conflicts(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();
    let temp_dir = TempDir::new().unwrap();

    c.bench_function("merge_no_conflicts", |b| {
        b.to_async(&rt).iter(|| async {
            let merger = Merger::new(temp_dir.path().to_path_buf());

            // Create test data for merge
            let base_content = b"line 1\nline 2\nline 3\n";
            let ours_content = b"line 0\nline 1\nline 2\nline 3\n";
            let theirs_content = b"line 1\nline 2\nline 3\nline 4\n";

            let result = merger
                .merge_contents(
                    black_box(base_content),
                    black_box(ours_content),
                    black_box(theirs_content),
                    black_box(&MergeStrategy::Recursive),
                )
                .await
                .unwrap();

            black_box(result);
        });
    });
}

/// Benchmark 3-way merge with conflicts
fn bench_merge_with_conflicts(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();
    let temp_dir = TempDir::new().unwrap();

    c.bench_function("merge_with_conflicts", |b| {
        b.to_async(&rt).iter(|| async {
            let merger = Merger::new(temp_dir.path().to_path_buf());

            // Create conflicting changes
            let base_content = b"line 1\nline 2\nline 3\n";
            let ours_content = b"line 1\nline 2 modified by us\nline 3\n";
            let theirs_content = b"line 1\nline 2 modified by them\nline 3\n";

            let result = merger
                .merge_contents(
                    black_box(base_content),
                    black_box(ours_content),
                    black_box(theirs_content),
                    black_box(&MergeStrategy::Recursive),
                )
                .await;

            black_box(result);
        });
    });
}

/// Benchmark branch switching
fn bench_branch_switch(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();
    let temp_dir = TempDir::new().unwrap();

    c.bench_function("branch_switch", |b| {
        b.to_async(&rt).iter(|| async {
            let branch_manager = BranchManager::new(temp_dir.path().to_path_buf());

            // Create and switch between branches
            let branch_name = "feature-branch";
            let commit_oid = ObjectId::from_hex(&format!("{:064x}", 12345)).unwrap();

            branch_manager
                .create_branch(black_box(branch_name), black_box(&commit_oid))
                .await
                .unwrap();

            branch_manager
                .switch_branch(black_box(branch_name))
                .await
                .unwrap();
        });
    });
}

/// Benchmark conflict detection
fn bench_conflict_detection(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();
    let temp_dir = TempDir::new().unwrap();
    let merger = Merger::new(temp_dir.path().to_path_buf());

    c.bench_function("conflict_detection", |b| {
        let base_content = b"line 1\nline 2\nline 3\nline 4\nline 5\n";
        let ours_content = b"line 1\nline 2 modified\nline 3\nline 4\nline 5 modified\n";
        let theirs_content = b"line 1\nline 2 changed\nline 3\nline 4\nline 5 changed\n";

        b.to_async(&rt).iter(|| async {
            let conflicts = merger
                .detect_conflicts(
                    black_box(base_content),
                    black_box(ours_content),
                    black_box(theirs_content),
                )
                .await
                .unwrap();

            black_box(conflicts);
        });
    });
}

criterion_group!(
    benches,
    bench_lca_linear,
    bench_lca_branching,
    bench_merge_no_conflicts,
    bench_merge_with_conflicts,
    bench_branch_switch,
    bench_conflict_detection
);
criterion_main!(benches);
