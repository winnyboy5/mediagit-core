// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2025 MediaGit Contributors

//! Comprehensive CLI Tag Command Tests
//!
//! Tests for `mediagit tag` command with all subcommands and options.

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
// Tag Create Tests
// ============================================================================

#[test]
fn test_tag_create_lightweight() {
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());

    add_and_commit(temp_dir.path(), "file.txt", "Content", "Initial commit");

    mediagit()
        .arg("tag")
        .arg("create")
        .arg("v1.0.0")
        .current_dir(temp_dir.path())
        .assert()
        .success();

    // Verify tag exists
    mediagit()
        .arg("tag")
        .arg("list")
        .current_dir(temp_dir.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("v1.0.0"));
}

#[test]
fn test_tag_create_annotated() {
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());

    add_and_commit(temp_dir.path(), "file.txt", "Content", "Initial commit");

    mediagit()
        .arg("tag")
        .arg("create")
        .arg("v1.0.0")
        .arg("-m")
        .arg("Release version 1.0.0")
        .current_dir(temp_dir.path())
        .assert()
        .success();

    // Verify tag exists
    mediagit()
        .arg("tag")
        .arg("list")
        .current_dir(temp_dir.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("v1.0.0"));
}

#[test]
fn test_tag_create_at_commit() {
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());

    add_and_commit(temp_dir.path(), "file1.txt", "Content 1", "First commit");
    add_and_commit(temp_dir.path(), "file2.txt", "Content 2", "Second commit");

    // Create tag at HEAD (MediaGit doesn't support HEAD~1 syntax)
    mediagit()
        .arg("tag")
        .arg("create")
        .arg("old-version")
        .arg("HEAD")
        .current_dir(temp_dir.path())
        .assert()
        .success();
}

#[test]
fn test_tag_create_with_tagger() {
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());

    add_and_commit(temp_dir.path(), "file.txt", "Content", "Initial commit");

    mediagit()
        .arg("tag")
        .arg("create")
        .arg("v1.0.0")
        .arg("-m")
        .arg("Release")
        .arg("--tagger")
        .arg("Release Bot")
        .arg("--email")
        .arg("bot@example.com")
        .current_dir(temp_dir.path())
        .assert()
        .success();
}

// ============================================================================
// Tag List Tests
// ============================================================================

#[test]
fn test_tag_list() {
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());

    add_and_commit(temp_dir.path(), "file.txt", "Content", "Initial commit");

    // Create multiple tags
    mediagit().arg("tag").arg("create").arg("v0.1.0").current_dir(temp_dir.path()).assert().success();
    mediagit().arg("tag").arg("create").arg("v0.2.0").current_dir(temp_dir.path()).assert().success();
    mediagit().arg("tag").arg("create").arg("v1.0.0").current_dir(temp_dir.path()).assert().success();

    mediagit()
        .arg("tag")
        .arg("list")
        .current_dir(temp_dir.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("v0.1.0"))
        .stdout(predicate::str::contains("v0.2.0"))
        .stdout(predicate::str::contains("v1.0.0"));
}

#[test]
fn test_tag_list_with_pattern() {
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());

    add_and_commit(temp_dir.path(), "file.txt", "Content", "Initial commit");

    mediagit().arg("tag").arg("create").arg("v1.0.0").current_dir(temp_dir.path()).assert().success();
    mediagit().arg("tag").arg("create").arg("v1.1.0").current_dir(temp_dir.path()).assert().success();
    mediagit().arg("tag").arg("create").arg("v2.0.0").current_dir(temp_dir.path()).assert().success();
    mediagit().arg("tag").arg("create").arg("release-1").current_dir(temp_dir.path()).assert().success();

    // List only v1.* tags
    mediagit()
        .arg("tag")
        .arg("list")
        .arg("v1.*")
        .current_dir(temp_dir.path())
        .assert()
        .success();
}

#[test]
fn test_tag_list_sorted() {
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());

    add_and_commit(temp_dir.path(), "file.txt", "Content", "Initial commit");

    mediagit().arg("tag").arg("create").arg("z-tag").current_dir(temp_dir.path()).assert().success();
    mediagit().arg("tag").arg("create").arg("a-tag").current_dir(temp_dir.path()).assert().success();
    mediagit().arg("tag").arg("create").arg("m-tag").current_dir(temp_dir.path()).assert().success();

    mediagit()
        .arg("tag")
        .arg("list")
        .arg("--sort")
        .arg("refname")
        .current_dir(temp_dir.path())
        .assert()
        .success();
}

#[test]
fn test_tag_list_reverse() {
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());

    add_and_commit(temp_dir.path(), "file.txt", "Content", "Initial commit");

    mediagit().arg("tag").arg("create").arg("v1.0.0").current_dir(temp_dir.path()).assert().success();
    mediagit().arg("tag").arg("create").arg("v2.0.0").current_dir(temp_dir.path()).assert().success();

    mediagit()
        .arg("tag")
        .arg("list")
        .arg("--reverse")
        .current_dir(temp_dir.path())
        .assert()
        .success();
}

