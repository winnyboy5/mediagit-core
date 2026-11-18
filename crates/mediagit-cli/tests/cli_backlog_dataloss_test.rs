// SPDX-License-Identifier: BUSL-1.1
// Copyright (C) 2025-2026 Aswin Krishnamoorthy

//! Wave 6 backlog data-loss items (DL-4, DL-5, DL-9): each test reproduces a
//! reported loss and guards its fix.

use assert_cmd::Command;
use std::fs;
use std::path::Path;
use std::process::Output;
use tempfile::TempDir;

#[allow(deprecated)]
fn mediagit() -> Command {
    let mut c = Command::cargo_bin("mediagit").unwrap();
    c.env("MEDIAGIT_AUTHOR_NAME", "Test User")
        .env("MEDIAGIT_AUTHOR_EMAIL", "test@example.com");
    c
}

fn run(dir: &Path, args: &[&str]) -> Output {
    mediagit().args(args).current_dir(dir).output().unwrap()
}

fn ok(dir: &Path, args: &[&str]) {
    let o = run(dir, args);
    assert!(
        o.status.success(),
        "{args:?} failed: {}{}",
        String::from_utf8_lossy(&o.stdout),
        String::from_utf8_lossy(&o.stderr)
    );
}

fn init_repo(dir: &Path) {
    ok(dir, &["init", "-q"]);
}

fn commit_file(dir: &Path, rel: &str, content: &str, msg: &str) {
    let full = dir.join(rel);
    if let Some(p) = full.parent() {
        fs::create_dir_all(p).unwrap();
    }
    fs::write(&full, content).unwrap();
    ok(dir, &["add", rel]);
    ok(dir, &["commit", "-m", msg]);
}

// DL-4: stash must leave untracked files and empty user folders alone.
#[test]
fn dl4_stash_keeps_untracked_file_and_empty_folder() {
    for pop in [false, true] {
        let t = TempDir::new().unwrap();
        let d = t.path();
        init_repo(d);
        commit_file(d, "a.txt", "one\n", "base");
        fs::write(d.join("a.txt"), "edited\n").unwrap();
        fs::write(d.join("untracked.txt"), "mine\n").unwrap();
        fs::create_dir(d.join("empty_dir")).unwrap();

        ok(d, &["stash", "push", "-m", "wip"]);
        ok(d, &["stash", if pop { "pop" } else { "apply" }]);

        assert_eq!(
            fs::read_to_string(d.join("untracked.txt")).unwrap(),
            "mine\n",
            "untracked file lost (pop={pop})"
        );
        assert!(
            d.join("empty_dir").is_dir(),
            "empty folder removed (pop={pop})"
        );
        assert_eq!(fs::read_to_string(d.join("a.txt")).unwrap(), "edited\n");
    }
}

// DL-5: `revert --no-commit` then commit must equal a normal revert's tree.
#[test]
fn dl5_revert_no_commit_nested_matches_normal_revert() {
    let build = |no_commit: bool| -> String {
        let t = TempDir::new().unwrap();
        let d = t.path();
        init_repo(d);
        commit_file(d, "top.txt", "t\n", "base");
        commit_file(d, "art/sub/a.txt", "v1\n", "add a");
        commit_file(d, "art/sub/a.txt", "v2\n", "change a");
        commit_file(d, "art/b.txt", "b\n", "add b");
        if no_commit {
            ok(d, &["revert", "--no-commit", "HEAD~1"]);
            let st = run(d, &["status"]);
            let st = String::from_utf8_lossy(&st.stdout);
            assert!(st.contains("a.txt"), "nested file not staged: {st}");
            ok(d, &["commit", "-m", "Revert change a"]);
        } else {
            ok(d, &["revert", "HEAD~1"]);
        }
        // State of every tracked file at the end.
        let mut out = String::new();
        for rel in ["top.txt", "art/sub/a.txt", "art/b.txt"] {
            let c = fs::read_to_string(d.join(rel)).unwrap_or_else(|_| "<missing>".into());
            out.push_str(&format!("{rel}={c}"));
        }
        let status = run(d, &["status"]);
        out.push_str(&String::from_utf8_lossy(&status.stdout));
        let o = run(d, &["show", "--stat", "HEAD"]);
        let s = String::from_utf8_lossy(&o.stdout).to_string();
        // Keep only file-change lines (drop ids/dates).
        for l in s
            .lines()
            .filter(|l| l.contains('|') || l.contains("art/") || l.contains("top"))
        {
            out.push_str(l.trim());
            out.push('\n');
        }
        out
    };
    assert_eq!(build(true), build(false));
}

