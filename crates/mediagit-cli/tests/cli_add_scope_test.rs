// SPDX-License-Identifier: BUSL-1.1
// Copyright (C) 2025-2026 Aswin Krishnamoorthy

//! `add <pathspec>` stages deletions only inside the pathspec. The repository
//! folder itself, in any spelling, means "everything" (BUG-008 fix round 2:
//! `repo_relative` now rejects the root as a FILE path, which must not narrow
//! the deletion scope).

use assert_cmd::Command;
use std::fs;
use std::path::Path;
use tempfile::TempDir;

#[allow(deprecated)]
fn mediagit() -> Command {
    let mut cmd = Command::cargo_bin("mediagit").unwrap();
    cmd.env("MEDIAGIT_AUTHOR_NAME", "Test User")
        .env("MEDIAGIT_AUTHOR_EMAIL", "test@example.com");
    cmd
}

fn run(cwd: &Path, args: &[&str]) -> String {
    let out = mediagit().args(args).current_dir(cwd).output().unwrap();
    assert!(
        out.status.success(),
        "{args:?} failed: {}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).to_string()
}

/// Repo with top.txt, dir/x.txt, dir/y.txt committed, then top.txt and
/// dir/x.txt deleted from the working folder.
fn repo_with_two_deletions() -> TempDir {
    let t = TempDir::new().unwrap();
    let r = t.path();
    run(r, &["init", "-q"]);
    fs::create_dir_all(r.join("dir")).unwrap();
    fs::write(r.join("top.txt"), "t").unwrap();
    fs::write(r.join("dir/x.txt"), "x").unwrap();
    fs::write(r.join("dir/y.txt"), "y").unwrap();
    run(r, &["add", "--all"]);
    run(r, &["commit", "-m", "base"]);
    fs::remove_file(r.join("top.txt")).unwrap();
    fs::remove_file(r.join("dir/x.txt")).unwrap();
    t
}

fn staged_deletions(r: &Path) -> Vec<String> {
    run(r, &["status", "--porcelain"])
        .lines()
        .filter(|l| l.starts_with('D'))
        .map(|l| l[1..].trim().to_string())
        .collect()
}

#[test]
fn dot_slash_stages_every_deletion() {
    let t = repo_with_two_deletions();
    run(t.path(), &["add", "./"]);
    let d = staged_deletions(t.path());
    assert!(d.iter().any(|l| l.ends_with("top.txt")), "{d:?}");
    assert!(d.iter().any(|l| l.ends_with("dir/x.txt")), "{d:?}");
}

#[test]
fn absolute_repository_path_stages_every_deletion() {
    let t = repo_with_two_deletions();
    let abs = t.path().to_string_lossy().to_string();
    run(t.path(), &["add", &abs]);
    let d = staged_deletions(t.path());
    assert!(d.iter().any(|l| l.ends_with("top.txt")), "{d:?}");
    assert!(d.iter().any(|l| l.ends_with("dir/x.txt")), "{d:?}");
}

#[test]
fn parent_of_subdirectory_stages_every_deletion() {
    let t = repo_with_two_deletions();
    run(&t.path().join("dir"), &["add", ".."]);
    let d = staged_deletions(t.path());
    assert!(d.iter().any(|l| l.ends_with("top.txt")), "{d:?}");
    assert!(d.iter().any(|l| l.ends_with("dir/x.txt")), "{d:?}");
}

#[test]
fn subdirectory_with_trailing_slash_stages_only_its_deletions() {
    let t = repo_with_two_deletions();
    run(t.path(), &["add", "dir/"]);
    let d = staged_deletions(t.path());
    assert!(d.iter().any(|l| l.ends_with("dir/x.txt")), "{d:?}");
    assert!(
        !d.iter().any(|l| l.ends_with("top.txt")),
        "top.txt is outside the pathspec: {d:?}"
    );
}
