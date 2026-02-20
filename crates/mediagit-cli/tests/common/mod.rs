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

//! Common test helpers for MediaGit CLI tests.
//!
//! This module re-exports utilities from mediagit-test-utils and provides
//! additional CLI-specific helper functions.

pub use mediagit_test_utils::{
    mediagit, MediagitCommand, TestRepo, TestPaths, TestFixtures,
    assert_repo_initialized, assert_file_tracked, assert_branch_exists,
    assert_branch_not_exists, assert_on_branch,
};

use assert_cmd::Command;
use std::fs;
use std::path::Path;

/// Initialize a repository in the given directory (quiet mode).
pub fn init_repo(dir: &Path) {
    mediagit()
        .arg("init")
        .arg("-q")
        .current_dir(dir)
        .assert()
        .success();
}

/// Add files and create a commit.
pub fn add_and_commit(dir: &Path, name: &str, content: &str, message: &str) {
    fs::write(dir.join(name), content).expect("Failed to write file");
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

/// Create a new branch.
pub fn create_branch(dir: &Path, name: &str) {
    mediagit()
        .arg("branch")
        .arg("create")
        .arg(name)
        .current_dir(dir)
        .assert()
        .success();
}

/// Switch to a branch.
pub fn switch_branch(dir: &Path, name: &str) {
    mediagit()
        .arg("branch")
        .arg("switch")
        .arg(name)
        .current_dir(dir)
        .assert()
        .success();
}

/// Create and switch to a branch in one operation.
pub fn create_and_switch_branch(dir: &Path, name: &str) {
    create_branch(dir, name);
    switch_branch(dir, name);
}

/// Get the test-files directory path.
pub fn test_files_dir() -> std::path::PathBuf {
    TestPaths::test_files_dir()
}

/// Copy a test file to the repository.
pub fn copy_test_file(test_file: &str, repo_dir: &Path, dest_name: &str) -> std::path::PathBuf {
    let source = test_files_dir().join(test_file);
    let dest = repo_dir.join(dest_name);

    if source.exists() {
        if let Some(parent) = dest.parent() {
            fs::create_dir_all(parent).ok();
        }
        fs::copy(&source, &dest).ok();
    }

    dest
}

/// Get the file size in bytes.
pub fn file_size_bytes(path: &Path) -> u64 {
    fs::metadata(path)
        .map(|m| m.len())
        .unwrap_or(0)
}
