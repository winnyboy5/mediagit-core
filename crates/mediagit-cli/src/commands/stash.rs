// SPDX-License-Identifier: BUSL-1.1
// Copyright (C) 2025-2026 Aswin Krishnamoorthy

use super::super::repo::{create_storage_backend, find_repo_root};
use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use console::style;
use mediagit_versioning::{
    CheckoutManager, Commit, Index, ObjectDatabase, ObjectType, Oid, RefDatabase, Tree,
};
use serde::Serialize;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use crate::output;

/// `--machine` data for every `stash` subcommand.
#[derive(Debug, Clone, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[cfg_attr(test, ts(export, export_to = "../../../ui/src/api/generated/"))]
pub struct StashReport {
    /// `save`, `apply`, `pop`, `drop`, `clear`, `list` or `show`.
    pub action: String,
    /// Stash entry acted on (`apply`, `pop`, `drop`, `show`).
    #[cfg_attr(test, ts(type = "number | null"))]
    pub index: Option<u64>,
    /// Stash commit; `None` for `list`/`clear`, and for a `save` with nothing to stash.
    pub oid: Option<String>,
    pub message: Option<String>,
    /// `apply`/`pop`: working-tree files updated.
    #[cfg_attr(test, ts(type = "number | null"))]
    pub files_updated: Option<u64>,
    /// `list` (all entries, newest first) and `show` (the one entry).
    pub stashes: Vec<StashEntryReport>,
    /// `save`: the paths that went into the stash, sorted.
    pub paths: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[cfg_attr(test, ts(export, export_to = "../../../ui/src/api/generated/"))]
pub struct StashEntryReport {
    #[cfg_attr(test, ts(type = "number"))]
    pub index: u64,
    pub oid: String,
    pub message: String,
    pub branch: Option<String>,
    /// RFC 3339.
    pub timestamp: String,
}

/// `path` is `spec` itself or lies under it (both repo-relative, `/`-separated).
fn selected_by(path: &Path, spec: &str) -> bool {
    let p = path.to_string_lossy().replace('\\', "/");
    p == spec || p.starts_with(&format!("{spec}/"))
}

fn stash_report(action: &str) -> StashReport {
    StashReport {
        action: action.into(),
        index: None,
        oid: None,
        message: None,
        files_updated: None,
        stashes: Vec::new(),
        paths: Vec::new(),
    }
}

fn entry_report(index: usize, e: &StashEntry) -> StashEntryReport {
    StashEntryReport {
        index: index as u64,
        oid: e.commit_oid.clone(),
        message: e.message.clone(),
        branch: e.branch.clone(),
        timestamp: e.timestamp.clone(),
    }
}

fn quiet(flag: bool) -> bool {
    flag || output::machine_mode()
}

/// Stash changes in working directory
#[derive(Parser, Debug)]
pub struct StashCmd {
    #[command(subcommand)]
    pub command: StashSubcommand,
}

#[derive(Subcommand, Debug)]
pub enum StashSubcommand {
    /// Save changes to stash, optionally only for the given paths
    Push(PushOpts),

    /// Save changes to stash
    Save(SaveOpts),

    /// Apply stashed changes
    Apply(ApplyOpts),

    /// List stashed changes
    List(ListOpts),

    /// Show stash contents
    Show(ShowOpts),

    /// Remove a stash entry
    Drop(DropOpts),

    /// Apply and remove stash entry
    Pop(PopOpts),

    /// Clear all stashes
    Clear(ClearOpts),
}

/// `stash push`: git's shape, where every positional is a path.
#[derive(Parser, Debug)]
pub struct PushOpts {
    /// Stash message
    #[arg(short = 'm', long = "message", value_name = "MESSAGE")]
    pub message: Option<String>,

    /// Include untracked files
    #[arg(short = 'u', long)]
    pub include_untracked: bool,

    /// Stash only these paths (files or directories); everything else is left alone
    #[arg(value_name = "PATHS")]
    pub paths: Vec<PathBuf>,

    /// Quiet mode
    #[arg(short, long)]
    pub quiet: bool,
}

#[derive(Parser, Debug)]
pub struct SaveOpts {
    /// Stash message (flag form, e.g. -m "WIP")
    #[arg(short = 'm', long = "message", value_name = "MESSAGE")]
    pub message_flag: Option<String>,

    /// Stash message (positional, for git-compatible `stash save "msg"`)
    #[arg(value_name = "MESSAGE")]
    pub message_positional: Option<String>,

    /// Include untracked files
    #[arg(short = 'u', long)]
    pub include_untracked: bool,

    /// Stash only these paths (files or directories); everything else is left alone
    #[arg(value_name = "PATHS")]
    pub paths: Vec<PathBuf>,

    /// Quiet mode
    #[arg(short, long)]
    pub quiet: bool,
}

#[derive(Parser, Debug)]
pub struct ApplyOpts {
    /// Stash index (default: 0)
    #[arg(value_name = "STASH")]
    pub stash: Option<usize>,

    /// Reinstate index changes
    #[arg(long)]
    pub index: bool,

    /// WT-7: overwrite uncommitted changes that differ from the stash.
    /// Without this, `apply` refuses rather than destroying work that exists
    /// nowhere else.
    #[arg(short, long)]
    pub force: bool,

