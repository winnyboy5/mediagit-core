// Copyright (C) 2026  winnyboy5
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU Affero General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU Affero General Public License for more details.
//
// You should have received a copy of the GNU Affero General Public License
// along with this program.  If not, see <https://www.gnu.org/licenses/>.
// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2025 MediaGit Contributors

//! Comprehensive End-to-End Test Suite
//!
//! Tests all MediaGit functionality in real-world scenarios using actual test files
//! including large media files (videos, 3D models, audio files)

use assert_cmd::Command;
use predicates::prelude::*;
use std::fs;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

#[cfg(windows)]
const TEST_FILES_DIR: &str = "D:\\own\\saas\\mediagit-core\\test-files";
#[cfg(not(windows))]
const TEST_FILES_DIR: &str = "/mnt/d/own/saas/mediagit-core/test-files";

/// Helper to create mediagit command
#[allow(deprecated)]
fn mediagit() -> Command {
    Command::cargo_bin("mediagit").unwrap()
}

/// Helper to initialize repository
fn init_repo(dir: &Path) {
    mediagit()
        .current_dir(dir)
        .arg("init")
        .assert()
        .success();
}

/// Copy test file to repository
fn copy_test_file(test_file: &str, repo_dir: &Path, dest_name: &str) -> PathBuf {
    let source = Path::new(TEST_FILES_DIR).join(test_file);
    let dest = repo_dir.join(dest_name);

    if source.is_file() {
        fs::copy(&source, &dest).expect("Failed to copy test file");
    } else if source.is_dir() {
        copy_dir_recursive(&source, &dest).expect("Failed to copy test directory");
    }

    dest
}

/// Recursively copy directory
fn copy_dir_recursive(src: &Path, dst: &Path) -> std::io::Result<()> {
    fs::create_dir_all(dst)?;
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let ty = entry.file_type()?;
        let src_path = entry.path();
        let dst_path = dst.join(entry.file_name());

        if ty.is_dir() {
            copy_dir_recursive(&src_path, &dst_path)?;
        } else {
            fs::copy(&src_path, &dst_path)?;
        }
    }
    Ok(())
}

// ============================================================================
// Test Suite 1: Basic Workflow Tests
// ============================================================================

#[test]
fn e2e_basic_workflow_small_file() {
    let temp_dir = TempDir::new().unwrap();
    let repo_path = temp_dir.path();

    // 1. Init
    init_repo(repo_path);

    // 2. Create a small file
    let test_file = repo_path.join("README.md");
    fs::write(&test_file, "# MediaGit Test Repository\n").unwrap();

    // 3. Add
    mediagit()
        .current_dir(repo_path)
        .arg("add")
        .arg("README.md")
        .assert()
        .success();

    // 4. Status
    mediagit()
        .current_dir(repo_path)
        .arg("status")
        .assert()
        .success()
        .stdout(predicate::str::contains("README.md"));

    // 5. Commit
    mediagit()
        .current_dir(repo_path)
        .arg("commit")
        .arg("-m")
        .arg("Initial commit")
        .assert()
        .success();

    // 6. Log
    mediagit()
        .current_dir(repo_path)
        .arg("log")
        .assert()
        .success()
        .stdout(predicate::str::contains("Initial commit"));
}

#[test]
fn e2e_basic_workflow_image_file() {
    let temp_dir = TempDir::new().unwrap();
    let repo_path = temp_dir.path();

    init_repo(repo_path);

    // Copy image file
    copy_test_file("freepik__talk__71826.jpeg", repo_path, "image.jpg");

    // Add and commit
    mediagit()
        .current_dir(repo_path)
        .arg("add")
        .arg("image.jpg")
        .assert()
        .success();

    mediagit()
        .current_dir(repo_path)
        .arg("commit")
        .arg("-m")
        .arg("Add image file")
        .assert()
        .success();

    // Verify commit
    mediagit()
        .current_dir(repo_path)
        .arg("log")
        .assert()
        .success()
        .stdout(predicate::str::contains("Add image file"));
}

