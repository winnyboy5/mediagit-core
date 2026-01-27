// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2025 MediaGit Contributors

//! Cross-platform path utilities for tests.
//!
//! Provides utilities to handle path differences between Windows and Unix
//! systems, ensuring tests work consistently across platforms.

use std::env;
use std::path::{Path, PathBuf};

/// Cross-platform path utilities for test files.
pub struct TestPaths;

impl TestPaths {
    /// Get the path to the test-files directory.
    ///
    /// This resolves to the `test-files` directory at the repository root,
    /// handling both Windows and Unix path formats.
    pub fn test_files_dir() -> PathBuf {
        // Try to find the test-files directory relative to the project root
        Self::project_root().join("test-files")
    }

    /// Get the project root directory.
    ///
    /// Walks up from the current directory or CARGO_MANIFEST_DIR to find
    /// the workspace root (directory containing root Cargo.toml).
    pub fn project_root() -> PathBuf {
        // Start from CARGO_MANIFEST_DIR if available
        let start = env::var("CARGO_MANIFEST_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|_| env::current_dir().expect("Failed to get current directory"));

        let mut current = start.as_path();

        // Walk up until we find the workspace root (has [workspace] in Cargo.toml)
        loop {
            let cargo_toml = current.join("Cargo.toml");
            if cargo_toml.exists() {
                if let Ok(content) = std::fs::read_to_string(&cargo_toml) {
                    if content.contains("[workspace]") {
                        return current.to_path_buf();
                    }
                }
            }

            if let Some(parent) = current.parent() {
                current = parent;
            } else {
                // Fallback to CARGO_MANIFEST_DIR or current directory
                return start;
            }
        }
    }

    /// Normalize a path for cross-platform comparison.
    ///
    /// Uses dunce to handle Windows UNC paths (\\?\) and ensures consistent
    /// path separators.
    pub fn normalize(path: &Path) -> PathBuf {
        dunce::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
    }

    /// Get a test file path.
    ///
    /// Convenience method to get the path to a specific file in the test-files directory.
    pub fn test_file(name: &str) -> PathBuf {
        Self::test_files_dir().join(name)
    }

    /// Check if a test file exists.
    pub fn test_file_exists(name: &str) -> bool {
        Self::test_file(name).exists()
    }

    /// Convert a path to a string suitable for command-line arguments.
    ///
    /// On Windows, this handles UNC path prefixes and converts to a string
    /// that can be passed to commands.
    pub fn to_arg_string(path: &Path) -> String {
        Self::normalize(path).to_string_lossy().into_owned()
    }

    /// Create a platform-independent path from components.
    ///
    /// Joins path components using the correct separator for the current platform.
    pub fn join_components(components: &[&str]) -> PathBuf {
        let mut path = PathBuf::new();
        for component in components {
            path.push(component);
        }
        path
    }
}

/// Assert that two paths are equal after normalization.
///
/// Handles platform-specific path differences.
#[macro_export]
macro_rules! assert_paths_eq {
    ($left:expr, $right:expr) => {
        assert_eq!(
            $crate::TestPaths::normalize($left),
            $crate::TestPaths::normalize($right),
            "Paths are not equal"
        );
    };
    ($left:expr, $right:expr, $($arg:tt)+) => {
        assert_eq!(
            $crate::TestPaths::normalize($left),
            $crate::TestPaths::normalize($right),
            $($arg)+
        );
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_project_root_exists() {
        let root = TestPaths::project_root();
        assert!(root.exists(), "Project root should exist: {:?}", root);
    }

    #[test]
    fn test_project_root_has_cargo_toml() {
        let root = TestPaths::project_root();
        let cargo_toml = root.join("Cargo.toml");
        assert!(cargo_toml.exists(), "Project root should have Cargo.toml");
    }

    #[test]
    fn test_join_components() {
        let path = TestPaths::join_components(&["src", "commands", "init.rs"]);
        assert!(path.ends_with("init.rs"));
    }
}
