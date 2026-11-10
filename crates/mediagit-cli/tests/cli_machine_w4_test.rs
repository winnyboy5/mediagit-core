// SPDX-License-Identifier: BUSL-1.1
// Copyright (C) 2025-2026 Aswin Krishnamoorthy

//! `--machine` contract tests for the W4 command set, the subcommands that were
//! `json_unsupported` before: auth register|passwd|key|admin, branch
//! protect|show, commit --dry-run, reflog show --all|delete|expire, remote
//! rename|show|set-url, tag show|verify, verify <commit>|--start|--end and
//! bisect replay; plus the cherry-pick sequencer.
//!
//! Every `run` call also asserts the contract's hard rule: stdout is exactly
//! one JSON line, success or failure.
//!
//! Run: `cargo test -p mediagit-cli --test cli_machine_w4_test`

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

/// Run with `--machine` (plus `extra` global flags and `env`); asserts one
/// stdout line.
fn run_env(
    dir: &Path,
    env: &[(&str, &str)],
    extra: &[&str],
    args: &[&str],
    stdin: Option<&str>,
) -> Run {
    let mut cmd = mediagit();
    for (k, v) in env {
        cmd.env(k, v);
    }
    cmd.arg("--machine").args(extra).args(args).current_dir(dir);
    cmd.write_stdin(stdin.unwrap_or("").to_string());
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
    let r = run_env(dir, &[], &[], args, None);
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

fn assert_no_leak(r: &Run, secret: &str) {
    assert!(!r.stdout.contains(secret), "secret in stdout: {}", r.stdout);
    assert!(!r.stderr.contains(secret), "secret in stderr: {}", r.stderr);
}

// ---------------------------------------------------------------------------
// commit --dry-run, branch protect|show
// ---------------------------------------------------------------------------

#[test]
fn commit_dry_run_reports_what_would_be_committed() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_repo(dir);
    commit_file(dir, "base.txt", b"base", "base");
    let before = head_oid(dir);

    // Nothing staged: a real commit would refuse.
    let d = ok(dir, &["commit", "-m", "x", "--dry-run"]);
    assert_eq!(d["would_commit"], false, "{d}");
    assert_eq!(d["staged"], 0);

    fs::write(dir.join("a.txt"), "a").unwrap();
    human(dir, &["add", "a.txt"]);
    let d = ok(dir, &["commit", "-m", "add a\n\nbody", "--dry-run"]);
    assert_eq!(d["would_commit"], true, "{d}");
    assert_eq!(d["staged"], 1);
    assert_eq!(d["staged_paths"][0], "a.txt");
    assert_eq!(d["summary"], "add a");

    assert_eq!(head_oid(dir), before, "dry-run must not commit");
    // Without a message there is nothing to preview: a prompt, not a hang.
    assert_eq!(err(dir, &["commit", "--dry-run"]), "needs_input");
}

#[test]
fn branch_protect_and_show() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_repo(dir);
    commit_file(dir, "a.txt", b"a", "c1");
    let oid = head_oid(dir);

    let d = ok(dir, &["branch", "show"]);
    assert_eq!(d["name"], "main");
    assert_eq!(d["oid"], oid);
    assert_eq!(d["protected"], false);

    let d = ok(dir, &["branch", "protect", "main", "--require-reviews"]);
    assert_eq!(d["protected"], true);
    assert_eq!(d["changed"], true);
    assert_eq!(d["require_reviews"], true);
    assert_eq!(ok(dir, &["branch", "show", "main"])["protected"], true);

    let d = ok(dir, &["branch", "protect", "main", "--unprotect"]);
    assert_eq!(d["protected"], false);
    assert_eq!(d["changed"], true);
    // Nothing left to remove.
    assert_eq!(
        ok(dir, &["branch", "protect", "main", "--unprotect"])["changed"],
        false
    );
    assert_eq!(err(dir, &["branch", "show", "nope"]), "error");
}

// ---------------------------------------------------------------------------
// remote rename|show|set-url
// ---------------------------------------------------------------------------

#[test]
fn remote_rename_set_url_and_show() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_repo(dir);
    // Nothing listens on port 1: `show` must degrade, not fail.
    ok(dir, &["remote", "add", "origin", "http://127.0.0.1:1/r"]);

    let d = ok(dir, &["remote", "show", "origin"]);
    assert_eq!(d["name"], "origin");
    assert_eq!(d["fetch_url"], "http://127.0.0.1:1/r");
    assert_eq!(d["reachable"], false, "{d}");
    assert!(!d["error"].as_str().unwrap().is_empty());

    let d = ok(
        dir,
        &["remote", "set-url", "origin", "http://127.0.0.1:1/s"],
    );
    assert_eq!(d["remotes"][0]["url"], "http://127.0.0.1:1/s");
    let d = ok(
        dir,
        &[
            "remote",
            "set-url",
            "--push",
            "origin",
            "http://127.0.0.1:1/p",
        ],
    );
    assert_eq!(d["remotes"][0]["name"], "origin");
    let d = ok(dir, &["remote", "show", "origin"]);
    assert_eq!(d["fetch_url"], "http://127.0.0.1:1/s");
    assert_eq!(d["push_url"], "http://127.0.0.1:1/p");

    let d = ok(dir, &["remote", "rename", "origin", "upstream"]);
    assert_eq!(d["remotes"].as_array().unwrap().len(), 1);
    assert_eq!(d["remotes"][0]["name"], "upstream");
    assert_eq!(err(dir, &["remote", "show", "origin"]), "error");
    assert_eq!(err(dir, &["remote", "rename", "origin", "x"]), "error");
    assert_eq!(
        err(dir, &["remote", "set-url", "upstream", "ftp://x"]),
        "error"
    );
}

