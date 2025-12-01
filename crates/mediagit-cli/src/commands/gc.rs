use anyhow::Result;
use clap::Parser;
use console::style;
use dialoguer::Confirm;
use indicatif::{ProgressBar, ProgressStyle};
use mediagit_storage::StorageBackend;
use mediagit_versioning::{BranchManager, Commit, Oid, RefDatabase, RefType};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;
use tracing::{debug, info, warn};

/// Clean up repository and optimize storage
#[derive(Parser, Debug)]
pub struct GcCmd {
    /// Aggressive optimization (includes protected branches)
    #[arg(long)]
    pub aggressive: bool,

    /// Prune unreachable objects
    #[arg(long)]
    pub prune: Option<String>,

    /// Auto gc threshold
    #[arg(long, value_name = "NUM")]
    pub auto: bool,

    /// Show what would be done without deleting
    #[arg(long)]
    pub dry_run: bool,

    /// Skip confirmation prompts (auto-confirm deletions)
    #[arg(long, short)]
    pub yes: bool,

    /// Quiet mode (minimal output)
    #[arg(short, long)]
    pub quiet: bool,

    /// Verbose mode (detailed output)
    #[arg(short, long)]
    pub verbose: bool,
}

/// Statistics collected during GC operation
#[derive(Debug, Default)]
struct GcStats {
    /// Total objects scanned
    objects_scanned: u64,

    /// Reachable objects found
    reachable_objects: u64,

    /// Unreachable objects found
    unreachable_objects: u64,

    /// Objects deleted
    objects_deleted: u64,

    /// Space reclaimed in bytes
    bytes_reclaimed: u64,

    /// Time taken for operation
    duration_secs: f64,

    /// Errors encountered
    errors: Vec<String>,
}

impl GcStats {
    /// Format bytes into human-readable units
    fn format_bytes(bytes: u64) -> String {
        const KB: u64 = 1024;
        const MB: u64 = KB * 1024;
        const GB: u64 = MB * 1024;

        if bytes >= GB {
            format!("{:.2} GB", bytes as f64 / GB as f64)
        } else if bytes >= MB {
            format!("{:.2} MB", bytes as f64 / MB as f64)
        } else if bytes >= KB {
            format!("{:.2} KB", bytes as f64 / KB as f64)
        } else {
            format!("{} bytes", bytes)
        }
    }

    /// Print statistics summary
    fn print_summary(&self, quiet: bool) {
        if quiet {
            return;
        }

        println!("\n{}", style("=== GC Statistics ===").bold().cyan());
        println!("{:<25} {}", "Objects scanned:", style(self.objects_scanned).yellow());
        println!("{:<25} {}", "Reachable objects:", style(self.reachable_objects).green());
        println!("{:<25} {}", "Unreachable objects:", style(self.unreachable_objects).red());
        println!("{:<25} {}", "Objects deleted:", style(self.objects_deleted).red().bold());
        println!("{:<25} {}", "Space reclaimed:", style(Self::format_bytes(self.bytes_reclaimed)).yellow().bold());
        println!("{:<25} {:.2}s", "Time taken:", self.duration_secs);

        if !self.errors.is_empty() {
            println!("\n{}", style(format!("⚠ {} errors encountered", self.errors.len())).yellow());
        }
    }
}

/// Garbage collector for unreferenced objects
struct GarbageCollector {
    storage: Arc<dyn StorageBackend>,
    refdb: RefDatabase,
    branch_mgr: BranchManager,
}

impl GarbageCollector {
    fn new(storage: Arc<dyn StorageBackend>) -> Self {
        Self {
            storage: storage.clone(),
            refdb: RefDatabase::new(storage.clone()),
            branch_mgr: BranchManager::new(storage),
        }
    }

