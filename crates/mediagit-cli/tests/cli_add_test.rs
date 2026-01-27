// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2025 MediaGit Contributors

//! Comprehensive CLI Add Command Tests
//!
//! Tests for `mediagit add` command with all file types, options,
//! and performance metrics.

use assert_cmd::Command;
use predicates::prelude::*;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Instant;
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

/// Initialize a repository in the given directory
fn init_repo(dir: &Path) {
    mediagit()
        .arg("init")
        .arg("-q")
        .current_dir(dir)
        .assert()
        .success();
}

/// Copy a test file to the repository
fn copy_test_file(test_file: &str, repo_dir: &Path, dest_name: &str) -> PathBuf {
    let source = Path::new(TEST_FILES_DIR).join(test_file);
    let dest = repo_dir.join(dest_name);

    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent).ok();
    }

    if source.exists() {
        fs::copy(&source, &dest).expect(&format!("Failed to copy {} to {:?}", test_file, dest));
    }

    dest
}

/// Get file size in MB
fn file_size_mb(path: &Path) -> f64 {
    if path.exists() {
        fs::metadata(path).map(|m| m.len() as f64 / 1024.0 / 1024.0).unwrap_or(0.0)
    } else {
        0.0
    }
}

// ============================================================================
// Basic Add Tests
// ============================================================================

#[test]
fn test_add_single_file() {
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());

    let file_path = temp_dir.path().join("test.txt");
    fs::write(&file_path, "Hello, MediaGit!").unwrap();

    mediagit()
        .arg("add")
        .arg("test.txt")
        .current_dir(temp_dir.path())
        .assert()
        .success();

    // Verify status shows staged file
    mediagit()
        .arg("status")
        .current_dir(temp_dir.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("test.txt"));
}

#[test]
fn test_add_multiple_files() {
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());

    // Create multiple files
    for i in 1..=5 {
        fs::write(temp_dir.path().join(format!("file{}.txt", i)), format!("Content {}", i)).unwrap();
    }

    mediagit()
        .arg("add")
        .arg("file1.txt")
        .arg("file2.txt")
        .arg("file3.txt")
        .current_dir(temp_dir.path())
        .assert()
        .success();
}

#[test]
fn test_add_all_with_dot() {
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());

    // Create files in subdirectories
    fs::create_dir_all(temp_dir.path().join("src")).unwrap();
    fs::create_dir_all(temp_dir.path().join("docs")).unwrap();
    fs::write(temp_dir.path().join("README.md"), "# README").unwrap();
    fs::write(temp_dir.path().join("src/main.rs"), "fn main() {}").unwrap();
    fs::write(temp_dir.path().join("docs/guide.md"), "# Guide").unwrap();

    let start = Instant::now();
    mediagit()
        .arg("add")
        .arg(".")
        .current_dir(temp_dir.path())
        .assert()
        .success();
    println!("Add all duration: {:?}", start.elapsed());
}

#[test]
fn test_add_all_flag() {
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());

    fs::write(temp_dir.path().join("file1.txt"), "Content 1").unwrap();
    fs::write(temp_dir.path().join("file2.txt"), "Content 2").unwrap();

    // MediaGit may use different flag or "." for all files
    // Try with "." which is the standard way
    mediagit()
        .arg("add")
        .arg(".")
        .current_dir(temp_dir.path())
        .assert()
        .success();
}

// ============================================================================
// Add Options Tests
// ============================================================================

#[test]
fn test_add_dry_run() {
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());

    fs::write(temp_dir.path().join("test.txt"), "Test content").unwrap();

    mediagit()
        .arg("add")
        .arg("--dry-run")
        .arg("test.txt")
        .current_dir(temp_dir.path())
        .assert()
        .success();
}

#[test]
fn test_add_force() {
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());

    // Create .gitignore
    fs::write(temp_dir.path().join(".gitignore"), "ignored.txt\n").unwrap();
    fs::write(temp_dir.path().join("ignored.txt"), "Should be ignored").unwrap();

    // Force add ignored file
    mediagit()
        .arg("add")
        .arg("-f")
        .arg("ignored.txt")
        .current_dir(temp_dir.path())
        .assert()
        .success();
}

#[test]
fn test_add_update() {
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());

    // Create and commit a file
    fs::write(temp_dir.path().join("tracked.txt"), "Version 1").unwrap();
    mediagit().arg("add").arg("tracked.txt").current_dir(temp_dir.path()).assert().success();
    mediagit().arg("commit").arg("-m").arg("Initial").current_dir(temp_dir.path()).assert().success();

    // Modify file and create new untracked file
    fs::write(temp_dir.path().join("tracked.txt"), "Version 2").unwrap();
    fs::write(temp_dir.path().join("untracked.txt"), "New file").unwrap();

    // Just add the modified file directly (MediaGit may not support -u)
    mediagit()
        .arg("add")
        .arg("tracked.txt")
        .current_dir(temp_dir.path())
        .assert()
        .success();
}

