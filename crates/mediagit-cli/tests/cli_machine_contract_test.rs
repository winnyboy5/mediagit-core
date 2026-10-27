// SPDX-License-Identifier: BUSL-1.1
// Copyright (C) 2025-2026 Aswin Krishnamoorthy

//! `--machine` contract tests (spike: `status` and `version`).
//!
//! Run: `cargo test --test cli_machine_contract_test`

use assert_cmd::Command;
use serde_json::Value;
use std::path::Path;
use tempfile::TempDir;

#[allow(deprecated)]
fn mediagit() -> Command {
    Command::cargo_bin("mediagit").unwrap()
}

fn init_repo(dir: &Path) {
    mediagit()
        .arg("init")
        .arg("-q")
        .current_dir(dir)
        .assert()
        .success();
}

/// Run and assert stdout is exactly one JSON line; returns (exit code, envelope).
fn run_machine(dir: &Path, args: &[&str]) -> (i32, Value) {
    let out = mediagit()
        .arg("--machine")
        .args(args)
        .current_dir(dir)
        .output()
        .unwrap();
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert_eq!(stdout.lines().count(), 1, "stdout was: {stdout:?}");
    (
        out.status.code().unwrap(),
        serde_json::from_str(&stdout).unwrap(),
    )
}

#[test]
fn version_machine_envelope() {
    let tmp = TempDir::new().unwrap();
    let (code, v) = run_machine(tmp.path(), &["version"]);
    assert_eq!(code, 0);
    assert_eq!(v["v"], 1);
    assert_eq!(v["ok"], true);
    assert_eq!(v["data"]["version"], env!("CARGO_PKG_VERSION"));
    assert_eq!(v["data"]["contract"], 1);
}

#[test]
fn status_machine_on_fresh_repo() {
    let tmp = TempDir::new().unwrap();
    init_repo(tmp.path());
    let (code, v) = run_machine(tmp.path(), &["status"]);
    assert_eq!(code, 0);
    assert_eq!(v["ok"], true);
    assert_eq!(v["data"]["branch"]["name"], "main");
    assert_eq!(v["data"]["branch"]["has_commits"], false);
}

#[test]
fn status_machine_outside_repo_is_error_envelope() {
    let tmp = TempDir::new().unwrap();
    let (code, v) = run_machine(tmp.path(), &["status"]);
    assert_ne!(code, 0);
    assert_eq!(v["ok"], false);
    assert_eq!(v["error"]["code"], "error");
    assert!(!v["error"]["message"].as_str().unwrap().is_empty());
}

#[test]
fn unsupported_command_is_json_unsupported_exit_2() {
    let tmp = TempDir::new().unwrap();
    init_repo(tmp.path());
    let (code, v) = run_machine(tmp.path(), &["stats"]);
    assert_eq!(code, 2);
    assert_eq!(v["ok"], false);
    assert_eq!(v["error"]["code"], "json_unsupported");
}

#[test]
fn status_without_machine_is_unchanged() {
    let tmp = TempDir::new().unwrap();
    init_repo(tmp.path());
    let out = mediagit()
        .arg("status")
        .current_dir(tmp.path())
        .output()
        .unwrap();
    assert!(out.status.success());
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert!(stdout.contains("Repository Status"), "{stdout}");
    assert!(serde_json::from_str::<Value>(&stdout).is_err());
}
