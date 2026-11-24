// SPDX-License-Identifier: BUSL-1.1
// Copyright (C) 2025-2026 Aswin Krishnamoorthy

//! `restore`: put chosen files back to a saved version (default: the latest).
//!
//! Throwing away work is never silent. Nothing is changed until the person
//! confirms: a y/N prompt that lists every file (default No), or `--yes`.
//! `--machine` and non-terminal runs without `--yes` are refused before any
//! file is touched. Files that are not in the saved version (new files) are
//! only deleted with `--delete-new` as well.

use anyhow::{Context, Result};
use clap::Parser;
use console::style;
use mediagit_versioning::{
    CheckoutManager, FileMode, Index, ObjectDatabase, Oid, RefDatabase, resolve_revision,
};
use serde::Serialize;
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

use super::super::output;
use super::super::repo::{create_storage_backend, find_repo_root, repo_relative};

/// Put files back to a saved version, throwing away their unsaved changes
#[derive(Parser, Debug)]
#[command(after_help = "EXAMPLES:
    # Throw away the edits to one file (asks first)
    mediagit restore -- art/hero.psd

    # Take a file from an older version
    mediagit restore --source HEAD~2 -- art/hero.psd

    # Also delete new files that were never saved (asks first)
    mediagit restore --delete-new -- render/tmp.exr

    # Scripts: confirm up front
    mediagit restore --yes -- art/hero.psd

Unsaved changes are lost for good: MediaGit has no copy of them.")]
pub struct RestoreCmd {
    /// Version to take the files from (default: HEAD, the latest saved version)
    #[arg(long, value_name = "REV")]
    pub source: Option<String>,

    /// Also delete files that are not in that version (new files). They are gone for good
    #[arg(long)]
    pub delete_new: bool,

    /// Do not ask for confirmation (required with --machine and in scripts)
    #[arg(short = 'y', long)]
    pub yes: bool,

    /// Quiet mode
    #[arg(short, long)]
    pub quiet: bool,

    /// Files to restore (files only, not folders)
    #[arg(value_name = "PATHS", required = true)]
    pub paths: Vec<PathBuf>,
}

/// `--machine` data for `restore`.
#[derive(Debug, Clone, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[cfg_attr(test, ts(export, export_to = "../../../ui/src/api/generated/"))]
pub struct RestoreReport {
    /// The version the files came from (commit id).
    pub source: String,
    /// Files written back from the source version.
    pub restored: Vec<String>,
    /// New files deleted (`--delete-new`).
    pub deleted: Vec<String>,
    /// Files whose staged change was dropped.
    pub unstaged: Vec<String>,
    /// Files that could not be restored or deleted, with the reason.
    pub failed: Vec<RestoreFailure>,
}

#[derive(Debug, Clone, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[cfg_attr(test, ts(export, export_to = "../../../ui/src/api/generated/"))]
pub struct RestoreFailure {
    pub path: String,
    pub message: String,
}

/// What happens to one path.
enum Step {
    /// Write the source version back (unsaved edits are lost).
    Restore(Oid, FileMode),
    /// Not in the source: delete it (needs `--delete-new`).
    Delete,
}

impl RestoreCmd {
    fn quiet(&self) -> bool {
        self.quiet || output::machine_mode()
    }

