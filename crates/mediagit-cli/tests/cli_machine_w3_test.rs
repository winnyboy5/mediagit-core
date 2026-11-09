// SPDX-License-Identifier: BUSL-1.1
// Copyright (C) 2025-2026 Aswin Krishnamoorthy

//! `--machine` contract tests for the W3 command set: stats, gc, fsck, verify,
//! auth, key, config, sparse-checkout, rebase, cherry-pick, bisect, reflog.
//!
//! Every `run` call also asserts the contract's hard rule: stdout is exactly
//! one JSON line, success or failure.
//!
//! Run: `cargo test -p mediagit-cli --test cli_machine_w3_test`

use assert_cmd::Command;
use serde_json::Value;
use std::fs;
use std::path::Path;
use tempfile::TempDir;

#[allow(deprecated)]
fn mediagit() -> Command {
    let mut c = Command::cargo_bin("mediagit").unwrap();
    c.env("MEDIAGIT_AUTHOR_NAME", "Test User")
        .env("MEDIAGIT_AUTHOR_EMAIL", "test@example.com")
        // Never touch the developer's real keychain or inherit their secrets.
        .env("MEDIAGIT_NO_KEYRING", "1")
        .env_remove("MEDIAGIT_TOKEN")
        .env_remove("MEDIAGIT_API_KEY")
        .env_remove("MEDIAGIT_ENCRYPTION_KEYFILE");
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

struct Run {
    code: i32,
    stdout: String,
    stderr: String,
    json: Value,
}

/// Run with `--machine` (plus `extra` global flags); asserts one stdout line.
fn run_full(dir: &Path, extra: &[&str], args: &[&str], stdin: Option<&str>) -> Run {
    let mut cmd = mediagit();
    cmd.arg("--machine").args(extra).args(args).current_dir(dir);
    if let Some(s) = stdin {
        cmd.write_stdin(s.to_string());
    } else {
        cmd.write_stdin(String::new());
    }
    let out = cmd.output().unwrap();
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert_eq!(
        stdout.lines().count(),
        1,
        "stdout of {args:?} was: {stdout:?}"
    );
    Run {
        code: out.status.code().unwrap(),
        json: serde_json::from_str(&stdout).unwrap(),
        stdout,
        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
    }
}

fn run(dir: &Path, args: &[&str]) -> (i32, Value) {
    let r = run_full(dir, &[], args, None);
    (r.code, r.json)
}

fn ok(dir: &Path, args: &[&str]) -> Value {
    let (code, v) = run(dir, args);
    assert_eq!(code, 0, "{args:?} -> {v}");
    assert_eq!(v["v"], 1);
    assert_eq!(v["ok"], true, "{args:?} -> {v}");
    v["data"].clone()
}

fn err(dir: &Path, args: &[&str]) -> String {
    let (code, v) = run(dir, args);
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

/// A directory that is not a repository.
fn no_repo() -> TempDir {
    TempDir::new().unwrap()
}

// ---------------------------------------------------------------------------
// stats, gc, fsck, verify
// ---------------------------------------------------------------------------

#[test]
fn stats_machine_reports_counts() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_repo(dir);
    commit_file(dir, "a.txt", b"hello stats", "c1");

    let d = ok(dir, &["stats"]);
    assert_eq!(d["commits"], 1);
    assert_eq!(d["files"], 1);
    assert_eq!(d["local_branches"], 1);
    assert!(d["total_bytes"].as_u64().unwrap() > 0, "{d}");
    assert!(d["loose_objects"].as_u64().unwrap() > 0, "{d}");
    assert_eq!(d["authors"][0]["name"], "Test User");
    // -q must not blank the envelope.
    assert_eq!(ok(dir, &["stats", "-q"])["commits"], 1);

    assert_eq!(err(no_repo().path(), &["stats"]), "error");
}

#[test]
fn gc_machine_dry_run_and_real() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_repo(dir);
    commit_file(dir, "a.txt", b"gc me", "c1");

    let d = ok(dir, &["gc", "--dry-run"]);
    assert_eq!(d["dry_run"], true);
    assert!(d["objects_scanned"].as_u64().unwrap() > 0, "{d}");
    assert_eq!(d["errors"], serde_json::json!([]));

    let d = ok(dir, &["gc", "--yes"]);
    assert_eq!(d["dry_run"], false);
    assert_eq!(d["objects_deleted"], 0);

    assert_eq!(err(dir, &["gc", "--aggressive"]), "invalid_args");
    assert_eq!(err(no_repo().path(), &["gc"]), "error");
}

