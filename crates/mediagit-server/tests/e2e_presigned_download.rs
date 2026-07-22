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

//! E2E tests for presigned-URL chunk download endpoints.
//!
//! Covers:
//! - Happy path: backend mints presigned GET URL → client fetches chunk bytes
//!   directly from mock bucket, hash verified, chunk lands in ODB.
//! - Fallback to proxy (Ok(None)): LocalBackend returns null → client falls
//!   through to `GET /chunks/{hex}` → chunk still retrieved correctly.
//! - 403 fallback: presigned GET returns 403 → client falls back to proxy
//!   GET `/chunks/{hex}` → chunk still retrieved correctly.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use tempfile::TempDir;
use tokio::net::TcpListener;

use axum::response::IntoResponse;
use mediagit_protocol::ProtocolClient;
use mediagit_storage::{LocalBackend, PresignedDownload, StorageBackend};
use mediagit_versioning::chunking::{ChunkManifest, ChunkRef, ChunkType};
use mediagit_versioning::{ObjectDatabase, Oid};

// ── Test helpers ─────────────────────────────────────────────────────────────

/// A storage backend that delegates all operations to an inner `LocalBackend`
/// but overrides `presign_get` to return a caller-supplied URL.
///
/// Used in tests 1 and 3 to inject a mock bucket URL so
/// `POST /chunks/download-urls` returns a presigned entry instead of `null`.
#[derive(Debug)]
struct PresignOverrideBackend {
    inner: LocalBackend,
    /// The URL template: `{url_prefix}/chunks/{key}`.
    presign_url_prefix: String,
}

impl PresignOverrideBackend {
    fn new(inner: LocalBackend, presign_url_prefix: impl Into<String>) -> Self {
        Self {
            inner,
            presign_url_prefix: presign_url_prefix.into(),
        }
    }
}

#[async_trait::async_trait]
impl StorageBackend for PresignOverrideBackend {
    async fn get(&self, key: &str) -> anyhow::Result<Vec<u8>> {
        self.inner.get(key).await
    }

    async fn put(&self, key: &str, data: &[u8]) -> anyhow::Result<()> {
        self.inner.put(key, data).await
    }

    async fn exists(&self, key: &str) -> anyhow::Result<bool> {
        self.inner.exists(key).await
    }

    async fn head(&self, key: &str) -> anyhow::Result<Option<u64>> {
        self.inner.head(key).await
    }

    async fn delete(&self, key: &str) -> anyhow::Result<()> {
        self.inner.delete(key).await
    }

    async fn list_objects(&self, prefix: &str) -> anyhow::Result<Vec<String>> {
        self.inner.list_objects(prefix).await
    }

    async fn presign_get(
        &self,
        key: &str,
        _ttl: std::time::Duration,
    ) -> anyhow::Result<Option<PresignedDownload>> {
        // Return a URL pointing to the mock bucket server at `{prefix}/{key}`.
        let url = format!("{}/{}", self.presign_url_prefix, key);
        Ok(Some(PresignedDownload {
            url,
            headers: vec![],
            expires_in_secs: 3600,
        }))
    }
}

/// Start the main MediaGit server with the given repos_dir.
/// Returns `(base_url, join_handle)`.
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

/// Start the main MediaGit server with a pre-injected storage backend for `repo`.
///
/// Pre-populating `state.storage_backends` before `create_router` means the
/// handler's fast-path cache hit returns our `PresignOverrideBackend` on every
/// request — no real config file needed.
async fn start_test_server_with_backend(
    repos_dir: std::path::PathBuf,
    repo: &str,
    backend: Arc<dyn StorageBackend>,
) -> (String, tokio::task::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let base_url = format!("http://{}", addr);

    let state = Arc::new(mediagit_server::AppState::new(repos_dir.clone()));

    // Pre-populate the storage-backend cache so the handler's fast path hits
    // our mock instead of trying to load a config file.
    {
        let repo_path = repos_dir.join(repo);
        let mut map = state.storage_backends.write().await;
        map.insert(repo_path, backend);
    }

    let app = mediagit_server::create_router(state);
    let handle = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
    (base_url, handle)
}

/// Open a client-side ODB backed by `LocalBackend` rooted at `mediagit_dir`.
async fn open_odb(mediagit_dir: &std::path::Path) -> ObjectDatabase {
    let storage: Arc<dyn StorageBackend> = Arc::new(LocalBackend::new(mediagit_dir).await.unwrap());
    ObjectDatabase::new(storage, 100)
}

