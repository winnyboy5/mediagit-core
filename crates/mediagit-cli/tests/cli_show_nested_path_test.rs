// SPDX-License-Identifier: BUSL-1.1
// Copyright (C) 2025-2026 Aswin Krishnamoorthy

//! Regression tests for QA-003: `show <rev>:<path>` for a nested path.
//!
//! Commits build a single-level (flat) tree keyed by the full relative path
//! (see `commit.rs`) — no nested `Directory` entries are ever produced. The
//! old `find_path_in_tree` walker assumed git-style nested subtrees, so any
//! nested path (e.g. `gen/blob.bin`) always failed with "not found".

use assert_cmd::Command;
use predicates::prelude::*;
use std::fs;
use std::path::Path;
use tempfile::TempDir;

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

#[test]
fn show_nested_path_returns_exact_content() {
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());

    let nested_dir = temp_dir.path().join("gen");
    fs::create_dir_all(&nested_dir).unwrap();
    fs::write(nested_dir.join("blob.bin"), b"nested blob content").unwrap();

    mediagit()
        .arg("add")
        .arg("gen/blob.bin")
        .current_dir(temp_dir.path())
        .assert()
        .success();
    mediagit()
        .arg("commit")
        .arg("-m")
        .arg("add nested blob")
        .current_dir(temp_dir.path())
        .assert()
        .success();

    mediagit()
        .arg("show")
        .arg("HEAD:gen/blob.bin")
        .current_dir(temp_dir.path())
        .assert()
        .success()
        .stdout("nested blob content");
}

#[test]
fn show_missing_nested_path_fails_cleanly() {
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());

    let nested_dir = temp_dir.path().join("gen");
    fs::create_dir_all(&nested_dir).unwrap();
    fs::write(nested_dir.join("blob.bin"), b"nested blob content").unwrap();

    mediagit()
        .arg("add")
        .arg("gen/blob.bin")
        .current_dir(temp_dir.path())
        .assert()
        .success();
    mediagit()
        .arg("commit")
        .arg("-m")
        .arg("add nested blob")
        .current_dir(temp_dir.path())
        .assert()
        .success();

    mediagit()
        .arg("show")
        .arg("HEAD:missing/x")
        .current_dir(temp_dir.path())
        .assert()
        .failure()
        .stderr(predicate::str::contains("not found"));
}
