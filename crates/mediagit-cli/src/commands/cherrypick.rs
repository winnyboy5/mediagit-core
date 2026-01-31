use anyhow::{Context, Result};
use clap::Parser;
use console::style;
use mediagit_versioning::{CheckoutManager, Commit, Index, MergeEngine, ObjectDatabase, Oid, Ref, RefDatabase, Tree};
use std::path::PathBuf;
use std::sync::Arc;
use super::super::repo::find_repo_root;

/// Apply changes from existing commits
#[derive(Parser, Debug)]
pub struct CherryPickCmd {
    /// Commit hash(es) to cherry-pick
    #[arg(value_name = "COMMITS", required = true)]
    pub commits: Vec<String>,

    /// Continue cherry-pick after resolving conflicts
    #[arg(long)]
    pub continue_pick: bool,

    /// Abort cherry-pick operation
    #[arg(long)]
    pub abort: bool,

    /// Skip current commit and continue
    #[arg(long)]
    pub skip: bool,

    /// Don't automatically commit
    #[arg(short = 'n', long)]
    pub no_commit: bool,

    /// Edit commit message before committing
    #[arg(short, long)]
    pub edit: bool,

    /// Use original commit message
    #[arg(short = 'x', long)]
    pub append_message: bool,

    /// Quiet mode
    #[arg(short, long)]
    pub quiet: bool,
}

impl CherryPickCmd {
    pub async fn execute(&self) -> Result<()> {
        let repo_root = find_repo_root()?;

        // Handle special operations
        if self.abort {
            return self.abort_cherrypick(&repo_root).await;
        }

        if self.continue_pick {
            return self.continue_cherrypick(&repo_root).await;
        }

        if self.skip {
            return self.skip_cherrypick(&repo_root).await;
        }

        // Start new cherry-pick operation
        self.start_cherrypick(&repo_root).await
    }

    async fn start_cherrypick(&self, repo_root: &PathBuf) -> Result<()> {
        let mediagit_dir = repo_root.join(".mediagit");
        let storage = Arc::new(mediagit_storage::LocalBackend::new(&mediagit_dir).await?);
        let odb = ObjectDatabase::with_smart_compression(storage.clone(), 1000);
        let refdb = RefDatabase::new(&mediagit_dir);

        // Get current HEAD
        let current_oid = refdb.resolve("HEAD").await
            .context("Failed to resolve HEAD")?;

        if !self.quiet {
            println!(
                "{} Starting cherry-pick on branch at {}",
                style("→").cyan(),
                style(current_oid.to_hex()).yellow()
            );
        }

        // Process each commit to cherry-pick
        let mut picked_commits = Vec::new();
        for commit_ref in &self.commits {
            let commit_oid = self.resolve_commit(&refdb, commit_ref).await
                .context(format!("Failed to resolve commit: {}", commit_ref))?;

            if !self.quiet {
                println!(
                    "{} Applying commit {}",
                    style("→").cyan(),
                    style(commit_oid.to_hex()).yellow()
                );
            }

            match self.apply_commit(&odb, &refdb, repo_root, &commit_oid).await {
                Ok(()) => {
                    picked_commits.push(commit_oid);

                    if !self.no_commit {
                        // Create commit automatically
                        self.create_cherry_pick_commit(&odb, &refdb, repo_root, &commit_oid).await?;
                    }
                }
                Err(e) => {
                    // Save cherry-pick state for continuation
                    self.save_cherrypick_state(repo_root, &self.commits, &picked_commits).await?;

                    println!(
                        "{} Cherry-pick failed with conflicts",
                        style("✗").red()
                    );
                    println!("{} {}", style("Error:").red(), e);
                    println!();
                    println!("Resolve conflicts, then run:");
                    println!("  {} to continue", style("mediagit cherry-pick --continue").yellow());
                    println!("  {} to abort", style("mediagit cherry-pick --abort").yellow());
                    println!("  {} to skip this commit", style("mediagit cherry-pick --skip").yellow());

                    return Err(e);
                }
            }
        }

        if !self.quiet {
            println!(
                "{} Successfully cherry-picked {} commit(s)",
                style("✓").green(),
                picked_commits.len()
            );
        }

        Ok(())
    }

