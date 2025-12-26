//! Integration tests for shallow clone CLI commands
//!
//! Tests the mediagit shallow command functionality including:
//! - Status checking
//! - Shallow clone creation
//! - Unshallow operation
//! - Error handling

use anyhow::Result;
use mediagit_storage::LocalBackend;
use mediagit_versioning::{
    Commit, ObjectDatabase, Ref, RefDatabase, ShallowDatabase, Signature, Tree,
};
use std::sync::Arc;
use tempfile::TempDir;

/// Helper to create a test repository with commits
async fn create_test_repository(commit_count: usize) -> Result<(TempDir, Arc<ObjectDatabase>)> {
    let temp_dir = TempDir::new()?;
    let mediagit_dir = temp_dir.path().join(".mediagit");
    std::fs::create_dir_all(&mediagit_dir)?;

    // Initialize storage
    let storage: Arc<dyn mediagit_storage::StorageBackend> =
        Arc::new(LocalBackend::new(&mediagit_dir).await?);
    let odb = Arc::new(ObjectDatabase::with_smart_compression(storage.clone(), 1000));

    // Initialize ref database
    let refdb = RefDatabase::new(&mediagit_dir);

    // Create empty tree
    let empty_tree = Tree::default();
    let tree_oid = empty_tree.write(&odb).await?;

    // Create commit chain
    let mut previous_oid = None;
    let author = Signature::now("Test User".to_string(), "test@example.com".to_string());

    for i in 0..commit_count {
        let mut commit = Commit::new(
            tree_oid.clone(),
            author.clone(),
            author.clone(),
            format!("Commit {}", i + 1),
        );

        // Add parent if exists
        if let Some(parent) = previous_oid {
            commit.add_parent(parent);
        }

        let commit_oid = commit.write(&odb).await?;
        previous_oid = Some(commit_oid.clone());

        // Update HEAD to point to latest commit
        if i == commit_count - 1 {
            let main_ref = Ref::new_direct(
                "refs/heads/main".to_string(),
                commit_oid.clone(),
            );
            refdb.write(&main_ref).await?;

            let head = Ref::new_symbolic("HEAD".to_string(), "refs/heads/main".to_string());
            refdb.write(&head).await?;
        }
    }

    Ok((temp_dir, odb))
}

#[tokio::test]
async fn test_shallow_status_non_shallow_repo() -> Result<()> {
    let (temp_dir, _odb) = create_test_repository(10).await?;
    let mediagit_dir = temp_dir.path().join(".mediagit");

    let shallow_db = ShallowDatabase::new(temp_dir.path());
    assert!(!shallow_db.is_shallow());

    Ok(())
}

#[tokio::test]
async fn test_shallow_create_depth_1() -> Result<()> {
    let (temp_dir, odb) = create_test_repository(100).await?;
    let mediagit_dir = temp_dir.path().join(".mediagit");

    // Get HEAD commit
    let refdb = RefDatabase::new(&mediagit_dir);
    let head_oid = refdb.resolve("HEAD").await?;

    // Create shallow clone with depth=1
    use mediagit_versioning::CommitWalker;
    let mut walker = CommitWalker::new(odb.clone());
    let result = walker.walk_shallow(&head_oid, 0).await?;

    assert_eq!(result.commits.len(), 1, "Depth=1 should have 1 commit");
    assert_eq!(
        result.shallow_boundaries.len(),
        1,
        "Should have 1 boundary"
    );

    // Write shallow boundaries
    let shallow_db = ShallowDatabase::new(temp_dir.path());
    shallow_db.write_boundaries(&result.shallow_boundaries)?;

    // Verify shallow state
    assert!(shallow_db.is_shallow());
    let boundaries = shallow_db.read_boundaries()?;
    assert_eq!(boundaries.len(), 1);

    Ok(())
}

#[tokio::test]
async fn test_shallow_create_depth_10() -> Result<()> {
    let (temp_dir, odb) = create_test_repository(100).await?;
    let mediagit_dir = temp_dir.path().join(".mediagit");

    let refdb = RefDatabase::new(&mediagit_dir);
    let head_oid = refdb.resolve("HEAD").await?;

    // Create shallow clone with depth=10
    use mediagit_versioning::CommitWalker;
    let mut walker = CommitWalker::new(odb.clone());
    let result = walker.walk_shallow(&head_oid, 9).await?;

    assert_eq!(result.commits.len(), 10, "Depth=10 should have 10 commits");

    let shallow_db = ShallowDatabase::new(temp_dir.path());
    shallow_db.write_boundaries(&result.shallow_boundaries)?;

    assert!(shallow_db.is_shallow());

    Ok(())
}

#[tokio::test]
async fn test_shallow_unshallow() -> Result<()> {
    let (temp_dir, odb) = create_test_repository(50).await?;
    let mediagit_dir = temp_dir.path().join(".mediagit");

    // Create shallow clone
    let refdb = RefDatabase::new(&mediagit_dir);
    let head_oid = refdb.resolve("HEAD").await?;

    use mediagit_versioning::CommitWalker;
    let mut walker = CommitWalker::new(odb.clone());
    let result = walker.walk_shallow(&head_oid, 0).await?;

    let shallow_db = ShallowDatabase::new(temp_dir.path());
    shallow_db.write_boundaries(&result.shallow_boundaries)?;

    assert!(shallow_db.is_shallow());

    // Unshallow (remove boundaries)
    shallow_db.write_boundaries(&std::collections::HashSet::new())?;

    assert!(!shallow_db.is_shallow());

    Ok(())
}