#[test]
fn remote_add_token_is_refused_on_argv() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_repo(dir);
    assert_eq!(
        err(
            dir,
            &[
                "remote",
                "add",
                "o",
                "http://127.0.0.1:1/r",
                "--token",
                "sekrit"
            ]
        ),
        "invalid_args"
    );
    assert_eq!(
        ok(dir, &["remote", "list"])["remotes"],
        serde_json::json!([])
    );

    // The token comes on stdin instead. The tests run with the keychain off
    // (MEDIAGIT_NO_KEYRING), so the add is refused with a clear error: the
    // token is never written to config.toml, echoed, or half-saved.
    let token = "tok-Zq81-not-for-logs";
    let r = run_env(
        dir,
        &[],
        &["--secret-stdin"],
        &[
            "remote",
            "add",
            "o",
            "http://127.0.0.1:1/r",
            "--token-stdin",
        ],
        Some(&format!(
            "{token}
"
        )),
    );
    assert_ne!(r.code, 0, "{}", r.stdout);
    assert_eq!(
        r.json["error"]["code"], "keychain_unavailable",
        "{}",
        r.stdout
    );
    assert_no_leak(&r, token);
    assert_eq!(
        ok(dir, &["remote", "list"])["remotes"],
        serde_json::json!([])
    );
    let cfg = fs::read_to_string(dir.join(".mediagit/config.toml")).unwrap();
    assert!(!cfg.contains(token), "token must not be in config.toml");

    // Without --secret-stdin the token prompt is needs_input, not a hang.
    let r = run_env(
        dir,
        &[],
        &[],
        &[
            "remote",
            "add",
            "p",
            "http://127.0.0.1:1/p",
            "--token-stdin",
        ],
        Some(&format!(
            "{token}
"
        )),
    );
    assert_eq!(r.json["error"]["code"], "needs_input", "{}", r.stdout);
    assert_no_leak(&r, token);
}

// ---------------------------------------------------------------------------
// tag show|verify
// ---------------------------------------------------------------------------

#[test]
fn tag_show_and_verify() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_repo(dir);
    commit_file(dir, "a.txt", b"a", "c1");
    let oid = head_oid(dir);
    human(dir, &["tag", "create", "light"]);
    human(
        dir,
        &["tag", "create", "rel", "-a", "-m", "release notes\n\nmore"],
    );

    let d = ok(dir, &["tag", "show", "light"]);
    assert_eq!(d["annotated"], false);
    assert_eq!(d["oid"], oid);
    assert!(d["message"].is_null());

    let d = ok(dir, &["tag", "show", "rel"]);
    assert_eq!(d["annotated"], true);
    assert_eq!(d["oid"], oid, "an annotated tag reports its target commit");
    assert!(d["message"].as_str().unwrap().starts_with("release notes"));
    assert_eq!(d["signed"], false);
    assert!(!d["date"].as_str().unwrap().is_empty());

    let d = ok(dir, &["tag", "verify", "rel"]);
    assert_eq!(d["annotated"], true);
    assert_eq!(d["signed"], false);
    assert!(d["fingerprint"].is_null());
    assert_eq!(ok(dir, &["tag", "verify", "light"])["annotated"], false);

    assert_eq!(err(dir, &["tag", "show", "nope"]), "error");
    assert_eq!(err(dir, &["tag", "verify", "nope"]), "error");
}

// ---------------------------------------------------------------------------
// reflog show --all|delete|expire
// ---------------------------------------------------------------------------

#[test]
fn reflog_all_delete_and_expire() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_repo(dir);
    commit_file(dir, "a.txt", b"a", "c1");
    commit_file(dir, "b.txt", b"b", "c2");
    commit_file(dir, "c.txt", b"c", "c3");

    let d = ok(dir, &["reflog", "show", "--all"]);
    let refs = d["refs"].as_array().unwrap();
    assert!(!refs.is_empty(), "{d}");
    let head = refs.iter().find(|r| r["reference"] == "HEAD").unwrap();
    assert!(head["entries"].as_array().unwrap().len() >= 3, "{d}");
    let limited = ok(dir, &["reflog", "show", "--all", "-n", "1"]);
    for r in limited["refs"].as_array().unwrap() {
        assert!(r["entries"].as_array().unwrap().len() <= 1, "{limited}");
    }

    // Destructive without --yes: a prompt, and nothing changed.
    assert_eq!(
        err(dir, &["reflog", "expire", "--keep", "1"]),
        "needs_input"
    );
    assert_eq!(err(dir, &["reflog", "delete", "HEAD"]), "needs_input");
    let after = ok(dir, &["reflog", "HEAD"]);
    assert!(after["entries"].as_array().unwrap().len() >= 3, "{after}");

    let d = ok(dir, &["reflog", "expire", "HEAD", "--keep", "1", "--yes"]);
    assert_eq!(d["action"], "expire");
    assert!(d["expired"].as_u64().unwrap() >= 2, "{d}");
    assert_eq!(d["refs"][0], "HEAD");
    assert_eq!(
        ok(dir, &["reflog", "HEAD"])["entries"]
            .as_array()
            .unwrap()
            .len(),
        1
    );

    let d = ok(dir, &["reflog", "delete", "HEAD", "--yes"]);
    assert_eq!(d["action"], "delete");
    assert_eq!(d["changed"], true);
    let d = ok(dir, &["reflog", "delete", "HEAD", "--yes"]);
    assert_eq!(d["changed"], false, "second delete finds nothing: {d}");

    // Expire everything with nothing left to expire.
    let d = ok(dir, &["reflog", "expire", "--yes"]);
    assert_eq!(d["action"], "expire");
}

