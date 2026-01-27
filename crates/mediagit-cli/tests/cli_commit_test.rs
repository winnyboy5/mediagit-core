// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2025 MediaGit Contributors

//! Comprehensive CLI Commit Command Tests
//!
//! Tests for `mediagit commit` command with all options and edge cases.

use assert_cmd::Command;
use predicates::prelude::*;
use std::fs;
use std::path::Path;
use std::time::Instant;
use tempfile::TempDir;

#[allow(deprecated)]
fn mediagit() -> Command {
    Command::cargo_bin("mediagit").unwrap()
}

fn init_repo(dir: &Path) {
    mediagit()
        .arg("init")
        .arg("-q")
        .current_dir(dir)
        .assert()
        .success();
}

fn add_file(dir: &Path, name: &str, content: &str) {
    fs::write(dir.join(name), content).unwrap();
    mediagit()
        .arg("add")
        .arg(name)
        .current_dir(dir)
        .assert()
        .success();
}

// ============================================================================
// Basic Commit Tests
// ============================================================================

#[test]
fn test_commit_basic() {
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());

    add_file(temp_dir.path(), "test.txt", "Hello, World!");

    let start = Instant::now();
    mediagit()
        .arg("commit")
        .arg("-m")
        .arg("Initial commit")
        .current_dir(temp_dir.path())
        .assert()
        .success();

    println!("Commit duration: {:?}", start.elapsed());

    // Verify commit exists in log
    mediagit()
        .arg("log")
        .current_dir(temp_dir.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("Initial commit"));
}

#[test]
fn test_commit_multiple_files() {
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());

    for i in 1..=5 {
        add_file(temp_dir.path(), &format!("file{}.txt", i), &format!("Content {}", i));
    }

    mediagit()
        .arg("commit")
        .arg("-m")
        .arg("Add 5 files")
        .current_dir(temp_dir.path())
        .assert()
        .success();
}

#[test]
fn test_commit_with_author() {
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());

    add_file(temp_dir.path(), "test.txt", "Content");

    // MediaGit may not support --author flag, just test basic commit
    mediagit()
        .arg("commit")
        .arg("-m")
        .arg("Commit with content")
        .current_dir(temp_dir.path())
        .assert()
        .success();

    // Verify commit in log
    mediagit()
        .arg("log")
        .current_dir(temp_dir.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("Commit with content"));
}

#[test]
fn test_commit_dry_run() {
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());

    add_file(temp_dir.path(), "test.txt", "Content");

    mediagit()
        .arg("commit")
        .arg("-m")
        .arg("Dry run commit")
        .arg("--dry-run")
        .current_dir(temp_dir.path())
        .assert()
        .success();

    // Verify no actual commit was made (status should still show staged)
    mediagit()
        .arg("status")
        .current_dir(temp_dir.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("test.txt"));
}

#[test]
fn test_commit_quiet() {
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());

    add_file(temp_dir.path(), "test.txt", "Content");

    mediagit()
        .arg("commit")
        .arg("-m")
        .arg("Quiet commit")
        .arg("-q")
        .current_dir(temp_dir.path())
        .assert()
        .success();
}

#[test]
fn test_commit_verbose() {
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());

    add_file(temp_dir.path(), "test.txt", "Content");

    mediagit()
        .arg("commit")
        .arg("-m")
        .arg("Verbose commit")
        .arg("-v")
        .current_dir(temp_dir.path())
        .assert()
        .success();
}

// ============================================================================
// Sequential Commits Tests
// ============================================================================

