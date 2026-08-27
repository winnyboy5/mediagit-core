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

//! End-to-end tests for the M3 bitmap-accelerated have-closure negotiation.
//!
//! `download_pack`'s have-closure computation short-circuits via a commit's
//! `bitmaps/<oid>.bitmap` artifact when one exists (see
//! `mediagit-server/src/handlers/repo.rs` and `mediagit_versioning::bitmap`).
//! These tests seed a bitmap directly (bypassing the async post-receive
//! generation hook, which would otherwise race the test) and verify:
//! (1) the short-circuit actually fires (`AppState::bitmap_hits` counter),
//! (2) the resulting pack contents are byte-for-byte identical to a run with
//!     `MEDIAGIT_BITMAP=0` (bitmaps disabled, forcing the BFS path) — the
//!     bitmap must never change *what* gets shipped, only how it's computed.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;
use tempfile::TempDir;
use tokio::net::TcpListener;

use mediagit_protocol::ProtocolClient;
use mediagit_storage::{LocalBackend, StorageBackend};
use mediagit_versioning::{
    Commit, FileMode, ObjectDatabase, ObjectType, Oid, ReachabilityBitmap, Ref, RefDatabase,
    Signature, Tree, TreeEntry, bitmap_key,
};

/// `MEDIAGIT_BITMAP` is a process-global env var, but `cargo test` runs the
/// `#[tokio::test]` functions in this file concurrently on separate OS
/// threads within one process. Any test that reads (`bitmap_enabled()`,
/// exercised indirectly via the server's post-receive hook) or mutates that
/// var must hold this lock for the duration, or a concurrent mutation in one
/// test can flip the knob underneath another.
static BITMAP_ENV_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

async fn start_test_server(
    repos_dir: PathBuf,
) -> (
    String,
    Arc<mediagit_server::AppState>,
    tokio::task::JoinHandle<()>,
) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let base_url = format!("http://{}", addr);

    let state = Arc::new(mediagit_server::AppState::new(repos_dir.clone()));
    let app = mediagit_server::create_router(Arc::clone(&state));
    let handle = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

    (base_url, state, handle)
}

async fn open_odb(mediagit_dir: &std::path::Path) -> ObjectDatabase {
    let storage: Arc<dyn StorageBackend> = Arc::new(LocalBackend::new(mediagit_dir).await.unwrap());
    ObjectDatabase::new(storage, 100)
}

/// Server-side storage, wrapped exactly as production does — see
/// `e2e_incremental_fetch.rs::open_server_odb` for the rationale.
async fn open_server_odb(repo_root: &std::path::Path) -> (ObjectDatabase, Arc<dyn StorageBackend>) {
    let mediagit_dir = repo_root.join(".mediagit");
    let inner: Arc<dyn StorageBackend> = Arc::new(LocalBackend::new(&mediagit_dir).await.unwrap());
    let ns = mediagit_storage::sanitize_namespace(
        &repo_root
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default(),
    );
    let namespaced = mediagit_storage::NamespacedBackend::new(inner, ns).unwrap();
    let mut config = mediagit_config::Config::load(repo_root).await.unwrap();
    let repo_id = match &config.repo_id {
        Some(id) if !id.trim().is_empty() => id.clone(),
        _ => {
            let id = mediagit_storage::generate_repo_id();
            config.repo_id = Some(id.clone());
            config.save(repo_root).unwrap();
            id
        }
    };
    mediagit_storage::check_or_write_layout_marker(
        &namespaced,
        mediagit_config::CURRENT_LAYOUT_VERSION,
        &repo_id,
    )
    .await
    .unwrap();
    let storage: Arc<dyn StorageBackend> = Arc::new(namespaced);
    (ObjectDatabase::new(Arc::clone(&storage), 100), storage)
}

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

    let author = Signature::now("Test".to_string(), "t@e".to_string());
    let mut commit = Commit::new(tree_oid, author.clone(), author, "msg".to_string());
    if let Some(p) = parent {
        commit.parents.push(p);
    }
    let commit_oid = commit.write(odb).await.unwrap();
    (commit_oid, tree_oid, blob_oid)
}