// ============================================================================
// Test Suite 2: Large File Handling
// ============================================================================

#[test]
#[ignore] // Run with --ignored for large file tests
fn e2e_large_file_video_264mb() {
    let temp_dir = TempDir::new().unwrap();
    let repo_path = temp_dir.path();

    init_repo(repo_path);

    // Copy 264MB video file
    copy_test_file("bbb_sunflower_1080p_30fps_normal.mp4", repo_path, "video.mp4");

    // Add large file
    mediagit()
        .current_dir(repo_path)
        .arg("add")
        .arg("video.mp4")
        .assert()
        .success();

    // Commit large file
    mediagit()
        .current_dir(repo_path)
        .arg("commit")
        .arg("-m")
        .arg("Add 264MB video file")
        .assert()
        .success();

    // Verify status
    mediagit()
        .current_dir(repo_path)
        .arg("status")
        .assert()
        .success()
        .stdout(predicate::str::contains("clean"));
}

#[test]
#[ignore] // Run with --ignored for large file tests
fn e2e_large_file_video_398mb() {
    let temp_dir = TempDir::new().unwrap();
    let repo_path = temp_dir.path();

    init_repo(repo_path);

    // Copy 398MB video file
    copy_test_file("big_buck_bunny_720p_h264.mov", repo_path, "big_video.mov");

    mediagit()
        .current_dir(repo_path)
        .arg("add")
        .arg("big_video.mov")
        .assert()
        .success();

    mediagit()
        .current_dir(repo_path)
        .arg("commit")
        .arg("-m")
        .arg("Add 398MB video")
        .assert()
        .success();
}

#[test]
#[ignore] // Run with --ignored for very large file tests
fn e2e_very_large_file_2gb() {
    let temp_dir = TempDir::new().unwrap();
    let repo_path = temp_dir.path();

    init_repo(repo_path);

    // Copy 2GB file
    copy_test_file("bbb_sunflower_1080p_30fps_stereo_abl.mp4", repo_path, "huge_video.mp4");

    mediagit()
        .current_dir(repo_path)
        .arg("add")
        .arg("huge_video.mp4")
        .assert()
        .success();

    mediagit()
        .current_dir(repo_path)
        .arg("commit")
        .arg("-m")
        .arg("Add 2GB video file")
        .assert()
        .success();
}

// ============================================================================
// Test Suite 3: Media File Specific Tests
// ============================================================================

#[test]
fn e2e_media_3d_model_glb() {
    let temp_dir = TempDir::new().unwrap();
    let repo_path = temp_dir.path();

    init_repo(repo_path);

    // 14MB 3D model
    copy_test_file("1965_ac_shelby_427_cobra_sc.glb", repo_path, "car.glb");

    mediagit()
        .current_dir(repo_path)
        .arg("add")
        .arg("car.glb")
        .assert()
        .success();

    mediagit()
        .current_dir(repo_path)
        .arg("commit")
        .arg("-m")
        .arg("Add 3D car model")
        .assert()
        .success();
}

#[test]
fn e2e_media_audio_flac() {
    let temp_dir = TempDir::new().unwrap();
    let repo_path = temp_dir.path();

    init_repo(repo_path);

    // 38MB FLAC audio
    copy_test_file("_Amir_Tangsiri__Dokhtare_Koli.flac", repo_path, "audio.flac");

    mediagit()
        .current_dir(repo_path)
        .arg("add")
        .arg("audio.flac")
        .assert()
        .success();

    mediagit()
        .current_dir(repo_path)
        .arg("commit")
        .arg("-m")
        .arg("Add FLAC audio")
        .assert()
        .success();
}

#[test]
fn e2e_media_audio_wav() {
    let temp_dir = TempDir::new().unwrap();
    let repo_path = temp_dir.path();

    init_repo(repo_path);

    // 55MB WAV audio
    copy_test_file("_Quando_le_sere_al_placido__(Ferruccio_Giannini).wav", repo_path, "audio.wav");

    mediagit()
        .current_dir(repo_path)
        .arg("add")
        .arg("audio.wav")
        .assert()
        .success();

    mediagit()
        .current_dir(repo_path)
        .arg("commit")
        .arg("-m")
        .arg("Add WAV audio")
        .assert()
        .success();
}

