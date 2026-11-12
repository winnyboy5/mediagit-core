// SPDX-License-Identifier: BUSL-1.1
// Copyright (C) 2025-2026 Aswin Krishnamoorthy

//! Integration tests for FSCK (File System Check) functionality

use mediagit_storage::{LocalBackend, StorageBackend};
use mediagit_versioning::{
    Commit, FileMode, FsckChecker, FsckOptions, FsckRepair, IssueCategory, IssueSeverity,
    ObjectDatabase, ObjectType, Oid, Ref, RefDatabase, Signature, Tree, TreeEntry,
};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tempfile::TempDir;

/// Create a test repository with some objects
async fn setup_test_repo() -> (TempDir, Arc<LocalBackend>, ObjectDatabase) {
    let temp_dir = TempDir::new().unwrap();
    let storage = Arc::new(
        LocalBackend::new(temp_dir.path().to_str().unwrap())
            .await
            .unwrap(),
    );
    let odb = ObjectDatabase::new(storage.clone(), 100);

    (temp_dir, storage, odb)
}

#[tokio::test]
async fn test_fsck_clean_repository() {
    let (_temp_dir, storage, odb) = setup_test_repo().await;

    // Write some valid objects
    let blob1 = odb
        .write(ObjectType::Blob, b"test content 1")
        .await
        .unwrap();
    let _blob2 = odb
        .write(ObjectType::Blob, b"test content 2")
        .await
        .unwrap();
    let _blob3 = odb
        .write(ObjectType::Blob, b"test content 3")
        .await
        .unwrap();

    // Create a commit
    let commit = Commit::new(
        blob1, // Using blob1 as tree for simplicity
        Signature::now("Test".to_string(), "test@example.com".to_string()),
        Signature::now("Test".to_string(), "test@example.com".to_string()),
        "Test commit".to_string(),
    );
    let commit_data = mediagit_versioning::format::serialize(&commit).unwrap();
    let commit_oid = odb.write(ObjectType::Commit, &commit_data).await.unwrap();

    // Create a reference
    let main_ref = Ref::new_direct("refs/heads/main".to_string(), commit_oid);
    let ref_data = mediagit_versioning::format::serialize(&main_ref).unwrap();
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
async fn test_fsck_detect_corrupted_object() {
    let (_temp_dir, storage, odb) = setup_test_repo().await;

    // Write a valid object
    let valid_oid = odb.write(ObjectType::Blob, b"valid content").await.unwrap();

    // Corrupt the object by writing different content at the same location.
    // ODB reads/writes objects by the bare `oid.to_hex()` key (LocalBackend
    // applies "objects/" + shard-fanout internally when resolving the
    // physical path) — the logical StorageBackend key has neither.
    let corrupted_key = valid_oid.to_hex();
    storage
        .put(&corrupted_key, b"corrupted data")
        .await
        .unwrap();

    // Run FSCK
    let checker = FsckChecker::new(storage);
    let report = checker.check(FsckOptions::default()).await.unwrap();

    // Should detect checksum mismatch
    assert!(report.has_errors());
    assert!(report.corrupted_objects > 0);

    let errors = report.issues_by_severity(IssueSeverity::Error);
    assert!(!errors.is_empty());
    assert!(
        errors
            .iter()
            .any(|e| matches!(e.category, IssueCategory::ChecksumMismatch))
    );
}

#[tokio::test]
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
    let commit_data = mediagit_versioning::format::serialize(&commit).unwrap();
    let commit_oid = odb.write(ObjectType::Commit, &commit_data).await.unwrap();

    // Create a reference to the commit
    let main_ref = Ref::new_direct("refs/heads/main".to_string(), commit_oid);
    let ref_data = mediagit_versioning::format::serialize(&main_ref).unwrap();
    storage.put("refs/heads/main", &ref_data).await.unwrap();

    // Now delete the tree object that the commit references
    // (bare `oid.to_hex()` key — see test_fsck_detect_corrupted_object).
    let tree_key = tree.to_hex();
    storage.delete(&tree_key).await.unwrap();

    // Run FSCK with connectivity check
    let checker = FsckChecker::new(storage);
    let options = FsckOptions {
        check_connectivity: true,
        ..Default::default()
    };
    let report = checker.check(options).await.unwrap();

    // Should detect missing tree object
    assert!(report.has_errors());
    assert!(report.missing_objects > 0);

    let errors = report.issues_by_severity(IssueSeverity::Error);
    assert!(!errors.is_empty());
    assert!(
        errors
            .iter()
            .any(|e| matches!(e.category, IssueCategory::MissingObject))
    );
}

