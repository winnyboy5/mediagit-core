// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2025 MediaGit Contributors

//! CLI command helpers for testing mediagit commands.
//!
//! Provides convenient wrappers around assert_cmd for testing the mediagit CLI.

use assert_cmd::Command;
use std::path::Path;

/// Creates a new mediagit Command for testing.
///
/// # Example
/// ```ignore
/// use mediagit_test_utils::mediagit;
///
/// mediagit()
///     .arg("init")
///     .current_dir(temp_dir.path())
///     .assert()
///     .success();
/// ```
#[allow(deprecated)] // cargo_bin is deprecated but still works for our use case
pub fn mediagit() -> Command {
    Command::cargo_bin("mediagit").expect("mediagit binary not found")
}

/// Fluent API wrapper for common mediagit command patterns.
///
/// Provides a more ergonomic interface for building and executing mediagit commands.
pub struct MediagitCommand {
    cmd: Command,
}

impl MediagitCommand {
    /// Create a new MediagitCommand.
    pub fn new() -> Self {
        Self {
            cmd: mediagit(),
        }
    }

    /// Set the working directory for the command.
    pub fn in_dir(mut self, dir: &Path) -> Self {
        self.cmd.current_dir(dir);
        self
    }

    /// Add an argument to the command.
    pub fn arg(mut self, arg: &str) -> Self {
        self.cmd.arg(arg);
        self
    }

    /// Add multiple arguments to the command.
    pub fn args(mut self, args: &[&str]) -> Self {
        self.cmd.args(args);
        self
    }

    /// Execute the command and assert success.
    pub fn run_success(mut self) -> assert_cmd::assert::Assert {
        self.cmd.assert().success()
    }

    /// Execute the command and assert failure.
    pub fn run_failure(mut self) -> assert_cmd::assert::Assert {
        self.cmd.assert().failure()
    }

    /// Get the underlying Command for custom assertions.
    pub fn into_inner(self) -> Command {
        self.cmd
    }

    /// Initialize a repository in the given directory (quiet mode).
    pub fn init_quiet(dir: &Path) {
        mediagit()
            .arg("init")
            .arg("-q")
            .current_dir(dir)
            .assert()
            .success();
    }

    /// Add files to the staging area.
    pub fn add(dir: &Path, paths: &[&str]) {
        let mut cmd = mediagit();
        cmd.arg("add").current_dir(dir);
        for path in paths {
            cmd.arg(path);
        }
        cmd.assert().success();
    }

    /// Create a commit with the given message.
    pub fn commit(dir: &Path, message: &str) {
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

    /// Get repository status.
    pub fn status(dir: &Path) -> assert_cmd::assert::Assert {
        mediagit()
            .arg("status")
            .current_dir(dir)
            .assert()
    }
}

impl Default for MediagitCommand {
    fn default() -> Self {
        Self::new()
    }
}