    /// Build reachability graph from all branch refs
    async fn build_reachability_set(&self) -> Result<HashSet<Oid>> {
        info!("Building reachability graph from refs");
        let mut reachable = HashSet::new();

        // Get all branches
        let branches = self.branch_mgr.list().await?;
        debug!("Found {} branches to traverse", branches.len());

        // Get HEAD ref
        if let Ok(head) = self.refdb.read("HEAD").await {
            if let Some(oid) = head.oid {
                self.traverse_commit_chain(&oid, &mut reachable).await?;
            } else if head.ref_type == RefType::Symbolic {
                // HEAD is symbolic, resolve it
                if let Ok(oid) = self.refdb.resolve("HEAD").await {
                    self.traverse_commit_chain(&oid, &mut reachable).await?;
                }
            }
        }

        // Traverse from all branch refs
        for branch in branches {
            self.traverse_commit_chain(&branch.oid, &mut reachable).await?;
        }

        // Get all tags
        let tags = self.branch_mgr.list_tags().await?;
        debug!("Found {} tags to traverse", tags.len());

        for tag_name in tags {
            if let Ok(tag_ref) = self.refdb.read(&format!("refs/tags/{}", tag_name)).await {
                if let Some(oid) = tag_ref.oid {
                    self.traverse_commit_chain(&oid, &mut reachable).await?;
                }
            }
        }

        info!("Reachability analysis complete: {} objects reachable", reachable.len());
        Ok(reachable)
    }

    /// Traverse commit → tree → blob chains
    fn traverse_commit_chain<'a>(
        &'a self,
        start_oid: &'a Oid,
        reachable: &'a mut HashSet<Oid>,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<()>> + 'a>> {
        Box::pin(async move {
            // Avoid re-traversing
            if reachable.contains(start_oid) {
                return Ok(());
            }

            reachable.insert(*start_oid);

            // Try to read commit
            let key = format!("objects/{}", start_oid.to_path());
            let data = match self.storage.get(&key).await {
                Ok(d) => d,
                Err(_) => {
                    debug!("Object {} not found or not a commit", start_oid);
                    return Ok(());
                }
            };

            // Try to deserialize as commit
            if let Ok(commit) = bincode::deserialize::<Commit>(&data) {
                // Mark tree as reachable
                reachable.insert(commit.tree);

                // Traverse parent commits
                for parent in &commit.parents {
                    self.traverse_commit_chain(parent, reachable).await?;
                }
            }

            Ok(())
        })
    }

    /// List all objects in ODB
    async fn list_all_objects(&self) -> Result<Vec<(Oid, u64)>> {
        debug!("Enumerating all objects in storage");
        let mut objects = Vec::new();

        let object_keys = self.storage.list_objects("objects/").await?;

        for key in object_keys {
            // Extract OID from key like "objects/ab/cd123..."
            if let Some(path_part) = key.strip_prefix("objects/") {
                let hex = path_part.replace('/', "");
                if hex.len() == 64 {
                    if let Ok(oid) = Oid::from_hex(&hex) {
                        // Get object size
                        let size = match self.storage.get(&key).await {
                            Ok(data) => data.len() as u64,
                            Err(_) => 0,
                        };
                        objects.push((oid, size));
                    }
                }
            }
        }

        debug!("Found {} objects in storage", objects.len());
        Ok(objects)
    }

    /// Identify unreferenced objects
    async fn find_unreachable_objects(&self, reachable: &HashSet<Oid>) -> Result<Vec<(Oid, u64)>> {
        let all_objects = self.list_all_objects().await?;

        let unreachable: Vec<(Oid, u64)> = all_objects
            .into_iter()
            .filter(|(oid, _)| !reachable.contains(oid))
            .collect();

        info!("Found {} unreachable objects", unreachable.len());
        Ok(unreachable)
    }

    /// Delete unreachable objects with safety checks
    async fn delete_objects(
        &self,
        objects: &[(Oid, u64)],
        dry_run: bool,
        verbose: bool,
    ) -> Result<GcStats> {
        let mut stats = GcStats::default();

        if objects.is_empty() {
            return Ok(stats);
        }

        let progress = if !dry_run && !verbose {
            let pb = ProgressBar::new(objects.len() as u64);
            pb.set_style(
                ProgressStyle::default_bar()
                    .template("{spinner:.green} [{bar:40.cyan/blue}] {pos}/{len} ({eta})")
                    .unwrap()
                    .progress_chars("#>-"),
            );
            Some(pb)
        } else {
            None
        };

        for (oid, size) in objects {
            let key = format!("objects/{}", oid.to_path());

            if dry_run {
                if verbose {
                    println!("[DRY RUN] Would delete: {} ({} bytes)", oid, size);
                }
                stats.objects_deleted += 1;
                stats.bytes_reclaimed += size;
            } else {
                match self.storage.delete(&key).await {
                    Ok(_) => {
                        if verbose {
                            println!("Deleted: {} ({} bytes)", oid, size);
                        }
                        stats.objects_deleted += 1;
                        stats.bytes_reclaimed += size;
                    }
                    Err(e) => {
                        let err_msg = format!("Failed to delete {}: {}", oid, e);
                        warn!("{}", err_msg);
                        stats.errors.push(err_msg);
                    }
                }
            }

            if let Some(ref pb) = progress {
                pb.inc(1);
            }
        }

        if let Some(pb) = progress {
            pb.finish_with_message("Deletion complete");
        }

        Ok(stats)
    }
}

