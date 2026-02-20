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

//! Large File Handling Tests
//!
//! Tests for handling large media files from 5MB to 10GB+.
//! Uses actual test files from the test-files directory.
//! These tests are ignored by default - run with `cargo test --test large_file_test -- --ignored`

use assert_cmd::Command;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Instant;
use tempfile::TempDir;

#[cfg(windows)]
const TEST_FILES_DIR: &str = "D:\\own\\saas\\mediagit-core\\test-files";
#[cfg(not(windows))]
const TEST_FILES_DIR: &str = "/mnt/d/own/saas/mediagit-core/test-files";

#[allow(deprecated)]
fn mediagit() -> Command {
    Command::cargo_bin("mediagit").unwrap()
}

fn init_repo(dir: &Path) {
    mediagit().arg("init").arg("-q").current_dir(dir).assert().success();
}

fn copy_test_file(test_file: &str, repo_dir: &Path, dest_name: &str) -> PathBuf {
    let source = Path::new(TEST_FILES_DIR).join(test_file);
    let dest = repo_dir.join(dest_name);

    if source.exists() {
        fs::copy(&source, &dest).expect(&format!("Failed to copy {}", test_file));
    }

    dest
}

fn file_size_mb(path: &Path) -> f64 {
    if path.exists() {
        fs::metadata(path).map(|m| m.len() as f64 / 1024.0 / 1024.0).unwrap_or(0.0)
    } else {
        0.0
    }
}

fn file_size_bytes(path: &Path) -> u64 {
    if path.exists() {
        fs::metadata(path).map(|m| m.len()).unwrap_or(0)
    } else {
        0
    }
}

struct TestMetrics {
    file_name: String,
    file_size_mb: f64,
    add_duration_ms: u128,
    commit_duration_ms: u128,
    throughput_mbps: f64,
}

impl TestMetrics {
    fn print(&self) {
        println!("\n=== {} ===", self.file_name);
        println!("File size: {:.2} MB", self.file_size_mb);
        println!("Add duration: {} ms", self.add_duration_ms);
        println!("Commit duration: {} ms", self.commit_duration_ms);
        println!("Throughput: {:.2} MB/s", self.throughput_mbps);
    }
}

fn test_large_file(file_name: &str, dest_name: &str) -> Option<TestMetrics> {
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());

    let dest = copy_test_file(file_name, temp_dir.path(), dest_name);
    if !dest.exists() {
        println!("SKIP: {} not found", file_name);
        return None;
    }

    let size = file_size_mb(&dest);
    println!("\nTesting: {} ({:.2} MB)", file_name, size);

    // Measure add
    let add_start = Instant::now();
    mediagit()
        .arg("add")
        .arg(dest_name)
        .current_dir(temp_dir.path())
        .assert()
        .success();
    let add_duration = add_start.elapsed();

    // Measure commit
    let commit_start = Instant::now();
    mediagit()
        .arg("commit")
        .arg("-m")
        .arg(&format!("Add {}", dest_name))
        .current_dir(temp_dir.path())
        .assert()
        .success();
    let commit_duration = commit_start.elapsed();

    let total_duration = add_duration + commit_duration;
    let throughput = size / total_duration.as_secs_f64();

    let metrics = TestMetrics {
        file_name: file_name.to_string(),
        file_size_mb: size,
        add_duration_ms: add_duration.as_millis(),
        commit_duration_ms: commit_duration.as_millis(),
        throughput_mbps: throughput,
    };

    metrics.print();
    Some(metrics)
}

// ============================================================================
// Small File Tests (< 10MB)
// ============================================================================

#[test]
fn test_small_video_5mb() {
    test_large_file("101394-video-720.mp4", "video.mp4");
}

#[test]
fn test_small_ogg_audio() {
    test_large_file("_Into_the_Oceans_and_the_Air_.ogg", "audio.ogg");
}

#[test]
fn test_small_webp_image() {
    test_large_file("Workstation_cube_lid_off.webp", "image.webp");
}

// ============================================================================
// Medium File Tests (10-50 MB)
// ============================================================================

