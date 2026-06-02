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

//! Regression test: push must ship chunk-deltas as deltas, not rematerialized
//! full chunks.
//!
//! Prior to the fix, `upload_chunked_objects` routed every missing chunk
//! through `get_compressed_chunk`, which rematerialized any local chunk-delta
//! into a full compressed chunk and PUT it at `/chunks/:id`. The server never
//! received deltas, so a subsequent clone always saw the inflated full chunks
//! and reported near-zero storage savings (e.g. 2.1 % on PSD files that were
//! 21 % compressed locally). This test pins the behavior by driving the
//! protocol client directly and asserting server on-disk layout.

use std::path::PathBuf;
use std::sync::Arc;
use tempfile::TempDir;
use tokio::net::TcpListener;

use mediagit_protocol::ProtocolClient;
use mediagit_storage::{LocalBackend, StorageBackend};
use mediagit_versioning::chunking::{ChunkManifest, ChunkRef, ChunkType};
use mediagit_versioning::{ObjectDatabase, Oid};

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

/// Construct a client-side repo whose manifest references one full chunk
/// (the base) and one delta chunk (targeting that base). No actual delta
/// encoding is performed — the test only checks how bytes travel, not how
/// they decode.
async fn seed_client_with_delta(
    client_mediagit: &std::path::Path,
) -> (Oid, Oid, Oid, Vec<u8>, Vec<u8>) {
    let odb = open_odb(client_mediagit).await;

    // Use deterministic payloads so the OIDs are reproducible.
    let base_payload = b"BASE-CHUNK: the quick brown fox jumps over the lazy dog".to_vec();
    let delta_payload =
        b"DELTA-AGAINST-BASE: small diff against the base chunk (opaque to the server)".to_vec();

    let base_id = Oid::hash(&base_payload);
    let delta_id = Oid::hash(&delta_payload);

    odb.put_compressed_chunk(&base_id, &base_payload)
        .await
        .expect("put base chunk");
    odb.write_chunk_delta(&delta_id, &base_id, &delta_payload)
        .await
        .expect("write local chunk-delta");

    let manifest = ChunkManifest {
        chunks: vec![
            ChunkRef {
                id: base_id,
                offset: 0,
                size: base_payload.len(),
                chunk_type: ChunkType::Generic,
                codec_hint: Default::default(),
            },
            ChunkRef {
                id: delta_id,
                offset: base_payload.len() as u64,
                size: delta_payload.len(),
                chunk_type: ChunkType::Generic,
                codec_hint: Default::default(),
            },
        ],
        total_size: (base_payload.len() + delta_payload.len()) as u64,
        filename: Some("fixture.bin".to_string()),
    };

    let blob_oid = Oid::hash(b"fixture-manifest-blob-oid");
    odb.put_manifest(&blob_oid, &manifest)
        .await
        .expect("put manifest");

    (blob_oid, base_id, delta_id, base_payload, delta_payload)
}

/// Asserts that after push, the server stores the full chunk under `chunks/`
/// and the delta under `chunk-deltas/` with its `.meta` sidecar — the exact
/// shape the clone path's `/chunk-deltas/check` endpoint walks.
#[tokio::test]
async fn push_preserves_chunk_deltas_on_server() {
    // Cloud packs (MEDIAGIT_CLOUD_PACKS, default ON) bundle full chunks into a
    // single pack object instead of writing each at chunks/<id>. This test pins
    // the per-chunk on-disk layout of the chunk-delta path — chunks/<base> plus
    // chunk-deltas/<delta>{,.meta} — which the clone-side /chunk-deltas/check
    // endpoint walks, so it must run with packing disabled. Packed delta
    // preservation is covered separately by the F-series cloud-pack tests.
    std::env::set_var("MEDIAGIT_CLOUD_PACKS", "0");

    // ── Client ──────────────────────────────────────────────────────────
    let client_temp = TempDir::new().unwrap();
    let client_mediagit = client_temp.path().join(".mediagit");
    tokio::fs::create_dir_all(client_mediagit.join("objects"))
        .await
        .unwrap();
    let (blob_oid, base_id, delta_id, _base_payload, delta_payload) =
        seed_client_with_delta(&client_mediagit).await;

    // ── Server ──────────────────────────────────────────────────────────
    let server_temp = TempDir::new().unwrap();
    let server_repos = server_temp.path().join("repos");
    let server_repo = server_repos.join("delta-push-test");
    let server_mediagit = server_repo.join(".mediagit");
    tokio::fs::create_dir_all(server_mediagit.join("objects"))
        .await
        .unwrap();
    tokio::fs::create_dir_all(server_mediagit.join("refs/heads"))
        .await
        .unwrap();

    let (base_url, _server_handle) = start_test_server(server_repos.clone()).await;
    let client = ProtocolClient::new(format!("{}/delta-push-test", base_url));

    // ── Push ────────────────────────────────────────────────────────────
    let client_odb = open_odb(&client_mediagit).await;
    let uploaded = client
        .upload_chunked_objects(&client_odb, &[blob_oid], |_, _| {})
        .await
        .expect("upload_chunked_objects");
    assert_eq!(uploaded, 2, "expected both chunks to be uploaded");

    // ── Assert server layout ────────────────────────────────────────────
    let server_storage: Arc<dyn StorageBackend> =
        Arc::new(LocalBackend::new(&server_mediagit).await.unwrap());

    let base_key = format!("chunks/{}", base_id.to_hex());
    assert!(
        server_storage.exists(&base_key).await.unwrap(),
        "base chunk missing on server at {}",
        base_key
    );

    let delta_full_key = format!("chunks/{}", delta_id.to_hex());
    assert!(
        !server_storage
            .exists(&delta_full_key)
            .await
            .unwrap_or(false),
        "delta id should NOT be stored as a full chunk — regression: push rematerialized the delta"
    );

    let delta_key = format!("chunk-deltas/{}", delta_id.to_hex());
    let on_disk_delta = server_storage
        .get(&delta_key)
        .await
        .expect("chunk-delta payload missing on server");
    assert_eq!(
        on_disk_delta, delta_payload,
        "server-stored chunk-delta payload should be byte-identical to the pushed bytes"
    );

    let meta_key = format!("chunk-deltas/{}.meta", delta_id.to_hex());
    let meta_bytes = server_storage
        .get(&meta_key)
        .await
        .expect("chunk-delta meta missing on server");
    let meta_str = std::str::from_utf8(&meta_bytes).unwrap();
    assert_eq!(
        meta_str.trim(),
        format!("base:{}", base_id.to_hex()),
        "chunk-delta meta must record the base OID"
    );

    // ── Idempotence: a second push must not rematerialize (server's
    // /chunks/check now treats delta-form as present, so the client
    // should skip re-uploading everything).
    let uploaded_again = client
        .upload_chunked_objects(&client_odb, &[blob_oid], |_, _| {})
        .await
        .expect("second upload_chunked_objects");
    assert_eq!(
        uploaded_again, 0,
        "re-push should be a no-op — chunk-delta must count as 'exists' in /chunks/check"
    );

    // And confirm the server didn't sprout a full-chunk form of the delta id.
    assert!(
        !server_storage
            .exists(&delta_full_key)
            .await
            .unwrap_or(false),
        "re-push must not create chunks/<delta_id> — would duplicate storage"
    );
}
