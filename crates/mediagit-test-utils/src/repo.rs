// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2025 MediaGit Contributors

//! Test repository helper for integration tests.
//!
//! Provides a TestRepo struct that manages temporary directories and common
//! repository operations for testing.

use crate::cli::MediagitCommand;
use std::fs;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

/// A test repository with automatic cleanup.
///
/// Wraps a temporary directory and provides helper methods for common
/// repository operations during testing.
///
/// # Example
/// ```ignore
/// use mediagit_test_utils::TestRepo;
///
/// let repo = TestRepo::initialized();
/// repo.write_file("file.txt", b"content");
/// repo.add(&["file.txt"]);
/// repo.commit("Add file");
/// ```
pub struct TestRepo {
    temp_dir: TempDir,
}

impl TestRepo {
    /// Create a new empty test directory (not initialized as a repo).
    pub fn new() -> Self {
        Self {
            temp_dir: TempDir::new().expect("Failed to create temp directory"),
        }
    }

    /// Create a new test directory and initialize it as a mediagit repository.
    pub fn initialized() -> Self {
        let repo = Self::new();
        MediagitCommand::init_quiet(repo.path());
        repo
    }

    /// Create a new test repository with an initial commit.
    ///
    /// Creates a basic file and commits it to establish a working repository
    /// with at least one commit.
    pub fn with_initial_commit() -> Self {
        let repo = Self::initialized();
        repo.write_file("README.md", b"# Test Repository\n");
        repo.add(&["README.md"]);
        repo.commit("Initial commit");
        repo
    }

    /// Get the path to the repository directory.
    pub fn path(&self) -> &Path {
        self.temp_dir.path()
    }

    /// Get the path to the .mediagit directory.
    pub fn mediagit_dir(&self) -> PathBuf {
        self.temp_dir.path().join(".mediagit")
    }

    /// Write a file to the repository.
    pub fn write_file(&self, name: &str, content: &[u8]) {
        let path = self.temp_dir.path().join(name);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("Failed to create parent directories");
        }
        fs::write(&path, content).expect("Failed to write file");
    }

    /// Write a text file to the repository.
    pub fn write_text_file(&self, name: &str, content: &str) {
        self.write_file(name, content.as_bytes());
    }

    /// Read a file from the repository.
    pub fn read_file(&self, name: &str) -> Vec<u8> {
        fs::read(self.temp_dir.path().join(name)).expect("Failed to read file")
    }

    /// Read a text file from the repository.
    pub fn read_text_file(&self, name: &str) -> String {
        fs::read_to_string(self.temp_dir.path().join(name)).expect("Failed to read text file")
    }

    /// Check if a file exists in the repository.
    pub fn file_exists(&self, name: &str) -> bool {
        self.temp_dir.path().join(name).exists()
    }

    /// Delete a file from the repository.
    pub fn delete_file(&self, name: &str) {
        let path = self.temp_dir.path().join(name);
        if path.exists() {
            fs::remove_file(&path).expect("Failed to delete file");
        }
    }

    /// Create a directory in the repository.
    pub fn create_dir(&self, name: &str) {
        fs::create_dir_all(self.temp_dir.path().join(name)).expect("Failed to create directory");
    }

    /// Add files to the staging area.
    pub fn add(&self, paths: &[&str]) {
        MediagitCommand::add(self.path(), paths);
    }

    /// Create a commit with the given message.
    pub fn commit(&self, message: &str) {
        MediagitCommand::commit(self.path(), message);
    }

    /// Add a file and commit it in one operation.
    pub fn add_and_commit(&self, name: &str, content: &[u8], message: &str) {
        self.write_file(name, content);
        self.add(&[name]);
        self.commit(message);
    }

    /// Create a new branch.
    pub fn create_branch(&self, name: &str) {
        MediagitCommand::create_branch(self.path(), name);
    }

    /// Switch to a branch.
    pub fn switch_branch(&self, name: &str) {
        MediagitCommand::switch_branch(self.path(), name);
    }

    /// Get the path to a file in the repository.
    pub fn file_path(&self, name: &str) -> PathBuf {
        self.temp_dir.path().join(name)
    }
}

impl Default for TestRepo {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_repo_creation() {
        let repo = TestRepo::new();
        assert!(repo.path().exists());
    }

    #[test]
    fn test_write_and_read_file() {
        let repo = TestRepo::new();
        repo.write_file("test.txt", b"Hello, World!");
        assert_eq!(repo.read_file("test.txt"), b"Hello, World!");
    }

    #[test]
    fn test_file_exists() {
        let repo = TestRepo::new();
        assert!(!repo.file_exists("nonexistent.txt"));
        repo.write_file("exists.txt", b"content");
        assert!(repo.file_exists("exists.txt"));
    }

    #[test]
    fn test_nested_file_creation() {
        let repo = TestRepo::new();
        repo.write_file("path/to/nested/file.txt", b"content");
        assert!(repo.file_exists("path/to/nested/file.txt"));
    }
}