// DL-9: an untracked file at a path the picked commit does not change must
// not stop the pick, and must survive it.
#[test]
fn dl9_cherry_pick_ignores_untracked_file_outside_the_pick() {
    let t = TempDir::new().unwrap();
    let d = t.path();
    init_repo(d);
    commit_file(d, "base.txt", "base\n", "base");
    ok(d, &["branch", "create", "feature"]);
    ok(d, &["branch", "switch", "feature"]);
    commit_file(d, "early.txt", "early\n", "early");
    commit_file(d, "picked.txt", "picked\n", "picked");
    ok(d, &["branch", "switch", "refs/heads/main"]);

    // Untracked on main; feature tracks it, but only the first commit adds it.
    fs::write(d.join("early.txt"), "mine\n").unwrap();
    ok(d, &["cherry-pick", "feature"]);

    assert_eq!(fs::read_to_string(d.join("early.txt")).unwrap(), "mine\n");
    assert_eq!(
        fs::read_to_string(d.join("picked.txt")).unwrap(),
        "picked\n"
    );
}

// Control: an untracked file at a path the pick writes is still refused.
#[test]
fn dl9_cherry_pick_still_refuses_untracked_file_the_pick_writes() {
    let t = TempDir::new().unwrap();
    let d = t.path();
    init_repo(d);
    commit_file(d, "base.txt", "base\n", "base");
    ok(d, &["branch", "create", "feature"]);
    ok(d, &["branch", "switch", "feature"]);
    commit_file(d, "picked.txt", "picked\n", "picked");
    ok(d, &["branch", "switch", "refs/heads/main"]);

    fs::write(d.join("picked.txt"), "mine\n").unwrap();
    let o = run(d, &["cherry-pick", "feature"]);
    assert!(!o.status.success());
    assert_eq!(fs::read_to_string(d.join("picked.txt")).unwrap(), "mine\n");
}

// DL-4 (other half): a folder the checkout emptied is removed; a folder the
// user made empty is kept.
#[test]
fn dl4_checkout_removes_emptied_folders_and_keeps_user_folders() {
    let t = TempDir::new().unwrap();
    let d = t.path();
    init_repo(d);
    commit_file(d, "keep.txt", "k\n", "base");
    ok(d, &["branch", "create", "b"]);
    commit_file(d, "art/sub/a.txt", "a\n", "add art");
    assert!(d.join("art/sub/a.txt").exists());
    fs::create_dir(d.join("mine")).unwrap();

    ok(d, &["branch", "switch", "b"]);

    assert!(!d.join("art/sub").exists(), "emptied art/sub left behind");
    assert!(!d.join("art").exists(), "emptied art/ left behind");
    assert!(d.join("mine").is_dir(), "user's empty folder was removed");
}

// DL-4 (full checkout): `stash push` rewinds the tree with a whole-tree
// checkout. A folder that rewind emptied is removed; the user's empty folder
// is kept.
#[test]
fn dl4_stash_push_removes_emptied_folders_and_keeps_user_folders() {
    let t = TempDir::new().unwrap();
    let d = t.path();
    init_repo(d);
    commit_file(d, "keep.txt", "k\n", "base");
    fs::create_dir_all(d.join("art/sub")).unwrap();
    fs::write(d.join("art/sub/a.txt"), "a\n").unwrap();
    ok(d, &["add", "art/sub/a.txt"]);
    fs::create_dir(d.join("mine")).unwrap();

    ok(d, &["stash", "push", "-m", "wip"]);

    assert!(
        !d.join("art/sub/a.txt").exists(),
        "stashed file still there"
    );
    assert!(!d.join("art/sub").exists(), "emptied art/sub left behind");
    assert!(!d.join("art").exists(), "emptied art/ left behind");
    assert!(d.join("mine").is_dir(), "user's empty folder was removed");
}