#[tokio::test]
async fn test_shallow_file_format() -> Result<()> {
    let (temp_dir, odb) = create_test_repository(20).await?;
    let mediagit_dir = temp_dir.path().join(".mediagit");

    let refdb = RefDatabase::new(&mediagit_dir);
    let head_oid = refdb.resolve("HEAD").await?;

    use mediagit_versioning::CommitWalker;
    let mut walker = CommitWalker::new(odb.clone());
    let result = walker.walk_shallow(&head_oid, 2).await?;

    let shallow_db = ShallowDatabase::new(temp_dir.path());

    // Only write boundaries if there are any
    if !result.shallow_boundaries.is_empty() {
        shallow_db.write_boundaries(&result.shallow_boundaries)?;

        // Verify shallow file exists
        let shallow_file = mediagit_dir.join("shallow");
        assert!(shallow_file.exists(), "Shallow file should exist after writing {} boundaries", result.shallow_boundaries.len());

        // Read file and verify format
        let content = std::fs::read_to_string(&shallow_file)?;

        // Filter out comments and empty lines to get actual OIDs
        let oid_lines: Vec<&str> = content
            .lines()
            .map(|l| l.trim())
            .filter(|l| !l.is_empty() && !l.starts_with('#'))
            .collect();

        assert_eq!(
            oid_lines.len(),
            result.shallow_boundaries.len(),
            "Shallow file should have one OID line per boundary"
        );

        // Each OID line should be a valid OID (64 hex characters)
        for line in &oid_lines {
            assert_eq!(line.len(), 64, "Each OID line should be 64 hex characters");
            assert!(
                line.chars().all(|c| c.is_ascii_hexdigit()),
                "Each OID line should contain only hex digits"
            );
        }
    } else {
        // Test expects boundaries - fail if there are none
        panic!("Expected shallow boundaries but got none (depth=3, commit_count=20)");
    }

    Ok(())
}

#[tokio::test]
async fn test_shallow_storage_reduction() -> Result<()> {
    let (temp_dir, odb) = create_test_repository(1000).await?;
    let mediagit_dir = temp_dir.path().join(".mediagit");

    let refdb = RefDatabase::new(&mediagit_dir);
    let head_oid = refdb.resolve("HEAD").await?;

    // Create shallow clone with depth=1 (CI/CD use case)
    use mediagit_versioning::CommitWalker;
    let mut walker = CommitWalker::new(odb.clone());
    let result = walker.walk_shallow(&head_oid, 0).await?;

    let total_commits = 1000;
    let shallow_commits = result.commits.len();

    // Calculate storage reduction
    let reduction_pct = ((total_commits - shallow_commits) as f64 / total_commits as f64) * 100.0;

    assert_eq!(shallow_commits, 1, "Depth=1 should only have 1 commit");
    assert!(
        reduction_pct > 99.0,
        "Storage reduction should be >99% for depth=1 on 1000 commits"
    );

    Ok(())
}

#[tokio::test]
async fn test_shallow_boundary_persistence() -> Result<()> {
    let (temp_dir, odb) = create_test_repository(50).await?;
    let mediagit_dir = temp_dir.path().join(".mediagit");

    let refdb = RefDatabase::new(&mediagit_dir);
    let head_oid = refdb.resolve("HEAD").await?;

    use mediagit_versioning::CommitWalker;
    let mut walker = CommitWalker::new(odb.clone());
    let result = walker.walk_shallow(&head_oid, 4).await?;

    let shallow_db = ShallowDatabase::new(temp_dir.path());
    shallow_db.write_boundaries(&result.shallow_boundaries)?;

    // Create new ShallowDatabase instance to verify persistence
    let shallow_db2 = ShallowDatabase::new(temp_dir.path());
    assert!(shallow_db2.is_shallow());

    let boundaries1 = shallow_db.read_boundaries()?;
    let boundaries2 = shallow_db2.read_boundaries()?;

    assert_eq!(
        boundaries1, boundaries2,
        "Boundaries should persist across ShallowDatabase instances"
    );

    Ok(())
}

#[tokio::test]
async fn test_shallow_multiple_boundaries() -> Result<()> {
    // Note: This test would require a repository with merge commits
    // to naturally have multiple boundaries. For now, we test the
    // data structure can handle multiple boundaries.

    let temp_dir = TempDir::new()?;
    let mediagit_dir = temp_dir.path().join(".mediagit");
    std::fs::create_dir_all(&mediagit_dir)?;

    let shallow_db = ShallowDatabase::new(temp_dir.path());

    // Create some fake boundary OIDs
    use mediagit_versioning::Oid;
    let mut boundaries = std::collections::HashSet::new();
    let oid1 = Oid::hash(b"commit1");
    let oid2 = Oid::hash(b"commit2");
    let oid3 = Oid::hash(b"commit3");

    boundaries.insert(oid1);
    boundaries.insert(oid2);
    boundaries.insert(oid3);

    shallow_db.write_boundaries(&boundaries)?;

    let read_boundaries = shallow_db.read_boundaries()?;
    assert_eq!(read_boundaries.len(), 3);
    assert!(read_boundaries.contains(&oid1));
    assert!(read_boundaries.contains(&oid2));
    assert!(read_boundaries.contains(&oid3));

    Ok(())
}
