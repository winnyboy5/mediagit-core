// SPDX-License-Identifier: BUSL-1.1
// Copyright (C) 2025-2026 Aswin Krishnamoorthy

//! `rebase <UPSTREAM> <BRANCH>` rebases BRANCH (it switches there first, as
//! git does). BRANCH used to be parsed and ignored, so the current branch was
//! rebased instead.

use assert_cmd::Command;
use serde_json::Value;
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

fn commit(repo: &Path, file: &str, body: &str, msg: &str) {
    std::fs::write(repo.join(file), body).unwrap();
    run(repo, &["add", file]);
    run(repo, &["commit", "-m", msg]);
}

fn machine(repo: &Path, args: &[&str]) -> (i32, Value) {
    let out = mediagit()
        .arg("--machine")
        .args(args)
        .current_dir(repo)
        .output()
        .unwrap();
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert_eq!(stdout.lines().count(), 1, "{stdout}");
    (
        out.status.code().unwrap(),
        serde_json::from_str(&stdout).unwrap(),
    )
}

fn head_branch(repo: &Path) -> String {
    let (_, v) = machine(repo, &["branch", "list"]);
    v["data"]["current"].as_str().unwrap_or("").to_owned()
}

fn tip(repo: &Path, rev: &str) -> String {
    let (_, v) = machine(repo, &["log", "-n", "1", rev]);
    v["data"]["commits"][0]["oid"].as_str().unwrap().to_owned()
}

/// main: base, m1 (m.txt). feature: base, f1 (f.txt). HEAD on main.
fn repo() -> TempDir {
    let dir = TempDir::new().unwrap();
    let r = dir.path();
    run(r, &["init", "-q"]);
    commit(r, "a.txt", "base", "base");
    run(r, &["branch", "create", "feature"]);
    run(r, &["branch", "switch", "feature"]);
    commit(r, "f.txt", "feature", "f1");
    run(r, &["branch", "switch", "main"]);
    commit(r, "m.txt", "main", "m1");
    dir
}

#[test]
fn the_named_branch_is_rebased_and_main_is_not_touched() {
    let dir = repo();
    let r = dir.path();
    let main_before = tip(r, "main");
    let (code, v) = machine(r, &["rebase", "main", "feature"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(head_branch(r), "feature");
    assert_eq!(tip(r, "main"), main_before, "main must not move");
    // feature now sits on main: it has both files.
    assert!(r.join("m.txt").exists() && r.join("f.txt").exists());
    let (_, v) = machine(r, &["log", "main..feature"]);
    assert_eq!(v["data"]["commits"].as_array().unwrap().len(), 1, "{v}");
}

#[test]
fn unsaved_work_the_switch_would_overwrite_refuses_before_anything_moves() {
    let dir = repo();
    let r = dir.path();
    // m.txt exists on main only: switching to feature would delete this edit.
    std::fs::write(r.join("m.txt"), "unsaved").unwrap();
    let (main_before, feature_before) = (tip(r, "main"), tip(r, "feature"));
    let (code, _) = machine(r, &["rebase", "main", "feature"]);
    assert_ne!(code, 0);
    assert_eq!(head_branch(r), "main");
    assert_eq!(tip(r, "main"), main_before);
    assert_eq!(tip(r, "feature"), feature_before);
    assert_eq!(std::fs::read_to_string(r.join("m.txt")).unwrap(), "unsaved");
}

#[test]
fn a_bad_upstream_refuses_before_switching() {
    let dir = repo();
    let r = dir.path();
    let (code, _) = machine(r, &["rebase", "no-such-upstream", "feature"]);
    assert_ne!(code, 0);
    assert_eq!(head_branch(r), "main", "a failed rebase must not switch");
    assert!(!r.join("f.txt").exists());
}

#[test]
fn a_staged_change_refuses_before_switching() {
    let dir = repo();
    let r = dir.path();
    // A new file the switch would carry: only rebase's own check refuses it.
    std::fs::write(r.join("new.txt"), "staged").unwrap();
    run(r, &["add", "new.txt"]);
    let (code, _) = machine(r, &["rebase", "main", "feature"]);
    assert_ne!(code, 0);
    assert_eq!(head_branch(r), "main", "a failed rebase must not switch");
    assert!(!r.join("f.txt").exists());
}

#[test]
fn a_missing_branch_refuses_with_nothing_moved() {
    let dir = repo();
    let r = dir.path();
    let main_before = tip(r, "main");
    let (code, _) = machine(r, &["rebase", "main", "no-such-branch"]);
    assert_ne!(code, 0);
    assert_eq!(head_branch(r), "main");
    assert_eq!(tip(r, "main"), main_before);
}

#[test]
fn an_untracked_file_upstream_would_overwrite_refuses_before_switching() {
    // dev and feature never had m.txt; main tracks it. From dev, an untracked
    // m.txt is safe for the switch to feature but not for the rebase onto main.
    let dir = TempDir::new().unwrap();
    let r = dir.path();
    run(r, &["init", "-q"]);
    commit(r, "a.txt", "base", "base");
    run(r, &["branch", "create", "dev"]);
    run(r, &["branch", "create", "feature"]);
    run(r, &["branch", "switch", "feature"]);
    commit(r, "f.txt", "feature", "f1");
    run(r, &["branch", "switch", "main"]);
    commit(r, "m.txt", "main", "m1");
    run(r, &["branch", "switch", "dev"]);
    std::fs::write(r.join("m.txt"), "mine, untracked").unwrap();
    let (code, _) = machine(r, &["rebase", "main", "feature"]);
    assert_ne!(code, 0);
    assert_eq!(head_branch(r), "dev", "a failed rebase must not switch");
    assert_eq!(
        std::fs::read_to_string(r.join("m.txt")).unwrap(),
        "mine, untracked"
    );
}

#[test]
fn a_head_relative_upstream_means_the_branch_you_are_on() {
    // main: base, m1, m2. HEAD~1 from main is m1; from feature it would be base.
    let dir = repo();
    let r = dir.path();
    commit(r, "m2.txt", "main 2", "m2");
    let (code, v) = machine(r, &["rebase", "HEAD~1", "feature"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(head_branch(r), "feature");
    assert!(
        r.join("m.txt").exists(),
        "feature must sit on m1 (main's HEAD~1)"
    );
    assert!(!r.join("m2.txt").exists(), "m2 is past main's HEAD~1");
}
