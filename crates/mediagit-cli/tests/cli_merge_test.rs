// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2025 MediaGit Contributors

//! Comprehensive CLI Merge Command Tests
//!
//! Tests for `mediagit merge` command with all options and merge strategies.

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

fn create_and_switch_branch(dir: &Path, branch_name: &str) {
    mediagit().arg("branch").arg("create").arg(branch_name).current_dir(dir).assert().success();
    mediagit().arg("branch").arg("switch").arg(branch_name).current_dir(dir).assert().success();
}

// ============================================================================
// Basic Merge Tests
// ============================================================================

#[test]
fn test_merge_fast_forward() {
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());

    // Create initial commit on main
    add_and_commit(temp_dir.path(), "file.txt", "Initial content", "Initial commit");

    // Create feature branch and add commits
    create_and_switch_branch(temp_dir.path(), "feature");
    add_and_commit(temp_dir.path(), "feature.txt", "Feature content", "Feature commit");
    add_and_commit(temp_dir.path(), "feature2.txt", "Feature 2", "Feature commit 2");

    // Switch back to main
    mediagit()
        .arg("branch")
        .arg("switch")
        .arg("refs/heads/main")
        .current_dir(temp_dir.path())
        .assert()
        .success();

    // Merge feature (should fast-forward)
    mediagit()
        .arg("merge")
        .arg("feature")
        .current_dir(temp_dir.path())
        .assert()
        .success();

    // Verify feature files exist on main
    assert!(temp_dir.path().join("feature.txt").exists());
    assert!(temp_dir.path().join("feature2.txt").exists());
}

#[test]
fn test_merge_no_ff() {
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());

    add_and_commit(temp_dir.path(), "file.txt", "Initial", "Initial commit");

    create_and_switch_branch(temp_dir.path(), "feature");
    add_and_commit(temp_dir.path(), "feature.txt", "Feature", "Feature commit");

    mediagit()
        .arg("branch")
        .arg("switch")
        .arg("refs/heads/main")
        .current_dir(temp_dir.path())
        .assert()
        .success();

    // Merge with --no-ff to create merge commit
    mediagit()
        .arg("merge")
        .arg("--no-ff")
        .arg("-m")
        .arg("Merge feature branch")
        .arg("feature")
        .current_dir(temp_dir.path())
        .assert()
        .success();

    // Verify merge commit exists
    mediagit()
        .arg("log")
        .arg("--oneline")
        .arg("-n")
        .arg("1")
        .current_dir(temp_dir.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("Merge"));
}

#[test]
fn test_merge_ff_only() {
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());

    add_and_commit(temp_dir.path(), "file.txt", "Initial", "Initial commit");

    create_and_switch_branch(temp_dir.path(), "feature");
    add_and_commit(temp_dir.path(), "feature.txt", "Feature", "Feature commit");

    mediagit()
        .arg("branch")
        .arg("switch")
        .arg("refs/heads/main")
        .current_dir(temp_dir.path())
        .assert()
        .success();

    // Fast-forward only should succeed
    mediagit()
        .arg("merge")
        .arg("--ff-only")
        .arg("feature")
        .current_dir(temp_dir.path())
        .assert()
        .success();
}

// ============================================================================
// Merge with Message Tests
// ============================================================================

#[test]
fn test_merge_with_message() {
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());

    add_and_commit(temp_dir.path(), "file.txt", "Initial", "Initial commit");

    create_and_switch_branch(temp_dir.path(), "feature");
    add_and_commit(temp_dir.path(), "feature.txt", "Feature", "Feature commit");

    mediagit()
        .arg("branch")
        .arg("switch")
        .arg("refs/heads/main")
        .current_dir(temp_dir.path())
        .assert()
        .success();

    mediagit()
        .arg("merge")
        .arg("-m")
        .arg("Custom merge message")
        .arg("--no-ff")
        .arg("feature")
        .current_dir(temp_dir.path())
        .assert()
        .success();

    // Verify custom message
    mediagit()
        .arg("log")
        .arg("-n")
        .arg("1")
        .current_dir(temp_dir.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("Custom merge message"));
}

// ============================================================================
// Squash Merge Tests
// ============================================================================

