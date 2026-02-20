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
// SPDX-License-Identifier: AGPL-3.0
// Copyright (C) 2025 MediaGit Contributors

//! Integration tests for Git filter driver

use mediagit_git::{FilterConfig, FilterDriver, PointerFile};
use std::fs;
use std::process::Command;
use tempfile::TempDir;

/// Helper to initialize a Git repository for testing
fn init_git_repo() -> TempDir {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");

    let output = Command::new("git")
        .args(["init"])
        .current_dir(temp_dir.path())
        .output()
        .expect("Failed to init git repo");

    assert!(output.status.success(), "Git init failed");

    // Configure git user for commits
    Command::new("git")
        .args(["config", "user.name", "Test User"])
        .current_dir(temp_dir.path())
        .output()
        .expect("Failed to set git user.name");

    Command::new("git")
        .args(["config", "user.email", "test@example.com"])
        .current_dir(temp_dir.path())
        .output()
        .expect("Failed to set git user.email");

    temp_dir
}

#[test]
fn test_pointer_file_roundtrip() {
    let oid = "4d7a214614ab2935c943f9e0ff69d22eadbb8f32b1258daaa5e2ca24d17e2393";
    let size = 123456789;

    // Create pointer file
    let pointer = PointerFile::new(oid.to_string(), size);
    let text = pointer.to_string();

    // Parse it back
    let parsed = PointerFile::parse(&text).expect("Failed to parse pointer file");

    assert_eq!(pointer, parsed);
    assert_eq!(parsed.oid, oid);
    assert_eq!(parsed.size, size);
}

#[test]
fn test_pointer_file_detection() {
    let valid_pointer = "version https://mediagit.dev/spec/v1\n\
                         oid sha256:4d7a214614ab2935c943f9e0ff69d22eadbb8f32b1258daaa5e2ca24d17e2393\n\
                         size 123456789\n";

    assert!(PointerFile::is_pointer(valid_pointer));

    let not_pointer = "This is just regular file content\n\
                       with multiple lines\n\
                       and no pointer structure";

    assert!(!PointerFile::is_pointer(not_pointer));
}

#[test]
fn test_filter_driver_install() {
    let temp_dir = init_git_repo();
    let config = FilterConfig::default();
    let driver = FilterDriver::new(config).expect("Failed to create filter driver");

    // Install filter driver
    driver
        .install(temp_dir.path())
        .expect("Failed to install filter driver");

    // Verify git config was updated
    let output = Command::new("git")
        .args(["config", "--local", "filter.mediagit.clean"])
        .current_dir(temp_dir.path())
        .output()
        .expect("Failed to get git config");

    assert!(output.status.success());
    let config_value = String::from_utf8_lossy(&output.stdout);
    assert!(config_value.contains("mediagit filter-clean"));
}

#[test]
fn test_track_pattern() {
    let temp_dir = init_git_repo();
    let config = FilterConfig::default();
    let driver = FilterDriver::new(config).expect("Failed to create filter driver");

    // Track a pattern
    driver
        .track_pattern(temp_dir.path(), "*.psd")
        .expect("Failed to track pattern");

    // Verify .gitattributes was created
    let gitattributes_path = temp_dir.path().join(".gitattributes");
    assert!(gitattributes_path.exists());

    // Verify content
    let content = fs::read_to_string(&gitattributes_path).expect("Failed to read .gitattributes");
    assert!(content.contains("*.psd filter=mediagit"));
}

#[test]
fn test_track_multiple_patterns() {
    let temp_dir = init_git_repo();
    let config = FilterConfig::default();
    let driver = FilterDriver::new(config).expect("Failed to create filter driver");

    // Track multiple patterns
    driver
        .track_pattern(temp_dir.path(), "*.psd")
        .expect("Failed to track *.psd");
    driver
        .track_pattern(temp_dir.path(), "*.mp4")
        .expect("Failed to track *.mp4");
    driver
        .track_pattern(temp_dir.path(), "*.wav")
        .expect("Failed to track *.wav");

    // Verify .gitattributes content
    let gitattributes_path = temp_dir.path().join(".gitattributes");
    let content = fs::read_to_string(&gitattributes_path).expect("Failed to read .gitattributes");

    assert!(content.contains("*.psd filter=mediagit"));
    assert!(content.contains("*.mp4 filter=mediagit"));
    assert!(content.contains("*.wav filter=mediagit"));
}

#[test]
fn test_untrack_pattern() {
    let temp_dir = init_git_repo();
    let config = FilterConfig::default();
    let driver = FilterDriver::new(config).expect("Failed to create filter driver");

    // Track and then untrack
    driver
        .track_pattern(temp_dir.path(), "*.psd")
        .expect("Failed to track pattern");
    driver
        .track_pattern(temp_dir.path(), "*.mp4")
        .expect("Failed to track pattern");

    driver
        .untrack_pattern(temp_dir.path(), "*.psd")
        .expect("Failed to untrack pattern");

    // Verify .gitattributes content
    let gitattributes_path = temp_dir.path().join(".gitattributes");
    let content = fs::read_to_string(&gitattributes_path).expect("Failed to read .gitattributes");

    assert!(!content.contains("*.psd filter=mediagit"));
    assert!(content.contains("*.mp4 filter=mediagit"));
}

#[test]
fn test_pointer_file_format_validation() {
    // Test invalid version
    let invalid_version = "version https://wrong.com/spec/v1\n\
                           oid sha256:4d7a214614ab2935c943f9e0ff69d22eadbb8f32b1258daaa5e2ca24d17e2393\n\
                           size 123456789\n";
    assert!(!PointerFile::is_pointer(invalid_version));

    // Test missing oid
    let missing_oid = "version https://mediagit.dev/spec/v1\n\
                       size 123456789\n";
    let result = PointerFile::parse(missing_oid);
    assert!(result.is_err());

    // Test invalid oid format
    let invalid_oid = "version https://mediagit.dev/spec/v1\n\
                       oid md5:invalid\n\
                       size 123456789\n";
    let result = PointerFile::parse(invalid_oid);
    assert!(result.is_err());

    // Test invalid hash length
    let invalid_hash = "version https://mediagit.dev/spec/v1\n\
                        oid sha256:tooshort\n\
                        size 123456789\n";
    let result = PointerFile::parse(invalid_hash);
    assert!(result.is_err());
}

#[test]
fn test_filter_config_defaults() {
    let config = FilterConfig::default();
    assert_eq!(config.min_file_size, 1024 * 1024); // 1 MB
    assert!(config.storage_path.is_none());
    assert!(!config.skip_binary_check);
}

#[test]
fn test_filter_driver_with_custom_config() {
    let config = FilterConfig {
        min_file_size: 5 * 1024 * 1024, // 5 MB
        storage_path: Some("/custom/path".to_string()),
        skip_binary_check: true,
    };

    let driver = FilterDriver::new(config).expect("Failed to create filter driver");
    assert_eq!(driver.config().min_file_size, 5 * 1024 * 1024);
    assert_eq!(driver.config().storage_path, Some("/custom/path".to_string()));
    assert!(driver.config().skip_binary_check);
}

#[test]
fn test_pointer_file_size_limits() {
    // Test that very large "pointer" files are rejected
    let large_content = "version https://mediagit.dev/spec/v1\n".to_string()
        + &"x".repeat(1000);

    assert!(!PointerFile::is_pointer(&large_content));

    let result = PointerFile::parse(&large_content);
    assert!(result.is_err());
}
