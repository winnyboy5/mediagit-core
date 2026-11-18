// SPDX-License-Identifier: BUSL-1.1
// Copyright (C) 2025-2026 Aswin Krishnamoorthy

//! SEC-5 fix round 1: pull and clone must not depend on the order in which
//! chunk bases and chunk deltas arrive.
//!
//! `write_chunk_delta` rebuilds a delta before it stores it, so it needs the
//! base to be readable. Objects and chunks arrive in no fixed order, so a
//! delta whose base has not arrived yet must be set aside and written once the
//! base is there. It must never become visible unverified, and it must never
//! be fetched as a full chunk (the server holds it only as a delta).

use std::path::PathBuf;
use std::sync::Arc;
use tempfile::TempDir;
use tokio::net::TcpListener;

use mediagit_compression::Compressor;
use mediagit_protocol::ProtocolClient;
use mediagit_storage::{LocalBackend, StorageBackend};
use mediagit_versioning::chunking::{ChunkManifest, ChunkRef, ChunkType};
use mediagit_versioning::{ObjectDatabase, Oid};

async fn start_test_server(repos_dir: PathBuf) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base_url = format!("http://{}", listener.local_addr().unwrap());
    let state = Arc::new(mediagit_server::AppState::new(repos_dir));
    let app = mediagit_server::create_router(state);
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
    base_url
}

async fn open_odb(mediagit_dir: &std::path::Path) -> ObjectDatabase {
    tokio::fs::create_dir_all(mediagit_dir.join("objects"))
        .await
        .unwrap();
    let storage: Arc<dyn StorageBackend> = Arc::new(LocalBackend::new(mediagit_dir).await.unwrap());
    ObjectDatabase::new(storage, 100)
}

fn delta_payload(base: &[u8], target: &[u8]) -> Vec<u8> {
    mediagit_compression::ZlibCompressor::default_level()
        .compress(&mediagit_versioning::DeltaEncoder::encode(base, target).to_bytes())
        .unwrap()
}

fn manifest_of(chunks: &[(Oid, usize)]) -> ChunkManifest {
    let mut offset = 0u64;
    let refs = chunks
        .iter()
        .map(|(id, size)| {
            let r = ChunkRef {
                id: *id,
                offset,
                size: *size,
                chunk_type: ChunkType::Generic,
                codec_hint: Default::default(),
            };
            offset += *size as u64;
            r
        })
        .collect();
    ChunkManifest {
        chunks: refs,
        total_size: offset,
        filename: Some("fixture.bin".to_string()),
    }
}

struct Fixture {
    /// Manifest oids: [base], [d1 = delta on base], [d2 = delta on d1].
    base_obj: Oid,
    d1_obj: Oid,
    d2_obj: Oid,
    d2_id: Oid,
    d2_bytes: Vec<u8>,
    d1_id: Oid,
    client: ProtocolClient,
    _dirs: (TempDir, TempDir),
}

/// A server holding a base chunk and a two-link delta chain, each in its own
/// object (versions of one file). The server stores d1 and d2 only as deltas.
async fn fixture(name: &str) -> Fixture {
    mediagit_test_utils::set_var("MEDIAGIT_CLOUD_PACKS", "0");
    // Sequential manifest fetch keeps the object order the test asks for.
    mediagit_test_utils::set_var("MEDIAGIT_PULL_PIPELINE", "0");
    // Parallel tests would share the staging file name of the same chunk.
    mediagit_test_utils::set_var("MEDIAGIT_STREAM_CHUNK_TO_DISK", "0");

    let client_temp = TempDir::new().unwrap();
    let odb = open_odb(&client_temp.path().join(".mediagit")).await;
    let base = b"base chunk bytes, long enough to delta against. ".repeat(64);
    let t1 = [base.as_slice(), b" version two tail".as_slice()].concat();
    let t2 = [t1.as_slice(), b" version three tail".as_slice()].concat();
    let (base_id, d1_id, d2_id) = (Oid::hash(&base), Oid::hash(&t1), Oid::hash(&t2));
    odb.write_full_chunk(&base_id, &base).await.unwrap();
    odb.write_chunk_delta(&d1_id, &base_id, &delta_payload(&base, &t1))
        .await
        .unwrap();
    odb.write_chunk_delta(&d2_id, &d1_id, &delta_payload(&t1, &t2))
        .await
        .unwrap();
    let base_obj = Oid::hash(b"obj-base");
    let d1_obj = Oid::hash(b"obj-d1");
    let d2_obj = Oid::hash(b"obj-d2");
    odb.put_manifest(&base_obj, &manifest_of(&[(base_id, base.len())]))
        .await
        .unwrap();
    odb.put_manifest(&d1_obj, &manifest_of(&[(d1_id, t1.len())]))
        .await
        .unwrap();
    odb.put_manifest(&d2_obj, &manifest_of(&[(d2_id, t2.len())]))
        .await
        .unwrap();

    let server_temp = TempDir::new().unwrap();
    let repos = server_temp.path().join("repos");
    let repo = repos.join(name);
    tokio::fs::create_dir_all(repo.join(".mediagit/objects"))
        .await
        .unwrap();
    tokio::fs::create_dir_all(repo.join(".mediagit/refs/heads"))
        .await
        .unwrap();
    let url = start_test_server(repos).await;
    let client = ProtocolClient::new(format!("{url}/{name}"));
    // One object per push, base first: the client ships a delta as a delta
    // only when its base is already on the server.
    for obj in [base_obj, d1_obj, d2_obj] {
        client
            .upload_chunked_objects(&odb, &[obj], |_, _| {})
            .await
            .expect("push fixture");
    }
    Fixture {
        base_obj,
        d1_obj,
        d2_obj,
        d2_id,
        d2_bytes: t2,
        d1_id,
        client,
        _dirs: (client_temp, server_temp),
    }
}