    /// Quiet mode
    #[arg(short, long)]
    pub quiet: bool,
}

#[derive(Parser, Debug)]
pub struct ListOpts {
    /// Show verbose output
    #[arg(short, long)]
    pub verbose: bool,
}

#[derive(Parser, Debug)]
pub struct ShowOpts {
    /// Stash index (default: 0)
    #[arg(value_name = "STASH")]
    pub stash: Option<usize>,

    /// Show patch
    #[arg(short = 'p', long)]
    pub patch: bool,
}

#[derive(Parser, Debug)]
pub struct DropOpts {
    /// Stash index (default: 0)
    #[arg(value_name = "STASH")]
    pub stash: Option<usize>,

    /// Quiet mode
    #[arg(short, long)]
    pub quiet: bool,
}

#[derive(Parser, Debug)]
pub struct PopOpts {
    /// Stash index (default: 0)
    #[arg(value_name = "STASH")]
    pub stash: Option<usize>,

    /// Reinstate index changes
    #[arg(long)]
    pub index: bool,

    /// WT-7: overwrite uncommitted changes that differ from the stash.
    /// `pop` is apply-then-drop, so an unguarded clobber loses both the
    /// working edit and the stash entry that could have restored it.
    #[arg(short, long)]
    pub force: bool,

    /// Quiet mode
    #[arg(short, long)]
    pub quiet: bool,
}

#[derive(Parser, Debug)]
pub struct ClearOpts {
    /// Force clear without confirmation
    #[arg(short, long)]
    pub force: bool,
}

/// Content hash of a worktree file; files at or above the streaming threshold
/// are hashed in constant memory (same split as `add`/`status`).
fn hash_worktree_file(path: &Path) -> Option<Oid> {
    let large = std::fs::metadata(path).is_ok_and(|m| m.len() >= super::utils::STREAMING_THRESHOLD);
    if large {
        Oid::from_file(path).ok()
    } else {
        std::fs::read(path).ok().map(|b| Oid::hash(&b))
    }
}

impl StashCmd {
    pub async fn execute(&self) -> Result<()> {
        match &self.command {
            StashSubcommand::Push(opts) => {
                self.save(&SaveOpts {
                    message_flag: opts.message.clone(),
                    message_positional: None,
                    include_untracked: opts.include_untracked,
                    paths: opts.paths.clone(),
                    quiet: opts.quiet,
                })
                .await
            }
            StashSubcommand::Save(opts) => self.save(opts).await,
            StashSubcommand::Apply(opts) => self.apply(opts).await,
            StashSubcommand::List(opts) => self.list(opts).await,
            StashSubcommand::Show(opts) => self.show(opts).await,
            StashSubcommand::Drop(opts) => self.drop(opts).await,
            StashSubcommand::Pop(opts) => self.pop(opts).await,
            StashSubcommand::Clear(opts) => self.clear(opts).await,
        }
    }

