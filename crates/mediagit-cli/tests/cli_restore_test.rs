// SPDX-License-Identifier: BUSL-1.1
// Copyright (C) 2025-2026 Aswin Krishnamoorthy

//! `mediagit restore`: per-file discard. Nothing is thrown away without the
//! person's confirmation (prompt or `--yes`), and new files are only deleted
//! with `--delete-new` as well.

use assert_cmd::Command;
use serde_json::Value;
use std::path::Path;
use tempfile::TempDir;

#[allow(deprecated)]
fn mediagit() -> Command {
    let mut c = Command::cargo_bin("mediagit").unwrap();
    c.env("MEDIAGIT_AUTHOR_NAME", "Test User")
        .env("MEDIAGIT_AUTHOR_EMAIL", "test@example.com");
    c
}

fn run(repo: &Path, args: &[&str]) {
    mediagit().args(args).current_dir(repo).assert().success();
}

fn read(repo: &Path, p: &str) -> Vec<u8> {
    std::fs::read(repo.join(p)).unwrap()
}

fn porcelain(repo: &Path) -> String {
    let out = mediagit()
        .args(["status", "--porcelain"])
        .current_dir(repo)
        .output()
        .unwrap();
    String::from_utf8_lossy(&out.stdout).trim().to_owned()
}

/// `a.txt` = "v1" then "v2" (HEAD); `keep.txt` saved too.
fn repo() -> TempDir {
    let dir = TempDir::new().unwrap();
    let r = dir.path();
    run(r, &["init", "-q"]);
    std::fs::write(r.join("a.txt"), "v1").unwrap();
    std::fs::write(r.join("keep.txt"), "keep").unwrap();
    run(r, &["add", "a.txt", "keep.txt"]);
    run(r, &["commit", "-m", "one"]);
    std::fs::write(r.join("a.txt"), "v2").unwrap();
    run(r, &["add", "a.txt"]);
    run(r, &["commit", "-m", "two"]);
    dir
}

/// `--machine restore <args>`: (exit code, envelope). Asserts one stdout line.
fn machine(repo: &Path, args: &[&str]) -> (i32, Value) {
    let out = mediagit()
        .arg("--machine")
        .arg("restore")
        .args(args)
        .current_dir(repo)
        .output()
        .unwrap();
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert_eq!(stdout.lines().count(), 1, "{stdout}");
    (
        out.status.code().unwrap(),
        serde_json::from_str(&stdout).unwrap(),
    )
}

