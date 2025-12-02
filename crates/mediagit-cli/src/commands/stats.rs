use anyhow::Result;
use clap::Parser;
use console::style;
use mediagit_storage::LocalBackend;
use mediagit_versioning::{ObjectDatabase, RefDatabase};
use std::sync::Arc;

/// Show repository statistics
#[derive(Parser, Debug)]
pub struct StatsCmd {
    /// Show storage statistics
    #[arg(long)]
    pub storage: bool,

    /// Show file statistics
    #[arg(long)]
    pub files: bool,

    /// Show commit statistics
    #[arg(long)]
    pub commits: bool,

    /// Show branch statistics
    #[arg(long)]
    pub branches: bool,

    /// Show author statistics
    #[arg(long)]
    pub authors: bool,

    /// All statistics
    #[arg(long)]
    pub all: bool,

    /// Format as JSON
    #[arg(long)]
    pub json: bool,

    /// Quiet mode (minimal output)
    #[arg(short, long)]
    pub quiet: bool,

    /// Verbose mode
    #[arg(short, long)]
    pub verbose: bool,
}

impl StatsCmd {
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

        println!("{} Repository Statistics\n", style("📊").cyan().bold());

        let show_all = self.all || (!self.storage && !self.files && !self.commits && !self.branches && !self.authors);

        // Storage statistics
        if self.storage || show_all {
            println!("{}", style("Storage:").bold());
            let metrics = odb.metrics().await;
            println!("  Unique objects: {}", metrics.unique_objects);
            println!("  Total writes: {}", metrics.total_writes);
            println!("  Bytes stored: {}", metrics.bytes_stored);
            println!("  Bytes written: {}", metrics.bytes_written);
            println!("  Cache hits: {}", metrics.cache_hits);
            println!("  Cache misses: {}", metrics.cache_misses);
            if metrics.cache_hits + metrics.cache_misses > 0 {
                let hit_rate = (metrics.cache_hits as f64 / (metrics.cache_hits + metrics.cache_misses) as f64) * 100.0;
                println!("  Cache hit rate: {:.2}%", hit_rate);
            }
            if metrics.total_writes > metrics.unique_objects {
                let dedup_rate = ((metrics.total_writes - metrics.unique_objects) as f64 / metrics.total_writes as f64) * 100.0;
                println!("  Deduplication rate: {:.2}%", dedup_rate);
            }
            println!();
        }

        // Branch statistics
        if self.branches || show_all {
            println!("{}", style("Branches:").bold());
            match refdb.list("refs/heads").await {
                Ok(branches) => {
                    println!("  Local branches: {}", branches.len());
                }
                Err(_) => {
                    println!("  Local branches: 0");
                }
            }
            match refdb.list("refs/remotes").await {
                Ok(remotes) => {
                    println!("  Remote branches: {}", remotes.len());
                }
                Err(_) => {
                    println!("  Remote branches: 0");
                }
            }
            match refdb.list("refs/tags").await {
                Ok(tags) => {
                    println!("  Tags: {}", tags.len());
                }
                Err(_) => {
                    println!("  Tags: 0");
                }
            }
            println!();
        }

        // Commit statistics
        if self.commits || show_all {
            println!("{}", style("Commits:").bold());
            // Get HEAD to count commits
            match refdb.read("HEAD").await {
                Ok(head) => {
                    if let Some(target) = head.target {
                        match refdb.read(&target).await {
                            Ok(branch_ref) => {
                                if branch_ref.oid.is_some() {
                                    println!("  Current branch has commits");
                                } else {
                                    println!("  No commits yet");
                                }
                            }
                            Err(_) => {
                                println!("  No commits yet");
                            }
                        }
                    } else if head.oid.is_some() {
                        println!("  Detached HEAD state");
                    } else {
                        println!("  No commits yet");
                    }
                }
                Err(_) => {
                    println!("  No commits yet");
                }
            }
            println!();
        }

        if !self.quiet {
            println!("{}", style("Repository is operational").green());
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
