// SPDX-License-Identifier: BUSL-1.1
// Copyright (C) 2025-2026 Aswin Krishnamoorthy

//! File System Check (FSCK) command - Repository integrity verification

use crate::repo::create_storage_backend;
use anyhow::{Context, Result};
use clap::Parser;
use console::style;
use mediagit_versioning::{
    FsckChecker, FsckOptions, FsckRepair, IssueSeverity, QuarantineStatus, RefDatabase,
    RepairOutcome, list_quarantine, sweep_quarantine,
};
use serde::Serialize;

use crate::output;

/// Check repository integrity with comprehensive verification
///
/// The fsck command performs a comprehensive integrity check of your MediaGit repository:
/// - Verifies BLAKE3 checksums of all objects
/// - Validates references point to existing commits
/// - Checks commit graph connectivity
/// - Detects missing or corrupted objects
/// - Optionally finds dangling (unreferenced) objects
#[derive(Parser, Debug)]
#[command(after_help = "EXAMPLES:
    # Basic integrity check
    mediagit fsck

    # Full check including dangling objects
    mediagit fsck --full

    # Quick check (objects and refs only)
    mediagit fsck --quick

    # Repair mode (fix repairable issues)
    mediagit fsck --repair

    # Dry-run repair to see what would be fixed
    mediagit fsck --repair --dry-run

FSCK vs VERIFY:
    fsck    - Comprehensive integrity check (full graph analysis)
            - Checks connectivity, finds dangling/unreachable objects
            - Supports repair mode (--repair)
            - Use for thorough repository audits and recovery

    verify  - Fast integrity check (checksums + refs only)
            - Skips connectivity and dangling object checks
            - Use for quick health checks and CI pipelines

SEE ALSO:
    mediagit-verify(1), mediagit-gc(1)")]
pub struct FsckCmd {
    /// Full check including dangling objects (slower)
    #[arg(long)]
    pub full: bool,

    /// Quick check (objects and refs only, no connectivity)
    #[arg(long)]
    pub quick: bool,

    /// Show all objects checked
    #[arg(long)]
    pub all: bool,

    /// Show lost/dangling objects
    #[arg(long)]
    pub lost_found: bool,

    /// Don't check for dangling objects
    #[arg(long)]
    pub no_dangling: bool,

    /// Attempt to repair issues automatically
    #[arg(long)]
    pub repair: bool,

    /// Dry run (show what would be repaired without making changes)
    #[arg(long)]
    pub dry_run: bool,

    /// Delete every copy in .mediagit/quarantine (asks first; runs no check or repair)
    #[arg(long)]
    pub purge_quarantine: bool,

    /// Do not ask for confirmation (with --purge-quarantine)
    #[arg(long)]
    pub yes: bool,

    /// Limit number of objects to check (0 = unlimited)
    #[arg(long, default_value = "0")]
    pub max_objects: u64,

    /// Quiet mode (only errors)
    #[arg(short, long)]
    pub quiet: bool,

    /// Verbose mode (detailed progress)
    #[arg(short, long)]
    pub verbose: bool,

    /// Same as -C/--repository (kept for old scripts)
    #[arg(long, value_name = "PATH", hide = true)]
    pub path: Option<String>,
}

/// `--machine` data for `fsck`. Errors found: `healthy:false` and exit 1
/// (the envelope is still `ok:true` so the issue list is not lost).
#[derive(Debug, Clone, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[cfg_attr(test, ts(export, export_to = "../../../ui/src/api/generated/"))]
pub struct FsckCliReport {
    /// No issue of severity `error`.
    pub healthy: bool,
    #[cfg_attr(test, ts(type = "number"))]
    pub objects_checked: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub refs_checked: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub corrupted_objects: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub broken_refs: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub missing_objects: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub dangling_objects: u64,
    /// `--repair`: issues actually repaired (`--dry-run`: that would be).
    #[cfg_attr(test, ts(type = "number | null"))]
    pub repaired: Option<u64>,
    /// `--repair`: damaged objects rewritten from a good copy (confirmed by a re-check).
    #[cfg_attr(test, ts(type = "number | null"))]
    pub restored: Option<u64>,
    /// `--repair`: damaged objects moved to `.mediagit/quarantine`.
    #[cfg_attr(test, ts(type = "number | null"))]
    pub quarantined: Option<u64>,
    /// `--repair`: broken refs removed (confirmed by a re-check).
    #[cfg_attr(test, ts(type = "number | null"))]
    pub refs_removed: Option<u64>,
    /// `--repair`: files that can no longer be restored (empty otherwise).
    pub unrestorable_files: Vec<String>,
    /// `--repair`: things that went wrong mid-repair (empty otherwise).
    pub repair_errors: Vec<String>,
    /// Damaged copies held in `.mediagit/quarantine`.
    #[cfg_attr(test, ts(type = "number"))]
    pub quarantine_count: u64,
    /// Total size of those copies.
    #[cfg_attr(test, ts(type = "number"))]
    pub quarantine_bytes: u64,
    pub issues: Vec<FsckIssueReport>,
}

