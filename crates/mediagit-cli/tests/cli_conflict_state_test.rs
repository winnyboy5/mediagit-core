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

// ============================================================================
// WT-5 — `revert --continue` must actually revert.
//
// On conflict, revert staged the *unmodified HEAD tree* and bailed, writing no
// markers. `do_continue` then built a tree from that index — HEAD's own tree —
// and committed it as "the revert". The command reported success and changed
// nothing.
// ============================================================================

/// Three edits to one line: reverting the middle commit must conflict against
/// the third, because the region it wants to restore has since moved on.
fn setup_revert_conflict(dir: &Path) -> String {
    init_repo(dir);
    commit_file(dir, "shared.txt", "v1\n", "first");
    commit_file(dir, "shared.txt", "v2\n", "second");
    let target = {
        let out = run(dir, &["log", "--oneline"]);
        let text = String::from_utf8_lossy(&out.stdout);
        // HEAD is "second"; grab its short oid before we move past it.
        text.lines()
            .next()
            .unwrap()
            .split_whitespace()
            .next()
            .unwrap()
            .to_string()
    };
    commit_file(dir, "shared.txt", "v3\n", "third");
    target
}

#[test]
fn wt5_revert_conflict_writes_markers_to_the_working_tree() {
    let temp = TempDir::new().unwrap();
    let target = setup_revert_conflict(temp.path());

    let out = run(temp.path(), &["revert", &target]);
    assert!(
        !out.status.success(),
        "revert should have stopped on a conflict"
    );

    let content = fs::read_to_string(temp.path().join("shared.txt")).unwrap();
    assert!(
        content.contains("<<<<<<<") && content.contains(">>>>>>>"),
        "no conflict markers written; nothing for the user to resolve. Got:\n{content}"
    );
}

/// The dangerous path: `--continue` *without* resolving anything.
///
/// Because no markers were written, a user has no signal that resolution is
/// required, so running `--continue` straight away is the natural thing to do.
/// It then committed the stale HEAD tree as "the revert" — a no-op reporting
/// success, which is worse than an error because the user believes the commit
/// was reverted.
#[test]
fn wt5_revert_continue_without_resolving_must_not_commit_a_no_op() {
    let temp = TempDir::new().unwrap();
    let target = setup_revert_conflict(temp.path());

    let out = run(temp.path(), &["revert", &target]);
    assert!(!out.status.success(), "expected a conflict stop");

    let before = fs::read_to_string(temp.path().join("shared.txt")).unwrap();
    let log_before = String::from_utf8_lossy(&run(temp.path(), &["log", "--oneline"]).stdout)
        .lines()
        .count();

    let cont = run(temp.path(), &["revert", "--continue"]);

    if cont.status.success() {
        // If it claims success, it must have actually changed something.
        let after = fs::read_to_string(temp.path().join("shared.txt")).unwrap();
        let log_after = String::from_utf8_lossy(&run(temp.path(), &["log", "--oneline"]).stdout)
            .lines()
            .count();
        assert!(
            after != before && log_after > log_before,
            "revert --continue reported success but committed a no-op \
             (content unchanged and/or no new commit)"
        );
    }
    // Refusing is the correct outcome — nothing to assert beyond not lying.
}

/// Resolving then continuing must commit the resolution.
#[test]
fn wt5_revert_continue_commits_the_resolution() {
    let temp = TempDir::new().unwrap();
    let target = setup_revert_conflict(temp.path());

    let out = run(temp.path(), &["revert", &target]);
    assert!(!out.status.success(), "expected a conflict stop");

    // Resolve to something distinguishable from HEAD ("v3").
    fs::write(temp.path().join("shared.txt"), "resolved-revert\n").unwrap();
    mediagit()
        .args(["add", "shared.txt"])
        .current_dir(temp.path())
        .assert()
        .success();

    let cont = run(temp.path(), &["revert", "--continue"]);
    assert!(
        cont.status.success(),
        "revert --continue failed: {}",
        String::from_utf8_lossy(&cont.stderr)
    );

    let content = fs::read_to_string(temp.path().join("shared.txt")).unwrap();
    assert_eq!(
        content, "resolved-revert\n",
        "revert --continue discarded the resolution and committed HEAD's tree instead"
    );
}

// ============================================================================
// WT-9 — the conflict signal must work for BINARY files.
//
// MediaGit versions media and binary assets. A conflicting PSD never receives
// `<<<<<<<` markers: inlining them would corrupt the file, so the resolver
// checks out one side provisionally instead. Any guard that inspects file
// *content* therefore sees a clean tree and concludes the conflict was
// resolved — waving through exactly the file types this system exists for,
// and committing a side the user never reviewed.
//
// The index now records unresolved paths directly, so the signal is
// content-independent.
// ============================================================================

/// Bytes with an embedded NUL, which is how the resolver detects "binary".
fn binary_blob(seed: u8) -> Vec<u8> {
    let mut v = vec![0x89, 0x50, 0x4E, 0x47, 0x00, 0x1A, 0x0A];
    v.extend((0..4096u32).map(|i| (i as u8) ^ seed));
    v
}

fn commit_binary(dir: &Path, rel: &str, bytes: &[u8], msg: &str) {
    fs::write(dir.join(rel), bytes).unwrap();
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

#[test]
fn wt9_binary_conflict_blocks_continue_despite_having_no_markers() {
    let temp = TempDir::new().unwrap();
    init_repo(temp.path());
    commit_binary(temp.path(), "asset.psd", &binary_blob(0x00), "base asset");

    mediagit()
        .args(["branch", "create", "feature"])
        .current_dir(temp.path())
        .assert()
        .success();

    commit_binary(
        temp.path(),
        "asset.psd",
        &binary_blob(0x11),
        "main edits asset",
    );

    mediagit()
        .args(["branch", "switch", "feature"])
        .current_dir(temp.path())
        .assert()
        .success();
    commit_binary(
        temp.path(),
        "asset.psd",
        &binary_blob(0x22),
        "feature edits asset",
    );

    let out = run(temp.path(), &["rebase", "main"]);
    assert!(
        !out.status.success(),
        "rebase should have stopped on a binary conflict"
    );

    // The resolver must NOT have inlined markers into the binary.
    let on_disk = fs::read(temp.path().join("asset.psd")).unwrap();
    assert!(
        !on_disk.windows(7).any(|w| w == b"<<<<<<<"),
        "text conflict markers were inlined into a binary file"
    );

    // ...and yet --continue must still refuse, because nothing was reviewed.
    let cont = run(temp.path(), &["rebase", "--continue"]);
    assert!(
        !cont.status.success(),
        "rebase --continue accepted an unreviewed BINARY conflict — the guard \
         is content-based and blind to media files"
    );
    let msg = String::from_utf8_lossy(&cont.stderr);
    assert!(
        msg.contains("unresolved"),
        "expected an unresolved-path refusal, got: {msg}"
    );

    // Acknowledging by staging must unblock it, with no editing required.
    mediagit()
        .args(["add", "asset.psd"])
        .current_dir(temp.path())
        .assert()
        .success();
    let cont2 = run(temp.path(), &["rebase", "--continue"]);
    assert!(
        cont2.status.success(),
        "staging the binary should acknowledge the conflict: {}",
        String::from_utf8_lossy(&cont2.stderr)
    );
}
