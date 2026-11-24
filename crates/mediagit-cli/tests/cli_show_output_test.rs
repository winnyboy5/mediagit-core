// SPDX-License-Identifier: BUSL-1.1
// Copyright (C) 2025-2026 Aswin Krishnamoorthy

//! `show <rev>:<path> --output <file>` writes atomically: a failed read leaves
//! the target untouched (it used to be emptied first), and missing parent
//! folders are created. The app's "Restore this file" and the merge resolver
//! write through this path.

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

/// Commit `a.txt`; returns the object files its `add` created.
fn commit_a(repo: &Path) -> Vec<PathBuf> {
    run(repo, &["init", "-q"]);
    std::fs::write(repo.join("a.txt"), b"saved version\n").unwrap();
    let before = object_files(repo);
    run(repo, &["add", "a.txt"]);
    let added: Vec<PathBuf> = object_files(repo).difference(&before).cloned().collect();
    run(repo, &["commit", "-m", "c"]);
    added
}

#[test]
fn failed_read_leaves_the_target_untouched() {
    let dir = TempDir::new().unwrap();
    let repo = dir.path();
    let blob_files = commit_a(repo);
    assert!(!blob_files.is_empty(), "fixture: add stored the blob");
    for f in &blob_files {
        std::fs::remove_file(f).unwrap();
    }
    let target = repo.join("out.txt");
    std::fs::write(&target, b"my unsaved work").unwrap();

    mediagit()
        .args(["show", "HEAD:a.txt", "--output"])
        .arg(&target)
        .current_dir(repo)
        .assert()
        .failure();
    assert_eq!(std::fs::read(&target).unwrap(), b"my unsaved work");
}

#[test]
fn output_into_a_missing_folder_is_created() {
    let dir = TempDir::new().unwrap();
    let repo = dir.path();
    commit_a(repo);
    let target = repo.join("gone").join("deeper").join("a.txt");

    run(
        repo,
        &["show", "HEAD:a.txt", "--output", target.to_str().unwrap()],
    );
    assert_eq!(std::fs::read(&target).unwrap(), b"saved version\n");
}
