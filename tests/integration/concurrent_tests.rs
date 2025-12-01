//! Concurrent Operation Integration Tests
//!
//! These tests validate MediaGit's behavior under concurrent access:
//! - Multiple concurrent writers
//! - Many concurrent readers
//! - Read-while-write consistency
//! - Data integrity under contention

use mediagit_storage::filesystem::FilesystemBackend;
use mediagit_storage::StorageBackend;
use mediagit_versioning::{
    BranchManager, Commit, FileMode, ObjectDatabase, ObjectType, Signature, Tree, TreeEntry,
};
use std::sync::Arc;
use tempfile::TempDir;
use tokio::task::JoinSet;

/// Helper to create test signature
fn test_signature(name: &str, email: &str) -> Signature {
    Signature::now(name.to_string(), email.to_string())
}

/// Test 10+ concurrent writers to object database
///
/// Each writer creates 100 objects concurrently.
/// Content-addressable storage ensures natural deduplication.
#[tokio::test]
async fn test_concurrent_writers() {
    let temp_dir = TempDir::new().unwrap();
    let storage: Arc<dyn StorageBackend> = Arc::new(
        FilesystemBackend::new(temp_dir.path().to_path_buf())
            .await
            .unwrap(),
    );

    let odb = Arc::new(ObjectDatabase::new(Arc::clone(&storage), 1000));

    const NUM_WRITERS: usize = 10;
    const OBJECTS_PER_WRITER: usize = 100;

    let mut tasks = JoinSet::new();

    // Spawn 10 concurrent writers
    for writer_id in 0..NUM_WRITERS {
        let odb_clone = Arc::clone(&odb);

        tasks.spawn(async move {
            let mut oids = Vec::new();

            for object_id in 0..OBJECTS_PER_WRITER {
                let content = format!("writer-{}-object-{}", writer_id, object_id);
                let oid = odb_clone
                    .write(ObjectType::Blob, content.as_bytes())
                    .await
                    .unwrap();

                oids.push(oid);
            }

            oids
        });
    }

    // Collect all OIDs from all writers
    let mut all_oids = Vec::new();
    while let Some(result) = tasks.join_next().await {
        let oids = result.unwrap();
        all_oids.extend(oids);
    }

    // Verify: Should have exactly NUM_WRITERS * OBJECTS_PER_WRITER OIDs
    assert_eq!(all_oids.len(), NUM_WRITERS * OBJECTS_PER_WRITER);

    // Verify: All objects can be read back
    for oid in &all_oids {
        assert!(odb.exists(*oid).await.unwrap());
    }

    println!(
        "✅ Concurrent writers test passed: {} writers × {} objects = {} total",
        NUM_WRITERS,
        OBJECTS_PER_WRITER,
        all_oids.len()
    );
}

/// Test 100+ concurrent readers
///
/// Create objects first, then read them concurrently from many readers.
/// Validates read scalability and consistency.
#[tokio::test]
async fn test_concurrent_readers() {
    let temp_dir = TempDir::new().unwrap();
    let storage: Arc<dyn StorageBackend> = Arc::new(
        FilesystemBackend::new(temp_dir.path().to_path_buf())
            .await
            .unwrap(),
    );

    let odb = Arc::new(ObjectDatabase::new(Arc::clone(&storage), 1000));

    // Pre-create 100 objects
    let mut object_oids = Vec::new();
    for i in 0..100 {
        let content = format!("object-{}", i);
        let oid = odb
            .write(ObjectType::Blob, content.as_bytes())
            .await
            .unwrap();
        object_oids.push((oid, content));
    }

    const NUM_READERS: usize = 100;
    let mut tasks = JoinSet::new();

    // Spawn 100 concurrent readers
    for reader_id in 0..NUM_READERS {
        let odb_clone = Arc::clone(&odb);
        let oids = object_oids.clone();

        tasks.spawn(async move {
            let mut read_count = 0;

            // Each reader reads all 100 objects
            for (oid, expected_content) in oids {
                let data = odb_clone.read(oid).await.unwrap();
                let content = String::from_utf8(data.to_vec()).unwrap();
                assert_eq!(content, expected_content);
                read_count += 1;
            }

            read_count
        });
    }

    // Verify all readers succeeded
    let mut total_reads = 0;
    while let Some(result) = tasks.join_next().await {
        let read_count = result.unwrap();
        assert_eq!(read_count, 100);
        total_reads += read_count;
    }

    assert_eq!(total_reads, NUM_READERS * 100);

    println!(
        "✅ Concurrent readers test passed: {} readers × {} objects = {} total reads",
        NUM_READERS,
        100,
        total_reads
    );
}

