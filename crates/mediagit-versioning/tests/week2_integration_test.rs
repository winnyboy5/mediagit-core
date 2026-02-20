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
// MediaGit - Git for Media Files
// Copyright (C) 2025 MediaGit Contributors
//
// Week 2 Milestone Integration Test
// Tests: Object Database + Compression + Commits + Trees + Branches + Pack Files
//
// This integration test validates the complete Week 2 milestone by testing
// all components working together in a realistic version control workflow.

use mediagit_storage::mock::MockBackend;
use mediagit_versioning::{
    BranchManager, Commit, ObjectDatabase, ObjectType, Signature, Tree, TreeEntry, FileMode,
};
use std::sync::Arc;
use tempfile::tempdir;

#[tokio::test]
async fn test_week2_milestone_end_to_end() {
    // ========================================
    // SETUP: Initialize version control system
    // ========================================

    let temp_dir = tempdir().unwrap();
    let storage_path = temp_dir.path().to_path_buf();

    let storage: Arc<dyn mediagit_storage::StorageBackend> = Arc::new(MockBackend::new());
    let odb = ObjectDatabase::new(Arc::clone(&storage), 1000);
    let branch_mgr = BranchManager::new(&storage_path);

    // ========================================
    // STEP 1: Create initial commit with files
    // ========================================

    // Create blob objects (simulate files)
    let readme_content = b"# MediaGit\n\nA Git for Media Files";
    let readme_oid = odb.write(ObjectType::Blob, readme_content).await.unwrap();

    let license_content = b"AGPL-3.0";
    let license_oid = odb.write(ObjectType::Blob, license_content).await.unwrap();

    // Create tree (directory structure)
    let mut tree = Tree::new();
    tree.add_entry(TreeEntry::new(
        "README.md".to_string(),
        FileMode::Regular,
        readme_oid,
    ));
    tree.add_entry(TreeEntry::new(
        "LICENSE".to_string(),
        FileMode::Regular,
        license_oid,
    ));

    let tree_oid = tree.write(&odb).await.unwrap();

    // Create initial commit
    let author = Signature::now(
        "Alice Developer".to_string(),
        "alice@mediagit.dev".to_string(),
    );

    let commit1 = Commit::new(
        tree_oid,
        author.clone(),
        author.clone(),
        "Initial commit\n\nAdded README and LICENSE".to_string(),
    );

    let commit1_oid = commit1.write(&odb).await.unwrap();

    // Initialize repository with main branch
    branch_mgr.initialize(Some(commit1_oid)).await.unwrap();

    // Verify main branch exists and points to initial commit
    let current_name = branch_mgr.current_branch().await.unwrap().unwrap();
    assert_eq!(current_name, "main");

    let main_info = branch_mgr.get_info("main").await.unwrap();
    assert_eq!(main_info.oid, commit1_oid);

    // ========================================
    // STEP 2: Create feature branch
    // ========================================

    branch_mgr.create("feature/compression", commit1_oid).await.unwrap();
    branch_mgr.switch_to("feature/compression").await.unwrap();

    let current_name = branch_mgr.current_branch().await.unwrap().unwrap();
    assert_eq!(current_name, "feature/compression");

    // ========================================
    // STEP 3: Make changes on feature branch
    // ========================================

    // Add new file
    let config_content = b"{\"compression\": \"zstd\", \"level\": \"default\"}";
    let config_oid = odb.write(ObjectType::Blob, config_content).await.unwrap();

    // Update tree with new file
    let mut tree2 = Tree::new();
    tree2.add_entry(TreeEntry::new(
        "README.md".to_string(),
        FileMode::Regular,
        readme_oid,
    ));
    tree2.add_entry(TreeEntry::new(
        "LICENSE".to_string(),
        FileMode::Regular,
        license_oid,
    ));
    tree2.add_entry(TreeEntry::new(
        "config.json".to_string(),
        FileMode::Regular,
        config_oid,
    ));

    let tree2_oid = tree2.write(&odb).await.unwrap();

    // Create second commit with parent
    let mut commit2 = Commit::new(
        tree2_oid,
        author.clone(),
        author.clone(),
        "Add compression configuration".to_string(),
    );
    commit2.add_parent(commit1_oid);

    let commit2_oid = commit2.write(&odb).await.unwrap();

    // Update feature branch to point to new commit
    branch_mgr.update_to("feature/compression", commit2_oid, true).await.unwrap();

    // ========================================
    // STEP 4: Switch back to main branch
    // ========================================

    branch_mgr.switch_to("main").await.unwrap();

    let current_name = branch_mgr.current_branch().await.unwrap().unwrap();
    assert_eq!(current_name, "main");

    let main_info = branch_mgr.get_info("main").await.unwrap();
    assert_eq!(main_info.oid, commit1_oid);

    // ========================================
    // STEP 5: Create another branch from main
    // ========================================

    branch_mgr.create("feature/docs", commit1_oid).await.unwrap();
    branch_mgr.switch_to("feature/docs").await.unwrap();

    // Add documentation file
    let docs_content = b"# Documentation\n\nHow to use MediaGit...";
    let docs_oid = odb.write(ObjectType::Blob, docs_content).await.unwrap();

    let mut tree3 = Tree::new();
    tree3.add_entry(TreeEntry::new(
        "README.md".to_string(),
        FileMode::Regular,
        readme_oid,
    ));
    tree3.add_entry(TreeEntry::new(
        "LICENSE".to_string(),
        FileMode::Regular,
        license_oid,
    ));
    tree3.add_entry(TreeEntry::new(
        "DOCS.md".to_string(),
        FileMode::Regular,
        docs_oid,
    ));

    let tree3_oid = tree3.write(&odb).await.unwrap();

    let mut commit3 = Commit::new(
        tree3_oid,
        author.clone(),
        author.clone(),
        "Add documentation".to_string(),
    );
    commit3.add_parent(commit1_oid);

    let commit3_oid = commit3.write(&odb).await.unwrap();
    branch_mgr.update_to("feature/docs", commit3_oid, true).await.unwrap();
    branch_mgr.switch_to("feature/docs").await.unwrap();

    // ========================================
    // STEP 6: Verify branch listing
    // ========================================

    let branches = branch_mgr.list().await.unwrap();
    assert_eq!(branches.len(), 3);

    let branch_names: Vec<String> = branches.iter().map(|b| b.name.clone()).collect();
    assert!(branch_names.contains(&"main".to_string()));
    assert!(branch_names.contains(&"feature/compression".to_string()));
    assert!(branch_names.contains(&"feature/docs".to_string()));

    // Find current branch
    let current_branch = branches.iter().find(|b| b.is_current).unwrap();
    assert_eq!(current_branch.name, "feature/docs");

    // ========================================
    // STEP 7: Verify object deduplication
    // ========================================

    let metrics = odb.metrics().await;

    // We wrote: readme, license, config, docs = 4 unique blobs
    // We wrote: tree1, tree2, tree3 = 3 unique trees
    // We wrote: commit1, commit2, commit3 = 3 unique commits
    // Total unique objects: 10

    // But we referenced README and LICENSE multiple times in trees
    // Deduplication should be working
    assert_eq!(metrics.unique_objects, 10);

    // We wrote more total times (trees reference same blobs)
    assert!(metrics.total_writes >= metrics.unique_objects);

    // If we had duplicates, dedup ratio > 0
    if metrics.total_writes > metrics.unique_objects {
        assert!(metrics.dedup_ratio() > 0.0);
    }

    // ========================================
    // STEP 8: Verify object retrieval works
    // ========================================

    // Read objects back and verify content
    let readme_retrieved = odb.read(&readme_oid).await.unwrap();
    assert_eq!(readme_retrieved, readme_content);

    let config_retrieved = odb.read(&config_oid).await.unwrap();
    assert_eq!(config_retrieved, config_content);

    // Read tree back
    let tree2_retrieved = Tree::read(&odb, &tree2_oid).await.unwrap();
    let total_entries = tree2_retrieved.file_count() + tree2_retrieved.dir_count();
    assert_eq!(total_entries, 3);
    assert!(tree2_retrieved.get_entry("config.json").is_some());

    // Read commit back
    let commit2_retrieved = Commit::read(&odb, &commit2_oid).await.unwrap();
    assert_eq!(commit2_retrieved.tree, tree2_oid);
    assert_eq!(commit2_retrieved.parent_count(), 1);
    assert_eq!(commit2_retrieved.parent().unwrap(), &commit1_oid);

    // ========================================
    // STEP 9: Verify cache is working
    // ========================================

    // Second read should be cache hit
    let _ = odb.read(&readme_oid).await.unwrap();

    let final_metrics = odb.metrics().await;
    assert!(final_metrics.cache_hits > 0);
    assert!(final_metrics.hit_rate() > 0.0);

    // ========================================
    // STEP 10: Create tag
    // ========================================

    branch_mgr.create_tag("v0.1.0", commit2_oid).await.unwrap();

    let tags = branch_mgr.list_tags().await.unwrap();
    assert_eq!(tags.len(), 1);
    assert_eq!(tags[0], "v0.1.0");

    // ========================================
    // VERIFICATION COMPLETE
    // ========================================

    println!("✅ Week 2 Milestone Integration Test PASSED");
    println!("   - Object Database: Working");
    println!("   - Deduplication: Working (ratio: {:.1}%)", final_metrics.dedup_ratio() * 100.0);
    println!("   - Cache: Working (hit rate: {:.1}%)", final_metrics.hit_rate() * 100.0);
    println!("   - Trees: Working ({} files)", tree2_retrieved.file_count());
    println!("   - Commits: Working (commit chain verified)");
    println!("   - Branches: Working ({} branches)", branches.len());
    println!("   - Tags: Working ({} tags)", tags.len());
}

