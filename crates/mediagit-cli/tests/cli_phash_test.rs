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

//! Integration tests for P4a: pHash-guided delta-base nomination.
//!
//! Uses the `dev-tests/dedup-pairs/` fixtures — jpg_v1.jpg/jpg_v2.jpg are a
//! quality-90/quality-85 re-export of the same photo: perceptually
//! near-identical, byte-dissimilar (exactly the case plain byte-sampling
//! similarity detection misses, and the case pHash exists for).

use assert_cmd::Command;
use mediagit_versioning::Oid;
use std::fs;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

#[cfg(windows)]
const DEDUP_PAIRS_DIR: &str = "D:\\own\\saas\\mediagit-core\\dev-tests\\dedup-pairs";
#[cfg(not(windows))]
const DEDUP_PAIRS_DIR: &str = "/mnt/d/own/saas/mediagit-core/dev-tests/dedup-pairs";

/// Mirrors the private `Entry` layout in `mediagit_cli::phash_index` — same
/// field order/types, so postcard decodes it identically without needing to
/// expose the type. Used only to verify the index round-tripped real data.
#[derive(serde::Deserialize)]
struct IndexEntry {
    #[allow(dead_code)]
    hash: u64,
    oid: [u8; 32],
}

#[allow(deprecated)]
fn mediagit() -> Command {
    Command::cargo_bin("mediagit").unwrap()
}

fn init_repo(dir: &Path) {
    mediagit()
        .arg("init")
        .arg("-q")
        .current_dir(dir)
        .assert()
        .success();
}

fn fixture_paths() -> Option<(PathBuf, PathBuf)> {
    let v1 = Path::new(DEDUP_PAIRS_DIR).join("jpg_v1.jpg");
    let v2 = Path::new(DEDUP_PAIRS_DIR).join("jpg_v2.jpg");
    if v1.exists() && v2.exists() {
        Some((v1, v2))
    } else {
        None
    }
}

/// Where the filesystem storage backend actually lands the `deltas/<hex>.meta`
/// key: it sanitizes the key to `deltas__<hex>.meta` and shards it under the
/// first four characters of the sanitized key (always `de/lt` for deltas),
/// beneath the backend root `.mediagit/objects/objects/`.
fn delta_meta_path(repo_root: &Path, oid: &Oid) -> PathBuf {
    repo_root
        .join(".mediagit")
        .join("objects")
        .join("objects")
        .join("de")
        .join("lt")
        .join(format!("deltas__{}.meta", oid.to_hex()))
}

fn read_index_entries(repo_root: &Path) -> Vec<IndexEntry> {
    let idx_path = repo_root.join(".mediagit").join("phash.idx");
    let bytes =
        fs::read(&idx_path).unwrap_or_else(|_| panic!("phash.idx not found at {idx_path:?}"));
    assert_eq!(bytes.first(), Some(&1u8), "format version byte must be 1");
    postcard::from_bytes(&bytes[1..]).expect("phash.idx should decode as Vec<IndexEntry>")
}

#[test]
fn test_phash_nominates_and_records_reexported_jpeg() {
    let Some((v1_path, v2_path)) = fixture_paths() else {
        println!("SKIP: dev-tests/dedup-pairs fixtures not found");
        return;
    };

    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());

    let v1_data = fs::read(&v1_path).unwrap();
    let v2_data = fs::read(&v2_path).unwrap();
    let v1_oid = Oid::hash(&v1_data);
    let v2_oid = Oid::hash(&v2_data);

    let dest = temp_dir.path().join("photo.jpg");

    // Commit v1 first, so it exists in the ODB as a candidate base.
    fs::copy(&v1_path, &dest).unwrap();
    mediagit()
        .arg("add")
        .arg("photo.jpg")
        .current_dir(temp_dir.path())
        .assert()
        .success();
    mediagit()
        .arg("commit")
        .arg("-m")
        .arg("v1")
        .current_dir(temp_dir.path())
        .assert()
        .success();

    // Unrelated noise files alongside, so the add batch isn't just the one
    // image (mirrors a real mixed-content commit).
    fs::write(temp_dir.path().join("noise1.bin"), vec![7u8; 4096]).unwrap();
    fs::write(temp_dir.path().join("noise2.bin"), vec![9u8; 8192]).unwrap();

    // Overwrite with the re-exported v2 and add everything.
    fs::copy(&v2_path, &dest).unwrap();
    mediagit()
        .arg("add")
        .arg(".")
        .current_dir(temp_dir.path())
        .assert()
        .success();

    // 1) The index must exist and have recorded both jpg versions —
    // this is the mechanism engaging, independent of the 80% gate outcome.
    let entries = read_index_entries(temp_dir.path());
    let recorded_oids: Vec<[u8; 32]> = entries.iter().map(|e| e.oid).collect();
    assert!(
        recorded_oids.contains(v1_oid.as_bytes()),
        "phash index should have recorded jpg_v1"
    );
    assert!(
        recorded_oids.contains(v2_oid.as_bytes()),
        "phash index should have recorded jpg_v2"
    );

    // 2) The 80% delta gate is the real decider, and for a RE-ENCODED pair it
    // must REJECT: quality-90 -> quality-85 re-quantizes every coefficient, so
    // the two files share almost no byte-level content (measured delta ≈ 95%
    // of original). pHash correctly nominates, the gate correctly declines —
    // pin that, so a future gate change that starts storing near-full-size
    // "deltas" for re-encodes gets caught here.
    let meta_path = delta_meta_path(temp_dir.path(), &v2_oid);
    assert!(
        !meta_path.exists(),
        "re-encoded JPEG pair shares too few bytes for delta; the 80% gate \
         must reject the pHash-nominated attempt"
    );
    let _ = v1_oid;
}

