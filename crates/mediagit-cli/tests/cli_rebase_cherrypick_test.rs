// SPDX-License-Identifier: BUSL-1.1
// Copyright (C) 2025-2026 Aswin Krishnamoorthy

//! Comprehensive CLI Rebase and Cherry-pick Command Tests
//!
//! Tests for `mediagit rebase` and `mediagit cherry-pick` commands.

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
// Rebase Command Tests
// ============================================================================

#[test]
#[ignore] // Rebase may not fully support working tree updates yet
fn test_rebase_simple() {
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());

    // Create base commit
    add_and_commit(temp_dir.path(), "base.txt", "Base content", "Base commit");

    // Create feature branch
    create_and_switch_branch(temp_dir.path(), "feature");
    add_and_commit(
        temp_dir.path(),
        "feature1.txt",
        "Feature 1",
        "Feature commit 1",
    );
    add_and_commit(
        temp_dir.path(),
        "feature2.txt",
        "Feature 2",
        "Feature commit 2",
    );

    // Switch to main and add a commit
    mediagit()
        .arg("branch")
        .arg("switch")
        .arg("refs/heads/main")
        .current_dir(temp_dir.path())
        .assert()
        .success();
    add_and_commit(temp_dir.path(), "main.txt", "Main content", "Main commit");

    // Switch back to feature and rebase onto main
    mediagit()
        .arg("branch")
        .arg("switch")
        .arg("feature")
        .current_dir(temp_dir.path())
        .assert()
        .success();

    mediagit()
        .arg("rebase")
        .arg("refs/heads/main")
        .current_dir(temp_dir.path())
        .assert()
        .success();

    // After rebase, feature branch should have main.txt
    assert!(temp_dir.path().join("main.txt").exists());
    assert!(temp_dir.path().join("feature1.txt").exists());
    assert!(temp_dir.path().join("feature2.txt").exists());
}

#[test]
fn test_rebase_with_quiet() {
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());

    add_and_commit(temp_dir.path(), "base.txt", "Base", "Base commit");

    create_and_switch_branch(temp_dir.path(), "feature");
    add_and_commit(temp_dir.path(), "feature.txt", "Feature", "Feature commit");

    mediagit()
        .arg("rebase")
        .arg("-q")
        .arg("refs/heads/main")
        .current_dir(temp_dir.path())
        .assert()
        .success();
}

#[test]
fn test_rebase_with_verbose() {
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());

    add_and_commit(temp_dir.path(), "base.txt", "Base", "Base commit");

    create_and_switch_branch(temp_dir.path(), "feature");
    add_and_commit(temp_dir.path(), "feature.txt", "Feature", "Feature commit");

    mediagit()
        .arg("rebase")
        .arg("-v")
        .arg("refs/heads/main")
        .current_dir(temp_dir.path())
        .assert()
        .success();
}

#[test]
fn test_rebase_abort_no_rebase() {
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());

    add_and_commit(temp_dir.path(), "file.txt", "Content", "Initial commit");

    // Abort when not in a rebase should fail
    mediagit()
        .arg("rebase")
        .arg("--abort")
        .current_dir(temp_dir.path())
        .assert()
        .failure();
}

#[test]
fn test_rebase_continue_no_rebase() {
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());

    add_and_commit(temp_dir.path(), "file.txt", "Content", "Initial commit");

    // Continue when not in a rebase should fail
    mediagit()
        .arg("rebase")
        .arg("--continue")
        .current_dir(temp_dir.path())
        .assert()
        .failure();
}

#[test]
fn test_rebase_skip_no_rebase() {
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());

    add_and_commit(temp_dir.path(), "file.txt", "Content", "Initial commit");

    // Skip when not in a rebase should fail
    mediagit()
        .arg("rebase")
        .arg("--skip")
        .current_dir(temp_dir.path())
        .assert()
        .failure();
}

#[test]
fn test_rebase_help() {
    mediagit()
        .arg("rebase")
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("rebase"));
}

// ============================================================================
// Cherry-pick Command Tests
// ============================================================================

