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

//! End-to-end tests verifying that pack negotiation actually ships a delta.
//!
//! The whole point of the `have` plumbing is that the server stops walking
//! at objects the client already has. These tests drive the protocol
//! directly via `ProtocolClient::download_pack_streaming` rather than
//! through the CLI, so the assertions stay tight on the server's pack
//! contents rather than on CLI output.

use std::path::PathBuf;
use std::sync::Arc;
use tempfile::TempDir;
use tokio::net::TcpListener;

use mediagit_protocol::ProtocolClient;
use mediagit_storage::{LocalBackend, StorageBackend};
use mediagit_versioning::{
    Commit, FileMode, ObjectDatabase, ObjectType, Oid, Ref, RefDatabase, Signature, Tree, TreeEntry,
};

async fn start_test_server(repos_dir: PathBuf) -> (String, tokio::task::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let base_url = format!("http://{}", addr);

    let state = Arc::new(mediagit_server::AppState::new(repos_dir.clone()));
    let app = mediagit_server::create_router(state);
    let handle = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    // Give the server a moment to bind before the first request
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

    (base_url, handle)
}

async fn open_odb(mediagit_dir: &std::path::Path) -> ObjectDatabase {
    let storage: Arc<dyn StorageBackend> = Arc::new(LocalBackend::new(mediagit_dir).await.unwrap());
    ObjectDatabase::new(storage, 100)
}

/// Create a commit with a new blob and single-file tree on top of `parent`.
/// Returns `(commit_oid, tree_oid, blob_oid)`.
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

/// End-to-end: second fetch with a valid `have` must ship only the delta
/// and not the whole history, and a fetch where `have` already covers `want`
/// must ship nothing at all.
///
/// This is the regression test for the bug where the server was ignoring the
/// `have` field and always sending a full pack.
#[tokio::test]
async fn incremental_fetch_sends_only_delta() {
    // ---------- set up a server repo with a single commit C1 ----------
    let server_temp = TempDir::new().unwrap();
    let server_repos = server_temp.path().join("repos");
    let server_repo = server_repos.join("delta-repo");
    let server_mediagit = server_repo.join(".mediagit");
    tokio::fs::create_dir_all(server_mediagit.join("objects"))
        .await
        .unwrap();
    tokio::fs::create_dir_all(server_mediagit.join("refs/heads"))
        .await
        .unwrap();

    let server_odb = open_odb(&server_mediagit).await;
    let (c1, t1, b1) = commit_with_file(&server_odb, b"v1", "a.txt", None).await;

    let refdb = RefDatabase::new(server_mediagit.clone());
    refdb
        .write(&Ref::new_direct("refs/heads/main".to_string(), c1))
        .await
        .unwrap();

    let (base_url, _server_handle) = start_test_server(server_repos.clone()).await;
    let client = ProtocolClient::new(format!("{}/delta-repo", base_url));

    // ---------- 1. Cold fetch (have=[]): must receive {C1, T1, B1} ----------
    let cold_temp = TempDir::new().unwrap();
    let cold_mediagit = cold_temp.path().join(".mediagit");
    tokio::fs::create_dir_all(cold_mediagit.join("objects"))
        .await
        .unwrap();
    let cold_odb = open_odb(&cold_mediagit).await;

    client
        .download_pack_streaming(&cold_odb, vec![c1.to_hex()], vec![])
        .await
        .expect("cold fetch");

    assert!(cold_odb.read(&c1).await.is_ok(), "cold fetch missing c1");
    assert!(cold_odb.read(&t1).await.is_ok(), "cold fetch missing t1");
    assert!(cold_odb.read(&b1).await.is_ok(), "cold fetch missing b1");

    // ---------- extend server history with C2 on top of C1 ----------
    let (c2, t2, b2) = commit_with_file(&server_odb, b"v2", "b.txt", Some(c1)).await;
    refdb
        .write(&Ref::new_direct("refs/heads/main".to_string(), c2))
        .await
        .unwrap();

    // ---------- 2. Incremental fetch (have=[C1]): must ONLY add C2/T2/B2 ----------
    // Use a FRESH ODB so we can directly count which objects were written.
    let delta_temp = TempDir::new().unwrap();
    let delta_mediagit = delta_temp.path().join(".mediagit");
    tokio::fs::create_dir_all(delta_mediagit.join("objects"))
        .await
        .unwrap();
    let delta_odb = open_odb(&delta_mediagit).await;

    client
        .download_pack_streaming(&delta_odb, vec![c2.to_hex()], vec![c1.to_hex()])
        .await
        .expect("incremental fetch");

    // The pack must contain C2/T2/B2 (client needs them to reconstruct C2)...
    assert!(delta_odb.read(&c2).await.is_ok(), "delta fetch missing c2");
    assert!(delta_odb.read(&t2).await.is_ok(), "delta fetch missing t2");
    assert!(delta_odb.read(&b2).await.is_ok(), "delta fetch missing b2");

    // ...and MUST NOT contain C1/T1/B1 — that's the whole point of negotiation.
    assert!(
        delta_odb.read(&c1).await.is_err(),
        "delta fetch wrongly included c1 (have cutoff failed)"
    );
    assert!(
        delta_odb.read(&t1).await.is_err(),
        "delta fetch wrongly included t1 (have cutoff failed)"
    );
    assert!(
        delta_odb.read(&b1).await.is_err(),
        "delta fetch wrongly included b1 (have cutoff failed)"
    );

    // ---------- 3. Up-to-date fetch (have=[C2], want=[C2]) ships nothing ----------
    let noop_temp = TempDir::new().unwrap();
    let noop_mediagit = noop_temp.path().join(".mediagit");
    tokio::fs::create_dir_all(noop_mediagit.join("objects"))
        .await
        .unwrap();
    let noop_odb = open_odb(&noop_mediagit).await;

    client
        .download_pack_streaming(&noop_odb, vec![c2.to_hex()], vec![c2.to_hex()])
        .await
        .expect("no-op fetch must not error");

    assert!(
        noop_odb.read(&c2).await.is_err(),
        "up-to-date fetch wrongly re-sent c2"
    );
    assert!(
        noop_odb.read(&c1).await.is_err(),
        "up-to-date fetch wrongly re-sent c1"
    );
}