#[test]
fn fsck_and_verify_machine_healthy_and_unsupported_range() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_repo(dir);
    commit_file(dir, "a.txt", b"fsck me", "c1");

    let d = ok(dir, &["fsck"]);
    assert_eq!(d["healthy"], true);
    assert!(d["objects_checked"].as_u64().unwrap() > 0, "{d}");
    assert_eq!(d["issues"], serde_json::json!([]));

    let d = ok(dir, &["verify"]);
    assert_eq!(d["healthy"], true);

    // (`fsck` outside a repository "succeeds" with 0 objects: pre-existing.)
    assert_eq!(err(dir, &["fsck", "--max-objects", "abc"]), "invalid_args");
}

#[test]
fn fsck_machine_reports_corruption_with_exit_1() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_repo(dir);
    commit_file(dir, "a.txt", b"fsck me", "c1");

    // Overwrite every loose object payload with garbage.
    fn clobber(p: &Path) {
        for e in fs::read_dir(p).unwrap().flatten() {
            let path = e.path();
            if path.is_dir() {
                clobber(&path);
            } else if path.file_name().and_then(|n| n.to_str()) != Some("LAYOUT") {
                fs::write(&path, b"corrupt").unwrap();
            }
        }
    }
    clobber(&dir.join(".mediagit/objects"));

    let (code, v) = run(dir, &["fsck"]);
    assert_eq!(code, 1, "{v}");
    assert_eq!(v["ok"], true, "{v}");
    assert_eq!(v["data"]["healthy"], false);
    assert!(!v["data"]["issues"].as_array().unwrap().is_empty());
}

// ---------------------------------------------------------------------------
// config, sparse-checkout, reflog
// ---------------------------------------------------------------------------

#[test]
fn config_machine_get_set_unset_list() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_repo(dir);

    let d = ok(dir, &["config", "get", "author.name"]);
    assert_eq!(d["entries"][0]["key"], "author.name");
    assert!(d["entries"][0]["value"].is_null());

    let d = ok(dir, &["config", "set", "author.name", "Ada"]);
    assert_eq!(d["action"], "set");
    assert_eq!(
        ok(dir, &["config", "get", "author.name"])["entries"][0]["value"],
        "Ada"
    );
    let d = ok(dir, &["config", "list"]);
    let entries = d["entries"].as_array().unwrap();
    assert!(entries.len() >= 4, "{d}");
    assert!(
        entries
            .iter()
            .any(|e| e["key"] == "author.name" && e["value"] == "Ada")
    );
    assert!(ok(dir, &["config", "unset", "author.name"])["entries"][0]["value"].is_null());

    assert_eq!(err(dir, &["config", "get", "no.such.key"]), "error");
    assert_eq!(
        err(dir, &["config", "set", "author.email", "nope"]),
        "error"
    );
}

#[test]
fn sparse_checkout_machine_set_list_disable() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_repo(dir);
    fs::create_dir_all(dir.join("keep")).unwrap();
    fs::create_dir_all(dir.join("drop")).unwrap();
    fs::write(dir.join("keep/a.txt"), "a").unwrap();
    fs::write(dir.join("drop/b.txt"), "b").unwrap();
    human(dir, &["add", "keep/a.txt", "drop/b.txt"]);
    human(dir, &["commit", "-m", "c1"]);

    assert_eq!(ok(dir, &["sparse-checkout", "list"])["enabled"], false);

    let d = ok(dir, &["sparse-checkout", "set", "keep"]);
    assert_eq!(d["action"], "set");
    assert_eq!(d["mode"], "cone");
    assert_eq!(d["patterns"], serde_json::json!(["keep"]));
    assert_eq!(d["files_removed"], 1);
    assert!(!dir.join("drop/b.txt").exists());

    let d = ok(dir, &["sparse-checkout", "list"]);
    assert_eq!(d["enabled"], true);
    assert_eq!(d["patterns"], serde_json::json!(["keep"]));

    let d = ok(dir, &["sparse-checkout", "disable"]);
    assert_eq!(d["enabled"], false);
    assert_eq!(d["files_added"], 1);
    assert!(dir.join("drop/b.txt").exists());

    assert_eq!(err(no_repo().path(), &["sparse-checkout", "list"]), "error");
}

