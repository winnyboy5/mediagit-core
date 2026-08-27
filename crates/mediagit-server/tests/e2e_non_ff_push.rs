// SPDX-License-Identifier: BUSL-1.1
// Copyright (C) 2025-2026 Aswin Krishnamoorthy

//! Regression tests for BUG-V10-C6: server silently accepted non-fast-forward
//! pushes, allowing client A to overwrite commits from client B.
//!
//! Test 1: non-FF push without force must be rejected.
//! Test 2: force push must succeed AND write a reflog entry.

use std::path::PathBuf;
use std::sync::Arc;
use tempfile::TempDir;
use tokio::net::TcpListener;

use mediagit_protocol::{ProtocolClient, RefUpdate};
use mediagit_storage::{LocalBackend, StorageBackend};
use mediagit_versioning::{
    Commit, FileMode, ObjectDatabase, ObjectType, Oid, Ref, RefDatabase, Signature, Tree, TreeEntry,
};

// ---------------------------------------------------------------------------
// Helpers (mirrors e2e_push_pull_tags.rs)
// ---------------------------------------------------------------------------

async fn start_test_server(repos_dir: PathBuf) -> (String, tokio::task::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let base_url = format!("http://{}", addr);

    let state = Arc::new(mediagit_server::AppState::new(repos_dir.clone()));
    let app = mediagit_server::create_router(state);
    let handle = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
    (base_url, handle)
}

async fn open_odb(mediagit_dir: &std::path::Path) -> ObjectDatabase {
    let storage: Arc<dyn StorageBackend> = Arc::new(LocalBackend::new(mediagit_dir).await.unwrap());
    ObjectDatabase::new(storage, 100)
}

/// Create a commit with a single blob + tree, optionally on top of `parent`.
async fn commit_with_file(
    odb: &ObjectDatabase,
    content: &[u8],
    filename: &str,
    parent: Option<Oid>,
) -> (Oid, Oid, Oid) {
    let blob_oid = odb.write(ObjectType::Blob, content).await.unwrap();
    let mut tree = Tree::new();
    tree.add_entry(TreeEntry::new(
        filename.to_string(),
        FileMode::Regular,
        blob_oid,
    ));
    let tree_oid = tree.write(odb).await.unwrap();

    let author = Signature::now("Tester".to_string(), "t@test.io".to_string());
    let mut commit = Commit::new(tree_oid, author.clone(), author, "test commit".to_string());
    if let Some(p) = parent {
        commit.parents.push(p);
    }
    let commit_oid = commit.write(odb).await.unwrap();
    (commit_oid, tree_oid, blob_oid)
}

/// Copy a raw object from `src` ODB into `dst` ODB under the given type.
async fn mirror_object(src: &ObjectDatabase, dst: &ObjectDatabase, oid: Oid, obj_type: ObjectType) {
    let raw = src.read(&oid).await.unwrap();
    dst.write(obj_type, &raw).await.unwrap();
}

// ---------------------------------------------------------------------------
// Test 1: non-FF push without force must be rejected
// ---------------------------------------------------------------------------

/// Regression for BUG-V10-C6.
///
/// Topology:
///   A ← commit A (server seeded: A then B, tip = B)
///   └─ B ← server tip
///   A
///   └─ C ← client diverges here (parent = A, not B)
///
/// Client pushes C with force=false.
/// Expected: rejection with "non-fast-forward" in error; server tip still B.
#[tokio::test]
async fn e2e_push_rejects_non_ff_without_force() {
    // ── Server: seed A → B ───────────────────────────────────────────────────
    let server_temp = TempDir::new().unwrap();
    let server_repos = server_temp.path().join("repos");
    let server_repo = server_repos.join("nff-repo");
    let server_mediagit = server_repo.join(".mediagit");
    tokio::fs::create_dir_all(server_mediagit.join("objects"))
        .await
        .unwrap();
    tokio::fs::create_dir_all(server_mediagit.join("refs/heads"))
        .await
        .unwrap();

    let server_odb = open_odb(&server_mediagit).await;
    let (commit_a, tree_a, blob_a) =
        commit_with_file(&server_odb, b"content-a", "file.txt", None).await;
    let (commit_b, _, _) =
        commit_with_file(&server_odb, b"content-b", "file.txt", Some(commit_a)).await;

    let server_refdb = RefDatabase::new(&server_mediagit);
    server_refdb
        .write(&Ref::new_direct("refs/heads/main".to_string(), commit_b))
        .await
        .unwrap();

    let (base_url, _server_handle) = start_test_server(server_repos.clone()).await;

    // ── Client: knows A, creates divergent C ─────────────────────────────────
    let client_temp = TempDir::new().unwrap();
    let client_mediagit = client_temp.path().join(".mediagit");
    tokio::fs::create_dir_all(client_mediagit.join("objects"))
        .await
        .unwrap();

    let client_odb = open_odb(&client_mediagit).await;
    // Mirror A's objects into client ODB (simulates a prior fetch up to A)
    mirror_object(&server_odb, &client_odb, blob_a, ObjectType::Blob).await;
    mirror_object(&server_odb, &client_odb, tree_a, ObjectType::Tree).await;
    mirror_object(&server_odb, &client_odb, commit_a, ObjectType::Commit).await;

    // Client creates C with parent = A (diverges from B)
    let (commit_c, _, _) =
        commit_with_file(&client_odb, b"content-c", "file.txt", Some(commit_a)).await;

    // ── Push C → server, force=false ─────────────────────────────────────────
    let client = ProtocolClient::new(format!("{}/nff-repo", base_url));
    let (resp, _stats) = client
        .push(
            &client_odb,
            vec![RefUpdate {
                name: "refs/heads/main".to_string(),
                old_oid: Some(commit_a.to_hex()),
                new_oid: commit_c.to_hex(),
                delete: false,
            }],
            false, // force=false
        )
        .await
        .expect("push transport must succeed even if ref update is rejected");

    // ── Assertions ────────────────────────────────────────────────────────────
    assert!(
        !resp.success,
        "non-FF push without force must set success=false"
    );

    let ref_result = resp
        .results
        .iter()
        .find(|r| r.ref_name == "refs/heads/main")
        .expect("result for refs/heads/main must be present");
    assert!(!ref_result.success, "per-ref success must be false");

    let err_msg = ref_result
        .error
        .as_deref()
        .unwrap_or("")
        .to_ascii_lowercase();
    assert!(
        err_msg.contains("non-fast-forward"),
        "error must contain 'non-fast-forward', got: {:?}",
        ref_result.error
    );

    // Server ref must still point to B
    let server_refdb2 = RefDatabase::new(&server_mediagit);
    let server_ref = server_refdb2.read("refs/heads/main").await.unwrap();
    let server_tip = server_ref.oid.expect("server ref must have an OID");
    assert_eq!(
        server_tip, commit_b,
        "server tip must still be B after rejected non-FF push"
    );
}