    async fn save(&self, opts: &SaveOpts) -> Result<()> {
        let repo_root = find_repo_root()?;
        let mediagit_dir = repo_root.join(".mediagit");
        let storage = create_storage_backend(&repo_root).await?;
        let odb = ObjectDatabase::with_smart_compression(storage.clone(), 1000);
        let refdb = RefDatabase::new(&mediagit_dir);

        // Load index (staged changes)
        let index = Index::load(&repo_root)?;

        // Get current HEAD
        let current_oid = refdb
            .resolve("HEAD")
            .await
            .context("Failed to resolve HEAD")?;

        // Load HEAD commit tree to detect working-tree modifications
        let head_data = odb
            .read(&current_oid)
            .await
            .context("Failed to read HEAD commit")?;
        let head_commit =
            Commit::deserialize(&head_data).context("Failed to deserialize HEAD commit")?;
        let tree_data = odb
            .read(&head_commit.tree)
            .await
            .context("Failed to read HEAD tree")?;
        let head_tree = Tree::deserialize(&tree_data).context("Failed to deserialize HEAD tree")?;

        // Build HEAD file map (path -> oid)
        let mut head_files: HashMap<PathBuf, Oid> = HashMap::new();
        let mut head_modes: HashMap<PathBuf, mediagit_versioning::FileMode> = HashMap::new();
        for entry in head_tree.iter() {
            head_files.insert(PathBuf::from(&entry.name), entry.oid);
            head_modes.insert(PathBuf::from(&entry.name), entry.mode);
        }

        // `stash push <paths>`: only these (files or directories) are stashed.
        let specs: Vec<String> = opts
            .paths
            .iter()
            .map(|p| {
                p.to_string_lossy()
                    .replace('\\', "/")
                    .trim_start_matches("./")
                    .trim_end_matches('/')
                    .to_string()
            })
            .collect();
        let whole_tree = specs.is_empty() || specs.iter().any(|s| s.is_empty() || s == ".");
        let selected = |p: &Path| {
            whole_tree || {
                let p = p.to_string_lossy().replace('\\', "/");
                specs
                    .iter()
                    .any(|s| p == *s || p.starts_with(&format!("{s}/")))
            }
        };

        // Scan working directory for modifications against HEAD
        let working_files = self.scan_working_directory(&repo_root)?;
        let mut working_tree_changes: Vec<(PathBuf, Oid)> = Vec::new();

        for (path, head_oid) in &head_files {
            if !working_files.contains(path) {
                continue; // Deleted file — tracked by absence
            }
            let full_path = repo_root.join(path);
            let Some(working_oid) = hash_worktree_file(&full_path) else {
                continue;
            };
            if working_oid != *head_oid {
                working_tree_changes.push((path.clone(), working_oid));
            }
        }

        // Untracked/new files: `checkout_commit` below (restore to HEAD) removes
        // ANY file not in the target tree, with no concept of "untracked" - so
        // an untracked WIP file is silently destroyed unless stash captures it
        // first (DES-3). Captured by default since the reset is destructive
        // regardless of `-u`; the flag is accepted for git-compatible parsing.
        let mut untracked_changes: Vec<(PathBuf, Oid)> = Vec::new();
        for path in &working_files {
            if head_files.contains_key(path) || index.contains(path) {
                continue;
            }
            let full_path = repo_root.join(path);
            if let Some(oid) = hash_worktree_file(&full_path) {
                untracked_changes.push((path.clone(), oid));
            }
        }

        // A path that names nothing mediagit knows is a typo, not "no changes".
        if !whole_tree {
            for s in &specs {
                let known = head_files.keys().any(|p| selected_by(p, s))
                    || working_files.iter().any(|p| selected_by(p, s))
                    || index.entries().any(|e| selected_by(&e.path, s));
                if !known {
                    return Err(output::CodedError::new(
                        "invalid_args",
                        format!("pathspec '{s}' did not match any file known to mediagit"),
                    )
                    .into());
                }
            }
        }
        working_tree_changes.retain(|(p, _)| selected(p));
        untracked_changes.retain(|(p, _)| selected(p));
        let staged: Vec<&mediagit_versioning::IndexEntry> =
            index.entries().filter(|e| selected(&e.path)).collect();

        // BUG-3 fix: Check index, working-tree changes, AND untracked files
        if staged.is_empty() && working_tree_changes.is_empty() && untracked_changes.is_empty() {
            if !quiet(opts.quiet) {
                println!("{} No changes to stash", style("ℹ").blue());
            }
            if output::machine_mode() {
                return output::emit_ok(&stash_report("save"));
            }
            return Ok(());
        }

        // Build stash tree: start from HEAD tree as base, then overlay modifications
        let mut tree = mediagit_versioning::Tree::new();

        // 1. Start with all HEAD tree entries as base
        for entry in head_tree.iter() {
            tree.add_entry(mediagit_versioning::TreeEntry::new(
                entry.name.clone(),
                entry.mode,
                entry.oid,
            ));
        }

        // 2. Override with working-tree modifications and untracked files
        //    (write blobs to ODB)
        for (path, content_oid) in working_tree_changes.iter().chain(untracked_changes.iter()) {
            let full_path = repo_root.join(path);
            // Large files are chunked straight from disk (same threshold as `add`).
            let large = std::fs::metadata(&full_path)
                .is_ok_and(|m| m.len() >= super::utils::STREAMING_THRESHOLD);
            if large {
                let filename = full_path.file_name().and_then(|n| n.to_str()).unwrap_or("");
                let blob_oid = odb
                    .write_chunked_from_file(&full_path, filename, None, Some(*content_oid))
                    .await
                    .context(format!("Failed to write blob for {}", path.display()))?;
                tree.add_entry(mediagit_versioning::TreeEntry::new(
                    path.to_string_lossy().to_string(),
                    mediagit_versioning::FileMode::Regular,
                    blob_oid,
                ));
            } else if let Ok(content) = std::fs::read(&full_path) {
                let blob_oid = odb
                    .write(ObjectType::Blob, &content)
                    .await
                    .context(format!("Failed to write blob for {}", path.display()))?;
                tree.add_entry(mediagit_versioning::TreeEntry::new(
                    path.to_string_lossy().to_string(),
                    mediagit_versioning::FileMode::Regular,
                    blob_oid,
                ));
            }
        }

        // 3. Override with index entries (staged changes take priority)
        for entry in &staged {
            tree.add_entry(mediagit_versioning::TreeEntry::new(
                entry.path.to_string_lossy().to_string(),
                mediagit_versioning::FileMode::Regular,
                entry.oid,
            ));
        }

        // What went into the stash, for the report and for the partial cleanup.
        let mut stashed: Vec<PathBuf> = working_tree_changes
            .iter()
            .map(|(p, _)| p.clone())
            .chain(untracked_changes.iter().map(|(p, _)| p.clone()))
            .chain(
                staged
                    .iter()
                    .map(|e| PathBuf::from(e.path.to_string_lossy().replace('\\', "/"))),
            )
            .collect();
        stashed.sort();
        stashed.dedup();

        let tree_oid = tree.write(&odb).await?;

        // Create stash commit (-m flag takes priority over positional)
        let message = opts
            .message_flag
            .as_deref()
            .or(opts.message_positional.as_deref())
            .unwrap_or("WIP on branch")
            .to_string();

        let stash_signature = mediagit_versioning::Signature {
            name: "Stash".to_string(),
            email: "stash@mediagit.local".to_string(),
            timestamp: chrono::Utc::now(),
        };

        let commit = Commit {
            tree: tree_oid,
            parents: vec![current_oid],
            author: stash_signature.clone(),
            committer: stash_signature,
            message: message.clone(),
        };

        let commit_oid = commit.write(&odb).await?;

        // Save stash entry
        let stash_entry = StashEntry {
            commit_oid: commit_oid.to_hex(),
            message: message.clone(),
            timestamp: chrono::Utc::now().to_rfc3339(),
            branch: self.get_current_branch(&refdb).await?,
            staged: staged
                .iter()
                .map(|e| e.path.to_string_lossy().replace('\\', "/"))
                .collect(),
        };

        self.save_stash_entry(&mediagit_dir, stash_entry)?;

        // Clean working directory
        let mut index = Index::load(&repo_root)?;
        let checkout_mgr = CheckoutManager::new(&odb, &repo_root);
        if whole_tree {
            index.clear();
            index.save(&repo_root)?;

            // Restore to HEAD state
            checkout_mgr.checkout_commit(&current_oid).await?;
        } else {
            // Only the stashed paths go back to HEAD; every other change stays.
            for path in &stashed {
                match (head_files.get(path), head_modes.get(path)) {
                    (Some(oid), Some(mode)) => {
                        checkout_mgr.materialize_file(path, oid, *mode).await?;
                    }
                    // New file: it exists only in the stash now.
                    _ => {
                        let _ = std::fs::remove_file(repo_root.join(path));
                    }
                }
                index.remove_entry(path);
            }
            index.save(&repo_root)?;
        }

        if !quiet(opts.quiet) {
            println!(
                "{} Saved working directory and index state",
                style("✓").green()
            );
            println!("  WIP on: {}", message);
        }

        if output::machine_mode() {
            return output::emit_ok(&StashReport {
                index: Some(0),
                oid: Some(commit_oid.to_hex()),
                message: Some(message),
                paths: stashed
                    .iter()
                    .map(|p| p.to_string_lossy().replace('\\', "/"))
                    .collect(),
                ..stash_report("save")
            });
        }

        Ok(())
    }