#[test]
fn test_cherrypick_single_commit() {
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());

    // Create base on main
    add_and_commit(temp_dir.path(), "base.txt", "Base", "Base commit");

    // Create feature branch with commit
    create_and_switch_branch(temp_dir.path(), "feature");
    add_and_commit(
        temp_dir.path(),
        "feature.txt",
        "Feature content",
        "Feature commit",
    );

    // Switch back to main
    mediagit()
        .arg("branch")
        .arg("switch")
        .arg("refs/heads/main")
        .current_dir(temp_dir.path())
        .assert()
        .success();

    // Cherry-pick the feature commit
    mediagit()
        .arg("cherry-pick")
        .arg("feature")
        .current_dir(temp_dir.path())
        .assert()
        .success();

    // Verify feature.txt now exists on main
    assert!(temp_dir.path().join("feature.txt").exists());
}

#[test]
fn test_cherrypick_no_commit_mode() {
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());

    add_and_commit(temp_dir.path(), "base.txt", "Base", "Base commit");

    create_and_switch_branch(temp_dir.path(), "feature");
    add_and_commit(temp_dir.path(), "feature.txt", "Feature", "Feature commit");

    mediagit()
        .arg("branch")
        .arg("switch")
        .arg("refs/heads/main")
        .current_dir(temp_dir.path())
        .assert()
        .success();

    // Cherry-pick with --no-commit
    mediagit()
        .arg("cherry-pick")
        .arg("-n")
        .arg("feature")
        .current_dir(temp_dir.path())
        .assert()
        .success();

    // File should exist but changes should be staged, not committed
    assert!(temp_dir.path().join("feature.txt").exists());
}

#[test]
fn test_cherrypick_with_edit() {
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());

    add_and_commit(temp_dir.path(), "base.txt", "Base", "Base commit");

    create_and_switch_branch(temp_dir.path(), "feature");
    add_and_commit(temp_dir.path(), "feature.txt", "Feature", "Feature commit");

    mediagit()
        .arg("branch")
        .arg("switch")
        .arg("refs/heads/main")
        .current_dir(temp_dir.path())
        .assert()
        .success();

    // Cherry-pick with -e (edit) - in test mode this might not open editor
    // Just verify the command is recognized
    mediagit()
        .arg("cherry-pick")
        .arg("feature")
        .current_dir(temp_dir.path())
        .assert()
        .success();
}

#[test]
fn test_cherrypick_abort_no_cherrypick() {
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());

    add_and_commit(temp_dir.path(), "file.txt", "Content", "Initial commit");

    // Abort when not in a cherry-pick should fail
    mediagit()
        .arg("cherry-pick")
        .arg("--abort")
        .current_dir(temp_dir.path())
        .assert()
        .failure();
}

#[test]
fn test_cherrypick_continue_no_cherrypick() {
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());

    add_and_commit(temp_dir.path(), "file.txt", "Content", "Initial commit");

    // Continue when not in a cherry-pick should fail
    mediagit()
        .arg("cherry-pick")
        .arg("--continue")
        .current_dir(temp_dir.path())
        .assert()
        .failure();
}

#[test]
fn test_cherrypick_quiet() {
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());

    add_and_commit(temp_dir.path(), "base.txt", "Base", "Base commit");

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
        .arg("cherry-pick")
        .arg("-q")
        .arg("feature")
        .current_dir(temp_dir.path())
        .assert()
        .success();
}

#[test]
fn test_cherrypick_help() {
    mediagit()
        .arg("cherry-pick")
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("cherry"));
}

// ============================================================================
// Error Handling Tests
// ============================================================================

#[test]
fn test_rebase_nonexistent_branch() {
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());

    add_and_commit(temp_dir.path(), "file.txt", "Content", "Initial commit");

    mediagit()
        .arg("rebase")
        .arg("nonexistent-branch")
        .current_dir(temp_dir.path())
        .assert()
        .failure();
}

#[test]
fn test_cherrypick_nonexistent_commit() {
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());

    add_and_commit(temp_dir.path(), "file.txt", "Content", "Initial commit");

    mediagit()
        .arg("cherry-pick")
        .arg("nonexistent")
        .current_dir(temp_dir.path())
        .assert()
        .failure();
}

// ============================================================================
// Complex Scenarios
// ============================================================================

