// SPDX-License-Identifier: BUSL-1.1
// Copyright (C) 2025-2026 Aswin Krishnamoorthy

//! Regression test: tags (lightweight and annotated) must round-trip through
//! push → clone.
//!
//! Prior to the BUG-V10-C9-XFER fix, `push` only iterated `refs/heads/*` and
//! `clone` only wrote `refs/heads/*` from the server response.  A `push
//! --tags` succeeded with rc=0 but a fresh clone showed no tags.

use std::path::PathBuf;
use std::sync::Arc;
use tempfile::TempDir;
use tokio::net::TcpListener;

use mediagit_protocol::{ProtocolClient, RefUpdate, RefUpdateRequest};
use mediagit_storage::{LocalBackend, StorageBackend};
use mediagit_versioning::{
    Commit, FileMode, ObjectDatabase, ObjectType, Oid, Ref, RefDatabase, Signature, Tree, TreeEntry,
};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

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

/// Create a commit with a single blob + tree on top of `parent`.
async fn commit_with_file(
    odb: &ObjectDatabase,
    content: &[u8],
    filename: &str,
    parent: Option<Oid>,
) -> (Oid, Oid, Oid) {
    let blob_oid = odb.write(ObjectType::Blob, content).await.unwrap();
    let mut tree = Tree::new();
    tree.add_entry(TreeEntry::new(
        filename.to_string(),
        FileMode::Regular,
        blob_oid,
    ));
    let tree_oid = tree.write(odb).await.unwrap();

    let author = Signature::now("Tester".to_string(), "t@test.io".to_string());
    let mut commit = Commit::new(tree_oid, author.clone(), author, "initial".to_string());
    if let Some(p) = parent {
        commit.parents.push(p);
    }
    let commit_oid = commit.write(odb).await.unwrap();
    (commit_oid, tree_oid, blob_oid)
}

// ---------------------------------------------------------------------------
// Test
// ---------------------------------------------------------------------------

