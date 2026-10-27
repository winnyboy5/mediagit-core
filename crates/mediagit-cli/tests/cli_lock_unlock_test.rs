// SPDX-License-Identifier: BUSL-1.1
// Copyright (C) 2025-2026 Aswin Krishnamoorthy

//! `lock unlock` on a server without auth: the owner releases their own lock
//! without `--force`, someone else cannot, and `--force` still works.
//!
//! Before, a no-auth server had no identity to compare on unlock, so every
//! plain unlock was refused (403) and only `--force` worked, including in the
//! desktop app's Unlock button on the user's own lock.

use assert_cmd::Command;
use std::fs;
use std::path::Path;
use std::sync::Arc;
use tempfile::TempDir;
use tokio::net::TcpListener;

#[allow(deprecated)]
fn mediagit(dir: &Path, author: &str) -> Command {
    let mut c = Command::cargo_bin("mediagit").unwrap();
    c.env("MEDIAGIT_AUTHOR_NAME", author)
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

fn locked_paths(dir: &Path) -> String {
    let out = mediagit(dir, "alice")
        .args(["lock", "list"])
        .assert()
        .success();
    String::from_utf8(out.get_output().stdout.clone()).unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn owner_unlocks_without_force_on_no_auth_server() {
    let repos = TempDir::new().unwrap();
    let base = start_test_server(repos.path().to_path_buf()).await;
    fs::create_dir_all(repos.path().join("locks-repo")).unwrap();

    let work = TempDir::new().unwrap();
    let p = work.path();
    mediagit(p, "alice").args(["init", "-q"]).assert().success();
    fs::write(p.join("hero.psd"), b"layers").unwrap();
    mediagit(p, "alice")
        .args(["add", "hero.psd"])
        .assert()
        .success();
    mediagit(p, "alice")
        .args(["commit", "-m", "init"])
        .assert()
        .success();
    let url = format!("{base}/locks-repo");
    mediagit(p, "alice")
        .args(["remote", "add", "origin", &url])
        .assert()
        .success();
    mediagit(p, "alice")
        .args(["push", "-u", "origin"])
        .assert()
        .success();

    // Someone else cannot release alice's lock without --force.
    mediagit(p, "alice")
        .args(["lock", "create", "hero.psd"])
        .assert()
        .success();
    mediagit(p, "bob")
        .args(["lock", "unlock", "hero.psd"])
        .assert()
        .failure();
    assert!(locked_paths(p).contains("hero.psd"));

    // The owner can, without --force.
    mediagit(p, "alice")
        .args(["lock", "unlock", "hero.psd"])
        .assert()
        .success();
    assert!(!locked_paths(p).contains("hero.psd"));

    // --force still releases anyone's lock.
    mediagit(p, "alice")
        .args(["lock", "create", "hero.psd"])
        .assert()
        .success();
    mediagit(p, "bob")
        .args(["lock", "unlock", "hero.psd", "--force"])
        .assert()
        .success();
    assert!(!locked_paths(p).contains("hero.psd"));
}
