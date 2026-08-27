// SPDX-License-Identifier: BUSL-1.1
// Copyright (C) 2025-2026 Aswin Krishnamoorthy

//! Tests for the `mediagit log --format` feature

use assert_cmd::Command;
use predicates::prelude::*;
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

/// UX-5: `--since`/`--until` were declared, shown in `log --help`, and never
/// read, so a date-bounded log returned the entire history regardless. A
/// window that excludes everything must therefore show nothing.
#[test]
fn log_since_excludes_commits_before_the_bound() {
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());
    add_and_commit(temp_dir.path(), "file.txt", "Content", "Findable commit");

    // Sanity: the commit is visible without a date filter.
    mediagit()
        .arg("log")
        .current_dir(temp_dir.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("Findable commit"));

    // Everything was committed today, so a window starting in 2099 is empty.
    mediagit()
        .arg("log")
        .arg("--since")
        .arg("2099-01-01")
        .current_dir(temp_dir.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("Findable commit").not());

    // ...and one ending in 1999 is too.
    mediagit()
        .arg("log")
        .arg("--until")
        .arg("1999-12-31")
        .current_dir(temp_dir.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("Findable commit").not());
}

/// A window that does contain the commit must still show it — otherwise the
/// filter is just a fancier way of hiding history.
#[test]
fn log_since_keeps_commits_inside_the_window() {
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());
    add_and_commit(temp_dir.path(), "file.txt", "Content", "Findable commit");

    mediagit()
        .arg("log")
        .arg("--since")
        .arg("2000-01-01")
        .arg("--until")
        .arg("2099-12-31")
        .current_dir(temp_dir.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("Findable commit"));
}

/// A malformed date must fail loudly rather than being ignored.
#[test]
fn log_rejects_an_unparseable_date() {
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());
    add_and_commit(temp_dir.path(), "file.txt", "Content", "Findable commit");

    mediagit()
        .arg("log")
        .arg("--since")
        .arg("last tuesday")
        .current_dir(temp_dir.path())
        .assert()
        .failure();
}
