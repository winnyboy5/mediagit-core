//! Record changes to the repository.
//!
//! The `commit` command creates a new commit containing the currently staged changes.

use anyhow::{Context, Result};
use clap::Parser;
use mediagit_versioning::{Commit, Index, ObjectDatabase, Oid, Ref, RefDatabase, Reflog, ReflogEntry, Signature, Tree, TreeEntry, FileMode};
use super::super::repo::{find_repo_root, create_storage_backend};

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

    /// Add untracked files to index and commit
    #[arg(long)]
    pub include: bool,

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

        // Validate inputs
        if self.message.is_none() && !self.edit && self.file.is_none() {
            return Err(anyhow::anyhow!(
                "please provide a commit message with -m, -F, or -e"
            ));
        }

        let message = self.message.as_deref().unwrap_or("Initial commit");

        // Validate empty message (ISS-007 fix)
        if message.trim().is_empty() {
            return Err(anyhow::anyhow!(
                "aborting commit due to empty commit message"
            ));
        }

        // Find repository root
        let repo_root = find_repo_root()?;

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
                bincode::deserialize(&parent_commit_data)
                    .context("Failed to deserialize parent commit")?;

            let parent_tree_data = odb.read(&parent_commit.tree).await?;
            let parent_tree: Tree = bincode::deserialize(&parent_tree_data)
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
        let author_name = std::env::var("GIT_AUTHOR_NAME")
            .or_else(|_| std::env::var("USER"))
            .unwrap_or_else(|_| "Unknown".to_string());
        let author_email = std::env::var("GIT_AUTHOR_EMAIL")
            .unwrap_or_else(|_| "unknown@localhost".to_string());

        let signature = Signature::now(author_name.clone(), author_email.clone());

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
        index.save(&repo_root)
            .context("Failed to clear index")?;

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
            _ => {
                Err(anyhow::anyhow!("HEAD is in an invalid state"))
            }
        };

        // If ref update failed, restore the index backup
        if let Err(e) = ref_update_result {
            // Attempt to restore index - log but don't fail on restore error
            if let Err(restore_err) = index_backup.save(&repo_root) {
                tracing::error!("Failed to restore index after ref update failure: {}", restore_err);
            }
            return Err(e);
        }

        // Record reflog entry for HEAD and the branch
        let reflog = Reflog::new(&storage_path);
        let old_oid = parent_oid.unwrap_or_else(|| Oid::from_bytes([0u8; 32]));
        let reflog_msg = format!("commit: {}", message);
        let entry = ReflogEntry::now(old_oid, commit_oid, &author_name, &author_email, &reflog_msg);
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
                output::detail("Message", message);
                output::detail("Author", &format!("{} <{}>", author_name, author_email));
            }
        }

        Ok(())
    }

}
