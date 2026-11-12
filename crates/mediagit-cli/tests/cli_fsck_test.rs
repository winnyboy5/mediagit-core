// SPDX-License-Identifier: BUSL-1.1
// Copyright (C) 2025-2026 Aswin Krishnamoorthy

//! `fsck` must report objects that are gone from the object store (BUG-002).
//! Pins the CLI wiring (refdb + repo root + options), not just the library.

use assert_cmd::Command;
use std::collections::BTreeSet;
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

fn object_files(repo: &Path) -> BTreeSet<PathBuf> {
    files_under(&repo.join(".mediagit/objects"))
        .into_iter()
        .collect()
}

fn run(repo: &Path, args: &[&str]) {
    mediagit().args(args).current_dir(repo).assert().success();
}

/// Init a repo and commit `name`; returns the object files that `add` created.
fn commit_file(repo: &Path, name: &str, data: &[u8]) -> Vec<PathBuf> {
    run(repo, &["init", "-q"]);
    std::fs::write(repo.join(name), data).unwrap();
    let before = object_files(repo);
    run(repo, &["add", name]);
    let added: Vec<PathBuf> = object_files(repo).difference(&before).cloned().collect();
    run(repo, &["commit", "-m", "c"]);
    added
}

/// Pseudo-random (xorshift) bytes: constant bytes give CDC no cut points.
fn random_bytes(n: usize) -> Vec<u8> {
    let mut x: u64 = 0x9E37_79B9_7F4A_7C15;
    (0..n)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            (x >> 24) as u8
        })
        .collect()
}

fn assert_fsck_fails(repo: &Path) {
    assert_fsck_args_fail(repo, &[]);
}

fn assert_fsck_args_fail(repo: &Path, args: &[&str]) {
    let out = mediagit()
        .arg("fsck")
        .args(args)
        .current_dir(repo)
        .assert()
        .failure()
        .get_output()
        .clone();
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
    .to_lowercase();
    assert!(
        text.contains("missing"),
        "fsck failed without naming a missing object: {text}"
    );
}

#[test]
fn fsck_fails_when_a_referenced_blob_is_missing() {
    let tmp = TempDir::new().unwrap();
    let added = commit_file(tmp.path(), "a.txt", b"hello world");
    assert_eq!(added.len(), 1, "fixture: add should write exactly the blob");
    std::fs::remove_file(&added[0]).unwrap();
    assert_fsck_fails(tmp.path());
    assert_fsck_args_fail(tmp.path(), &["--quick"]);
}

/// Flip one byte in the middle of a stored object file.
fn flip_byte(path: &Path) {
    let mut bytes = std::fs::read(path).unwrap();
    let mid = bytes.len() / 2;
    bytes[mid] ^= 0xFF;
    std::fs::write(path, bytes).unwrap();
}

fn fsck_text(repo: &Path, args: &[&str]) -> (bool, String) {
    let out = mediagit()
        .arg("fsck")
        .args(args)
        .current_dir(repo)
        .assert()
        .get_output()
        .clone();
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    (out.status.success(), text)
}

/// Repair must bring the object back from the intact working file; then the
/// saved version has to be readable again (BUG-046: it used to be deleted).
fn assert_repair_restores(name: &str, data: &[u8], victim: impl Fn(&[PathBuf]) -> PathBuf) {
    let tmp = TempDir::new().unwrap();
    let repo = tmp.path();
    let added = commit_file(repo, name, data);
    flip_byte(&victim(&added));
    assert!(!fsck_text(repo, &["--full"]).0, "fixture: damage unseen");

    let (ok, text) = fsck_text(repo, &["--repair"]);
    assert!(
        ok,
        "repair from an intact working file must succeed: {text}"
    );
    let (ok, text) = fsck_text(repo, &["--full"]);
    assert!(ok, "repository must be clean after repair: {text}");

    std::fs::remove_file(repo.join(name)).unwrap();
    run(repo, &["reset", "--hard", "HEAD"]);
    assert_eq!(
        std::fs::read(repo.join(name)).unwrap(),
        data,
        "the restored version must equal the original bytes"
    );
}

#[test]
fn fsck_repair_restores_damaged_blob_from_working_file() {
    assert_repair_restores("photo.bin", &random_bytes(64 * 1024), |added| {
        assert_eq!(added.len(), 1, "fixture: add should write exactly the blob");
        added[0].clone()
    });
}

#[test]
fn fsck_repair_restores_damaged_chunk_from_working_file() {
    assert_repair_restores("big.bin", &random_bytes(6 * 1024 * 1024), |added| {
        added
            .iter()
            .find(|p| p.components().any(|c| c.as_os_str() == "chunks"))
            .expect("fixture: file was not chunked")
            .clone()
    });
}

#[test]
fn fsck_repair_without_a_source_quarantines_and_stays_red() {
    let tmp = TempDir::new().unwrap();
    let repo = tmp.path();
    let data = random_bytes(64 * 1024);
    let added = commit_file(repo, "photo.bin", &data);
    assert_eq!(added.len(), 1, "fixture: add should write exactly the blob");
    flip_byte(&added[0]);
    let damaged = std::fs::read(&added[0]).unwrap();
    std::fs::remove_file(repo.join("photo.bin")).unwrap();

    let (ok, text) = fsck_text(repo, &["--repair"]);
    assert!(
        !ok,
        "nothing could be restored, exit must be non-zero: {text}"
    );
    assert!(
        text.contains("photo.bin"),
        "output must name the file that can no longer be restored: {text}"
    );
    let quarantined = files_under(&repo.join(".mediagit/quarantine"));
    assert_eq!(quarantined.len(), 1, "damaged bytes must be kept: {text}");
    assert_eq!(
        std::fs::read(&quarantined[0]).unwrap(),
        damaged,
        "quarantine must hold the exact damaged bytes"
    );
    let (ok, text) = fsck_text(repo, &[]);
    assert!(!ok, "fsck must stay red after a failed repair: {text}");
}

#[test]
fn fsck_fails_when_a_chunk_of_a_chunked_file_is_missing() {
    let tmp = TempDir::new().unwrap();
    let added = commit_file(tmp.path(), "big.bin", &random_bytes(6 * 1024 * 1024));
    let chunk = added
        .iter()
        .find(|p| p.components().any(|c| c.as_os_str() == "chunks"))
        .expect("fixture: file was not chunked");
    std::fs::remove_file(chunk).unwrap();
    assert_fsck_fails(tmp.path());
    assert_fsck_args_fail(tmp.path(), &["--quick"]);
    assert_fsck_args_fail(tmp.path(), &["--max-objects", "1"]);
}
