//! End-to-End Workflow Integration Tests
//!
//! These tests validate complete MediaGit workflows from initialization through
//! merge operations, simulating real user workflows.

use mediagit_storage::filesystem::FilesystemBackend;
use mediagit_storage::StorageBackend;
use mediagit_versioning::{
    BranchManager, Commit, FileMode, ObjectDatabase, ObjectType, Signature, Tree, TreeEntry,
};
use std::sync::Arc;
use tempfile::TempDir;

/// Helper to create test signature
fn test_signature(name: &str, email: &str) -> Signature {
    Signature::now(name.to_string(), email.to_string())
}

/// Test complete workflow: init → add → commit → branch → merge
///
/// Workflow steps:
/// 1. Initialize repository (main branch)
/// 2. Add files (create blobs)
/// 3. Create initial commit
/// 4. Create feature branch
/// 5. Make changes on feature branch
/// 6. Merge feature branch back to main
#[tokio::test]
async fn test_complete_workflow_init_to_merge() {
    // ========================================
    // SETUP: Initialize version control system
    // ========================================

    let temp_dir = TempDir::new().unwrap();
    let storage: Arc<dyn StorageBackend> = Arc::new(
        FilesystemBackend::new(temp_dir.path().to_path_buf())
            .await
            .unwrap(),
    );

    let odb = ObjectDatabase::new(Arc::clone(&storage), 1000);
    let branch_mgr = BranchManager::new(Arc::clone(&storage));

    // ========================================
    // STEP 1: Initialize repository (mediagit init)
    // ========================================

    // Create initial files
    let readme_content = b"# MediaGit E2E Test\n\nInitial commit";
    let readme_oid = odb
        .write(ObjectType::Blob, readme_content)
        .await
        .unwrap();

    let mut initial_tree = Tree::new();
    initial_tree.add_entry(TreeEntry::new(
        "README.md".to_string(),
        FileMode::Regular,
        readme_oid,
    ));

    let tree_oid = initial_tree.write(&odb).await.unwrap();

    // Create initial commit (mediagit commit -m "Initial commit")
    let author = test_signature("Alice Developer", "alice@example.com");
    let commit1 = Commit::new(
        tree_oid,
        author.clone(),
        author.clone(),
        "Initial commit\n\nAdded README".to_string(),
    );
    let commit1_oid = commit1.write(&odb).await.unwrap();

    // Initialize repository with main branch
    branch_mgr.initialize(Some(commit1_oid)).await.unwrap();

    // Verify initialization
    assert_eq!(
        branch_mgr.current_branch().await.unwrap().unwrap(),
        "main"
    );

    // ========================================
    // STEP 2: Create feature branch (mediagit branch feature-docs)
    // ========================================

    branch_mgr
        .create_branch("feature-docs", commit1_oid)
        .await
        .unwrap();

    // Switch to feature branch (mediagit checkout feature-docs)
    branch_mgr.checkout("feature-docs").await.unwrap();
    assert_eq!(
        branch_mgr.current_branch().await.unwrap().unwrap(),
        "feature-docs"
    );

    // ========================================
    // STEP 3: Make changes on feature branch (mediagit add + commit)
    // ========================================

    // Modify README
    let updated_readme = b"# MediaGit E2E Test\n\nInitial commit\n\n## Features\n- Version control for media";
    let updated_readme_oid = odb
        .write(ObjectType::Blob, updated_readme)
        .await
        .unwrap();

    // Add new file
    let docs_content = b"# Documentation\n\nGetting started guide";
    let docs_oid = odb.write(ObjectType::Blob, docs_content).await.unwrap();

    // Create updated tree
    let mut feature_tree = Tree::new();
    feature_tree.add_entry(TreeEntry::new(
        "README.md".to_string(),
        FileMode::Regular,
        updated_readme_oid,
    ));
    feature_tree.add_entry(TreeEntry::new(
        "DOCS.md".to_string(),
        FileMode::Regular,
        docs_oid,
    ));

    let feature_tree_oid = feature_tree.write(&odb).await.unwrap();

    // Create commit on feature branch
    let commit2 = Commit::new_with_parent(
        feature_tree_oid,
        commit1_oid,
        author.clone(),
        author.clone(),
        "Add documentation\n\nUpdated README and added DOCS.md".to_string(),
    );
    let commit2_oid = commit2.write(&odb).await.unwrap();

    // Update feature branch pointer
    branch_mgr.update_branch("feature-docs", commit2_oid).await.unwrap();

    // ========================================
    // STEP 4: Switch back to main (mediagit checkout main)
    // ========================================

    branch_mgr.checkout("main").await.unwrap();
    assert_eq!(
        branch_mgr.current_branch().await.unwrap().unwrap(),
        "main"
    );

    // ========================================
    // STEP 5: Merge feature branch (mediagit merge feature-docs)
    // ========================================

    // For this simple case (fast-forward merge), we just update main
    branch_mgr.update_branch("main", commit2_oid).await.unwrap();

    let main_info = branch_mgr.get_info("main").await.unwrap();
    assert_eq!(main_info.oid, commit2_oid);

    // ========================================
    // VERIFICATION: Validate final state
    // ========================================

    // Verify commit chain
    let final_commit = odb
        .read_commit(commit2_oid)
        .await
        .unwrap();
    assert_eq!(final_commit.parent(), Some(commit1_oid));
    assert_eq!(final_commit.message(), "Add documentation\n\nUpdated README and added DOCS.md");

    // Verify tree has both files
    let final_tree = odb.read_tree(feature_tree_oid).await.unwrap();
    assert_eq!(final_tree.entries().len(), 2);

    let entries: Vec<_> = final_tree.entries().iter().collect();
    assert!(entries.iter().any(|e| e.name == "README.md"));
    assert!(entries.iter().any(|e| e.name == "DOCS.md"));

    println!("✅ Complete workflow test passed: init → add → commit → branch → merge");
}

