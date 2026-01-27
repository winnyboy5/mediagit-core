// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2025 MediaGit Contributors

//! Comprehensive CLI Remote Command Tests
//!
//! Tests for `mediagit remote` command with all subcommands and options.

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
    mediagit().arg("init").arg("-q").current_dir(dir).assert().success();
}

#[allow(dead_code)]
fn add_and_commit(dir: &Path, name: &str, content: &str, message: &str) {
    fs::write(dir.join(name), content).unwrap();
    mediagit().arg("add").arg(name).current_dir(dir).assert().success();
    mediagit().arg("commit").arg("-m").arg(message).current_dir(dir).assert().success();
}

// ============================================================================
// Remote Add Tests
// ============================================================================

#[test]
fn test_remote_add() {
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());

    mediagit()
        .arg("remote")
        .arg("add")
        .arg("origin")
        .arg("http://localhost:3000/test-repo")
        .current_dir(temp_dir.path())
        .assert()
        .success();

    // Verify remote was added
    mediagit()
        .arg("remote")
        .arg("list")
        .current_dir(temp_dir.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("origin"));
}

#[test]
fn test_remote_add_multiple() {
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());

    mediagit()
        .arg("remote")
        .arg("add")
        .arg("origin")
        .arg("http://localhost:3000/test-repo")
        .current_dir(temp_dir.path())
        .assert()
        .success();

    mediagit()
        .arg("remote")
        .arg("add")
        .arg("upstream")
        .arg("http://localhost:3000/upstream-repo")
        .current_dir(temp_dir.path())
        .assert()
        .success();

    mediagit()
        .arg("remote")
        .arg("add")
        .arg("backup")
        .arg("http://backup-server:3000/backup-repo")
        .current_dir(temp_dir.path())
        .assert()
        .success();

    // Verify all remotes exist
    mediagit()
        .arg("remote")
        .arg("list")
        .current_dir(temp_dir.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("origin"))
        .stdout(predicate::str::contains("upstream"))
        .stdout(predicate::str::contains("backup"));
}

// ============================================================================
// Remote List Tests
// ============================================================================

#[test]
fn test_remote_list() {
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());

    mediagit()
        .arg("remote")
        .arg("add")
        .arg("origin")
        .arg("http://localhost:3000/test-repo")
        .current_dir(temp_dir.path())
        .assert()
        .success();

    mediagit()
        .arg("remote")
        .arg("list")
        .current_dir(temp_dir.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("origin"));
}

#[test]
fn test_remote_list_verbose() {
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());

    mediagit()
        .arg("remote")
        .arg("add")
        .arg("origin")
        .arg("http://localhost:3000/test-repo")
        .current_dir(temp_dir.path())
        .assert()
        .success();

    mediagit()
        .arg("remote")
        .arg("list")
        .arg("-v")
        .current_dir(temp_dir.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("http://localhost:3000/test-repo"));
}

#[test]
fn test_remote_list_empty() {
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());

    mediagit()
        .arg("remote")
        .arg("list")
        .current_dir(temp_dir.path())
        .assert()
        .success();
}

// ============================================================================
// Remote Remove Tests
// ============================================================================

#[test]
fn test_remote_remove() {
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());

    mediagit()
        .arg("remote")
        .arg("add")
        .arg("origin")
        .arg("http://localhost:3000/test-repo")
        .current_dir(temp_dir.path())
        .assert()
        .success();

    mediagit()
        .arg("remote")
        .arg("remove")
        .arg("origin")
        .current_dir(temp_dir.path())
        .assert()
        .success();

    // Verify remote is gone
    mediagit()
        .arg("remote")
        .arg("list")
        .current_dir(temp_dir.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("origin").not());
}

// ============================================================================
// Remote Rename Tests
// ============================================================================

