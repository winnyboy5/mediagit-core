// SPDX-License-Identifier: BUSL-1.1
// Copyright (C) 2025-2026 Aswin Krishnamoorthy

//! `log A..B` (in B, not in A) and `log A...B` (in either, not both), on the
//! human and `--machine` paths.
//!
//! History:  c1 - c2 - m3          (main)
//!                  \
//!                   f1 - f2       (feature)

use assert_cmd::Command;
use serde_json::Value;
use std::collections::BTreeSet;
use std::path::Path;
use tempfile::TempDir;

#[allow(deprecated)]
fn mediagit() -> Command {
    let mut c = Command::cargo_bin("mediagit").unwrap();
    c.env("MEDIAGIT_AUTHOR_NAME", "Test User")
        .env("MEDIAGIT_AUTHOR_EMAIL", "test@example.com");
    c
}

fn run(repo: &Path, args: &[&str]) {
    mediagit().args(args).current_dir(repo).assert().success();
}

fn commit(repo: &Path, file: &str, msg: &str) {
    std::fs::write(repo.join(file), msg).unwrap();
    run(repo, &["add", file]);
    run(repo, &["commit", "-m", msg]);
}

fn diverged() -> TempDir {
    let dir = TempDir::new().unwrap();
    let repo = dir.path();
    run(repo, &["init", "-q"]);
    commit(repo, "a.txt", "c1");
    commit(repo, "a.txt", "c2");
    run(repo, &["branch", "create", "feature"]);
    run(repo, &["branch", "switch", "feature"]);
    commit(repo, "f.txt", "f1");
    commit(repo, "f.txt", "f2");
    run(repo, &["branch", "switch", "main"]);
    commit(repo, "m.txt", "m3");
    dir
}

/// Commit summaries `log --machine <args>` lists.
fn machine(repo: &Path, args: &[&str]) -> BTreeSet<String> {
    let out = mediagit()
        .arg("--machine")
        .arg("log")
        .args(args)
        .current_dir(repo)
        .output()
        .unwrap();
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert_eq!(stdout.lines().count(), 1, "{stdout}");
    let v: Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(v["ok"], true, "{v}");
    v["data"]["commits"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["summary"].as_str().unwrap().to_owned())
        .collect()
}

/// Commit summaries the human `log --oneline <args>` prints.
fn human(repo: &Path, args: &[&str]) -> BTreeSet<String> {
    let out = mediagit()
        .args(["log", "--oneline"])
        .args(args)
        .current_dir(repo)
        .output()
        .unwrap();
    assert!(out.status.success(), "{out:?}");
    let text = String::from_utf8_lossy(&out.stdout);
    ["c1", "c2", "m3", "f1", "f2"]
        .into_iter()
        .filter(|m| text.lines().any(|l| l.trim_end().ends_with(m)))
        .map(str::to_owned)
        .collect()
}

fn set(items: &[&str]) -> BTreeSet<String> {
    items.iter().map(|s| (*s).to_owned()).collect()
}

#[test]
fn two_dot_range_lists_only_the_far_side() {
    let dir = diverged();
    let repo = dir.path();
    for list in [machine, human] {
        assert_eq!(list(repo, &["main..feature"]), set(&["f1", "f2"]));
        assert_eq!(list(repo, &["feature..main"]), set(&["m3"]));
        // Empty side means HEAD (main).
        assert_eq!(list(repo, &["..feature"]), set(&["f1", "f2"]));
        assert_eq!(list(repo, &["feature.."]), set(&["m3"]));
    }
}

#[test]
fn three_dot_range_lists_both_sides_without_the_shared_history() {
    let dir = diverged();
    let repo = dir.path();
    for list in [machine, human] {
        assert_eq!(list(repo, &["main...feature"]), set(&["f1", "f2", "m3"]));
    }
}

#[test]
fn a_path_filter_at_the_range_boundary_does_not_panic() {
    let dir = diverged();
    let repo = dir.path();
    // f1's parent (c2) is outside the range: its tree is read, not indexed.
    for list in [machine, human] {
        assert_eq!(
            list(repo, &["main..feature", "--", "f.txt"]),
            set(&["f1", "f2"])
        );
    }
}

#[test]
fn all_with_a_range_and_bad_ranges_are_refused() {
    let dir = diverged();
    let repo = dir.path();
    for args in [
        &["--all", "main..feature"][..],
        &[".."],
        &["a....b"],
        &["main..nope"],
    ] {
        mediagit()
            .arg("log")
            .args(args)
            .current_dir(repo)
            .assert()
            .failure();
    }
}
