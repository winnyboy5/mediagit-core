// Copyright (C) 2025 MediaGit Contributors
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Merge operations performance benchmarks
//!
//! Performance targets:
//! - Merge operations: <200ms for 50-file trees

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion};
use mediagit_storage::LocalBackend;
use mediagit_versioning::{
    Commit, FileMode, MergeEngine, MergeStrategy, ObjectDatabase, ObjectType, Oid, Signature, Tree,
    TreeEntry,
};
use std::collections::BTreeMap;
use std::sync::Arc;
use tempfile::TempDir;

/// Setup merge engine with temporary storage (async version)
async fn setup_merge_engine_async() -> (MergeEngine, ObjectDatabase, TempDir) {
    let temp_dir = TempDir::new().unwrap();
    let storage = Arc::new(LocalBackend::new(temp_dir.path().to_str().unwrap()).await.unwrap());
    let odb_for_merge = Arc::new(ObjectDatabase::new(storage.clone(), 1000));
    let merge_engine = MergeEngine::new(odb_for_merge);
    let odb = ObjectDatabase::new(storage, 1000);
    (merge_engine, odb, temp_dir)
}

/// Create a test signature
fn test_signature() -> Signature {
    Signature {
        name: "Test User".to_string(),
        email: "test@example.com".to_string(),
        timestamp: chrono::Utc::now(),
    }
}

/// Create a test commit
async fn create_commit(
    odb: &ObjectDatabase,
    tree_oid: Oid,
    parents: Vec<Oid>,
    message: &str,
) -> Oid {
    let commit = Commit {
        tree: tree_oid,
        parents,
        author: test_signature(),
        committer: test_signature(),
        message: message.to_string(),
    };

    let commit_data = bincode::serialize(&commit).unwrap();
    odb.write(ObjectType::Commit, &commit_data).await.unwrap()
}

/// Create a tree with specified number of entries
async fn create_tree(odb: &ObjectDatabase, num_files: usize) -> Oid {
    let mut entries = BTreeMap::new();

    for i in 0..num_files {
        let filename = format!("file_{:04}.txt", i); // Zero-padded for consistent sorting
        let content = format!("Content of file {}", i).into_bytes();
        let blob_oid = odb.write(ObjectType::Blob, &content).await.unwrap();

        entries.insert(filename.clone(), TreeEntry {
            mode: FileMode::Regular,
            name: filename,
            oid: blob_oid,
        });
    }

    let tree = Tree { entries };
    let tree_data = bincode::serialize(&tree).unwrap();
    odb.write(ObjectType::Tree, &tree_data).await.unwrap()
}

/// Create a modified tree with some changed files
async fn create_modified_tree(
    odb: &ObjectDatabase,
    base_tree_oid: Oid,
    num_changes: usize,
) -> Oid {
    // Read base tree
    let base_tree_data = odb.read(&base_tree_oid).await.unwrap();
    let mut base_tree: Tree = bincode::deserialize(&base_tree_data).unwrap();

    // Modify some entries
    let keys: Vec<String> = base_tree.entries.keys().take(num_changes).cloned().collect();
    for key in keys {
        if let Some(entry) = base_tree.entries.get_mut(&key) {
            let new_content = format!("Modified content for {}", key).into_bytes();
            let new_blob_oid = odb.write(ObjectType::Blob, &new_content).await.unwrap();
            entry.oid = new_blob_oid;
        }
    }

    let tree_data = bincode::serialize(&base_tree).unwrap();
    odb.write(ObjectType::Tree, &tree_data).await.unwrap()
}

fn bench_merge_fast_forward(c: &mut Criterion) {
    let rt = tokio::runtime::Runtime::new().unwrap();

    c.bench_function("merge_fast_forward", |b| {
        let (merge_engine, odb, _temp) = rt.block_on(setup_merge_engine_async());

        // Setup: create base and descendant commits
        let (base_commit, descendant_commit) = rt.block_on(async {
            let tree_oid = create_tree(&odb, 10).await;
            let base_commit = create_commit(&odb, tree_oid, vec![], "Base commit").await;

            let new_tree = create_modified_tree(&odb, tree_oid, 2).await;
            let descendant_commit =
                create_commit(&odb, new_tree, vec![base_commit], "Descendant commit").await;

            (base_commit, descendant_commit)
        });

        b.to_async(&rt).iter(|| async {
            black_box(
                merge_engine
                    .merge(
                        &black_box(base_commit),
                        &black_box(descendant_commit),
                        MergeStrategy::Recursive,
                    )
                    .await
                    .unwrap(),
            )
        });
    });
}

