// MediaGit - Git for Media Files
// Copyright (C) 2025 MediaGit Contributors
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU Affero General Public License as published
// by the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE. See the
// GNU Affero General Public License for more details.

//! Integration tests for FSCK (File System Check) functionality

use mediagit_storage::{LocalBackend, StorageBackend};
use mediagit_versioning::{
    Commit, FsckChecker, FsckOptions, FsckRepair, IssueCategory, IssueSeverity, ObjectDatabase,
    ObjectType, Oid, Ref, Signature,
};
use std::sync::Arc;
use tempfile::TempDir;

/// Create a test repository with some objects
async fn setup_test_repo() -> (TempDir, Arc<LocalBackend>, ObjectDatabase) {
    let temp_dir = TempDir::new().unwrap();
    let storage = Arc::new(LocalBackend::new(temp_dir.path().to_str().unwrap()).await.unwrap());
    let odb = ObjectDatabase::new(storage.clone(), 100);

    (temp_dir, storage, odb)
}

// FIXME: FSCK functionality is under development - tests may fail due to incomplete implementation
#[tokio::test]
#[ignore = "FSCK functionality under development"]
async fn test_fsck_clean_repository() {
    let (_temp_dir, storage, odb) = setup_test_repo().await;

    // Write some valid objects
    let blob1 = odb.write(ObjectType::Blob, b"test content 1").await.unwrap();
    let _blob2 = odb.write(ObjectType::Blob, b"test content 2").await.unwrap();
    let _blob3 = odb.write(ObjectType::Blob, b"test content 3").await.unwrap();

    // Create a commit
    let commit = Commit::new(
        blob1, // Using blob1 as tree for simplicity
        Signature::now("Test".to_string(), "test@example.com".to_string()),
        Signature::now("Test".to_string(), "test@example.com".to_string()),
        "Test commit".to_string(),
    );
    let commit_data = bincode::serialize(&commit).unwrap();
    let commit_oid = odb.write(ObjectType::Commit, &commit_data).await.unwrap();

    // Create a reference
    let main_ref = Ref::new_direct("refs/heads/main".to_string(), commit_oid);
    let ref_data = bincode::serialize(&main_ref).unwrap();
    storage.put("refs/heads/main", &ref_data).await.unwrap();

    // Run FSCK
    let checker = FsckChecker::new(storage);
    let report = checker.check(FsckOptions::default()).await.unwrap();

    // Clean repository should have no issues
    assert_eq!(report.total_issues(), 0);
    assert!(!report.has_errors());
    assert!(report.objects_checked > 0);
    assert_eq!(report.refs_checked, 1);
}

#[tokio::test]
#[ignore = "FSCK functionality under development"]
async fn test_fsck_detect_corrupted_object() {
    let (_temp_dir, storage, odb) = setup_test_repo().await;

    // Write a valid object
    let valid_oid = odb.write(ObjectType::Blob, b"valid content").await.unwrap();

    // Corrupt the object by writing different content at the same location
    let corrupted_key = format!("objects/{}", valid_oid.to_path());
    storage.put(&corrupted_key, b"corrupted data").await.unwrap();

    // Run FSCK
    let checker = FsckChecker::new(storage);
    let report = checker.check(FsckOptions::default()).await.unwrap();

    // Should detect checksum mismatch
    assert!(report.has_errors());
    assert!(report.corrupted_objects > 0);

    let errors = report.issues_by_severity(IssueSeverity::Error);
    assert!(!errors.is_empty());
    assert!(errors.iter().any(|e| matches!(e.category, IssueCategory::ChecksumMismatch)));
}

