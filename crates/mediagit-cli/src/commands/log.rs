use anyhow::{Context, Result};
use clap::Parser;
use console::style;
use mediagit_storage::LocalBackend;
use mediagit_versioning::{Commit, ObjectDatabase, Oid, RefDatabase};
use std::collections::HashSet;
use std::sync::Arc;

/// Show commit history
#[derive(Parser, Debug)]
pub struct LogCmd {
    /// Revision range (e.g., main..feature, v1.0..v2.0)
    #[arg(value_name = "REVISION")]
    pub revision: Option<String>,

    /// Maximum number of commits to show
    #[arg(short = 'n', long, value_name = "NUM")]
    pub max_count: Option<usize>,

    /// Skip N commits
    #[arg(long, value_name = "NUM")]
    pub skip: Option<usize>,

    /// Show abbreviated commit hash
    #[arg(long)]
    pub oneline: bool,

    /// Show graph representation
    #[arg(long)]
    pub graph: bool,

    /// Show commit statistics
    #[arg(long)]
    pub stat: bool,

    /// Show patches
    #[arg(short = 'p', long)]
    pub patch: bool,

    /// Show commits by author
    #[arg(long, value_name = "PATTERN")]
    pub author: Option<String>,

    /// Show commits with matching message
    #[arg(long, value_name = "PATTERN")]
    pub grep: Option<String>,

    /// Show commits since date
    #[arg(long, value_name = "DATE")]
    pub since: Option<String>,

    /// Show commits until date
    #[arg(long, value_name = "DATE")]
    pub until: Option<String>,

    /// Show only commits affecting these paths
    #[arg(value_name = "PATHS")]
    pub paths: Vec<String>,

    /// Show verbose output
    #[arg(short, long)]
    pub verbose: bool,

    /// Quiet mode
    #[arg(short, long)]
    pub quiet: bool,
}

impl LogCmd {
    pub async fn execute(&self) -> Result<()> {
        if self.quiet {
            return Ok(());
        }

        let repo_root = self.find_repo_root()?;
        let storage_path = repo_root.join(".mediagit/objects");
        let storage: Arc<dyn mediagit_storage::StorageBackend> =
            Arc::new(LocalBackend::new(&storage_path).await?);
        let refdb = RefDatabase::new(storage.clone());
        let odb = ObjectDatabase::new(storage, 1000);

        // Get starting commit OID
        let start_oid = if let Some(revision) = &self.revision {
            // Try to resolve revision as a reference
            let ref_result = refdb.read(revision).await;
            match ref_result {
                Ok(r) => r.oid.ok_or_else(|| anyhow::anyhow!("Revision {} has no commit", revision))?,
                Err(_) => {
                    // Try to parse as OID
                    Oid::from_hex(revision)
                        .map_err(|_| anyhow::anyhow!("Invalid revision: {}", revision))?
                }
            }
        } else {
            // Use HEAD
            let head = refdb.read("HEAD").await?;
            match head.oid {
                Some(oid) => oid,
                None => {
                    // HEAD might be symbolic, resolve it
                    if let Some(target) = head.target {
                        let target_ref = refdb.read(&target).await?;
                        target_ref.oid.ok_or_else(|| anyhow::anyhow!("No commits yet"))?
                    } else {
                        anyhow::bail!("No commits yet");
                    }
                }
            }
        };

        // Traverse commit history
        let mut commits_to_show = Vec::new();
        let mut visited = HashSet::new();
        let mut stack = vec![start_oid];

        while let Some(oid) = stack.pop() {
            if visited.contains(&oid) {
                continue;
            }
            visited.insert(oid);

            // Read commit object
            let data = odb.read(&oid).await?;
            let commit = Commit::deserialize(&data)
                .with_context(|| format!("Failed to deserialize commit {}", oid))?;

            // Apply filters
            if let Some(author_pattern) = &self.author {
                if !commit.author.name.contains(author_pattern)
                    && !commit.author.email.contains(author_pattern)
                {
                    // Add parents to stack even if this commit is filtered
                    for parent in &commit.parents {
                        if !visited.contains(parent) {
                            stack.push(*parent);
                        }
                    }
                    continue;
                }
            }

            if let Some(grep_pattern) = &self.grep {
                if !commit.message.contains(grep_pattern) {
                    // Add parents to stack even if this commit is filtered
                    for parent in &commit.parents {
                        if !visited.contains(parent) {
                            stack.push(*parent);
                        }
                    }
                    continue;
                }
            }

            commits_to_show.push((oid, commit.clone()));

            // Add parents to stack
            for parent in &commit.parents {
                if !visited.contains(parent) {
                    stack.push(*parent);
                }
            }

            // Check if we've reached the limit
            if let Some(max_count) = self.max_count {
                if commits_to_show.len() >= max_count + self.skip.unwrap_or(0) {
                    break;
                }
            }
        }

        // Apply skip
        if let Some(skip) = self.skip {
            if skip < commits_to_show.len() {
                commits_to_show.drain(0..skip);
            } else {
                commits_to_show.clear();
            }
        }

        // Display commits
        if commits_to_show.is_empty() {
            println!("{}", style("No commits to show").dim());
            return Ok(());
        }

        for (oid, commit) in commits_to_show {
            if self.oneline {
                // One-line format
                let short_oid = &oid.to_string()[..7];
                let short_msg = commit.message.lines().next().unwrap_or("");
                println!("{} {}", style(short_oid).yellow(), short_msg);
            } else {
                // Full format
                println!("{} {}", style("commit").yellow().bold(), style(oid).yellow());
                println!("Author: {} <{}>", commit.author.name, commit.author.email);
                println!("Date:   {}", commit.author.timestamp);
                println!();
                for line in commit.message.lines() {
                    println!("    {}", line);
                }
                println!();
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