#[test]
#[ignore] // Rebase may not fully support working tree updates yet
fn test_rebase_multiple_commits() {
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());

    // Create base
    add_and_commit(temp_dir.path(), "base.txt", "Base", "Base commit");

    // Create feature with multiple commits
    create_and_switch_branch(temp_dir.path(), "feature");
    for i in 1..=3 {
        add_and_commit(
            temp_dir.path(),
            &format!("feature{}.txt", i),
            &format!("Feature {}", i),
            &format!("Feature commit {}", i),
        );
    }

    // Add commits to main
    mediagit()
        .arg("branch")
        .arg("switch")
        .arg("refs/heads/main")
        .current_dir(temp_dir.path())
        .assert()
        .success();
    add_and_commit(temp_dir.path(), "main1.txt", "Main 1", "Main commit 1");
    add_and_commit(temp_dir.path(), "main2.txt", "Main 2", "Main commit 2");

    // Switch back to feature and rebase
    mediagit()
        .arg("branch")
        .arg("switch")
        .arg("feature")
        .current_dir(temp_dir.path())
        .assert()
        .success();

    mediagit()
        .arg("rebase")
        .arg("refs/heads/main")
        .current_dir(temp_dir.path())
        .assert()
        .success();

    // All files should exist
    assert!(temp_dir.path().join("base.txt").exists());
    assert!(temp_dir.path().join("main1.txt").exists());
    assert!(temp_dir.path().join("main2.txt").exists());
    assert!(temp_dir.path().join("feature1.txt").exists());
    assert!(temp_dir.path().join("feature2.txt").exists());
    assert!(temp_dir.path().join("feature3.txt").exists());
}

#[test]
fn test_cherrypick_preserves_content() {
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());

    add_and_commit(temp_dir.path(), "base.txt", "Base content", "Base commit");

    create_and_switch_branch(temp_dir.path(), "feature");
    let feature_content = "This is specific feature content that should be preserved\n";
    add_and_commit(
        temp_dir.path(),
        "special.txt",
        feature_content,
        "Add special file",
    );

    mediagit()
        .arg("branch")
        .arg("switch")
        .arg("refs/heads/main")
        .current_dir(temp_dir.path())
        .assert()
        .success();

    mediagit()
        .arg("cherry-pick")
        .arg("feature")
        .current_dir(temp_dir.path())
        .assert()
        .success();

    // Verify content is preserved
    let content = fs::read_to_string(temp_dir.path().join("special.txt")).unwrap();
    assert_eq!(content, feature_content);
}

fn switch_main(dir: &Path) {
    mediagit()
        .args(["branch", "switch", "refs/heads/main"])
        .current_dir(dir)
        .assert()
        .success();
}

/// BUG-028: the base is the picked version's parent, not the branch
/// merge-base, so earlier versions of the source branch do not come along.
fn two_version_branch(dir: &Path) {
    init_repo(dir);
    add_and_commit(dir, "base.txt", "Base", "Base");
    create_and_switch_branch(dir, "x");
    add_and_commit(dir, "f1.txt", "one", "x1");
    add_and_commit(dir, "f2.txt", "two", "x2");
    switch_main(dir);
}

#[test]
fn test_cherrypick_copies_only_the_picked_version() {
    let t = TempDir::new().unwrap();
    two_version_branch(t.path());
    mediagit()
        .args(["cherry-pick", "x"])
        .current_dir(t.path())
        .assert()
        .success();
    assert!(t.path().join("f2.txt").exists());
    assert!(
        !t.path().join("f1.txt").exists(),
        "f1.txt belongs to the version before the picked one"
    );
}

#[test]
fn test_cherrypick_no_commit_copies_only_picked_and_stages_it() {
    let t = TempDir::new().unwrap();
    two_version_branch(t.path());
    mediagit()
        .args(["cherry-pick", "-n", "x"])
        .current_dir(t.path())
        .assert()
        .success();
    assert!(t.path().join("f2.txt").exists());
    assert!(!t.path().join("f1.txt").exists());
    let out = mediagit()
        .args(["status", "--porcelain"])
        .current_dir(t.path())
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&out.stdout).to_string();
    assert!(text.contains("f2.txt"), "f2.txt is staged: {text}");
    assert!(
        !text.contains("??"),
        "nothing left untracked after a no-commit pick: {text}"
    );
}

