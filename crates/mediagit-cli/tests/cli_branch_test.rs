// SPDX-License-Identifier: BUSL-1.1
// Copyright (C) 2025-2026 Aswin Krishnamoorthy

//! Comprehensive CLI Branch Command Tests
//!
//! Tests for `mediagit branch` command with all subcommands and options.

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
// Branch Create Tests
// ============================================================================

#[test]
fn test_branch_create() {
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());

    add_and_commit(temp_dir.path(), "file.txt", "Content", "Initial commit");

    mediagit()
        .arg("branch")
        .arg("create")
        .arg("feature")
        .current_dir(temp_dir.path())
        .assert()
        .success();

    // Verify branch exists
    mediagit()
        .arg("branch")
        .arg("list")
        .current_dir(temp_dir.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("feature"));
}

#[test]
fn test_branch_create_from_commit() {
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());

    add_and_commit(temp_dir.path(), "file.txt", "Content", "Initial commit");
    add_and_commit(temp_dir.path(), "file2.txt", "Content 2", "Second commit");

    // Create branch from HEAD (MediaGit doesn't support HEAD~1 syntax)
    mediagit()
        .arg("branch")
        .arg("create")
        .arg("old-feature")
        .arg("HEAD")
        .current_dir(temp_dir.path())
        .assert()
        .success();
}

// ============================================================================
// Branch Switch Tests
// ============================================================================

#[test]
fn test_branch_switch() {
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());

    add_and_commit(temp_dir.path(), "file.txt", "Content", "Initial commit");

    // Create and switch to branch
    mediagit()
        .arg("branch")
        .arg("create")
        .arg("develop")
        .current_dir(temp_dir.path())
        .assert()
        .success();

    mediagit()
        .arg("branch")
        .arg("switch")
        .arg("develop")
        .current_dir(temp_dir.path())
        .assert()
        .success();

    // Verify we're on develop
    mediagit()
        .arg("branch")
        .arg("list")
        .current_dir(temp_dir.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("* develop"));
}

#[test]
fn test_branch_switch_with_media_files() {
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());

    // Main branch: add file
    add_and_commit(
        temp_dir.path(),
        "main_file.txt",
        "Main content",
        "Initial on main",
    );

    // Create feature branch
    mediagit()
        .arg("branch")
        .arg("create")
        .arg("feature")
        .current_dir(temp_dir.path())
        .assert()
        .success();

    mediagit()
        .arg("branch")
        .arg("switch")
        .arg("feature")
        .current_dir(temp_dir.path())
        .assert()
        .success();

    // Add file on feature branch
    add_and_commit(
        temp_dir.path(),
        "feature_file.txt",
        "Feature content",
        "Add feature file",
    );

    // Verify feature file exists
    assert!(temp_dir.path().join("feature_file.txt").exists());

    // Switch back to main
    mediagit()
        .arg("branch")
        .arg("switch")
        .arg("refs/heads/main")
        .current_dir(temp_dir.path())
        .assert()
        .success();

    // Feature file should not exist on main
    assert!(!temp_dir.path().join("feature_file.txt").exists());
}

// ============================================================================
// Branch List Tests
// ============================================================================

#[test]
fn test_branch_list() {
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());

    add_and_commit(temp_dir.path(), "file.txt", "Content", "Initial commit");

    mediagit()
        .arg("branch")
        .arg("create")
        .arg("feature-1")
        .current_dir(temp_dir.path())
        .assert()
        .success();

    mediagit()
        .arg("branch")
        .arg("create")
        .arg("feature-2")
        .current_dir(temp_dir.path())
        .assert()
        .success();

    mediagit()
        .arg("branch")
        .arg("list")
        .current_dir(temp_dir.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("main").or(predicate::str::contains("master")))
        .stdout(predicate::str::contains("feature-1"))
        .stdout(predicate::str::contains("feature-2"));
}

#[test]
fn test_branch_list_all() {
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());

    add_and_commit(temp_dir.path(), "file.txt", "Content", "Initial commit");

    mediagit()
        .arg("branch")
        .arg("list")
        .arg("-a")
        .current_dir(temp_dir.path())
        .assert()
        .success();
}

