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

//! Branch operations performance benchmarks
//!
//! Performance targets:
//! - Branch switching: <100ms

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion};
use mediagit_storage::LocalBackend;
use mediagit_versioning::{BranchManager, Commit, ObjectDatabase, ObjectType, Oid, Signature, Tree};
use std::collections::BTreeMap;
use std::sync::Arc;
use tempfile::TempDir;

/// Setup branch manager with temporary storage (async version)
async fn setup_branch_manager_async() -> (BranchManager, ObjectDatabase, TempDir) {
    let temp_dir = TempDir::new().unwrap();
    let storage = Arc::new(LocalBackend::new(temp_dir.path().to_str().unwrap()).await.unwrap());
    let branch_mgr = BranchManager::new(temp_dir.path());
    let odb = ObjectDatabase::new(storage, 1000);
    (branch_mgr, odb, temp_dir)
}

/// Create a test commit
async fn create_test_commit(
    odb: &ObjectDatabase,
    tree_oid: Oid,
    parent: Option<Oid>,
    message: &str,
) -> Oid {
    let signature = Signature {
        name: "Test User".to_string(),
        email: "test@example.com".to_string(),
        timestamp: chrono::Utc::now(),
    };

    let commit = Commit {
        tree: tree_oid,
        parents: parent.map(|p| vec![p]).unwrap_or_default(),
        author: signature.clone(),
        committer: signature,
        message: message.to_string(),
    };

    let commit_data = bincode::serialize(&commit).unwrap();
    odb.write(ObjectType::Commit, &commit_data).await.unwrap()
}

/// Create a test tree
async fn create_test_tree(odb: &ObjectDatabase) -> Oid {
    let tree = Tree { entries: BTreeMap::new() };
    let tree_data = bincode::serialize(&tree).unwrap();
    odb.write(ObjectType::Tree, &tree_data).await.unwrap()
}

fn bench_branch_create(c: &mut Criterion) {
    let rt = tokio::runtime::Runtime::new().unwrap();

    c.bench_function("branch_create", |b| {
        b.to_async(&rt).iter_with_setup(
            || {
                let (branch_mgr, odb, _temp) = rt.block_on(setup_branch_manager_async());
                let commit_oid = rt.block_on(async {
                    let tree_oid = create_test_tree(&odb).await;
                    create_test_commit(&odb, tree_oid, None, "Initial commit").await
                });
                (branch_mgr, commit_oid)
            },
            |(branch_mgr, commit_oid)| async move {
                let branch_name = format!("feature/{}", uuid::Uuid::new_v4());
                let _: () = branch_mgr
                .create(&branch_name, commit_oid)
                .await
                .unwrap();
                black_box(
                    (),
                )
            },
        );
    });
}

fn bench_branch_switch(c: &mut Criterion) {
    let rt = tokio::runtime::Runtime::new().unwrap();

    // Target: <100ms
    c.bench_function("branch_switch", |b| {
        let (branch_mgr, odb, _temp) = rt.block_on(setup_branch_manager_async());

        // Setup: create main and feature branches
        let (_main_commit, _feature_commit) = rt.block_on(async {
            let tree_oid = create_test_tree(&odb).await;
            let main_commit = create_test_commit(&odb, tree_oid, None, "Main commit").await;
            let feature_commit =
                create_test_commit(&odb, tree_oid, Some(main_commit), "Feature commit").await;

            branch_mgr.create("main", main_commit).await.unwrap();
            branch_mgr.create("feature", feature_commit).await.unwrap();
            branch_mgr.switch_to("main").await.unwrap();

            (main_commit, feature_commit)
        });

        // Use counter instead of mutable toggle to avoid closure capture issues
        let counter = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));

        b.to_async(&rt).iter(|| {
            let branch_mgr = &branch_mgr;
            let counter = counter.clone();
            async move {
                let idx = counter.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                if idx.is_multiple_of(2) {
                    let _: () = branch_mgr.switch_to("feature").await.unwrap();
                    black_box(());
                } else {
                    let _: () = branch_mgr.switch_to("main").await.unwrap();
                    black_box(());
                }
            }
        });
    });
}

fn bench_branch_list(c: &mut Criterion) {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let mut group = c.benchmark_group("branch_list");

    for count in [10, 50, 100].iter() {
        group.bench_with_input(
            BenchmarkId::new("list", count),
            count,
            |b, &count| {
                let (branch_mgr, odb, _temp) = rt.block_on(setup_branch_manager_async());

                // Setup: create multiple branches
                rt.block_on(async {
                    let tree_oid = create_test_tree(&odb).await;
                    let commit_oid =
                        create_test_commit(&odb, tree_oid, None, "Initial commit").await;

                    for i in 0..count {
                        let branch_name = format!("branch_{}", i);
                        branch_mgr.create(&branch_name, commit_oid).await.unwrap();
                    }
                });

                b.to_async(&rt)
                    .iter(|| async { black_box(branch_mgr.list().await.unwrap()) });
            },
        );
    }

    group.finish();
}

