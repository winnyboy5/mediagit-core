use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use console::style;
use mediagit_versioning::{CheckoutManager, Commit, Index, ObjectDatabase, Oid, RefDatabase};
use std::path::PathBuf;
use super::super::repo::{find_repo_root, create_storage_backend};

/// Stash changes in working directory
#[derive(Parser, Debug)]
pub struct StashCmd {
    #[command(subcommand)]
    pub command: StashSubcommand,
}

#[derive(Subcommand, Debug)]
pub enum StashSubcommand {
    /// Save changes to stash
    Save(SaveOpts),

    /// Apply stashed changes
    Apply(ApplyOpts),

    /// List stashed changes
    List(ListOpts),

    /// Show stash contents
    Show(ShowOpts),

    /// Remove a stash entry
    Drop(DropOpts),

    /// Apply and remove stash entry
    Pop(PopOpts),

    /// Clear all stashes
    Clear(ClearOpts),
}

#[derive(Parser, Debug)]
pub struct SaveOpts {
    /// Stash message (flag form, e.g. -m "WIP")
    #[arg(short = 'm', long = "message", value_name = "MESSAGE")]
    pub message_flag: Option<String>,

    /// Stash message (positional, for git-compatible `stash save "msg"`)
    #[arg(value_name = "MESSAGE")]
    pub message_positional: Option<String>,

    /// Include untracked files
    #[arg(short = 'u', long)]
    pub include_untracked: bool,

    /// Stash only specific paths
    #[arg(value_name = "PATHS")]
    pub paths: Vec<PathBuf>,

    /// Quiet mode
    #[arg(short, long)]
    pub quiet: bool,
}

#[derive(Parser, Debug)]
pub struct ApplyOpts {
    /// Stash index (default: 0)
    #[arg(value_name = "STASH")]
    pub stash: Option<usize>,

    /// Reinstate index changes
    #[arg(long)]
    pub index: bool,

    /// Quiet mode
    #[arg(short, long)]
    pub quiet: bool,
}

#[derive(Parser, Debug)]
pub struct ListOpts {
    /// Show verbose output
    #[arg(short, long)]
    pub verbose: bool,
}

#[derive(Parser, Debug)]
pub struct ShowOpts {
    /// Stash index (default: 0)
    #[arg(value_name = "STASH")]
    pub stash: Option<usize>,

    /// Show patch
    #[arg(short = 'p', long)]
    pub patch: bool,
}

#[derive(Parser, Debug)]
pub struct DropOpts {
    /// Stash index (default: 0)
    #[arg(value_name = "STASH")]
    pub stash: Option<usize>,

    /// Quiet mode
    #[arg(short, long)]
    pub quiet: bool,
}

#[derive(Parser, Debug)]
pub struct PopOpts {
    /// Stash index (default: 0)
    #[arg(value_name = "STASH")]
    pub stash: Option<usize>,

    /// Reinstate index changes
    #[arg(long)]
    pub index: bool,

    /// Quiet mode
    #[arg(short, long)]
    pub quiet: bool,
}

#[derive(Parser, Debug)]
pub struct ClearOpts {
    /// Force clear without confirmation
    #[arg(short, long)]
    pub force: bool,
}

impl StashCmd {
    pub async fn execute(&self) -> Result<()> {
        match &self.command {
            StashSubcommand::Save(opts) => self.save(opts).await,
            StashSubcommand::Apply(opts) => self.apply(opts).await,
            StashSubcommand::List(opts) => self.list(opts).await,
            StashSubcommand::Show(opts) => self.show(opts).await,
            StashSubcommand::Drop(opts) => self.drop(opts).await,
            StashSubcommand::Pop(opts) => self.pop(opts).await,
            StashSubcommand::Clear(opts) => self.clear(opts).await,
        }
    }

