// SPDX-License-Identifier: BUSL-1.1
// Copyright (C) 2025-2026 Aswin Krishnamoorthy

//! BUG-032: `push --force-with-lease` must use the LOCAL remote-tracking ref
//! (what this clone last saw) as the lease, not the server's fresh answer.

use assert_cmd::Command;
use std::fs;
use std::path::Path;
use std::sync::Arc;
use tempfile::TempDir;
use tokio::net::TcpListener;

#[allow(deprecated)]
fn mediagit() -> Command {
    let mut c = Command::cargo_bin("mediagit").unwrap();
    c.env("MEDIAGIT_AUTHOR_NAME", "Test User")
        .env("MEDIAGIT_AUTHOR_EMAIL", "test@example.com");
    c
}

fn run(dir: &Path, args: &[&str]) -> assert_cmd::assert::Assert {
    mediagit().args(args).current_dir(dir).assert()
}

fn commit_file(dir: &Path, name: &str, content: &str, message: &str) {
    fs::write(dir.join(name), content).unwrap();
    run(dir, &["add", name]).success();
    run(dir, &["commit", "-m", message]).success();
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

async fn server_tip(repos_root: &Path, repo: &str) -> String {
    let refdb = mediagit_versioning::RefDatabase::new(repos_root.join(repo).join(".mediagit"));
    refdb
        .read("refs/heads/main")
        .await
        .unwrap()
        .oid
        .unwrap()
        .to_hex()
}

async fn tracking(dir: &Path, branch: &str) -> Option<String> {
    let refdb = mediagit_versioning::RefDatabase::new(dir.join(".mediagit"));
    refdb
        .read(&format!("refs/remotes/origin/{branch}"))
        .await
        .ok()
        .and_then(|r| r.oid)
        .map(|o| o.to_hex())
}

async fn server_branch(repos_root: &Path, repo: &str, branch: &str) -> Option<String> {
    let refdb = mediagit_versioning::RefDatabase::new(repos_root.join(repo).join(".mediagit"));
    refdb
        .read(&format!("refs/heads/{branch}"))
        .await
        .ok()
        .and_then(|r| r.oid)
        .map(|o| o.to_hex())
}

struct Setup {
    repos_root: TempDir,
    _parents: TempDir,
    ana: std::path::PathBuf,
    ben: std::path::PathBuf,
    _server: tokio::task::JoinHandle<()>,
}

/// Source pushes v1; Ana and Ben clone it; Ben pushes b1. Ana is now stale.
async fn stale_ana() -> Setup {
    let repos_root = TempDir::new().unwrap();
    let (base_url, server) = start_test_server(repos_root.path().to_path_buf()).await;
    fs::create_dir_all(repos_root.path().join("lease-repo")).unwrap();
    let url = format!("{}/lease-repo", base_url);

    let source = TempDir::new().unwrap();
    run(source.path(), &["init", "-q"]).success();
    commit_file(source.path(), "a.txt", "v1\n", "v1");
    run(source.path(), &["remote", "add", "origin", &url]).success();
    run(source.path(), &["push", "-u", "origin"]).success();

    let parents = TempDir::new().unwrap();
    let ana = parents.path().join("ana");
    let ben = parents.path().join("ben");
    run(parents.path(), &["clone", &url, ana.to_str().unwrap()]).success();
    run(parents.path(), &["clone", &url, ben.to_str().unwrap()]).success();

    commit_file(&ben, "b.txt", "b1\n", "b1");
    run(&ben, &["push"]).success();
    commit_file(&ana, "c.txt", "a1\n", "a1");

    Setup {
        repos_root,
        _parents: parents,
        ana,
        ben,
        _server: server,
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn stale_lease_is_rejected_and_server_keeps_bens_work() {
    let s = stale_ana().await;
    let before = server_tip(s.repos_root.path(), "lease-repo").await;
    let tracking_before = tracking(&s.ana, "main").await;

    let out = mediagit()
        .args(["push", "--force-with-lease", "origin", "main"])
        .current_dir(&s.ana)
        .output()
        .unwrap();
    assert!(
        !out.status.success(),
        "stale lease must be refused:\n{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        text.contains("changed since last fetch"),
        "message:
{text}"
    );
    assert_eq!(
        server_tip(s.repos_root.path(), "lease-repo").await,
        before,
        "Ben's tip must survive"
    );
    assert_eq!(
        tracking(&s.ana, "main").await,
        tracking_before,
        "tracking ref must not move on a refused push"
    );
    let _ = &s.ben;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn lease_succeeds_after_fetch_and_updates_tracking_ref() {
    let s = stale_ana().await;
    // Rebase onto Ben's work by merging, then the lease matches.
    run(&s.ana, &["fetch", "origin"]).success();
    run(&s.ana, &["merge", "origin/main"]).success();
    run(&s.ana, &["push", "--force-with-lease", "origin", "main"]).success();

    let tip = server_tip(s.repos_root.path(), "lease-repo").await;
    let refdb = mediagit_versioning::RefDatabase::new(s.ana.join(".mediagit"));
    let tracking = refdb.read("refs/remotes/origin/main").await.unwrap();
    assert_eq!(tracking.oid.unwrap().to_hex(), tip);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn lease_without_tracking_ref_says_fetch_first() {
    let s = stale_ana().await;
    let before = server_tip(s.repos_root.path(), "lease-repo").await;
    fs::remove_file(s.ana.join(".mediagit/refs/remotes/origin/main")).unwrap();

    let out = mediagit()
        .args(["push", "--force-with-lease", "origin", "main"])
        .current_dir(&s.ana)
        .output()
        .unwrap();
    assert!(!out.status.success());
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(text.contains("fetch"), "expected fetch-first hint:\n{text}");
    assert_eq!(server_tip(s.repos_root.path(), "lease-repo").await, before);
}

/// Two branches (main, feat) at v1 on the server; Ben advances feat only.
/// Ana has both branches locally with fresh tracking for main, stale for feat.
async fn stale_feat() -> Setup {
    let repos_root = TempDir::new().unwrap();
    let (base_url, server) = start_test_server(repos_root.path().to_path_buf()).await;
    fs::create_dir_all(repos_root.path().join("lease-repo")).unwrap();
    let url = format!("{}/lease-repo", base_url);

    let source = TempDir::new().unwrap();
    run(source.path(), &["init", "-q"]).success();
    commit_file(
        source.path(),
        "a.txt",
        "v1
",
        "v1",
    );
    run(source.path(), &["branch", "create", "feat"]).success();
    run(source.path(), &["remote", "add", "origin", &url]).success();
    run(source.path(), &["push", "-u", "origin"]).success();
    run(source.path(), &["push", "origin", "feat"]).success();

    let parents = TempDir::new().unwrap();
    let ana = parents.path().join("ana");
    let ben = parents.path().join("ben");
    run(parents.path(), &["clone", &url, ana.to_str().unwrap()]).success();
    run(parents.path(), &["clone", &url, ben.to_str().unwrap()]).success();

    run(&ben, &["branch", "create", "feat"]).success();
    run(&ben, &["branch", "switch", "feat"]).success();
    commit_file(
        &ben, "b.txt", "b1
", "b1",
    );
    run(&ben, &["push", "origin", "feat"]).success();

    run(&ana, &["branch", "create", "feat"]).success();
    Setup {
        repos_root,
        _parents: parents,
        ana,
        ben,
        _server: server,
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn push_all_with_one_stale_branch_changes_nothing_and_names_it() {
    let s = stale_feat().await;
    commit_file(
        &s.ana,
        "c.txt",
        "a1
",
        "a1 on main",
    );
    run(&s.ana, &["branch", "switch", "feat"]).success();
    commit_file(
        &s.ana,
        "d.txt",
        "a2
",
        "a2 on feat",
    );

    let main_before = server_branch(s.repos_root.path(), "lease-repo", "main").await;
    let feat_before = server_branch(s.repos_root.path(), "lease-repo", "feat").await;
    let main_tracking = tracking(&s.ana, "main").await;

    let out = mediagit()
        .args(["push", "--all", "--force-with-lease", "origin"])
        .current_dir(&s.ana)
        .output()
        .unwrap();
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        !out.status.success(),
        "must fail:
{text}"
    );
    assert!(
        text.contains("refs/heads/feat: ref changed since last fetch"),
        "must name the stale branch:
{text}"
    );
    assert_eq!(
        server_branch(s.repos_root.path(), "lease-repo", "main").await,
        main_before,
        "fresh branch must not be written when another is stale"
    );
    assert_eq!(
        server_branch(s.repos_root.path(), "lease-repo", "feat").await,
        feat_before
    );
    assert_eq!(tracking(&s.ana, "main").await, main_tracking);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn delete_with_stale_lease_is_refused_and_fresh_lease_deletes() {
    let s = stale_feat().await;
    let feat_before = server_branch(s.repos_root.path(), "lease-repo", "feat").await;
    assert!(feat_before.is_some());

    let args = ["push", "--delete", "--force-with-lease", "origin", "feat"];
    let out = mediagit().args(args).current_dir(&s.ana).output().unwrap();
    assert!(!out.status.success(), "stale delete must fail");
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        text.contains("ref changed since last fetch"),
        "must be the server's lease refusal, not the client fetch-first bail:
{text}"
    );
    assert_eq!(
        server_branch(s.repos_root.path(), "lease-repo", "feat").await,
        feat_before,
        "remote branch must stay"
    );

    run(&s.ana, &["fetch", "origin", "feat"]).success();
    let out = mediagit().args(args).current_dir(&s.ana).output().unwrap();
    assert!(
        out.status.success(),
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(
        server_branch(s.repos_root.path(), "lease-repo", "feat").await,
        None
    );
}
