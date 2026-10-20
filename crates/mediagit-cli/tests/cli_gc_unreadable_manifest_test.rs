// SPDX-License-Identifier: BUSL-1.1
// Copyright (C) 2025-2026 Aswin Krishnamoorthy

//! gc must refuse to run, not delete, when a reachable chunk manifest cannot
//! be read.
//!
//! The orphan-chunk scan collects every chunk id named by a reachable
//! manifest and deletes the rest. A manifest that failed to parse was logged
//! at debug and skipped, so its chunks looked unreachable and were deleted:
//! one corrupt or too-new manifest turned into permanent loss of the file.

use assert_cmd::Command;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

#[allow(deprecated)]
fn mediagit() -> Command {
    let mut c = Command::cargo_bin("mediagit").unwrap();
    c.env("MEDIAGIT_AUTHOR_NAME", "Test User")
        .env("MEDIAGIT_AUTHOR_EMAIL", "test@example.com");
    c
}

fn files_under(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return out;
    };
    for e in entries.flatten() {
        let p = e.path();
        if p.is_dir() {
            out.extend(files_under(&p));
        } else {
            out.push(p);
        }
    }
    out
}

/// Files under `objects/` whose path has a `kind` component (`manifests` or
/// `chunks`). The store nests these under a per-repo namespace directory.
fn store_files(objects: &Path, kind: &str) -> Vec<PathBuf> {
    files_under(objects)
        .into_iter()
        .filter(|p| p.components().any(|c| c.as_os_str() == kind))
        .collect()
}

#[test]
fn gc_aborts_without_deleting_when_reachable_manifest_is_unreadable() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    mediagit()
        .args(["init", "-q"])
        .current_dir(dir)
        .assert()
        .success();

    // Above the 5 MiB streaming threshold, so it is chunked. Pseudo-random
    // (xorshift) bytes: constant bytes give CDC no cut points.
    let mut x: u64 = 0x9E37_79B9_7F4A_7C15;
    let data: Vec<u8> = (0..6 * 1024 * 1024)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            (x >> 24) as u8
        })
        .collect();
    std::fs::write(dir.join("big.bin"), &data).unwrap();
    mediagit()
        .args(["add", "big.bin"])
        .current_dir(dir)
        .assert()
        .success();
    mediagit()
        .args(["commit", "-m", "big"])
        .current_dir(dir)
        .assert()
        .success();

    let objects = dir.join(".mediagit/objects");
    let manifests = store_files(&objects, "manifests");
    let chunks_before = store_files(&objects, "chunks").len();
    assert_eq!(manifests.len(), 1, "fixture: expected one manifest");
    assert!(chunks_before > 0, "fixture: file was not chunked");

    std::fs::write(&manifests[0], b"\xFF\xFFnot a manifest").unwrap();

    let out = mediagit()
        .args(["gc", "--yes"])
        .env("MEDIAGIT_GC_REFLOG_HORIZON_DAYS", "0")
        .env("MEDIAGIT_GC_GRACE_SECS", "0")
        .current_dir(dir)
        .assert()
        .failure()
        .get_output()
        .clone();
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("manifest"),
        "error should name the manifest: {stderr}"
    );
    assert_eq!(
        store_files(&objects, "chunks").len(),
        chunks_before,
        "gc deleted chunks of an unreadable manifest"
    );
}
