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

//! Integration tests for `mediagit download` (#6, M2 Step 2).
//!
//! Spins up an in-process MediaGit server (same pattern as
//! `mediagit-server/tests/e2e_push_pull.rs`), pushes a real repo to it via
//! the `mediagit` binary, then exercises `mediagit download` against the
//! server without a local repository — the whole point of the command.

use assert_cmd::Command;
use predicates::prelude::*;
use std::fs;
use std::path::Path;
use std::sync::{Arc, Mutex};
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

/// Start an in-process MediaGit server on a random port serving `repos_dir`.
/// Returns the base URL (no trailing slash, no repo segment).
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

/// Set up a repo with one committed file, push it to `base_url/repo_name`.
/// Returns the commit oid (stdout of `commit` isn't parsed; instead reads
/// `mediagit show --oneline` via `log -n 1` for the OID).
///
/// `repos_root` is the server's repos directory — the target repo directory
/// must exist there before a push can succeed (`GET /info/refs` 404s on a
/// server-side directory that was never created; matches
/// `mediagit-cli/tests/server_integration_test.rs`'s `fs::create_dir_all`
/// pattern for `bare_repo`).
fn push_repo_with_file(
    repos_root: &Path,
    local_dir: &Path,
    base_url: &str,
    repo_name: &str,
    file_name: &str,
    content: &str,
) -> String {
    fs::create_dir_all(repos_root.join(repo_name)).unwrap();
    init_repo(local_dir);
    fs::write(local_dir.join(file_name), content).unwrap();
    mediagit()
        .arg("add")
        .arg(file_name)
        .current_dir(local_dir)
        .assert()
        .success();
    mediagit()
        .arg("commit")
        .arg("-m")
        .arg("add file")
        .current_dir(local_dir)
        .assert()
        .success();

    let remote_url = format!("{}/{}", base_url, repo_name);
    mediagit()
        .arg("remote")
        .arg("add")
        .arg("origin")
        .arg(&remote_url)
        .current_dir(local_dir)
        .assert()
        .success();

    mediagit()
        .arg("push")
        .arg("-u")
        .arg("origin")
        .current_dir(local_dir)
        .assert()
        .success();

    // Capture the commit OID via `log -n 1` oneline output ("<oid8> ...").
    let output = mediagit()
        .arg("log")
        .arg("--oneline")
        .arg("-n")
        .arg("1")
        .current_dir(local_dir)
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    stdout
        .split_whitespace()
        .next()
        .expect("log --oneline produced output")
        .to_string()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn download_by_full_url_matches_committed_file_exactly() {
    let repos_root = TempDir::new().unwrap();
    let (base_url, _handle) = start_test_server(repos_root.path().to_path_buf()).await;

    let repo_dir = TempDir::new().unwrap();
    let content = "hello from mediagit download\nsecond line\n";
    push_repo_with_file(
        repos_root.path(),
        repo_dir.path(),
        &base_url,
        "dl-repo",
        "greeting.txt",
        content,
    );

    // Download WITHOUT a local repository — a fresh empty directory, no `init`.
    let out_dir = TempDir::new().unwrap();
    let file_url = format!("{}/dl-repo/greeting.txt", base_url);
    mediagit()
        .arg("download")
        .arg(&file_url)
        .current_dir(out_dir.path())
        .assert()
        .success();

    let downloaded = fs::read_to_string(out_dir.path().join("greeting.txt")).unwrap();
    assert_eq!(downloaded, content);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn download_honors_ref_flag() {
    let repos_root = TempDir::new().unwrap();
    let (base_url, _handle) = start_test_server(repos_root.path().to_path_buf()).await;

    let repo_dir = TempDir::new().unwrap();
    let first_oid = push_repo_with_file(
        repos_root.path(),
        repo_dir.path(),
        &base_url,
        "ref-repo",
        "data.txt",
        "version-one\n",
    );

    // Second commit changes the file and pushes again.
    fs::write(repo_dir.path().join("data.txt"), "version-two\n").unwrap();
    mediagit()
        .arg("add")
        .arg("data.txt")
        .current_dir(repo_dir.path())
        .assert()
        .success();
    mediagit()
        .arg("commit")
        .arg("-m")
        .arg("update file")
        .current_dir(repo_dir.path())
        .assert()
        .success();
    mediagit()
        .arg("push")
        .current_dir(repo_dir.path())
        .assert()
        .success();

    // Downloading HEAD (default) gets the new content.
    let out_dir = TempDir::new().unwrap();
    let file_url = format!("{}/ref-repo/data.txt", base_url);
    mediagit()
        .arg("download")
        .arg(&file_url)
        .arg("-o")
        .arg("head.txt")
        .current_dir(out_dir.path())
        .assert()
        .success();
    assert_eq!(
        fs::read_to_string(out_dir.path().join("head.txt")).unwrap(),
        "version-two\n"
    );

    // Downloading the first commit's ref gets the old content.
    mediagit()
        .arg("download")
        .arg(&file_url)
        .arg("--ref")
        .arg(&first_oid)
        .arg("-o")
        .arg("old.txt")
        .current_dir(out_dir.path())
        .assert()
        .success();
    assert_eq!(
        fs::read_to_string(out_dir.path().join("old.txt")).unwrap(),
        "version-one\n"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn download_rejects_path_traversal() {
    let repos_root = TempDir::new().unwrap();
    let (base_url, _handle) = start_test_server(repos_root.path().to_path_buf()).await;

    let repo_dir = TempDir::new().unwrap();
    push_repo_with_file(
        repos_root.path(),
        repo_dir.path(),
        &base_url,
        "traversal-repo",
        "f.txt",
        "x\n",
    );

    let out_dir = TempDir::new().unwrap();
    let traversal_url = format!("{}/traversal-repo/../../../etc/passwd", base_url);
    mediagit()
        .arg("download")
        .arg(&traversal_url)
        .current_dir(out_dir.path())
        .assert()
        .failure()
        .stderr(predicate::str::contains("..").or(predicate::str::contains("traversal")));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn download_missing_file_is_clean_error() {
    let repos_root = TempDir::new().unwrap();
    let (base_url, _handle) = start_test_server(repos_root.path().to_path_buf()).await;

    let repo_dir = TempDir::new().unwrap();
    push_repo_with_file(
        repos_root.path(),
        repo_dir.path(),
        &base_url,
        "missing-repo",
        "f.txt",
        "x\n",
    );

    let out_dir = TempDir::new().unwrap();
    let missing_url = format!("{}/missing-repo/does-not-exist.txt", base_url);
    mediagit()
        .arg("download")
        .arg(&missing_url)
        .current_dir(out_dir.path())
        .assert()
        .failure();

    // No stray output file should have been created for a failed download.
    assert!(!out_dir.path().join("does-not-exist.txt").exists());
}

#[test]
fn download_help() {
    mediagit()
        .arg("download")
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("download"));
}

// --- F4: full-URL credential gating (match-remote-else-strip) ---
//
// A minimal capturing server (not `mediagit_server`, since only the
// `Authorization`/`x-api-key` request headers on the browse-endpoint route
// matter here) records whatever headers arrived on
// `GET /{repo}/files/{*path}` and replies with fixed content. Every test
// below passes `--ref` explicitly to skip the `GET /info/refs` default-ref
// lookup, which this minimal server doesn't implement.
//
// Env vars are passed directly to the spawned `mediagit` subprocess via
// `Command::env`, never mutated on the test-process itself, so these tests
// don't race with `credentials_resolution_test.rs`'s in-process env
// mutation and need no shared lock. All three below now pin
// `MEDIAGIT_NO_KEYRING=1` so they exercise only the env tier and never read
// this dev machine's real OS credential store (they didn't before I11,
// which happened to be safe only because env was already checked first).
//
// I11 note: before origin-keying, `download`'s keychain lookup used the
// literal string "origin" as the account key in host-matched full-URL mode
// (repo.rs's old `resolve_credentials(&repo_root, &config, "origin")`
// call), rather than the configured remote's own URL -- a real but
// low-stakes inconsistency. Origin-keying makes this converge for free:
// both paths now key off the *origin* of the resolved "origin" remote's
// URL, which is exactly what `remote_origin_maps_explicit_default_port_and_bare_url_to_one_key`
// in `repo.rs` and the opt-in real-keychain tests in
// `credentials_resolution_test.rs` cover directly.

#[derive(Default)]
struct CapturedHeaders {
    authorization: Option<String>,
    x_api_key: Option<String>,
    hit: bool,
}

async fn start_capture_server() -> (
    String,
    Arc<Mutex<CapturedHeaders>>,
    tokio::task::JoinHandle<()>,
) {
    use axum::extract::State;
    use axum::http::HeaderMap;
    use axum::routing::get;

    let captured = Arc::new(Mutex::new(CapturedHeaders::default()));

    async fn capture_handler(
        State(captured): State<Arc<Mutex<CapturedHeaders>>>,
        headers: HeaderMap,
    ) -> &'static [u8] {
        let mut c = captured.lock().unwrap();
        c.hit = true;
        c.authorization = headers
            .get("authorization")
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_string());
        c.x_api_key = headers
            .get("x-api-key")
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_string());
        b"captured-content"
    }

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let base_url = format!("http://{}", addr);
    let app = axum::Router::new()
        .route("/{repo}/files/{*path}", get(capture_handler))
        .with_state(captured.clone());
    let handle = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
    (base_url, captured, handle)
}

fn init_repo_with_origin(dir: &Path, origin_url: &str) {
    init_repo(dir);
    mediagit()
        .arg("remote")
        .arg("add")
        .arg("origin")
        .arg(origin_url)
        .current_dir(dir)
        .assert()
        .success();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn download_full_url_no_repo_strips_env_token() {
    let (base_url, captured, _handle) = start_capture_server().await;

    // Fresh, non-repo directory — find_repo_root() must fail here.
    let out_dir = TempDir::new().unwrap();
    let file_url = format!("{}/some-repo/file.bin", base_url);
    mediagit()
        .arg("download")
        .arg(&file_url)
        .arg("--ref")
        .arg("main")
        .env("MEDIAGIT_TOKEN", "should-not-be-sent")
        .env("MEDIAGIT_NO_KEYRING", "1")
        .current_dir(out_dir.path())
        .assert()
        .success();

    let c = captured.lock().unwrap();
    assert!(c.hit, "server never received the download request");
    assert_eq!(
        c.authorization, None,
        "no repo present — token must not be attached"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn download_full_url_matching_remote_host_attaches_env_token() {
    let (base_url, captured, _handle) = start_capture_server().await;

    let repo_dir = TempDir::new().unwrap();
    // origin's URL shares scheme+host+port with the typed download URL below,
    // even though the repo segment differs — only the host triple matters.
    init_repo_with_origin(repo_dir.path(), &format!("{}/origin-repo", base_url));

    let file_url = format!("{}/some-repo/file.bin", base_url);
    mediagit()
        .arg("download")
        .arg(&file_url)
        .arg("--ref")
        .arg("main")
        .env("MEDIAGIT_TOKEN", "s3cr3t-token")
        .env("MEDIAGIT_NO_KEYRING", "1")
        .current_dir(repo_dir.path())
        .assert()
        .success();

    let c = captured.lock().unwrap();
    assert!(c.hit, "server never received the download request");
    assert_eq!(c.authorization.as_deref(), Some("Bearer s3cr3t-token"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn download_full_url_different_host_strips_env_token() {
    let (base_url, captured, _handle) = start_capture_server().await;
    // A second server on a different port stands in for "origin" — the
    // typed URL below points at `base_url`, whose port differs.
    let (other_base_url, _other_captured, _other_handle) = start_capture_server().await;

    let repo_dir = TempDir::new().unwrap();
    init_repo_with_origin(repo_dir.path(), &format!("{}/origin-repo", other_base_url));

    let file_url = format!("{}/some-repo/file.bin", base_url);
    mediagit()
        .arg("download")
        .arg(&file_url)
        .arg("--ref")
        .arg("main")
        .env("MEDIAGIT_TOKEN", "should-not-be-sent")
        .env("MEDIAGIT_NO_KEYRING", "1")
        .current_dir(repo_dir.path())
        .assert()
        .success();

    let c = captured.lock().unwrap();
    assert!(c.hit, "server never received the download request");
    assert_eq!(
        c.authorization, None,
        "origin's host differs from the typed URL — token must not be attached"
    );
}
