// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2025 MediaGit Contributors

//! Performance Benchmark Tests
//!
//! Comprehensive performance benchmarks for MediaGit operations.
//! Measures throughput, timing, and efficiency metrics.
//!
//! Run with: `cargo test --test performance_benchmark_test -- --ignored --nocapture`

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
        fs::copy(&source, &dest).ok();
    }
    dest
}

fn file_size_mb(path: &Path) -> f64 {
    fs::metadata(path).map(|m| m.len() as f64 / 1024.0 / 1024.0).unwrap_or(0.0)
}

// ============================================================================
// Initialization Benchmarks
// ============================================================================

#[test]
#[ignore]
fn benchmark_init() {
    println!("\n=== INIT BENCHMARK ===\n");

    let mut times = Vec::new();

    for i in 0..10 {
        let temp_dir = TempDir::new().unwrap();
        let start = Instant::now();
        mediagit().arg("init").arg("-q").current_dir(temp_dir.path()).assert().success();
        let duration = start.elapsed();
        times.push(duration.as_micros());
        println!("Run {}: {} µs", i + 1, duration.as_micros());
    }

    let avg = times.iter().sum::<u128>() / times.len() as u128;
    let min = *times.iter().min().unwrap();
    let max = *times.iter().max().unwrap();

    println!("\nSummary:");
    println!("  Min: {} µs", min);
    println!("  Max: {} µs", max);
    println!("  Avg: {} µs ({:.3} ms)", avg, avg as f64 / 1000.0);
}

// ============================================================================
// Add Benchmarks
// ============================================================================

#[test]
#[ignore]
fn benchmark_add_small_files() {
    println!("\n=== ADD SMALL FILES BENCHMARK ===\n");

    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());

    // Create 100 small files
    for i in 0..100 {
        fs::write(
            temp_dir.path().join(format!("file_{:03}.txt", i)),
            format!("Content for file number {}\n", i),
        ).unwrap();
    }

    let total_size: u64 = fs::read_dir(temp_dir.path())
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().map_or(false, |ext| ext == "txt"))
        .filter_map(|e| e.metadata().ok())
        .map(|m| m.len())
        .sum();

    println!("Total files: 100");
    println!("Total size: {} bytes", total_size);

    let start = Instant::now();
    mediagit().arg("add").arg(".").current_dir(temp_dir.path()).assert().success();
    let duration = start.elapsed();

    println!("\nAdd duration: {:?}", duration);
    println!("Files per second: {:.2}", 100.0 / duration.as_secs_f64());
    println!("Throughput: {:.2} KB/s", (total_size as f64 / 1024.0) / duration.as_secs_f64());
}

#[test]
#[ignore]
fn benchmark_add_medium_files() {
    println!("\n=== ADD MEDIUM FILES BENCHMARK ===\n");

    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());

    // Create 10 medium files (~1MB each)
    for i in 0..10 {
        let content = "X".repeat(1024 * 1024); // 1MB
        fs::write(
            temp_dir.path().join(format!("medium_{:02}.bin", i)),
            content.as_bytes(),
        ).unwrap();
    }

    let start = Instant::now();
    mediagit().arg("add").arg(".").current_dir(temp_dir.path()).assert().success();
    let duration = start.elapsed();

    println!("10 x 1MB files");
    println!("Add duration: {:?}", duration);
    println!("Throughput: {:.2} MB/s", 10.0 / duration.as_secs_f64());
}

#[test]
#[ignore]
fn benchmark_add_large_file() {
    println!("\n=== ADD LARGE FILE BENCHMARK ===\n");

    let test_files = vec![
        ("101394-video-720.mp4", "5MB video"),
        ("1965_ac_shelby_427_cobra_sc.glb", "13MB 3D model"),
        ("_Amir_Tangsiri__Dokhtare_Koli.flac", "39MB audio"),
        ("_Quando_le_sere_al_placido__(Ferruccio_Giannini).wav", "57MB audio"),
    ];

    for (file, desc) in test_files {
        let temp_dir = TempDir::new().unwrap();
        init_repo(temp_dir.path());

        let dest = copy_test_file(file, temp_dir.path(), "test_file");
        if !dest.exists() {
            println!("{}: SKIPPED (file not found)", desc);
            continue;
        }

        let size = file_size_mb(&dest);
        let start = Instant::now();
        mediagit().arg("add").arg("test_file").current_dir(temp_dir.path()).assert().success();
        let duration = start.elapsed();

        let throughput = size / duration.as_secs_f64();
        println!("{}: {:.2} MB in {:?} = {:.2} MB/s", desc, size, duration, throughput);
    }
}

// ============================================================================
// Commit Benchmarks
// ============================================================================

