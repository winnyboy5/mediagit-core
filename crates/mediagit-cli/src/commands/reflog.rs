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

//! Show reference logs.
//!
//! The reflog command displays a log of when the tips of branches and other
//! references were updated in the local repository.

use anyhow::Result;
use clap::{Parser, Subcommand};
use console::style;
use mediagit_versioning::Reflog;

use super::super::repo::find_repo_root;

/// Show reference logs (reflog)
///
/// Reference logs record when the tips of branches and other references
/// were updated in the local repository.
#[derive(Parser, Debug)]
#[command(after_help = "EXAMPLES:
    # Show reflog for HEAD
    mediagit reflog

    # Show reflog for a specific branch
    mediagit reflog refs/heads/main

    # Show last 5 reflog entries
    mediagit reflog -n 5

    # Show all reflogs
    mediagit reflog show --all

    # Delete reflog for a branch
    mediagit reflog delete refs/heads/feature

    # Expire old reflog entries
    mediagit reflog expire --expire=30

SEE ALSO:
    mediagit-log(1), mediagit-reset(1), mediagit-branch(1)")]
pub struct ReflogCmd {
    #[command(subcommand)]
    pub action: Option<ReflogAction>,

    /// Reference to show reflog for (default: HEAD)
    #[arg(value_name = "REF")]
    pub reference: Option<String>,

    /// Number of entries to show
    #[arg(short = 'n', long, value_name = "COUNT")]
    pub count: Option<usize>,

    /// Quiet mode - only show OIDs
    #[arg(short, long)]
    pub quiet: bool,
}

#[derive(Subcommand, Debug)]
pub enum ReflogAction {
    /// Show reflog entries (default action)
    Show {
        /// Reference to show reflog for
        #[arg(value_name = "REF")]
        reference: Option<String>,

        /// Number of entries to show
        #[arg(short = 'n', long, value_name = "COUNT")]
        count: Option<usize>,

        /// Show reflogs for all refs
        #[arg(long)]
        all: bool,
    },

    /// Delete reflog for a reference
    Delete {
        /// Reference to delete reflog for
        #[arg(required = true)]
        reference: String,
    },

    /// Prune/expire old reflog entries
    Expire {
        /// Reference to expire (default: all)
        #[arg(value_name = "REF")]
        reference: Option<String>,

        /// Number of entries to keep
        #[arg(long, default_value = "90")]
        keep: usize,
    },
}

impl ReflogCmd {
    pub async fn execute(&self) -> Result<()> {
        let repo_root = find_repo_root()?;
        let storage_path = repo_root.join(".mediagit");
        let reflog = Reflog::new(&storage_path);

        match &self.action {
            Some(ReflogAction::Show { reference, count, all }) => {
                if *all {
                    self.show_all(&reflog).await
                } else {
                    let ref_name = reference.as_deref().unwrap_or("HEAD");
                    self.show_reflog(&reflog, ref_name, *count).await
                }
            }
            Some(ReflogAction::Delete { reference }) => {
                self.delete_reflog(&reflog, reference).await
            }
            Some(ReflogAction::Expire { reference, keep }) => {
                self.expire_reflog(&reflog, reference.as_deref(), *keep).await
            }
            None => {
                // Default action: show reflog
                let ref_name = self.reference.as_deref().unwrap_or("HEAD");
                self.show_reflog(&reflog, ref_name, self.count).await
            }
        }
    }

    async fn show_reflog(&self, reflog: &Reflog, ref_name: &str, limit: Option<usize>) -> Result<()> {
        let entries = reflog.read(ref_name, limit).await?;

        if entries.is_empty() {
            if !self.quiet {
                println!("{} No reflog entries for {}", style("ℹ").cyan(), ref_name);
            }
            return Ok(());
        }

        if !self.quiet {
            println!("{} Reflog for {}", style("📋").cyan().bold(), style(ref_name).yellow());
            println!();
        }

        for (i, entry) in entries.iter().enumerate() {
            if self.quiet {
                println!("{}", entry.new_oid.to_hex());
            } else {
                let short_oid = &entry.new_oid.to_hex()[..7];
                let timestamp = entry.committer.timestamp.format("%Y-%m-%d %H:%M:%S");
                
                println!(
                    "{} {} {}: {}",
                    style(format!("{}@{{{}}}", ref_name, i)).yellow(),
                    style(short_oid).green(),
                    style(timestamp).dim(),
                    entry.message
                );
            }
        }

        Ok(())
    }

    async fn show_all(&self, reflog: &Reflog) -> Result<()> {
        let refs = reflog.list_refs().await?;

        if refs.is_empty() {
            println!("{} No reflogs found", style("ℹ").cyan());
            return Ok(());
        }

        println!("{} Found {} refs with reflogs", style("📋").cyan().bold(), refs.len());
        println!();

        for ref_name in refs {
            let entries = reflog.read(&ref_name, Some(5)).await?;
            if !entries.is_empty() {
                println!("{} {} ({} entries)", 
                    style("→").cyan(),
                    style(&ref_name).yellow(),
                    entries.len()
                );
                
                for (i, entry) in entries.iter().enumerate().take(3) {
                    let short_oid = &entry.new_oid.to_hex()[..7];
                    println!("    {}@{{{}}}: {} - {}", 
                        ref_name, i, 
                        style(short_oid).green(),
                        entry.message
                    );
                }
                
                if entries.len() > 3 {
                    println!("    {} more entries...", entries.len() - 3);
                }
                println!();
            }
        }

        Ok(())
    }

    async fn delete_reflog(&self, reflog: &Reflog, ref_name: &str) -> Result<()> {
        if reflog.delete(ref_name).await? {
            println!("{} Deleted reflog for {}", style("✓").green(), ref_name);
        } else {
            println!("{} No reflog found for {}", style("ℹ").cyan(), ref_name);
        }
        Ok(())
    }

    async fn expire_reflog(&self, reflog: &Reflog, ref_name: Option<&str>, keep: usize) -> Result<()> {
        if let Some(ref_name) = ref_name {
            let expired = reflog.expire(ref_name, keep).await?;
            if expired > 0 {
                println!(
                    "{} Expired {} entries from {} (keeping {})",
                    style("✓").green(),
                    expired,
                    ref_name,
                    keep
                );
            } else {
                println!("{} No entries to expire for {}", style("ℹ").cyan(), ref_name);
            }
        } else {
            // Expire all refs
            let refs = reflog.list_refs().await?;
            let mut total_expired = 0;

            for ref_name in refs {
                let expired = reflog.expire(&ref_name, keep).await?;
                if expired > 0 {
                    println!(
                        "{} Expired {} entries from {}",
                        style("✓").green(),
                        expired,
                        ref_name
                    );
                    total_expired += expired;
                }
            }

            if total_expired == 0 {
                println!("{} No entries to expire", style("ℹ").cyan());
            } else {
                println!();
                println!(
                    "{} Total: expired {} entries",
                    style("✓").green().bold(),
                    total_expired
                );
            }
        }
        Ok(())
    }
}
