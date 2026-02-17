use anyhow::Result;
use chrono::Duration;
use clap::Parser;
use console::style;
use indicatif::HumanBytes;
use mediagit_versioning::{Commit, ObjectDatabase, Oid, RefDatabase, Tree};
use std::collections::{HashMap, HashSet};
use std::path::Path;
use super::super::repo::{find_repo_root, create_storage_backend};

/// Format a duration as a human-readable "time ago" string
fn format_duration_ago(duration: Duration) -> String {
    let secs = duration.num_seconds();
    if secs < 60 {
        format!("{} seconds ago", secs)
    } else if secs < 3600 {
        format!("{} minutes ago", secs / 60)
    } else if secs < 86400 {
        format!("{} hours ago", secs / 3600)
    } else {
        format!("{} days ago", secs / 86400)
    }
}

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

/// Storage statistics gathered from disk
#[derive(Debug, Default)]
struct StorageStats {
    loose_object_count: u64,
    loose_bytes: u64,
    pack_count: u64,
    pack_bytes: u64,
    chunk_count: u64,
    chunk_bytes: u64,
    manifest_count: u64,
}

/// Commit history statistics
#[derive(Debug, Default)]
struct CommitStats {
    total_commits: u64,
    first_commit_date: Option<chrono::DateTime<chrono::Utc>>,
    last_commit_date: Option<chrono::DateTime<chrono::Utc>>,
}

/// Author statistics
#[derive(Debug, Clone)]
struct AuthorStat {
    name: String,
    email: String,
    commit_count: u64,
}

/// File statistics from tree
#[derive(Debug, Default)]
struct FileStats {
    total_files: u64,
    #[allow(dead_code)]
    total_size_estimate: u64,
    media_files: u64,
    text_files: u64,
    other_files: u64,
}

