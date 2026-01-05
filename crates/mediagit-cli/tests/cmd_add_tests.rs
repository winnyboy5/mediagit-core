//! Tests for the `add` command
//!
//! Tests file staging operations with various options and error conditions.

#![allow(deprecated)]

use assert_cmd::Command;
use predicates::prelude::*;
use std::fs;
use tempfile::TempDir;

#[test]
fn test_add_single_file() {
    let temp_dir = TempDir::new().unwrap();
    let test_file = temp_dir.path().join("test.txt");
    fs::write(&test_file, "test content").unwrap();

    let mut cmd = Command::cargo_bin("mediagit").unwrap();
    cmd.arg("add")
       .arg(test_file.to_str().unwrap())
       .current_dir(temp_dir.path())
       .assert()
       .failure()
       .stderr(predicate::str::contains("Not a mediagit repository"));
}

#[test]
fn test_add_multiple_files() {
    let temp_dir = TempDir::new().unwrap();
    let file1 = temp_dir.path().join("file1.txt");
    let file2 = temp_dir.path().join("file2.txt");
    fs::write(&file1, "content 1").unwrap();
    fs::write(&file2, "content 2").unwrap();

    let mut cmd = Command::cargo_bin("mediagit").unwrap();
    cmd.arg("add")
       .arg(file1.to_str().unwrap())
       .arg(file2.to_str().unwrap())
       .current_dir(temp_dir.path())
       .assert()
       .failure()
       .stderr(predicate::str::contains("Not a mediagit repository"));
}

#[test]
fn test_add_all_flag() {
    let temp_dir = TempDir::new().unwrap();
    fs::write(temp_dir.path().join("file1.txt"), "content").unwrap();
    fs::write(temp_dir.path().join("file2.txt"), "content").unwrap();

    let mut cmd = Command::cargo_bin("mediagit").unwrap();
    cmd.arg("add")
       .arg("--all")
       .arg(".")
       .current_dir(temp_dir.path())
       .assert()
       .failure()
       .stderr(predicate::str::contains("Not a mediagit repository"));
}

#[test]
fn test_add_patch_mode() {
    let temp_dir = TempDir::new().unwrap();
    let test_file = temp_dir.path().join("test.txt");
    fs::write(&test_file, "test content").unwrap();

    let mut cmd = Command::cargo_bin("mediagit").unwrap();
    cmd.arg("add")
       .arg("--patch")
       .arg(test_file.to_str().unwrap())
       .current_dir(temp_dir.path())
       .assert()
       .failure()
       .stderr(predicate::str::contains("Not a mediagit repository"));
}

#[test]
fn test_add_dry_run() {
    let temp_dir = TempDir::new().unwrap();
    let test_file = temp_dir.path().join("test.txt");
    fs::write(&test_file, "test content").unwrap();

    let mut cmd = Command::cargo_bin("mediagit").unwrap();
    cmd.arg("add")
       .arg("--dry-run")
       .arg(test_file.to_str().unwrap())
       .current_dir(temp_dir.path())
       .assert()
       .failure()
       .stderr(predicate::str::contains("Not a mediagit repository"));
}

#[test]
fn test_add_force_flag() {
    let temp_dir = TempDir::new().unwrap();
    let test_file = temp_dir.path().join("test.txt");
    fs::write(&test_file, "test content").unwrap();

    let mut cmd = Command::cargo_bin("mediagit").unwrap();
    cmd.arg("add")
       .arg("--force")
       .arg(test_file.to_str().unwrap())
       .current_dir(temp_dir.path())
       .assert()
       .failure()
       .stderr(predicate::str::contains("Not a mediagit repository"));
}

#[test]
fn test_add_update_flag() {
    let temp_dir = TempDir::new().unwrap();
    let test_file = temp_dir.path().join("test.txt");
    fs::write(&test_file, "test content").unwrap();

    let mut cmd = Command::cargo_bin("mediagit").unwrap();
    cmd.arg("add")
       .arg("--update")
       .arg(test_file.to_str().unwrap())
       .current_dir(temp_dir.path())
       .assert()
       .failure()
       .stderr(predicate::str::contains("Not a mediagit repository"));
}

