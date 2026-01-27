// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2025 MediaGit Contributors

//! Integration tests for mediagit-migration crate
//!
//! Tests the public API of the migration module including state management,
//! progress tracking, and integrity verification.

use mediagit_migration::{MigrationState, IntegrityVerifier};
use mediagit_storage::mock::MockBackend;
use mediagit_storage::StorageBackend;
use std::sync::Arc;
use tempfile::tempdir;

#[tokio::test]
async fn test_migration_state_create_and_persist() {
    let dir = tempdir().unwrap();
    let state_path = dir.path().join("migration_state.json");

    // Create a new migration state
    let mut state = MigrationState::new(
        "local".to_string(),
        "s3".to_string(),
        1000,
        serde_json::json!({"original": "config"}),
    );

    // Mark some objects as migrated
    state.mark_migrated("objects/abc123".to_string());
    state.mark_migrated("objects/def456".to_string());
    state.mark_migrated("objects/ghi789".to_string());

    // Save the state
    state.save(&state_path).await.unwrap();

    // Reload and verify
    let loaded = MigrationState::load(&state_path).await.unwrap();

    assert_eq!(loaded.source_backend, "local");
    assert_eq!(loaded.target_backend, "s3");
    assert_eq!(loaded.total_objects, 1000);
    assert_eq!(loaded.migrated_objects.len(), 3);
    assert!(loaded.is_migrated("objects/abc123"));
    assert!(loaded.is_migrated("objects/def456"));
    assert!(!loaded.is_migrated("objects/unknown"));
}

#[tokio::test]
async fn test_migration_state_progress_calculation() {
    let mut state = MigrationState::new(
        "local".to_string(),
        "gcs".to_string(),
        100,
        serde_json::json!({}),
    );

    // Initially 0% progress
    assert_eq!(state.progress(), 0.0);
    assert_eq!(state.remaining(), 100);

    // Migrate 25 objects
    for i in 0..25 {
        state.mark_migrated(format!("obj_{}", i));
    }

    assert_eq!(state.progress(), 0.25);
    assert_eq!(state.remaining(), 75);

    // Migrate to 100%
    for i in 25..100 {
        state.mark_migrated(format!("obj_{}", i));
    }

    assert_eq!(state.progress(), 1.0);
    assert_eq!(state.remaining(), 0);
}

#[tokio::test]
async fn test_migration_state_failure_tracking() {
    let mut state = MigrationState::new(
        "s3".to_string(),
        "local".to_string(),
        10,
        serde_json::json!({}),
    );

    // Mark some as migrated, some as failed
    state.mark_migrated("obj_1".to_string());
    state.mark_migrated("obj_2".to_string());
    state.mark_failed("obj_3".to_string(), "Network timeout".to_string());
    state.mark_failed("obj_4".to_string(), "Access denied".to_string());

    assert_eq!(state.migrated_objects.len(), 2);
    assert_eq!(state.failed_objects.len(), 2);
    assert!(!state.is_migrated("obj_3")); // Failed objects are not in migrated set
}

#[tokio::test]
async fn test_migration_state_delete() {
    let dir = tempdir().unwrap();
    let state_path = dir.path().join("state.json");

    let state = MigrationState::new(
        "local".to_string(),
        "s3".to_string(),
        10,
        serde_json::json!({}),
    );

    state.save(&state_path).await.unwrap();
    assert!(MigrationState::exists(&state_path).await);

    MigrationState::delete(&state_path).await.unwrap();
    assert!(!MigrationState::exists(&state_path).await);
}

#[tokio::test]
async fn test_integrity_verifier_success() {
    let source = MockBackend::new();
    let target = MockBackend::new();

    let test_data = b"test object data".to_vec();
    let key = "objects/test_object";

    // Put same data in both backends
    source.put(key, &test_data).await.unwrap();
    target.put(key, &test_data).await.unwrap();

    let verifier = IntegrityVerifier::new(
        Arc::new(source) as Arc<dyn StorageBackend>,
        Arc::new(target) as Arc<dyn StorageBackend>,
    );
    let result = verifier.verify_object(key).await.unwrap();

    assert!(result.passed, "Verification should pass for matching data");
    assert!(result.error.is_none());
}

#[tokio::test]
async fn test_integrity_verifier_checksum_mismatch() {
    let source = MockBackend::new();
    let target = MockBackend::new();

    let key = "objects/mismatched";

    // Put different data in each backend
    source.put(key, b"original data").await.unwrap();
    target.put(key, b"different data").await.unwrap();

    let verifier = IntegrityVerifier::new(
        Arc::new(source) as Arc<dyn StorageBackend>,
        Arc::new(target) as Arc<dyn StorageBackend>,
    );
    let result = verifier.verify_object(key).await.unwrap();

    assert!(!result.passed, "Verification should fail for mismatched data");
    assert!(result.error.is_some());
}

#[tokio::test]
async fn test_integrity_verifier_missing_object() {
    let source = MockBackend::new();
    let target = MockBackend::new();

    let key = "objects/only_in_source";

    // Only put in source
    source.put(key, b"test data").await.unwrap();

    let verifier = IntegrityVerifier::new(
        Arc::new(source) as Arc<dyn StorageBackend>,
        Arc::new(target) as Arc<dyn StorageBackend>,
    );
    let result = verifier.verify_object(key).await.unwrap();

    assert!(!result.passed, "Verification should fail for missing target object");
}

#[tokio::test]
async fn test_verifier_bulk_verification() {
    let source = MockBackend::new();
    let target = MockBackend::new();

    let keys: Vec<String> = (0..5).map(|i| format!("objects/obj_{}", i)).collect();

    // Put matching data for all objects
    for key in &keys {
        let data = format!("data for {}", key);
        source.put(key, data.as_bytes()).await.unwrap();
        target.put(key, data.as_bytes()).await.unwrap();
    }

    let verifier = IntegrityVerifier::new(
        Arc::new(source) as Arc<dyn StorageBackend>,
        Arc::new(target) as Arc<dyn StorageBackend>,
    );
    let results = verifier.verify_all(&keys).await.unwrap();

    assert_eq!(results.len(), 5);
    assert!(results.iter().all(|r| r.passed));
}
