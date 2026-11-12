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
    // Its value is the only pointer to a lost commit: saved, not discarded.
    let saved = repo
        .root
        .join(".mediagit/quarantine/refs/refs/heads/broken");
    assert_eq!(
        std::fs::read_to_string(&saved).unwrap().trim(),
        fake_oid.to_hex(),
        "the removed ref's value must be kept in quarantine"
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
    // A repository with history whose HEAD names a branch that is gone.
    repo.commit_photo(&photo_bytes()).await;
    repo.refdb()
        .write(&Ref::new_symbolic(
            "HEAD".to_string(),
            "refs/heads/gone".to_string(),
        ))
        .await
        .unwrap();

    let checker = repo.checker();
    let report = checker.check(FsckOptions::default()).await.unwrap();
    assert!(
        report
            .issues
            .iter()
            .any(|i| i.category == IssueCategory::BrokenReference && i.repairable),
        "fixture: this HEAD must be reported: {:?}",
        report.issues
    );
    repo.repair(&checker).repair(&report, false).await.unwrap();

    assert!(repo.refdb().exists("HEAD").await.unwrap());
}

/// A fresh repository has a HEAD that names a branch with no commit yet.
/// That is healthy, not a broken reference (IMP-011).
#[tokio::test]
async fn test_fsck_unborn_head_in_empty_repository_is_healthy() {
    let repo = Layout::new().await;
    repo.refdb()
        .write(&Ref::new_symbolic(
            "HEAD".to_string(),
            "refs/heads/main".to_string(),
        ))
        .await
        .unwrap();
    for options in [FsckOptions::default(), FsckOptions::quick()] {
        let report = repo.checker().check(options).await.unwrap();
        assert_eq!(report.total_issues(), 0, "got: {:?}", report.issues);
    }
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

/// Wraps the real store and injects faults / records reads.
#[derive(Debug)]
struct Probe {
    inner: Arc<LocalBackend>,
    fail_exists: std::sync::Mutex<Option<String>>,
    fail_delete_nth: std::sync::Mutex<Option<usize>>,
    deletes: std::sync::atomic::AtomicUsize,
    gets: std::sync::Mutex<Vec<String>>,
}

impl Probe {
    fn new(inner: Arc<LocalBackend>) -> Arc<Self> {
        Arc::new(Probe {
            inner,
            fail_exists: Default::default(),
            fail_delete_nth: Default::default(),
            deletes: Default::default(),
            gets: Default::default(),
        })
    }
}

#[async_trait::async_trait]
impl StorageBackend for Probe {
    async fn get(&self, key: &str) -> anyhow::Result<Vec<u8>> {
        self.gets.lock().unwrap().push(key.to_string());
        self.inner.get(key).await
    }
    async fn put(&self, key: &str, data: &[u8]) -> anyhow::Result<()> {
        self.inner.put(key, data).await
    }
    async fn exists(&self, key: &str) -> anyhow::Result<bool> {
        if self.fail_exists.lock().unwrap().as_deref() == Some(key) {
            anyhow::bail!("injected I/O error probing {key}");
        }
        self.inner.exists(key).await
    }
    async fn delete(&self, key: &str) -> anyhow::Result<()> {
        let n = self
            .deletes
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst)
            + 1;
        if *self.fail_delete_nth.lock().unwrap() == Some(n) {
            anyhow::bail!("injected failure on delete #{n}");
        }
        self.inner.delete(key).await
    }
    async fn list_objects(&self, prefix: &str) -> anyhow::Result<Vec<String>> {
        self.inner.list_objects(prefix).await
    }
    async fn head(&self, key: &str) -> anyhow::Result<Option<u64>> {
        self.inner.head(key).await
    }
}

impl Layout {
    fn probe_checker(&self, probe: &Arc<Probe>) -> FsckChecker {
        FsckChecker::new_with_refdb(probe.clone(), Some(self.refdb())).with_repo_root(&self.root)
    }

    fn probe_repair(&self, probe: &Arc<Probe>, checker: &FsckChecker) -> FsckRepair {
        FsckRepair::new(probe.clone())
            .with_odb(checker.odb())
            .with_refdb(self.refdb())
            .with_repo_root(&self.root)
    }

    /// Commit `entries` as the tree of one commit on refs/heads/main.
    async fn commit_entries(&self, odb: &ObjectDatabase, entries: &[(&str, Oid)]) -> Oid {
        let mut tree = Tree::new();
        for (name, blob) in entries {
            tree.add_entry(TreeEntry::new(name.to_string(), FileMode::Regular, *blob));
        }
        let tree_oid = tree.write(odb).await.unwrap();
        let sig = Signature::now("T".to_string(), "t@example.com".to_string());
        let commit = Commit::new(tree_oid, sig.clone(), sig, "c".to_string());
        let commit_oid = commit.write(odb).await.unwrap();
        self.refdb()
            .write(&Ref::new_direct("refs/heads/main".to_string(), commit_oid))
            .await
            .unwrap();
        commit_oid
    }
}

