// SPDX-License-Identifier: BUSL-1.1
// Copyright (C) 2025-2026 Aswin Krishnamoorthy

//! Integration tests for M5b sparse checkout (#8): `mediagit sparse-checkout
//! set/list/disable`, and the "status must not report sparse-excluded paths
//! as deleted" regression (M4's report named the exact insertion point:
//! the `deleted_files` collection block in `commands/status.rs`).

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

fn commit_file(dir: &Path, rel_path: &str, content: &str, message: &str) {
    let full = dir.join(rel_path);
    if let Some(parent) = full.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(&full, content).unwrap();
    mediagit()
        .arg("add")
        .arg(rel_path)
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

/// A repo with two files, one under `included/`, one under `excluded/`,
/// committed on `main`.
fn two_dir_repo() -> TempDir {
    let temp = TempDir::new().unwrap();
    init_repo(temp.path());
    commit_file(temp.path(), "included/keep.txt", "keep me", "add keep");
    commit_file(temp.path(), "excluded/drop.txt", "drop me", "add drop");
    temp
}

#[test]
fn sparse_set_cone_mode_removes_excluded_materializes_included() {
    let temp = two_dir_repo();
    assert!(temp.path().join("included/keep.txt").exists());
    assert!(temp.path().join("excluded/drop.txt").exists());

    mediagit()
        .arg("sparse-checkout")
        .arg("set")
        .arg("included")
        .current_dir(temp.path())
        .assert()
        .success();

    assert!(temp.path().join("included/keep.txt").exists());
    assert!(
        !temp.path().join("excluded/drop.txt").exists(),
        "set must remove newly-excluded tracked files"
    );
}

#[test]
fn sparse_set_pattern_mode_matches_glob() {
    let temp = TempDir::new().unwrap();
    init_repo(temp.path());
    commit_file(temp.path(), "a.keep", "keep", "add a");
    commit_file(temp.path(), "b.drop", "drop", "add b");

    mediagit()
        .arg("sparse-checkout")
        .arg("set")
        .arg("--patterns")
        .arg("*.keep")
        .current_dir(temp.path())
        .assert()
        .success();

    assert!(temp.path().join("a.keep").exists());
    assert!(!temp.path().join("b.drop").exists());
}

#[test]
fn sparse_list_shows_mode_and_patterns() {
    let temp = two_dir_repo();
    mediagit()
        .arg("sparse-checkout")
        .arg("set")
        .arg("included")
        .current_dir(temp.path())
        .assert()
        .success();

    mediagit()
        .arg("sparse-checkout")
        .arg("list")
        .current_dir(temp.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("cone"))
        .stdout(predicate::str::contains("included"));
}

#[test]
fn sparse_list_reports_disabled_when_no_patterns_set() {
    let temp = two_dir_repo();
    mediagit()
        .arg("sparse-checkout")
        .arg("list")
        .current_dir(temp.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("disabled"));
}

#[test]
fn sparse_disable_restores_full_tree_byte_identical() {
    let temp = two_dir_repo();
    let original = fs::read(temp.path().join("excluded/drop.txt")).unwrap();

    mediagit()
        .arg("sparse-checkout")
        .arg("set")
        .arg("included")
        .current_dir(temp.path())
        .assert()
        .success();
    assert!(!temp.path().join("excluded/drop.txt").exists());

    mediagit()
        .arg("sparse-checkout")
        .arg("disable")
        .current_dir(temp.path())
        .assert()
        .success();

    let restored_path = temp.path().join("excluded/drop.txt");
    assert!(
        restored_path.exists(),
        "disable must restore excluded files"
    );
    assert_eq!(
        fs::read(&restored_path).unwrap(),
        original,
        "restored content must be byte-identical"
    );

    mediagit()
        .arg("sparse-checkout")
        .arg("list")
        .current_dir(temp.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("disabled"));
}

#[test]
fn sparse_branch_switch_keeps_filter() {
    let temp = two_dir_repo();
    mediagit()
        .arg("sparse-checkout")
        .arg("set")
        .arg("included")
        .current_dir(temp.path())
        .assert()
        .success();
    assert!(!temp.path().join("excluded/drop.txt").exists());

    mediagit()
        .arg("branch")
        .arg("create")
        .arg("feature")
        .current_dir(temp.path())
        .assert()
        .success();
    mediagit()
        .arg("branch")
        .arg("switch")
        .arg("feature")
        .current_dir(temp.path())
        .assert()
        .success();

    commit_file(
        temp.path(),
        "included/keep.txt",
        "keep me v2",
        "update keep",
    );

    mediagit()
        .arg("branch")
        .arg("switch")
        .arg("main")
        .current_dir(temp.path())
        .assert()
        .success();

    assert!(
        !temp.path().join("excluded/drop.txt").exists(),
        "excluded file must stay absent across branch switch"
    );
    assert!(temp.path().join("included/keep.txt").exists());
}

/// Repro-first: `status` must not report a sparse-excluded path as deleted.
/// Before the M5b filter in `deleted_files`'s collection block, this file's
/// absence (removed by `sparse-checkout set`) would show up as `D` both in
/// human output and porcelain.
#[test]
fn status_does_not_report_sparse_excluded_files_as_deleted() {
    let temp = two_dir_repo();

    mediagit()
        .arg("sparse-checkout")
        .arg("set")
        .arg("included")
        .current_dir(temp.path())
        .assert()
        .success();
    assert!(!temp.path().join("excluded/drop.txt").exists());

    // Human output: no "deleted" section for the excluded file.
    mediagit()
        .arg("status")
        .current_dir(temp.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("drop.txt").not());

    // Porcelain: no "D excluded/drop.txt" line.
    let output = mediagit()
        .arg("status")
        .arg("--porcelain")
        .current_dir(temp.path())
        .output()
        .unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        !stdout.contains("drop.txt"),
        "sparse-excluded file must not appear in porcelain output at all:\n{stdout}"
    );

    // --json: not in `deleted`.
    let output = mediagit()
        .arg("status")
        .arg("--json")
        .current_dir(temp.path())
        .output()
        .unwrap();
    assert!(output.status.success());
    let json: serde_json::Value =
        serde_json::from_str(&String::from_utf8_lossy(&output.stdout)).unwrap();
    let deleted = json["deleted"].as_array().unwrap();
    assert!(
        !deleted
            .iter()
            .any(|v| v.as_str().unwrap_or("").contains("drop.txt")),
        "sparse-excluded file must not appear in --json deleted[]: {deleted:?}"
    );
}