#[test]
#[ignore]
fn benchmark_commit() {
    println!("\n=== COMMIT BENCHMARK ===\n");

    let mut times = Vec::new();

    for i in 0..10 {
        let temp_dir = TempDir::new().unwrap();
        init_repo(temp_dir.path());

        // Create and add a file
        fs::write(temp_dir.path().join("test.txt"), format!("Content {}\n", i)).unwrap();
        mediagit().arg("add").arg("test.txt").current_dir(temp_dir.path()).assert().success();

        let start = Instant::now();
        mediagit()
            .arg("commit")
            .arg("-m")
            .arg(format!("Commit {}", i))
            .current_dir(temp_dir.path())
            .assert()
            .success();
        let duration = start.elapsed();
        times.push(duration.as_micros());
        println!("Run {}: {} µs", i + 1, duration.as_micros());
    }

    let avg = times.iter().sum::<u128>() / times.len() as u128;
    println!("\nAverage commit time: {} µs ({:.3} ms)", avg, avg as f64 / 1000.0);
}

#[test]
#[ignore]
fn benchmark_commit_many_files() {
    println!("\n=== COMMIT MANY FILES BENCHMARK ===\n");

    let file_counts = vec![10, 50, 100, 200];

    for count in file_counts {
        let temp_dir = TempDir::new().unwrap();
        init_repo(temp_dir.path());

        // Create files
        for i in 0..count {
            fs::write(
                temp_dir.path().join(format!("file_{:04}.txt", i)),
                format!("Content {}\n", i),
            ).unwrap();
        }

        // Add all
        mediagit().arg("add").arg(".").current_dir(temp_dir.path()).assert().success();

        // Commit
        let start = Instant::now();
        mediagit()
            .arg("commit")
            .arg("-m")
            .arg(format!("Add {} files", count))
            .current_dir(temp_dir.path())
            .assert()
            .success();
        let duration = start.elapsed();

        println!("{} files: {:?} ({:.0} files/s)", count, duration, count as f64 / duration.as_secs_f64());
    }
}

// ============================================================================
// Status Benchmarks
// ============================================================================

#[test]
#[ignore]
fn benchmark_status() {
    println!("\n=== STATUS BENCHMARK ===\n");

    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());

    // Create and commit some files
    for i in 0..20 {
        fs::write(temp_dir.path().join(format!("file_{:02}.txt", i)), format!("Content {}\n", i)).unwrap();
    }
    mediagit().arg("add").arg(".").current_dir(temp_dir.path()).assert().success();
    mediagit().arg("commit").arg("-m").arg("Initial").current_dir(temp_dir.path()).assert().success();

    // Modify some files
    for i in 0..5 {
        fs::write(temp_dir.path().join(format!("file_{:02}.txt", i)), "Modified\n").unwrap();
    }

    // Add new untracked files
    for i in 20..25 {
        fs::write(temp_dir.path().join(format!("file_{:02}.txt", i)), "New file\n").unwrap();
    }

    let mut times = Vec::new();
    for _ in 0..10 {
        let start = Instant::now();
        mediagit().arg("status").current_dir(temp_dir.path()).assert().success();
        times.push(start.elapsed().as_micros());
    }

    let avg = times.iter().sum::<u128>() / times.len() as u128;
    println!("Average status time: {} µs ({:.3} ms)", avg, avg as f64 / 1000.0);
}

// ============================================================================
// Log Benchmarks
// ============================================================================

#[test]
#[ignore]
fn benchmark_log() {
    println!("\n=== LOG BENCHMARK ===\n");

    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());

    // Create multiple commits
    for i in 0..50 {
        fs::write(temp_dir.path().join("file.txt"), format!("Version {}\n", i)).unwrap();
        mediagit().arg("add").arg("file.txt").current_dir(temp_dir.path()).assert().success();
        mediagit().arg("commit").arg("-m").arg(format!("Commit {}", i)).current_dir(temp_dir.path()).assert().success();
    }

    // Benchmark log
    let mut times = Vec::new();
    for _ in 0..10 {
        let start = Instant::now();
        mediagit().arg("log").arg("--oneline").current_dir(temp_dir.path()).assert().success();
        times.push(start.elapsed().as_micros());
    }

    let avg = times.iter().sum::<u128>() / times.len() as u128;
    println!("50 commits - Average log time: {} µs ({:.3} ms)", avg, avg as f64 / 1000.0);
}

// ============================================================================
// Branch Benchmarks
// ============================================================================

#[test]
#[ignore]
fn benchmark_branch_switch() {
    println!("\n=== BRANCH SWITCH BENCHMARK ===\n");

    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());

    // Create initial commit
    fs::write(temp_dir.path().join("base.txt"), "Base content\n").unwrap();
    mediagit().arg("add").arg("base.txt").current_dir(temp_dir.path()).assert().success();
    mediagit().arg("commit").arg("-m").arg("Base").current_dir(temp_dir.path()).assert().success();

    // Create branches
    for i in 0..5 {
        mediagit().arg("branch").arg("create").arg(format!("branch-{}", i)).current_dir(temp_dir.path()).assert().success();
    }

    let mut times = Vec::new();
    for i in 0..5 {
        let start = Instant::now();
        mediagit().arg("branch").arg("switch").arg(format!("branch-{}", i)).current_dir(temp_dir.path()).assert().success();
        times.push(start.elapsed().as_micros());
    }

    let avg = times.iter().sum::<u128>() / times.len() as u128;
    println!("Average branch switch time: {} µs ({:.3} ms)", avg, avg as f64 / 1000.0);
}

