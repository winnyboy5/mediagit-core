//! Tests for the `merge` command
//!
//! Tests branch merging operations with various strategies and options.

#![allow(deprecated)]

use assert_cmd::Command;
use predicates::prelude::*;
use tempfile::TempDir;

#[test]
fn test_merge_branch() {
    let temp_dir = TempDir::new().unwrap();
    let mut cmd = Command::cargo_bin("mediagit").unwrap();

    cmd.arg("merge")
       .arg("feature-branch")
       .current_dir(temp_dir.path())
       .assert()
       .failure()
       .stderr(predicate::str::contains("Not a mediagit repository"));
}

#[test]
fn test_merge_with_message() {
    let temp_dir = TempDir::new().unwrap();
    let mut cmd = Command::cargo_bin("mediagit").unwrap();

    cmd.arg("merge")
       .arg("feature-branch")
       .arg("-m")
       .arg("Merge feature branch")
       .current_dir(temp_dir.path())
       .assert()
       .failure()
       .stderr(predicate::str::contains("Not a mediagit repository"));
}

#[test]
fn test_merge_no_fast_forward() {
    let temp_dir = TempDir::new().unwrap();
    let mut cmd = Command::cargo_bin("mediagit").unwrap();

    cmd.arg("merge")
       .arg("--no-ff")
       .arg("feature-branch")
       .current_dir(temp_dir.path())
       .assert()
       .failure()
       .stderr(predicate::str::contains("Not a mediagit repository"));
}

#[test]
fn test_merge_fast_forward_only() {
    let temp_dir = TempDir::new().unwrap();
    let mut cmd = Command::cargo_bin("mediagit").unwrap();

    cmd.arg("merge")
       .arg("--ff-only")
       .arg("feature-branch")
       .current_dir(temp_dir.path())
       .assert()
       .failure()
       .stderr(predicate::str::contains("Not a mediagit repository"));
}

#[test]
fn test_merge_squash() {
    let temp_dir = TempDir::new().unwrap();
    let mut cmd = Command::cargo_bin("mediagit").unwrap();

    cmd.arg("merge")
       .arg("--squash")
       .arg("feature-branch")
       .current_dir(temp_dir.path())
       .assert()
       .failure()
       .stderr(predicate::str::contains("Not a mediagit repository"));
}

#[test]
fn test_merge_with_strategy() {
    let temp_dir = TempDir::new().unwrap();
    let mut cmd = Command::cargo_bin("mediagit").unwrap();

    cmd.arg("merge")
       .arg("--strategy")
       .arg("recursive")
       .arg("feature-branch")
       .current_dir(temp_dir.path())
       .assert()
       .failure()
       .stderr(predicate::str::contains("Not a mediagit repository"));
}

#[test]
fn test_merge_with_strategy_option() {
    let temp_dir = TempDir::new().unwrap();
    let mut cmd = Command::cargo_bin("mediagit").unwrap();

    cmd.arg("merge")
       .arg("-s")
       .arg("recursive")
       .arg("-X")
       .arg("ours")
       .arg("feature-branch")
       .current_dir(temp_dir.path())
       .assert()
       .failure()
       .stderr(predicate::str::contains("Not a mediagit repository"));
}

#[test]
fn test_merge_no_commit() {
    let temp_dir = TempDir::new().unwrap();
    let mut cmd = Command::cargo_bin("mediagit").unwrap();

    cmd.arg("merge")
       .arg("--no-commit")
       .arg("feature-branch")
       .current_dir(temp_dir.path())
       .assert()
       .failure()
       .stderr(predicate::str::contains("Not a mediagit repository"));
}

#[test]
fn test_merge_abort() {
    let temp_dir = TempDir::new().unwrap();
    let mut cmd = Command::cargo_bin("mediagit").unwrap();

    cmd.arg("merge")
       .arg("--abort")
       .arg("feature-branch")
       .current_dir(temp_dir.path())
       .assert()
       .failure()
       .stderr(predicate::str::contains("Not a mediagit repository"));
}