#[tokio::test]
async fn test_fsck_detect_broken_reference() {
    let (_temp_dir, storage, _odb) = setup_test_repo().await;

    // Create a reference pointing to a non-existent commit
    let fake_oid = Oid::hash(b"nonexistent");
    let broken_ref = Ref::new_direct("refs/heads/broken".to_string(), fake_oid);
    let ref_data = mediagit_versioning::format::serialize(&broken_ref).unwrap();
    storage.put("refs/heads/broken", &ref_data).await.unwrap();

    // Run FSCK
    let checker = FsckChecker::new(storage);
    let report = checker.check(FsckOptions::default()).await.unwrap();

    // Should detect broken reference
    assert!(report.has_errors());
    assert!(report.broken_refs > 0);

    let errors = report.issues_by_severity(IssueSeverity::Error);
    assert!(!errors.is_empty());
    assert!(
        errors
            .iter()
            .any(|e| matches!(e.category, IssueCategory::BrokenReference))
    );
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
    let commit_data = mediagit_versioning::format::serialize(&commit).unwrap();
    let commit_oid = odb.write(ObjectType::Commit, &commit_data).await.unwrap();

    // Create reference
    let main_ref = Ref::new_direct("refs/heads/main".to_string(), commit_oid);
    let ref_data = mediagit_versioning::format::serialize(&main_ref).unwrap();
    storage.put("refs/heads/main", &ref_data).await.unwrap();

    // Run full FSCK (includes dangling object detection)
    let checker = FsckChecker::new(storage);
    let report = checker.check(FsckOptions::full()).await.unwrap();

    // blob2 is dangling (not referenced by any commit)
    let info_issues = report.issues_by_severity(IssueSeverity::Info);
    assert!(
        info_issues.iter().any(|e| {
            matches!(e.category, IssueCategory::DanglingObject) && e.oid == Some(blob2)
        })
    );
}

/// A repository laid out like the CLI's: `<root>/.mediagit` holds the object
/// store and the refs, `<root>` is the working tree.
struct Layout {
    _tmp: TempDir,
    root: PathBuf,
    storage: Arc<LocalBackend>,
}

impl Layout {
    async fn new() -> Self {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path().to_path_buf();
        let storage = Arc::new(
            LocalBackend::new(root.join(".mediagit").to_str().unwrap())
                .await
                .unwrap(),
        );
        Layout {
            _tmp: tmp,
            root,
            storage,
        }
    }

    fn refdb(&self) -> RefDatabase {
        RefDatabase::new(self.root.join(".mediagit"))
    }

    fn checker(&self) -> FsckChecker {
        FsckChecker::new_with_refdb(self.storage.clone(), Some(self.refdb()))
            .with_repo_root(&self.root)
    }

    fn repair(&self, checker: &FsckChecker) -> FsckRepair {
        FsckRepair::new(self.storage.clone())
            .with_odb(checker.odb())
            .with_refdb(self.refdb())
            .with_repo_root(&self.root)
    }

    /// Commit one file `photo.bin` (also left in the working tree).
    async fn commit_photo(&self, data: &[u8]) -> Oid {
        let odb = ObjectDatabase::with_smart_compression(self.storage.clone(), 1000);
        let blob = odb
            .write_with_path(ObjectType::Blob, data, "photo.bin")
            .await
            .unwrap();
        let mut tree = Tree::new();
        tree.add_entry(TreeEntry::new(
            "photo.bin".to_string(),
            FileMode::Regular,
            blob,
        ));
        let tree_oid = tree.write(&odb).await.unwrap();
        let sig = Signature::now("T".to_string(), "t@example.com".to_string());
        let commit = Commit::new(tree_oid, sig.clone(), sig, "c".to_string());
        let commit_oid = commit.write(&odb).await.unwrap();
        self.refdb()
            .write(&Ref::new_direct("refs/heads/main".to_string(), commit_oid))
            .await
            .unwrap();
        std::fs::write(self.root.join("photo.bin"), data).unwrap();
        blob
    }

    /// Flip one byte of the stored loose object; returns the damaged bytes.
    async fn damage(&self, oid: Oid) -> Vec<u8> {
        let mut bytes = self.storage.get(&oid.to_hex()).await.unwrap();
        let mid = bytes.len() / 2;
        bytes[mid] ^= 0xFF;
        self.storage.put(&oid.to_hex(), &bytes).await.unwrap();
        bytes
    }
}

