// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2025 MediaGit Contributors

//! CLI command integration tests
//!
//! Tests all MediaGit CLI commands end-to-end including:
//! - Command parsing and validation
//! - Argument validation
//! - Error message quality
//! - Command execution and output

use assert_cmd::Command;
use predicates::prelude::*;
use tempfile::TempDir;

/// Helper to create a test command
fn mediagit_cmd() -> Command {
    Command::cargo_bin("mediagit").unwrap()
}

/// Helper to initialize a test repository
fn init_test_repo() -> TempDir {
    let temp_dir = TempDir::new().unwrap();
    mediagit_cmd()
        .current_dir(&temp_dir)
        .arg("init")
        .assert()
        .success();
    temp_dir
}

#[test]
fn test_cli_no_args() {
    mediagit_cmd()
        .assert()
        .failure()
        .stderr(predicate::str::contains("Usage"));
}

#[test]
fn test_cli_help() {
    mediagit_cmd()
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("MediaGit"))
        .stdout(predicate::str::contains("USAGE"));
}

#[test]
fn test_cli_version() {
    mediagit_cmd()
        .arg("--version")
        .assert()
        .success()
        .stdout(predicate::str::contains("mediagit"));
}

// ============================================================================
// INIT Command Tests
// ============================================================================

#[test]
fn test_init_creates_repository() {
    let temp_dir = TempDir::new().unwrap();

    mediagit_cmd()
        .current_dir(&temp_dir)
        .arg("init")
        .assert()
        .success()
        .stdout(predicate::str::contains("Initialized"));

    // Verify .mediagit directory was created
    assert!(temp_dir.path().join(".mediagit").exists());
    assert!(temp_dir.path().join(".mediagit/objects").exists());
    assert!(temp_dir.path().join(".mediagit/refs").exists());
}

#[test]
fn test_init_already_initialized() {
    let temp_dir = TempDir::new().unwrap();

    // Initialize once
    mediagit_cmd()
        .current_dir(&temp_dir)
        .arg("init")
        .assert()
        .success();

    // Try to initialize again
    mediagit_cmd()
        .current_dir(&temp_dir)
        .arg("init")
        .assert()
        .failure()
        .stderr(predicate::str::contains("already initialized"));
}

#[test]
fn test_init_with_custom_path() {
    let temp_dir = TempDir::new().unwrap();
    let custom_path = temp_dir.path().join("custom-repo");

    mediagit_cmd()
        .arg("init")
        .arg(&custom_path)
        .assert()
        .success();

    assert!(custom_path.join(".mediagit").exists());
}

// ============================================================================
// ADD Command Tests
// ============================================================================

#[test]
fn test_add_requires_init() {
    let temp_dir = TempDir::new().unwrap();

    mediagit_cmd()
        .current_dir(&temp_dir)
        .arg("add")
        .arg("file.txt")
        .assert()
        .failure()
        .stderr(predicate::str::contains("not a mediagit repository"));
}

#[test]
fn test_add_nonexistent_file() {
    let repo = init_test_repo();

    mediagit_cmd()
        .current_dir(&repo)
        .arg("add")
        .arg("nonexistent.txt")
        .assert()
        .failure()
        .stderr(predicate::str::contains("not found").or(predicate::str::contains("does not exist")));
}

#[test]
fn test_add_file_success() {
    let repo = init_test_repo();
    let test_file = repo.path().join("test.txt");
    std::fs::write(&test_file, b"test content").unwrap();

    mediagit_cmd()
        .current_dir(&repo)
        .arg("add")
        .arg("test.txt")
        .assert()
        .success()
        .stdout(predicate::str::contains("Added").or(predicate::str::contains("Staged")));
}

#[test]
fn test_add_multiple_files() {
    let repo = init_test_repo();
    std::fs::write(repo.path().join("file1.txt"), b"content 1").unwrap();
    std::fs::write(repo.path().join("file2.txt"), b"content 2").unwrap();

    mediagit_cmd()
        .current_dir(&repo)
        .arg("add")
        .arg("file1.txt")
        .arg("file2.txt")
        .assert()
        .success();
}

#[test]
fn test_add_directory() {
    let repo = init_test_repo();
    let dir = repo.path().join("test_dir");
    std::fs::create_dir(&dir).unwrap();
    std::fs::write(dir.join("file.txt"), b"content").unwrap();

    mediagit_cmd()
        .current_dir(&repo)
        .arg("add")
        .arg("test_dir/")
        .assert()
        .success();
}

// ============================================================================
// STATUS Command Tests
// ============================================================================