#[test]
fn reflog_machine_lists_entries() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_repo(dir);
    commit_file(dir, "a.txt", b"1", "c1");
    commit_file(dir, "a.txt", b"2", "c2");

    let d = ok(dir, &["reflog"]);
    assert_eq!(d["reference"], "HEAD");
    let entries = d["entries"].as_array().unwrap();
    assert!(entries.len() >= 2, "{d}");
    assert_eq!(entries[0]["index"], 0);
    assert_eq!(entries[0]["new_oid"], head_oid(dir));
    assert!(entries[0]["time"].as_i64().unwrap() > 0);
    assert_eq!(
        ok(dir, &["reflog", "-n", "1"])["entries"]
            .as_array()
            .unwrap()
            .len(),
        1
    );

    assert_eq!(err(no_repo().path(), &["reflog"]), "error");
}

// ---------------------------------------------------------------------------
// rebase, cherry-pick, bisect
// ---------------------------------------------------------------------------

/// main and feature both rewrite `shared.txt`; HEAD is on feature.
fn diverged_repo(dir: &Path) {
    init_repo(dir);
    commit_file(dir, "shared.txt", b"base\n", "base");
    human(dir, &["branch", "create", "feature"]);
    human(dir, &["branch", "switch", "feature"]);
    commit_file(dir, "shared.txt", b"feature side\n", "feature change");
    human(dir, &["branch", "switch", "main"]);
    commit_file(dir, "shared.txt", b"main side\n", "main change");
    human(dir, &["branch", "switch", "feature"]);
}

#[test]
fn rebase_machine_success_conflict_and_abort() {
    // Clean replay.
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_repo(dir);
    commit_file(dir, "base.txt", b"base", "base");
    human(dir, &["branch", "create", "feature"]);
    human(dir, &["branch", "switch", "feature"]);
    commit_file(dir, "f.txt", b"f", "f1");
    human(dir, &["branch", "switch", "main"]);
    commit_file(dir, "m.txt", b"m", "m1");
    human(dir, &["branch", "switch", "feature"]);
    let d = ok(dir, &["rebase", "main"]);
    assert_eq!(d["action"], "start");
    assert_eq!(d["status"], "rebased");
    assert_eq!(d["commits_rebased"], 1);
    assert_eq!(d["head"], head_oid(dir));
    assert!(dir.join("m.txt").exists() && dir.join("f.txt").exists());

    // Conflict, then abort restores the original head.
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    diverged_repo(dir);
    let before = head_oid(dir);
    let (code, v) = run(dir, &["rebase", "main"]);
    assert_eq!(code, 1, "{v}");
    assert_eq!(v["ok"], true);
    assert_eq!(v["data"]["status"], "conflict");
    assert_eq!(v["data"]["conflicts"], serde_json::json!(["shared.txt"]));
    assert!(v["data"]["conflict_commit"].is_string());

    let d = ok(dir, &["rebase", "--abort"]);
    assert_eq!(d["status"], "aborted");
    assert_eq!(d["head"], before);

    assert_eq!(err(dir, &["rebase", "--continue"]), "error");
    assert_eq!(err(dir, &["rebase"]), "error");
}