// ---------------------------------------------------------------------------
// verify <commit> | --start | --end
// ---------------------------------------------------------------------------

#[test]
fn verify_commit_range() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_repo(dir);
    commit_file(dir, "a.txt", b"a", "c1");
    let c1 = head_oid(dir);
    commit_file(dir, "b.txt", b"b", "c2");
    commit_file(dir, "c.txt", b"c", "c3");
    let c3 = head_oid(dir);

    let d = ok(dir, &["verify", "HEAD"]);
    assert_eq!(d["end"], c3);
    assert!(d["start"].is_null());
    assert_eq!(d["commits"], 3);
    assert_eq!(d["verified"], 3);
    assert_eq!(d["errors"], 0);
    assert_eq!(d["problems"], serde_json::json!([]));

    let d = ok(dir, &["verify", "--start", &c1, "--end", "HEAD"]);
    assert_eq!(d["start"], c1);
    // The start commit itself is excluded.
    assert_eq!(d["commits"], 2, "{d}");

    assert_eq!(err(dir, &["verify", "no-such-ref"]), "error");

    // A corrupt commit is reported as data, and still fails the command.
    let obj = walk_files(&dir.join(".mediagit/objects"))
        .into_iter()
        .find(|p| p.file_name().unwrap().to_str().unwrap() == c3)
        .expect("commit object file");
    fs::write(&obj, b"garbage").unwrap();
    let r = run_env(dir, &[], &[], &["verify", "--end", &c3], None);
    assert_eq!(r.code, 1, "{}", r.stdout);
    assert_eq!(r.json["data"]["errors"], 1, "{}", r.stdout);
    assert_eq!(r.json["data"]["problems"].as_array().unwrap().len(), 1);
}

fn walk_files(dir: &Path) -> Vec<std::path::PathBuf> {
    let mut out = Vec::new();
    for e in fs::read_dir(dir).unwrap().flatten() {
        let p = e.path();
        if p.is_dir() {
            out.extend(walk_files(&p));
        } else {
            out.push(p);
        }
    }
    out
}

// ---------------------------------------------------------------------------
// bisect replay
// ---------------------------------------------------------------------------

#[test]
fn bisect_replay_emits_one_envelope_with_the_final_state() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_repo(dir);
    let mut oids = Vec::new();
    for i in 1..=5 {
        commit_file(dir, "f.txt", format!("v{i}").as_bytes(), &format!("c{i}"));
        oids.push(head_oid(dir));
    }
    let first_bad = 2usize; // c3

    // Drive a real session to completion and keep its log.
    let mut d = ok(dir, &["bisect", "start", &oids[4], &oids[0]]);
    let mut guard = 0;
    while d["status"] == "testing" {
        guard += 1;
        assert!(guard < 10, "bisect did not converge: {d}");
        let cur = d["current"].as_str().unwrap().to_string();
        let idx = oids.iter().position(|o| *o == cur).unwrap();
        d = ok(
            dir,
            &["bisect", if idx >= first_bad { "bad" } else { "good" }],
        );
    }
    assert_eq!(d["status"], "complete", "{d}");
    assert_eq!(d["first_bad"], oids[first_bad]);

    let log = ok(dir, &["bisect", "log"])["log"]
        .as_array()
        .unwrap()
        .iter()
        .map(|l| l.as_str().unwrap().to_string())
        .collect::<Vec<_>>()
        .join("\n");
    let logfile = tmp.path().join("bisect.log");
    fs::write(&logfile, log).unwrap();
    ok(dir, &["bisect", "reset"]);

    // `run` asserts a single stdout line: the replay's per-step envelopes must
    // not leak out.
    let d = ok(dir, &["bisect", "replay", logfile.to_str().unwrap()]);
    assert_eq!(d["action"], "replay");
    assert_eq!(d["status"], "complete", "{d}");
    assert_eq!(d["first_bad"], oids[first_bad]);

    assert_eq!(
        err(dir, &["bisect", "replay", "does-not-exist.log"]),
        "error"
    );
}

// ---------------------------------------------------------------------------
// cherry-pick sequencer: an empty pick mid-sequence can be skipped
// ---------------------------------------------------------------------------

#[test]
fn cherry_pick_empty_commit_mid_sequence_can_be_skipped() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_repo(dir);
    commit_file(dir, "base.txt", b"base", "base");
    let mut picks = Vec::new();
    for name in ["x", "y", "z"] {
        human(dir, &["branch", "create", name]);
        human(dir, &["branch", "switch", name]);
        commit_file(dir, &format!("{name}.txt"), name.as_bytes(), name);
        picks.push(head_oid(dir));
        human(dir, &["branch", "switch", "main"]);
    }
    // y is already on main, so picking it again will be empty.
    assert_eq!(ok(dir, &["cherry-pick", &picks[1]])["status"], "picked");

    let before_seq = head_oid(dir);
    assert_eq!(
        err(dir, &["cherry-pick", &picks[0], &picks[1], &picks[2]]),
        "empty_commit"
    );
    // x was picked on top of before_seq; y produced nothing.
    assert_ne!(head_oid(dir), before_seq);
    assert!(
        dir.join(".mediagit/CHERRY_PICK_STATE").exists(),
        "an empty pick must leave sequencer state behind"
    );

    let d = ok(dir, &["cherry-pick", "--skip"]);
    assert_eq!(d["action"], "skip");
    assert_eq!(d["status"], "picked", "{d}");
    assert_eq!(d["commits"].as_array().unwrap().len(), 1, "{d}");
    assert!(
        dir.join("z.txt").exists(),
        "z must be applied after the skip"
    );
    assert!(dir.join("x.txt").exists());
    assert!(dir.join("y.txt").exists());
}

