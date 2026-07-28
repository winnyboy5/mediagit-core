// MediaGit - Git for Media Files
// Copyright (C) 2025 MediaGit Contributors
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU Affero General Public License as published
// by the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE. See the
// GNU Affero General Public License for more details.

//! Phase 4 — conflict state must survive a stop-and-resume.
//!
//! `rebase` and `revert` both stop on conflict and offer `--continue`, but
//! neither recorded what conflicted, and neither wrote conflict markers for
//! the user to resolve. `rebase` additionally removed the conflicting commit
//! from `commits_remaining` and persisted that *before* attempting the merge,
//! so `--continue` resumed past it — silently discarding the user's commit
//! while reporting "Successfully rebased".
//!
//! The existing QA suite only ever asserted that the *initial* conflict exits
//! non-zero. It never resolved one and continued, which is why these were
//! invisible.

use assert_cmd::Command;
use std::fs;
use std::path::Path;
use tempfile::TempDir;

#[allow(deprecated)]
fn mediagit() -> Command {
    Command::cargo_bin("mediagit").unwrap()
}

fn init_repo(dir: &Path) {
    mediagit()
        .args(["init", "-q"])
        .current_dir(dir)
        .assert()
        .success();
}

fn commit_file(dir: &Path, rel: &str, content: &str, msg: &str) {
    let full = dir.join(rel);
    if let Some(p) = full.parent() {
        fs::create_dir_all(p).unwrap();
    }
    fs::write(&full, content).unwrap();
    mediagit()
        .args(["add", rel])
        .current_dir(dir)
        .assert()
        .success();
    mediagit()
        .args(["commit", "-m", msg])
        .current_dir(dir)
        .assert()
        .success();
}

fn run(dir: &Path, args: &[&str]) -> std::process::Output {
    mediagit()
        .args(args)
        .current_dir(dir)
        .output()
        .expect("command ran")
}

/// Diverge `main` and `feature` on the same line of the same file so that
/// replaying `feature` onto `main` must conflict.
fn setup_conflicting_branches(dir: &Path) {
    init_repo(dir);
    commit_file(dir, "shared.txt", "base\n", "base");

    mediagit()
        .args(["branch", "create", "feature"])
        .current_dir(dir)
        .assert()
        .success();

    // main moves first
    commit_file(dir, "shared.txt", "main-change\n", "main edit");

    // feature edits the same line from the same base
    mediagit()
        .args(["branch", "switch", "feature"])
        .current_dir(dir)
        .assert()
        .success();
    commit_file(dir, "shared.txt", "feature-change\n", "feature edit");
    // A second, non-conflicting commit — it must survive too.
    commit_file(dir, "other.txt", "feature-only\n", "feature second");
}

// ============================================================================
// WT-4 — `rebase --continue` must not discard the conflicting commit.
// ============================================================================

#[test]
fn wt4_rebase_continue_preserves_the_conflicting_commit() {
    let temp = TempDir::new().unwrap();
    setup_conflicting_branches(temp.path());

    let out = run(temp.path(), &["rebase", "main"]);
    assert!(
        !out.status.success(),
        "rebase should have stopped on a conflict, but succeeded"
    );

    // Resolve by hand, exactly as a user would.
    fs::write(temp.path().join("shared.txt"), "resolved\n").unwrap();
    mediagit()
        .args(["add", "shared.txt"])
        .current_dir(temp.path())
        .assert()
        .success();

    let cont = run(temp.path(), &["rebase", "--continue"]);
    assert!(
        cont.status.success(),
        "rebase --continue failed: {}",
        String::from_utf8_lossy(&cont.stderr)
    );

    // The resolution must be what landed — not silently reverted to main's side.
    assert_eq!(
        fs::read_to_string(temp.path().join("shared.txt")).unwrap(),
        "resolved\n",
        "the resolved conflict content was discarded"
    );

    // And the feature branch's *other* commit must still be there.
    let log = run(temp.path(), &["log", "--oneline"]);
    let log_text = String::from_utf8_lossy(&log.stdout);
    assert!(
        log_text.contains("feature second"),
        "a non-conflicting commit was dropped by the rebase; log:\n{log_text}"
    );
    assert!(
        log_text.contains("feature edit"),
        "the CONFLICTING commit was silently discarded; log:\n{log_text}"
    );
}

/// The conflict must be visible in the working tree. Without markers there is
/// nothing for the user to resolve, so `--continue` can only ever guess.
#[test]
fn wt4_rebase_conflict_writes_markers_to_the_working_tree() {
    let temp = TempDir::new().unwrap();
    setup_conflicting_branches(temp.path());

    let out = run(temp.path(), &["rebase", "main"]);
    assert!(!out.status.success(), "expected a conflict stop");

    let content = fs::read_to_string(temp.path().join("shared.txt")).unwrap();
    assert!(
        content.contains("<<<<<<<") && content.contains(">>>>>>>"),
        "no conflict markers written; the user has nothing to resolve. Got:\n{content}"
    );
}

/// Negative control: a rebase with no conflict must still work end to end.
/// A "fix" that simply refused to rebase would satisfy the tests above.
#[test]
fn wt4_clean_rebase_still_succeeds() {
    let temp = TempDir::new().unwrap();
    init_repo(temp.path());
    commit_file(temp.path(), "base.txt", "base\n", "base");

    mediagit()
        .args(["branch", "create", "feature"])
        .current_dir(temp.path())
        .assert()
        .success();

    commit_file(temp.path(), "main-only.txt", "main\n", "main edit");

    mediagit()
        .args(["branch", "switch", "feature"])
        .current_dir(temp.path())
        .assert()
        .success();
    commit_file(temp.path(), "feature-only.txt", "feature\n", "feature edit");

    let out = run(temp.path(), &["rebase", "main"]);
    assert!(
        out.status.success(),
        "clean rebase failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    // Both sides present after replay.
    assert!(temp.path().join("main-only.txt").exists());
    assert!(temp.path().join("feature-only.txt").exists());
}