impl GcCmd {
    pub async fn execute(&self) -> Result<()> {
        let start = Instant::now();

        // Determine repository root
        let repo_root = std::env::current_dir()?;

        if !repo_root.join(".mediagit").exists() {
            anyhow::bail!("Not a MediaGit repository (no .mediagit directory found)");
        }

        if self.dry_run && !self.quiet {
            println!("{} Running in dry-run mode (no changes will be made)", style("ℹ").blue());
        }

        // Load storage backend (assume local for now)
        let storage_path = repo_root.join(".mediagit");
        let storage: Arc<dyn StorageBackend> = Arc::new(mediagit_storage::LocalBackend::new(storage_path).await?);

        let gc = GarbageCollector::new(storage);
        let mut stats = GcStats::default();

        // Step 1: Build reachability graph
        if !self.quiet {
            println!("{} Building reachability graph from refs...", style("→").cyan());
        }
        let reachable = gc.build_reachability_set().await?;
        stats.reachable_objects = reachable.len() as u64;

        // Step 2: List all objects
        if !self.quiet {
            println!("{} Scanning object database...", style("→").cyan());
        }
        let all_objects = gc.list_all_objects().await?;
        stats.objects_scanned = all_objects.len() as u64;

        // Step 3: Identify unreachable objects
        if !self.quiet {
            println!("{} Identifying unreachable objects...", style("→").cyan());
        }
        let unreachable = gc.find_unreachable_objects(&reachable).await?;
        stats.unreachable_objects = unreachable.len() as u64;

        if unreachable.is_empty() {
            println!("{} No unreachable objects found. Repository is clean.", style("✓").green());
            return Ok(());
        }

        // Calculate total size to reclaim
        let total_size: u64 = unreachable.iter().map(|(_, size)| size).sum();

        if self.dry_run {
            println!(
                "\n{} Would delete {} objects ({} total)",
                style("ℹ").blue(),
                unreachable.len(),
                GcStats::format_bytes(total_size)
            );

            if self.verbose {
                println!("\nObjects to be deleted:");
                for (oid, size) in &unreachable {
                    println!("  {} ({} bytes)", oid, size);
                }
            }

            stats.objects_deleted = unreachable.len() as u64;
            stats.bytes_reclaimed = total_size;
        } else {
            // Confirmation required if >100 objects and not --yes flag
            if unreachable.len() > 100 && !self.yes {
                let confirmed = Confirm::new()
                    .with_prompt(format!(
                        "Delete {} unreachable objects ({})? This action cannot be undone.",
                        unreachable.len(),
                        GcStats::format_bytes(total_size)
                    ))
                    .default(false)
                    .interact()?;

                if !confirmed {
                    println!("{} GC cancelled by user", style("✗").red());
                    return Ok(());
                }
            }

            // Step 4: Delete objects
            if !self.quiet {
                println!("{} Deleting unreachable objects...", style("→").cyan());
            }
            let delete_stats = gc.delete_objects(&unreachable, false, self.verbose).await?;
            stats.objects_deleted = delete_stats.objects_deleted;
            stats.bytes_reclaimed = delete_stats.bytes_reclaimed;
            stats.errors = delete_stats.errors;

            if !self.quiet {
                println!(
                    "{} Deleted {} objects, reclaimed {}",
                    style("✓").green(),
                    stats.objects_deleted,
                    GcStats::format_bytes(stats.bytes_reclaimed)
                );
            }
        }

        stats.duration_secs = start.elapsed().as_secs_f64();
        stats.print_summary(self.quiet);

        if !stats.errors.is_empty() {
            anyhow::bail!("{} errors occurred during GC", stats.errors.len());
        }

        Ok(())
    }
}
