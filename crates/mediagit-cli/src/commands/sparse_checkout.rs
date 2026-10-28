// SPDX-License-Identifier: BUSL-1.1
// Copyright (C) 2025-2026 Aswin Krishnamoorthy

//! Sparse checkout (#8): materialize only part of the working tree.
//!
//! `sparse-checkout set` writes `.mediagit/info/sparse-checkout` and applies
//! it immediately — newly-excluded tracked files are removed from disk,
//! newly-included ones are materialized. `list` shows the active patterns.
//! `disable` removes the pattern file and restores the full tree.
//!
//! Ordinary checkout operations (branch switch, clone, etc.) treat the
//! sparse filter as *read-only*: they never write excluded files and never
//! delete an excluded file that happens to exist on disk (see
//! `mediagit_versioning::sparse`). Only this command materializes/removes
//! files in response to a pattern change.

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use mediagit_versioning::{CheckoutManager, ObjectDatabase, RefDatabase, SparseFilter, SparseMode};

use serde::Serialize;

use super::super::output;
use super::super::repo::{create_storage_backend, find_repo_root};

/// `--machine` data for `sparse-checkout set|list|disable`.
#[derive(Debug, Clone, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[cfg_attr(test, ts(export, export_to = "../../../ui/src/api/generated/"))]
pub struct SparseReport {
    /// `set`, `list` or `disable`.
    pub action: String,
    pub enabled: bool,
    /// `cone` or `pattern`; `None` when disabled.
    pub mode: Option<String>,
    pub patterns: Vec<String>,
    /// `set`/`disable`: files materialized.
    #[cfg_attr(test, ts(type = "number | null"))]
    pub files_added: Option<u64>,
    /// `set`: files removed from disk.
    #[cfg_attr(test, ts(type = "number | null"))]
    pub files_removed: Option<u64>,
}

fn mode_name(mode: SparseMode) -> String {
    if matches!(mode, SparseMode::Cone) {
        "cone"
    } else {
        "pattern"
    }
    .into()
}

fn set_report(mode: SparseMode, patterns: &[String], added: usize, removed: usize) -> SparseReport {
    SparseReport {
        action: "set".into(),
        enabled: true,
        mode: Some(mode_name(mode)),
        patterns: patterns.to_vec(),
        files_added: Some(added as u64),
        files_removed: Some(removed as u64),
    }
}

/// Manage sparse checkout (partial working tree)
#[derive(Parser, Debug)]
pub struct SparseCheckoutCmd {
    #[command(subcommand)]
    pub subcommand: SparseCheckoutSubcommand,
}

#[derive(Subcommand, Debug)]
pub enum SparseCheckoutSubcommand {
    /// Set sparse-checkout patterns and apply them to the working tree
    Set(SetOpts),
    /// List the active sparse-checkout patterns
    #[command(alias = "ls")]
    List(ListOpts),
    /// Disable sparse checkout and restore the full working tree
    Disable(DisableOpts),
}

/// Set sparse-checkout patterns
#[derive(Parser, Debug)]
#[command(after_help = "EXAMPLES:
    # Cone mode (default): include these directories, recursively
    mediagit sparse-checkout set assets/textures assets/audio

    # Pattern mode: gitignore-style globs (a match means \"include\")
    mediagit sparse-checkout set --patterns '*.png' '*.wav'

SEE ALSO:
    mediagit-status(1)")]
pub struct SetOpts {
    /// Cone mode (default): directory prefixes. Pattern mode (--patterns):
    /// gitignore-style globs, where a match means the file is included.
    #[arg(value_name = "PATTERN", required = true)]
    pub patterns: Vec<String>,

    /// Interpret PATTERN as gitignore-style globs instead of cone-mode directory prefixes
    #[arg(long = "patterns")]
    pub pattern_mode: bool,

    /// Quiet mode
    #[arg(short, long)]
    pub quiet: bool,
}

/// List active sparse-checkout patterns
#[derive(Parser, Debug)]
pub struct ListOpts {}

/// Disable sparse checkout, restoring the full working tree
#[derive(Parser, Debug)]
pub struct DisableOpts {
    /// Quiet mode
    #[arg(short, long)]
    pub quiet: bool,
}

impl SparseCheckoutCmd {
    pub async fn execute(&self) -> Result<()> {
        match &self.subcommand {
            SparseCheckoutSubcommand::Set(opts) => opts.execute().await,
            SparseCheckoutSubcommand::List(opts) => opts.execute().await,
            SparseCheckoutSubcommand::Disable(opts) => opts.execute().await,
        }
    }
}

