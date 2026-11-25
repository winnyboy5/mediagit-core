// SPDX-License-Identifier: BUSL-1.1
// Copyright (C) 2025-2026 Aswin Krishnamoorthy

//! R-12: a command that hits a file another program holds open says so in
//! plain words, names the file, and shows no raw `os error N` text unless
//! `--verbose` asks for the technical details.

#![cfg(windows)]

use assert_cmd::Command;
use std::fs;
use std::os::windows::fs::OpenOptionsExt;
use std::path::Path;
use tempfile::TempDir;

#[allow(deprecated)]
fn mediagit() -> Command {
    let mut c = Command::cargo_bin("mediagit").unwrap();
    c.env("MEDIAGIT_AUTHOR_NAME", "Test User")
        .env("MEDIAGIT_AUTHOR_EMAIL", "test@example.com")
        .env("MEDIAGIT_NO_KEYRING", "1");
    c
}

fn run(dir: &Path, args: &[&str]) -> std::process::Output {
    mediagit().args(args).current_dir(dir).output().unwrap()
}

/// A repository with a.txt and b.txt committed, then b.txt edited and held open
/// with share mode 0 (no other process may read, write or delete it).
fn locked_repo() -> (TempDir, fs::File) {
    let t = TempDir::new().unwrap();
    let d = t.path();
    assert!(run(d, &["init", "-q"]).status.success());
    fs::write(d.join("a.txt"), "a v1").unwrap();
    fs::write(d.join("b.txt"), "b v1").unwrap();
    assert!(run(d, &["add", "--all"]).status.success());
    assert!(run(d, &["commit", "-m", "v1"]).status.success());
    fs::write(d.join("b.txt"), "b edited").unwrap();
    let held = fs::OpenOptions::new()
        .read(true)
        .share_mode(0)
        .open(d.join("b.txt"))
        .unwrap();
    (t, held)
}

fn assert_plain(out: &std::process::Output) {
    assert!(!out.status.success(), "must still fail");
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(!text.contains("os error"), "raw OS text shown: {text}");
    assert!(!text.contains("Failed to "), "internal chain shown: {text}");
    assert!(
        text.contains("b.txt"),
        "the locked file is not named: {text}"
    );
    assert!(
        text.contains("in use by another program") || text.contains("denied"),
        "not a plain sentence: {text}"
    );
}

#[test]
fn add_over_a_locked_file_is_plain_and_names_it() {
    let (t, _held) = locked_repo();
    assert_plain(&run(t.path(), &["add", "--all"]));
}

#[test]
fn reset_hard_over_a_locked_file_is_plain_and_names_it() {
    let (t, _held) = locked_repo();
    assert_plain(&run(t.path(), &["reset", "--hard", "HEAD"]));
}

#[test]
fn verbose_still_shows_the_technical_details() {
    let (t, _held) = locked_repo();
    let out = run(t.path(), &["--verbose", "add", "--all"]);
    assert!(!out.status.success());
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(
        err.contains("os error"),
        "--verbose lost the raw cause: {err}"
    );
}

#[test]
fn machine_mode_keeps_the_code_and_the_details() {
    let (t, _held) = locked_repo();
    let out = run(t.path(), &["--machine", "add", "--all"]);
    assert!(!out.status.success());
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(text.contains("file_in_use"), "code lost: {text}");
    assert!(text.contains("os error"), "machine details lost: {text}");
}
