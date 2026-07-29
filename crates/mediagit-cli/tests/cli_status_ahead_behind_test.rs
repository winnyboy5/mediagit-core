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

//! M4: `status` ahead/behind + upstream display, and `--json` schema.
//!
//! `status.rs`'s `StatusCmd::execute` builds one `StatusReport` struct per
//! invocation; the human branch header and `--json` both render from that
//! same struct (see the doc comment on `StatusReport` in
//! `commands/status.rs`). The struct itself isn't reachable from here — it
//! lives in the `mediagit-cli` binary's private `commands` tree, not the
//! library surface — so this file mirrors its JSON shape instead, the same
//! pattern `cli_phash_test.rs` uses for `mediagit_cli::phash_index::Entry`.
//! The cross-checks below (human text vs. JSON fields for the same repo
//! state) are the actual "one struct feeds both" regression guard.

use assert_cmd::Command;
use serde::Deserialize;
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

fn commit_file(dir: &Path, name: &str, content: &str, message: &str) {
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

fn read_config(repo_dir: &Path) -> mediagit_config::Config {
    let toml_str = fs::read_to_string(repo_dir.join(".mediagit/config.toml")).unwrap();
    toml::from_str(&toml_str).unwrap()
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

#[derive(Debug, Deserialize)]
struct TestUpstream {
    name: String,
    ahead: Option<u64>,
    behind: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct TestBranch {
    name: Option<String>,
    upstream: Option<TestUpstream>,
}

#[derive(Debug, Deserialize)]
struct TestReport {
    format_version: u32,
    branch: TestBranch,
    staged: Vec<serde_json::Value>,
    modified: Vec<String>,
    deleted: Vec<String>,
    untracked: Vec<String>,
    #[allow(dead_code)]
    ignored: Vec<String>,
    #[allow(dead_code)]
    summary: serde_json::Value,
}

fn status_json(dir: &Path) -> TestReport {
    let output = mediagit()
        .arg("status")
        .arg("--json")
        .current_dir(dir)
        .output()
        .unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    serde_json::from_str(&stdout).unwrap_or_else(|e| panic!("bad json: {e}\n{stdout}"))
}

fn status_human_branch(dir: &Path) -> String {
    let output = mediagit()
        .arg("status")
        .arg("-b")
        .current_dir(dir)
        .output()
        .unwrap();
    String::from_utf8_lossy(&output.stdout).to_string()
}

#[test]
fn no_upstream_header_unchanged_and_json_null() {
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());
    commit_file(temp_dir.path(), "a.txt", "hello\n", "initial");

    let human = status_human_branch(temp_dir.path());
    assert!(human.contains("On branch: main"), "got:\n{human}");
    assert!(
        !human.contains('\u{2014}'),
        "no-upstream header must not gain a suffix, got:\n{human}"
    );

    let report = status_json(temp_dir.path());
    assert_eq!(report.format_version, 1);
    assert!(report.branch.upstream.is_none());
}

#[test]
fn gone_upstream_shows_name_without_counts() {
    // Upstream configured but the tracking ref was never fetched locally —
    // status must show the upstream name with no counts, and must not
    // touch the network to find out (no server started in this test).
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());
    commit_file(temp_dir.path(), "a.txt", "hello\n", "initial");

    let mut config = read_config(temp_dir.path());
    config.set_branch_upstream("main", "origin", "refs/heads/main");
    config.save(temp_dir.path()).unwrap();
    // Deliberately no refs/remotes/origin/main written.

    let human = status_human_branch(temp_dir.path());
    assert!(human.contains("origin/main"), "got:\n{human}");
    assert!(!human.contains("ahead"), "got:\n{human}");
    assert!(!human.contains("behind"), "got:\n{human}");

    let report = status_json(temp_dir.path());
    let upstream = report.branch.upstream.expect("upstream should be reported");
    assert_eq!(upstream.name, "origin/main");
    assert_eq!(upstream.ahead, None);
    assert_eq!(upstream.behind, None);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ahead_only_reports_local_commits_not_on_remote() {
    let repos_root = TempDir::new().unwrap();
    let (base_url, _handle) = start_test_server(repos_root.path().to_path_buf()).await;

    let source_dir = TempDir::new().unwrap();
    init_repo(source_dir.path());
    commit_file(source_dir.path(), "a.txt", "v1\n", "initial");

    fs::create_dir_all(repos_root.path().join("ahead-repo")).unwrap();
    let remote_url = format!("{}/{}", base_url, "ahead-repo");
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

    // Local commit not yet pushed.
    commit_file(source_dir.path(), "b.txt", "v2\n", "second");

    let human = status_human_branch(source_dir.path());
    assert!(human.contains("ahead 1"), "got:\n{human}");
    assert!(!human.contains("behind"), "got:\n{human}");

    let report = status_json(source_dir.path());
    let upstream = report.branch.upstream.expect("upstream should be reported");
    assert_eq!(upstream.ahead, Some(1));
    assert_eq!(upstream.behind, Some(0));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn behind_only_reports_remote_commits_not_local() {
    let repos_root = TempDir::new().unwrap();
    let (base_url, _handle) = start_test_server(repos_root.path().to_path_buf()).await;

    let source_dir = TempDir::new().unwrap();
    init_repo(source_dir.path());
    commit_file(source_dir.path(), "a.txt", "v1\n", "initial");

    fs::create_dir_all(repos_root.path().join("behind-repo")).unwrap();
    let remote_url = format!("{}/{}", base_url, "behind-repo");
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

    let clone_parent = TempDir::new().unwrap();
    let clone_dir = clone_parent.path().join("cloned");
    mediagit()
        .arg("clone")
        .arg(&remote_url)
        .arg(&clone_dir)
        .current_dir(clone_parent.path())
        .assert()
        .success();

    // Advance the remote from the second clone.
    commit_file(&clone_dir, "b.txt", "v2\n", "second");
    mediagit()
        .arg("push")
        .current_dir(&clone_dir)
        .assert()
        .success();

    // source_dir doesn't know yet — fetch only updates the tracking ref.
    mediagit()
        .arg("fetch")
        .arg("origin")
        .current_dir(source_dir.path())
        .assert()
        .success();

    let human = status_human_branch(source_dir.path());
    assert!(human.contains("behind 1"), "got:\n{human}");
    assert!(!human.contains("ahead"), "got:\n{human}");

    let report = status_json(source_dir.path());
    let upstream = report.branch.upstream.expect("upstream should be reported");
    assert_eq!(upstream.ahead, Some(0));
    assert_eq!(upstream.behind, Some(1));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn diverged_reports_both_ahead_and_behind() {
    let repos_root = TempDir::new().unwrap();
    let (base_url, _handle) = start_test_server(repos_root.path().to_path_buf()).await;

    let source_dir = TempDir::new().unwrap();
    init_repo(source_dir.path());
    commit_file(source_dir.path(), "a.txt", "v1\n", "initial");

    fs::create_dir_all(repos_root.path().join("diverged-repo")).unwrap();
    let remote_url = format!("{}/{}", base_url, "diverged-repo");
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

    let clone_parent = TempDir::new().unwrap();
    let clone_dir = clone_parent.path().join("cloned");
    mediagit()
        .arg("clone")
        .arg(&remote_url)
        .arg(&clone_dir)
        .current_dir(clone_parent.path())
        .assert()
        .success();

    // Remote gains a commit from the second clone...
    commit_file(&clone_dir, "b.txt", "v2\n", "remote-side");
    mediagit()
        .arg("push")
        .current_dir(&clone_dir)
        .assert()
        .success();
    mediagit()
        .arg("fetch")
        .arg("origin")
        .current_dir(source_dir.path())
        .assert()
        .success();

    // ...while source_dir independently gains its own, unpushed commit.
    commit_file(source_dir.path(), "c.txt", "v3\n", "local-side");

    let human = status_human_branch(source_dir.path());
    assert!(human.contains("ahead 1"), "got:\n{human}");
    assert!(human.contains("behind 1"), "got:\n{human}");

    let report = status_json(source_dir.path());
    let upstream = report.branch.upstream.expect("upstream should be reported");
    assert_eq!(upstream.ahead, Some(1));
    assert_eq!(upstream.behind, Some(1));
}

#[test]
fn json_schema_round_trips_and_matches_human_output() {
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());
    commit_file(temp_dir.path(), "a.txt", "v1\n", "initial");
    fs::write(temp_dir.path().join("a.txt"), "v1 changed\n").unwrap();
    fs::write(temp_dir.path().join("untracked.txt"), "new\n").unwrap();

    let report = status_json(temp_dir.path());
    assert_eq!(report.format_version, 1);
    assert_eq!(report.branch.name.as_deref(), Some("main"));
    assert_eq!(report.modified, vec!["a.txt".to_string()]);
    assert_eq!(report.untracked, vec!["untracked.txt".to_string()]);
    assert!(report.staged.is_empty());
    assert!(report.deleted.is_empty());

    // Same repo state via the human path — both must agree, since both are
    // rendered from the one StatusReport built in StatusCmd::execute.
    let human = mediagit()
        .arg("status")
        .current_dir(temp_dir.path())
        .output()
        .unwrap();
    let human_stdout = String::from_utf8_lossy(&human.stdout);
    assert!(human_stdout.contains("a.txt"));
    assert!(human_stdout.contains("untracked.txt"));
}

#[test]
fn json_output_is_a_single_document_with_no_leading_text() {
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());
    commit_file(temp_dir.path(), "a.txt", "v1\n", "initial");

    let output = mediagit()
        .arg("status")
        .arg("--json")
        .current_dir(temp_dir.path())
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    // If the "Repository Status" header (or any other human text) leaked in
    // ahead of the JSON, this parse fails — --json must skip all of it.
    let _: serde_json::Value = serde_json::from_str(&stdout).unwrap_or_else(|e| {
        panic!("expected --json stdout to be a single JSON document: {e}\n{stdout}")
    });
}