    async fn apply_commit(
        &self,
        odb: &ObjectDatabase,
        refdb: &RefDatabase,
        repo_root: &PathBuf,
        commit_oid: &Oid,
    ) -> Result<()> {
        // Load the commit
        let commit = Commit::read(odb, commit_oid).await
            .context("Failed to read commit")?;

        // Get parent commit
        if commit.parents.is_empty() {
            anyhow::bail!("Cannot cherry-pick initial commit");
        }

        let parent_oid = &commit.parents[0];
        let _parent_commit = Commit::read(odb, parent_oid).await
            .context("Failed to read parent commit")?;

        // Get current HEAD commit
        let current_oid = refdb.resolve("HEAD").await?;
        let _current_commit = Commit::read(odb, &current_oid).await?;

        // Create Arc for MergeEngine (it requires Arc<ObjectDatabase>)
        let mediagit_dir = repo_root.join(".mediagit");
        let storage = Arc::new(mediagit_storage::LocalBackend::new(&mediagit_dir).await?);
        let odb_arc = Arc::new(ObjectDatabase::with_smart_compression(storage.clone(), 1000));

        // Perform three-way merge: current HEAD vs commit being cherry-picked
        let merger = MergeEngine::new(odb_arc.clone());
        let merge_result = merger
            .merge(&current_oid, commit_oid, mediagit_versioning::MergeStrategy::Recursive)
            .await?;

        if !merge_result.conflicts.is_empty() {
            // Write conflict markers to files
            self.write_conflicts(repo_root, &merge_result)?;
            anyhow::bail!("Merge conflicts detected");
        }

        // Checkout the merged tree if merge was successful
        if let Some(tree_oid) = merge_result.tree_oid {
            let checkout_mgr = CheckoutManager::new(&odb_arc, repo_root);
            let commit_to_checkout = Commit {
                tree: tree_oid,
                parents: vec![current_oid],
                author: commit.author.clone(),
                committer: commit.committer.clone(),
                message: commit.message.clone(),
            };
            // Write temporary commit to get OID for checkout
            let temp_oid = commit_to_checkout.write(&odb_arc).await?;
            checkout_mgr.checkout_commit(&temp_oid).await?;
        }

        Ok(())
    }

    async fn create_cherry_pick_commit(
        &self,
        odb: &ObjectDatabase,
        refdb: &RefDatabase,
        repo_root: &PathBuf,
        original_oid: &Oid,
    ) -> Result<()> {
        // Load original commit for message
        let original_commit = Commit::read(odb, original_oid).await?;

        // Build commit message
        let mut message = original_commit.message.clone();
        if self.append_message {
            message.push_str(&format!("\n\n(cherry picked from commit {})", original_oid.to_hex()));
        }

        // Build tree from index
        let index = Index::load(repo_root)?;
        let mut tree = Tree::new();
        for entry in index.entries() {
            tree.add_entry(mediagit_versioning::TreeEntry::new(
                entry.path.to_string_lossy().to_string(),
                mediagit_versioning::FileMode::Regular,
                entry.oid,
            ));
        }
        let tree_oid = tree.write(odb).await?;

        // Get current HEAD as parent
        let current_oid = refdb.resolve("HEAD").await?;

        // Create new commit
        let new_commit = Commit {
            tree: tree_oid,
            parents: vec![current_oid],
            author: original_commit.author.clone(),
            committer: mediagit_versioning::Signature {
                name: original_commit.committer.name.clone(),
                email: original_commit.committer.email.clone(),
                timestamp: chrono::Utc::now(),
            },
            message,
        };

        let commit_oid = new_commit.write(odb).await?;

        // Update HEAD
        let head = refdb.read("HEAD").await?;
        if let Some(target) = head.target {
            let new_ref = Ref::new_direct(target, commit_oid);
            refdb.write(&new_ref).await?;
        }

        if !self.quiet {
            println!(
                "{} Created commit {}",
                style("✓").green(),
                style(commit_oid.to_hex()).yellow()
            );
        }

        Ok(())
    }

    async fn continue_cherrypick(&self, repo_root: &PathBuf) -> Result<()> {
        let state_path = repo_root.join(".mediagit/CHERRY_PICK_STATE");
        if !state_path.exists() {
            anyhow::bail!("No cherry-pick in progress");
        }

        // Load state
        let state_json = std::fs::read_to_string(&state_path)?;
        let state: CherryPickState = serde_json::from_str(&state_json)?;

        // Note: Conflict checking would need additional state tracking
        // For now, assume user has resolved conflicts if they're continuing

        // Create commit for current pick
        let mediagit_dir = repo_root.join(".mediagit");
        let storage = Arc::new(mediagit_storage::LocalBackend::new(&mediagit_dir).await?);
        let odb = ObjectDatabase::with_smart_compression(storage.clone(), 1000);
        let refdb = RefDatabase::new(&mediagit_dir);

        if let Some(current) = &state.current_commit {
            let current_oid = Oid::from_hex(current)?;
            self.create_cherry_pick_commit(&odb, &refdb, repo_root, &current_oid).await?;
        }

        // Continue with remaining commits
        if state.remaining_commits.is_empty() {
            // Clean up state
            std::fs::remove_file(&state_path)?;

            if !self.quiet {
                println!("{} Cherry-pick complete", style("✓").green());
            }
            return Ok(());
        }

        // Process remaining commits
        let remaining: Vec<String> = state.remaining_commits.clone();
        drop(state);  // Release state before recursive call

        let cmd = Self {
            commits: remaining,
            continue_pick: false,
            abort: false,
            skip: false,
            no_commit: self.no_commit,
            edit: self.edit,
            append_message: self.append_message,
            quiet: self.quiet,
        };

        cmd.start_cherrypick(repo_root).await
    }

