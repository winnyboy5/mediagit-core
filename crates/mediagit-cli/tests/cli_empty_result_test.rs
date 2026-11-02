// SPDX-License-Identifier: BUSL-1.1
// Copyright (C) 2025-2026 Aswin Krishnamoorthy

//! #97: cherry-pick and revert refuse an empty result; rebase drops a replayed
//! commit that adds nothing (unless `--keep-empty`). None may record an empty
//! commit and call it success.

use assert_cmd::Command;
use predicates::prelude::*;
use std::fs;
use std::path::Path;
use tempfile::TempDir;

#[allow(deprecated)]
fn mediagit() -> Command {
    let mut c = Command::cargo_bin("mediagit").unwrap();
    c.env("MEDIAGIT_AUTHOR_NAME", "Test User")
        .env("MEDIAGIT_AUTHOR_EMAIL", "test@example.com");
    c
}

fn run(dir: &Path, args: &[&str]) {
    mediagit().args(args).current_dir(dir).assert().success();
}

fn commit_file(dir: &Path, name: &str, content: &str, msg: &str) {
    fs::write(dir.join(name), content).unwrap();
    run(dir, &["add", name]);
    run(dir, &["commit", "-m", msg]);
}

fn head_oid(dir: &Path) -> String {
    fs::read_to_string(dir.join(".mediagit/refs/heads/main"))
        .or_else(|_| fs::read_to_string(dir.join(".mediagit/refs/heads/feature")))
        .unwrap()
        .trim()
        .to_string()
}

fn branch_oid(dir: &Path, branch: &str) -> String {
    fs::read_to_string(dir.join(format!(".mediagit/refs/heads/{branch}")))
        .unwrap()
        .trim()
        .to_string()
}

fn repo_with_feature_commit() -> TempDir {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    run(dir, &["init", "-q"]);
    commit_file(dir, "base.txt", "base", "base");
    run(dir, &["branch", "create", "feature"]);
    run(dir, &["branch", "switch", "feature"]);
    commit_file(dir, "f.txt", "f", "f1");
    run(dir, &["branch", "switch", "main"]);
    tmp
}

#[test]
fn cherry_pick_of_an_already_applied_change_is_refused() {
    let tmp = repo_with_feature_commit();
    let dir = tmp.path();
    run(dir, &["cherry-pick", "feature"]);
    let after_first = branch_oid(dir, "main");

    mediagit()
        .args(["cherry-pick", "feature"])
        .current_dir(dir)
        .assert()
        .failure()
        .stderr(predicate::str::contains("already on this branch"));
    assert_eq!(
        branch_oid(dir, "main"),
        after_first,
        "no commit may be created"
    );
}

#[test]
fn revert_of_an_already_reverted_change_is_refused() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    run(dir, &["init", "-q"]);
    commit_file(dir, "a.txt", "one\n", "first");
    commit_file(dir, "b.txt", "two\n", "add b");
    let add_b = head_oid(dir);

    run(dir, &["revert", &add_b]);
    let after_first = head_oid(dir);

    mediagit()
        .args(["revert", &add_b])
        .current_dir(dir)
        .assert()
        .failure()
        .stderr(predicate::str::contains("already reverted"));
    assert_eq!(head_oid(dir), after_first, "no commit may be created");
}

fn repo_where_upstream_already_has_the_change() -> TempDir {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    run(dir, &["init", "-q"]);
    commit_file(dir, "base.txt", "base", "base");
    run(dir, &["branch", "create", "feature"]);
    run(dir, &["branch", "switch", "feature"]);
    commit_file(dir, "same.txt", "same", "feature adds same.txt");
    run(dir, &["branch", "switch", "main"]);
    commit_file(dir, "same.txt", "same", "main adds same.txt");
    run(dir, &["branch", "switch", "feature"]);
    tmp
}

#[test]
fn rebase_drops_a_commit_whose_change_is_already_upstream() {
    let tmp = repo_where_upstream_already_has_the_change();
    let dir = tmp.path();
    let main_head = branch_oid(dir, "main");
    run(dir, &["rebase", "main"]);
    // The feature commit became empty and is dropped (git's behaviour).
    assert_eq!(branch_oid(dir, "feature"), main_head);
}

#[test]
fn rebase_keep_empty_keeps_the_empty_commit() {
    let tmp = repo_where_upstream_already_has_the_change();
    let dir = tmp.path();
    let main_head = branch_oid(dir, "main");
    run(dir, &["rebase", "--keep-empty", "main"]);
    assert_ne!(branch_oid(dir, "feature"), main_head);
}
