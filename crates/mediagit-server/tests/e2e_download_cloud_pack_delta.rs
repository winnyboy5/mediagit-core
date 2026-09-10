// SPDX-License-Identifier: BUSL-1.1
// Copyright (C) 2025-2026 Aswin Krishnamoorthy

//! Regression test (qa-suite campaign 20260715-201716, persona_gamedev G5):
//! `GET /{repo}/files/{path}` must be able to stream a chunked file whose
//! chunk-delta BASE chunk lives only inside a Track F cloud pack
//! (`packs/<pack_oid>`, no `.pack` extension) rather than as a loose
//! `chunks/<id>` object. Before the fix, `download_file_by_path`
//! (handlers/browse.rs) streamed chunks via `ObjectDatabase::get_chunk`,
//! whose pack fallback (`list_pack_files`) filtered storage keys under
//! `packs/` to `.pack`-suffixed names only — the legacy `gc --repack`
//! format — silently excluding cloud-pack objects. The download died
//! mid-stream with "Failed to read base chunk ...: not found loose or in
//! packs", surfacing to the client as "error decoding response body: end
//! of file before message length reached", even though `clone` (which
//! never asks the server to reconstruct bytes itself — the client fetches
//! packs directly) worked fine on the same repo.
//!
//! Companion to `mediagit-versioning/tests/cloud_pack_chunk_delta_download.rs`
//! (same root cause, exercised at the `ObjectDatabase::get_chunk` level).
//! This test drives it through the actual HTTP handler.

use std::path::PathBuf;
use std::sync::Arc;
use tempfile::TempDir;
use tokio::net::TcpListener;

use mediagit_compression::{SmartCompressor, TypeAwareCompressor};
use mediagit_storage::{LocalBackend, StorageBackend};
use mediagit_versioning::chunking::{ChunkManifest, ChunkRef, ChunkType};
use mediagit_versioning::{
    Commit, Delta, DeltaEncoder, FileMode, ObjectDatabase, ObjectType, Oid, PackKind, Ref,
    RefDatabase, Signature, StreamingPackWriter, Tree, TreeEntry,
};

type CompObjectType = mediagit_compression::ObjectType;

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

/// Mirrors `e2e_nested_path_browse.rs::open_server_odb`: the production
/// server wraps storage in `NamespacedBackend` (layout v2), so seeding must
/// go through the same wrap or the server reads from a different physical
/// path than the test writes to.
async fn open_server_storage(repo_root: &std::path::Path) -> Arc<dyn StorageBackend> {
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
    Arc::new(namespaced)
}

#[tokio::test]
async fn download_file_reconstructs_chunk_delta_whose_base_is_only_in_a_cloud_pack() {
    let server_temp = TempDir::new().unwrap();
    let server_repos = server_temp.path().join("repos");
    let server_repo = server_repos.join("g5-repo");
    let server_mediagit = server_repo.join(".mediagit");
    tokio::fs::create_dir_all(server_mediagit.join("objects"))
        .await
        .unwrap();
    tokio::fs::create_dir_all(server_mediagit.join("refs/heads"))
        .await
        .unwrap();

    let storage = open_server_storage(&server_repo).await;
    let odb = ObjectDatabase::with_smart_compression(storage.clone(), 100);
    let smart = SmartCompressor::new();

    // ── Base chunk: pushed via cloud packs (MEDIAGIT_CLOUD_PACKS default
    // ON) — lives only inside a pack object, never as a loose chunk. ─────
    let base_payload = b"HERO.GLB BASE CHUNK: binary mesh data placeholder bytes. ".repeat(64);
    let base_id = Oid::hash(&base_payload);
    let compressed_base = smart
        .compress_typed(&base_payload, CompObjectType::Unknown)
        .expect("compress base");

    let temp_dir = tempfile::TempDir::new().unwrap();
    let mut writer = StreamingPackWriter::new_open_ended(PackKind::CloudObject, temp_dir.path())
        .await
        .expect("open cloud pack writer");
    writer
        .write_object(base_id, ObjectType::Blob, &compressed_base)
        .await
        .expect("write base chunk into pack");
    let result = writer.finalize_cloud().await.expect("finalize cloud pack");
    let pack_bytes = tokio::fs::read(&result.temp_path).await.unwrap();
    let pack_oid_hex: String = result
        .pack_oid
        .iter()
        .map(|b| format!("{:02x}", b))
        .collect();
    storage
        .put(&format!("packs/{}", pack_oid_hex), &pack_bytes)
        .await
        .expect("upload cloud pack object");

    // ── Second chunk: a real delta against the base. ──────────────────────
    let leaf_payload = {
        let mut v = base_payload.clone();
        v.extend_from_slice(b" -- appended tail bytes for the second chunk");
        v
    };
    let delta: Delta = DeltaEncoder::encode(&base_payload, &leaf_payload);
    let delta_id = Oid::hash(&leaf_payload);
    let compressed_delta = smart
        .compress_typed(&delta.to_bytes(), CompObjectType::Unknown)
        .expect("compress delta");
    odb.write_chunk_delta(&delta_id, &base_id, &compressed_delta)
        .await
        .expect("write chunk delta");

    // ── Manifest: file = base chunk followed by the delta-reconstructed
    // chunk, exactly how a real chunked upload lays out a manifest. ───────
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
                size: leaf_payload.len(),
                chunk_type: ChunkType::Generic,
                codec_hint: Default::default(),
            },
        ],
        total_size: (base_payload.len() + leaf_payload.len()) as u64,
        filename: Some("hero.glb".to_string()),
    };
    let blob_oid = Oid::hash(b"g5-hero-glb-manifest-marker");
    odb.put_manifest(&blob_oid, &manifest)
        .await
        .expect("put manifest");

    // ── Commit a flat tree entry pointing at the chunked blob, and a ref. ─
    let mut tree = Tree::new();
    tree.add_entry(TreeEntry::new(
        "hero.glb".to_string(),
        FileMode::Regular,
        blob_oid,
    ));
    let tree_oid = tree.write(&odb).await.unwrap();
    let author = Signature::now("Test".to_string(), "t@e".to_string());
    let commit = Commit::new(tree_oid, author.clone(), author, "add hero.glb".to_string());
    let commit_oid = commit.write(&odb).await.unwrap();
    let refdb = RefDatabase::new(server_mediagit.clone());
    refdb
        .write(&Ref::new_direct("refs/heads/main".to_string(), commit_oid))
        .await
        .unwrap();

    // ── The actual regression: GET /files/hero.glb must stream the full,
    // byte-exact reconstructed file, not die mid-stream. ──────────────────
    let (base_url, _handle) = start_test_server(server_repos).await;
    mediagit_protocol::ensure_crypto_provider();
    let client = reqwest::Client::new();
    let resp = client
        .get(format!("{}/g5-repo/files/hero.glb?ref=main", base_url))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200, "download must return 200");
    let body = resp.bytes().await.expect(
        "response body must stream to completion — regression: died mid-stream on cloud-packed base chunk",
    );

    let mut expected = base_payload.clone();
    expected.extend_from_slice(&leaf_payload);
    assert_eq!(
        &body[..],
        &expected[..],
        "downloaded bytes must match base chunk + delta-reconstructed chunk"
    );
}
