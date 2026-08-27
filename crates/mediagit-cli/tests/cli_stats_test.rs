// SPDX-License-Identifier: BUSL-1.1
// Copyright (C) 2025-2026 Aswin Krishnamoorthy

//! Regression test for `mediagit stats --storage`: pack files must be counted
//! after a real `gc --repack`. Two independent pre-M1 bugs both had to be
//! fixed for this to work:
//!   1. `gc.rs`'s repack path built `ObjectDatabase::new` (plain zlib) to
//!      read loose objects for repacking, but `add`/`commit` write via
//!      `ObjectDatabase::with_smart_compression` (zstd/brotli) — repack
//!      silently packed 0 objects.
//!   2. `stats.rs`'s `compute_storage_stats` walked `.mediagit/{packs,objects}`
//!      directly, but pack files actually live one level deeper
//!      (`.mediagit/objects/packs/` — `base_path` config points at
//!      `.mediagit/objects`), so `pack_files` was always 0 even for a real
//!      pack. Fixed by routing through `StorageBackend::list_objects`
//!      instead of raw filesystem walks (layout v2, M1).

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

#[test]
fn test_stats_reports_pack_count_after_repack() {
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());

    for i in 1..=5 {
        add_and_commit(
            temp_dir.path(),
            &format!("file{}.txt", i),
            &format!("Content {}", i),
            &format!("Commit {}", i),
        );
    }

    let repack_output = mediagit()
        .args(["gc", "--repack", "-y", "--verbose"])
        .current_dir(temp_dir.path())
        .output()
        .unwrap();
    assert!(repack_output.status.success());
    let repack_stdout = String::from_utf8_lossy(&repack_output.stdout);
    assert!(
        repack_stdout.contains("Packed") && !repack_stdout.contains("Packed 0 objects"),
        "expected gc --repack to pack a nonzero number of real objects, got:\n{repack_stdout}"
    );

    let output = mediagit()
        .args(["stats", "--storage", "--json"])
        .current_dir(temp_dir.path())
        .output()
        .unwrap();
    assert!(output.status.success());

    let stdout = String::from_utf8_lossy(&output.stdout);
    let json: serde_json::Value = serde_json::from_str(&stdout)
        .unwrap_or_else(|e| panic!("stats --json did not produce valid JSON: {e}\n{stdout}"));

    let pack_files = json["storage"]["pack_files"]
        .as_u64()
        .expect("storage.pack_files missing from stats --json output");

    assert!(
        pack_files > 0,
        "expected pack_files > 0 after gc --repack, got {pack_files} (json: {json})"
    );
}