#[test]
fn a_changed_file_goes_back_to_the_saved_version() {
    let dir = repo();
    let r = dir.path();
    std::fs::write(r.join("a.txt"), "my edit").unwrap();
    let (code, v) = machine(r, &["--yes", "--", "a.txt"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["restored"][0], "a.txt");
    assert_eq!(read(r, "a.txt"), b"v2");
    assert_eq!(porcelain(r), "");
}

#[test]
fn a_staged_change_is_restored_and_unstaged() {
    let dir = repo();
    let r = dir.path();
    std::fs::write(r.join("a.txt"), "staged edit").unwrap();
    run(r, &["add", "a.txt"]);
    let (code, v) = machine(r, &["--yes", "--", "a.txt"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["unstaged"][0], "a.txt");
    assert_eq!(read(r, "a.txt"), b"v2");
    assert_eq!(porcelain(r), "");
}

#[test]
fn a_deleted_file_comes_back() {
    let dir = repo();
    let r = dir.path();
    std::fs::remove_file(r.join("a.txt")).unwrap();
    let (code, _) = machine(r, &["--yes", "--", "a.txt"]);
    assert_eq!(code, 0);
    assert_eq!(read(r, "a.txt"), b"v2");
}

#[test]
fn an_older_version_is_taken_with_source() {
    let dir = repo();
    let r = dir.path();
    let (code, _) = machine(r, &["--yes", "--source", "HEAD~1", "--", "a.txt"]);
    assert_eq!(code, 0);
    assert_eq!(read(r, "a.txt"), b"v1");
}

// ---- confirmation gates: nothing changes without the person's yes ----

#[test]
fn machine_without_yes_asks_and_changes_nothing() {
    let dir = repo();
    let r = dir.path();
    std::fs::write(r.join("a.txt"), "my edit").unwrap();
    run(r, &["add", "a.txt"]);
    let index_before = read(r, ".mediagit/index");
    let (code, v) = machine(r, &["--", "a.txt"]);
    assert_ne!(code, 0);
    assert_eq!(v["error"]["code"], "needs_input", "{v}");
    assert_eq!(read(r, "a.txt"), b"my edit");
    assert_eq!(read(r, ".mediagit/index"), index_before);
}

#[test]
fn a_script_without_yes_is_refused_and_changes_nothing() {
    let dir = repo();
    let r = dir.path();
    std::fs::write(r.join("a.txt"), "my edit").unwrap();
    // assert_cmd gives the child a pipe, not a terminal.
    mediagit()
        .args(["restore", "--", "a.txt"])
        .current_dir(r)
        .write_stdin("y\n")
        .assert()
        .failure();
    assert_eq!(read(r, "a.txt"), b"my edit");
}

// ---- new files ----

#[test]
fn a_new_file_is_refused_without_delete_new_even_with_yes() {
    let dir = repo();
    let r = dir.path();
    std::fs::write(r.join("new.psd"), "hours of work").unwrap();
    let (code, v) = machine(r, &["--yes", "--", "new.psd"]);
    assert_ne!(code, 0);
    assert_eq!(v["error"]["code"], "would_delete", "{v}");
    assert_eq!(read(r, "new.psd"), b"hours of work");
}

#[test]
fn delete_new_with_yes_deletes_the_file_and_its_empty_folder() {
    let dir = repo();
    let r = dir.path();
    std::fs::create_dir_all(r.join("render/tmp")).unwrap();
    std::fs::write(r.join("render/tmp/x.exr"), "scratch").unwrap();
    let (code, v) = machine(r, &["--yes", "--delete-new", "--", "render/tmp/x.exr"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["deleted"][0], "render/tmp/x.exr");
    assert!(!r.join("render").exists(), "empty folders left behind");
    assert!(r.join("a.txt").exists());
}

#[test]
fn a_staged_new_file_is_deleted_and_unstaged() {
    let dir = repo();
    let r = dir.path();
    std::fs::write(r.join("new.txt"), "draft").unwrap();
    run(r, &["add", "new.txt"]);
    let (code, v) = machine(r, &["--yes", "--delete-new", "--", "new.txt"]);
    assert_eq!(code, 0, "{v}");
    assert!(!r.join("new.txt").exists());
    assert_eq!(porcelain(r), "");
}

// ---- refused paths ----

#[test]
fn folders_and_paths_outside_the_repo_are_refused() {
    let dir = repo();
    let r = dir.path();
    std::fs::create_dir_all(r.join("art")).unwrap();
    std::fs::write(r.join("art/x.png"), "x").unwrap();
    for p in [
        "art",
        "../outside.txt",
        ".mediagit/index",
        ".MediaGit/index",
        "missing.txt",
    ] {
        let (code, v) = machine(r, &["--yes", "--delete-new", "--", p]);
        assert_ne!(code, 0, "{p}");
        assert_eq!(v["error"]["code"], "invalid_args", "{p}: {v}");
    }
    assert!(r.join("art/x.png").exists());
    assert!(r.join(".mediagit/index").exists());
}

// ---- big files and partial failure ----

#[test]
fn a_large_chunked_file_round_trips_byte_for_byte() {
    let dir = TempDir::new().unwrap();
    let r = dir.path();
    run(r, &["init", "-q"]);
    let mut x: u64 = 0x9E37_79B9_7F4A_7C15;
    let big: Vec<u8> = (0..12 * 1024 * 1024)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            x as u8
        })
        .collect();
    std::fs::write(r.join("big.bin"), &big).unwrap();
    run(r, &["add", "big.bin"]);
    run(r, &["commit", "-m", "big"]);
    std::fs::write(r.join("big.bin"), b"overwritten").unwrap();
    let (code, _) = machine(r, &["--yes", "--", "big.bin"]);
    assert_eq!(code, 0);
    assert!(read(r, "big.bin") == big, "content differs");
}

/// A file another program holds open is reported in `failed`; the others
/// still restore, and the command exits non-zero.
#[cfg(windows)]
#[test]
fn a_locked_file_fails_alone() {
    use std::os::windows::fs::OpenOptionsExt;
    let dir = repo();
    let r = dir.path();
    std::fs::write(r.join("a.txt"), "edit a").unwrap();
    std::fs::write(r.join("keep.txt"), "edit keep").unwrap();
    let held = std::fs::OpenOptions::new()
        .read(true)
        .share_mode(0)
        .open(r.join("a.txt"))
        .unwrap();
    let (code, v) = machine(r, &["--yes", "--", "a.txt", "keep.txt"]);
    drop(held);
    assert_ne!(code, 0, "{v}");
    assert_eq!(v["data"]["failed"][0]["path"], "a.txt", "{v}");
    assert_eq!(v["data"]["restored"][0], "keep.txt", "{v}");
    assert_eq!(read(r, "keep.txt"), b"keep");
    assert_eq!(read(r, "a.txt"), b"edit a");
}
