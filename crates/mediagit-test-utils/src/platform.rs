// SPDX-License-Identifier: BUSL-1.1
// Copyright (C) 2025-2026 Aswin Krishnamoorthy

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
            if cargo_toml.exists()
                && let Ok(content) = std::fs::read_to_string(&cargo_toml)
                && content.contains("[workspace]")
            {
                return current.to_path_buf();
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

    /// Announce a fixture root once, and say plainly whether it is there.
    ///
    /// `test-files/` and `dev-tests/dedup-pairs/` are gitignored
    /// (`.gitignore:200`), so they exist on a developer machine and NEVER in
    /// CI. The media tests therefore have to skip rather than fail — but a skip
    /// that prints nothing is indistinguishable from a pass, which is the exact
    /// failure mode recorded all over this repo's QA suite ("a SKIP reads as
    /// green").
    ///
    /// Checked at the ROOT rather than per file because that is the real
    /// failure mode: either the developer has the fixture set or nobody does.
    /// One greppable `MEDIAGIT-TEST-SKIP:` line per test binary is enough to
    /// tell "this suite had nothing to run" from "this suite passed", and it
    /// also makes a MISRESOLVED root obvious — previously indistinguishable
    /// from an absent one, which is how eight test files sat pointing at
    /// `/mnt/d/own/saas/mediagit-core/test-files` and silently skipped
    /// everywhere except one developer's WSL install.
    ///
    /// `MEDIAGIT_REQUIRE_TEST_FILES=1` turns the absence into a panic, so a
    /// machine that DOES have the fixtures can prove the suite actually ran
    /// instead of assuming it did. That is the other half of the guard: without
    /// it, "skipped" and "ran" look the same from the outside.
    pub fn announce_fixture_root(root: PathBuf, label: &str) -> PathBuf {
        if !root.exists() {
            let strict = env::var("MEDIAGIT_REQUIRE_TEST_FILES").as_deref() == Ok("1");
            assert!(
                !strict,
                "MEDIAGIT_REQUIRE_TEST_FILES=1 but {label} is missing at {}",
                root.display()
            );
            eprintln!(
                "MEDIAGIT-TEST-SKIP: {label} not found at {} \
                 - media assertions in this binary will skip",
                root.display()
            );
        }
        root
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
    ($left:expr_2021, $right:expr_2021) => {
        assert_eq!(
            $crate::TestPaths::normalize($left),
            $crate::TestPaths::normalize($right),
            "Paths are not equal"
        );
    };
    ($left:expr_2021, $right:expr_2021, $($arg:tt)+) => {
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

    /// Both halves of the fixture-root guard.
    ///
    /// The whole point of `announce_fixture_root` is that a CI run with no
    /// `test-files/` must be TELLABLE from a run that actually exercised the
    /// media tests. That needs the absent case to do something observable, and
    /// it needs the strict case to actually fail — a guard proven only in the
    /// direction that stays quiet is the failure mode this repo keeps
    /// recording.
    ///
    /// Both directions live in ONE test on purpose: `MEDIAGIT_REQUIRE_TEST_FILES`
    /// is process-global and Rust runs tests on threads, so splitting them would
    /// let the strict half leak into the lenient half.
    #[test]
    fn announce_fixture_root_reports_absence_and_can_be_made_fatal() {
        let missing = std::path::PathBuf::from("definitely-not-a-real-fixture-root-9f3a");
        assert!(!missing.exists(), "test premise: this path must not exist");

        // SAFETY: test-only. This is the only test in this binary that touches
        // MEDIAGIT_REQUIRE_TEST_FILES, so no other thread observes the change.
        unsafe { env::remove_var("MEDIAGIT_REQUIRE_TEST_FILES") };

        // Lenient (the CI case): returns the path, having announced the skip.
        let returned = TestPaths::announce_fixture_root(missing.clone(), "test-fixtures/");
        assert_eq!(
            returned, missing,
            "the path must still be returned so callers keep their existing skip logic"
        );

        // Strict (the developer case): the same absence is now fatal.
        unsafe { env::set_var("MEDIAGIT_REQUIRE_TEST_FILES", "1") };
        let strict = std::panic::catch_unwind(|| {
            TestPaths::announce_fixture_root(
                std::path::PathBuf::from("definitely-not-a-real-fixture-root-9f3a"),
                "test-fixtures/",
            )
        });
        unsafe { env::remove_var("MEDIAGIT_REQUIRE_TEST_FILES") };
        assert!(
            strict.is_err(),
            "MEDIAGIT_REQUIRE_TEST_FILES=1 must turn a missing fixture root into a failure"
        );

        // And a root that DOES exist is silent in both modes — otherwise the
        // marker would be noise and nobody would grep for it.
        let real = TestPaths::project_root();
        assert_eq!(
            TestPaths::announce_fixture_root(real.clone(), "workspace root"),
            real
        );
    }

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