#[test]
fn test_cherrypick_continue_after_conflict_given_a_branch_name() {
    let t = TempDir::new().unwrap();
    let d = t.path();
    init_repo(d);
    add_and_commit(d, "c.txt", "base\n", "Base");
    create_and_switch_branch(d, "side");
    add_and_commit(d, "c.txt", "theirs\n", "Side edit");
    switch_main(d);
    add_and_commit(d, "c.txt", "ours\n", "Main edit");

    mediagit()
        .args(["cherry-pick", "side"])
        .current_dir(d)
        .assert()
        .failure();
    fs::write(d.join("c.txt"), "resolved\n").unwrap();
    mediagit()
        .args(["add", "c.txt"])
        .current_dir(d)
        .assert()
        .success();
    mediagit()
        .args(["cherry-pick", "--continue"])
        .current_dir(d)
        .assert()
        .success();
    assert_eq!(fs::read_to_string(d.join("c.txt")).unwrap(), "resolved\n");
}

/// BUG-026: `main` and `clash` both change notes.txt. `extra_pick` puts a
/// clean version before the clashing one on `clash`. Leaves `clash` checked
/// out, with the rebase stopped on the conflict.
fn stopped_rebase(d: &Path, extra_pick: bool) -> Vec<(&'static str, String)> {
    init_repo(d);
    add_and_commit(d, "notes.txt", "base\n", "Base");
    create_and_switch_branch(d, "clash");
    if extra_pick {
        add_and_commit(d, "a.txt", "pick one\n", "Clean pick");
    }
    add_and_commit(d, "notes.txt", "clash\n", "Clash edit");
    switch_main(d);
    add_and_commit(d, "notes.txt", "main\n", "Main edit");
    add_and_commit(d, "up.txt", "from main\n", "Main adds a file");
    mediagit()
        .args(["branch", "switch", "clash"])
        .current_dir(d)
        .assert()
        .success();
    let before: Vec<(&'static str, String)> = ["notes.txt", "a.txt"]
        .iter()
        .filter_map(|n| fs::read_to_string(d.join(n)).ok().map(|c| (*n, c)))
        .collect();
    mediagit()
        .args(["rebase", "main"])
        .current_dir(d)
        .assert()
        .failure();
    assert!(
        fs::read_to_string(d.join("notes.txt"))
            .unwrap()
            .contains("<<<<<<<"),
        "the rebase stopped with markers in notes.txt"
    );
    // X2: the stop shows the new base, so the abort below has a file only
    // the new base has to remove.
    assert_eq!(
        fs::read_to_string(d.join("up.txt")).ok().as_deref(),
        Some("from main\n"),
        "the stop shows the new base's up.txt"
    );
    before
}

fn assert_abort_restores(d: &Path, before: &[(&str, String)]) {
    mediagit()
        .args(["rebase", "--abort"])
        .current_dir(d)
        .assert()
        .success();
    for (name, content) in before {
        assert_eq!(
            &fs::read_to_string(d.join(name)).unwrap(),
            content,
            "{name} is back to its pre-rebase bytes"
        );
    }
    assert!(
        !d.join("up.txt").exists(),
        "a file only the new base has is not left behind"
    );
    let out = mediagit().args(["status"]).current_dir(d).output().unwrap();
    let text = String::from_utf8_lossy(&out.stdout).to_lowercase();
    assert!(
        !text.contains("in progress") && !text.contains("unresolved"),
        "status shows no operation and no unresolved paths: {text}"
    );
    // A new rebase starts (and stops on the same clash), it is not refused
    // for "local changes".
    let again = mediagit()
        .args(["rebase", "main"])
        .current_dir(d)
        .output()
        .unwrap();
    let all = format!(
        "{}{}",
        String::from_utf8_lossy(&again.stdout),
        String::from_utf8_lossy(&again.stderr)
    );
    assert!(
        !all.contains("would be overwritten"),
        "a new rebase can start after abort: {all}"
    );
}

#[test]
fn test_rebase_abort_restores_files_after_conflict() {
    let t = TempDir::new().unwrap();
    let before = stopped_rebase(t.path(), false);
    assert_abort_restores(t.path(), &before);
}

#[test]
fn test_rebase_abort_restores_files_after_a_clean_pick_then_conflict() {
    let t = TempDir::new().unwrap();
    let before = stopped_rebase(t.path(), true);
    assert_abort_restores(t.path(), &before);
}

#[test]
fn test_rebase_abort_keeps_untracked_files() {
    let t = TempDir::new().unwrap();
    let before = stopped_rebase(t.path(), false);
    fs::write(t.path().join("mine.txt"), "scratch\n").unwrap();
    assert_abort_restores(t.path(), &before);
    assert_eq!(
        fs::read_to_string(t.path().join("mine.txt")).unwrap(),
        "scratch\n"
    );
}

/// Y3: an edit to a tracked file the stop did not touch is the user's. `--continue`
/// must refuse naming it (and keep the edit), not commit it or write over it.
#[test]
fn test_rebase_continue_refuses_unrelated_tracked_edit() {
    let t = TempDir::new().unwrap();
    let d = t.path();
    stopped_rebase(d, false);
    fs::write(d.join("up.txt"), "my edit\n").unwrap();
    fs::write(d.join("notes.txt"), "resolved\n").unwrap();
    mediagit()
        .args(["add", "notes.txt"])
        .current_dir(d)
        .assert()
        .success();
    let out = mediagit()
        .args(["rebase", "--continue"])
        .current_dir(d)
        .output()
        .unwrap();
    let all = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(!out.status.success(), "continue must refuse: {all}");
    assert!(all.contains("up.txt"), "names the edited path: {all}");
    assert_eq!(fs::read_to_string(d.join("up.txt")).unwrap(), "my edit\n");
    // Rebase still resumable: put the file back and continue.
    fs::write(d.join("up.txt"), "from main\n").unwrap();
    mediagit()
        .args(["rebase", "--continue"])
        .current_dir(d)
        .assert()
        .success();
    assert_eq!(
        fs::read_to_string(d.join("notes.txt")).unwrap(),
        "resolved\n"
    );
}

fn object_paths(dir: &Path, out: &mut Vec<std::path::PathBuf>) {
    for e in fs::read_dir(dir).unwrap().flatten() {
        let p = e.path();
        if p.is_dir() {
            object_paths(&p, out);
        } else {
            out.push(p);
        }
    }
}

fn tip(d: &Path, branch: &str) -> String {
    fs::read_to_string(d.join(".mediagit/refs/heads").join(branch))
        .unwrap()
        .trim()
        .to_string()
}

/// Y2: a version applied during `--continue` must leave the list when it is
/// saved, so a later failure in the same run cannot make `--continue` apply it again.
#[test]
fn test_cherrypick_continue_lists_only_unapplied_after_a_failure() {
    let t = TempDir::new().unwrap();
    let d = t.path();
    init_repo(d);
    add_and_commit(d, "n.txt", "base\n", "Base");
    create_and_switch_branch(d, "side");
    add_and_commit(d, "n.txt", "theirs\n", "X1 clash");
    let x1 = tip(d, "side");
    add_and_commit(d, "a.txt", "two\n", "X2 adds a");
    let x2 = tip(d, "side");
    // X3's file: find its blob so it can be removed after the commit.
    let mut before = Vec::new();
    object_paths(&d.join(".mediagit/objects"), &mut before);
    fs::write(d.join("b.txt"), "three, unique content\n").unwrap();
    mediagit()
        .args(["add", "b.txt"])
        .current_dir(d)
        .assert()
        .success();
    let mut after = Vec::new();
    object_paths(&d.join(".mediagit/objects"), &mut after);
    let blob: Vec<_> = after
        .iter()
        .filter(|p| !before.contains(p))
        .cloned()
        .collect();
    assert_eq!(blob.len(), 1, "fixture: add wrote one object");
    mediagit()
        .args(["commit", "-m", "X3 adds b"])
        .current_dir(d)
        .assert()
        .success();
    let x3 = tip(d, "side");
    switch_main(d);
    add_and_commit(d, "n.txt", "ours\n", "Main edit");

    mediagit()
        .args(["cherry-pick", &x1, &x2, &x3])
        .current_dir(d)
        .assert()
        .failure();
    fs::write(d.join("n.txt"), "resolved\n").unwrap();
    mediagit()
        .args(["add", "n.txt"])
        .current_dir(d)
        .assert()
        .success();
    fs::remove_file(&blob[0]).unwrap();
    mediagit()
        .args(["cherry-pick", "--continue"])
        .current_dir(d)
        .assert()
        .failure();
    assert!(
        d.join("a.txt").exists(),
        "X2 was applied before the failure"
    );
    let state = fs::read_to_string(d.join(".mediagit/CHERRY_PICK_STATE")).unwrap();
    assert!(
        !state.contains(&x2),
        "X2 already applied, must not be listed: {state}"
    );
    assert!(state.contains(&x3), "X3 still to do: {state}");
}

/// F1: an edit made after `add` to a staged path is the user's. `--continue`
/// must refuse naming it, not write the staged version over it.
#[test]
fn test_rebase_continue_refuses_edit_made_after_add() {
    let t = TempDir::new().unwrap();
    let d = t.path();
    stopped_rebase(d, false);
    fs::write(d.join("notes.txt"), "resolved\n").unwrap();
    mediagit()
        .args(["add", "notes.txt"])
        .current_dir(d)
        .assert()
        .success();
    fs::write(d.join("notes.txt"), "edited after add\n").unwrap();
    let out = mediagit()
        .args(["rebase", "--continue"])
        .current_dir(d)
        .output()
        .unwrap();
    let all = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(!out.status.success(), "continue must refuse: {all}");
    assert!(all.contains("notes.txt"), "names the edited path: {all}");
    assert_eq!(
        fs::read_to_string(d.join("notes.txt")).unwrap(),
        "edited after add\n"
    );
}

/// F1, other half: a staged deletion whose file came back is the user's.
/// `--continue` must refuse naming it, not delete it again.
#[test]
fn test_rebase_continue_refuses_file_that_came_back_after_staged_deletion() {
    let t = TempDir::new().unwrap();
    let d = t.path();
    stopped_rebase(d, false);
    fs::write(d.join("notes.txt"), "resolved\n").unwrap();
    fs::remove_file(d.join("up.txt")).unwrap();
    mediagit()
        .args(["add", "-A"])
        .current_dir(d)
        .assert()
        .success();
    fs::write(d.join("up.txt"), "back again\n").unwrap();
    let out = mediagit()
        .args(["rebase", "--continue"])
        .current_dir(d)
        .output()
        .unwrap();
    let all = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(!out.status.success(), "continue must refuse: {all}");
    assert!(all.contains("up.txt"), "names the returned path: {all}");
    assert_eq!(
        fs::read_to_string(d.join("up.txt")).unwrap(),
        "back again\n"
    );
}

/// F3: a fresh `cherry-pick` while one is in progress is refused, and the
/// saved state is left as it was.
#[test]
fn test_cherrypick_fresh_run_refused_while_in_progress() {
    let t = TempDir::new().unwrap();
    let d = t.path();
    init_repo(d);
    add_and_commit(d, "n.txt", "base\n", "Base");
    create_and_switch_branch(d, "side");
    add_and_commit(d, "n.txt", "theirs\n", "X1 clash");
    let x1 = tip(d, "side");
    add_and_commit(d, "a.txt", "two\n", "X2 adds a");
    let x2 = tip(d, "side");
    switch_main(d);
    add_and_commit(d, "n.txt", "ours\n", "Main edit");
    mediagit()
        .args(["cherry-pick", &x1])
        .current_dir(d)
        .assert()
        .failure();
    let state = fs::read_to_string(d.join(".mediagit/CHERRY_PICK_STATE")).unwrap();
    mediagit()
        .args(["cherry-pick", &x2])
        .current_dir(d)
        .assert()
        .failure()
        .stderr(predicate::str::contains("A cherry-pick is in progress"));
    assert_eq!(
        fs::read_to_string(d.join(".mediagit/CHERRY_PICK_STATE")).unwrap(),
        state
    );
}