    pub async fn execute(&self) -> Result<()> {
        let repo_root = find_repo_root()?;
        let storage = create_storage_backend(&repo_root).await?;
        let refdb = RefDatabase::new(repo_root.join(".mediagit"));
        let odb = ObjectDatabase::with_smart_compression(storage, 1000);
        let checkout = CheckoutManager::new(&odb, &repo_root);

        // The source version and its files. An unborn HEAD has none.
        let rev = self.source.as_deref().unwrap_or("HEAD");
        let source = match resolve_revision(rev, &refdb, &odb).await {
            Ok(oid) => Some(super::show::ShowCmd::peel_to_commit(oid, &odb).await?),
            // Only a branch with no commits yet means "nothing saved": any
            // other failure to read HEAD must not turn tracked files into
            // "new" ones (which --delete-new would delete).
            Err(_) if self.source.is_none() && head_is_unborn(&refdb).await? => None,
            Err(e) => {
                return Err(output::CodedError::new(
                    "invalid_args",
                    format!("Cannot find version '{rev}': {e:#}"),
                )
                .into());
            }
        };
        let source_files: HashMap<String, (Oid, FileMode)> = match &source {
            Some(c) => checkout
                .tracked_files_at(c)
                .await?
                .into_iter()
                .map(|(p, v)| (key_of(&p), v))
                .collect(),
            None => HashMap::new(),
        };

        // Plan first (read-only): a refusal or a "no" changes nothing.
        let index = Index::load(&repo_root)?;
        let cwd = std::env::current_dir()?;
        let mut plan: BTreeMap<String, Step> = BTreeMap::new();
        for p in &self.paths {
            let invalid =
                |e: anyhow::Error| output::CodedError::new("invalid_args", format!("{e:#}"));
            let spelled = repo_relative(&repo_root, &cwd.join(p)).map_err(invalid)?;
            // What the file system really names it: a folder that links out of
            // the repo is refused, and alias names (`MEDIAG~1`, `.mediagit.`,
            // other letter case) become the real name before any check.
            let rel = on_disk_rel(&repo_root, &repo_root.join(&spelled)).map_err(invalid)?;
            let key = key_of(&rel);
            // The repository's own data is never a file to restore or delete
            // (case-insensitive: Windows and macOS fold `.MediaGit`).
            if key
                .split('/')
                .next()
                .is_some_and(|c| c.eq_ignore_ascii_case(".mediagit"))
            {
                return Err(output::CodedError::new(
                    "invalid_args",
                    format!("'{key}' is MediaGit's own data, not a file in the project"),
                )
                .into());
            }
            let abs = repo_root.join(&rel);
            if abs.is_dir() {
                return Err(output::CodedError::new(
                    "invalid_args",
                    format!("'{key}' is a folder; name the files to restore"),
                )
                .into());
            }
            let step = match source_files.get(&key) {
                Some((oid, mode)) => Step::Restore(*oid, *mode),
                None if abs.symlink_metadata().is_ok() || index.contains(&rel) => Step::Delete,
                None => {
                    return Err(output::CodedError::new(
                        "invalid_args",
                        format!("'{key}' is not in version {rev} and not in the working folder"),
                    )
                    .into());
                }
            };
            plan.insert(key, step);
        }

        let (restore, delete): (Vec<_>, Vec<_>) = plan
            .iter()
            .partition(|(_, s)| matches!(s, Step::Restore(..)));
        if !delete.is_empty() && !self.delete_new {
            let names: Vec<&str> = delete.iter().map(|(k, _)| k.as_str()).collect();
            return Err(output::CodedError::new(
                "would_delete",
                format!(
                    "{} file(s) are not in version {rev}, so restoring means deleting them: {}. \
                     Pass --delete-new to delete them for good.",
                    names.len(),
                    names.join(", ")
                ),
            )
            .into());
        }

        if !self.yes {
            self.confirm(&repo_root, &restore, &delete)?;
        }
        // Read again after the prompt: an `add` made while it waited stays.
        let mut index = Index::load(&repo_root)?;

        // Act: every path is tried; failures are collected, not fatal.
        let mut report = RestoreReport {
            source: source.map(|o| o.to_hex()).unwrap_or_default(),
            restored: Vec::new(),
            deleted: Vec::new(),
            unstaged: Vec::new(),
            failed: Vec::new(),
        };
        for (key, step) in &plan {
            let rel = PathBuf::from(key);
            let done = match step {
                Step::Restore(oid, mode) => checkout.materialize_file(&rel, oid, *mode).await,
                Step::Delete => delete_new_file(&repo_root, &rel),
            };
            match done {
                Ok(()) => {
                    let staged = index.remove_entry(&rel).is_some();
                    let unmarked = index.unmark_deleted(&rel);
                    if staged || unmarked {
                        report.unstaged.push(key.clone());
                    }
                    match step {
                        Step::Restore(..) => report.restored.push(key.clone()),
                        Step::Delete => report.deleted.push(key.clone()),
                    }
                }
                Err(e) => report.failed.push(RestoreFailure {
                    path: key.clone(),
                    message: plain(&e),
                }),
            }
        }
        index.save(&repo_root)?;

        if !self.quiet() {
            for p in &report.restored {
                println!("{} Restored {}", style("✓").green(), p);
            }
            for p in &report.deleted {
                println!("{} Deleted {}", style("✓").green(), p);
            }
            for f in &report.failed {
                eprintln!("{} {}: {}", style("✗").red(), f.path, f.message);
            }
        }
        let failed = !report.failed.is_empty();
        if output::machine_mode() {
            output::emit_ok(&report)?;
            if failed {
                std::process::exit(1);
            }
            return Ok(());
        }
        if failed {
            anyhow::bail!(
                "{} file(s) could not be restored; the others were",
                report.failed.len()
            );
        }
        Ok(())
    }

