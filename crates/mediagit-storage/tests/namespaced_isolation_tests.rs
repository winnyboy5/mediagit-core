// SPDX-License-Identifier: BUSL-1.1
// Copyright (C) 2025-2026 Aswin Krishnamoorthy

//! Layout v2 (M1) namespace-isolation tests: two repos, one shared storage
//! root/bucket, different `NamespacedBackend` namespaces. Write/list/delete
//! in repo A must never touch repo B's keys — this is the correctness
//! property the whole `NamespacedBackend` wrapper exists for.

use mediagit_storage::{NamespacedBackend, StorageBackend, local::LocalBackend};
use std::sync::Arc;
use tempfile::TempDir;

/// Two repos sharing one `LocalBackend` root (same physical directory tree,
/// different namespace prefixes).
#[tokio::test]
async fn local_backend_two_namespaces_never_collide() {
    let temp_dir = TempDir::new().unwrap();
    let shared_root: Arc<dyn StorageBackend> =
        Arc::new(LocalBackend::new(temp_dir.path()).await.unwrap());

    let repo_a = NamespacedBackend::new(shared_root.clone(), "repo-a").unwrap();
    let repo_b = NamespacedBackend::new(shared_root.clone(), "repo-b").unwrap();

    // Write the SAME logical key with different content in each namespace.
    repo_a.put("chunks/deadbeef", b"content-a").await.unwrap();
    repo_b.put("chunks/deadbeef", b"content-b").await.unwrap();
    repo_a
        .put("manifests/cafef00d", b"manifest-a")
        .await
        .unwrap();

    // Reads never cross the namespace boundary.
    assert_eq!(repo_a.get("chunks/deadbeef").await.unwrap(), b"content-a");
    assert_eq!(repo_b.get("chunks/deadbeef").await.unwrap(), b"content-b");

    // list_objects("") (what gc's orphan sweeps use) only ever sees the
    // calling repo's own keys.
    let mut a_keys = repo_a.list_objects("").await.unwrap();
    a_keys.sort();
    assert_eq!(a_keys, vec!["chunks/deadbeef", "manifests/cafef00d"]);

    let b_keys = repo_b.list_objects("").await.unwrap();
    assert_eq!(b_keys, vec!["chunks/deadbeef"]);

    // Deleting/gc'ing repo A's key must not affect repo B's key of the same
    // logical name.
    repo_a.delete("chunks/deadbeef").await.unwrap();
    assert!(!repo_a.exists("chunks/deadbeef").await.unwrap());
    assert!(repo_b.exists("chunks/deadbeef").await.unwrap());
    assert_eq!(repo_b.get("chunks/deadbeef").await.unwrap(), b"content-b");

    // And physically, the two namespaces landed in disjoint subtrees.
    assert!(temp_dir.path().join("repo-a").exists());
    assert!(temp_dir.path().join("repo-b").exists());
}

/// Same isolation property against a real MinIO bucket (one bucket, two
/// namespaces). Requires MinIO at localhost:9000 (minioadmin/minioadmin,
/// bucket "test-bucket" — see `minio_docker_tests.rs` prerequisites).
#[tokio::test]
#[ignore] // Requires MinIO running at localhost:9000 (see module docs)
async fn minio_two_namespaces_never_collide() {
    let inner: Arc<dyn StorageBackend> = Arc::new(
        mediagit_storage::minio::MinIOBackend::new(
            "http://localhost:9000",
            "test-bucket",
            "minioadmin",
            "minioadmin",
        )
        .await
        .expect("MinIO must be reachable at localhost:9000 for this test"),
    );

    let repo_a = NamespacedBackend::new(inner.clone(), "iso-test-repo-a").unwrap();
    let repo_b = NamespacedBackend::new(inner.clone(), "iso-test-repo-b").unwrap();

    // Clean slate for this test's keys (idempotent — ignore errors).
    let _ = repo_a.delete("chunks/isolation-test-key").await;
    let _ = repo_b.delete("chunks/isolation-test-key").await;

    repo_a
        .put("chunks/isolation-test-key", b"from-a")
        .await
        .unwrap();
    repo_b
        .put("chunks/isolation-test-key", b"from-b")
        .await
        .unwrap();

    assert_eq!(
        repo_a.get("chunks/isolation-test-key").await.unwrap(),
        b"from-a"
    );
    assert_eq!(
        repo_b.get("chunks/isolation-test-key").await.unwrap(),
        b"from-b"
    );

    let a_keys = repo_a.list_objects("chunks/").await.unwrap();
    assert!(a_keys.contains(&"chunks/isolation-test-key".to_string()));
    assert!(a_keys.iter().all(|k| !k.contains("iso-test-repo-b")));

    repo_a.delete("chunks/isolation-test-key").await.unwrap();
    assert!(!repo_a.exists("chunks/isolation-test-key").await.unwrap());
    assert!(repo_b.exists("chunks/isolation-test-key").await.unwrap());

    // Cleanup.
    let _ = repo_b.delete("chunks/isolation-test-key").await;
}

// ---------------------------------------------------------------------
// Namespace-collision guard (M2 Step 0, data-loss class): two
// *independently created* repos whose default namespace collides (same
// sanitized directory basename) against one shared storage root/bucket.
// Without a repo_id check, the second repo silently merges into the
// first's key space and gc's orphan sweep would delete the other's
// objects. `check_or_write_layout_marker` must hard-fail the second open.
// ---------------------------------------------------------------------

