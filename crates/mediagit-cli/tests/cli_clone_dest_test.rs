// SPDX-License-Identifier: BUSL-1.1
// Copyright (C) 2025-2026 Aswin Krishnamoorthy

//! BUG-010: `clone` accepts an existing empty folder, refuses a non-empty one,
//! and never deletes a folder the user made, even when the clone fails.

#![allow(clippy::unwrap_used)]

use assert_cmd::Command;
use predicates::prelude::*;
use std::fs;
use std::path::Path;
use tempfile::TempDir;

fn mediagit(dir: &Path) -> Command {
    let mut c = Command::cargo_bin("mediagit").unwrap();
    c.env("MEDIAGIT_AUTHOR_NAME", "T")
        .env("MEDIAGIT_AUTHOR_EMAIL", "t@example.com")
        .env("MEDIAGIT_NO_KEYRING", "1")
        .current_dir(dir);
    c
}

fn source_repo(root: &Path) -> std::path::PathBuf {
    let src = root.join("src");
    fs::create_dir(&src).unwrap();
    mediagit(&src).args(["init", "-q"]).assert().success();
    fs::write(src.join("a.txt"), "hello").unwrap();
    mediagit(&src).args(["add", "a.txt"]).assert().success();
    mediagit(&src)
        .args(["commit", "-m", "one"])
        .assert()
        .success();
    src
}

#[test]
fn clone_fills_an_existing_empty_folder() {
    let t = TempDir::new().unwrap();
    let src = source_repo(t.path());
    let dst = t.path().join("dst");
    fs::create_dir(&dst).unwrap();
    mediagit(t.path())
        .args(["clone", src.to_str().unwrap(), "dst"])
        .assert()
        .success();
    assert_eq!(fs::read_to_string(dst.join("a.txt")).unwrap(), "hello");
}

#[test]
fn clone_refuses_a_non_empty_folder_and_leaves_it_alone() {
    let t = TempDir::new().unwrap();
    let src = source_repo(t.path());
    let dst = t.path().join("dst");
    fs::create_dir(&dst).unwrap();
    fs::write(dst.join("mine.txt"), "keep").unwrap();
    mediagit(t.path())
        .args(["clone", src.to_str().unwrap(), "dst"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("not an empty folder"));
    assert_eq!(fs::read_to_string(dst.join("mine.txt")).unwrap(), "keep");
}

#[test]
fn a_failed_clone_keeps_the_users_empty_folder() {
    let t = TempDir::new().unwrap();
    let src = source_repo(t.path());
    // Local source, branch that does not exist: fails after the copy.
    let dst = t.path().join("dst");
    fs::create_dir(&dst).unwrap();
    mediagit(t.path())
        .args(["clone", "--branch", "nope", src.to_str().unwrap(), "dst"])
        .assert()
        .failure();
    assert!(dst.is_dir(), "the folder the user made must survive");
    assert_eq!(fs::read_dir(&dst).unwrap().count(), 0, "debris removed");

    // Remote path: nothing listens there, so it fails during setup.
    mediagit(t.path())
        .args(["clone", "http://127.0.0.1:1/repo", "dst"])
        .assert()
        .failure();
    assert!(dst.is_dir(), "the folder the user made must survive");
    assert_eq!(fs::read_dir(&dst).unwrap().count(), 0, "debris removed");
}
