// SPDX-License-Identifier: BUSL-1.1
// Copyright (C) 2025-2026 Aswin Krishnamoorthy

//! VC-2 — gc must not collect objects it has only just seen written.
//!
//! Rooting is inherently racy against a concurrent writer. A chunk uploaded by
//! one client but not yet referenced by any ref is unreachable, and therefore
//! collectible, in the window before its commit lands. The victim cannot
//! recover: push dedups unconditionally and never re-verifies, so the loss
//! surfaces much later as a terminal 404 on clone.
//!
//! `gc.rs` carried a note saying a prune grace period "would be real defence in
//! depth here … It is NOT implemented", because `StorageBackend` exposed no
//! modification time. `StorageBackend::modified_at` now supplies one.
//!
//! ## What makes this test load-bearing
//!
//! It compares two runs that differ in exactly one environment variable against
//! identical repository state. Asserting only "the object survived" would pass
//! against a gc that collects nothing at all, so the `grace=0` arm is required:
//! it proves the object really was collectible garbage, and therefore that its
//! survival in the other arm is the grace period doing work rather than the
//! object being reachable after all.
//!
//! The reflog horizon is disabled in both arms. Reflog independently roots
//! recently-moved objects, and leaving it on would let a *reflog* root
//! masquerade as grace-period protection.

use assert_cmd::Command;
use std::path::Path;
use tempfile::TempDir;

#[allow(deprecated)]
fn mediagit() -> Command {
    let mut c = Command::cargo_bin("mediagit").unwrap();
    // `commit` refuses an unconfigured identity (UX-6) rather than authoring as
    // Unknown, so declare one the way a real user would.
    c.env("MEDIAGIT_AUTHOR_NAME", "Test User")
        .env("MEDIAGIT_AUTHOR_EMAIL", "test@example.com");
    c
}

/// Every object file currently in the store.
///
/// Counted by walking the directory rather than by parsing gc's own output: a
/// bug that made gc *report* a deletion it did not perform, or perform one it
/// did not report, would be invisible to a test that trusted the report.
fn object_count(repo: &Path) -> usize {
    fn walk(dir: &Path, n: &mut usize) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for e in entries.flatten() {
            let p = e.path();
            if p.is_dir() {
                walk(&p, n);
            } else {
                *n += 1;
            }
        }
    }
    let mut n = 0;
    walk(&repo.join(".mediagit/objects"), &mut n);
    n
}

fn init_repo(dir: &Path) {
    mediagit()
        .args(["init", "-q"])
        .current_dir(dir)
        .assert()
        .success();
}

/// Stage `content` at `rel`. Staging the same path twice orphans the first
/// blob: the index entry is replaced, and with the reflog horizon disabled
/// nothing else roots it.
fn add_file(dir: &Path, rel: &str, content: &str) {
    std::fs::write(dir.join(rel), content).unwrap();
    mediagit()
        .args(["add", rel])
        .current_dir(dir)
        .assert()
        .success();
}

/// Run gc with the reflog horizon disabled and an explicit grace window.
fn gc_with_grace(dir: &Path, grace_secs: &str) {
    mediagit()
        .args(["gc", "--yes"])
        .env("MEDIAGIT_GC_REFLOG_HORIZON_DAYS", "0")
        .env("MEDIAGIT_GC_GRACE_SECS", grace_secs)
        .current_dir(dir)
        .assert()
        .success();
}

/// Build a repo containing at least one freshly-written unreachable object.
fn repo_with_fresh_garbage() -> TempDir {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_repo(dir);
    add_file(dir, "asset.txt", "first revision");
    // Replaces the index entry; the first blob is now unreferenced.
    add_file(
        dir,
        "asset.txt",
        "second revision, different bytes entirely",
    );
    tmp
}

#[test]
fn a_default_grace_period_protects_freshly_written_objects() {
    let tmp = repo_with_fresh_garbage();
    let dir = tmp.path();

    let before = object_count(dir);
    // One hour. Everything just written is far inside the window.
    gc_with_grace(dir, "3600");
    let after = object_count(dir);

    assert_eq!(
        after,
        before,
        "gc collected {} object(s) that were written seconds ago. The prune \
         grace period did not protect them -- check that the backend in use \
         actually implements StorageBackend::modified_at, and that any wrapper \
         (NamespacedBackend) forwards it rather than inheriting the Ok(None) \
         default.",
        before.saturating_sub(after)
    );
}

#[test]
fn grace_zero_still_collects_so_the_other_arm_is_meaningful() {
    let tmp = repo_with_fresh_garbage();
    let dir = tmp.path();

    let before = object_count(dir);
    gc_with_grace(dir, "0");
    let after = object_count(dir);

    assert!(
        after < before,
        "with the grace period disabled gc collected nothing ({before} objects \
         before and after), so this repo contained no collectable garbage. That \
         makes the sibling test vacuous -- it would pass whether or not the \
         grace period works. Fix the fixture, not the assertion."
    );
}
