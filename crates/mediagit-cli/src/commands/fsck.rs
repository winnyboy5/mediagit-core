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

//! File System Check (FSCK) command - Repository integrity verification

use anyhow::{Context, Result};
use clap::Parser;
use console::style;
use mediagit_versioning::{FsckChecker, FsckOptions, FsckRepair, IssueSeverity};
use crate::repo::create_storage_backend;

/// Check repository integrity with comprehensive verification
///
/// The fsck command performs a comprehensive integrity check of your MediaGit repository:
/// - Verifies SHA-256 checksums of all objects
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

impl FsckCmd {
    pub async fn execute(&self) -> Result<()> {
        // Determine repository path
        let repo_path_str = self.path.as_deref().unwrap_or(".");
        let repo_path = std::path::PathBuf::from(repo_path_str);
        let mediagit_dir = repo_path.join(".mediagit");

        if !self.quiet {
            println!(
                "{} Checking repository integrity at {}",
                style("🔍").cyan().bold(),
                style(&mediagit_dir.display()).dim()
            );
        }

        // Create storage backend
        let storage = create_storage_backend(&repo_path).await
            .context("Failed to open repository. Is this a MediaGit repository?")?;

        // Create FSCK checker
        let checker = FsckChecker::new(storage.clone());

        // Configure options
        let options = self.build_options();

        if self.verbose {
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
        self.display_report(&report)?;

        // Repair if requested
        if self.repair && !report.repairable_issues().is_empty() {
            if !self.quiet {
                println!();
                println!(
                    "{} Attempting to repair {} issue(s)...",
                    style("🔧").yellow().bold(),
                    report.repairable_issues().len()
                );
            }

            let repair = FsckRepair::new(storage);
            let repaired = repair
                .repair(&report, self.dry_run)
                .await
                .context("Repair failed")?;

            if !self.quiet {
                if self.dry_run {
                    println!(
                        "{} [DRY RUN] Would repair {} issue(s)",
                        style("ℹ").blue().bold(),
                        repaired
                    );
                } else {
                    println!(
                        "{} Successfully repaired {} issue(s)",
                        style("✅").green().bold(),
                        repaired
                    );
                }
            }
        }

        // Exit with error if critical issues found
        if report.has_errors() {
            if !self.quiet {
                println!();
                println!(
                    "{} Repository has critical integrity issues!",
                    style("⚠").red().bold()
                );
            }
            anyhow::bail!("Integrity check failed with {} error(s)", report.issues_by_severity(IssueSeverity::Error).len());
        }

        Ok(())
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
            let mut opts = FsckOptions::default();
            opts.verbose = self.verbose;
            opts.check_dangling = self.lost_found || (!self.no_dangling && self.full);
            if self.max_objects > 0 {
                opts.max_objects = self.max_objects;
            }
            opts
        }
    }

    fn display_report(&self, report: &mediagit_versioning::FsckReport) -> Result<()> {
        if !self.quiet {
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

        if !warnings.is_empty() && !self.quiet {
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
                if self.all && self.verbose {
                    if let Some(oid) = issue.oid {
                        println!("    OID: {}", style(oid.to_string()).dim());
                    }
                }
            }
            println!();
        }

        // Summary
        if !self.quiet {
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

                if report.repairable_issues().len() > 0 {
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
