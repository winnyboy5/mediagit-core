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

//! QA-009 regression: `fetch`/`pull` had no code path for refs/tags/* — tags
//! only ever arrived via `clone`. There is no `--tags` flag; tags auto-fetch
//! like git (tag objects are tiny).
//!
//! Note on tag representation: a lightweight tag is a ref pointing directly
//! at a commit. An annotated tag (`tag create <name> -m <msg>`) writes a
//! first-class `Tag` ODB object and `refs/tags/<name>` points at *that*
//! object's OID (not the commit's) — see `mediagit_versioning::tag_object`.
//! The old `{tag_ref}.meta` sidecar mechanism is legacy and unused by
//! current `tag create`.
//!
//! Covers:
//! - lightweight + annotated tags, pushed AFTER an existing clone was made,
//!   arrive via a plain `mediagit fetch` into that pre-existing clone.
//! - annotated tag message content round-trips (via the Tag ODB object).
//! - a tag pointing at a commit NOT reachable from any branch the clone
//!   fetches (its objects live nowhere else locally) still resolves — the
//!   Tag object itself, and its target commit/tree/blob closure, must be
//!   downloaded on demand. A naive "just write the ref" implementation
//!   leaves the ref pointing at a hole.
//! - re-fetch is idempotent (no error, no duplicate side effects).

use assert_cmd::Command;
use predicates::prelude::*;
use std::fs;
use std::path::Path;
use std::sync::Arc;
use tempfile::TempDir;
use tokio::net::TcpListener;

use mediagit_versioning::{ObjectDatabase, Oid, RefDatabase, Tag};

#[allow(deprecated)]
fn mediagit() -> Command {
    {
        // `commit` refuses an unconfigured identity (UX-6) instead of
        // authoring as `Unknown <unknown@localhost>`, so tests declare one
        // the way a real user would.
        let mut c = Command::cargo_bin("mediagit").unwrap();
        c.env("MEDIAGIT_AUTHOR_NAME", "Test User")
            .env("MEDIAGIT_AUTHOR_EMAIL", "test@example.com");
        c
    }
}

fn init_repo(dir: &Path) {
    mediagit()
        .arg("init")
        .arg("-q")
        .current_dir(dir)
        .assert()
        .success();
}

fn add_and_commit(dir: &Path, name: &str, content: &str, message: &str) {
    fs::write(dir.join(name), content).unwrap();
    mediagit()
        .arg("add")
        .arg(name)
        .current_dir(dir)
        .assert()
        .success();
    mediagit()
        .arg("commit")
        .arg("-m")
        .arg(message)
        .current_dir(dir)
        .assert()
        .success();
}

async fn start_test_server(repos_dir: std::path::PathBuf) -> (String, tokio::task::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let base_url = format!("http://{}", addr);
    let state = Arc::new(mediagit_server::AppState::new(repos_dir));
    let app = mediagit_server::create_router(state);
    let handle = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
    (base_url, handle)
}

/// Open an ODB the same way the CLI does: through
/// `mediagit_cli::repo::create_storage_backend`, which wraps storage in a
/// `NamespacedBackend` per layout v2 (objects live under
/// `.mediagit/<repo_namespace>/objects/...`, not `.mediagit/objects/...`).
/// `repo_root` is the repo's top-level directory (parent of `.mediagit`).
async fn open_odb(repo_root: &Path) -> ObjectDatabase {
    let storage = mediagit_cli::repo::create_storage_backend(repo_root)
        .await
        .unwrap();
    ObjectDatabase::with_smart_compression(storage, 100)
}

/// Read a ref's OID (hex) straight off disk — both source and clone repos
/// store refs as plain files, so this works without spinning up an ODB.
fn read_ref_oid_from_disk(mediagit_dir: &Path, ref_relpath: &str) -> String {
    fs::read_to_string(mediagit_dir.join(ref_relpath))
        .unwrap_or_else(|e| panic!("failed to read {}: {}", ref_relpath, e))
        .trim()
        .to_string()
}