impl StatsCmd {
    pub async fn execute(&self) -> Result<()> {
        if self.quiet {
            return Ok(());
        }

        let repo_root = find_repo_root()?;
        let storage_path = repo_root.join(".mediagit");
        let storage = create_storage_backend(&repo_root).await?;
        let refdb = RefDatabase::new(&storage_path);
        let odb = ObjectDatabase::with_smart_compression(storage.clone(), 1000);

        // Handle Prometheus format output
        if self.prometheus {
            return self.output_prometheus(&storage_path, &odb, &refdb).await;
        }

        // Handle JSON format output
        if self.json {
            return self.output_json(&storage_path, &odb, &refdb).await;
        }

        println!("{} Repository Statistics\n", style("📊").cyan().bold());

        let show_all = self.all || (!self.storage && !self.files && !self.commits && !self.branches && !self.authors && !self.compression);

        // Operation Statistics from persisted data
        if show_all {
            self.show_operation_stats(&storage_path);
        }

        // Storage statistics (real data from disk)
        if self.storage || show_all {
            let stats = self.compute_storage_stats(&storage_path).await?;
            println!("{}", style("Storage:").bold());
            
            let total_objects = stats.loose_object_count + stats.chunk_count;
            let total_bytes = stats.loose_bytes + stats.pack_bytes + stats.chunk_bytes;
            
            println!("  Total objects: {} ({} loose, {} chunks)", 
                total_objects, stats.loose_object_count, stats.chunk_count);
            println!("  Total size: {} (loose: {}, packs: {}, chunks: {})",
                HumanBytes(total_bytes),
                HumanBytes(stats.loose_bytes),
                HumanBytes(stats.pack_bytes),
                HumanBytes(stats.chunk_bytes));
            
            if stats.pack_count > 0 {
                println!("  Pack files: {}", stats.pack_count);
            }
            if stats.manifest_count > 0 {
                println!("  Chunk manifests: {}", stats.manifest_count);
            }
            
            if self.verbose {
                let metrics = odb.metrics().await;
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
                Ok(branches) => println!("  Local branches: {}", branches.len()),
                Err(_) => println!("  Local branches: 0"),
            }
            match refdb.list("remotes").await {
                Ok(remotes) => println!("  Remote branches: {}", remotes.len()),
                Err(_) => println!("  Remote branches: 0"),
            }
            match refdb.list("tags").await {
                Ok(tags) => println!("  Tags: {}", tags.len()),
                Err(_) => println!("  Tags: 0"),
            }
            println!();
        }

        // Commit statistics (real data from walking history)
        if self.commits || show_all {
            println!("{}", style("Commits:").bold());
            match self.compute_commit_stats(&odb, &refdb).await {
                Ok(stats) => {
                    if stats.total_commits > 0 {
                        println!("  Total commits: {}", stats.total_commits);
                        if let Some(first) = stats.first_commit_date {
                            println!("  First commit: {}", first.format("%Y-%m-%d"));
                        }
                        if let Some(last) = stats.last_commit_date {
                            println!("  Last commit: {}", last.format("%Y-%m-%d"));
                        }
                    } else {
                        println!("  No commits yet");
                    }
                }
                Err(_) => println!("  No commits yet"),
            }
            println!();
        }

        // File statistics (real data from HEAD tree)
        if self.files || show_all {
            println!("{}", style("Files:").bold());
            match self.compute_file_stats(&odb, &refdb).await {
                Ok(stats) => {
                    println!("  Tracked files: {}", stats.total_files);
                    if stats.media_files > 0 || stats.text_files > 0 {
                        println!("  Media files: {}", stats.media_files);
                        println!("  Text files: {}", stats.text_files);
                        if stats.other_files > 0 {
                            println!("  Other files: {}", stats.other_files);
                        }
                    }
                }
                Err(_) => println!("  No files tracked"),
            }
            println!();
        }

        // Author statistics (real data from commit history)
        if self.authors || show_all {
            println!("{}", style("Authors:").bold());
            match self.compute_author_stats(&odb, &refdb).await {
                Ok(authors) => {
                    if authors.is_empty() {
                        println!("  No authors yet");
                    } else {
                        for author in authors.iter().take(10) {
                            println!("  {} <{}>: {} commit{}",
                                author.name,
                                author.email,
                                author.commit_count,
                                if author.commit_count == 1 { "" } else { "s" });
                        }
                        if authors.len() > 10 {
                            println!("  ... and {} more", authors.len() - 10);
                        }
                    }
                }
                Err(_) => println!("  No authors yet"),
            }
            println!();
        }

        // Compression statistics (from storage analysis)
        if self.compression || show_all {
            self.show_compression_stats(&storage_path).await?;
        }

        if !self.quiet {
            println!("{}", style("Repository is operational").green());
        }

        Ok(())
    }

    /// Compute storage statistics by walking the .mediagit directory
    async fn compute_storage_stats(&self, storage_path: &Path) -> Result<StorageStats> {
        let mut stats = StorageStats::default();
        
        // Count loose objects in objects/ directory
        let objects_dir = storage_path.join("objects");
        if objects_dir.exists() {
            for entry in walkdir::WalkDir::new(&objects_dir)
                .into_iter()
                .filter_map(|e| e.ok())
                .filter(|e| e.file_type().is_file())
            {
                let path = entry.path();
                let filename = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
                
                // Skip pack and index files
                if filename.ends_with(".pack") || filename.ends_with(".idx") {
                    if filename.ends_with(".pack") {
                        stats.pack_count += 1;
                        if let Ok(meta) = std::fs::metadata(path) {
                            stats.pack_bytes += meta.len();
                        }
                    }
                } else {
                    // Loose object (64-char hex filename in sharded directory)
                    stats.loose_object_count += 1;
                    if let Ok(meta) = std::fs::metadata(path) {
                        stats.loose_bytes += meta.len();
                    }
                }
            }
        }
        
        // Count chunks
        let chunks_dir = storage_path.join("chunks");
        if chunks_dir.exists() {
            for entry in walkdir::WalkDir::new(&chunks_dir)
                .into_iter()
                .filter_map(|e| e.ok())
                .filter(|e| e.file_type().is_file())
            {
                stats.chunk_count += 1;
                if let Ok(meta) = std::fs::metadata(entry.path()) {
                    stats.chunk_bytes += meta.len();
                }
            }
        }
        
        // Count manifests
        let manifests_dir = storage_path.join("manifests");
        if manifests_dir.exists() {
            for _entry in walkdir::WalkDir::new(&manifests_dir)
                .into_iter()
                .filter_map(|e| e.ok())
                .filter(|e| e.file_type().is_file())
            {
                stats.manifest_count += 1;
            }
        }
        
        Ok(stats)
    }