#[test]
fn test_status_requires_init() {
    let temp_dir = TempDir::new().unwrap();

    mediagit_cmd()
        .current_dir(&temp_dir)
        .arg("status")
        .assert()
        .failure()
        .stderr(predicate::str::contains("not a mediagit repository"));
}

#[test]
fn test_status_clean_repo() {
    let repo = init_test_repo();

    mediagit_cmd()
        .current_dir(&repo)
        .arg("status")
        .assert()
        .success()
        .stdout(predicate::str::contains("clean").or(predicate::str::contains("nothing to commit")));
}

#[test]
fn test_status_shows_untracked() {
    let repo = init_test_repo();
    std::fs::write(repo.path().join("untracked.txt"), b"content").unwrap();

    mediagit_cmd()
        .current_dir(&repo)
        .arg("status")
        .assert()
        .success()
        .stdout(predicate::str::contains("untracked").or(predicate::str::contains("Untracked")));
}

// ============================================================================
// COMMIT Command Tests
// ============================================================================

#[test]
fn test_commit_requires_init() {
    let temp_dir = TempDir::new().unwrap();

    mediagit_cmd()
        .current_dir(&temp_dir)
        .arg("commit")
        .arg("-m")
        .arg("test commit")
        .assert()
        .failure();
}

#[test]
fn test_commit_requires_message() {
    let repo = init_test_repo();

    mediagit_cmd()
        .current_dir(&repo)
        .arg("commit")
        .assert()
        .failure()
        .stderr(predicate::str::contains("message").or(predicate::str::contains("-m")));
}

#[test]
fn test_commit_nothing_to_commit() {
    let repo = init_test_repo();

    mediagit_cmd()
        .current_dir(&repo)
        .arg("commit")
        .arg("-m")
        .arg("test")
        .assert()
        .failure()
        .stderr(predicate::str::contains("nothing to commit").or(predicate::str::contains("no changes")));
}

#[test]
fn test_commit_success() {
    let repo = init_test_repo();
    let test_file = repo.path().join("test.txt");
    std::fs::write(&test_file, b"test content").unwrap();

    // Add file first
    mediagit_cmd()
        .current_dir(&repo)
        .arg("add")
        .arg("test.txt")
        .assert()
        .success();

    // Commit
    mediagit_cmd()
        .current_dir(&repo)
        .arg("commit")
        .arg("-m")
        .arg("Initial commit")
        .assert()
        .success()
        .stdout(predicate::str::contains("commit").or(predicate::str::contains("Created")));
}

// ============================================================================
// BRANCH Command Tests
// ============================================================================

#[test]
fn test_branch_list() {
    let repo = init_test_repo();

    mediagit_cmd()
        .current_dir(&repo)
        .arg("branch")
        .arg("list")
        .assert()
        .success();
}

#[test]
fn test_branch_create() {
    let repo = init_test_repo();

    mediagit_cmd()
        .current_dir(&repo)
        .arg("branch")
        .arg("create")
        .arg("feature-branch")
        .assert()
        .success();
}

#[test]
fn test_branch_create_invalid_name() {
    let repo = init_test_repo();

    mediagit_cmd()
        .current_dir(&repo)
        .arg("branch")
        .arg("create")
        .arg("invalid name with spaces")
        .assert()
        .failure()
        .stderr(predicate::str::contains("invalid").or(predicate::str::contains("branch name")));
}

#[test]
fn test_branch_delete() {
    let repo = init_test_repo();

    // Create branch first
    mediagit_cmd()
        .current_dir(&repo)
        .arg("branch")
        .arg("create")
        .arg("temp-branch")
        .assert()
        .success();

    // Delete it
    mediagit_cmd()
        .current_dir(&repo)
        .arg("branch")
        .arg("delete")
        .arg("temp-branch")
        .assert()
        .success();
}

#[test]
fn test_branch_switch() {
    let repo = init_test_repo();

    // Create branch
    mediagit_cmd()
        .current_dir(&repo)
        .arg("branch")
        .arg("create")
        .arg("new-branch")
        .assert()
        .success();

    // Switch to it
    mediagit_cmd()
        .current_dir(&repo)
        .arg("branch")
        .arg("switch")
        .arg("new-branch")
        .assert()
        .success();
}

// ============================================================================
// LOG Command Tests
// ============================================================================

#[test]
fn test_log_empty_repo() {
    let repo = init_test_repo();

    mediagit_cmd()
        .current_dir(&repo)
        .arg("log")
        .assert()
        .success()
        .stdout(predicate::str::contains("no commits").or(predicate::str::is_empty()));
}