    async fn save(&self, opts: &SaveOpts) -> Result<()> {
        let repo_root = find_repo_root()?;
        let mediagit_dir = repo_root.join(".mediagit");
        let storage = create_storage_backend(&repo_root).await?;
        let odb = ObjectDatabase::with_smart_compression(storage.clone(), 1000);
        let refdb = RefDatabase::new(&mediagit_dir);

        // Check if there are changes to stash
        let index = Index::load(&repo_root)?;
        if index.is_empty() {
            if !opts.quiet {
                println!("{} No changes to stash", style("ℹ").blue());
            }
            return Ok(());
        }

        // Get current HEAD
        let current_oid = refdb.resolve("HEAD").await
            .context("Failed to resolve HEAD")?;

        // Build tree from index
        let mut tree = mediagit_versioning::Tree::new();
        for entry in index.entries() {
            tree.add_entry(mediagit_versioning::TreeEntry::new(
                entry.path.to_string_lossy().to_string(),
                mediagit_versioning::FileMode::Regular,
                entry.oid,
            ));
        }
        let tree_oid = tree.write(&odb).await?;

        // Create stash commit (-m flag takes priority over positional)
        let message = opts.message_flag.as_deref()
            .or(opts.message_positional.as_deref())
            .unwrap_or("WIP on branch")
            .to_string();

        let stash_signature = mediagit_versioning::Signature {
            name: "Stash".to_string(),
            email: "stash@mediagit.local".to_string(),
            timestamp: chrono::Utc::now(),
        };

        let commit = Commit {
            tree: tree_oid,
            parents: vec![current_oid],
            author: stash_signature.clone(),
            committer: stash_signature,
            message: message.clone(),
        };

        let commit_oid = commit.write(&odb).await?;

        // Save stash entry
        let stash_entry = StashEntry {
            commit_oid: commit_oid.to_hex(),
            message: message.clone(),
            timestamp: chrono::Utc::now().to_rfc3339(),
            branch: self.get_current_branch(&refdb).await?,
        };

        self.save_stash_entry(&mediagit_dir, stash_entry)?;

        // Clean working directory
        let mut index = Index::load(&repo_root)?;
        index.clear();
        index.save(&repo_root)?;

        // Restore to HEAD state
        let checkout_mgr = CheckoutManager::new(&odb, &repo_root);
        checkout_mgr.checkout_commit(&current_oid).await?;

        if !opts.quiet {
            println!(
                "{} Saved working directory and index state",
                style("✓").green()
            );
            println!("  WIP on: {}", message);
        }

        Ok(())
    }

    async fn apply(&self, opts: &ApplyOpts) -> Result<()> {
        let repo_root = find_repo_root()?;
        let mediagit_dir = repo_root.join(".mediagit");
        let storage = create_storage_backend(&repo_root).await?;
        let odb = ObjectDatabase::with_smart_compression(storage.clone(), 1000);

        // Load stash entry
        let stash_index = opts.stash.unwrap_or(0);
        let stash_entry = self.load_stash_entry(&mediagit_dir, stash_index)?;

        let stash_oid = Oid::from_hex(&stash_entry.commit_oid)?;

        // Apply stash tree on top of current working directory (overlay, not replace).
        // checkout_commit would wipe files not in the stash tree.
        let checkout_mgr = CheckoutManager::new(&odb, &repo_root);
        let files_updated = checkout_mgr.apply_tree_overlay(&stash_oid).await?;

        if !opts.quiet {
            println!(
                "{} Applied stash entry {}",
                style("✓").green(),
                stash_index
            );
            if files_updated > 0 {
                println!("  Updated {} file(s)", files_updated);
            }
        }

        Ok(())
    }

    async fn list(&self, opts: &ListOpts) -> Result<()> {
        let repo_root = find_repo_root()?;
        let mediagit_dir = repo_root.join(".mediagit");

        let stash_list = self.load_all_stashes(&mediagit_dir)?;

        if stash_list.is_empty() {
            println!("No stashes found");
            return Ok(());
        }

        for (index, entry) in stash_list.iter().enumerate() {
            if opts.verbose {
                println!(
                    "{}: {} on {}",
                    style(format!("stash@{{{}}}", index)).yellow(),
                    style(&entry.message).green(),
                    style(&entry.branch.as_deref().unwrap_or("unknown")).cyan()
                );
                println!("  Commit: {}", entry.commit_oid);
                println!("  Date: {}", entry.timestamp);
                println!();
            } else {
                println!(
                    "{}: {}",
                    style(format!("stash@{{{}}}", index)).yellow(),
                    entry.message
                );
            }
        }

        Ok(())
    }

