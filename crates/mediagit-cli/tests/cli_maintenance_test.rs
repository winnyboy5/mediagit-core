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

//! Comprehensive CLI Maintenance Command Tests
//!
//! Tests for `gc`, `fsck`, `verify`, and `stats` commands.

use assert_cmd::Command;
use predicates::prelude::*;
use std::fs;
use std::path::Path;
use std::time::Instant;
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
        .arg("init")
        .arg("-q")
        .current_dir(dir)
        .assert()
        .success();
}

fn add_and_commit(dir: &Path, name: &str, content: &str, message: &str) {
    fs::write(dir.join(name), content).unwrap();
    mediagit()
        .arg("add")
        .arg(name)
        .current_dir(dir)
        .assert()
        .success();
    mediagit()
        .arg("commit")
        .arg("-m")
        .arg(message)
        .current_dir(dir)
        .assert()
        .success();
}

// ============================================================================
// GC Command Tests
// ============================================================================

#[test]
fn test_gc_basic() {
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());

    // Create some commits to have objects to GC
    for i in 1..=5 {
        add_and_commit(
            temp_dir.path(),
            &format!("file{}.txt", i),
            &format!("Content {}", i),
            &format!("Commit {}", i),
        );
    }

    let start = Instant::now();
    mediagit()
        .arg("gc")
        .current_dir(temp_dir.path())
        .assert()
        .success();

    println!("GC duration: {:?}", start.elapsed());
}

#[test]
fn test_gc_aggressive() {
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());

    for i in 1..=3 {
        add_and_commit(
            temp_dir.path(),
            &format!("file{}.txt", i),
            &format!("Content {}", i),
            &format!("Commit {}", i),
        );
    }

    mediagit()
        .arg("gc")
        .arg("--aggressive")
        .current_dir(temp_dir.path())
        .assert()
        .success();
}

#[test]
fn test_gc_dry_run() {
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());

    add_and_commit(temp_dir.path(), "file.txt", "Content", "Initial commit");

    mediagit()
        .arg("gc")
        .arg("--dry-run")
        .current_dir(temp_dir.path())
        .assert()
        .success();
}

#[test]
fn test_gc_auto() {
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());

    add_and_commit(temp_dir.path(), "file.txt", "Content", "Initial commit");

    mediagit()
        .arg("gc")
        .arg("--auto")
        .current_dir(temp_dir.path())
        .assert()
        .success();
}

#[test]
fn test_gc_no_prune() {
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());

    add_and_commit(temp_dir.path(), "file.txt", "Content", "Initial commit");

    // Test with --no-prune flag (gc prunes by default)
    mediagit()
        .arg("gc")
        .arg("--no-prune")
        .current_dir(temp_dir.path())
        .assert()
        .success();
}

#[test]
fn test_gc_quiet() {
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());

    add_and_commit(temp_dir.path(), "file.txt", "Content", "Initial commit");

    mediagit()
        .arg("gc")
        .arg("-q")
        .current_dir(temp_dir.path())
        .assert()
        .success();
}

#[test]
fn test_gc_verbose() {
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());

    add_and_commit(temp_dir.path(), "file.txt", "Content", "Initial commit");

    mediagit()
        .arg("gc")
        .arg("-v")
        .current_dir(temp_dir.path())
        .assert()
        .success();
}

// ============================================================================
// Bitmap Maintenance Tests (M3, #2b)
// ============================================================================

/// `gc` must regenerate a reachability bitmap for every current branch tip,
/// and must never treat the `bitmaps/` namespace as orphan data in the
/// unreachable-object sweep (derived data — see `mediagit_versioning::bitmap`).
#[test]
fn test_gc_regenerates_bitmaps_for_branch_tips() {
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());

    add_and_commit(temp_dir.path(), "a.txt", "content a", "commit A");

    mediagit()
        .arg("branch")
        .arg("create")
        .arg("feature")
        .current_dir(temp_dir.path())
        .assert()
        .success();
    mediagit()
        .arg("branch")
        .arg("switch")
        .arg("feature")
        .current_dir(temp_dir.path())
        .assert()
        .success();
    add_and_commit(temp_dir.path(), "b.txt", "content b", "commit B");

    // Two branch tips (main, feature) -> two bitmaps regenerated, none
    // pruned (nothing unreachable yet).
    mediagit()
        .arg("gc")
        .current_dir(temp_dir.path())
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "Regenerated 2 bitmap(s), pruned 0 orphaned bitmap(s)",
        ));
}