/// A read/probe error is not "missing": it must never make a good branch
/// repairable (CRITICAL 1).
#[tokio::test]
async fn test_fsck_probe_error_is_not_a_missing_commit() {
    let repo = Layout::new().await;
    let blob = repo.commit_photo(&photo_bytes()).await;
    let head = repo
        .refdb()
        .read("refs/heads/main")
        .await
        .unwrap()
        .oid
        .unwrap();
    let probe = Probe::new(repo.storage.clone());
    *probe.fail_exists.lock().unwrap() = Some(head.to_hex());

    let checker = repo.probe_checker(&probe);
    // No object pass: it would read (and cache) the commit before the ref
    // check probes it.
    let report = checker
        .check(FsckOptions {
            check_objects: false,
            ..FsckOptions::default()
        })
        .await
        .unwrap();

    assert!(report.has_errors(), "an unreadable commit is an error");
    assert!(
        report.repairable_issues().is_empty(),
        "an I/O error must not be repairable: {:?}",
        report.issues
    );
    repo.probe_repair(&probe, &checker)
        .repair(&report, false)
        .await
        .unwrap();
    assert!(repo.refdb().exists("refs/heads/main").await.unwrap());
    let _ = blob;
}

/// Even from a stale report that lists the ref as broken, repair re-probes
/// the target and leaves a ref whose commit exists (CRITICAL 1b).
#[tokio::test]
async fn test_fsck_repair_reprobes_before_removing_a_ref() {
    let repo = Layout::new().await;
    repo.commit_photo(&photo_bytes()).await;
    let mut report = mediagit_versioning::FsckReport::new();
    report.add_issue(
        mediagit_versioning::FsckIssue::new(
            IssueSeverity::Error,
            IssueCategory::BrokenReference,
            "stale".to_string(),
        )
        .with_ref("refs/heads/main".to_string())
        .repairable(),
    );
    let checker = repo.checker();
    let done = repo.repair(&checker).repair(&report, false).await.unwrap();
    assert!(repo.refdb().exists("refs/heads/main").await.unwrap());
    assert!(done.refs_removed.is_empty());
}

/// Lazy clones write tracking refs for branches whose objects were never
/// downloaded. Repair never touches `refs/remotes/` (CRITICAL 2).
#[tokio::test]
async fn test_fsck_repair_never_removes_remote_tracking_refs() {
    let repo = Layout::new().await;
    repo.refdb()
        .write(&Ref::new_direct(
            "refs/remotes/origin/feature".to_string(),
            Oid::hash(b"lazy, not downloaded"),
        ))
        .await
        .unwrap();

    let checker = repo.checker();
    let report = checker.check(FsckOptions::default()).await.unwrap();
    let issue = report
        .issues
        .iter()
        .find(|i| i.ref_name.as_deref() == Some("refs/remotes/origin/feature"))
        .expect("still reported");
    assert!(!issue.repairable, "{issue:?}");
    assert!(issue.message.contains("fetch"), "hint: {}", issue.message);

    let done = repo.repair(&checker).repair(&report, false).await.unwrap();
    assert!(
        repo.refdb()
            .exists("refs/remotes/origin/feature")
            .await
            .unwrap()
    );
    assert!(done.refs_removed.is_empty());

    // Even a hand-built report cannot make repair remove it.
    let mut forced = mediagit_versioning::FsckReport::new();
    forced.add_issue(
        mediagit_versioning::FsckIssue::new(
            IssueSeverity::Error,
            IssueCategory::BrokenReference,
            "forced".to_string(),
        )
        .with_ref("refs/remotes/origin/feature".to_string())
        .repairable(),
    );
    let done = repo.repair(&checker).repair(&forced, false).await.unwrap();
    assert!(
        repo.refdb()
            .exists("refs/remotes/origin/feature")
            .await
            .unwrap()
    );
    assert!(done.refs_removed.is_empty());
}

/// A delta-stored blob is found by a cheap probe; the walk must not
/// reconstruct it (IMPORTANT 5).
#[tokio::test]
async fn test_fsck_walk_does_not_read_delta_stored_blobs() {
    let repo = Layout::new().await;
    let odb = ObjectDatabase::new(repo.storage.clone(), 100);
    let mut base = vec![b'a'; 10_000];
    base.extend_from_slice(b"UNIQUE_TAIL_BASE");
    let base_oid = odb.write(ObjectType::Blob, &base).await.unwrap();
    let mut target = vec![b'a'; 10_000];
    target.extend_from_slice(b"UNIQUE_TAIL_TARG");
    let delta_oid = odb
        .write_delta_against_base(ObjectType::Blob, &target, "f.bin", base_oid)
        .await
        .unwrap()
        .expect("fixture: delta accepted");
    repo.commit_entries(&odb, &[("f.bin", delta_oid)]).await;
    let probe = Probe::new(repo.storage.clone());

    let report = repo
        .probe_checker(&probe)
        .check(FsckOptions {
            check_objects: false,
            check_refs: false,
            check_dangling: false,
            check_chunk_deltas: false,
            check_connectivity: true,
            ..FsckOptions::default()
        })
        .await
        .unwrap();

    assert!(!report.has_errors(), "{:?}", report.issues);
    let gets = probe.gets.lock().unwrap();
    assert!(
        !gets
            .iter()
            .any(|k| k.contains(&delta_oid.to_hex()) || k.contains(&base_oid.to_hex())),
        "the walk reconstructed a delta chain: {gets:?}"
    );
}

