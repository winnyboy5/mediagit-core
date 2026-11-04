// SPDX-License-Identifier: BUSL-1.1
// Copyright (C) 2025-2026 Aswin Krishnamoorthy

//! Show reference logs.
//!
//! The reflog command displays a log of when the tips of branches and other
//! references were updated in the local repository.

use anyhow::Result;
use clap::{Parser, Subcommand};
use console::style;
use mediagit_versioning::Reflog;

use serde::Serialize;

use super::super::output;
use super::super::repo::find_repo_root;

/// `--machine` data for `reflog [show] [<ref>]`, newest entry first.
#[derive(Debug, Clone, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[cfg_attr(test, ts(export, export_to = "../../../ui/src/api/generated/"))]
pub struct ReflogReport {
    pub reference: String,
    pub entries: Vec<ReflogEntryReport>,
}

/// `--machine` data for `reflog show --all`: the newest entries of every ref.
#[derive(Debug, Clone, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[cfg_attr(test, ts(export, export_to = "../../../ui/src/api/generated/"))]
pub struct ReflogAllReport {
    pub refs: Vec<ReflogReport>,
}

/// `--machine` data for `reflog delete` and `reflog expire`.
#[derive(Debug, Clone, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[cfg_attr(test, ts(export, export_to = "../../../ui/src/api/generated/"))]
pub struct ReflogMaintainReport {
    /// `delete` or `expire`.
    pub action: String,
    /// `delete`: whether the reflog existed. `expire`: whether anything went.
    pub changed: bool,
    /// `expire`: entries removed.
    #[cfg_attr(test, ts(type = "number"))]
    pub expired: u64,
    /// Refs the action touched.
    pub refs: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[cfg_attr(test, ts(export, export_to = "../../../ui/src/api/generated/"))]
pub struct ReflogEntryReport {
    /// `<ref>@{index}`.
    #[cfg_attr(test, ts(type = "number"))]
    pub index: u64,
    pub old_oid: String,
    pub new_oid: String,
    pub message: String,
    /// Unix seconds.
    #[cfg_attr(test, ts(type = "number"))]
    pub time: i64,
    pub committer: String,
}

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
    mediagit reflog expire --keep=30

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

        /// Confirm the deletion (required with --machine)
        #[arg(long)]
        yes: bool,
    },

    /// Prune/expire old reflog entries
    Expire {
        /// Reference to expire (default: all)
        #[arg(value_name = "REF")]
        reference: Option<String>,

        /// Number of entries to keep
        #[arg(long, default_value = "90")]
        keep: usize,

        /// Confirm the expiry (required with --machine)
        #[arg(long)]
        yes: bool,
    },
}

fn entry_report(ref_name: &str, entries: &[mediagit_versioning::ReflogEntry]) -> ReflogReport {
    ReflogReport {
        reference: ref_name.to_string(),
        entries: entries
            .iter()
            .enumerate()
            .map(|(i, e)| ReflogEntryReport {
                index: i as u64,
                old_oid: e.old_oid.to_hex(),
                new_oid: e.new_oid.to_hex(),
                message: e.message.clone(),
                time: e.committer.timestamp.timestamp(),
                committer: e.committer.name.clone(),
            })
            .collect(),
    }
}