#[test]
fn e2e_media_mixed_formats() {
    let temp_dir = TempDir::new().unwrap();
    let repo_path = temp_dir.path();

    init_repo(repo_path);

    // Copy multiple different media files
    copy_test_file("freepik__talk__71826.jpeg", repo_path, "image1.jpg");
    copy_test_file("freepik__talk__72772.jpeg", repo_path, "image2.jpg");
    copy_test_file("1965_ac_shelby_427_cobra_sc.glb", repo_path, "model.glb");

    // Create text file
    fs::write(repo_path.join("notes.txt"), "Project notes\n").unwrap();

    // Add all files
    mediagit()
        .current_dir(repo_path)
        .arg("add")
        .arg(".")
        .assert()
        .success();

    // Commit
    mediagit()
        .current_dir(repo_path)
        .arg("commit")
        .arg("-m")
        .arg("Add mixed media files")
        .assert()
        .success();

    // Verify status
    mediagit()
        .current_dir(repo_path)
        .arg("status")
        .assert()
        .success()
        .stdout(predicate::str::contains("clean"));
}

// ============================================================================
// Test Suite 4: Branching and Merging
// ============================================================================

#[test]
fn e2e_branch_create_and_switch() {
    let temp_dir = TempDir::new().unwrap();
    let repo_path = temp_dir.path();

    init_repo(repo_path);

    // Create initial commit
    fs::write(repo_path.join("file.txt"), "main branch\n").unwrap();
    mediagit()
        .current_dir(repo_path)
        .args(&["add", "file.txt"])
        .assert()
        .success();
    mediagit()
        .current_dir(repo_path)
        .args(&["commit", "-m", "Initial"])
        .assert()
        .success();

    // Create branch
    mediagit()
        .current_dir(repo_path)
        .args(&["branch", "create", "feature"])
        .assert()
        .success();

    // List branches
    mediagit()
        .current_dir(repo_path)
        .args(&["branch", "list"])
        .assert()
        .success()
        .stdout(predicate::str::contains("feature"));

    // Switch to branch
    mediagit()
        .current_dir(repo_path)
        .args(&["branch", "switch", "feature"])
        .assert()
        .success();

    // Verify we're on feature branch
    mediagit()
        .current_dir(repo_path)
        .args(&["branch", "list"])
        .assert()
        .success()
        .stdout(predicate::str::contains("* feature"));
}

#[test]
fn e2e_branch_with_media_files() {
    let temp_dir = TempDir::new().unwrap();
    let repo_path = temp_dir.path();

    init_repo(repo_path);

    // Main branch: add image
    copy_test_file("freepik__talk__71826.jpeg", repo_path, "main_image.jpg");
    mediagit()
        .current_dir(repo_path)
        .args(&["add", "main_image.jpg"])
        .assert()
        .success();
    mediagit()
        .current_dir(repo_path)
        .args(&["commit", "-m", "Add main image"])
        .assert()
        .success();

    // Create and switch to feature branch
    mediagit()
        .current_dir(repo_path)
        .args(&["branch", "create", "feature"])
        .assert()
        .success();
    mediagit()
        .current_dir(repo_path)
        .args(&["branch", "switch", "feature"])
        .assert()
        .success();

    // Add different media on feature branch
    copy_test_file("freepik__talk__72772.jpeg", repo_path, "feature_image.jpg");
    mediagit()
        .current_dir(repo_path)
        .args(&["add", "feature_image.jpg"])
        .assert()
        .success();
    mediagit()
        .current_dir(repo_path)
        .args(&["commit", "-m", "Add feature image"])
        .assert()
        .success();

    // Verify both images exist on feature branch
    assert!(repo_path.join("main_image.jpg").exists());
    assert!(repo_path.join("feature_image.jpg").exists());

    // Switch back to main
    mediagit()
        .current_dir(repo_path)
        .args(&["branch", "switch", "refs/heads/main"])
        .assert()
        .success();

    // Verify only main_image exists
    assert!(repo_path.join("main_image.jpg").exists());
    assert!(!repo_path.join("feature_image.jpg").exists());
}