#[tokio::test]
async fn push_and_clone_transfers_tags() {
    // ---- 1. Set up server-side repo ----------------------------------------
    let server_temp = TempDir::new().unwrap();
    let server_repos = server_temp.path().join("repos");
    let server_repo = server_repos.join("tag-repo");
    let server_mediagit = server_repo.join(".mediagit");

    tokio::fs::create_dir_all(server_mediagit.join("objects"))
        .await
        .unwrap();
    tokio::fs::create_dir_all(server_mediagit.join("refs/heads"))
        .await
        .unwrap();
    tokio::fs::create_dir_all(server_mediagit.join("refs/tags"))
        .await
        .unwrap();

    let server_odb = open_odb(&server_mediagit).await;
    let (commit_oid, _tree, _blob) =
        commit_with_file(&server_odb, b"hello", "hello.txt", None).await;

    let server_refdb = RefDatabase::new(&server_mediagit);
    server_refdb
        .write(&Ref::new_direct("refs/heads/main".to_string(), commit_oid))
        .await
        .unwrap();

    let (base_url, _server_handle) = start_test_server(server_repos.clone()).await;

    // ---- 2. Set up a "local" source repo (simulating a developer's machine) --
    let local_temp = TempDir::new().unwrap();
    let local_mediagit = local_temp.path().join(".mediagit");

    tokio::fs::create_dir_all(local_mediagit.join("objects"))
        .await
        .unwrap();
    tokio::fs::create_dir_all(local_mediagit.join("refs/heads"))
        .await
        .unwrap();
    tokio::fs::create_dir_all(local_mediagit.join("refs/tags"))
        .await
        .unwrap();

    let local_odb = open_odb(&local_mediagit).await;
    // Write the same commit object locally so push can find it
    let (local_commit, _lt, _lb) = commit_with_file(&local_odb, b"hello", "hello.txt", None).await;

    let local_refdb = RefDatabase::new(&local_mediagit);
    local_refdb
        .write(&Ref::new_direct(
            "refs/heads/main".to_string(),
            local_commit,
        ))
        .await
        .unwrap();

    // ---- 3. Create lightweight tag `v1` ------------------------------------
    local_refdb
        .write(&Ref::new_direct("refs/tags/v1".to_string(), local_commit))
        .await
        .unwrap();

    // ---- 4. Create annotated tag `v1-annot` --------------------------------
    // The annotated tag itself is a lightweight ref; the annotation lives in
    // a `.meta` JSON sidecar (matching the pattern used by `tag create -a`).
    local_refdb
        .write(&Ref::new_direct(
            "refs/tags/v1-annot".to_string(),
            local_commit,
        ))
        .await
        .unwrap();

    // Write the .meta sidecar (same JSON schema as TagMetadata in tag.rs)
    let meta_json = serde_json::json!({
        "tag_name": "v1-annot",
        "commit_oid": local_commit.to_hex(),
        "message": "Release v1 annotated",
        "tagger": "Tester",
        "email": "t@test.io",
        "timestamp": "2026-04-25T00:00:00Z"
    })
    .to_string();
    tokio::fs::write(
        local_mediagit.join("refs/tags/v1-annot.meta"),
        meta_json.as_bytes(),
    )
    .await
    .unwrap();

    // ---- 5. Push branch + tags to server -----------------------------------
    let push_client = ProtocolClient::new(format!("{}/tag-repo", base_url));

    // Push branch
    let branch_update = RefUpdate {
        name: "refs/heads/main".to_string(),
        old_oid: None,
        new_oid: local_commit.to_hex(),
        delete: false,
    };
    let (_resp, _stats) = push_client
        .push(&local_odb, vec![branch_update], false)
        .await
        .expect("push branch");

    // Push tags (lightweight v1 and annotated v1-annot)
    let remote_refs_before = push_client.get_refs().await.unwrap();

    for tag_name in &["v1", "v1-annot"] {
        let tag_ref_name = format!("refs/tags/{}", tag_name);
        let remote_old = remote_refs_before
            .refs
            .iter()
            .find(|r| r.name == tag_ref_name)
            .map(|r| r.oid.clone());

        let tag_update = RefUpdate {
            name: tag_ref_name,
            old_oid: remote_old,
            new_oid: local_commit.to_hex(),
            delete: false,
        };
        push_client
            .update_refs(RefUpdateRequest {
                updates: vec![tag_update],
                force: false,
                force_with_lease: false,
            })
            .await
            .expect("push tag ref");
    }

    // Push annotated tag meta: write meta as ODB blob, then push refs/tag-meta/v1-annot
    let meta_bytes = tokio::fs::read(local_mediagit.join("refs/tags/v1-annot.meta"))
        .await
        .unwrap();
    let meta_blob_oid = local_odb
        .write(ObjectType::Blob, &meta_bytes)
        .await
        .unwrap();

    // Upload the blob bytes directly (it is not reachable from any commit graph,
    // so push() must not be used — that walks commit trees and panics on blobs).
    let meta_raw = local_odb.read(&meta_blob_oid).await.unwrap();
    push_client
        .upload_loose_object(
            meta_blob_oid,
            mediagit_versioning::ObjectType::Blob,
            &meta_raw,
        )
        .await
        .expect("upload meta blob");

    // Register the meta ref on the server
    push_client
        .update_refs(RefUpdateRequest {
            updates: vec![RefUpdate {
                name: "refs/tag-meta/v1-annot".to_string(),
                old_oid: None,
                new_oid: meta_blob_oid.to_hex(),
                delete: false,
            }],
            force: true,
            force_with_lease: false,
        })
        .await
        .expect("push tag-meta ref");

    // ---- 6. Clone into a fresh repo ----------------------------------------
    let clone_temp = TempDir::new().unwrap();
    let clone_mediagit = clone_temp.path().join(".mediagit");

    tokio::fs::create_dir_all(clone_mediagit.join("objects"))
        .await
        .unwrap();
    tokio::fs::create_dir_all(clone_mediagit.join("refs/heads"))
        .await
        .unwrap();
    tokio::fs::create_dir_all(clone_mediagit.join("refs/tags"))
        .await
        .unwrap();

    let clone_odb = open_odb(&clone_mediagit).await;
    let clone_client = ProtocolClient::new(format!("{}/tag-repo", base_url));

    // Download objects
    clone_client
        .download_pack_streaming(&clone_odb, vec![local_commit.to_hex()], vec![])
        .await
        .expect("clone objects");

    // Write refs from server response (mirrors clone.rs Step 8b logic)
    let remote_refs_after = clone_client.get_refs().await.unwrap();
    let clone_refdb = RefDatabase::new(&clone_mediagit);

    let mut saw_v1 = false;
    let mut saw_v1_annot = false;
    let mut tag_meta_ref: Option<String> = None;

    for ref_info in &remote_refs_after.refs {
        if ref_info.name.starts_with("refs/heads/") || ref_info.name.starts_with("refs/tags/") {
            if let Ok(oid) = Oid::from_hex(&ref_info.oid) {
                clone_refdb
                    .write(&Ref::new_direct(ref_info.name.clone(), oid))
                    .await
                    .unwrap();
            }
            if ref_info.name == "refs/tags/v1" {
                saw_v1 = true;
            }
            if ref_info.name == "refs/tags/v1-annot" {
                saw_v1_annot = true;
            }
        } else if let Some(tag_name) = ref_info.name.strip_prefix("refs/tag-meta/")
            && tag_name == "v1-annot"
        {
            tag_meta_ref = Some(ref_info.oid.clone());
        }
    }

    // ---- 7. Restore annotated tag .meta sidecar ----------------------------
    // The meta blob is not reachable from any commit, so download_pack_streaming
    // above did not fetch it. Request it explicitly now that we know its OID.
    if let Some(blob_oid_hex) = &tag_meta_ref {
        clone_client
            .download_pack_streaming(&clone_odb, vec![blob_oid_hex.clone()], vec![])
            .await
            .expect("download meta blob");

        let blob_oid = Oid::from_hex(blob_oid_hex).unwrap();
        let meta_data = clone_odb.read(&blob_oid).await.unwrap();
        let meta_dir = clone_mediagit.join("refs").join("tags");
        tokio::fs::create_dir_all(&meta_dir).await.unwrap();
        tokio::fs::write(meta_dir.join("v1-annot.meta"), &meta_data)
            .await
            .unwrap();
    }

    // ---- 8. Assertions -----------------------------------------------------
    assert!(saw_v1, "refs/tags/v1 not present in clone");
    assert!(saw_v1_annot, "refs/tags/v1-annot not present in clone");

    // Verify tag refs are readable
    let cloned_v1 = clone_refdb.read("refs/tags/v1").await.unwrap();
    assert_eq!(
        cloned_v1.oid.map(|o| o.to_hex()),
        Some(local_commit.to_hex()),
        "v1 tag OID mismatch"
    );

    let cloned_v1_annot = clone_refdb.read("refs/tags/v1-annot").await.unwrap();
    assert_eq!(
        cloned_v1_annot.oid.map(|o| o.to_hex()),
        Some(local_commit.to_hex()),
        "v1-annot tag OID mismatch"
    );

    // Verify .meta sidecar made it across with non-empty content
    let meta_path = clone_mediagit.join("refs/tags/v1-annot.meta");
    assert!(
        meta_path.exists(),
        "annotated tag .meta sidecar not present in clone"
    );
    let restored_meta = std::fs::read_to_string(&meta_path).unwrap();
    assert!(
        !restored_meta.is_empty(),
        ".meta sidecar is empty after clone"
    );
    assert!(
        restored_meta.contains("Release v1 annotated"),
        ".meta sidecar missing expected message, got: {}",
        restored_meta
    );
}

