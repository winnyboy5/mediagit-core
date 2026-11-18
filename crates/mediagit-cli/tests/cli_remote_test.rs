// SPDX-License-Identifier: BUSL-1.1
// Copyright (C) 2025-2026 Aswin Krishnamoorthy

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

#[allow(dead_code)]
fn add_and_commit(dir: &Path, name: &str, content: &str, message: &str) {
    fs::write(dir.join(name), content).unwrap();
    mediagit()
        .arg("add")
        .arg(name)
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

/// BUG-034: the saved sign-in is keyed by server and shared with other
/// projects and `auth login`, so `remote remove` must leave it. Opt-in: it
/// writes a real OS keychain entry (and deletes it again).
#[test]
fn test_remote_remove_keeps_the_saved_sign_in() {
    if std::env::var_os("MEDIAGIT_TEST_REAL_KEYRING").is_none() {
        eprintln!("skipping: set MEDIAGIT_TEST_REAL_KEYRING=1 to run against the real OS keychain");
        return;
    }
    let account = "http://127.0.0.1:9981";
    let entry = keyring::Entry::new("mediagit", account).unwrap();
    let _ = entry.delete_credential();
    entry.set_password("bearer:bug034-test-token").unwrap();

    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());
    mediagit()
        .args(["remote", "add", "origin", "http://127.0.0.1:9981/hero"])
        .env_remove("MEDIAGIT_NO_KEYRING")
        .current_dir(temp_dir.path())
        .assert()
        .success();
    mediagit()
        .args(["remote", "remove", "origin"])
        .env_remove("MEDIAGIT_NO_KEYRING")
        .current_dir(temp_dir.path())
        .assert()
        .success();

    let kept = entry.get_password();
    let _ = entry.delete_credential();
    assert_eq!(kept.as_deref().ok(), Some("bearer:bug034-test-token"));
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