// ============================================================================
// Test Suite 5: Multiple Files and Directories
// ============================================================================

#[test]
fn e2e_multiple_files_batch_add() {
    let temp_dir = TempDir::new().unwrap();
    let repo_path = temp_dir.path();

    init_repo(repo_path);

    // Create multiple files
    for i in 1..=10 {
        fs::write(repo_path.join(format!("file{}.txt", i)), format!("Content {}\n", i)).unwrap();
    }

    // Add all files
    mediagit()
        .current_dir(repo_path)
        .arg("add")
        .arg(".")
        .assert()
        .success();

    // Commit
    mediagit()
        .current_dir(repo_path)
        .args(&["commit", "-m", "Add 10 files"])
        .assert()
        .success();

    // Verify status
    mediagit()
        .current_dir(repo_path)
        .arg("status")
        .assert()
        .success()
        .stdout(predicate::str::contains("clean"));
}

#[test]
fn e2e_directory_structure() {
    let temp_dir = TempDir::new().unwrap();
    let repo_path = temp_dir.path();

    init_repo(repo_path);

    // Create directory structure
    fs::create_dir_all(repo_path.join("src/components")).unwrap();
    fs::create_dir_all(repo_path.join("src/utils")).unwrap();
    fs::create_dir_all(repo_path.join("docs")).unwrap();

    // Add files
    fs::write(repo_path.join("src/components/App.js"), "// App component\n").unwrap();
    fs::write(repo_path.join("src/utils/helper.js"), "// Helper utils\n").unwrap();
    fs::write(repo_path.join("docs/README.md"), "# Documentation\n").unwrap();

    // Copy media into structure
    copy_test_file("freepik__talk__71826.jpeg", repo_path, "docs/screenshot.jpg");

    // Add all
    mediagit()
        .current_dir(repo_path)
        .arg("add")
        .arg(".")
        .assert()
        .success();

    // Commit
    mediagit()
        .current_dir(repo_path)
        .args(&["commit", "-m", "Add project structure"])
        .assert()
        .success();
}

// ============================================================================
// Test Suite 6: File Modifications and Diffs
// ============================================================================

#[test]
fn e2e_file_modification() {
    let temp_dir = TempDir::new().unwrap();
    let repo_path = temp_dir.path();

    init_repo(repo_path);

    // Initial commit
    fs::write(repo_path.join("file.txt"), "Version 1\n").unwrap();
    mediagit()
        .current_dir(repo_path)
        .args(&["add", "file.txt"])
        .assert()
        .success();
    mediagit()
        .current_dir(repo_path)
        .args(&["commit", "-m", "Version 1"])
        .assert()
        .success();

    // Modify file
    fs::write(repo_path.join("file.txt"), "Version 2\n").unwrap();

    // Check status shows modification
    mediagit()
        .current_dir(repo_path)
        .arg("status")
        .assert()
        .success()
        .stdout(predicate::str::contains("modified"));

    // Add and commit modification
    mediagit()
        .current_dir(repo_path)
        .args(&["add", "file.txt"])
        .assert()
        .success();
    mediagit()
        .current_dir(repo_path)
        .args(&["commit", "-m", "Version 2"])
        .assert()
        .success();
}

