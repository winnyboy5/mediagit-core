// MediaGit - Git for Media Files
// Copyright (C) 2025 MediaGit Contributors
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU Affero General Public License as published
// by the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.

//! `.mediagitignore` pattern matching.
//!
//! Provides [`IgnoreMatcher`], a wrapper around [`ignore::gitignore::Gitignore`] that
//! reads `.mediagitignore` files using the same glob syntax as `.gitignore`:
//!
//! - `*.tmp` — ignore all `.tmp` files
//! - `build/` — ignore the entire `build/` directory
//! - `!important.log` — negation: do NOT ignore `important.log`
//! - `# comment` — line comments
//!
//! Used by both `add` and `status` commands.

use anyhow::Result;
use ignore::gitignore::{Gitignore, GitignoreBuilder};
use std::path::Path;

/// Wraps a compiled set of `.mediagitignore` patterns.
///
/// Constructed via [`IgnoreMatcher::new`]. If no `.mediagitignore` file exists
/// the matcher is effectively a no-op (every `is_ignored` call returns `false`).
pub struct IgnoreMatcher {
    matcher: Gitignore,
}

impl IgnoreMatcher {
    /// Build an [`IgnoreMatcher`] from the `.mediagitignore` file in `repo_root`.
    ///
    /// Silently succeeds if the file does not exist — callers get a matcher that
    /// never ignores anything.  Returns `Err` only if the file exists but cannot
    /// be parsed or read.
    pub fn new(repo_root: &Path) -> Result<Self> {
        let mut builder = GitignoreBuilder::new(repo_root);

        let ignore_path = repo_root.join(".mediagitignore");
        if ignore_path.exists() {
            // `add` returns an Option<ignore::Error>; we convert to anyhow::Error.
            if let Some(err) = builder.add(&ignore_path) {
                return Err(anyhow::anyhow!("Failed to parse .mediagitignore: {}", err));
            }
        }

        let matcher = builder.build()?;
        Ok(Self { matcher })
    }

    /// Returns `true` if `path` (relative to repo root) matches a `.mediagitignore`
    /// pattern and should be excluded.
    ///
    /// `is_dir` should be `true` when the path refers to a directory — this allows
    /// directory-level patterns like `build/` to prune entire subtrees.
    pub fn is_ignored(&self, path: &Path, is_dir: bool) -> bool {
        self.matcher
            .matched_path_or_any_parents(path, is_dir)
            .is_ignore()
    }

    /// Returns `true` if a `.mediagitignore` file exists in `repo_root`.
    ///
    /// Useful for producing informational messages without constructing a matcher.
    #[allow(dead_code)]
    pub fn has_ignore_file(repo_root: &Path) -> bool {
        repo_root.join(".mediagitignore").exists()
    }
}