    async fn show(&self, opts: &ShowOpts) -> Result<()> {
        let repo_root = find_repo_root()?;
        let mediagit_dir = repo_root.join(".mediagit");

        let stash_index = opts.stash.unwrap_or(0);
        let stash_entry = self.load_stash_entry(&mediagit_dir, stash_index)?;

        println!("{}", style(format!("stash@{{{}}}", stash_index)).yellow().bold());
        println!("Message:  {}", stash_entry.message);
        println!("Branch:   {}", stash_entry.branch.as_deref().unwrap_or("unknown"));
        println!("Commit:   {}", stash_entry.commit_oid);
        println!("Date:     {}", stash_entry.timestamp);

        if opts.patch {
            // Future enhancement: integrate diff display here
            println!("\n{} Patch display feature coming soon", style("ℹ").blue());
        }

        Ok(())
    }

    async fn drop(&self, opts: &DropOpts) -> Result<()> {
        let repo_root = find_repo_root()?;
        let mediagit_dir = repo_root.join(".mediagit");

        let stash_index = opts.stash.unwrap_or(0);

        // Load all stashes
        let mut stash_list = self.load_all_stashes(&mediagit_dir)?;

        if stash_index >= stash_list.len() {
            anyhow::bail!("Stash entry {} not found", stash_index);
        }

        // Remove stash entry
        let removed = stash_list.remove(stash_index);

        // Save updated list
        self.save_all_stashes(&mediagit_dir, &stash_list)?;

        if !opts.quiet {
            println!(
                "{} Dropped stash@{{{}}} ({})",
                style("✓").green(),
                stash_index,
                removed.message
            );
        }

        Ok(())
    }

    async fn pop(&self, opts: &PopOpts) -> Result<()> {
        let _repo_root = find_repo_root()?;

        // Apply stash
        let apply_opts = ApplyOpts {
            stash: opts.stash,
            index: opts.index,
            quiet: opts.quiet,
        };
        self.apply(&apply_opts).await?;

        // Drop stash after successful apply
        let drop_opts = DropOpts {
            stash: opts.stash,
            quiet: opts.quiet,
        };
        self.drop(&drop_opts).await?;

        Ok(())
    }

    async fn clear(&self, opts: &ClearOpts) -> Result<()> {
        let repo_root = find_repo_root()?;
        let mediagit_dir = repo_root.join(".mediagit");

        if !opts.force {
            // Prompt for confirmation
            use dialoguer::Confirm;
            let confirmed = Confirm::new()
                .with_prompt("Clear all stash entries?")
                .default(false)
                .interact()?;

            if !confirmed {
                println!("Cancelled");
                return Ok(());
            }
        }

        // Clear stash list
        let stash_path = mediagit_dir.join("STASH_LIST");
        if stash_path.exists() {
            std::fs::remove_file(&stash_path)?;
        }

        println!("{} Cleared all stash entries", style("✓").green());

        Ok(())
    }

    async fn get_current_branch(&self, refdb: &RefDatabase) -> Result<Option<String>> {
        let head = refdb.read("HEAD").await?;
        Ok(head.target.map(|t| {
            t.strip_prefix("refs/heads/")
                .unwrap_or(&t)
                .to_string()
        }))
    }

    fn save_stash_entry(&self, mediagit_dir: &PathBuf, entry: StashEntry) -> Result<()> {
        let mut stash_list = self.load_all_stashes(mediagit_dir)?;
        stash_list.insert(0, entry); // Add to front
        self.save_all_stashes(mediagit_dir, &stash_list)
    }

    fn load_stash_entry(&self, mediagit_dir: &PathBuf, index: usize) -> Result<StashEntry> {
        let stash_list = self.load_all_stashes(mediagit_dir)?;

        stash_list.get(index)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("Stash entry {} not found", index))
    }

    fn load_all_stashes(&self, mediagit_dir: &PathBuf) -> Result<Vec<StashEntry>> {
        let stash_path = mediagit_dir.join("STASH_LIST");

        if !stash_path.exists() {
            return Ok(Vec::new());
        }

        let stash_json = std::fs::read_to_string(&stash_path)?;
        let stash_list: Vec<StashEntry> = serde_json::from_str(&stash_json)?;

        Ok(stash_list)
    }

    fn save_all_stashes(&self, mediagit_dir: &PathBuf, stash_list: &[StashEntry]) -> Result<()> {
        let stash_path = mediagit_dir.join("STASH_LIST");
        let stash_json = serde_json::to_string_pretty(stash_list)?;
        std::fs::write(&stash_path, stash_json)?;

        Ok(())
    }

}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct StashEntry {
    commit_oid: String,
    message: String,
    timestamp: String,
    branch: Option<String>,
}
