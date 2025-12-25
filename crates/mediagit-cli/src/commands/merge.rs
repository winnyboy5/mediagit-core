use anyhow::{Context, Result};
use clap::Parser;
use console::style;
use mediagit_storage::LocalBackend;
use mediagit_versioning::{CheckoutManager, Commit, MergeEngine, MergeStrategy, ObjectDatabase, ObjectType, Oid, Ref, RefDatabase, Signature};
use std::sync::Arc;

/// Merge branches
///
/// Join two or more development histories together. By default, creates a
/// merge commit combining the changes from both branches.
#[derive(Parser, Debug)]
#[command(after_help = "EXAMPLES:
    # Merge feature branch into current branch
    mediagit merge feature-branch

    # Merge with custom message
    mediagit merge feature-branch -m \"Merge feature X\"

    # Force merge commit (no fast-forward)
    mediagit merge --no-ff feature-branch

    # Fast-forward only merge
    mediagit merge --ff-only feature-branch

    # Merge with specific strategy
    mediagit merge -s recursive feature-branch

    # Abort merge after conflicts
    mediagit merge --abort

    # Continue merge after resolving conflicts
    mediagit merge --continue

SEE ALSO:
    mediagit-branch(1), mediagit-rebase(1), mediagit-cherry-pick(1)")]
pub struct MergeCmd {
    /// Branch to merge
    #[arg(value_name = "BRANCH", required = true)]
    pub branch: String,

    /// Merge message
    #[arg(short, long, value_name = "MESSAGE")]
    pub message: Option<String>,

    /// Create a merge commit even if fast-forward is possible
    #[arg(long)]
    pub no_ff: bool,

    /// Perform fast-forward only merge
    #[arg(long)]
    pub ff_only: bool,

    /// Squash commits before merging
    #[arg(long)]
    pub squash: bool,

    /// Merge strategy
    #[arg(short = 's', long, value_name = "STRATEGY")]
    pub strategy: Option<String>,

    /// Merge strategy option
    #[arg(short = 'X', long, value_name = "OPTION")]
    pub strategy_option: Option<String>,

    /// Don't commit merge result
    #[arg(long)]
    pub no_commit: bool,

    /// Abort merge
    #[arg(long)]
    pub abort: bool,

    /// Continue after resolving conflicts
    #[arg(long)]
    pub continue_merge: bool,

    /// Quiet mode
    #[arg(short, long)]
    pub quiet: bool,

    /// Verbose mode
    #[arg(short, long)]
    pub verbose: bool,
}

