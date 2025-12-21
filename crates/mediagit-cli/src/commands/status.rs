use anyhow::{Context, Result};
use clap::Parser;
use mediagit_storage::LocalBackend;
use mediagit_versioning::{Index, ObjectDatabase, ObjectType, Oid, Ref, RefDatabase};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// Show the working tree status
///
/// Displays the state of the working directory and staging area, showing which
/// changes have been staged, which haven't, and which files aren't being tracked.
#[derive(Parser, Debug)]
#[command(after_help = "EXAMPLES:
    # Show repository status
    mediagit status

    # Show status with branch information
    mediagit status -b

    # Show status in short format
    mediagit status -s

    # Show status in porcelain format (for scripts)
    mediagit status --porcelain

    # Show tracked files only
    mediagit status --tracked

    # Show untracked files only
    mediagit status --untracked

SEE ALSO:
    mediagit-add(1), mediagit-commit(1), mediagit-diff(1)")]
pub struct StatusCmd {
    /// Show tracked files
    #[arg(long)]
    pub tracked: bool,

    /// Show untracked files
    #[arg(long)]
    pub untracked: bool,

    /// Show ignored files
    #[arg(long)]
    pub ignored: bool,

    /// Show short format
    #[arg(short, long)]
    pub short: bool,

    /// Show porcelain format (for scripts)
    #[arg(long)]
    pub porcelain: bool,

    /// Show branch information
    #[arg(short = 'b', long)]
    pub branch: bool,

    /// Show ahead/behind commits
    #[arg(long)]
    pub ahead_behind: bool,

    /// Quiet mode
    #[arg(short, long)]
    pub quiet: bool,

    /// Verbose mode
    #[arg(short, long)]
    pub verbose: bool,
}

