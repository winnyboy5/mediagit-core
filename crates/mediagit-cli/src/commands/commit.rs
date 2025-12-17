//! Record changes to the repository.
//!
//! The `commit` command creates a new commit containing the currently staged changes.

use anyhow::{Context, Result};
use clap::Parser;
use mediagit_storage::LocalBackend;
use mediagit_versioning::{Commit, Index, ObjectDatabase, Ref, RefDatabase, Signature, Tree, TreeEntry, FileMode};
use std::sync::Arc;

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
        let repo_root = self.find_repo_root()?;

        if self.dry_run {
            output::info("Running in dry-run mode");
            return Ok(());
        }

        if !self.quiet {
            output::progress("Creating commit...");
        }

        // Initialize storage and databases
        let storage_path = repo_root.join(".mediagit");
        let storage: Arc<dyn mediagit_storage::StorageBackend> =
            Arc::new(LocalBackend::new(&storage_path).await?);
        let odb = ObjectDatabase::new(storage.clone(), 1000);
        let refdb = RefDatabase::new(&storage_path);

        // Load the index
        let index = Index::load(&repo_root)?;

        // Check if there are staged changes
        if index.is_empty() && !self.allow_empty {
            output::warning("No changes staged for commit");
            output::info("Use \"mediagit add <file>...\" to stage changes");
            output::info("Use \"mediagit commit --allow-empty\" to create an empty commit");
            return Ok(());
        }

        // Get current HEAD to find parent (resolve symbolic refs) - do this before building tree
        let parent_oid = refdb.resolve("HEAD").await.ok();

        // Build tree from parent commit (if exists) + index entries
        // Each commit should be a complete snapshot, not just changes
        let mut tree = Tree::new();

        // First, if we have a parent commit, copy all its tree entries
        if let Some(parent_oid_val) = &parent_oid {
            // Read parent commit and its tree
            let parent_commit_data = odb.read(parent_oid_val).await?;
            let parent_commit: mediagit_versioning::Commit =
                bincode::deserialize(&parent_commit_data)
                    .context("Failed to deserialize parent commit")?;

            let parent_tree_data = odb.read(&parent_commit.tree).await?;
            let parent_tree: Tree = bincode::deserialize(&parent_tree_data)
                .context("Failed to deserialize parent tree")?;

            // Copy all entries from parent tree
            for entry in parent_tree.iter() {
                tree.add_entry(entry.clone());
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

        // Update HEAD reference
        let head_ref = refdb.read("HEAD").await?;
        match head_ref {
            Ref {
                ref_type: mediagit_versioning::RefType::Symbolic,
                target: Some(branch),
                ..
            } => {
                // Update branch reference
                let branch_ref = Ref::new_direct(branch.clone(), commit_oid);
                refdb
                    .write(&branch_ref)
                    .await
                    .context("Failed to update branch reference")?;
            }
            _ => {
                anyhow::bail!("HEAD is not pointing to a branch");
            }
        }

        // Clear the index after successful commit
        let mut index = Index::load(&repo_root)?;
        index.clear();
        index.save(&repo_root)
            .context("Failed to clear index")?;

        if !self.quiet {
            output::success(&format!("Created commit {}", commit_oid));
            if self.verbose {
                output::detail("Message", message);
                output::detail("Author", &format!("{} <{}>", author_name, author_email));
            }
        }

        Ok(())
    }

    fn find_repo_root(&self) -> Result<std::path::PathBuf> {
        let mut current = std::env::current_dir()?;

        loop {
            if current.join(".mediagit").exists() {
                return Ok(current);
            }

            if !current.pop() {
                anyhow::bail!("Not a mediagit repository");
            }
        }
    }
}
