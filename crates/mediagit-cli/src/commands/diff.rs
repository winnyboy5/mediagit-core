use anyhow::{Context, Result};
use clap::Parser;
use console::style;
use mediagit_storage::LocalBackend;
use mediagit_versioning::{resolve_revision, Commit, ObjectDatabase, Oid, RefDatabase};
use std::sync::Arc;

/// Show changes between commits
///
/// Display differences between commits, commit and working tree, or between
/// the index and working tree.
#[derive(Parser, Debug)]
#[command(after_help = "EXAMPLES:
    # Show changes between two commits
    mediagit diff abc123 def456

    # Show changes from HEAD to working directory
    mediagit diff HEAD

    # Show staged changes
    mediagit diff --cached

    # Show changes with statistics
    mediagit diff --stat abc123 def456

    # Show changes for specific files
    mediagit diff -- path/to/file.psd

    # Compare with previous commit
    mediagit diff HEAD~1 HEAD

SEE ALSO:
    mediagit-status(1), mediagit-log(1), mediagit-show(1)")]
pub struct DiffCmd {
    /// First revision to compare
    #[arg(value_name = "REVISION1")]
    pub from: Option<String>,

    /// Second revision to compare
    #[arg(value_name = "REVISION2")]
    pub to: Option<String>,

    /// Compare with working directory
    #[arg(long)]
    pub cached: bool,

    /// Show word-level changes
    #[arg(long)]
    pub word_diff: bool,

    /// Show statistics
    #[arg(long)]
    pub stat: bool,

    /// Show summary
    #[arg(long)]
    pub summary: bool,

    /// Number of context lines
    #[arg(short = 'U', long, value_name = "NUM")]
    pub unified: Option<usize>,

    /// Paths to diff
    #[arg(value_name = "PATHS")]
    pub paths: Vec<String>,

    /// Quiet mode
    #[arg(short, long)]
    pub quiet: bool,
}

impl DiffCmd {
    pub async fn execute(&self) -> Result<()> {
        if self.quiet {
            return Ok(());
        }

        let repo_root = self.find_repo_root()?;
        let storage_path = repo_root.join(".mediagit");
        let storage: Arc<dyn mediagit_storage::StorageBackend> =
            Arc::new(LocalBackend::new(&storage_path).await?);
        let refdb = RefDatabase::new(&storage_path);
        let odb = ObjectDatabase::new(storage, 1000);

        // Resolve commits using revision parser (supports HEAD~N)
        let (from_oid, to_oid) = self.resolve_commits(&refdb, &odb).await?;

        // Read commits
        let from_data = odb.read(&from_oid).await?;
        let from_commit = Commit::deserialize(&from_data)
            .context(format!("Failed to deserialize commit {}", from_oid))?;

        let to_data = odb.read(&to_oid).await?;
        let to_commit = Commit::deserialize(&to_data)
            .context(format!("Failed to deserialize commit {}", to_oid))?;

        // Display diff header
        println!("{} Comparing commits:", style("📊").cyan().bold());
        println!("  From: {} ({})", from_oid, from_commit.message.lines().next().unwrap_or(""));
        println!("  To:   {} ({})", to_oid, to_commit.message.lines().next().unwrap_or(""));
        println!();

        // Basic tree comparison
        if from_commit.tree == to_commit.tree {
            println!("{}", style("No changes between commits").dim());
        } else {
            println!("{}", style("Trees differ:").bold());
            println!("  From tree: {}", from_commit.tree);
            println!("  To tree:   {}", to_commit.tree);
            println!();
            println!("{}", style("Full diff functionality requires tree traversal and comparison").dim());
            println!("{}", style("This feature will be enhanced in a future release").dim());
        }

        Ok(())
    }

    async fn resolve_commits(&self, refdb: &RefDatabase, odb: &ObjectDatabase) -> Result<(Oid, Oid)> {
        let from_oid = if let Some(from) = &self.from {
            resolve_revision(from, refdb, odb).await
                .context(format!("Cannot resolve from revision: {}", from))?
        } else {
            // Default: Use HEAD
            resolve_revision("HEAD", refdb, odb).await
                .context("No commits yet")?
        };

        let to_oid = if let Some(to) = &self.to {
            resolve_revision(to, refdb, odb).await
                .context(format!("Cannot resolve to revision: {}", to))?
        } else {
            // Default: Use HEAD
            resolve_revision("HEAD", refdb, odb).await
                .context("No commits yet")?
        };

        Ok((from_oid, to_oid))
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
