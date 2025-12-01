use anyhow::{Context, Result};
use clap::Parser;
use console::style;
use mediagit_storage::LocalBackend;
use mediagit_versioning::{Commit, LcaFinder, ObjectDatabase, ObjectType, Oid, Ref, RefDatabase, Signature};
use std::collections::HashSet;
use std::sync::Arc;

/// Rebase commits
#[derive(Parser, Debug)]
pub struct RebaseCmd {
    /// Upstream branch to rebase onto
    #[arg(value_name = "UPSTREAM")]
    pub upstream: String,

    /// Branch to rebase (defaults to current)
    #[arg(value_name = "BRANCH")]
    pub branch: Option<String>,

    /// Interactive rebase
    #[arg(short, long)]
    pub interactive: bool,

    /// Rebase merge commits
    #[arg(short = 'm', long)]
    pub rebase_merges: bool,

    /// Keep empty commits
    #[arg(long)]
    pub keep_empty: bool,

    /// Autosquash (automatically apply fixup/squash)
    #[arg(long)]
    pub autosquash: bool,

    /// Abort rebase
    #[arg(long)]
    pub abort: bool,

    /// Continue after resolving conflicts
    #[arg(long)]
    pub continue_rebase: bool,

    /// Skip current commit
    #[arg(long)]
    pub skip: bool,

    /// Quiet mode
    #[arg(short, long)]
    pub quiet: bool,

    /// Verbose mode
    #[arg(short, long)]
    pub verbose: bool,
}

impl RebaseCmd {
    pub async fn execute(&self) -> Result<()> {
        // Handle special operations
        if self.abort {
            return self.abort_rebase().await;
        }
        if self.continue_rebase {
            return self.continue_rebase_process().await;
        }
        if self.skip {
            return self.skip_commit().await;
        }

        // Interactive and merge rebases not yet supported
        if self.interactive {
            anyhow::bail!("Interactive rebase not yet implemented. Use non-interactive rebase.");
        }
        if self.rebase_merges {
            anyhow::bail!("Rebase with merge commits not yet implemented.");
        }

        // Find repository root
        let repo_root = self.find_repo_root()?;
        let storage_path = repo_root.join(".mediagit/objects");
        let storage: Arc<dyn mediagit_storage::StorageBackend> =
            Arc::new(LocalBackend::new(&storage_path).await?);
        let refdb = RefDatabase::new(storage.clone());
        let odb = Arc::new(ObjectDatabase::new(storage, 1000));

        // Resolve upstream branch
        let upstream_oid = self.resolve_branch(&refdb, &self.upstream).await?;

        // Get current HEAD
        let head = refdb.read("HEAD").await?;
        let head_target = head.target.clone();
        let current_oid = match head.oid {
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

        if !self.quiet {
            println!(
                "{} Rebasing onto {}...",
                style("🔄").cyan().bold(),
                style(&self.upstream).yellow()
            );
        }

        // Find merge base (common ancestor)
        let lca_finder = LcaFinder::new(odb.clone());
        let merge_bases = lca_finder.find_merge_base(&current_oid, &upstream_oid).await?;

        if merge_bases.is_empty() {
            anyhow::bail!("No common ancestor found");
        }

        let base_oid = merge_bases[0];

        if self.verbose {
            println!("  Merge base: {}", &base_oid.to_string()[..7]);
        }

        // Check if already up to date
        if lca_finder.is_ancestor(&upstream_oid, &current_oid).await? {
            if !self.quiet {
                println!("{} Already up to date", style("✓").green());
            }
            return Ok(());
        }

        // Collect commits to rebase (from base to current)
        let commits_to_rebase = self.collect_commits(&odb, &base_oid, &current_oid).await?;

        if commits_to_rebase.is_empty() {
            if !self.quiet {
                println!("{} No commits to rebase", style("ℹ").blue());
            }
            return Ok(());
        }

        if !self.quiet {
            println!(
                "  Rebasing {} commit(s)...",
                commits_to_rebase.len()
            );
        }

        // Rebase commits one by one
        let mut new_parent = upstream_oid;

        for (i, original_commit) in commits_to_rebase.iter().enumerate() {
            if self.verbose {
                println!(
                    "  [{}/{}] {}",
                    i + 1,
                    commits_to_rebase.len(),
                    original_commit.message.lines().next().unwrap_or("")
                );
            }

            // Create new commit with same changes but new parent
            let new_commit = Commit {
                tree: original_commit.tree,
                parents: vec![new_parent],
                author: original_commit.author.clone(),
                committer: Signature::now(
                    original_commit.committer.name.clone(),
                    original_commit.committer.email.clone(),
                ),
                message: original_commit.message.clone(),
            };

            let commit_data = new_commit.serialize()?;
            let commit_oid = odb.write(ObjectType::Commit, &commit_data).await?;

            new_parent = commit_oid;
        }

        // Update HEAD to point to new commit chain
        if let Some(ref target) = head_target {
            let new_ref = Ref::new_direct(target.clone(), new_parent);
            refdb.write(&new_ref).await?;
        } else {
            let new_ref = Ref::new_direct("HEAD".to_string(), new_parent);
            refdb.write(&new_ref).await?;
        }

        if !self.quiet {
            println!(
                "{} Successfully rebased {} commit(s)",
                style("✓").green().bold(),
                commits_to_rebase.len()
            );
        }

        Ok(())
    }

    async fn collect_commits(
        &self,
        odb: &Arc<ObjectDatabase>,
        base_oid: &Oid,
        head_oid: &Oid,
    ) -> Result<Vec<Commit>> {
        let mut commits = Vec::new();
        let mut visited = HashSet::new();
        let mut current = *head_oid;

        // Walk back from head to base
        loop {
            if visited.contains(&current) || current == *base_oid {
                break;
            }
            visited.insert(current);

            let data = odb.read(&current).await?;
            let commit = Commit::deserialize(&data)?;

            commits.push(commit.clone());

            // Follow first parent
            if let Some(parent) = commit.parents.first() {
                current = *parent;
            } else {
                break;
            }
        }

        // Reverse to get chronological order
        commits.reverse();
        Ok(commits)
    }

    async fn resolve_branch(&self, refdb: &RefDatabase, branch: &str) -> Result<Oid> {
        // Try as direct OID
        if let Ok(oid) = Oid::from_hex(branch) {
            return Ok(oid);
        }

        // Try as reference
        let ref_result = refdb.read(branch).await;
        match ref_result {
            Ok(r) => r.oid.context(format!("Branch {} has no commit", branch)),
            Err(_) => {
                // Try with refs/heads prefix
                let with_prefix = format!("refs/heads/{}", branch);
                let ref_result = refdb.read(&with_prefix).await;
                match ref_result {
                    Ok(r) => r.oid.context(format!("Branch {} has no commit", branch)),
                    Err(_) => anyhow::bail!("Cannot resolve branch: {}", branch),
                }
            }
        }
    }

    async fn abort_rebase(&self) -> Result<()> {
        if !self.quiet {
            println!("{} Aborting rebase...", style("✗").red());
        }
        anyhow::bail!("Rebase abort not yet implemented")
    }

    async fn continue_rebase_process(&self) -> Result<()> {
        if !self.quiet {
            println!("{} Continuing rebase...", style("→").cyan());
        }
        anyhow::bail!("Rebase continue not yet implemented")
    }

    async fn skip_commit(&self) -> Result<()> {
        if !self.quiet {
            println!("{} Skipping commit...", style("→").cyan());
        }
        anyhow::bail!("Rebase skip not yet implemented")
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
