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

//! DC-7/D5: an encrypted repository, pushed and read back, over a real server.
//!
//! Everything else in the encryption suite tests one layer. This is the only
//! place the whole thing runs: a client seals objects under a repo key,
//! escrows that key, pushes, and the server reads what arrived and gets the
//! original bytes.
//!
//! It is worth the setup because the failure it guards against is silent. A
//! verification path left holding a bare compressor does not error; it fails
//! to decrypt, calls the entry corrupt, and quarantines a pack that was fine.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use tempfile::TempDir;
use tokio::net::TcpListener;

use mediagit_protocol::ProtocolClient;
use mediagit_protocol::client::escrow::EscrowedKey;
use mediagit_security::encryption::EncryptionKey;
use mediagit_storage::{LocalBackend, StorageBackend};
use mediagit_versioning::{
    Commit, FileMode, ObjectDatabase, ObjectType, Oid, Ref, RefDatabase, Signature, Tree, TreeEntry,
};

const REPO_KEY: [u8; 32] = [0x5c; 32];
const SERVER_MASTER: [u8; 32] = [0xa7; 32];

fn key(bytes: &[u8]) -> EncryptionKey {
    EncryptionKey::from_bytes(bytes.to_vec()).expect("32 bytes is a key")
}

/// A server holding `master`, or none at all.
async fn start_server(repos_dir: PathBuf, master: Option<EncryptionKey>) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base_url = format!("http://{}", listener.local_addr().unwrap());
    let state = Arc::new(mediagit_server::AppState::new(repos_dir).with_encryption_master(master));
    let app = mediagit_server::create_router(state);
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
    base_url
}

/// Storage opened the way the server's own `build_storage_backend` does.
/// A raw `LocalBackend` would land objects at a different physical path than
/// the HTTP handlers read from.
async fn open_storage(repo_path: &Path) -> anyhow::Result<Arc<dyn StorageBackend>> {
    let inner: Arc<dyn StorageBackend> =
        Arc::new(LocalBackend::new(repo_path.join(".mediagit")).await?);
    let ns = mediagit_storage::sanitize_namespace(
        &repo_path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default(),
    );
    let namespaced = mediagit_storage::NamespacedBackend::new(inner, ns)?;
    let mut config = mediagit_config::Config::load(repo_path).await?;
    if config.repo_id.as_deref().unwrap_or("").trim().is_empty() {
        config.repo_id = Some(mediagit_storage::generate_repo_id());
        config.save(repo_path)?;
    }
    Ok(Arc::new(namespaced))
}

/// An ODB for `repo_path`, sealing under `k` if given.
async fn odb_at(repo_path: &Path, k: Option<EncryptionKey>) -> ObjectDatabase {
    tokio::fs::create_dir_all(repo_path.join(".mediagit/refs/heads"))
        .await
        .unwrap();
    let storage = open_storage(repo_path).await.unwrap();
    ObjectDatabase::with_smart_compression(storage, 1000).with_at_rest_key(k)
}

/// One commit holding one file, and the ref pointing at it.
async fn commit_one_file(
    repo_path: &Path,
    odb: &ObjectDatabase,
    content: &[u8],
) -> anyhow::Result<Oid> {
    let blob = odb.write(ObjectType::Blob, content).await?;
    let mut tree = Tree::new();
    tree.add_entry(TreeEntry::new("asset.bin".into(), FileMode::Regular, blob));
    let tree_oid = tree.write(odb).await?;
    let who = Signature::now("Test User".into(), "test@example.com".into());
    let commit = Commit::new(tree_oid, who.clone(), who, "seal me".into())
        .write(odb)
        .await?;

    let refdb = RefDatabase::new(repo_path.join(".mediagit"));
    refdb
        .write(&Ref::new_direct("refs/heads/main".into(), commit))
        .await?;
    refdb
        .write(&Ref::new_symbolic("HEAD".into(), "refs/heads/main".into()))
        .await?;
    Ok(commit)
}

