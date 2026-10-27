// SPDX-License-Identifier: BUSL-1.1
// Copyright (C) 2025-2026 Aswin Krishnamoorthy

//! `--machine` contract tests for the W2 command set: merge, diff, stash,
//! reset, revert, download.
//!
//! Every `run_machine` call also asserts the contract's hard rule: stdout is
//! exactly one JSON line, success or failure.
//!
//! Run: `cargo test -p mediagit-cli --test cli_machine_w2_test`

use assert_cmd::Command;
use serde_json::Value;
use std::fs;
use std::path::Path;
use tempfile::TempDir;

#[allow(deprecated)]
fn mediagit() -> Command {
    let mut c = Command::cargo_bin("mediagit").unwrap();
    c.env("MEDIAGIT_AUTHOR_NAME", "Test User")
        .env("MEDIAGIT_AUTHOR_EMAIL", "test@example.com");
    c
}

fn human(dir: &Path, args: &[&str]) {
    mediagit().args(args).current_dir(dir).assert().success();
}

fn init_repo(dir: &Path) {
    human(dir, &["init", "-q"]);
}

fn commit_file(dir: &Path, name: &str, content: &[u8], msg: &str) {
    fs::write(dir.join(name), content).unwrap();
    human(dir, &["add", name]);
    human(dir, &["commit", "-m", msg]);
}

/// Run with `--machine`; asserts stdout is exactly one JSON line.
fn run_machine(dir: &Path, args: &[&str]) -> (i32, Value) {
    let out = mediagit()
        .arg("--machine")
        .args(args)
        .current_dir(dir)
        .output()
        .unwrap();
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert_eq!(
        stdout.lines().count(),
        1,
        "stdout of {args:?} was: {stdout:?}"
    );
    (
        out.status.code().unwrap(),
        serde_json::from_str(&stdout).unwrap(),
    )
}

fn ok(dir: &Path, args: &[&str]) -> Value {
    let (code, v) = run_machine(dir, args);
    assert_eq!(code, 0, "{args:?} -> {v}");
    assert_eq!(v["v"], 1);
    assert_eq!(v["ok"], true, "{args:?} -> {v}");
    v["data"].clone()
}

fn err(dir: &Path, args: &[&str]) -> String {
    let (code, v) = run_machine(dir, args);
    assert_ne!(code, 0, "{args:?} -> {v}");
    assert_eq!(v["ok"], false, "{args:?} -> {v}");
    assert!(!v["error"]["message"].as_str().unwrap().is_empty());
    v["error"]["code"].as_str().unwrap().to_string()
}

fn head_oid(dir: &Path) -> String {
    ok(dir, &["log", "-n", "1"])["commits"][0]["oid"]
        .as_str()
        .unwrap()
        .to_string()
}

/// main and feature both change `shared.txt`: merging feature into main conflicts.
fn conflicted_repo(dir: &Path) {
    init_repo(dir);
    commit_file(dir, "shared.txt", b"base\n", "base");
    human(dir, &["branch", "create", "feature"]);
    human(dir, &["branch", "switch", "feature"]);
    commit_file(dir, "shared.txt", b"feature side\n", "feature change");
    human(dir, &["branch", "switch", "main"]);
    commit_file(dir, "shared.txt", b"main side\n", "main change");
}

#[test]
fn merge_machine_fast_forward_and_merge_commit() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_repo(dir);
    commit_file(dir, "base.txt", b"base", "base");
    human(dir, &["branch", "create", "feature"]);
    human(dir, &["branch", "switch", "feature"]);
    commit_file(dir, "f1.txt", b"f1", "f1");
    let feature_head = head_oid(dir);
    human(dir, &["branch", "switch", "main"]);

    let d = ok(dir, &["merge", "feature"]);
    assert_eq!(d["action"], "merge");
    assert_eq!(d["status"], "fast_forward");
    assert_eq!(d["oid"], feature_head);
    assert_eq!(d["conflicts"].as_array().unwrap().len(), 0);

    // Diverge, then a true merge commit.
    human(dir, &["branch", "create", "other"]);
    human(dir, &["branch", "switch", "other"]);
    commit_file(dir, "o1.txt", b"o1", "o1");
    human(dir, &["branch", "switch", "main"]);
    commit_file(dir, "m1.txt", b"m1", "m1");
    let d = ok(dir, &["merge", "other"]);
    assert_eq!(d["status"], "merged");
    assert_eq!(d["oid"], head_oid(dir));
}

#[test]
fn merge_machine_error_unknown_branch() {
    let tmp = TempDir::new().unwrap();
    init_repo(tmp.path());
    commit_file(tmp.path(), "a.txt", b"a", "a");
    assert_eq!(err(tmp.path(), &["merge", "nope"]), "error");
    assert_eq!(err(tmp.path(), &["merge", "--abort"]), "error");
}