#[test]
fn cherry_pick_single_empty_commit_leaves_no_sequencer_state() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_repo(dir);
    commit_file(dir, "base.txt", b"base", "base");
    human(dir, &["branch", "create", "x"]);
    human(dir, &["branch", "switch", "x"]);
    commit_file(dir, "x.txt", b"x", "x");
    let x = head_oid(dir);
    human(dir, &["branch", "switch", "main"]);
    assert_eq!(ok(dir, &["cherry-pick", &x])["status"], "picked");

    // Alone, and as the last of a sequence: nothing to continue.
    assert_eq!(err(dir, &["cherry-pick", &x]), "empty_commit");
    assert!(!dir.join(".mediagit/CHERRY_PICK_STATE").exists());
    human(dir, &["branch", "create", "w"]);
    human(dir, &["branch", "switch", "w"]);
    commit_file(dir, "w.txt", b"w", "w");
    let w = head_oid(dir);
    human(dir, &["branch", "switch", "main"]);
    assert_eq!(err(dir, &["cherry-pick", &w, &x]), "empty_commit");
    assert!(!dir.join(".mediagit/CHERRY_PICK_STATE").exists());
    assert!(dir.join("w.txt").exists(), "the earlier pick stays applied");
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
async fn auth_machine_register_passwd_keys_and_admin() {
    use mediagit_security::auth::user::Role;
    let server = TempDir::new().unwrap();
    let api_keys = std::sync::Arc::new(mediagit_security::auth::ApiKeyAuth::new());
    let state = mediagit_server::AppState::new_with_auth(
        server.path().to_path_buf(),
        "test-secret-0123456789-0123456789-abcdef",
        api_keys.clone(),
    );
    let svc = state.auth_service.clone().unwrap();
    let admin_pw = "Adm1n-copper-valley-37!";
    svc.credentials_store
        .create_user_with_role(
            "u-admin".into(),
            "root".into(),
            "root@example.com".into(),
            admin_pw,
            Role::Admin,
        )
        .await
        .unwrap();
    let (admin_key, _) = api_keys
        .generate_key("u-admin".into(), "test".into(), vec![])
        .await
        .unwrap();
    let bob_pw = "Bob-battery-staple-Qw4!";
    svc.credentials_store
        .create_user_with_role(
            "u-bob".into(),
            "bob".into(),
            "bob@example.com".into(),
            bob_pw,
            Role::Write,
        )
        .await
        .unwrap();
    let bob_token = svc.jwt_auth.generate_token("u-bob", vec![]).unwrap();
    let base = serve(state).await;

    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    let as_admin = [("MEDIAGIT_API_KEY", admin_key.as_str())];
    let as_bob = [("MEDIAGIT_TOKEN", bob_token.as_str())];
    let flags = ["--secret-stdin"];

    // register: username/email by flag, password on stdin only.
    let carol_pw = "Orchid-lantern-copper-55!";
    let r = run_env(
        dir,
        &[],
        &flags,
        &[
            "auth",
            "register",
            "--server",
            &base,
            "--username",
            "carol",
            "--email",
            "carol@example.com",
        ],
        Some(&format!("{carol_pw}\n")),
    );
    assert_eq!(r.code, 0, "{}", r.stdout);
    assert_eq!(r.json["data"]["action"], "register");
    assert_eq!(r.json["data"]["logged_in"], true);
    assert_eq!(r.json["data"]["identity"]["username"], "carol");
    assert_no_leak(&r, carol_pw);
    // The password is not an argument: the flag does not exist.
    let r = run_env(
        dir,
        &[],
        &flags,
        &["auth", "register", "--server", &base, "--password", "x"],
        Some("x\n"),
    );
    assert_ne!(r.code, 0);
    // Without --secret-stdin the prompt is needs_input.
    let r = run_env(
        dir,
        &[],
        &[],
        &[
            "auth",
            "register",
            "--server",
            &base,
            "--username",
            "dave",
            "--email",
            "dave@example.com",
        ],
        Some("whatever-Pass-1!\n"),
    );
    assert_eq!(r.json["error"]["code"], "needs_input", "{}", r.stdout);

    // passwd: two secrets, in prompt order, never echoed.
    let new_pw = "Fresh-lantern-Zx8-quartz!";
    let r = run_env(
        dir,
        &as_bob,
        &flags,
        &["auth", "passwd", "--server", &base],
        Some(&format!("{bob_pw}\n{new_pw}\n")),
    );
    assert_eq!(r.code, 0, "{}", r.stdout);
    assert_eq!(r.json["data"]["action"], "passwd");
    assert_no_leak(&r, bob_pw);
    assert_no_leak(&r, new_pw);
    // The old password no longer logs in; the new one does.
    let r = run_env(
        dir,
        &[],
        &flags,
        &["auth", "login", "--server", &base, "--username", "bob"],
        Some(&format!("{bob_pw}\n")),
    );
    assert_ne!(r.code, 0);
    let r = run_env(
        dir,
        &[],
        &flags,
        &["auth", "login", "--server", &base, "--username", "bob"],
        Some(&format!("{new_pw}\n")),
    );
    assert_eq!(r.code, 0, "{}", r.stdout);

    // key create / list / revoke
    let r = run_env(
        dir,
        &as_admin,
        &[],
        &["auth", "key", "create", "--server", &base, "--name", "ci"],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.stdout);
    let d = &r.json["data"];
    assert_eq!(d["action"], "key-create");
    let key_id = d["id"].as_str().unwrap().to_string();
    assert!(!d["key"].as_str().unwrap().is_empty());
    let r = run_env(
        dir,
        &as_admin,
        &[],
        &["auth", "key", "list", "--server", &base],
        None,
    );
    let keys = r.json["data"]["keys"].as_array().unwrap();
    assert!(
        keys.iter().any(|k| k["id"] == key_id.as_str()),
        "{}",
        r.stdout
    );
    // Revoke without --yes: a prompt, and the key survives.
    let r = run_env(
        dir,
        &as_admin,
        &[],
        &["auth", "key", "revoke", "--server", &base, &key_id],
        None,
    );
    assert_eq!(r.json["error"]["code"], "needs_input", "{}", r.stdout);
    let r = run_env(
        dir,
        &as_admin,
        &[],
        &["auth", "key", "list", "--server", &base],
        None,
    );
    assert!(
        r.json["data"]["keys"]
            .as_array()
            .unwrap()
            .iter()
            .any(|k| k["id"] == key_id.as_str())
    );
    let r = run_env(
        dir,
        &as_admin,
        &[],
        &["auth", "key", "revoke", "--server", &base, &key_id, "--yes"],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.stdout);
    assert_eq!(r.json["data"]["action"], "key-revoke");

    // admin: list-users, set-role, create-user, reset-password, grant, revoke-grant
    let r = run_env(
        dir,
        &as_admin,
        &[],
        &["auth", "admin", "list-users", "--server", &base],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.stdout);
    let users = r.json["data"]["users"].as_array().unwrap();
    assert!(
        users
            .iter()
            .any(|u| u["username"] == "root" && u["role"] == "admin"),
        "{}",
        r.stdout
    );
    assert!(users.iter().any(|u| u["username"] == "bob"));

    let r = run_env(
        dir,
        &as_admin,
        &[],
        &[
            "auth", "admin", "set-role", "--server", &base, "bob", "read",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.stdout);
    assert_eq!(r.json["data"]["role"], "read");

    let erin_pw = "Quartz-meadow-harbor-71!";
    let r = run_env(
        dir,
        &as_admin,
        &flags,
        &[
            "auth",
            "admin",
            "create-user",
            "--server",
            &base,
            "erin",
            "--role",
            "write",
            "--email",
            "erin@example.com",
        ],
        Some(&format!("{erin_pw}\n")),
    );
    assert_eq!(r.code, 0, "{}", r.stdout);
    assert_eq!(r.json["data"]["name"], "erin");
    assert_eq!(r.json["data"]["role"], "write");
    assert_no_leak(&r, erin_pw);

    let reset_pw = "Reset-harbor-meadow-19!";
    let r = run_env(
        dir,
        &as_admin,
        &flags,
        &["auth", "admin", "reset-password", "--server", &base, "erin"],
        Some(&format!("{reset_pw}\n")),
    );
    assert_eq!(r.code, 0, "{}", r.stdout);
    assert_eq!(r.json["data"]["action"], "reset-password");
    assert_no_leak(&r, reset_pw);
    let r = run_env(
        dir,
        &[],
        &flags,
        &["auth", "login", "--server", &base, "--username", "erin"],
        Some(&format!("{reset_pw}\n")),
    );
    assert_eq!(r.code, 0, "{}", r.stdout);

    // disable-user / enable-user: --yes required to disable; login then fails.
    let dis = [
        "auth",
        "admin",
        "disable-user",
        "--server",
        base.as_str(),
        "erin",
    ];
    let r = run_env(dir, &as_admin, &[], &dis, None);
    assert_eq!(r.json["error"]["code"], "needs_input", "{}", r.stdout);
    let mut dis_yes = dis.to_vec();
    dis_yes.push("--yes");
    let r = run_env(dir, &as_admin, &[], &dis_yes, None);
    assert_eq!(r.code, 0, "{}", r.stdout);
    assert_eq!(r.json["data"]["action"], "disable-user");
    assert_eq!(r.json["data"]["disabled"], true);
    let r = run_env(
        dir,
        &[],
        &flags,
        &["auth", "login", "--server", &base, "--username", "erin"],
        Some(&format!("{reset_pw}\n")),
    );
    assert_ne!(
        r.code, 0,
        "a disabled account must not log in: {}",
        r.stdout
    );
    let r = run_env(
        dir,
        &as_admin,
        &[],
        &["auth", "admin", "enable-user", "--server", &base, "erin"],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.stdout);
    assert_eq!(r.json["data"]["action"], "enable-user");
    assert_eq!(r.json["data"]["disabled"], false);

    // Grants must name a repo that exists.
    let r = run_env(
        dir,
        &as_admin,
        &[],
        &["remote", "repos", "create", "--server", &base, "art"],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.stdout);
    let r = run_env(
        dir,
        &as_admin,
        &[],
        &[
            "auth", "admin", "grant", "--server", &base, "erin", "art", "write",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.stdout);
    assert_eq!(r.json["data"]["level"], "write");
    assert_eq!(r.json["data"]["repo"], "art");

    let r = run_env(
        dir,
        &as_admin,
        &[],
        &["auth", "admin", "list-grants", "--server", &base, "erin"],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.stdout);
    assert_eq!(r.json["data"]["action"], "list-grants");
    assert_eq!(r.json["data"]["grants"][0]["repo"], "art", "{}", r.stdout);
    assert_eq!(r.json["data"]["grants"][0]["level"], "write");
    let r = run_env(
        dir,
        &as_admin,
        &[],
        &["auth", "admin", "repo-access", "--server", &base, "art"],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.stdout);
    assert_eq!(r.json["data"]["action"], "repo-access");
    assert_eq!(
        r.json["data"]["access"][0]["username"], "erin",
        "{}",
        r.stdout
    );
    assert_eq!(r.json["data"]["access"][0]["level"], "write");
    let r = run_env(
        dir,
        &as_admin,
        &[],
        &["auth", "admin", "repo-access", "--server", &base, "nope"],
        None,
    );
    assert_ne!(r.code, 0);
    assert_eq!(r.json["ok"], false);
    // list-users now carries the account state.
    let r = run_env(
        dir,
        &as_admin,
        &[],
        &["auth", "admin", "list-users", "--server", &base],
        None,
    );
    let users = r.json["data"]["users"].as_array().unwrap();
    assert!(
        users.iter().all(|u| u["disabled"] == false),
        "erin was re-enabled: {}",
        r.stdout
    );

    let r = run_env(
        dir,
        &as_admin,
        &[],
        &[
            "auth",
            "admin",
            "revoke-grant",
            "--server",
            &base,
            "erin",
            "art",
        ],
        None,
    );
    assert_eq!(r.json["error"]["code"], "needs_input", "{}", r.stdout);
    let r = run_env(
        dir,
        &as_admin,
        &[],
        &[
            "auth",
            "admin",
            "revoke-grant",
            "--server",
            &base,
            "erin",
            "art",
            "--yes",
        ],
        None,
    );
    assert_eq!(r.code, 0, "{}", r.stdout);
    assert_eq!(r.json["data"]["action"], "revoke-grant");

    // A non-admin is refused with a clean envelope.
    let r = run_env(
        dir,
        &as_bob,
        &[],
        &["auth", "admin", "list-users", "--server", &base],
        None,
    );
    assert_ne!(r.code, 0);
    assert_eq!(r.json["ok"], false);
}

// ---------------------------------------------------------------------------
// remote repos list|create|delete (hosted-repo admin)
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn remote_repos_list_create_delete() {
    use mediagit_security::auth::user::Role;
    let server = TempDir::new().unwrap();
    let api_keys = std::sync::Arc::new(mediagit_security::auth::ApiKeyAuth::new());
    let state = mediagit_server::AppState::new_with_auth(
        server.path().to_path_buf(),
        "test-secret-0123456789-0123456789-abcdef",
        api_keys.clone(),
    );
    let svc = state.auth_service.clone().unwrap();
    svc.credentials_store
        .create_user_with_role(
            "u-admin".into(),
            "root".into(),
            "root@example.com".into(),
            "Adm1n-copper-valley-37!",
            Role::Admin,
        )
        .await
        .unwrap();
    let (key, _) = api_keys
        .generate_key("u-admin".into(), "test".into(), vec![])
        .await
        .unwrap();
    let base = serve(state).await;
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    let env = [("MEDIAGIT_API_KEY", key.as_str())];
    let go = |args: &[&str]| run_env(dir, &env, &[], args, None);

    let r = go(&["remote", "repos", "create", "--server", &base, "art"]);
    assert_eq!(r.code, 0, "{}", r.stdout);
    assert_eq!(r.json["data"]["action"], "create");
    assert_eq!(r.json["data"]["repo"]["name"], "art");

    let r = go(&["remote", "repos", "list", "--server", &base]);
    assert_eq!(r.code, 0, "{}", r.stdout);
    assert!(
        r.json["data"]["repos"]
            .as_array()
            .unwrap()
            .iter()
            .any(|x| x["name"] == "art")
    );

    // Duplicate name is a clean error.
    let r = go(&["remote", "repos", "create", "--server", &base, "art"]);
    assert_ne!(r.code, 0);
    assert_eq!(r.json["ok"], false);

    // Destructive without --yes: a prompt, and the repo survives.
    let r = go(&["remote", "repos", "delete", "--server", &base, "art"]);
    assert_eq!(r.json["error"]["code"], "needs_input", "{}", r.stdout);
    let r = go(&["remote", "repos", "list", "--server", &base]);
    assert!(
        r.json["data"]["repos"]
            .as_array()
            .unwrap()
            .iter()
            .any(|x| x["name"] == "art")
    );

    let r = go(&[
        "remote", "repos", "delete", "--server", &base, "art", "--yes",
    ]);
    assert_eq!(r.code, 0, "{}", r.stdout);
    assert_eq!(r.json["data"]["deleted"], true);
    assert_eq!(r.json["data"]["storage_retained"], false);
    let r = go(&["remote", "repos", "list", "--server", &base]);
    assert!(
        !r.json["data"]["repos"]
            .as_array()
            .unwrap()
            .iter()
            .any(|x| x["name"] == "art")
    );
}

// ---------------------------------------------------------------------------
// stash push/save [PATHS]
// ---------------------------------------------------------------------------

#[test]
fn stash_push_with_paths_stashes_only_those() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_repo(dir);
    fs::create_dir(dir.join("sub")).unwrap();
    commit_file(dir, "a.txt", b"a0", "a");
    commit_file(dir, "b.txt", b"b0", "b");
    commit_file(dir, "sub/c.txt", b"c0", "c");
    fs::write(dir.join("a.txt"), "a1").unwrap();
    fs::write(dir.join("b.txt"), "b1").unwrap();
    fs::write(dir.join("sub/c.txt"), "c1").unwrap();
    fs::write(dir.join("new.txt"), "n").unwrap();

    // Stash a file and a directory; b.txt and new.txt stay untouched.
    let d = ok(dir, &["stash", "push", "-m", "partial", "a.txt", "sub"]);
    assert_eq!(d["message"], "partial");
    assert_eq!(d["paths"], serde_json::json!(["a.txt", "sub/c.txt"]), "{d}");
    assert_eq!(fs::read_to_string(dir.join("a.txt")).unwrap(), "a0");
    assert_eq!(fs::read_to_string(dir.join("sub/c.txt")).unwrap(), "c0");
    assert_eq!(fs::read_to_string(dir.join("b.txt")).unwrap(), "b1");
    assert_eq!(fs::read_to_string(dir.join("new.txt")).unwrap(), "n");

    // Pop brings the stashed paths back.
    ok(dir, &["stash", "pop"]);
    assert_eq!(fs::read_to_string(dir.join("a.txt")).unwrap(), "a1");
    assert_eq!(fs::read_to_string(dir.join("sub/c.txt")).unwrap(), "c1");
    assert_eq!(fs::read_to_string(dir.join("b.txt")).unwrap(), "b1");

    // `save "msg" <path>` takes paths after the message.
    let d = ok(dir, &["stash", "save", "second", "b.txt"]);
    assert_eq!(d["paths"], serde_json::json!(["b.txt"]), "{d}");
    assert_eq!(fs::read_to_string(dir.join("b.txt")).unwrap(), "b0");
    assert_eq!(fs::read_to_string(dir.join("a.txt")).unwrap(), "a1");
}

#[test]
fn stash_push_with_unknown_path_is_a_clear_error() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_repo(dir);
    commit_file(dir, "a.txt", b"a0", "a");
    fs::write(dir.join("a.txt"), "a1").unwrap();

    assert_eq!(err(dir, &["stash", "push", "nope.txt"]), "invalid_args");
    // Nothing was stashed.
    assert_eq!(fs::read_to_string(dir.join("a.txt")).unwrap(), "a1");
    assert_eq!(
        ok(dir, &["stash", "list"])["stashes"],
        serde_json::json!([])
    );
}