/// `--machine` data for `fsck --purge-quarantine`.
#[derive(Debug, Clone, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[cfg_attr(test, ts(export, export_to = "../../../ui/src/api/generated/"))]
pub struct FsckPurgeReport {
    #[cfg_attr(test, ts(type = "number"))]
    pub deleted_count: u64,
    #[cfg_attr(test, ts(type = "number"))]
    pub deleted_bytes: u64,
    /// Files that need a purged copy and so can no longer be restored.
    pub unrestorable_files: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[cfg_attr(test, ts(export, export_to = "../../../ui/src/api/generated/"))]
pub struct FsckIssueReport {
    /// `info`, `warning` or `error`.
    pub severity: String,
    pub category: String,
    pub message: String,
    pub oid: Option<String>,
    pub ref_name: Option<String>,
    pub repairable: bool,
}

/// Emit the machine report for `fsck` / `verify`; exits 1 when unhealthy.
pub(super) fn emit_integrity(
    report: &mediagit_versioning::FsckReport,
    repair: Option<&RepairOutcome>,
    quarantine: &QuarantineStatus,
) -> Result<()> {
    let lost_files =
        repair.is_some_and(|o| !o.unrestorable_files.is_empty() || !o.errors.is_empty());
    let healthy = !report.has_errors() && !lost_files;
    output::emit_ok(&FsckCliReport {
        healthy,
        objects_checked: report.objects_checked,
        refs_checked: report.refs_checked,
        corrupted_objects: report.corrupted_objects,
        broken_refs: report.broken_refs,
        missing_objects: report.missing_objects,
        dangling_objects: report.dangling_objects,
        repaired: repair.map(RepairOutcome::repaired),
        restored: repair.map(|o| o.restored.len() as u64),
        quarantined: repair.map(|o| o.quarantined.len() as u64),
        refs_removed: repair.map(|o| o.refs_removed.len() as u64),
        unrestorable_files: repair.map_or_else(Vec::new, |o| o.unrestorable_files.clone()),
        repair_errors: repair.map_or_else(Vec::new, |o| o.errors.clone()),
        quarantine_count: quarantine.count,
        quarantine_bytes: quarantine.bytes,
        issues: report
            .issues
            .iter()
            .map(|i| FsckIssueReport {
                severity: format!("{:?}", i.severity).to_lowercase(),
                category: format!("{:?}", i.category),
                message: i.message.clone(),
                oid: i.oid.map(|o| o.to_hex()),
                ref_name: i.ref_name.clone(),
                repairable: i.repairable,
            })
            .collect(),
    })?;
    if !healthy {
        std::process::exit(1);
    }
    Ok(())
}

/// One plain line, on stdout, or stderr in `--machine` (stdout is one JSON line).
fn note(msg: &str) {
    if output::machine_mode() {
        eprintln!("{msg}");
    } else {
        println!("  {msg}");
    }
}

fn describe_remote_error(remote: &str, e: &anyhow::Error) -> String {
    let text = crate::output::scrub(&format!("{e:#}"));
    let lower = text.to_lowercase();
    if ["401", "403", "unauthorized", "forbidden"]
        .iter()
        .any(|k| lower.contains(k))
    {
        format!("The server for '{remote}' refused the login (authentication failed).")
    } else if [
        "connect",
        "error sending request",
        "dns",
        "timed out",
        "refused",
    ]
    .iter()
    .any(|k| lower.contains(k))
    {
        let first = text.lines().next().unwrap_or("");
        format!("The server for '{remote}' could not be reached: {first}")
    } else {
        format!("The server for '{remote}' returned an error: {text}")
    }
}

/// A ref that repair removed (its commit was missing) comes back when the
/// remote delivered that commit: the saved value under quarantine/refs is the
/// pointer. The saved copy goes with it.
async fn restore_removed_refs(
    refdb: &RefDatabase,
    odb: &mediagit_versioning::ObjectDatabase,
    done: &mut RepairOutcome,
) {
    for (name, saved) in done.saved_refs.clone() {
        if !done.refs_removed.contains(&name) {
            continue;
        }
        let Ok(text) = tokio::fs::read_to_string(&saved).await else {
            continue;
        };
        let Ok(oid) = mediagit_versioning::Oid::from_hex(text.trim()) else {
            continue;
        };
        // A real read that parses as a commit, not just "the file exists".
        let readable = match odb.read(&oid).await {
            Ok(bytes) => mediagit_versioning::format::deserialize_exact::<
                mediagit_versioning::Commit,
            >(&bytes)
            .is_ok(),
            Err(_) => false,
        };
        if !readable {
            continue;
        }
        let r = mediagit_versioning::Ref::new_direct(name.clone(), oid);
        if refdb.write(&r).await.is_ok() {
            let _ = tokio::fs::remove_file(&saved).await;
            done.refs_removed.retain(|n| n != &name);
            done.restored.push(oid);
            note(&format!("Restored reference {name} from the remote."));
        }
    }
}

/// Try to fetch what no local source could restore. Every failure is one
/// plain line and falls through to quarantine; nothing here may hang (the
/// client's own connect, read and pull-deadline timeouts apply).
async fn restore_from_remote(
    repo_path: &std::path::Path,
    mediagit_dir: &std::path::Path,
    repair: &FsckRepair,
    done: &mut RepairOutcome,
) {
    let config = match mediagit_config::Config::load(repo_path).await {
        Ok(c) => c,
        Err(e) => {
            note(&format!(
                "The repository config could not be read, so no remote was tried: {e}"
            ));
            return;
        }
    };
    let name = if config.remotes.contains_key("origin") {
        "origin".to_string()
    } else if config.remotes.len() == 1 {
        config.remotes.keys().next().cloned().unwrap_or_default()
    } else if config.remotes.is_empty() {
        note("No remote is configured, so nothing could be fetched.");
        return;
    } else {
        note("Several remotes are configured and none is called origin, so none was tried.");
        return;
    };
    let url = match config.resolve_remote_url(&name) {
        Ok(u) => u,
        Err(e) => {
            note(&format!("The URL of remote '{name}' is unusable: {e}"));
            return;
        }
    };
    let client = mediagit_protocol::ProtocolClient::new(url)
        .with_credentials(crate::repo::resolve_credentials(repo_path, &config, &name));
    let refs = match client.get_refs().await {
        Ok(r) => r.refs,
        Err(e) => {
            note(&describe_remote_error(&name, &e));
            return;
        }
    };
    if let Err(e) = crate::encryption::verify_remote_key_compatible(repo_path, &client).await {
        note(&format!("Remote '{name}' cannot be used: {e}"));
        return;
    }

    // Current branch first, then the rest.
    let current = RefDatabase::new(mediagit_dir)
        .read("HEAD")
        .await
        .ok()
        .and_then(|h| h.target);
    let mut branches: Vec<String> = refs
        .iter()
        .filter(|r| r.name.starts_with("refs/heads/"))
        .map(|r| r.name.clone())
        .collect();
    branches.sort_by_key(|b| Some(b) != current.as_ref());

    let Some(odb) = repair.odb() else { return };
    let refdb = RefDatabase::new(mediagit_dir);
    for branch in branches {
        match client.pull_streaming(&odb, &branch, vec![]).await {
            Ok(chunked) if !chunked.is_empty() => {
                if let Err(e) = client
                    .download_chunked_objects(&odb, &chunked, |_, _, _| {})
                    .await
                {
                    note(&describe_remote_error(&name, &e));
                    break;
                }
            }
            Ok(_) => {}
            Err(e) => {
                note(&describe_remote_error(&name, &e));
                break;
            }
        }
        let mut healthy = Vec::new();
        for oid in done.lost_oids() {
            if repair.is_healthy(&oid).await {
                healthy.push(oid);
            }
        }
        done.mark_restored(&healthy);
        restore_removed_refs(&refdb, &odb, done).await;
        if done.lost_oids().is_empty() && done.refs_removed.is_empty() {
            return;
        }
    }
    if !done.lost_oids().is_empty() {
        note(&format!(
            "The server for '{name}' did not have a good copy of {} object(s).",
            done.lost_oids().len()
        ));
    }
}

impl FsckCmd {
    fn quiet(&self) -> bool {
        self.quiet || output::machine_mode()
    }