/// Every file under `dir` with its size.
fn snapshot(dir: &Path) -> BTreeMap<PathBuf, u64> {
    let mut out = BTreeMap::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return out;
    };
    for e in entries.flatten() {
        let p = e.path();
        if p.is_dir() {
            out.extend(snapshot(&p));
        } else {
            out.insert(p.clone(), std::fs::metadata(&p).unwrap().len());
        }
    }
    out
}

fn photo_bytes() -> Vec<u8> {
    (0..4096u32)
        .map(|i| (i.wrapping_mul(2_654_435_761) >> 13) as u8)
        .collect()
}

#[tokio::test]
async fn test_fsck_repair_broken_reference() {
    let repo = Layout::new().await;
    // Written through RefDatabase, as the real commands do: a raw
    // `storage.put` lands somewhere else and hid BUG-007.
    let fake_oid = Oid::hash(b"nonexistent");
    repo.refdb()
        .write(&Ref::new_direct("refs/heads/broken".to_string(), fake_oid))
        .await
        .unwrap();

    let checker = repo.checker();
    let report = checker.check(FsckOptions::default()).await.unwrap();
    assert!(report.has_errors());
    assert_eq!(report.repairable_issues().len(), 1);

    repo.repair(&checker).repair(&report, false).await.unwrap();

    assert!(
        !repo.refdb().exists("refs/heads/broken").await.unwrap(),
        "the broken ref must really be gone"
    );
    let again = repo.checker().check(FsckOptions::default()).await.unwrap();
    assert!(
        !again.has_errors(),
        "re-check must confirm: {:?}",
        again.issues
    );
}

#[tokio::test]
async fn test_fsck_repair_corrupt_blob_without_source_is_quarantined() {
    let repo = Layout::new().await;
    let blob = repo.commit_photo(&photo_bytes()).await;
    let damaged = repo.damage(blob).await;
    std::fs::remove_file(repo.root.join("photo.bin")).unwrap();

    let checker = repo.checker();
    let report = checker.check(FsckOptions::default()).await.unwrap();
    assert!(
        report
            .issues
            .iter()
            .any(|i| i.category == IssueCategory::ChecksumMismatch && i.oid == Some(blob)),
        "fixture: damage unseen: {:?}",
        report.issues
    );

    repo.repair(&checker).repair(&report, false).await.unwrap();

    let kept: Vec<_> = snapshot(&repo.root.join(".mediagit/quarantine"))
        .into_keys()
        .collect();
    assert_eq!(kept.len(), 1, "damaged bytes must be kept, got {kept:?}");
    assert_eq!(std::fs::read(&kept[0]).unwrap(), damaged);
    assert!(
        kept[0].to_string_lossy().contains(&blob.to_hex()),
        "quarantine entry must identify the object: {kept:?}"
    );
    assert!(!repo.storage.exists(&blob.to_hex()).await.unwrap());

    let again = repo.checker().check(FsckOptions::default()).await.unwrap();
    assert!(
        again
            .issues
            .iter()
            .any(|i| i.category == IssueCategory::MissingObject && i.oid == Some(blob)),
        "the object is gone, fsck must stay red: {:?}",
        again.issues
    );
}

#[tokio::test]
async fn test_fsck_repair_dry_run_changes_nothing_on_disk() {
    let repo = Layout::new().await;
    let blob = repo.commit_photo(&photo_bytes()).await;
    repo.damage(blob).await;
    repo.refdb()
        .write(&Ref::new_direct(
            "refs/heads/broken".to_string(),
            Oid::hash(b"nonexistent"),
        ))
        .await
        .unwrap();
    std::fs::remove_file(repo.root.join("photo.bin")).unwrap();

    let checker = repo.checker();
    let report = checker.check(FsckOptions::default()).await.unwrap();
    let before = snapshot(&repo.root);

    let plan = repo.repair(&checker).repair(&report, true).await.unwrap();

    assert_eq!(before, snapshot(&repo.root), "dry run touched the disk");
    assert!(plan.dry_run);

    // The real run does exactly what the plan said.
    let done = repo.repair(&checker).repair(&report, false).await.unwrap();
    assert_eq!(plan.quarantined, done.quarantined);
    assert_eq!(plan.restored, done.restored);
    assert_eq!(plan.refs_removed, done.refs_removed);
    assert_eq!(plan.unrestorable_files, done.unrestorable_files);
    assert_eq!(done.unrestorable_files, vec!["photo.bin".to_string()]);
}

