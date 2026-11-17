// SPDX-License-Identifier: BUSL-1.1
// Copyright (C) 2025-2026 Aswin Krishnamoorthy

//! BUG-003: read commands find the repository from a subfolder, and in a
//! folder with no repository they fail and write nothing.

use assert_cmd::Command;
use std::path::Path;
use tempfile::TempDir;

#[allow(deprecated)]
fn mediagit() -> Command {
    let mut c = Command::cargo_bin("mediagit").unwrap();
    c.env("MEDIAGIT_AUTHOR_NAME", "Test User")
        .env("MEDIAGIT_AUTHOR_EMAIL", "test@example.com")
        .env_remove("MEDIAGIT_REPO");
    c
}

const READ_COMMANDS: &[&[&str]] = &[&["fsck"], &["verify"], &["tag", "list"], &["gc"]];

fn entry_count(dir: &Path) -> usize {
    std::fs::read_dir(dir).unwrap().count()
}

#[test]
fn read_commands_refuse_outside_a_repository_and_write_nothing() {
    for args in READ_COMMANDS {
        let dir = TempDir::new().unwrap();
        mediagit()
            .args(*args)
            .current_dir(dir.path())
            .assert()
            .failure();
        assert_eq!(
            entry_count(dir.path()),
            0,
            "`{}` left files in a folder that is not a repository",
            args.join(" ")
        );
    }
}

#[test]
fn read_commands_use_the_parent_repository_from_a_subfolder() {
    let dir = TempDir::new().unwrap();
    let sub = dir.path().join("design").join("sub");
    std::fs::create_dir_all(&sub).unwrap();
    mediagit()
        .args(["init", "-q"])
        .current_dir(dir.path())
        .assert()
        .success();
    std::fs::write(sub.join("b.txt"), b"hello").unwrap();
    mediagit()
        .args(["add", "design"])
        .current_dir(dir.path())
        .assert()
        .success();
    mediagit()
        .args(["commit", "-m", "c"])
        .current_dir(dir.path())
        .assert()
        .success();

    for args in READ_COMMANDS {
        mediagit().args(*args).current_dir(&sub).assert().success();
        assert!(
            !sub.join(".mediagit").exists(),
            "`{}` created a nested repository",
            args.join(" ")
        );
    }
}
