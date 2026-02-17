// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2025 MediaGit Contributors

//! Comprehensive CLI Revert Command Tests
//!
//! Tests for `mediagit revert` command with all options.
//!
//! Run: `cargo test --test cli_revert_test`

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

fn add_and_commit(dir: &Path, name: &str, content: &str, message: &str) {
    fs::write(dir.join(name), content).unwrap();
    mediagit().arg("add").arg(name).current_dir(dir).assert().success();
    mediagit().arg("commit").arg("-m").arg(message).current_dir(dir).assert().success();
}

// ============================================================================
// Revert Help Tests
// ============================================================================

#[test]
fn test_revert_help() {
    mediagit()
        .arg("revert")
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("--no-commit"))
        .stdout(predicate::str::contains("--continue"))
        .stdout(predicate::str::contains("--abort"));
}

// ============================================================================
// Basic Revert Tests
// ============================================================================

#[test]
#[ignore] // Requires merge engine
fn test_revert_head() {
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());

    add_and_commit(temp_dir.path(), "file.txt", "v1", "First commit");
    add_and_commit(temp_dir.path(), "file.txt", "v2", "Second commit");

    // Revert HEAD
    mediagit()
        .arg("revert")
        .arg("HEAD")
        .current_dir(temp_dir.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("Created revert commit"));

    // File should now be v1 (reverted)
    let content = fs::read_to_string(temp_dir.path().join("file.txt")).unwrap();
    assert_eq!(content, "v1");
}

#[test]
#[ignore] // Requires merge engine
fn test_revert_by_oid_prefix() {
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());

    add_and_commit(temp_dir.path(), "file.txt", "v1", "First commit");
    add_and_commit(temp_dir.path(), "file.txt", "v2", "Second commit");

    // Get latest commit OID from log
    let output = mediagit()
        .arg("log")
        .arg("--oneline")
        .arg("-n")
        .arg("1")
        .current_dir(temp_dir.path())
        .output()
        .unwrap();

    let log_line = String::from_utf8_lossy(&output.stdout);
    let oid_prefix: String = log_line.chars().take(7).collect();

    mediagit()
        .arg("revert")
        .arg(&oid_prefix)
        .current_dir(temp_dir.path())
        .assert()
        .success();
}

// ============================================================================
// Revert with --no-commit
// ============================================================================

#[test]
#[ignore] // Requires merge engine
fn test_revert_no_commit() {
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());

    add_and_commit(temp_dir.path(), "file.txt", "v1", "First commit");
    add_and_commit(temp_dir.path(), "file.txt", "v2", "Second commit");

    mediagit()
        .arg("revert")
        .arg("-n")
        .arg("HEAD")
        .current_dir(temp_dir.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("not committed"));

    // File content should be reverted
    let content = fs::read_to_string(temp_dir.path().join("file.txt")).unwrap();
    assert_eq!(content, "v1");
}

// ============================================================================
// Revert with Custom Message
// ============================================================================

#[test]
#[ignore] // Requires merge engine
fn test_revert_custom_message() {
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());

    add_and_commit(temp_dir.path(), "file.txt", "v1", "Initial");
    add_and_commit(temp_dir.path(), "file.txt", "v2", "Change to revert");

    mediagit()
        .arg("revert")
        .arg("-m")
        .arg("Custom revert message")
        .arg("HEAD")
        .current_dir(temp_dir.path())
        .assert()
        .success();

    // Check log for custom message
    mediagit()
        .arg("log")
        .arg("-n")
        .arg("1")
        .current_dir(temp_dir.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("Custom revert message"));
}

// ============================================================================
// Revert Multiple Commits
// ============================================================================

#[test]
#[ignore] // Requires merge engine
fn test_revert_multiple_commits() {
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());

    add_and_commit(temp_dir.path(), "file.txt", "v1", "Commit 1");
    add_and_commit(temp_dir.path(), "file.txt", "v2", "Commit 2");
    add_and_commit(temp_dir.path(), "file.txt", "v3", "Commit 3");

    mediagit()
        .arg("revert")
        .arg("HEAD~1")
        .arg("HEAD")
        .current_dir(temp_dir.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("Successfully reverted 2 commit(s)"));
}

// ============================================================================
// Revert Continue/Abort/Skip Tests
// ============================================================================

#[test]
fn test_revert_continue_no_revert_in_progress() {
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());
    add_and_commit(temp_dir.path(), "file.txt", "content", "Initial");

    // Should fail - no revert in progress
    mediagit()
        .arg("revert")
        .arg("--continue")
        .current_dir(temp_dir.path())
        .assert()
        .failure();
}

#[test]
fn test_revert_abort_no_revert_in_progress() {
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());
    add_and_commit(temp_dir.path(), "file.txt", "content", "Initial");

    // Should fail - no revert in progress
    mediagit()
        .arg("revert")
        .arg("--abort")
        .current_dir(temp_dir.path())
        .assert()
        .failure();
}

#[test]
fn test_revert_skip_no_revert_in_progress() {
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());
    add_and_commit(temp_dir.path(), "file.txt", "content", "Initial");

    // Should fail - no revert in progress
    mediagit()
        .arg("revert")
        .arg("--skip")
        .current_dir(temp_dir.path())
        .assert()
        .failure();
}

// ============================================================================
// Error Case Tests
// ============================================================================

#[test]
fn test_revert_no_commits_specified() {
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());
    add_and_commit(temp_dir.path(), "file.txt", "content", "Initial");

    // Should fail - must specify at least one commit
    mediagit()
        .arg("revert")
        .current_dir(temp_dir.path())
        .assert()
        .failure();
}

#[test]
fn test_revert_no_repo() {
    let temp_dir = TempDir::new().unwrap();

    mediagit()
        .arg("revert")
        .arg("HEAD")
        .current_dir(temp_dir.path())
        .assert()
        .failure();
}

#[test]
#[ignore] // Requires error handling
fn test_revert_initial_commit_fails() {
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());
    add_and_commit(temp_dir.path(), "file.txt", "content", "Initial");

    // Reverting the initial commit should fail (no parent)
    mediagit()
        .arg("revert")
        .arg("HEAD")
        .current_dir(temp_dir.path())
        .assert()
        .failure()
        .stderr(predicate::str::contains("Cannot revert initial commit"));
}

// ============================================================================
// Quiet Mode Tests
// ============================================================================

#[test]
#[ignore] // Requires merge engine
fn test_revert_quiet() {
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());
    add_and_commit(temp_dir.path(), "file.txt", "v1", "Commit 1");
    add_and_commit(temp_dir.path(), "file.txt", "v2", "Commit 2");

    mediagit()
        .arg("revert")
        .arg("-q")
        .arg("HEAD")
        .current_dir(temp_dir.path())
        .assert()
        .success()
        .stdout(predicate::str::is_empty());
}