    pub async fn execute(&self) -> Result<()> {
        // Determine repository path
        let repo_path = match self.path.as_deref() {
            Some(p) => crate::repo::find_repo_root_from(std::path::Path::new(p))?,
            None => crate::repo::find_repo_root()?,
        };
        let mediagit_dir = repo_path.join(".mediagit");

        if self.purge_quarantine {
            return self.purge_quarantine(&repo_path, &mediagit_dir).await;
        }

        if !self.quiet() {
            println!(
                "{} Checking repository integrity at {}",
                style("🔍").cyan().bold(),
                style(&mediagit_dir.display()).dim()
            );
        }

        // Create storage backend
        let storage = create_storage_backend(&repo_path)
            .await
            .context("Failed to open repository. Is this a MediaGit repository?")?;

        // Create RefDatabase for accurate ref listing
        let refdb = RefDatabase::new(&mediagit_dir);

        // Create FSCK checker with RefDatabase. The repo root (not
        // `.mediagit`) is what dangling detection needs: it protects staged,
        // stashed, mid-operation and reflog-referenced objects, which `gc`
        // protects too and `--repair` would otherwise delete.
        let checker =
            FsckChecker::new_with_refdb(storage.clone(), Some(refdb)).with_repo_root(&repo_path);

        // BUG-006: a branch whose commit is now here is no longer "not
        // downloaded yet". A dry run changes nothing.
        if !self.dry_run {
            crate::repo::prune_lazy_refs(&repo_path, &checker.odb()).await;
        }

        // Configure options
        let options = self.build_options();

        if self.verbose && !output::machine_mode() {
            println!("{} Configuration:", style("⚙").dim());
            println!("  • Check objects: {}", options.check_objects);
            println!("  • Check references: {}", options.check_refs);
            println!("  • Check connectivity: {}", options.check_connectivity);
            println!("  • Check dangling: {}", options.check_dangling);
            if options.max_objects > 0 {
                println!("  • Max objects: {}", options.max_objects);
            }
            println!();
        }

        // Run integrity check
        let report = checker
            .check(options)
            .await
            .context("Failed to complete integrity check")?;

        // Display results
        if !output::machine_mode() {
            self.display_report(&report)?;
        }

        // Repair if requested
        let mut report = report;
        let mut outcome = None;
        if self.repair && !report.repairable_issues().is_empty() {
            if !self.quiet() {
                println!();
                println!(
                    "{} Attempting to repair {} issue(s)...",
                    style("🔧").yellow().bold(),
                    report.repairable_issues().len()
                );
            }

            // Reuse the checker's ODB: restoring an object re-stores content,
            // so it must compress exactly the way the reader expects.
            let repair = FsckRepair::new(storage.clone())
                .with_odb(checker.odb())
                .with_refdb(RefDatabase::new(&mediagit_dir))
                .with_repo_root(&repo_path);
            let mut done = repair
                .repair(&report, self.dry_run)
                .await
                .context("Repair failed")?;

            // Last source before giving up: the remote.
            if !self.dry_run
                && (!done.unrestorable_files.is_empty()
                    || !done.quarantined.is_empty()
                    || !done.refs_removed.is_empty())
            {
                restore_from_remote(&repo_path, &mediagit_dir, &repair, &mut done).await;
            } else if self.dry_run && !done.unrestorable_files.is_empty() && !output::machine_mode()
            {
                println!("(A configured remote would be tried before these are given up.)");
            }

            // Never claim a repair the repository does not bear out: check
            // again with a cold checker and report only what that confirms.
            // The exit code and issue list then describe the repository as
            // it is now, not as it was before the repair.
            if !self.dry_run {
                let fresh = FsckChecker::new_with_refdb(
                    storage.clone(),
                    Some(RefDatabase::new(&mediagit_dir)),
                )
                .with_repo_root(&repo_path);
                let recheck = fresh
                    .check(self.build_options())
                    .await
                    .context("Failed to re-check after repair")?;
                done.confirm_against(&recheck);
                report = recheck;
            }

            if !self.quiet() {
                self.print_repair(&done);
                if !self.dry_run {
                    println!();
                    println!("{} After repair:", style("🔍").cyan().bold());
                    self.display_report(&report)?;
                }
            }
            outcome = Some(done);
        }

        // The one automatic deletion: a quarantined copy whose object has
        // verified healthy again. A dry run only counts.
        let sweep_odb = FsckChecker::new_with_refdb(storage.clone(), None).odb();
        // Never in a dry run, and not after a check that could not read
        // something (an InvalidFormat issue): then nothing is "known good".
        let can_delete = !self.dry_run
            && !report
                .issues
                .iter()
                .any(|i| i.category == mediagit_versioning::IssueCategory::InvalidFormat);
        let quarantine = sweep_quarantine(
            &sweep_odb,
            Some(&RefDatabase::new(&mediagit_dir)),
            &repo_path,
            can_delete,
        )
        .await
        .context("Failed to read the quarantine folder")?;

        if output::machine_mode() {
            return emit_integrity(&report, outcome.as_ref(), &quarantine);
        }
        if !self.quiet() {
            if quarantine.removed > 0 {
                println!(
                    "Removed {} quarantined copy(ies) whose object is healthy again.",
                    quarantine.removed
                );
            }
            if quarantine.count > 0 {
                println!(
                    "Quarantine: {} damaged cop(ies), {} bytes, in .mediagit/quarantine                      (mediagit fsck --purge-quarantine deletes them).",
                    quarantine.count, quarantine.bytes
                );
            }
        }

        // Exit with error if critical issues found, or a file is lost
        let unrestorable = outcome
            .as_ref()
            .is_some_and(|o| !o.unrestorable_files.is_empty() || !o.errors.is_empty());
        if report.has_errors() || unrestorable {
            if !self.quiet() {
                println!();
                println!(
                    "{} Repository has critical integrity issues!",
                    style("⚠").red().bold()
                );
            }
            anyhow::bail!(
                "Integrity check failed with {} error(s)",
                report.issues_by_severity(IssueSeverity::Error).len()
            );
        }

        Ok(())
    }