/// Deleting a branch makes its tip commit (and the bitmap seeded for it by
/// the prior `gc`) unreachable. The next `gc` must prune that bitmap and
/// must not flag it as corruption — `fsck` stays clean throughout.
#[test]
fn test_gc_prunes_bitmap_for_deleted_branch() {
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());

    add_and_commit(temp_dir.path(), "a.txt", "content a", "commit A");
    mediagit()
        .arg("branch")
        .arg("create")
        .arg("feature")
        .current_dir(temp_dir.path())
        .assert()
        .success();
    mediagit()
        .arg("branch")
        .arg("switch")
        .arg("feature")
        .current_dir(temp_dir.path())
        .assert()
        .success();
    add_and_commit(temp_dir.path(), "b.txt", "content b", "commit B");

    // First gc: seeds bitmaps for both main (A) and feature (B).
    mediagit()
        .arg("gc")
        .current_dir(temp_dir.path())
        .assert()
        .success();

    // Switch off feature so it can be deleted, then delete it — B is now
    // unreachable, but its bitmap file is still sitting on disk.
    mediagit()
        .arg("branch")
        .arg("switch")
        .arg("main")
        .current_dir(temp_dir.path())
        .assert()
        .success();
    mediagit()
        .arg("branch")
        .arg("delete")
        .arg("feature")
        .arg("-D")
        .current_dir(temp_dir.path())
        .assert()
        .success();

    // Second gc: must prune exactly the orphaned (feature/B) bitmap and
    // regenerate exactly the surviving (main/A) one. Reflog protection is
    // disabled so B is truly unreachable (like git's gc.reflogExpire=now).
    mediagit()
        .arg("gc")
        .env("MEDIAGIT_GC_REFLOG_HORIZON_DAYS", "0")
        .current_dir(temp_dir.path())
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "Regenerated 1 bitmap(s), pruned 1 orphaned bitmap(s)",
        ));

    // The prune must never register as a corruption finding.
    mediagit()
        .arg("fsck")
        .current_dir(temp_dir.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("PERFECT"));
}

/// `MEDIAGIT_BITMAP=0` disables bitmap generation entirely — gc must not
/// print the bitmap-maintenance step at all, and must still succeed.
#[test]
fn test_gc_skips_bitmap_maintenance_when_disabled() {
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());
    add_and_commit(temp_dir.path(), "a.txt", "content a", "commit A");

    mediagit()
        .arg("gc")
        .env("MEDIAGIT_BITMAP", "0")
        .current_dir(temp_dir.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("bitmap").not());
}

/// M5a regression: a commit reachable ONLY through an annotated tag (no
/// branch points at it) must survive `gc` — this is the exact reachability
/// gap the M3 review flagged for M5 to close (`walk_reachable`/gc's
/// `build_reachability_set` must walk THROUGH a Tag object to its target).
#[test]
fn test_gc_preserves_commit_reachable_only_via_annotated_tag() {
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());

    add_and_commit(temp_dir.path(), "a.txt", "content a", "commit A");
    add_and_commit(temp_dir.path(), "b.txt", "content b", "commit B");

    // Tag commit B (currently HEAD) with an annotated tag before it gets
    // orphaned from the branch.
    mediagit()
        .arg("tag")
        .arg("create")
        .arg("relB")
        .arg("-a")
        .arg("-m")
        .arg("release B")
        .current_dir(temp_dir.path())
        .assert()
        .success();

    // Read commit B's OID back out via `tag show` (parses the "Commit:"
    // line) rather than the ref file directly — refs are postcard-binary,
    // not plain hex text.
    let show_output = mediagit()
        .arg("tag")
        .arg("show")
        .arg("relB")
        .current_dir(temp_dir.path())
        .output()
        .unwrap();
    let show_stdout = String::from_utf8_lossy(&show_output.stdout);
    let commit_b_oid = show_stdout
        .lines()
        .find_map(|l| l.strip_prefix("Commit:  "))
        .expect("tag show must print a Commit: line")
        .trim()
        .to_string();

    // main moves back to A; B is now reachable ONLY through the tag.
    mediagit()
        .arg("reset")
        .arg("--hard")
        .arg("HEAD~1")
        .current_dir(temp_dir.path())
        .assert()
        .success();

    mediagit()
        .arg("gc")
        .current_dir(temp_dir.path())
        .assert()
        .success();

    // Commit B's object must still be readable after gc.
    mediagit()
        .arg("show")
        .arg(&commit_b_oid)
        .current_dir(temp_dir.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("commit B"));

    mediagit()
        .arg("fsck")
        .current_dir(temp_dir.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("PERFECT"));
}

// ============================================================================
// Auto-GC Trigger Tests (post-commit/pull/clone)
// ============================================================================