/// Stale / unknown `have` OIDs must be tolerated: the walker skips them
/// silently and the fetch still succeeds. This protects against corrupted
/// local state or clients racing with server-side GC.
#[tokio::test]
async fn unknown_have_oids_do_not_break_fetch() {
    let server_temp = TempDir::new().unwrap();
    let server_repos = server_temp.path().join("repos");
    let server_repo = server_repos.join("unknown-have-repo");
    let server_mediagit = server_repo.join(".mediagit");
    tokio::fs::create_dir_all(server_mediagit.join("objects"))
        .await
        .unwrap();
    tokio::fs::create_dir_all(server_mediagit.join("refs/heads"))
        .await
        .unwrap();

    let server_odb = open_odb(&server_mediagit).await;
    let (c1, _t1, _b1) = commit_with_file(&server_odb, b"v1", "a.txt", None).await;
    RefDatabase::new(server_mediagit.clone())
        .write(&Ref::new_direct("refs/heads/main".to_string(), c1))
        .await
        .unwrap();

    let (base_url, _server_handle) = start_test_server(server_repos.clone()).await;
    let client = ProtocolClient::new(format!("{}/unknown-have-repo", base_url));

    let fresh_temp = TempDir::new().unwrap();
    let fresh_mediagit = fresh_temp.path().join(".mediagit");
    tokio::fs::create_dir_all(fresh_mediagit.join("objects"))
        .await
        .unwrap();
    let fresh_odb = open_odb(&fresh_mediagit).await;

    // Fabricate a have OID the server has never seen.
    let stale = Oid::hash(b"nonexistent-have-from-stale-client");

    client
        .download_pack_streaming(&fresh_odb, vec![c1.to_hex()], vec![stale.to_hex()])
        .await
        .expect("stale have must not break fetch");

    assert!(
        fresh_odb.read(&c1).await.is_ok(),
        "fetch with stale have must still deliver wanted objects"
    );
}