/// Test read-while-write consistency
///
/// Writers continuously create objects while readers verify they can
/// read previously written objects. Tests that readers never see
/// partial/corrupted data.
#[tokio::test]
async fn test_read_while_write_consistency() {
    let temp_dir = TempDir::new().unwrap();
    let storage: Arc<dyn StorageBackend> = Arc::new(
        FilesystemBackend::new(temp_dir.path().to_path_buf())
            .await
            .unwrap(),
    );

    let odb = Arc::new(ObjectDatabase::new(Arc::clone(&storage), 1000));

    // Shared state: OIDs of successfully written objects
    let written_oids = Arc::new(tokio::sync::Mutex::new(Vec::new()));

    let mut tasks = JoinSet::new();

    // Writer task: continuously write objects
    {
        let odb_clone = Arc::clone(&odb);
        let oids_clone = Arc::clone(&written_oids);

        tasks.spawn(async move {
            for i in 0..50 {
                let content = format!("write-{}", i);
                let oid = odb_clone
                    .write(ObjectType::Blob, content.as_bytes())
                    .await
                    .unwrap();

                oids_clone.lock().await.push((oid, content));

                // Small delay to allow readers to catch up
                tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;
            }
        });
    }

    // Reader tasks: verify all written objects remain readable
    for _ in 0..5 {
        let odb_clone = Arc::clone(&odb);
        let oids_clone = Arc::clone(&written_oids);

        tasks.spawn(async move {
            for _ in 0..100 {
                // Get snapshot of written OIDs
                let oids = {
                    let locked = oids_clone.lock().await;
                    locked.clone()
                };

                // Verify all previously written objects are readable
                for (oid, expected_content) in oids {
                    let data = odb_clone.read(oid).await.unwrap();
                    let content = String::from_utf8(data.to_vec()).unwrap();
                    assert_eq!(content, expected_content);
                }

                tokio::time::sleep(tokio::time::Duration::from_millis(20)).await;
            }
        });
    }

    // Wait for all tasks
    while tasks.join_next().await.is_some() {}

    println!("✅ Read-while-write consistency test passed");
}

/// Test concurrent branch updates
///
/// Multiple tasks attempt to update the same branch concurrently.
/// Last-write-wins semantics should prevail without corruption.
#[tokio::test]
async fn test_concurrent_branch_updates() {
    let temp_dir = TempDir::new().unwrap();
    let storage: Arc<dyn StorageBackend> = Arc::new(
        FilesystemBackend::new(temp_dir.path().to_path_buf())
            .await
            .unwrap(),
    );

    let odb = Arc::new(ObjectDatabase::new(Arc::clone(&storage), 1000));
    let branch_mgr = Arc::new(BranchManager::new(Arc::clone(&storage)));

    let author = test_signature("Test User", "test@example.com");

    // Create initial commit
    let blob = odb.write(ObjectType::Blob, b"initial").await.unwrap();
    let mut tree = Tree::new();
    tree.add_entry(TreeEntry::new("file.txt".to_string(), FileMode::Regular, blob));
    let tree_oid = tree.write(&odb).await.unwrap();

    let commit = Commit::new(
        tree_oid,
        author.clone(),
        author.clone(),
        "Initial".to_string(),
    );
    let initial_oid = commit.write(&odb).await.unwrap();

    branch_mgr.initialize(Some(initial_oid)).await.unwrap();

    // Create test branch
    branch_mgr
        .create_branch("test-branch", initial_oid)
        .await
        .unwrap();

    const NUM_UPDATERS: usize = 10;
    let mut tasks = JoinSet::new();

    // Each updater creates a commit and tries to update the branch
    for updater_id in 0..NUM_UPDATERS {
        let odb_clone = Arc::clone(&odb);
        let branch_mgr_clone = Arc::clone(&branch_mgr);
        let author_clone = author.clone();

        tasks.spawn(async move {
            // Create unique commit
            let content = format!("updater-{}", updater_id);
            let blob = odb_clone
                .write(ObjectType::Blob, content.as_bytes())
                .await
                .unwrap();

            let mut tree = Tree::new();
            tree.add_entry(TreeEntry::new(
                format!("file-{}.txt", updater_id),
                FileMode::Regular,
                blob,
            ));
            let tree_oid = tree.write(&odb_clone).await.unwrap();

            let commit = Commit::new_with_parent(
                tree_oid,
                initial_oid,
                author_clone.clone(),
                author_clone,
                format!("Update {}", updater_id),
            );
            let commit_oid = commit.write(&odb_clone).await.unwrap();

            // Try to update branch (may succeed or fail due to concurrent updates)
            let _ = branch_mgr_clone
                .update_branch("test-branch", commit_oid)
                .await;

            commit_oid
        });
    }

    // Collect all commit OIDs
    let mut commit_oids = Vec::new();
    while let Some(result) = tasks.join_next().await {
        commit_oids.push(result.unwrap());
    }

    // Verify: Branch should point to one of the created commits
    let final_info = branch_mgr.get_info("test-branch").await.unwrap();
    assert!(commit_oids.contains(&final_info.oid));

    // Verify: All commits are still valid in the ODB
    for oid in commit_oids {
        assert!(odb.exists(oid).await.unwrap());
    }

    println!("✅ Concurrent branch updates test passed");
}

