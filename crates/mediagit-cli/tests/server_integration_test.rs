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

//! Server Integration Tests
//!
//! Tests for push, pull, fetch, and clone operations with a MediaGit server.
//! Requires Docker with MinIO running: `docker compose -f docker-compose.test.yml up -d`
//!
//! Run with: `cargo test --test server_integration_test -- --ignored`

use assert_cmd::Command;
use predicates::prelude::*;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Child, Stdio};
use std::time::{Duration, Instant};
use std::thread;
use tempfile::TempDir;

#[cfg(windows)]
const TEST_FILES_DIR: &str = "D:\\own\\saas\\mediagit-core\\test-files";
#[cfg(not(windows))]
const TEST_FILES_DIR: &str = "/mnt/d/own/saas/mediagit-core/test-files";

const SERVER_PORT: u16 = 3000;
const SERVER_HOST: &str = "127.0.0.1";

#[allow(deprecated)]
fn mediagit() -> Command {
    Command::cargo_bin("mediagit").unwrap()
}

#[allow(dead_code, deprecated)]
fn mediagit_server() -> Command {
    Command::cargo_bin("mediagit-server").unwrap()
}

fn init_repo(dir: &Path) {
    mediagit().arg("init").arg("-q").current_dir(dir).assert().success();
}

fn add_and_commit(dir: &Path, name: &str, content: &str, message: &str) {
    fs::write(dir.join(name), content).unwrap();
    mediagit().arg("add").arg(name).current_dir(dir).assert().success();
    mediagit().arg("commit").arg("-m").arg(message).current_dir(dir).assert().success();
}

fn copy_test_file(test_file: &str, repo_dir: &Path, dest_name: &str) -> PathBuf {
    let source = Path::new(TEST_FILES_DIR).join(test_file);
    let dest = repo_dir.join(dest_name);
    if source.exists() {
        fs::copy(&source, &dest).ok();
    }
    dest
}

fn server_url(repo_name: &str) -> String {
    format!("http://{}:{}/{}", SERVER_HOST, SERVER_PORT, repo_name)
}

/// Start a MediaGit server for testing
#[allow(dead_code)]
fn start_test_server(repos_dir: &Path) -> Option<Child> {
    let config_content = format!(
        r#"port = {}
host = "{}"
repos_dir = "{}"
"#,
        SERVER_PORT,
        SERVER_HOST,
        repos_dir.to_string_lossy().replace("\\", "/")
    );

    let config_path = repos_dir.join("test-server.toml");
    fs::write(&config_path, config_content).ok()?;

    let child = std::process::Command::new("cargo")
        .args(["run", "--release", "--bin", "mediagit-server", "--", "-c", &config_path.to_string_lossy()])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .ok()?;

    // Wait for server to start
    thread::sleep(Duration::from_secs(3));

    Some(child)
}

// ============================================================================
// Remote Configuration Tests
// ============================================================================

#[test]
fn test_remote_add_origin() {
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());

    add_and_commit(temp_dir.path(), "file.txt", "Content", "Initial commit");

    mediagit()
        .arg("remote")
        .arg("add")
        .arg("origin")
        .arg(&server_url("test-repo"))
        .current_dir(temp_dir.path())
        .assert()
        .success();

    // Verify remote was added
    mediagit()
        .arg("remote")
        .arg("list")
        .arg("-v")
        .current_dir(temp_dir.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("origin"))
        .stdout(predicate::str::contains(&server_url("test-repo")));
}

// ============================================================================
// Push Command Tests (without actual server)
// ============================================================================

#[test]
fn test_push_help() {
    mediagit()
        .arg("push")
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("push"));
}

#[test]
fn test_push_no_remote() {
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());

    add_and_commit(temp_dir.path(), "file.txt", "Content", "Initial commit");

    // Push without remote should fail
    mediagit()
        .arg("push")
        .current_dir(temp_dir.path())
        .assert()
        .failure();
}