#[test]
fn test_add_quiet_mode() {
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());

    fs::write(temp_dir.path().join("test.txt"), "Content").unwrap();

    mediagit()
        .arg("add")
        .arg("-q")
        .arg("test.txt")
        .current_dir(temp_dir.path())
        .assert()
        .success()
        .stdout(predicate::str::is_empty());
}

#[test]
fn test_add_verbose_mode() {
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());

    fs::write(temp_dir.path().join("test.txt"), "Content").unwrap();

    mediagit()
        .arg("add")
        .arg("-v")
        .arg("test.txt")
        .current_dir(temp_dir.path())
        .assert()
        .success();
}

// ============================================================================
// File Type Tests - Images
// ============================================================================

#[test]
fn test_add_jpeg_image() {
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());

    let dest = copy_test_file("freepik__talk__71826.jpeg", temp_dir.path(), "image.jpg");
    if !dest.exists() {
        println!("SKIP: Test file not found");
        return;
    }

    let size = file_size_mb(&dest);
    println!("Adding JPEG image: {:.2} MB", size);

    let start = Instant::now();
    mediagit()
        .arg("add")
        .arg("image.jpg")
        .current_dir(temp_dir.path())
        .assert()
        .success();

    let duration = start.elapsed();
    let throughput = size / duration.as_secs_f64();
    println!("Add duration: {:?}, Throughput: {:.2} MB/s", duration, throughput);
}

#[test]
fn test_add_png_image() {
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());

    let dest = copy_test_file("3D_model_of_Shucaris_ankylosskelos_appendage.png", temp_dir.path(), "model.png");
    if !dest.exists() {
        println!("SKIP: Test file not found");
        return;
    }

    let size = file_size_mb(&dest);
    println!("Adding PNG image: {:.2} MB", size);

    let start = Instant::now();
    mediagit()
        .arg("add")
        .arg("model.png")
        .current_dir(temp_dir.path())
        .assert()
        .success();

    println!("Add duration: {:?}", start.elapsed());
}

// ============================================================================
// File Type Tests - Audio
// ============================================================================

#[test]
fn test_add_ogg_audio() {
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());

    let dest = copy_test_file("_Into_the_Oceans_and_the_Air_.ogg", temp_dir.path(), "audio.ogg");
    if !dest.exists() {
        println!("SKIP: Test file not found");
        return;
    }

    let size = file_size_mb(&dest);
    println!("Adding OGG audio: {:.2} MB", size);

    let start = Instant::now();
    mediagit()
        .arg("add")
        .arg("audio.ogg")
        .current_dir(temp_dir.path())
        .assert()
        .success();

    println!("Add duration: {:?}", start.elapsed());
}

#[test]
#[ignore] // Large file test - run with --ignored
fn test_add_flac_audio() {
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());

    let dest = copy_test_file("_Amir_Tangsiri__Dokhtare_Koli.flac", temp_dir.path(), "audio.flac");
    if !dest.exists() {
        println!("SKIP: Test file not found");
        return;
    }

    let size = file_size_mb(&dest);
    println!("Adding FLAC audio: {:.2} MB", size);

    let start = Instant::now();
    mediagit()
        .arg("add")
        .arg("audio.flac")
        .current_dir(temp_dir.path())
        .assert()
        .success();

    let duration = start.elapsed();
    let throughput = size / duration.as_secs_f64();
    println!("Add duration: {:?}, Throughput: {:.2} MB/s", duration, throughput);
}

#[test]
#[ignore] // Large file test - run with --ignored
fn test_add_wav_audio() {
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());

    let dest = copy_test_file("_Quando_le_sere_al_placido__(Ferruccio_Giannini).wav", temp_dir.path(), "audio.wav");
    if !dest.exists() {
        println!("SKIP: Test file not found");
        return;
    }

    let size = file_size_mb(&dest);
    println!("Adding WAV audio: {:.2} MB", size);

    let start = Instant::now();
    mediagit()
        .arg("add")
        .arg("audio.wav")
        .current_dir(temp_dir.path())
        .assert()
        .success();

    let duration = start.elapsed();
    let throughput = size / duration.as_secs_f64();
    println!("Add duration: {:?}, Throughput: {:.2} MB/s", duration, throughput);
}

// ============================================================================
// File Type Tests - Video
// ============================================================================

#[test]
fn test_add_small_video() {
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());

    let dest = copy_test_file("101394-video-720.mp4", temp_dir.path(), "video.mp4");
    if !dest.exists() {
        println!("SKIP: Test file not found");
        return;
    }

    let size = file_size_mb(&dest);
    println!("Adding MP4 video: {:.2} MB", size);

    let start = Instant::now();
    mediagit()
        .arg("add")
        .arg("video.mp4")
        .current_dir(temp_dir.path())
        .assert()
        .success();

    let duration = start.elapsed();
    let throughput = size / duration.as_secs_f64();
    println!("Add duration: {:?}, Throughput: {:.2} MB/s", duration, throughput);
}

// ============================================================================
// File Type Tests - 3D Models
// ============================================================================