fn bench_merge_no_conflict(c: &mut Criterion) {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let mut group = c.benchmark_group("merge_no_conflict");

    // Target: <200ms for 50-file trees
    for num_files in [10, 25, 50].iter() {
        group.bench_with_input(
            BenchmarkId::new("files", num_files),
            num_files,
            |b, &num_files| {
                let (merge_engine, odb, _temp) = rt.block_on(setup_merge_engine_async());

                // Setup: create divergent commits with non-overlapping changes
                let (our_commit, their_commit) = rt.block_on(async {
                    let base_tree = create_tree(&odb, num_files).await;
                    let base_commit = create_commit(&odb, base_tree, vec![], "Base").await;

                    // Read base tree
                    let base_tree_data = odb.read(&base_tree).await.unwrap();
                    let mut our_tree: Tree = bincode::deserialize(&base_tree_data).unwrap();
                    let mut their_tree: Tree = bincode::deserialize(&base_tree_data).unwrap();

                    // Modify first half in our branch
                    let our_keys: Vec<String> = our_tree.entries.keys().take(num_files / 2).cloned().collect();
                    for key in our_keys {
                        if let Some(entry) = our_tree.entries.get_mut(&key) {
                            let content = format!("Our change for {}", key).into_bytes();
                            let blob_oid = odb.write(ObjectType::Blob, &content).await.unwrap();
                            entry.oid = blob_oid;
                        }
                    }

                    // Modify second half in their branch
                    let their_keys: Vec<String> = their_tree.entries.keys().skip(num_files / 2).cloned().collect();
                    for key in their_keys {
                        if let Some(entry) = their_tree.entries.get_mut(&key) {
                            let content = format!("Their change for {}", key).into_bytes();
                            let blob_oid = odb.write(ObjectType::Blob, &content).await.unwrap();
                            entry.oid = blob_oid;
                        }
                    }

                    let our_tree_oid = odb
                        .write(ObjectType::Tree, &bincode::serialize(&our_tree).unwrap())
                        .await
                        .unwrap();
                    let their_tree_oid = odb
                        .write(ObjectType::Tree, &bincode::serialize(&their_tree).unwrap())
                        .await
                        .unwrap();

                    let our_commit =
                        create_commit(&odb, our_tree_oid, vec![base_commit], "Our changes").await;
                    let their_commit = create_commit(
                        &odb,
                        their_tree_oid,
                        vec![base_commit],
                        "Their changes",
                    )
                    .await;

                    (our_commit, their_commit)
                });

                b.to_async(&rt).iter(|| async {
                    black_box(
                        merge_engine
                            .merge(
                                &black_box(our_commit),
                                &black_box(their_commit),
                                MergeStrategy::Recursive,
                            )
                            .await
                            .unwrap(),
                    )
                });
            },
        );
    }

    group.finish();
}

fn bench_merge_with_conflicts(c: &mut Criterion) {
    let rt = tokio::runtime::Runtime::new().unwrap();

    c.bench_function("merge_with_conflicts", |b| {
        let (merge_engine, odb, _temp) = rt.block_on(setup_merge_engine_async());

        // Setup: create divergent commits with conflicting changes
        let (our_commit, their_commit) = rt.block_on(async {
            let base_tree = create_tree(&odb, 10).await;
            let base_commit = create_commit(&odb, base_tree, vec![], "Base").await;

            // Read base tree
            let base_tree_data = odb.read(&base_tree).await.unwrap();
            let mut our_tree: Tree = bincode::deserialize(&base_tree_data).unwrap();
            let mut their_tree: Tree = bincode::deserialize(&base_tree_data).unwrap();

            // Modify same files differently (creates conflicts)
            let conflict_keys: Vec<String> = our_tree.entries.keys().take(5).cloned().collect();
            for key in conflict_keys {
                let our_content = format!("Our conflicting change for {}", key).into_bytes();
                let their_content = format!("Their conflicting change for {}", key).into_bytes();

                let our_blob = odb.write(ObjectType::Blob, &our_content).await.unwrap();
                let their_blob = odb.write(ObjectType::Blob, &their_content).await.unwrap();

                if let Some(entry) = our_tree.entries.get_mut(&key) {
                    entry.oid = our_blob;
                }
                if let Some(entry) = their_tree.entries.get_mut(&key) {
                    entry.oid = their_blob;
                }
            }

            let our_tree_oid = odb
                .write(ObjectType::Tree, &bincode::serialize(&our_tree).unwrap())
                .await
                .unwrap();
            let their_tree_oid = odb
                .write(ObjectType::Tree, &bincode::serialize(&their_tree).unwrap())
                .await
                .unwrap();

            let our_commit =
                create_commit(&odb, our_tree_oid, vec![base_commit], "Our changes").await;
            let their_commit =
                create_commit(&odb, their_tree_oid, vec![base_commit], "Their changes").await;

            (our_commit, their_commit)
        });

        b.to_async(&rt).iter(|| async {
            black_box(
                merge_engine
                    .merge(
                        &black_box(our_commit),
                        &black_box(their_commit),
                        MergeStrategy::Recursive,
                    )
                    .await
                    .unwrap(),
            )
        });
    });
}

