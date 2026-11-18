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

// ---- Task 3.2 fix round 1 ----

/// The bytes of `rel` in the commit HEAD points at, `None` if it has no such file.
fn at_head(dir: &Path, rel: &str) -> Option<String> {
    let out = run(dir, &["show", &format!("HEAD:{rel}")]);
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).to_string())
}

fn tip(dir: &Path, branch: &str) -> String {
    fs::read_to_string(dir.join(".mediagit/refs/heads").join(branch))
        .unwrap()
        .trim()
        .to_string()
}

fn switch(dir: &Path, branch: &str) {
    mediagit()
        .args(["branch", "switch", branch])
        .current_dir(dir)
        .assert()
        .success();
}

fn create_branch(dir: &Path, branch: &str) {
    mediagit()
        .args(["branch", "create", branch])
        .current_dir(dir)
        .assert()
        .success();
}

/// `revert A B`: A = "second" (conflicts with "third"), B = the commit that
/// added b.txt. keep.txt is tracked from the start.
fn revert_two_with_conflict(dir: &Path) -> (String, String) {
    init_repo(dir);
    commit_file(dir, "keep.txt", "keep\n", "keep");
    commit_file(dir, "shared.txt", "v1\n", "first");
    commit_file(dir, "shared.txt", "v2\n", "second");
    let a = tip(dir, "main");
    commit_file(dir, "shared.txt", "v3\n", "third");
    commit_file(dir, "b.txt", "b\n", "add b");
    let b = tip(dir, "main");
    let out = run(dir, &["revert", &a, &b]);
    assert!(!out.status.success(), "revert A should stop on a conflict");
    (a, b)
}

/// B1: skip the conflicting revert, then `--continue` as `status` suggested.
/// The tip must still hold every file, and B must be reverted.
#[test]
fn fix2_revert_skip_then_continue_keeps_the_tree_and_applies_the_rest() {
    let t = TempDir::new().unwrap();
    let d = t.path();
    revert_two_with_conflict(d);
    let skip = run(d, &["revert", "--skip"]);
    assert!(
        skip.status.success(),
        "{}",
        String::from_utf8_lossy(&skip.stderr)
    );
    // Whatever skip left, the documented next step must not empty the tip.
    let _ = run(d, &["revert", "--continue"]);
    assert_eq!(at_head(d, "keep.txt").as_deref(), Some("keep\n"));
    assert_eq!(at_head(d, "shared.txt").as_deref(), Some("v3\n"));
    assert_eq!(at_head(d, "b.txt"), None, "B was never reverted");
    assert!(!d.join("b.txt").exists());
    assert_no_operation(d, "revert --skip with a revert left");
}