impl SetOpts {
    pub async fn execute(&self) -> Result<()> {
        let repo_root = find_repo_root()?;
        let storage_path = repo_root.join(".mediagit");
        let storage = create_storage_backend(&repo_root).await?;
        let refdb = RefDatabase::new(&storage_path);
        let odb = ObjectDatabase::with_smart_compression(storage.clone(), 1000);

        let old_filter = SparseFilter::load(&repo_root)?;
        let mode = if self.pattern_mode {
            SparseMode::Pattern
        } else {
            SparseMode::Cone
        };
        let new_filter = SparseFilter::preview(&repo_root, mode, &self.patterns)?;

        let Ok(head_oid) = refdb.resolve("HEAD").await else {
            SparseFilter::write(&repo_root, mode, &self.patterns)?;
            if !self.quiet {
                output::info("No commits yet; patterns saved, nothing to materialize.");
            }
            if output::machine_mode() {
                return output::emit_ok(&set_report(mode, &self.patterns, 0, 0));
            }
            return Ok(());
        };

        let checkout_mgr = CheckoutManager::new(&odb, &repo_root);
        let tracked = checkout_mgr
            .tracked_files_at(&head_oid)
            .await
            .context("Failed to list tracked files at HEAD")?;

        // WT-8: newly-excluded files were deleted unconditionally. Refuse
        // before the patterns are persisted, so a refusal leaves the sparse
        // configuration exactly as it was.
        let at_risk =
            crate::worktree_guard::AtRisk::check(&repo_root, &odb, Some(&head_oid), None).await?;
        let doomed: Vec<std::path::PathBuf> = at_risk
            .modified
            .iter()
            .filter(|p| old_filter.is_included(p) && !new_filter.is_included(p))
            .cloned()
            .collect();
        if !doomed.is_empty() {
            return Err(crate::worktree_guard::AtRisk {
                modified: doomed,
                untracked_collisions: Vec::new(),
            }
            .bail("sparse-checkout set"));
        }

        SparseFilter::write(&repo_root, mode, &self.patterns)?;

        let mut removed = 0usize;
        let mut added = 0usize;
        for (path, (oid, file_mode)) in &tracked {
            let old_included = old_filter.is_included(path);
            let new_included = new_filter.is_included(path);
            if old_included && !new_included {
                let full_path = repo_root.join(path);
                if full_path.exists() {
                    std::fs::remove_file(&full_path)
                        .with_context(|| format!("Failed to remove '{}'", full_path.display()))?;
                    removed += 1;
                }
            } else if !old_included && new_included {
                checkout_mgr
                    .materialize_file(path, oid, *file_mode)
                    .await
                    .with_context(|| format!("Failed to materialize '{}'", path.display()))?;
                added += 1;
            }
        }

        if !self.quiet {
            output::success(&format!(
                "Sparse checkout set: {} pattern(s) ({} mode) — {} file(s) removed, {} file(s) added",
                self.patterns.len(),
                if matches!(mode, SparseMode::Cone) {
                    "cone"
                } else {
                    "pattern"
                },
                removed,
                added
            ));
        }
        if output::machine_mode() {
            return output::emit_ok(&set_report(mode, &self.patterns, added, removed));
        }
        Ok(())
    }
}

impl ListOpts {
    pub async fn execute(&self) -> Result<()> {
        let repo_root = find_repo_root()?;
        let filter = SparseFilter::load(&repo_root)?;
        if output::machine_mode() {
            return output::emit_ok(&SparseReport {
                action: "list".into(),
                enabled: filter.is_enabled(),
                mode: filter.is_enabled().then(|| mode_name(filter.mode())),
                patterns: if filter.is_enabled() {
                    filter.patterns().to_vec()
                } else {
                    Vec::new()
                },
                files_added: None,
                files_removed: None,
            });
        }
        if !filter.is_enabled() {
            println!("Sparse checkout is disabled (full working tree).");
            return Ok(());
        }
        println!(
            "Mode: {}",
            if matches!(filter.mode(), SparseMode::Cone) {
                "cone"
            } else {
                "pattern"
            }
        );
        for pattern in filter.patterns() {
            println!("{pattern}");
        }
        Ok(())
    }
}

impl DisableOpts {
    pub async fn execute(&self) -> Result<()> {
        let repo_root = find_repo_root()?;
        let storage_path = repo_root.join(".mediagit");
        let storage = create_storage_backend(&repo_root).await?;
        let refdb = RefDatabase::new(&storage_path);
        let odb = ObjectDatabase::with_smart_compression(storage.clone(), 1000);

        let old_filter = SparseFilter::load(&repo_root)?;
        let disabled_report = || SparseReport {
            action: "disable".into(),
            enabled: false,
            mode: None,
            patterns: Vec::new(),
            files_added: Some(0),
            files_removed: Some(0),
        };
        if !old_filter.is_enabled() {
            if !self.quiet {
                output::info("Sparse checkout is already disabled.");
            }
            if output::machine_mode() {
                return output::emit_ok(&disabled_report());
            }
            return Ok(());
        }
        SparseFilter::remove(&repo_root)?;

        let mut restored = 0usize;
        if let Ok(head_oid) = refdb.resolve("HEAD").await {
            let checkout_mgr = CheckoutManager::new(&odb, &repo_root);
            let tracked = checkout_mgr
                .tracked_files_at(&head_oid)
                .await
                .context("Failed to list tracked files at HEAD")?;
            for (path, (oid, file_mode)) in &tracked {
                if !old_filter.is_included(path) {
                    checkout_mgr
                        .materialize_file(path, oid, *file_mode)
                        .await
                        .with_context(|| format!("Failed to materialize '{}'", path.display()))?;
                    restored += 1;
                }
            }
        }

        if !self.quiet {
            output::success(&format!(
                "Sparse checkout disabled; {} file(s) restored.",
                restored
            ));
        }
        if output::machine_mode() {
            return output::emit_ok(&SparseReport {
                files_added: Some(restored as u64),
                ..disabled_report()
            });
        }
        Ok(())
    }
}