#[test]
fn test_merge_continue() {
    let temp_dir = TempDir::new().unwrap();
    let mut cmd = Command::cargo_bin("mediagit").unwrap();

    cmd.arg("merge")
       .arg("--continue-merge")
       .arg("feature-branch")
       .current_dir(temp_dir.path())
       .assert()
       .failure()
       .stderr(predicate::str::contains("Not a mediagit repository"));
}

#[test]
fn test_merge_quiet_mode() {
    let temp_dir = TempDir::new().unwrap();
    let mut cmd = Command::cargo_bin("mediagit").unwrap();

    cmd.arg("merge")
       .arg("--quiet")
       .arg("feature-branch")
       .current_dir(temp_dir.path())
       .assert()
       .failure()
       .stderr(predicate::str::contains("Not a mediagit repository"));
}

#[test]
fn test_merge_verbose_mode() {
    let temp_dir = TempDir::new().unwrap();
    let mut cmd = Command::cargo_bin("mediagit").unwrap();

    cmd.arg("merge")
       .arg("--verbose")
       .arg("feature-branch")
       .current_dir(temp_dir.path())
       .assert()
       .failure()
       .stderr(predicate::str::contains("Not a mediagit repository"));
}

#[test]
fn test_merge_missing_branch_name() {
    let temp_dir = TempDir::new().unwrap();
    let mut cmd = Command::cargo_bin("mediagit").unwrap();

    cmd.arg("merge")
       .current_dir(temp_dir.path())
       .assert()
       .failure()
       .stderr(predicate::str::contains("required"));
}

#[test]
fn test_merge_conflicting_ff_options() {
    let temp_dir = TempDir::new().unwrap();
    let mut cmd = Command::cargo_bin("mediagit").unwrap();

    // Both --no-ff and --ff-only should conflict
    cmd.arg("merge")
       .arg("--no-ff")
       .arg("--ff-only")
       .arg("feature-branch")
       .current_dir(temp_dir.path())
       .assert()
       .failure();
}

#[test]
fn test_merge_help_output() {
    let mut cmd = Command::cargo_bin("mediagit").unwrap();

    cmd.arg("merge")
       .arg("--help")
       .assert()
       .success()
       .stdout(predicate::str::contains("Merge branches"));
}

#[test]
fn test_merge_combined_options() {
    let temp_dir = TempDir::new().unwrap();
    let mut cmd = Command::cargo_bin("mediagit").unwrap();

    cmd.arg("merge")
       .arg("--no-ff")
       .arg("-m")
       .arg("Merge with no fast-forward")
       .arg("--verbose")
       .arg("feature-branch")
       .current_dir(temp_dir.path())
       .assert()
       .failure()
       .stderr(predicate::str::contains("Not a mediagit repository"));
}

#[test]
fn test_merge_strategy_ours() {
    let temp_dir = TempDir::new().unwrap();
    let mut cmd = Command::cargo_bin("mediagit").unwrap();

    cmd.arg("merge")
       .arg("-s")
       .arg("ours")
       .arg("feature-branch")
       .current_dir(temp_dir.path())
       .assert()
       .failure()
       .stderr(predicate::str::contains("Not a mediagit repository"));
}

#[test]
fn test_merge_strategy_theirs() {
    let temp_dir = TempDir::new().unwrap();
    let mut cmd = Command::cargo_bin("mediagit").unwrap();

    cmd.arg("merge")
       .arg("-s")
       .arg("theirs")
       .arg("feature-branch")
       .current_dir(temp_dir.path())
       .assert()
       .failure()
       .stderr(predicate::str::contains("Not a mediagit repository"));
}

#[test]
fn test_merge_squash_with_message() {
    let temp_dir = TempDir::new().unwrap();
    let mut cmd = Command::cargo_bin("mediagit").unwrap();

    cmd.arg("merge")
       .arg("--squash")
       .arg("-m")
       .arg("Squash merge message")
       .arg("feature-branch")
       .current_dir(temp_dir.path())
       .assert()
       .failure()
       .stderr(predicate::str::contains("Not a mediagit repository"));
}