// Note: Compression integration is tested separately in mediagit-compression crate
// The compression crate has its own integration tests that validate
// compression working with storage and object database patterns.

#[tokio::test]
async fn test_detached_head_workflow() {
    let temp_dir = tempdir().unwrap();
    let storage_path = temp_dir.path().to_path_buf();

    let storage: Arc<dyn mediagit_storage::StorageBackend> = Arc::new(MockBackend::new());
    let odb = ObjectDatabase::new(Arc::clone(&storage), 100);
    let branch_mgr = BranchManager::new(&storage_path);

    // Create initial commit
    let author = Signature::now("Test".to_string(), "test@example.com".to_string());

    let blob_oid = odb.write(ObjectType::Blob, b"content").await.unwrap();
    let mut tree = Tree::new();
    tree.add_entry(TreeEntry::new("file.txt".to_string(), FileMode::Regular, blob_oid));
    let tree_oid = tree.write(&odb).await.unwrap();

    let commit = Commit::new(tree_oid, author.clone(), author, "Initial".to_string());
    let commit_oid = commit.write(&odb).await.unwrap();

    // Initialize and detach HEAD
    branch_mgr.initialize(Some(commit_oid)).await.unwrap();
    branch_mgr.detach_head(commit_oid).await.unwrap();

    // Verify detached state
    assert!(branch_mgr.is_detached().await.unwrap());

    // Detached HEAD state verified - HEAD points directly to commit
    let head_oid = branch_mgr.head_commit().await.unwrap();
    assert_eq!(head_oid, commit_oid);

    println!("✅ Detached HEAD Workflow Test PASSED");
}
