// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2025 MediaGit Contributors

//! Comprehensive CLI Maintenance Command Tests
//!
//! Tests for `gc`, `fsck`, `verify`, and `stats` commands.

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
    mediagit().arg("init").arg("-q").current_dir(dir).assert().success();
}

fn add_and_commit(dir: &Path, name: &str, content: &str, message: &str) {
    fs::write(dir.join(name), content).unwrap();
    mediagit().arg("add").arg(name).current_dir(dir).assert().success();
    mediagit().arg("commit").arg("-m").arg(message).current_dir(dir).assert().success();
}

// ============================================================================
// GC Command Tests
// ============================================================================

#[test]
fn test_gc_basic() {
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());

    // Create some commits to have objects to GC
    for i in 1..=5 {
        add_and_commit(temp_dir.path(), &format!("file{}.txt", i), &format!("Content {}", i), &format!("Commit {}", i));
    }

    let start = Instant::now();
    mediagit()
        .arg("gc")
        .current_dir(temp_dir.path())
        .assert()
        .success();

    println!("GC duration: {:?}", start.elapsed());
}

#[test]
fn test_gc_aggressive() {
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());

    for i in 1..=3 {
        add_and_commit(temp_dir.path(), &format!("file{}.txt", i), &format!("Content {}", i), &format!("Commit {}", i));
    }

    mediagit()
        .arg("gc")
        .arg("--aggressive")
        .current_dir(temp_dir.path())
        .assert()
        .success();
}

#[test]
fn test_gc_dry_run() {
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());

    add_and_commit(temp_dir.path(), "file.txt", "Content", "Initial commit");

    mediagit()
        .arg("gc")
        .arg("--dry-run")
        .current_dir(temp_dir.path())
        .assert()
        .success();
}

#[test]
fn test_gc_auto() {
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());

    add_and_commit(temp_dir.path(), "file.txt", "Content", "Initial commit");

    mediagit()
        .arg("gc")
        .arg("--auto")
        .current_dir(temp_dir.path())
        .assert()
        .success();
}

#[test]
fn test_gc_prune() {
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());

    add_and_commit(temp_dir.path(), "file.txt", "Content", "Initial commit");

    mediagit()
        .arg("gc")
        .arg("--prune")
        .arg("7")
        .current_dir(temp_dir.path())
        .assert()
        .success();
}

#[test]
fn test_gc_quiet() {
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());

    add_and_commit(temp_dir.path(), "file.txt", "Content", "Initial commit");

    mediagit()
        .arg("gc")
        .arg("-q")
        .current_dir(temp_dir.path())
        .assert()
        .success();
}

#[test]
fn test_gc_verbose() {
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());

    add_and_commit(temp_dir.path(), "file.txt", "Content", "Initial commit");

    mediagit()
        .arg("gc")
        .arg("-v")
        .current_dir(temp_dir.path())
        .assert()
        .success();
}

// ============================================================================
// FSCK Command Tests
// ============================================================================

#[test]
fn test_fsck_basic() {
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());

    add_and_commit(temp_dir.path(), "file.txt", "Content", "Initial commit");

    let start = Instant::now();
    mediagit()
        .arg("fsck")
        .current_dir(temp_dir.path())
        .assert()
        .success();

    println!("FSCK duration: {:?}", start.elapsed());
}

#[test]
fn test_fsck_full() {
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());

    for i in 1..=3 {
        add_and_commit(temp_dir.path(), &format!("file{}.txt", i), &format!("Content {}", i), &format!("Commit {}", i));
    }

    mediagit()
        .arg("fsck")
        .arg("--full")
        .current_dir(temp_dir.path())
        .assert()
        .success();
}

#[test]
fn test_fsck_quick() {
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());

    add_and_commit(temp_dir.path(), "file.txt", "Content", "Initial commit");

    mediagit()
        .arg("fsck")
        .arg("--quick")
        .current_dir(temp_dir.path())
        .assert()
        .success();
}

#[test]
fn test_fsck_repair_dry_run() {
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());

    add_and_commit(temp_dir.path(), "file.txt", "Content", "Initial commit");

    mediagit()
        .arg("fsck")
        .arg("--repair")
        .arg("--dry-run")
        .current_dir(temp_dir.path())
        .assert()
        .success();
}

#[test]
fn test_fsck_verbose() {
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());

    add_and_commit(temp_dir.path(), "file.txt", "Content", "Initial commit");

    mediagit()
        .arg("fsck")
        .arg("-v")
        .current_dir(temp_dir.path())
        .assert()
        .success();
}

// ============================================================================
// Verify Command Tests
// ============================================================================

#[test]
fn test_verify_basic() {
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());

    add_and_commit(temp_dir.path(), "file.txt", "Content", "Initial commit");

    mediagit()
        .arg("verify")
        .current_dir(temp_dir.path())
        .assert()
        .success();
}

#[test]
fn test_verify_file_integrity() {
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());

    add_and_commit(temp_dir.path(), "file.txt", "Content", "Initial commit");

    mediagit()
        .arg("verify")
        .arg("--file-integrity")
        .current_dir(temp_dir.path())
        .assert()
        .success();
}

