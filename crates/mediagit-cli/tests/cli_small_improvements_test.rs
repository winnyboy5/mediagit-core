// SPDX-License-Identifier: BUSL-1.1
// Copyright (C) 2025-2026 Aswin Krishnamoorthy

//! Small CLI improvements from the QA live campaign (6C1):
//! IMP-005 status shows the branch, IMP-006 nested init warns, IMP-008 a refused
//! init prints no banner, IMP-021 an empty .mediagit folder is damaged,
//! IMP-061 bisect replay skips the log header, IMP-004 `--path` hidden.
//!
//! Run: `cargo test -p mediagit-cli --test cli_small_improvements_test`

use assert_cmd::Command;
use std::fs;
use std::path::Path;
use tempfile::TempDir;

#[allow(deprecated)]
fn mediagit() -> Command {
    let mut c = Command::cargo_bin("mediagit").unwrap();
    c.env("MEDIAGIT_AUTHOR_NAME", "Test User")
        .env("MEDIAGIT_AUTHOR_EMAIL", "test@example.com")
        .env("MEDIAGIT_NO_KEYRING", "1")
        .env_remove("MEDIAGIT_REPO");
    c
}

fn run(dir: &Path, args: &[&str]) -> (bool, String, String) {
    let o = mediagit().args(args).current_dir(dir).output().unwrap();
    (
        o.status.success(),
        String::from_utf8_lossy(&o.stdout).into_owned(),
        String::from_utf8_lossy(&o.stderr).into_owned(),
    )
}

fn ok(dir: &Path, args: &[&str]) -> String {
    let (good, out, err) = run(dir, args);
    assert!(good, "{args:?} failed:\n{out}\n{err}");
    out
}

#[test]
fn plain_status_names_the_branch() {
    let tmp = TempDir::new().unwrap();
    ok(tmp.path(), &["init", "-q"]);
    let out = ok(tmp.path(), &["status"]);
    assert!(out.contains("On branch: main"), "got:\n{out}");
    // Short and porcelain stay free of the extra line.
    assert!(!ok(tmp.path(), &["status", "--porcelain"]).contains("On branch"));
    assert!(!ok(tmp.path(), &["status", "--short"]).contains("On branch"));
}

#[test]
fn refused_init_prints_no_banner() {
    let tmp = TempDir::new().unwrap();
    ok(tmp.path(), &["init", "-q"]);
    let (good, out, err) = run(tmp.path(), &["init"]);
    assert!(!good);
    assert!(
        !out.contains("Initializing"),
        "banner before refusal:\n{out}"
    );
    assert!(err.contains("already initialized"), "got:\n{err}");
}

#[test]
fn init_inside_a_repository_warns() {
    let tmp = TempDir::new().unwrap();
    ok(tmp.path(), &["init", "-q"]);
    let sub = tmp.path().join("sub");
    fs::create_dir(&sub).unwrap();
    let out = ok(&sub, &["init"]);
    assert!(out.contains("is inside the repository at"), "got:\n{out}");
    // A top-level init says nothing of the kind.
    let other = TempDir::new().unwrap();
    assert!(!ok(other.path(), &["init"]).contains("is inside the repository"));
}

#[test]
fn empty_mediagit_folder_is_damaged_not_new() {
    let tmp = TempDir::new().unwrap();
    fs::create_dir(tmp.path().join(".mediagit")).unwrap();
    let (good, out, err) = run(tmp.path(), &["status"]);
    assert!(!good, "status accepted an empty .mediagit:\n{out}");
    assert!(err.contains("damaged or incomplete"), "got:\n{err}");
}

#[test]
fn bisect_replay_accepts_its_own_log_output() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    ok(dir, &["init", "-q"]);
    let mut oids = Vec::new();
    for i in 1..=3 {
        fs::write(dir.join("f.txt"), format!("v{i}")).unwrap();
        ok(dir, &["add", "f.txt"]);
        ok(dir, &["commit", "-m", &format!("c{i}")]);
        let v: serde_json::Value =
            serde_json::from_str(ok(dir, &["--machine", "log", "-n", "1"]).trim()).unwrap();
        oids.push(v["data"]["commits"][0]["oid"].as_str().unwrap().to_string());
    }
    ok(dir, &["bisect", "start", &oids[2], &oids[0]]);
    let log = ok(dir, &["bisect", "log"]);
    assert!(log.contains("Bisect Log:"), "got:\n{log}");
    let file = dir.join("bisect.log");
    fs::write(&file, &log).unwrap();
    ok(dir, &["bisect", "reset"]);
    let out = ok(dir, &["bisect", "replay", file.to_str().unwrap()]);
    assert!(!out.contains("Unknown log entry"), "got:\n{out}");
}

#[test]
fn fsck_and_verify_hide_the_path_flag() {
    let tmp = TempDir::new().unwrap();
    for cmd in ["fsck", "verify"] {
        let out = ok(tmp.path(), &[cmd, "--help"]);
        assert!(!out.contains("--path"), "{cmd} help shows --path:\n{out}");
    }
}
