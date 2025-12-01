//! Tests for the `init` command
//!
//! Tests repository initialization with various options and error conditions.

use assert_cmd::Command;
use predicates::prelude::*;
use tempfile::TempDir;

#[test]
fn test_init_default_path() {
    let temp_dir = TempDir::new().unwrap();
    let mut cmd = Command::cargo_bin("mediagit").unwrap();

    cmd.arg("init")
       .current_dir(temp_dir.path())
       .assert()
       .failure() // Expected to fail with "not yet implemented"
       .stderr(predicate::str::contains("not yet implemented"));
}

#[test]
fn test_init_with_explicit_path() {
    let temp_dir = TempDir::new().unwrap();
    let repo_path = temp_dir.path().join("my-repo");

    let mut cmd = Command::cargo_bin("mediagit").unwrap();
    cmd.arg("init")
       .arg(repo_path.to_str().unwrap())
       .assert()
       .failure()
       .stderr(predicate::str::contains("not yet implemented"));
}

#[test]
fn test_init_bare_repository() {
    let temp_dir = TempDir::new().unwrap();
    let mut cmd = Command::cargo_bin("mediagit").unwrap();

    cmd.arg("init")
       .arg("--bare")
       .current_dir(temp_dir.path())
       .assert()
       .failure()
       .stderr(predicate::str::contains("not yet implemented"));
}

#[test]
fn test_init_custom_initial_branch() {
    let temp_dir = TempDir::new().unwrap();
    let mut cmd = Command::cargo_bin("mediagit").unwrap();

    cmd.arg("init")
       .arg("--initial-branch")
       .arg("develop")
       .current_dir(temp_dir.path())
       .assert()
       .failure()
       .stderr(predicate::str::contains("not yet implemented"));
}

#[test]
fn test_init_with_template() {
    let temp_dir = TempDir::new().unwrap();
    let template_dir = TempDir::new().unwrap();

    let mut cmd = Command::cargo_bin("mediagit").unwrap();
    cmd.arg("init")
       .arg("--template")
       .arg(template_dir.path().to_str().unwrap())
       .current_dir(temp_dir.path())
       .assert()
       .failure()
       .stderr(predicate::str::contains("not yet implemented"));
}

#[test]
fn test_init_quiet_mode() {
    let temp_dir = TempDir::new().unwrap();
    let mut cmd = Command::cargo_bin("mediagit").unwrap();

    cmd.arg("init")
       .arg("--quiet")
       .current_dir(temp_dir.path())
       .assert()
       .failure()
       .stderr(predicate::str::contains("not yet implemented"));
}

#[test]
fn test_init_help_output() {
    let mut cmd = Command::cargo_bin("mediagit").unwrap();

    cmd.arg("init")
       .arg("--help")
       .assert()
       .success()
       .stdout(predicate::str::contains("Initialize a new MediaGit repository"));
}

#[test]
fn test_init_combined_options() {
    let temp_dir = TempDir::new().unwrap();
    let mut cmd = Command::cargo_bin("mediagit").unwrap();

    cmd.arg("init")
       .arg("--bare")
       .arg("--initial-branch")
       .arg("main")
       .arg("--quiet")
       .current_dir(temp_dir.path())
       .assert()
       .failure()
       .stderr(predicate::str::contains("not yet implemented"));
}

#[test]
fn test_init_invalid_path() {
    let mut cmd = Command::cargo_bin("mediagit").unwrap();

    // Test with a path that contains invalid characters (platform-specific)
    cmd.arg("init")
       .arg("/invalid\0path")
       .assert()
       .failure();
}

#[test]
fn test_init_readonly_directory() {
    // This test is platform-specific and may need adjustment
    #[cfg(unix)]
    {
        use std::fs;
        use std::os::unix::fs::PermissionsExt;

        let temp_dir = TempDir::new().unwrap();
        let readonly_dir = temp_dir.path().join("readonly");
        fs::create_dir(&readonly_dir).unwrap();

        // Set directory to read-only
        let mut perms = fs::metadata(&readonly_dir).unwrap().permissions();
        perms.set_mode(0o444);
        fs::set_permissions(&readonly_dir, perms).unwrap();

        let mut cmd = Command::cargo_bin("mediagit").unwrap();
        cmd.arg("init")
           .current_dir(&readonly_dir)
           .assert()
           .failure();

        // Restore permissions for cleanup
        let mut perms = fs::metadata(&readonly_dir).unwrap().permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&readonly_dir, perms).unwrap();
    }
}
