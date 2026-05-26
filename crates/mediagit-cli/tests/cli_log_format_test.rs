// MediaGit - Git for Media Files
// Copyright (C) 2025 MediaGit Contributors
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU Affero General Public License as published
// by the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE. See the
// GNU Affero General Public License for more details.

//! Tests for the `mediagit log --format` feature

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

/// Test log --format with %H%n%s (full hash + newline + subject)
#[test]
fn test_log_format_hash_and_subject() {
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());

    add_and_commit(temp_dir.path(), "file1.txt", "Content 1", "First commit");
    add_and_commit(temp_dir.path(), "file2.txt", "Content 2", "Second commit");
    add_and_commit(temp_dir.path(), "file3.txt", "Content 3", "Third commit");

    let output = mediagit()
        .arg("log")
        .arg("--format=%H%n%s")
        .current_dir(temp_dir.path())
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let stdout = String::from_utf8_lossy(&output);

    // Each commit should appear with full hash followed by subject
    assert!(
        stdout.contains("Third commit"),
        "Third commit not found in output"
    );
    assert!(
        stdout.contains("Second commit"),
        "Second commit not found in output"
    );
    assert!(
        stdout.contains("First commit"),
        "First commit not found in output"
    );

    // Output should have newlines separating hash and subject
    let lines: Vec<&str> = stdout.lines().collect();
    assert!(
        lines.len() >= 6,
        "Expected at least 6 lines (3 commits × 2 lines each), got {}",
        lines.len()
    );
}

/// Test log --format with %h (abbreviated hash)
#[test]
fn test_log_format_short_hash() {
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());

    add_and_commit(temp_dir.path(), "file.txt", "Content", "Test commit");

    mediagit()
        .arg("log")
        .arg("--format=%h %s")
        .current_dir(temp_dir.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("Test commit"));
}

/// Test log --format with author info
#[test]
fn test_log_format_author() {
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());

    add_and_commit(temp_dir.path(), "file.txt", "Content", "Commit with author");

    mediagit()
        .arg("log")
        .arg("--format=%aN <%ae>: %s")
        .current_dir(temp_dir.path())
        .assert()
        .success();
}

/// Test log --format with %% (literal percent)
#[test]
fn test_log_format_literal_percent() {
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());

    add_and_commit(temp_dir.path(), "file.txt", "Content", "Test");

    mediagit()
        .arg("log")
        .arg("--format=100%% done: %s")
        .current_dir(temp_dir.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("100% done:"));
}

/// Test that --format and --oneline work together (--oneline should take precedence)
#[test]
fn test_log_format_with_oneline() {
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());

    add_and_commit(temp_dir.path(), "file.txt", "Content", "Test commit");

    // When both are provided, --oneline should be used (it's checked first in the code)
    mediagit()
        .arg("log")
        .arg("--format=%H")
        .arg("--oneline")
        .current_dir(temp_dir.path())
        .assert()
        .success();
}
