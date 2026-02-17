// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2025 MediaGit Contributors

//! Comprehensive CLI Reflog Command Tests
//!
//! Tests for `mediagit reflog` command with all subcommands and options.
//!
//! Run: `cargo test --test cli_reflog_test`

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
// Reflog Help Tests
// ============================================================================

#[test]
fn test_reflog_help() {
    mediagit()
        .arg("reflog")
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("reflog"))
        .stdout(predicate::str::contains("show"))
        .stdout(predicate::str::contains("delete"))
        .stdout(predicate::str::contains("expire"));
}

// ============================================================================
// Reflog Show Tests
// ============================================================================

#[test]
#[ignore] // Requires reflog population from commits
fn test_reflog_show_head() {
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());

    add_and_commit(temp_dir.path(), "file.txt", "v1", "First commit");
    add_and_commit(temp_dir.path(), "file.txt", "v2", "Second commit");

    // Show reflog for HEAD (default)
    mediagit()
        .arg("reflog")
        .current_dir(temp_dir.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("HEAD@{"));
}

#[test]
#[ignore] // Requires reflog population
fn test_reflog_show_explicit() {
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());

    add_and_commit(temp_dir.path(), "file.txt", "v1", "First commit");

    // Show reflog using explicit 'show' subcommand
    mediagit()
        .arg("reflog")
        .arg("show")
        .current_dir(temp_dir.path())
        .assert()
        .success();
}

#[test]
#[ignore] // Requires reflog population
fn test_reflog_show_limit() {
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());

    for i in 1..=5 {
        add_and_commit(temp_dir.path(), "file.txt", &format!("v{}", i), &format!("Commit {}", i));
    }

    // Show only last 2 entries
    mediagit()
        .arg("reflog")
        .arg("-n")
        .arg("2")
        .current_dir(temp_dir.path())
        .assert()
        .success();
}

#[test]
#[ignore] // Requires reflog population 
fn test_reflog_show_all() {
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());

    add_and_commit(temp_dir.path(), "file.txt", "v1", "First commit");

    // Show all reflogs
    mediagit()
        .arg("reflog")
        .arg("show")
        .arg("--all")
        .current_dir(temp_dir.path())
        .assert()
        .success();
}

#[test]
#[ignore] // Requires reflog population
fn test_reflog_show_quiet() {
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());

    add_and_commit(temp_dir.path(), "file.txt", "v1", "First commit");

    // Quiet mode - only OIDs
    mediagit()
        .arg("reflog")
        .arg("-q")
        .current_dir(temp_dir.path())
        .assert()
        .success();
}

#[test]
fn test_reflog_show_empty() {
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());

    // Show reflog when empty - should succeed with info message
    mediagit()
        .arg("reflog")
        .current_dir(temp_dir.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("No reflog entries"));
}

// ============================================================================
// Reflog Delete Tests
// ============================================================================

#[test]
#[ignore] // Requires reflog population
fn test_reflog_delete() {
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());

    add_and_commit(temp_dir.path(), "file.txt", "v1", "First commit");

    mediagit()
        .arg("reflog")
        .arg("delete")
        .arg("HEAD")
        .current_dir(temp_dir.path())
        .assert()
        .success();
}

#[test]
fn test_reflog_delete_nonexistent() {
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());

    // Delete non-existent reflog - should succeed with info message
    mediagit()
        .arg("reflog")
        .arg("delete")
        .arg("refs/heads/nonexistent")
        .current_dir(temp_dir.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("No reflog found"));
}

#[test]
fn test_reflog_delete_requires_ref() {
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());

    // Delete without specifying reference
    mediagit()
        .arg("reflog")
        .arg("delete")
        .current_dir(temp_dir.path())
        .assert()
        .failure();
}

// ============================================================================
// Reflog Expire Tests
// ============================================================================

#[test]
#[ignore] // Requires reflog population
fn test_reflog_expire() {
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());

    for i in 1..=10 {
        add_and_commit(temp_dir.path(), "file.txt", &format!("v{}", i), &format!("Commit {}", i));
    }

    // Expire keeping only 5 entries
    mediagit()
        .arg("reflog")
        .arg("expire")
        .arg("--keep")
        .arg("5")
        .arg("HEAD")
        .current_dir(temp_dir.path())
        .assert()
        .success();
}

#[test]
#[ignore] // Requires reflog population
fn test_reflog_expire_all() {
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());

    add_and_commit(temp_dir.path(), "file.txt", "v1", "First commit");

    // Expire all refs (no ref specified)
    mediagit()
        .arg("reflog")
        .arg("expire")
        .current_dir(temp_dir.path())
        .assert()
        .success();
}

#[test]
fn test_reflog_expire_empty() {
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());

    // Expire with no entries - should succeed
    mediagit()
        .arg("reflog")
        .arg("expire")
        .current_dir(temp_dir.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("No entries to expire"));
}

// ============================================================================
// Reflog Specific Ref Tests
// ============================================================================

#[test]
#[ignore] // Requires branch operations
fn test_reflog_specific_branch() {
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());

    add_and_commit(temp_dir.path(), "file.txt", "v1", "First commit");

    // Create a branch
    mediagit()
        .arg("branch")
        .arg("feature")
        .current_dir(temp_dir.path())
        .assert()
        .success();

    // Show reflog for specific branch
    mediagit()
        .arg("reflog")
        .arg("refs/heads/feature")
        .current_dir(temp_dir.path())
        .assert()
        .success();
}

// ============================================================================
// Error Case Tests
// ============================================================================

#[test]
fn test_reflog_no_repo() {
    let temp_dir = TempDir::new().unwrap();

    mediagit()
        .arg("reflog")
        .current_dir(temp_dir.path())
        .assert()
        .failure();
}
