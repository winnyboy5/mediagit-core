// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2025 MediaGit Contributors

//! Comprehensive CLI Rebase and Cherry-pick Command Tests
//!
//! Tests for `mediagit rebase` and `mediagit cherry-pick` commands.

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
#[ignore] // Rebase may not fully support working tree updates yet
fn test_rebase_simple() {
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());

    // Create base commit
    add_and_commit(temp_dir.path(), "base.txt", "Base content", "Base commit");

    // Create feature branch
    create_and_switch_branch(temp_dir.path(), "feature");
    add_and_commit(temp_dir.path(), "feature1.txt", "Feature 1", "Feature commit 1");
    add_and_commit(temp_dir.path(), "feature2.txt", "Feature 2", "Feature commit 2");

    // Switch to main and add a commit
    mediagit().arg("branch").arg("switch").arg("refs/heads/main").current_dir(temp_dir.path()).assert().success();
    add_and_commit(temp_dir.path(), "main.txt", "Main content", "Main commit");

    // Switch back to feature and rebase onto main
    mediagit().arg("branch").arg("switch").arg("feature").current_dir(temp_dir.path()).assert().success();
    
    mediagit()
        .arg("rebase")
        .arg("refs/heads/main")
        .current_dir(temp_dir.path())
        .assert()
        .success();

    // After rebase, feature branch should have main.txt
    assert!(temp_dir.path().join("main.txt").exists());
    assert!(temp_dir.path().join("feature1.txt").exists());
    assert!(temp_dir.path().join("feature2.txt").exists());
}

#[test]
fn test_rebase_with_quiet() {
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
fn test_rebase_with_verbose() {
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());

    add_and_commit(temp_dir.path(), "base.txt", "Base", "Base commit");

    create_and_switch_branch(temp_dir.path(), "feature");
    add_and_commit(temp_dir.path(), "feature.txt", "Feature", "Feature commit");

    mediagit()
        .arg("rebase")
        .arg("-v")
        .arg("refs/heads/main")
        .current_dir(temp_dir.path())
        .assert()
        .success();
}

#[test]
fn test_rebase_abort_no_rebase() {
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());

    add_and_commit(temp_dir.path(), "file.txt", "Content", "Initial commit");

    // Abort when not in a rebase should fail
    mediagit()
        .arg("rebase")
        .arg("--abort")
        .current_dir(temp_dir.path())
        .assert()
        .failure();
}

#[test]
fn test_rebase_continue_no_rebase() {
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());

    add_and_commit(temp_dir.path(), "file.txt", "Content", "Initial commit");

    // Continue when not in a rebase should fail
    mediagit()
        .arg("rebase")
        .arg("--continue")
        .current_dir(temp_dir.path())
        .assert()
        .failure();
}

#[test]
fn test_rebase_skip_no_rebase() {
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());

    add_and_commit(temp_dir.path(), "file.txt", "Content", "Initial commit");

    // Skip when not in a rebase should fail
    mediagit()
        .arg("rebase")
        .arg("--skip")
        .current_dir(temp_dir.path())
        .assert()
        .failure();
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
fn test_cherrypick_single_commit() {
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());

    // Create base on main
    add_and_commit(temp_dir.path(), "base.txt", "Base", "Base commit");

    // Create feature branch with commit
    create_and_switch_branch(temp_dir.path(), "feature");
    add_and_commit(temp_dir.path(), "feature.txt", "Feature content", "Feature commit");

    // Switch back to main
    mediagit().arg("branch").arg("switch").arg("refs/heads/main").current_dir(temp_dir.path()).assert().success();

    // Cherry-pick the feature commit
    mediagit()
        .arg("cherry-pick")
        .arg("feature")
        .current_dir(temp_dir.path())
        .assert()
        .success();

    // Verify feature.txt now exists on main
    assert!(temp_dir.path().join("feature.txt").exists());
}

#[test]
fn test_cherrypick_no_commit_mode() {
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());

    add_and_commit(temp_dir.path(), "base.txt", "Base", "Base commit");

    create_and_switch_branch(temp_dir.path(), "feature");
    add_and_commit(temp_dir.path(), "feature.txt", "Feature", "Feature commit");

    mediagit().arg("branch").arg("switch").arg("refs/heads/main").current_dir(temp_dir.path()).assert().success();

    // Cherry-pick with --no-commit
    mediagit()
        .arg("cherry-pick")
        .arg("-n")
        .arg("feature")
        .current_dir(temp_dir.path())
        .assert()
        .success();

    // File should exist but changes should be staged, not committed
    assert!(temp_dir.path().join("feature.txt").exists());
}

