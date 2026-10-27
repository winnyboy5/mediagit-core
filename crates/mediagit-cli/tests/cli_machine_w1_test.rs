// SPDX-License-Identifier: BUSL-1.1
// Copyright (C) 2025-2026 Aswin Krishnamoorthy

//! `--machine` contract tests for the W1 command set: init, add, commit, log,
//! show, branch, tag, remote, media info, lock, fetch, pull, push, clone.
//!
//! Every `run_machine` call also asserts the contract's hard rule: stdout is
//! exactly one JSON line, success or failure.
//!
//! Run: `cargo test -p mediagit-cli --test cli_machine_w1_test`

use assert_cmd::Command;
use serde_json::Value;
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

/// Human-mode command that must succeed (test setup).
fn human(dir: &Path, args: &[&str]) {
    mediagit().args(args).current_dir(dir).assert().success();
}

fn init_repo(dir: &Path) {
    human(dir, &["init", "-q"]);
}

fn commit_file(dir: &Path, name: &str, content: &[u8], msg: &str) {
    let path = dir.join(name);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(path, content).unwrap();
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

/// Success envelope; returns `data`.
fn ok(dir: &Path, args: &[&str]) -> Value {
    let (code, v) = run_machine(dir, args);
    assert_eq!(code, 0, "{args:?} -> {v}");
    assert_eq!(v["v"], 1);
    assert_eq!(v["ok"], true, "{args:?} -> {v}");
    v["data"].clone()
}

/// Failure envelope; returns the error code.
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

fn paths_of(v: &Value) -> Vec<String> {
    v.as_array()
        .unwrap()
        .iter()
        .map(|e| e["path"].as_str().unwrap().to_string())
        .collect()
}

#[test]
fn init_machine_reports_path_and_branch() {
    let tmp = TempDir::new().unwrap();
    let d = ok(tmp.path(), &["init", "proj"]);
    assert!(d["path"].as_str().unwrap().contains("proj"));
    assert_eq!(d["branch"], "main");
    assert!(tmp.path().join("proj/.mediagit").is_dir());
}

#[test]
fn add_and_commit_machine() {
    let tmp = TempDir::new().unwrap();
    init_repo(tmp.path());
    fs::write(tmp.path().join("a.txt"), "hello").unwrap();

    let d = ok(tmp.path(), &["add", "a.txt"]);
    assert_eq!(d["added"], 1);
    assert_eq!(d["modified"], 0);
    assert_eq!(d["deleted"], 0);
    assert_eq!(d["bytes"], 5);

    let d = ok(tmp.path(), &["commit", "-m", "first commit"]);
    assert_eq!(d["oid"].as_str().unwrap().len(), 64);
    assert_eq!(d["branch"], "main");
    assert_eq!(d["summary"], "first commit");
    assert_eq!(d["oid"], head_oid(tmp.path()));

    // Nothing staged: a clean failure envelope, not a prompt.
    err(tmp.path(), &["commit", "-m", "again"]);

    // No -m: would open an editor, so the GUI must supply the message.
    fs::write(tmp.path().join("b.txt"), "b").unwrap();
    ok(tmp.path(), &["add", "b.txt"]);
    assert_eq!(err(tmp.path(), &["commit"]), "needs_input");
}

#[test]
fn log_machine_is_topological_across_a_merge() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_repo(dir);
    commit_file(dir, "base.txt", b"base", "base");
    human(dir, &["branch", "create", "feature"]);
    human(dir, &["branch", "switch", "feature"]);
    commit_file(dir, "f1.txt", b"f1", "f1");
    human(dir, &["branch", "switch", "main"]);
    commit_file(dir, "m1.txt", b"m1", "m1");

    // Before the merge: HEAD history vs --all.
    assert_eq!(ok(dir, &["log"])["commits"].as_array().unwrap().len(), 2);
    assert_eq!(
        ok(dir, &["log", "--all"])["commits"]
            .as_array()
            .unwrap()
            .len(),
        3
    );

    human(dir, &["merge", "feature", "-m", "merge feature"]);

    let d = ok(dir, &["log"]);
    let commits = d["commits"].as_array().unwrap();
    assert_eq!(commits.len(), 4, "{d}");
    let index: std::collections::HashMap<&str, usize> = commits
        .iter()
        .enumerate()
        .map(|(i, c)| (c["oid"].as_str().unwrap(), i))
        .collect();
    for (i, c) in commits.iter().enumerate() {
        for p in c["parents"].as_array().unwrap() {
            let pi = index[p.as_str().unwrap()];
            assert!(pi > i, "parent listed before its child: {d}");
        }
    }
    assert_eq!(commits[0]["parents"].as_array().unwrap().len(), 2);
    assert_eq!(commits[0]["author"]["name"], "Test User");
    assert_eq!(commits[0]["author"]["email"], "test@example.com");
    assert!(commits[0]["time"].as_i64().unwrap() > 0);

    let refs = d["refs"].as_array().unwrap();
    for name in ["main", "feature"] {
        assert!(
            refs.iter()
                .any(|r| r["kind"] == "branch" && r["name"].as_str().unwrap().ends_with(name)),
            "missing branch {name}: {d}"
        );
    }

    // Window and path filters.
    assert_eq!(
        ok(dir, &["log", "-n", "2"])["commits"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    assert_eq!(
        ok(dir, &["log", "--skip", "3"])["commits"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    // Path filter: the commit that added f1.txt, plus the merge that brought
    // it into main (changed against its first parent). base/m1 never touch it.
    let by_path = ok(dir, &["log", "HEAD", "--", "f1.txt"]);
    let mut summaries: Vec<&str> = by_path["commits"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["summary"].as_str().unwrap())
        .collect();
    summaries.sort();
    assert_eq!(summaries, ["f1", "merge feature"]);

    assert_eq!(err(dir, &["log", "no-such-rev"]), "error");
}

#[test]
fn show_machine_commit_and_tree_for_nested_folders() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_repo(dir);
    fs::create_dir_all(dir.join("a/b")).unwrap();
    fs::write(dir.join("a/b/c.txt"), "ccc").unwrap();
    fs::write(dir.join("a/d.txt"), "dddd").unwrap();
    fs::write(dir.join("top.txt"), "t").unwrap();
    human(dir, &["add", "."]);
    human(dir, &["commit", "-m", "nested"]);

    let d = ok(dir, &["show", "HEAD"]);
    assert_eq!(d["oid"], head_oid(dir));
    assert_eq!(d["message"].as_str().unwrap().trim(), "nested");
    let files = d["files"].as_array().unwrap();
    assert_eq!(files.len(), 3);
    assert!(files.iter().all(|f| f["status"] == "added"));
    assert!(d.get("tree").is_none(), "tree only with --tree: {d}");

    let d = ok(dir, &["show", "HEAD", "--tree"]);
    let tree = &d["tree"];
    assert_eq!(paths_of(tree), ["a/b/c.txt", "a/d.txt", "top.txt"]);
    let sizes: Vec<u64> = tree
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["size"].as_u64().unwrap())
        .collect();
    assert_eq!(sizes, [3, 4, 1]);
    assert!(tree[0]["oid"].as_str().unwrap().len() == 64);

    // Human mode: one `<size>\t<path>` line per file, sorted by path.
    let out = mediagit()
        .args(["show", "HEAD", "--tree"])
        .current_dir(dir)
        .output()
        .unwrap();
    assert!(out.status.success());
    assert_eq!(
        String::from_utf8(out.stdout).unwrap().replace("\r\n", "\n"),
        "3\ta/b/c.txt\n4\ta/d.txt\n1\ttop.txt\n"
    );

    // Errors.
    assert_eq!(err(dir, &["show", "no-such-rev"]), "error");
    assert_eq!(err(dir, &["show", "HEAD:top.txt"]), "invalid_args");
    assert_eq!(
        err(dir, &["show", "HEAD:top.txt", "--tree"]),
        "invalid_args"
    );
}

#[test]
fn show_machine_output_writes_identical_bytes_for_a_large_blob() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_repo(dir);
    // 1.5 MiB of non-repeating bytes (xorshift), so it is not trivially deduped.
    let mut x: u64 = 0x9E37_79B9_7F4A_7C15;
    let big: Vec<u8> = (0..1_572_864)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            (x & 0xFF) as u8
        })
        .collect();
    commit_file(dir, "big.bin", &big, "big");

    let out = dir.join("out.bin");
    let d = ok(
        dir,
        &["show", "HEAD:big.bin", "--output", out.to_str().unwrap()],
    );
    assert_eq!(d["path"], "big.bin");
    assert_eq!(d["size"], big.len() as u64);
    assert_eq!(
        blake3::hash(&fs::read(&out).unwrap()),
        blake3::hash(&big),
        "streamed blob differs from the committed bytes"
    );
}

