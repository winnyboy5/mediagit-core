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

//! Record changes to the repository.
//!
//! The `commit` command creates a new commit containing the currently staged changes.

use super::super::repo::{create_storage_backend, find_repo_root};
use anyhow::{Context, Result};
use clap::Parser;
use mediagit_versioning::{
    Commit, FileMode, Index, ObjectDatabase, Oid, Ref, RefDatabase, Reflog, ReflogEntry, Signature,
    Tree, TreeEntry,
};

/// Record changes to the repository
///
/// Creates a new commit containing the currently staged changes. The commit
/// captures a snapshot of the project's currently staged changes along with
/// a descriptive message from the user.
#[derive(Parser, Debug)]
#[command(after_help = "EXAMPLES:
    # Commit staged changes with inline message
    mediagit commit -m \"Add new character model\"

    # Commit all modified tracked files
    mediagit commit -am \"Update texture maps\"

    # Commit with detailed message from file
    mediagit commit -F commit-message.txt

    # Preview what would be committed
    mediagit commit --dry-run

SEE ALSO:
    mediagit-add(1), mediagit-status(1), mediagit-log(1), mediagit-amend(1)")]
pub struct CommitCmd {
    /// Commit message
    #[arg(short, long, value_name = "MESSAGE")]
    pub message: Option<String>,

    /// Edit commit message in text editor
    #[arg(short = 'e', long)]
    pub edit: bool,

    /// Use the given file as the commit message
    #[arg(short = 'F', long, value_name = "FILE")]
    pub file: Option<String>,

    /// Stage modified and deleted files before committing
    #[arg(short = 'a', long)]
    pub all: bool,

    /// Stage listed paths before committing
    #[arg(long, value_name = "PATHS", num_args = 0..)]
    pub include: Vec<String>,

    /// Override the commit author
    #[arg(long, value_name = "NAME <EMAIL>")]
    pub author: Option<String>,

    /// Override the commit date
    #[arg(long, value_name = "DATE")]
    pub date: Option<String>,

    /// Allow empty commits
    #[arg(long)]
    pub allow_empty: bool,

    /// Sign off the commit
    #[arg(short = 's', long)]
    pub signoff: bool,

    /// Show what would be committed
    #[arg(long)]
    pub dry_run: bool,

    /// Quiet mode
    #[arg(short, long)]
    pub quiet: bool,

    /// Verbose mode
    #[arg(short, long)]
    pub verbose: bool,
}

