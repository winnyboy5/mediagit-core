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

//! E2E tests for presigned-URL chunk upload endpoints.
//!
//! Covers:
//! - `POST /chunks/upload-urls` returns null for LocalBackend (no presigning)
//! - `POST /chunks/complete` correctly identifies missing vs present chunks
//! - Full push via ProtocolClient still succeeds (fallback path)
//! - Re-push uploads zero chunks (idempotence)

use std::collections::HashMap;
use std::sync::Arc;
use tempfile::TempDir;
use tokio::net::TcpListener;

use mediagit_protocol::ProtocolClient;
use mediagit_storage::{LocalBackend, StorageBackend};
use mediagit_versioning::chunking::{ChunkManifest, ChunkRef, ChunkType};
use mediagit_versioning::{ObjectDatabase, Oid};

async fn start_test_server(repos_dir: std::path::PathBuf) -> (String, tokio::task::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let base_url = format!("http://{}", addr);
    let state = Arc::new(mediagit_server::AppState::new(repos_dir));
    let app = mediagit_server::create_router(state);
    let handle = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
    (base_url, handle)
}

async fn open_odb(mediagit_dir: &std::path::Path) -> ObjectDatabase {
    let storage: Arc<dyn StorageBackend> = Arc::new(LocalBackend::new(mediagit_dir).await.unwrap());
    ObjectDatabase::new(storage, 100)
}

/// PUT a chunk directly to the server via the proxy path.
async fn proxy_put_chunk(
    client: &reqwest::Client,
    base_url: &str,
    repo: &str,
    id: &Oid,
    data: &[u8],
) {
    let url = format!("{}/{}/chunks/{}", base_url, repo, id.to_hex());
    let resp = client.put(&url).body(data.to_vec()).send().await.unwrap();
    assert!(
        resp.status().is_success(),
        "proxy PUT failed: {}",
        resp.status()
    );
}

/// ── Test 1 ─────────────────────────────────────────────────────────────────
/// `POST /chunks/upload-urls` must return null for every chunk when the
/// backend is LocalBackend (which does not implement presigning).
#[tokio::test]
async fn presign_upload_urls_returns_null_for_local_backend() {
    let repos_tmp = TempDir::new().unwrap();
    let repo = "test-repo";
    let repo_mg = repos_tmp.path().join(repo).join(".mediagit");
    tokio::fs::create_dir_all(repo_mg.join("objects"))
        .await
        .unwrap();
    tokio::fs::create_dir_all(repo_mg.join("refs/heads"))
        .await
        .unwrap();
    let (base_url, _handle) = start_test_server(repos_tmp.path().to_path_buf()).await;

    let client = reqwest::Client::new();

    let ids = vec!["aabbcc".to_string(), "ddeeff".to_string()];
    let sizes: HashMap<String, u64> = ids.iter().map(|id| (id.clone(), 1024u64)).collect();

    let url = format!("{}/{}/chunks/upload-urls", base_url, repo);
    let body = serde_json::json!({ "chunk_ids": ids, "sizes": sizes });
    let resp = client.post(&url).json(&body).send().await.unwrap();

    assert!(
        resp.status().is_success(),
        "upload-urls returned {}",
        resp.status()
    );

    let map: HashMap<String, Option<serde_json::Value>> = resp.json().await.unwrap();
    for id in &ids {
        assert!(
            matches!(map.get(id), Some(None)),
            "LocalBackend should return null for chunk {id}, got {:?}",
            map.get(id)
        );
    }
}