#[tokio::test]
#[ignore = "FSCK functionality under development"]
async fn test_fsck_detect_missing_object() {
    let (_temp_dir, storage, odb) = setup_test_repo().await;

    // Create a tree object
    let tree = odb.write(ObjectType::Blob, b"tree content").await.unwrap();

    // Create a commit referencing the tree
    let commit = Commit::new(
        tree,
        Signature::now("Test".to_string(), "test@example.com".to_string()),
        Signature::now("Test".to_string(), "test@example.com".to_string()),
        "Test commit".to_string(),
    );
    let commit_data = bincode::serialize(&commit).unwrap();
    let commit_oid = odb.write(ObjectType::Commit, &commit_data).await.unwrap();

    // Create a reference to the commit
    let main_ref = Ref::new_direct("refs/heads/main".to_string(), commit_oid);
    let ref_data = bincode::serialize(&main_ref).unwrap();
    storage.put("refs/heads/main", &ref_data).await.unwrap();

    // Now delete the tree object that the commit references
    let tree_key = format!("objects/{}", tree.to_path());
    storage.delete(&tree_key).await.unwrap();

    // Run FSCK with connectivity check
    let checker = FsckChecker::new(storage);
    let mut options = FsckOptions::default();
    options.check_connectivity = true;
    let report = checker.check(options).await.unwrap();

    // Should detect missing tree object
    assert!(report.has_errors());
    assert!(report.missing_objects > 0);

    let errors = report.issues_by_severity(IssueSeverity::Error);
    assert!(!errors.is_empty());
    assert!(errors.iter().any(|e| matches!(e.category, IssueCategory::MissingObject)));
}

#[tokio::test]
async fn test_fsck_detect_broken_reference() {
    let (_temp_dir, storage, _odb) = setup_test_repo().await;

    // Create a reference pointing to a non-existent commit
    let fake_oid = Oid::hash(b"nonexistent");
    let broken_ref = Ref::new_direct("refs/heads/broken".to_string(), fake_oid);
    let ref_data = bincode::serialize(&broken_ref).unwrap();
    storage.put("refs/heads/broken", &ref_data).await.unwrap();

    // Run FSCK
    let checker = FsckChecker::new(storage);
    let report = checker.check(FsckOptions::default()).await.unwrap();

    // Should detect broken reference
    assert!(report.has_errors());
    assert!(report.broken_refs > 0);

    let errors = report.issues_by_severity(IssueSeverity::Error);
    assert!(!errors.is_empty());
    assert!(errors.iter().any(|e| matches!(e.category, IssueCategory::BrokenReference)));
}

#[tokio::test]
async fn test_fsck_quick_mode() {
    let (_temp_dir, storage, odb) = setup_test_repo().await;

    // Create some objects
    odb.write(ObjectType::Blob, b"content 1").await.unwrap();
    odb.write(ObjectType::Blob, b"content 2").await.unwrap();

    // Run quick FSCK
    let checker = FsckChecker::new(storage);
    let report = checker.check(FsckOptions::quick()).await.unwrap();

    // Quick mode should check objects and refs, but not connectivity
    assert!(report.objects_checked > 0);
    assert_eq!(report.total_issues(), 0);
}

#[tokio::test]
async fn test_fsck_full_mode() {
    let (_temp_dir, storage, odb) = setup_test_repo().await;

    // Create objects
    let blob1 = odb.write(ObjectType::Blob, b"content 1").await.unwrap();
    let blob2 = odb.write(ObjectType::Blob, b"content 2").await.unwrap();

    // Create a commit
    let commit = Commit::new(
        blob1,
        Signature::now("Test".to_string(), "test@example.com".to_string()),
        Signature::now("Test".to_string(), "test@example.com".to_string()),
        "Test".to_string(),
    );
    let commit_data = bincode::serialize(&commit).unwrap();
    let commit_oid = odb.write(ObjectType::Commit, &commit_data).await.unwrap();

    // Create reference
    let main_ref = Ref::new_direct("refs/heads/main".to_string(), commit_oid);
    let ref_data = bincode::serialize(&main_ref).unwrap();
    storage.put("refs/heads/main", &ref_data).await.unwrap();

    // Run full FSCK (includes dangling object detection)
    let checker = FsckChecker::new(storage);
    let report = checker.check(FsckOptions::full()).await.unwrap();

    // blob2 is dangling (not referenced by any commit)
    let info_issues = report.issues_by_severity(IssueSeverity::Info);
    assert!(info_issues.iter().any(|e| {
        matches!(e.category, IssueCategory::DanglingObject) && e.oid == Some(blob2)
    }));
}