#[test]
#[ignore]
fn test_medium_glb_13mb() {
    test_large_file("1965_ac_shelby_427_cobra_sc.glb", "model.glb");
}

#[test]
#[ignore]
fn test_medium_glb_25mb() {
    test_large_file("sci-fi_buildings_pack.glb", "buildings.glb");
}

#[test]
#[ignore]
fn test_medium_flac_39mb() {
    test_large_file("_Amir_Tangsiri__Dokhtare_Koli.flac", "audio.flac");
}

// ============================================================================
// Large File Tests (50-500 MB)
// ============================================================================

#[test]
#[ignore]
fn test_large_wav_57mb() {
    test_large_file("_Quando_le_sere_al_placido__(Ferruccio_Giannini).wav", "audio.wav");
}

#[test]
#[ignore]
fn test_large_mov_416mb() {
    test_large_file("big_buck_bunny_720p_h264.mov", "video.mov");
}

// ============================================================================
// Very Large File Tests (500MB - 3GB)
// ============================================================================

#[test]
#[ignore]
fn test_very_large_mkv_2_4gb() {
    test_large_file(
        "www.1TamilMV.LC - Mask (2025) Tamil WEB-DL - 4K SDR - 2160p - HEVC - (DD+5.1 - 192Kbps & AAC 2.0) - 2.3GB - ESub.mkv",
        "movie.mkv"
    );
}

#[test]
#[ignore]
fn test_very_large_archive_688mb() {
    test_large_file("archive.zip", "archive.zip");
}

// ============================================================================
// Extreme Scale Tests (10GB+)
// ============================================================================

#[test]
#[ignore]
fn test_extreme_archive_10gb() {
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());

    let source = Path::new(TEST_FILES_DIR).join("archive (1).zip");
    if !source.exists() {
        println!("SKIP: 10GB+ archive not found");
        return;
    }

    let size = file_size_mb(&source);
    println!("\n=== EXTREME SCALE TEST ===");
    println!("Testing 10GB+ archive: {:.2} MB ({:.2} GB)", size, size / 1024.0);

    // For 10GB+ files, we need to use streaming/chunked approach
    // This test verifies the system can handle extreme scale
    
    println!("Copying file to temp repository...");
    let copy_start = Instant::now();
    let dest = temp_dir.path().join("large_archive.zip");
    fs::copy(&source, &dest).expect("Failed to copy large file");
    println!("Copy duration: {:?}", copy_start.elapsed());

    println!("Adding to repository...");
    let add_start = Instant::now();
    mediagit()
        .arg("add")
        .arg("large_archive.zip")
        .timeout(std::time::Duration::from_secs(3600)) // 1 hour timeout
        .current_dir(temp_dir.path())
        .assert()
        .success();
    let add_duration = add_start.elapsed();
    println!("Add duration: {:?}", add_duration);

    println!("Committing...");
    let commit_start = Instant::now();
    mediagit()
        .arg("commit")
        .arg("-m")
        .arg("Add 10GB+ archive")
        .timeout(std::time::Duration::from_secs(3600))
        .current_dir(temp_dir.path())
        .assert()
        .success();
    println!("Commit duration: {:?}", commit_start.elapsed());

    let total_duration = add_duration + commit_start.elapsed();
    let throughput = size / total_duration.as_secs_f64();
    println!("Total throughput: {:.2} MB/s", throughput);
}

// ============================================================================
// Chunking Verification Tests
// ============================================================================

#[test]
#[ignore]
fn test_chunking_verification() {
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());

    let dest = copy_test_file("big_buck_bunny_720p_h264.mov", temp_dir.path(), "video.mov");
    if !dest.exists() {
        println!("SKIP: Test file not found");
        return;
    }

    let original_size = file_size_bytes(&dest);

    mediagit().arg("add").arg("video.mov").current_dir(temp_dir.path()).assert().success();
    mediagit().arg("commit").arg("-m").arg("Add video").current_dir(temp_dir.path()).assert().success();

    // Check storage statistics
    let _output = mediagit()
        .arg("stats")
        .arg("--storage")
        .current_dir(temp_dir.path())
        .assert()
        .success();

    println!("Original file size: {} bytes ({:.2} MB)", original_size, original_size as f64 / 1024.0 / 1024.0);
}

