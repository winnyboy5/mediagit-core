// MediaGit - Git for Media Files
// Copyright (C) 2025 MediaGit Contributors
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU Affero General Public License as published
// by the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE. See the
// GNU Affero General Public License for more details.

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