/// Build a one-chunk manifest, store the chunk in `odb`, and return `(file_oid, chunk_id, chunk_data)`.
async fn make_single_chunk_manifest(odb: &ObjectDatabase) -> (Oid, Oid, Vec<u8>) {
    let chunk_data = vec![0xCAu8; 512];
    let chunk_id = Oid::hash(&chunk_data);
    odb.put_compressed_chunk(&chunk_id, &chunk_data)
        .await
        .unwrap();

    let file_oid = Oid::hash(b"e2e-download-test-file");
    let manifest = ChunkManifest {
        chunks: vec![ChunkRef {
            id: chunk_id,
            offset: 0,
            size: chunk_data.len(),
            chunk_type: ChunkType::Generic,
            codec_hint: Default::default(),
        }],
        total_size: chunk_data.len() as u64,
        filename: Some("test-dl.bin".to_string()),
    };
    odb.put_manifest(&file_oid, &manifest).await.unwrap();
    (file_oid, chunk_id, chunk_data)
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

// ── Test 1 ────────────────────────────────────────────────────────────────────
/// Happy path: the backend returns a valid presigned GET URL.
///
/// A mock "bucket" server serves the raw chunk bytes.  The client resolves the
/// presigned URL, fetches the bytes directly (never hitting the proxy route),
/// verifies the BLAKE3 hash, and writes the chunk into the local ODB.
#[tokio::test]
async fn presigned_download_happy_path() {
    // ── setup: server-side repo dirs ────────────────────────────────────────
    let repos_tmp = TempDir::new().unwrap();
    let client_tmp = TempDir::new().unwrap();
    let repo = "dl-happy-repo";

    let server_repo_mg = repos_tmp.path().join(repo).join(".mediagit");
    tokio::fs::create_dir_all(server_repo_mg.join("objects"))
        .await
        .unwrap();
    tokio::fs::create_dir_all(server_repo_mg.join("refs/heads"))
        .await
        .unwrap();

    // ── setup: client-side ODB with one chunk + manifest ────────────────────
    let client_mg = client_tmp.path().join(".mediagit");
    tokio::fs::create_dir_all(client_mg.join("objects"))
        .await
        .unwrap();
    let server_odb = open_odb(&server_repo_mg).await;
    let (_file_oid, chunk_id, chunk_data) = make_single_chunk_manifest(&server_odb).await;

    // ── setup: mock "bucket" server — serves the raw chunk bytes ────────────
    // Track how many times the bucket was actually hit.
    let bucket_hit_count = Arc::new(AtomicUsize::new(0));
    let bucket_hit_count_clone = Arc::clone(&bucket_hit_count);

    // Clone the chunk data for the closure.
    let chunk_data_for_bucket = chunk_data.clone();
    let chunk_hex = chunk_id.to_hex();

    let bucket_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let bucket_addr = bucket_listener.local_addr().unwrap();
    let bucket_base_url = format!("http://{}", bucket_addr);

    let chunk_hex_route = chunk_hex.clone();
    let bucket_app = axum::Router::new().route(
        "/chunks/{id}",
        axum::routing::get(
            move |axum::extract::Path(id): axum::extract::Path<String>| {
                let data = chunk_data_for_bucket.clone();
                let hit_count = Arc::clone(&bucket_hit_count_clone);
                let expected = chunk_hex_route.clone();
                async move {
                    if id == expected {
                        hit_count.fetch_add(1, Ordering::SeqCst);
                        (
                            axum::http::StatusCode::OK,
                            [(axum::http::header::CONTENT_TYPE, "application/octet-stream")],
                            data,
                        )
                            .into_response()
                    } else {
                        axum::http::StatusCode::NOT_FOUND.into_response()
                    }
                }
            },
        ),
    );
    tokio::spawn(async move {
        axum::serve(bucket_listener, bucket_app).await.unwrap();
    });
    tokio::time::sleep(tokio::time::Duration::from_millis(30)).await;

    // ── setup: main server with PresignOverrideBackend ───────────────────────
    // The inner LocalBackend shares the same .mediagit dir as server_odb, so
    // `exists` and `get` see the chunks written above.
    let inner = LocalBackend::new(&server_repo_mg).await.unwrap();
    let override_backend: Arc<dyn StorageBackend> =
        Arc::new(PresignOverrideBackend::new(inner, bucket_base_url.clone()));

    let (server_url, _handle) =
        start_test_server_with_backend(repos_tmp.path().to_path_buf(), repo, override_backend)
            .await;

    // ── setup: client download ODB (starts empty) ────────────────────────────
    let dl_mg = TempDir::new().unwrap();
    let dl_odb_dir = dl_mg.path().join(".mediagit");
    tokio::fs::create_dir_all(dl_odb_dir.join("objects"))
        .await
        .unwrap();
    let dl_odb = open_odb(&dl_odb_dir).await;

    // Upload manifest to server so the client can download it.
    // (server_odb already has the manifest; the server reads from storage_backend
    //  which is our override_backend → inner LocalBackend at server_repo_mg.)
    // The manifest is stored in server_repo_mg/manifests/<oid> via put_manifest.
    // download_manifest via ProtocolClient hits GET /:repo/manifests/:oid.
    // That handler calls storage.get("manifests/<oid>") — inner LocalBackend serves it.

    // ── upload the manifest bytes into the server's storage via proxy ────────
    // The manifest was already written by `make_single_chunk_manifest` into
    // server_odb which shares inner LocalBackend at server_repo_mg.
    // Manifest reads go through the handlers' get_or_init_storage which hits
    // our cached override backend, so we are good.

    // ── run download ─────────────────────────────────────────────────────────
    // We need a manifest OID that the server knows about.
    // Re-create a matching file_oid and use it with download_chunked_objects.
    let file_oid = Oid::hash(b"e2e-download-test-file");

    let protocol = ProtocolClient::new(format!("{}/{}", server_url, repo));
    let downloaded = protocol
        .download_chunked_objects(&dl_odb, &[file_oid], |_, _, _| {})
        .await
        .expect("download_chunked_objects should succeed");

    assert_eq!(downloaded, 1, "one chunk should have been downloaded");

    // The chunk must be present in the local ODB.
    assert!(
        dl_odb.chunk_exists(&chunk_id).await.unwrap_or(false),
        "chunk must be in the local ODB after download"
    );

    // The mock bucket must have been hit exactly once (direct presigned path).
    let hits = bucket_hit_count.load(Ordering::SeqCst);
    assert_eq!(
        hits, 1,
        "mock bucket should have been hit exactly once (direct download path), got {hits}"
    );
}

// ── Test 2 ────────────────────────────────────────────────────────────────────
/// Fallback to proxy: backend returns `Ok(None)` for `presign_get`.
///
/// With `LocalBackend`, `POST /chunks/download-urls` returns `null` for every
/// chunk. The client falls through to `GET /chunks/{hex}` (proxy route).
/// The chunk must still be retrieved correctly.
#[tokio::test]
async fn presigned_download_fallback_to_proxy() {
    let repos_tmp = TempDir::new().unwrap();
    let client_tmp = TempDir::new().unwrap();
    let repo = "dl-proxy-repo";

    let server_repo_mg = repos_tmp.path().join(repo).join(".mediagit");
    tokio::fs::create_dir_all(server_repo_mg.join("objects"))
        .await
        .unwrap();
    tokio::fs::create_dir_all(server_repo_mg.join("refs/heads"))
        .await
        .unwrap();

    // Push a chunk + manifest into the server-side storage via the proxy PUT.
    let http_client = reqwest::Client::new();
    let (server_url, _handle) = start_test_server(repos_tmp.path().to_path_buf()).await;

    let chunk_data = vec![0xBBu8; 768];
    let chunk_id = Oid::hash(&chunk_data);

    // PUT the raw chunk bytes via the proxy route.
    proxy_put_chunk(&http_client, &server_url, repo, &chunk_id, &chunk_data).await;

    // PUT the manifest via the server's manifest upload endpoint.
    let file_oid = Oid::hash(b"e2e-proxy-fallback-file");
    let manifest = ChunkManifest {
        chunks: vec![ChunkRef {
            id: chunk_id,
            offset: 0,
            size: chunk_data.len(),
            chunk_type: ChunkType::Generic,
            codec_hint: Default::default(),
        }],
        total_size: chunk_data.len() as u64,
        filename: Some("proxy-test.bin".to_string()),
    };
    // Serialize and PUT the manifest (magic-enveloped, as real clients do).
    let manifest_bytes = manifest.to_bytes().unwrap();
    let manifest_url = format!("{}/{}/manifests/{}", server_url, repo, file_oid.to_hex());
    let resp = http_client
        .put(&manifest_url)
        .body(manifest_bytes)
        .send()
        .await
        .unwrap();
    assert!(
        resp.status().is_success(),
        "manifest PUT failed: {}",
        resp.status()
    );

    // ── client download ODB starts empty ────────────────────────────────────
    let dl_mg = client_tmp.path().join(".mediagit");
    tokio::fs::create_dir_all(dl_mg.join("objects"))
        .await
        .unwrap();
    let dl_odb = open_odb(&dl_mg).await;

    let protocol = ProtocolClient::new(format!("{}/{}", server_url, repo));
    let downloaded = protocol
        .download_chunked_objects(&dl_odb, &[file_oid], |_, _, _| {})
        .await
        .expect("proxy-fallback download should succeed");

    assert_eq!(
        downloaded, 1,
        "one chunk should have been downloaded via proxy"
    );

    assert!(
        dl_odb.chunk_exists(&chunk_id).await.unwrap_or(false),
        "chunk must be in the local ODB after proxy-fallback download"
    );

    // Verify the data round-trips correctly.
    let got = dl_odb.get_chunk(&chunk_id).await.unwrap();
    assert_eq!(
        got, chunk_data,
        "chunk data must match after proxy download"
    );
}

// ── Test 3 ────────────────────────────────────────────────────────────────────
/// 403 fallback: the presigned GET URL returns HTTP 403 Forbidden.
///
/// The client must detect the non-2xx response, log a debug message, and fall
/// back to the server-proxied `GET /chunks/{hex}` route. The chunk must still
/// be retrieved correctly via the fallback path.
#[tokio::test]
async fn presigned_download_403_falls_back_to_proxy() {
    // ── setup: server-side repo dirs ─────────────────────────────────────────
    let repos_tmp = TempDir::new().unwrap();
    let repo = "dl-403-repo";

    let server_repo_mg = repos_tmp.path().join(repo).join(".mediagit");
    tokio::fs::create_dir_all(server_repo_mg.join("objects"))
        .await
        .unwrap();
    tokio::fs::create_dir_all(server_repo_mg.join("refs/heads"))
        .await
        .unwrap();

    // ── setup: chunk + manifest in server storage ────────────────────────────
    let chunk_data = vec![0xDDu8; 256];
    let chunk_id = Oid::hash(&chunk_data);

    let server_odb = open_odb(&server_repo_mg).await;
    server_odb
        .put_compressed_chunk(&chunk_id, &chunk_data)
        .await
        .unwrap();

    let file_oid = Oid::hash(b"e2e-403-fallback-file");
    let manifest = ChunkManifest {
        chunks: vec![ChunkRef {
            id: chunk_id,
            offset: 0,
            size: chunk_data.len(),
            chunk_type: ChunkType::Generic,
            codec_hint: Default::default(),
        }],
        total_size: chunk_data.len() as u64,
        filename: Some("four-oh-three.bin".to_string()),
    };
    server_odb.put_manifest(&file_oid, &manifest).await.unwrap();

    // ── setup: mock "bucket" that always returns 403 ─────────────────────────
    let forbidden_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let forbidden_addr = forbidden_listener.local_addr().unwrap();
    let forbidden_base_url = format!("http://{}", forbidden_addr);

    let forbidden_app = axum::Router::new().route(
        "/chunks/{_id}",
        axum::routing::get(|| async { axum::http::StatusCode::FORBIDDEN }),
    );
    tokio::spawn(async move {
        axum::serve(forbidden_listener, forbidden_app)
            .await
            .unwrap();
    });
    tokio::time::sleep(tokio::time::Duration::from_millis(30)).await;

    // ── setup: main server with PresignOverrideBackend pointing at 403 server ─
    // The inner LocalBackend is rooted at server_repo_mg so the proxy GET
    // route can serve the chunk from local storage as fallback.
    let inner = LocalBackend::new(&server_repo_mg).await.unwrap();
    let override_backend: Arc<dyn StorageBackend> =
        Arc::new(PresignOverrideBackend::new(inner, forbidden_base_url));

    let (server_url, _handle) =
        start_test_server_with_backend(repos_tmp.path().to_path_buf(), repo, override_backend)
            .await;

    // ── setup: client download ODB starts empty ───────────────────────────────
    let dl_tmp = TempDir::new().unwrap();
    let dl_mg = dl_tmp.path().join(".mediagit");
    tokio::fs::create_dir_all(dl_mg.join("objects"))
        .await
        .unwrap();
    let dl_odb = open_odb(&dl_mg).await;

    // ── run download ─────────────────────────────────────────────────────────
    // The client will:
    //   1. POST /chunks/download-urls → receives presigned URL pointing at
    //      the 403 server.
    //   2. Attempt direct GET → 403 received → fallback triggered.
    //   3. GET /chunks/{hex} via proxy → chunk bytes returned → ODB written.
    let protocol = ProtocolClient::new(format!("{}/{}", server_url, repo));
    let downloaded = protocol
        .download_chunked_objects(&dl_odb, &[file_oid], |_, _, _| {})
        .await
        .expect("download should succeed via proxy fallback after 403");

    assert_eq!(
        downloaded, 1,
        "one chunk should have been downloaded via fallback"
    );

    assert!(
        dl_odb.chunk_exists(&chunk_id).await.unwrap_or(false),
        "chunk must be in the local ODB after 403-fallback download"
    );

    // Verify the data round-trips correctly.
    let got = dl_odb.get_chunk(&chunk_id).await.unwrap();
    assert_eq!(
        got, chunk_data,
        "chunk data must match after 403-fallback download"
    );
}