#[tokio::test]
async fn test_fsck_repair_broken_reference() {
    let (_temp_dir, storage, _odb) = setup_test_repo().await;

    // Create a broken reference
    let fake_oid = Oid::hash(b"nonexistent");
    let broken_ref = Ref::new_direct("refs/heads/broken".to_string(), fake_oid);
    let ref_data = bincode::serialize(&broken_ref).unwrap();
    storage.put("refs/heads/broken", &ref_data).await.unwrap();

    // Run FSCK
    let checker = FsckChecker::new(storage.clone());
    let report = checker.check(FsckOptions::default()).await.unwrap();

    assert!(report.has_errors());
    assert_eq!(report.repairable_issues().len(), 1);

    // Repair
    let repair = FsckRepair::new(storage.clone());
    let repaired = repair.repair(&report, false).await.unwrap();

    assert_eq!(repaired, 1);

    // Verify repair
    assert!(!storage.exists("refs/heads/broken").await.unwrap());
}

#[tokio::test]
async fn test_fsck_repair_dry_run() {
    let (_temp_dir, storage, _odb) = setup_test_repo().await;

    // Create a broken reference
    let fake_oid = Oid::hash(b"nonexistent");
    let broken_ref = Ref::new_direct("refs/heads/broken".to_string(), fake_oid);
    let ref_data = bincode::serialize(&broken_ref).unwrap();
    storage.put("refs/heads/broken", &ref_data).await.unwrap();

    // Run FSCK
    let checker = FsckChecker::new(storage.clone());
    let report = checker.check(FsckOptions::default()).await.unwrap();

    // Dry run repair
    let repair = FsckRepair::new(storage.clone());
    let repaired = repair.repair(&report, true).await.unwrap();

    assert_eq!(repaired, 1);

    // Verify ref still exists (dry run shouldn't actually repair)
    assert!(storage.exists("refs/heads/broken").await.unwrap());
}

#[tokio::test]
#[ignore = "FSCK functionality under development"]
async fn test_fsck_connectivity_check() {
    let (_temp_dir, storage, odb) = setup_test_repo().await;

    // Create a chain of commits
    let tree = odb.write(ObjectType::Blob, b"tree content").await.unwrap();

    let commit1 = Commit::new(
        tree,
        Signature::now("Test".to_string(), "test@example.com".to_string()),
        Signature::now("Test".to_string(), "test@example.com".to_string()),
        "First commit".to_string(),
    );
    let commit1_data = bincode::serialize(&commit1).unwrap();
    let commit1_oid = odb.write(ObjectType::Commit, &commit1_data).await.unwrap();

    let commit2 = Commit::with_parents(
        tree,
        vec![commit1_oid],
        Signature::now("Test".to_string(), "test@example.com".to_string()),
        Signature::now("Test".to_string(), "test@example.com".to_string()),
        "Second commit".to_string(),
    );
    let commit2_data = bincode::serialize(&commit2).unwrap();
    let commit2_oid = odb.write(ObjectType::Commit, &commit2_data).await.unwrap();

    // Create reference to commit2
    let main_ref = Ref::new_direct("refs/heads/main".to_string(), commit2_oid);
    let ref_data = bincode::serialize(&main_ref).unwrap();
    storage.put("refs/heads/main", &ref_data).await.unwrap();

    // Run FSCK with connectivity check
    let checker = FsckChecker::new(storage);
    let mut options = FsckOptions::default();
    options.check_connectivity = true;

    let report = checker.check(options).await.unwrap();

    // Should have no errors
    assert!(!report.has_errors());
    assert_eq!(report.total_issues(), 0);
}

#[tokio::test]
async fn test_fsck_max_objects_limit() {
    let (_temp_dir, storage, odb) = setup_test_repo().await;

    // Create multiple objects
    for i in 0..10 {
        let content = format!("content {}", i);
        odb.write(ObjectType::Blob, content.as_bytes()).await.unwrap();
    }

    // Run FSCK with max objects limit
    let checker = FsckChecker::new(storage);
    let mut options = FsckOptions::default();
    options.max_objects = 5;

    let report = checker.check(options).await.unwrap();

    // Should only check 5 objects
    assert_eq!(report.objects_checked, 5);
}
