// SPDX-License-Identifier: BUSL-1.1
// Copyright (C) 2025-2026 Aswin Krishnamoorthy

//! SEC-5 fix round 1 (R2): the server must rebuild a pushed chunk delta before
//! it stores it. A delta that does not rebuild its chunk is refused, and a
//! delta whose base is not stored yet is answered "try again later" so the
//! client can send the base first. Nothing unverified becomes visible.

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

fn delta_payload(base: &[u8], target: &[u8]) -> Vec<u8> {
    mediagit_compression::ZlibCompressor::default_level()
        .compress(&mediagit_versioning::DeltaEncoder::encode(base, target).to_bytes())
        .unwrap()
}

struct Fixture {
    url: String,
    repo_mediagit: PathBuf,
    base: Vec<u8>,
    t1: Vec<u8>,
    t2: Vec<u8>,
    _dirs: (TempDir, TempDir),
}

/// A server that holds only the base chunk of a two-link chain base -> t1 -> t2.
async fn fixture(name: &str) -> Fixture {
    mediagit_test_utils::set_var("MEDIAGIT_CLOUD_PACKS", "0");
    let client_temp = TempDir::new().unwrap();
    let cm = client_temp.path().join(".mediagit");
    tokio::fs::create_dir_all(cm.join("objects")).await.unwrap();
    let storage: Arc<dyn StorageBackend> = Arc::new(LocalBackend::new(&cm).await.unwrap());
    let odb = ObjectDatabase::new(storage, 100);
    let base = b"base chunk bytes, long enough to delta against. ".repeat(64);
    let t1 = [base.as_slice(), b" version two tail".as_slice()].concat();
    let t2 = [t1.as_slice(), b" version three tail".as_slice()].concat();
    let base_id = Oid::hash(&base);
    odb.write_full_chunk(&base_id, &base).await.unwrap();
    let obj = Oid::hash(b"obj-base");
    odb.put_manifest(
        &obj,
        &ChunkManifest {
            chunks: vec![ChunkRef {
                id: base_id,
                offset: 0,
                size: base.len(),
                chunk_type: ChunkType::Generic,
                codec_hint: Default::default(),
            }],
            total_size: base.len() as u64,
            filename: Some("f.bin".into()),
        },
    )
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
    let url = format!("{}/{name}", start_test_server(repos).await);
    ProtocolClient::new(url.clone())
        .upload_chunked_objects(&odb, &[obj], |_, _| {})
        .await
        .expect("push base");
    Fixture {
        url,
        repo_mediagit: repo.join(".mediagit"),
        base,
        t1,
        t2,
        _dirs: (client_temp, server_temp),
    }
}

async fn put_delta(f: &Fixture, id: &Oid, base: &Oid, body: Vec<u8>) -> reqwest::StatusCode {
    reqwest::Client::new()
        .put(format!("{}/chunk-deltas/{}", f.url, id.to_hex()))
        .header("x-mediagit-delta-base", base.to_hex())
        .body(body)
        .send()
        .await
        .unwrap()
        .status()
}

async fn server_has_delta(f: &Fixture, id: &Oid) -> bool {
    let inner: Arc<dyn StorageBackend> =
        Arc::new(LocalBackend::new(&f.repo_mediagit).await.unwrap());
    let ns = mediagit_storage::sanitize_namespace(
        &f.repo_mediagit
            .parent()
            .unwrap()
            .file_name()
            .unwrap()
            .to_string_lossy(),
    );
    let s = mediagit_storage::NamespacedBackend::new(inner, ns).unwrap();
    let k = format!("chunk-deltas/{}", id.to_hex());
    s.exists(&k).await.unwrap() || s.exists(&format!("{k}.meta")).await.unwrap()
}

#[tokio::test]
async fn a_delta_that_rebuilds_the_wrong_bytes_is_refused() {
    let f = fixture("verify-a").await;
    let (base_id, id) = (Oid::hash(&f.base), Oid::hash(&f.t1));
    let evil = [f.base.as_slice(), b" EVIL".as_slice()].concat();
    let status = put_delta(&f, &id, &base_id, delta_payload(&f.base, &evil)).await;
    assert_eq!(status.as_u16(), 422, "a bad delta must be refused");
    assert!(!server_has_delta(&f, &id).await, "bad delta went live");
}