#[test]
fn cherry_pick_machine_success_conflict_and_abort() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_repo(dir);
    commit_file(dir, "base.txt", b"base", "base");
    human(dir, &["branch", "create", "feature"]);
    human(dir, &["branch", "switch", "feature"]);
    commit_file(dir, "f.txt", b"f", "f1");
    let f1 = head_oid(dir);
    human(dir, &["branch", "switch", "main"]);

    let d = ok(dir, &["cherry-pick", &f1]);
    assert_eq!(d["action"], "start");
    assert_eq!(d["status"], "picked");
    assert_eq!(d["commits"][0], head_oid(dir));
    assert!(dir.join("f.txt").exists());

    // Conflict, then abort.
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    diverged_repo(dir);
    let feature_head = head_oid(dir);
    human(dir, &["branch", "switch", "main"]);
    let before = head_oid(dir);
    let (code, v) = run(dir, &["cherry-pick", &feature_head]);
    assert_eq!(code, 1, "{v}");
    assert_eq!(v["data"]["status"], "conflict");
    assert_eq!(v["data"]["conflicts"], serde_json::json!(["shared.txt"]));

    let d = ok(dir, &["cherry-pick", "--abort"]);
    assert_eq!(d["status"], "aborted");
    assert_eq!(head_oid(dir), before);

    assert_eq!(err(dir, &["cherry-pick", "--continue"]), "error");
    assert_eq!(err(dir, &["cherry-pick", "no-such-commit"]), "error");
}

#[test]
fn bisect_machine_finds_first_bad_commit() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_repo(dir);
    let mut oids = Vec::new();
    for v in 1..=6 {
        commit_file(dir, "v.txt", format!("{v}").as_bytes(), &format!("c{v}"));
        oids.push(head_oid(dir));
    }
    let first_bad = &oids[3]; // versions >= 4 are bad

    // No good commit yet.
    let d = ok(dir, &["bisect", "start"]);
    assert_eq!(d["status"], "waiting_for_good");
    assert!(d["current"].is_null());
    human(dir, &["bisect", "reset"]);

    let d = ok(dir, &["bisect", "start", "HEAD", &oids[0]]);
    assert_eq!(d["action"], "start");
    assert_eq!(d["status"], "testing");
    assert!(
        d["current"].is_string() && d["remaining_steps"].is_number(),
        "{d}"
    );

    let mut d = d;
    for _ in 0..8 {
        if d["status"] == "complete" {
            break;
        }
        let version: u32 = fs::read_to_string(dir.join("v.txt"))
            .unwrap()
            .parse()
            .unwrap();
        d = ok(dir, &["bisect", if version >= 4 { "bad" } else { "good" }]);
    }
    assert_eq!(d["status"], "complete", "{d}");
    assert_eq!(d["first_bad"], *first_bad);

    let d = ok(dir, &["bisect", "log"]);
    assert!(d["log"].as_array().unwrap().len() >= 3);
    assert_eq!(ok(dir, &["bisect", "reset"])["status"], "reset");

    assert_eq!(err(dir, &["bisect", "good"]), "error");
    // (bisect replay is covered by cli_machine_w4_test.)
}

// ---------------------------------------------------------------------------
// key + secrets on stdin
// ---------------------------------------------------------------------------

const PASSPHRASE: &str = "correct-horse-battery-staple-7731";

fn assert_no_leak(r: &Run, secret: &str) {
    assert!(!r.stdout.contains(secret), "secret in stdout: {}", r.stdout);
    assert!(!r.stderr.contains(secret), "secret in stderr: {}", r.stderr);
}

