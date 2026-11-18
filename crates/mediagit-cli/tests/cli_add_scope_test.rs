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

/// Run without requiring success; returns (success, stdout + stderr).
fn try_run(cwd: &Path, args: &[&str]) -> (bool, String) {
    let out = mediagit().args(args).current_dir(cwd).output().unwrap();
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    (out.status.success(), text)
}

/// A pathspec that resolves to nothing in the repository must never widen to
/// "everything": no deletion is staged and the command says what was wrong.
fn assert_no_widening(r: &Path, cwd: &Path, spec: &str) {
    let (ok, text) = try_run(cwd, &["add", spec]);
    assert!(
        staged_deletions(r).is_empty(),
        "add {spec} staged deletions: {:?}\n{text}",
        staged_deletions(r)
    );
    // Today this is a warning with exit 0 (nothing matched), not an error.
    let _ = ok;
    assert!(
        (text.contains("does not exist") || text.contains("not inside the repository"))
            && text.contains(spec),
        "add {spec} must name the bad pathspec:
{text}"
    );
}

#[test]
fn pathspec_outside_the_repository_stages_no_deletion() {
    let t = repo_with_two_deletions();
    let outside = TempDir::new().unwrap();
    let spec = outside.path().join("elsewhere.txt");
    fs::write(&spec, "o").unwrap();
    assert_no_widening(t.path(), t.path(), spec.to_str().unwrap());
}

#[test]
fn dotdot_sibling_from_a_subdirectory_stages_no_deletion() {
    let t = repo_with_two_deletions();
    // ../sibling resolves above the repository root from a subfolder of it.
    let sub = t.path().join("dir");
    assert_no_widening(t.path(), &sub, "../../sibling-folder");
}

#[test]
fn missing_name_stages_no_deletion() {
    let t = repo_with_two_deletions();
    assert_no_widening(t.path(), t.path(), "a-name-that-does-not-exist");
}

/// Windows only: the repository root spelled with different letter case. It
/// must act as the root (every deletion staged) or say plainly that nothing
/// matched; never succeed silently with nothing staged.
#[cfg(windows)]
#[test]
fn root_in_a_different_case_is_the_root_or_an_error() {
    let t = repo_with_two_deletions();
    let root = dunce::canonicalize(t.path()).unwrap();
    let swapped: String = root
        .to_string_lossy()
        .chars()
        .map(|c| {
            if c.is_lowercase() {
                c.to_uppercase().next().unwrap()
            } else {
                c.to_lowercase().next().unwrap()
            }
        })
        .collect();
    let (ok, text) = try_run(t.path(), &["add", &swapped]);
    let staged = staged_deletions(t.path());
    eprintln!("different-case root: ok={ok} staged={staged:?}\n{text}");
    assert!(
        staged.len() == 2 || !ok || text.to_lowercase().contains("no files"),
        "silent no-op for a different-case root (ok={ok}, staged={staged:?}):\n{text}"
    );
}

/// A staged deletion is never "unchanged": restoring the file with HEAD's exact
/// bytes and adding it again must cancel the deletion, so the commit keeps it.
#[test]
fn restored_file_cancels_staged_deletion() {
    let t = repo_with_two_deletions();
    let r = t.path();
    run(r, &["add", "top.txt"]);
    assert!(staged_deletions(r).iter().any(|l| l.ends_with("top.txt")));
    fs::write(r.join("top.txt"), "t").unwrap();
    run(r, &["add", "top.txt"]);
    assert!(
        !staged_deletions(r).iter().any(|l| l.ends_with("top.txt")),
        "deletion survived re-add"
    );
    run(r, &["add", "dir/x.txt"]);
    run(r, &["commit", "-m", "second"]);
    let left = run(r, &["status", "--porcelain"]);
    assert!(
        !left.contains("top.txt"),
        "top.txt dropped by commit: {left}"
    );
    let tree = run(r, &["ls-files"]);
    assert!(tree.contains("top.txt"), "top.txt not tracked: {tree}");
}
