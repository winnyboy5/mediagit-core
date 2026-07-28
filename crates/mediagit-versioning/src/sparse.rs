// MediaGit - Git for Media Files
// Copyright (C) 2025 MediaGit Contributors
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU Affero General Public License as published
// by the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE. See the
// GNU Affero General Public License for more details.

//! Sparse checkout: cone (directory-prefix) and pattern (gitignore-style
//! glob) filtering of which tracked files are materialized in the working
//! tree.
//!
//! Patterns live in `<repo_root>/.mediagit/info/sparse-checkout`. The file
//! being absent, or containing no pattern lines (only blanks/comments), means
//! sparse checkout is **disabled** — every path is included (full checkout).
//!
//! # Semantics (MediaGit's own — this is not a git port)
//!
//! - Excluded files are simply never written by checkout.
//! - Symmetrically, a file that already exists on disk but is
//!   sparse-excluded is **never deleted** by checkout (branch switch,
//!   `checkout_commit`, etc.) — it's just outside the cone, not a deletion.
//! - Materializing newly-included files and removing newly-excluded ones is
//!   the explicit job of the `sparse-checkout set`/`disable` CLI commands,
//!   not of ordinary checkout operations.
//!
//! Reuses the `ignore` crate (already a workspace dependency, used by
//! `mediagit-cli` for `.mediagitignore`) for pattern-mode gitignore-style
//! matching, rather than adding a new glob dependency.

use ignore::gitignore::{Gitignore, GitignoreBuilder};
use std::fs;
use std::path::{Path, PathBuf};

/// How pattern lines in the sparse-checkout file are interpreted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SparseMode {
    /// Each pattern is a directory prefix; a path is included if it is that
    /// directory or under it (recursively).
    Cone,
    /// Patterns are gitignore-style globs. A path is included if it matches
    /// a (non-negated) pattern — the inverse of `.mediagitignore`, where a
    /// match means *exclude*.
    Pattern,
}

const MODE_MARKER_CONE: &str = "# mode: cone";
const MODE_MARKER_PATTERN: &str = "# mode: pattern";

/// Compiled sparse-checkout filter for one repository.
pub struct SparseFilter {
    enabled: bool,
    mode: SparseMode,
    /// Raw pattern lines as configured (unmodified, for `sparse-checkout list`).
    patterns: Vec<String>,
    /// Cone mode only: normalized (forward-slash, no leading/trailing slash) prefixes.
    cone_dirs: Vec<String>,
    /// Pattern mode only: a match means "include".
    matcher: Option<Gitignore>,
}

impl SparseFilter {
    /// A no-op filter — sparse checkout disabled, every path included.
    pub fn disabled() -> Self {
        Self {
            enabled: false,
            mode: SparseMode::Cone,
            patterns: Vec::new(),
            cone_dirs: Vec::new(),
            matcher: None,
        }
    }

    /// Load from `<repo_root>/.mediagit/info/sparse-checkout`. A missing file
    /// or one with no pattern lines is treated as disabled.
    pub fn load(repo_root: &Path) -> anyhow::Result<Self> {
        let path = sparse_file_path(repo_root);
        if !path.exists() {
            return Ok(Self::disabled());
        }
        let content = fs::read_to_string(&path)?;
        Self::parse(repo_root, &content)
    }

    /// Build the filter `patterns` *would* produce, without writing it.
    ///
    /// WT-8: `sparse-checkout set` must decide whether the new patterns would
    /// delete modified files *before* persisting them, so a refusal leaves the
    /// on-disk configuration untouched.
    pub fn preview(
        repo_root: &Path,
        mode: SparseMode,
        patterns: &[String],
    ) -> anyhow::Result<Self> {
        let marker = match mode {
            SparseMode::Cone => MODE_MARKER_CONE,
            SparseMode::Pattern => MODE_MARKER_PATTERN,
        };
        Self::parse(repo_root, &format!("{marker}\n{}\n", patterns.join("\n")))
    }

    fn parse(repo_root: &Path, content: &str) -> anyhow::Result<Self> {
        let mut mode = SparseMode::Cone;
        let mut patterns = Vec::new();
        for line in content.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            if line == MODE_MARKER_CONE {
                mode = SparseMode::Cone;
                continue;
            }
            if line == MODE_MARKER_PATTERN {
                mode = SparseMode::Pattern;
                continue;
            }
            if line.starts_with('#') {
                continue;
            }
            patterns.push(line.to_string());
        }

        if patterns.is_empty() {
            return Ok(Self::disabled());
        }

