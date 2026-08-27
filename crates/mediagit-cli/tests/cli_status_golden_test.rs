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

//! Golden-output snapshot tests for `mediagit status`.
//!
//! CRITICAL REGRESSION RULE: the `--porcelain` output format is frozen. Future
//! edits to status.rs may only ADD new lines to porcelain output, never change
//! or remove existing ones. The human (non-porcelain) format may change; when it
//! does, update `EXPECTED_HUMAN` below to match the new intentional format.

use assert_cmd::Command;
use std::fs;
use std::path::Path;
use tempfile::TempDir;

#[allow(deprecated)]
fn mediagit() -> Command {
    {
        // `commit` refuses an unconfigured identity (UX-6) instead of
        // authoring as `Unknown <unknown@localhost>`, so tests declare one
        // the way a real user would.
        let mut c = Command::cargo_bin("mediagit").unwrap();
        c.env("MEDIAGIT_AUTHOR_NAME", "Test User")
            .env("MEDIAGIT_AUTHOR_EMAIL", "test@example.com");
        c
    }
}

fn init_repo(dir: &Path) {
    mediagit()
        .arg("init")
        .arg("-q")
        .current_dir(dir)
        .assert()
        .success();
}

/// Build a fixture repo with one modified, one deleted, one untracked, and one
/// staged file — the minimum mix needed to exercise every status code path.
fn build_fixture(dir: &Path) {
    init_repo(dir);

    fs::write(dir.join("modified.txt"), "hello world\n").unwrap();
    fs::write(dir.join("deleted.txt"), "to-delete content\n").unwrap();

    mediagit()
        .args(["add", "modified.txt", "deleted.txt", "-q"])
        .current_dir(dir)
        .assert()
        .success();
    mediagit()
        .args(["commit", "-m", "initial", "-q"])
        .current_dir(dir)
        .assert()
        .success();

    // Modify a tracked file
    fs::write(dir.join("modified.txt"), "hello world CHANGED\n").unwrap();
    // Delete a tracked file from the working tree
    fs::remove_file(dir.join("deleted.txt")).unwrap();
    // Stage a new file
    fs::write(dir.join("staged.txt"), "staged content\n").unwrap();
    mediagit()
        .args(["add", "staged.txt", "-q"])
        .current_dir(dir)
        .assert()
        .success();
    // Leave an untracked file
    fs::write(dir.join("untracked.txt"), "an untracked file\n").unwrap();
}

#[test]
fn test_status_porcelain_golden() {
    let temp_dir = TempDir::new().unwrap();
    build_fixture(temp_dir.path());

    let output = mediagit()
        .arg("status")
        .arg("--porcelain")
        .current_dir(temp_dir.path())
        .output()
        .unwrap();

    let stdout = String::from_utf8_lossy(&output.stdout);

    // Porcelain lines pinned today — these must never change, only gain siblings.
    assert!(
        stdout.contains("A  staged.txt"),
        "missing staged line, got:\n{stdout}"
    );
    assert!(
        stdout.contains(" M modified.txt"),
        "missing modified line, got:\n{stdout}"
    );
    assert!(
        stdout.contains(" D deleted.txt"),
        "missing deleted line, got:\n{stdout}"
    );
    assert!(
        stdout.contains("?? untracked.txt"),
        "missing untracked line, got:\n{stdout}"
    );
}

#[test]
fn test_status_human_golden() {
    let temp_dir = TempDir::new().unwrap();
    build_fixture(temp_dir.path());

    let output = mediagit()
        .arg("status")
        .current_dir(temp_dir.path())
        .output()
        .unwrap();

    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(
        stdout.contains("staged.txt"),
        "missing staged.txt:\n{stdout}"
    );
    assert!(
        stdout.contains("modified.txt"),
        "missing modified.txt:\n{stdout}"
    );
    assert!(
        stdout.contains("deleted.txt"),
        "missing deleted.txt:\n{stdout}"
    );
    assert!(
        stdout.contains("untracked.txt"),
        "missing untracked.txt:\n{stdout}"
    );
}