// ---------------------------------------------------------------------------
// Test 2: force push must succeed AND write a reflog entry
// ---------------------------------------------------------------------------

/// Force push (force=true) must:
///  - succeed (server tip moves to C)
///  - write a reflog entry in <server_repo>/.mediagit/logs/refs/heads/main
///    with old_oid=B, new_oid=C on the same line
#[tokio::test]
async fn e2e_force_push_writes_reflog() {
    // ── Server: seed A → B ───────────────────────────────────────────────────
    let server_temp = TempDir::new().unwrap();
    let server_repos = server_temp.path().join("repos");
    let server_repo = server_repos.join("force-repo");
    let server_mediagit = server_repo.join(".mediagit");
    tokio::fs::create_dir_all(server_mediagit.join("objects"))
        .await
        .unwrap();
    tokio::fs::create_dir_all(server_mediagit.join("refs/heads"))
        .await
        .unwrap();

    let server_odb = open_odb(&server_mediagit).await;
    let (commit_a, tree_a, blob_a) =
        commit_with_file(&server_odb, b"content-a", "file.txt", None).await;
    let (commit_b, _, _) =
        commit_with_file(&server_odb, b"content-b", "file.txt", Some(commit_a)).await;

    let server_refdb = RefDatabase::new(&server_mediagit);
    server_refdb
        .write(&Ref::new_direct("refs/heads/main".to_string(), commit_b))
        .await
        .unwrap();

    let (base_url, _server_handle) = start_test_server(server_repos.clone()).await;

    // ── Client: divergent commit C ────────────────────────────────────────────
    let client_temp = TempDir::new().unwrap();
    let client_mediagit = client_temp.path().join(".mediagit");
    tokio::fs::create_dir_all(client_mediagit.join("objects"))
        .await
        .unwrap();

    let client_odb = open_odb(&client_mediagit).await;
    mirror_object(&server_odb, &client_odb, blob_a, ObjectType::Blob).await;
    mirror_object(&server_odb, &client_odb, tree_a, ObjectType::Tree).await;
    mirror_object(&server_odb, &client_odb, commit_a, ObjectType::Commit).await;

    let (commit_c, _, _) =
        commit_with_file(&client_odb, b"content-c", "file.txt", Some(commit_a)).await;

    // ── Force push ────────────────────────────────────────────────────────────
    let client = ProtocolClient::new(format!("{}/force-repo", base_url));
    let (resp, _stats) = client
        .push(
            &client_odb,
            vec![RefUpdate {
                name: "refs/heads/main".to_string(),
                old_oid: Some(commit_a.to_hex()),
                new_oid: commit_c.to_hex(),
                delete: false,
            }],
            true, // force=true
        )
        .await
        .expect("push transport");

    assert!(resp.success, "force push must succeed");
    let ref_result = resp
        .results
        .iter()
        .find(|r| r.ref_name == "refs/heads/main")
        .unwrap();
    assert!(ref_result.success, "per-ref result must be success");

    // ── Server tip must now be C ──────────────────────────────────────────────
    let server_refdb2 = RefDatabase::new(&server_mediagit);
    let server_ref = server_refdb2.read("refs/heads/main").await.unwrap();
    let server_tip = server_ref.oid.expect("server ref must have OID");
    assert_eq!(
        server_tip, commit_c,
        "server tip must be C after force push"
    );

    // ── Reflog must exist and contain B→C entry ───────────────────────────────
    let reflog_path = server_mediagit
        .join("logs")
        .join("refs")
        .join("heads")
        .join("main");
    assert!(
        reflog_path.exists(),
        "reflog file must exist at {:?}",
        reflog_path
    );

    let reflog_content = tokio::fs::read_to_string(&reflog_path)
        .await
        .expect("read reflog file");
    assert!(!reflog_content.is_empty(), "reflog must not be empty");

    // At least one line must have old=B new=C (first two space-separated tokens)
    let b_hex = commit_b.to_hex();
    let c_hex = commit_c.to_hex();
    let found = reflog_content.lines().any(|line| {
        let mut parts = line.split_whitespace();
        let old = parts.next().unwrap_or("");
        let new = parts.next().unwrap_or("");
        old == b_hex && new == c_hex
    });
    assert!(
        found,
        "reflog must contain entry old={} new={}; got:\n{}",
        b_hex, c_hex, reflog_content
    );
}
