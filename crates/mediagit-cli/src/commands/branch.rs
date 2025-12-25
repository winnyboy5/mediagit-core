use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use mediagit_storage::LocalBackend;
use mediagit_versioning::{Ref, RefDatabase};
use std::sync::Arc;
use std::time::Instant;
use crate::progress::{ProgressTracker, OperationStats};

/// Manage branches
///
/// Create, list, rename, and delete branches. Branches are lightweight references
/// to commits that allow parallel development workflows.
#[derive(Parser, Debug)]
#[command(after_help = "EXAMPLES:
    # List all local branches
    mediagit branch list

    # List all branches with verbose output
    mediagit branch list -v

    # Create a new branch
    mediagit branch create feature-branch

    # Create branch from specific commit
    mediagit branch create hotfix abc123

    # Switch to a branch
    mediagit branch switch main

    # Create and switch to new branch
    mediagit branch switch -c feature-branch

    # Rename current branch
    mediagit branch rename new-name

    # Rename specific branch
    mediagit branch rename old-name new-name

    # Delete a branch
    mediagit branch delete feature-branch

    # Show branch information
    mediagit branch show

SEE ALSO:
    mediagit-checkout(1), mediagit-merge(1), mediagit-tag(1)")]
pub struct BranchCmd {
    #[command(subcommand)]
    pub subcommand: BranchSubcommand,
}

#[derive(Subcommand, Debug)]
pub enum BranchSubcommand {
    /// List branches
    #[command(alias = "ls")]
    List(ListOpts),

    /// Create a new branch
    Create(CreateOpts),

    /// Switch to a branch
    #[command(alias = "checkout", alias = "co")]
    Switch(SwitchOpts),

    /// Delete a branch
    #[command(alias = "rm")]
    Delete(DeleteOpts),

    /// Protect a branch
    Protect(ProtectOpts),

    /// Rename a branch
    #[command(alias = "move", alias = "mv")]
    Rename(RenameOpts),

    /// Show branch information
    Show(ShowOpts),

    /// Merge a branch
    Merge(MergeOpts),
}

/// List branches
#[derive(Parser, Debug)]
pub struct ListOpts {
    /// List remote branches
    #[arg(short, long)]
    pub remote: bool,

    /// List all branches (local and remote)
    #[arg(short = 'a', long)]
    pub all: bool,

    /// Show verbose output
    #[arg(short, long)]
    pub verbose: bool,

    /// Quiet mode
    #[arg(short, long)]
    pub quiet: bool,

    /// Sort branches
    #[arg(long, value_name = "KEY")]
    pub sort: Option<String>,
}

/// Create a new branch
#[derive(Parser, Debug)]
pub struct CreateOpts {
    /// Branch name
    #[arg(value_name = "NAME")]
    pub name: String,

    /// Start point (defaults to HEAD)
    #[arg(value_name = "START_POINT")]
    pub start_point: Option<String>,

    /// Set upstream branch
    #[arg(short = 'u', long, value_name = "UPSTREAM")]
    pub set_upstream: Option<String>,

    /// Track a remote branch
    #[arg(long)]
    pub track: bool,

    /// Don't track
    #[arg(long)]
    pub no_track: bool,

    /// Quiet mode
    #[arg(short, long)]
    pub quiet: bool,
}

/// Switch to a branch
#[derive(Parser, Debug)]
pub struct SwitchOpts {
    /// Branch name
    #[arg(value_name = "BRANCH")]
    pub branch: String,

    /// Create and switch to new branch
    #[arg(short, long)]
    pub create: bool,

    /// Force switch even if local changes
    #[arg(short = 'f', long)]
    pub force: bool,

    /// Don't check out files
    #[arg(long)]
    pub no_guess: bool,

    /// Quiet mode
    #[arg(short, long)]
    pub quiet: bool,
}

/// Delete a branch
#[derive(Parser, Debug)]
pub struct DeleteOpts {
    /// Branch names to delete
    #[arg(value_name = "BRANCHES", required = true)]
    pub branches: Vec<String>,

    /// Force delete (ignore merge status)
    #[arg(short = 'D', long)]
    pub force: bool,

    /// Delete only if merged
    #[arg(short = 'd', long)]
    pub delete_merged: bool,

    /// Quiet mode
    #[arg(short, long)]
    pub quiet: bool,
}

/// Protect a branch
#[derive(Parser, Debug)]
pub struct ProtectOpts {
    /// Branch name
    #[arg(value_name = "BRANCH")]
    pub branch: String,

    /// Require pull request reviews before merge
    #[arg(long)]
    pub require_reviews: bool,

    /// Unprotect the branch
    #[arg(long)]
    pub unprotect: bool,