    /// `--purge-quarantine`: delete every quarantined copy, after asking.
    /// Runs no check and no repair.
    async fn purge_quarantine(
        &self,
        repo_path: &std::path::Path,
        mediagit_dir: &std::path::Path,
    ) -> Result<()> {
        let storage = create_storage_backend(repo_path)
            .await
            .context("Failed to open repository. Is this a MediaGit repository?")?;
        let checker = FsckChecker::new_with_refdb(storage.clone(), None);

        // Copies whose object is healthy again are not "lost": drop them first
        // (never in a dry run).
        sweep_quarantine(
            &checker.odb(),
            Some(&RefDatabase::new(mediagit_dir)),
            repo_path,
            !self.dry_run,
        )
        .await?;
        let entries = list_quarantine(repo_path).await?;
        let bytes: u64 = entries.iter().map(|e| e.bytes).sum();

        if entries.is_empty() {
            if output::machine_mode() {
                return output::emit_ok(&FsckPurgeReport {
                    deleted_count: 0,
                    deleted_bytes: 0,
                    unrestorable_files: Vec::new(),
                });
            }
            println!("The quarantine is empty. Nothing to delete.");
            return Ok(());
        }

        // Name the files before the evidence goes.
        let oids: Vec<_> = entries.iter().filter_map(|e| e.oid).collect();
        let repair = FsckRepair::new(storage)
            .with_odb(checker.odb())
            .with_refdb(RefDatabase::new(mediagit_dir));
        let mut lost: Vec<String> = repair
            .paths_referencing(&oids)
            .await?
            .into_values()
            .flatten()
            .map(|f| f.path)
            .collect();
        lost.sort();
        lost.dedup();

        if self.dry_run {
            if output::machine_mode() {
                return output::emit_ok(&FsckPurgeReport {
                    deleted_count: 0,
                    deleted_bytes: 0,
                    unrestorable_files: lost,
                });
            }
            println!(
                "[DRY RUN] Would delete {} quarantined cop(ies), {} bytes. Nothing was deleted.",
                entries.len(),
                bytes
            );
            if !lost.is_empty() {
                println!(
                    "{} file(s) have saved versions that could no longer be restored:",
                    lost.len()
                );
                for f in &lost {
                    println!("    {f}");
                }
            }
            return Ok(());
        }

        if !self.yes {
            output::ensure_interactive("fsck --purge-quarantine confirmation (pass --yes)")?;
            if !std::io::IsTerminal::is_terminal(&std::io::stdin()) {
                anyhow::bail!(
                    "stdin is not a terminal; pass --yes to delete {} quarantined cop(ies)",
                    entries.len()
                );
            }
            let confirmed = dialoguer::Confirm::new()
                .with_prompt(format!(
                    "Delete {} quarantined cop(ies) ({} bytes)? This cannot be undone.",
                    entries.len(),
                    bytes
                ))
                .default(false)
                .interact()?;
            if !confirmed {
                anyhow::bail!("cancelled; nothing was deleted");
            }
        }

        for e in &entries {
            tokio::fs::remove_file(&e.path)
                .await
                .with_context(|| format!("deleting {}", e.path.display()))?;
        }
        // Only the listed (and confirmed) files were deleted. Folders go only
        // if now empty, deepest first; anything else in there stays.
        let mut dirs: Vec<std::path::PathBuf> = entries
            .iter()
            .filter_map(|e| e.path.parent().map(|p| p.to_path_buf()))
            .collect();
        dirs.sort();
        dirs.dedup();
        dirs.sort_by_key(|d| std::cmp::Reverse(d.components().count()));
        let root = mediagit_dir.join("quarantine");
        for d in dirs {
            let mut cur = d.as_path();
            while cur.starts_with(&root) && cur != root {
                if tokio::fs::remove_dir(cur).await.is_err() {
                    break;
                }
                match cur.parent() {
                    Some(p) => cur = p,
                    None => break,
                }
            }
        }
        let _ = tokio::fs::remove_dir(&root).await;

        if output::machine_mode() {
            return output::emit_ok(&FsckPurgeReport {
                deleted_count: entries.len() as u64,
                deleted_bytes: bytes,
                unrestorable_files: lost,
            });
        }
        println!(
            "Deleted {} quarantined cop(ies), {} bytes.",
            entries.len(),
            bytes
        );
        if !lost.is_empty() {
            println!(
                "{} file(s) have saved versions that can no longer be restored:",
                lost.len()
            );
            for f in &lost {
                println!("    {f}");
            }
        }
        Ok(())
    }

