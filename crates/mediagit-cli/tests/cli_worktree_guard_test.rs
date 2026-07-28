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

//! Phase 1 working-tree data-loss guard (WT-1/2/3/8, UX-1).
//!
//! Every command that rewrites the working tree must refuse rather than
//! destroy uncommitted or untracked work. These tests are the safety axis:
//! they assert on the *bytes still on disk* after the command, not just on
//! the exit status.

use assert_cmd::Command;
use predicates::prelude::*;
use std::fs;
use std::path::Path;
use tempfile::TempDir;

#[allow(deprecated)]
fn mediagit() -> Command {
    Command::cargo_bin("mediagit").unwrap()
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

fn branch_create(dir: &Path, name: &str) {
    mediagit()
        .args(["branch", "create", name])
        .current_dir(dir)
        .assert()
        .success();
}

/// Short OID of the commit `n` steps back from HEAD (0 = HEAD).
/// `bisect` cannot resolve `HEAD~N` even though `log` can (see FOUND-1), so
/// tests must pass a literal OID.
fn commit_oid_back(dir: &Path, n: usize) -> String {
    let out = mediagit()
        .args(["log", "--oneline"])
        .current_dir(dir)
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let text = String::from_utf8(out).unwrap();
    text.lines()
        .nth(n)
        .unwrap_or_else(|| panic!("no commit {n} back"))
        .split_whitespace()
        .next()
        .unwrap()
        .to_string()
}

fn branch_switch(dir: &Path, name: &str) {
    mediagit()
        .args(["branch", "switch", name])
        .current_dir(dir)
        .assert()
        .success();
}

// ============================================================================
// WT-1 — untracked files must survive a working-tree rewrite
// ============================================================================

/// `reset --hard` discards *tracked* modifications by design, but it must
/// never delete an untracked file. `clean_working_directory` removes every
/// working-tree file absent from the target tree, with no tracked/untracked
/// distinction.
#[test]
fn wt1_reset_hard_preserves_untracked_file() {
    let temp = TempDir::new().unwrap();
    init_repo(temp.path());
    commit_file(temp.path(), "tracked.txt", "v1", "initial");

    fs::write(temp.path().join("scratch.txt"), "unsaved work").unwrap();

    mediagit()
        .args(["reset", "--hard", "HEAD"])
        .current_dir(temp.path())
        .assert()
        .success();

    assert!(
        temp.path().join("scratch.txt").exists(),
        "reset --hard deleted an untracked file"
    );
    assert_eq!(
        fs::read_to_string(temp.path().join("scratch.txt")).unwrap(),
        "unsaved work"
    );
}

/// Same hazard on the merge path: a merge that does not touch the untracked
/// path must leave it alone (and must still succeed — the guard refuses only
/// on real collisions).
#[test]
fn wt1_merge_preserves_untracked_file() {
    let temp = TempDir::new().unwrap();
    init_repo(temp.path());
    commit_file(temp.path(), "base.txt", "base", "initial");

    branch_create(temp.path(), "feature");
    branch_switch(temp.path(), "feature");
    commit_file(temp.path(), "feature.txt", "feat", "feature work");
    branch_switch(temp.path(), "main");

    fs::write(temp.path().join("scratch.txt"), "unsaved work").unwrap();

    mediagit()
        .args(["merge", "feature"])
        .current_dir(temp.path())
        .assert()
        .success();

    assert!(
        temp.path().join("scratch.txt").exists(),
        "merge deleted an untracked file"
    );
    assert_eq!(
        fs::read_to_string(temp.path().join("scratch.txt")).unwrap(),
        "unsaved work"
    );
}

/// An untracked file that *collides* with a path the target tree will
/// materialize must block the operation — overwriting it is silent data loss.
#[test]
fn wt1_merge_refuses_untracked_collision() {
    let temp = TempDir::new().unwrap();
    init_repo(temp.path());
    commit_file(temp.path(), "base.txt", "base", "initial");

    branch_create(temp.path(), "feature");
    branch_switch(temp.path(), "feature");
    commit_file(temp.path(), "shared.txt", "from feature", "feature work");
    branch_switch(temp.path(), "main");

    fs::write(temp.path().join("shared.txt"), "my untracked version").unwrap();

    mediagit()
        .args(["merge", "feature"])
        .current_dir(temp.path())
        .assert()
        .failure()
        .stderr(predicate::str::contains("shared.txt"));

    assert_eq!(
        fs::read_to_string(temp.path().join("shared.txt")).unwrap(),
        "my untracked version",
        "merge overwrote an untracked file"
    );
}

// ============================================================================
// WT-2 — the dirty-check must recurse into subdirectories
// ============================================================================

/// The single dirty-check in the codebase iterated only the top level of the
/// HEAD tree, so a modified tracked file in *any* subdirectory was invisible
/// and `branch switch` overwrote it. A top-level fixture passes today and
/// proves nothing — this one is nested.
#[test]
fn wt2_switch_refuses_dirty_nested_file() {
    let temp = TempDir::new().unwrap();
    init_repo(temp.path());
    commit_file(temp.path(), "src/deep/file.txt", "v1", "initial");

    branch_create(temp.path(), "feature");
    branch_switch(temp.path(), "feature");
    commit_file(temp.path(), "src/deep/file.txt", "v2", "feature edit");
    branch_switch(temp.path(), "main");

    let nested = temp.path().join("src/deep/file.txt");
    fs::write(&nested, "LOCAL WORK").unwrap();

    mediagit()
        .args(["branch", "switch", "feature"])
        .current_dir(temp.path())
        .assert()
        .failure()
        .stderr(predicate::str::contains("file.txt"));

    assert_eq!(
        fs::read_to_string(&nested).unwrap(),
        "LOCAL WORK",
        "branch switch overwrote a modified file in a subdirectory"
    );
}

/// `-f/--force` must still bypass the guard.
#[test]
fn wt2_switch_force_still_overwrites() {
    let temp = TempDir::new().unwrap();
    init_repo(temp.path());
    commit_file(temp.path(), "src/deep/file.txt", "v1", "initial");

    branch_create(temp.path(), "feature");
    branch_switch(temp.path(), "feature");
    commit_file(temp.path(), "src/deep/file.txt", "v2", "feature edit");
    branch_switch(temp.path(), "main");

    let nested = temp.path().join("src/deep/file.txt");
    fs::write(&nested, "LOCAL WORK").unwrap();

    mediagit()
        .args(["branch", "switch", "-f", "feature"])
        .current_dir(temp.path())
        .assert()
        .success();

    assert_eq!(fs::read_to_string(&nested).unwrap(), "v2");
}

// ============================================================================
// WT-3 — merge / rebase / cherry-pick / revert had no dirty check at all
// ============================================================================

#[test]
fn wt3_merge_refuses_dirty_tracked_file() {
    let temp = TempDir::new().unwrap();
    init_repo(temp.path());
    commit_file(temp.path(), "shared.txt", "v1", "initial");

    branch_create(temp.path(), "feature");
    branch_switch(temp.path(), "feature");
    commit_file(temp.path(), "shared.txt", "v2", "feature edit");
    branch_switch(temp.path(), "main");

    let shared = temp.path().join("shared.txt");
    fs::write(&shared, "LOCAL WORK").unwrap();

    mediagit()
        .args(["merge", "feature"])
        .current_dir(temp.path())
        .assert()
        .failure()
        .stderr(predicate::str::contains("shared.txt"));

    assert_eq!(
        fs::read_to_string(&shared).unwrap(),
        "LOCAL WORK",
        "merge overwrote an uncommitted change"
    );
}

#[test]
fn wt3_rebase_refuses_dirty_tracked_file() {
    let temp = TempDir::new().unwrap();
    init_repo(temp.path());
    commit_file(temp.path(), "shared.txt", "v1", "initial");

    branch_create(temp.path(), "feature");
    branch_switch(temp.path(), "feature");
    commit_file(temp.path(), "other.txt", "feat", "feature work");
    branch_switch(temp.path(), "main");
    commit_file(temp.path(), "shared.txt", "v2", "main edit");
    branch_switch(temp.path(), "feature");

    let other = temp.path().join("other.txt");
    fs::write(&other, "LOCAL WORK").unwrap();

    mediagit()
        .args(["rebase", "main"])
        .current_dir(temp.path())
        .assert()
        .failure()
        .stderr(predicate::str::contains("other.txt"));

    assert_eq!(
        fs::read_to_string(&other).unwrap(),
        "LOCAL WORK",
        "rebase overwrote an uncommitted change"
    );
}

#[test]
fn wt3_cherry_pick_refuses_dirty_tracked_file() {
    let temp = TempDir::new().unwrap();
    init_repo(temp.path());
    commit_file(temp.path(), "shared.txt", "v1", "initial");

    branch_create(temp.path(), "feature");
    branch_switch(temp.path(), "feature");
    commit_file(temp.path(), "picked.txt", "picked", "feature work");
    branch_switch(temp.path(), "main");

    let shared = temp.path().join("shared.txt");
    fs::write(&shared, "LOCAL WORK").unwrap();

    mediagit()
        .args(["cherry-pick", "feature"])
        .current_dir(temp.path())
        .assert()
        .failure()
        .stderr(predicate::str::contains("shared.txt"));

    assert_eq!(
        fs::read_to_string(&shared).unwrap(),
        "LOCAL WORK",
        "cherry-pick overwrote an uncommitted change"
    );
}

#[test]
fn wt3_revert_refuses_dirty_tracked_file() {
    let temp = TempDir::new().unwrap();
    init_repo(temp.path());
    commit_file(temp.path(), "base.txt", "base", "initial");
    commit_file(temp.path(), "target.txt", "to revert", "second");

    let base = temp.path().join("base.txt");
    fs::write(&base, "LOCAL WORK").unwrap();

    mediagit()
        .args(["revert", "HEAD"])
        .current_dir(temp.path())
        .assert()
        .failure()
        .stderr(predicate::str::contains("base.txt"));

    assert_eq!(
        fs::read_to_string(&base).unwrap(),
        "LOCAL WORK",
        "revert overwrote an uncommitted change"
    );
}

// ============================================================================
// WT-8 — sparse-checkout set deleted newly-excluded files unconditionally
// ============================================================================

#[test]
fn wt8_sparse_set_refuses_to_delete_modified_file() {
    let temp = TempDir::new().unwrap();
    init_repo(temp.path());
    commit_file(temp.path(), "included/keep.txt", "keep", "add keep");
    commit_file(temp.path(), "excluded/drop.txt", "drop", "add drop");

    let drop = temp.path().join("excluded/drop.txt");
    fs::write(&drop, "LOCAL WORK").unwrap();

    mediagit()
        .args(["sparse-checkout", "set", "included"])
        .current_dir(temp.path())
        .assert()
        .failure()
        .stderr(predicate::str::contains("drop.txt"));

    assert_eq!(
        fs::read_to_string(&drop).unwrap(),
        "LOCAL WORK",
        "sparse-checkout deleted a file with uncommitted changes"
    );
}

/// A clean exclusion is still allowed — the guard must not break the feature.
#[test]
fn wt8_sparse_set_still_excludes_clean_files() {
    let temp = TempDir::new().unwrap();
    init_repo(temp.path());
    commit_file(temp.path(), "included/keep.txt", "keep", "add keep");
    commit_file(temp.path(), "excluded/drop.txt", "drop", "add drop");

    mediagit()
        .args(["sparse-checkout", "set", "included"])
        .current_dir(temp.path())
        .assert()
        .success();

    assert!(!temp.path().join("excluded/drop.txt").exists());
    assert!(temp.path().join("included/keep.txt").exists());
}

// ============================================================================
// UX-1 — `branch delete -d` must actually check merged-ness
// ============================================================================

#[test]
fn ux1_delete_merged_refuses_unmerged_branch() {
    let temp = TempDir::new().unwrap();
    init_repo(temp.path());
    commit_file(temp.path(), "base.txt", "base", "initial");

    branch_create(temp.path(), "feature");
    branch_switch(temp.path(), "feature");
    commit_file(temp.path(), "feature.txt", "feat", "unmerged work");
    branch_switch(temp.path(), "main");

    mediagit()
        .args(["branch", "delete", "-d", "feature"])
        .current_dir(temp.path())
        .assert()
        .failure()
        .stderr(predicate::str::contains("not fully merged"));

    mediagit()
        .args(["branch", "list"])
        .current_dir(temp.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("feature"));
}

#[test]
fn ux1_delete_merged_allows_merged_branch() {
    let temp = TempDir::new().unwrap();
    init_repo(temp.path());
    commit_file(temp.path(), "base.txt", "base", "initial");

    branch_create(temp.path(), "feature");

    // `feature` points at HEAD — trivially merged.
    mediagit()
        .args(["branch", "delete", "-d", "feature"])
        .current_dir(temp.path())
        .assert()
        .success();

    mediagit()
        .args(["branch", "list"])
        .current_dir(temp.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("feature").not());
}

#[test]
fn ux1_force_delete_still_deletes_unmerged_branch() {
    let temp = TempDir::new().unwrap();
    init_repo(temp.path());
    commit_file(temp.path(), "base.txt", "base", "initial");

    branch_create(temp.path(), "feature");
    branch_switch(temp.path(), "feature");
    commit_file(temp.path(), "feature.txt", "feat", "unmerged work");
    branch_switch(temp.path(), "main");

    mediagit()
        .args(["branch", "delete", "-D", "feature"])
        .current_dir(temp.path())
        .assert()
        .success();

    mediagit()
        .args(["branch", "list"])
        .current_dir(temp.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("feature").not());
}

// ============================================================================
// WT-1 (completion) — the invariant must hold for EVERY working-tree rewrite,
// not just the commands enumerated in the first pass. `bisect` re-checks-out
// on every step, so an unbounded checkout deleted untracked work repeatedly.
// ============================================================================

/// Bisect bounds deletions but deliberately does NOT refuse on a dirty tree:
/// it re-checks-out on each step, so refusing would make the feature unusable.
/// Untracked work must survive every hop regardless.
#[test]
fn wt1_bisect_preserves_untracked_file_across_steps() {
    let temp = TempDir::new().unwrap();
    init_repo(temp.path());
    commit_file(temp.path(), "src/app.txt", "v1", "good commit");
    commit_file(temp.path(), "src/app.txt", "v2", "second");
    commit_file(temp.path(), "src/app.txt", "v3", "bad commit");

    // Untracked WIP that exists nowhere else.
    let wip = temp.path().join("notes.md");
    fs::write(&wip, "UNTRACKED WIP").unwrap();

    mediagit()
        .args(["bisect", "start"])
        .current_dir(temp.path())
        .assert()
        .success();
    mediagit()
        .args(["bisect", "bad"])
        .current_dir(temp.path())
        .assert()
        .success();

    let oldest = commit_oid_back(temp.path(), 2);

    // Marking good moves HEAD to a midpoint — a working-tree rewrite.
    mediagit()
        .args(["bisect", "good", &oldest])
        .current_dir(temp.path())
        .assert()
        .success();

    assert!(
        wip.exists(),
        "bisect deleted an untracked file while stepping"
    );
    assert_eq!(fs::read_to_string(&wip).unwrap(), "UNTRACKED WIP");

    // And reset must not eat it either.
    mediagit()
        .args(["bisect", "reset"])
        .current_dir(temp.path())
        .assert()
        .success();
    assert!(wip.exists(), "bisect reset deleted an untracked file");
}

/// Negative control: bisect must still actually move HEAD. A guard that
/// silently refused to check out would pass the test above for the wrong
/// reason.
#[test]
fn wt1_bisect_still_steps_through_history() {
    let temp = TempDir::new().unwrap();
    init_repo(temp.path());
    commit_file(temp.path(), "f.txt", "v1", "good");
    commit_file(temp.path(), "f.txt", "v2", "mid");
    commit_file(temp.path(), "f.txt", "v3", "bad");

    mediagit()
        .args(["bisect", "start"])
        .current_dir(temp.path())
        .assert()
        .success();
    mediagit()
        .args(["bisect", "bad"])
        .current_dir(temp.path())
        .assert()
        .success();

    let oldest = commit_oid_back(temp.path(), 2);
    mediagit()
        .args(["bisect", "good", &oldest])
        .current_dir(temp.path())
        .assert()
        .success();

    // HEAD moved off the bad commit: the working tree reflects an earlier rev.
    let content = fs::read_to_string(temp.path().join("f.txt")).unwrap();
    assert_ne!(content, "v3", "bisect never moved the working tree");
}

// ============================================================================
// Inverse regression — bounding deletions must NOT strand tracked files.
// `with_tracked_paths` limits what checkout may delete. If that bound is too
// aggressive, a file tracked at HEAD but absent from the target tree would be
// left on disk after a switch — a silent correctness bug in the opposite
// direction, introduced by the very fix meant to prevent data loss. Nothing
// else in this suite would catch it.
// ============================================================================

#[test]
fn bound_still_deletes_tracked_file_absent_from_target() {
    let temp = TempDir::new().unwrap();
    init_repo(temp.path());
    commit_file(temp.path(), "keep.txt", "keep", "base");
    commit_file(
        temp.path(),
        "src/gone.txt",
        "doomed",
        "add file to be removed",
    );

    // Branch that still has gone.txt.
    branch_create(temp.path(), "has-file");

    // On main, delete and commit it.
    fs::remove_file(temp.path().join("src/gone.txt")).unwrap();
    mediagit()
        .args(["add", "-A"])
        .current_dir(temp.path())
        .assert()
        .success();
    mediagit()
        .args(["commit", "-m", "remove gone.txt"])
        .current_dir(temp.path())
        .assert()
        .success();

    // Switch onto the branch that has it — it must reappear.
    branch_switch(temp.path(), "has-file");
    assert!(
        temp.path().join("src/gone.txt").exists(),
        "tracked file missing after switching to a branch that contains it"
    );

    // Switch back — it is tracked at the old HEAD and absent from the target,
    // so the checkout is still entitled to delete it. The bound must not
    // preserve it.
    branch_switch(temp.path(), "main");
    assert!(
        !temp.path().join("src/gone.txt").exists(),
        "with_tracked_paths stranded a tracked file that the target tree omits"
    );
    assert!(temp.path().join("keep.txt").exists(), "keep.txt lost");
}

/// The test above exercises `checkout_diff`. `reset --hard` is the path that
/// actually runs `clean_working_directory` under the new bound, so it needs
/// its own check: a tracked file absent from the reset target must still go.
#[test]
fn bound_still_deletes_tracked_file_on_reset_hard() {
    let temp = TempDir::new().unwrap();
    init_repo(temp.path());
    commit_file(temp.path(), "base.txt", "base", "base");
    let base = commit_oid_back(temp.path(), 0);
    commit_file(temp.path(), "src/added.txt", "added later", "add file");

    assert!(temp.path().join("src/added.txt").exists());

    // Untracked WIP must survive; the tracked file must not.
    let wip = temp.path().join("wip.tmp");
    fs::write(&wip, "WIP").unwrap();

    mediagit()
        .args(["reset", "--hard", &base])
        .current_dir(temp.path())
        .assert()
        .success();

    assert!(
        !temp.path().join("src/added.txt").exists(),
        "reset --hard left behind a tracked file absent from the target commit"
    );
    assert!(wip.exists(), "reset --hard deleted untracked work");
    assert!(temp.path().join("base.txt").exists(), "base.txt lost");
}