#[test]
fn branch_machine_lifecycle_and_dirty_switch_refusal() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_repo(dir);
    commit_file(dir, "a.txt", b"one", "c1");

    let d = ok(dir, &["branch", "create", "feature"]);
    assert_eq!(d["action"], "create");
    assert_eq!(d["name"], "feature");
    assert_eq!(d["oid"], head_oid(dir));

    let d = ok(dir, &["branch", "list"]);
    assert_eq!(d["current"], "main");
    let names: Vec<&str> = d["branches"]
        .as_array()
        .unwrap()
        .iter()
        .map(|b| b["name"].as_str().unwrap())
        .collect();
    assert!(names.contains(&"main") && names.contains(&"feature"), "{d}");

    // Make `feature` differ from main, come back, then dirty a.txt.
    human(dir, &["branch", "switch", "feature"]);
    commit_file(dir, "a.txt", b"two", "c2");
    human(dir, &["branch", "switch", "main"]);
    fs::write(dir.join("a.txt"), "dirty edit").unwrap();
    assert_eq!(err(dir, &["branch", "switch", "feature"]), "error");
    assert_eq!(ok(dir, &["branch", "list"])["current"], "main");
    fs::write(dir.join("a.txt"), "one").unwrap();

    let d = ok(dir, &["branch", "switch", "feature"]);
    assert_eq!(d["action"], "switch");
    assert_eq!(d["name"], "feature");
    assert_eq!(ok(dir, &["branch", "list"])["current"], "feature");

    let d = ok(dir, &["branch", "rename", "feature", "feat2"]);
    assert_eq!(d["action"], "rename");
    assert_eq!(d["name"], "feat2");
    assert_eq!(d["previous_name"], "feature");

    human(dir, &["branch", "switch", "main"]);
    let d = ok(dir, &["branch", "delete", "-D", "feat2"]);
    assert_eq!(d["deleted"], serde_json::json!(["feat2"]));

    err(dir, &["branch", "switch", "no-such-branch"]);
}