#[test]
fn stash_push_without_paths_still_stashes_everything() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_repo(dir);
    commit_file(dir, "a.txt", b"a0", "a");
    commit_file(dir, "b.txt", b"b0", "b");
    fs::write(dir.join("a.txt"), "a1").unwrap();
    fs::write(dir.join("b.txt"), "b1").unwrap();
    let d = ok(dir, &["stash", "push"]);
    assert_eq!(d["paths"], serde_json::json!(["a.txt", "b.txt"]), "{d}");
    assert_eq!(fs::read_to_string(dir.join("a.txt")).unwrap(), "a0");
    assert_eq!(fs::read_to_string(dir.join("b.txt")).unwrap(), "b0");
}

// ---------------------------------------------------------------------------
// config schema / generic set|get|unset
// ---------------------------------------------------------------------------

fn config_text(dir: &Path) -> String {
    fs::read_to_string(dir.join(".mediagit/config.toml")).unwrap()
}

#[test]
fn config_schema_describes_every_key() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_repo(dir);

    let d = ok(dir, &["config", "schema"]);
    let keys = d["keys"].as_array().unwrap();
    let find = |k: &str| {
        keys.iter()
            .find(|e| e["key"] == k)
            .unwrap_or_else(|| panic!("{k}"))
    };
    assert_eq!(find("author.email")["type"], "string");
    assert_eq!(find("performance.upload_concurrency")["type"], "int");
    assert_eq!(find("performance.upload_concurrency")["min"], 1);
    assert_eq!(find("storage.sync")["type"], "bool");
    assert_eq!(find("storage.sync")["default"], "false");
    assert_eq!(find("remotes.<name>.token")["secret"], true);
    assert_eq!(find("author.name")["secret"], false);
    assert!(keys.iter().all(|e| !e["doc"].as_str().unwrap().is_empty()));
    // Needs no repository.
    let outside = TempDir::new().unwrap();
    assert!(
        !ok(outside.path(), &["config", "schema"])["keys"]
            .as_array()
            .unwrap()
            .is_empty()
    );
}