/// Simulates what `create_storage_backend` does at `init`/`clone` for two
/// separately-created repos that happen to share a directory basename
/// (e.g. `~/work/myrepo` and `~/scratch/myrepo`) pointed at the same
/// physical storage root: same `sanitize_namespace(basename)`, but distinct
/// `repo_id`s generated at each repo's own `init` time.
#[tokio::test]
async fn same_basename_repos_collide_on_shared_root() {
    let temp_dir = TempDir::new().unwrap();
    let shared_root: Arc<dyn StorageBackend> =
        Arc::new(LocalBackend::new(temp_dir.path()).await.unwrap());

    let ns = mediagit_storage::sanitize_namespace("myrepo");
    let repo_a = NamespacedBackend::new(shared_root.clone(), ns.clone()).unwrap();
    let repo_b = NamespacedBackend::new(shared_root.clone(), ns).unwrap();

    let repo_id_a = mediagit_storage::generate_repo_id();
    let repo_id_b = mediagit_storage::generate_repo_id();
    assert_ne!(repo_id_a, repo_id_b, "sanity: generated ids must differ");

    // First repo's `init` writes the marker for its own identity.
    mediagit_storage::check_or_write_layout_marker(&repo_a, 2u32, &repo_id_a)
        .await
        .unwrap();

    // Second, unrelated repo computes the SAME namespace and tries to open
    // against the same shared root — must hard-fail, not silently merge.
    let err = mediagit_storage::check_or_write_layout_marker(&repo_b, 2u32, &repo_id_b)
        .await
        .unwrap_err();
    let msg = err.to_string();
    assert!(msg.contains("namespace collision"), "got: {msg}");
    assert!(msg.contains(&repo_id_a));
    assert!(msg.contains(&repo_id_b));
}

/// The SAME repo re-opening its own storage (same repo_id) must keep working.
#[tokio::test]
async fn same_repo_reopening_is_fine() {
    let temp_dir = TempDir::new().unwrap();
    let shared_root: Arc<dyn StorageBackend> =
        Arc::new(LocalBackend::new(temp_dir.path()).await.unwrap());
    let ns = mediagit_storage::sanitize_namespace("myrepo");
    let repo_id = mediagit_storage::generate_repo_id();

    for _ in 0..3 {
        let backend = NamespacedBackend::new(shared_root.clone(), ns.clone()).unwrap();
        mediagit_storage::check_or_write_layout_marker(&backend, 2u32, &repo_id)
            .await
            .unwrap();
    }
}

/// Explicit distinct namespaces (the existing isolation fix) still work
/// even when both repos additionally go through the marker/repo_id check —
/// no false-positive collision when the namespaces genuinely differ.
#[tokio::test]
async fn explicit_distinct_namespaces_never_collide_at_marker() {
    let temp_dir = TempDir::new().unwrap();
    let shared_root: Arc<dyn StorageBackend> =
        Arc::new(LocalBackend::new(temp_dir.path()).await.unwrap());

    let repo_a = NamespacedBackend::new(shared_root.clone(), "repo-a").unwrap();
    let repo_b = NamespacedBackend::new(shared_root.clone(), "repo-b").unwrap();

    mediagit_storage::check_or_write_layout_marker(
        &repo_a,
        2u32,
        &mediagit_storage::generate_repo_id(),
    )
    .await
    .unwrap();
    mediagit_storage::check_or_write_layout_marker(
        &repo_b,
        2u32,
        &mediagit_storage::generate_repo_id(),
    )
    .await
    .unwrap();
}

/// MinIO variant of the collision repro: one bucket, two independently
/// "created" repos computing the same namespace. Requires MinIO at
/// localhost:9000 (see module docs for the other MinIO test).
#[tokio::test]
#[ignore] // Requires MinIO running at localhost:9000 (see module docs)
async fn minio_same_basename_repos_collide_on_shared_bucket() {
    let inner: Arc<dyn StorageBackend> = Arc::new(
        mediagit_storage::minio::MinIOBackend::new(
            "http://localhost:9000",
            "test-bucket",
            "minioadmin",
            "minioadmin",
        )
        .await
        .expect("MinIO must be reachable at localhost:9000 for this test"),
    );

    let ns = mediagit_storage::sanitize_namespace("collision-test-repo");
    let repo_a = NamespacedBackend::new(inner.clone(), ns.clone()).unwrap();
    let repo_b = NamespacedBackend::new(inner.clone(), ns).unwrap();

    // Clean slate: remove any marker left by a previous run.
    let _ = repo_a.delete(mediagit_storage::LAYOUT_MARKER_KEY).await;

    let repo_id_a = mediagit_storage::generate_repo_id();
    let repo_id_b = mediagit_storage::generate_repo_id();

    mediagit_storage::check_or_write_layout_marker(&repo_a, 2u32, &repo_id_a)
        .await
        .unwrap();

    let err = mediagit_storage::check_or_write_layout_marker(&repo_b, 2u32, &repo_id_b)
        .await
        .unwrap_err();
    assert!(err.to_string().contains("namespace collision"));

    // Cleanup.
    let _ = repo_a.delete(mediagit_storage::LAYOUT_MARKER_KEY).await;
}