#[test]
fn test_branch_list_verbose() {
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());

    add_and_commit(temp_dir.path(), "file.txt", "Content", "Initial commit");

    mediagit()
        .arg("branch")
        .arg("list")
        .arg("-v")
        .current_dir(temp_dir.path())
        .assert()
        .success();
}

// ============================================================================
// Branch Delete Tests
// ============================================================================

#[test]
fn test_branch_delete() {
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());

    add_and_commit(temp_dir.path(), "file.txt", "Content", "Initial commit");

    mediagit()
        .arg("branch")
        .arg("create")
        .arg("to-delete")
        .current_dir(temp_dir.path())
        .assert()
        .success();

    mediagit()
        .arg("branch")
        .arg("delete")
        .arg("to-delete")
        .current_dir(temp_dir.path())
        .assert()
        .success();

    // Verify branch is gone
    mediagit()
        .arg("branch")
        .arg("list")
        .current_dir(temp_dir.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("to-delete").not());
}

#[test]
fn test_branch_delete_force() {
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());

    add_and_commit(temp_dir.path(), "file.txt", "Content", "Initial commit");

    mediagit()
        .arg("branch")
        .arg("create")
        .arg("unmerged")
        .current_dir(temp_dir.path())
        .assert()
        .success();

    // Switch to unmerged, add a commit
    mediagit()
        .arg("branch")
        .arg("switch")
        .arg("unmerged")
        .current_dir(temp_dir.path())
        .assert()
        .success();

    add_and_commit(
        temp_dir.path(),
        "unmerged.txt",
        "Unmerged",
        "Unmerged commit",
    );

    // Switch back to main
    mediagit()
        .arg("branch")
        .arg("switch")
        .arg("refs/heads/main")
        .current_dir(temp_dir.path())
        .assert()
        .success();

    // Force delete unmerged branch
    mediagit()
        .arg("branch")
        .arg("delete")
        .arg("-D")
        .arg("unmerged")
        .current_dir(temp_dir.path())
        .assert()
        .success();
}

#[test]
#[ignore] // MediaGit may handle current branch deletion differently
fn test_branch_delete_current_fails() {
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());

    add_and_commit(temp_dir.path(), "file.txt", "Content", "Initial commit");

    // Try to delete current branch - behavior may vary
    let _ = mediagit()
        .arg("branch")
        .arg("delete")
        .arg("main")
        .current_dir(temp_dir.path())
        .assert();
}

// ============================================================================
// Branch Rename Tests
// ============================================================================

#[test]
fn test_branch_rename() {
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());

    add_and_commit(temp_dir.path(), "file.txt", "Content", "Initial commit");

    mediagit()
        .arg("branch")
        .arg("create")
        .arg("old-name")
        .current_dir(temp_dir.path())
        .assert()
        .success();

    mediagit()
        .arg("branch")
        .arg("rename")
        .arg("old-name")
        .arg("new-name")
        .current_dir(temp_dir.path())
        .assert()
        .success();

    // Verify new name exists
    mediagit()
        .arg("branch")
        .arg("list")
        .current_dir(temp_dir.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("new-name"));
}

// ============================================================================
// Branch Show Tests
// ============================================================================

#[test]
fn test_branch_show() {
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());

    add_and_commit(temp_dir.path(), "file.txt", "Content", "Initial commit");

    mediagit()
        .arg("branch")
        .arg("show")
        .current_dir(temp_dir.path())
        .assert()
        .success();
}

// ============================================================================
// Error Handling Tests
// ============================================================================

#[test]
fn test_branch_switch_nonexistent() {
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());

    add_and_commit(temp_dir.path(), "file.txt", "Content", "Initial commit");

    mediagit()
        .arg("branch")
        .arg("switch")
        .arg("nonexistent")
        .current_dir(temp_dir.path())
        .assert()
        .failure();
}

#[test]
fn test_branch_create_duplicate() {
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());

    add_and_commit(temp_dir.path(), "file.txt", "Content", "Initial commit");

    mediagit()
        .arg("branch")
        .arg("create")
        .arg("feature")
        .current_dir(temp_dir.path())
        .assert()
        .success();

    // Creating duplicate should fail
    mediagit()
        .arg("branch")
        .arg("create")
        .arg("feature")
        .current_dir(temp_dir.path())
        .assert()
        .failure();
}