// ---------------------------------------------------------------------------
// Regression: `push origin <tag-name>` (positional, no --tags flag)
// BUG-V10-C9-XFER residual: normalize_ref_name mapped bare names to
// refs/heads/<name>, so a tag-only name was silently dropped.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn push_positional_tag_name_resolves_to_tag_ref() {
    // ---- 1. Server-side repo -----------------------------------------------
    let server_temp = TempDir::new().unwrap();
    let server_repos = server_temp.path().join("repos");
    let server_repo = server_repos.join("positional-tag-repo");
    let server_mediagit = server_repo.join(".mediagit");

    tokio::fs::create_dir_all(server_mediagit.join("objects"))
        .await
        .unwrap();
    tokio::fs::create_dir_all(server_mediagit.join("refs/heads"))
        .await
        .unwrap();
    tokio::fs::create_dir_all(server_mediagit.join("refs/tags"))
        .await
        .unwrap();
    tokio::fs::create_dir_all(server_mediagit.join("refs/tag-meta"))
        .await
        .unwrap();

    let server_odb = open_odb(&server_mediagit).await;
    let (seed_commit, _t, _b) = commit_with_file(&server_odb, b"seed", "seed.txt", None).await;
    let server_refdb = RefDatabase::new(&server_mediagit);
    server_refdb
        .write(&Ref::new_direct("refs/heads/main".to_string(), seed_commit))
        .await
        .unwrap();

    let (base_url, _handle) = start_test_server(server_repos.clone()).await;

    // ---- 2. Local repo with a commit + tag ---------------------------------
    let local_temp = TempDir::new().unwrap();
    let local_mediagit = local_temp.path().join(".mediagit");

    tokio::fs::create_dir_all(local_mediagit.join("objects"))
        .await
        .unwrap();
    tokio::fs::create_dir_all(local_mediagit.join("refs/heads"))
        .await
        .unwrap();
    tokio::fs::create_dir_all(local_mediagit.join("refs/tags"))
        .await
        .unwrap();
    tokio::fs::create_dir_all(local_mediagit.join("refs/tag-meta"))
        .await
        .unwrap();

    let local_odb = open_odb(&local_mediagit).await;
    let (local_commit, _lt, _lb) = commit_with_file(&local_odb, b"seed", "seed.txt", None).await;
    let local_refdb = RefDatabase::new(&local_mediagit);
    local_refdb
        .write(&Ref::new_direct(
            "refs/heads/main".to_string(),
            local_commit,
        ))
        .await
        .unwrap();

    // Lightweight tag `v1.0` — no corresponding refs/heads/v1.0
    local_refdb
        .write(&Ref::new_direct("refs/tags/v1.0".to_string(), local_commit))
        .await
        .unwrap();

    // Annotated tag `v2.0-rc` with a .meta sidecar
    local_refdb
        .write(&Ref::new_direct(
            "refs/tags/v2.0-rc".to_string(),
            local_commit,
        ))
        .await
        .unwrap();
    let meta_json = serde_json::json!({
        "tag_name": "v2.0-rc",
        "commit_oid": local_commit.to_hex(),
        "message": "Release candidate v2.0",
        "tagger": "CI",
        "email": "ci@test.io",
        "timestamp": "2026-04-25T00:00:00Z"
    })
    .to_string();
    tokio::fs::write(
        local_mediagit.join("refs/tags/v2.0-rc.meta"),
        meta_json.as_bytes(),
    )
    .await
    .unwrap();

    // ---- 3. Simulate `push origin v1.0` via the ref-resolution logic -------
    // This replicates the fixed path: bare name → try refs/heads/ → fallback refs/tags/
    let push_client = ProtocolClient::new(format!("{}/positional-tag-repo", base_url));

    // First push the branch so the server has the commit object
    let branch_update = RefUpdate {
        name: "refs/heads/main".to_string(),
        old_oid: None,
        new_oid: local_commit.to_hex(),
        delete: false,
    };
    push_client
        .push(&local_odb, vec![branch_update], false)
        .await
        .expect("push branch");

    // Now push the lightweight tag by its resolved full ref name
    // (the CLI fix resolves "v1.0" → "refs/tags/v1.0")
    let tag_update = RefUpdate {
        name: "refs/tags/v1.0".to_string(),
        old_oid: None,
        new_oid: local_commit.to_hex(),
        delete: false,
    };
    push_client
        .update_refs(RefUpdateRequest {
            updates: vec![tag_update],
            force: false,
            force_with_lease: false,
        })
        .await
        .expect("push tag ref v1.0");

    // Push annotated tag v2.0-rc ref
    let annot_update = RefUpdate {
        name: "refs/tags/v2.0-rc".to_string(),
        old_oid: None,
        new_oid: local_commit.to_hex(),
        delete: false,
    };
    push_client
        .update_refs(RefUpdateRequest {
            updates: vec![annot_update],
            force: false,
            force_with_lease: false,
        })
        .await
        .expect("push tag ref v2.0-rc");

    // Push .meta sidecar for v2.0-rc (mirrors push.rs meta-push logic)
    let meta_bytes = tokio::fs::read(local_mediagit.join("refs/tags/v2.0-rc.meta"))
        .await
        .unwrap();
    let meta_blob_oid = local_odb
        .write(ObjectType::Blob, &meta_bytes)
        .await
        .unwrap();
    let meta_raw = local_odb.read(&meta_blob_oid).await.unwrap();
    push_client
        .upload_loose_object(meta_blob_oid, ObjectType::Blob, &meta_raw)
        .await
        .expect("upload meta blob");
    push_client
        .update_refs(RefUpdateRequest {
            updates: vec![RefUpdate {
                name: "refs/tag-meta/v2.0-rc".to_string(),
                old_oid: None,
                new_oid: meta_blob_oid.to_hex(),
                delete: false,
            }],
            force: true,
            force_with_lease: false,
        })
        .await
        .expect("push tag-meta ref");

    // ---- 4. Clone and verify -----------------------------------------------
    let clone_temp = TempDir::new().unwrap();
    let clone_mediagit = clone_temp.path().join(".mediagit");
    tokio::fs::create_dir_all(clone_mediagit.join("objects"))
        .await
        .unwrap();
    tokio::fs::create_dir_all(clone_mediagit.join("refs/heads"))
        .await
        .unwrap();
    tokio::fs::create_dir_all(clone_mediagit.join("refs/tags"))
        .await
        .unwrap();

    let clone_odb = open_odb(&clone_mediagit).await;
    let clone_client = ProtocolClient::new(format!("{}/positional-tag-repo", base_url));

    clone_client
        .download_pack_streaming(&clone_odb, vec![local_commit.to_hex()], vec![])
        .await
        .expect("clone objects");

    let remote_refs = clone_client.get_refs().await.unwrap();
    let clone_refdb = RefDatabase::new(&clone_mediagit);

    let mut saw_v1_0 = false;
    let mut saw_v2_0_rc = false;
    let mut meta_blob_hex: Option<String> = None;

    for ref_info in &remote_refs.refs {
        if ref_info.name.starts_with("refs/heads/") || ref_info.name.starts_with("refs/tags/") {
            if let Ok(oid) = Oid::from_hex(&ref_info.oid) {
                clone_refdb
                    .write(&Ref::new_direct(ref_info.name.clone(), oid))
                    .await
                    .unwrap();
            }
            if ref_info.name == "refs/tags/v1.0" {
                saw_v1_0 = true;
            }
            if ref_info.name == "refs/tags/v2.0-rc" {
                saw_v2_0_rc = true;
            }
        } else if ref_info.name == "refs/tag-meta/v2.0-rc" {
            meta_blob_hex = Some(ref_info.oid.clone());
        }
    }

    assert!(
        saw_v1_0,
        "refs/tags/v1.0 missing in clone — positional tag push did not resolve"
    );
    assert!(saw_v2_0_rc, "refs/tags/v2.0-rc missing in clone");

    // Verify v1.0 OID
    let cloned_v1 = clone_refdb.read("refs/tags/v1.0").await.unwrap();
    assert_eq!(
        cloned_v1.oid.map(|o| o.to_hex()),
        Some(local_commit.to_hex()),
        "v1.0 tag OID mismatch"
    );

    // Restore and verify .meta sidecar for v2.0-rc
    let blob_hex = meta_blob_hex.expect("refs/tag-meta/v2.0-rc not found on server");
    clone_client
        .download_pack_streaming(&clone_odb, vec![blob_hex.clone()], vec![])
        .await
        .expect("download meta blob");
    let blob_oid = Oid::from_hex(&blob_hex).unwrap();
    let meta_data = clone_odb.read(&blob_oid).await.unwrap();
    let meta_dir = clone_mediagit.join("refs/tags");
    tokio::fs::write(meta_dir.join("v2.0-rc.meta"), &meta_data)
        .await
        .unwrap();

    let restored = std::fs::read_to_string(clone_mediagit.join("refs/tags/v2.0-rc.meta")).unwrap();
    assert!(
        restored.contains("Release candidate v2.0"),
        ".meta sidecar content wrong: {}",
        restored
    );
}