#[test]
fn config_set_checks_the_type_and_leaves_the_file_alone_on_failure() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_repo(dir);
    let before = config_text(dir);

    for args in [
        ["config", "set", "storage.sync", "maybe"],
        ["config", "set", "performance.pack_workers", "0"],
        ["config", "set", "performance.pack_workers", "many"],
        ["config", "set", "storage.file_permissions", "rwx"],
        ["config", "set", "cdc_seed", "1"],
    ] {
        assert_eq!(err(dir, &args), "error", "{args:?}");
        assert_eq!(config_text(dir), before, "{args:?} must not touch the file");
    }

    let d = ok(dir, &["config", "set", "storage.sync", "true"]);
    assert_eq!(d["entries"][0]["value"], "true");
    assert_eq!(
        ok(dir, &["config", "get", "storage.sync"])["entries"][0]["value"],
        "true"
    );
    assert!(ok(dir, &["config", "unset", "storage.sync"])["entries"][0]["value"].is_null());
    assert_eq!(
        ok(dir, &["config", "get", "storage.sync"])["entries"][0]["value"],
        Value::Null
    );
}

#[test]
fn config_set_is_validated_against_the_real_config_struct() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_repo(dir);
    let before = config_text(dir);

    // The type-correct value for a key the current backend cannot have.
    assert_eq!(
        err(dir, &["config", "set", "storage.region", "eu-west-1"]),
        "error"
    );
    assert_eq!(config_text(dir), before);
    // A remote token with no remote: the struct requires a url.
    let r = run_env(
        dir,
        &[],
        &["--secret-stdin"],
        &["config", "set", "remotes.ghost.token"],
        Some("tok\n"),
    );
    assert_ne!(r.code, 0, "{}", r.stdout);
    assert_eq!(config_text(dir), before);
}