fn bench_branch_delete(c: &mut Criterion) {
    let rt = tokio::runtime::Runtime::new().unwrap();

    c.bench_function("branch_delete", |b| {
        b.to_async(&rt).iter_with_setup(
            || {
                let (branch_mgr, odb, _temp) = rt.block_on(setup_branch_manager_async());
                let branch_name = format!("temp_{}", uuid::Uuid::new_v4());

                rt.block_on(async {
                    let tree_oid = create_test_tree(&odb).await;
                    let commit_oid =
                        create_test_commit(&odb, tree_oid, None, "Test commit").await;
                    branch_mgr.create(&branch_name, commit_oid).await.unwrap();
                });

                (branch_mgr, branch_name)
            },
            |(branch_mgr, branch_name)| async move {
                let _: () = branch_mgr.delete(&branch_name).await.unwrap();
                black_box(())
            },
        );
    });
}

fn bench_branch_get_current(c: &mut Criterion) {
    let rt = tokio::runtime::Runtime::new().unwrap();

    c.bench_function("branch_get_current", |b| {
        let (branch_mgr, odb, _temp) = rt.block_on(setup_branch_manager_async());

        // Setup: create and switch to a branch
        rt.block_on(async {
            let tree_oid = create_test_tree(&odb).await;
            let commit_oid = create_test_commit(&odb, tree_oid, None, "Initial commit").await;
            branch_mgr.create("main", commit_oid).await.unwrap();
            branch_mgr.switch_to("main").await.unwrap();
        });

        b.to_async(&rt)
            .iter(|| async { black_box(branch_mgr.current_branch().await.unwrap()) });
    });
}

fn bench_branch_exists(c: &mut Criterion) {
    let rt = tokio::runtime::Runtime::new().unwrap();

    c.bench_function("branch_exists_check", |b| {
        let (branch_mgr, odb, _temp) = rt.block_on(setup_branch_manager_async());

        // Setup: create branches
        rt.block_on(async {
            let tree_oid = create_test_tree(&odb).await;
            let commit_oid = create_test_commit(&odb, tree_oid, None, "Initial commit").await;
            branch_mgr.create("main", commit_oid).await.unwrap();
        });

        b.to_async(&rt)
            .iter(|| async { black_box(branch_mgr.exists("main").await.unwrap()) });
    });
}

fn bench_branch_update(c: &mut Criterion) {
    let rt = tokio::runtime::Runtime::new().unwrap();

    c.bench_function("branch_update", |b| {
        let (branch_mgr, odb, _temp) = rt.block_on(setup_branch_manager_async());

        // Setup: create branch and commits
        let (initial_commit, new_commit) = rt.block_on(async {
            let tree_oid = create_test_tree(&odb).await;
            let initial_commit = create_test_commit(&odb, tree_oid, None, "Initial commit").await;
            let new_commit =
                create_test_commit(&odb, tree_oid, Some(initial_commit), "New commit").await;
            branch_mgr.create("main", initial_commit).await.unwrap();
            (initial_commit, new_commit)
        });

        // Use counter instead of mutable toggle
        let counter = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));

        b.to_async(&rt).iter(|| {
            let branch_mgr = &branch_mgr;
            let counter = counter.clone();
            async move {
                let idx = counter.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                let commit = if idx.is_multiple_of(2) { new_commit } else { initial_commit };
                let _: () = branch_mgr
                .update_to("main", commit, false)
                .await
                .unwrap();
                black_box(
                    (),
                )
            }
        });
    });
}

fn bench_branch_concurrent_ops(c: &mut Criterion) {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let mut group = c.benchmark_group("branch_concurrent");

    for threads in [2, 4].iter() {
        group.bench_with_input(
            BenchmarkId::new("concurrent_read", threads),
            threads,
            |b, &threads| {
                let (branch_mgr, odb, _temp) = rt.block_on(setup_branch_manager_async());

                // Setup: create branches
                rt.block_on(async {
                    let tree_oid = create_test_tree(&odb).await;
                    let commit_oid =
                        create_test_commit(&odb, tree_oid, None, "Initial commit").await;

                    for i in 0..20 {
                        let branch_name = format!("branch_{}", i);
                        branch_mgr.create(&branch_name, commit_oid).await.unwrap();
                    }
                });

                // Wrap branch_mgr in Arc for concurrent access
                let branch_mgr = Arc::new(branch_mgr);

                b.to_async(&rt).iter(|| {
                    let branch_mgr = branch_mgr.clone();
                    async move {
                        let mut handles = Vec::new();

                        for _ in 0..threads {
                            let branch_mgr = branch_mgr.clone();
                            handles.push(tokio::spawn(async move {
                                for i in 0..5 {
                                    let branch_name = format!("branch_{}", i);
                                    black_box(branch_mgr.exists(&branch_name).await.unwrap());
                                }
                            }));
                        }

                        for handle in handles {
                            handle.await.unwrap();
                        }
                    }
                });
            },
        );
    }

    group.finish();
}

criterion_group!(
    benches,
    bench_branch_create,
    bench_branch_switch,
    bench_branch_list,
    bench_branch_delete,
    bench_branch_get_current,
    bench_branch_exists,
    bench_branch_update,
    bench_branch_concurrent_ops
);
criterion_main!(benches);