#[test]
fn test_remote_rename() {
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());

    mediagit()
        .arg("remote")
        .arg("add")
        .arg("origin")
        .arg("http://localhost:3000/test-repo")
        .current_dir(temp_dir.path())
        .assert()
        .success();

    mediagit()
        .arg("remote")
        .arg("rename")
        .arg("origin")
        .arg("upstream")
        .current_dir(temp_dir.path())
        .assert()
        .success();

    // Verify rename
    mediagit()
        .arg("remote")
        .arg("list")
        .current_dir(temp_dir.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("upstream"))
        .stdout(predicate::str::contains("origin").not());
}

// ============================================================================
// Remote Set-URL Tests
// ============================================================================

#[test]
fn test_remote_set_url() {
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());

    mediagit()
        .arg("remote")
        .arg("add")
        .arg("origin")
        .arg("http://localhost:3000/old-repo")
        .current_dir(temp_dir.path())
        .assert()
        .success();

    mediagit()
        .arg("remote")
        .arg("set-url")
        .arg("origin")
        .arg("http://localhost:3000/new-repo")
        .current_dir(temp_dir.path())
        .assert()
        .success();

    // Verify new URL
    mediagit()
        .arg("remote")
        .arg("list")
        .arg("-v")
        .current_dir(temp_dir.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("new-repo"));
}

// ============================================================================
// Remote Show Tests
// ============================================================================

#[test]
fn test_remote_show() {
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());

    mediagit()
        .arg("remote")
        .arg("add")
        .arg("origin")
        .arg("http://localhost:3000/test-repo")
        .current_dir(temp_dir.path())
        .assert()
        .success();

    mediagit()
        .arg("remote")
        .arg("show")
        .arg("origin")
        .current_dir(temp_dir.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("origin"));
}

// ============================================================================
// Error Handling Tests
// ============================================================================

#[test]
fn test_remote_add_duplicate() {
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());

    mediagit()
        .arg("remote")
        .arg("add")
        .arg("origin")
        .arg("http://localhost:3000/test-repo")
        .current_dir(temp_dir.path())
        .assert()
        .success();

    // Adding duplicate should fail
    mediagit()
        .arg("remote")
        .arg("add")
        .arg("origin")
        .arg("http://localhost:3000/other-repo")
        .current_dir(temp_dir.path())
        .assert()
        .failure();
}

#[test]
fn test_remote_remove_nonexistent() {
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());

    mediagit()
        .arg("remote")
        .arg("remove")
        .arg("nonexistent")
        .current_dir(temp_dir.path())
        .assert()
        .failure();
}

#[test]
fn test_remote_rename_nonexistent() {
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());

    mediagit()
        .arg("remote")
        .arg("rename")
        .arg("nonexistent")
        .arg("newname")
        .current_dir(temp_dir.path())
        .assert()
        .failure();
}

#[test]
fn test_remote_set_url_nonexistent() {
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());

    mediagit()
        .arg("remote")
        .arg("set-url")
        .arg("nonexistent")
        .arg("http://localhost:3000/repo")
        .current_dir(temp_dir.path())
        .assert()
        .failure();
}

#[test]
fn test_remote_help() {
    mediagit()
        .arg("remote")
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("remote"));
}

// ============================================================================
// URL Format Tests
// ============================================================================

#[test]
fn test_remote_add_https_url() {
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());

    mediagit()
        .arg("remote")
        .arg("add")
        .arg("secure")
        .arg("https://server.example.com/repo")
        .current_dir(temp_dir.path())
        .assert()
        .success();
}

#[test]
fn test_remote_add_with_port() {
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());

    mediagit()
        .arg("remote")
        .arg("add")
        .arg("custom")
        .arg("http://server.example.com:8080/repo")
        .current_dir(temp_dir.path())
        .assert()
        .success();
}

#[test]
fn test_remote_add_with_path() {
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());

    mediagit()
        .arg("remote")
        .arg("add")
        .arg("path-remote")
        .arg("http://server.example.com/org/team/project/repo")
        .current_dir(temp_dir.path())
        .assert()
        .success();
}
