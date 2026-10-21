// SPDX-License-Identifier: BUSL-1.1
// Copyright (C) 2025-2026 Aswin Krishnamoorthy

//! gc must keep the base chunks of every reachable chunk-delta.
//!
//! gc marked chunks live only when a reachable manifest named them. A
//! chunk-delta's base is named by its `.meta` sidecar, not by any manifest of
//! the reachable version, so once the version that introduced the base became
//! unreachable gc deleted it and the reachable version could no longer be
//! reconstructed.

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

fn commit(dir: &Path, path: &str, msg: &str) {
    mediagit()
        .args(["add", path])
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
fn gc_keeps_delta_bases_of_reachable_version() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    mediagit()
        .args(["init", "-q"])
        .current_dir(dir)
        .assert()
        .success();
    std::fs::write(dir.join("readme.txt"), b"root").unwrap();
    commit(dir, "readme.txt", "root");

    let mut x: u64 = 0x2545_F491_4F6C_DD1D;
    let mut w = Vec::with_capacity(6 << 20);
    for _ in 0..(6usize << 20) / 4 {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        let f = ((x >> 40) as f32 / 16_777_216.0) - 0.5;
        w.extend_from_slice(&f.to_le_bytes());
    }
    std::fs::write(dir.join("m.safetensors"), safetensors(&w)).unwrap();
    commit(dir, "m.safetensors", "v1");

    // Training step: flip the low mantissa byte of ~2% of the floats.
    for i in (0..w.len() / 4).step_by(50) {
        w[i * 4] ^= 0x5a;
    }
    let v2 = safetensors(&w);
    std::fs::write(dir.join("m.safetensors"), &v2).unwrap();
    commit(dir, "m.safetensors", "v2");
    assert!(
        !store_files(dir, "chunk-deltas").is_empty(),
        "fixture: v2 stored no chunk-deltas"
    );

    // Rewrite history so v1 is unreachable: v2 re-committed directly on root.
    mediagit()
        .args(["reset", "--soft", "HEAD~2"])
        .current_dir(dir)
        .assert()
        .success();
    commit(dir, "m.safetensors", "v2 squashed");

    mediagit()
        .args(["gc", "--yes"])
        .current_dir(dir)
        .assert()
        .success();

    std::fs::remove_file(dir.join("m.safetensors")).unwrap();
    mediagit()
        .args(["reset", "--hard", "HEAD"])
        .current_dir(dir)
        .assert()
        .success();
    let got = std::fs::read(dir.join("m.safetensors")).unwrap();
    assert!(got == v2, "v2 is not byte-exact after gc");

    let out = mediagit().arg("fsck").current_dir(dir).assert().success();
    let stdout = String::from_utf8_lossy(&out.get_output().stdout).to_string();
    assert!(stdout.contains("PERFECT"), "fsck not clean: {stdout}");
}