impl StatusCmd {
    pub async fn execute(&self) -> Result<()> {
        use crate::output;

        // Find repository root
        let repo_root = self.find_repo_root()?;

        if !self.quiet {
            output::header("Repository Status");
        }

        // Initialize storage and ref database
        let storage_path = repo_root.join(".mediagit");
        let storage: Arc<dyn mediagit_storage::StorageBackend> =
            Arc::new(LocalBackend::new(&storage_path).await?);
        let refdb = RefDatabase::new(&storage_path);

        // Read HEAD
        let head = refdb
            .read("HEAD")
            .await
            .context("Failed to read HEAD reference")?;

        // Display current branch
        if self.branch || self.verbose {
            match &head {
                Ref {
                    ref_type: mediagit_versioning::RefType::Symbolic,
                    target: Some(branch),
                    ..
                } => {
                    let branch_name = branch.strip_prefix("refs/heads/").unwrap_or(branch);
                    output::success(&format!("On branch: {}", branch_name));
                }
                Ref {
                    ref_type: mediagit_versioning::RefType::Direct,
                    oid: Some(oid),
                    ..
                } => {
                    output::info(&format!("HEAD detached at {}", oid));
                }
                _ => {
                    output::warning("HEAD reference is invalid");
                }
            }
        }

        // Check if we have any commits by trying to resolve HEAD
        let has_commits = refdb.resolve("HEAD").await.is_ok();

        // Load index and initialize ODB for file comparison (ISS-005 fix)
        let index = Index::load(&repo_root)?;
        let odb = ObjectDatabase::with_smart_compression(storage.clone(), 1000);

        // Scan working directory
        let working_files = self.scan_working_directory(&repo_root)?;

        // Get HEAD commit tree for comparison (index is cleared after commit)
        let mut head_files: HashMap<PathBuf, Oid> = HashMap::new();
        if let Ok(head_oid) = refdb.resolve("HEAD").await {
            if let Ok(commit_data) = odb.read(&head_oid).await {
                if let Ok(commit) = bincode::deserialize::<mediagit_versioning::Commit>(&commit_data) {
                    if let Ok(tree_data) = odb.read(&commit.tree).await {
                        if let Ok(tree) = bincode::deserialize::<mediagit_versioning::Tree>(&tree_data) {
                            for entry in tree.iter() {
                                head_files.insert(PathBuf::from(&entry.name), entry.oid);
                            }
                        }
                    }
                }
            }
        }

        // Build index file map (path -> oid) for staged changes
        let mut index_files: HashMap<PathBuf, Oid> = HashMap::new();
        for entry in index.entries() {
            index_files.insert(entry.path.clone(), entry.oid);
        }

        // Detect modified files (in HEAD but different in working dir)
        let mut modified_files = Vec::new();
        for (path, head_oid) in &head_files {
            if working_files.contains(path) {
                // File exists in working dir, check if modified
                let full_path = repo_root.join(path);
                if let Ok(content) = std::fs::read(&full_path) {
                    if let Ok(working_oid) = odb.write(ObjectType::Blob, &content).await {
                        if working_oid != *head_oid && !index_files.contains_key(path) {
                            // Modified but not staged
                            modified_files.push(path.clone());
                        }
                    }
                }
            }
        }

        // Detect deleted files (in HEAD, not in working dir, not staged for deletion)
        let mut deleted_files = Vec::new();
        for path in head_files.keys() {
            if !working_files.contains(path) && !index_files.contains_key(path) {
                deleted_files.push(path.clone());
            }
        }

        // Detect untracked files (in working dir, not in HEAD, not in index)
        let mut untracked_files = Vec::new();
        for path in &working_files {
            if !head_files.contains_key(path) && !index_files.contains_key(path) {
                untracked_files.push(path.clone());
            }
        }

        // Display staged files
        if !index.is_empty() && !self.quiet {
            output::header("Changes to be committed:");
            println!("  (use \"mediagit reset <file>...\" to unstage)");
            println!();

            for entry in index.entries() {
                let status_prefix = if self.short { "A " } else { "  new file:   " };
                output::success(&format!("{}{}", status_prefix, entry.path.display()));
            }
            println!();
        }

        // Display modified files (ISS-005 fix)
        if !modified_files.is_empty() && !self.quiet {
            output::header("Changes not staged for commit:");
            println!("  (use \"mediagit add <file>...\" to update what will be committed)");
            println!();

            for path in &modified_files {
                let status_prefix = if self.short { " M " } else { "  modified:   " };
                println!("{}{}", status_prefix, path.display());
            }
            println!();
        }

        // Display deleted files (ISS-005 fix)
        if !deleted_files.is_empty() && !self.quiet {
            if modified_files.is_empty() {
                output::header("Changes not staged for commit:");
                println!("  (use \"mediagit add <file>...\" to update what will be committed)");
                println!();
            }

            for path in &deleted_files {
                let status_prefix = if self.short { " D " } else { "  deleted:    " };
                println!("{}{}", status_prefix, path.display());
            }
            println!();
        }

        // Display untracked files (ISS-005 fix)
        if !untracked_files.is_empty() && !self.quiet {
            output::header("Untracked files:");
            println!("  (use \"mediagit add <file>...\" to include in what will be committed)");
            println!();

            for path in &untracked_files {
                println!("  {}", path.display());
            }
            println!();
        }

        // Display clean status (ISS-005 fix)
        if !self.quiet {
            if index.is_empty() && modified_files.is_empty() && deleted_files.is_empty() && untracked_files.is_empty() {
                if !has_commits {
                    output::info("No commits yet");
                }
                output::info("Nothing to commit, working tree clean");
            } else if !index.is_empty() || !modified_files.is_empty() || !deleted_files.is_empty() {
                // Has changes
            } else if !untracked_files.is_empty() {
                output::info("no changes added to commit (use \"mediagit add\" to track)");
            }
        }

        Ok(())
    }

    // ISS-005 fix: Helper function to scan working directory
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
                // Store as relative path
                if let Ok(rel_path) = path.strip_prefix(repo_root) {
                    files.insert(rel_path.to_path_buf());
                }
            } else if path.is_dir() {
                self.scan_directory_recursive(repo_root, &path, files)?;
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
                anyhow::bail!("Not a mediagit repository (or any parent up to mount point)");
            }
        }
    }
}