    async fn apply(&self, opts: &ApplyOpts) -> Result<()> {
        let (index, files_updated, entry) = self.apply_inner(opts).await?;
        if output::machine_mode() {
            return output::emit_ok(&StashReport {
                index: Some(index as u64),
                oid: Some(entry.commit_oid),
                message: Some(entry.message),
                files_updated: Some(files_updated as u64),
                ..stash_report("apply")
            });
        }
        Ok(())
    }

    /// Apply without emitting, so `pop` can report once for apply+drop.
    async fn apply_inner(&self, opts: &ApplyOpts) -> Result<(usize, usize, StashEntry)> {
        let repo_root = find_repo_root()?;
        let mediagit_dir = repo_root.join(".mediagit");
        let storage = create_storage_backend(&repo_root).await?;
        let odb = ObjectDatabase::with_smart_compression(storage.clone(), 1000);

        // Load stash entry
        let stash_index = opts.stash.unwrap_or(0);
        let stash_entry = self.load_stash_entry(&mediagit_dir, stash_index)?;

        let stash_oid = Oid::from_hex(&stash_entry.commit_oid)?;

        // WT-7: refuse to overwrite divergent uncommitted work.
        //
        // The overlay below writes every stashed path unconditionally. If you
        // stashed an edit and then made a *different* edit to the same file,
        // that second edit was silently destroyed — and unlike the stashed
        // version, it existed nowhere else. Stash is supposed to be the safe
        // place to park work, so losing work to it is especially bad.
        //
        // A path is at risk when the working copy differs from BOTH the
        // stashed content (so applying would change it) and from HEAD (so the
        // difference is unsaved work rather than a clean checkout).
        let clobbered = Self::paths_at_risk(&repo_root, &odb, &stash_oid).await?;
        if !clobbered.is_empty() && !opts.force {
            anyhow::bail!(
                "Refusing to apply: {} path(s) have uncommitted changes that differ \
                 from the stash and would be overwritten:\n  {}\n\
                 Commit or stash them first, or re-run with --force to discard them.",
                clobbered.len(),
                clobbered
                    .iter()
                    .map(|p| p.display().to_string())
                    .collect::<Vec<_>>()
                    .join("\n  ")
            );
        }

        // Apply stash tree on top of current working directory (overlay, not replace).
        // checkout_commit would wipe files not in the stash tree.
        // Only paths the stash changed relative to where it was taken: a
        // partial stash's tree also carries every untouched file at its HEAD
        // content, and writing those back would destroy later edits to them.
        let checkout_mgr = CheckoutManager::new(&odb, &repo_root);
        let changed = Self::stash_changes(&repo_root, &odb, &stash_oid).await?;
        // --index: restage what was staged at save time (only paths the stash changed).
        let restage: Vec<(PathBuf, Oid, u32)> = if opts.index {
            let staged: HashSet<PathBuf> = stash_entry.staged.iter().map(PathBuf::from).collect();
            changed
                .iter()
                .filter(|(p, _)| staged.contains(*p))
                .map(|(p, (oid, mode))| (p.clone(), *oid, mode.as_u32()))
                .collect()
        } else {
            Vec::new()
        };
        let files_updated = checkout_mgr
            .write_entries(
                changed
                    .into_iter()
                    .map(|(path, (oid, mode))| (path, oid, mode))
                    .collect(),
            )
            .await?;
        if !restage.is_empty() {
            let mut index = Index::load(&repo_root)?;
            for (path, oid, mode) in restage {
                index.add_entry(mediagit_versioning::IndexEntry::new(
                    path, oid, mode, 0, None,
                ));
            }
            index.save(&repo_root)?;
        }

        if !quiet(opts.quiet) {
            println!("{} Applied stash entry {}", style("✓").green(), stash_index);
            if files_updated > 0 {
                println!("  Updated {} file(s)", files_updated);
            }
        }

        Ok((stash_index, files_updated as usize, stash_entry))
    }