#[test]
fn merge_machine_conflict_lists_paths_then_abort() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    conflicted_repo(dir);

    let (code, v) = run_machine(dir, &["merge", "feature"]);
    assert_eq!(code, 1, "{v}");
    assert_eq!(v["ok"], true);
    let d = &v["data"];
    assert_eq!(d["status"], "conflict");
    assert!(d["oid"].is_null());
    let conflicts = d["conflicts"].as_array().unwrap();
    assert_eq!(conflicts.len(), 1);
    assert_eq!(conflicts[0]["path"], "shared.txt");
    assert_eq!(conflicts[0]["state"], "unresolved");
    assert_eq!(conflicts[0]["kind"], "modify_modify");

    let d = ok(dir, &["merge", "--abort"]);
    assert_eq!(d["action"], "abort");
    assert_eq!(d["status"], "aborted");
}

#[test]
fn diff_machine_commits_working_and_cached() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_repo(dir);
    commit_file(dir, "a.txt", b"one", "first");
    let first = head_oid(dir);
    fs::write(dir.join("a.txt"), "two!").unwrap();
    fs::write(dir.join("blob.bin"), [0u8, 1, 2, 3]).unwrap();
    human(dir, &["add", "a.txt", "blob.bin"]);
    human(dir, &["commit", "-m", "second"]);
    let second = head_oid(dir);

    let d = ok(dir, &["diff", &first, &second]);
    assert_eq!(d["mode"], "commits");
    assert_eq!(d["from"], first);
    assert_eq!(d["to"], second);
    let files = d["files"].as_array().unwrap();
    assert_eq!(files.len(), 2);
    assert_eq!(files[0]["path"], "a.txt");
    assert_eq!(files[0]["status"], "modified");
    assert_eq!(files[0]["old_size"], 3);
    assert_eq!(files[0]["new_size"], 4);
    assert_eq!(files[0]["binary"], false);
    assert_eq!(files[1]["path"], "blob.bin");
    assert_eq!(files[1]["status"], "added");
    assert_eq!(files[1]["binary"], true);

    // Working tree vs HEAD: clean, then one edit and one new file.
    assert_eq!(ok(dir, &["diff"])["files"].as_array().unwrap().len(), 0);
    fs::write(dir.join("a.txt"), "three").unwrap();
    fs::write(dir.join("new.txt"), "n").unwrap();
    let d = ok(dir, &["diff"]);
    assert_eq!(d["mode"], "working");
    assert_eq!(d["from"], second);
    assert!(d["to"].is_null());
    let files = d["files"].as_array().unwrap();
    assert_eq!(files.len(), 2);
    assert_eq!(files[0]["path"], "a.txt");
    assert_eq!(files[0]["new_size"], 5);
    assert_eq!(files[1]["path"], "new.txt");
    assert_eq!(files[1]["status"], "added");

    // Staged changes; -q must not blank the envelope.
    human(dir, &["add", "new.txt"]);
    let d = ok(dir, &["diff", "--cached", "-q"]);
    assert_eq!(d["mode"], "cached");
    assert_eq!(d["files"][0]["path"], "new.txt");
}

#[test]
fn diff_machine_error_unknown_revision() {
    let tmp = TempDir::new().unwrap();
    init_repo(tmp.path());
    commit_file(tmp.path(), "a.txt", b"a", "a");
    assert_eq!(err(tmp.path(), &["diff", "nope", "HEAD"]), "error");
}

#[test]
fn stash_machine_save_list_apply_pop_drop() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_repo(dir);
    commit_file(dir, "a.txt", b"one", "first");

    // Nothing to stash: still an envelope, oid is null.
    assert!(ok(dir, &["stash", "save"])["oid"].is_null());

    fs::write(dir.join("a.txt"), "edited").unwrap();
    let d = ok(dir, &["stash", "save", "-m", "wip a"]);
    assert_eq!(d["action"], "save");
    assert_eq!(d["message"], "wip a");
    assert_eq!(d["oid"].as_str().unwrap().len(), 64);
    assert_eq!(fs::read_to_string(dir.join("a.txt")).unwrap(), "one");

    let d = ok(dir, &["stash", "list"]);
    assert_eq!(d["action"], "list");
    let stashes = d["stashes"].as_array().unwrap();
    assert_eq!(stashes.len(), 1);
    assert_eq!(stashes[0]["index"], 0);
    assert_eq!(stashes[0]["message"], "wip a");

    let d = ok(dir, &["stash", "apply"]);
    assert_eq!(d["action"], "apply");
    assert_eq!(d["index"], 0);
    assert_eq!(fs::read_to_string(dir.join("a.txt")).unwrap(), "edited");

    // Back to clean, then pop (apply + drop) reports once.
    human(dir, &["reset", "--hard"]);
    let d = ok(dir, &["stash", "pop"]);
    assert_eq!(d["action"], "pop");
    assert_eq!(
        ok(dir, &["stash", "list"])["stashes"]
            .as_array()
            .unwrap()
            .len(),
        0
    );

    fs::write(dir.join("a.txt"), "again").unwrap();
    ok(dir, &["stash", "save", "-m", "second"]);
    let d = ok(dir, &["stash", "drop"]);
    assert_eq!(d["action"], "drop");
    assert_eq!(d["message"], "second");
}

