// Copyright (C) 2026  winnyboy5
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU Affero General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU Affero General Public License for more details.
//
// You should have received a copy of the GNU Affero General Public License
// along with this program.  If not, see <https://www.gnu.org/licenses/>.
use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use console::style;
use mediagit_versioning::{CheckoutManager, Commit, Index, ObjectDatabase, ObjectType, Oid, RefDatabase, Tree};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
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

        // Load index (staged changes)
        let index = Index::load(&repo_root)?;

        // Get current HEAD
        let current_oid = refdb.resolve("HEAD").await
            .context("Failed to resolve HEAD")?;

        // Load HEAD commit tree to detect working-tree modifications
        let head_data = odb.read(&current_oid).await
            .context("Failed to read HEAD commit")?;
        let head_commit = Commit::deserialize(&head_data)
            .context("Failed to deserialize HEAD commit")?;
        let tree_data = odb.read(&head_commit.tree).await
            .context("Failed to read HEAD tree")?;
        let head_tree = Tree::deserialize(&tree_data)
            .context("Failed to deserialize HEAD tree")?;

        // Build HEAD file map (path -> oid)
        let mut head_files: HashMap<PathBuf, Oid> = HashMap::new();
        for entry in head_tree.iter() {
            head_files.insert(PathBuf::from(&entry.name), entry.oid);
        }

        // Scan working directory for modifications against HEAD
        let working_files = self.scan_working_directory(&repo_root)?;
        let mut working_tree_changes: Vec<(PathBuf, Oid)> = Vec::new();

        for (path, head_oid) in &head_files {
            if !working_files.contains(path) {
                continue; // Deleted file — tracked by absence
            }
            let full_path = repo_root.join(path);
            let working_oid = if let Ok(content) = std::fs::read(&full_path) {
                Oid::hash(&content)
            } else {
                continue;
            };
            if working_oid != *head_oid {
                working_tree_changes.push((path.clone(), working_oid));
            }
        }

        // BUG-3 fix: Check BOTH index and working-tree changes
        if index.is_empty() && working_tree_changes.is_empty() {
            if !opts.quiet {
                println!("{} No changes to stash", style("ℹ").blue());
            }
            return Ok(());
        }

        // Build stash tree: start from HEAD tree as base, then overlay modifications
        let mut tree = mediagit_versioning::Tree::new();

        // 1. Start with all HEAD tree entries as base
        for entry in head_tree.iter() {
            tree.add_entry(mediagit_versioning::TreeEntry::new(
                entry.name.clone(),
                entry.mode,
                entry.oid,
            ));
        }

        // 2. Override with working-tree modifications (write blobs to ODB)
        for (path, _working_oid) in &working_tree_changes {
            let full_path = repo_root.join(path);
            if let Ok(content) = std::fs::read(&full_path) {
                let blob_oid = odb.write(ObjectType::Blob, &content).await
                    .context(format!("Failed to write blob for {}", path.display()))?;
                tree.add_entry(mediagit_versioning::TreeEntry::new(
                    path.to_string_lossy().to_string(),
                    mediagit_versioning::FileMode::Regular,
                    blob_oid,
                ));
            }
        }

        // 3. Override with index entries (staged changes take priority)
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

    /// Scan working directory for files (excluding .mediagit)
    fn scan_working_directory(&self, repo_root: &Path) -> Result<HashSet<PathBuf>> {
        let mut files = HashSet::new();
        self.scan_directory_recursive(repo_root, repo_root, &mut files)?;
        Ok(files)
    }

    fn scan_directory_recursive(&self, repo_root: &Path, current_dir: &Path, files: &mut HashSet<PathBuf>) -> Result<()> {
        for entry in std::fs::read_dir(current_dir)? {
            let entry = entry?;
            let path = entry.path();

            // Skip .mediagit directory
            if path.file_name().and_then(|n| n.to_str()) == Some(".mediagit") {
                continue;
            }

            if path.is_file() {
                if let Ok(rel_path) = path.strip_prefix(repo_root) {
                    files.insert(rel_path.to_path_buf());
                }
            } else if path.is_dir() {
                self.scan_directory_recursive(repo_root, &path, files)?;
            }
        }
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
