// SPDX-License-Identifier: BUSL-1.1
// Copyright (C) 2025-2026 Aswin Krishnamoorthy

//! Upstream-tracking plumbing tests (M2 Step 3).
//!
//! `clone` writes tracking for the default branch; `branch switch -c
//! --track <remote>/<branch>` records it too. M2 only owns the plumbing
//! (write + read-back via `mediagit_config::Config::get_branch_upstream`) —
//! ahead/behind computation and status display are M4's job.

use assert_cmd::Command;
use std::fs;
use std::path::Path;
use std::sync::Arc;
use tempfile::TempDir;
use tokio::net::TcpListener;

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

fn read_config(repo_dir: &Path) -> mediagit_config::Config {
    let toml_str = fs::read_to_string(repo_dir.join(".mediagit/config.toml")).unwrap();
    toml::from_str(&toml_str).unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn clone_writes_upstream_tracking_for_default_branch() {
    let repos_root = TempDir::new().unwrap();
    let (base_url, _handle) = start_test_server(repos_root.path().to_path_buf()).await;

    // Set up and push a source repo with the default branch "main".
    let source_dir = TempDir::new().unwrap();
    init_repo(source_dir.path());
    fs::write(source_dir.path().join("f.txt"), "content\n").unwrap();
    mediagit()
        .arg("add")
        .arg("f.txt")
        .current_dir(source_dir.path())
        .assert()
        .success();
    mediagit()
        .arg("commit")
        .arg("-m")
        .arg("initial")
        .current_dir(source_dir.path())
        .assert()
        .success();
    fs::create_dir_all(repos_root.path().join("track-clone-repo")).unwrap();
    let remote_url = format!("{}/{}", base_url, "track-clone-repo");
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

    // Clone it fresh.
    let clone_parent = TempDir::new().unwrap();
    let clone_dir = clone_parent.path().join("cloned");
    mediagit()
        .arg("clone")
        .arg(&remote_url)
        .arg(&clone_dir)
        .current_dir(clone_parent.path())
        .assert()
        .success();

    let config = read_config(&clone_dir);
    let upstream = config.get_branch_upstream("main");
    assert_eq!(upstream, Some(("origin", "refs/heads/main")));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn branch_track_shorthand_records_upstream() {
    let repos_root = TempDir::new().unwrap();
    let (base_url, _handle) = start_test_server(repos_root.path().to_path_buf()).await;

    let source_dir = TempDir::new().unwrap();
    init_repo(source_dir.path());
    fs::write(source_dir.path().join("f.txt"), "content\n").unwrap();
    mediagit()
        .arg("add")
        .arg("f.txt")
        .current_dir(source_dir.path())
        .assert()
        .success();
    mediagit()
        .arg("commit")
        .arg("-m")
        .arg("initial")
        .current_dir(source_dir.path())
        .assert()
        .success();
    fs::create_dir_all(repos_root.path().join("track-branch-repo")).unwrap();
    let remote_url = format!("{}/{}", base_url, "track-branch-repo");
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

    // A second branch pushed to the remote so we have something distinct to track.
    mediagit()
        .arg("branch")
        .arg("create")
        .arg("feat-a")
        .current_dir(source_dir.path())
        .assert()
        .success();
    mediagit()
        .arg("branch")
        .arg("switch")
        .arg("feat-a")
        .current_dir(source_dir.path())
        .assert()
        .success();
    mediagit()
        .arg("push")
        .arg("-u")
        .arg("origin")
        .arg("feat-a")
        .current_dir(source_dir.path())
        .assert()
        .success();

    // Clone (fetches only default branch objects, but ref negotiation
    // exposes remote branches) then fetch feat-a to get its tracking ref.
    let clone_parent = TempDir::new().unwrap();
    let clone_dir = clone_parent.path().join("cloned2");
    mediagit()
        .arg("clone")
        .arg(&remote_url)
        .arg(&clone_dir)
        .current_dir(clone_parent.path())
        .assert()
        .success();
    mediagit()
        .arg("fetch")
        .arg("origin")
        .arg("feat-a")
        .current_dir(&clone_dir)
        .assert()
        .success();

    // Create+switch to a local branch tracking origin/feat-a via shorthand.
    mediagit()
        .arg("branch")
        .arg("switch")
        .arg("-c")
        .arg("--track")
        .arg("origin/feat-a")
        .current_dir(&clone_dir)
        .assert()
        .success();

    let config = read_config(&clone_dir);
    // Local branch is named "feat-a" (shorthand strips the remote prefix).
    let upstream = config.get_branch_upstream("feat-a");
    assert_eq!(upstream, Some(("origin", "refs/heads/feat-a")));
}

#[test]
fn upstream_config_round_trips_through_save_and_load() {
    let temp_dir = TempDir::new().unwrap();
    let mut config = mediagit_config::Config::default();
    config.set_branch_upstream("main", "origin", "refs/heads/main");
    config.save(temp_dir.path()).unwrap();

    let reloaded = read_config(temp_dir.path());
    assert_eq!(
        reloaded.get_branch_upstream("main"),
        Some(("origin", "refs/heads/main"))
    );
}
