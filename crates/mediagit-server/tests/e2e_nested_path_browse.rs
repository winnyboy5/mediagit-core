// SPDX-License-Identifier: BUSL-1.1
// Copyright (C) 2025-2026 Aswin Krishnamoorthy

//! Regression tests for QA-003: server-side path resolvers (`resolve_path_to_blob`,
//! `resolve_path_to_tree`) assumed git-style nested subtrees, but commits build a
//! single-level (flat) tree keyed by full relative path (see `commit.rs`). Any
//! nested path (e.g. `gen/blob.bin`) 404'd on every backend.

use std::path::PathBuf;
use std::sync::Arc;
use tempfile::TempDir;
use tokio::net::TcpListener;

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

    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

    (base_url, handle)
}

/// See `e2e_incremental_fetch.rs::open_server_odb` for why this uses a
/// namespaced backend instead of a raw `LocalBackend`: the production server
/// wraps storage in `NamespacedBackend` (layout v2), and seeding through a
/// raw backend would write objects at a different physical path than the
/// server later reads them from.
async fn open_server_odb(repo_root: &std::path::Path) -> ObjectDatabase {
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
    ObjectDatabase::new(storage, 100)
}

/// Build a commit whose tree has flat (full relative path) entries, exactly
/// as `mediagit-cli/src/commands/commit.rs` builds them: no nested
/// `Directory` entries, one BTreeMap key per file with the full path as the
/// key (e.g. `"a/b/c.txt"`, `"gen/blob.bin"`).
async fn commit_flat_tree(odb: &ObjectDatabase, files: &[(&str, &[u8])]) -> Oid {
    let mut tree = Tree::new();
    for (path, content) in files {
        let blob_oid = odb.write(ObjectType::Blob, content).await.unwrap();
        tree.add_entry(TreeEntry::new(
            path.to_string(),
            FileMode::Regular,
            blob_oid,
        ));
    }
    let tree_oid = tree.write(odb).await.unwrap();

    let author = Signature::now("Test".to_string(), "t@e".to_string());
    let commit = Commit::new(tree_oid, author.clone(), author, "msg".to_string());
    commit.write(odb).await.unwrap()
}

async fn setup_repo(repo_name: &str, files: &[(&str, &[u8])]) -> (TempDir, PathBuf) {
    let server_temp = TempDir::new().unwrap();
    let server_repos = server_temp.path().join("repos");
    let server_repo = server_repos.join(repo_name);
    let server_mediagit = server_repo.join(".mediagit");
    tokio::fs::create_dir_all(server_mediagit.join("objects"))
        .await
        .unwrap();
    tokio::fs::create_dir_all(server_mediagit.join("refs/heads"))
        .await
        .unwrap();

    let odb = open_server_odb(&server_repo).await;
    let commit_oid = commit_flat_tree(&odb, files).await;

    let refdb = RefDatabase::new(server_mediagit.clone());
    refdb
        .write(&Ref::new_direct("refs/heads/main".to_string(), commit_oid))
        .await
        .unwrap();

    (server_temp, server_repos)
}

#[tokio::test]
async fn resolves_nested_file_blob() {
    let (_temp, server_repos) = setup_repo(
        "nested-repo",
        &[("gen/blob.bin", b"nested blob content" as &[u8])],
    )
    .await;

    let (base_url, _handle) = start_test_server(server_repos).await;
    mediagit_protocol::ensure_crypto_provider();
    let client = reqwest::Client::new();

    let resp = client
        .get(format!(
            "{}/nested-repo/files/gen/blob.bin?ref=main",
            base_url
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200, "nested file must resolve");
    let body = resp.bytes().await.unwrap();
    assert_eq!(&body[..], b"nested blob content");
}

#[tokio::test]
async fn lists_directory_synthesized_from_flat_prefix() {
    let (_temp, server_repos) = setup_repo(
        "dir-listing-repo",
        &[
            ("a/b/c.txt", b"c" as &[u8]),
            ("a/other.txt", b"other" as &[u8]),
            ("root.txt", b"root" as &[u8]),
        ],
    )
    .await;

    let (base_url, _handle) = start_test_server(server_repos).await;
    mediagit_protocol::ensure_crypto_provider();
    let client = reqwest::Client::new();

    // Root listing: "a" is a synthesized directory, "root.txt" is a file.
    let resp = client
        .get(format!("{}/dir-listing-repo/tree?ref=main", base_url))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    let entries = body["entries"].as_array().unwrap();
    let names: Vec<&str> = entries
        .iter()
        .map(|e| e["name"].as_str().unwrap())
        .collect();
    assert!(
        names.contains(&"a"),
        "root listing missing dir 'a': {:?}",
        names
    );
    assert!(
        names.contains(&"root.txt"),
        "root listing missing file 'root.txt': {:?}",
        names
    );
    let a_entry = entries.iter().find(|e| e["name"] == "a").unwrap();
    assert_eq!(a_entry["type"], "tree");

    // Listing "a/": "b" is a dir, "other.txt" is a file.
    let resp = client
        .get(format!("{}/dir-listing-repo/tree/a?ref=main", base_url))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    let entries = body["entries"].as_array().unwrap();
    let names: Vec<&str> = entries
        .iter()
        .map(|e| e["name"].as_str().unwrap())
        .collect();
    assert!(
        names.contains(&"b"),
        "'a/' listing missing dir 'b': {:?}",
        names
    );
    assert!(
        names.contains(&"other.txt"),
        "'a/' listing missing file 'other.txt': {:?}",
        names
    );
    let b_entry = entries.iter().find(|e| e["name"] == "b").unwrap();
    assert_eq!(b_entry["type"], "tree");
}

#[tokio::test]
async fn root_listing_unchanged_for_single_level_repo() {
    let (_temp, server_repos) = setup_repo(
        "flat-repo",
        &[("a.txt", b"a" as &[u8]), ("b.txt", b"b" as &[u8])],
    )
    .await;

    let (base_url, _handle) = start_test_server(server_repos).await;
    mediagit_protocol::ensure_crypto_provider();
    let client = reqwest::Client::new();

    let resp = client
        .get(format!("{}/flat-repo/tree?ref=main", base_url))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    let entries = body["entries"].as_array().unwrap();
    assert_eq!(entries.len(), 2);
    let names: Vec<&str> = entries
        .iter()
        .map(|e| e["name"].as_str().unwrap())
        .collect();
    assert!(names.contains(&"a.txt"));
    assert!(names.contains(&"b.txt"));
    for e in entries {
        assert_eq!(e["type"], "blob");
    }
}

#[tokio::test]
async fn missing_nested_path_returns_404_not_panic() {
    let (_temp, server_repos) = setup_repo(
        "missing-path-repo",
        &[("gen/blob.bin", b"present" as &[u8])],
    )
    .await;

    let (base_url, _handle) = start_test_server(server_repos).await;
    mediagit_protocol::ensure_crypto_provider();
    let client = reqwest::Client::new();

    let resp = client
        .get(format!(
            "{}/missing-path-repo/files/missing/x?ref=main",
            base_url
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 404);
}
