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

//! Integration tests for M5b sparse checkout (#8): `mediagit sparse-checkout
//! set/list/disable`, and the "status must not report sparse-excluded paths
//! as deleted" regression (M4's report named the exact insertion point:
//! the `deleted_files` collection block in `commands/status.rs`).

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

fn commit_file(dir: &Path, rel_path: &str, content: &str, message: &str) {
    let full = dir.join(rel_path);
    if let Some(parent) = full.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(&full, content).unwrap();
    mediagit()
        .arg("add")
        .arg(rel_path)
        .current_dir(dir)
        .assert()
        .success();
    mediagit()
        .arg("commit")
        .arg("-m")
        .arg(message)
        .current_dir(dir)
        .assert()
        .success();
}

/// A repo with two files, one under `included/`, one under `excluded/`,
/// committed on `main`.
fn two_dir_repo() -> TempDir {
    let temp = TempDir::new().unwrap();
    init_repo(temp.path());
    commit_file(temp.path(), "included/keep.txt", "keep me", "add keep");
    commit_file(temp.path(), "excluded/drop.txt", "drop me", "add drop");
    temp
}

#[test]
fn sparse_set_cone_mode_removes_excluded_materializes_included() {
    let temp = two_dir_repo();
    assert!(temp.path().join("included/keep.txt").exists());
    assert!(temp.path().join("excluded/drop.txt").exists());

    mediagit()
        .arg("sparse-checkout")
        .arg("set")
        .arg("included")
        .current_dir(temp.path())
        .assert()
        .success();

    assert!(temp.path().join("included/keep.txt").exists());
    assert!(
        !temp.path().join("excluded/drop.txt").exists(),
        "set must remove newly-excluded tracked files"
    );
}

#[test]
fn sparse_set_pattern_mode_matches_glob() {
    let temp = TempDir::new().unwrap();
    init_repo(temp.path());
    commit_file(temp.path(), "a.keep", "keep", "add a");
    commit_file(temp.path(), "b.drop", "drop", "add b");

    mediagit()
        .arg("sparse-checkout")
        .arg("set")
        .arg("--patterns")
        .arg("*.keep")
        .current_dir(temp.path())
        .assert()
        .success();

    assert!(temp.path().join("a.keep").exists());
    assert!(!temp.path().join("b.drop").exists());
}

#[test]
fn sparse_list_shows_mode_and_patterns() {
    let temp = two_dir_repo();
    mediagit()
        .arg("sparse-checkout")
        .arg("set")
        .arg("included")
        .current_dir(temp.path())
        .assert()
        .success();

    mediagit()
        .arg("sparse-checkout")
        .arg("list")
        .current_dir(temp.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("cone"))
        .stdout(predicate::str::contains("included"));
}

#[test]
fn sparse_list_reports_disabled_when_no_patterns_set() {
    let temp = two_dir_repo();
    mediagit()
        .arg("sparse-checkout")
        .arg("list")
        .current_dir(temp.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("disabled"));
}

#[test]
fn sparse_disable_restores_full_tree_byte_identical() {
    let temp = two_dir_repo();
    let original = fs::read(temp.path().join("excluded/drop.txt")).unwrap();

    mediagit()
        .arg("sparse-checkout")
        .arg("set")
        .arg("included")
        .current_dir(temp.path())
        .assert()
        .success();
    assert!(!temp.path().join("excluded/drop.txt").exists());

    mediagit()
        .arg("sparse-checkout")
        .arg("disable")
        .current_dir(temp.path())
        .assert()
        .success();

    let restored_path = temp.path().join("excluded/drop.txt");
    assert!(
        restored_path.exists(),
        "disable must restore excluded files"
    );
    assert_eq!(
        fs::read(&restored_path).unwrap(),
        original,
        "restored content must be byte-identical"
    );

    mediagit()
        .arg("sparse-checkout")
        .arg("list")
        .current_dir(temp.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("disabled"));
}

#[test]
fn sparse_branch_switch_keeps_filter() {
    let temp = two_dir_repo();
    mediagit()
        .arg("sparse-checkout")
        .arg("set")
        .arg("included")
        .current_dir(temp.path())
        .assert()
        .success();
    assert!(!temp.path().join("excluded/drop.txt").exists());

    mediagit()
        .arg("branch")
        .arg("create")
        .arg("feature")
        .current_dir(temp.path())
        .assert()
        .success();
    mediagit()
        .arg("branch")
        .arg("switch")
        .arg("feature")
        .current_dir(temp.path())
        .assert()
        .success();

    commit_file(
        temp.path(),
        "included/keep.txt",
        "keep me v2",
        "update keep",
    );

    mediagit()
        .arg("branch")
        .arg("switch")
        .arg("main")
        .current_dir(temp.path())
        .assert()
        .success();

    assert!(
        !temp.path().join("excluded/drop.txt").exists(),
        "excluded file must stay absent across branch switch"
    );
    assert!(temp.path().join("included/keep.txt").exists());
}

/// Repro-first: `status` must not report a sparse-excluded path as deleted.
/// Before the M5b filter in `deleted_files`'s collection block, this file's
/// absence (removed by `sparse-checkout set`) would show up as `D` both in
/// human output and porcelain.
#[test]
fn status_does_not_report_sparse_excluded_files_as_deleted() {
    let temp = two_dir_repo();

    mediagit()
        .arg("sparse-checkout")
        .arg("set")
        .arg("included")
        .current_dir(temp.path())
        .assert()
        .success();
    assert!(!temp.path().join("excluded/drop.txt").exists());

    // Human output: no "deleted" section for the excluded file.
    mediagit()
        .arg("status")
        .current_dir(temp.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("drop.txt").not());

    // Porcelain: no "D excluded/drop.txt" line.
    let output = mediagit()
        .arg("status")
        .arg("--porcelain")
        .current_dir(temp.path())
        .output()
        .unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        !stdout.contains("drop.txt"),
        "sparse-excluded file must not appear in porcelain output at all:\n{stdout}"
    );

    // --json: not in `deleted`.
    let output = mediagit()
        .arg("status")
        .arg("--json")
        .current_dir(temp.path())
        .output()
        .unwrap();
    assert!(output.status.success());
    let json: serde_json::Value =
        serde_json::from_str(&String::from_utf8_lossy(&output.stdout)).unwrap();
    let deleted = json["deleted"].as_array().unwrap();
    assert!(
        !deleted
            .iter()
            .any(|v| v.as_str().unwrap_or("").contains("drop.txt")),
        "sparse-excluded file must not appear in --json deleted[]: {deleted:?}"
    );
}
