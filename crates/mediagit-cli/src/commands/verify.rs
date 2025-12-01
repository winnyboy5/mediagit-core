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

//! Verify command - Quick integrity verification

use anyhow::{Context, Result};
use clap::Parser;
use console::style;
use mediagit_storage::LocalBackend;
use mediagit_versioning::{FsckChecker, FsckOptions, IssueSeverity};
use std::sync::Arc;

/// Verify repository integrity with quick checks
///
/// The verify command performs a fast integrity check focusing on:
/// - Object checksum verification
/// - Reference validation
///
/// For comprehensive checks including connectivity and dangling objects,
/// use `mediagit fsck --full` instead.
///
/// # Examples
///
/// Quick verification:
/// ```bash
/// mediagit verify
/// ```
///
/// Verify with detailed output:
/// ```bash
/// mediagit verify --detailed
/// ```
///
/// Verify specific commit range:
/// ```bash
/// mediagit verify --start abc123 --end def456
/// ```
#[derive(Parser, Debug)]
pub struct VerifyCmd {
    /// Verify file integrity (checksums)
    #[arg(long)]
    pub file_integrity: bool,

    /// Verify all checksums
    #[arg(long)]
    pub checksums: bool,

    /// Start at this commit (future: not yet implemented)
    #[arg(long, value_name = "COMMIT")]
    pub start: Option<String>,

    /// End at this commit (future: not yet implemented)
    #[arg(long, value_name = "COMMIT")]
    pub end: Option<String>,

    /// Quick verification (minimal checks)
    #[arg(long)]
    pub quick: bool,

    /// Detailed verification report
    #[arg(long)]
    pub detailed: bool,

    /// Quiet mode
    #[arg(short, long)]
    pub quiet: bool,

    /// Verbose mode
    #[arg(short, long)]
    pub verbose: bool,

    /// Repository path (defaults to current directory)
    #[arg(long, value_name = "PATH")]
    pub path: Option<String>,
}

impl VerifyCmd {
    pub async fn execute(&self) -> Result<()> {
        // Determine repository path
        let repo_path = self.path.as_deref().unwrap_or(".");
        let mediagit_dir = format!("{}/.mediagit", repo_path);

        if !self.quiet {
            println!(
                "{} Verifying repository integrity...",
                style("✔").cyan().bold()
            );
        }

        // Create storage backend
        let storage = Arc::new(
            LocalBackend::new(&mediagit_dir).await
                .context("Failed to open repository. Is this a MediaGit repository?")?,
        );

        // Create FSCK checker (verify is a lightweight wrapper)
        let checker = FsckChecker::new(storage);

        // Configure options - verify uses quick mode by default
        let mut options = if self.quick {
            FsckOptions::quick()
        } else {
            let mut opts = FsckOptions::default();
            opts.check_connectivity = false; // Verify skips connectivity
            opts.check_dangling = false; // Verify doesn't check dangling
            opts
        };

        options.verbose = self.verbose || self.detailed;

        // Run verification
        let report = checker
            .check(options)
            .await
            .context("Verification failed")?;

        // Display results
        self.display_report(&report)?;

        // Exit with error if issues found
        if report.has_errors() {
            anyhow::bail!(
                "Verification failed with {} error(s)",
                report.issues_by_severity(IssueSeverity::Error).len()
            );
        }

        if !self.quiet {
            println!(
                "{} All verifications passed",
                style("✅").green().bold()
            );
        }

        Ok(())
    }

    fn display_report(&self, report: &mediagit_versioning::FsckReport) -> Result<()> {
        if self.detailed || self.verbose {
            println!();
            println!("{} Verification Statistics:", style("📊").cyan().bold());
            println!("  • Objects verified: {}", report.objects_checked);
            println!("  • References verified: {}", report.refs_checked);

            if report.corrupted_objects > 0 {
                println!(
                    "  • Corrupted objects: {}",
                    style(report.corrupted_objects).red().bold()
                );
            }
            if report.broken_refs > 0 {
                println!(
                    "  • Broken references: {}",
                    style(report.broken_refs).red().bold()
                );
            }
            if report.missing_objects > 0 {
                println!(
                    "  • Missing objects: {}",
                    style(report.missing_objects).red().bold()
                );
            }
            println!();
        }

        // Display errors if any
        let errors = report.issues_by_severity(IssueSeverity::Error);
        if !errors.is_empty() {
            println!("{} Verification Errors:", style("❌").red().bold());
            for issue in errors {
                println!("  • {}", style(&issue.message).red());
                if self.detailed {
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

        // Display warnings if detailed
        if self.detailed {
            let warnings = report.issues_by_severity(IssueSeverity::Warning);
            if !warnings.is_empty() {
                println!("{} Warnings:", style("⚠").yellow().bold());
                for issue in warnings {
                    println!("  • {}", style(&issue.message).yellow());
                }
                println!();
            }
        }

        Ok(())
    }
}