    /// Compute commit statistics by walking commit history from HEAD
    async fn compute_commit_stats(&self, odb: &ObjectDatabase, refdb: &RefDatabase) -> Result<CommitStats> {
        let mut stats = CommitStats::default();
        
        // Get HEAD commit OID
        let head_oid = self.resolve_head(refdb).await?;
        
        // Walk commit history
        let mut visited = HashSet::new();
        let mut queue = vec![head_oid];
        
        while let Some(oid) = queue.pop() {
            if visited.contains(&oid) {
                continue;
            }
            visited.insert(oid);
            
            // Read commit
            match Commit::read(odb, &oid).await {
                Ok(commit) => {
                    stats.total_commits += 1;
                    
                    // Track date range
                    let commit_date = commit.author.timestamp;
                    match stats.first_commit_date {
                        None => stats.first_commit_date = Some(commit_date),
                        Some(first) if commit_date < first => stats.first_commit_date = Some(commit_date),
                        _ => {}
                    }
                    match stats.last_commit_date {
                        None => stats.last_commit_date = Some(commit_date),
                        Some(last) if commit_date > last => stats.last_commit_date = Some(commit_date),
                        _ => {}
                    }
                    
                    // Add parents to queue
                    for parent in &commit.parents {
                        queue.push(*parent);
                    }
                }
                Err(_) => continue, // Skip unreadable commits
            }
        }
        
        Ok(stats)
    }

    /// Compute author statistics from commit history
    async fn compute_author_stats(&self, odb: &ObjectDatabase, refdb: &RefDatabase) -> Result<Vec<AuthorStat>> {
        let mut author_counts: HashMap<String, (String, String, u64)> = HashMap::new();
        
        // Get HEAD commit OID
        let head_oid = self.resolve_head(refdb).await?;
        
        // Walk commit history
        let mut visited = HashSet::new();
        let mut queue = vec![head_oid];
        
        while let Some(oid) = queue.pop() {
            if visited.contains(&oid) {
                continue;
            }
            visited.insert(oid);
            
            // Read commit
            if let Ok(commit) = Commit::read(odb, &oid).await {
                let key = format!("{}|{}", commit.author.name, commit.author.email);
                let entry = author_counts.entry(key).or_insert_with(|| {
                    (commit.author.name.clone(), commit.author.email.clone(), 0)
                });
                entry.2 += 1;
                
                // Add parents to queue
                for parent in &commit.parents {
                    queue.push(*parent);
                }
            }
        }
        
        // Convert to vec and sort by commit count
        let mut authors: Vec<AuthorStat> = author_counts
            .into_values()
            .map(|(name, email, count)| AuthorStat { name, email, commit_count: count })
            .collect();
        authors.sort_by(|a, b| b.commit_count.cmp(&a.commit_count));
        
        Ok(authors)
    }

    /// Compute file statistics from HEAD tree
    async fn compute_file_stats(&self, odb: &ObjectDatabase, refdb: &RefDatabase) -> Result<FileStats> {
        let mut stats = FileStats::default();
        
        // Get HEAD commit
        let head_oid = self.resolve_head(refdb).await?;
        let commit = Commit::read(odb, &head_oid).await?;
        
        // Walk tree recursively
        self.walk_tree_for_stats(odb, &commit.tree, &mut stats).await?;
        
        Ok(stats)
    }