#[test]
fn test_push_dry_run() {
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());

    add_and_commit(temp_dir.path(), "file.txt", "Content", "Initial commit");

    mediagit()
        .arg("remote")
        .arg("add")
        .arg("origin")
        .arg(&server_url("test-repo"))
        .current_dir(temp_dir.path())
        .assert()
        .success();

    // Dry run should not actually connect
    mediagit()
        .arg("push")
        .arg("--dry-run")
        .current_dir(temp_dir.path())
        .assert()
        .failure(); // Will fail because server isn't running
}

// ============================================================================
// Pull Command Tests (without actual server)
// ============================================================================

#[test]
fn test_pull_help() {
    mediagit()
        .arg("pull")
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("pull"));
}

#[test]
fn test_pull_no_remote() {
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());

    add_and_commit(temp_dir.path(), "file.txt", "Content", "Initial commit");

    mediagit()
        .arg("pull")
        .current_dir(temp_dir.path())
        .assert()
        .failure();
}

// ============================================================================
// Fetch Command Tests (without actual server)
// ============================================================================

#[test]
fn test_fetch_help() {
    mediagit()
        .arg("fetch")
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("fetch"));
}

#[test]
fn test_fetch_no_remote() {
    let temp_dir = TempDir::new().unwrap();
    init_repo(temp_dir.path());

    add_and_commit(temp_dir.path(), "file.txt", "Content", "Initial commit");

    mediagit()
        .arg("fetch")
        .current_dir(temp_dir.path())
        .assert()
        .failure();
}

// ============================================================================
// Clone Command Tests (without actual server)
// ============================================================================

#[test]
fn test_clone_help() {
    mediagit()
        .arg("clone")
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("clone"));
}

#[test]
fn test_clone_invalid_url() {
    let temp_dir = TempDir::new().unwrap();

    mediagit()
        .arg("clone")
        .arg("http://nonexistent-server:9999/repo")
        .current_dir(temp_dir.path())
        .assert()
        .failure();
}

// ============================================================================
// Server Integration Tests (require running server)
// These tests are ignored by default
// ============================================================================

#[test]
#[ignore]
fn test_push_to_server() {
    let temp_dir = TempDir::new().unwrap();
    let server_repos = TempDir::new().unwrap();

    // Initialize bare repo on "server"
    let bare_repo = server_repos.path().join("test-repo.git");
    fs::create_dir_all(&bare_repo).unwrap();
    mediagit()
        .arg("init")
        .arg("--bare")
        .current_dir(&bare_repo)
        .assert()
        .success();

    // Initialize client repo
    init_repo(temp_dir.path());
    add_and_commit(temp_dir.path(), "file.txt", "Test content", "Initial commit");

    // Add remote (using file:// protocol for testing without server)
    let remote_url = format!("file://{}", bare_repo.to_string_lossy().replace("\\", "/"));
    mediagit()
        .arg("remote")
        .arg("add")
        .arg("origin")
        .arg(&remote_url)
        .current_dir(temp_dir.path())
        .assert()
        .success();

    // Push
    mediagit()
        .arg("push")
        .arg("-u")
        .arg("origin")
        .arg("main")
        .current_dir(temp_dir.path())
        .assert()
        .success();
}

#[test]
#[ignore]
fn test_clone_from_server() {
    let server_repos = TempDir::new().unwrap();
    let clone_dir = TempDir::new().unwrap();

    // Create source repo
    let source_repo = server_repos.path().join("source-repo");
    fs::create_dir_all(&source_repo).unwrap();
    init_repo(&source_repo);
    add_and_commit(&source_repo, "README.md", "# Test Repository\n", "Initial commit");
    add_and_commit(&source_repo, "code.txt", "Some code content", "Add code");

    // Clone
    let remote_url = format!("file://{}", source_repo.to_string_lossy().replace("\\", "/"));
    mediagit()
        .arg("clone")
        .arg(&remote_url)
        .arg("cloned-repo")
        .current_dir(clone_dir.path())
        .assert()
        .success();

    // Verify clone
    let cloned_repo = clone_dir.path().join("cloned-repo");
    assert!(cloned_repo.join("README.md").exists());
    assert!(cloned_repo.join("code.txt").exists());
}