#[test]
fn key_machine_keyfile_init_status_and_recover() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_repo(dir);
    let keyfile = tmp.path().join("master.key");
    fs::write(&keyfile, "ab".repeat(32)).unwrap();
    let kf = keyfile.to_str().unwrap();

    let keyed = |args: &[&str], stdin: Option<&str>, flags: &[&str]| {
        let mut cmd = mediagit();
        cmd.env("MEDIAGIT_ENCRYPTION_KEYFILE", kf)
            .arg("--machine")
            .args(flags)
            .args(args)
            .current_dir(dir)
            .write_stdin(stdin.unwrap_or("").to_string());
        let out = cmd.output().unwrap();
        let stdout = String::from_utf8(out.stdout).unwrap();
        assert_eq!(stdout.lines().count(), 1, "{args:?}: {stdout:?}");
        (
            out.status.code().unwrap(),
            serde_json::from_str::<Value>(&stdout).unwrap(),
        )
    };

    let (_, v) = keyed(&["key", "status"], None, &[]);
    assert_eq!(v["data"]["enabled"], false);

    let (code, v) = keyed(&["key", "init"], None, &[]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["enabled"], true);
    let code_str = v["data"]["recovery_code"].as_str().unwrap().to_string();
    assert!(code_str.len() > 32);
    // A second init is a clean error.
    assert_eq!(keyed(&["key", "init"], None, &[]).1["ok"], false);

    let (_, v) = keyed(&["key", "status"], None, &[]);
    assert_eq!(v["data"]["enabled"], true);
    assert_eq!(v["data"]["recovery_slot"], true);

    // The recovery code is never accepted on argv in machine mode ...
    let (code, v) = keyed(&["key", "recover", &code_str], None, &[]);
    assert_ne!(code, 0);
    assert_eq!(v["error"]["code"], "invalid_args");
    // ... without --secret-stdin it is a prompt ...
    let (_, v) = keyed(&["key", "recover"], None, &[]);
    assert_eq!(v["error"]["code"], "needs_input");
    // ... and with it, one line on stdin.
    let (code, v) = keyed(
        &["key", "recover"],
        Some(&format!("{code_str}\n")),
        &["--secret-stdin"],
    );
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["action"], "recover");
}

#[test]
fn passphrase_is_read_from_stdin_only_and_never_echoed() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_repo(dir);

    // No keyfile, keychain disabled: the master key is a passphrase. Without
    // the flag, the prompt is `needs_input` and nothing is read from stdin.
    let r = run_full(dir, &[], &["key", "init"], Some(&format!("{PASSPHRASE}\n")));
    assert_eq!(r.json["error"]["code"], "needs_input", "{}", r.stdout);
    assert_no_leak(&r, PASSPHRASE);

    // With --secret-stdin the first line is the passphrase.
    let r = run_full(
        dir,
        &["--secret-stdin"],
        &["key", "init"],
        Some(&format!("{PASSPHRASE}\n")),
    );
    assert_eq!(r.code, 0, "{}", r.stdout);
    assert!(
        r.json["data"]["source"]
            .as_str()
            .unwrap()
            .contains("assphrase"),
        "{}",
        r.stdout
    );
    assert_no_leak(&r, PASSPHRASE);

    // A command that reads objects needs the passphrase again.
    fs::write(dir.join("a.txt"), "secret data").unwrap();
    let r = run_full(
        dir,
        &[],
        &["add", "a.txt"],
        Some(&format!("{PASSPHRASE}\n")),
    );
    assert_eq!(r.json["error"]["code"], "needs_input", "{}", r.stdout);
    assert_no_leak(&r, PASSPHRASE);

    let r = run_full(
        dir,
        &["--secret-stdin"],
        &["add", "a.txt"],
        Some(&format!("{PASSPHRASE}\n")),
    );
    assert_eq!(r.code, 0, "{}", r.stdout);
    assert_no_leak(&r, PASSPHRASE);

    // A wrong passphrase fails cleanly and does not echo what was typed.
    let r = run_full(
        dir,
        &["--secret-stdin"],
        &["commit", "-m", "x"],
        Some("not-the-passphrase-0000\n"),
    );
    assert_ne!(r.code, 0);
    assert_eq!(r.json["ok"], false);
    assert_no_leak(&r, "not-the-passphrase-0000");

    // Empty stdin with the flag is invalid_args, not a hang.
    let r = run_full(dir, &["--secret-stdin"], &["add", "a.txt"], Some(""));
    assert_eq!(r.json["error"]["code"], "invalid_args", "{}", r.stdout);

    // The flag is meaningless without --machine.
    mediagit()
        .args(["--secret-stdin", "status"])
        .current_dir(dir)
        .assert()
        .failure();
}