impl CommitCmd {
    pub async fn execute(&self) -> Result<()> {
        use crate::output;

        // The -a (--all) flag is not supported in MediaGit.
        // MediaGit uses an explicit `add` → `commit` workflow by design,
        // because `add` performs heavy processing (chunking, delta encoding,
        // compression) that is inappropriate to silently trigger from commit.
        if self.all {
            return Err(anyhow::anyhow!(
                "commit -a is not supported in MediaGit.\n\
                 Use 'mediagit add .' followed by 'mediagit commit -m \"...\"' instead."
            ));
        }

        // Determine the commit message with git's precedence: -m wins, then
        // -F (read from file), then -e (or no source, if a tty) opens an
        // editor. If none of -m/-F/-e was given and stdin isn't a tty, there
        // is no way to obtain a message non-interactively.
        let message: String = if let Some(m) = &self.message {
            m.clone()
        } else if let Some(file_path) = &self.file {
            std::fs::read_to_string(file_path)
                .with_context(|| format!("Failed to read commit message file: {}", file_path))?
                .trim_end_matches(['\n', '\r'])
                .to_string()
        } else if self.edit || std::io::IsTerminal::is_terminal(&std::io::stdin()) {
            edit_commit_message()?
        } else {
            return Err(anyhow::anyhow!("no commit message provided"));
        };

        // Validate empty message (ISS-007 fix)
        if message.trim().is_empty() {
            return Err(anyhow::anyhow!(
                "Aborting commit due to empty commit message"
            ));
        }

        // Find repository root (needed for config loading, signoff identity resolution)
        let repo_root = find_repo_root()?;

        // Stage paths if --include is used
        if !self.include.is_empty() {
            let staged_count =
                super::add::stage_files_for_commit(&self.include, &repo_root).await?;
            if !self.quiet {
                output::info(&format!("Staged {} file(s) from --include", staged_count));
            }
        }

        // Apply --signoff if requested (after resolving author identity below)
        // Defer actual appending until after author_name/email are resolved

        if self.dry_run {
            output::info("Running in dry-run mode");
            return Ok(());
        }

        if !self.quiet {
            output::progress("Creating commit...");
        }

        // Initialize storage and databases
        let storage_path = repo_root.join(".mediagit");
        let storage = create_storage_backend(&repo_root).await?;
        let odb = ObjectDatabase::with_smart_compression(storage.clone(), 1000);
        let refdb = RefDatabase::new(&storage_path);

        // Load the index
        let index = Index::load(&repo_root)?;

        // Check if there are staged changes
        if index.is_empty() && !self.allow_empty {
            output::warning("No changes staged for commit");
            output::info("Use \"mediagit add <file>...\" to stage changes");
            output::info("Use \"mediagit commit --allow-empty\" to create an empty commit");
            anyhow::bail!("nothing to commit");
        }

        // Get current HEAD to find parent (resolve symbolic refs) - do this before building tree
        let parent_oid = refdb.resolve("HEAD").await.ok();

        // Build tree from parent commit (if exists) + index entries
        // Each commit should be a complete snapshot, not just changes
        let mut tree = Tree::new();

        // First, if we have a parent commit, copy all its tree entries
        // BUT skip files that are marked for deletion in the index
        if let Some(parent_oid_val) = &parent_oid {
            // Read parent commit and its tree
            let parent_commit_data = odb.read(parent_oid_val).await?;
            let parent_commit: mediagit_versioning::Commit =
                mediagit_versioning::format::deserialize(&parent_commit_data)
                    .context("Failed to deserialize parent commit")?;

            let parent_tree_data = odb.read(&parent_commit.tree).await?;
            let parent_tree: Tree = mediagit_versioning::format::deserialize(&parent_tree_data)
                .context("Failed to deserialize parent tree")?;

            // Build a set of deleted paths for fast lookup (normalized for cross-platform)
            let deleted_paths: std::collections::HashSet<String> = index
                .deleted_paths()
                .map(|p| p.to_string_lossy().replace('\\', "/"))
                .collect();

            // Copy entries from parent tree, but skip deleted ones
            for entry in parent_tree.iter() {
                // Normalize entry name for comparison
                let entry_name_normalized = entry.name.replace('\\', "/");
                if !deleted_paths.contains(&entry_name_normalized) {
                    tree.add_entry(entry.clone());
                }
            }
        }

        // Then, add/update entries from index (these override parent entries with same name)
        for entry in index.entries() {
            let file_mode = if entry.mode & 0o111 != 0 {
                FileMode::Executable
            } else {
                FileMode::Regular
            };

            // Use full path, not just filename
            tree.add_entry(TreeEntry::new(
                entry.path.to_string_lossy().to_string(),
                file_mode,
                entry.oid,
            ));
        }

        let tree_bytes = tree.serialize()?;
        let tree_oid = odb
            .write(mediagit_versioning::ObjectType::Tree, &tree_bytes)
            .await
            .context("Failed to write tree object")?;

        // Create commit signature
        // Priority: --author CLI flag > MEDIAGIT_AUTHOR_* env vars > config.toml [author] > $USER > defaults
        let config = mediagit_config::Config::load(&repo_root)
            .await
            .unwrap_or_default();

        let (author_name, author_email) = if let Some(author_str) = &self.author {
            // Parse "Name <email>" format from --author flag
            if let (Some(lt), Some(gt)) = (author_str.rfind('<'), author_str.rfind('>')) {
                let name = author_str[..lt].trim().to_string();
                let email = author_str[lt + 1..gt].trim().to_string();
                (name, email)
            } else {
                (author_str.clone(), "unknown@localhost".to_string())
            }
        } else {
            let name = std::env::var("MEDIAGIT_AUTHOR_NAME").unwrap_or_else(|_| {
                // Priority: config.toml [author].name > $USER > fallback
                config.author.name.clone().unwrap_or_else(|| {
                    std::env::var("USER").unwrap_or_else(|_| "Unknown".to_string())
                })
            });
            let email = std::env::var("MEDIAGIT_AUTHOR_EMAIL").unwrap_or_else(|_| {
                config.author.email.clone().unwrap_or_else(|| {
                    // Derive email from $USER@localhost if available
                    std::env::var("USER")
                        .map(|u| format!("{}@localhost", u))
                        .unwrap_or_else(|_| "unknown@localhost".to_string())
                })
            });
            (name, email)
        };

        // Apply --signoff if requested (append after author identity is resolved)
        let message = if self.signoff {
            let signoff_line = format!("Signed-off-by: {} <{}>", author_name, author_email);
            // Only append if not already present
            if message.contains(&signoff_line) {
                message
            } else {
                format!("{}\n\n{}", message, signoff_line)
            }
        } else {
            message
        };

        // Parse --date if provided, otherwise use current time
        let signature = if let Some(date_str) = &self.date {
            // Parse RFC3339 date format (e.g., "2026-01-15T10:30:00Z")
            let date = chrono::DateTime::parse_from_rfc3339(date_str)
                .with_context(|| format!("Invalid RFC3339 date format: {}", date_str))?;
            let utc_date = date.with_timezone(&chrono::Utc);
            Signature::new(author_name.clone(), author_email.clone(), utc_date)
        } else {
            Signature::now(author_name.clone(), author_email.clone())
        };

        // Create commit object
        let commit = if let Some(parent) = parent_oid {
            Commit::with_parents(
                tree_oid,
                vec![parent],
                signature.clone(),
                signature,
                message.to_string(),
            )
        } else {
            Commit::new(tree_oid, signature.clone(), signature, message.to_string())
        };

        // Serialize and write commit
        let commit_bytes = commit.serialize()?;
        let commit_oid = odb
            .write(mediagit_versioning::ObjectType::Commit, &commit_bytes)
            .await
            .context("Failed to write commit object")?;

        // Clear the index BEFORE updating refs for atomicity
        // If ref update fails after this, user can re-stage and retry.
        // This prevents the issue where ref is updated but index isn't cleared.
        let mut index = Index::load(&repo_root)?;
        let index_backup = index.clone();
        index.clear();
        index.save(&repo_root).context("Failed to clear index")?;

        // Update HEAD reference
        let head_ref = refdb.read("HEAD").await?;
        let ref_update_result = match head_ref {
            Ref {
                ref_type: mediagit_versioning::RefType::Symbolic,
                target: Some(branch),
                ..
            } => {
                // Update branch reference (normal case)
                let branch_ref = Ref::new_direct(branch.clone(), commit_oid);
                refdb
                    .write(&branch_ref)
                    .await
                    .context("Failed to update branch reference")
            }
            Ref {
                ref_type: mediagit_versioning::RefType::Direct,
                ..
            } => {
                // Detached HEAD - update HEAD directly to point to new commit
                let head_direct = Ref::new_direct("HEAD".to_string(), commit_oid);
                refdb
                    .write(&head_direct)
                    .await
                    .context("Failed to update HEAD in detached state")
            }
            _ => Err(anyhow::anyhow!("HEAD is in an invalid state")),
        };

        // If ref update failed, restore the index backup
        if let Err(e) = ref_update_result {
            // Attempt to restore index - log but don't fail on restore error
            if let Err(restore_err) = index_backup.save(&repo_root) {
                tracing::error!(
                    "Failed to restore index after ref update failure: {}",
                    restore_err
                );
            }
            return Err(e);
        }

        // Record reflog entry for HEAD and the branch
        let reflog = Reflog::new(&storage_path);
        let old_oid = parent_oid.unwrap_or_else(|| Oid::from_bytes([0u8; 32]));
        let reflog_msg = format!("commit: {}", message);
        let entry = ReflogEntry::now(
            old_oid,
            commit_oid,
            &author_name,
            &author_email,
            &reflog_msg,
        );
        // Best-effort: don't fail the commit if reflog write fails
        let _ = reflog.append("HEAD", &entry).await;
        if let Ok(head_ref) = refdb.read("HEAD").await {
            if let Some(branch) = head_ref.target {
                let _ = reflog.append(&branch, &entry).await;
            }
        }

        if !self.quiet {
            output::success(&format!("Created commit {}", commit_oid));
            if self.verbose {
                output::detail("Message", &message);
                output::detail("Author", &format!("{} <{}>", author_name, author_email));
            }
        }

        // Best-effort auto-gc: reclaims orphans from re-staged files.
        // Thresholded internally, silent unless work was actually done.
        let _ =
            crate::auto_gc::maybe_run(&repo_root, crate::auto_gc::TriggerMode::PostCommit).await;

        Ok(())
    }
}