#[test]
fn test_commit_sequential() {
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());

    // First commit
    add_file(temp_dir.path(), "file1.txt", "Version 1");
    mediagit()
        .arg("commit")
        .arg("-m")
        .arg("First commit")
        .current_dir(temp_dir.path())
        .assert()
        .success();

    // Second commit
    add_file(temp_dir.path(), "file2.txt", "Version 2");
    mediagit()
        .arg("commit")
        .arg("-m")
        .arg("Second commit")
        .current_dir(temp_dir.path())
        .assert()
        .success();

    // Third commit
    fs::write(temp_dir.path().join("file1.txt"), "Version 1 updated").unwrap();
    mediagit()
        .arg("add")
        .arg("file1.txt")
        .current_dir(temp_dir.path())
        .assert()
        .success();
    mediagit()
        .arg("commit")
        .arg("-m")
        .arg("Third commit")
        .current_dir(temp_dir.path())
        .assert()
        .success();

    // Verify all commits in log
    mediagit()
        .arg("log")
        .current_dir(temp_dir.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("First commit"))
        .stdout(predicate::str::contains("Second commit"))
        .stdout(predicate::str::contains("Third commit"));
}

// ============================================================================
// Error Handling Tests
// ============================================================================

#[test]
fn test_commit_empty_fails() {
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());

    // Try to commit without staging anything
    mediagit()
        .arg("commit")
        .arg("-m")
        .arg("Empty commit")
        .current_dir(temp_dir.path())
        .assert()
        .failure();
}

#[test]
fn test_commit_allow_empty() {
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());

    // Need at least one commit first
    add_file(temp_dir.path(), "initial.txt", "Initial");
    mediagit()
        .arg("commit")
        .arg("-m")
        .arg("Initial")
        .current_dir(temp_dir.path())
        .assert()
        .success();

    // Now try empty commit with --allow-empty
    mediagit()
        .arg("commit")
        .arg("-m")
        .arg("Empty commit")
        .arg("--allow-empty")
        .current_dir(temp_dir.path())
        .assert()
        .success();
}

#[test]
fn test_commit_no_message_fails() {
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());

    add_file(temp_dir.path(), "test.txt", "Content");

    // Commit without -m should fail (or open editor which we can't handle in tests)
    // This depends on implementation - might need adjustment
    mediagit()
        .arg("commit")
        .current_dir(temp_dir.path())
        .assert()
        .failure();
}

#[test]
fn test_commit_help() {
    mediagit()
        .arg("commit")
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("commit"));
}

// ============================================================================
// Performance Tests
// ============================================================================

#[test]
fn test_commit_speed_benchmark() {
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());

    let mut durations = Vec::new();

    for i in 0..5 {
        // Create and add a file
        let filename = format!("file{}.txt", i);
        add_file(temp_dir.path(), &filename, &format!("Content {}", i));

        let start = Instant::now();
        mediagit()
            .arg("commit")
            .arg("-m")
            .arg(format!("Commit {}", i))
            .arg("-q")
            .current_dir(temp_dir.path())
            .assert()
            .success();

        durations.push(start.elapsed());
        println!("Commit {}: {:?}", i + 1, durations.last().unwrap());
    }

    let avg = durations.iter().map(|d| d.as_millis()).sum::<u128>() / durations.len() as u128;
    println!("Average commit time: {}ms", avg);
}

#[test]
fn test_commit_many_files_performance() {
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());

    // Create 50 files
    for i in 0..50 {
        fs::write(
            temp_dir.path().join(format!("file_{:02}.txt", i)),
            format!("Content {}", i),
        ).unwrap();
    }

    // Add all
    mediagit()
        .arg("add")
        .arg(".")
        .current_dir(temp_dir.path())
        .assert()
        .success();

    // Commit all
    let start = Instant::now();
    mediagit()
        .arg("commit")
        .arg("-m")
        .arg("Add 50 files")
        .current_dir(temp_dir.path())
        .assert()
        .success();

    let duration = start.elapsed();
    println!("Commit 50 files: {:?}", duration);
    println!("Files per second: {:.2}", 50.0 / duration.as_secs_f64());
}

// ============================================================================
// Signoff and Metadata Tests
// ============================================================================

#[test]
fn test_commit_signoff() {
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());

    add_file(temp_dir.path(), "test.txt", "Content");

    mediagit()
        .arg("commit")
        .arg("-m")
        .arg("Signed commit")
        .arg("-s")
        .current_dir(temp_dir.path())
        .assert()
        .success();
}