#[test]
#[ignore]
fn test_push_with_media_files() {
    let temp_dir = TempDir::new().unwrap();
    let server_repos = TempDir::new().unwrap();

    // Initialize bare repo
    let bare_repo = server_repos.path().join("media-repo.git");
    fs::create_dir_all(&bare_repo).unwrap();
    mediagit().arg("init").arg("--bare").current_dir(&bare_repo).assert().success();

    // Initialize client repo with media files
    init_repo(temp_dir.path());
    
    // Copy a test image
    let dest = copy_test_file("freepik__talk__71826.jpeg", temp_dir.path(), "image.jpg");
    if dest.exists() {
        mediagit().arg("add").arg("image.jpg").current_dir(temp_dir.path()).assert().success();
        mediagit().arg("commit").arg("-m").arg("Add image").current_dir(temp_dir.path()).assert().success();
    }

    add_and_commit(temp_dir.path(), "README.md", "# Media Project\n", "Add readme");

    // Add remote and push
    let remote_url = format!("file://{}", bare_repo.to_string_lossy().replace("\\", "/"));
    mediagit().arg("remote").arg("add").arg("origin").arg(&remote_url).current_dir(temp_dir.path()).assert().success();

    let start = Instant::now();
    mediagit()
        .arg("push")
        .arg("-u")
        .arg("origin")
        .arg("main")
        .current_dir(temp_dir.path())
        .assert()
        .success();
    println!("Push duration: {:?}", start.elapsed());
}

#[test]
#[ignore]
fn test_fetch_updates() {
    let server_repos = TempDir::new().unwrap();
    let client1_dir = TempDir::new().unwrap();
    let client2_dir = TempDir::new().unwrap();

    // Create shared bare repo
    let bare_repo = server_repos.path().join("shared-repo.git");
    fs::create_dir_all(&bare_repo).unwrap();
    mediagit().arg("init").arg("--bare").current_dir(&bare_repo).assert().success();

    let remote_url = format!("file://{}", bare_repo.to_string_lossy().replace("\\", "/"));

    // Client 1: Initialize and push
    init_repo(client1_dir.path());
    add_and_commit(client1_dir.path(), "file1.txt", "Content 1", "Client 1 commit");
    mediagit().arg("remote").arg("add").arg("origin").arg(&remote_url).current_dir(client1_dir.path()).assert().success();
    mediagit().arg("push").arg("-u").arg("origin").arg("main").current_dir(client1_dir.path()).assert().success();

    // Client 2: Clone
    mediagit()
        .arg("clone")
        .arg(&remote_url)
        .arg("repo")
        .current_dir(client2_dir.path())
        .assert()
        .success();

    // Client 1: Make more commits
    add_and_commit(client1_dir.path(), "file2.txt", "Content 2", "Client 1 second commit");
    mediagit().arg("push").current_dir(client1_dir.path()).assert().success();

    // Client 2: Fetch updates
    let client2_repo = client2_dir.path().join("repo");
    mediagit()
        .arg("fetch")
        .current_dir(&client2_repo)
        .assert()
        .success();
}

#[test]
#[ignore]
fn test_pull_updates() {
    let server_repos = TempDir::new().unwrap();
    let client1_dir = TempDir::new().unwrap();
    let client2_dir = TempDir::new().unwrap();

    // Create shared bare repo
    let bare_repo = server_repos.path().join("pull-test-repo.git");
    fs::create_dir_all(&bare_repo).unwrap();
    mediagit().arg("init").arg("--bare").current_dir(&bare_repo).assert().success();

    let remote_url = format!("file://{}", bare_repo.to_string_lossy().replace("\\", "/"));

    // Client 1: Initialize and push
    init_repo(client1_dir.path());
    add_and_commit(client1_dir.path(), "initial.txt", "Initial content", "Initial commit");
    mediagit().arg("remote").arg("add").arg("origin").arg(&remote_url).current_dir(client1_dir.path()).assert().success();
    mediagit().arg("push").arg("-u").arg("origin").arg("main").current_dir(client1_dir.path()).assert().success();

    // Client 2: Clone
    mediagit()
        .arg("clone")
        .arg(&remote_url)
        .arg("repo")
        .current_dir(client2_dir.path())
        .assert()
        .success();

    // Client 1: Add more content
    add_and_commit(client1_dir.path(), "update.txt", "Updated content", "Update commit");
    mediagit().arg("push").current_dir(client1_dir.path()).assert().success();

    // Client 2: Pull updates
    let client2_repo = client2_dir.path().join("repo");
    mediagit()
        .arg("pull")
        .current_dir(&client2_repo)
        .assert()
        .success();

    // Verify update was pulled
    assert!(client2_repo.join("update.txt").exists());
}

