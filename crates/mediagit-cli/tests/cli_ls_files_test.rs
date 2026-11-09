// SPDX-License-Identifier: BUSL-1.1
// Copyright (C) 2025-2026 Aswin Krishnamoorthy

//! `mediagit ls-files --machine`: every working-folder file with its status.

use assert_cmd::Command;
use serde_json::Value;
use std::fs;
use std::path::Path;
use tempfile::TempDir;

#[allow(deprecated)]
fn mediagit(dir: &Path, args: &[&str]) -> std::process::Output {
    Command::cargo_bin("mediagit")
        .unwrap()
        .env("MEDIAGIT_AUTHOR_NAME", "Test User")
        .env("MEDIAGIT_AUTHOR_EMAIL", "test@example.com")
        .args(args)
        .current_dir(dir)
        .output()
        .unwrap()
}

fn run(dir: &Path, args: &[&str]) {
    let o = mediagit(dir, args);
    assert!(
        o.status.success(),
        "{args:?}: {}",
        String::from_utf8_lossy(&o.stderr)
    );
}

fn write(dir: &Path, name: &str, body: &[u8]) {
    let p = dir.join(name);
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, body).unwrap();
}

/// `(path, status, staged)` rows, sorted as emitted.
fn ls(dir: &Path, extra: &[&str]) -> Vec<(String, String, bool)> {
    let mut args = vec!["--machine", "ls-files"];
    args.extend_from_slice(extra);
    let o = mediagit(dir, &args);
    let v: Value = serde_json::from_slice(&o.stdout).unwrap();
    assert_eq!(v["ok"], true, "{v}");
    v["data"]["files"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| {
            (
                f["path"].as_str().unwrap().to_owned(),
                f["status"].as_str().unwrap().to_owned(),
                f["staged"].as_bool().unwrap(),
            )
        })
        .collect()
}

fn repo() -> TempDir {
    let d = TempDir::new().unwrap();
    run(d.path(), &["init", "-q"]);
    d
}

fn row(rows: &[(String, String, bool)], path: &str) -> (String, bool) {
    let r = rows
        .iter()
        .find(|r| r.0 == path)
        .unwrap_or_else(|| panic!("{path} missing in {rows:?}"));
    (r.1.clone(), r.2)
}

#[test]
fn untracked_file_is_new_and_unstaged() {
    let d = repo();
    write(d.path(), "a.bin", b"hello");
    let rows = ls(d.path(), &[]);
    assert_eq!(row(&rows, "a.bin"), ("new".into(), false));
    assert!(rows.iter().all(|r| !r.0.starts_with(".mediagit")));
}

#[test]
fn staged_new_file_is_flagged_staged() {
    let d = repo();
    write(d.path(), "a.bin", b"hello");
    run(d.path(), &["add", "a.bin"]);
    assert_eq!(row(&ls(d.path(), &[]), "a.bin"), ("new".into(), true));
}

#[test]
fn committed_modified_and_deleted() {
    let d = repo();
    write(d.path(), "keep.bin", b"1");
    write(d.path(), "edit.bin", b"2");
    write(d.path(), "gone.bin", b"3");
    run(d.path(), &["add", "."]);
    run(d.path(), &["commit", "-m", "c1"]);
    write(d.path(), "edit.bin", b"changed");
    fs::remove_file(d.path().join("gone.bin")).unwrap();
    let rows = ls(d.path(), &[]);
    assert_eq!(row(&rows, "keep.bin"), ("unchanged".into(), false));
    assert_eq!(row(&rows, "edit.bin"), ("modified".into(), false));
    assert_eq!(row(&rows, "gone.bin"), ("deleted".into(), false));
    run(d.path(), &["add", "edit.bin"]);
    assert_eq!(
        row(&ls(d.path(), &[]), "edit.bin"),
        ("modified".into(), true)
    );
}

#[test]
fn ignored_only_with_flag() {
    let d = repo();
    write(d.path(), ".mediagitignore", b"*.tmp\n");
    write(d.path(), "x.tmp", b"t");
    write(d.path(), "y.bin", b"y");
    assert!(ls(d.path(), &[]).iter().all(|r| r.0 != "x.tmp"));
    assert_eq!(row(&ls(d.path(), &["--ignored"]), "x.tmp").0, "ignored");
}

#[test]
fn nested_dirs_use_forward_slashes_and_paths_filter() {
    let d = repo();
    write(d.path(), "a/b/c.bin", b"1");
    write(d.path(), "a/d.bin", b"2");
    write(d.path(), "ab/e.bin", b"3");
    let all = ls(d.path(), &[]);
    assert_eq!(row(&all, "a/b/c.bin").0, "new");
    let only: Vec<_> = ls(d.path(), &["a"]).into_iter().map(|r| r.0).collect();
    assert_eq!(only, ["a/b/c.bin", "a/d.bin"]);
}

#[test]
fn two_thousand_files() {
    let d = repo();
    for i in 0..2000 {
        write(d.path(), &format!("d{}/f{i}.bin", i % 20), b"x");
    }
    let rows = ls(d.path(), &[]);
    assert_eq!(rows.iter().filter(|r| r.1 == "new").count(), 2000);
}
