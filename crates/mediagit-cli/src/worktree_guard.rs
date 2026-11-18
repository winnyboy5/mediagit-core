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
use mediagit_versioning::{
    CheckoutManager, Commit, FileMode, Index, ObjectDatabase, Oid, Signature, Tree, TreeEntry,
};
use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// Working-tree paths a pending checkout would destroy.
#[derive(Debug, Default)]
pub struct AtRisk {
    /// Tracked at HEAD, edited in the working tree.
    pub modified: Vec<PathBuf>,
    /// Untracked, but present in the target tree.
    pub untracked_collisions: Vec<PathBuf>,
    /// Staged new file (absent at HEAD) at a path the target brings with a
    /// different blob. `reset --hard` discards these on purpose, like git.
    pub staged_collisions: Vec<PathBuf>,
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
        Self::run(repo_root, odb, head, target, None).await
    }

    /// Like [`Self::check`], but with `base` set only the paths `target`
    /// changes relative to `base` count as collisions. A cherry-pick writes
    /// just the picked version's own change, so an untracked file at any other
    /// path of its tree is not at risk.
    pub async fn check_changed(
        repo_root: &Path,
        odb: &ObjectDatabase,
        head: Option<&Oid>,
        target: Option<&Oid>,
        base: Option<&Oid>,
    ) -> Result<Self> {
        Self::run(repo_root, odb, head, target, base).await
    }

    async fn run(
        repo_root: &Path,
        odb: &ObjectDatabase,
        head: Option<&Oid>,
        target: Option<&Oid>,
        base: Option<&Oid>,
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
        // BUG-039: the stat cache answers for files `status`/`add` already
        // hashed; a miss is hashed once and recorded for the next run.
        use rayon::prelude::*;
        let mut stat = mediagit_versioning::StatCache::load(repo_root);
        let hashed: Vec<_> = head_files
            .par_iter()
            .filter_map(|(path, (oid, _))| {
                let (w, fresh) = working_oid(repo_root, path, &stat)?;
                Some((path.clone(), w != *oid, w, fresh.map(|m| (m, w))))
            })
            .collect();
        let mut modified: Vec<PathBuf> = Vec::new();
        let mut working: std::collections::HashMap<PathBuf, Oid> = Default::default();
        for (path, differs, w, fresh) in hashed {
            if let Some((meta, oid)) = fresh {
                stat.record(&path.to_string_lossy(), &meta, oid);
            }
            if differs {
                working.insert(path.clone(), w);
                modified.push(path);
            }
        }
        let _ = stat.save();
        // A file already holding the target's bytes is not at risk: the
        // checkout would rewrite identical content (a `stash push` before
        // `bisect reset`, or a step that failed after writing some files).
        if let (Some(target_oid), false) = (target, modified.is_empty()) {
            let target_files = CheckoutManager::new(odb, repo_root)
                .tracked_files_at(target_oid)
                .await?;
            modified.retain(|p| target_files.get(p).map(|(t, _)| t) != working.get(p));
        }
        modified.sort();

        // C1: a path staged but not committed (absent at HEAD) is invisible to
        // `modified` and `untracked_paths`. If the target brings a different
        // blob for it, the checkout would overwrite the file and the staged
        // blob would be left unreferenced.
        let staged_new: Vec<(PathBuf, Oid)> = Index::load(repo_root)?
            .entries()
            .filter(|e| !head_files.contains_key(&e.path))
            .map(|e| (e.path.clone(), e.oid))
            .collect();

        let (untracked_collisions, staged_collisions) = match target {
            Some(target_oid) => {
                let untracked = untracked_paths(repo_root, &head_files.keys().cloned().collect())?;
                if untracked.is_empty() && staged_new.is_empty() {
                    Default::default()
                } else {
                    let target_files = CheckoutManager::new(odb, repo_root)
                        .tracked_files_at(target_oid)
                        .await?;
                    let base_files = match base {
                        Some(b) => Some(
                            CheckoutManager::new(odb, repo_root)
                                .tracked_files_at(b)
                                .await?,
                        ),
                        None => None,
                    };
                    let changed = |p: &PathBuf, t: &Oid| {
                        base_files
                            .as_ref()
                            .is_none_or(|b| b.get(p).map(|(o, _)| o) != Some(t))
                    };
                    let mut hits: Vec<PathBuf> = untracked
                        .into_iter()
                        .filter(|p| target_files.get(p).is_some_and(|(t, _)| changed(p, t)))
                        .collect();
                    hits.sort();
                    let mut staged_hits: Vec<PathBuf> = staged_new
                        .into_iter()
                        .filter_map(|(p, staged)| {
                            let (t, _) = target_files.get(&p)?;
                            (*t != staged && changed(&p, t)).then_some(p)
                        })
                        .collect();
                    staged_hits.sort();
                    (hits, staged_hits)
                }
            }
            None => Default::default(),
        };

        Ok(Self {
            modified,
            untracked_collisions,
            staged_collisions,
        })
    }

    pub fn is_empty(&self) -> bool {
        self.modified.is_empty()
            && self.untracked_collisions.is_empty()
            && self.staged_collisions.is_empty()
    }

    /// Refuse, naming the paths. `operation` is the user-facing verb phrase,
    /// e.g. `"merge"` or `"branch switch"`.
    pub fn bail(&self, operation: &str) -> anyhow::Error {
        self.bail_with(
            operation,
            &format!("commit your changes or stash them before you {operation}"),
            &format!("move or remove them before you {operation}"),
        )
    }

    /// [`Self::bail`] with the remedy for each hazard spelled out by the caller.
    pub fn bail_with(
        &self,
        operation: &str,
        fix_modified: &str,
        fix_untracked: &str,
    ) -> anyhow::Error {
        let mut msg = String::new();
        if !self.modified.is_empty() {
            msg.push_str(&format!(
                "Your local changes to the following files would be overwritten by {operation}:\n{}\nPlease {fix_modified}.",
                indent(&self.modified)
            ));
        }
        let mut collisions: Vec<PathBuf> = self
            .untracked_collisions
            .iter()
            .chain(&self.staged_collisions)
            .cloned()
            .collect();
        collisions.sort();
        if !collisions.is_empty() {
            if !msg.is_empty() {
                msg.push_str("\n\n");
            }
            msg.push_str(&format!(
                "These files are not committed (new or staged) and would be overwritten by {operation}:\n{}\nPlease {fix_untracked}.",
                indent(&collisions)
            ));
        }
        crate::output::CodedError::new("would_overwrite", msg).into()
    }

    /// Refuse unless clean (bisect wording: committing is not an exit there).
    pub fn ensure_clean_bisect(&self, operation: &str) -> Result<()> {
        if self.is_empty() {
            return Ok(());
        }
        let fix = "stash or undo them, then run `mediagit bisect reset`";
        Err(self.bail_with(operation, fix, fix))
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

/// Paths a checkout is entitled to delete: tracked at `head`.
///
/// A staged new file (index entry absent from `head`) is NOT deletable: like
/// git, every checkout carries it, so neither its bytes nor its index entry
/// are lost. Feeds [`CheckoutManager::with_tracked_paths`] so untracked work
/// survives a working-tree rewrite (WT-1).
pub async fn tracked_paths(
    repo_root: &Path,
    odb: &ObjectDatabase,
    head: Option<&Oid>,
) -> Result<HashSet<PathBuf>> {
    Ok(match head {
        Some(oid) => CheckoutManager::new(odb, repo_root)
            .tracked_files_at(oid)
            .await?
            .into_keys()
            .collect(),
        None => HashSet::new(),
    })
}

/// [`tracked_paths`] plus every index entry: for the destructive paths that
/// discard staged work on purpose (`reset --hard`, abort of a stopped step).
pub async fn tracked_paths_with_staged(
    repo_root: &Path,
    odb: &ObjectDatabase,
    head: Option<&Oid>,
) -> Result<HashSet<PathBuf>> {
    let mut paths = tracked_paths(repo_root, odb, head).await?;
    paths.extend(Index::load(repo_root)?.entries().map(|e| e.path.clone()));
    Ok(paths)
}

/// Stage `merged` on top of `head_tree` (`--no-commit` of cherry-pick/revert),
/// keeping staged new files that are in neither tree — they are carried, not
/// dropped from the index.
pub fn restage_no_commit(repo_root: &Path, head_tree: &Tree, merged: &Tree) -> Result<()> {
    let norm = |p: &Path| p.to_string_lossy().replace('\\', "/");
    let old = Index::load(repo_root)?;
    let carried: Vec<_> = old
        .entries()
        .filter(|e| !head_tree.has_entry(&norm(&e.path)) && !merged.has_entry(&norm(&e.path)))
        .cloned()
        .collect();
    let mut index = Index::new();
    mediagit_versioning::stage_tree(&mut index, head_tree, merged);
    for e in carried {
        index.add_entry(e);
    }
    index.save(repo_root)?;
    Ok(())
}

/// Drop a stopped operation's conflict: put every tracked file back to
/// `head` (untracked files are kept), clear the index and remove the merge
/// files, so no marker text or unresolved flag outlives the operation.
/// Files of `on_disk` (the commit the stop checked out) count as tracked too.
pub async fn drop_conflict(
    repo_root: &Path,
    odb: &ObjectDatabase,
    head: &Oid,
    on_disk: &Oid,
) -> Result<()> {
    let mut tracked = tracked_paths_with_staged(repo_root, odb, Some(head)).await?;
    tracked.extend(
        CheckoutManager::new(odb, repo_root)
            .tracked_files_at(on_disk)
            .await?
            .into_keys(),
    );
    CheckoutManager::new(odb, repo_root)
        .with_tracked_paths(tracked)
        .checkout_commit(head)
        .await?;
    let mut index = Index::load(repo_root)?;
    index.clear();
    index.save(repo_root)?;
    mediagit_versioning::clear_merge_state(&repo_root.join(".mediagit"))?;
    Ok(())
}

/// Drop the conflict of a step being skipped (`revert --skip`,
/// `cherry-pick --skip`). Only the paths the stopped step touched — staged
/// or unresolved — go back to `head`; other tracked files keep the user's
/// edits (J4) and untracked files are kept. Then the index and the merge
/// files are cleared. [`drop_conflict`] is the full restore an abort does.
pub async fn drop_skipped_step(repo_root: &Path, odb: &ObjectDatabase, head: &Oid) -> Result<()> {
    let index = Index::load(repo_root)?;
    let head_tree = Tree::read(odb, &Commit::read(odb, head).await?.tree).await?;
    let norm = |p: &Path| p.to_string_lossy().replace('\\', "/");

    // What the step left behind: HEAD with its staged changes on top.
    let mut left = head_tree.clone();
    for p in index.deleted_paths() {
        left.remove_entry(&norm(p));
    }
    for e in index
        .entries()
        .filter(|e| !mediagit_versioning::is_stage_debris_key(&e.path.to_string_lossy()))
    {
        let mode = FileMode::from_u32(e.mode).unwrap_or(FileMode::Regular);
        left.add_entry(TreeEntry::new(norm(&e.path), mode, e.oid));
    }
    // An unresolved path's bytes are the step's whatever its entry says:
    // leaving it out of `left` makes the diff below always rewrite it.
    for p in index.unresolved_paths() {
        if head_tree.has_entry(&norm(&p)) {
            left.remove_entry(&norm(&p));
        }
    }

    // Differential checkout from that state to HEAD touches only the paths
    // that differ (sparse filter and case twins included).
    let sig = || Signature::now("MediaGit".to_string(), "mediagit@local".to_string());
    let left_commit = Commit::with_parents(
        left.write(odb).await?,
        vec![*head],
        sig(),
        sig(),
        "skipped step".to_string(),
    )
    .write(odb)
    .await?;
    CheckoutManager::new(odb, repo_root)
        .checkout_diff(&left_commit, head)
        .await?;

    let mut index = index;
    index.clear();
    index.save(repo_root)?;
    mediagit_versioning::clear_merge_state(&repo_root.join(".mediagit"))?;
    Ok(())
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
fn working_oid(
    repo_root: &Path,
    rel: &Path,
    stat: &mediagit_versioning::StatCache,
) -> Option<(Oid, Option<std::fs::Metadata>)> {
    let full_path = repo_root.join(rel);
    // Same lookup `status` makes: symlink_metadata, never the link's target.
    let sym = std::fs::symlink_metadata(&full_path).ok()?;
    if let Some(oid) = stat.lookup(&rel.to_string_lossy(), &sym) {
        return Some((oid, None));
    }
    let len = std::fs::metadata(&full_path).ok()?.len();
    let oid = if len >= crate::commands::utils::STREAMING_THRESHOLD {
        Oid::from_file(&full_path).ok()?
    } else {
        // nosemgrep: rust.actix.path-traversal.tainted-path.tainted-path -- read-only probe of local worktree
        Oid::hash(&std::fs::read(&full_path).ok()?)
    };
    Some((oid, Some(sym)))
}

/// Staged paths whose working file no longer matches what was staged, plus
/// staged deletions whose file is back.
pub fn edited_since_staged(repo_root: &Path, index: &Index) -> Vec<PathBuf> {
    let stat = mediagit_versioning::StatCache::load(repo_root);
    let mut out: Vec<PathBuf> = index
        .entries()
        .filter(|e| working_oid(repo_root, &e.path, &stat).is_some_and(|(w, _)| w != e.oid))
        .map(|e| e.path.clone())
        .collect();
    out.extend(
        index
            .deleted_paths()
            .filter(|p| repo_root.join(p).exists())
            .cloned(),
    );
    out
}

fn indent(paths: &[PathBuf]) -> String {
    paths
        .iter()
        .map(|p| format!("\t{}", p.display()))
        .collect::<Vec<_>>()
        .join("\n")
}
