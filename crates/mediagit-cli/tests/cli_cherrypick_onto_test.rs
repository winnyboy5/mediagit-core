// SPDX-License-Identifier: BUSL-1.1
// Copyright (C) 2025-2026 Aswin Krishnamoorthy

//! `cherry-pick --onto <branch>`: copy a version onto another branch without
//! switching. The working folder, the index, HEAD and the current branch are
//! never touched; a copy that would conflict is refused with nothing changed.
//! Also: a pick on a detached HEAD moves HEAD (it used to drop the commit).

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

/// Tip commit id of `rev`.
fn tip(repo: &Path, rev: &str) -> String {
    let (code, v) = machine(repo, &["log", "-n", "1", rev]);
    assert_eq!(code, 0, "{v}");
    v["data"]["commits"][0]["oid"].as_str().unwrap().to_owned()
}

/// Content of `path` at `rev`.
fn file_at(repo: &Path, rev: &str, path: &str) -> String {
    let out = repo.join(".peek");
    run(
        repo,
        &[
            "show",
            &format!("{rev}:{path}"),
            "--output",
            out.to_str().unwrap(),
        ],
    );
    let s = std::fs::read_to_string(&out).unwrap();
    std::fs::remove_file(&out).unwrap();
    s
}

/// main: a.txt=base, then c1 (b.txt=from-main). feature: off base, f.txt.
/// HEAD stays on main.
fn repo() -> TempDir {
    let dir = TempDir::new().unwrap();
    let r = dir.path();
    run(r, &["init", "-q"]);
    commit(r, "a.txt", "base", "base");
    run(r, &["branch", "create", "feature"]);
    run(r, &["branch", "switch", "feature"]);
    commit(r, "f.txt", "feature work", "f1");
    run(r, &["branch", "switch", "main"]);
    commit(r, "b.txt", "from-main", "c1");
    dir
}

#[test]
fn copies_onto_another_branch_without_touching_files_or_head() {
    let dir = repo();
    let r = dir.path();
    // Unsaved work in the working folder and the index must survive.
    std::fs::write(r.join("a.txt"), "my unsaved edit").unwrap();
    std::fs::write(r.join("wip.txt"), "staged wip").unwrap();
    run(r, &["add", "wip.txt"]);
    let index_before = std::fs::read(r.join(".mediagit/index")).unwrap();
    let main_before = tip(r, "main");
    let feature_before = tip(r, "feature");

    let (code, v) = machine(r, &["cherry-pick", "--onto", "feature", "main"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["onto"], "feature");
    assert_eq!(v["data"]["commits"].as_array().unwrap().len(), 1);

    assert_eq!(file_at(r, "feature", "b.txt"), "from-main");
    assert_eq!(file_at(r, "feature", "f.txt"), "feature work");
    assert_ne!(tip(r, "feature"), feature_before);
    assert_eq!(tip(r, "main"), main_before);
    assert_eq!(
        std::fs::read_to_string(r.join("a.txt")).unwrap(),
        "my unsaved edit"
    );
    assert_eq!(
        std::fs::read(r.join(".mediagit/index")).unwrap(),
        index_before
    );
    let reflog = std::fs::read_to_string(r.join(".mediagit/logs/refs/heads/feature")).unwrap();
    assert!(reflog.contains("cherry-pick --onto feature"), "{reflog}");
}

#[test]
fn two_versions_are_copied_in_order() {
    let dir = repo();
    let r = dir.path();
    let c1 = tip(r, "main");
    commit(r, "b.txt", "from-main v2", "c2");
    let c2 = tip(r, "main");
    let (code, v) = machine(r, &["cherry-pick", "--onto", "feature", &c1, &c2]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["commits"].as_array().unwrap().len(), 2);
    assert_eq!(file_at(r, "feature", "b.txt"), "from-main v2");
}

#[test]
fn a_conflicting_copy_is_refused_and_changes_nothing() {
    let dir = repo();
    let r = dir.path();
    run(r, &["branch", "switch", "feature"]);
    commit(r, "a.txt", "feature side", "fa");
    run(r, &["branch", "switch", "main"]);
    commit(r, "a.txt", "main side", "ma");
    let feature_before = tip(r, "feature");

    let (code, v) = machine(r, &["cherry-pick", "--onto", "feature", "main"]);
    assert_ne!(code, 0);
    assert_eq!(v["error"]["code"], "would_conflict", "{v}");
    assert!(
        v["error"]["message"].as_str().unwrap().contains("a.txt"),
        "{v}"
    );
    assert_eq!(tip(r, "feature"), feature_before);
    assert!(!r.join(".mediagit/CHERRY_PICK_STATE").exists());
    assert_eq!(
        std::fs::read_to_string(r.join("a.txt")).unwrap(),
        "main side"
    );
}

#[test]
fn bad_targets_and_no_commit_are_refused() {
    let dir = repo();
    let r = dir.path();
    for args in [
        &["cherry-pick", "--onto", "nope", "main"][..],
        &["cherry-pick", "--onto", "feature", "--no-commit", "main"],
    ] {
        let (code, v) = machine(r, args);
        assert_ne!(code, 0, "{args:?}");
        assert_eq!(v["error"]["code"], "invalid_args", "{v}");
    }
}

#[test]
fn a_pick_on_a_detached_head_moves_head() {
    let dir = repo();
    let r = dir.path();
    let c1 = tip(r, "main");
    // Detach HEAD at feature's tip (HEAD holds the id directly).
    let feature_ref = std::fs::read(r.join(".mediagit/refs/heads/feature")).unwrap();
    std::fs::write(r.join(".mediagit/HEAD"), feature_ref).unwrap();
    // Make the working folder match feature (b.txt is main's only).
    std::fs::remove_file(r.join("b.txt")).unwrap();
    std::fs::write(r.join("f.txt"), "feature work").unwrap();
    let (code, v) = machine(r, &["cherry-pick", &c1]);
    assert_eq!(code, 0, "{v}");
    let new = v["data"]["commits"][0].as_str().unwrap().to_owned();
    assert_eq!(tip(r, "HEAD"), new, "HEAD did not move to the new commit");
    assert_eq!(
        std::fs::read_to_string(r.join("b.txt")).unwrap(),
        "from-main"
    );
}
