// SPDX-License-Identifier: BUSL-1.1
// Copyright (C) 2025-2026 Aswin Krishnamoorthy

//! Comprehensive CLI Merge Command Tests
//!
//! Tests for `mediagit merge` command with all options and merge strategies.

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

fn create_and_switch_branch(dir: &Path, branch_name: &str) {
    mediagit()
        .arg("branch")
        .arg("create")
        .arg(branch_name)
        .current_dir(dir)
        .assert()
        .success();
    mediagit()
        .arg("branch")
        .arg("switch")
        .arg(branch_name)
        .current_dir(dir)
        .assert()
        .success();
}

// ============================================================================
// Basic Merge Tests
// ============================================================================

#[test]
fn test_merge_fast_forward() {
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());

    // Create initial commit on main
    add_and_commit(
        temp_dir.path(),
        "file.txt",
        "Initial content",
        "Initial commit",
    );

    // Create feature branch and add commits
    create_and_switch_branch(temp_dir.path(), "feature");
    add_and_commit(
        temp_dir.path(),
        "feature.txt",
        "Feature content",
        "Feature commit",
    );
    add_and_commit(
        temp_dir.path(),
        "feature2.txt",
        "Feature 2",
        "Feature commit 2",
    );

    // Switch back to main
    mediagit()
        .arg("branch")
        .arg("switch")
        .arg("refs/heads/main")
        .current_dir(temp_dir.path())
        .assert()
        .success();

    // Merge feature (should fast-forward)
    mediagit()
        .arg("merge")
        .arg("feature")
        .current_dir(temp_dir.path())
        .assert()
        .success();

    // Verify feature files exist on main
    assert!(temp_dir.path().join("feature.txt").exists());
    assert!(temp_dir.path().join("feature2.txt").exists());
}

#[test]
fn test_merge_no_ff() {
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());

    add_and_commit(temp_dir.path(), "file.txt", "Initial", "Initial commit");

    create_and_switch_branch(temp_dir.path(), "feature");
    add_and_commit(temp_dir.path(), "feature.txt", "Feature", "Feature commit");

    mediagit()
        .arg("branch")
        .arg("switch")
        .arg("refs/heads/main")
        .current_dir(temp_dir.path())
        .assert()
        .success();

    // Merge with --no-ff to create merge commit
    mediagit()
        .arg("merge")
        .arg("--no-ff")
        .arg("-m")
        .arg("Merge feature branch")
        .arg("feature")
        .current_dir(temp_dir.path())
        .assert()
        .success();

    // Verify merge commit exists
    mediagit()
        .arg("log")
        .arg("--oneline")
        .arg("-n")
        .arg("1")
        .current_dir(temp_dir.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("Merge"));
}

#[test]
fn test_merge_ff_only() {
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());

    add_and_commit(temp_dir.path(), "file.txt", "Initial", "Initial commit");

    create_and_switch_branch(temp_dir.path(), "feature");
    add_and_commit(temp_dir.path(), "feature.txt", "Feature", "Feature commit");

    mediagit()
        .arg("branch")
        .arg("switch")
        .arg("refs/heads/main")
        .current_dir(temp_dir.path())
        .assert()
        .success();

    // Fast-forward only should succeed
    mediagit()
        .arg("merge")
        .arg("--ff-only")
        .arg("feature")
        .current_dir(temp_dir.path())
        .assert()
        .success();
}

// ============================================================================
// Merge with Message Tests
// ============================================================================

#[test]
fn test_merge_with_message() {
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());

    add_and_commit(temp_dir.path(), "file.txt", "Initial", "Initial commit");

    create_and_switch_branch(temp_dir.path(), "feature");
    add_and_commit(temp_dir.path(), "feature.txt", "Feature", "Feature commit");

    mediagit()
        .arg("branch")
        .arg("switch")
        .arg("refs/heads/main")
        .current_dir(temp_dir.path())
        .assert()
        .success();

    mediagit()
        .arg("merge")
        .arg("-m")
        .arg("Custom merge message")
        .arg("--no-ff")
        .arg("feature")
        .current_dir(temp_dir.path())
        .assert()
        .success();

    // Verify custom message
    mediagit()
        .arg("log")
        .arg("-n")
        .arg("1")
        .current_dir(temp_dir.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("Custom merge message"));
}

// ============================================================================
// Squash Merge Tests
// ============================================================================