#[test]
fn config_set_keeps_comments_unknown_keys_and_a_u64_seed() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_repo(dir);
    // A top-level u64 above i64::MAX (which `toml::Value` cannot hold) and a
    // comment, placed before the first table.
    let rest: String = config_text(dir)
        .lines()
        .filter(|l| !l.starts_with("cdc_seed"))
        .map(|l| format!("{l}\n"))
        .collect();
    let seeded = format!("# hand-written note\ncdc_seed = 18446744073709551615 # big\n{rest}");
    fs::write(dir.join(".mediagit/config.toml"), &seeded).unwrap();

    ok(dir, &["config", "set", "author.name", "Ada"]);
    let after = config_text(dir);
    assert!(after.contains("# hand-written note"), "{after}");
    assert!(after.contains("18446744073709551615 # big"), "{after}");
    assert!(after.contains("name = \"Ada\""), "{after}");
}

#[test]
fn config_secret_keys_come_from_stdin_and_are_never_shown() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_repo(dir);
    ok(dir, &["remote", "add", "origin", "http://127.0.0.1:1/r"]);
    let secret = "tok-Zq81-config-secret";

    // On argv: refused, and the file never sees it.
    let r = run_env(
        dir,
        &[],
        &["--secret-stdin"],
        &["config", "set", "remotes.origin.token", secret],
        Some(&format!("{secret}\n")),
    );
    assert_eq!(r.json["error"]["code"], "invalid_args", "{}", r.stdout);
    assert!(!config_text(dir).contains(secret));
    assert_no_leak(&r, secret);

    // No stdin permission: a prompt, not a hang.
    let r = run_env(
        dir,
        &[],
        &[],
        &["config", "set", "remotes.origin.token"],
        Some(&format!("{secret}\n")),
    );
    assert_eq!(r.json["error"]["code"], "needs_input", "{}", r.stdout);

    // On stdin: stored, reported as "set", never echoed.
    let r = run_env(
        dir,
        &[],
        &["--secret-stdin"],
        &["config", "set", "remotes.origin.token"],
        Some(&format!("{secret}\n")),
    );
    assert_eq!(r.code, 0, "{}", r.stdout);
    assert_eq!(r.json["data"]["entries"][0]["value"], "set");
    assert_eq!(r.json["data"]["entries"][0]["secret"], true);
    assert_no_leak(&r, secret);
    assert!(
        config_text(dir).contains(secret),
        "pinned: tokens are stored in plain text"
    );

    let r = run_env(
        dir,
        &[],
        &[],
        &["config", "get", "remotes.origin.token"],
        None,
    );
    assert_eq!(r.json["data"]["entries"][0]["value"], "set");
    assert_no_leak(&r, secret);
    let r = run_env(dir, &[], &[], &["config", "list"], None);
    assert_no_leak(&r, secret);
    let listed = r.json["data"]["entries"].as_array().unwrap();
    assert!(
        listed
            .iter()
            .any(|e| e["key"] == "remotes.origin.token" && e["value"] == "set")
    );
}