/// Keys this census deliberately does not require to carry an envelope.
///
/// - `LAYOUT` is bookkeeping, not an object.
/// - `packs/` is a container: its **entries** are sealed individually, and the
///   framing around them has to stay readable or nothing could find an entry
///   by offset. `pack_entries_are_sealed_inside_a_plaintext_container` pins
///   that distinction rather than leaving it to this comment.
/// - `chunk-deltas/*.meta` is a **known gap**: the delta sidecars are written
///   and read across the client ODB, three server handlers, fsck and gc, and
///   one of them is an HTTP endpoint that stores a client-supplied body. That
///   is the ODB-bypass shape this codebase has shipped seven times, so it is
///   not being changed in the same release that introduces the rest of this.
///   It leaks which chunk deltas against which base -- shape, not content.
fn census_exempt(key: &str) -> bool {
    key.ends_with("LAYOUT") || key.starts_with("packs/") || key.ends_with(".meta")
}

/// (sealed, total) across the parts of this repository that must be sealed.
async fn seal_census(repo_path: &Path) -> (usize, usize) {
    let storage = open_storage(repo_path).await.unwrap();
    let keys = storage.list_objects("").await.unwrap_or_default();
    let mut total = 0;
    let mut sealed = 0;
    for k in keys {
        if census_exempt(&k) {
            continue;
        }
        if let Ok(bytes) = storage.get(&k).await {
            total += 1;
            if mediagit_security::envelope::is_sealed(&bytes) {
                sealed += 1;
            }
        }
    }
    (sealed, total)
}

fn push_main(commit: Oid) -> Vec<mediagit_protocol::RefUpdate> {
    vec![mediagit_protocol::RefUpdate {
        name: "refs/heads/main".into(),
        old_oid: None,
        new_oid: commit.to_hex(),
        delete: false,
    }]
}

#[tokio::test]
async fn an_encrypted_repository_survives_a_push_and_reads_back() {
    let server_temp = TempDir::new().unwrap();
    let client_temp = TempDir::new().unwrap();
    let server_repos = server_temp.path().join("repos");
    let server_repo = server_repos.join("secret-repo");
    let client_repo = client_temp.path().join("secret-repo");

    // The server's repo exists but is empty -- the state `repo create` leaves,
    // and the only state encryption may be enabled from.
    tokio::fs::create_dir_all(server_repo.join(".mediagit/refs/heads"))
        .await
        .unwrap();

    let base_url = start_server(server_repos.clone(), Some(key(&SERVER_MASTER))).await;
    let client = ProtocolClient::new(format!("{base_url}/secret-repo"));

    // --- client side: a keyed repository with real content
    let payload = b"a PSD nobody else gets to read".repeat(500);
    let odb = odb_at(&client_repo, Some(key(&REPO_KEY))).await;
    let commit = commit_one_file(&client_repo, &odb, &payload).await.unwrap();

    let (sealed, total) = seal_census(&client_repo).await;
    assert!(total > 0, "the client wrote nothing");
    assert_eq!(sealed, total, "every client object must be sealed");

    // --- escrow, then push. Escrow first, always: the server cannot verify
    // what arrives without the key, and a push that landed before it would
    // quarantine its own packs.
    assert_eq!(
        client.get_encryption_key().await.unwrap(),
        EscrowedKey::Absent
    );
    client.put_encryption_key(&REPO_KEY).await.unwrap();

    let (resp, _stats) = client
        .push(&odb, push_main(commit), false)
        .await
        .expect("encrypted push must succeed once the key is escrowed");
    assert!(resp.success, "push reported failure: {resp:?}");

    // --- the server has it, and it is sealed on the server's disk too
    let server_main = RefDatabase::new(server_repo.join(".mediagit"))
        .read("refs/heads/main")
        .await
        .unwrap();
    assert_eq!(server_main.oid.unwrap().to_hex(), commit.to_hex());

    let (sealed, total) = seal_census(&server_repo).await;
    assert!(total > 0, "the server stored nothing");
    assert_eq!(
        sealed, total,
        "the server must not have written plaintext for a keyed repository"
    );

    // --- and a reader holding the key gets the original bytes back. This is
    // the assertion the whole feature reduces to.
    let server_odb = odb_at(&server_repo, Some(key(&REPO_KEY))).await;
    let tree_oid = Commit::read(&server_odb, &commit).await.unwrap().tree;
    let tree = Tree::read(&server_odb, &tree_oid).await.unwrap();
    let blob = tree
        .entries
        .get("asset.bin")
        .expect("the file we committed");
    assert_eq!(
        server_odb.read(&blob.oid).await.unwrap(),
        payload,
        "round trip must be byte-exact"
    );
}

