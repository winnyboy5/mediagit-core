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

//! Phase 2 — the gc root set must cover every mechanism a user can still
//! recover work through.
//!
//! `build_reachability_set` rooted HEAD, `refs/heads/*`, tags, in-horizon
//! reflog, and the index — but nothing else. Anything reachable only through
//! a stash, a remote-tracking ref, or an in-progress operation's state file
//! was classified as garbage and deleted.
//!
//! Reflog masks some of this (it roots recently-moved commits), so these
//! tests disable the reflog horizon with `MEDIAGIT_GC_REFLOG_HORIZON_DAYS=0`
//! to assert the *intended* root actually exists rather than accidentally
//! surviving via reflog.

use assert_cmd::Command;
use std::fs;
use std::path::Path;
use tempfile::TempDir;

#[allow(deprecated)]
fn mediagit() -> Command {
    Command::cargo_bin("mediagit").unwrap()
}

/// gc with reflog protection switched off, so only real roots count.
fn gc_without_reflog(dir: &Path) {
    mediagit()
        .args(["gc", "--yes"])
        .env("MEDIAGIT_GC_REFLOG_HORIZON_DAYS", "0")
        .current_dir(dir)
        .assert()
        .success();
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

// ============================================================================
// WT-6 — stashes are recorded in STASH_LIST, never as refs, so gc never saw
// them. `gc` after `stash push` collected the stashed tree and blobs.
// ============================================================================

#[test]
fn gc_preserves_stashed_work() {
    let temp = TempDir::new().unwrap();
    init_repo(temp.path());
    commit_file(temp.path(), "src/file.txt", "committed", "base");

    // Stash a modification — the only copy of "STASHED CONTENT" now lives in
    // the stash entry's tree, referenced from STASH_LIST alone.
    fs::write(temp.path().join("src/file.txt"), "STASHED CONTENT").unwrap();
    mediagit()
        .args(["stash", "push", "-m", "wip"])
        .current_dir(temp.path())
        .assert()
        .success();

    gc_without_reflog(temp.path());

    // The stash must still be applicable.
    mediagit()
        .args(["stash", "pop"])
        .current_dir(temp.path())
        .assert()
        .success();

    assert_eq!(
        fs::read_to_string(temp.path().join("src/file.txt")).unwrap(),
        "STASHED CONTENT",
        "gc collected the stashed content"
    );
}

// ============================================================================
// Negative control — gc must still actually collect real garbage. A root set
// that simply protected everything would pass every test above for the wrong
// reason.
// ============================================================================

#[test]
fn gc_still_collects_unreachable_objects() {
    let temp = TempDir::new().unwrap();
    init_repo(temp.path());
    commit_file(temp.path(), "a.txt", "v1", "one");

    // Create a branch, commit to it, then delete the branch. Its commit is
    // now unreachable by every mechanism.
    mediagit()
        .args(["branch", "create", "doomed"])
        .current_dir(temp.path())
        .assert()
        .success();
    mediagit()
        .args(["branch", "switch", "doomed"])
        .current_dir(temp.path())
        .assert()
        .success();
    commit_file(temp.path(), "garbage.txt", "unreachable", "doomed commit");
    mediagit()
        .args(["branch", "switch", "main"])
        .current_dir(temp.path())
        .assert()
        .success();
    mediagit()
        .args(["branch", "delete", "doomed", "--force"])
        .current_dir(temp.path())
        .assert()
        .success();

    let before = count_objects(temp.path());
    gc_without_reflog(temp.path());
    let after = count_objects(temp.path());

    assert!(
        after < before,
        "gc collected nothing ({before} -> {after}); the root set may be over-protecting"
    );
}

fn count_objects(dir: &Path) -> usize {
    fn walk(p: &Path, n: &mut usize) {
        if let Ok(rd) = fs::read_dir(p) {
            for e in rd.flatten() {
                let path = e.path();
                if path.is_dir() {
                    walk(&path, n);
                } else {
                    *n += 1;
                }
            }
        }
    }
    let mut n = 0;
    walk(&dir.join(".mediagit").join("objects"), &mut n);
    n
}