/// `--continue` on a multi-commit revert commits the resolution, then
/// reverts the rest.
#[test]
fn fix2_revert_continue_applies_the_remaining_reverts() {
    let t = TempDir::new().unwrap();
    let d = t.path();
    revert_two_with_conflict(d);
    resolve(d, "shared.txt");
    let out = run(d, &["revert", "--continue"]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(at_head(d, "keep.txt").as_deref(), Some("keep\n"));
    assert_eq!(at_head(d, "shared.txt").as_deref(), Some("resolved\n"));
    assert_eq!(at_head(d, "b.txt"), None, "B was never reverted");
    assert_no_operation(d, "revert --continue with a revert left");
}

fn assert_refused_without_state(d: &Path, head_before: &str, what: &str) {
    assert!(
        !d.join(".mediagit/CHERRY_PICK_STATE").exists(),
        "{what}: a refusal saved cherry-pick state"
    );
    assert_no_operation(d, what);
    assert!(
        !run(d, &["cherry-pick", "--continue"]).status.success(),
        "{what}: --continue found something to continue"
    );
    assert_eq!(tip(d, "main"), head_before, "{what}: the branch moved");
}

/// J3: an untracked file in the way is a refusal, not a conflict.
#[test]
fn fix2_cherry_pick_untracked_in_the_way_leaves_no_state() {
    let t = TempDir::new().unwrap();
    let d = t.path();
    init_repo(d);
    commit_file(d, "base.txt", "base\n", "base");
    create_branch(d, "feature");
    switch(d, "feature");
    commit_file(d, "f.txt", "theirs\n", "add f");
    let pick = tip(d, "feature");
    switch(d, "main");
    fs::write(d.join("f.txt"), "mine\n").unwrap();
    let before = tip(d, "main");
    let out = run(d, &["cherry-pick", &pick]);
    assert!(!out.status.success(), "the pick must refuse");
    assert_eq!(fs::read_to_string(d.join("f.txt")).unwrap(), "mine\n");
    assert_refused_without_state(d, &before, "untracked in the way");
    assert_eq!(at_head(d, "base.txt").as_deref(), Some("base\n"));
}

/// J3: a root commit cannot be picked; refused before anything changes.
#[test]
fn fix2_cherry_pick_root_commit_leaves_no_state() {
    let t = TempDir::new().unwrap();
    let d = t.path();
    init_repo(d);
    commit_file(d, "base.txt", "base\n", "base");
    let root = tip(d, "main");
    commit_file(d, "base.txt", "next\n", "next");
    let before = tip(d, "main");
    let out = run(d, &["cherry-pick", &root]);
    assert!(!out.status.success(), "a root pick must refuse");
    assert_refused_without_state(d, &before, "root commit");
    assert_eq!(at_head(d, "base.txt").as_deref(), Some("next\n"));
}

/// J1: a conflicting pick of x2 must not bring x1's file, and must keep the
/// change HEAD made to g.txt after the branch point.
#[test]
fn fix2_conflicting_pick_applies_only_its_own_change() {
    let t = TempDir::new().unwrap();
    let d = t.path();
    init_repo(d);
    commit_file(d, "c.txt", "base\n", "base c");
    commit_file(d, "g.txt", "g0\n", "base g");
    create_branch(d, "x");
    switch(d, "x");
    commit_file(d, "f1.txt", "f1\n", "x1");
    commit_file(d, "c.txt", "x2\n", "x2");
    let x2 = tip(d, "x");
    switch(d, "main");
    commit_file(d, "c.txt", "main\n", "main c");
    commit_file(d, "g.txt", "g-main\n", "main g");
    assert!(!run(d, &["cherry-pick", &x2]).status.success());
    assert!(!d.join("f1.txt").exists(), "x1's file written at the stop");
    assert_eq!(fs::read_to_string(d.join("g.txt")).unwrap(), "g-main\n");
    resolve(d, "c.txt");
    let out = run(d, &["cherry-pick", "--continue"]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(at_head(d, "f1.txt"), None, "x1's file was committed");
    assert_eq!(at_head(d, "g.txt").as_deref(), Some("g-main\n"));
    assert_eq!(at_head(d, "c.txt").as_deref(), Some("resolved\n"));
    assert!(!d.join("f1.txt").exists());
}

/// X1: `-n` with several versions is refused before anything is written
/// (a later stop would turn `--continue` into one commit of all of them).
fn assert_no_commit_of_two_refused(d: &Path, args: &[&str], state_file: &str) {
    let before = tip(d, "main");
    let status_before = run(d, &["status", "--porcelain"]).stdout;
    let out = run(d, args);
    assert!(!out.status.success(), "{args:?} must refuse");
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("one at a time"), "{args:?}: {err}");
    assert_eq!(tip(d, "main"), before, "{args:?}: the branch moved");
    assert_eq!(
        run(d, &["status", "--porcelain"]).stdout,
        status_before,
        "{args:?}: files or index changed"
    );
    assert!(
        !d.join(".mediagit").join(state_file).exists(),
        "{args:?}: state saved"
    );
    assert_no_operation(d, "refused -n with two versions");
}

#[test]
fn fix3_cherry_pick_no_commit_with_two_versions_refuses() {
    let t = TempDir::new().unwrap();
    let d = t.path();
    init_repo(d);
    commit_file(d, "base.txt", "base\n", "base");
    create_branch(d, "feature");
    switch(d, "feature");
    commit_file(d, "a.txt", "a\n", "add a");
    let a = tip(d, "feature");
    commit_file(d, "b.txt", "b\n", "add b");
    let b = tip(d, "feature");
    switch(d, "main");
    assert_no_commit_of_two_refused(d, &["cherry-pick", "-n", &a, &b], "CHERRY_PICK_STATE");
    assert!(!d.join("a.txt").exists() && !d.join("b.txt").exists());
}

#[test]
fn fix3_revert_no_commit_with_two_versions_refuses() {
    let t = TempDir::new().unwrap();
    let d = t.path();
    init_repo(d);
    commit_file(d, "a.txt", "a\n", "add a");
    let a = tip(d, "main");
    commit_file(d, "b.txt", "b\n", "add b");
    let b = tip(d, "main");
    assert_no_commit_of_two_refused(d, &["revert", "--no-commit", &a, &b], "REVERT_STATE");
    assert!(d.join("a.txt").exists() && d.join("b.txt").exists());
}

/// X3: `cherry-pick P1 P2` stops on P1. notes.txt is tracked and not part of
/// either pick.
fn pick_two_with_conflict(d: &Path) -> String {
    init_repo(d);
    commit_file(d, "notes.txt", "notes\n", "notes");
    commit_file(d, "shared.txt", "base\n", "base");
    create_branch(d, "feature");
    commit_file(d, "shared.txt", "main-change\n", "main edit");
    switch(d, "feature");
    commit_file(d, "shared.txt", "feature-change\n", "feature edit");
    let p1 = tip(d, "feature");
    commit_file(d, "b.txt", "b\n", "add b");
    let p2 = tip(d, "feature");
    switch(d, "main");
    assert!(!run(d, &["cherry-pick", &p1, &p2]).status.success());
    p2
}

/// X3: the rest of the sequence is refused (an unrelated edit). The state
/// must still list it; once the edit is gone, `--continue` applies it.
fn assert_rest_kept_then_continue(d: &Path, cmd: &str, state_file: &str, rest: &str) {
    let state = fs::read_to_string(d.join(".mediagit").join(state_file))
        .unwrap_or_else(|_| panic!("{cmd}: the refusal forgot the rest (no state)"));
    assert!(state.contains(rest), "{cmd}: state lost {rest}: {state}");
    assert!(
        d.join("b.txt").exists() == (cmd == "revert"),
        "{cmd}: rest ran"
    );
    fs::write(d.join(cmd_edit_file(cmd)), cmd_edit_original(cmd)).unwrap();
    let out = run(d, &[cmd, "--continue"]);
    assert!(
        out.status.success(),
        "{cmd} --continue: {}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(
        at_head(d, "b.txt").is_some(),
        cmd == "cherry-pick",
        "{cmd}: the rest was not applied"
    );
    assert!(!d.join(".mediagit").join(state_file).exists());
    assert_no_operation(d, cmd);
}

fn cmd_edit_file(cmd: &str) -> &'static str {
    if cmd == "revert" {
        "keep.txt"
    } else {
        "notes.txt"
    }
}