#[test]
fn test_verify_checksums() {
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());

    add_and_commit(temp_dir.path(), "file.txt", "Content", "Initial commit");

    mediagit()
        .arg("verify")
        .arg("--checksums")
        .current_dir(temp_dir.path())
        .assert()
        .success();
}

#[test]
fn test_verify_quick() {
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());

    add_and_commit(temp_dir.path(), "file.txt", "Content", "Initial commit");

    mediagit()
        .arg("verify")
        .arg("--quick")
        .current_dir(temp_dir.path())
        .assert()
        .success();
}

#[test]
fn test_verify_detailed() {
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());

    add_and_commit(temp_dir.path(), "file.txt", "Content", "Initial commit");

    mediagit()
        .arg("verify")
        .arg("--detailed")
        .current_dir(temp_dir.path())
        .assert()
        .success();
}

// ============================================================================
// Stats Command Tests
// ============================================================================

#[test]
fn test_stats_basic() {
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());

    add_and_commit(temp_dir.path(), "file.txt", "Content", "Initial commit");

    mediagit()
        .arg("stats")
        .current_dir(temp_dir.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("Repository Statistics").or(predicate::str::contains("Stats")));
}

#[test]
fn test_stats_all() {
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());

    for i in 1..=5 {
        add_and_commit(temp_dir.path(), &format!("file{}.txt", i), &format!("Content with longer text for commit {}", i), &format!("Commit {}", i));
    }

    mediagit()
        .arg("stats")
        .arg("--all")
        .current_dir(temp_dir.path())
        .assert()
        .success();
}

#[test]
fn test_stats_storage() {
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());

    add_and_commit(temp_dir.path(), "file.txt", "Content", "Initial commit");

    mediagit()
        .arg("stats")
        .arg("--storage")
        .current_dir(temp_dir.path())
        .assert()
        .success();
}

#[test]
fn test_stats_compression() {
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());

    // Add some content that can be compressed
    add_and_commit(temp_dir.path(), "text.txt", "This is some text that should compress well. ".repeat(100).as_str(), "Add text");

    mediagit()
        .arg("stats")
        .arg("--compression")
        .current_dir(temp_dir.path())
        .assert()
        .success();
}

#[test]
fn test_stats_branches() {
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());

    add_and_commit(temp_dir.path(), "file.txt", "Content", "Initial commit");

    // Create some branches
    mediagit().arg("branch").arg("create").arg("feature-1").current_dir(temp_dir.path()).assert().success();
    mediagit().arg("branch").arg("create").arg("feature-2").current_dir(temp_dir.path()).assert().success();

    mediagit()
        .arg("stats")
        .arg("--branches")
        .current_dir(temp_dir.path())
        .assert()
        .success();
}

#[test]
fn test_stats_json() {
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());

    add_and_commit(temp_dir.path(), "file.txt", "Content", "Initial commit");

    mediagit()
        .arg("stats")
        .arg("--json")
        .current_dir(temp_dir.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("{"));
}

#[test]
fn test_stats_files() {
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());

    add_and_commit(temp_dir.path(), "file1.txt", "Content 1", "First");
    add_and_commit(temp_dir.path(), "file2.txt", "Content 2", "Second");
    add_and_commit(temp_dir.path(), "file3.txt", "Content 3", "Third");

    mediagit()
        .arg("stats")
        .arg("--files")
        .current_dir(temp_dir.path())
        .assert()
        .success();
}

// ============================================================================
// Help Tests
// ============================================================================

#[test]
fn test_gc_help() {
    mediagit()
        .arg("gc")
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("garbage").or(predicate::str::contains("gc")));
}

#[test]
fn test_fsck_help() {
    mediagit()
        .arg("fsck")
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("fsck").or(predicate::str::contains("check")));
}

#[test]
fn test_verify_help() {
    mediagit()
        .arg("verify")
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("verify"));
}

#[test]
fn test_stats_help() {
    mediagit()
        .arg("stats")
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("stats").or(predicate::str::contains("statistics")));
}

// ============================================================================
// Performance Benchmarks
// ============================================================================

#[test]
fn test_maintenance_benchmark() {
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());

    // Create a repo with some commits
    for i in 1..=10 {
        add_and_commit(
            temp_dir.path(),
            &format!("file{}.txt", i),
            &format!("Content for file {} with some text to make it larger.", i),
            &format!("Commit {}", i),
        );
    }

    // Benchmark fsck
    let start = Instant::now();
    mediagit().arg("fsck").arg("-q").current_dir(temp_dir.path()).assert().success();
    println!("FSCK (10 commits): {:?}", start.elapsed());

    // Benchmark verify
    let start = Instant::now();
    mediagit().arg("verify").arg("-q").current_dir(temp_dir.path()).assert().success();
    println!("Verify (10 commits): {:?}", start.elapsed());

    // Benchmark gc
    let start = Instant::now();
    mediagit().arg("gc").arg("-q").current_dir(temp_dir.path()).assert().success();
    println!("GC (10 commits): {:?}", start.elapsed());

    // Benchmark stats
    let start = Instant::now();
    mediagit().arg("stats").arg("-q").current_dir(temp_dir.path()).assert().success();
    println!("Stats (10 commits): {:?}", start.elapsed());
}
