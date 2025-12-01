// CLI Integration Tests
// Tests the actual mediagit CLI commands that are implemented

use std::path::PathBuf;
use std::process::Command;
use tempfile::TempDir;

// Helper to get the CLI binary path
fn cli_binary() -> PathBuf {
    let mut path = std::env::current_exe().unwrap();
    path.pop(); // Remove test binary name
    path.pop(); // Remove 'deps' directory
    path.push("mediagit");

    // If mediagit doesn't exist, try mediagit.exe (Windows)
    if !path.exists() {
        path.set_extension("exe");
    }

    path
}

#[test]
fn test_cli_binary_exists() {
    let binary = cli_binary();
    assert!(binary.exists(), "CLI binary should exist at {:?}", binary);
}

#[test]
fn test_cli_help_command() {
    let output = Command::new(cli_binary())
        .arg("--help")
        .output()
        .expect("Failed to execute CLI");

    assert!(output.status.success(), "help should succeed");

    let stdout = String::from_utf8_lossy(&output.stdout);
    // Check for basic CLI structure
    assert!(stdout.contains("MediaGit") || stdout.contains("mediagit") || stdout.contains("Usage"),
            "help should show usage information");
}

#[test]
fn test_cli_version_command() {
    let output = Command::new(cli_binary())
        .arg("--version")
        .output()
        .expect("Failed to execute CLI");

    assert!(output.status.success(), "version should succeed");

    let stdout = String::from_utf8_lossy(&output.stdout);
    // Should contain version number
    assert!(stdout.contains("mediagit") || stdout.contains("0.1"),
            "version should show program name and version");
}

#[test]
fn test_cli_init_command() {
    let temp = TempDir::new().unwrap();
    let repo_path = temp.path().join("test-repo");

    let output = Command::new(cli_binary())
        .arg("init")
        .arg(&repo_path)
        .output()
        .expect("Failed to execute CLI");

    // Check if command succeeded
    if !output.status.success() {
        eprintln!("Init stdout: {}", String::from_utf8_lossy(&output.stdout));
        eprintln!("Init stderr: {}", String::from_utf8_lossy(&output.stderr));
    }

    assert!(output.status.success(), "init command should succeed");

    // Verify .mediagit directory was created
    assert!(repo_path.join(".mediagit").exists(), ".mediagit directory should exist");
    assert!(repo_path.join(".mediagit/objects").exists(), "objects directory should exist");
    assert!(repo_path.join(".mediagit/refs").exists(), "refs directory should exist");
}

#[test]
fn test_cli_init_with_verbose() {
    let temp = TempDir::new().unwrap();
    let repo_path = temp.path().join("test-repo-verbose");

    let output = Command::new(cli_binary())
        .arg("init")
        .arg(&repo_path)
        .arg("--verbose")
        .output()
        .expect("Failed to execute CLI");

    assert!(output.status.success(), "init with --verbose should succeed");

    // Verify repository was created
    assert!(repo_path.join(".mediagit").exists());
}

#[test]
fn test_cli_init_existing_directory() {
    let temp = TempDir::new().unwrap();
    let repo_path = temp.path().join("existing-repo");
    std::fs::create_dir(&repo_path).unwrap();

    let output = Command::new(cli_binary())
        .arg("init")
        .arg(&repo_path)
        .output()
        .expect("Failed to execute CLI");

    // Should succeed even for existing directory
    assert!(output.status.success(), "init on existing directory should succeed");
    assert!(repo_path.join(".mediagit").exists());
}

#[test]
fn test_cli_invalid_command() {
    let output = Command::new(cli_binary())
        .arg("nonexistent-command")
        .output()
        .expect("Failed to execute CLI");

    // Should fail with unrecognized command
    assert!(!output.status.success(), "invalid command should fail");

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("unrecognized") || stderr.contains("error") || stderr.contains("invalid"),
            "should show error for invalid command");
}

#[test]
fn test_cli_no_args() {
    let output = Command::new(cli_binary())
        .output()
        .expect("Failed to execute CLI");

    // CLI with no args might show help or error - either is acceptable
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(
        stdout.contains("Usage") || stdout.contains("mediagit") ||
        stderr.contains("Usage") || stderr.contains("mediagit"),
        "no args should show usage or error"
    );
}

// Note: Tests for push, pull, status, remote commands require:
// 1. remote command implementation (not yet implemented)
// 2. Proper test repository setup with valid refs
// 3. Running test server
// These should be added after those features are fully implemented

// Placeholder test to document future work
#[test]
#[ignore = "Requires remote command implementation"]
fn test_cli_push_pull_integration() {
    // This test requires:
    // - Remote command to configure remotes
    // - Test server running
    // - Proper repository initialization
    // TODO: Implement after remote command is added
}
