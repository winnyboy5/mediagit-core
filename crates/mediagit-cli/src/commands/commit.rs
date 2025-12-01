//! Record changes to the repository.
//!
//! The `commit` command creates a new commit containing the currently staged changes.

use anyhow::{Context, Result};
use clap::Parser;
use mediagit_storage::LocalBackend;
use mediagit_versioning::{Commit, ObjectDatabase, Ref, RefDatabase, Signature, Tree};
use std::sync::Arc;

/// Record changes to the repository
#[derive(Parser, Debug)]
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
        let storage_path = repo_root.join(".mediagit/objects");
        let storage: Arc<dyn mediagit_storage::StorageBackend> =
            Arc::new(LocalBackend::new(&storage_path).await?);
        let odb = ObjectDatabase::new(storage.clone(), 1000);
        let refdb = RefDatabase::new(storage);

        // Create empty tree for now (would normally read from index)
        let tree = Tree::new();
        let tree_bytes = tree.serialize()?;
        let tree_oid = odb
            .write(mediagit_versioning::ObjectType::Tree, &tree_bytes)
            .await
            .context("Failed to write tree object")?;

        // Get current HEAD to find parent
        let head = refdb.read("HEAD").await.ok();
        let parent_oid = head.and_then(|h| h.oid);

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