#[test]
fn stash_machine_errors() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_repo(dir);
    commit_file(dir, "a.txt", b"a", "a");
    assert_eq!(err(dir, &["stash", "drop", "5"]), "error");
    // Confirmation prompt: the GUI must pass --force itself.
    assert_eq!(err(dir, &["stash", "clear"]), "needs_input");
    assert_eq!(ok(dir, &["stash", "clear", "--force"])["action"], "clear");
}

#[test]
fn reset_machine_modes_and_paths() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_repo(dir);
    commit_file(dir, "a.txt", b"one", "first");
    let first = head_oid(dir);
    commit_file(dir, "b.txt", b"two", "second");

    let d = ok(dir, &["reset", "--soft", &first]);
    assert_eq!(d["mode"], "soft");
    assert_eq!(d["oid"], first);
    let d = ok(dir, &["reset", "--hard", &first]);
    assert_eq!(d["mode"], "hard");
    let d = ok(dir, &["reset"]);
    assert_eq!(d["mode"], "mixed");

    fs::write(dir.join("c.txt"), "c").unwrap();
    human(dir, &["add", "c.txt"]);
    let d = ok(dir, &["reset", "c.txt"]);
    assert_eq!(d["mode"], "paths");
    assert_eq!(d["unstaged"], 1);
    assert!(d["oid"].is_null());

    assert_eq!(err(dir, &["reset", "--hard", "no-such-rev"]), "error");
}

#[test]
fn revert_machine_commit_and_conflict() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_repo(dir);
    commit_file(dir, "a.txt", b"one\n", "first");
    commit_file(dir, "b.txt", b"two\n", "add b");
    let add_b = head_oid(dir);

    let d = ok(dir, &["revert", &add_b]);
    assert_eq!(d["action"], "revert");
    assert_eq!(d["status"], "reverted");
    assert_eq!(d["commits"][0], head_oid(dir));
    assert!(!dir.join("b.txt").exists());

    // Conflict: revert a change that a later commit built on.
    commit_file(dir, "c.txt", b"v1\n", "c v1");
    let c1 = head_oid(dir);
    commit_file(dir, "c.txt", b"v2\n", "c v2");
    let (code, v) = run_machine(dir, &["revert", &c1]);
    assert_eq!(code, 1, "{v}");
    assert_eq!(v["ok"], true);
    assert_eq!(v["data"]["status"], "conflict");
    assert_eq!(v["data"]["conflicts"][0], "c.txt");

    let d = ok(dir, &["revert", "--abort"]);
    assert_eq!(d["status"], "aborted");

    assert_eq!(err(dir, &["revert"]), "error");
}

#[test]
fn download_machine_local_file_and_errors() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_repo(dir);
    commit_file(dir, "a.txt", b"hello", "first");
    let d = ok(dir, &["download", "a.txt", "-o", "out.txt"]);
    assert_eq!(d["path"], "a.txt");
    assert_eq!(d["bytes"], 5);
    assert_eq!(d["source"], "local");
    assert_eq!(fs::read_to_string(dir.join("out.txt")).unwrap(), "hello");

    assert_eq!(err(dir, &["download", "../etc/passwd"]), "error");
}

#[test]
fn tag_machine_create_and_delete() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_repo(dir);
    commit_file(dir, "a.txt", b"x", "c1");
    let head = head_oid(dir);

    let d = ok(dir, &["tag", "create", "v1"]);
    assert_eq!(d["action"], "create");
    assert_eq!(d["created"]["name"], "v1");
    assert_eq!(d["created"]["oid"], head);
    assert_eq!(d["created"]["annotated"], false);

    let d = ok(dir, &["tag", "create", "v2", "-m", "release notes"]);
    assert_eq!(d["created"]["annotated"], true);
    assert_eq!(d["created"]["message"], "release notes");

    // Duplicate without --force, and an invalid name, are clean errors.
    assert_eq!(err(dir, &["tag", "create", "v1"]), "error");
    assert_eq!(err(dir, &["tag", "create", "bad name"]), "error");

    let d = ok(dir, &["tag", "delete", "v1", "nope"]);
    assert_eq!(d["action"], "delete");
    assert_eq!(d["deleted"], serde_json::json!(["v1"]));
    assert_eq!(d["missing"], serde_json::json!(["nope"]));
    assert_eq!(
        ok(dir, &["tag", "list"])["tags"].as_array().unwrap().len(),
        1
    );
}