/// Test multi-step workflow with multiple commits
///
/// Workflow:
/// 1. Create initial repository with 2 files
/// 2. Make 3 sequential commits on main
/// 3. Create branch at commit 2
/// 4. Make changes on both main and branch
/// 5. Verify history and branch divergence
#[tokio::test]
async fn test_multi_commit_workflow() {
    let temp_dir = TempDir::new().unwrap();
    let storage: Arc<dyn StorageBackend> = Arc::new(
        FilesystemBackend::new(temp_dir.path().to_path_buf())
            .await
            .unwrap(),
    );

    let odb = ObjectDatabase::new(Arc::clone(&storage), 1000);
    let branch_mgr = BranchManager::new(Arc::clone(&storage));

    let author = test_signature("Bob Developer", "bob@example.com");

    // ========================================
    // Commit 1: Initial files
    // ========================================

    let file1 = odb.write(ObjectType::Blob, b"File 1 v1").await.unwrap();
    let file2 = odb.write(ObjectType::Blob, b"File 2 v1").await.unwrap();

    let mut tree1 = Tree::new();
    tree1.add_entry(TreeEntry::new("file1.txt".to_string(), FileMode::Regular, file1));
    tree1.add_entry(TreeEntry::new("file2.txt".to_string(), FileMode::Regular, file2));
    let tree1_oid = tree1.write(&odb).await.unwrap();

    let commit1 = Commit::new(tree1_oid, author.clone(), author.clone(), "Commit 1".to_string());
    let commit1_oid = commit1.write(&odb).await.unwrap();

    branch_mgr.initialize(Some(commit1_oid)).await.unwrap();

    // ========================================
    // Commit 2: Update file1
    // ========================================

    let file1_v2 = odb.write(ObjectType::Blob, b"File 1 v2").await.unwrap();

    let mut tree2 = Tree::new();
    tree2.add_entry(TreeEntry::new("file1.txt".to_string(), FileMode::Regular, file1_v2));
    tree2.add_entry(TreeEntry::new("file2.txt".to_string(), FileMode::Regular, file2));
    let tree2_oid = tree2.write(&odb).await.unwrap();

    let commit2 = Commit::new_with_parent(
        tree2_oid,
        commit1_oid,
        author.clone(),
        author.clone(),
        "Commit 2: Update file1".to_string(),
    );
    let commit2_oid = commit2.write(&odb).await.unwrap();

    branch_mgr.update_branch("main", commit2_oid).await.unwrap();

    // ========================================
    // Create feature branch at commit 2
    // ========================================

    branch_mgr.create_branch("feature", commit2_oid).await.unwrap();

    // ========================================
    // Commit 3 on main: Update file2
    // ========================================

    let file2_v2 = odb.write(ObjectType::Blob, b"File 2 v2 (main)").await.unwrap();

    let mut tree3 = Tree::new();
    tree3.add_entry(TreeEntry::new("file1.txt".to_string(), FileMode::Regular, file1_v2));
    tree3.add_entry(TreeEntry::new("file2.txt".to_string(), FileMode::Regular, file2_v2));
    let tree3_oid = tree3.write(&odb).await.unwrap();

    let commit3 = Commit::new_with_parent(
        tree3_oid,
        commit2_oid,
        author.clone(),
        author.clone(),
        "Commit 3: Update file2 on main".to_string(),
    );
    let commit3_oid = commit3.write(&odb).await.unwrap();

    branch_mgr.update_branch("main", commit3_oid).await.unwrap();

    // ========================================
    // Commit 4 on feature: Add file3
    // ========================================

    branch_mgr.checkout("feature").await.unwrap();

    let file3 = odb.write(ObjectType::Blob, b"File 3 (feature)").await.unwrap();

    let mut tree4 = Tree::new();
    tree4.add_entry(TreeEntry::new("file1.txt".to_string(), FileMode::Regular, file1_v2));
    tree4.add_entry(TreeEntry::new("file2.txt".to_string(), FileMode::Regular, file2));
    tree4.add_entry(TreeEntry::new("file3.txt".to_string(), FileMode::Regular, file3));
    let tree4_oid = tree4.write(&odb).await.unwrap();

    let commit4 = Commit::new_with_parent(
        tree4_oid,
        commit2_oid,
        author.clone(),
        author.clone(),
        "Commit 4: Add file3 on feature".to_string(),
    );
    let commit4_oid = commit4.write(&odb).await.unwrap();

    branch_mgr.update_branch("feature", commit4_oid).await.unwrap();

    // ========================================
    // VERIFICATION
    // ========================================

    // Verify main has 3 commits
    let main_commit = odb.read_commit(commit3_oid).await.unwrap();
    assert_eq!(main_commit.parent(), Some(commit2_oid));

    // Verify feature has diverged from main
    let feature_commit = odb.read_commit(commit4_oid).await.unwrap();
    assert_eq!(feature_commit.parent(), Some(commit2_oid));

    // Verify both branches share commit2 as common ancestor
    assert_eq!(main_commit.parent(), feature_commit.parent());

    // Verify main has 2 files
    let main_tree = odb.read_tree(tree3_oid).await.unwrap();
    assert_eq!(main_tree.entries().len(), 2);

    // Verify feature has 3 files
    let feature_tree = odb.read_tree(tree4_oid).await.unwrap();
    assert_eq!(feature_tree.entries().len(), 3);

    println!("✅ Multi-commit workflow test passed with branch divergence");
}