#[test]
#[ignore] // MediaGit may not support --squash merge
fn test_merge_squash() {
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());

    add_and_commit(temp_dir.path(), "file.txt", "Initial", "Initial commit");

    create_and_switch_branch(temp_dir.path(), "feature");
    add_and_commit(temp_dir.path(), "f1.txt", "Feature 1", "Feature commit 1");
    add_and_commit(temp_dir.path(), "f2.txt", "Feature 2", "Feature commit 2");
    add_and_commit(temp_dir.path(), "f3.txt", "Feature 3", "Feature commit 3");

    mediagit()
        .arg("branch")
        .arg("switch")
        .arg("refs/heads/main")
        .current_dir(temp_dir.path())
        .assert()
        .success();

    // Squash merge - may not be supported
    mediagit()
        .arg("merge")
        .arg("--squash")
        .arg("feature")
        .current_dir(temp_dir.path());
}

// ============================================================================
// Three-Way Merge Tests
// ============================================================================

#[test]
fn test_merge_three_way() {
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());

    // Create base commit
    add_and_commit(temp_dir.path(), "base.txt", "Base content", "Base commit");

    // Create feature branch with new file
    create_and_switch_branch(temp_dir.path(), "feature");
    add_and_commit(
        temp_dir.path(),
        "feature.txt",
        "Feature content",
        "Feature commit",
    );

    // Switch back to main and add different file
    mediagit()
        .arg("branch")
        .arg("switch")
        .arg("refs/heads/main")
        .current_dir(temp_dir.path())
        .assert()
        .success();

    add_and_commit(temp_dir.path(), "main.txt", "Main content", "Main commit");

    // Now merge feature into main (should be a three-way merge)
    mediagit()
        .arg("merge")
        .arg("-m")
        .arg("Merge feature into main")
        .arg("feature")
        .current_dir(temp_dir.path())
        .assert()
        .success();

    // Both files should exist
    assert!(temp_dir.path().join("feature.txt").exists());
    assert!(temp_dir.path().join("main.txt").exists());
}

// ============================================================================
// Merge Abort Tests
// ============================================================================

#[test]
fn test_merge_abort_no_merge() {
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());

    add_and_commit(temp_dir.path(), "file.txt", "Content", "Initial commit");

    // Abort when not in a merge should fail gracefully
    mediagit()
        .arg("merge")
        .arg("--abort")
        .current_dir(temp_dir.path())
        .assert()
        .failure();
}

// ============================================================================
// Error Handling Tests
// ============================================================================

#[test]
fn test_merge_nonexistent_branch() {
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());

    add_and_commit(temp_dir.path(), "file.txt", "Content", "Initial commit");

    mediagit()
        .arg("merge")
        .arg("nonexistent")
        .current_dir(temp_dir.path())
        .assert()
        .failure();
}

#[test]
fn test_merge_already_up_to_date() {
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());

    add_and_commit(temp_dir.path(), "file.txt", "Content", "Initial commit");

    // Create branch at same commit
    mediagit()
        .arg("branch")
        .arg("create")
        .arg("same")
        .current_dir(temp_dir.path())
        .assert()
        .success();

    // Merge should succeed (whether it says "already up to date" or not)
    mediagit()
        .arg("merge")
        .arg("same")
        .current_dir(temp_dir.path())
        .assert()
        .success();
}

#[test]
fn test_merge_help() {
    mediagit()
        .arg("merge")
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("merge"));
}

// ============================================================================
// Quiet and Verbose Tests
// ============================================================================

#[test]
fn test_merge_quiet() {
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());

    add_and_commit(temp_dir.path(), "file.txt", "Initial", "Initial commit");

    create_and_switch_branch(temp_dir.path(), "feature");
    add_and_commit(temp_dir.path(), "feature.txt", "Feature", "Feature commit");

    mediagit()
        .arg("branch")
        .arg("switch")
        .arg("refs/heads/main")
        .current_dir(temp_dir.path())
        .assert()
        .success();

    mediagit()
        .arg("merge")
        .arg("-q")
        .arg("feature")
        .current_dir(temp_dir.path())
        .assert()
        .success();
}

#[test]
fn test_merge_verbose() {
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());

    add_and_commit(temp_dir.path(), "file.txt", "Initial", "Initial commit");

    create_and_switch_branch(temp_dir.path(), "feature");
    add_and_commit(temp_dir.path(), "feature.txt", "Feature", "Feature commit");

    mediagit()
        .arg("branch")
        .arg("switch")
        .arg("refs/heads/main")
        .current_dir(temp_dir.path())
        .assert()
        .success();

    mediagit()
        .arg("merge")
        .arg("-v")
        .arg("feature")
        .current_dir(temp_dir.path())
        .assert()
        .success();
}