impl MergeCmd {
    pub async fn execute(&self) -> Result<()> {
        // Handle abort/continue first
        if self.abort {
            return self.abort_merge().await;
        }
        if self.continue_merge {
            return self.continue_merge_process().await;
        }

        // Find repository root
        let repo_root = self.find_repo_root()?;
        let storage_path = repo_root.join(".mediagit");
        let storage: Arc<dyn mediagit_storage::StorageBackend> =
            Arc::new(LocalBackend::new(&storage_path).await?);
        let refdb = RefDatabase::new(&storage_path);
        let odb = Arc::new(ObjectDatabase::with_smart_compression(storage, 1000));

        // Resolve branch to OID
        let their_oid = self.resolve_branch(&refdb).await?;

        // Get current HEAD commit
        let head = refdb.read("HEAD").await?;
        let head_target = head.target.clone();
        let our_oid = match head.oid {
            Some(oid) => oid,
            None => {
                if let Some(ref target) = head_target {
                    let target_ref = refdb.read(target).await?;
                    target_ref.oid.context("HEAD has no commit yet")?
                } else {
                    anyhow::bail!("HEAD has no commit yet");
                }
            }
        };

        // Parse merge strategy
        let strategy = match self.strategy.as_deref() {
            Some("ours") => MergeStrategy::Ours,
            Some("theirs") => MergeStrategy::Theirs,
            Some("recursive") | None => MergeStrategy::Recursive,
            Some(s) => anyhow::bail!("Unknown merge strategy: {}", s),
        };

        if !self.quiet {
            println!(
                "{} Merging {} into {}...",
                style("🔀").cyan().bold(),
                style(&self.branch).yellow(),
                style("HEAD").cyan()
            );
            println!("{} Analyzing commit history...", style("🔍").cyan());
        }

        // Create merge engine and perform merge
        let engine = MergeEngine::new(odb.clone());

        if !self.quiet {
            println!("{} Computing merge...", style("⚙️ ").cyan());
        }

        let result = engine.merge(&our_oid, &their_oid, strategy).await?;

        // Handle merge result
        if let Some(ff_info) = &result.fast_forward {
            if ff_info.is_fast_forward {
                if self.ff_only || !self.no_ff {
                    // Fast-forward merge
                    if !self.quiet {
                        println!(
                            "{} Fast-forwarding {} -> {}",
                            style("✓").green(),
                            &ff_info.from.to_string()[..7],
                            &ff_info.to.to_string()[..7]
                        );
                    }

                    // Update HEAD to point to their commit
                    if let Some(ref target) = head_target {
                        let new_ref = Ref::new_direct(target.clone(), their_oid);
                        refdb.write(&new_ref).await?;
                    } else {
                        let new_ref = Ref::new_direct("HEAD".to_string(), their_oid);
                        refdb.write(&new_ref).await?;
                    }

                    // Update working directory to match the merged commit (ISS-008 fix)
                    let checkout_mgr = CheckoutManager::new(&odb, &repo_root);
                    checkout_mgr.checkout_commit(&their_oid).await
                        .context("Failed to update working directory after fast-forward merge")?;

                    return Ok(());
                } else if self.ff_only {
                    anyhow::bail!("Fast-forward only requested but not possible");
                }
            }
        }

        // Check for conflicts
        if !result.conflicts.is_empty() {
            println!(
                "{} Merge conflicts detected in {} file(s):",
                style("⚠").yellow().bold(),
                result.conflicts.len()
            );
            for conflict in &result.conflicts {
                println!("  {} {}", style("conflict:").red(), conflict.path);
                if self.verbose {
                    println!("    Type: {:?}", conflict.conflict_type);
                }
            }
            anyhow::bail!(
                "Automatic merge failed. Fix conflicts and run 'mediagit merge --continue'"
            );
        }

        // No conflicts - create merge commit
        if !self.no_commit {
            let tree_oid = result.tree_oid.context("No merged tree created")?;

            // Create commit signature
            let author_name = std::env::var("GIT_AUTHOR_NAME")
                .or_else(|_| std::env::var("USER"))
                .unwrap_or_else(|_| "Unknown".to_string());
            let author_email = std::env::var("GIT_AUTHOR_EMAIL")
                .unwrap_or_else(|_| "unknown@localhost".to_string());

            let signature = Signature::now(author_name, author_email);

            let message = self.message.clone().unwrap_or_else(|| {
                format!("Merge branch '{}' into HEAD", self.branch)
            });

            let merge_commit = Commit {
                tree: tree_oid,
                parents: vec![our_oid, their_oid],
                author: signature.clone(),
                committer: signature,
                message,
            };

            let commit_data = merge_commit.serialize()?;
            let commit_oid = odb.write(ObjectType::Commit, &commit_data).await?;

            // Update HEAD
            if let Some(ref target) = head_target {
                let new_ref = Ref::new_direct(target.clone(), commit_oid);
                refdb.write(&new_ref).await?;
            } else {
                let new_ref = Ref::new_direct("HEAD".to_string(), commit_oid);
                refdb.write(&new_ref).await?;
            }

            // Update working directory to match the merged commit (ISS-008 fix)
            let checkout_mgr = CheckoutManager::new(&odb, &repo_root);
            checkout_mgr.checkout_commit(&commit_oid).await
                .context("Failed to update working directory after merge commit")?;

            if !self.quiet {
                println!(
                    "{} Merge committed: {}",
                    style("✓").green().bold(),
                    &commit_oid.to_string()[..7]
                );
            }
        } else if !self.quiet {
            println!("{} Merge successful (not committed)", style("✓").green());
        }

        Ok(())
    }

    async fn resolve_branch(&self, refdb: &RefDatabase) -> Result<Oid> {
        // Try as direct OID
        if let Ok(oid) = Oid::from_hex(&self.branch) {
            return Ok(oid);
        }

        // Try as reference
        let ref_result = refdb.read(&self.branch).await;
        match ref_result {
            Ok(r) => r
                .oid
                .context(format!("Branch {} has no commit", self.branch)),
            Err(_) => {
                // Try with refs/heads prefix
                let with_prefix = format!("refs/heads/{}", self.branch);
                let ref_result = refdb.read(&with_prefix).await;
                match ref_result {
                    Ok(r) => r
                        .oid
                        .context(format!("Branch {} has no commit", self.branch)),
                    Err(_) => anyhow::bail!("Cannot resolve branch: {}", self.branch),
                }
            }
        }
    }