/// Commit succeeds with auto-gc enabled (default). The threshold-gated
/// auto-gc is silent when there's nothing significant to reclaim, so this
/// is primarily a smoke test that the trigger doesn't break commit.
#[test]
fn test_commit_runs_with_auto_gc_enabled() {
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());
    add_and_commit(temp_dir.path(), "file.txt", "v1", "first");
    // Re-stage and commit to exercise the orphan-creating path.
    fs::write(temp_dir.path().join("file.txt"), "v2").unwrap();
    mediagit()
        .arg("add")
        .arg("file.txt")
        .current_dir(temp_dir.path())
        .assert()
        .success();
    mediagit()
        .arg("commit")
        .arg("-m")
        .arg("v2")
        .current_dir(temp_dir.path())
        .assert()
        .success();
}

/// Env var opt-out: setting MEDIAGIT_NO_AUTO_GC=1 must not break commit.
#[test]
fn test_commit_with_auto_gc_disabled_via_env() {
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());
    add_and_commit(temp_dir.path(), "file.txt", "v1", "first");
    fs::write(temp_dir.path().join("file.txt"), "v2").unwrap();
    mediagit()
        .arg("add")
        .arg("file.txt")
        .current_dir(temp_dir.path())
        .assert()
        .success();
    mediagit()
        .env("MEDIAGIT_NO_AUTO_GC", "1")
        .arg("commit")
        .arg("-m")
        .arg("v2")
        .current_dir(temp_dir.path())
        .assert()
        .success();
}

/// Per-repo opt-out via marker file `.mediagit/no-autogc`.
#[test]
fn test_commit_with_auto_gc_disabled_via_marker() {
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());
    fs::write(temp_dir.path().join(".mediagit").join("no-autogc"), b"").unwrap();
    add_and_commit(temp_dir.path(), "file.txt", "Content", "Initial commit");
}

// ============================================================================
// FSCK Command Tests
// ============================================================================

#[test]
fn test_fsck_basic() {
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());

    add_and_commit(temp_dir.path(), "file.txt", "Content", "Initial commit");

    let start = Instant::now();
    mediagit()
        .arg("fsck")
        .current_dir(temp_dir.path())
        .assert()
        .success();

    println!("FSCK duration: {:?}", start.elapsed());
}

#[test]
fn test_fsck_full() {
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());

    for i in 1..=3 {
        add_and_commit(
            temp_dir.path(),
            &format!("file{}.txt", i),
            &format!("Content {}", i),
            &format!("Commit {}", i),
        );
    }

    mediagit()
        .arg("fsck")
        .arg("--full")
        .current_dir(temp_dir.path())
        .assert()
        .success();
}

#[test]
fn test_fsck_quick() {
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());

    add_and_commit(temp_dir.path(), "file.txt", "Content", "Initial commit");

    mediagit()
        .arg("fsck")
        .arg("--quick")
        .current_dir(temp_dir.path())
        .assert()
        .success();
}

#[test]
fn test_fsck_repair_dry_run() {
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());

    add_and_commit(temp_dir.path(), "file.txt", "Content", "Initial commit");

    mediagit()
        .arg("fsck")
        .arg("--repair")
        .arg("--dry-run")
        .current_dir(temp_dir.path())
        .assert()
        .success();
}

#[test]
fn test_fsck_verbose() {
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());

    add_and_commit(temp_dir.path(), "file.txt", "Content", "Initial commit");

    mediagit()
        .arg("fsck")
        .arg("-v")
        .current_dir(temp_dir.path())
        .assert()
        .success();
}

// ============================================================================
// Verify Command Tests
// ============================================================================

#[test]
fn test_verify_basic() {
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());

    add_and_commit(temp_dir.path(), "file.txt", "Content", "Initial commit");

    mediagit()
        .arg("verify")
        .current_dir(temp_dir.path())
        .assert()
        .success();
}

#[test]
fn test_verify_file_integrity() {
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());

    add_and_commit(temp_dir.path(), "file.txt", "Content", "Initial commit");

    mediagit()
        .arg("verify")
        .arg("--file-integrity")
        .current_dir(temp_dir.path())
        .assert()
        .success();
}

#[test]
fn test_verify_checksums() {
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());

    add_and_commit(temp_dir.path(), "file.txt", "Content", "Initial commit");

    mediagit()
        .arg("verify")
        .arg("--checksums")
        .current_dir(temp_dir.path())
        .assert()
        .success();
}

#[test]
fn test_verify_quick() {
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());

    add_and_commit(temp_dir.path(), "file.txt", "Content", "Initial commit");

    mediagit()
        .arg("verify")
        .arg("--quick")
        .current_dir(temp_dir.path())
        .assert()
        .success();
}

#[test]
fn test_verify_detailed() {
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());

    add_and_commit(temp_dir.path(), "file.txt", "Content", "Initial commit");

    mediagit()
        .arg("verify")
        .arg("--detailed")
        .current_dir(temp_dir.path())
        .assert()
        .success();
}

// ============================================================================
// Stats Command Tests
// ============================================================================