/// A saved ref is dropped once a ref of that name exists again, and kept
/// while it does not.
#[tokio::test]
async fn test_fsck_sweep_drops_saved_ref_when_the_ref_is_back() {
    let repo = Layout::new().await;
    let fake = Oid::hash(b"nonexistent");
    repo.refdb()
        .write(&Ref::new_direct("refs/heads/broken".to_string(), fake))
        .await
        .unwrap();
    let checker = repo.checker();
    let report = checker.check(FsckOptions::default()).await.unwrap();
    repo.repair(&checker).repair(&report, false).await.unwrap();
    let saved = repo
        .root
        .join(".mediagit/quarantine/refs/refs/heads/broken");
    assert!(saved.exists());

    let odb = ObjectDatabase::with_smart_compression(repo.storage.clone(), 10);
    let refdb = repo.refdb();
    let st = mediagit_versioning::sweep_quarantine(&odb, Some(&refdb), &repo.root, true)
        .await
        .unwrap();
    assert!(saved.exists(), "ref is still gone: keep the saved value");
    assert_eq!((st.removed, st.count), (0, 1));

    refdb
        .write(&Ref::new_direct("refs/heads/broken".to_string(), fake))
        .await
        .unwrap();
    let st = mediagit_versioning::sweep_quarantine(&odb, Some(&refdb), &repo.root, true)
        .await
        .unwrap();
    assert!(!saved.exists(), "ref is back: the saved copy goes");
    assert_eq!((st.removed, st.count), (1, 0));
}

/// The report is older than the repair: an object that reads fine now must
/// not be quarantined (IMPORTANT 3).
#[tokio::test]
async fn test_fsck_repair_skips_an_object_that_is_healthy_now() {
    let repo = Layout::new().await;
    let blob = repo.commit_photo(&photo_bytes()).await;
    let mut report = mediagit_versioning::FsckReport::new();
    report.add_issue(
        mediagit_versioning::FsckIssue::new(
            IssueSeverity::Error,
            IssueCategory::ChecksumMismatch,
            "stale".to_string(),
        )
        .with_oid(blob)
        .repairable(),
    );
    let before = snapshot(&repo.root);

    let checker = repo.checker();
    let done = repo.repair(&checker).repair(&report, false).await.unwrap();

    assert!(done.restored.is_empty() && done.quarantined.is_empty());
    assert!(done.unrestorable_files.is_empty());
    assert_eq!(before, snapshot(&repo.root), "a healthy object was touched");
}

/// The second of two pieces cannot be moved: the outcome says so; the object
/// is neither "restored" nor "quarantined", and no byte is lost (IMPORTANT 4).
#[tokio::test]
async fn test_fsck_repair_reports_a_partly_failed_quarantine() {
    let repo = Layout::new().await;
    let odb = ObjectDatabase::with_smart_compression(repo.storage.clone(), 10);
    let data = photo_bytes();
    let chunk = Oid::hash(&data);
    odb.write_full_chunk(&chunk, &data).await.unwrap();
    let chunk_key = format!("chunks/{}", chunk.to_hex());
    let meta_key = format!("chunk-deltas/{}.meta", chunk.to_hex());
    repo.storage
        .put(&chunk_key, b"damaged chunk")
        .await
        .unwrap();
    repo.storage
        .put(&meta_key, b"damaged sidecar")
        .await
        .unwrap();

    let mut report = mediagit_versioning::FsckReport::new();
    report.add_issue(
        mediagit_versioning::FsckIssue::new(
            IssueSeverity::Error,
            IssueCategory::ChecksumMismatch,
            "damaged".to_string(),
        )
        .with_oid(chunk)
        .repairable(),
    );
    let probe = Probe::new(repo.storage.clone());
    *probe.fail_delete_nth.lock().unwrap() = Some(2);
    let checker = repo.probe_checker(&probe);

    let done = repo
        .probe_repair(&probe, &checker)
        .repair(&report, false)
        .await
        .unwrap();

    assert_eq!(done.errors.len(), 1, "{:?}", done.errors);
    assert!(done.errors[0].contains(&chunk.to_hex()));
    assert!(done.restored.is_empty() && done.quarantined.is_empty());
    // Nothing was lost: piece 1 is in quarantine, piece 2 is still in the store
    // and also copied to quarantine before the failed delete.
    assert!(repo.storage.exists(&meta_key).await.unwrap());
    let kept = snapshot(&repo.root.join(".mediagit/quarantine"));
    assert_eq!(kept.len(), 2, "{kept:?}");
}
