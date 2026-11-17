// SPDX-License-Identifier: BUSL-1.1
// Copyright (C) 2025-2026 Aswin Krishnamoorthy

//! BUG-039: one edit in a large project. `add --all` after a commit used to
//! scan every working file per committed path and re-read every file, so it cost
//! as much as the first add. The stat cache and a set lookup remove both.
//!
//! The timing test compares two runs of the same binary on the same machine, so
//! it carries no absolute number. The correctness tests guard the stat skip: an
//! edit must never be missed, whatever its size or age.

use assert_cmd::Command;
use std::fs;
use std::path::Path;
use std::time::{Duration, Instant, SystemTime};
use tempfile::TempDir;

#[allow(deprecated)]
fn mediagit(dir: &Path) -> Command {
    let mut c = Command::cargo_bin("mediagit").unwrap();
    c.env("MEDIAGIT_AUTHOR_NAME", "Test User")
        .env("MEDIAGIT_AUTHOR_EMAIL", "test@example.com")
        .env_remove("MEDIAGIT_STAT_CACHE")
        .current_dir(dir);
    c
}

fn run(dir: &Path, args: &[&str]) -> String {
    let out = mediagit(dir).args(args).output().unwrap();
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(out.status.success(), "{args:?} failed: {text}");
    text
}

/// Moves a file's mtime an hour back so a cache entry recorded now is not racy.
fn age(path: &Path) {
    fs::File::options()
        .write(true)
        .open(path)
        .unwrap()
        .set_modified(SystemTime::now() - Duration::from_secs(3600))
        .unwrap();
}

/// `n` distinct files of about 64 KB, spread over folders of 100, aged.
fn repo_with(n: usize) -> TempDir {
    let d = TempDir::new().unwrap();
    run(d.path(), &["init", "-q"]);
    for i in 0..n {
        let dir = d.path().join(format!("d{:03}", i / 100));
        fs::create_dir_all(&dir).unwrap();
        let f = dir.join(format!("f{i:05}.txt"));
        fs::write(&f, format!("file {i}\n").repeat(6000)).unwrap();
        age(&f);
    }
    d
}

#[test]
fn readd_after_one_edit_costs_a_fraction_of_the_first_add() {
    let d = repo_with(3000);
    let p = d.path();

    let t = Instant::now();
    run(p, &["add", "--all", "-q"]);
    let first = t.elapsed();
    run(p, &["commit", "-m", "first", "-q"]);

    let edited = p.join("d000").join("f00000.txt");
    fs::write(&edited, "edited\n".repeat(400)).unwrap();

    let t = Instant::now();
    let out = run(p, &["add", "--all"]);
    let second = t.elapsed();

    assert!(out.contains("Staged 1 file"), "only the edit stages: {out}");
    assert!(
        second * 4 < first,
        "re-add after one edit took {second:?}, first add {first:?}: should be under 25%"
    );
}

#[test]
fn same_size_edit_is_staged_whether_the_old_entry_is_trusted_or_racy() {
    let d = repo_with(20);
    let p = d.path();
    // A file whose mtime is fresh: its entry is racy and never trusted.
    let fresh = p.join("fresh.txt");
    fs::write(&fresh, "AAAA").unwrap();
    // An aged file: its entry is trusted by the next add.
    let old = p.join("d000").join("f00001.txt");

    run(p, &["add", "--all", "-q"]);
    run(p, &["commit", "-m", "first", "-q"]);
    // Warm the cache the way `status` does, then edit both keeping the size.
    run(p, &["status"]);

    let old_text = fs::read_to_string(&old).unwrap();
    assert!(old_text.starts_with("file 1\n"));
    fs::write(&old, old_text.replacen("file 1\n", "file 9\n", 1)).unwrap();
    // An old but different mtime: not racy, so only the mtime check can see it.
    age(&old);
    fs::write(&fresh, "BBBB").unwrap();

    let out = run(p, &["add", "--all"]);
    assert!(
        out.contains("Staged 2 file"),
        "both edits must stage: {out}"
    );
}

#[test]
fn hard_reset_restores_a_same_size_edit_the_cache_has_seen() {
    let d = repo_with(20);
    let p = d.path();
    let f = p.join("d000").join("f00002.txt");

    run(p, &["add", "--all", "-q"]);
    run(p, &["commit", "-m", "first", "-q"]);
    run(p, &["status"]);
    let committed = fs::read_to_string(&f).unwrap();

    fs::write(&f, committed.replacen("file 2\n", "file 8\n", 1)).unwrap();
    run(p, &["reset", "--hard", "HEAD"]);

    assert_eq!(fs::read_to_string(&f).unwrap(), committed);
}

/// Windows only: holds `path` open with no sharing, so nothing else can read
/// its bytes while the handle lives. Metadata and a zero-access open still
/// work, which is all the stat cache needs.
#[cfg(windows)]
fn lock_exclusively(path: &Path) -> fs::File {
    use std::os::windows::fs::OpenOptionsExt;
    fs::OpenOptions::new()
        .read(true)
        .share_mode(0)
        .open(path)
        .unwrap()
}

/// The stat skip is what makes these succeed: the unchanged file is locked,
/// so any attempt to read or rewrite it fails the command.
#[cfg(windows)]
#[test]
fn unchanged_files_are_not_read_by_add_or_reset_when_the_cache_is_warm() {
    let d = repo_with(10);
    let p = d.path();
    let locked = p.join("d000").join("f00003.txt");
    let other = p.join("d000").join("f00004.txt");

    // Entries whose change time is within the racy margin of the command start
    // are never cached, so let the fresh files age past it first.
    std::thread::sleep(std::time::Duration::from_millis(2500));
    run(p, &["add", "--all", "-q"]);
    run(p, &["commit", "-m", "first", "-q"]);
    run(p, &["status"]);

    let _held = lock_exclusively(&locked);
    fs::write(&other, "edited\n".repeat(50)).unwrap();
    let out = run(p, &["add", "--all"]);
    assert!(out.contains("Staged 1 file"), "only the edit stages: {out}");
    run(p, &["commit", "-m", "second", "-q"]);

    run(p, &["status"]);
    fs::remove_file(&other).unwrap();
    run(p, &["reset", "--hard", "HEAD"]);
    assert!(other.exists(), "the deleted file is restored");
}
