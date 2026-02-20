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
//! Merge Algorithm Integration Tests
//!
//! Tests for complete merge operations covering:
//! - Merge strategies (Recursive, Ours, Theirs)
//! - Fast-forward detection
//! - Conflict detection and resolution
//! - 3-way merge correctness

use mediagit_storage::mock::MockBackend;
use mediagit_versioning::{
    Commit, FileMode, MergeEngine, MergeStrategy, ObjectDatabase, ObjectType, Oid, Signature,
    Tree, TreeEntry,
};
use std::sync::Arc;

/// Helper to create test tree with entries
async fn create_test_tree(odb: &Arc<ObjectDatabase>, entries: Vec<(&str, &[u8])>) -> Oid {
    let mut tree = Tree::new();
    for (path, content) in entries {
        let blob_oid = odb.write(ObjectType::Blob, content).await.unwrap();

        tree.add_entry(TreeEntry::new(
            path.to_string(),
            FileMode::Regular,
            blob_oid,
        ));
    }

    tree.write(odb).await.unwrap()
}

/// Helper to create commit with tree
async fn create_commit_with_tree(
    odb: &Arc<ObjectDatabase>,
    message: &str,
    tree_oid: Oid,
    parents: Vec<Oid>,
) -> Oid {
    let sig = Signature::now(
        "Test Author".to_string(),
        "test@example.com".to_string(),
    );

    let commit = Commit::with_parents(
        tree_oid,
        parents,
        sig.clone(),
        sig,
        message.to_string(),
    );

    commit.write(odb).await.unwrap()
}

/// Test fast-forward merge detection
///
/// History: A <- B <- C
/// Merging B into C should be detected as fast-forward
#[tokio::test]
async fn test_fast_forward_merge() {
    let storage = Arc::new(MockBackend::new());
    let odb = Arc::new(ObjectDatabase::new(storage, 100));
    let merge_engine = MergeEngine::new(odb.clone());

    // Create linear history
    let tree_a = create_test_tree(&odb, vec![("file.txt", b"content A")]).await;
    let commit_a = create_commit_with_tree(&odb, "A", tree_a, vec![]).await;

    let tree_b = create_test_tree(&odb, vec![("file.txt", b"content B")]).await;
    let commit_b = create_commit_with_tree(&odb, "B", tree_b, vec![commit_a]).await;

    let tree_c = create_test_tree(&odb, vec![("file.txt", b"content C")]).await;
    let commit_c = create_commit_with_tree(&odb, "C", tree_c, vec![commit_b]).await;

    // Merge B into C (should be fast-forward)
    let result = merge_engine
        .merge(&commit_b, &commit_c, MergeStrategy::Recursive)
        .await
        .unwrap();

    assert!(result.is_fast_forward());
    assert!(result.success);
    assert!(result.conflicts.is_empty());
}

/// Test recursive merge strategy with no conflicts
///
/// History:
///     B   C
///      \ /
///       A
/// B and C modify different files
#[tokio::test]
async fn test_recursive_merge_no_conflict() {
    let storage = Arc::new(MockBackend::new());
    let odb = Arc::new(ObjectDatabase::new(storage, 100));
    let merge_engine = MergeEngine::new(odb.clone());

    // Base commit A
    let tree_a = create_test_tree(&odb, vec![("file1.txt", b"base1"), ("file2.txt", b"base2")])
        .await;
    let commit_a = create_commit_with_tree(&odb, "A", tree_a, vec![]).await;

    // Branch B modifies file1
    let tree_b = create_test_tree(
        &odb,
        vec![("file1.txt", b"modified in B"), ("file2.txt", b"base2")],
    )
    .await;
    let commit_b = create_commit_with_tree(&odb, "B", tree_b, vec![commit_a]).await;

    // Branch C modifies file2
    let tree_c = create_test_tree(
        &odb,
        vec![("file1.txt", b"base1"), ("file2.txt", b"modified in C")],
    )
    .await;
    let commit_c = create_commit_with_tree(&odb, "C", tree_c, vec![commit_a]).await;

    // Merge B and C - should succeed without conflicts
    let result = merge_engine
        .merge(&commit_b, &commit_c, MergeStrategy::Recursive)
        .await
        .unwrap();

    assert!(result.success);
    assert!(result.conflicts.is_empty());
    assert!(result.tree_oid.is_some());
}