/// The case pHash actually unlocks: a byte-similar image edit (here a COM
/// metadata segment injected into the same JPEG encode — compressed image
/// data untouched). `should_use_delta()` categorically skips jpg, so before
/// P4a this file could never delta at all; with pHash nominating the prior
/// version, the delta passes the 80% gate easily and gets stored.
#[test]
fn test_phash_delta_accepted_for_metadata_edit() {
    let v1_path = Path::new(DEDUP_PAIRS_DIR).join("jpg_v1.jpg");
    let v2_path = Path::new(DEDUP_PAIRS_DIR).join("jpg_meta_v2.jpg");
    if !v1_path.exists() || !v2_path.exists() {
        println!("SKIP: dev-tests/dedup-pairs metadata-edit fixtures not found");
        return;
    }

    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());

    let v1_data = fs::read(&v1_path).unwrap();
    let v2_data = fs::read(&v2_path).unwrap();
    let v1_oid = Oid::hash(&v1_data);
    let v2_oid = Oid::hash(&v2_data);

    let dest = temp_dir.path().join("photo.jpg");
    fs::copy(&v1_path, &dest).unwrap();
    mediagit()
        .arg("add")
        .arg("photo.jpg")
        .current_dir(temp_dir.path())
        .assert()
        .success();
    mediagit()
        .arg("commit")
        .arg("-m")
        .arg("v1")
        .current_dir(temp_dir.path())
        .assert()
        .success();

    fs::copy(&v2_path, &dest).unwrap();
    mediagit()
        .arg("add")
        .arg("photo.jpg")
        .current_dir(temp_dir.path())
        .assert()
        .success();

    let meta_path = delta_meta_path(temp_dir.path(), &v2_oid);
    assert!(
        meta_path.exists(),
        "metadata-edit JPEG is byte-similar; the pHash-nominated delta must \
         pass the 80% gate and be stored"
    );
    let meta = fs::read_to_string(&meta_path).unwrap();
    assert!(
        meta.contains(&v1_oid.to_hex()),
        "delta metadata should reference jpg_v1 as base, got: {meta}"
    );
}

#[test]
fn test_phash_kill_switch_restores_prior_behavior() {
    let Some((v1_path, v2_path)) = fixture_paths() else {
        println!("SKIP: dev-tests/dedup-pairs fixtures not found");
        return;
    };

    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());

    let v2_data = fs::read(&v2_path).unwrap();
    let v2_oid = Oid::hash(&v2_data);

    let dest = temp_dir.path().join("photo.jpg");
    fs::copy(&v1_path, &dest).unwrap();
    mediagit()
        .arg("add")
        .env("MEDIAGIT_PHASH", "0")
        .arg("photo.jpg")
        .current_dir(temp_dir.path())
        .assert()
        .success();
    mediagit()
        .arg("commit")
        .arg("-m")
        .arg("v1")
        .current_dir(temp_dir.path())
        .assert()
        .success();

    fs::copy(&v2_path, &dest).unwrap();
    mediagit()
        .arg("add")
        .env("MEDIAGIT_PHASH", "0")
        .arg("photo.jpg")
        .current_dir(temp_dir.path())
        .assert()
        .success();

    // With the kill switch set: no index file at all, and no delta attempt
    // for jpg_v2 (should_use_delta() already forbids delta for jpg/png
    // unconditionally, so this must be exactly the pre-P4a behavior).
    let idx_path = temp_dir.path().join(".mediagit").join("phash.idx");
    assert!(
        !idx_path.exists(),
        "MEDIAGIT_PHASH=0 must not create the index at all"
    );

    let meta_path = delta_meta_path(temp_dir.path(), &v2_oid);
    assert!(
        !meta_path.exists(),
        "MEDIAGIT_PHASH=0 must not attempt a delta for jpg (matches pre-P4a behavior)"
    );
}
