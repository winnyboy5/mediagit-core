// SPDX-License-Identifier: BUSL-1.1
// Copyright (C) 2025-2026 Aswin Krishnamoorthy

//! gc / fsck / repack / fsck --repair on a repo holding XOR-planes ("XP")
//! chunk-deltas (`MEDIAGIT_TENSOR_DELTA=1`, safetensors). XP deltas live in
//! `chunk-deltas/` like ZD ones; none of these maintenance paths may delete
//! or corrupt them, and every committed version must stay byte-exact.

use assert_cmd::Command;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

#[allow(deprecated)]
fn mediagit() -> Command {
    let mut c = Command::cargo_bin("mediagit").unwrap();
    c.env("MEDIAGIT_AUTHOR_NAME", "Test User")
        .env("MEDIAGIT_AUTHOR_EMAIL", "test@example.com")
        .env("MEDIAGIT_TENSOR_DELTA", "1")
        .env("MEDIAGIT_GC_REFLOG_HORIZON_DAYS", "0")
        .env("MEDIAGIT_GC_GRACE_SECS", "0");
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

fn store_files(dir: &Path, kind: &str) -> Vec<PathBuf> {
    files_under(&dir.join(".mediagit/objects"))
        .into_iter()
        .filter(|p| p.components().any(|c| c.as_os_str() == kind))
        .collect()
}

/// 6 MiB of F32 weights in a minimal single-tensor safetensors file.
fn safetensors(weights: &[u8]) -> Vec<u8> {
    let header = format!(
        r#"{{"w":{{"dtype":"F32","shape":[{}],"data_offsets":[0,{}]}}}}"#,
        weights.len() / 4,
        weights.len()
    );
    let mut out = (header.len() as u64).to_le_bytes().to_vec();
    out.extend_from_slice(header.as_bytes());
    out.extend_from_slice(weights);
    out
}

/// Repo with three committed versions of `m.safetensors`; returns the bytes
/// of each (oldest first).
fn xp_repo() -> (TempDir, Vec<Vec<u8>>) {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    mediagit()
        .args(["init", "-q"])
        .current_dir(dir)
        .assert()
        .success();

    let mut x: u64 = 0x2545_F491_4F6C_DD1D;
    let mut w = Vec::with_capacity(6 << 20);
    for _ in 0..(6usize << 20) / 4 {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        let f = ((x >> 40) as f32 / 16_777_216.0) - 0.5;
        w.extend_from_slice(&f.to_le_bytes());
    }

    let mut versions = Vec::new();
    for v in 0..3usize {
        // Training step: flip the low mantissa byte of ~2% of the floats.
        for i in (v % 50..w.len() / 4).step_by(50) {
            w[i * 4] ^= 0x5a;
        }
        let file = safetensors(&w);
        std::fs::write(dir.join("m.safetensors"), &file).unwrap();
        mediagit()
            .args(["add", "m.safetensors"])
            .current_dir(dir)
            .assert()
            .success();
        mediagit()
            .args(["commit", "-m", &format!("v{v}")])
            .current_dir(dir)
            .assert()
            .success();
        versions.push(file);
    }

    assert!(
        !store_files(dir, "chunk-deltas").is_empty(),
        "fixture: no chunk-deltas were written (XP path not exercised)"
    );
    (tmp, versions)
}

/// Check out every version (newest first, walking HEAD back) and compare bytes.
fn assert_all_versions_exact(dir: &Path, versions: &[Vec<u8>]) {
    for (i, expected) in versions.iter().enumerate().rev() {
        let _ = std::fs::remove_file(dir.join("m.safetensors"));
        let step = if i + 1 == versions.len() {
            "HEAD"
        } else {
            "HEAD~1"
        };
        mediagit()
            .args(["reset", "--hard", step])
            .current_dir(dir)
            .assert()
            .success();
        let got = std::fs::read(dir.join("m.safetensors")).unwrap();
        assert!(got == *expected, "version {i} is not byte-exact");
    }
}

fn assert_fsck_perfect(dir: &Path) {
    let out = mediagit().arg("fsck").current_dir(dir).assert().success();
    let stdout = String::from_utf8_lossy(&out.get_output().stdout).to_string();
    assert!(stdout.contains("PERFECT"), "fsck not clean: {stdout}");
}

#[test]
fn gc_keeps_every_xp_chunk_and_version() {
    let (tmp, versions) = xp_repo();
    let dir = tmp.path();
    let chunks = store_files(dir, "chunks").len();
    let deltas = store_files(dir, "chunk-deltas").len();

    mediagit()
        .args(["gc", "--yes"])
        .current_dir(dir)
        .assert()
        .success();

    assert_eq!(
        store_files(dir, "chunks").len(),
        chunks,
        "gc deleted chunks"
    );
    assert_eq!(
        store_files(dir, "chunk-deltas").len(),
        deltas,
        "gc deleted chunk-deltas"
    );
    assert_fsck_perfect(dir);
    assert_all_versions_exact(dir, &versions);
}

#[test]
fn repack_then_checkout_is_byte_exact() {
    let (tmp, versions) = xp_repo();
    let dir = tmp.path();

    mediagit()
        .args(["gc", "--repack", "--yes"])
        .current_dir(dir)
        .assert()
        .success();

    assert_fsck_perfect(dir);
    assert_all_versions_exact(dir, &versions);
}

#[test]
fn fsck_repair_deletes_nothing_on_xp_repo() {
    let (tmp, versions) = xp_repo();
    let dir = tmp.path();
    let chunks = store_files(dir, "chunks").len();
    let deltas = store_files(dir, "chunk-deltas").len();

    mediagit()
        .args(["fsck", "--repair"])
        .current_dir(dir)
        .assert()
        .success();

    assert_eq!(
        store_files(dir, "chunks").len(),
        chunks,
        "repair deleted chunks"
    );
    assert_eq!(
        store_files(dir, "chunk-deltas").len(),
        deltas,
        "repair deleted chunk-deltas"
    );
    assert_all_versions_exact(dir, &versions);
}
