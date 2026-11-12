// SPDX-License-Identifier: BUSL-1.1
// Copyright (C) 2025-2026 Aswin Krishnamoorthy

//! `verify` must notice objects that are gone from the object store (BUG-002).
//!
//! It used to build its checker with no refdb and with connectivity off, so a
//! committed repository whose blob, manifest or chunk had been deleted still
//! printed "All verifications passed".

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

/// Init a repo and commit `name`; returns the object files that `add` created
/// (the blob, or the manifest and chunks).
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

/// Run `verify` with `args` and demand a failure that mentions a missing object.
fn assert_verify_fails(repo: &Path, args: &[&str]) {
    let out = mediagit()
        .arg("verify")
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
        "verify {args:?} failed without naming a missing object: {text}"
    );
}

#[test]
fn verify_passes_on_an_intact_repository() {
    let tmp = TempDir::new().unwrap();
    commit_file(tmp.path(), "a.txt", b"hello world");
    run(tmp.path(), &["verify"]);
    run(tmp.path(), &["verify", "--checksums"]);
    run(tmp.path(), &["verify", "--quick"]);
}

#[test]
fn verify_fails_when_a_referenced_blob_is_missing() {
    let tmp = TempDir::new().unwrap();
    let added = commit_file(tmp.path(), "a.txt", b"hello world");
    assert_eq!(added.len(), 1, "fixture: add should write exactly the blob");
    std::fs::remove_file(&added[0]).unwrap();

    assert_verify_fails(tmp.path(), &[]);
    assert_verify_fails(tmp.path(), &["--checksums"]);
    assert_verify_fails(tmp.path(), &["--quick"]);
}

/// Two commits; the second adds `b.txt`, whose blob is then deleted.
fn two_commits_second_blob_missing(repo: &Path) {
    commit_file(repo, "a.txt", b"first");
    std::fs::write(repo.join("b.txt"), b"second").unwrap();
    let before = object_files(repo);
    run(repo, &["add", "b.txt"]);
    let added: Vec<PathBuf> = object_files(repo).difference(&before).cloned().collect();
    run(repo, &["commit", "-m", "second"]);
    assert_eq!(added.len(), 1, "fixture: add should write exactly the blob");
    std::fs::remove_file(&added[0]).unwrap();
}

#[test]
fn verify_commit_form_fails_when_a_blob_is_missing() {
    let tmp = TempDir::new().unwrap();
    two_commits_second_blob_missing(tmp.path());
    assert_verify_fails(tmp.path(), &["HEAD"]);
}

#[test]
fn verify_range_form_fails_when_a_blob_is_missing() {
    let tmp = TempDir::new().unwrap();
    two_commits_second_blob_missing(tmp.path());
    assert_verify_fails(tmp.path(), &["--start", "HEAD~1", "--end", "HEAD"]);
}

#[test]
fn verify_commit_and_range_forms_pass_on_an_intact_repository() {
    let tmp = TempDir::new().unwrap();
    commit_file(tmp.path(), "a.txt", b"first");
    run(tmp.path(), &["verify", "HEAD"]);
    run(tmp.path(), &["verify", "--start", "HEAD", "--end", "HEAD"]);
}

#[test]
fn verify_fails_when_a_chunk_of_a_chunked_file_is_missing() {
    let tmp = TempDir::new().unwrap();
    // Above the 5 MiB streaming threshold, so the file is chunked.
    let added = commit_file(tmp.path(), "big.bin", &random_bytes(6 * 1024 * 1024));
    let chunk = added
        .iter()
        .find(|p| p.components().any(|c| c.as_os_str() == "chunks"))
        .expect("fixture: file was not chunked");
    std::fs::remove_file(chunk).unwrap();

    assert_verify_fails(tmp.path(), &[]);
    assert_verify_fails(tmp.path(), &["--checksums"]);
    assert_verify_fails(tmp.path(), &["--quick"]);
}
