// SPDX-License-Identifier: BUSL-1.1
// Copyright (C) 2025-2026 Aswin Krishnamoorthy

//! D4: `--log-format json` must produce logs something can actually parse.
//!
//! `mediagit-observability` has had a working JSON renderer, with tests, since
//! it was written — and until 2026-09-18 **neither shipping binary could select
//! it**. The CLI depended on the crate and hardcoded `LogFormat::Pretty`; the
//! server did not depend on it at all and built a bare `fmt::layer()`.
//!
//! The crate's own tests are the reason that went unnoticed: every one of them
//! asserts the shape of a `LogConfig` builder. Not one emits a log line. A test
//! that `LogFormat::Json` exists cannot fail when nothing can reach it.
//!
//! So these run the real binary and read what comes out of it.

#![allow(clippy::unwrap_used)]

use assert_cmd::Command;
use std::path::Path;
use tempfile::TempDir;

fn mediagit() -> Command {
    let mut c = Command::cargo_bin("mediagit").unwrap();
    c.env("MEDIAGIT_AUTHOR_NAME", "Test User")
        .env("MEDIAGIT_AUTHOR_EMAIL", "test@example.com")
        .env("MEDIAGIT_NO_KEYRING", "1")
        // Logs are filtered to `warn` unless asked otherwise, and a test that
        // captures no lines proves nothing either way.
        .env("MEDIAGIT_LOG", "debug")
        .env_remove("MEDIAGIT_LOG_FORMAT");
    c
}

fn init_repo(dir: &Path) {
    let mut c = Command::cargo_bin("mediagit").unwrap();
    c.env("MEDIAGIT_NO_KEYRING", "1")
        .arg("init")
        .arg("-q")
        .current_dir(dir)
        .assert()
        .success();
}

/// Every captured log line must parse as JSON with the fields a collector
/// needs. Asserting "the output contains a `{`" would pass against a pretty
/// line that happens to print a struct.
#[test]
fn log_format_json_emits_parseable_json_lines() {
    let dir = TempDir::new().unwrap();
    init_repo(dir.path());

    let out = mediagit()
        .arg("--log-format")
        .arg("json")
        .arg("status")
        .current_dir(dir.path())
        .assert()
        .success()
        .get_output()
        .stderr
        .clone();
    let stderr = String::from_utf8(out).unwrap();

    let lines: Vec<&str> = stderr.lines().filter(|l| !l.trim().is_empty()).collect();
    assert!(
        !lines.is_empty(),
        "no log lines were captured, so this test would pass against a binary \
         that logged nothing at all"
    );

    for line in &lines {
        let v: serde_json::Value = serde_json::from_str(line)
            .unwrap_or_else(|e| panic!("log line is not JSON ({e}): {line}"));
        assert!(v.get("timestamp").is_some(), "no timestamp: {line}");
        assert!(v.get("level").is_some(), "no level: {line}");
        assert!(v.get("target").is_some(), "no target: {line}");
        assert!(
            v.pointer("/fields/message").is_some(),
            "no message field: {line}"
        );
    }
}

/// The other half. Without it, a build that emitted JSON unconditionally would
/// pass the test above — and would have silently changed what every existing
/// script and terminal sees.
#[test]
fn the_default_format_is_still_human_readable() {
    let dir = TempDir::new().unwrap();
    init_repo(dir.path());

    let out = mediagit()
        .arg("status")
        .current_dir(dir.path())
        .assert()
        .success()
        .get_output()
        .stderr
        .clone();
    let stderr = String::from_utf8(out).unwrap();

    let lines: Vec<&str> = stderr.lines().filter(|l| !l.trim().is_empty()).collect();
    assert!(!lines.is_empty(), "no log lines were captured");
    assert!(
        lines
            .iter()
            .all(|l| serde_json::from_str::<serde_json::Value>(l).is_err()),
        "the default format must not be JSON:\n{stderr}"
    );
}

/// `MEDIAGIT_LOG_FORMAT` reaches the same switch, so the format can be changed
/// without editing a command line — which is what a CI job or a wrapper script
/// actually has available.
#[test]
fn the_env_var_selects_the_format_too() {
    let dir = TempDir::new().unwrap();
    init_repo(dir.path());

    let out = mediagit()
        .env("MEDIAGIT_LOG_FORMAT", "json")
        .arg("status")
        .current_dir(dir.path())
        .assert()
        .success()
        .get_output()
        .stderr
        .clone();
    let stderr = String::from_utf8(out).unwrap();

    let first = stderr.lines().find(|l| !l.trim().is_empty()).unwrap();
    serde_json::from_str::<serde_json::Value>(first)
        .unwrap_or_else(|e| panic!("MEDIAGIT_LOG_FORMAT=json was ignored ({e}): {first}"));
}

/// An unrecognised format is an ERROR. A silent fallback to pretty would hand a
/// script that asked for JSON a log it cannot parse, and it would find out
/// somewhere else entirely — the exact failure mode this whole cycle is about.
#[test]
fn an_unknown_format_is_refused_rather_than_ignored() {
    let dir = TempDir::new().unwrap();
    init_repo(dir.path());

    mediagit()
        .arg("--log-format")
        .arg("jsn")
        .arg("status")
        .current_dir(dir.path())
        .assert()
        .failure()
        .stderr(predicates::str::contains("jsn"));
}
