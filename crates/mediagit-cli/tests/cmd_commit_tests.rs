//! Tests for the `commit` command
//!
//! Tests commit creation with various options and validation.

#![allow(deprecated)]

use assert_cmd::Command;
use predicates::prelude::*;
use std::fs;
use tempfile::TempDir;

#[test]
fn test_commit_with_message() {
    let temp_dir = TempDir::new().unwrap();
    let mut cmd = Command::cargo_bin("mediagit").unwrap();

    cmd.arg("commit")
       .arg("-m")
       .arg("Test commit message")
       .current_dir(temp_dir.path())
       .assert()
       .failure()
       .stderr(predicate::str::contains("Not a mediagit repository"));
}

#[test]
fn test_commit_without_message() {
    let temp_dir = TempDir::new().unwrap();
    let mut cmd = Command::cargo_bin("mediagit").unwrap();

    cmd.arg("commit")
       .current_dir(temp_dir.path())
       .assert()
       .failure()
       .stderr(predicate::str::contains("please provide a commit message"));
}

#[test]
fn test_commit_with_edit_flag() {
    let temp_dir = TempDir::new().unwrap();
    let mut cmd = Command::cargo_bin("mediagit").unwrap();

    cmd.arg("commit")
       .arg("--edit")
       .current_dir(temp_dir.path())
       .assert()
       .failure()
       .stderr(predicate::str::contains("Not a mediagit repository"));
}

#[test]
fn test_commit_with_message_file() {
    let temp_dir = TempDir::new().unwrap();
    let msg_file = temp_dir.path().join("commit_msg.txt");
    fs::write(&msg_file, "Commit message from file").unwrap();

    let mut cmd = Command::cargo_bin("mediagit").unwrap();
    cmd.arg("commit")
       .arg("-F")
       .arg(msg_file.to_str().unwrap())
       .current_dir(temp_dir.path())
       .assert()
       .failure()
       .stderr(predicate::str::contains("Not a mediagit repository"));
}

#[test]
fn test_commit_all_flag() {
    let temp_dir = TempDir::new().unwrap();
    let mut cmd = Command::cargo_bin("mediagit").unwrap();

    cmd.arg("commit")
       .arg("-a")
       .arg("-m")
       .arg("Commit all changes")
       .current_dir(temp_dir.path())
       .assert()
       .failure()
       .stderr(predicate::str::contains("Not a mediagit repository"));
}

#[test]
fn test_commit_include_flag() {
    let temp_dir = TempDir::new().unwrap();
    let mut cmd = Command::cargo_bin("mediagit").unwrap();

    cmd.arg("commit")
       .arg("--include")
       .arg("-m")
       .arg("Include untracked files")
       .current_dir(temp_dir.path())
       .assert()
       .failure()
       .stderr(predicate::str::contains("Not a mediagit repository"));
}

#[test]
fn test_commit_custom_author() {
    let temp_dir = TempDir::new().unwrap();
    let mut cmd = Command::cargo_bin("mediagit").unwrap();

    cmd.arg("commit")
       .arg("-m")
       .arg("Test commit")
       .arg("--author")
       .arg("Test User <test@example.com>")
       .current_dir(temp_dir.path())
       .assert()
       .failure()
       .stderr(predicate::str::contains("Not a mediagit repository"));
}

#[test]
fn test_commit_custom_date() {
    let temp_dir = TempDir::new().unwrap();
    let mut cmd = Command::cargo_bin("mediagit").unwrap();

    cmd.arg("commit")
       .arg("-m")
       .arg("Test commit")
       .arg("--date")
       .arg("2024-01-01T00:00:00Z")
       .current_dir(temp_dir.path())
       .assert()
       .failure()
       .stderr(predicate::str::contains("Not a mediagit repository"));
}

