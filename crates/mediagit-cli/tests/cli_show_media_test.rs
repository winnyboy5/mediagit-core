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

//! Integration tests for P4b: media metadata surfaced in `mediagit show`.
//!
//! Uses the `dev-tests/dedup-pairs/jpg_v1.jpg` fixture (a real 3000x4000
//! JPEG re-export) to verify the `media: ...` line, the `MEDIAGIT_MEDIA_META=0`
//! kill switch, and that malformed media bytes never panic `show`.

use assert_cmd::Command;
use predicates::prelude::*;
use std::fs;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

#[cfg(windows)]
const DEDUP_PAIRS_DIR: &str = "D:\\own\\saas\\mediagit-core\\dev-tests\\dedup-pairs";
#[cfg(not(windows))]
const DEDUP_PAIRS_DIR: &str = "/mnt/d/own/saas/mediagit-core/dev-tests/dedup-pairs";

#[allow(deprecated)]
fn mediagit() -> Command {
    {
        // `commit` refuses an unconfigured identity (UX-6) instead of
        // authoring as `Unknown <unknown@localhost>`, so tests declare one
        // the way a real user would.
        let mut c = Command::cargo_bin("mediagit").unwrap();
        c.env("MEDIAGIT_AUTHOR_NAME", "Test User")
            .env("MEDIAGIT_AUTHOR_EMAIL", "test@example.com");
        c
    }
}

fn init_repo(dir: &Path) {
    mediagit()
        .arg("init")
        .arg("-q")
        .current_dir(dir)
        .assert()
        .success();
}

fn jpg_fixture() -> Option<PathBuf> {
    let path = Path::new(DEDUP_PAIRS_DIR).join("jpg_v1.jpg");
    path.exists().then_some(path)
}

/// Commit `src` into a fresh repo under `dest_name`. Returns the repo dir.
fn commit_file(src: &Path, dest_name: &str) -> TempDir {
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());

    let dest = temp_dir.path().join(dest_name);
    fs::copy(src, &dest).unwrap();

    mediagit()
        .arg("add")
        .arg(dest_name)
        .current_dir(temp_dir.path())
        .assert()
        .success();
    mediagit()
        .arg("commit")
        .arg("-m")
        .arg("add media file")
        .current_dir(temp_dir.path())
        .assert()
        .success();

    temp_dir
}

#[test]
fn test_show_prints_media_line_for_committed_jpg() {
    let Some(jpg_path) = jpg_fixture() else {
        println!("SKIP: dev-tests/dedup-pairs/jpg_v1.jpg not found");
        return;
    };

    let temp_dir = commit_file(&jpg_path, "photo.jpg");

    mediagit()
        .arg("show")
        .arg("HEAD")
        .current_dir(temp_dir.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("media:"))
        .stdout(predicate::str::contains("3000x4000"))
        .stdout(predicate::str::contains("jpeg"));
}

#[test]
fn test_show_media_line_suppressed_by_kill_switch() {
    let Some(jpg_path) = jpg_fixture() else {
        println!("SKIP: dev-tests/dedup-pairs/jpg_v1.jpg not found");
        return;
    };

    let temp_dir = commit_file(&jpg_path, "photo.jpg");

    mediagit()
        .arg("show")
        .arg("HEAD")
        .env("MEDIAGIT_MEDIA_META", "0")
        .current_dir(temp_dir.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("media:").not());
}

#[test]
fn test_show_truncated_jpg_does_not_panic() {
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());

    // Malformed/truncated JPEG bytes: a plausible-looking header with no
    // valid image data behind it.
    let dest = temp_dir.path().join("broken.jpg");
    fs::write(
        &dest,
        [0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x10, b'J', b'F', b'I', b'F'],
    )
    .unwrap();

    mediagit()
        .arg("add")
        .arg("broken.jpg")
        .current_dir(temp_dir.path())
        .assert()
        .success();
    mediagit()
        .arg("commit")
        .arg("-m")
        .arg("add broken jpg")
        .current_dir(temp_dir.path())
        .assert()
        .success();

    mediagit()
        .arg("show")
        .arg("HEAD")
        .current_dir(temp_dir.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("media:").not());
}