// ============================================================================
// QA-002 regression: `merge --continue-merge` must not leave `::stageN`
// conflict-index debris in the committed tree, and a later branch switch
// must not try to materialize a `file.bin::stage1`-style colon path (illegal
// on Windows, os error 123).
// ============================================================================

#[test]
fn test_merge_continue_purges_stage_debris_and_branch_switch_round_trips() {
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());

    // Base commit on main.
    add_and_commit(temp_dir.path(), "file.bin", "base content", "base");

    // Diverge on `side`.
    create_and_switch_branch(temp_dir.path(), "side");
    add_and_commit(temp_dir.path(), "file.bin", "side content", "side change");

    // Diverge differently on `main` (real 3-way conflict, common base present).
    mediagit()
        .arg("branch")
        .arg("switch")
        .arg("main")
        .current_dir(temp_dir.path())
        .assert()
        .success();
    add_and_commit(temp_dir.path(), "file.bin", "main content", "main change");

    // Merging `side` into `main` must conflict.
    mediagit()
        .arg("merge")
        .arg("side")
        .current_dir(temp_dir.path())
        .assert()
        .failure();

    // Resolve and continue the merge.
    fs::write(temp_dir.path().join("file.bin"), "resolved content").unwrap();
    mediagit()
        .arg("add")
        .arg("file.bin")
        .current_dir(temp_dir.path())
        .assert()
        .success();
    mediagit()
        .arg("merge")
        .arg("--continue")
        .current_dir(temp_dir.path())
        .assert()
        .success();

    // The committed merge tree must not contain any `::stageN` debris paths.
    let show = mediagit()
        .arg("show")
        .arg("HEAD")
        .current_dir(temp_dir.path())
        .output()
        .unwrap();
    let show_text = String::from_utf8_lossy(&show.stdout);
    assert!(
        !show_text.contains("::stage"),
        "committed tree must not contain stage-debris entries: {show_text}"
    );

    // Branch-switch round trip: away to `side` (no debris in its tree), then
    // back to `main` (the merge commit's tree). Pre-fix this failed with
    // "os error 123" trying to materialize `file.bin::stage1/2`.
    mediagit()
        .arg("branch")
        .arg("switch")
        .arg("side")
        .current_dir(temp_dir.path())
        .assert()
        .success();
    mediagit()
        .arg("branch")
        .arg("switch")
        .arg("main")
        .current_dir(temp_dir.path())
        .assert()
        .success();

    assert!(temp_dir.path().join("file.bin").exists());
    assert!(!temp_dir.path().join("file.bin::stage1").exists());
    assert!(!temp_dir.path().join("file.bin::stage2").exists());
}

/// Two branches that both change `file.bin`; leaves `main` mid-merge.
fn conflicted_merge(dir: &Path) {
    init_repo(dir);
    add_and_commit(dir, "file.bin", "base content", "base");
    create_and_switch_branch(dir, "side");
    add_and_commit(dir, "file.bin", "side content", "side change");
    mediagit()
        .args(["branch", "switch", "main"])
        .current_dir(dir)
        .assert()
        .success();
    add_and_commit(dir, "file.bin", "main content", "main change");
    mediagit()
        .args(["merge", "side"])
        .current_dir(dir)
        .assert()
        .failure();
}

#[test]
fn test_merge_continue_refuses_while_conflicts_remain() {
    let temp_dir = TempDir::new().unwrap();
    let dir = temp_dir.path();
    conflicted_merge(dir);

    let out = mediagit()
        .args(["merge", "--continue"])
        .current_dir(dir)
        .assert()
        .failure();
    let err = String::from_utf8_lossy(&out.get_output().stderr).to_string();
    assert!(err.contains("file.bin"), "names the unresolved path: {err}");
    assert!(
        dir.join(".mediagit/MERGE_HEAD").exists(),
        "merge still open"
    );
}

