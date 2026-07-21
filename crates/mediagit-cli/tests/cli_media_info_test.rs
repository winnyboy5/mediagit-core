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

//! Integration tests for M5b `mediagit media info` (#7) and the
//! Model3DParser wiring into `media_meta.rs`.
//!
//! `media info` reads a plain filesystem path directly — it needs no
//! repository — so these tests point straight at `test-files/` fixtures.
//! Real fixtures per format (image/video/audio/3D); PSD uses the smallest
//! available real fixture (no small PSD exists in the tree).

use assert_cmd::Command;
use predicates::prelude::*;
use std::path::Path;
use tempfile::TempDir;

#[cfg(windows)]
const TEST_FILES_DIR: &str = "D:\\own\\saas\\mediagit-core\\test-files";
#[cfg(not(windows))]
const TEST_FILES_DIR: &str = "/mnt/d/own/saas/mediagit-core/test-files";

#[allow(deprecated)]
fn mediagit() -> Command {
    Command::cargo_bin("mediagit").unwrap()
}

fn fixture(rel: &str) -> Option<std::path::PathBuf> {
    let path = Path::new(TEST_FILES_DIR).join(rel);
    path.exists().then_some(path)
}

#[test]
fn media_info_image_human_output() {
    let Some(path) = fixture("freepik__talk__72772.jpeg") else {
        println!("SKIP: image fixture not found");
        return;
    };
    mediagit()
        .arg("media")
        .arg("info")
        .arg(&path)
        .assert()
        .success()
        .stdout(predicate::str::contains("width:"))
        .stdout(predicate::str::contains("height:"))
        .stdout(predicate::str::contains("format:"));
}

#[test]
fn media_info_image_json() {
    let Some(path) = fixture("freepik__talk__72772.jpeg") else {
        println!("SKIP: image fixture not found");
        return;
    };
    let output = mediagit()
        .arg("media")
        .arg("info")
        .arg(&path)
        .arg("--json")
        .output()
        .unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let value: serde_json::Value =
        serde_json::from_str(&stdout).unwrap_or_else(|e| panic!("bad json: {e}\n{stdout}"));
    assert!(value.get("width").and_then(|v| v.as_u64()).unwrap_or(0) > 0);
    assert!(value.get("height").and_then(|v| v.as_u64()).unwrap_or(0) > 0);
}

#[test]
fn media_info_video_human_output() {
    let Some(path) = fixture("video-variants/h265-reencode.mp4") else {
        println!("SKIP: video fixture not found");
        return;
    };
    mediagit()
        .arg("media")
        .arg("info")
        .arg(&path)
        .assert()
        .success()
        .stdout(predicate::str::contains("duration_seconds:"));
}

#[test]
fn media_info_audio_human_output() {
    let Some(path) = fixture("pioneer-master/pioneer-master/data/sounds/Interface/Click.ogg")
    else {
        println!("SKIP: audio fixture not found");
        return;
    };
    mediagit()
        .arg("media")
        .arg("info")
        .arg(&path)
        .assert()
        .success()
        .stdout(predicate::str::contains("sample_rate:"))
        .stdout(predicate::str::contains("channels:"));
}

/// No small PSD fixture exists anywhere in the tree. `psd/26952784_food_flyer_19.psd`
/// (the smallest one, ~70 MB) hits a pre-existing `mediagit-media` PSD-parser
/// limitation (spot-color channel id 3) unrelated to this command, so this
/// uses a slightly larger fixture that the parser handles cleanly.
#[test]
fn media_info_psd_human_output() {
    let Some(path) = fixture("caricature-photo-effect/10785860.psd") else {
        println!("SKIP: PSD fixture not found");
        return;
    };
    mediagit()
        .arg("media")
        .arg("info")
        .arg(&path)
        .assert()
        .success()
        .stdout(predicate::str::contains("width:"))
        .stdout(predicate::str::contains("color_mode:"));
}

#[test]
fn media_info_3d_model_human_output() {
    let Some(path) = fixture("1900s_telephone.stl") else {
        println!("SKIP: 3D fixture not found");
        return;
    };
    mediagit()
        .arg("media")
        .arg("info")
        .arg(&path)
        .assert()
        .success()
        .stdout(predicate::str::contains("vertex_count:"))
        .stdout(predicate::str::contains("face_count:"));
}

#[test]
fn media_info_3d_model_json() {
    let Some(path) = fixture("1900s_telephone.stl") else {
        println!("SKIP: 3D fixture not found");
        return;
    };
    let output = mediagit()
        .arg("media")
        .arg("info")
        .arg(&path)
        .arg("--json")
        .output()
        .unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let value: serde_json::Value =
        serde_json::from_str(&stdout).unwrap_or_else(|e| panic!("bad json: {e}\n{stdout}"));
    assert!(value.get("vertex_count").is_some());
}

#[test]
fn media_info_unsupported_extension_clean_message_exit_zero() {
    let temp = TempDir::new().unwrap();
    let path = temp.path().join("notes.txt");
    std::fs::write(&path, b"hello").unwrap();

    mediagit()
        .arg("media")
        .arg("info")
        .arg(&path)
        .assert()
        .success()
        .stdout(predicate::str::contains("Unsupported media type"));
}

/// Files over the 256 MB cap must be short-circuited before parsing — use a
/// sparse file (via `set_len`, no actual bytes written) so the test stays
/// cheap regardless of the cap's exact size.
#[test]
fn media_info_over_cap_short_circuits() {
    let temp = TempDir::new().unwrap();
    let path = temp.path().join("huge.png");
    let file = std::fs::File::create(&path).unwrap();
    file.set_len(257 * 1024 * 1024).unwrap();
    drop(file);

    mediagit()
        .arg("media")
        .arg("info")
        .arg(&path)
        .assert()
        .success()
        .stdout(predicate::str::contains("exceeds"))
        .stdout(predicate::str::contains("cap"));
}

#[test]
fn media_info_missing_file_errors() {
    mediagit()
        .arg("media")
        .arg("info")
        .arg("does/not/exist.png")
        .assert()
        .failure();
}