/// End-to-end: a bitmap seeded for the `have` tip must (a) make the server's
/// `bitmap_hits` counter fire and (b) produce a pack identical to the
/// BFS-only (`MEDIAGIT_BITMAP=0`) path.
#[tokio::test]
async fn bitmap_short_circuit_matches_bfs_disabled_run() {
    // ---------- server repo: C1 (base) -> C2 (child) ----------
    let server_temp = TempDir::new().unwrap();
    let server_repos = server_temp.path().join("repos");
    let server_repo = server_repos.join("bitmap-repo");
    let server_mediagit = server_repo.join(".mediagit");
    tokio::fs::create_dir_all(server_mediagit.join("objects"))
        .await
        .unwrap();
    tokio::fs::create_dir_all(server_mediagit.join("refs/heads"))
        .await
        .unwrap();

    let (server_odb, server_storage) = open_server_odb(&server_repo).await;
    let (c1, t1, b1) = commit_with_file(&server_odb, b"v1", "a.txt", None).await;
    let (c2, t2, b2) = commit_with_file(&server_odb, b"v2", "b.txt", Some(c1)).await;

    RefDatabase::new(server_mediagit.clone())
        .write(&Ref::new_direct("refs/heads/main".to_string(), c2))
        .await
        .unwrap();

    // Seed the bitmap for the have-tip (C1) directly — this is what the
    // async post-receive hook (repo.rs update_refs) and `gc` would produce;
    // seeding it here avoids racing the background task in a test.
    let bitmap = ReachabilityBitmap::generate(&server_odb, c1).await.unwrap();
    server_storage
        .put(&bitmap_key(&c1), &bitmap.serialize().unwrap())
        .await
        .unwrap();

    let (base_url, state, _server_handle) = start_test_server(server_repos.clone()).await;
    let client = ProtocolClient::new(format!("{}/bitmap-repo", base_url));

    // ---------- fetch with have=[C1], want=[C2]: bitmap path ----------
    assert_eq!(state.bitmap_hits.load(Ordering::Relaxed), 0);

    let bitmap_temp = TempDir::new().unwrap();
    let bitmap_mediagit = bitmap_temp.path().join(".mediagit");
    tokio::fs::create_dir_all(bitmap_mediagit.join("objects"))
        .await
        .unwrap();
    let bitmap_client_odb = open_odb(&bitmap_mediagit).await;

    client
        .download_pack_streaming(&bitmap_client_odb, vec![c2.to_hex()], vec![c1.to_hex()])
        .await
        .expect("bitmap-accelerated fetch");

    assert_eq!(
        state.bitmap_hits.load(Ordering::Relaxed),
        1,
        "the have-tip bitmap must have been used exactly once (short-circuit fired)"
    );

    // Correctness: must ship C2/T2/B2, must NOT ship C1/T1/B1.
    assert!(bitmap_client_odb.read(&c2).await.is_ok());
    assert!(bitmap_client_odb.read(&t2).await.is_ok());
    assert!(bitmap_client_odb.read(&b2).await.is_ok());
    assert!(bitmap_client_odb.read(&c1).await.is_err());
    assert!(bitmap_client_odb.read(&t1).await.is_err());
    assert!(bitmap_client_odb.read(&b1).await.is_err());

    // ---------- same fetch with MEDIAGIT_BITMAP=0: BFS-only path ----------
    // SAFETY: test-only env var scoping. `MEDIAGIT_BITMAP` is process-global
    // and `bitmap_retention_deletes_previous_tip_bitmap_on_ref_advance` below
    // relies on it defaulting to enabled — hold `BITMAP_ENV_LOCK` for the
    // whole mutated window so the two tests can't race each other.
    let _env_guard = BITMAP_ENV_LOCK.lock().await;
    mediagit_test_utils::set_var("MEDIAGIT_BITMAP", "0");

    let bfs_temp = TempDir::new().unwrap();
    let bfs_mediagit = bfs_temp.path().join(".mediagit");
    tokio::fs::create_dir_all(bfs_mediagit.join("objects"))
        .await
        .unwrap();
    let bfs_client_odb = open_odb(&bfs_mediagit).await;

    client
        .download_pack_streaming(&bfs_client_odb, vec![c2.to_hex()], vec![c1.to_hex()])
        .await
        .expect("BFS-only fetch");

    mediagit_test_utils::remove_var("MEDIAGIT_BITMAP");

    // The counter must not have moved (bitmap path was disabled)...
    assert_eq!(
        state.bitmap_hits.load(Ordering::Relaxed),
        1,
        "MEDIAGIT_BITMAP=0 must not use the bitmap short-circuit"
    );

    // ...and the delivered object set must be identical either way: bitmap
    // is a pure speedup, never a correctness dependency.
    assert!(bfs_client_odb.read(&c2).await.is_ok());
    assert!(bfs_client_odb.read(&t2).await.is_ok());
    assert!(bfs_client_odb.read(&b2).await.is_ok());
    assert!(bfs_client_odb.read(&c1).await.is_err());
    assert!(bfs_client_odb.read(&t1).await.is_err());
    assert!(bfs_client_odb.read(&b1).await.is_err());
}