    /// Plain-words account of a repair run (`--dry-run`: the plan).
    fn print_repair(&self, o: &RepairOutcome) {
        let (restored, quarantined, removed, flattened) = if o.dry_run {
            (
                "Would restore",
                "Would quarantine",
                "Would remove",
                "Would flatten",
            )
        } else {
            ("Restored", "Quarantined", "Removed", "Flattened")
        };
        println!();
        if o.dry_run {
            println!(
                "{} [DRY RUN] Nothing was changed.",
                style("ℹ").blue().bold()
            );
        }
        if !o.restored.is_empty() {
            println!(
                "  • {restored} {} damaged object(s) from files in the working folder",
                o.restored.len()
            );
        }
        if !o.quarantined.is_empty() {
            println!(
                "  • {quarantined} {} damaged object(s) in .mediagit/quarantine (nothing is deleted)",
                o.quarantined.len()
            );
        }
        if !o.refs_removed.is_empty() {
            println!("  • {removed} {} broken reference(s)", o.refs_removed.len());
        }
        if !o.flattened.is_empty() {
            println!(
                "  • {flattened} {} over-deep delta chain(s)",
                o.flattened.len()
            );
        }
        for e in &o.errors {
            println!("{} {e}", style("✖").red().bold());
        }
        if !o.unrestorable_files.is_empty() {
            println!(
                "{} {} file(s) have saved versions that {} no longer be restored:",
                style("✖").red().bold(),
                o.unrestorable_files.len(),
                if o.dry_run { "could" } else { "can" }
            );
            for f in &o.unrestorable_files {
                println!("    {f}");
            }
        } else if o.restored.is_empty()
            && o.quarantined.is_empty()
            && o.refs_removed.is_empty()
            && o.flattened.is_empty()
        {
            println!(
                "{} Nothing could be repaired. See the warnings above for why each issue was skipped.",
                style("✖").red().bold()
            );
        }
    }