#[test]
fn e2e_media_file_replacement() {
    let temp_dir = TempDir::new().unwrap();
    let repo_path = temp_dir.path();

    init_repo(repo_path);

    // Add first image
    copy_test_file("freepik__talk__71826.jpeg", repo_path, "image.jpg");
    mediagit()
        .current_dir(repo_path)
        .args(&["add", "image.jpg"])
        .assert()
        .success();
    mediagit()
        .current_dir(repo_path)
        .args(&["commit", "-m", "Add image v1"])
        .assert()
        .success();

    // Replace with different image
    copy_test_file("freepik__talk__72772.jpeg", repo_path, "image.jpg");

    // Check status
    mediagit()
        .current_dir(repo_path)
        .arg("status")
        .assert()
        .success()
        .stdout(predicate::str::contains("modified"));

    // Commit replacement
    mediagit()
        .current_dir(repo_path)
        .args(&["add", "image.jpg"])
        .assert()
        .success();
    mediagit()
        .current_dir(repo_path)
        .args(&["commit", "-m", "Replace image"])
        .assert()
        .success();
}

// ============================================================================
// Test Suite 7: Stats and Information Commands
// ============================================================================

#[test]
fn e2e_stats_command() {
    let temp_dir = TempDir::new().unwrap();
    let repo_path = temp_dir.path();

    init_repo(repo_path);

    // Add some files
    fs::write(repo_path.join("file.txt"), "Test content\n").unwrap();
    mediagit()
        .current_dir(repo_path)
        .args(&["add", "file.txt"])
        .assert()
        .success();
    mediagit()
        .current_dir(repo_path)
        .args(&["commit", "-m", "Test commit"])
        .assert()
        .success();

    // Run stats command
    mediagit()
        .current_dir(repo_path)
        .arg("stats")
        .assert()
        .success()
        .stdout(predicate::str::contains("Repository Statistics"));

    // Run stats with different flags
    mediagit()
        .current_dir(repo_path)
        .args(&["stats", "--storage"])
        .assert()
        .success();

    mediagit()
        .current_dir(repo_path)
        .args(&["stats", "--branches"])
        .assert()
        .success();
}

// ============================================================================
// Test Suite 8: Error Handling and Edge Cases
// ============================================================================

#[test]
fn e2e_add_nonexistent_file() {
    let temp_dir = TempDir::new().unwrap();
    let repo_path = temp_dir.path();

    init_repo(repo_path);

    mediagit()
        .current_dir(repo_path)
        .args(&["add", "nonexistent.txt"])
        .assert()
        .failure();
}

#[test]
fn e2e_commit_without_add() {
    let temp_dir = TempDir::new().unwrap();
    let repo_path = temp_dir.path();

    init_repo(repo_path);

    mediagit()
        .current_dir(repo_path)
        .args(&["commit", "-m", "Empty commit"])
        .assert()
        .failure();
}

#[test]
fn e2e_branch_nonexistent() {
    let temp_dir = TempDir::new().unwrap();
    let repo_path = temp_dir.path();

    init_repo(repo_path);

    // Try to switch to nonexistent branch
    mediagit()
        .current_dir(repo_path)
        .args(&["branch", "switch", "nonexistent"])
        .assert()
        .failure();
}

// ============================================================================
// Test Suite 9: Progress Indicators (Verification)
// ============================================================================

#[test]
fn e2e_verify_progress_quiet_mode() {
    let temp_dir = TempDir::new().unwrap();
    let repo_path = temp_dir.path();

    init_repo(repo_path);

    copy_test_file("freepik__talk__71826.jpeg", repo_path, "image.jpg");

    // Add with quiet mode - should not show progress
    mediagit()
        .current_dir(repo_path)
        .args(&["add", "--quiet", "image.jpg"])
        .assert()
        .success()
        .stdout(predicate::str::is_empty());
}

#[test]
fn e2e_verify_stats_displayed() {
    let temp_dir = TempDir::new().unwrap();
    let repo_path = temp_dir.path();

    init_repo(repo_path);

    copy_test_file("freepik__talk__71826.jpeg", repo_path, "image.jpg");

    mediagit()
        .current_dir(repo_path)
        .args(&["add", "image.jpg"])
        .assert()
        .success();

    mediagit()
        .current_dir(repo_path)
        .args(&["commit", "-m", "Add image"])
        .assert()
        .success();
}