/// A stale/corrupt bitmap for the have-tip must fall back to BFS, not error
/// or (worse) silently under/over-report the closure.
#[tokio::test]
async fn corrupt_bitmap_falls_back_to_bfs() {
    let server_temp = TempDir::new().unwrap();
    let server_repos = server_temp.path().join("repos");
    let server_repo = server_repos.join("corrupt-bitmap-repo");
    let server_mediagit = server_repo.join(".mediagit");
    tokio::fs::create_dir_all(server_mediagit.join("objects"))
        .await
        .unwrap();
    tokio::fs::create_dir_all(server_mediagit.join("refs/heads"))
        .await
        .unwrap();

    let (server_odb, server_storage) = open_server_odb(&server_repo).await;
    let (c1, _t1, _b1) = commit_with_file(&server_odb, b"v1", "a.txt", None).await;
    let (c2, t2, b2) = commit_with_file(&server_odb, b"v2", "b.txt", Some(c1)).await;

    RefDatabase::new(server_mediagit.clone())
        .write(&Ref::new_direct("refs/heads/main".to_string(), c2))
        .await
        .unwrap();

    // Corrupt bitmap: garbage bytes at the have-tip's key.
    server_storage
        .put(&bitmap_key(&c1), b"not a valid bitmap")
        .await
        .unwrap();

    let (base_url, state, _server_handle) = start_test_server(server_repos.clone()).await;
    let client = ProtocolClient::new(format!("{}/corrupt-bitmap-repo", base_url));

    let client_temp = TempDir::new().unwrap();
    let client_mediagit = client_temp.path().join(".mediagit");
    tokio::fs::create_dir_all(client_mediagit.join("objects"))
        .await
        .unwrap();
    let client_odb = open_odb(&client_mediagit).await;

    client
        .download_pack_streaming(&client_odb, vec![c2.to_hex()], vec![c1.to_hex()])
        .await
        .expect("fetch must succeed via BFS fallback despite corrupt bitmap");

    assert_eq!(
        state.bitmap_hits.load(Ordering::Relaxed),
        0,
        "corrupt bitmap must not count as a hit"
    );
    assert!(client_odb.read(&c2).await.is_ok());
    assert!(client_odb.read(&t2).await.is_ok());
    assert!(client_odb.read(&b2).await.is_ok());
    assert!(client_odb.read(&c1).await.is_err());
}

