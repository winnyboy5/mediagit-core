// SPDX-License-Identifier: BUSL-1.1
// Copyright (C) 2025-2026 Aswin Krishnamoorthy

//! File System Check (FSCK) command - Repository integrity verification

use crate::repo::create_storage_backend;
use anyhow::{Context, Result};
use clap::Parser;
use console::style;
use mediagit_versioning::{
    FsckChecker, FsckOptions, FsckRepair, IssueSeverity, RefDatabase, RepairOutcome,
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

    /// Limit number of objects to check (0 = unlimited)
    #[arg(long, default_value = "0")]
    pub max_objects: u64,

    /// Quiet mode (only errors)
    #[arg(short, long)]
    pub quiet: bool,

    /// Verbose mode (detailed progress)
    #[arg(short, long)]
    pub verbose: bool,

    /// Repository path (defaults to current directory)
    #[arg(long, value_name = "PATH")]
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
    pub issues: Vec<FsckIssueReport>,
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
) -> Result<()> {
    let lost_files = repair.is_some_and(|o| !o.unrestorable_files.is_empty());
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

impl FsckCmd {
    fn quiet(&self) -> bool {
        self.quiet || output::machine_mode()
    }

    pub async fn execute(&self) -> Result<()> {
        // Determine repository path
        let repo_path_str = self.path.as_deref().unwrap_or(".");
        let repo_path = std::path::PathBuf::from(repo_path_str);
        let mediagit_dir = repo_path.join(".mediagit");

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

        if output::machine_mode() {
            return emit_integrity(&report, outcome.as_ref());
        }

        // Exit with error if critical issues found, or a file is lost
        let unrestorable = outcome
            .as_ref()
            .is_some_and(|o| !o.unrestorable_files.is_empty());
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
        if !o.unrestorable_files.is_empty() {
            println!(
                "{} {} file(s) {} no longer be restored:",
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
                println!(
                    "{} Repository integrity: {} ({} warning(s), {} info)",
                    style("✓").green(),
                    style("OK").green(),
                    warnings.len(),
                    info.len()
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
