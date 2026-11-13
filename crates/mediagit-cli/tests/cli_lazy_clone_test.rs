// SPDX-License-Identifier: BUSL-1.1
// Copyright (C) 2025-2026 Aswin Krishnamoorthy

//! BUG-006: a fresh clone of a repository with several branches must pass
//! `fsck`, and `branch switch <other>` must fetch that branch on demand.
//!
//! A clone downloads the default branch only. It records the other tips as
//! tracking refs and lists them in `.mediagit/lazy-refs`. fsck reports a
//! listed tip as "not downloaded yet" and stays strict about everything else.

use assert_cmd::Command;
use std::fs;
use std::path::{Path, PathBuf};
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

fn run(dir: &Path, args: &[&str]) {
    mediagit().args(args).current_dir(dir).assert().success();
}

/// Run and return (success, stdout + stderr).
fn run_text(dir: &Path, args: &[&str]) -> (bool, String) {
    let out = mediagit()
        .args(args)
        .current_dir(dir)
        .assert()
        .get_output()
        .clone();
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    (out.status.success(), text)
}

fn commit_file(dir: &Path, name: &str, data: &[u8], message: &str) {
    fs::write(dir.join(name), data).unwrap();
    run(dir, &["add", name]);
    run(dir, &["commit", "-m", message]);
}

async fn start_test_server(repos_dir: PathBuf) -> (String, tokio::task::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base_url = format!("http://{}", listener.local_addr().unwrap());
    let state = Arc::new(mediagit_server::AppState::new(repos_dir));
    let app = mediagit_server::create_router(state);
    let handle = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
    (base_url, handle)
}

const MAIN_FILE: &[u8] = b"main file\n";
const LOOK_FILE: &[u8] = b"look-dev only file\n";
const LOOK_MAIN_EDIT: &[u8] = b"main file, edited on look-dev\n";

