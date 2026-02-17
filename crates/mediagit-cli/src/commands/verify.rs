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
use mediagit_storage::StorageBackend;
use mediagit_versioning::{Commit, FsckChecker, FsckOptions, IssueSeverity, ObjectDatabase, Oid, RefDatabase};
use std::collections::HashSet;
use std::path::Path;
use std::sync::Arc;
use crate::repo::create_storage_backend;

/// Verify repository integrity with quick checks
///
/// The verify command performs a fast integrity check focusing on:
/// - Object checksum verification
/// - Reference validation
///
/// For comprehensive checks including connectivity and dangling objects,
/// use `mediagit fsck --full` instead.
#[derive(Parser, Debug)]
#[command(after_help = "EXAMPLES:
    # Quick verification
    mediagit verify

    # Verify with detailed output
    mediagit verify --detailed

    # Verify specific commit range
    mediagit verify --start abc123 --end def456

VERIFY vs FSCK:
    verify  - Fast integrity check (checksums + refs only)
            - Use for quick health checks and CI pipelines
            - Does NOT check connectivity or dangling objects

    fsck    - Comprehensive integrity check (full graph analysis)
            - Use for thorough repository audits
            - Checks connectivity, finds dangling/unreachable objects
            - Supports repair mode (--repair)

SEE ALSO:
    mediagit-fsck(1)")]
pub struct VerifyCmd {
    /// Verify file integrity (checksums)
    #[arg(long)]
    pub file_integrity: bool,

    /// Verify all checksums
    #[arg(long)]
    pub checksums: bool,

    /// Start at this commit (verify commits from this point)
    #[arg(long, value_name = "COMMIT")]
    pub start: Option<String>,

    /// End at this commit (verify commits up to and including this point)
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
        let repo_path_str = self.path.as_deref().unwrap_or(".");
        let repo_path = std::path::PathBuf::from(repo_path_str);
        let mediagit_dir = repo_path.join(".mediagit");

        if !self.quiet {
            println!(
                "{} Verifying repository integrity...",
                style("✔").cyan().bold()
            );
        }

        // Create storage backend
        let storage = create_storage_backend(&repo_path).await
            .context("Failed to open repository. Is this a MediaGit repository?")?;

        // Handle commit range verification
        if self.start.is_some() || self.end.is_some() {
            return self.verify_commit_range(&mediagit_dir, storage.clone()).await;
        }

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

    /// Verify a specific range of commits
    async fn verify_commit_range(
        &self,
        mediagit_dir: &Path,
        storage: Arc<dyn StorageBackend>,
    ) -> Result<()> {
        let refdb = RefDatabase::new(mediagit_dir);
        let odb = Arc::new(ObjectDatabase::with_smart_compression(storage.clone(), 1000));

        // Resolve start commit (defaults to root)
        let start_oid = if let Some(ref start) = self.start {
            self.resolve_commit(&refdb, start).await?
        } else {
            None
        };

        // Resolve end commit (defaults to HEAD)
        let end_oid = if let Some(ref end) = self.end {
            self.resolve_commit(&refdb, end).await?
        } else {
            // Default to HEAD
            let head = refdb.read("HEAD").await?;
            if let Some(target) = &head.target {
                let target_ref = refdb.read(target).await?;
                target_ref.oid
            } else {
                head.oid
            }
        };

        let end_oid = end_oid.ok_or_else(|| anyhow::anyhow!("Cannot determine end commit"))?;

        if !self.quiet {
            let start_str = start_oid
                .map(|o| o.to_string()[..7].to_string())
                .unwrap_or_else(|| "(root)".to_string());
            println!(
                "  Range: {} → {}",
                style(&start_str).cyan(),
                style(&end_oid.to_string()[..7]).cyan()
            );
        }

        // Collect commits in range
        let commits_in_range = self.collect_commits_in_range(&odb, start_oid, end_oid).await?;

        if !self.quiet {
            println!("  Commits to verify: {}", commits_in_range.len());
        }

        // Verify each commit
        let mut errors = 0;
        let mut verified = 0;

        for commit_oid in &commits_in_range {
            // Verify commit object exists and is valid
            match odb.read(commit_oid).await {
                Ok(data) => {
                    // Try to parse as commit
                    match Commit::deserialize(&data) {
                        Ok(commit) => {
                            verified += 1;

                            if self.verbose {
                                let msg = commit.message.lines().next().unwrap_or("");
                                println!(
                                    "  {} {} {}",
                                    style("✓").green(),
                                    &commit_oid.to_string()[..7],
                                    style(msg).dim()
                                );
                            }

                            // Verify tree exists
                            if odb.read(&commit.tree).await.is_err() {
                                errors += 1;
                                if !self.quiet {
                                    println!(
                                        "  {} {} missing tree {}",
                                        style("✗").red(),
                                        &commit_oid.to_string()[..7],
                                        &commit.tree.to_string()[..7]
                                    );
                                }
                            }
                        }
                        Err(e) => {
                            errors += 1;
                            if !self.quiet {
                                println!(
                                    "  {} {} invalid commit: {}",
                                    style("✗").red(),
                                    &commit_oid.to_string()[..7],
                                    e
                                );
                            }
                        }
                    }
                }
                Err(e) => {
                    errors += 1;
                    if !self.quiet {
                        println!(
                            "  {} {} missing or corrupt: {}",
                            style("✗").red(),
                            &commit_oid.to_string()[..7],
                            e
                        );
                    }
                }
            }
        }

        // Summary
        if !self.quiet {
            println!();
            if errors == 0 {
                println!(
                    "{} Verified {} commits in range - all OK",
                    style("✅").green().bold(),
                    verified
                );
            } else {
                println!(
                    "{} Verified {} commits, {} errors found",
                    style("❌").red().bold(),
                    verified,
                    errors
                );
            }
        }

        if errors > 0 {
            anyhow::bail!("Verification failed with {} error(s)", errors);
        }

        Ok(())
    }

    /// Resolve a commit reference to an OID
    async fn resolve_commit(&self, refdb: &RefDatabase, spec: &str) -> Result<Option<Oid>> {
        // Try as hex OID first
        if let Ok(oid) = Oid::from_hex(spec) {
            return Ok(Some(oid));
        }

        // Try as reference
        if let Ok(r) = refdb.read(spec).await {
            return Ok(r.oid);
        }

        // Try with refs/heads/ prefix
        let with_prefix = format!("refs/heads/{}", spec);
        if let Ok(r) = refdb.read(&with_prefix).await {
            return Ok(r.oid);
        }

        anyhow::bail!("Cannot resolve commit: {}", spec)
    }

    /// Collect commits between start and end (walking back from end to start)
    async fn collect_commits_in_range(
        &self,
        odb: &Arc<ObjectDatabase>,
        start_oid: Option<Oid>,
        end_oid: Oid,
    ) -> Result<Vec<Oid>> {
        let mut commits = Vec::new();
        let mut visited = HashSet::new();
        let mut queue = vec![end_oid];

        while let Some(current) = queue.pop() {
            // Stop if we've reached the start commit
            if let Some(start) = start_oid {
                if current == start {
                    continue;
                }
            }

            if visited.contains(&current) {
                continue;
            }
            visited.insert(current);
            commits.push(current);

            // Read commit to get parents
            if let Ok(data) = odb.read(&current).await {
                if let Ok(commit) = Commit::deserialize(&data) {
                    for parent in commit.parents {
                        if !visited.contains(&parent) {
                            queue.push(parent);
                        }
                    }
                }
            }
        }

        // Reverse to get chronological order
        commits.reverse();
        Ok(commits)
    }
}