    /// Files whose content in the stash differs from the commit it was taken on.
    async fn stash_changes(
        repo_root: &Path,
        odb: &ObjectDatabase,
        stash_oid: &Oid,
    ) -> Result<HashMap<PathBuf, (Oid, mediagit_versioning::FileMode)>> {
        let mgr = CheckoutManager::new(odb, repo_root);
        let mut files = mgr.tracked_files_at(stash_oid).await?;
        let stash_commit = Commit::read(odb, stash_oid).await?;
        if let Some(parent) = stash_commit.parents.first()
            && let Ok(base) = mgr.tracked_files_at(parent).await
        {
            files.retain(|path, (oid, _)| base.get(path).map(|(o, _)| o) != Some(oid));
        }
        Ok(files)
    }

    /// WT-7: stashed paths whose working copy holds unsaved work that the
    /// overlay would destroy.
    ///
    /// At risk means the working file differs from BOTH:
    ///   - the stashed content (so applying would actually change it), and
    ///   - HEAD (so the difference is unsaved work, not a clean checkout).
    ///
    /// Files matching the stash are already applied; files matching HEAD are
    /// clean and safe to overwrite. Only the third case is unrecoverable —
    /// that content exists nowhere else in the repository.
    ///
    /// Compares content hashes rather than scanning for text markers, so it
    /// behaves identically for media and binary assets.
    async fn paths_at_risk(
        repo_root: &Path,
        odb: &ObjectDatabase,
        stash_oid: &Oid,
    ) -> Result<Vec<PathBuf>> {
        let stash_commit = Commit::read(odb, stash_oid).await?;
        let stash_files = Self::stash_changes(repo_root, odb, stash_oid).await?;

        // HEAD content for the same paths, to tell "unsaved work" from "clean".
        let refdb = RefDatabase::new(repo_root.join(".mediagit"));
        let head_files = match refdb.resolve("HEAD").await {
            Ok(head_oid) => CheckoutManager::new(odb, repo_root)
                .tracked_files_at(&head_oid)
                .await
                .unwrap_or_default(),
            Err(_) => Default::default(),
        };
        let _ = stash_commit;

        let mut at_risk = Vec::new();
        for (path, (stash_entry_oid, _)) in &stash_files {
            let full = repo_root.join(path);
            // nosemgrep: rust.actix.path-traversal.tainted-path.tainted-path -- read-only probe of local worktree
            let Some(working) = hash_worktree_file(&full) else {
                continue; // absent: the overlay creates it, nothing to lose
            };
            if working == *stash_entry_oid {
                continue; // already matches the stash
            }
            if head_files.get(path).map(|(o, _)| *o) == Some(working) {
                continue; // clean checkout of HEAD — safe to overwrite
            }
            at_risk.push(path.clone());
        }
        at_risk.sort();
        Ok(at_risk)
    }

    async fn list(&self, opts: &ListOpts) -> Result<()> {
        let repo_root = find_repo_root()?;
        let mediagit_dir = repo_root.join(".mediagit");

        let stash_list = self.load_all_stashes(&mediagit_dir)?;

        if output::machine_mode() {
            return output::emit_ok(&StashReport {
                stashes: stash_list
                    .iter()
                    .enumerate()
                    .map(|(i, e)| entry_report(i, e))
                    .collect(),
                ..stash_report("list")
            });
        }

        if stash_list.is_empty() {
            println!("No stashes found");
            return Ok(());
        }

        for (index, entry) in stash_list.iter().enumerate() {
            if opts.verbose {
                println!(
                    "{}: {} on {}",
                    style(format!("stash@{{{}}}", index)).yellow(),
                    style(&entry.message).green(),
                    style(&entry.branch.as_deref().unwrap_or("unknown")).cyan()
                );
                println!("  Commit: {}", entry.commit_oid);
                println!("  Date: {}", entry.timestamp);
                println!();
            } else {
                println!(
                    "{}: {}",
                    style(format!("stash@{{{}}}", index)).yellow(),
                    entry.message
                );
            }
        }

        Ok(())
    }

    async fn show(&self, opts: &ShowOpts) -> Result<()> {
        let repo_root = find_repo_root()?;
        let mediagit_dir = repo_root.join(".mediagit");

        let stash_index = opts.stash.unwrap_or(0);
        let stash_entry = self.load_stash_entry(&mediagit_dir, stash_index)?;

        if output::machine_mode() {
            return output::emit_ok(&StashReport {
                index: Some(stash_index as u64),
                oid: Some(stash_entry.commit_oid.clone()),
                message: Some(stash_entry.message.clone()),
                stashes: vec![entry_report(stash_index, &stash_entry)],
                ..stash_report("show")
            });
        }

        println!(
            "{}",
            style(format!("stash@{{{}}}", stash_index)).yellow().bold()
        );
        println!("Message:  {}", stash_entry.message);
        println!(
            "Branch:   {}",
            stash_entry.branch.as_deref().unwrap_or("unknown")
        );
        println!("Commit:   {}", stash_entry.commit_oid);
        println!("Date:     {}", stash_entry.timestamp);

        if opts.patch {
            // Future enhancement: integrate diff display here
            println!("\n{} Patch display feature coming soon", style("ℹ").blue());
        }

        Ok(())
    }