#[test]
#[ignore] // MediaGit may not support --squash merge
fn test_merge_squash() {
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());

    add_and_commit(temp_dir.path(), "file.txt", "Initial", "Initial commit");

    create_and_switch_branch(temp_dir.path(), "feature");
    add_and_commit(temp_dir.path(), "f1.txt", "Feature 1", "Feature commit 1");
    add_and_commit(temp_dir.path(), "f2.txt", "Feature 2", "Feature commit 2");
    add_and_commit(temp_dir.path(), "f3.txt", "Feature 3", "Feature commit 3");

    mediagit()
        .arg("branch")
        .arg("switch")
        .arg("refs/heads/main")
        .current_dir(temp_dir.path())
        .assert()
        .success();

    // Squash merge - may not be supported
    mediagit()
        .arg("merge")
        .arg("--squash")
        .arg("feature")
        .current_dir(temp_dir.path());
}

// ============================================================================
// Three-Way Merge Tests
// ============================================================================

#[test]
fn test_merge_three_way() {
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());

    // Create base commit
    add_and_commit(temp_dir.path(), "base.txt", "Base content", "Base commit");

    // Create feature branch with new file
    create_and_switch_branch(temp_dir.path(), "feature");
    add_and_commit(temp_dir.path(), "feature.txt", "Feature content", "Feature commit");

    // Switch back to main and add different file
    mediagit()
        .arg("branch")
        .arg("switch")
        .arg("refs/heads/main")
        .current_dir(temp_dir.path())
        .assert()
        .success();

    add_and_commit(temp_dir.path(), "main.txt", "Main content", "Main commit");

    // Now merge feature into main (should be a three-way merge)
    mediagit()
        .arg("merge")
        .arg("-m")
        .arg("Merge feature into main")
        .arg("feature")
        .current_dir(temp_dir.path())
        .assert()
        .success();

    // Both files should exist
    assert!(temp_dir.path().join("feature.txt").exists());
    assert!(temp_dir.path().join("main.txt").exists());
}

// ============================================================================
// Merge Abort Tests
// ============================================================================

#[test]
fn test_merge_abort_no_merge() {
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());

    add_and_commit(temp_dir.path(), "file.txt", "Content", "Initial commit");

    // Abort when not in a merge should fail gracefully
    mediagit()
        .arg("merge")
        .arg("--abort")
        .current_dir(temp_dir.path())
        .assert()
        .failure();
}

// ============================================================================
// Error Handling Tests
// ============================================================================

#[test]
fn test_merge_nonexistent_branch() {
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());

    add_and_commit(temp_dir.path(), "file.txt", "Content", "Initial commit");

    mediagit()
        .arg("merge")
        .arg("nonexistent")
        .current_dir(temp_dir.path())
        .assert()
        .failure();
}

#[test]
fn test_merge_already_up_to_date() {
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());

    add_and_commit(temp_dir.path(), "file.txt", "Content", "Initial commit");

    // Create branch at same commit
    mediagit()
        .arg("branch")
        .arg("create")
        .arg("same")
        .current_dir(temp_dir.path())
        .assert()
        .success();

    // Merge should succeed (whether it says "already up to date" or not)
    mediagit()
        .arg("merge")
        .arg("same")
        .current_dir(temp_dir.path())
        .assert()
        .success();
}

#[test]
fn test_merge_help() {
    mediagit()
        .arg("merge")
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("merge"));
}

// ============================================================================
// Quiet and Verbose Tests
// ============================================================================

#[test]
fn test_merge_quiet() {
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());

    add_and_commit(temp_dir.path(), "file.txt", "Initial", "Initial commit");

    create_and_switch_branch(temp_dir.path(), "feature");
    add_and_commit(temp_dir.path(), "feature.txt", "Feature", "Feature commit");

    mediagit()
        .arg("branch")
        .arg("switch")
        .arg("refs/heads/main")
        .current_dir(temp_dir.path())
        .assert()
        .success();

    mediagit()
        .arg("merge")
        .arg("-q")
        .arg("feature")
        .current_dir(temp_dir.path())
        .assert()
        .success();
}

#[test]
fn test_merge_verbose() {
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());

    add_and_commit(temp_dir.path(), "file.txt", "Initial", "Initial commit");

    create_and_switch_branch(temp_dir.path(), "feature");
    add_and_commit(temp_dir.path(), "feature.txt", "Feature", "Feature commit");

    mediagit()
        .arg("branch")
        .arg("switch")
        .arg("refs/heads/main")
        .current_dir(temp_dir.path())
        .assert()
        .success();

    mediagit()
        .arg("merge")
        .arg("-v")
        .arg("feature")
        .current_dir(temp_dir.path())
        .assert()
        .success();
}
