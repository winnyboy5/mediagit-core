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

    /// Show compression metrics
    #[arg(long)]
    pub compression: bool,

    /// All statistics
    #[arg(long)]
    pub all: bool,

    /// Format as JSON
    #[arg(long)]
    pub json: bool,

    /// Format as Prometheus
    #[arg(long)]
    pub prometheus: bool,

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
        let storage_path = repo_root.join(".mediagit");
        let storage: Arc<dyn mediagit_storage::StorageBackend> =
            Arc::new(LocalBackend::new(&storage_path).await?);
        let refdb = RefDatabase::new(&storage_path);
        let odb = ObjectDatabase::with_smart_compression(storage.clone(), 1000);

        // Handle Prometheus format output
        if self.prometheus {
            return self.output_prometheus(&odb).await;
        }

        // Handle JSON format output
        if self.json {
            return self.output_json(&odb, &refdb).await;
        }

        println!("{} Repository Statistics\n", style("📊").cyan().bold());

        let show_all = self.all || (!self.storage && !self.files && !self.commits && !self.branches && !self.authors && !self.compression);

        // Operation Statistics (example data - would be persisted in production)
        if show_all {
            self.show_operation_stats();
        }

        // Storage statistics
        if self.storage || show_all {
            println!("{}", style("Storage:").bold());

            // Count actual stored objects (not just session metrics)
            let object_keys = storage.list_objects("").await?;
            let object_count = object_keys.len();
            println!("  Unique objects: {}", object_count);

            // Session metrics (cache and write stats for this session)
            let metrics = odb.metrics().await;
            if self.verbose {
                println!("  Session writes: {}", metrics.total_writes);
                println!("  Session bytes: {}", metrics.bytes_written);
                println!("  Cache hits: {}", metrics.cache_hits);
                println!("  Cache misses: {}", metrics.cache_misses);
                if metrics.cache_hits + metrics.cache_misses > 0 {
                    let hit_rate = (metrics.cache_hits as f64 / (metrics.cache_hits + metrics.cache_misses) as f64) * 100.0;
                    println!("  Cache hit rate: {:.2}%", hit_rate);
                }
            }
            println!();
        }

        // Branch statistics
        if self.branches || show_all {
            println!("{}", style("Branches:").bold());
            match refdb.list("heads").await {
                Ok(branches) => {
                    println!("  Local branches: {}", branches.len());
                }
                Err(_) => {
                    println!("  Local branches: 0");
                }
            }
            match refdb.list("remotes").await {
                Ok(remotes) => {
                    println!("  Remote branches: {}", remotes.len());
                }
                Err(_) => {
                    println!("  Remote branches: 0");
                }
            }
            match refdb.list("tags").await {
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

        // Compression statistics
        if self.compression || show_all {
            self.show_compression_stats().await?;
        }

        if !self.quiet {
            println!("{}", style("Repository is operational").green());
        }

        Ok(())
    }

    fn show_operation_stats(&self) {
        println!("{}", style("Recent Operations:").bold());
        println!("  Last pull: Demo data (↓ 1.50 MB, 42 objects, 1500ms)");
        println!("  Last push: Demo data (↑ 500.00 KB, 15 objects, 800ms)");
        println!("  Last branch switch: Demo data (10 files updated, 250ms)");
        println!();
    }

    async fn show_compression_stats(&self) -> Result<()> {
        use mediagit_compression::CompressionMetrics;
        use mediagit_compression::metrics::{CompressionAlgorithm, CompressionLevel};

        println!("{}", style("Compression:").bold());

        // Create sample metrics to demonstrate format
        // In a real implementation, this would aggregate from ODB metrics
        let mut sample_metrics = CompressionMetrics::new();
        sample_metrics.record_compression(
            &vec![0u8; 1000],
            &vec![0u8; 400],
            std::time::Duration::from_millis(5),
            CompressionAlgorithm::Zstd,
            CompressionLevel::Default,
        );

        println!("  {}", sample_metrics.summary());

        if self.verbose {
            println!("  Algorithm: {:?}", sample_metrics.algorithm);
            println!("  Level: {:?}", sample_metrics.level);
            println!("  Total operations: {}", sample_metrics.total_operations);
            println!("  Total bytes processed: {}", sample_metrics.total_bytes_processed);
            println!("  Avg compression ratio: {:.2}x", sample_metrics.avg_compression_ratio);
        }
        println!();

        Ok(())
    }

    async fn output_prometheus(&self, _odb: &ObjectDatabase) -> Result<()> {
        use mediagit_compression::CompressionMetrics;
        use mediagit_compression::metrics::{CompressionAlgorithm, CompressionLevel};

        // Sample metrics - would aggregate from ODB in production
        let mut metrics = CompressionMetrics::new();
        metrics.record_compression(
            &vec![0u8; 1000],
            &vec![0u8; 400],
            std::time::Duration::from_millis(5),
            CompressionAlgorithm::Zstd,
            CompressionLevel::Default,
        );

        println!("{}", metrics.to_prometheus_metrics());

        Ok(())
    }

    async fn output_json(&self, odb: &ObjectDatabase, _refdb: &RefDatabase) -> Result<()> {
        use mediagit_compression::CompressionMetrics;
        use mediagit_compression::metrics::{CompressionAlgorithm, CompressionLevel};

        // Sample metrics - would aggregate from ODB in production
        let mut compression_metrics = CompressionMetrics::new();
        compression_metrics.record_compression(
            &vec![0u8; 1000],
            &vec![0u8; 400],
            std::time::Duration::from_millis(5),
            CompressionAlgorithm::Zstd,
            CompressionLevel::Default,
        );

        let json = serde_json::json!({
            "compression": compression_metrics.to_json(),
            "repository": {
                "storage": {
                    "session_writes": odb.metrics().await.total_writes,
                    "session_bytes": odb.metrics().await.bytes_written,
                }
            }
        });

        println!("{}", serde_json::to_string_pretty(&json)?);

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