    async fn abort_merge(&self) -> Result<()> {
        if !self.quiet {
            println!("{} Aborting merge...", style("✗").red());
        }

        let repo_root = self.find_repo_root()?;
        let mediagit_dir = repo_root.join(".mediagit");

        // Clean up merge state files
        let merge_head = mediagit_dir.join("MERGE_HEAD");
        let merge_msg = mediagit_dir.join("MERGE_MSG");
        let merge_mode = mediagit_dir.join("MERGE_MODE");

        let mut cleaned = 0;

        if merge_head.exists() {
            std::fs::remove_file(&merge_head)
                .context("Failed to remove MERGE_HEAD")?;
            cleaned += 1;
        }

        if merge_msg.exists() {
            std::fs::remove_file(&merge_msg)
                .context("Failed to remove MERGE_MSG")?;
            cleaned += 1;
        }

        if merge_mode.exists() {
            std::fs::remove_file(&merge_mode)
                .context("Failed to remove MERGE_MODE")?;
            cleaned += 1;
        }

        if !self.quiet {
            println!(
                "{} Merge aborted. Cleaned up {} state file(s).",
                style("✓").green(),
                cleaned
            );
        }

        Ok(())
    }

    async fn continue_merge_process(&self) -> Result<()> {
        if !self.quiet {
            println!("{} Continuing merge...", style("→").cyan());
        }

        let repo_root = self.find_repo_root()?;
        let mediagit_dir = repo_root.join(".mediagit");

        // Check if merge is in progress
        let merge_head_path = mediagit_dir.join("MERGE_HEAD");
        if !merge_head_path.exists() {
            anyhow::bail!("No merge in progress");
        }

        // Read MERGE_HEAD to get the OID being merged
        let merge_head_content = std::fs::read_to_string(&merge_head_path)
            .context("Failed to read MERGE_HEAD")?;
        let merge_oid = mediagit_versioning::Oid::from_hex(merge_head_content.trim())?;

        // Read MERGE_MSG if it exists
        let merge_msg_path = mediagit_dir.join("MERGE_MSG");
        let message = if merge_msg_path.exists() {
            std::fs::read_to_string(&merge_msg_path)
                .context("Failed to read MERGE_MSG")?
        } else {
            format!("Merge commit {}", merge_oid.to_hex())
        };

        // Create commit with resolved changes
        if !self.quiet {
            println!("{} Creating merge commit...", style("→").cyan());
        }

        // Load index and create tree
        let index = mediagit_versioning::Index::load(&repo_root)?;
        if index.is_empty() {
            anyhow::bail!("No changes staged. Use 'add' to stage resolved files.");
        }

        let storage = Arc::new(mediagit_storage::LocalBackend::new(&mediagit_dir).await?);
        let odb = mediagit_versioning::ObjectDatabase::with_smart_compression(storage, 1000);

        let mut tree = mediagit_versioning::Tree::new();
        for entry in index.entries() {
            tree.add_entry(mediagit_versioning::TreeEntry::new(
                entry.path.to_string_lossy().to_string(),
                mediagit_versioning::FileMode::Regular,
                entry.oid,
            ));
        }
        let tree_oid = tree.write(&odb).await?;

        // Get current HEAD
        let refdb = mediagit_versioning::RefDatabase::new(&mediagit_dir);
        let current_oid = refdb.resolve("HEAD").await?;

        // Create merge commit with two parents
        let signature = mediagit_versioning::Signature {
            name: "MediaGit User".to_string(),
            email: "user@mediagit.local".to_string(),
            timestamp: chrono::Utc::now(),
        };

        let commit = mediagit_versioning::Commit {
            tree: tree_oid,
            parents: vec![current_oid, merge_oid],
            author: signature.clone(),
            committer: signature,
            message,
        };

        let commit_oid = commit.write(&odb).await?;

        // Update HEAD (force=false, safe update)
        refdb.update("HEAD", commit_oid, false).await?;

        // Clean up merge state after successful commit
        if merge_head_path.exists() {
            std::fs::remove_file(&merge_head_path).ok();
        }
        if merge_msg_path.exists() {
            std::fs::remove_file(&merge_msg_path).ok();
        }
        let merge_mode = mediagit_dir.join("MERGE_MODE");
        if merge_mode.exists() {
            std::fs::remove_file(&merge_mode).ok();
        }

        if !self.quiet {
            println!("{} Merge continued successfully", style("✓").green());
            println!("  Created merge commit: {}", commit_oid.to_hex());
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