// ---------------------------------------------------------------------------
// remote ls-tree (in-process mediagit-server)
// ---------------------------------------------------------------------------

async fn serve(state: mediagit_server::AppState) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let app = mediagit_server::create_router(std::sync::Arc::new(state));
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    base
}

/// Source repo with `a.txt` and `sub/b.txt`, pushed to a server at `repo`.
async fn pushed_source(server: &Path, base: &str, repo: &str) -> TempDir {
    fs::create_dir_all(server.join(repo)).unwrap();
    let src = TempDir::new().unwrap();
    init_repo(src.path());
    fs::create_dir_all(src.path().join("sub")).unwrap();
    fs::write(src.path().join("sub/b.txt"), "bbbb").unwrap();
    commit_file(src.path(), "a.txt", b"hello", "c1");
    human(src.path(), &["add", "sub/b.txt"]);
    human(src.path(), &["commit", "-m", "c2"]);
    human(
        src.path(),
        &["remote", "add", "origin", &format!("{base}/{repo}")],
    );
    human(src.path(), &["push", "-u", "origin"]);
    src
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn remote_ls_tree_machine_root_subfolder_and_errors() {
    let server_dir = TempDir::new().unwrap();
    let base = serve(mediagit_server::AppState::new(
        server_dir.path().to_path_buf(),
    ))
    .await;
    let src = pushed_source(server_dir.path(), &base, "w2-tree").await;
    let dir = src.path();

    let d = ok(dir, &["remote", "ls-tree", "origin"]);
    assert_eq!(d["remote"], "origin");
    assert_eq!(d["ref"], "refs/heads/main");
    assert_eq!(d["path"], "");
    let entries = d["entries"].as_array().unwrap();
    assert_eq!(entries.len(), 2);
    let a = entries.iter().find(|e| e["name"] == "a.txt").unwrap();
    assert_eq!(a["kind"], "file");
    assert_eq!(a["path"], "a.txt");
    assert_eq!(a["size"], 5);
    assert_eq!(a["oid"].as_str().unwrap().len(), 64);
    let sub = entries.iter().find(|e| e["name"] == "sub").unwrap();
    assert_eq!(sub["kind"], "dir");
    assert!(sub["size"].is_null());

    let d = ok(
        dir,
        &["remote", "ls-tree", "origin", "sub", "--ref", "main"],
    );
    assert_eq!(d["path"], "sub");
    assert_eq!(d["entries"][0]["path"], "sub/b.txt");
    assert_eq!(d["entries"][0]["size"], 4);

    // Human output: <type>\t<size>\t<path>.
    let out = mediagit()
        .args(["remote", "ls-tree", "origin", "sub"])
        .current_dir(dir)
        .output()
        .unwrap();
    assert_eq!(
        String::from_utf8(out.stdout).unwrap().trim(),
        "blob\t4\tsub/b.txt"
    );

    assert_eq!(
        err(dir, &["remote", "ls-tree", "origin", "no/such/dir"]),
        "error"
    );
    assert_eq!(err(dir, &["remote", "ls-tree", "origin", "../x"]), "error");
    assert_eq!(err(dir, &["remote", "ls-tree", "nope"]), "error");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn remote_ls_tree_machine_auth_required_is_a_clean_error() {
    let server_dir = TempDir::new().unwrap();
    let api_keys = std::sync::Arc::new(mediagit_security::auth::ApiKeyAuth::new());
    let base = serve(mediagit_server::AppState::new_with_auth(
        server_dir.path().to_path_buf(),
        "test-secret-0123456789-0123456789-abcdef",
        api_keys,
    ))
    .await;
    let tmp = TempDir::new().unwrap();
    init_repo(tmp.path());
    commit_file(tmp.path(), "a.txt", b"a", "c1");
    human(
        tmp.path(),
        &["remote", "add", "origin", &format!("{base}/locked")],
    );

    let out = mediagit()
        .env_remove("MEDIAGIT_TOKEN")
        .env_remove("MEDIAGIT_API_KEY")
        .args(["--machine", "remote", "ls-tree", "origin", "--ref", "main"])
        .current_dir(tmp.path())
        .output()
        .unwrap();
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert_eq!(stdout.lines().count(), 1, "{stdout:?}");
    let v: Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(v["ok"], false);
    assert!(
        v["error"]["message"].as_str().unwrap().contains("401"),
        "{v}"
    );
}