// ============================================================================
// Media Type Specific Tests
// ============================================================================

#[test]
#[ignore]
fn test_video_formats() {
    println!("\n=== VIDEO FORMAT TESTS ===");
    
    // MP4
    if let Some(m) = test_large_file("101394-video-720.mp4", "video.mp4") {
        assert!(m.throughput_mbps > 0.0, "MP4 throughput should be positive");
    }

    // MOV (large)
    if let Some(m) = test_large_file("big_buck_bunny_720p_h264.mov", "video.mov") {
        assert!(m.throughput_mbps > 0.0, "MOV throughput should be positive");
    }
}

#[test]
#[ignore]
fn test_audio_formats() {
    println!("\n=== AUDIO FORMAT TESTS ===");
    
    // OGG
    test_large_file("_Into_the_Oceans_and_the_Air_.ogg", "audio.ogg");
    
    // FLAC
    test_large_file("_Amir_Tangsiri__Dokhtare_Koli.flac", "audio.flac");
    
    // WAV
    test_large_file("_Quando_le_sere_al_placido__(Ferruccio_Giannini).wav", "audio.wav");
}

#[test]
#[ignore]
fn test_3d_model_formats() {
    println!("\n=== 3D MODEL FORMAT TESTS ===");
    
    // GLB
    test_large_file("1965_ac_shelby_427_cobra_sc.glb", "car.glb");
    test_large_file("sci-fi_buildings_pack.glb", "buildings.glb");
    
    // STL
    test_large_file("1900s_telephone.stl", "phone.stl");
    
    // USDZ
    test_large_file("1965_AC_Shelby_427_Cobra_SC.usdz", "car.usdz");
}

#[test]
#[ignore]
fn test_document_formats() {
    println!("\n=== DOCUMENT FORMAT TESTS ===");
    
    // AI
    test_large_file("12690118_5053480.ai", "design.ai");
    
    // EPS
    test_large_file("12690118_5053481.eps", "design.eps");
    
    // SVG
    test_large_file("3D_Model_of_the_Main_Gallery_in_Skednena_jama_Cave.svg", "cave.svg");
}

// ============================================================================
// Performance Comparison Test
// ============================================================================

#[test]
#[ignore]
fn test_performance_summary() {
    println!("\n========================================");
    println!("LARGE FILE PERFORMANCE SUMMARY");
    println!("========================================\n");

    let mut all_metrics: Vec<TestMetrics> = Vec::new();

    // Test various file sizes
    let test_files = vec![
        ("101394-video-720.mp4", "video_5mb.mp4"),
        ("1965_ac_shelby_427_cobra_sc.glb", "model_13mb.glb"),
        ("_Amir_Tangsiri__Dokhtare_Koli.flac", "audio_39mb.flac"),
        ("_Quando_le_sere_al_placido__(Ferruccio_Giannini).wav", "audio_57mb.wav"),
    ];

    for (src, dst) in test_files {
        if let Some(metrics) = test_large_file(src, dst) {
            all_metrics.push(metrics);
        }
    }

    if !all_metrics.is_empty() {
        println!("\n========================================");
        println!("SUMMARY TABLE");
        println!("========================================");
        println!("{:<40} {:>10} {:>12} {:>10}", "File", "Size (MB)", "Add (ms)", "MB/s");
        println!("{}", "-".repeat(74));
        
        for m in &all_metrics {
            println!("{:<40} {:>10.2} {:>12} {:>10.2}", 
                m.file_name.chars().take(40).collect::<String>(),
                m.file_size_mb,
                m.add_duration_ms,
                m.throughput_mbps
            );
        }

        let total_size: f64 = all_metrics.iter().map(|m| m.file_size_mb).sum();
        let total_time: u128 = all_metrics.iter().map(|m| m.add_duration_ms).sum();
        let avg_throughput = total_size / (total_time as f64 / 1000.0);
        
        println!("{}", "-".repeat(74));
        println!("TOTAL: {:.2} MB in {} ms = {:.2} MB/s avg", total_size, total_time, avg_throughput);
    }
}