/// Poll `storage` for `key` until it either exists or reaches the deadline.
/// The post-receive bitmap hook (`repo.rs::update_refs`) runs in a detached
/// `tokio::spawn`, so its effects are not visible immediately after the push
/// response returns — a bare sleep would be flaky under load.
async fn wait_until_exists(storage: &dyn StorageBackend, key: &str, deadline: Duration) -> bool {
    let start = std::time::Instant::now();
    loop {
        if storage.exists(key).await.unwrap_or(false) {
            return true;
        }
        if start.elapsed() >= deadline {
            return false;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

/// Poll `storage` for `key` until it is absent (deleted) or the deadline
/// passes.
async fn wait_until_absent(storage: &dyn StorageBackend, key: &str, deadline: Duration) -> bool {
    let start = std::time::Instant::now();
    loop {
        if !storage.exists(key).await.unwrap_or(true) {
            return true;
        }
        if start.elapsed() >= deadline {
            return false;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

/// F3 retention: pushing a second commit that advances a ref must, once the
/// new tip's bitmap is persisted, delete the previous tip's bitmap — steady
/// state is ~one bitmap per ref, not one per historical tip.
#[tokio::test]
async fn bitmap_retention_deletes_previous_tip_bitmap_on_ref_advance() {
    // See `BITMAP_ENV_LOCK` docs: this test relies on `bitmap_enabled()`
    // defaulting to on, which requires `MEDIAGIT_BITMAP` unset for the whole
    // test, so it must not race `bitmap_short_circuit_matches_bfs_disabled_run`'s
    // temporary `MEDIAGIT_BITMAP=0` window.
    let _env_guard = BITMAP_ENV_LOCK.lock().await;

    let server_temp = TempDir::new().unwrap();
    let server_repos = server_temp.path().join("repos");
    let server_repo = server_repos.join("retention-repo");
    let server_mediagit = server_repo.join(".mediagit");
    tokio::fs::create_dir_all(server_mediagit.join("objects"))
        .await
        .unwrap();
    tokio::fs::create_dir_all(server_mediagit.join("refs/heads"))
        .await
        .unwrap();

    // Establish the namespaced layout server-side (repo_id + LAYOUT marker)
    // before the HTTP server starts serving it — see `open_server_odb` docs.
    let (_server_odb, server_storage) = open_server_odb(&server_repo).await;

    let (base_url, _state, _server_handle) = start_test_server(server_repos.clone()).await;
    let client = ProtocolClient::new(format!("{}/retention-repo", base_url));

    // ---------- client repo: push commit A (tip1), no previous ref ----------
    let client_temp = TempDir::new().unwrap();
    let client_mediagit = client_temp.path().join(".mediagit");
    tokio::fs::create_dir_all(client_mediagit.join("objects"))
        .await
        .unwrap();
    let client_odb = open_odb(&client_mediagit).await;

    let (tip1, _t1, _b1) = commit_with_file(&client_odb, b"v1", "a.txt", None).await;

    let update1 = mediagit_protocol::RefUpdate {
        name: "refs/heads/main".to_string(),
        old_oid: None,
        new_oid: tip1.to_hex(),
        delete: false,
    };
    let (response1, _stats1) = client
        .push(&client_odb, vec![update1], false)
        .await
        .expect("push of tip1 must succeed");
    assert!(response1.success);

    let tip1_key = bitmap_key(&tip1);
    assert!(
        wait_until_exists(server_storage.as_ref(), &tip1_key, Duration::from_secs(5)).await,
        "bitmap for tip1 must appear once the post-receive hook completes"
    );

    // ---------- push commit B (tip2), advancing the same ref ----------
    let (tip2, _t2, _b2) = commit_with_file(&client_odb, b"v2", "b.txt", Some(tip1)).await;

    let update2 = mediagit_protocol::RefUpdate {
        name: "refs/heads/main".to_string(),
        old_oid: Some(tip1.to_hex()),
        new_oid: tip2.to_hex(),
        delete: false,
    };
    let (response2, _stats2) = client
        .push(&client_odb, vec![update2], false)
        .await
        .expect("push of tip2 must succeed");
    assert!(response2.success);

    let tip2_key = bitmap_key(&tip2);
    assert!(
        wait_until_exists(server_storage.as_ref(), &tip2_key, Duration::from_secs(5)).await,
        "bitmap for tip2 must appear once the post-receive hook completes"
    );
    assert!(
        wait_until_absent(server_storage.as_ref(), &tip1_key, Duration::from_secs(5)).await,
        "bitmap for tip1 must be deleted once tip2's bitmap is persisted"
    );
}