async fn fresh_clone_odb() -> (TempDir, ObjectDatabase) {
    let t = TempDir::new().unwrap();
    let odb = open_odb(&t.path().join(".mediagit")).await;
    (t, odb)
}

/// The clone keeps the chunk as a delta (the server's saving), and does not
/// hold a second, full copy of it.
fn stored_as_delta_only(dir: &TempDir, id: &Oid) -> bool {
    let root = dir.path().join(".mediagit");
    let hex = id.to_hex();
    let has = |kind: &str, suffix: &str| {
        root.join(kind)
            .join(&hex[0..2])
            .join(&hex[2..4])
            .join(format!("{hex}{suffix}"))
            .exists()
    };
    has("chunk-deltas", ".meta") && !has("chunks", "")
}

/// v2's delta arrives before v1's base (objects in the order a clone may see).
#[tokio::test]
async fn delta_object_before_its_base_object_still_pulls() {
    let f = fixture("order-a").await;
    let (t, odb) = fresh_clone_odb().await;
    f.client
        .download_chunked_objects(&odb, &[f.d1_obj, f.base_obj], |_, _, _| {})
        .await
        .expect("a delta that arrives before its base must still pull");
    assert!(
        stored_as_delta_only(&t, &f.d1_id),
        "d1 must be kept as a delta, not refetched as a full chunk"
    );
}

/// A chain delivered dependent-first: d2 (on d1), then d1 (on base), then base.
#[tokio::test]
async fn chain_delivered_dependent_first_pulls_and_rebuilds() {
    let f = fixture("order-b").await;
    let (t, odb) = fresh_clone_odb().await;
    f.client
        .download_chunked_objects(&odb, &[f.d2_obj, f.d1_obj, f.base_obj], |_, _, _| {})
        .await
        .expect("a dependent-first chain must still pull");
    assert!(stored_as_delta_only(&t, &f.d2_id), "d2 kept as a delta");
    assert!(stored_as_delta_only(&t, &f.d1_id), "d1 kept as a delta");
    let got = odb.get_chunk(&f.d2_id).await.expect("d2 readable");
    assert_eq!(got, f.d2_bytes, "d2 rebuilds to the exact bytes");
}

/// A delta the server holds that does not rebuild its chunk fails the pull
/// with a plain message, and nothing for that chunk becomes visible.
#[tokio::test]
async fn bad_delta_from_the_server_fails_the_pull_and_stays_invisible() {
    let f = fixture("order-c").await;
    // Corrupt d1 on the server: file a delta for other bytes under d1's id.
    let server_dir = f._dirs.1.path().join("repos/order-c/.mediagit");
    let inner: Arc<dyn StorageBackend> = Arc::new(LocalBackend::new(&server_dir).await.unwrap());
    let ns = mediagit_storage::NamespacedBackend::new(
        inner,
        mediagit_storage::sanitize_namespace("order-c"),
    )
    .unwrap();
    let base = b"base chunk bytes, long enough to delta against. ".repeat(64);
    let evil = [base.as_slice(), b" EVIL".as_slice()].concat();
    ns.put(
        &format!("chunk-deltas/{}", f.d1_id.to_hex()),
        &delta_payload(&base, &evil),
    )
    .await
    .unwrap();

    let (_t, odb) = fresh_clone_odb().await;
    let err = f
        .client
        .download_chunked_objects(&odb, &[f.d1_obj, f.base_obj], |_, _, _| {})
        .await
        .expect_err("a bad delta must fail the pull");
    let msg = format!("{err:#}");
    assert!(msg.contains("bad delta"), "plain message expected: {msg}");
    assert!(
        odb.get_chunk(&f.d1_id).await.is_err(),
        "the bad delta must not be readable"
    );
}