// ============================================================================
// DIFF Command Tests
// ============================================================================

#[test]
fn test_diff_no_changes() {
    let repo = init_test_repo();

    mediagit_cmd()
        .current_dir(&repo)
        .arg("diff")
        .assert()
        .success();
}

// ============================================================================
// VERIFY Command Tests
// ============================================================================

#[test]
fn test_verify_clean_repo() {
    let repo = init_test_repo();

    mediagit_cmd()
        .current_dir(&repo)
        .arg("verify")
        .assert()
        .success()
        .stdout(predicate::str::contains("OK").or(predicate::str::contains("verified")));
}

// ============================================================================
// FSCK Command Tests
// ============================================================================

#[test]
fn test_fsck_clean_repo() {
    let repo = init_test_repo();

    mediagit_cmd()
        .current_dir(&repo)
        .arg("fsck")
        .assert()
        .success();
}

// ============================================================================
// STATS Command Tests
// ============================================================================

#[test]
fn test_stats() {
    let repo = init_test_repo();

    mediagit_cmd()
        .current_dir(&repo)
        .arg("stats")
        .assert()
        .success()
        .stdout(predicate::str::contains("objects").or(predicate::str::contains("size")));
}

// ============================================================================
// Error Handling Tests
// ============================================================================

#[test]
fn test_invalid_command() {
    mediagit_cmd()
        .arg("invalid-command")
        .assert()
        .failure()
        .stderr(predicate::str::contains("unrecognized").or(predicate::str::contains("invalid")));
}

#[test]
fn test_help_for_subcommand() {
    mediagit_cmd()
        .arg("init")
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("init"));
}

#[test]
fn test_invalid_flag() {
    mediagit_cmd()
        .arg("--invalid-flag")
        .assert()
        .failure()
        .stderr(predicate::str::contains("unexpected").or(predicate::str::contains("unrecognized")));
}

// ============================================================================
// Output Format Tests
// ============================================================================

#[test]
fn test_status_colored_output() {
    let repo = init_test_repo();

    // With colors (default if TTY)
    mediagit_cmd()
        .current_dir(&repo)
        .arg("status")
        .assert()
        .success();
}

#[test]
fn test_status_no_color() {
    let repo = init_test_repo();

    mediagit_cmd()
        .current_dir(&repo)
        .arg("status")
        .arg("--no-color")
        .assert()
        .success();
}

// ============================================================================
// Complex Workflow Tests
// ============================================================================

#[test]
fn test_workflow_init_add_commit() {
    let repo = init_test_repo();

    // Create file
    std::fs::write(repo.path().join("file.txt"), b"content").unwrap();

    // Add file
    mediagit_cmd()
        .current_dir(&repo)
        .arg("add")
        .arg("file.txt")
        .assert()
        .success();

    // Check status
    mediagit_cmd()
        .current_dir(&repo)
        .arg("status")
        .assert()
        .success()
        .stdout(predicate::str::contains("staged").or(predicate::str::contains("Changes")));

    // Commit
    mediagit_cmd()
        .current_dir(&repo)
        .arg("commit")
        .arg("-m")
        .arg("Add file")
        .assert()
        .success();

    // Verify clean status
    mediagit_cmd()
        .current_dir(&repo)
        .arg("status")
        .assert()
        .success()
        .stdout(predicate::str::contains("clean").or(predicate::str::contains("nothing")));
}

#[test]
fn test_workflow_branch_create_switch() {
    let repo = init_test_repo();

    // Create and commit initial file
    std::fs::write(repo.path().join("main.txt"), b"main content").unwrap();
    mediagit_cmd()
        .current_dir(&repo)
        .args(&["add", "main.txt"])
        .assert()
        .success();
    mediagit_cmd()
        .current_dir(&repo)
        .args(&["commit", "-m", "Initial commit"])
        .assert()
        .success();

    // Create feature branch
    mediagit_cmd()
        .current_dir(&repo)
        .args(&["branch", "create", "feature"])
        .assert()
        .success();

    // Switch to feature branch
    mediagit_cmd()
        .current_dir(&repo)
        .args(&["branch", "switch", "feature"])
        .assert()
        .success();

    // Create feature file
    std::fs::write(repo.path().join("feature.txt"), b"feature content").unwrap();
    mediagit_cmd()
        .current_dir(&repo)
        .args(&["add", "feature.txt"])
        .assert()
        .success();
    mediagit_cmd()
        .current_dir(&repo)
        .args(&["commit", "-m", "Add feature"])
        .assert()
        .success();
}