#[tokio::test]
async fn a_good_delta_is_stored_and_a_repeat_is_accepted() {
    let f = fixture("verify-b").await;
    let (base_id, id) = (Oid::hash(&f.base), Oid::hash(&f.t1));
    let body = delta_payload(&f.base, &f.t1);
    assert_eq!(
        put_delta(&f, &id, &base_id, body.clone()).await.as_u16(),
        201
    );
    assert!(server_has_delta(&f, &id).await);
    assert!(put_delta(&f, &id, &base_id, body).await.is_success());
}

/// d2 is sent before d1 (its base). The server says "base missing" (424) and
/// stores nothing; after d1 is in, the same d2 is accepted.
#[tokio::test]
async fn a_delta_before_its_base_waits_then_is_accepted() {
    let f = fixture("verify-c").await;
    let (base_id, d1, d2) = (Oid::hash(&f.base), Oid::hash(&f.t1), Oid::hash(&f.t2));
    let d2_body = delta_payload(&f.t1, &f.t2);
    assert_eq!(put_delta(&f, &d2, &d1, d2_body.clone()).await.as_u16(), 424);
    assert!(
        !server_has_delta(&f, &d2).await,
        "unverifiable delta went live"
    );
    assert_eq!(
        put_delta(&f, &d1, &base_id, delta_payload(&f.base, &f.t1))
            .await
            .as_u16(),
        201
    );
    assert_eq!(put_delta(&f, &d2, &d1, d2_body).await.as_u16(), 201);
}

/// A push whose deltas depend on each other (a chain inside one manifest) is
/// sent unordered. Links that reach the server before their base wait and are
/// sent again; every link ends up stored as a delta, and the chain rebuilds.
#[tokio::test]
async fn pushing_a_delta_chain_lands_every_link_as_a_delta() {
    mediagit_test_utils::set_var("MEDIAGIT_CLOUD_PACKS", "0");
    let client_temp = TempDir::new().unwrap();
    let cm = client_temp.path().join(".mediagit");
    tokio::fs::create_dir_all(cm.join("objects")).await.unwrap();
    let storage: Arc<dyn StorageBackend> = Arc::new(LocalBackend::new(&cm).await.unwrap());
    let odb = ObjectDatabase::new(storage, 100);
    let mut versions = vec![b"base chunk bytes, long enough to delta against. ".repeat(64)];
    for i in 0..6 {
        let mut next = versions[i].clone();
        next.extend_from_slice(format!(" tail {i}").as_bytes());
        versions.push(next);
    }
    let ids: Vec<Oid> = versions.iter().map(|v| Oid::hash(v)).collect();
    odb.write_full_chunk(&ids[0], &versions[0]).await.unwrap();
    for i in 1..versions.len() {
        odb.write_chunk_delta(
            &ids[i],
            &ids[i - 1],
            &delta_payload(&versions[i - 1], &versions[i]),
        )
        .await
        .unwrap();
    }
    let mut offset = 0u64;
    let chunks = ids
        .iter()
        .zip(&versions)
        .map(|(id, v)| {
            let r = ChunkRef {
                id: *id,
                offset,
                size: v.len(),
                chunk_type: ChunkType::Generic,
                codec_hint: Default::default(),
            };
            offset += v.len() as u64;
            r
        })
        .collect();
    let obj = Oid::hash(b"obj-chain");
    odb.put_manifest(
        &obj,
        &ChunkManifest {
            chunks,
            total_size: offset,
            filename: Some("f.bin".into()),
        },
    )
    .await
    .unwrap();

    let server_temp = TempDir::new().unwrap();
    let repos = server_temp.path().join("repos");
    let repo = repos.join("verify-d");
    tokio::fs::create_dir_all(repo.join(".mediagit/objects"))
        .await
        .unwrap();
    tokio::fs::create_dir_all(repo.join(".mediagit/refs/heads"))
        .await
        .unwrap();
    let url = format!("{}/verify-d", start_test_server(repos).await);
    ProtocolClient::new(url.clone())
        .upload_chunked_objects(&odb, &[obj], |_, _| {})
        .await
        .expect("push chain");

    let f = Fixture {
        url,
        repo_mediagit: repo.join(".mediagit"),
        base: versions[0].clone(),
        t1: versions[1].clone(),
        t2: versions[2].clone(),
        _dirs: (client_temp, server_temp),
    };
    for id in &ids[1..] {
        assert!(server_has_delta(&f, id).await, "{id} missing as a delta");
    }
}