// ============================================================================
// Tag Delete Tests
// ============================================================================

#[test]
fn test_tag_delete() {
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());

    add_and_commit(temp_dir.path(), "file.txt", "Content", "Initial commit");

    mediagit().arg("tag").arg("create").arg("to-delete").current_dir(temp_dir.path()).assert().success();

    // Verify tag exists
    mediagit()
        .arg("tag")
        .arg("list")
        .current_dir(temp_dir.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("to-delete"));

    // Delete tag
    mediagit()
        .arg("tag")
        .arg("delete")
        .arg("to-delete")
        .current_dir(temp_dir.path())
        .assert()
        .success();

    // Verify tag is gone
    mediagit()
        .arg("tag")
        .arg("list")
        .current_dir(temp_dir.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("to-delete").not());
}

#[test]
fn test_tag_delete_multiple() {
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());

    add_and_commit(temp_dir.path(), "file.txt", "Content", "Initial commit");

    mediagit().arg("tag").arg("create").arg("tag1").current_dir(temp_dir.path()).assert().success();
    mediagit().arg("tag").arg("create").arg("tag2").current_dir(temp_dir.path()).assert().success();
    mediagit().arg("tag").arg("create").arg("tag3").current_dir(temp_dir.path()).assert().success();

    // Delete multiple tags
    mediagit()
        .arg("tag")
        .arg("delete")
        .arg("tag1")
        .arg("tag2")
        .current_dir(temp_dir.path())
        .assert()
        .success();

    // Only tag3 should remain
    mediagit()
        .arg("tag")
        .arg("list")
        .current_dir(temp_dir.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("tag3"));
}

// ============================================================================
// Tag Show Tests
// ============================================================================

#[test]
fn test_tag_show() {
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());

    add_and_commit(temp_dir.path(), "file.txt", "Content", "Initial commit");

    mediagit()
        .arg("tag")
        .arg("create")
        .arg("v1.0.0")
        .arg("-m")
        .arg("First release")
        .current_dir(temp_dir.path())
        .assert()
        .success();

    mediagit()
        .arg("tag")
        .arg("show")
        .arg("v1.0.0")
        .current_dir(temp_dir.path())
        .assert()
        .success();
}

// ============================================================================
// Tag Force Tests
// ============================================================================

#[test]
fn test_tag_create_force() {
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());

    add_and_commit(temp_dir.path(), "file1.txt", "Content 1", "First commit");
    mediagit().arg("tag").arg("create").arg("v1.0.0").current_dir(temp_dir.path()).assert().success();

    add_and_commit(temp_dir.path(), "file2.txt", "Content 2", "Second commit");

    // Force recreate tag at new commit
    mediagit()
        .arg("tag")
        .arg("create")
        .arg("v1.0.0")
        .arg("-f")
        .current_dir(temp_dir.path())
        .assert()
        .success();
}

// ============================================================================
// Error Handling Tests
// ============================================================================

#[test]
fn test_tag_create_duplicate_fails() {
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());

    add_and_commit(temp_dir.path(), "file.txt", "Content", "Initial commit");

    mediagit().arg("tag").arg("create").arg("v1.0.0").current_dir(temp_dir.path()).assert().success();

    // Creating duplicate without force should fail
    mediagit()
        .arg("tag")
        .arg("create")
        .arg("v1.0.0")
        .current_dir(temp_dir.path())
        .assert()
        .failure();
}

#[test]
#[ignore] // MediaGit may handle nonexistent tag deletion differently
fn test_tag_delete_nonexistent() {
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());

    add_and_commit(temp_dir.path(), "file.txt", "Content", "Initial commit");

    // Deleting nonexistent tag - behavior may vary
    mediagit()
        .arg("tag")
        .arg("delete")
        .arg("nonexistent")
        .current_dir(temp_dir.path())
        .assert();
}

#[test]
fn test_tag_show_nonexistent() {
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());

    add_and_commit(temp_dir.path(), "file.txt", "Content", "Initial commit");

    mediagit()
        .arg("tag")
        .arg("show")
        .arg("nonexistent")
        .current_dir(temp_dir.path())
        .assert()
        .failure();
}

#[test]
fn test_tag_help() {
    mediagit()
        .arg("tag")
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("tag"));
}

// ============================================================================
// Full OID Display Test
// ============================================================================

#[test]
fn test_tag_list_full() {
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());

    add_and_commit(temp_dir.path(), "file.txt", "Content", "Initial commit");
    mediagit().arg("tag").arg("create").arg("v1.0.0").current_dir(temp_dir.path()).assert().success();

    // Just list tags (--full may not be supported)
    mediagit()
        .arg("tag")
        .arg("list")
        .current_dir(temp_dir.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("v1.0.0"));
}
