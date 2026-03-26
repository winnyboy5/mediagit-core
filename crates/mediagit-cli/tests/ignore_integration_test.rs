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

// Integration tests for .mediagitignore support in `add` and `status` commands.
//
// These tests use `mediagit-test-utils` for repo setup and `assert_cmd` for CLI invocation.

use std::fs;
use tempfile::TempDir;

/// Helper: create a temp dir, run `mediagit init`, write .mediagitignore, create files.
fn setup_repo(ignore_content: &str, files: &[(&str, &str)]) -> TempDir {
    let dir = TempDir::new().expect("tempdir");
    let root = dir.path();

    // init
    std::process::Command::new("mediagit")
        .args(["init"])
        .current_dir(root)
        .output()
        .expect("mediagit init");

    // write .mediagitignore
    if !ignore_content.is_empty() {
        fs::write(root.join(".mediagitignore"), ignore_content).expect("write .mediagitignore");
    }

    // create test files
    for (path, content) in files {
        if let Some(parent) = std::path::Path::new(path).parent() {
            if !parent.as_os_str().is_empty() {
                fs::create_dir_all(root.join(parent)).expect("create parent dir");
            }
        }
        fs::write(root.join(path), content).expect("write test file");
    }

    dir
}

/// Run a mediagit command in the given directory and return (stdout, success)
fn run(dir: &std::path::Path, args: &[&str]) -> (String, bool) {
    let out = std::process::Command::new("mediagit")
        .args(args)
        .current_dir(dir)
        .output()
        .expect("run mediagit");
    let stdout =
        String::from_utf8_lossy(&out.stdout).to_string() + &String::from_utf8_lossy(&out.stderr);
    (stdout, out.status.success())
}

#[test]
fn test_add_skips_ignored_files() {
    let dir = setup_repo(
        "*.tmp\ndebug.log\n",
        &[
            ("file.txt", "data"),
            ("file.tmp", "temp"),
            ("debug.log", "log"),
        ],
    );
    let root = dir.path();

    let (_out, ok) = run(root, &["add", "--all"]);
    assert!(ok, "add --all should succeed");

    // Check index: file.txt should be staged, file.tmp and debug.log should not
    let (status_out, _) = run(root, &["status", "--porcelain"]);
    assert!(status_out.contains("file.txt"), "file.txt should be staged");
    assert!(
        !status_out.contains("file.tmp"),
        "file.tmp should be ignored"
    );
    assert!(
        !status_out.contains("debug.log"),
        "debug.log should be ignored"
    );
}

#[test]
fn test_add_force_overrides_ignore() {
    let dir = setup_repo("*.tmp\n", &[("file.tmp", "temp data")]);
    let root = dir.path();

    // --force should override .mediagitignore
    let (_out, ok) = run(root, &["add", "--force", "file.tmp"]);
    assert!(ok, "add --force should succeed");

    let (status_out, _) = run(root, &["status", "--porcelain"]);
    assert!(
        status_out.contains("file.tmp"),
        "file.tmp should be staged with --force"
    );
}

#[test]
fn test_status_hides_ignored_from_untracked() {
    let dir = setup_repo("*.tmp\n", &[("normal.txt", "data"), ("secret.tmp", "temp")]);
    let root = dir.path();

    let (status_out, _) = run(root, &["status"]);
    // secret.tmp should NOT appear in untracked output
    assert!(
        !status_out.contains("secret.tmp"),
        "ignored file should not be in untracked list"
    );
    // normal.txt should appear as untracked
    assert!(
        status_out.contains("normal.txt"),
        "normal file should appear as untracked"
    );
}

#[test]
fn test_status_shows_ignored_with_flag() {
    let dir = setup_repo(
        "*.tmp\n",
        &[("normal.txt", "data"), ("ignored.tmp", "temp")],
    );
    let root = dir.path();

    let (status_out, _) = run(root, &["status", "--ignored"]);
    assert!(
        status_out.contains("ignored.tmp"),
        "ignored file should appear with --ignored flag"
    );
}

#[test]
fn test_status_porcelain_ignored() {
    let dir = setup_repo("*.log\n", &[("app.log", "log data")]);
    let root = dir.path();

    let (status_out, _) = run(root, &["status", "--porcelain", "--ignored"]);
    // Porcelain format should use "!! " prefix for ignored files
    assert!(
        status_out.contains("!! "),
        "porcelain output should use !! prefix for ignored"
    );
    assert!(
        status_out.contains("app.log"),
        "ignored file should be listed"
    );
}

#[test]
fn test_ignore_directory_pattern() {
    let dir = setup_repo(
        "build/\n",
        &[
            ("src/main.rs", "fn main() {}"),
            ("build/output.bin", "binary"),
            ("build/artifact.o", "obj"),
        ],
    );
    let root = dir.path();

    let (_out, ok) = run(root, &["add", "--all"]);
    assert!(ok, "add --all should succeed");

    let (status_out, _) = run(root, &["status", "--porcelain"]);
    assert!(
        status_out.contains("src/main.rs"),
        "src/main.rs should be staged"
    );
    assert!(
        !status_out.contains("build/output.bin"),
        "build/ dir should be ignored"
    );
    assert!(
        !status_out.contains("build/artifact.o"),
        "build/ dir should be ignored"
    );
}

#[test]
fn test_ignore_negation() {
    let dir = setup_repo(
        "*.log\n!important.log\n",
        &[("debug.log", "debug"), ("important.log", "keep this")],
    );
    let root = dir.path();

    let (_out, ok) = run(root, &["add", "--all"]);
    assert!(ok, "add --all should succeed");

    let (status_out, _) = run(root, &["status", "--porcelain"]);
    // important.log negates the *.log pattern so it should be staged
    assert!(
        status_out.contains("important.log"),
        "important.log should be staged (negation)"
    );
    assert!(
        !status_out.contains("debug.log"),
        "debug.log should be ignored"
    );
}

#[test]
fn test_no_ignore_file_behavior_unchanged() {
    // No .mediagitignore file: all files should be add-able as before
    let dir = setup_repo("", &[("file.txt", "data"), ("file.tmp", "temp")]);
    let root = dir.path();

    let (_out, ok) = run(root, &["add", "--all"]);
    assert!(ok, "add --all should succeed without .mediagitignore");

    let (status_out, _) = run(root, &["status", "--porcelain"]);
    assert!(status_out.contains("file.txt"), "file.txt should be staged");
    assert!(
        status_out.contains("file.tmp"),
        "file.tmp should be staged (no ignore file)"
    );
}
