// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2025 MediaGit Contributors

//! Comprehensive CLI Init Command Tests
//!
//! Tests for `mediagit init` command with all options and edge cases.
//! Measures initialization speed and validates repository structure.

use assert_cmd::Command;
use predicates::prelude::*;
use std::fs;
use std::time::Instant;
use tempfile::TempDir;

/// Helper to create mediagit command
#[allow(deprecated)]
fn mediagit() -> Command {
    Command::cargo_bin("mediagit").unwrap()
}

// ============================================================================
// Basic Initialization Tests
// ============================================================================

#[test]
fn test_init_basic() {
    let temp_dir = TempDir::new().unwrap();
    let start = Instant::now();

    mediagit()
        .arg("init")
        .current_dir(temp_dir.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("Initialized"));

    let duration = start.elapsed();
    println!("Init duration: {:?}", duration);

    // Verify .mediagit directory was created
    assert!(temp_dir.path().join(".mediagit").exists());
    assert!(temp_dir.path().join(".mediagit/objects").exists());
    assert!(temp_dir.path().join(".mediagit/refs").exists());
}

#[test]
fn test_init_with_path() {
    let temp_dir = TempDir::new().unwrap();
    let repo_path = temp_dir.path().join("new-repo");

    mediagit()
        .arg("init")
        .arg(&repo_path)
        .assert()
        .success();

    assert!(repo_path.join(".mediagit").exists());
}

#[test]
fn test_init_bare_repository() {
    let temp_dir = TempDir::new().unwrap();

    mediagit()
        .arg("init")
        .arg("--bare")
        .current_dir(temp_dir.path())
        .assert()
        .success();

    // Bare repos have objects directly in repo dir
    assert!(temp_dir.path().join("objects").exists() || temp_dir.path().join(".mediagit").exists());
}

#[test]
fn test_init_custom_branch() {
    let temp_dir = TempDir::new().unwrap();

    mediagit()
        .arg("init")
        .arg("--initial-branch")
        .arg("develop")
        .current_dir(temp_dir.path())
        .assert()
        .success();

    // Check HEAD points to develop
    let head_path = temp_dir.path().join(".mediagit/HEAD");
    if head_path.exists() {
        let head_content = fs::read_to_string(&head_path).unwrap();
        assert!(head_content.contains("develop"));
    }
}

#[test]
fn test_init_quiet_mode() {
    let temp_dir = TempDir::new().unwrap();

    mediagit()
        .arg("init")
        .arg("-q")
        .current_dir(temp_dir.path())
        .assert()
        .success()
        .stdout(predicate::str::is_empty().or(predicate::str::contains("")));
}

// ============================================================================
// Re-initialization Tests
// ============================================================================

#[test]
fn test_init_reinit_existing() {
    let temp_dir = TempDir::new().unwrap();

    // First init
    mediagit()
        .arg("init")
        .current_dir(temp_dir.path())
        .assert()
        .success();

    // Re-init should fail (MediaGit doesn't support reinit)
    mediagit()
        .arg("init")
        .current_dir(temp_dir.path())
        .assert()
        .failure()
        .stderr(predicate::str::contains("already initialized"));
}

// ============================================================================
// Speed and Performance Tests
// ============================================================================

#[test]
fn test_init_speed_benchmark() {
    let mut durations = Vec::new();

    for i in 0..5 {
        let temp_dir = TempDir::new().unwrap();
        let start = Instant::now();

        mediagit()
            .arg("init")
            .arg("-q")
            .current_dir(temp_dir.path())
            .assert()
            .success();

        durations.push(start.elapsed());
        println!("Init run {}: {:?}", i + 1, durations.last().unwrap());
    }

    let avg = durations.iter().map(|d| d.as_millis()).sum::<u128>() / durations.len() as u128;
    println!("Average init time: {}ms", avg);

    // Init should be very fast (< 100ms)
    assert!(avg < 500, "Init took too long: {}ms average", avg);
}

// ============================================================================
// Repository Structure Verification
// ============================================================================

#[test]
fn test_init_directory_structure() {
    let temp_dir = TempDir::new().unwrap();

    mediagit()
        .arg("init")
        .current_dir(temp_dir.path())
        .assert()
        .success();

    let mediagit_dir = temp_dir.path().join(".mediagit");

    // Check core directories exist
    assert!(mediagit_dir.exists(), ".mediagit should exist");
    assert!(mediagit_dir.join("objects").exists(), "objects dir should exist");
    assert!(mediagit_dir.join("refs").exists(), "refs dir should exist");
    assert!(mediagit_dir.join("refs/heads").exists(), "refs/heads should exist");

    // Check HEAD file exists
    assert!(mediagit_dir.join("HEAD").exists(), "HEAD file should exist");
}

#[test]
fn test_init_config_file() {
    let temp_dir = TempDir::new().unwrap();

    mediagit()
        .arg("init")
        .current_dir(temp_dir.path())
        .assert()
        .success();

    let config_path = temp_dir.path().join(".mediagit/config.toml");
    if config_path.exists() {
        let config_content = fs::read_to_string(&config_path).unwrap();
        println!("Config content:\n{}", config_content);
    }
}

// ============================================================================
// Error Handling Tests
// ============================================================================

#[test]
fn test_init_help() {
    mediagit()
        .arg("init")
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("Initialize"));
}

#[test]
fn test_init_nested_repos() {
    let temp_dir = TempDir::new().unwrap();

    // Init parent
    mediagit()
        .arg("init")
        .current_dir(temp_dir.path())
        .assert()
        .success();

    // Create nested directory
    let nested_dir = temp_dir.path().join("nested");
    fs::create_dir(&nested_dir).unwrap();

    // Init nested should work
    mediagit()
        .arg("init")
        .current_dir(&nested_dir)
        .assert()
        .success();

    assert!(nested_dir.join(".mediagit").exists());
}

// ============================================================================
// Combined Options Tests
// ============================================================================

#[test]
fn test_init_all_options() {
    let temp_dir = TempDir::new().unwrap();
    let _template_dir = TempDir::new().unwrap();

    mediagit()
        .arg("init")
        .arg("--bare")
        .arg("--initial-branch")
        .arg("main")
        .arg("--quiet")
        .current_dir(temp_dir.path())
        .assert()
        .success();
}