    /// Quiet mode
    #[arg(short, long)]
    pub quiet: bool,
}

/// Rename a branch
#[derive(Parser, Debug)]
pub struct RenameOpts {
    /// New branch name
    #[arg(value_name = "NEW_NAME")]
    pub new_name: String,

    /// Old branch name (current branch if not specified)
    #[arg(value_name = "OLD_NAME")]
    pub old_name: Option<String>,

    /// Force rename
    #[arg(short = 'f', long)]
    pub force: bool,

    /// Quiet mode
    #[arg(short, long)]
    pub quiet: bool,
}

/// Show branch information
#[derive(Parser, Debug)]
pub struct ShowOpts {
    /// Branch name (current branch if not specified)
    #[arg(value_name = "BRANCH")]
    pub branch: Option<String>,

    /// Show verbose information
    #[arg(short, long)]
    pub verbose: bool,
}

/// Merge a branch
#[derive(Parser, Debug)]
pub struct MergeOpts {
    /// Branch to merge
    #[arg(value_name = "BRANCH", required = true)]
    pub branch: String,

    /// Create a merge commit
    #[arg(long)]
    pub no_ff: bool,

    /// Perform a fast-forward only merge
    #[arg(long)]
    pub ff_only: bool,

    /// Merge message
    #[arg(short, long, value_name = "MESSAGE")]
    pub message: Option<String>,

    /// Quit if merge conflicts occur
    #[arg(long)]
    pub abort: bool,

    /// Continue after resolving conflicts
    #[arg(long)]
    pub continue_merge: bool,

    /// Quiet mode
    #[arg(short, long)]
    pub quiet: bool,
}

impl BranchCmd {
    pub async fn execute(&self) -> Result<()> {
        match &self.subcommand {
            BranchSubcommand::List(opts) => self.list(opts).await,
            BranchSubcommand::Create(opts) => self.create(opts).await,
            BranchSubcommand::Switch(opts) => self.switch(opts).await,
            BranchSubcommand::Delete(opts) => self.delete(opts).await,
            BranchSubcommand::Protect(opts) => self.protect(opts).await,
            BranchSubcommand::Rename(opts) => self.rename(opts).await,
            BranchSubcommand::Show(opts) => self.show(opts).await,
            BranchSubcommand::Merge(opts) => self.merge(opts).await,
        }
    }