// ---------------------------------------------------------------------------
// auth (in-process mediagit-server)
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

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn auth_machine_status_whoami_logout_with_auth_disabled() {
    let server = TempDir::new().unwrap();
    let base = serve(mediagit_server::AppState::new(server.path().to_path_buf())).await;
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();

    let d = ok(dir, &["auth", "status", "--server", &base]);
    assert_eq!(d["action"], "status");
    assert_eq!(d["auth_enabled"], false);
    assert_eq!(d["logged_in"], false);
    let d = ok(dir, &["auth", "whoami", "--server", &base]);
    assert_eq!(d["auth_enabled"], false);
    assert!(d["identity"].is_null());

    let d = ok(dir, &["auth", "logout", "--server", &base]);
    assert_eq!(d["action"], "logout");
    assert_eq!(d["removed"], 0);

    // Unreachable server: a clean error envelope.
    assert_eq!(
        err(dir, &["auth", "status", "--server", "http://127.0.0.1:1"]),
        "error"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn auth_machine_login_reads_password_from_stdin_only() {
    let server = TempDir::new().unwrap();
    let api_keys = std::sync::Arc::new(mediagit_security::auth::ApiKeyAuth::new());
    let base = serve(mediagit_server::AppState::new_with_auth(
        server.path().to_path_buf(),
        "test-secret-0123456789-0123456789-abcdef",
        api_keys,
    ))
    .await;
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    let password = "Zx9-battery-staple-Qw4!";

    // Register through the human, piped-stdin path.
    mediagit()
        .args(["auth", "register", "--server", &base])
        .write_stdin(format!("alice\nalice@example.com\n{password}\n"))
        .current_dir(dir)
        .assert()
        .success();

    // Auth enforced, no credential: status says so.
    let d = ok(dir, &["auth", "status", "--server", &base]);
    assert_eq!(d["auth_enabled"], true);
    assert_eq!(d["logged_in"], false);

    // Without --secret-stdin the password prompt is needs_input.
    let r = run_full(
        dir,
        &[],
        &["auth", "login", "--server", &base, "--username", "alice"],
        Some(&format!("{password}\n")),
    );
    assert_eq!(r.json["error"]["code"], "needs_input", "{}", r.stdout);
    assert_no_leak(&r, password);

    // A secret on argv is refused outright in machine mode.
    assert_eq!(
        err(dir, &["auth", "login", "--server", &base, "--token", "abc"]),
        "invalid_args"
    );

    // With it, the password is one line on stdin and never echoed.
    let r = run_full(
        dir,
        &["--secret-stdin"],
        &["auth", "login", "--server", &base, "--username", "alice"],
        Some(&format!("{password}\n")),
    );
    assert_eq!(r.code, 0, "{}", r.stdout);
    assert_eq!(r.json["data"]["logged_in"], true);
    assert_eq!(r.json["data"]["identity"]["username"], "alice");
    assert_no_leak(&r, password);

    // Wrong password: clean failure, still no echo.
    let r = run_full(
        dir,
        &["--secret-stdin"],
        &["auth", "login", "--server", &base, "--username", "alice"],
        Some("wrong-Password-1!\n"),
    );
    assert_ne!(r.code, 0);
    assert_eq!(r.json["ok"], false);
    assert_no_leak(&r, "wrong-Password-1!");
}

// ---------------------------------------------------------------------------
// empty results: refuse (cherry-pick, revert) or drop (rebase), never commit
// ---------------------------------------------------------------------------

#[test]
fn cherry_pick_of_an_already_applied_change_is_refused() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_repo(dir);
    commit_file(dir, "base.txt", b"base", "base");
    human(dir, &["branch", "create", "feature"]);
    human(dir, &["branch", "switch", "feature"]);
    commit_file(dir, "f.txt", b"f", "f1");
    let f1 = head_oid(dir);
    human(dir, &["branch", "switch", "main"]);

    assert_eq!(ok(dir, &["cherry-pick", &f1])["status"], "picked");
    let after_first = head_oid(dir);

    assert_eq!(err(dir, &["cherry-pick", &f1]), "empty_commit");
    assert_eq!(head_oid(dir), after_first, "no commit may be created");

    // Human mode: non-zero exit, same message, still no commit.
    mediagit()
        .args(["cherry-pick", &f1])
        .current_dir(dir)
        .assert()
        .failure()
        .stderr(predicates::str::contains("already on this branch"));
    assert_eq!(head_oid(dir), after_first);
}

#[test]
fn revert_of_an_already_reverted_change_is_refused() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_repo(dir);
    commit_file(dir, "a.txt", b"one\n", "first");
    commit_file(dir, "b.txt", b"two\n", "add b");
    let add_b = head_oid(dir);

    assert_eq!(ok(dir, &["revert", &add_b])["status"], "reverted");
    let after_first = head_oid(dir);

    assert_eq!(err(dir, &["revert", &add_b]), "empty_commit");
    assert_eq!(head_oid(dir), after_first, "no commit may be created");
}