#[test]
fn tag_list_machine() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_repo(dir);
    commit_file(dir, "a.txt", b"x", "c1");
    human(dir, &["tag", "create", "v1"]);
    human(dir, &["tag", "create", "v2", "-m", "release notes"]);

    let d = ok(dir, &["tag", "list"]);
    let tags = d["tags"].as_array().unwrap();
    assert_eq!(tags.len(), 2, "{d}");
    let v1 = tags.iter().find(|t| t["name"] == "v1").unwrap();
    let v2 = tags.iter().find(|t| t["name"] == "v2").unwrap();
    assert_eq!(v1["annotated"], false);
    assert_eq!(v2["annotated"], true);
    assert_eq!(v2["message"].as_str().unwrap().trim(), "release notes");
    assert_eq!(v1["oid"].as_str().unwrap().len(), 64);

    assert_eq!(err(dir, &["tag", "show", "v1"]), "json_unsupported");
}

#[test]
fn remote_machine_list_add_remove() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_repo(dir);

    assert_eq!(
        ok(dir, &["remote", "list"])["remotes"],
        serde_json::json!([])
    );
    let d = ok(
        dir,
        &["remote", "add", "origin", "http://example.invalid/r"],
    );
    assert_eq!(d["remotes"][0]["name"], "origin");
    assert_eq!(d["remotes"][0]["url"], "http://example.invalid/r");
    assert_eq!(
        ok(dir, &["remote", "list"])["remotes"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        ok(dir, &["remote", "remove", "origin"])["remotes"],
        serde_json::json!([])
    );
    err(dir, &["remote", "remove", "origin"]);
}

#[test]
fn media_info_machine_wraps_the_parsed_struct() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    // 100 samples of 8-bit mono PCM at 8 kHz.
    let mut wav = Vec::new();
    wav.extend(b"RIFF");
    wav.extend((36u32 + 100).to_le_bytes());
    wav.extend(b"WAVEfmt ");
    wav.extend(16u32.to_le_bytes());
    wav.extend(1u16.to_le_bytes());
    wav.extend(1u16.to_le_bytes());
    wav.extend(8000u32.to_le_bytes());
    wav.extend(8000u32.to_le_bytes());
    wav.extend(1u16.to_le_bytes());
    wav.extend(8u16.to_le_bytes());
    wav.extend(b"data");
    wav.extend(100u32.to_le_bytes());
    wav.extend(std::iter::repeat_n(128u8, 100));
    fs::write(dir.join("t.wav"), &wav).unwrap();

    let d = ok(dir, &["media", "info", "t.wav"]);
    assert!(d.is_object(), "{d}");
    err(dir, &["media", "info", "missing.wav"]);
}

// ---------------------------------------------------------------------------
// Remote commands (in-process mediagit-server)
// ---------------------------------------------------------------------------