    async fn list(&self, opts: &ListOpts) -> Result<()> {
        use crate::output;

        let repo_root = self.find_repo_root()?;
        let storage_path = repo_root.join(".mediagit");
        let _storage: Arc<dyn mediagit_storage::StorageBackend> =
            Arc::new(LocalBackend::new(&storage_path).await?);
        let refdb = RefDatabase::new(&storage_path);

        // List local branches from refs/heads (pass namespace only, not full path)
        let branches = refdb.list("heads").await?;

        if branches.is_empty() {
            if !opts.quiet {
                output::info("No branches found");
            }
            return Ok(());
        }

        // Get current branch
        let head = refdb.read("HEAD").await.ok();
        let current_branch = head.and_then(|h| h.target);

        for branch_name in branches {
            let is_current = current_branch
                .as_ref()
                .map(|cb| cb == &branch_name)
                .unwrap_or(false);

            let prefix = if is_current { "* " } else { "  " };
            let display_name = branch_name.strip_prefix("refs/heads/").unwrap_or(&branch_name);

            if opts.verbose {
                let branch_ref = refdb.read(&branch_name).await.ok();
                let oid_display = branch_ref.and_then(|r| r.oid).map(|o| o.to_string()).unwrap_or_else(|| "unknown".to_string());
                output::info(&format!("{}{} -> {}", prefix, display_name, oid_display));
            } else {
                println!("{}{}", prefix, display_name);
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

    async fn create(&self, opts: &CreateOpts) -> Result<()> {
        use crate::output;

        let repo_root = self.find_repo_root()?;
        let storage_path = repo_root.join(".mediagit");
        let _storage: Arc<dyn mediagit_storage::StorageBackend> =
            Arc::new(LocalBackend::new(&storage_path).await?);
        let refdb = RefDatabase::new(&storage_path);

        // Validate branch name
        if opts.name.contains("..") || opts.name.starts_with('/') || opts.name.ends_with('/') {
            anyhow::bail!("Invalid branch name: {}", opts.name);
        }

        let branch_ref_name = format!("refs/heads/{}", opts.name);

        // Check if branch already exists
        if refdb.read(&branch_ref_name).await.is_ok() {
            anyhow::bail!("Branch '{}' already exists", opts.name);
        }

        // Get start point (defaults to HEAD) - resolve symbolic refs
        let start_oid = if let Some(start_point) = &opts.start_point {
            // Try to resolve the start point as a reference or commit
            refdb.resolve(start_point).await
                .context(format!("Invalid start point: {}", start_point))?
        } else {
            // Use HEAD as start point (resolve symbolic ref)
            refdb.resolve("HEAD").await
                .context("HEAD has no commit yet")?
        };

        // Create the branch reference
        let branch_ref = Ref::new_direct(branch_ref_name.clone(), start_oid);
        refdb.write(&branch_ref).await?;

        if !opts.quiet {
            output::success(&format!("Created branch '{}' at {}", opts.name, start_oid));
        }

        Ok(())
    }

    async fn switch(&self, opts: &SwitchOpts) -> Result<()> {
        use crate::output;
        use mediagit_versioning::{CheckoutManager, Index, ObjectDatabase};

        let start_time = Instant::now();
        let mut stats = OperationStats::new();
        let progress = ProgressTracker::new(opts.quiet);

        let repo_root = self.find_repo_root()?;
        let storage_path = repo_root.join(".mediagit");
        let storage: Arc<dyn mediagit_storage::StorageBackend> =
            Arc::new(LocalBackend::new(&storage_path).await?);
        let refdb = RefDatabase::new(&storage_path);

        // Strip refs/heads/ prefix if already present
        let branch_name = opts.branch.strip_prefix("refs/heads/").unwrap_or(&opts.branch);
        let branch_ref_name = format!("refs/heads/{}", branch_name);

        // If create flag is set, create the branch first
        if opts.create {
            // Check if branch already exists
            if refdb.read(&branch_ref_name).await.is_ok() {
                anyhow::bail!("Branch '{}' already exists", opts.branch);
            }

            // Get current HEAD for start point (resolve symbolic ref)
            let start_oid = refdb.resolve("HEAD").await
                .context("HEAD has no commit yet")?;

            // Create the branch reference
            let branch_ref = Ref::new_direct(branch_ref_name.clone(), start_oid);
            refdb.write(&branch_ref).await?;

            if !opts.quiet {
                output::success(&format!("Created branch '{}'", opts.branch));
            }
        } else {
            // Verify branch exists
            refdb.read(&branch_ref_name)
                .await
                .context(format!("Branch '{}' not found", opts.branch))?;
        }

        // Get the commit that the target branch points to
        let target_commit_oid = refdb.resolve(&branch_ref_name).await
            .context(format!("Failed to resolve branch '{}' to a commit", opts.branch))?;

        // Update HEAD to point to the branch
        let head = Ref::new_symbolic("HEAD".to_string(), branch_ref_name.clone());
        refdb.write(&head).await?;

        // Update working directory to match the target branch's commit
        let odb = ObjectDatabase::with_smart_compression(storage.clone(), 1000);
        let checkout_mgr = CheckoutManager::new(&odb, &repo_root);

        let checkout_pb = progress.spinner("Updating working directory");
        let files_updated = checkout_mgr.checkout_commit(&target_commit_oid).await
            .context("Failed to update working directory")?;
        checkout_pb.finish_with_message("Working directory updated");

        stats.files_updated = files_updated as u64;

        // Clear the index (staging area) when switching branches
        // This ensures a clean state on the new branch
        let mut index = Index::load(&repo_root)?;
        index.clear();
        index.save(&repo_root)?;

        if !opts.quiet {
            output::success(&format!("Switched to branch '{}'", opts.branch));
            if files_updated > 0 {
                output::info(&format!("Updated {} file(s) in working directory", files_updated));
            }
        }

        // Print operation summary
        stats.duration_ms = start_time.elapsed().as_millis() as u64;
        if !opts.quiet && stats.files_updated > 0 {
            println!("\n📊 {}", stats.summary());
        }

        Ok(())
    }

    async fn delete(&self, opts: &DeleteOpts) -> Result<()> {
        use crate::output;

        let repo_root = self.find_repo_root()?;
        let storage_path = repo_root.join(".mediagit");
        let _storage: Arc<dyn mediagit_storage::StorageBackend> =
            Arc::new(LocalBackend::new(&storage_path).await?);
        let refdb = RefDatabase::new(&storage_path);

        // Get current branch to prevent deletion
        let head = refdb.read("HEAD").await?;
        let current_branch = head.target;

        let mut deleted_count = 0;

        for branch_name in &opts.branches {
            let branch_ref_name = format!("refs/heads/{}", branch_name);

            // Check if trying to delete current branch
            if Some(&branch_ref_name) == current_branch.as_ref() {
                if !opts.quiet {
                    output::warning(&format!("Cannot delete current branch '{}'", branch_name));
                }
                continue;
            }

            // Verify branch exists
            match refdb.read(&branch_ref_name).await {
                Ok(_) => {
                    // Delete the branch reference
                    refdb.delete(&branch_ref_name).await?;
                    deleted_count += 1;

                    if !opts.quiet {
                        output::success(&format!("Deleted branch '{}'", branch_name));
                    }
                }
                Err(_) => {
                    if !opts.quiet {
                        output::warning(&format!("Branch '{}' not found", branch_name));
                    }
                }
            }
        }

        if !opts.quiet && deleted_count == 0 {
            output::info("No branches were deleted");
        }

        Ok(())
    }

    async fn protect(&self, _opts: &ProtectOpts) -> Result<()> {
        // NOTE: Branch protection implementation pending
        // Requires: protection rules storage, validation enforcement
        anyhow::bail!("Branch protection not yet implemented")
    }

    async fn rename(&self, opts: &RenameOpts) -> Result<()> {
        use crate::output;

        let repo_root = self.find_repo_root()?;
        let storage_path = repo_root.join(".mediagit");
        let _storage: Arc<dyn mediagit_storage::StorageBackend> =
            Arc::new(LocalBackend::new(&storage_path).await?);
        let refdb = RefDatabase::new(&storage_path);

        // Determine old branch name (current branch if not specified)
        let old_branch = if let Some(old_name) = &opts.old_name {
            old_name.clone()
        } else {
            // Get current branch from HEAD
            let head = refdb.read("HEAD").await?;
            match head.target {
                Some(target) => target
                    .strip_prefix("refs/heads/")
                    .unwrap_or(&target)
                    .to_string(),
                None => anyhow::bail!("HEAD is not pointing to a branch"),
            }
        };

        let old_ref_name = format!("refs/heads/{}", old_branch);
        let new_ref_name = format!("refs/heads/{}", opts.new_name);

        // Validate new branch name
        if opts.new_name.contains("..") || opts.new_name.starts_with('/') || opts.new_name.ends_with('/') {
            anyhow::bail!("Invalid branch name: {}", opts.new_name);
        }

        // Check if old branch exists
        let old_ref = refdb
            .read(&old_ref_name)
            .await
            .context(format!("Branch '{}' not found", old_branch))?;

        // Check if new branch already exists (unless force)
        if !opts.force && refdb.read(&new_ref_name).await.is_ok() {
            anyhow::bail!("Branch '{}' already exists. Use --force to overwrite.", opts.new_name);
        }

        // Get the OID from the old branch
        let branch_oid = old_ref.oid.ok_or_else(|| anyhow::anyhow!("Branch has no commit"))?;

        // Create new branch reference
        let new_ref = Ref::new_direct(new_ref_name.clone(), branch_oid);
        refdb.write(&new_ref).await?;

        // Update HEAD if renaming current branch
        let head = refdb.read("HEAD").await?;
        if head.target.as_ref() == Some(&old_ref_name) {
            let new_head = Ref::new_symbolic("HEAD".to_string(), new_ref_name.clone());
            refdb.write(&new_head).await?;
        }

        // Delete old branch reference
        refdb.delete(&old_ref_name).await?;

        if !opts.quiet {
            output::success(&format!("Renamed branch '{}' to '{}'", old_branch, opts.new_name));
        }

        Ok(())
    }

    async fn show(&self, opts: &ShowOpts) -> Result<()> {
        use crate::output;

        let repo_root = self.find_repo_root()?;
        let storage_path = repo_root.join(".mediagit");
        let _storage: Arc<dyn mediagit_storage::StorageBackend> =
            Arc::new(LocalBackend::new(&storage_path).await?);
        let refdb = RefDatabase::new(&storage_path);

        // Determine which branch to show
        let branch_name = if let Some(name) = &opts.branch {
            name.clone()
        } else {
            // Get current branch from HEAD
            let head = refdb.read("HEAD").await?;
            match head.target {
                Some(target) => target
                    .strip_prefix("refs/heads/")
                    .unwrap_or(&target)
                    .to_string(),
                None => anyhow::bail!("HEAD is not pointing to a branch"),
            }
        };

        let branch_ref_name = format!("refs/heads/{}", branch_name);

        // Get branch reference
        let branch_ref = refdb
            .read(&branch_ref_name)
            .await
            .context(format!("Branch '{}' not found", branch_name))?;

        output::header(&format!("Branch: {}", branch_name));

        if let Some(oid) = branch_ref.oid {
            output::detail("Commit", &oid.to_string());
        } else {
            output::info("No commits yet");
        }

        Ok(())
    }

    async fn merge(&self, _opts: &MergeOpts) -> Result<()> {
        // NOTE: Branch merge implementation pending (delegates to mediagit merge command)
        // Requires: conflict check, merge execution, commit creation
        anyhow::bail!("Branch merge not yet implemented (use 'mediagit merge' instead)")
    }
}