#[test]
fn test_commit_allow_empty() {
    let temp_dir = TempDir::new().unwrap();
    let mut cmd = Command::cargo_bin("mediagit").unwrap();

    cmd.arg("commit")
       .arg("-m")
       .arg("Empty commit")
       .arg("--allow-empty")
       .current_dir(temp_dir.path())
       .assert()
       .failure()
       .stderr(predicate::str::contains("Not a mediagit repository"));
}

#[test]
fn test_commit_signoff() {
    let temp_dir = TempDir::new().unwrap();
    let mut cmd = Command::cargo_bin("mediagit").unwrap();

    cmd.arg("commit")
       .arg("-m")
       .arg("Test commit")
       .arg("--signoff")
       .current_dir(temp_dir.path())
       .assert()
       .failure()
       .stderr(predicate::str::contains("Not a mediagit repository"));
}

#[test]
fn test_commit_dry_run() {
    let temp_dir = TempDir::new().unwrap();
    let mut cmd = Command::cargo_bin("mediagit").unwrap();

    cmd.arg("commit")
       .arg("-m")
       .arg("Test commit")
       .arg("--dry-run")
       .current_dir(temp_dir.path())
       .assert()
       .failure()
       .stderr(predicate::str::contains("Not a mediagit repository"));
}

#[test]
fn test_commit_quiet_mode() {
    let temp_dir = TempDir::new().unwrap();
    let mut cmd = Command::cargo_bin("mediagit").unwrap();

    cmd.arg("commit")
       .arg("-m")
       .arg("Test commit")
       .arg("--quiet")
       .current_dir(temp_dir.path())
       .assert()
       .failure()
       .stderr(predicate::str::contains("Not a mediagit repository"));
}

#[test]
fn test_commit_verbose_mode() {
    let temp_dir = TempDir::new().unwrap();
    let mut cmd = Command::cargo_bin("mediagit").unwrap();

    cmd.arg("commit")
       .arg("-m")
       .arg("Test commit")
       .arg("--verbose")
       .current_dir(temp_dir.path())
       .assert()
       .failure()
       .stderr(predicate::str::contains("Not a mediagit repository"));
}

#[test]
fn test_commit_multiline_message() {
    let temp_dir = TempDir::new().unwrap();
    let mut cmd = Command::cargo_bin("mediagit").unwrap();

    cmd.arg("commit")
       .arg("-m")
       .arg("First line\n\nDetailed description here")
       .current_dir(temp_dir.path())
       .assert()
       .failure()
       .stderr(predicate::str::contains("Not a mediagit repository"));
}

#[test]
fn test_commit_help_output() {
    let mut cmd = Command::cargo_bin("mediagit").unwrap();

    cmd.arg("commit")
       .arg("--help")
       .assert()
       .success()
       .stdout(predicate::str::contains("Record changes to the repository"));
}

#[test]
fn test_commit_with_nonexistent_message_file() {
    let temp_dir = TempDir::new().unwrap();

    let mut cmd = Command::cargo_bin("mediagit").unwrap();
    cmd.arg("commit")
       .arg("-F")
       .arg("nonexistent.txt")
       .current_dir(temp_dir.path())
       .assert()
       .failure()
       .stderr(predicate::str::contains("Not a mediagit repository"));
}

#[test]
fn test_commit_empty_message() {
    let temp_dir = TempDir::new().unwrap();
    let mut cmd = Command::cargo_bin("mediagit").unwrap();

    cmd.arg("commit")
       .arg("-m")
       .arg("")
       .current_dir(temp_dir.path())
       .assert()
       .failure()
       .stderr(predicate::str::contains("aborting commit due to empty commit message"));
}

#[test]
fn test_commit_combined_flags() {
    let temp_dir = TempDir::new().unwrap();
    let mut cmd = Command::cargo_bin("mediagit").unwrap();

    cmd.arg("commit")
       .arg("-a")
       .arg("-m")
       .arg("Combined flags test")
       .arg("--signoff")
       .arg("--verbose")
       .current_dir(temp_dir.path())
       .assert()
       .failure()
       .stderr(predicate::str::contains("Not a mediagit repository"));
}
