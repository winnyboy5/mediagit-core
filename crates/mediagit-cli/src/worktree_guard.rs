// SPDX-License-Identifier: BUSL-1.1
// Copyright (C) 2025-2026 Aswin Krishnamoorthy

//! The single working-tree safety check (WT-1/2/3/8).
//!
//! Every command that rewrites the working tree — `reset --hard`, `merge`,
//! `rebase`, `cherry-pick`, `revert`, `sparse-checkout set`, `branch switch` —
//! routes through [`AtRisk::check`] before touching a file, and refuses when
//! uncommitted or untracked work would be destroyed. Previously each command
//! either had no check at all (WT-3) or a top-level-only one (WT-2).
//!
//! Two independent hazards, reported separately because the remedies differ:
//!
//! - **modified** — a file tracked at HEAD whose working-tree content differs.
//!   The checkout would overwrite the edit. Commit or stash.
//! - **untracked collisions** — a file that is not tracked at all but which
//!   the target tree would materialize over. Commit, stash, or delete it.
//!
//! The third hazard, deletion of *non*-colliding untracked files, is not a
//! refusal: it is prevented at the root by
//! [`CheckoutManager::with_tracked_paths`], fed by [`tracked_paths`].

use crate::ignore_rules::IgnoreMatcher;
use anyhow::Result;
use mediagit_versioning::{CheckoutManager, Index, ObjectDatabase, Oid};
use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// Working-tree paths a pending checkout would destroy.
#[derive(Debug, Default)]
pub struct AtRisk {
    /// Tracked at HEAD, edited in the working tree.
    pub modified: Vec<PathBuf>,
    /// Untracked, but present in the target tree.
    pub untracked_collisions: Vec<PathBuf>,
}

impl AtRisk {
    /// Inspect the working tree against `head` (the commit it currently
    /// reflects) and `target` (the commit about to be checked out).
    ///
    /// `head` is `None` on an unborn HEAD — nothing is tracked, so nothing can
    /// be "modified". `target` is `None` when the caller is not checking out a
    /// commit at all (`sparse-checkout set`), which skips collision detection.
    pub async fn check(
        repo_root: &Path,
        odb: &ObjectDatabase,
        head: Option<&Oid>,
        target: Option<&Oid>,
    ) -> Result<Self> {
        let head_files = match head {
            Some(oid) => {
                CheckoutManager::new(odb, repo_root)
                    .tracked_files_at(oid)
                    .await?
            }
            None => Default::default(),
        };

        // WT-2: `tracked_files_at` flattens the whole tree, subdirectories
        // included — the bug was a top-level-only `tree.iter()`.
        let mut modified: Vec<PathBuf> = head_files
            .iter()
            .filter(|(path, (oid, _))| {
                working_oid(&repo_root.join(path)).is_some_and(|w| w != *oid)
            })
            .map(|(path, _)| path.clone())
            .collect();
        modified.sort();

        let untracked_collisions = match target {
            Some(target_oid) => {
                let untracked = untracked_paths(repo_root, &head_files.keys().cloned().collect())?;
                if untracked.is_empty() {
                    Vec::new()
                } else {
                    let target_files = CheckoutManager::new(odb, repo_root)
                        .tracked_files_at(target_oid)
                        .await?;
                    let mut hits: Vec<PathBuf> = untracked
                        .into_iter()
                        .filter(|p| target_files.contains_key(p))
                        .collect();
                    hits.sort();
                    hits
                }
            }
            None => Vec::new(),
        };

        Ok(Self {
            modified,
            untracked_collisions,
        })
    }

    pub fn is_empty(&self) -> bool {
        self.modified.is_empty() && self.untracked_collisions.is_empty()
    }

    /// Refuse, naming the paths. `operation` is the user-facing verb phrase,
    /// e.g. `"merge"` or `"branch switch"`.
    pub fn bail(&self, operation: &str) -> anyhow::Error {
        let mut msg = String::new();
        if !self.modified.is_empty() {
            msg.push_str(&format!(
                "Your local changes to the following files would be overwritten by {operation}:\n{}\nPlease commit your changes or stash them before you {operation}.",
                indent(&self.modified)
            ));
        }
        if !self.untracked_collisions.is_empty() {
            if !msg.is_empty() {
                msg.push_str("\n\n");
            }
            msg.push_str(&format!(
                "The following untracked working tree files would be overwritten by {operation}:\n{}\nPlease move or remove them before you {operation}.",
                indent(&self.untracked_collisions)
            ));
        }
        anyhow::anyhow!(msg)
    }

    /// Refuse unless clean.
    pub fn ensure_clean(&self, operation: &str) -> Result<()> {
        if self.is_empty() {
            Ok(())
        } else {
            Err(self.bail(operation))
        }
    }
}

/// Paths a checkout is entitled to delete: tracked at `head`, or staged.
///
/// "Tracked" here means HEAD tree ∪ index — anything the repository already
/// knows about. Feeds
/// [`CheckoutManager::with_tracked_paths`] so untracked work survives a
/// working-tree rewrite (WT-1).
pub async fn tracked_paths(
    repo_root: &Path,
    odb: &ObjectDatabase,
    head: Option<&Oid>,
) -> Result<HashSet<PathBuf>> {
    let mut paths: HashSet<PathBuf> = match head {
        Some(oid) => CheckoutManager::new(odb, repo_root)
            .tracked_files_at(oid)
            .await?
            .into_keys()
            .collect(),
        None => HashSet::new(),
    };
    paths.extend(Index::load(repo_root)?.entries().map(|e| e.path.clone()));
    Ok(paths)
}

/// Working-tree files that are neither tracked nor ignored.
fn untracked_paths(repo_root: &Path, tracked: &HashSet<PathBuf>) -> Result<HashSet<PathBuf>> {
    let index = Index::load(repo_root)?;
    let index_files: HashSet<PathBuf> = index.entries().map(|e| e.path.clone()).collect();

    let mut ignored: HashSet<PathBuf> = HashSet::new();
    let matcher = IgnoreMatcher::new(repo_root).ok();
    // Single status-equivalent scan of the working directory (perf budget).
    let status_cmd = crate::commands::status::StatusCmd {
        tracked: false,
        untracked: false,
        ignored: false,
        short: false,
        porcelain: false,
        branch: false,
        quiet: true,
        verbose: false,
        json: false,
    };
    let working = status_cmd.scan_working_directory(repo_root, &matcher, &mut ignored)?;

    Ok(working
        .into_iter()
        .filter(|p| !tracked.contains(p) && !index_files.contains(p) && !ignored.contains(p))
        .collect())
}

/// Hash a working-tree file the same way `add`/`status` do; `None` if it is
/// missing or unreadable (absence is not this guard's concern).
fn working_oid(full_path: &Path) -> Option<Oid> {
    let metadata = std::fs::metadata(full_path).ok()?;
    if metadata.len() >= crate::commands::utils::STREAMING_THRESHOLD {
        Oid::from_file(full_path).ok()
    } else {
        std::fs::read(full_path).ok().map(|c| Oid::hash(&c))
    }
}

fn indent(paths: &[PathBuf]) -> String {
    paths
        .iter()
        .map(|p| format!("\t{}", p.display()))
        .collect::<Vec<_>>()
        .join("\n")
}
