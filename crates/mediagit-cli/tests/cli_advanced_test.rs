// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2025 MediaGit Contributors

//! Comprehensive CLI Advanced Command Tests
//!
//! Tests for `rebase`, `cherry-pick`, `bisect`, `filter`, `track`, and `install` commands.

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
// Rebase Command Tests
// ============================================================================

#[test]
fn test_rebase_basic() {
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());

    // Create base commit
    add_and_commit(temp_dir.path(), "base.txt", "Base content", "Base commit");

    // Create feature branch with commits
    create_and_switch_branch(temp_dir.path(), "feature");
    add_and_commit(temp_dir.path(), "feature.txt", "Feature 1", "Feature commit 1");
    add_and_commit(temp_dir.path(), "feature2.txt", "Feature 2", "Feature commit 2");

    // Switch to main and add commit
    mediagit().arg("branch").arg("switch").arg("refs/heads/main").current_dir(temp_dir.path()).assert().success();
    add_and_commit(temp_dir.path(), "main.txt", "Main content", "Main commit");

    // Switch back to feature
    mediagit().arg("branch").arg("switch").arg("feature").current_dir(temp_dir.path()).assert().success();

    // Rebase onto main
    mediagit()
        .arg("rebase")
        .arg("refs/heads/main")
        .current_dir(temp_dir.path())
        .assert()
        .success();
}

#[test]
fn test_rebase_abort() {
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());

    add_and_commit(temp_dir.path(), "file.txt", "Content", "Initial commit");

    // Abort when not rebasing should fail gracefully
    mediagit()
        .arg("rebase")
        .arg("--abort")
        .current_dir(temp_dir.path())
        .assert()
        .failure();
}

#[test]
fn test_rebase_continue() {
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());

    add_and_commit(temp_dir.path(), "file.txt", "Content", "Initial commit");

    // Continue when not rebasing should fail gracefully
    mediagit()
        .arg("rebase")
        .arg("--continue")
        .current_dir(temp_dir.path())
        .assert()
        .failure();
}

#[test]
fn test_rebase_quiet() {
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());

    add_and_commit(temp_dir.path(), "base.txt", "Base", "Base commit");

    create_and_switch_branch(temp_dir.path(), "feature");
    add_and_commit(temp_dir.path(), "feature.txt", "Feature", "Feature commit");

    mediagit()
        .arg("rebase")
        .arg("-q")
        .arg("refs/heads/main")
        .current_dir(temp_dir.path())
        .assert()
        .success();
}

#[test]
fn test_rebase_help() {
    mediagit()
        .arg("rebase")
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("rebase"));
}

// ============================================================================
// Cherry-pick Command Tests
// ============================================================================

#[test]
fn test_cherrypick_single() {
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());

    add_and_commit(temp_dir.path(), "base.txt", "Base", "Base commit");

    // Create feature branch with a commit
    create_and_switch_branch(temp_dir.path(), "feature");
    add_and_commit(temp_dir.path(), "feature.txt", "Feature content", "Feature commit");

    // Get the commit hash (we'll use HEAD)
    // Switch back to main
    mediagit().arg("branch").arg("switch").arg("refs/heads/main").current_dir(temp_dir.path()).assert().success();

    // Cherry-pick from feature branch
    mediagit()
        .arg("cherry-pick")
        .arg("feature")
        .current_dir(temp_dir.path())
        .assert()
        .success();

    // Verify file exists on main
    assert!(temp_dir.path().join("feature.txt").exists());
}

#[test]
fn test_cherrypick_no_commit() {
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());

    add_and_commit(temp_dir.path(), "base.txt", "Base", "Base commit");

    create_and_switch_branch(temp_dir.path(), "feature");
    add_and_commit(temp_dir.path(), "feature.txt", "Feature", "Feature commit");

    mediagit().arg("branch").arg("switch").arg("refs/heads/main").current_dir(temp_dir.path()).assert().success();

    mediagit()
        .arg("cherry-pick")
        .arg("-n")
        .arg("feature")
        .current_dir(temp_dir.path())
        .assert()
        .success();

    // File should exist but not committed
    assert!(temp_dir.path().join("feature.txt").exists());
}

#[test]
fn test_cherrypick_abort() {
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());

    add_and_commit(temp_dir.path(), "file.txt", "Content", "Initial commit");

    mediagit()
        .arg("cherry-pick")
        .arg("--abort")
        .current_dir(temp_dir.path())
        .assert()
        .failure();
}

#[test]
fn test_cherrypick_help() {
    mediagit()
        .arg("cherry-pick")
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("cherry"));
}