/// A server holding `main` and `look-dev` (which differs from main), plus a
/// fresh clone of it. Returns (server, clone parent, clone dir, remote url, server repos dir).
async fn two_branch_clone() -> (
    tokio::task::JoinHandle<()>,
    TempDir,
    PathBuf,
    TempDir,
    TempDir,
) {
    let repos_root = TempDir::new().unwrap();
    let (base_url, handle) = start_test_server(repos_root.path().to_path_buf()).await;
    fs::create_dir_all(repos_root.path().join("lazy")).unwrap();
    let remote_url = format!("{base_url}/lazy");

    let source = TempDir::new().unwrap();
    run(source.path(), &["init", "-q"]);
    commit_file(source.path(), "main.txt", MAIN_FILE, "main one");
    run(source.path(), &["branch", "create", "look-dev"]);
    run(source.path(), &["branch", "switch", "look-dev"]);
    commit_file(source.path(), "look.txt", LOOK_FILE, "look one");
    commit_file(source.path(), "main.txt", LOOK_MAIN_EDIT, "look two");
    run(source.path(), &["branch", "switch", "main"]);
    run(source.path(), &["remote", "add", "origin", &remote_url]);
    run(source.path(), &["push", "origin", "--all"]);

    let parent = TempDir::new().unwrap();
    let clone_dir = parent.path().join("copy");
    run(
        parent.path(),
        &["clone", &remote_url, clone_dir.to_str().unwrap()],
    );
    (handle, parent, clone_dir, source, repos_root)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn fresh_clone_of_two_branches_is_clean_then_switch_downloads_the_branch() {
    let (_server, _parent, clone, _source, _repos) = two_branch_clone().await;

    // Fresh clone: healthy, with exactly one "not downloaded" note.
    for args in [&["fsck", "--full"][..], &["fsck"][..], &["verify"][..]] {
        let (ok, text) = run_text(&clone, args);
        assert!(ok, "{args:?} must pass on a fresh clone: {text}");
    }
    let (ok, text) = run_text(&clone, &["fsck", "--full"]);
    assert!(ok, "{text}");
    assert_eq!(
        text.matches("not downloaded yet").count(),
        1,
        "expected exactly one note: {text}"
    );
    assert!(
        text.contains("look-dev"),
        "note must name the branch: {text}"
    );

    // gc walks every ref as a root; a not-downloaded tip must not stop it.
    let (ok, text) = run_text(&clone, &["gc", "--dry-run"]);
    assert!(ok, "gc --dry-run on a fresh clone: {text}");

    // repair must leave the lazy ref alone.
    run(&clone, &["fsck", "--full", "--repair"]);
    assert!(
        clone
            .join(".mediagit/refs/remotes/origin/look-dev")
            .exists(),
        "repair removed the tracking ref"
    );

    // Switch fetches the branch on demand.
    run(&clone, &["branch", "switch", "look-dev"]);
    assert_eq!(fs::read(clone.join("look.txt")).unwrap(), LOOK_FILE);
    assert_eq!(fs::read(clone.join("main.txt")).unwrap(), LOOK_MAIN_EDIT);

    // Now fully clean, with no note.
    let (ok, text) = run_text(&clone, &["fsck", "--full"]);
    assert!(ok, "fsck after switch: {text}");
    assert!(
        !text.contains("not downloaded yet"),
        "note must be gone after the branch is downloaded: {text}"
    );
    let (ok, text) = run_text(&clone, &["verify"]);
    assert!(ok, "verify after switch: {text}");
    assert!(
        !clone.join(".mediagit/lazy-refs").exists(),
        "a stale lazy entry could hide a later loss of the commit"
    );

    // And back again.
    run(&clone, &["branch", "switch", "main"]);
    assert_eq!(fs::read(clone.join("main.txt")).unwrap(), MAIN_FILE);
    assert!(!clone.join("look.txt").exists());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn switch_with_server_down_refuses_plainly_and_changes_nothing() {
    let (server, _parent, clone, _source, _repos) = two_branch_clone().await;
    server.abort();
    let _ = server.await;

    let head_before = fs::read_to_string(clone.join(".mediagit/HEAD")).unwrap();
    let (ok, text) = run_text(&clone, &["branch", "switch", "look-dev"]);
    assert!(!ok, "switch must fail offline: {text}");
    assert!(
        text.contains("This branch is not downloaded yet. Connect to the server and try again."),
        "wrong message: {text}"
    );
    assert!(
        !text.to_lowercase().contains("error sending request") && !text.contains("Caused by"),
        "message must be one plain line: {text}"
    );

    assert_eq!(
        fs::read_to_string(clone.join(".mediagit/HEAD")).unwrap(),
        head_before,
        "HEAD moved"
    );
    assert!(
        !clone.join(".mediagit/refs/heads/look-dev").exists(),
        "a local branch was created"
    );
    assert_eq!(fs::read(clone.join("main.txt")).unwrap(), MAIN_FILE);
    assert!(!clone.join("look.txt").exists());
    // Still healthy, note still there.
    let (ok, text) = run_text(&clone, &["fsck", "--full"]);
    assert!(ok, "{text}");
    assert!(text.contains("not downloaded yet"), "{text}");
}

/// The oracle stays strict: a commit that is NOT a listed lazy tip but is
/// missing is still an error.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn lost_commit_of_the_checked_out_branch_is_still_an_error() {
    let (_server, _parent, clone, _source, _repos) = two_branch_clone().await;
    let main_tip = fs::read_to_string(clone.join(".mediagit/refs/heads/main"))
        .unwrap()
        .trim()
        .to_string();
    let victim = find_object_file(&clone.join(".mediagit"), &main_tip)
        .unwrap_or_else(|| panic!("no loose object file for {main_tip}"));
    fs::remove_file(victim).unwrap();

    let (ok, text) = run_text(&clone, &["fsck", "--full"]);
    assert!(!ok, "fsck must fail when the main tip is gone: {text}");
    assert!(text.to_lowercase().contains("missing"), "{text}");
}

/// A listed tip only counts while its value is unchanged: a tracking ref
/// moved to a different missing commit is an error again.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn lazy_entry_does_not_excuse_a_different_missing_commit() {
    let (_server, _parent, clone, _source, _repos) = two_branch_clone().await;
    let r = clone.join(".mediagit/refs/remotes/origin/look-dev");
    let old = fs::read_to_string(&r).unwrap();
    let forged = "ab".repeat(old.trim().len() / 2);
    assert_ne!(old.trim(), forged);
    fs::write(&r, format!("{forged}\n")).unwrap();
    let (ok, text) = run_text(&clone, &["fsck", "--full"]);
    assert!(!ok, "a ref not matching the lazy list must fail: {text}");
}

fn find_object_file(dir: &Path, hex: &str) -> Option<PathBuf> {
    for e in fs::read_dir(dir).ok()?.flatten() {
        let p = e.path();
        if p.is_dir() {
            if let Some(f) = find_object_file(&p, hex) {
                return Some(f);
            }
        } else {
            let joined: String = p
                .to_string_lossy()
                .chars()
                .filter(|c| c.is_ascii_hexdigit())
                .collect();
            if joined.ends_with(hex) {
                return Some(p);
            }
        }
    }
    None
}

/// Whoever downloads the branch ends the exemption: afterwards the note is
/// gone and the commit is checked like any other.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn fetch_and_pull_of_the_branch_clear_the_note() {
    for how in [
        &["fetch", "origin", "look-dev"][..],
        &["pull", "origin", "look-dev"][..],
    ] {
        let (_server, _parent, clone, _source, _repos) = two_branch_clone().await;
        run(&clone, how);
        let (ok, text) = run_text(&clone, &["fsck", "--full"]);
        assert!(ok, "{how:?}: {text}");
        assert!(
            !text.contains("not downloaded yet"),
            "{how:?} left the note: {text}"
        );
        assert!(
            !clone.join(".mediagit/lazy-refs").exists(),
            "{how:?} left the lazy list"
        );
    }
}