// ---- BUG-041: whole-tree `add` must not treat sparse-excluded files as deleted ----

fn run(dir: &Path, args: &[&str]) -> String {
    let out = mediagit().args(args).current_dir(dir).output().unwrap();
    assert!(
        out.status.success(),
        "{args:?} failed: {}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}

/// keep/k.txt in the cone; big/b1.txt and big/b2.txt outside it.
fn keep_big_repo() -> TempDir {
    let temp = TempDir::new().unwrap();
    init_repo(temp.path());
    commit_file(temp.path(), "keep/k.txt", "k1", "k");
    commit_file(temp.path(), "big/b1.txt", "b1 content", "b1");
    commit_file(temp.path(), "big/b2.txt", "b2 content", "b2");
    temp
}

/// The committed tree (not just diff text) still holds both excluded files
/// with their original bytes.
fn assert_big_intact(d: &Path) {
    let tree = run(d, &["show", "--tree", "HEAD"]);
    assert!(
        tree.contains("big/b1.txt") && tree.contains("big/b2.txt"),
        "HEAD tree lost excluded files: {tree}"
    );
    assert_eq!(
        run(d, &["show", "HEAD:big/b1.txt"]).trim_end(),
        "b1 content"
    );
    assert_eq!(
        run(d, &["show", "HEAD:big/b2.txt"]).trim_end(),
        "b2 content"
    );
}

fn whole_tree_add_keeps_excluded(flag: &str) {
    let temp = keep_big_repo();
    let d = temp.path();
    run(d, &["sparse-checkout", "set", "keep"]);
    fs::write(d.join("keep/k.txt"), "k2 edited").unwrap();

    let out = run(d, &["add", flag]);
    assert!(!out.contains("deletion"), "no deletion staged: {out}");
    run(d, &["commit", "-m", "v2"]);

    let diff = run(d, &["diff", "HEAD~1", "HEAD"]);
    assert!(diff.contains("k.txt"), "edit must be committed: {diff}");
    assert_big_intact(d);

    run(d, &["sparse-checkout", "disable"]);
    assert_eq!(
        fs::read_to_string(d.join("big/b1.txt")).unwrap(),
        "b1 content"
    );
    assert_eq!(
        fs::read_to_string(d.join("big/b2.txt")).unwrap(),
        "b2 content"
    );
}

#[test]
fn sparse_add_all_does_not_stage_excluded_as_deleted() {
    whole_tree_add_keeps_excluded("--all");
}

#[test]
fn sparse_add_update_does_not_stage_excluded_as_deleted() {
    whole_tree_add_keeps_excluded("-u");
}

#[test]
fn sparse_add_all_still_stages_real_deletion_inside_cone() {
    let temp = keep_big_repo();
    let d = temp.path();
    commit_file(d, "keep/gone.txt", "bye", "gone");
    run(d, &["sparse-checkout", "set", "keep"]);
    fs::remove_file(d.join("keep/gone.txt")).unwrap();

    let out = run(d, &["add", "--all"]);
    assert!(out.contains("Staged 1 deletion"), "{out}");
    run(d, &["commit", "-m", "rm"]);
    let diff = run(d, &["diff", "HEAD~1", "HEAD"]);
    assert!(diff.contains("gone.txt"), "{diff}");
    assert!(!diff.contains("b1.txt"), "{diff}");
}

#[test]
fn non_sparse_add_all_still_stages_deletion() {
    let temp = keep_big_repo();
    let d = temp.path();
    fs::remove_file(d.join("big/b1.txt")).unwrap();
    let out = run(d, &["add", "--all"]);
    assert!(out.contains("Staged 1 deletion"), "{out}");
}

#[test]
fn sparse_diff_working_tree_does_not_list_excluded_as_deleted() {
    let temp = keep_big_repo();
    let d = temp.path();
    run(d, &["sparse-checkout", "set", "keep"]);
    let diff = run(d, &["diff"]);
    assert!(
        !diff.contains("b1.txt") && !diff.contains("b2.txt"),
        "{diff}"
    );
}

/// Other add forms under sparse checkout: the excluded files must survive a commit.
fn add_form_keeps_excluded(target: &str) {
    let temp = keep_big_repo();
    let d = temp.path();
    run(d, &["sparse-checkout", "set", "keep"]);
    fs::write(d.join("keep/k.txt"), "k2 edited").unwrap();

    // Some forms may refuse (nothing on disk); the commit still must not lose files.
    let _ = mediagit()
        .args(["add", target])
        .current_dir(d)
        .output()
        .unwrap();
    run(d, &["add", "keep"]);
    run(d, &["commit", "-m", "v2"]);
    assert_big_intact(d);
}

#[test]
fn sparse_add_dot_keeps_excluded() {
    add_form_keeps_excluded(".");
}

#[test]
fn sparse_add_included_dir_keeps_excluded() {
    add_form_keeps_excluded("keep");
}

#[test]
fn sparse_add_excluded_dir_keeps_excluded() {
    add_form_keeps_excluded("big");
}

#[test]
fn sparse_add_excluded_file_by_name_keeps_excluded() {
    add_form_keeps_excluded("big/b1.txt");
}

/// Bringing a folder back must not overwrite an untracked file the user made
/// at a path the sparse filter had hidden.
fn hidden_path_with_untracked_file() -> TempDir {
    let temp = two_dir_repo();
    mediagit()
        .args(["sparse-checkout", "set", "included"])
        .current_dir(temp.path())
        .assert()
        .success();
    assert!(!temp.path().join("excluded/drop.txt").exists());
    fs::create_dir_all(temp.path().join("excluded")).unwrap();
    fs::write(temp.path().join("excluded/drop.txt"), "my own work").unwrap();
    temp
}

#[test]
fn sparse_widen_keeps_untracked_file_at_returning_path() {
    let temp = hidden_path_with_untracked_file();
    mediagit()
        .args(["sparse-checkout", "set", "included", "excluded"])
        .current_dir(temp.path())
        .assert()
        .failure();
    assert_eq!(
        fs::read_to_string(temp.path().join("excluded/drop.txt")).unwrap(),
        "my own work"
    );
    mediagit()
        .args(["sparse-checkout", "list"])
        .current_dir(temp.path())
        .assert()
        .stdout(predicate::str::contains("excluded").not());
}

#[test]
fn sparse_disable_keeps_untracked_file_at_returning_path() {
    let temp = hidden_path_with_untracked_file();
    mediagit()
        .args(["sparse-checkout", "disable"])
        .current_dir(temp.path())
        .assert()
        .failure();
    assert_eq!(
        fs::read_to_string(temp.path().join("excluded/drop.txt")).unwrap(),
        "my own work"
    );
    mediagit()
        .args(["sparse-checkout", "list"])
        .current_dir(temp.path())
        .assert()
        .stdout(predicate::str::contains("Mode"));
}
