// SPDX-License-Identifier: BUSL-1.1
// Copyright (C) 2025-2026 Aswin Krishnamoorthy

//! `mediagit remote set-url --push` must redirect the PUSH, not just the display.
//!
//! # Why this is a CLI test and not a unit test
//!
//! The defect was never in the resolver — it was that `push` called the wrong
//! one. `push.rs` resolved through `Config::resolve_remote_url`, which reads
//! `url`, so a remote's push URL was stored by `set-url --push`, printed back
//! as "Changed push URL for 'origin'", shown by `remote show`, and ignored by
//! every push.
//!
//! A unit test on `resolve_push_url` does not catch that. It was written first,
//! and reverting `push.rs` to the fetch-side resolver left it green — the test
//! guarded the helper while the bug lived at the call site. This file asserts
//! against the real binary's own resolution instead.
//!
//! `push --verbose` prints `Remote URL: <resolved>` immediately after resolving
//! and before opening any connection, so the assertion needs no server: the
//! push is expected to fail at connect, and what is under test is which URL it
//! tried.

#![allow(clippy::unwrap_used)]

use assert_cmd::Command;
use std::fs;
use std::path::Path;
use tempfile::TempDir;

fn mediagit() -> Command {
    let mut c = Command::cargo_bin("mediagit").unwrap();
    c.env("MEDIAGIT_AUTHOR_NAME", "Test User")
        .env("MEDIAGIT_AUTHOR_EMAIL", "test@example.com")
        .env("MEDIAGIT_NO_KEYRING", "1")
        // Keep the resolution deterministic: an env credential would not change
        // the URL, but an env token changes which code path logs first.
        .env_remove("MEDIAGIT_TOKEN")
        .env_remove("MEDIAGIT_API_KEY");
    c
}

fn init_repo(dir: &Path) {
    mediagit()
        .arg("init")
        .arg("-q")
        .current_dir(dir)
        .assert()
        .success();
}

/// Ports chosen in the unassigned range and never bound by this suite, so the
/// connect fails fast instead of reaching something real.
const FETCH_URL: &str = "http://127.0.0.1:59731/fetch-side";
const PUSH_URL: &str = "http://127.0.0.1:59732/push-side";

fn repo_with_split_urls() -> TempDir {
    let dir = TempDir::new().unwrap();
    init_repo(dir.path());

    mediagit()
        .args(["remote", "add", "origin", FETCH_URL])
        .current_dir(dir.path())
        .assert()
        .success();

    mediagit()
        .args(["remote", "set-url", "--push", "origin", PUSH_URL])
        .current_dir(dir.path())
        .assert()
        .success();

    // Something to push, so the command gets past the empty-repo checks.
    fs::write(dir.path().join("a.txt"), "content").unwrap();
    mediagit()
        .args(["add", "a.txt"])
        .current_dir(dir.path())
        .assert()
        .success();
    mediagit()
        .args(["commit", "-m", "seed"])
        .current_dir(dir.path())
        .assert()
        .success();

    dir
}

/// The load-bearing half: a push must dial the push URL.
#[test]
fn push_uses_the_push_url_not_the_fetch_url() {
    let dir = repo_with_split_urls();

    let out = mediagit()
        .args(["push", "origin", "--verbose"])
        .current_dir(dir.path())
        .output()
        .unwrap();
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );

    assert!(
        combined.contains(PUSH_URL),
        "push must resolve to the push URL {PUSH_URL}, got:\n{combined}"
    );
    assert!(
        !combined.contains(FETCH_URL),
        "push must not dial the fetch URL {FETCH_URL}, got:\n{combined}"
    );
}

/// The other half. Without it, a build that sent everything to the push URL
/// would pass the test above while breaking every fetch.
#[test]
fn fetch_still_uses_the_fetch_url() {
    let dir = repo_with_split_urls();

    let out = mediagit()
        .args(["fetch", "origin", "--verbose"])
        .current_dir(dir.path())
        .output()
        .unwrap();
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );

    assert!(
        !combined.contains(PUSH_URL),
        "setting a push URL must not redirect fetches, got:\n{combined}"
    );
}

/// A remote with no push URL must still push to `url` — the common case, and
/// the one a naive fix breaks.
#[test]
fn a_remote_without_a_push_url_still_pushes_to_its_url() {
    let dir = TempDir::new().unwrap();
    init_repo(dir.path());

    mediagit()
        .args(["remote", "add", "origin", FETCH_URL])
        .current_dir(dir.path())
        .assert()
        .success();

    fs::write(dir.path().join("a.txt"), "content").unwrap();
    mediagit()
        .args(["add", "a.txt"])
        .current_dir(dir.path())
        .assert()
        .success();
    mediagit()
        .args(["commit", "-m", "seed"])
        .current_dir(dir.path())
        .assert()
        .success();

    let out = mediagit()
        .args(["push", "origin", "--verbose"])
        .current_dir(dir.path())
        .output()
        .unwrap();
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );

    assert!(
        combined.contains(FETCH_URL),
        "with no push URL set, push must fall back to `url`, got:\n{combined}"
    );
}