fn bench_merge_strategies(c: &mut Criterion) {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let mut group = c.benchmark_group("merge_strategies");

    for strategy in [MergeStrategy::Recursive, MergeStrategy::Ours, MergeStrategy::Theirs].iter() {
        let strategy_name = format!("{:?}", strategy);

        group.bench_with_input(
            BenchmarkId::new("strategy", &strategy_name),
            strategy,
            |b, &strategy| {
                let (merge_engine, odb, _temp) = rt.block_on(setup_merge_engine_async());

                // Setup: create divergent commits
                let (our_commit, their_commit) = rt.block_on(async {
                    let base_tree = create_tree(&odb, 10).await;
                    let base_commit = create_commit(&odb, base_tree, vec![], "Base").await;

                    let our_tree = create_modified_tree(&odb, base_tree, 3).await;
                    let their_tree = create_modified_tree(&odb, base_tree, 3).await;

                    let our_commit =
                        create_commit(&odb, our_tree, vec![base_commit], "Our changes").await;
                    let their_commit =
                        create_commit(&odb, their_tree, vec![base_commit], "Their changes").await;

                    (our_commit, their_commit)
                });

                b.to_async(&rt).iter(|| async {
                    black_box(
                        merge_engine
                            .merge(
                                &black_box(our_commit),
                                &black_box(their_commit),
                                strategy,
                            )
                            .await
                            .unwrap(),
                    )
                });
            },
        );
    }

    group.finish();
}

fn bench_merge_lca_finding(c: &mut Criterion) {
    let rt = tokio::runtime::Runtime::new().unwrap();

    c.bench_function("merge_lca_finding", |b| {
        let (merge_engine, odb, _temp) = rt.block_on(setup_merge_engine_async());

        // Setup: create commit history with LCA
        let (our_commit, their_commit) = rt.block_on(async {
            let tree = create_tree(&odb, 5).await;

            // Common ancestor
            let base = create_commit(&odb, tree, vec![], "Base").await;

            // Our branch
            let our1 = create_commit(&odb, tree, vec![base], "Our 1").await;
            let our2 = create_commit(&odb, tree, vec![our1], "Our 2").await;

            // Their branch
            let their1 = create_commit(&odb, tree, vec![base], "Their 1").await;
            let their2 = create_commit(&odb, tree, vec![their1], "Their 2").await;

            (our2, their2)
        });

        b.to_async(&rt).iter(|| async {
            black_box(
                merge_engine
                    .merge(
                        &black_box(our_commit),
                        &black_box(their_commit),
                        MergeStrategy::Recursive,
                    )
                    .await
                    .unwrap(),
            )
        });
    });
}

fn bench_merge_tree_diff(c: &mut Criterion) {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let mut group = c.benchmark_group("merge_tree_diff");

    for num_changes in [5, 10, 20].iter() {
        group.bench_with_input(
            BenchmarkId::new("changes", num_changes),
            num_changes,
            |b, &num_changes| {
                let (merge_engine, odb, _temp) = rt.block_on(setup_merge_engine_async());

                // Setup: create commits with specified number of changes
                let (our_commit, their_commit) = rt.block_on(async {
                    let base_tree = create_tree(&odb, 50).await;
                    let base_commit = create_commit(&odb, base_tree, vec![], "Base").await;

                    let our_tree = create_modified_tree(&odb, base_tree, num_changes).await;
                    let their_tree = create_modified_tree(&odb, base_tree, num_changes).await;

                    let our_commit =
                        create_commit(&odb, our_tree, vec![base_commit], "Our changes").await;
                    let their_commit =
                        create_commit(&odb, their_tree, vec![base_commit], "Their changes").await;

                    (our_commit, their_commit)
                });

                b.to_async(&rt).iter(|| async {
                    black_box(
                        merge_engine
                            .merge(
                                &black_box(our_commit),
                                &black_box(their_commit),
                                MergeStrategy::Recursive,
                            )
                            .await
                            .unwrap(),
                    )
                });
            },
        );
    }

    group.finish();
}

criterion_group!(
    benches,
    bench_merge_fast_forward,
    bench_merge_no_conflict,
    bench_merge_with_conflicts,
    bench_merge_strategies,
    bench_merge_lca_finding,
    bench_merge_tree_diff
);
criterion_main!(benches);