    /// Ask before throwing anything away. Lists every file and the total size.
    fn confirm(
        &self,
        repo_root: &Path,
        restore: &[(&String, &Step)],
        delete: &[(&String, &Step)],
    ) -> Result<()> {
        output::ensure_interactive("restore confirmation (pass --yes)")?;
        if !std::io::IsTerminal::is_terminal(&std::io::stdin()) {
            anyhow::bail!("stdin is not a terminal; pass --yes to restore without asking");
        }
        let size = |k: &str| {
            std::fs::metadata(repo_root.join(k))
                .map(|m| m.len())
                .unwrap_or(0)
        };
        let mut total = 0u64;
        if !restore.is_empty() {
            println!("Your edits to these files are lost:");
            for (k, _) in restore {
                let n = size(k);
                total += n;
                println!("  {k} ({n} bytes)");
            }
        }
        if !delete.is_empty() {
            println!("These files were never saved and are deleted:");
            for (k, _) in delete {
                let n = size(k);
                total += n;
                println!("  {k} ({n} bytes)");
            }
        }
        let confirmed = dialoguer::Confirm::new()
            .with_prompt(format!(
                "Throw away {} file(s) ({total} bytes)? This cannot be undone.",
                restore.len() + delete.len()
            ))
            .default(false)
            .interact()?;
        if !confirmed {
            anyhow::bail!("cancelled; nothing was changed");
        }
        Ok(())
    }
}

/// A repo-relative path as the index and trees spell it (`/`-separated on
/// Windows; elsewhere a backslash is an ordinary name character).
fn key_of(p: &Path) -> String {
    let s = p.to_string_lossy();
    if cfg!(windows) {
        s.replace('\\', "/")
    } else {
        s.into_owned()
    }
}

/// `abs` relative to the repo root, as the file system names it. Every folder
/// on the way is resolved (links, junctions, short and case-folded names); the
/// file's own name is taken as stored unless it is a link itself, which is
/// restored or deleted as a link. Errors when the path leaves the repository.
fn on_disk_rel(repo_root: &Path, abs: &Path) -> Result<PathBuf> {
    let root = dunce::canonicalize(repo_root)?;
    let name = abs.file_name().context("no file name")?;
    let mut dir = abs.parent().context("no folder")?;
    // Folders not on disk yet (a deleted file's deleted folder) keep their spelling.
    let mut missing = Vec::new();
    while !dir.is_dir() {
        missing.push(dir.file_name().context("no folder name")?.to_owned());
        dir = dir.parent().context("no folder")?;
    }
    let real = dunce::canonicalize(dir)?;
    let mut rel = real
        .strip_prefix(&root)
        .map_err(|_| anyhow::anyhow!("'{}' leaves the repository through a link", abs.display()))?
        .to_path_buf();
    rel.extend(missing.iter().rev());
    let stored = match abs.symlink_metadata() {
        Ok(m) if !m.file_type().is_symlink() => dunce::canonicalize(abs)
            .ok()
            .and_then(|c| c.file_name().map(|n| n.to_owned())),
        _ => None,
    };
    rel.push(stored.as_deref().unwrap_or(name));
    if rel.as_os_str().is_empty() {
        anyhow::bail!("'{}' is the repository folder itself", abs.display());
    }
    Ok(rel)
}

/// HEAD names a branch that has no commit yet (a new repository).
async fn head_is_unborn(refdb: &RefDatabase) -> Result<bool> {
    match refdb.read("HEAD").await?.target {
        Some(branch) => Ok(!refdb.exists(&branch).await?),
        None => Ok(false),
    }
}

/// Delete one new file, then any folders it leaves empty (never the repo root).
fn delete_new_file(repo_root: &Path, rel: &Path) -> Result<()> {
    let abs = repo_root.join(rel);
    match std::fs::remove_file(&abs) {
        Ok(()) => {}
        // Staged but already gone from disk: dropping the index entry is the job.
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e).with_context(|| format!("Cannot delete {}", rel.display())),
    }
    let mut dir = abs.parent();
    while let Some(d) = dir {
        if d == repo_root || std::fs::remove_dir(d).is_err() {
            break; // not empty, or the root
        }
        dir = d.parent();
    }
    Ok(())
}

/// One short, safe line for a failed file.
fn plain(e: &anyhow::Error) -> String {
    mediagit_cli::friendly::friendly(e)
        .map(|f| f.message)
        .unwrap_or_else(|| mediagit_cli::friendly::scrubbed_chain(e))
}