    async fn skip_cherrypick(&self, repo_root: &PathBuf) -> Result<()> {
        let state_path = repo_root.join(".mediagit/CHERRY_PICK_STATE");
        if !state_path.exists() {
            anyhow::bail!("No cherry-pick in progress");
        }

        // Load state
        let state_json = std::fs::read_to_string(&state_path)?;
        let state: CherryPickState = serde_json::from_str(&state_json)?;

        if !self.quiet {
            println!(
                "{} Skipping commit {}",
                style("→").cyan(),
                state.current_commit.as_deref().unwrap_or("unknown")
            );
        }

        // Continue with remaining commits (skip current)
        if state.remaining_commits.is_empty() {
            std::fs::remove_file(&state_path)?;

            if !self.quiet {
                println!("{} Cherry-pick complete", style("✓").green());
            }
            return Ok(());
        }

        let remaining: Vec<String> = state.remaining_commits.clone();
        drop(state);

        let cmd = Self {
            commits: remaining,
            continue_pick: false,
            abort: false,
            skip: false,
            no_commit: self.no_commit,
            edit: self.edit,
            append_message: self.append_message,
            quiet: self.quiet,
        };

        cmd.start_cherrypick(repo_root).await
    }

    async fn abort_cherrypick(&self, repo_root: &PathBuf) -> Result<()> {
        let state_path = repo_root.join(".mediagit/CHERRY_PICK_STATE");
        if !state_path.exists() {
            anyhow::bail!("No cherry-pick in progress");
        }

        // Load state to get original HEAD
        let state_json = std::fs::read_to_string(&state_path)?;
        let state: CherryPickState = serde_json::from_str(&state_json)?;

        if let Some(original_head) = &state.original_head {
            // Reset to original HEAD
            let mediagit_dir = repo_root.join(".mediagit");
            let storage = Arc::new(mediagit_storage::LocalBackend::new(&mediagit_dir).await?);
            let odb = ObjectDatabase::with_smart_compression(storage.clone(), 1000);
            let refdb = RefDatabase::new(&mediagit_dir);

            let original_oid = Oid::from_hex(original_head)?;

            // Update HEAD reference
            let head = refdb.read("HEAD").await?;
            if let Some(target) = head.target {
                let reset_ref = Ref::new_direct(target, original_oid);
                refdb.write(&reset_ref).await?;
            }

            // Restore working directory
            let checkout_mgr = mediagit_versioning::CheckoutManager::new(&odb, repo_root);
            checkout_mgr.checkout_commit(&original_oid).await?;
        }

        // Clean up state
        std::fs::remove_file(&state_path)?;

        if !self.quiet {
            println!("{} Cherry-pick aborted", style("✓").green());
        }

        Ok(())
    }

    async fn save_cherrypick_state(
        &self,
        repo_root: &PathBuf,
        all_commits: &[String],
        picked_commits: &[Oid],
    ) -> Result<()> {
        let mediagit_dir = repo_root.join(".mediagit");
        let refdb = RefDatabase::new(&mediagit_dir);

        let original_head = refdb.resolve("HEAD").await?.to_hex();

        let current_commit = all_commits.get(picked_commits.len()).map(|s| s.clone());
        let remaining_commits = all_commits[picked_commits.len() + 1..].to_vec();

        let state = CherryPickState {
            original_head: Some(original_head),
            current_commit,
            remaining_commits,
        };

        let state_json = serde_json::to_string_pretty(&state)?;
        std::fs::write(mediagit_dir.join("CHERRY_PICK_STATE"), state_json)?;

        Ok(())
    }

    fn write_conflicts(&self, repo_root: &PathBuf, merge_result: &mediagit_versioning::MergeResult) -> Result<()> {
        // Write conflict markers to files
        for conflict in &merge_result.conflicts {
            let file_path = repo_root.join(&conflict.path);

            // Build conflict marker content
            let ours_content = conflict.ours.as_ref()
                .map(|s| format!("{:?}", s.oid))
                .unwrap_or_else(|| "(deleted)".to_string());
            let theirs_content = conflict.theirs.as_ref()
                .map(|s| format!("{:?}", s.oid))
                .unwrap_or_else(|| "(deleted)".to_string());

            let conflict_content = format!(
                "<<<<<<< HEAD\n{}\n=======\n{}\n>>>>>>> cherry-pick\n",
                ours_content,
                theirs_content
            );
            std::fs::write(&file_path, conflict_content)?;
        }
        Ok(())
    }

    async fn resolve_commit(&self, refdb: &RefDatabase, commit_ref: &str) -> Result<Oid> {
        // Try direct OID first
        if let Ok(oid) = Oid::from_hex(commit_ref) {
            return Ok(oid);
        }

        // Try as branch reference
        let branch_ref = format!("refs/heads/{}", commit_ref);
        if refdb.exists(&branch_ref).await? {
            return refdb.resolve(&branch_ref).await;
        }

        // Try as tag reference
        let tag_ref = format!("refs/tags/{}", commit_ref);
        if refdb.exists(&tag_ref).await? {
            return refdb.resolve(&tag_ref).await;
        }

        // Try resolving directly
        refdb.resolve(commit_ref).await
            .context(format!("Cannot resolve commit reference: {}", commit_ref))
    }

}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
struct CherryPickState {
    original_head: Option<String>,
    current_commit: Option<String>,
    remaining_commits: Vec<String>,
}
