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

use super::super::output;
use super::super::repo::{create_storage_backend, find_repo_root};

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
        Ok(())
    }
}

impl ListOpts {
    pub async fn execute(&self) -> Result<()> {
        let repo_root = find_repo_root()?;
        let filter = SparseFilter::load(&repo_root)?;
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
        if !old_filter.is_enabled() {
            if !self.quiet {
                output::info("Sparse checkout is already disabled.");
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
        Ok(())
    }
}