fn cmd_edit_original(cmd: &str) -> &'static str {
    if cmd == "revert" { "keep\n" } else { "notes\n" }
}

#[test]
fn fix3_cherry_pick_continue_refused_rest_stays_listed() {
    let t = TempDir::new().unwrap();
    let d = t.path();
    let p2 = pick_two_with_conflict(d);
    fs::write(d.join("notes.txt"), "my edit\n").unwrap();
    resolve(d, "shared.txt");
    assert!(!run(d, &["cherry-pick", "--continue"]).status.success());
    assert_eq!(at_head(d, "shared.txt").as_deref(), Some("resolved\n"));
    assert_rest_kept_then_continue(d, "cherry-pick", "CHERRY_PICK_STATE", &p2);
    assert_eq!(at_head(d, "shared.txt").as_deref(), Some("resolved\n"));
}

#[test]
fn fix3_cherry_pick_skip_refused_rest_stays_listed() {
    let t = TempDir::new().unwrap();
    let d = t.path();
    let p2 = pick_two_with_conflict(d);
    fs::write(d.join("notes.txt"), "my edit\n").unwrap();
    assert!(!run(d, &["cherry-pick", "--skip"]).status.success());
    assert_rest_kept_then_continue(d, "cherry-pick", "CHERRY_PICK_STATE", &p2);
    assert_eq!(at_head(d, "shared.txt").as_deref(), Some("main-change\n"));
}

#[test]
fn fix3_revert_continue_refused_rest_stays_listed() {
    let t = TempDir::new().unwrap();
    let d = t.path();
    let (_, b) = revert_two_with_conflict(d);
    fs::write(d.join("keep.txt"), "my edit\n").unwrap();
    resolve(d, "shared.txt");
    assert!(!run(d, &["revert", "--continue"]).status.success());
    assert_eq!(at_head(d, "shared.txt").as_deref(), Some("resolved\n"));
    assert_rest_kept_then_continue(d, "revert", "REVERT_STATE", &b);
    assert_eq!(at_head(d, "shared.txt").as_deref(), Some("resolved\n"));
}

#[test]
fn fix3_revert_skip_refused_rest_stays_listed() {
    let t = TempDir::new().unwrap();
    let d = t.path();
    let (_, b) = revert_two_with_conflict(d);
    fs::write(d.join("keep.txt"), "my edit\n").unwrap();
    assert!(!run(d, &["revert", "--skip"]).status.success());
    assert_rest_kept_then_continue(d, "revert", "REVERT_STATE", &b);
    assert_eq!(at_head(d, "shared.txt").as_deref(), Some("v3\n"));
}