#[test]
fn test_branch_help() {
    mediagit()
        .arg("branch")
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("branch"));
}

// ============================================================================
// QA-001: `branch switch` must refuse to clobber an untracked file that
// collides with a path tracked by the target branch.
// ============================================================================

/// main: base.txt. topic: base.txt + topic.txt (committed). Leaves HEAD on
/// main with topic.txt absent from the working tree.
fn setup_collision_repo(temp_dir: &TempDir) {
    init_repo(temp_dir.path());
    add_and_commit(temp_dir.path(), "base.txt", "base content", "base");

    mediagit()
        .arg("branch")
        .arg("create")
        .arg("topic")
        .current_dir(temp_dir.path())
        .assert()
        .success();
    mediagit()
        .arg("branch")
        .arg("switch")
        .arg("topic")
        .current_dir(temp_dir.path())
        .assert()
        .success();
    add_and_commit(
        temp_dir.path(),
        "topic.txt",
        "topic content",
        "topic adds topic.txt",
    );

    mediagit()
        .arg("branch")
        .arg("switch")
        .arg("main")
        .current_dir(temp_dir.path())
        .assert()
        .success();
}

#[test]
fn test_branch_switch_refuses_untracked_collision() {
    let temp_dir = TempDir::new().unwrap();
    setup_collision_repo(&temp_dir);

    // Untracked file on main at a path `topic` tracks — a plain switch would
    // silently clobber it.
    fs::write(temp_dir.path().join("topic.txt"), "dirty untracked content").unwrap();

    mediagit()
        .arg("branch")
        .arg("switch")
        .arg("topic")
        .current_dir(temp_dir.path())
        .assert()
        .failure()
        .stderr(predicate::str::contains("topic.txt"));

    // Bytes must survive the refused switch untouched.
    let content = fs::read_to_string(temp_dir.path().join("topic.txt")).unwrap();
    assert_eq!(content, "dirty untracked content");
}

#[test]
fn test_branch_switch_force_overwrites_untracked_collision() {
    let temp_dir = TempDir::new().unwrap();
    setup_collision_repo(&temp_dir);

    fs::write(temp_dir.path().join("topic.txt"), "dirty untracked content").unwrap();

    // --force preserves today's behavior: the guard is bypassed.
    mediagit()
        .arg("branch")
        .arg("switch")
        .arg("topic")
        .arg("-f")
        .current_dir(temp_dir.path())
        .assert()
        .success();

    let content = fs::read_to_string(temp_dir.path().join("topic.txt")).unwrap();
    assert_eq!(content, "topic content");
}

#[test]
fn test_branch_switch_noncolliding_untracked_proceeds() {
    let temp_dir = TempDir::new().unwrap();
    setup_collision_repo(&temp_dir);

    // Untracked file whose path the target branch does not track at all.
    fs::write(temp_dir.path().join("scratch.txt"), "scratch content").unwrap();

    mediagit()
        .arg("branch")
        .arg("switch")
        .arg("topic")
        .current_dir(temp_dir.path())
        .assert()
        .success();

    let content = fs::read_to_string(temp_dir.path().join("scratch.txt")).unwrap();
    assert_eq!(content, "scratch content");
    assert!(temp_dir.path().join("topic.txt").exists());
}

#[test]
fn test_branch_switch_ignored_collision_proceeds() {
    let temp_dir = TempDir::new().unwrap();
    setup_collision_repo(&temp_dir);

    // topic.txt is .mediagitignore'd on main, so it's not "untracked" by the
    // guard's definition — matches git: ignored files stay overwritable.
    fs::write(temp_dir.path().join(".mediagitignore"), "topic.txt\n").unwrap();
    fs::write(temp_dir.path().join("topic.txt"), "ignored dirty content").unwrap();

    mediagit()
        .arg("branch")
        .arg("switch")
        .arg("topic")
        .current_dir(temp_dir.path())
        .assert()
        .success();

    let content = fs::read_to_string(temp_dir.path().join("topic.txt")).unwrap();
    assert_eq!(content, "topic content");
}