#[test]
fn test_merge_abort_and_continue_refuse_during_sequencer_operations() {
    for (marker, is_dir, command) in [
        ("rebase-apply", true, "rebase"),
        ("CHERRY_PICK_STATE", false, "cherry-pick"),
        ("REVERT_STATE", false, "revert"),
    ] {
        let temp_dir = TempDir::new().unwrap();
        let dir = temp_dir.path();
        conflicted_merge(dir);
        let path = dir.join(".mediagit").join(marker);
        if is_dir {
            fs::create_dir(&path).unwrap();
        } else {
            fs::write(&path, "x").unwrap();
        }
        let before = fs::read_to_string(dir.join("file.bin")).unwrap();

        for flag in ["--abort", "--continue"] {
            let out = mediagit()
                .args(["merge", flag])
                .current_dir(dir)
                .assert()
                .failure();
            let err = String::from_utf8_lossy(&out.get_output().stderr).to_string();
            assert!(
                err.contains(&format!("{command} {flag}")),
                "{marker} {flag}: names the right command: {err}"
            );
        }
        assert!(path.exists(), "{marker} untouched");
        assert!(
            dir.join(".mediagit/MERGE_HEAD").exists(),
            "{marker}: merge state untouched"
        );
        assert_eq!(fs::read_to_string(dir.join("file.bin")).unwrap(), before);
    }
}

// BUG-024: `--ff-only` on diverged branches must refuse and change nothing.
#[test]
fn test_merge_ff_only_refuses_diverged_branches() {
    let temp_dir = TempDir::new().unwrap();
    let dir = temp_dir.path();
    init_repo(dir);
    add_and_commit(dir, "base.txt", "Base", "Base commit");
    create_and_switch_branch(dir, "feature");
    add_and_commit(dir, "feature.txt", "Feature", "Feature commit");
    mediagit()
        .args(["branch", "switch", "refs/heads/main"])
        .current_dir(dir)
        .assert()
        .success();
    add_and_commit(dir, "main.txt", "Main", "Main commit");

    let tip = dir.join(".mediagit/refs/heads/main");
    let before = fs::read_to_string(&tip).unwrap();

    mediagit()
        .args(["merge", "--ff-only", "feature"])
        .current_dir(dir)
        .assert()
        .failure()
        .stderr(predicate::str::contains("ast-forward"));

    assert_eq!(fs::read_to_string(&tip).unwrap(), before, "tip moved");
    assert!(!dir.join(".mediagit/MERGE_HEAD").exists());
    assert!(!dir.join("feature.txt").exists(), "working files changed");
    assert_eq!(fs::read_to_string(dir.join("main.txt")).unwrap(), "Main");
}

fn main_tip(dir: &Path) -> String {
    fs::read_to_string(dir.join(".mediagit/refs/heads/main")).unwrap()
}

fn log_text(dir: &Path) -> String {
    let out = mediagit().arg("log").current_dir(dir).assert().success();
    String::from_utf8_lossy(&out.get_output().stdout).to_string()
}

// BUG-025 / C-1: a branch with nothing new is "Already up to date", no commit.
#[test]
fn test_merge_already_up_to_date_makes_no_commit() {
    let temp_dir = TempDir::new().unwrap();
    let dir = temp_dir.path();
    init_repo(dir);
    add_and_commit(dir, "file.txt", "Content", "Initial commit");
    mediagit()
        .args(["branch", "create", "twin"])
        .current_dir(dir)
        .assert()
        .success();
    let before = main_tip(dir);

    // Same commit, plain and --ff-only.
    for flags in [vec![], vec!["--ff-only"]] {
        mediagit()
            .arg("merge")
            .args(&flags)
            .arg("twin")
            .current_dir(dir)
            .assert()
            .success()
            .stdout(predicate::str::contains("Already up to date"));
        assert_eq!(main_tip(dir), before, "tip moved for {flags:?}");
    }

    // Theirs is an ancestor of ours.
    add_and_commit(dir, "more.txt", "More", "Second commit");
    let before = main_tip(dir);
    for flags in [vec![], vec!["--ff-only"]] {
        mediagit()
            .arg("merge")
            .args(&flags)
            .arg("twin")
            .current_dir(dir)
            .assert()
            .success()
            .stdout(predicate::str::contains("Already up to date"));
        assert_eq!(main_tip(dir), before, "tip moved for {flags:?}");
    }
}

/// `main` and `other` each have one new version, in different files.
fn diverged_clean(dir: &Path) {
    init_repo(dir);
    add_and_commit(dir, "base.txt", "Base", "Base commit");
    create_and_switch_branch(dir, "other");
    add_and_commit(dir, "other.txt", "Other", "Other commit");
    mediagit()
        .args(["branch", "switch", "main"])
        .current_dir(dir)
        .assert()
        .success();
    add_and_commit(dir, "main.txt", "Main", "Main commit");
}

