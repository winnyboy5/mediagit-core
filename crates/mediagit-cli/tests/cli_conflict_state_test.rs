// SPDX-License-Identifier: BUSL-1.1
// Copyright (C) 2025-2026 Aswin Krishnamoorthy

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
    {
        // `commit` refuses an unconfigured identity (UX-6) instead of
        // authoring as `Unknown <unknown@localhost>`, so tests declare one
        // the way a real user would.
        let mut c = Command::cargo_bin("mediagit").unwrap();
        c.env("MEDIAGIT_AUTHOR_NAME", "Test User")
            .env("MEDIAGIT_AUTHOR_EMAIL", "test@example.com");
        c
    }
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

// ============================================================================
// WT-10 — `status` must report an operation in progress.
//
// `status` checked HEAD, the index and the working tree, but never looked for
// MERGE_HEAD, rebase-apply/state.json, CHERRY_PICK_STATE or REVERT_STATE. A
// user who hit a conflict, walked away, and came back had no way to discover
// the repository was mid-operation — the one command whose entire job is
// answering "what state am I in?" didn't answer it.
// ============================================================================

#[test]
fn wt10_status_reports_rebase_in_progress_and_unresolved_paths() {
    let temp = TempDir::new().unwrap();
    setup_conflicting_branches(temp.path());

    let out = run(temp.path(), &["rebase", "main"]);
    assert!(!out.status.success(), "expected a conflict stop");

    let status = run(temp.path(), &["status"]);
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&status.stdout),
        String::from_utf8_lossy(&status.stderr)
    );
    let lower = text.to_lowercase();

    assert!(
        lower.contains("rebase"),
        "status did not mention the in-progress rebase. Output:\n{text}"
    );
    assert!(
        lower.contains("unresolved") || lower.contains("conflict"),
        "status did not surface the unresolved path(s). Output:\n{text}"
    );
    assert!(
        text.contains("shared.txt"),
        "status did not name the conflicted file. Output:\n{text}"
    );
}

/// Negative control: a clean repository must not claim an operation is in
/// progress. A check that always fired would satisfy the test above.
#[test]
fn wt10_status_silent_when_no_operation_in_progress() {
    let temp = TempDir::new().unwrap();
    init_repo(temp.path());
    commit_file(temp.path(), "a.txt", "v1\n", "base");

    let status = run(temp.path(), &["status"]);
    let lower = String::from_utf8_lossy(&status.stdout).to_lowercase();

    assert!(
        !lower.contains("in progress"),
        "status claimed an operation was in progress on a clean repo:\n{lower}"
    );
}

// ============================================================================
// WT-7 — `stash apply` must not silently destroy divergent uncommitted work.
//
// `apply_tree_overlay` writes every stashed path over the working tree with no
// comparison against what is already there. If you stashed an edit, then made
// a *different* edit to the same file, applying the stash overwrote the second
// edit with no warning and no way to recover it — the second edit exists
// nowhere else.
// ============================================================================

#[test]
fn wt7_stash_apply_refuses_to_clobber_divergent_uncommitted_work() {
    let temp = TempDir::new().unwrap();
    init_repo(temp.path());
    commit_file(temp.path(), "work.txt", "committed\n", "base");

    // Stash one edit.
    fs::write(temp.path().join("work.txt"), "STASHED EDIT\n").unwrap();
    mediagit()
        .args(["stash", "push", "-m", "wip"])
        .current_dir(temp.path())
        .assert()
        .success();

    // Now make a DIFFERENT uncommitted edit to the same file.
    fs::write(temp.path().join("work.txt"), "LATER WORK\n").unwrap();

    let out = run(temp.path(), &["stash", "apply"]);

    let after = fs::read_to_string(temp.path().join("work.txt")).unwrap();
    if out.status.success() {
        assert_ne!(
            after, "STASHED EDIT\n",
            "stash apply silently overwrote divergent uncommitted work — \
             'LATER WORK' existed nowhere else and is now unrecoverable"
        );
    } else {
        // Refusing is the correct outcome; the work must be untouched.
        assert_eq!(
            after, "LATER WORK\n",
            "stash apply refused but still modified the file"
        );
    }
}

/// Negative control: applying onto a clean tree must still work.
#[test]
fn wt7_stash_apply_still_works_on_a_clean_tree() {
    let temp = TempDir::new().unwrap();
    init_repo(temp.path());
    commit_file(temp.path(), "work.txt", "committed\n", "base");

    fs::write(temp.path().join("work.txt"), "STASHED EDIT\n").unwrap();
    mediagit()
        .args(["stash", "push", "-m", "wip"])
        .current_dir(temp.path())
        .assert()
        .success();

    // Tree is back to HEAD after stashing; applying must restore the stash.
    let out = run(temp.path(), &["stash", "apply"]);
    assert!(
        out.status.success(),
        "stash apply failed on a clean tree: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(
        fs::read_to_string(temp.path().join("work.txt")).unwrap(),
        "STASHED EDIT\n",
        "stash apply did not restore the stashed content"
    );
}