    async fn drop(&self, opts: &DropOpts) -> Result<()> {
        let (index, removed) = self.drop_inner(opts).await?;
        if output::machine_mode() {
            return output::emit_ok(&StashReport {
                index: Some(index as u64),
                oid: Some(removed.commit_oid),
                message: Some(removed.message),
                ..stash_report("drop")
            });
        }
        Ok(())
    }

    /// Drop without emitting, so `pop` can report once for apply+drop.
    async fn drop_inner(&self, opts: &DropOpts) -> Result<(usize, StashEntry)> {
        let repo_root = find_repo_root()?;
        let mediagit_dir = repo_root.join(".mediagit");

        let stash_index = opts.stash.unwrap_or(0);

        // Load all stashes
        let mut stash_list = self.load_all_stashes(&mediagit_dir)?;

        if stash_index >= stash_list.len() {
            anyhow::bail!("Stash entry {} not found", stash_index);
        }

        // Remove stash entry
        let removed = stash_list.remove(stash_index);

        // Save updated list
        self.save_all_stashes(&mediagit_dir, &stash_list)?;

        if !quiet(opts.quiet) {
            println!(
                "{} Dropped stash@{{{}}} ({})",
                style("✓").green(),
                stash_index,
                removed.message
            );
        }

        Ok((stash_index, removed))
    }

    async fn pop(&self, opts: &PopOpts) -> Result<()> {
        let _repo_root = find_repo_root()?;

        // Apply stash
        let apply_opts = ApplyOpts {
            stash: opts.stash,
            index: opts.index,
            force: opts.force,
            quiet: opts.quiet,
        };
        let (index, files_updated, entry) = self.apply_inner(&apply_opts).await?;

        // Drop stash after successful apply
        let drop_opts = DropOpts {
            stash: opts.stash,
            quiet: opts.quiet,
        };
        self.drop_inner(&drop_opts).await?;

        if output::machine_mode() {
            return output::emit_ok(&StashReport {
                index: Some(index as u64),
                oid: Some(entry.commit_oid),
                message: Some(entry.message),
                files_updated: Some(files_updated as u64),
                ..stash_report("pop")
            });
        }

        Ok(())
    }

    async fn clear(&self, opts: &ClearOpts) -> Result<()> {
        let repo_root = find_repo_root()?;
        let mediagit_dir = repo_root.join(".mediagit");

        if !opts.force {
            // Prompt for confirmation — fall back gracefully when stdin is not a terminal
            use dialoguer::Confirm;
            crate::output::ensure_interactive("stash clear confirmation")?;
            let confirmed = match Confirm::new()
                .with_prompt("Clear all stash entries?")
                .default(false)
                .interact()
            {
                Ok(answer) => answer,
                Err(_) => {
                    // Non-interactive context (pipe, script, CI) — require --force to proceed
                    anyhow::bail!(
                        "stdin is not a terminal. Use 'mediagit stash clear --force' to clear without confirmation."
                    );
                }
            };

            if !confirmed {
                println!("Cancelled");
                return Ok(());
            }
        }

        // Clear stash list
        let stash_path = mediagit_dir.join("STASH_LIST");
        if stash_path.exists() {
            std::fs::remove_file(&stash_path)?;
        }

        if output::machine_mode() {
            return output::emit_ok(&stash_report("clear"));
        }
        println!("{} Cleared all stash entries", style("✓").green());

        Ok(())
    }

    async fn get_current_branch(&self, refdb: &RefDatabase) -> Result<Option<String>> {
        let head = refdb.read("HEAD").await?;
        Ok(head
            .target
            .map(|t| t.strip_prefix("refs/heads/").unwrap_or(&t).to_string()))
    }

    fn save_stash_entry(&self, mediagit_dir: &Path, entry: StashEntry) -> Result<()> {
        let mut stash_list = self.load_all_stashes(mediagit_dir)?;
        stash_list.insert(0, entry); // Add to front
        self.save_all_stashes(mediagit_dir, &stash_list)
    }

    fn load_stash_entry(&self, mediagit_dir: &std::path::Path, index: usize) -> Result<StashEntry> {
        let stash_list = self.load_all_stashes(mediagit_dir)?;

        stash_list
            .get(index)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("Stash entry {} not found", index))
    }

    fn load_all_stashes(&self, mediagit_dir: &std::path::Path) -> Result<Vec<StashEntry>> {
        let stash_path = mediagit_dir.join("STASH_LIST");

        if !stash_path.exists() {
            return Ok(Vec::new());
        }

        // nosemgrep: rust.actix.path-traversal.tainted-path.tainted-path -- repo-internal constant path
        let stash_json = std::fs::read_to_string(&stash_path)?;
        let stash_list: Vec<StashEntry> = serde_json::from_str(&stash_json)?;

        Ok(stash_list)
    }

    fn save_all_stashes(
        &self,
        mediagit_dir: &std::path::Path,
        stash_list: &[StashEntry],
    ) -> Result<()> {
        let stash_path = mediagit_dir.join("STASH_LIST");
        let stash_json = serde_json::to_string_pretty(stash_list)?;
        std::fs::write(&stash_path, stash_json)?;

        Ok(())
    }

    /// Scan working directory for files (excluding .mediagit)
    fn scan_working_directory(&self, repo_root: &Path) -> Result<HashSet<PathBuf>> {
        let mut files = HashSet::new();
        self.scan_directory_recursive(repo_root, repo_root, &mut files)?;
        Ok(files)
    }