        match mode {
            SparseMode::Cone => {
                let cone_dirs = patterns.iter().map(|p| normalize_pattern(p)).collect();
                Ok(Self {
                    enabled: true,
                    mode,
                    patterns,
                    cone_dirs,
                    matcher: None,
                })
            }
            SparseMode::Pattern => {
                let mut builder = GitignoreBuilder::new(repo_root);
                for p in &patterns {
                    builder.add_line(None, p).map_err(|err| {
                        anyhow::anyhow!("Invalid sparse-checkout pattern '{p}': {err}")
                    })?;
                }
                let matcher = builder.build()?;
                Ok(Self {
                    enabled: true,
                    mode,
                    patterns,
                    cone_dirs: Vec::new(),
                    matcher: Some(matcher),
                })
            }
        }
    }

    /// Write `patterns` (with a mode marker header) to
    /// `<repo_root>/.mediagit/info/sparse-checkout`, creating the `info/`
    /// directory if needed.
    pub fn write(repo_root: &Path, mode: SparseMode, patterns: &[String]) -> anyhow::Result<()> {
        let path = sparse_file_path(repo_root);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut out = String::new();
        out.push_str(match mode {
            SparseMode::Cone => MODE_MARKER_CONE,
            SparseMode::Pattern => MODE_MARKER_PATTERN,
        });
        out.push('\n');
        for p in patterns {
            out.push_str(p);
            out.push('\n');
        }
        fs::write(&path, out)?;
        Ok(())
    }

    /// Remove the sparse-checkout file entirely — disables sparse checkout.
    pub fn remove(repo_root: &Path) -> anyhow::Result<()> {
        let path = sparse_file_path(repo_root);
        if path.exists() {
            fs::remove_file(&path)?;
        }
        Ok(())
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    pub fn mode(&self) -> SparseMode {
        self.mode
    }

    /// Raw pattern lines as configured (for display, e.g. `sparse-checkout list`).
    pub fn patterns(&self) -> &[String] {
        &self.patterns
    }

    /// `true` if `rel_path` (relative to repo root) should be checked out.
    /// Always `true` when sparse checkout is disabled.
    pub fn is_included(&self, rel_path: &Path) -> bool {
        if !self.enabled {
            return true;
        }
        match self.mode {
            SparseMode::Cone => {
                let p = normalize_path(rel_path);
                self.cone_dirs
                    .iter()
                    .any(|d| p == *d || p.starts_with(&format!("{d}/")))
            }
            SparseMode::Pattern => self
                .matcher
                .as_ref()
                .map(|m| m.matched_path_or_any_parents(rel_path, false).is_ignore())
                .unwrap_or(true),
        }
    }
}

fn sparse_file_path(repo_root: &Path) -> PathBuf {
    repo_root
        .join(".mediagit")
        .join("info")
        .join("sparse-checkout")
}

fn normalize_pattern(p: &str) -> String {
    p.trim().trim_matches('/').replace('\\', "/")
}

fn normalize_path(p: &Path) -> String {
    let s = p.to_string_lossy().replace('\\', "/");
    s.trim_matches('/').to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn absent_file_is_disabled() {
        let dir = TempDir::new().unwrap();
        let filter = SparseFilter::load(dir.path()).unwrap();
        assert!(!filter.is_enabled());
        assert!(filter.is_included(Path::new("anything/at/all.png")));
    }

    #[test]
    fn empty_file_is_disabled() {
        let dir = TempDir::new().unwrap();
        SparseFilter::write(dir.path(), SparseMode::Cone, &[]).unwrap();
        let filter = SparseFilter::load(dir.path()).unwrap();
        assert!(!filter.is_enabled());
    }

    #[test]
    fn cone_mode_includes_prefix_and_excludes_others() {
        let dir = TempDir::new().unwrap();
        SparseFilter::write(
            dir.path(),
            SparseMode::Cone,
            &["assets/textures".to_string()],
        )
        .unwrap();
        let filter = SparseFilter::load(dir.path()).unwrap();
        assert!(filter.is_enabled());
        assert_eq!(filter.mode(), SparseMode::Cone);
        assert!(filter.is_included(Path::new("assets/textures")));
        assert!(filter.is_included(Path::new("assets/textures/wood.png")));
        assert!(!filter.is_included(Path::new("assets/audio/track.wav")));
        assert!(!filter.is_included(Path::new("readme.md")));
    }

    #[test]
    fn pattern_mode_matches_glob() {
        let dir = TempDir::new().unwrap();
        SparseFilter::write(dir.path(), SparseMode::Pattern, &["*.png".to_string()]).unwrap();
        let filter = SparseFilter::load(dir.path()).unwrap();
        assert_eq!(filter.mode(), SparseMode::Pattern);
        assert!(filter.is_included(Path::new("assets/wood.png")));
        assert!(!filter.is_included(Path::new("assets/track.wav")));
    }

    #[test]
    fn pattern_mode_dir_star_matches_nested_files() {
        let dir = TempDir::new().unwrap();
        SparseFilter::write(dir.path(), SparseMode::Pattern, &["assets/*".to_string()]).unwrap();
        let filter = SparseFilter::load(dir.path()).unwrap();
        assert!(filter.is_included(Path::new("assets/wood.png")));
        assert!(filter.is_included(Path::new("assets/textures/wood.png")));
        assert!(filter.is_included(Path::new("assets/textures/deep/wood.png")));
        assert!(!filter.is_included(Path::new("readme.md")));
    }

    #[test]
    fn pattern_mode_dir_slash_matches_nested_files() {
        let dir = TempDir::new().unwrap();
        SparseFilter::write(dir.path(), SparseMode::Pattern, &["assets/".to_string()]).unwrap();
        let filter = SparseFilter::load(dir.path()).unwrap();
        assert!(filter.is_included(Path::new("assets/wood.png")));
        assert!(filter.is_included(Path::new("assets/textures/deep/wood.png")));
        assert!(!filter.is_included(Path::new("readme.md")));
    }

    #[test]
    fn windows_separators_normalize_in_cone_mode() {
        let dir = TempDir::new().unwrap();
        SparseFilter::write(
            dir.path(),
            SparseMode::Cone,
            &["assets/textures".to_string()],
        )
        .unwrap();
        let filter = SparseFilter::load(dir.path()).unwrap();
        assert!(filter.is_included(Path::new("assets\\textures\\wood.png")));
    }
}