/// The chunked path, which is the one that actually carries media.
///
/// A blob over 1 MiB is split into chunks that travel to the server
/// individually rather than inside the pack, and each one is content-verified
/// on arrival by `upload_chunk`. That verification is a decrypt on an
/// encrypted repository, and if it is holding a bare compressor it does not
/// error -- it rejects the upload as a hash mismatch, or quarantines the pack
/// the chunk belongs to. Nothing above this test would notice.
#[tokio::test]
async fn a_chunked_asset_survives_an_encrypted_push() {
    let server_temp = TempDir::new().unwrap();
    let client_temp = TempDir::new().unwrap();
    let server_repos = server_temp.path().join("repos");
    let server_repo = server_repos.join("secret-repo");
    let client_repo = client_temp.path().join("secret-repo");
    tokio::fs::create_dir_all(server_repo.join(".mediagit/refs/heads"))
        .await
        .unwrap();

    let base_url = start_server(server_repos.clone(), Some(key(&SERVER_MASTER))).await;
    let client = ProtocolClient::new(format!("{base_url}/secret-repo"));

    // Chunking needs a strategy, and it only engages above 1 MiB. Compressible
    // content on purpose: a chunk that compresses is a chunk whose sealed form
    // differs in length from its plaintext, which is where framing bugs live.
    tokio::fs::create_dir_all(client_repo.join(".mediagit/refs/heads"))
        .await
        .unwrap();
    let storage = open_storage(&client_repo).await.unwrap();
    let odb = ObjectDatabase::with_optimizations(
        storage,
        1000,
        Some(mediagit_versioning::ChunkStrategy::Fixed { size: 256 * 1024 }),
        true,
        0,
    )
    .with_at_rest_key(Some(key(&REPO_KEY)));

    let payload = b"chunked media, sealed, over the wire. ".repeat(90_000);
    assert!(payload.len() > 1024 * 1024, "must be big enough to chunk");

    let blob = odb
        .write_chunked(ObjectType::Blob, &payload, "asset.bin")
        .await
        .unwrap();
    assert!(
        odb.is_chunked(&blob).await.unwrap(),
        "this test is pointless unless the blob actually chunked"
    );

    let mut tree = Tree::new();
    tree.add_entry(TreeEntry::new("asset.bin".into(), FileMode::Regular, blob));
    let tree_oid = tree.write(&odb).await.unwrap();
    let who = Signature::now("Test User".into(), "test@example.com".into());
    let commit = Commit::new(tree_oid, who.clone(), who, "big one".into())
        .write(&odb)
        .await
        .unwrap();
    RefDatabase::new(client_repo.join(".mediagit"))
        .write(&Ref::new_direct("refs/heads/main".into(), commit))
        .await
        .unwrap();

    client.put_encryption_key(&REPO_KEY).await.unwrap();
    let (resp, _stats) = client
        .push(&odb, push_main(commit), false)
        .await
        .expect("a chunked encrypted push must succeed");
    assert!(resp.success, "push reported failure: {resp:?}");

    let (sealed, total) = seal_census(&server_repo).await;
    assert!(total > 1, "the server should hold several chunks");
    assert_eq!(sealed, total, "every chunk on the server must be sealed");

    // The pack container is plaintext framing around sealed entries. Assert
    // that rather than exempting it on trust: a pack with no envelope anywhere
    // inside it would mean the chunks went up in the clear.
    let st = open_storage(&server_repo).await.unwrap();
    let mut packs = 0;
    for k in st.list_objects("").await.unwrap_or_default() {
        if !k.starts_with("packs/") {
            continue;
        }
        packs += 1;
        let bytes = st.get(&k).await.unwrap();
        assert!(
            bytes
                .windows(4)
                .any(|w| w == &mediagit_security::envelope::MGEN_MAGIC[..]),
            "pack {k} holds no sealed entry at all"
        );
    }
    assert!(
        packs > 0,
        "no pack was uploaded; this check measured nothing"
    );

    let server_odb = odb_at(&server_repo, Some(key(&REPO_KEY))).await;
    assert_eq!(
        server_odb.read(&blob).await.unwrap(),
        payload,
        "the reassembled asset must be byte-exact"
    );
}