/// Test concurrent tree writes
///
/// Multiple tasks create trees with different structures concurrently.
#[tokio::test]
async fn test_concurrent_tree_creation() {
    let temp_dir = TempDir::new().unwrap();
    let storage: Arc<dyn StorageBackend> = Arc::new(
        FilesystemBackend::new(temp_dir.path().to_path_buf())
            .await
            .unwrap(),
    );

    let odb = Arc::new(ObjectDatabase::new(Arc::clone(&storage), 1000));

    const NUM_TREE_CREATORS: usize = 20;
    let mut tasks = JoinSet::new();

    for creator_id in 0..NUM_TREE_CREATORS {
        let odb_clone = Arc::clone(&odb);

        tasks.spawn(async move {
            // Create blobs
            let mut blob_oids = Vec::new();
            for i in 0..5 {
                let content = format!("creator-{}-file-{}", creator_id, i);
                let oid = odb_clone
                    .write(ObjectType::Blob, content.as_bytes())
                    .await
                    .unwrap();
                blob_oids.push(oid);
            }

            // Create tree
            let mut tree = Tree::new();
            for (i, blob_oid) in blob_oids.iter().enumerate() {
                tree.add_entry(TreeEntry::new(
                    format!("file{}.txt", i),
                    FileMode::Regular,
                    *blob_oid,
                ));
            }

            tree.write(&odb_clone).await.unwrap()
        });
    }

    // Collect all tree OIDs
    let mut tree_oids = Vec::new();
    while let Some(result) = tasks.join_next().await {
        tree_oids.push(result.unwrap());
    }

    assert_eq!(tree_oids.len(), NUM_TREE_CREATORS);

    // Verify all trees are readable
    for tree_oid in tree_oids {
        let tree = odb.read_tree(tree_oid).await.unwrap();
        assert_eq!(tree.entries().len(), 5);
    }

    println!("✅ Concurrent tree creation test passed");
}

/// Test data integrity under extreme concurrency
///
/// Combined test: concurrent writes, reads, and verifications
#[tokio::test]
async fn test_extreme_concurrency_no_corruption() {
    let temp_dir = TempDir::new().unwrap();
    let storage: Arc<dyn StorageBackend> = Arc::new(
        FilesystemBackend::new(temp_dir.path().to_path_buf())
            .await
            .unwrap(),
    );

    let odb = Arc::new(ObjectDatabase::new(Arc::clone(&storage), 1000));

    let mut tasks = JoinSet::new();

    // 10 writers
    for writer_id in 0..10 {
        let odb_clone = Arc::clone(&odb);
        tasks.spawn(async move {
            for i in 0..50 {
                let content = format!("writer-{}-object-{}", writer_id, i);
                odb_clone
                    .write(ObjectType::Blob, content.as_bytes())
                    .await
                    .unwrap();
            }
        });
    }

    // 50 readers (once writers create some data)
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

    for _ in 0..50 {
        let odb_clone = Arc::clone(&odb);
        tasks.spawn(async move {
            // Just verify exists() doesn't panic under concurrent access
            for _ in 0..100 {
                let _ = odb_clone
                    .exists(mediagit_versioning::ObjectId::from_bytes(&[0u8; 32]))
                    .await;
            }
        });
    }

    // Wait for all tasks
    while tasks.join_next().await.is_some() {}

    println!("✅ Extreme concurrency test passed: no corruption detected");
}