/// ── Test 2 ─────────────────────────────────────────────────────────────────
/// `POST /chunks/complete` returns only the IDs that are absent from storage.
#[tokio::test]
async fn complete_chunk_uploads_reports_missing_correctly() {
    let repos_tmp = TempDir::new().unwrap();
    let repo = "test-repo";
    let repo_mg = repos_tmp.path().join(repo).join(".mediagit");
    tokio::fs::create_dir_all(repo_mg.join("objects"))
        .await
        .unwrap();
    tokio::fs::create_dir_all(repo_mg.join("refs/heads"))
        .await
        .unwrap();
    let (base_url, _handle) = start_test_server(repos_tmp.path().to_path_buf()).await;

    let client = reqwest::Client::new();

    let present_data = b"I am present on the server";
    let present_id = Oid::hash(present_data);
    proxy_put_chunk(&client, &base_url, repo, &present_id, present_data).await;

    let absent_id = Oid::hash(b"I was never uploaded");

    let url = format!("{}/{}/chunks/complete", base_url, repo);
    let body = serde_json::json!({
        "chunk_ids": [present_id.to_hex(), absent_id.to_hex()]
    });
    let resp = client.post(&url).json(&body).send().await.unwrap();
    assert!(
        resp.status().is_success(),
        "complete returned {}",
        resp.status()
    );

    #[derive(serde::Deserialize)]
    struct Resp {
        missing: Vec<String>,
    }
    let r: Resp = resp.json().await.unwrap();

    assert!(
        !r.missing.contains(&present_id.to_hex()),
        "present chunk should not appear in missing"
    );
    assert!(
        r.missing.contains(&absent_id.to_hex()),
        "absent chunk must appear in missing"
    );
}

/// ── Test 3 ─────────────────────────────────────────────────────────────────
/// Full push using ProtocolClient succeeds with the new presign-aware upload
/// path. LocalBackend returns None → every chunk goes through the proxy PUT
/// fallback. Idempotent re-push uploads zero additional chunks.
#[tokio::test]
async fn full_push_with_presign_fallback_is_idempotent() {
    let repos_tmp = TempDir::new().unwrap();
    let client_tmp = TempDir::new().unwrap();

    let repo = "push-repo";

    // Server-side repo directory must exist with the standard layout.
    let server_repo_mediagit = repos_tmp.path().join(repo).join(".mediagit");
    tokio::fs::create_dir_all(server_repo_mediagit.join("objects"))
        .await
        .unwrap();
    tokio::fs::create_dir_all(server_repo_mediagit.join("refs/heads"))
        .await
        .unwrap();

    let (base_url, _handle) = start_test_server(repos_tmp.path().to_path_buf()).await;

    let client_mg = client_tmp.path().join(".mediagit");
    tokio::fs::create_dir_all(client_mg.join("objects"))
        .await
        .unwrap();
    let odb = open_odb(&client_mg).await;

    // Build a manifest with two full chunks.
    let chunk_a_data = vec![0xAAu8; 512];
    let chunk_b_data = vec![0xBBu8; 768];
    let chunk_a_id = Oid::hash(&chunk_a_data);
    let chunk_b_id = Oid::hash(&chunk_b_data);

    odb.put_compressed_chunk(&chunk_a_id, &chunk_a_data)
        .await
        .unwrap();
    odb.put_compressed_chunk(&chunk_b_id, &chunk_b_data)
        .await
        .unwrap();

    let file_oid = Oid::hash(b"synthetic-file-oid");
    let manifest = ChunkManifest {
        chunks: vec![
            ChunkRef {
                id: chunk_a_id,
                offset: 0,
                size: chunk_a_data.len(),
                chunk_type: ChunkType::Generic,
                codec_hint: Default::default(),
            },
            ChunkRef {
                id: chunk_b_id,
                offset: chunk_a_data.len() as u64,
                size: chunk_b_data.len(),
                chunk_type: ChunkType::Generic,
                codec_hint: Default::default(),
            },
        ],
        total_size: (chunk_a_data.len() + chunk_b_data.len()) as u64,
        filename: Some("test.bin".to_string()),
    };
    odb.put_manifest(&file_oid, &manifest).await.unwrap();

    let client_url = format!("{}/{}", base_url, repo);
    let protocol = ProtocolClient::new(client_url);

    let uploaded_first = protocol
        .upload_chunked_objects(&odb, &[file_oid], |_, _| {})
        .await
        .expect("first push should succeed");

    assert_eq!(
        uploaded_first, 2,
        "both chunks must be uploaded on first push"
    );

    // Re-push: server already has both chunks → zero uploads.
    let uploaded_second = protocol
        .upload_chunked_objects(&odb, &[file_oid], |_, _| {})
        .await
        .expect("second push should succeed");

    assert_eq!(
        uploaded_second, 0,
        "idempotent re-push must upload zero chunks"
    );
}