/// Test recursive merge with conflicts
///
/// Both branches modify the same file
#[tokio::test]
async fn test_recursive_merge_with_conflict() {
    let storage = Arc::new(MockBackend::new());
    let odb = Arc::new(ObjectDatabase::new(storage, 100));
    let merge_engine = MergeEngine::new(odb.clone());

    // Base commit
    let tree_a = create_test_tree(&odb, vec![("file.txt", b"original content")]).await;
    let commit_a = create_commit_with_tree(&odb, "A", tree_a, vec![]).await;

    // Branch B modifies file
    let tree_b = create_test_tree(&odb, vec![("file.txt", b"content from B")]).await;
    let commit_b = create_commit_with_tree(&odb, "B", tree_b, vec![commit_a]).await;

    // Branch C modifies same file differently
    let tree_c = create_test_tree(&odb, vec![("file.txt", b"content from C")]).await;
    let commit_c = create_commit_with_tree(&odb, "C", tree_c, vec![commit_a]).await;

    // Merge should detect conflict
    let result = merge_engine
        .merge(&commit_b, &commit_c, MergeStrategy::Recursive)
        .await
        .unwrap();

    assert!(!result.success);
    assert!(result.has_conflicts());
    assert!(!result.conflicts.is_empty());

    // Verify conflict is on the expected file
    assert_eq!(result.conflicts[0].path, "file.txt");
}

/// Test "Ours" merge strategy
///
/// Always take our version on conflict
#[tokio::test]
async fn test_merge_strategy_ours() {
    let storage = Arc::new(MockBackend::new());
    let odb = Arc::new(ObjectDatabase::new(storage, 100));
    let merge_engine = MergeEngine::new(odb.clone());

    let tree_a = create_test_tree(&odb, vec![("file.txt", b"base")]).await;
    let commit_a = create_commit_with_tree(&odb, "A", tree_a, vec![]).await;

    let tree_b = create_test_tree(&odb, vec![("file.txt", b"ours")]).await;
    let commit_b = create_commit_with_tree(&odb, "B", tree_b, vec![commit_a]).await;

    let tree_c = create_test_tree(&odb, vec![("file.txt", b"theirs")]).await;
    let commit_c = create_commit_with_tree(&odb, "C", tree_c, vec![commit_a]).await;

    // Merge with "Ours" strategy
    let result = merge_engine
        .merge(&commit_b, &commit_c, MergeStrategy::Ours)
        .await
        .unwrap();

    assert!(result.success);
    assert!(result.conflicts.is_empty());
    assert_eq!(result.strategy, MergeStrategy::Ours);

    // Result should contain our version
    // (In real implementation, verify tree content matches commit_b)
}

/// Test "Theirs" merge strategy
///
/// Always take their version on conflict
#[tokio::test]
async fn test_merge_strategy_theirs() {
    let storage = Arc::new(MockBackend::new());
    let odb = Arc::new(ObjectDatabase::new(storage, 100));
    let merge_engine = MergeEngine::new(odb.clone());

    let tree_a = create_test_tree(&odb, vec![("file.txt", b"base")]).await;
    let commit_a = create_commit_with_tree(&odb, "A", tree_a, vec![]).await;

    let tree_b = create_test_tree(&odb, vec![("file.txt", b"ours")]).await;
    let commit_b = create_commit_with_tree(&odb, "B", tree_b, vec![commit_a]).await;

    let tree_c = create_test_tree(&odb, vec![("file.txt", b"theirs")]).await;
    let commit_c = create_commit_with_tree(&odb, "C", tree_c, vec![commit_a]).await;

    // Merge with "Theirs" strategy
    let result = merge_engine
        .merge(&commit_b, &commit_c, MergeStrategy::Theirs)
        .await
        .unwrap();

    assert!(result.success);
    assert!(result.conflicts.is_empty());
    assert_eq!(result.strategy, MergeStrategy::Theirs);
}

/// Test merge with file additions
///
/// One branch adds a new file
#[tokio::test]
async fn test_merge_with_file_addition() {
    let storage = Arc::new(MockBackend::new());
    let odb = Arc::new(ObjectDatabase::new(storage, 100));
    let merge_engine = MergeEngine::new(odb.clone());

    let tree_a = create_test_tree(&odb, vec![("existing.txt", b"content")]).await;
    let commit_a = create_commit_with_tree(&odb, "A", tree_a, vec![]).await;

    // Branch B adds new file
    let tree_b = create_test_tree(
        &odb,
        vec![("existing.txt", b"content"), ("new.txt", b"new content")],
    )
    .await;
    let commit_b = create_commit_with_tree(&odb, "B", tree_b, vec![commit_a]).await;

    // Branch C keeps original
    let commit_c = commit_a;

    let result = merge_engine
        .merge(&commit_b, &commit_c, MergeStrategy::Recursive)
        .await
        .unwrap();

    assert!(result.success);
    // Merged tree should include the new file
}

/// Test merge with file deletions
///
/// One branch deletes a file
#[tokio::test]
async fn test_merge_with_file_deletion() {
    let storage = Arc::new(MockBackend::new());
    let odb = Arc::new(ObjectDatabase::new(storage, 100));
    let merge_engine = MergeEngine::new(odb.clone());

    let tree_a = create_test_tree(&odb, vec![("file1.txt", b"content1"), ("file2.txt", b"content2")])
        .await;
    let commit_a = create_commit_with_tree(&odb, "A", tree_a, vec![]).await;

    // Branch B deletes file2
    let tree_b = create_test_tree(&odb, vec![("file1.txt", b"content1")]).await;
    let commit_b = create_commit_with_tree(&odb, "B", tree_b, vec![commit_a]).await;

    // Branch C keeps both files
    let commit_c = commit_a;

    let result = merge_engine
        .merge(&commit_b, &commit_c, MergeStrategy::Recursive)
        .await
        .unwrap();

    assert!(result.success);
    // Merged tree should have file deleted
}