// ---------------------------------------------------------------------------
// diff -- PATHS (prefix filter)
// ---------------------------------------------------------------------------

#[test]
fn diff_paths_filter_every_mode() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_repo(dir);
    fs::create_dir(dir.join("art")).unwrap();
    commit_file(dir, "a.txt", b"a0", "a");
    commit_file(dir, "art/b.txt", b"b0", "b");
    commit_file(dir, "art/c.txt", b"c0", "c");
    let names = |d: &Value| -> Vec<String> {
        d["files"]
            .as_array()
            .unwrap()
            .iter()
            .map(|f| f["path"].as_str().unwrap().to_string())
            .collect()
    };

    // working tree
    fs::write(dir.join("a.txt"), "a1").unwrap();
    fs::write(dir.join("art/b.txt"), "b1").unwrap();
    assert_eq!(names(&ok(dir, &["diff"])), ["a.txt", "art/b.txt"]);
    assert_eq!(names(&ok(dir, &["diff", "--", "a.txt"])), ["a.txt"]);
    assert_eq!(names(&ok(dir, &["diff", "--", "art"])), ["art/b.txt"]);
    assert_eq!(names(&ok(dir, &["diff", "--", "art/"])), ["art/b.txt"]);
    // A prefix of a name is not a path prefix.
    assert!(names(&ok(dir, &["diff", "--", "ar"])).is_empty());
    assert!(names(&ok(dir, &["diff", "--", "nope"])).is_empty());

    // staged
    human(dir, &["add", "a.txt", "art/b.txt"]);
    assert_eq!(
        names(&ok(dir, &["diff", "--cached"])),
        ["a.txt", "art/b.txt"]
    );
    assert_eq!(
        names(&ok(dir, &["diff", "--cached", "--", "art"])),
        ["art/b.txt"]
    );

    // two commits
    human(dir, &["commit", "-m", "both"]);
    assert_eq!(
        names(&ok(dir, &["diff", "HEAD~1", "HEAD"])),
        ["a.txt", "art/b.txt"]
    );
    assert_eq!(
        names(&ok(dir, &["diff", "HEAD~1", "HEAD", "--", "art"])),
        ["art/b.txt"]
    );
}