#[test]
fn test_stats_basic() {
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());

    add_and_commit(temp_dir.path(), "file.txt", "Content", "Initial commit");

    mediagit()
        .arg("stats")
        .current_dir(temp_dir.path())
        .assert()
        .success()
        .stdout(
            predicate::str::contains("Repository Statistics").or(predicate::str::contains("Stats")),
        );
}

#[test]
fn test_stats_all() {
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());

    for i in 1..=5 {
        add_and_commit(
            temp_dir.path(),
            &format!("file{}.txt", i),
            &format!("Content with longer text for commit {}", i),
            &format!("Commit {}", i),
        );
    }

    mediagit()
        .arg("stats")
        .arg("--all")
        .current_dir(temp_dir.path())
        .assert()
        .success();
}

#[test]
fn test_stats_storage() {
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());

    add_and_commit(temp_dir.path(), "file.txt", "Content", "Initial commit");

    mediagit()
        .arg("stats")
        .arg("--storage")
        .current_dir(temp_dir.path())
        .assert()
        .success();
}

#[test]
fn test_stats_compression() {
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());

    // Add some content that can be compressed
    add_and_commit(
        temp_dir.path(),
        "text.txt",
        "This is some text that should compress well. "
            .repeat(100)
            .as_str(),
        "Add text",
    );

    mediagit()
        .arg("stats")
        .arg("--compression")
        .current_dir(temp_dir.path())
        .assert()
        .success();
}

#[test]
fn test_stats_branches() {
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());

    add_and_commit(temp_dir.path(), "file.txt", "Content", "Initial commit");

    // Create some branches
    mediagit()
        .arg("branch")
        .arg("create")
        .arg("feature-1")
        .current_dir(temp_dir.path())
        .assert()
        .success();
    mediagit()
        .arg("branch")
        .arg("create")
        .arg("feature-2")
        .current_dir(temp_dir.path())
        .assert()
        .success();

    mediagit()
        .arg("stats")
        .arg("--branches")
        .current_dir(temp_dir.path())
        .assert()
        .success();
}

#[test]
fn test_stats_json() {
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());

    add_and_commit(temp_dir.path(), "file.txt", "Content", "Initial commit");

    mediagit()
        .arg("stats")
        .arg("--json")
        .current_dir(temp_dir.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("{"));
}

#[test]
fn test_stats_files() {
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());

    add_and_commit(temp_dir.path(), "file1.txt", "Content 1", "First");
    add_and_commit(temp_dir.path(), "file2.txt", "Content 2", "Second");
    add_and_commit(temp_dir.path(), "file3.txt", "Content 3", "Third");

    mediagit()
        .arg("stats")
        .arg("--files")
        .current_dir(temp_dir.path())
        .assert()
        .success();
}

// ============================================================================
// Help Tests
// ============================================================================

#[test]
fn test_gc_help() {
    mediagit()
        .arg("gc")
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("garbage").or(predicate::str::contains("gc")));
}

#[test]
fn test_fsck_help() {
    mediagit()
        .arg("fsck")
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("fsck").or(predicate::str::contains("check")));
}

#[test]
fn test_verify_help() {
    mediagit()
        .arg("verify")
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("verify"));
}

#[test]
fn test_stats_help() {
    mediagit()
        .arg("stats")
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("stats").or(predicate::str::contains("statistics")));
}

// ============================================================================
// Performance Benchmarks
// ============================================================================

#[test]
fn test_maintenance_benchmark() {
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());

    // Create a repo with some commits
    for i in 1..=10 {
        add_and_commit(
            temp_dir.path(),
            &format!("file{}.txt", i),
            &format!("Content for file {} with some text to make it larger.", i),
            &format!("Commit {}", i),
        );
    }

    // Benchmark fsck
    let start = Instant::now();
    mediagit()
        .arg("fsck")
        .arg("-q")
        .current_dir(temp_dir.path())
        .assert()
        .success();
    println!("FSCK (10 commits): {:?}", start.elapsed());

    // Benchmark verify
    let start = Instant::now();
    mediagit()
        .arg("verify")
        .arg("-q")
        .current_dir(temp_dir.path())
        .assert()
        .success();
    println!("Verify (10 commits): {:?}", start.elapsed());

    // Benchmark gc
    let start = Instant::now();
    mediagit()
        .arg("gc")
        .arg("-q")
        .current_dir(temp_dir.path())
        .assert()
        .success();
    println!("GC (10 commits): {:?}", start.elapsed());

    // Benchmark stats
    let start = Instant::now();
    mediagit()
        .arg("stats")
        .arg("-q")
        .current_dir(temp_dir.path())
        .assert()
        .success();
    println!("Stats (10 commits): {:?}", start.elapsed());
}
