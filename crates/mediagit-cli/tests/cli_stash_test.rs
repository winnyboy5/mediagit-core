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

//! Comprehensive CLI Stash Command Tests
//!
//! Tests for `mediagit stash` command with all subcommands and options.
//!
//! NOTE: The stash command is not fully implemented in MediaGit yet.
//! All tests are marked #[ignore] until the feature is complete.
//! Run with: `cargo test --test cli_stash_test -- --ignored`

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
// Stash Save Tests
// ============================================================================

#[test]
#[ignore]
fn test_stash_save() {
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());

    add_and_commit(temp_dir.path(), "file.txt", "Initial content", "Initial commit");

    fs::write(temp_dir.path().join("file.txt"), "Modified content").unwrap();

    mediagit()
        .arg("stash")
        .arg("save")
        .current_dir(temp_dir.path())
        .assert()
        .success();

    let content = fs::read_to_string(temp_dir.path().join("file.txt")).unwrap();
    assert_eq!(content, "Initial content");
}

#[test]
#[ignore]
fn test_stash_save_with_message() {
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());

    add_and_commit(temp_dir.path(), "file.txt", "Initial content", "Initial commit");

    fs::write(temp_dir.path().join("file.txt"), "Modified content").unwrap();

    mediagit()
        .arg("stash")
        .arg("save")
        .arg("WIP: feature work")
        .current_dir(temp_dir.path())
        .assert()
        .success();

    mediagit()
        .arg("stash")
        .arg("list")
        .current_dir(temp_dir.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("WIP: feature work"));
}

#[test]
#[ignore]
fn test_stash_list() {
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());

    add_and_commit(temp_dir.path(), "file.txt", "Initial content", "Initial commit");

    mediagit()
        .arg("stash")
        .arg("list")
        .current_dir(temp_dir.path())
        .assert()
        .success();
}

#[test]
#[ignore]
fn test_stash_pop() {
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());

    add_and_commit(temp_dir.path(), "file.txt", "Initial content", "Initial commit");

    fs::write(temp_dir.path().join("file.txt"), "Stashed content").unwrap();
    mediagit().arg("stash").arg("save").current_dir(temp_dir.path()).assert().success();

    mediagit()
        .arg("stash")
        .arg("pop")
        .current_dir(temp_dir.path())
        .assert()
        .success();

    let content = fs::read_to_string(temp_dir.path().join("file.txt")).unwrap();
    assert_eq!(content, "Stashed content");
}

#[test]
#[ignore]
fn test_stash_apply() {
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());

    add_and_commit(temp_dir.path(), "file.txt", "Initial content", "Initial commit");

    fs::write(temp_dir.path().join("file.txt"), "Stashed content").unwrap();
    mediagit().arg("stash").arg("save").current_dir(temp_dir.path()).assert().success();

    mediagit()
        .arg("stash")
        .arg("apply")
        .current_dir(temp_dir.path())
        .assert()
        .success();

    let content = fs::read_to_string(temp_dir.path().join("file.txt")).unwrap();
    assert_eq!(content, "Stashed content");
}

#[test]
#[ignore]
fn test_stash_drop() {
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());

    add_and_commit(temp_dir.path(), "file.txt", "Initial content", "Initial commit");

    fs::write(temp_dir.path().join("file.txt"), "Stashed content").unwrap();
    mediagit().arg("stash").arg("save").current_dir(temp_dir.path()).assert().success();

    mediagit()
        .arg("stash")
        .arg("drop")
        .current_dir(temp_dir.path())
        .assert()
        .success();
}

#[test]
#[ignore]
fn test_stash_clear() {
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());

    add_and_commit(temp_dir.path(), "file.txt", "Initial content", "Initial commit");

    for i in 1..=3 {
        fs::write(temp_dir.path().join("file.txt"), format!("Content {}", i)).unwrap();
        mediagit().arg("stash").arg("save").current_dir(temp_dir.path()).assert().success();
    }

    mediagit()
        .arg("stash")
        .arg("clear")
        .current_dir(temp_dir.path())
        .assert()
        .success();
}

#[test]
fn test_stash_help() {
    mediagit()
        .arg("stash")
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("stash"));
}