/// Open an editor (`$EDITOR`/`%EDITOR%`, else `notepad` on Windows or `vi`
/// elsewhere) on a temp file seeded with a commented status hint, matching
/// git's `-e` / no-message-source commit flow. Comment lines (starting with
/// `#`) are stripped from the result; emptiness is validated by the caller.
fn edit_commit_message() -> Result<String> {
    let editor = std::env::var("EDITOR")
        .or_else(|_| std::env::var("VISUAL"))
        .unwrap_or_else(|_| {
            if cfg!(windows) {
                "notepad".to_string()
            } else {
                "vi".to_string()
            }
        });

    let tmp_path =
        std::env::temp_dir().join(format!("MEDIAGIT_COMMIT_EDITMSG_{}", std::process::id()));
    std::fs::write(
        &tmp_path,
        "\n# Please enter the commit message for your changes. Lines starting\n\
         # with '#' will be ignored, and an empty message aborts the commit.\n",
    )
    .context("Failed to create commit message temp file")?;

    let status = std::process::Command::new(&editor)
        .arg(&tmp_path)
        .status()
        .with_context(|| format!("Failed to launch editor: {}", editor));
    let status = match status {
        Ok(s) => s,
        Err(e) => {
            let _ = std::fs::remove_file(&tmp_path);
            return Err(e);
        }
    };

    if !status.success() {
        let _ = std::fs::remove_file(&tmp_path);
        anyhow::bail!("Editor exited with an error, aborting commit");
    }

    let content =
        std::fs::read_to_string(&tmp_path).context("Failed to read commit message temp file")?;
    let _ = std::fs::remove_file(&tmp_path);

    let message = content
        .lines()
        .filter(|l| !l.trim_start().starts_with('#'))
        .collect::<Vec<_>>()
        .join("\n");

    Ok(message.trim().to_string())
}