#[tokio::test]
async fn test_fsck_repair_restores_blob_from_working_file() {
    let repo = Layout::new().await;
    let data = photo_bytes();
    let blob = repo.commit_photo(&data).await;
    repo.damage(blob).await;

    let checker = repo.checker();
    let report = checker.check(FsckOptions::default()).await.unwrap();
    repo.repair(&checker).repair(&report, false).await.unwrap();

    let again = repo.checker().check(FsckOptions::default()).await.unwrap();
    assert!(
        !again.has_errors(),
        "restored repo must be clean: {:?}",
        again.issues
    );
    let odb = ObjectDatabase::with_smart_compression(repo.storage.clone(), 10);
    assert_eq!(odb.read(&blob).await.unwrap(), data);
}

#[tokio::test]
async fn test_fsck_repair_never_removes_head() {
    let repo = Layout::new().await;
    // A fresh repository: HEAD names a branch that has no commit yet.
    repo.refdb()
        .write(&Ref::new_symbolic(
            "HEAD".to_string(),
            "refs/heads/main".to_string(),
        ))
        .await
        .unwrap();

    let checker = repo.checker();
    let report = checker.check(FsckOptions::default()).await.unwrap();
    repo.repair(&checker).repair(&report, false).await.unwrap();

    assert!(repo.refdb().exists("HEAD").await.unwrap());
}

#[tokio::test]
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
    let commit1_data = mediagit_versioning::format::serialize(&commit1).unwrap();
    let commit1_oid = odb.write(ObjectType::Commit, &commit1_data).await.unwrap();

    let commit2 = Commit::with_parents(
        tree,
        vec![commit1_oid],
        Signature::now("Test".to_string(), "test@example.com".to_string()),
        Signature::now("Test".to_string(), "test@example.com".to_string()),
        "Second commit".to_string(),
    );
    let commit2_data = mediagit_versioning::format::serialize(&commit2).unwrap();
    let commit2_oid = odb.write(ObjectType::Commit, &commit2_data).await.unwrap();

    // Create reference to commit2
    let main_ref = Ref::new_direct("refs/heads/main".to_string(), commit2_oid);
    let ref_data = mediagit_versioning::format::serialize(&main_ref).unwrap();
    storage.put("refs/heads/main", &ref_data).await.unwrap();

    // Run FSCK with connectivity check
    let checker = FsckChecker::new(storage);
    let options = FsckOptions {
        check_connectivity: true,
        ..Default::default()
    };

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
        odb.write(ObjectType::Blob, content.as_bytes())
            .await
            .unwrap();
    }

    // Run FSCK with max objects limit
    let checker = FsckChecker::new(storage);
    let options = FsckOptions {
        max_objects: 5,
        ..Default::default()
    };

    let report = checker.check(options).await.unwrap();

    // Should only check 5 objects
    assert_eq!(report.objects_checked, 5);
}

/// A crash in the middle of `repack` leaves an object both loose and packed.
/// `read` prefers the loose copy, so damage there hides an intact packed one.
#[tokio::test]
async fn test_fsck_repair_restores_damaged_loose_object_from_pack() {
    let repo = Layout::new().await;
    let data = photo_bytes();
    let blob = repo.commit_photo(&data).await;
    let odb = ObjectDatabase::with_smart_compression(repo.storage.clone(), 100);
    odb.repack(10_000, false).await.unwrap();
    assert!(
        repo.storage.exists(&blob.to_hex()).await.unwrap(),
        "fixture: loose copy must remain next to the pack"
    );
    let damaged = repo.damage(blob).await;
    std::fs::remove_file(repo.root.join("photo.bin")).unwrap();

    let checker = repo.checker();
    let report = checker.check(FsckOptions::default()).await.unwrap();
    assert!(
        report
            .issues
            .iter()
            .any(|i| i.category == IssueCategory::ChecksumMismatch && i.oid == Some(blob)),
        "fixture: damage unseen: {:?}",
        report.issues
    );

    let done = repo.repair(&checker).repair(&report, false).await.unwrap();

    assert_eq!(done.restored, vec![blob], "the pack holds a good copy");
    assert!(done.unrestorable_files.is_empty());
    let kept: Vec<_> = snapshot(&repo.root.join(".mediagit/quarantine"))
        .into_keys()
        .collect();
    assert_eq!(kept.len(), 1, "damaged loose bytes are kept, not deleted");
    assert_eq!(std::fs::read(&kept[0]).unwrap(), damaged);
    let again = repo.checker().check(FsckOptions::default()).await.unwrap();
    assert!(!again.has_errors(), "{:?}", again.issues);
}
