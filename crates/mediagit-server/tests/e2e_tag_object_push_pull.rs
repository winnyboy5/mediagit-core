// SPDX-License-Identifier: BUSL-1.1
// Copyright (C) 2025-2026 Aswin Krishnamoorthy

//! M5a regression: a repo containing a real `Tag` ODB object (annotated
//! tag) must round-trip through push → clone intact, exercising every
//! `ObjectType::Tag` match site end-to-end (streaming pack kind-byte,
//! server BFS walker, client graph collector) — this is exactly the class
//! of bug a missed match site would surface as (push succeeds but the tag
//! or its target silently never arrives, or the pack stream errors).
//!
//! Also proves the M3-review-flagged reachability gap is closed: a commit
//! reachable ONLY through an annotated tag (no branch points at it) must
//! still survive the server's post-receive bitmap generation and be
//! present after clone.

use std::path::PathBuf;
use std::sync::Arc;
use tempfile::TempDir;
use tokio::net::TcpListener;

use mediagit_protocol::{ProtocolClient, RefUpdate};
use mediagit_storage::{LocalBackend, StorageBackend};
use mediagit_versioning::{
    Commit, FileMode, ObjectDatabase, ObjectType, Oid, Ref, RefDatabase, Signature, Tag, Tree,
    TreeEntry,
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

async fn open_odb(mediagit_dir: &std::path::Path) -> ObjectDatabase {
    let storage: Arc<dyn StorageBackend> = Arc::new(LocalBackend::new(mediagit_dir).await.unwrap());
    ObjectDatabase::new(storage, 100)
}

/// Build commit -> tree -> blob, writing only the commit reachable ONLY
/// through the tag we create afterwards (no branch ref points at it).
async fn commit_with_file(odb: &ObjectDatabase, content: &[u8], filename: &str) -> (Oid, Oid, Oid) {
    let blob_oid = odb.write(ObjectType::Blob, content).await.unwrap();
    let mut tree = Tree::new();
    tree.add_entry(TreeEntry::new(
        filename.to_string(),
        FileMode::Regular,
        blob_oid,
    ));
    let tree_oid = tree.write(odb).await.unwrap();

    let author = Signature::now("Tester".to_string(), "t@test.io".to_string());
    let commit = Commit::new(tree_oid, author.clone(), author, "tag target".to_string());
    let commit_oid = commit.write(odb).await.unwrap();
    (commit_oid, tree_oid, blob_oid)
}

/// Core scenario, parameterized by an optional server-side `config.toml`
/// body (so the same logic drives both the always-on LocalBackend run and
/// the MinIO-backed run below).
async fn run_tag_object_push_clone(server_config_toml: Option<&str>) {
    let server_temp = TempDir::new().unwrap();
    let server_repos = server_temp.path().join("repos");
    let server_repo = server_repos.join("tag-object-repo");
    let server_mediagit = server_repo.join(".mediagit");

    tokio::fs::create_dir_all(server_mediagit.join("refs/heads"))
        .await
        .unwrap();
    tokio::fs::create_dir_all(server_mediagit.join("refs/tags"))
        .await
        .unwrap();

    if let Some(toml) = server_config_toml {
        tokio::fs::write(server_mediagit.join("config.toml"), toml)
            .await
            .unwrap();
    }

    // A branch tip that has NOTHING to do with the tagged commit — proves
    // the tagged commit isn't smuggled in via the branch push.
    let server_odb_local = open_odb(&server_mediagit).await;
    let unrelated_commit_oid = {
        let (oid, ..) = commit_with_file(&server_odb_local, b"unrelated", "unrelated.txt").await;
        oid
    };
    let server_refdb = RefDatabase::new(&server_mediagit);
    server_refdb
        .write(&Ref::new_direct(
            "refs/heads/main".to_string(),
            unrelated_commit_oid,
        ))
        .await
        .unwrap();

    let (base_url, _server_handle) = start_test_server(server_repos.clone()).await;

    // ---- local "developer" repo -------------------------------------------
    let local_temp = TempDir::new().unwrap();
    let local_mediagit = local_temp.path().join(".mediagit");
    tokio::fs::create_dir_all(local_mediagit.join("refs/heads"))
        .await
        .unwrap();
    tokio::fs::create_dir_all(local_mediagit.join("refs/tags"))
        .await
        .unwrap();

    let local_odb = open_odb(&local_mediagit).await;
    let (unrelated_local, ..) = commit_with_file(&local_odb, b"unrelated", "unrelated.txt").await;
    let local_refdb = RefDatabase::new(&local_mediagit);
    local_refdb
        .write(&Ref::new_direct(
            "refs/heads/main".to_string(),
            unrelated_local,
        ))
        .await
        .unwrap();

    // The tagged commit: reachable ONLY through the annotated tag below.
    let (tagged_commit, tagged_tree, tagged_blob) =
        commit_with_file(&local_odb, b"release payload", "release.txt").await;

    let tagger = Signature::now("Tester".to_string(), "t@test.io".to_string());
    let tag = Tag::new(
        tagged_commit,
        ObjectType::Commit,
        "v1.0.0".to_string(),
        tagger,
        "Release 1.0.0".to_string(),
    );
    let tag_oid = tag.write(&local_odb).await.unwrap();
    local_refdb
        .write(&Ref::new_direct("refs/tags/v1.0.0".to_string(), tag_oid))
        .await
        .unwrap();

    // ---- push branch + annotated tag ---------------------------------------
    let push_client = ProtocolClient::new(format!("{}/tag-object-repo", base_url));

    let branch_update = RefUpdate {
        name: "refs/heads/main".to_string(),
        old_oid: None,
        new_oid: unrelated_local.to_hex(),
        delete: false,
    };
    push_client
        .push(&local_odb, vec![branch_update], false)
        .await
        .expect("push branch");

    // The real `push()` path: seeds the walk from the tag ref's OID, which
    // must be detected as ObjectType::Tag (not assumed Commit) and walked
    // through to `tagged_commit`'s full closure — exactly the client-side
    // sweep site this test exists to exercise.
    let tag_update = RefUpdate {
        name: "refs/tags/v1.0.0".to_string(),
        old_oid: None,
        new_oid: tag_oid.to_hex(),
        delete: false,
    };
    push_client
        .push(&local_odb, vec![tag_update], false)
        .await
        .expect("push annotated tag");

    // Give the server's background bitmap-generation task a moment (it's
    // spawned fire-and-forget on ref update) — not required for correctness
    // (BFS fallback covers it), but exercises the bitmap path too.
    tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;

    // ---- clone into a fresh repo --------------------------------------------
    let clone_temp = TempDir::new().unwrap();
    let clone_mediagit = clone_temp.path().join(".mediagit");
    tokio::fs::create_dir_all(clone_mediagit.join("refs/heads"))
        .await
        .unwrap();
    tokio::fs::create_dir_all(clone_mediagit.join("refs/tags"))
        .await
        .unwrap();

    let clone_odb = open_odb(&clone_mediagit).await;
    let clone_client = ProtocolClient::new(format!("{}/tag-object-repo", base_url));

    // want = both branch tip and tag oid, mirroring what `clone` actually
    // requests (every advertised ref, tags included).
    clone_client
        .download_pack_streaming(
            &clone_odb,
            vec![unrelated_local.to_hex(), tag_oid.to_hex()],
            vec![],
        )
        .await
        .expect("clone objects (pack kind-byte + BFS walker for Tag)");

    // ---- assertions ---------------------------------------------------------
    // 1. The Tag object itself arrived and round-trips.
    let cloned_tag_data = clone_odb
        .read(&tag_oid)
        .await
        .expect("Tag object must be present after clone");
    let cloned_tag = Tag::deserialize(&cloned_tag_data).expect("must deserialize as Tag");
    assert_eq!(cloned_tag.target, tagged_commit);
    assert_eq!(cloned_tag.name, "v1.0.0");

    // 2. The tagged commit — reachable ONLY via the tag, not any branch —
    //    and its full closure (tree + blob) survived too. This is the
    //    M3-review reachability gap this whole cycle was flagged to close.
    let commit_data = clone_odb
        .read(&tagged_commit)
        .await
        .expect("tagged commit must be present (reachable only via the tag)");
    Commit::deserialize(&commit_data).expect("must deserialize as Commit");

    let tree_data = clone_odb
        .read(&tagged_tree)
        .await
        .expect("tagged commit's tree must be present");
    Tree::deserialize(&tree_data).expect("must deserialize as Tree");

    let blob_data = clone_odb
        .read(&tagged_blob)
        .await
        .expect("tagged commit's blob must be present");
    assert_eq!(blob_data, b"release payload");
}

#[tokio::test]
async fn tag_object_push_clone_round_trips_local_backend() {
    run_tag_object_push_clone(None).await;
}

/// Same scenario, but the SERVER's storage is a real MinIO bucket — proves
/// the pack kind-byte + BFS walker work end-to-end over the actual storage
/// stack a push/clone would use in production, not just an in-memory/local
/// stand-in. Requires a MinIO instance at localhost:9000 (minioadmin/minioadmin).
#[tokio::test]
#[ignore = "Requires MinIO at localhost:9000"]
async fn tag_object_push_clone_round_trips_minio_backend() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter("mediagit_server=debug,mediagit_storage=debug")
        .try_init();

    // Unique-ish namespace per run so repeated manual runs don't collide on
    // the repo_id/LAYOUT marker written by a previous run.
    let ns = format!("tag-e2e-{}", std::process::id());
    let config_toml = format!(
        r#"
repo_namespace = "{ns}"

[storage]
backend = "s3"
endpoint = "http://localhost:9000"
bucket = "mediagit-m5a-tag-test"
access_key_id = "minioadmin"
secret_access_key = "minioadmin"
region = "us-east-1"
"#
    );
    run_tag_object_push_clone(Some(&config_toml)).await;
}