#[test]
fn test_add_quiet_mode() {
    let temp_dir = TempDir::new().unwrap();
    let test_file = temp_dir.path().join("test.txt");
    fs::write(&test_file, "test content").unwrap();

    let mut cmd = Command::cargo_bin("mediagit").unwrap();
    cmd.arg("add")
       .arg("--quiet")
       .arg(test_file.to_str().unwrap())
       .current_dir(temp_dir.path())
       .assert()
       .failure()
       .stderr(predicate::str::contains("Not a mediagit repository"));
}

#[test]
fn test_add_verbose_mode() {
    let temp_dir = TempDir::new().unwrap();
    let test_file = temp_dir.path().join("test.txt");
    fs::write(&test_file, "test content").unwrap();

    let mut cmd = Command::cargo_bin("mediagit").unwrap();
    cmd.arg("add")
       .arg("--verbose")
       .arg(test_file.to_str().unwrap())
       .current_dir(temp_dir.path())
       .assert()
       .failure()
       .stderr(predicate::str::contains("Not a mediagit repository"));
}

#[test]
fn test_add_missing_required_argument() {
    let temp_dir = TempDir::new().unwrap();
    let mut cmd = Command::cargo_bin("mediagit").unwrap();

    cmd.arg("add")
       .current_dir(temp_dir.path())
       .assert()
       .failure()
       .stderr(predicate::str::contains("required"));
}

#[test]
fn test_add_nonexistent_file() {
    let temp_dir = TempDir::new().unwrap();

    let mut cmd = Command::cargo_bin("mediagit").unwrap();
    cmd.arg("add")
       .arg("nonexistent-file.txt")
       .current_dir(temp_dir.path())
       .assert()
       .failure();
}

#[test]
fn test_add_directory() {
    let temp_dir = TempDir::new().unwrap();
    let subdir = temp_dir.path().join("subdir");
    fs::create_dir(&subdir).unwrap();
    fs::write(subdir.join("file.txt"), "content").unwrap();

    let mut cmd = Command::cargo_bin("mediagit").unwrap();
    cmd.arg("add")
       .arg(subdir.to_str().unwrap())
       .current_dir(temp_dir.path())
       .assert()
       .failure()
       .stderr(predicate::str::contains("Not a mediagit repository"));
}

#[test]
fn test_add_glob_pattern() {
    let temp_dir = TempDir::new().unwrap();
    fs::write(temp_dir.path().join("file1.txt"), "content").unwrap();
    fs::write(temp_dir.path().join("file2.txt"), "content").unwrap();
    fs::write(temp_dir.path().join("file.rs"), "code").unwrap();

    let mut cmd = Command::cargo_bin("mediagit").unwrap();
    cmd.arg("add")
       .arg("*.txt")
       .current_dir(temp_dir.path())
       .assert()
       .failure()
       .stderr(predicate::str::contains("Not a mediagit repository"));
}

#[test]
fn test_add_ignore_removal_flag() {
    let temp_dir = TempDir::new().unwrap();
    let test_file = temp_dir.path().join("test.txt");
    fs::write(&test_file, "test content").unwrap();

    let mut cmd = Command::cargo_bin("mediagit").unwrap();
    cmd.arg("add")
       .arg("--ignore-removal")
       .arg(test_file.to_str().unwrap())
       .current_dir(temp_dir.path())
       .assert()
       .failure()
       .stderr(predicate::str::contains("Not a mediagit repository"));
}

#[test]
fn test_add_help_output() {
    let mut cmd = Command::cargo_bin("mediagit").unwrap();

    cmd.arg("add")
       .arg("--help")
       .assert()
       .success()
       .stdout(predicate::str::contains("Stage file contents for commit"));
}

#[test]
fn test_add_with_special_characters_in_filename() {
    let temp_dir = TempDir::new().unwrap();
    let special_file = temp_dir.path().join("file with spaces.txt");
    fs::write(&special_file, "content").unwrap();

    let mut cmd = Command::cargo_bin("mediagit").unwrap();
    cmd.arg("add")
       .arg(special_file.to_str().unwrap())
       .current_dir(temp_dir.path())
       .assert()
       .failure()
       .stderr(predicate::str::contains("Not a mediagit repository"));
}
