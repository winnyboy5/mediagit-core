// SPDX-License-Identifier: BUSL-1.1
// Copyright (C) 2025-2026 Aswin Krishnamoorthy

//! `diff -p`: line-by-line changes for text files only, opt-in. Default
//! `diff` output is unchanged; binary and large files get no hunks.

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

fn commit(repo: &Path, file: &str, body: &[u8], msg: &str) {
    std::fs::write(repo.join(file), body).unwrap();
    run(repo, &["add", file]);
    run(repo, &["commit", "-m", msg]);
}

fn machine(repo: &Path, args: &[&str]) -> Value {
    let out = mediagit()
        .args(["--machine", "diff"])
        .args(args)
        .current_dir(repo)
        .output()
        .unwrap();
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert_eq!(stdout.lines().count(), 1, "{stdout}");
    let v: Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(v["ok"], true, "{v}");
    v["data"].clone()
}

/// The `+`/`-`/` ` lines of a file's hunks.
fn lines(file: &Value) -> Vec<String> {
    file["text"]["hunks"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|h| h["lines"].as_array().unwrap().clone())
        .map(|l| {
            format!(
                "{}{}",
                l["kind"].as_str().unwrap(),
                l["text"].as_str().unwrap()
            )
        })
        .collect()
}

fn file<'a>(d: &'a Value, path: &str) -> &'a Value {
    d["files"]
        .as_array()
        .unwrap()
        .iter()
        .find(|f| f["path"] == path)
        .unwrap_or_else(|| panic!("{path} not in {d}"))
}

fn repo() -> TempDir {
    let dir = TempDir::new().unwrap();
    let r = dir.path();
    run(r, &["init", "-q"]);
    commit(r, "a.txt", b"one\ntwo\nthree\n", "c1");
    dir
}

#[test]
fn working_cached_and_commits_modes_all_give_hunks_with_p() {
    let dir = repo();
    let r = dir.path();
    std::fs::write(r.join("a.txt"), "one\nTWO\nthree\n").unwrap();
    let want = [" one", "-two", "+TWO", " three"];

    let d = machine(r, &["-p"]);
    assert_eq!(file(&d, "a.txt")["text"]["kind"], "text");
    assert_eq!(lines(file(&d, "a.txt")), want);

    run(r, &["add", "a.txt"]);
    let d = machine(r, &["--cached", "-p"]);
    assert_eq!(lines(file(&d, "a.txt")), want);

    run(r, &["commit", "-m", "c2"]);
    let d = machine(r, &["-p", "HEAD~1", "HEAD"]);
    assert_eq!(lines(file(&d, "a.txt")), want);
}

#[test]
fn without_p_there_is_no_text_and_no_hunks() {
    let dir = repo();
    let r = dir.path();
    std::fs::write(r.join("a.txt"), "changed\n").unwrap();
    let d = machine(r, &[]);
    assert!(file(&d, "a.txt").get("text").is_none(), "{d}");
    let out = mediagit().arg("diff").current_dir(r).output().unwrap();
    let human = String::from_utf8_lossy(&out.stdout);
    assert!(human.contains("a.txt") && !human.contains("@@"), "{human}");
}

#[test]
fn new_files_are_all_additions_and_the_path_filter_applies() {
    let dir = repo();
    let r = dir.path();
    std::fs::write(r.join("new.txt"), "x\ny\n").unwrap();
    std::fs::write(r.join("a.txt"), "other\n").unwrap();
    let d = machine(r, &["-p", "--", "new.txt"]);
    assert_eq!(d["files"].as_array().unwrap().len(), 1, "{d}");
    assert_eq!(lines(file(&d, "new.txt")), ["+x", "+y"]);
}

#[test]
fn binary_and_large_files_get_no_hunks() {
    let dir = repo();
    let r = dir.path();
    commit(r, "pic.bin", b"PNG\0\x01\x02", "bin");
    let mut x: u64 = 0x9E37_79B9_7F4A_7C15;
    let big: Vec<u8> = (0..12 * 1024 * 1024)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            b'a' + (x % 26) as u8
        })
        .collect();
    commit(r, "big.txt", &big, "big");
    std::fs::write(r.join("pic.bin"), b"PNG\0\x09").unwrap();
    std::fs::write(r.join("big.txt"), b"small now\n").unwrap();
    let d = machine(r, &["-p"]);
    assert_eq!(file(&d, "pic.bin")["text"]["kind"], "binary");
    assert!(
        file(&d, "pic.bin")["text"]["hunks"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert_eq!(file(&d, "big.txt")["text"]["kind"], "too_large");
}

#[test]
fn human_p_prints_a_unified_diff_after_the_list() {
    let dir = repo();
    let r = dir.path();
    std::fs::write(r.join("a.txt"), "one\nTWO\nthree\n").unwrap();
    let out = mediagit()
        .args(["diff", "-p"])
        .current_dir(r)
        .output()
        .unwrap();
    assert!(out.status.success());
    let s = String::from_utf8_lossy(&out.stdout);
    assert!(
        s.contains("--- a/a.txt\n+++ b/a.txt\n@@ -1,3 +1,3 @@\n one\n-two\n+TWO\n three\n"),
        "{s}"
    );
}