async fn start_test_server(repos_dir: std::path::PathBuf) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base_url = format!("http://{}", listener.local_addr().unwrap());
    let state = Arc::new(mediagit_server::AppState::new(repos_dir));
    let app = mediagit_server::create_router(state);
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
    base_url
}

/// Server + a source repo with one commit and `origin` pointing at it.
async fn source_with_remote(repo: &str) -> (TempDir, TempDir, String) {
    let repos_root = TempDir::new().unwrap();
    let base = start_test_server(repos_root.path().to_path_buf()).await;
    fs::create_dir_all(repos_root.path().join(repo)).unwrap();
    let url = format!("{base}/{repo}");

    let source = TempDir::new().unwrap();
    init_repo(source.path());
    commit_file(source.path(), "a.txt", b"first", "c1");
    human(source.path(), &["remote", "add", "origin", &url]);
    (repos_root, source, url)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn push_clone_fetch_pull_machine() {
    let (_server_dir, source, url) = source_with_remote("w1-sync").await;
    let src = source.path();

    // push: first push creates the branch on the remote.
    let c1 = head_oid(src);
    let d = ok(src, &["push", "-u", "origin"]);
    assert_eq!(d["remote"], "origin");
    assert_eq!(d["result"], "pushed");
    let up = &d["ref_updates"][0];
    assert_eq!(up["name"], "refs/heads/main");
    assert_eq!(up["old"], Value::Null);
    assert_eq!(up["new"], c1);
    assert!(d["objects_sent"].as_u64().unwrap() > 0, "{d}");

    // push again: nothing to do.
    let d = ok(src, &["push", "origin"]);
    assert_eq!(d["result"], "up_to_date");
    assert_eq!(d["ref_updates"], serde_json::json!([]));
    assert_eq!(d["skipped_up_to_date"], 1);

    // clone.
    let parent = TempDir::new().unwrap();
    let dest = parent.path().join("copy");
    let d = ok(parent.path(), &["clone", &url, dest.to_str().unwrap()]);
    assert_eq!(d["url"], url.as_str());
    assert_eq!(d["branch"], "main");
    assert_eq!(d["head"], c1);
    assert_eq!(d["files_checked_out"], 1);
    assert_eq!(fs::read_to_string(dest.join("a.txt")).unwrap(), "first");
    // Destination now exists: failure envelope, nothing but one line on stdout.
    err(parent.path(), &["clone", &url, dest.to_str().unwrap()]);

    // New commit upstream, dry-run first, then the real push.
    commit_file(src, "b.txt", b"second", "c2");
    let c2 = head_oid(src);
    let d = ok(src, &["push", "--dry-run", "origin"]);
    assert_eq!(d["result"], "dry_run");
    assert_eq!(d["ref_updates"][0]["old"], c1);
    assert_eq!(d["ref_updates"][0]["new"], c2);
    let d = ok(src, &["push", "origin"]);
    assert_eq!(d["result"], "pushed");
    assert_eq!(d["ref_updates"][0]["old"], c1);
    assert_eq!(d["ref_updates"][0]["new"], c2);

    // fetch moves only the tracking ref.
    let d = ok(&dest, &["fetch", "origin"]);
    assert_eq!(d["remote"], "origin");
    assert_eq!(d["branches_updated"], 1);
    assert_eq!(d["ref_updates"][0]["name"], "refs/remotes/origin/main");
    assert_eq!(d["ref_updates"][0]["old"], c1);
    assert_eq!(d["ref_updates"][0]["new"], c2);
    assert!(!dest.join("b.txt").exists());
    let d = ok(&dest, &["fetch", "origin"]);
    assert_eq!(d["branches_updated"], 0);
    assert_eq!(d["branches_up_to_date"], 1);
    assert_eq!(d["ref_updates"], serde_json::json!([]));

    // pull fast-forwards the working tree.
    let d = ok(&dest, &["pull", "origin"]);
    assert_eq!(d["result"], "fast_forward");
    assert_eq!(d["branch"], "refs/heads/main");
    assert_eq!(d["old_head"], c1);
    assert_eq!(d["new_head"], c2);
    assert_eq!(fs::read_to_string(dest.join("b.txt")).unwrap(), "second");
    let d = ok(&dest, &["pull", "origin"]);
    assert_eq!(d["result"], "up_to_date");
    assert_eq!(d["new_head"], c2);

    // Failures stay envelopes.
    human(src, &["remote", "add", "bad", "http://127.0.0.1:1/nope"]);
    err(src, &["push", "bad"]);
    err(&dest, &["fetch", "no-such-remote"]);
    err(&dest, &["pull", "no-such-remote"]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn pull_merge_rebase_and_push_delete_machine() {
    let (_server_dir, source, url) = source_with_remote("w1-diverge").await;
    let src = source.path();
    ok(src, &["push", "-u", "origin"]);

    let parent = TempDir::new().unwrap();
    let dest = parent.path().join("copy");
    ok(parent.path(), &["clone", &url, dest.to_str().unwrap()]);

    // Diverge: upstream gains s1, the clone gains d1; pull merges.
    commit_file(src, "s1.txt", b"s1", "s1");
    human(src, &["push", "origin"]);
    commit_file(&dest, "d1.txt", b"d1", "d1");
    let d = ok(&dest, &["pull", "origin"]);
    assert_eq!(d["result"], "merged", "{d}");
    assert_eq!(d["new_head"], head_oid(&dest));
    let log = ok(&dest, &["log", "-n", "1"]);
    assert_eq!(log["commits"][0]["parents"].as_array().unwrap().len(), 2);
    assert!(dest.join("s1.txt").exists() && dest.join("d1.txt").exists());

    // Diverge again; pull --rebase keeps history linear.
    commit_file(src, "s2.txt", b"s2", "s2");
    human(src, &["push", "origin"]);
    commit_file(&dest, "d2.txt", b"d2", "d2");
    let d = ok(&dest, &["pull", "--rebase", "origin"]);
    assert_eq!(d["result"], "rebased", "{d}");
    assert_eq!(d["new_head"], head_oid(&dest));
    let log = ok(&dest, &["log", "-n", "1"]);
    assert_eq!(log["commits"][0]["parents"].as_array().unwrap().len(), 1);

    // Delete a remote branch.
    human(src, &["branch", "create", "tmp"]);
    human(src, &["push", "origin", "tmp"]);
    let d = ok(src, &["push", "--delete", "origin", "tmp"]);
    assert_eq!(d["result"], "deleted", "{d}");
    assert_eq!(d["ref_updates"][0]["name"], "refs/heads/tmp");
    assert_eq!(d["ref_updates"][0]["new"], "");
}

#[test]
fn clone_local_path_machine() {
    let source = TempDir::new().unwrap();
    init_repo(source.path());
    commit_file(source.path(), "a.txt", b"local", "c1");
    let head = head_oid(source.path());

    let parent = TempDir::new().unwrap();
    let dest = parent.path().join("local-copy");
    let d = ok(
        parent.path(),
        &[
            "clone",
            source.path().to_str().unwrap(),
            dest.to_str().unwrap(),
        ],
    );
    assert_eq!(d["head"], head);
    assert_eq!(d["branch"], "main");
    assert_eq!(d["files_checked_out"], 1);
    assert_eq!(fs::read_to_string(dest.join("a.txt")).unwrap(), "local");

    err(parent.path(), &["clone", "no-such-dir", "x"]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn lock_machine_create_list_unlock() {
    let (_server_dir, source, _url) = source_with_remote("w1-locks").await;
    let src = source.path();

    let d = ok(src, &["lock", "create", "a.txt", "--owner", "alice"]);
    assert_eq!(d["path"], "a.txt");
    assert_eq!(d["owner"], "alice");
    let id = d["lock_id"].as_str().unwrap().to_string();

    let d = ok(src, &["lock", "list"]);
    let locks = d["locks"].as_array().unwrap();
    assert_eq!(locks.len(), 1, "{d}");
    assert_eq!(locks[0]["lock_id"], id.as_str());

    // A second lock on the same path is refused.
    err(src, &["lock", "create", "a.txt", "--owner", "bob"]);

    let d = ok(src, &["lock", "unlock", "--id", &id, "--force"]);
    assert_eq!(d["lock_id"], id.as_str());
    assert_eq!(ok(src, &["lock", "list"])["locks"], serde_json::json!([]));
}

#[test]
fn human_output_is_unchanged_without_machine() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_repo(dir);
    commit_file(dir, "a.txt", b"x", "c1");
    for args in [
        &["log"][..],
        &["show", "HEAD"],
        &["branch", "list"],
        &["tag", "list"],
        &["remote", "list"],
    ] {
        let out = mediagit().args(args).current_dir(dir).output().unwrap();
        assert!(out.status.success(), "{args:?}");
        let text = String::from_utf8(out.stdout).unwrap();
        assert!(
            serde_json::from_str::<Value>(&text).is_err(),
            "{args:?} printed JSON without --machine: {text}"
        );
    }
}