/// Read a tag ref's OID (hex) via the RefDatabase, panicking with a clear
/// message if it's missing — keeps assertion failures readable.
async fn read_tag_oid(mediagit_dir: &Path, tag_ref: &str) -> String {
    let refdb = RefDatabase::new(mediagit_dir);
    let r = refdb
        .read(tag_ref)
        .await
        .unwrap_or_else(|e| panic!("{} not found after fetch: {}", tag_ref, e));
    r.oid
        .unwrap_or_else(|| panic!("{} has no OID", tag_ref))
        .to_hex()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn fetch_brings_new_tags_into_existing_clone() {
    let repos_root = TempDir::new().unwrap();
    let (base_url, _handle) = start_test_server(repos_root.path().to_path_buf()).await;

    // ---- Source repo: initial commit, push branch ----
    let source_dir = TempDir::new().unwrap();
    init_repo(source_dir.path());
    add_and_commit(source_dir.path(), "f.txt", "content\n", "initial");

    fs::create_dir_all(repos_root.path().join("fetch-tags-repo")).unwrap();
    let remote_url = format!("{}/{}", base_url, "fetch-tags-repo");
    mediagit()
        .arg("remote")
        .arg("add")
        .arg("origin")
        .arg(&remote_url)
        .current_dir(source_dir.path())
        .assert()
        .success();
    mediagit()
        .arg("push")
        .arg("-u")
        .arg("origin")
        .current_dir(source_dir.path())
        .assert()
        .success();

    // ---- Pre-existing clone B, made BEFORE any tags exist ----
    let clone_parent = TempDir::new().unwrap();
    let clone_dir = clone_parent.path().join("existing-clone");
    mediagit()
        .arg("clone")
        .arg(&remote_url)
        .arg(&clone_dir)
        .current_dir(clone_parent.path())
        .assert()
        .success();

    // Sanity: no tags yet.
    mediagit()
        .arg("tag")
        .arg("list")
        .current_dir(&clone_dir)
        .assert()
        .success()
        .stdout(predicate::str::contains("v1.0").not());

    // ---- Source: create lightweight + annotated tags on the SAME commit,
    // then push them. (No --tags flag on fetch — the design under test.) ----
    mediagit()
        .arg("tag")
        .arg("create")
        .arg("v1.0")
        .current_dir(source_dir.path())
        .assert()
        .success();
    mediagit()
        .arg("tag")
        .arg("create")
        .arg("v2.0-annotated")
        .arg("-m")
        .arg("Release v2.0 notes")
        .current_dir(source_dir.path())
        .assert()
        .success();
    mediagit()
        .arg("push")
        .arg("origin")
        .arg("--tags")
        .current_dir(source_dir.path())
        .assert()
        .success();

    let source_head =
        read_ref_oid_from_disk(&source_dir.path().join(".mediagit"), "refs/heads/main");
    // Ground truth for the annotated tag's OID: the Tag *object*, not the
    // commit it targets (refs/tags/<name> points at the Tag object).
    let source_annotated_tag_oid = read_ref_oid_from_disk(
        &source_dir.path().join(".mediagit"),
        "refs/tags/v2.0-annotated",
    );

    // ---- Fetch into the pre-existing clone: no --tags flag exists; tags
    // must arrive automatically. ----
    mediagit()
        .arg("fetch")
        .arg("origin")
        .current_dir(&clone_dir)
        .assert()
        .success();

    let clone_mediagit = clone_dir.join(".mediagit");
    assert_eq!(
        read_tag_oid(&clone_mediagit, "refs/tags/v1.0").await,
        source_head,
        "lightweight tag v1.0 did not arrive with correct OID"
    );
    assert_eq!(
        read_tag_oid(&clone_mediagit, "refs/tags/v2.0-annotated").await,
        source_annotated_tag_oid,
        "annotated tag v2.0-annotated did not arrive with correct OID"
    );

    // The Tag object itself must be fetched (it's never reachable from a
    // commit tree) so its message content round-trips.
    let clone_odb = open_odb(&clone_dir).await;
    let tag_oid = Oid::from_hex(&source_annotated_tag_oid).unwrap();
    let tag = Tag::read(&clone_odb, &tag_oid)
        .await
        .expect("annotated tag object not fetched into local ODB");
    assert_eq!(tag.message, "Release v2.0 notes");
    assert_eq!(tag.target.to_hex(), source_head);

    // ---- Idempotency: re-fetch with nothing new must succeed and leave
    // the same state (no error, no duplicate/corruption). ----
    mediagit()
        .arg("fetch")
        .arg("origin")
        .current_dir(&clone_dir)
        .assert()
        .success();
    assert_eq!(
        read_tag_oid(&clone_mediagit, "refs/tags/v1.0").await,
        source_head,
        "re-fetch changed v1.0 OID"
    );
    assert_eq!(
        read_tag_oid(&clone_mediagit, "refs/tags/v2.0-annotated").await,
        source_annotated_tag_oid,
        "re-fetch changed v2.0-annotated OID"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn fetch_tag_on_unreachable_commit_pulls_its_own_objects() {
    // The critical case: a tag points at a commit that is NOT reachable from
    // any branch the clone fetches (an "unmerged"/unpushed-branch commit,
    // reachable only via the tag itself). A naive tag-ref-write-only fix
    // would write refs/tags/<name> pointing at an OID (the Tag object, whose
    // target commit/tree/blob are absent locally) — this test asserts the
    // full object closure actually lands.
    let repos_root = TempDir::new().unwrap();
    let (base_url, _handle) = start_test_server(repos_root.path().to_path_buf()).await;

    let source_dir = TempDir::new().unwrap();
    init_repo(source_dir.path());
    add_and_commit(source_dir.path(), "f.txt", "content\n", "initial");

    fs::create_dir_all(repos_root.path().join("fetch-tags-unreachable-repo")).unwrap();
    let remote_url = format!("{}/{}", base_url, "fetch-tags-unreachable-repo");
    mediagit()
        .arg("remote")
        .arg("add")
        .arg("origin")
        .arg(&remote_url)
        .current_dir(source_dir.path())
        .assert()
        .success();
    mediagit()
        .arg("push")
        .arg("-u")
        .arg("origin")
        .current_dir(source_dir.path())
        .assert()
        .success();

    // Pre-existing clone made from `main` only.
    let clone_parent = TempDir::new().unwrap();
    let clone_dir = clone_parent.path().join("existing-clone2");
    mediagit()
        .arg("clone")
        .arg(&remote_url)
        .arg(&clone_dir)
        .current_dir(clone_parent.path())
        .assert()
        .success();

    // Source: branch off, add a commit that is NEVER pushed as a branch,
    // tag it (annotated) there, then switch back to main before pushing —
    // pushing only refs/heads/main (already up to date, no-op) + tags.
    // `push --tags` with no refspec always includes the current branch, so
    // pushing while still on the un-tracked "feature" branch would trip the
    // "no upstream" guard even though we only want the tag transferred.
    mediagit()
        .arg("branch")
        .arg("create")
        .arg("feature")
        .current_dir(source_dir.path())
        .assert()
        .success();
    mediagit()
        .arg("branch")
        .arg("switch")
        .arg("feature")
        .current_dir(source_dir.path())
        .assert()
        .success();
    add_and_commit(
        source_dir.path(),
        "feature.txt",
        "feature content\n",
        "feature work",
    );

    mediagit()
        .arg("tag")
        .arg("create")
        .arg("v-feature")
        .arg("-m")
        .arg("Feature snapshot")
        .current_dir(source_dir.path())
        .assert()
        .success();

    let feature_head =
        read_ref_oid_from_disk(&source_dir.path().join(".mediagit"), "refs/heads/feature");
    let source_tag_oid =
        read_ref_oid_from_disk(&source_dir.path().join(".mediagit"), "refs/tags/v-feature");

    mediagit()
        .arg("branch")
        .arg("switch")
        .arg("main")
        .current_dir(source_dir.path())
        .assert()
        .success();
    mediagit()
        .arg("push")
        .arg("origin")
        .arg("--tags")
        .current_dir(source_dir.path())
        .assert()
        .success();

    // Confirm the "feature" branch itself was never pushed (this is the
    // point of the test — the tag's commit isn't reachable via any branch
    // fetch would otherwise pull).
    let server_repo_dir = repos_root.path().join("fetch-tags-unreachable-repo");
    assert!(
        !server_repo_dir
            .join(".mediagit/refs/heads/feature")
            .exists(),
        "test setup invalid: feature branch must not exist on the server"
    );

    mediagit()
        .arg("fetch")
        .arg("origin")
        .current_dir(&clone_dir)
        .assert()
        .success();

    let clone_mediagit = clone_dir.join(".mediagit");
    assert_eq!(
        read_tag_oid(&clone_mediagit, "refs/tags/v-feature").await,
        source_tag_oid,
        "v-feature tag did not arrive with correct OID"
    );

    // The Tag object AND its target commit (both unreachable from any
    // fetched branch) must actually be present locally — not just the ref
    // pointing at a hole.
    let clone_odb = open_odb(&clone_dir).await;
    let tag_oid = Oid::from_hex(&source_tag_oid).unwrap();
    let tag = Tag::read(&clone_odb, &tag_oid)
        .await
        .expect("tag object was not fetched into the local ODB");
    assert_eq!(tag.message, "Feature snapshot");
    assert_eq!(tag.target.to_hex(), feature_head);

    let commit_oid = Oid::from_hex(&feature_head).unwrap();
    assert!(
        clone_odb.read(&commit_oid).await.is_ok(),
        "tag's target commit object was not fetched into the local ODB"
    );
}