#[tokio::test]
async fn a_reader_without_the_key_gets_nothing_it_can_use() {
    let server_temp = TempDir::new().unwrap();
    let client_temp = TempDir::new().unwrap();
    let server_repos = server_temp.path().join("repos");
    let server_repo = server_repos.join("secret-repo");
    let client_repo = client_temp.path().join("secret-repo");
    tokio::fs::create_dir_all(server_repo.join(".mediagit/refs/heads"))
        .await
        .unwrap();

    let base_url = start_server(server_repos.clone(), Some(key(&SERVER_MASTER))).await;
    let client = ProtocolClient::new(format!("{base_url}/secret-repo"));

    let payload = b"fail closed or do not ship".repeat(200);
    let odb = odb_at(&client_repo, Some(key(&REPO_KEY))).await;
    let commit = commit_one_file(&client_repo, &odb, &payload).await.unwrap();
    client.put_encryption_key(&REPO_KEY).await.unwrap();
    client.push(&odb, push_main(commit), false).await.unwrap();

    // A reader with the bytes but not the key must fail, not hand back
    // ciphertext classified as "uncompressed" and passed off as content.
    let bare = odb_at(&server_repo, None).await;
    assert!(
        Commit::read(&bare, &commit).await.is_err(),
        "an unkeyed reader must fail closed on a sealed repository"
    );

    let wrong = odb_at(&server_repo, Some(key(&[0x11; 32]))).await;
    assert!(
        Commit::read(&wrong, &commit).await.is_err(),
        "the wrong key must fail closed"
    );
}

#[tokio::test]
async fn escrow_refuses_to_replace_a_key() {
    let server_temp = TempDir::new().unwrap();
    let server_repos = server_temp.path().join("repos");
    tokio::fs::create_dir_all(server_repos.join("secret-repo").join(".mediagit"))
        .await
        .unwrap();

    let base_url = start_server(server_repos, Some(key(&SERVER_MASTER))).await;
    let client = ProtocolClient::new(format!("{base_url}/secret-repo"));

    client.put_encryption_key(&REPO_KEY).await.unwrap();
    // Retrying the same key must work: a client that lost the response has to
    // be able to try again.
    client.put_encryption_key(&REPO_KEY).await.unwrap();

    // A different one must not. Everything already stored is sealed under the
    // first, and nothing records which key an object used.
    let err = client
        .put_encryption_key(&[0x02; 32])
        .await
        .expect_err("a different key must be refused");
    assert!(
        err.to_string().contains("Nothing was uploaded"),
        "the refusal must say nothing was uploaded, got: {err}"
    );
}

#[tokio::test]
async fn a_server_with_encryption_off_tells_the_client_so() {
    let server_temp = TempDir::new().unwrap();
    let server_repos = server_temp.path().join("repos");
    tokio::fs::create_dir_all(server_repos.join("plain-repo").join(".mediagit"))
        .await
        .unwrap();

    let base_url = start_server(server_repos, None).await;
    let client = ProtocolClient::new(format!("{base_url}/plain-repo"));

    // Indistinguishable from an older server without the route, deliberately:
    // the honest message is the same either way.
    assert_eq!(
        client.get_encryption_key().await.unwrap(),
        EscrowedKey::Absent
    );
    let err = client
        .put_encryption_key(&REPO_KEY)
        .await
        .expect_err("a server with encryption off must refuse escrow");
    assert!(
        err.to_string()
            .contains("does not support encrypted repositories"),
        "got: {err}"
    );
}