    /// Recursively walk tree to count files
    async fn walk_tree_for_stats(&self, odb: &ObjectDatabase, tree_oid: &Oid, stats: &mut FileStats) -> Result<()> {
        let tree = Tree::read(odb, tree_oid).await?;
        
        for entry in tree.iter() {
            if entry.is_tree() {
                // Recursively walk subdirectory
                Box::pin(self.walk_tree_for_stats(odb, &entry.oid, stats)).await?;
            } else {
                stats.total_files += 1;
                
                // Categorize by filename extension
                let name = entry.name.to_lowercase();
                let is_media = name.ends_with(".mp4") || name.ends_with(".mov") || 
                    name.ends_with(".avi") || name.ends_with(".mkv") ||
                    name.ends_with(".mp3") || name.ends_with(".wav") ||
                    name.ends_with(".flac") || name.ends_with(".aac") ||
                    name.ends_with(".jpg") || name.ends_with(".jpeg") ||
                    name.ends_with(".png") || name.ends_with(".gif") ||
                    name.ends_with(".webp") || name.ends_with(".psd") ||
                    name.ends_with(".tiff") || name.ends_with(".raw") ||
                    name.ends_with(".blend") || name.ends_with(".fbx") ||
                    name.ends_with(".obj") || name.ends_with(".gltf");
                    
                let is_text = name.ends_with(".txt") || name.ends_with(".md") ||
                    name.ends_with(".json") || name.ends_with(".yaml") ||
                    name.ends_with(".yml") || name.ends_with(".toml") ||
                    name.ends_with(".xml") || name.ends_with(".csv") ||
                    name.ends_with(".rs") || name.ends_with(".py") ||
                    name.ends_with(".js") || name.ends_with(".ts") ||
                    name.ends_with(".html") || name.ends_with(".css");
                
                if is_media {
                    stats.media_files += 1;
                } else if is_text {
                    stats.text_files += 1;
                } else {
                    stats.other_files += 1;
                }
            }
        }
        
        Ok(())
    }

    /// Resolve HEAD to a commit OID
    async fn resolve_head(&self, refdb: &RefDatabase) -> Result<Oid> {
        let head = refdb.read("HEAD").await?;
        
        if let Some(target) = head.target {
            // Symbolic ref (e.g., refs/heads/main)
            let branch = refdb.read(&target).await?;
            branch.oid.ok_or_else(|| anyhow::anyhow!("Branch has no commit"))
        } else {
            // Detached HEAD
            head.oid.ok_or_else(|| anyhow::anyhow!("HEAD has no commit"))
        }
    }

    fn show_operation_stats(&self, storage_path: &std::path::Path) {
        use crate::progress::OperationStats;
        
        println!("{}", style("Recent Operations:").bold());
        
        // Load and display last pull
        match OperationStats::load_last_by_type(storage_path, "pull") {
            Ok(Some(stats)) => {
                let time_ago = chrono::Utc::now().signed_duration_since(stats.timestamp);
                let time_str = format_duration_ago(time_ago);
                println!("  Last pull: {} ({})", stats.summary(), time_str);
            }
            _ => println!("  Last pull: No history"),
        }
        
        // Load and display last push
        match OperationStats::load_last_by_type(storage_path, "push") {
            Ok(Some(stats)) => {
                let time_ago = chrono::Utc::now().signed_duration_since(stats.timestamp);
                let time_str = format_duration_ago(time_ago);
                println!("  Last push: {} ({})", stats.summary(), time_str);
            }
            _ => println!("  Last push: No history"),
        }
        
        // Load and display last branch switch
        match OperationStats::load_last_by_type(storage_path, "switch") {
            Ok(Some(stats)) => {
                let time_ago = chrono::Utc::now().signed_duration_since(stats.timestamp);
                let time_str = format_duration_ago(time_ago);
                println!("  Last branch switch: {} ({})", stats.summary(), time_str);
            }
            _ => println!("  Last branch switch: No history"),
        }
        
        println!();
    }

    async fn show_compression_stats(&self, storage_path: &Path) -> Result<()> {
        println!("{}", style("Compression:").bold());

        // Compute actual compression stats from storage
        let stats = self.compute_storage_stats(storage_path).await?;
        
        let total_stored = stats.loose_bytes + stats.pack_bytes + stats.chunk_bytes;
        
        if total_stored > 0 {
            // Note: We can't compute true compression ratio without original sizes
            // but we can show what's stored
            println!("  Storage used: {}", HumanBytes(total_stored));
            println!("  Algorithm: zstd/brotli (type-aware)");
            
            if stats.chunk_count > 0 {
                println!("  Chunked objects: {} ({} total)", 
                    stats.manifest_count, HumanBytes(stats.chunk_bytes));
            }
        } else {
            println!("  No compressed data yet");
        }
        
        println!();
        Ok(())
    }