#[test]
fn rebase_drops_a_commit_whose_change_is_already_upstream() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_repo(dir);
    commit_file(dir, "base.txt", b"base", "base");
    human(dir, &["branch", "create", "feature"]);
    human(dir, &["branch", "switch", "feature"]);
    commit_file(dir, "same.txt", b"same", "feature adds same.txt");
    human(dir, &["branch", "switch", "main"]);
    commit_file(dir, "same.txt", b"same", "main adds same.txt");
    let main_head = head_oid(dir);
    human(dir, &["branch", "switch", "feature"]);

    let d = ok(dir, &["rebase", "main"]);
    assert_eq!(d["status"], "rebased");
    // The feature commit became empty, so it is dropped (git's behaviour):
    // the branch now sits exactly at upstream.
    assert_eq!(head_oid(dir), main_head);
}

// ---------------------------------------------------------------------------
// passphrase master: one unwrap per process, and it really is the master
// ---------------------------------------------------------------------------

#[test]
fn stdin_passphrase_selects_passphrase_master_even_with_a_keychain() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_repo(dir);
    // Keychain NOT disabled: `--secret-stdin` must still choose the passphrase
    // (and never touch the keychain on the way).
    let out = mediagit()
        .env_remove("MEDIAGIT_NO_KEYRING")
        .args(["--machine", "--secret-stdin", "key", "init"])
        .write_stdin(format!("{PASSPHRASE}\n"))
        .current_dir(dir)
        .output()
        .unwrap();
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert_eq!(stdout.lines().count(), 1, "{stdout:?}");
    let v: Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(out.status.code(), Some(0), "{v}");
    assert!(
        v["data"]["source"].as_str().unwrap().contains("assphrase"),
        "{v}"
    );
    assert!(!stdout.contains(PASSPHRASE));
    assert!(!String::from_utf8_lossy(&out.stderr).contains(PASSPHRASE));
    // The key file records the passphrase master (a salt is stored).
    let key_file = fs::read_to_string(dir.join(".mediagit/encryption-key")).unwrap();
    assert!(
        key_file.contains("passphrase") && key_file.contains("salt"),
        "{key_file}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn passphrase_repo_push_fetch_pull_read_one_line() {
    let server = TempDir::new().unwrap();
    // The server must hold an encryption master, or it refuses a keyed repo.
    let master = mediagit_security::encryption::EncryptionKey::from_bytes(vec![0xa7; 32]).unwrap();
    let base = serve(
        mediagit_server::AppState::new(server.path().to_path_buf())
            .with_encryption_master(Some(master)),
    )
    .await;
    fs::create_dir_all(server.path().join("pp")).unwrap();
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_repo(dir);
    let line = format!("{PASSPHRASE}\n");
    let step = |args: &[&str]| {
        let r = run_full(dir, &["--secret-stdin"], args, Some(&line));
        assert_eq!(r.code, 0, "{args:?} -> {}", r.stdout);
        assert_eq!(r.json["ok"], true, "{args:?} -> {}", r.stdout);
        assert_no_leak(&r, PASSPHRASE);
        r
    };

    step(&["key", "init"]);
    fs::write(dir.join("a.txt"), "payload").unwrap();
    step(&["add", "a.txt"]);
    step(&["commit", "-m", "c1"]);
    human(dir, &["remote", "add", "origin", &format!("{base}/pp")]);

    // Each of these used to unwrap the key twice -- two prompts, so with one
    // stdin line the second read hit EOF.
    let r = step(&["push", "-u", "origin"]);
    assert_eq!(r.json["data"]["remote"], "origin");
    step(&["fetch", "origin"]);
    step(&["pull", "origin"]);
}