/// Test workflow with branch deletion
#[tokio::test]
async fn test_workflow_with_branch_deletion() {
    let temp_dir = TempDir::new().unwrap();
    let storage: Arc<dyn StorageBackend> = Arc::new(
        FilesystemBackend::new(temp_dir.path().to_path_buf())
            .await
            .unwrap(),
    );

    let odb = ObjectDatabase::new(Arc::clone(&storage), 1000);
    let branch_mgr = BranchManager::new(Arc::clone(&storage));

    let author = test_signature("Carol Developer", "carol@example.com");

    // Create initial commit
    let blob = odb.write(ObjectType::Blob, b"content").await.unwrap();
    let mut tree = Tree::new();
    tree.add_entry(TreeEntry::new("file.txt".to_string(), FileMode::Regular, blob));
    let tree_oid = tree.write(&odb).await.unwrap();

    let commit = Commit::new(tree_oid, author.clone(), author.clone(), "Initial".to_string());
    let commit_oid = commit.write(&odb).await.unwrap();

    branch_mgr.initialize(Some(commit_oid)).await.unwrap();

    // Create and delete temporary branch
    branch_mgr.create_branch("temp-branch", commit_oid).await.unwrap();

    let branches = branch_mgr.list_branches().await.unwrap();
    assert_eq!(branches.len(), 2); // main + temp-branch

    // Delete temporary branch
    branch_mgr.delete_branch("temp-branch").await.unwrap();

    let branches_after = branch_mgr.list_branches().await.unwrap();
    assert_eq!(branches_after.len(), 1); // only main

    println!("✅ Branch deletion workflow test passed");
}