// ============================================================================
// Maintenance Command Benchmarks
// ============================================================================

#[test]
#[ignore]
fn benchmark_fsck() {
    println!("\n=== FSCK BENCHMARK ===\n");

    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());

    // Create commits
    for i in 0..20 {
        fs::write(temp_dir.path().join(format!("file_{:02}.txt", i)), format!("Content {}\n", i)).unwrap();
        mediagit().arg("add").arg(format!("file_{:02}.txt", i)).current_dir(temp_dir.path()).assert().success();
        mediagit().arg("commit").arg("-m").arg(format!("Commit {}", i)).current_dir(temp_dir.path()).assert().success();
    }

    let mut times = Vec::new();
    for _ in 0..5 {
        let start = Instant::now();
        mediagit().arg("fsck").arg("-q").current_dir(temp_dir.path()).assert().success();
        times.push(start.elapsed().as_millis());
    }

    let avg = times.iter().sum::<u128>() / times.len() as u128;
    println!("20 commits - Average fsck time: {} ms", avg);
}

#[test]
#[ignore]
fn benchmark_gc() {
    println!("\n=== GC BENCHMARK ===\n");

    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());

    // Create commits
    for i in 0..20 {
        fs::write(temp_dir.path().join(format!("file_{:02}.txt", i)), format!("Content {}\n", i)).unwrap();
        mediagit().arg("add").arg(format!("file_{:02}.txt", i)).current_dir(temp_dir.path()).assert().success();
        mediagit().arg("commit").arg("-m").arg(format!("Commit {}", i)).current_dir(temp_dir.path()).assert().success();
    }

    let start = Instant::now();
    mediagit().arg("gc").arg("-q").current_dir(temp_dir.path()).assert().success();
    let duration = start.elapsed();

    println!("20 commits - GC time: {:?}", duration);
}

// ============================================================================
// Comprehensive Performance Summary
// ============================================================================

#[test]
#[ignore]
fn benchmark_summary() {
    println!("\n");
    println!("╔════════════════════════════════════════════════════════════════╗");
    println!("║           MEDIAGIT PERFORMANCE BENCHMARK SUMMARY               ║");
    println!("╠════════════════════════════════════════════════════════════════╣");
    println!("║                                                                ║");

    // Init benchmark
    let mut init_times = Vec::new();
    for _ in 0..5 {
        let temp_dir = TempDir::new().unwrap();
        let start = Instant::now();
        mediagit().arg("init").arg("-q").current_dir(temp_dir.path()).assert().success();
        init_times.push(start.elapsed().as_micros());
    }
    let init_avg = init_times.iter().sum::<u128>() / init_times.len() as u128;
    println!("║  Init:            {:>8} µs  ({:>6.2} ms)                    ║", init_avg, init_avg as f64 / 1000.0);

    // Add benchmark
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());
    for i in 0..10 { fs::write(temp_dir.path().join(format!("f{}.txt", i)), "x").unwrap(); }
    let start = Instant::now();
    mediagit().arg("add").arg(".").current_dir(temp_dir.path()).assert().success();
    let add_time = start.elapsed().as_micros();
    println!("║  Add (10 files):  {:>8} µs  ({:>6.2} ms)                    ║", add_time, add_time as f64 / 1000.0);

    // Commit benchmark
    let start = Instant::now();
    mediagit().arg("commit").arg("-m").arg("Test").current_dir(temp_dir.path()).assert().success();
    let commit_time = start.elapsed().as_micros();
    println!("║  Commit:          {:>8} µs  ({:>6.2} ms)                    ║", commit_time, commit_time as f64 / 1000.0);

    // Status benchmark
    let start = Instant::now();
    mediagit().arg("status").current_dir(temp_dir.path()).assert().success();
    let status_time = start.elapsed().as_micros();
    println!("║  Status:          {:>8} µs  ({:>6.2} ms)                    ║", status_time, status_time as f64 / 1000.0);

    // Log benchmark
    let start = Instant::now();
    mediagit().arg("log").arg("--oneline").current_dir(temp_dir.path()).assert().success();
    let log_time = start.elapsed().as_micros();
    println!("║  Log:             {:>8} µs  ({:>6.2} ms)                    ║", log_time, log_time as f64 / 1000.0);

    println!("║                                                                ║");
    println!("╚════════════════════════════════════════════════════════════════╝");
    println!();
}
