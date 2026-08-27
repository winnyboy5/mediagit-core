// SPDX-License-Identifier: BUSL-1.1
// Copyright (C) 2025-2026 Aswin Krishnamoorthy

//! A clone must fail loudly rather than silently omit objects.
//!
//! Root cause of the intermittent `tag_object_push_clone_round_trips_*`
//! failure. `collect_objects_bfs` (the **want**-side walk) reads each object
//! with `odb.read(&oid).await.ok()` and, on `None`, logs a warning and
//! `continue`s — dropping that object from the pack *and* never exploring its
//! subtree. The server then answers 200 with a short pack, and the client,
//! which streams to the object count declared in the pack header, has no way
//! to notice. The damage only surfaces later as
//! "Object <oid> not found: no loose object and no pack files".
//!
//! Leniency is correct on the **have** side (`walk_reachable`) — a client may
//! claim haves that do not exist. It is wrong on the want side: the client
//! asked for these objects.
//!
//! Deleting a reachable object server-side reproduces deterministically what a
//! transient read failure does under load.

use std::path::PathBuf;
use std::sync::Arc;
use tempfile::TempDir;
use tokio::net::TcpListener;

use mediagit_protocol::{ProtocolClient, RefUpdate};
use mediagit_storage::{LocalBackend, StorageBackend};
use mediagit_versioning::{
    Commit, FileMode, ObjectDatabase, ObjectType, Oid, Signature, Tree, TreeEntry,
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

#[tokio::test]
async fn clone_fails_loudly_when_a_wanted_object_is_unreadable() {
    let server_temp = TempDir::new().unwrap();
    let repos_dir = server_temp.path().join("repos");
    let server_repo = repos_dir.join("incomplete-pack-repo");
    tokio::fs::create_dir_all(server_repo.join(".mediagit/refs/heads"))
        .await
        .unwrap();
    let (base_url, _srv) = start_test_server(repos_dir.clone()).await;

    // ---- build and push commit -> tree -> blob ------------------------------
    let local_temp = TempDir::new().unwrap();
    let local_mediagit = local_temp.path().join(".mediagit");
    tokio::fs::create_dir_all(local_mediagit.join("refs/heads"))
        .await
        .unwrap();
    let local_odb = open_odb(&local_mediagit).await;

    let blob_oid = local_odb
        .write(
            ObjectType::Blob,
            b"payload that must survive the round trip",
        )
        .await
        .unwrap();
    let mut tree = Tree::new();
    tree.add_entry(TreeEntry::new(
        "asset.bin".to_string(),
        FileMode::Regular,
        blob_oid,
    ));
    let tree_oid = tree.write(&local_odb).await.unwrap();
    let author = Signature::now("Tester".to_string(), "t@test.io".to_string());
    let commit = Commit::new(tree_oid, author.clone(), author, "initial".to_string());
    let commit_oid = commit.write(&local_odb).await.unwrap();

    let client = ProtocolClient::new(format!("{}/incomplete-pack-repo", base_url));
    client
        .push(
            &local_odb,
            vec![RefUpdate {
                name: "refs/heads/main".to_string(),
                old_oid: None,
                new_oid: commit_oid.to_hex(),
                delete: false,
            }],
            false,
        )
        .await
        .expect("push");

    // ---- make one reachable object unreadable on the server -----------------
    // Stands in for the transient read failure that produces this under load.
    // The blob is a leaf, so only the blob itself goes missing — the clearest
    // possible signal, and the one hardest to explain away.
    let removed = remove_loose_object(&server_repo, &blob_oid).await;
    assert!(
        removed,
        "test setup: expected a loose object for {blob_oid} under {}",
        server_repo.display()
    );

    // A second server on the same repos dir, so the walk actually hits the
    // filesystem: the first server's ObjectDatabase still has the blob in its
    // in-memory cache and would serve it happily, masking the deletion.
    let (cold_url, _srv2) = start_test_server(repos_dir.clone()).await;

    // ---- clone ---------------------------------------------------------------
    let clone_temp = TempDir::new().unwrap();
    let clone_mediagit = clone_temp.path().join(".mediagit");
    tokio::fs::create_dir_all(clone_mediagit.join("refs/heads"))
        .await
        .unwrap();
    let clone_odb = open_odb(&clone_mediagit).await;
    let clone_client = ProtocolClient::new(format!("{}/incomplete-pack-repo", cold_url));

    let result = clone_client
        .download_pack_streaming(&clone_odb, vec![commit_oid.to_hex()], vec![])
        .await;

    // The server cannot serve the closure it was asked for. Saying so is the
    // only acceptable outcome: a 200 with a short pack hands the client a
    // repository that looks complete and is not.
    let err = match result {
        Err(e) => e,
        Ok(_) => {
            // Prove the clone really is corrupt before failing, so the message
            // reports the actual consequence rather than a policy preference.
            let blob_present = clone_odb.read(&blob_oid).await.is_ok();
            panic!(
                "clone reported success while the pack was incomplete \
                 (blob present in clone: {blob_present}). A short pack must be \
                 an error, not a warning in the server log."
            );
        }
    };

    let msg = err.to_string().to_lowercase();
    assert!(
        msg.contains("incomplete")
            || msg.contains("unreadable")
            || msg.contains("missing")
            || msg.contains("not found"),
        "the error must name the incomplete closure so an operator can act; got: {err}"
    );
}

/// Delete the loose object file for `oid`, wherever the layout puts it.
///
/// Walks rather than hard-coding the sharding: this has changed before
/// (a grace-period feature was once written against `<2>/<62>` when the real
/// layout is `<namespace>/objects/<2>/<2>/<hex>`), and a helper that silently
/// finds nothing would make this test vacuously pass.
async fn remove_loose_object(repo_dir: &std::path::Path, oid: &Oid) -> bool {
    let hex = oid.to_hex();
    let mut stack = vec![repo_dir.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(mut entries) = tokio::fs::read_dir(&dir).await else {
            continue;
        };
        while let Ok(Some(entry)) = entries.next_entry().await {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if let Some(name) = path.file_name().and_then(|n| n.to_str())
                && hex.ends_with(name)
                && name.len() >= 32
                && tokio::fs::remove_file(&path).await.is_ok()
            {
                return true;
            }
        }
    }
    false
}

/// UX-3: `--force-with-lease` is a third mode, not a synonym for `--force`.
///
/// The flag was declared and demoed in `push --help` but never read, so it
/// silently did an ordinary push. Implementing it as an alias for `force`
/// would be worse than leaving it broken: `force` waives the `old_oid`
/// compare-and-swap as well as the ancestry check, so the user asking for the
/// *safe* option would get the dangerous one.
///
/// The distinction that matters: with a stale lease, force-with-lease must
/// refuse while plain force proceeds.
#[tokio::test]
async fn force_with_lease_refuses_when_the_remote_moved() {
    use mediagit_protocol::RefUpdateRequest;

    let server_temp = TempDir::new().unwrap();
    let repos_dir = server_temp.path().join("repos");
    let server_repo = repos_dir.join("lease-repo");
    tokio::fs::create_dir_all(server_repo.join(".mediagit/refs/heads"))
        .await
        .unwrap();
    let (base_url, _srv) = start_test_server(repos_dir.clone()).await;

    let local_temp = TempDir::new().unwrap();
    let local_mediagit = local_temp.path().join(".mediagit");
    tokio::fs::create_dir_all(local_mediagit.join("refs/heads"))
        .await
        .unwrap();
    let local_odb = open_odb(&local_mediagit).await;
    let client = ProtocolClient::new(format!("{}/lease-repo", base_url));

    // Three unrelated commits so neither is an ancestor of the other — any
    // update between them is a non-fast-forward.
    let mut oids = Vec::new();
    for i in 0..3 {
        let blob = local_odb
            .write(ObjectType::Blob, format!("content {i}").as_bytes())
            .await
            .unwrap();
        let mut tree = Tree::new();
        tree.add_entry(TreeEntry::new(format!("f{i}.bin"), FileMode::Regular, blob));
        let tree_oid = tree.write(&local_odb).await.unwrap();
        let who = Signature::now("Tester".to_string(), "t@test.io".to_string());
        let commit = Commit::new(tree_oid, who.clone(), who, format!("c{i}"));
        oids.push(commit.write(&local_odb).await.unwrap());
    }

    client
        .push(
            &local_odb,
            vec![RefUpdate {
                name: "refs/heads/main".to_string(),
                old_oid: None,
                new_oid: oids[0].to_hex(),
                delete: false,
            }],
            false,
        )
        .await
        .expect("initial push");

    // Someone else moves the branch.
    client
        .push(
            &local_odb,
            vec![RefUpdate {
                name: "refs/heads/main".to_string(),
                old_oid: Some(oids[0].to_hex()),
                new_oid: oids[1].to_hex(),
                delete: false,
            }],
            true,
        )
        .await
        .expect("concurrent update");

    // Our lease still names oids[0]; the remote is at oids[1].
    let stale = vec![RefUpdate {
        name: "refs/heads/main".to_string(),
        old_oid: Some(oids[0].to_hex()),
        new_oid: oids[2].to_hex(),
        delete: false,
    }];

    let leased = client
        .update_refs(RefUpdateRequest {
            updates: stale.clone(),
            force: true,
            force_with_lease: true,
        })
        .await
        .expect("request itself should succeed");
    assert!(
        !leased.results[0].success,
        "force-with-lease must refuse a stale lease — otherwise it silently \
         destroys the concurrent update it exists to protect"
    );

    // Plain force is the dangerous mode and must still overwrite.
    let forced = client
        .update_refs(RefUpdateRequest {
            updates: stale,
            force: true,
            force_with_lease: false,
        })
        .await
        .expect("request itself should succeed");
    assert!(
        forced.results[0].success,
        "plain --force must still overwrite; got: {:?}",
        forced.results[0].error
    );
}