// ============================================================================
// BUG-020 — leaving a conflicted operation must not leave a phantom merge.
// ============================================================================

fn merge_files_left(dir: &Path) -> Vec<&'static str> {
    ["MERGE_HEAD", "MERGE_MSG", "ORIG_HEAD"]
        .into_iter()
        .filter(|f| dir.join(".mediagit").join(f).exists())
        .collect()
}

fn assert_no_operation(dir: &Path, what: &str) {
    let left = merge_files_left(dir);
    assert!(left.is_empty(), "{what}: left behind {left:?}");
    let out = run(dir, &["status"]);
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(
        !text.contains("in progress"),
        "{what}: status still reports an operation:\n{text}"
    );
}

fn revert_conflict(dir: &Path) {
    let target = setup_revert_conflict(dir);
    let out = run(dir, &["revert", &target]);
    assert!(!out.status.success(), "revert should stop on a conflict");
    assert!(dir.join(".mediagit/MERGE_HEAD").exists(), "precondition");
}

fn resolve(dir: &Path, rel: &str) {
    fs::write(dir.join(rel), "resolved\n").unwrap();
    mediagit()
        .args(["add", rel])
        .current_dir(dir)
        .assert()
        .success();
}

#[test]
fn bug020_revert_abort_clears_merge_state() {
    let t = TempDir::new().unwrap();
    revert_conflict(t.path());
    assert!(run(t.path(), &["revert", "--abort"]).status.success());
    assert_no_operation(t.path(), "revert --abort");
}