/// Test delete-modify conflict
///
/// One branch deletes file, other modifies it
#[tokio::test]
async fn test_delete_modify_conflict() {
    let storage = Arc::new(MockBackend::new());
    let odb = Arc::new(ObjectDatabase::new(storage, 100));
    let merge_engine = MergeEngine::new(odb.clone());

    let tree_a = create_test_tree(&odb, vec![("file.txt", b"original")]).await;
    let commit_a = create_commit_with_tree(&odb, "A", tree_a, vec![]).await;

    // Branch B deletes file
    let tree_b = create_test_tree(&odb, vec![]).await;
    let commit_b = create_commit_with_tree(&odb, "B", tree_b, vec![commit_a]).await;

    // Branch C modifies file
    let tree_c = create_test_tree(&odb, vec![("file.txt", b"modified")]).await;
    let commit_c = create_commit_with_tree(&odb, "C", tree_c, vec![commit_a]).await;

    let result = merge_engine
        .merge(&commit_b, &commit_c, MergeStrategy::Recursive)
        .await
        .unwrap();

    // Should detect delete-modify conflict
    assert!(result.has_conflicts());
}

/// Test binary file conflict handling
#[tokio::test]
async fn test_binary_file_conflict() {
    let storage = Arc::new(MockBackend::new());
    let odb = Arc::new(ObjectDatabase::new(storage, 100));
    let merge_engine = MergeEngine::new(odb.clone());

    // Binary file (contains null bytes)
    let binary_base = vec![0x00, 0x01, 0x02, 0x03];
    let binary_ours = vec![0x00, 0x01, 0xFF, 0xFF];
    let binary_theirs = vec![0x00, 0x01, 0xAA, 0xBB];

    let tree_a = create_test_tree(&odb, vec![("binary.bin", &binary_base)]).await;
    let commit_a = create_commit_with_tree(&odb, "A", tree_a, vec![]).await;

    let tree_b = create_test_tree(&odb, vec![("binary.bin", &binary_ours)]).await;
    let commit_b = create_commit_with_tree(&odb, "B", tree_b, vec![commit_a]).await;

    let tree_c = create_test_tree(&odb, vec![("binary.bin", &binary_theirs)]).await;
    let commit_c = create_commit_with_tree(&odb, "C", tree_c, vec![commit_a]).await;

    let result = merge_engine
        .merge(&commit_b, &commit_c, MergeStrategy::Recursive)
        .await
        .unwrap();

    // Binary files should be marked as conflict
    assert!(result.has_conflicts());
    assert!(result.conflicts[0].path.contains("binary"));
}

/// Test merge performance for moderate complexity
#[tokio::test]
async fn test_merge_performance() {
    let storage = Arc::new(MockBackend::new());
    let odb = Arc::new(ObjectDatabase::new(storage, 100));
    let merge_engine = MergeEngine::new(odb.clone());

    // Create tree with many files
    let mut entries = Vec::new();
    for i in 0..50 {
        entries.push((format!("file{}.txt", i), format!("content {}", i)));
    }

    let entries_ref: Vec<(&str, &[u8])> = entries
        .iter()
        .map(|(k, v)| (k.as_str(), v.as_bytes()))
        .collect();

    let tree_a = create_test_tree(&odb, entries_ref.clone()).await;
    let commit_a = create_commit_with_tree(&odb, "A", tree_a, vec![]).await;

    // Modify some files in each branch
    let mut entries_b = entries.clone();
    entries_b[0].1 = "modified in B".to_string();
    let entries_b_ref: Vec<(&str, &[u8])> = entries_b
        .iter()
        .map(|(k, v)| (k.as_str(), v.as_bytes()))
        .collect();

    let tree_b = create_test_tree(&odb, entries_b_ref).await;
    let commit_b = create_commit_with_tree(&odb, "B", tree_b, vec![commit_a]).await;

    let mut entries_c = entries.clone();
    entries_c[25].1 = "modified in C".to_string();
    let entries_c_ref: Vec<(&str, &[u8])> = entries_c
        .iter()
        .map(|(k, v)| (k.as_str(), v.as_bytes()))
        .collect();

    let tree_c = create_test_tree(&odb, entries_c_ref).await;
    let commit_c = create_commit_with_tree(&odb, "C", tree_c, vec![commit_a]).await;

    let start = std::time::Instant::now();
    let result = merge_engine
        .merge(&commit_b, &commit_c, MergeStrategy::Recursive)
        .await
        .unwrap();
    let elapsed = start.elapsed();

    assert!(result.success);
    // Merge should be reasonably fast
    assert!(
        elapsed.as_millis() < 200,
        "Merge took {}ms, expected <200ms",
        elapsed.as_millis()
    );
}
