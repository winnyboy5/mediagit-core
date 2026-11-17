// SPDX-License-Identifier: BUSL-1.1
// Copyright (C) 2025-2026 Aswin Krishnamoorthy

//! BUG-033: when both sides changed the same file, `pull` must stop in the
//! same conflict state as `merge` (markers, MERGE_HEAD, unresolved path), and
//! `merge --continue` must finish it with a two-parent commit.

use assert_cmd::Command;
use std::fs;
use std::path::Path;
use std::sync::Arc;
use tempfile::TempDir;
use tokio::net::TcpListener;

#[allow(deprecated)]
fn mediagit(dir: &Path) -> Command {
    let mut c = Command::cargo_bin("mediagit").unwrap();
    c.env("MEDIAGIT_AUTHOR_NAME", "Test User")
        .env("MEDIAGIT_AUTHOR_EMAIL", "test@example.com")
        .current_dir(dir);
    c
}

async fn start_test_server(repos_dir: std::path::PathBuf) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let state = Arc::new(mediagit_server::AppState::new(repos_dir));
    let app = mediagit_server::create_router(state);
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
    format!("http://{}", addr)
}

fn save(dir: &Path, text: &str, msg: &str) {
    fs::write(dir.join("notes.txt"), text).unwrap();
    mediagit(dir).args(["add", "notes.txt"]).assert().success();
    mediagit(dir).args(["commit", "-m", msg]).assert().success();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn pull_conflict_stops_like_merge_and_continue_makes_two_parents() {
    let repos_root = TempDir::new().unwrap();
    let base_url = start_test_server(repos_root.path().to_path_buf()).await;
    fs::create_dir_all(repos_root.path().join("p33")).unwrap();
    let url = format!("{base_url}/p33");

    let ben = TempDir::new().unwrap();
    let b = ben.path();
    mediagit(b).args(["init", "-q"]).assert().success();
    save(b, "base\n", "base");
    mediagit(b)
        .args(["remote", "add", "origin", &url])
        .assert()
        .success();
    mediagit(b)
        .args(["push", "-u", "origin"])
        .assert()
        .success();

    let parent = TempDir::new().unwrap();
    let a = parent.path().join("ana");
    mediagit(parent.path())
        .arg("clone")
        .arg(&url)
        .arg(&a)
        .assert()
        .success();

    save(b, "ben\n", "ben edit");
    mediagit(b).args(["push", "origin"]).assert().success();
    save(&a, "ana\n", "ana edit");

    // The pull stops with a conflict, exit 1, like `merge` does.
    mediagit(&a).args(["pull", "origin"]).assert().code(1);
    assert!(
        a.join(".mediagit/MERGE_HEAD").exists(),
        "MERGE_HEAD written"
    );
    let on_disk = fs::read_to_string(a.join("notes.txt")).unwrap();
    assert!(on_disk.contains("<<<<<<<"), "markers on disk: {on_disk}");
    let status = mediagit(&a).arg("status").output().unwrap();
    assert!(
        String::from_utf8_lossy(&status.stdout).contains("notes.txt"),
        "status shows the conflict"
    );

    // Resolve and finish: the commit has two parents.
    fs::write(a.join("notes.txt"), "both\n").unwrap();
    mediagit(&a).args(["add", "notes.txt"]).assert().success();
    mediagit(&a)
        .args(["merge", "--continue"])
        .assert()
        .success();
    let head = fs::read_to_string(a.join(".mediagit/refs/heads/main")).unwrap();
    let storage = mediagit_cli::repo::create_storage_backend(&a)
        .await
        .unwrap();
    let odb = mediagit_versioning::ObjectDatabase::with_smart_compression(storage, 1000);
    let commit = mediagit_versioning::Commit::read(
        &odb,
        &mediagit_versioning::Oid::from_hex(head.trim()).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(commit.parents.len(), 2, "merge commit has two parents");
}