impl ReflogCmd {
    pub async fn execute(&self) -> Result<()> {
        // Deleting or expiring reflog entries cannot be undone.
        if let Some(
            ReflogAction::Delete { yes: false, .. } | ReflogAction::Expire { yes: false, .. },
        ) = &self.action
        {
            output::ensure_interactive("reflog confirmation (pass --yes)")?;
        }
        let repo_root = find_repo_root()?;
        let storage_path = repo_root.join(".mediagit");
        let reflog = Reflog::new(&storage_path);

        match &self.action {
            Some(ReflogAction::Show {
                reference,
                count,
                all,
            }) => {
                if *all {
                    self.show_all(&reflog, *count).await
                } else {
                    let ref_name = reference.as_deref().unwrap_or("HEAD");
                    self.show_reflog(&reflog, ref_name, *count).await
                }
            }
            Some(ReflogAction::Delete { reference, .. }) => {
                self.delete_reflog(&reflog, reference).await
            }
            Some(ReflogAction::Expire {
                reference, keep, ..
            }) => {
                self.expire_reflog(&reflog, reference.as_deref(), *keep)
                    .await
            }
            None => {
                // Default action: show reflog
                let ref_name = self.reference.as_deref().unwrap_or("HEAD");
                self.show_reflog(&reflog, ref_name, self.count).await
            }
        }
    }

    async fn show_reflog(
        &self,
        reflog: &Reflog,
        ref_name: &str,
        limit: Option<usize>,
    ) -> Result<()> {
        let entries = reflog.read(ref_name, limit).await?;

        if output::machine_mode() {
            return output::emit_ok(&entry_report(ref_name, &entries));
        }

        if entries.is_empty() {
            if !self.quiet {
                println!("{} No reflog entries for {}", style("ℹ").cyan(), ref_name);
            }
            return Ok(());
        }

        if !self.quiet {
            println!(
                "{} Reflog for {}",
                style("📋").cyan().bold(),
                style(ref_name).yellow()
            );
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

    async fn show_all(&self, reflog: &Reflog, count: Option<usize>) -> Result<()> {
        let refs = reflog.list_refs().await?;

        if output::machine_mode() {
            let mut out = Vec::new();
            for ref_name in refs {
                let entries = reflog.read(&ref_name, count).await?;
                out.push(entry_report(&ref_name, &entries));
            }
            return output::emit_ok(&ReflogAllReport { refs: out });
        }

        if refs.is_empty() {
            println!("{} No reflogs found", style("ℹ").cyan());
            return Ok(());
        }

        println!(
            "{} Found {} refs with reflogs",
            style("📋").cyan().bold(),
            refs.len()
        );
        println!();

        for ref_name in refs {
            let entries = reflog.read(&ref_name, Some(5)).await?;
            if !entries.is_empty() {
                println!(
                    "{} {} ({} entries)",
                    style("→").cyan(),
                    style(&ref_name).yellow(),
                    entries.len()
                );

                for (i, entry) in entries.iter().enumerate().take(3) {
                    let short_oid = &entry.new_oid.to_hex()[..7];
                    println!(
                        "    {}@{{{}}}: {} - {}",
                        ref_name,
                        i,
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
        let existed = reflog.delete(ref_name).await?;
        if output::machine_mode() {
            return output::emit_ok(&ReflogMaintainReport {
                action: "delete".into(),
                changed: existed,
                expired: 0,
                refs: if existed {
                    vec![ref_name.to_string()]
                } else {
                    Vec::new()
                },
            });
        }
        if existed {
            println!("{} Deleted reflog for {}", style("✓").green(), ref_name);
        } else {
            println!("{} No reflog found for {}", style("ℹ").cyan(), ref_name);
        }
        Ok(())
    }

    async fn expire_reflog(
        &self,
        reflog: &Reflog,
        ref_name: Option<&str>,
        keep: usize,
    ) -> Result<()> {
        if output::machine_mode() {
            let targets = match ref_name {
                Some(r) => vec![r.to_string()],
                None => reflog.list_refs().await?,
            };
            let mut total = 0u64;
            let mut touched = Vec::new();
            for r in targets {
                let n = reflog.expire(&r, keep).await? as u64;
                if n > 0 {
                    total += n;
                    touched.push(r);
                }
            }
            return output::emit_ok(&ReflogMaintainReport {
                action: "expire".into(),
                changed: total > 0,
                expired: total,
                refs: touched,
            });
        }
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
                println!(
                    "{} No entries to expire for {}",
                    style("ℹ").cyan(),
                    ref_name
                );
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