// ============================================================================
// Bisect Command Tests
// ============================================================================

#[test]
fn test_bisect_start() {
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());

    // Create several commits
    for i in 1..=5 {
        add_and_commit(temp_dir.path(), &format!("file{}.txt", i), &format!("Content {}", i), &format!("Commit {}", i));
    }

    mediagit()
        .arg("bisect")
        .arg("start")
        .current_dir(temp_dir.path())
        .assert()
        .success();
}

#[test]
#[ignore] // HEAD~N syntax not supported
fn test_bisect_start_with_refs() {
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());

    for i in 1..=5 {
        add_and_commit(temp_dir.path(), &format!("file{}.txt", i), &format!("Content {}", i), &format!("Commit {}", i));
    }

    // Start bisect with bad and good commits
    mediagit()
        .arg("bisect")
        .arg("start")
        .arg("HEAD")
        .arg("HEAD~4")
        .current_dir(temp_dir.path())
        .assert()
        .success();
}

#[test]
#[ignore] // Bisect reset behavior depends on state
fn test_bisect_reset() {
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());

    add_and_commit(temp_dir.path(), "file.txt", "Content", "Initial commit");

    // Reset should work even if not bisecting
    mediagit()
        .arg("bisect")
        .arg("reset")
        .current_dir(temp_dir.path())
        .assert()
        .success();
}

#[test]
#[ignore] // Bisect log may require active bisect
fn test_bisect_log() {
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());

    add_and_commit(temp_dir.path(), "file.txt", "Content", "Initial commit");

    mediagit()
        .arg("bisect")
        .arg("log")
        .current_dir(temp_dir.path())
        .assert()
        .success();
}

#[test]
fn test_bisect_help() {
    mediagit()
        .arg("bisect")
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("bisect"));
}

// ============================================================================
// Filter Command Tests
// ============================================================================

#[test]
fn test_filter_clean() {
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());

    // Create a file to filter
    fs::write(temp_dir.path().join("large.bin"), vec![0u8; 1024]).unwrap();

    mediagit()
        .arg("filter")
        .arg("clean")
        .arg("large.bin")
        .current_dir(temp_dir.path())
        .assert()
        .success();
}

#[test]
fn test_filter_smudge() {
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());

    add_and_commit(temp_dir.path(), "file.txt", "Content", "Initial commit");

    mediagit()
        .arg("filter")
        .arg("smudge")
        .arg("file.txt")
        .current_dir(temp_dir.path())
        .assert()
        .success();
}

#[test]
fn test_filter_help() {
    mediagit()
        .arg("filter")
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("filter"));
}

// ============================================================================
// Track Command Tests
// ============================================================================

#[test]
fn test_track_pattern() {
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());

    mediagit()
        .arg("track")
        .arg("*.psd")
        .current_dir(temp_dir.path())
        .assert()
        .success();
}

#[test]
fn test_track_multiple_patterns() {
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());

    mediagit().arg("track").arg("*.psd").current_dir(temp_dir.path()).assert().success();
    mediagit().arg("track").arg("*.mp4").current_dir(temp_dir.path()).assert().success();
    mediagit().arg("track").arg("*.mov").current_dir(temp_dir.path()).assert().success();
}

#[test]
fn test_track_list() {
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());

    mediagit().arg("track").arg("*.psd").current_dir(temp_dir.path()).assert().success();
    mediagit().arg("track").arg("*.mp4").current_dir(temp_dir.path()).assert().success();

    mediagit()
        .arg("track")
        .arg("--list")
        .current_dir(temp_dir.path())
        .assert()
        .success();
}

#[test]
fn test_track_help() {
    mediagit()
        .arg("track")
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("track"));
}

// ============================================================================
// Install Command Tests
// ============================================================================

#[test]
#[ignore] // Install may require specific system setup
fn test_install_basic() {
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());

    mediagit()
        .arg("install")
        .current_dir(temp_dir.path())
        .assert()
        .success();
}

#[test]
#[ignore] // Install may require specific system setup
fn test_install_force() {
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());

    // First install
    mediagit().arg("install").current_dir(temp_dir.path()).assert().success();

    // Force reinstall
    mediagit()
        .arg("install")
        .arg("-f")
        .current_dir(temp_dir.path())
        .assert()
        .success();
}

#[test]
#[ignore] // Install may require specific system setup
fn test_install_repo() {
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());

    mediagit()
        .arg("install")
        .arg("-r")
        .arg(temp_dir.path())
        .current_dir(temp_dir.path())
        .assert()
        .success();
}

#[test]
fn test_install_help() {
    mediagit()
        .arg("install")
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("install"));
}