/// Y1: after the rest was refused for an edit, `--abort` must not wipe it.
#[test]
fn fix4_cherry_pick_abort_after_refused_rest_keeps_the_edit() {
    let t = TempDir::new().unwrap();
    let d = t.path();
    pick_two_with_conflict(d);
    fs::write(
        d.join("notes.txt"),
        "my edit
",
    )
    .unwrap();
    resolve(d, "shared.txt");
    assert!(!run(d, &["cherry-pick", "--continue"]).status.success());
    let head = tip(d, "main");
    let out = run(d, &["cherry-pick", "--abort"]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(
        fs::read_to_string(d.join("notes.txt")).unwrap(),
        "my edit
"
    );
    assert_eq!(tip(d, "main"), head, "abort moved HEAD");
    assert!(!d.join(".mediagit/CHERRY_PICK_STATE").exists());
}

#[test]
fn fix4_revert_abort_after_refused_rest_refuses_and_keeps_the_edit() {
    let t = TempDir::new().unwrap();
    let d = t.path();
    revert_two_with_conflict(d);
    fs::write(
        d.join("keep.txt"),
        "my edit
",
    )
    .unwrap();
    resolve(d, "shared.txt");
    assert!(!run(d, &["revert", "--continue"]).status.success());
    let head = tip(d, "main");
    let out = run(d, &["revert", "--abort"]);
    assert!(!out.status.success(), "abort should refuse");
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("keep.txt"),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(
        fs::read_to_string(d.join("keep.txt")).unwrap(),
        "my edit
"
    );
    assert_eq!(tip(d, "main"), head, "refused abort moved the branch");
    assert!(d.join(".mediagit/REVERT_STATE").exists());
}

/// J4: a tracked file edited during the stop, not part of the conflict,
/// survives `--skip`.
#[test]
fn fix2_cherry_pick_skip_keeps_unrelated_edits() {
    let t = TempDir::new().unwrap();
    let d = t.path();
    init_repo(d);
    commit_file(d, "notes.txt", "notes\n", "notes");
    commit_file(d, "shared.txt", "base\n", "base");
    create_branch(d, "feature");
    commit_file(d, "shared.txt", "main-change\n", "main edit");
    switch(d, "feature");
    commit_file(d, "shared.txt", "feature-change\n", "feature edit");
    let pick = tip(d, "feature");
    switch(d, "main");
    assert!(!run(d, &["cherry-pick", &pick]).status.success());
    fs::write(d.join("notes.txt"), "my edit\n").unwrap();
    let out = run(d, &["cherry-pick", "--skip"]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(
        fs::read_to_string(d.join("notes.txt")).unwrap(),
        "my edit\n",
        "skip overwrote an edit it had no part in"
    );
    assert_eq!(
        fs::read_to_string(d.join("shared.txt")).unwrap(),
        "main-change\n"
    );
}

/// J4, revert side.
#[test]
fn fix2_revert_skip_keeps_unrelated_edits() {
    let t = TempDir::new().unwrap();
    let d = t.path();
    init_repo(d);
    commit_file(d, "keep.txt", "keep\n", "keep");
    commit_file(d, "shared.txt", "v1\n", "first");
    commit_file(d, "shared.txt", "v2\n", "second");
    let a = tip(d, "main");
    commit_file(d, "shared.txt", "v3\n", "third");
    assert!(!run(d, &["revert", &a]).status.success());
    fs::write(d.join("keep.txt"), "my edit\n").unwrap();
    let out = run(d, &["revert", "--skip"]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(
        fs::read_to_string(d.join("keep.txt")).unwrap(),
        "my edit\n",
        "skip overwrote an edit it had no part in"
    );
    assert_eq!(fs::read_to_string(d.join("shared.txt")).unwrap(), "v3\n");
}

// ============================================================================
// BUG-029 — keeping the bytes HEAD already has must still resolve the path.
// ============================================================================

/// Write `bytes`, stage the file, run `--continue`, and expect a clean finish.
fn keep_and_continue(dir: &Path, cmd: &str, bytes: &str) {
    fs::write(dir.join("shared.txt"), bytes).unwrap();
    let add = run(dir, &["add", "--", "shared.txt"]);
    assert!(
        add.status.success(),
        "{}",
        String::from_utf8_lossy(&add.stderr)
    );
    let out = run(dir, &[cmd, "--continue"]);
    assert!(
        out.status.success(),
        "{cmd} --continue after keeping HEAD's bytes: {}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert_no_operation(dir, cmd);
}

#[test]
fn bug029_rebase_keeping_head_bytes_resolves() {
    let t = TempDir::new().unwrap();
    setup_rebase_conflict(t.path());
    keep_and_continue(t.path(), "rebase", "feature-change\n");
}

/// Keeping HEAD's bytes makes the pick empty. The path must still count as decided
/// (so `--continue` gets past the unresolved check), and `--skip` then ends it cleanly.
#[test]
fn bug029_cherry_pick_keeping_head_bytes_resolves() {
    let t = TempDir::new().unwrap();
    let d = t.path();
    setup_cherry_pick_conflict(d);
    fs::write(
        d.join("shared.txt"),
        "main-change
",
    )
    .unwrap();
    assert!(run(d, &["add", "--", "shared.txt"]).status.success());
    let out = run(d, &["cherry-pick", "--continue"]);
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        !text.contains("unresolved"),
        "path still unresolved: {text}"
    );
    assert!(!out.status.success(), "an empty pick must stop: {text}");
    assert!(text.contains("nothing to commit"), "{text}");
    assert!(run(d, &["cherry-pick", "--skip"]).status.success());
    assert_no_operation(d, "cherry-pick");
}

#[test]
fn bug029_revert_keeping_head_bytes_resolves() {
    let t = TempDir::new().unwrap();
    let d = t.path();
    revert_conflict(d);
    let head_before = tip(d, "main");
    fs::write(d.join("shared.txt"), "v3\n").unwrap();
    assert!(run(d, &["add", "--", "shared.txt"]).status.success());
    // DL-8: the resolution equals HEAD, so there is nothing to commit.
    let out = run(d, &["revert", "--continue"]);
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        !text.contains("unresolved"),
        "path still unresolved: {text}"
    );
    assert!(!out.status.success(), "an empty revert must stop: {text}");
    assert!(text.contains("nothing to commit"), "{text}");
    assert_eq!(head_before, tip(d, "main"), "the branch moved");
    assert!(run(d, &["revert", "--skip"]).status.success());
    assert_no_operation(d, "revert");
}

/// The side that is "ours" in a rebase is the commit being built on, not HEAD.
#[test]
fn bug029_status_names_the_commit_holding_our_side() {
    let t = TempDir::new().unwrap();
    let d = t.path();
    let ours = |d: &Path| -> String {
        let out = run(d, &["status", "--machine"]);
        let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
        v["data"]["operation"]["ours_commit"]
            .as_str()
            .unwrap_or_default()
            .to_owned()
    };
    setup_rebase_conflict(d);
    assert_eq!(
        ours(d),
        tip(d, "main"),
        "rebase: our side is the upstream tip"
    );
    let t2 = TempDir::new().unwrap();
    setup_cherry_pick_conflict(t2.path());
    assert_eq!(
        ours(t2.path()),
        tip(t2.path(), "main"),
        "cherry-pick: our side is HEAD"
    );
}

/// A merge stopped on a binary clash at art.bin. The working copy holds our
/// side, whose bytes equal HEAD. notes.txt is a tracked file with no clash.
fn setup_binary_merge_clash(dir: &Path) {
    init_repo(dir);
    commit_file(dir, "notes.txt", "notes\n", "notes");
    commit_binary(dir, "art.bin", &binary_blob(0x00), "base art");
    switch_new(dir, "other");
    commit_binary(dir, "art.bin", &binary_blob(0x22), "other art");
    switch(dir, "main");
    commit_binary(dir, "art.bin", &binary_blob(0x11), "main art");
    let out = run(dir, &["merge", "other"]);
    assert!(!out.status.success(), "merge should stop on a binary clash");
    assert_eq!(
        status_data(dir)["unresolved"],
        serde_json::json!(["art.bin"])
    );
}

fn switch_new(dir: &Path, branch: &str) {
    for args in [["branch", "create", branch], ["branch", "switch", branch]] {
        mediagit().args(args).current_dir(dir).assert().success();
    }
}

/// Fix round 1, M1: a folder walk must not decide a clash; naming the file does.
#[test]
fn bug029_only_a_named_file_decides_a_clash() {
    let t = TempDir::new().unwrap();
    let d = t.path();
    setup_binary_merge_clash(d);
    fs::write(d.join("notes.txt"), "edited\n").unwrap();
    assert!(run(d, &["add", "."]).status.success());
    assert_eq!(
        status_data(d)["unresolved"],
        serde_json::json!(["art.bin"]),
        "add . decided a clash nobody named"
    );
    assert!(run(d, &["add", "--", "art.bin"]).status.success());
    assert_eq!(status_data(d)["unresolved"], serde_json::json!([]));
}

/// Fix round 1: deleting a clashing file is a decision; staging the deletion clears it.
#[test]
fn bug029_deleting_a_clashing_file_decides_it() {
    let t = TempDir::new().unwrap();
    let d = t.path();
    setup_binary_merge_clash(d);
    fs::remove_file(d.join("art.bin")).unwrap();
    assert!(run(d, &["add", "-A"]).status.success());
    assert_eq!(
        status_data(d)["unresolved"],
        serde_json::json!([]),
        "staged deletion left the clash unresolved"
    );
    let out = run(d, &["merge", "--continue"]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(!d.join("art.bin").exists());
    assert_eq!(status_data(d)["deleted"], serde_json::json!([]));
}

/// We deleted art.bin, they edited it. Deleting their copy keeps our deletion;
/// the provisional staged copy must not reach the commit.
#[test]
fn bug029_deleting_their_copy_keeps_our_deletion() {
    let t = TempDir::new().unwrap();
    let d = t.path();
    init_repo(d);
    commit_file(d, "notes.txt", "notes\n", "notes");
    commit_binary(d, "art.bin", &binary_blob(0x00), "base art");
    switch_new(d, "other");
    commit_binary(d, "art.bin", &binary_blob(0x22), "other art");
    switch(d, "main");
    fs::remove_file(d.join("art.bin")).unwrap();
    assert!(run(d, &["add", "-A"]).status.success());
    assert!(run(d, &["commit", "-m", "drop art"]).status.success());
    assert!(!run(d, &["merge", "other"]).status.success());
    assert_eq!(status_data(d)["unresolved"], serde_json::json!(["art.bin"]));
    fs::remove_file(d.join("art.bin")).unwrap();
    assert!(run(d, &["add", "-A"]).status.success());
    assert_eq!(status_data(d)["unresolved"], serde_json::json!([]));
    let out = run(d, &["merge", "--continue"]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let s = status_data(d);
    assert_eq!(
        s["deleted"],
        serde_json::json!([]),
        "their copy committed: {s}"
    );
}

/// Y4: a path an earlier branch commit adds and the stopping commit deletes
/// is written at the stop. An untracked file there must stop it, not be
/// overwritten.
#[test]
fn fix4_rebase_stop_does_not_overwrite_an_untracked_file() {
    let t = TempDir::new().unwrap();
    let d = t.path();
    init_repo(d);
    commit_file(
        d,
        "shared.txt",
        "base
",
        "base",
    );
    create_branch(d, "feature");
    commit_file(
        d,
        "shared.txt",
        "main-change
",
        "main edit",
    );
    switch(d, "feature");
    commit_file(
        d, "x.txt", "x
", "add x",
    );
    fs::remove_file(d.join("x.txt")).unwrap();
    fs::write(
        d.join("shared.txt"),
        "feature-change
",
    )
    .unwrap();
    assert!(run(d, &["add", "-A"]).status.success());
    assert!(
        run(d, &["commit", "-m", "drop x, edit shared"])
            .status
            .success()
    );
    fs::write(
        d.join("x.txt"),
        "mine
",
    )
    .unwrap();
    let out = run(d, &["rebase", "main"]);
    assert!(!out.status.success(), "rebase should refuse");
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("x.txt"), "{err}");
    assert_eq!(
        fs::read_to_string(d.join("x.txt")).unwrap(),
        "mine
"
    );
    assert_no_operation(d, "refused rebase");
    assert_eq!(
        fs::read_to_string(d.join("x.txt")).unwrap(),
        "mine
"
    );
}

/// Fix round 1: during a rebase stop, `add` compares against the commit being
/// built on. A file edited back to the moved branch's bytes was skipped as
/// "unchanged" and dropped from the resolved commit.
#[test]
fn rebase_add_compares_against_the_commit_being_built_on() {
    let t = TempDir::new().unwrap();
    let d = t.path();
    init_repo(d);
    commit_file(d, "shared.txt", "base\n", "base");
    commit_file(d, "extra.txt", "base\n", "extra");
    switch_new(d, "feature");
    commit_file(d, "shared.txt", "feature-change\n", "feature edit");
    switch(d, "main");
    commit_file(d, "shared.txt", "main-change\n", "main edit");
    commit_file(d, "extra.txt", "main\n", "main extra");
    switch(d, "feature");
    assert!(!run(d, &["rebase", "main"]).status.success());
    assert_eq!(fs::read_to_string(d.join("extra.txt")).unwrap(), "main\n");
    // The person puts extra.txt back to feature's bytes and decides shared.txt.
    fs::write(d.join("extra.txt"), "base\n").unwrap();
    fs::write(d.join("shared.txt"), "resolved\n").unwrap();
    assert!(
        run(d, &["add", "--", "extra.txt", "shared.txt"])
            .status
            .success()
    );
    let out = run(d, &["rebase", "--continue"]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(fs::read_to_string(d.join("extra.txt")).unwrap(), "base\n");
    let s = status_data(d);
    assert_eq!(s["modified"], serde_json::json!([]), "edit dropped: {s}");
}

/// Fix round 2 (F1): "unchanged" means equal to what is already staged. A
/// cleanly merged file holds feature's bytes in the index; putting it back to
/// the new base's bytes must restage it, not skip it as unchanged.
#[test]
fn rebase_add_restages_a_clean_file_put_back_to_the_base() {
    let t = TempDir::new().unwrap();
    let d = t.path();
    init_repo(d);
    commit_file(d, "shared.txt", "base\n", "base");
    commit_file(d, "extra.txt", "base\n", "extra");
    switch_new(d, "feature");
    fs::write(d.join("shared.txt"), "feature-change\n").unwrap();
    fs::write(d.join("extra.txt"), "feature\n").unwrap();
    assert!(run(d, &["add", "shared.txt", "extra.txt"]).status.success());
    assert!(run(d, &["commit", "-m", "feature both"]).status.success());
    switch(d, "main");
    commit_file(d, "shared.txt", "main-change\n", "main edit");
    switch(d, "feature");
    assert!(!run(d, &["rebase", "main"]).status.success());
    // extra.txt merged cleanly: feature's bytes are staged. Put it back to main's.
    fs::write(d.join("extra.txt"), "base\n").unwrap();
    fs::write(d.join("shared.txt"), "resolved\n").unwrap();
    assert!(
        run(d, &["add", "--", "extra.txt", "shared.txt"])
            .status
            .success()
    );
    let out = run(d, &["rebase", "--continue"]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(
        fs::read_to_string(d.join("extra.txt")).unwrap(),
        "base\n",
        "feature bytes committed over the person's choice"
    );
}

/// Fix round 2 (F2): a file the picked commit added cleanly, deleted by hand
/// and staged with `add -A`, must not come back in the rebased commit.
#[test]
fn rebase_add_all_stages_deletion_of_a_pick_added_file() {
    let t = TempDir::new().unwrap();
    let d = t.path();
    init_repo(d);
    commit_file(d, "shared.txt", "base\n", "base");
    switch_new(d, "feature");
    fs::write(d.join("shared.txt"), "feature-change\n").unwrap();
    fs::write(d.join("added.txt"), "new\n").unwrap();
    assert!(run(d, &["add", "shared.txt", "added.txt"]).status.success());
    assert!(run(d, &["commit", "-m", "feature both"]).status.success());
    switch(d, "main");
    commit_file(d, "shared.txt", "main-change\n", "main edit");
    switch(d, "feature");
    assert!(!run(d, &["rebase", "main"]).status.success());
    // added.txt merged cleanly and is staged; the person deletes it.
    fs::remove_file(d.join("added.txt")).unwrap();
    fs::write(d.join("shared.txt"), "resolved\n").unwrap();
    assert!(run(d, &["add", "-A"]).status.success());
    let out = run(d, &["rebase", "--continue"]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(!d.join("added.txt").exists(), "deleted file came back");
}

/// Fix round 4: `rebase --skip` drops the stopped commit only. The next
/// commit must still bring its change.
#[test]
fn fix4_rebase_skip_keeps_the_next_commits_change() {
    let t = TempDir::new().unwrap();
    let d = t.path();
    init_repo(d);
    commit_file(d, "shared.txt", "base\n", "base");
    switch_new(d, "feature");
    commit_file(d, "shared.txt", "feature-change\n", "feature edit");
    commit_file(d, "extra.txt", "extra\n", "add extra");
    switch(d, "main");
    commit_file(d, "shared.txt", "main-change\n", "main edit");
    switch(d, "feature");
    assert!(!run(d, &["rebase", "main"]).status.success());
    let out = run(d, &["rebase", "--skip"]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(fs::read_to_string(d.join("extra.txt")).unwrap(), "extra\n");
    let log = run(d, &["log", "--oneline"]);
    let log = String::from_utf8_lossy(&log.stdout);
    assert!(log.contains("add extra"), "{log}");
    // The file is in the commit: deleting it shows as a deletion.
    fs::remove_file(d.join("extra.txt")).unwrap();
    let s = status_data(d);
    assert_eq!(s["deleted"], serde_json::json!(["extra.txt"]), "{s}");
}

/// Fix round 4 (Y6): C1 and C3 conflict with main, C2 adds x.txt, C3 deletes
/// it. An untracked x.txt makes the second stop refuse. The saved state must
/// then match the disk.
fn setup_later_stop_refusal(dir: &Path) {
    init_repo(dir);
    commit_file(dir, "shared.txt", "base\n", "base");
    switch_new(dir, "feature");
    commit_file(dir, "shared.txt", "f1\n", "C1");
    commit_file(dir, "x.txt", "x\n", "C2 add x");
    fs::remove_file(dir.join("x.txt")).unwrap();
    fs::write(dir.join("shared.txt"), "f3\n").unwrap();
    assert!(run(dir, &["add", "-A"]).status.success());
    assert!(run(dir, &["commit", "-m", "C3 drop x"]).status.success());
    switch(dir, "main");
    commit_file(dir, "shared.txt", "main\n", "main edit");
    switch(dir, "feature");
    fs::write(dir.join("x.txt"), "mine\n").unwrap();
    assert!(!run(dir, &["rebase", "main"]).status.success());
    fs::write(dir.join("shared.txt"), "r1\n").unwrap();
    assert!(run(dir, &["add", "shared.txt"]).status.success());
    let out = run(dir, &["rebase", "--continue"]);
    assert!(!out.status.success(), "second stop should refuse");
    assert!(String::from_utf8_lossy(&out.stderr).contains("x.txt"));
}

#[test]
fn fix4_later_stop_refusal_then_abort_keeps_the_untracked_file() {
    let t = TempDir::new().unwrap();
    let d = t.path();
    setup_later_stop_refusal(d);
    assert!(run(d, &["rebase", "--abort"]).status.success());
    assert_eq!(fs::read_to_string(d.join("x.txt")).unwrap(), "mine\n");
}

#[test]
fn fix4_later_stop_refusal_then_continue_keeps_the_stopping_commit() {
    let t = TempDir::new().unwrap();
    let d = t.path();
    setup_later_stop_refusal(d);
    fs::rename(d.join("x.txt"), d.join("x.moved")).unwrap();
    let out = run(d, &["rebase", "--continue"]);
    assert!(!out.status.success(), "C3 should stop on its conflict");
    fs::write(d.join("shared.txt"), "r3\n").unwrap();
    assert!(run(d, &["add", "shared.txt"]).status.success());
    let out = run(d, &["rebase", "--continue"]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(!d.join("x.txt").exists());
    assert_eq!(fs::read_to_string(d.join("shared.txt")).unwrap(), "r3\n");
}

/// Fix round 5 (P2): a refused rebase has no stopped commit. `--skip` must
/// refuse and change nothing, so the clean commit C2 is not dropped.
#[test]
fn fix5_skip_without_a_stopped_commit_refuses() {
    let t = TempDir::new().unwrap();
    let d = t.path();
    setup_later_stop_refusal(d);
    let out = run(d, &["rebase", "--skip"]);
    assert!(!out.status.success(), "skip must refuse");
    assert!(String::from_utf8_lossy(&out.stderr).contains("No stopped commit"));
    fs::rename(d.join("x.txt"), d.join("x.moved")).unwrap();
    assert!(!run(d, &["rebase", "--continue"]).status.success());
    fs::write(
        d.join("shared.txt"),
        "r3
",
    )
    .unwrap();
    assert!(run(d, &["add", "shared.txt"]).status.success());
    assert!(run(d, &["rebase", "--continue"]).status.success());
    let log = String::from_utf8_lossy(&run(d, &["log", "--oneline"]).stdout).to_string();
    assert!(log.contains("C2 add x"), "C2 was dropped: {log}");
}

/// Fix round 5 (P1): any non-conflict error at a stop (here a new case-only
/// name pair at commit time) must leave the state matching the disk.
#[test]
fn fix5_case_collision_refusal_keeps_the_commit() {
    let t = TempDir::new().unwrap();
    let d = t.path();
    init_repo(d);
    commit_file(
        d, "a.txt", "a
", "base",
    );
    switch_new(d, "feature");
    commit_file(
        d,
        "foo.txt",
        "f
",
        "C1 add foo",
    );
    commit_file(
        d,
        "a.txt",
        "a2
",
        "C2 edit a",
    );
    switch(d, "main");
    commit_file(
        d,
        "Foo.txt",
        "F
",
        "main add Foo",
    );
    switch(d, "feature");
    let out = run(d, &["rebase", "main"]);
    assert!(!out.status.success(), "case pair should be refused");
    let out = run(d, &["rebase", "--continue"]);
    assert!(
        !out.status.success(),
        "continue must not commit an empty change"
    );
    let log = String::from_utf8_lossy(&run(d, &["log", "--oneline"]).stdout).to_string();
    assert!(
        log.contains("C1 add foo") && !log.contains("main add Foo"),
        "{log}"
    );
    assert_eq!(
        fs::read_to_string(d.join("foo.txt")).unwrap(),
        "f
"
    );
}

/// R1: a stop that failed after its first disk write leaves a stopped commit
/// with no recorded conflicts. `--continue` must refuse, not commit it as the
/// tip's tree and lose its change.
#[test]
fn r1_continue_refuses_stopped_commit_without_conflicts() {
    let t = TempDir::new().unwrap();
    let d = t.path();
    init_repo(d);
    commit_file(
        d,
        "shared.txt",
        "base
",
        "base",
    );
    switch_new(d, "feature");
    commit_file(
        d,
        "shared.txt",
        "f1
",
        "C1",
    );
    switch(d, "main");
    commit_file(
        d,
        "shared.txt",
        "main
",
        "main edit",
    );
    switch(d, "feature");
    assert!(!run(d, &["rebase", "main"]).status.success());
    let sf = d.join(".mediagit/rebase-apply/state.json");
    let mut v: serde_json::Value = serde_json::from_slice(&fs::read(&sf).unwrap()).unwrap();
    v["conflict_files"] = serde_json::json!([]);
    fs::write(&sf, serde_json::to_vec(&v).unwrap()).unwrap();
    fs::write(
        d.join("shared.txt"),
        "r1
",
    )
    .unwrap();
    assert!(run(d, &["add", "shared.txt"]).status.success());
    let out = run(d, &["rebase", "--continue"]);
    assert!(!out.status.success(), "continue must refuse");
    assert!(String::from_utf8_lossy(&out.stderr).contains("rebase --abort"));
    assert!(sf.exists(), "state kept");
}

/// DL-7: `rebase --skip` must put the skipped commit's conflict markers
/// away. The next stop used to find them still in the tree, where `add -A`
/// would commit them.
#[test]
fn dl7_rebase_skip_removes_the_skipped_commits_markers() {
    let t = TempDir::new().unwrap();
    let d = t.path();
    init_repo(d);
    commit_file(d, "shared.txt", "base\n", "base");
    commit_file(d, "other.txt", "base\n", "base other");
    create_branch(d, "feature");
    commit_file(d, "shared.txt", "main-change\n", "main edit");
    commit_file(d, "other.txt", "main-other\n", "main edit other");
    switch(d, "feature");
    commit_file(d, "shared.txt", "feature-change\n", "feature edit");
    commit_file(d, "other.txt", "feature-other\n", "feature edit other");
    assert!(!run(d, &["rebase", "main"]).status.success());
    // The first commit stopped; skip it. The second also conflicts and stops.
    let skip = run(d, &["rebase", "--skip"]);
    assert!(
        !skip.status.success(),
        "second commit should stop on its own conflict"
    );
    let shared = fs::read_to_string(d.join("shared.txt")).unwrap();
    assert_eq!(shared, "main-change\n", "skipped commit's markers remain");
}

// ============================================================================
// A stopped operation's MERGE_MSG names that operation, not "Merge branch".
// ============================================================================

fn merge_msg(dir: &Path) -> String {
    fs::read_to_string(dir.join(".mediagit/MERGE_MSG")).unwrap()
}

#[test]
fn stopped_revert_writes_its_own_merge_msg() {
    let t = TempDir::new().unwrap();
    revert_conflict(t.path());
    let msg = merge_msg(t.path());
    assert!(msg.starts_with("Revert \"second\""), "{msg}");
}

#[test]
fn stopped_cherry_pick_writes_the_picked_message() {
    let t = TempDir::new().unwrap();
    setup_cherry_pick_conflict(t.path());
    assert_eq!(merge_msg(t.path()).trim(), "feature edit");
}

#[test]
fn stopped_rebase_writes_the_replayed_message() {
    let t = TempDir::new().unwrap();
    setup_rebase_conflict(t.path());
    assert_eq!(merge_msg(t.path()).trim(), "feature edit");
}