// ============================================================================
// Large File Push/Pull Tests
// ============================================================================

#[test]
#[ignore]
fn test_push_large_file() {
    let temp_dir = TempDir::new().unwrap();
    let server_repos = TempDir::new().unwrap();

    let bare_repo = server_repos.path().join("large-file-repo.git");
    fs::create_dir_all(&bare_repo).unwrap();
    mediagit().arg("init").arg("--bare").current_dir(&bare_repo).assert().success();

    init_repo(temp_dir.path());

    // Copy a larger test file
    let dest = copy_test_file("1965_ac_shelby_427_cobra_sc.glb", temp_dir.path(), "model.glb");
    if !dest.exists() {
        println!("SKIP: Large test file not found");
        return;
    }

    let file_size = fs::metadata(&dest).map(|m| m.len()).unwrap_or(0);
    println!("Testing push with {:.2} MB file", file_size as f64 / 1024.0 / 1024.0);

    mediagit().arg("add").arg("model.glb").current_dir(temp_dir.path()).assert().success();
    mediagit().arg("commit").arg("-m").arg("Add large model").current_dir(temp_dir.path()).assert().success();

    let remote_url = format!("file://{}", bare_repo.to_string_lossy().replace("\\", "/"));
    mediagit().arg("remote").arg("add").arg("origin").arg(&remote_url).current_dir(temp_dir.path()).assert().success();

    let start = Instant::now();
    mediagit()
        .arg("push")
        .arg("-u")
        .arg("origin")
        .arg("main")
        .current_dir(temp_dir.path())
        .assert()
        .success();

    let duration = start.elapsed();
    let throughput = (file_size as f64 / 1024.0 / 1024.0) / duration.as_secs_f64();
    println!("Push duration: {:?}, Throughput: {:.2} MB/s", duration, throughput);
}

// ============================================================================
// Multi-branch Sync Tests
// ============================================================================

#[test]
#[ignore]
fn test_push_multiple_branches() {
    let temp_dir = TempDir::new().unwrap();
    let server_repos = TempDir::new().unwrap();

    let bare_repo = server_repos.path().join("multi-branch-repo.git");
    fs::create_dir_all(&bare_repo).unwrap();
    mediagit().arg("init").arg("--bare").current_dir(&bare_repo).assert().success();

    init_repo(temp_dir.path());
    add_and_commit(temp_dir.path(), "main.txt", "Main content", "Main commit");

    // Create feature branches
    mediagit().arg("branch").arg("create").arg("feature-1").current_dir(temp_dir.path()).assert().success();
    mediagit().arg("branch").arg("switch").arg("feature-1").current_dir(temp_dir.path()).assert().success();
    add_and_commit(temp_dir.path(), "feature1.txt", "Feature 1", "Feature 1 commit");

    mediagit().arg("branch").arg("switch").arg("refs/heads/main").current_dir(temp_dir.path()).assert().success();
    mediagit().arg("branch").arg("create").arg("feature-2").current_dir(temp_dir.path()).assert().success();
    mediagit().arg("branch").arg("switch").arg("feature-2").current_dir(temp_dir.path()).assert().success();
    add_and_commit(temp_dir.path(), "feature2.txt", "Feature 2", "Feature 2 commit");

    let remote_url = format!("file://{}", bare_repo.to_string_lossy().replace("\\", "/"));
    mediagit().arg("remote").arg("add").arg("origin").arg(&remote_url).current_dir(temp_dir.path()).assert().success();

    // Push all branches
    mediagit()
        .arg("push")
        .arg("--all")
        .current_dir(temp_dir.path())
        .assert()
        .success();
}