    async fn output_prometheus(&self, storage_path: &Path, odb: &ObjectDatabase, refdb: &RefDatabase) -> Result<()> {
        let storage_stats = self.compute_storage_stats(storage_path).await?;
        let commit_stats = self.compute_commit_stats(odb, refdb).await.unwrap_or_default();
        
        let total_bytes = storage_stats.loose_bytes + storage_stats.pack_bytes + storage_stats.chunk_bytes;
        let total_objects = storage_stats.loose_object_count + storage_stats.chunk_count;
        
        println!("# HELP mediagit_storage_bytes_total Total storage bytes");
        println!("# TYPE mediagit_storage_bytes_total gauge");
        println!("mediagit_storage_bytes_total {}", total_bytes);
        
        println!("# HELP mediagit_objects_total Total objects stored");
        println!("# TYPE mediagit_objects_total gauge");
        println!("mediagit_objects_total {}", total_objects);
        
        println!("# HELP mediagit_commits_total Total commits");
        println!("# TYPE mediagit_commits_total gauge");
        println!("mediagit_commits_total {}", commit_stats.total_commits);
        
        println!("# HELP mediagit_packs_total Pack files");
        println!("# TYPE mediagit_packs_total gauge");
        println!("mediagit_packs_total {}", storage_stats.pack_count);
        
        println!("# HELP mediagit_chunks_total Chunks stored");
        println!("# TYPE mediagit_chunks_total gauge");
        println!("mediagit_chunks_total {}", storage_stats.chunk_count);

        Ok(())
    }

    async fn output_json(&self, storage_path: &Path, odb: &ObjectDatabase, refdb: &RefDatabase) -> Result<()> {
        let storage_stats = self.compute_storage_stats(storage_path).await?;
        let commit_stats = self.compute_commit_stats(odb, refdb).await.unwrap_or_default();
        let author_stats = self.compute_author_stats(odb, refdb).await.unwrap_or_default();
        let file_stats = self.compute_file_stats(odb, refdb).await.unwrap_or_default();
        
        // Get branch counts
        let local_branches = refdb.list("heads").await.unwrap_or_default().len();
        let remote_branches = refdb.list("remotes").await.unwrap_or_default().len();
        let tags = refdb.list("tags").await.unwrap_or_default().len();
        
        let total_bytes = storage_stats.loose_bytes + storage_stats.pack_bytes + storage_stats.chunk_bytes;
        
        let json = serde_json::json!({
            "storage": {
                "total_bytes": total_bytes,
                "loose_bytes": storage_stats.loose_bytes,
                "pack_bytes": storage_stats.pack_bytes,
                "chunk_bytes": storage_stats.chunk_bytes,
                "loose_objects": storage_stats.loose_object_count,
                "pack_files": storage_stats.pack_count,
                "chunks": storage_stats.chunk_count,
                "manifests": storage_stats.manifest_count
            },
            "commits": {
                "total": commit_stats.total_commits,
                "first_date": commit_stats.first_commit_date.map(|d| d.to_rfc3339()),
                "last_date": commit_stats.last_commit_date.map(|d| d.to_rfc3339())
            },
            "branches": {
                "local": local_branches,
                "remote": remote_branches,
                "tags": tags
            },
            "files": {
                "total": file_stats.total_files,
                "media": file_stats.media_files,
                "text": file_stats.text_files,
                "other": file_stats.other_files
            },
            "authors": author_stats.iter().map(|a| serde_json::json!({
                "name": a.name,
                "email": a.email,
                "commits": a.commit_count
            })).collect::<Vec<_>>()
        });

        println!("{}", serde_json::to_string_pretty(&json)?);

        Ok(())
    }

}