#[test]
fn bug020_revert_continue_clears_merge_state() {
    let t = TempDir::new().unwrap();
    revert_conflict(t.path());
    resolve(t.path(), "shared.txt");
    let out = run(t.path(), &["revert", "--continue"]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_no_operation(t.path(), "revert --continue");
}

#[test]
fn bug020_revert_skip_clears_merge_state() {
    let t = TempDir::new().unwrap();
    revert_conflict(t.path());
    assert!(run(t.path(), &["revert", "--skip"]).status.success());
    assert_no_operation(t.path(), "revert --skip");
}

/// Returns nothing; cherry-picks the feature tip (full oid: the state file
/// stores the argument as typed) onto a main that edited the same line.
fn setup_cherry_pick_conflict(dir: &Path) {
    init_repo(dir);
    commit_file(
        dir,
        "shared.txt",
        "base
",
        "base",
    );
    mediagit()
        .args(["branch", "create", "feature"])
        .current_dir(dir)
        .assert()
        .success();
    commit_file(
        dir,
        "shared.txt",
        "main-change
",
        "main edit",
    );
    mediagit()
        .args(["branch", "switch", "feature"])
        .current_dir(dir)
        .assert()
        .success();
    commit_file(
        dir,
        "shared.txt",
        "feature-change
",
        "feature edit",
    );
    let tip = fs::read_to_string(dir.join(".mediagit/refs/heads/feature")).unwrap();
    mediagit()
        .args(["branch", "switch", "main"])
        .current_dir(dir)
        .assert()
        .success();
    let out = run(dir, &["cherry-pick", tip.trim()]);
    assert!(
        !out.status.success(),
        "cherry-pick should stop on a conflict"
    );
    assert!(dir.join(".mediagit/MERGE_HEAD").exists(), "precondition");
}

#[test]
fn bug020_cherry_pick_abort_clears_merge_state() {
    let t = TempDir::new().unwrap();
    setup_cherry_pick_conflict(t.path());
    assert!(run(t.path(), &["cherry-pick", "--abort"]).status.success());
    assert_no_operation(t.path(), "cherry-pick --abort");
}

#[test]
fn bug020_cherry_pick_skip_clears_merge_state() {
    let t = TempDir::new().unwrap();
    setup_cherry_pick_conflict(t.path());
    assert!(run(t.path(), &["cherry-pick", "--skip"]).status.success());
    assert_no_operation(t.path(), "cherry-pick --skip");
}

#[test]
fn bug020_cherry_pick_continue_refuses_unresolved_then_clears() {
    let t = TempDir::new().unwrap();
    setup_cherry_pick_conflict(t.path());
    let refused = run(t.path(), &["cherry-pick", "--continue"]);
    assert!(
        !refused.status.success(),
        "continue must refuse while a path is unresolved"
    );
    assert!(t.path().join(".mediagit/CHERRY_PICK_STATE").exists());
    resolve(t.path(), "shared.txt");
    let out = run(t.path(), &["cherry-pick", "--continue"]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_no_operation(t.path(), "cherry-pick --continue");
}

fn setup_rebase_conflict(dir: &Path) {
    setup_conflicting_branches(dir);
    let out = run(dir, &["rebase", "main"]);
    assert!(!out.status.success(), "rebase should stop on a conflict");
    assert!(dir.join(".mediagit/MERGE_HEAD").exists(), "precondition");
}

#[test]
fn bug020_rebase_abort_clears_merge_state() {
    let t = TempDir::new().unwrap();
    setup_rebase_conflict(t.path());
    assert!(run(t.path(), &["rebase", "--abort"]).status.success());
    assert_no_operation(t.path(), "rebase --abort");
}

#[test]
fn bug020_rebase_continue_clears_merge_state() {
    let t = TempDir::new().unwrap();
    setup_rebase_conflict(t.path());
    resolve(t.path(), "shared.txt");
    let out = run(t.path(), &["rebase", "--continue"]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_no_operation(t.path(), "rebase --continue");
}

#[test]
fn bug020_rebase_skip_clears_merge_state() {
    let t = TempDir::new().unwrap();
    setup_rebase_conflict(t.path());
    assert!(run(t.path(), &["rebase", "--skip"]).status.success());
    assert_no_operation(t.path(), "rebase --skip");
}

#[test]
fn bug020_merge_continue_clears_merge_state() {
    let t = TempDir::new().unwrap();
    let d = t.path();
    setup_conflicting_branches(d);
    mediagit()
        .args(["branch", "switch", "main"])
        .current_dir(d)
        .assert()
        .success();
    assert!(!run(d, &["merge", "feature"]).status.success());
    resolve(d, "shared.txt");
    let out = run(d, &["merge", "--continue"]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_no_operation(d, "merge --continue");
}

// ---- Task 3.1 fix round 1 ----

fn status_data(dir: &Path) -> serde_json::Value {
    let out = mediagit()
        .args(["--machine", "status"])
        .current_dir(dir)
        .output()
        .unwrap();
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    v["data"].clone()
}

/// After a conflict is dropped: no markers on disk, nothing unresolved or
/// staged, status clean, and an untracked file is still there.
fn assert_conflict_dropped(dir: &Path, what: &str) {
    assert_no_operation(dir, what);
    let body = fs::read_to_string(dir.join("shared.txt")).unwrap();
    assert!(!body.contains("<<<<<<<"), "{what}: markers left:\n{body}");
    let s = status_data(dir);
    for key in ["unresolved", "staged", "modified"] {
        assert_eq!(
            s[key],
            serde_json::json!([]),
            "{what}: {key} not empty: {s}"
        );
    }
    assert!(dir.join("keep.txt").exists(), "{what}: untracked file lost");
}

fn add_untracked(dir: &Path) {
    fs::write(dir.join("keep.txt"), "mine\n").unwrap();
}

#[test]
fn fix1_revert_skip_drops_the_conflict() {
    let t = TempDir::new().unwrap();
    revert_conflict(t.path());
    add_untracked(t.path());
    assert!(run(t.path(), &["revert", "--skip"]).status.success());
    assert_conflict_dropped(t.path(), "revert --skip");
}

#[test]
fn fix1_cherry_pick_skip_drops_the_conflict() {
    let t = TempDir::new().unwrap();
    setup_cherry_pick_conflict(t.path());
    add_untracked(t.path());
    assert!(run(t.path(), &["cherry-pick", "--skip"]).status.success());
    assert_conflict_dropped(t.path(), "cherry-pick --skip");
}

#[test]
fn fix1_cherry_pick_abort_drops_the_conflict() {
    let t = TempDir::new().unwrap();
    setup_cherry_pick_conflict(t.path());
    add_untracked(t.path());
    assert!(run(t.path(), &["cherry-pick", "--abort"]).status.success());
    assert_conflict_dropped(t.path(), "cherry-pick --abort");
}

#[test]
fn fix1_status_lists_unresolved_without_an_operation_file() {
    let t = TempDir::new().unwrap();
    let d = t.path();
    setup_cherry_pick_conflict(d);
    // A stale flag with every operation file gone must still be visible.
    fs::remove_file(d.join(".mediagit/CHERRY_PICK_STATE")).unwrap();
    for f in ["MERGE_HEAD", "MERGE_MSG", "MERGE_MODE", "ORIG_HEAD"] {
        let _ = fs::remove_file(d.join(".mediagit").join(f));
    }
    let s = status_data(d);
    assert_eq!(s["unresolved"], serde_json::json!(["shared.txt"]), "{s}");
    let human = String::from_utf8_lossy(&run(d, &["status"]).stdout).to_string();
    assert!(
        human.contains("shared.txt") && human.contains("nresolved"),
        "{human}"
    );
}

#[test]
fn fix1_cherry_pick_continue_refusal_text_is_clean() {
    let t = TempDir::new().unwrap();
    setup_cherry_pick_conflict(t.path());
    let out = run(t.path(), &["cherry-pick", "--continue"]);
    let err = String::from_utf8_lossy(&out.stderr).to_string();
    assert!(err.contains("still unresolved"), "{err}");
    for line in err.lines() {
        assert!(
            !line.trim_start().contains("  "),
            "run of spaces inside a sentence: {line:?}"
        );
    }
    assert!(err.contains("`mediagit cherry-pick --abort`"), "{err}");
}

/// A revert that fails while building its commit must stay continuable.
#[test]
fn fix1_revert_continue_keeps_state_when_the_commit_fails() {
    let t = TempDir::new().unwrap();
    let d = t.path();
    revert_conflict(d);
    resolve(d, "shared.txt");
    let state_path = d.join(".mediagit/REVERT_STATE");
    let good = fs::read_to_string(&state_path).unwrap();
    // The reverted commit's id is the last line; make it unreadable.
    let mut lines: Vec<&str> = good.lines().collect();
    let last = lines.len() - 1;
    let bad_id = "0".repeat(64);
    lines[last] = &bad_id;
    fs::write(&state_path, lines.join("\n")).unwrap();
    let failed = run(d, &["revert", "--continue"]);
    assert!(
        !failed.status.success(),
        "precondition: commit build must fail"
    );
    assert!(state_path.exists(), "state lost on a failed continue");
    fs::write(&state_path, good).unwrap();
    let again = run(d, &["revert", "--continue"]);
    assert!(
        again.status.success(),
        "{}",
        String::from_utf8_lossy(&again.stderr)
    );
}

fn snapshot(dir: &Path) -> Vec<(String, Vec<u8>)> {
    ["CHERRY_PICK_STATE", "MERGE_HEAD", "MERGE_MSG", "index"]
        .iter()
        .map(|f| {
            (
                f.to_string(),
                fs::read(dir.join(".mediagit").join(f)).unwrap_or_default(),
            )
        })
        .chain(std::iter::once((
            "shared.txt".to_string(),
            fs::read(dir.join("shared.txt")).unwrap(),
        )))
        .collect()
}

#[test]
fn fix1_refused_cherry_pick_skip_changes_nothing() {
    let t = TempDir::new().unwrap();
    let d = t.path();
    setup_cherry_pick_conflict(d);
    let state_path = d.join(".mediagit/CHERRY_PICK_STATE");
    let mut state: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&state_path).unwrap()).unwrap();
    state["remaining_commits"] = serde_json::json!(["no-such-ref"]);
    fs::write(&state_path, state.to_string()).unwrap();
    let before = snapshot(d);
    let out = run(d, &["cherry-pick", "--skip"]);
    assert!(
        !out.status.success(),
        "skip must refuse an unresolvable pick"
    );
    assert_eq!(before, snapshot(d), "a refused skip changed the repository");
}

#[test]
fn fix1_cherry_pick_skip_moves_on_to_the_next_pick() {
    let t = TempDir::new().unwrap();
    let d = t.path();
    init_repo(d);
    commit_file(d, "shared.txt", "base\n", "base");
    mediagit()
        .args(["branch", "create", "feature"])
        .current_dir(d)
        .assert()
        .success();
    commit_file(d, "shared.txt", "main-change\n", "main edit");
    mediagit()
        .args(["branch", "switch", "feature"])
        .current_dir(d)
        .assert()
        .success();
    commit_file(d, "shared.txt", "feature-change\n", "feature edit");
    let p1 = fs::read_to_string(d.join(".mediagit/refs/heads/feature")).unwrap();
    commit_file(d, "other.txt", "other\n", "feature add");
    let p2 = fs::read_to_string(d.join(".mediagit/refs/heads/feature")).unwrap();
    mediagit()
        .args(["branch", "switch", "main"])
        .current_dir(d)
        .assert()
        .success();
    assert!(
        !run(d, &["cherry-pick", p1.trim(), p2.trim()])
            .status
            .success()
    );
    let out = run(d, &["cherry-pick", "--skip"]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(d.join("other.txt").exists());
    assert_eq!(
        fs::read_to_string(d.join("shared.txt")).unwrap(),
        "main-change\n"
    );
    assert!(!d.join(".mediagit/CHERRY_PICK_STATE").exists());
    assert_no_operation(d, "cherry-pick --skip (two picks)");
}