// BUG-030: --no-commit leaves the merge applied and waiting to be finished.
#[test]
fn test_merge_no_commit_applies_and_continue_finishes() {
    let temp_dir = TempDir::new().unwrap();
    let dir = temp_dir.path();
    diverged_clean(dir);
    let before = main_tip(dir);

    mediagit()
        .args(["merge", "--no-commit", "other"])
        .current_dir(dir)
        .assert()
        .success();

    assert_eq!(main_tip(dir), before, "no-commit must not move the tip");
    assert_eq!(fs::read_to_string(dir.join("other.txt")).unwrap(), "Other");
    assert!(dir.join(".mediagit/MERGE_HEAD").exists(), "merge waiting");

    mediagit()
        .args(["merge", "--continue"])
        .current_dir(dir)
        .assert()
        .success();
    assert!(!dir.join(".mediagit/MERGE_HEAD").exists());
    assert_ne!(main_tip(dir), before);
    assert_eq!(fs::read_to_string(dir.join("other.txt")).unwrap(), "Other");
    assert_eq!(fs::read_to_string(dir.join("main.txt")).unwrap(), "Main");
    assert!(log_text(dir).contains("Merge branch 'other' into main"));
}

// BUG-030: --no-commit then --abort puts everything back.
#[test]
fn test_merge_no_commit_abort_restores() {
    let temp_dir = TempDir::new().unwrap();
    let dir = temp_dir.path();
    diverged_clean(dir);
    let before = main_tip(dir);

    mediagit()
        .args(["merge", "--no-commit", "other"])
        .current_dir(dir)
        .assert()
        .success();
    mediagit()
        .args(["merge", "--abort"])
        .current_dir(dir)
        .assert()
        .success();

    assert_eq!(main_tip(dir), before);
    assert!(!dir.join("other.txt").exists());
    assert!(!dir.join(".mediagit/MERGE_HEAD").exists());
    // The index is restored too: nothing from the merge stays staged.
    let status = mediagit().arg("status").current_dir(dir).output().unwrap();
    let status = String::from_utf8_lossy(&status.stdout).to_string();
    assert!(!status.contains("other.txt"), "{status}");
}

// BUG-030: a staged squash is saved with `commit`; the next step says so.
#[test]
fn test_merge_squash_no_commit_points_to_commit() {
    let temp_dir = TempDir::new().unwrap();
    let dir = temp_dir.path();
    diverged_clean(dir);

    let out = mediagit()
        .args(["merge", "--squash", "--no-commit", "other"])
        .current_dir(dir)
        .assert()
        .success();
    let out = String::from_utf8_lossy(&out.get_output().stdout).to_string();
    assert!(out.contains("commit -m"), "{out}");
    assert!(!out.contains("--continue"), "{out}");
    assert!(dir.join("other.txt").exists());
}

// A staged squash is saved with a plain `commit`: the saved text is the
// default message, and it is gone once the version exists.
#[test]
fn test_squash_then_plain_commit_uses_merge_msg() {
    let temp_dir = TempDir::new().unwrap();
    let dir = temp_dir.path();
    diverged_clean(dir);
    mediagit()
        .args(["merge", "--squash", "--no-commit", "other"])
        .current_dir(dir)
        .assert()
        .success();
    assert!(dir.join(".mediagit/MERGE_MSG").exists());

    mediagit().arg("commit").current_dir(dir).assert().success();

    let log = log_text(dir);
    assert!(
        log.contains("Squash merge branch 'other' into main"),
        "{log}"
    );
    assert!(!dir.join(".mediagit/MERGE_MSG").exists(), "MERGE_MSG left");
}

// BUG-023: the merge version is named after the branches and has the real author.
#[test]
fn test_merge_commit_message_and_author() {
    let temp_dir = TempDir::new().unwrap();
    let dir = temp_dir.path();
    diverged_clean(dir);
    mediagit()
        .args(["merge", "other"])
        .current_dir(dir)
        .assert()
        .success();
    let log = log_text(dir);
    assert!(log.contains("Merge branch 'other' into main"), "{log}");
    assert!(!log.contains("into HEAD"), "{log}");
}

// BUG-023: finishing a conflicted merge keeps the name and the real author.
#[test]
fn test_merge_continue_message_and_author() {
    let temp_dir = TempDir::new().unwrap();
    let dir = temp_dir.path();
    conflicted_merge(dir);
    mediagit()
        .args(["add", "file.bin"])
        .current_dir(dir)
        .assert()
        .success();
    mediagit()
        .args(["merge", "--continue"])
        .current_dir(dir)
        .assert()
        .success();
    let log = log_text(dir);
    assert!(log.contains("Merge branch 'side' into main"), "{log}");
    assert!(!log.contains("MediaGit User"), "{log}");
    assert!(log.contains("Test User"), "{log}");
}