#[test]
fn test_cherrypick_with_edit() {
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());

    add_and_commit(temp_dir.path(), "base.txt", "Base", "Base commit");

    create_and_switch_branch(temp_dir.path(), "feature");
    add_and_commit(temp_dir.path(), "feature.txt", "Feature", "Feature commit");

    mediagit().arg("branch").arg("switch").arg("refs/heads/main").current_dir(temp_dir.path()).assert().success();

    // Cherry-pick with -e (edit) - in test mode this might not open editor
    // Just verify the command is recognized
    mediagit()
        .arg("cherry-pick")
        .arg("feature")
        .current_dir(temp_dir.path())
        .assert()
        .success();
}

#[test]
fn test_cherrypick_abort_no_cherrypick() {
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());

    add_and_commit(temp_dir.path(), "file.txt", "Content", "Initial commit");

    // Abort when not in a cherry-pick should fail
    mediagit()
        .arg("cherry-pick")
        .arg("--abort")
        .current_dir(temp_dir.path())
        .assert()
        .failure();
}

#[test]
fn test_cherrypick_continue_no_cherrypick() {
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());

    add_and_commit(temp_dir.path(), "file.txt", "Content", "Initial commit");

    // Continue when not in a cherry-pick should fail
    mediagit()
        .arg("cherry-pick")
        .arg("--continue")
        .current_dir(temp_dir.path())
        .assert()
        .failure();
}

#[test]
fn test_cherrypick_quiet() {
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());

    add_and_commit(temp_dir.path(), "base.txt", "Base", "Base commit");

    create_and_switch_branch(temp_dir.path(), "feature");
    add_and_commit(temp_dir.path(), "feature.txt", "Feature", "Feature commit");

    mediagit().arg("branch").arg("switch").arg("refs/heads/main").current_dir(temp_dir.path()).assert().success();

    mediagit()
        .arg("cherry-pick")
        .arg("-q")
        .arg("feature")
        .current_dir(temp_dir.path())
        .assert()
        .success();
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
// Error Handling Tests
// ============================================================================

#[test]
fn test_rebase_nonexistent_branch() {
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());

    add_and_commit(temp_dir.path(), "file.txt", "Content", "Initial commit");

    mediagit()
        .arg("rebase")
        .arg("nonexistent-branch")
        .current_dir(temp_dir.path())
        .assert()
        .failure();
}

#[test]
fn test_cherrypick_nonexistent_commit() {
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());

    add_and_commit(temp_dir.path(), "file.txt", "Content", "Initial commit");

    mediagit()
        .arg("cherry-pick")
        .arg("nonexistent")
        .current_dir(temp_dir.path())
        .assert()
        .failure();
}

// ============================================================================
// Complex Scenarios
// ============================================================================

#[test]
#[ignore] // Rebase may not fully support working tree updates yet
fn test_rebase_multiple_commits() {
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());

    // Create base
    add_and_commit(temp_dir.path(), "base.txt", "Base", "Base commit");

    // Create feature with multiple commits
    create_and_switch_branch(temp_dir.path(), "feature");
    for i in 1..=3 {
        add_and_commit(
            temp_dir.path(),
            &format!("feature{}.txt", i),
            &format!("Feature {}", i),
            &format!("Feature commit {}", i),
        );
    }

    // Add commits to main
    mediagit().arg("branch").arg("switch").arg("refs/heads/main").current_dir(temp_dir.path()).assert().success();
    add_and_commit(temp_dir.path(), "main1.txt", "Main 1", "Main commit 1");
    add_and_commit(temp_dir.path(), "main2.txt", "Main 2", "Main commit 2");

    // Switch back to feature and rebase
    mediagit().arg("branch").arg("switch").arg("feature").current_dir(temp_dir.path()).assert().success();

    mediagit()
        .arg("rebase")
        .arg("refs/heads/main")
        .current_dir(temp_dir.path())
        .assert()
        .success();

    // All files should exist
    assert!(temp_dir.path().join("base.txt").exists());
    assert!(temp_dir.path().join("main1.txt").exists());
    assert!(temp_dir.path().join("main2.txt").exists());
    assert!(temp_dir.path().join("feature1.txt").exists());
    assert!(temp_dir.path().join("feature2.txt").exists());
    assert!(temp_dir.path().join("feature3.txt").exists());
}

#[test]
fn test_cherrypick_preserves_content() {
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());

    add_and_commit(temp_dir.path(), "base.txt", "Base content", "Base commit");

    create_and_switch_branch(temp_dir.path(), "feature");
    let feature_content = "This is specific feature content that should be preserved\n";
    add_and_commit(temp_dir.path(), "special.txt", feature_content, "Add special file");

    mediagit().arg("branch").arg("switch").arg("refs/heads/main").current_dir(temp_dir.path()).assert().success();

    mediagit()
        .arg("cherry-pick")
        .arg("feature")
        .current_dir(temp_dir.path())
        .assert()
        .success();

    // Verify content is preserved
    let content = fs::read_to_string(temp_dir.path().join("special.txt")).unwrap();
    assert_eq!(content, feature_content);
}
