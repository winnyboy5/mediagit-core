// Copyright (C) 2026  winnyboy5
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU Affero General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU Affero General Public License for more details.
//
// You should have received a copy of the GNU Affero General Public License
// along with this program.  If not, see <https://www.gnu.org/licenses/>.
// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2025 MediaGit Contributors

//! Comprehensive CLI Reset Command Tests
//!
//! Tests for `mediagit reset` command with all modes and options.
//!
//! Run: `cargo test --test cli_reset_test`

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
// Reset Help Tests
// ============================================================================

#[test]
fn test_reset_help() {
    mediagit()
        .arg("reset")
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("--soft"))
        .stdout(predicate::str::contains("--hard"))
        .stdout(predicate::str::contains("HEAD"));
}

// ============================================================================
// Reset Soft Mode Tests
// ============================================================================

#[test]
#[ignore] // Requires full working tree support
fn test_reset_soft_head() {
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());

    add_and_commit(temp_dir.path(), "file1.txt", "v1", "First commit");
    add_and_commit(temp_dir.path(), "file1.txt", "v2", "Second commit");

    // Soft reset to HEAD~1
    mediagit()
        .arg("reset")
        .arg("--soft")
        .arg("HEAD~1")
        .current_dir(temp_dir.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("HEAD is now at"));

    // File should still have new content (working tree unchanged)
    let content = fs::read_to_string(temp_dir.path().join("file1.txt")).unwrap();
    assert_eq!(content, "v2");
}

// ============================================================================
// Reset Mixed Mode Tests (default)
// ============================================================================

#[test]
#[ignore] // Requires full index support
fn test_reset_mixed_default() {
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());

    add_and_commit(temp_dir.path(), "file1.txt", "v1", "First commit");
    add_and_commit(temp_dir.path(), "file1.txt", "v2", "Second commit");

    // Mixed reset (default) to HEAD~1
    mediagit()
        .arg("reset")
        .arg("HEAD~1")
        .current_dir(temp_dir.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("changes are unstaged"));
}

// ============================================================================
// Reset Hard Mode Tests
// ============================================================================

#[test]
#[ignore] // Requires full checkout support
fn test_reset_hard() {
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());

    add_and_commit(temp_dir.path(), "file1.txt", "v1", "First commit");
    add_and_commit(temp_dir.path(), "file1.txt", "v2", "Second commit");

    // Hard reset to HEAD~1
    mediagit()
        .arg("reset")
        .arg("--hard")
        .arg("HEAD~1")
        .current_dir(temp_dir.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("working tree updated"));

    // File should now have old content
    let content = fs::read_to_string(temp_dir.path().join("file1.txt")).unwrap();
    assert_eq!(content, "v1");
}

// ============================================================================
// Reset Path (Unstage) Tests
// ============================================================================

#[test]
#[ignore] // Requires index tracking
fn test_reset_path_unstage() {
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());

    add_and_commit(temp_dir.path(), "file1.txt", "v1", "Initial");

    // Add a new file to staging
    fs::write(temp_dir.path().join("file2.txt"), "new file").unwrap();
    mediagit().arg("add").arg("file2.txt").current_dir(temp_dir.path()).assert().success();

    // Unstage file2.txt
    mediagit()
        .arg("reset")
        .arg("file2.txt")
        .current_dir(temp_dir.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("Unstaged"));

    // File should still exist in working directory
    assert!(temp_dir.path().join("file2.txt").exists());
}

#[test]
#[ignore] // Requires index tracking
fn test_reset_all_paths() {
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());

    add_and_commit(temp_dir.path(), "base.txt", "base", "Initial");

    // Add multiple files
    fs::write(temp_dir.path().join("a.txt"), "a").unwrap();
    fs::write(temp_dir.path().join("b.txt"), "b").unwrap();
    mediagit().arg("add").arg(".").current_dir(temp_dir.path()).assert().success();

    // Unstage all with reset .
    mediagit()
        .arg("reset")
        .arg(".")
        .current_dir(temp_dir.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("Unstaged all files"));
}

// ============================================================================
// Reset HEAD Reference Tests
// ============================================================================

#[test]
#[ignore] // Requires multiple commits
fn test_reset_head_tilde_n() {
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());

    add_and_commit(temp_dir.path(), "f.txt", "1", "Commit 1");
    add_and_commit(temp_dir.path(), "f.txt", "2", "Commit 2");
    add_and_commit(temp_dir.path(), "f.txt", "3", "Commit 3");

    // Reset 2 commits back
    mediagit()
        .arg("reset")
        .arg("--soft")
        .arg("HEAD~2")
        .current_dir(temp_dir.path())
        .assert()
        .success();
}

#[test]
#[ignore] // Requires commit history
fn test_reset_head_caret() {
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());

    add_and_commit(temp_dir.path(), "f.txt", "1", "Commit 1");
    add_and_commit(temp_dir.path(), "f.txt", "2", "Commit 2");

    // Reset using HEAD^
    mediagit()
        .arg("reset")
        .arg("--soft")
        .arg("HEAD^")
        .current_dir(temp_dir.path())
        .assert()
        .success();
}

// ============================================================================
// Error Case Tests
// ============================================================================

#[test]
fn test_reset_no_repo() {
    let temp_dir = TempDir::new().unwrap();

    mediagit()
        .arg("reset")
        .current_dir(temp_dir.path())
        .assert()
        .failure();
}

#[test]
#[ignore] // Requires error handling for invalid refs
fn test_reset_invalid_ref() {
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());
    add_and_commit(temp_dir.path(), "f.txt", "content", "Initial");

    mediagit()
        .arg("reset")
        .arg("nonexistent_ref")
        .current_dir(temp_dir.path())
        .assert()
        .failure()
        .stderr(predicate::str::contains("Unknown revision"));
}

// ============================================================================
// Quiet Mode Tests
// ============================================================================

#[test]
#[ignore]
fn test_reset_quiet() {
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());
    add_and_commit(temp_dir.path(), "f.txt", "v1", "Commit 1");
    add_and_commit(temp_dir.path(), "f.txt", "v2", "Commit 2");

    mediagit()
        .arg("reset")
        .arg("--soft")
        .arg("-q")
        .arg("HEAD~1")
        .current_dir(temp_dir.path())
        .assert()
        .success()
        .stdout(predicate::str::is_empty());
}
