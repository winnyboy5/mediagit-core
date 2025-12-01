//! Tests for the `status` command
//!
//! Tests repository status reporting with various options.

use assert_cmd::Command;
use predicates::prelude::*;
use std::fs;
use tempfile::TempDir;

#[test]
fn test_status_basic() {
    let temp_dir = TempDir::new().unwrap();
    let mut cmd = Command::cargo_bin("mediagit").unwrap();

    cmd.arg("status")
       .current_dir(temp_dir.path())
       .assert()
       .failure()
       .stderr(predicate::str::contains("not yet implemented"));
}

#[test]
fn test_status_short_format() {
    let temp_dir = TempDir::new().unwrap();
    let mut cmd = Command::cargo_bin("mediagit").unwrap();

    cmd.arg("status")
       .arg("--short")
       .current_dir(temp_dir.path())
       .assert()
       .failure()
       .stderr(predicate::str::contains("not yet implemented"));
}

#[test]
fn test_status_porcelain_format() {
    let temp_dir = TempDir::new().unwrap();
    let mut cmd = Command::cargo_bin("mediagit").unwrap();

    cmd.arg("status")
       .arg("--porcelain")
       .current_dir(temp_dir.path())
       .assert()
       .failure()
       .stderr(predicate::str::contains("not yet implemented"));
}

#[test]
fn test_status_show_branch() {
    let temp_dir = TempDir::new().unwrap();
    let mut cmd = Command::cargo_bin("mediagit").unwrap();

    cmd.arg("status")
       .arg("--branch")
       .current_dir(temp_dir.path())
       .assert()
       .failure()
       .stderr(predicate::str::contains("not yet implemented"));
}

#[test]
fn test_status_show_tracked() {
    let temp_dir = TempDir::new().unwrap();
    let mut cmd = Command::cargo_bin("mediagit").unwrap();

    cmd.arg("status")
       .arg("--tracked")
       .current_dir(temp_dir.path())
       .assert()
       .failure()
       .stderr(predicate::str::contains("not yet implemented"));
}

#[test]
fn test_status_show_untracked() {
    let temp_dir = TempDir::new().unwrap();
    let mut cmd = Command::cargo_bin("mediagit").unwrap();

    cmd.arg("status")
       .arg("--untracked")
       .current_dir(temp_dir.path())
       .assert()
       .failure()
       .stderr(predicate::str::contains("not yet implemented"));
}

#[test]
fn test_status_show_ignored() {
    let temp_dir = TempDir::new().unwrap();
    let mut cmd = Command::cargo_bin("mediagit").unwrap();

    cmd.arg("status")
       .arg("--ignored")
       .current_dir(temp_dir.path())
       .assert()
       .failure()
       .stderr(predicate::str::contains("not yet implemented"));
}

#[test]
fn test_status_ahead_behind() {
    let temp_dir = TempDir::new().unwrap();
    let mut cmd = Command::cargo_bin("mediagit").unwrap();

    cmd.arg("status")
       .arg("--ahead-behind")
       .current_dir(temp_dir.path())
       .assert()
       .failure()
       .stderr(predicate::str::contains("not yet implemented"));
}

#[test]
fn test_status_quiet_mode() {
    let temp_dir = TempDir::new().unwrap();
    let mut cmd = Command::cargo_bin("mediagit").unwrap();

    cmd.arg("status")
       .arg("--quiet")
       .current_dir(temp_dir.path())
       .assert()
       .failure()
       .stderr(predicate::str::contains("not yet implemented"));
}

#[test]
fn test_status_verbose_mode() {
    let temp_dir = TempDir::new().unwrap();
    let mut cmd = Command::cargo_bin("mediagit").unwrap();

    cmd.arg("status")
       .arg("--verbose")
       .current_dir(temp_dir.path())
       .assert()
       .failure()
       .stderr(predicate::str::contains("not yet implemented"));
}

#[test]
fn test_status_with_uninitialized_repo() {
    let temp_dir = TempDir::new().unwrap();
    let mut cmd = Command::cargo_bin("mediagit").unwrap();

    cmd.arg("status")
       .current_dir(temp_dir.path())
       .assert()
       .failure();
}

#[test]
fn test_status_help_output() {
    let mut cmd = Command::cargo_bin("mediagit").unwrap();

    cmd.arg("status")
       .arg("--help")
       .assert()
       .success()
       .stdout(predicate::str::contains("Show the working tree status"));
}

#[test]
fn test_status_combined_options() {
    let temp_dir = TempDir::new().unwrap();
    let mut cmd = Command::cargo_bin("mediagit").unwrap();

    cmd.arg("status")
       .arg("--short")
       .arg("--branch")
       .arg("--verbose")
       .current_dir(temp_dir.path())
       .assert()
       .failure()
       .stderr(predicate::str::contains("not yet implemented"));
}

#[test]
fn test_status_with_files() {
    let temp_dir = TempDir::new().unwrap();
    fs::write(temp_dir.path().join("file1.txt"), "content").unwrap();
    fs::write(temp_dir.path().join("file2.txt"), "content").unwrap();

    let mut cmd = Command::cargo_bin("mediagit").unwrap();
    cmd.arg("status")
       .current_dir(temp_dir.path())
       .assert()
       .failure()
       .stderr(predicate::str::contains("not yet implemented"));
}

#[test]
fn test_status_short_and_porcelain_conflict() {
    let temp_dir = TempDir::new().unwrap();
    let mut cmd = Command::cargo_bin("mediagit").unwrap();

    // --short and --porcelain might conflict
    cmd.arg("status")
       .arg("--short")
       .arg("--porcelain")
       .current_dir(temp_dir.path())
       .assert()
       .failure();
}