    fn build_options(&self) -> FsckOptions {
        if self.quick {
            FsckOptions::quick()
        } else if self.full {
            let mut opts = FsckOptions::full();
            opts.verbose = self.verbose;
            if self.max_objects > 0 {
                opts.max_objects = self.max_objects;
            }
            opts
        } else {
            let mut opts = FsckOptions {
                verbose: self.verbose,
                check_dangling: self.lost_found || (!self.no_dangling && self.full),
                ..FsckOptions::default()
            };
            if self.max_objects > 0 {
                opts.max_objects = self.max_objects;
            }
            opts
        }
    }

    fn display_report(&self, report: &mediagit_versioning::FsckReport) -> Result<()> {
        if !self.quiet() {
            println!();
            println!("{} Statistics:", style("📊").cyan().bold());
            println!("  • Objects checked: {}", report.objects_checked);
            println!("  • References checked: {}", report.refs_checked);
            println!();
        }

        // Display issues by severity
        let errors = report.issues_by_severity(IssueSeverity::Error);
        let warnings = report.issues_by_severity(IssueSeverity::Warning);
        let info = report.issues_by_severity(IssueSeverity::Info);

        if !errors.is_empty() {
            println!("{} Errors found:", style("❌").red().bold());
            for issue in &errors {
                println!("  • {}", style(&issue.message).red());
                if self.verbose {
                    if let Some(oid) = issue.oid {
                        println!("    OID: {}", style(oid.to_string()).dim());
                    }
                    if let Some(ref_name) = &issue.ref_name {
                        println!("    Ref: {}", style(ref_name).dim());
                    }
                    if issue.repairable {
                        println!("    {}", style("(Repairable with --repair)").yellow());
                    }
                }
            }
            println!();
        }

        if !warnings.is_empty() && !self.quiet() {
            println!("{} Warnings:", style("⚠").yellow().bold());
            for issue in &warnings {
                println!("  • {}", style(&issue.message).yellow());
                if self.verbose {
                    if let Some(oid) = issue.oid {
                        println!("    OID: {}", style(oid.to_string()).dim());
                    }
                    if let Some(ref_name) = &issue.ref_name {
                        println!("    Ref: {}", style(ref_name).dim());
                    }
                }
            }
            println!();
        }

        // Branches a clone left on the server on purpose: always worth a line.
        let not_downloaded: Vec<_> = info
            .iter()
            .copied()
            .filter(|i| i.category == mediagit_versioning::IssueCategory::NotDownloaded)
            .collect();
        if !not_downloaded.is_empty() && !self.quiet() {
            for issue in &not_downloaded {
                println!("{} {}", style("ℹ").blue().bold(), issue.message);
            }
            println!();
        }

        let info: Vec<_> = info
            .into_iter()
            .filter(|i| i.category != mediagit_versioning::IssueCategory::NotDownloaded)
            .collect();
        if !info.is_empty() && (self.verbose || self.lost_found) {
            println!("{} Information:", style("ℹ").blue().bold());
            for issue in &info {
                println!("  • {}", style(&issue.message).dim());
                if self.all
                    && self.verbose
                    && let Some(oid) = issue.oid
                {
                    println!("    OID: {}", style(oid.to_string()).dim());
                }
            }
            println!();
        }

        // Summary
        if !self.quiet() {
            if report.total_issues() == 0 {
                println!(
                    "{} Repository integrity: {}",
                    style("✅").green().bold(),
                    style("PERFECT").green().bold()
                );
            } else if !report.has_errors() {
                let mut parts = Vec::new();
                if !warnings.is_empty() {
                    parts.push(format!("{} warning(s)", warnings.len()));
                }
                if !info.is_empty() {
                    parts.push(format!("{} info", info.len()));
                }
                if !not_downloaded.is_empty() {
                    parts.push(format!(
                        "{} branch(es) still on the server",
                        not_downloaded.len()
                    ));
                }
                let detail = if parts.is_empty() {
                    String::new()
                } else {
                    format!(" ({})", parts.join(", "))
                };
                println!(
                    "{} Repository integrity: {}{}",
                    style("✓").green(),
                    style("OK").green(),
                    detail
                );
            } else {
                println!(
                    "{} Repository integrity: {} ({} error(s), {} warning(s))",
                    style("✗").red().bold(),
                    style("FAILED").red().bold(),
                    errors.len(),
                    warnings.len()
                );

                if !report.repairable_issues().is_empty() {
                    println!();
                    println!(
                        "{} {} issue(s) can be repaired with: {}",
                        style("💡").yellow(),
                        report.repairable_issues().len(),
                        style("mediagit fsck --repair").cyan()
                    );
                }
            }
        }

        Ok(())
    }
}