    #[allow(clippy::only_used_in_recursion)]
    fn scan_directory_recursive(
        &self,
        repo_root: &Path,
        current_dir: &Path,
        files: &mut HashSet<PathBuf>,
    ) -> Result<()> {
        // nosemgrep: rust.actix.path-traversal.tainted-path.tainted-path -- local worktree walk
        for entry in std::fs::read_dir(current_dir)? {
            let entry = entry?;
            let path = entry.path();

            // Skip .mediagit directory
            if path.file_name().and_then(|n| n.to_str()) == Some(".mediagit") {
                continue;
            }

            if path.is_file() {
                if let Ok(rel_path) = path.strip_prefix(repo_root) {
                    let normalized = PathBuf::from(rel_path.to_string_lossy().replace('\\', "/"));
                    files.insert(normalized);
                }
            } else if path.is_dir() {
                self.scan_directory_recursive(repo_root, &path, files)?;
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct StashEntry {
    commit_oid: String,
    message: String,
    timestamp: String,
    branch: Option<String>,
    /// Paths that were staged when stashed, so `apply --index` can restage them.
    #[serde(default)]
    staged: Vec<String>,
}

#[cfg(test)]
#[allow(unsafe_code)] // edition-2024: test-only env::set_var/remove_var requires unsafe
mod tests {
    use super::*;
    use crate::commands::utils::test_support::{REPO_ENV_LOCK, init_repo_with_commit};
    use clap::Parser;
    use tempfile::TempDir;

    fn parse(args: &[&str]) -> Result<StashCmd, clap::Error> {
        let mut full = vec!["stash"];
        full.extend_from_slice(args);
        StashCmd::try_parse_from(full)
    }

    #[test]
    fn parse_save_with_message_flag() {
        let cmd = parse(&["save", "-m", "WIP"]).unwrap();
        match cmd.command {
            StashSubcommand::Save(opts) => {
                assert_eq!(opts.message_flag.as_deref(), Some("WIP"));
                assert!(!opts.include_untracked);
            }
            _ => panic!("expected Save"),
        }
    }

    #[test]
    fn parse_push_positionals_are_paths() {
        let cmd = parse(&["push", "-m", "WIP", "a.txt", "dir", "-u"]).unwrap();
        match cmd.command {
            StashSubcommand::Push(opts) => {
                assert_eq!(opts.message.as_deref(), Some("WIP"));
                assert_eq!(opts.paths, [PathBuf::from("a.txt"), PathBuf::from("dir")]);
                assert!(opts.include_untracked);
            }
            _ => panic!("expected Push"),
        }
    }

    #[test]
    fn parse_apply_with_index() {
        let cmd = parse(&["apply", "1", "--index"]).unwrap();
        match cmd.command {
            StashSubcommand::Apply(opts) => {
                assert_eq!(opts.stash, Some(1));
                assert!(opts.index);
            }
            _ => panic!("expected Apply"),
        }
    }

    #[test]
    fn parse_list_verbose() {
        let cmd = parse(&["list", "-v"]).unwrap();
        match cmd.command {
            StashSubcommand::List(opts) => assert!(opts.verbose),
            _ => panic!("expected List"),
        }
    }

    #[test]
    fn parse_drop_and_pop_default_to_no_index() {
        let cmd = parse(&["drop"]).unwrap();
        match cmd.command {
            StashSubcommand::Drop(opts) => assert_eq!(opts.stash, None),
            _ => panic!("expected Drop"),
        }

        let cmd = parse(&["pop", "2"]).unwrap();
        match cmd.command {
            StashSubcommand::Pop(opts) => assert_eq!(opts.stash, Some(2)),
            _ => panic!("expected Pop"),
        }
    }

    #[test]
    fn parse_clear_force() {
        let cmd = parse(&["clear", "-f"]).unwrap();
        match cmd.command {
            StashSubcommand::Clear(opts) => assert!(opts.force),
            _ => panic!("expected Clear"),
        }
    }

    #[test]
    fn parse_missing_subcommand_is_error() {
        assert!(parse(&[]).is_err());
    }

    /// Guards `MEDIAGIT_REPO` across the `.await` points in `execute()`
    /// (see `REPO_ENV_LOCK` docs).
    #[allow(clippy::await_holding_lock)]
    async fn execute_in(repo_path: &std::path::Path, cmd: &StashCmd) -> Result<()> {
        let _guard = REPO_ENV_LOCK.lock().unwrap();
        // FIXME: Audit that the environment access only happens in single-threaded code.
        unsafe { std::env::set_var("MEDIAGIT_REPO", repo_path) };
        let result = cmd.execute().await;
        // FIXME: Audit that the environment access only happens in single-threaded code.
        unsafe { std::env::remove_var("MEDIAGIT_REPO") };
        result
    }

    #[tokio::test]
    async fn execute_no_repo_is_error() {
        let temp = TempDir::new().unwrap();
        let cmd = parse(&["list"]).unwrap();
        let err = execute_in(temp.path(), &cmd).await.unwrap_err();
        assert!(err.to_string().contains("Not a mediagit repository"));
    }

    #[tokio::test]
    async fn list_with_no_stashes_is_ok() {
        let temp = TempDir::new().unwrap();
        init_repo_with_commit(temp.path()).await;

        let cmd = parse(&["list"]).unwrap();
        execute_in(temp.path(), &cmd).await.unwrap();
    }

    #[tokio::test]
    async fn save_with_no_changes_is_ok() {
        let temp = TempDir::new().unwrap();
        init_repo_with_commit(temp.path()).await;

        let cmd = parse(&["save", "-m", "nothing to stash"]).unwrap();
        execute_in(temp.path(), &cmd).await.unwrap();
    }

    #[tokio::test]
    async fn drop_out_of_range_is_error() {
        let temp = TempDir::new().unwrap();
        init_repo_with_commit(temp.path()).await;

        let cmd = parse(&["drop", "0"]).unwrap();
        let err = execute_in(temp.path(), &cmd).await.unwrap_err();
        assert!(err.to_string().contains("Stash entry 0 not found"));
    }

    #[tokio::test]
    async fn apply_out_of_range_is_error() {
        let temp = TempDir::new().unwrap();
        init_repo_with_commit(temp.path()).await;

        let cmd = parse(&["apply", "0"]).unwrap();
        let err = execute_in(temp.path(), &cmd).await.unwrap_err();
        assert!(err.to_string().contains("Stash entry 0 not found"));
    }

    #[tokio::test]
    async fn show_out_of_range_is_error() {
        let temp = TempDir::new().unwrap();
        init_repo_with_commit(temp.path()).await;

        let cmd = parse(&["show", "0"]).unwrap();
        let err = execute_in(temp.path(), &cmd).await.unwrap_err();
        assert!(err.to_string().contains("Stash entry 0 not found"));
    }

    /// Commit `m.txt`, then stage a modification of it and a new `n.txt`.
    async fn stage_modified_and_new(root: &Path) {
        use mediagit_versioning::{FileMode, IndexEntry, Ref, Signature, TreeEntry};
        let storage = create_storage_backend(root).await.unwrap();
        let odb = ObjectDatabase::with_smart_compression(storage, 1000);
        let refdb = RefDatabase::new(root.join(".mediagit"));
        let base = odb.write(ObjectType::Blob, b"one").await.unwrap();
        let mut tree = Tree::new();
        tree.add_entry(TreeEntry::new("m.txt".into(), FileMode::Regular, base));
        let tree_oid = tree.write(&odb).await.unwrap();
        let sig = Signature::now("T".to_string(), "t@example.com".to_string());
        let parent = refdb.resolve("refs/heads/main").await.unwrap();
        let mut commit = Commit::new(tree_oid, sig.clone(), sig, "add m".to_string());
        commit.parents = vec![parent];
        let oid = commit.write(&odb).await.unwrap();
        refdb
            .write(&Ref::new_direct("refs/heads/main".to_string(), oid))
            .await
            .unwrap();
        std::fs::write(root.join("m.txt"), "two").unwrap();
        std::fs::write(root.join("n.txt"), "new").unwrap();
        let mut index = Index::new();
        for (name, body) in [("m.txt", "two"), ("n.txt", "new")] {
            let blob = odb.write(ObjectType::Blob, body.as_bytes()).await.unwrap();
            index.add_entry(IndexEntry::new(
                PathBuf::from(name),
                blob,
                0o100644,
                3,
                None,
            ));
        }
        index.save(root).unwrap();
    }

    fn staged_paths(root: &Path) -> Vec<String> {
        let mut v: Vec<String> = Index::load(root)
            .unwrap()
            .entries()
            .map(|e| e.path.to_string_lossy().into_owned())
            .collect();
        v.sort();
        v
    }

    #[tokio::test]
    async fn apply_and_pop_index_restage_only_with_flag() {
        for sub in ["apply", "pop"] {
            for with_index in [true, false] {
                let temp = TempDir::new().unwrap();
                init_repo_with_commit(temp.path()).await;
                stage_modified_and_new(temp.path()).await;
                execute_in(temp.path(), &parse(&["save", "-m", "s"]).unwrap())
                    .await
                    .unwrap();
                assert!(staged_paths(temp.path()).is_empty());

                let mut args = vec![sub];
                if with_index {
                    args.push("--index");
                }
                execute_in(temp.path(), &parse(&args).unwrap())
                    .await
                    .unwrap();
                let want: Vec<String> = if with_index {
                    vec!["m.txt".into(), "n.txt".into()]
                } else {
                    vec![]
                };
                assert_eq!(staged_paths(temp.path()), want, "{sub} index={with_index}");
                assert_eq!(
                    std::fs::read_to_string(temp.path().join("m.txt")).unwrap(),
                    "two"
                );
            }
        }
    }

    /// A worktree file at or above the streaming threshold must be chunked
    /// straight from disk. The old path read it whole and stored it with
    /// `odb.write`, which never produces a manifest, so `is_chunked` is the
    /// observable difference.
    #[tokio::test]
    async fn save_stores_large_file_chunked() {
        let temp = TempDir::new().unwrap();
        init_repo_with_commit(temp.path()).await;

        let data: Vec<u8> = (0..6 * 1024 * 1024)
            .map(|i| ((i * 7) % 251) as u8)
            .collect();
        std::fs::write(temp.path().join("big.bin"), &data).unwrap();
        let expected = Oid::hash(&data);

        execute_in(temp.path(), &parse(&["save", "-m", "big"]).unwrap())
            .await
            .unwrap();

        let storage = create_storage_backend(temp.path()).await.unwrap();
        let odb = ObjectDatabase::with_smart_compression(storage, 100);
        assert!(
            odb.is_chunked(&expected).await.unwrap(),
            "large stashed file must be stored chunked"
        );
    }
}