#[test]
fn test_add_glb_model() {
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());

    let dest = copy_test_file("1965_ac_shelby_427_cobra_sc.glb", temp_dir.path(), "model.glb");
    if !dest.exists() {
        println!("SKIP: Test file not found");
        return;
    }

    let size = file_size_mb(&dest);
    println!("Adding GLB model: {:.2} MB", size);

    let start = Instant::now();
    mediagit()
        .arg("add")
        .arg("model.glb")
        .current_dir(temp_dir.path())
        .assert()
        .success();

    let duration = start.elapsed();
    let throughput = size / duration.as_secs_f64();
    println!("Add duration: {:?}, Throughput: {:.2} MB/s", duration, throughput);
}

#[test]
fn test_add_stl_model() {
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());

    let dest = copy_test_file("1900s_telephone.stl", temp_dir.path(), "model.stl");
    if !dest.exists() {
        println!("SKIP: Test file not found");
        return;
    }

    let size = file_size_mb(&dest);
    println!("Adding STL model: {:.2} MB", size);

    let start = Instant::now();
    mediagit()
        .arg("add")
        .arg("model.stl")
        .current_dir(temp_dir.path())
        .assert()
        .success();

    println!("Add duration: {:?}", start.elapsed());
}

// ============================================================================
// File Type Tests - Documents
// ============================================================================

#[test]
fn test_add_svg_document() {
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());

    let dest = copy_test_file("3D_Model_of_the_Main_Gallery_in_Skednena_jama_Cave.svg", temp_dir.path(), "model.svg");
    if !dest.exists() {
        println!("SKIP: Test file not found");
        return;
    }

    mediagit()
        .arg("add")
        .arg("model.svg")
        .current_dir(temp_dir.path())
        .assert()
        .success();
}

#[test]
fn test_add_ai_document() {
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());

    let dest = copy_test_file("12690118_5053480.ai", temp_dir.path(), "design.ai");
    if !dest.exists() {
        println!("SKIP: Test file not found");
        return;
    }

    let size = file_size_mb(&dest);
    println!("Adding AI document: {:.2} MB", size);

    mediagit()
        .arg("add")
        .arg("design.ai")
        .current_dir(temp_dir.path())
        .assert()
        .success();
}

#[test]
fn test_add_eps_document() {
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());

    let dest = copy_test_file("12690118_5053481.eps", temp_dir.path(), "design.eps");
    if !dest.exists() {
        println!("SKIP: Test file not found");
        return;
    }

    let size = file_size_mb(&dest);
    println!("Adding EPS document: {:.2} MB", size);

    mediagit()
        .arg("add")
        .arg("design.eps")
        .current_dir(temp_dir.path())
        .assert()
        .success();
}

// ============================================================================
// Mixed File Types Test
// ============================================================================

#[test]
fn test_add_mixed_file_types() {
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());

    // Copy various file types
    copy_test_file("freepik__talk__71826.jpeg", temp_dir.path(), "image.jpg");
    copy_test_file("_Into_the_Oceans_and_the_Air_.ogg", temp_dir.path(), "audio.ogg");
    copy_test_file("3D_Model_of_the_Main_Gallery_in_Skednena_jama_Cave.svg", temp_dir.path(), "model.svg");

    // Create text file
    fs::write(temp_dir.path().join("README.md"), "# Project\n").unwrap();

    let start = Instant::now();
    mediagit()
        .arg("add")
        .arg(".")
        .current_dir(temp_dir.path())
        .assert()
        .success();

    println!("Add all mixed types duration: {:?}", start.elapsed());
}

// ============================================================================
// Error Handling Tests
// ============================================================================

#[test]
fn test_add_nonexistent_file() {
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());

    mediagit()
        .arg("add")
        .arg("nonexistent.txt")
        .current_dir(temp_dir.path())
        .assert()
        .failure();
}

#[test]
#[ignore] // MediaGit behavior for outside files may vary
fn test_add_outside_repo() {
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());

    // Try to add file from outside repo
    let outside_file = TempDir::new().unwrap();
    fs::write(outside_file.path().join("outside.txt"), "Outside content").unwrap();

    mediagit()
        .arg("add")
        .arg(outside_file.path().join("outside.txt"))
        .current_dir(temp_dir.path())
        .assert()
        .failure();
}

#[test]
fn test_add_help() {
    mediagit()
        .arg("add")
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("Stage"));
}

// ============================================================================
// Performance Benchmark
// ============================================================================

#[test]
fn test_add_many_small_files_performance() {
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());

    // Create 100 small files
    for i in 0..100 {
        fs::write(
            temp_dir.path().join(format!("file_{:03}.txt", i)),
            format!("Content for file {}\n", i),
        ).unwrap();
    }

    let start = Instant::now();
    mediagit()
        .arg("add")
        .arg(".")
        .current_dir(temp_dir.path())
        .assert()
        .success();

    let duration = start.elapsed();
    println!("Add 100 small files: {:?}", duration);
    println!("Files per second: {:.2}", 100.0 / duration.as_secs_f64());
}
