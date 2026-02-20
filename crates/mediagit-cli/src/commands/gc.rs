use anyhow::Result;
use clap::Parser;
use console::style;
use dialoguer::Confirm;
use mediagit_storage::StorageBackend;
use mediagit_versioning::{BranchManager, ChunkManifest, Commit, Oid, RefDatabase, RefType, Tree, FileMode};
use std::collections::HashSet;
use std::path::Path;
use std::sync::Arc;
use std::time::Instant;
use tracing::{debug, info, warn};
use crate::progress::ProgressTracker;
use crate::repo::create_storage_backend;

/// Clean up repository and optimize storage
#[derive(Parser, Debug)]
pub struct GcCmd {
    /// Aggressive optimization (includes protected branches)
    #[arg(long)]
    pub aggressive: bool,

    /// Skip pruning unreachable objects (by default, gc prunes)
    #[arg(long)]
    pub no_prune: bool,

    /// Auto gc threshold (run only if thresholds exceeded)
    #[arg(long)]
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

    /// Repack loose objects into pack files
    #[arg(long)]
    pub repack: bool,

    /// Maximum objects per pack file (0 = unlimited)
    #[arg(long, default_value = "0")]
    pub max_pack_size: usize,
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

    /// Orphan manifests deleted
    manifests_deleted: u64,

    /// Orphan chunks deleted
    chunks_deleted: u64,

    /// Chunk bytes reclaimed
    chunk_bytes_reclaimed: u64,

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

        if self.manifests_deleted > 0 || self.chunks_deleted > 0 {
            println!("{:<25} {}", "Manifests deleted:", style(self.manifests_deleted).red());
            println!("{:<25} {}", "Chunks deleted:", style(self.chunks_deleted).red());
            println!("{:<25} {}", "Chunk space reclaimed:", style(Self::format_bytes(self.chunk_bytes_reclaimed)).yellow().bold());
        }

        println!("{:<25} {:.2}s", "Time taken:", self.duration_secs);

        if !self.errors.is_empty() {
            println!("\n{}", style(format!("⚠ {} errors encountered", self.errors.len())).yellow());
        }
    }
}

/// Garbage collector for unreferenced objects
struct GarbageCollector {
    storage: Arc<dyn StorageBackend>,
    odb: mediagit_versioning::ObjectDatabase,
    refdb: RefDatabase,
    branch_mgr: BranchManager,
}

impl GarbageCollector {
    fn new(storage: Arc<dyn StorageBackend>, root_path: &Path) -> Self {
        // Create ODB for reading objects (including from pack files)
        let odb = mediagit_versioning::ObjectDatabase::with_smart_compression(storage.clone(), 1000);

        Self {
            storage: storage.clone(),
            odb,
            refdb: RefDatabase::new(root_path),
            branch_mgr: BranchManager::new(root_path),
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

            // Try to read commit (will check both loose objects and pack files)
            let data = match self.odb.read(start_oid).await {
                Ok(d) => d,
                Err(_) => {
                    debug!("Object {} not found or not a commit", start_oid);
                    return Ok(());
                }
            };

            // Try to deserialize as commit
            if let Ok(commit) = bincode::deserialize::<Commit>(&data) {
                // Traverse tree to mark tree + all blobs as reachable
                self.traverse_tree(&commit.tree, reachable).await?;

                // Traverse parent commits
                for parent in &commit.parents {
                    self.traverse_commit_chain(parent, reachable).await?;
                }
            }

            Ok(())
        })
    }

    /// Traverse tree to mark all blobs and subtrees as reachable
    fn traverse_tree<'a>(
        &'a self,
        tree_oid: &'a Oid,
        reachable: &'a mut HashSet<Oid>,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<()>> + 'a>> {
        Box::pin(async move {
            // Avoid re-traversing
            if reachable.contains(tree_oid) {
                return Ok(());
            }

            // Mark this tree as reachable
            reachable.insert(*tree_oid);

            // Read tree object (will check both loose objects and pack files)
            let data = match self.odb.read(tree_oid).await {
                Ok(d) => d,
                Err(_) => {
                    debug!("Tree object {} not found", tree_oid);
                    return Ok(());
                }
            };

            // Deserialize tree
            let tree = match bincode::deserialize::<Tree>(&data) {
                Ok(t) => t,
                Err(e) => {
                    debug!("Failed to deserialize tree {}: {}", tree_oid, e);
                    return Ok(());
                }
            };

            // Mark all entries as reachable and traverse subtrees
            for (_name, entry) in &tree.entries {
                if entry.mode == FileMode::Directory {
                    // Recursively traverse subtrees
                    self.traverse_tree(&entry.oid, reachable).await?;
                } else {
                    // Mark blob as reachable
                    reachable.insert(entry.oid);
                }
            }

            Ok(())
        })
    }

    /// List all objects in ODB
    async fn list_all_objects(&self) -> Result<Vec<(Oid, u64)>> {
        debug!("Enumerating all objects in storage");
        let mut objects = Vec::new();

        // List all objects (LocalBackend already operates within objects/ directory)
        let object_keys = self.storage.list_objects("").await?;

        for key in object_keys {
            // LocalBackend returns hex OIDs directly (no "objects/" prefix)
            // The key is already the hex string
            if true {
                let path_part = &key;
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
            let tracker = ProgressTracker::new(false);
            Some(tracker.object_bar("Deleting objects", objects.len() as u64))
        } else {
            None
        };

        for (oid, size) in objects {
            // Use hex OID directly - LocalBackend adds "objects/" and sharding
            let key = oid.to_hex();

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

    /// List all manifest keys in storage
    ///
    /// Manifests are stored as `manifests/{blob_oid_hex}` keys.
    /// Returns (blob_oid, storage_key) pairs.
    async fn list_all_manifests(&self) -> Result<Vec<(Oid, String)>> {
        let all_keys = self.storage.list_objects("").await?;
        let mut manifests = Vec::new();

        for key in all_keys {
            if let Some(hex) = key.strip_prefix("manifests/") {
                if hex.len() == 64 {
                    if let Ok(oid) = Oid::from_hex(hex) {
                        manifests.push((oid, key));
                    }
                }
            }
        }

        debug!("Found {} manifests in storage", manifests.len());
        Ok(manifests)
    }

    /// List all chunk keys in storage
    ///
    /// Chunks are stored as `chunks/{chunk_hash_hex}` keys.
    /// Returns (storage_key, size) pairs.
    async fn list_all_chunks(&self) -> Result<Vec<(String, u64)>> {
        let all_keys = self.storage.list_objects("").await?;
        let mut chunks = Vec::new();

        for key in all_keys {
            if key.starts_with("chunks/") {
                let size = match self.storage.get(&key).await {
                    Ok(data) => data.len() as u64,
                    Err(_) => 0,
                };
                chunks.push((key, size));
            }
        }

        debug!("Found {} chunks in storage", chunks.len());
        Ok(chunks)
    }

    /// Find orphaned manifests and chunks using the reachability set.
    ///
    /// Algorithm:
    /// 1. List all manifests → mark those whose blob OID is NOT reachable as orphan
    /// 2. Read all REACHABLE manifests → collect referenced chunk IDs
    /// 3. List all chunks → any chunk NOT referenced by a reachable manifest is orphan
    ///
    /// Returns (orphan_manifest_keys, orphan_chunk_keys_with_sizes)
    async fn find_orphan_chunks_and_manifests(
        &self,
        reachable: &HashSet<Oid>,
    ) -> Result<(Vec<String>, Vec<(String, u64)>)> {
        // Step 1: Classify manifests as reachable or orphan
        let all_manifests = self.list_all_manifests().await?;
        let mut orphan_manifest_keys = Vec::new();
        let mut reachable_manifest_oids = Vec::new();

        for (oid, key) in &all_manifests {
            if reachable.contains(oid) {
                reachable_manifest_oids.push(*oid);
            } else {
                orphan_manifest_keys.push(key.clone());
            }
        }

        debug!(
            "Manifests: {} reachable, {} orphaned",
            reachable_manifest_oids.len(),
            orphan_manifest_keys.len()
        );

        // Step 2: Read reachable manifests to collect referenced chunk IDs
        let mut reachable_chunk_keys: HashSet<String> = HashSet::new();

        for oid in &reachable_manifest_oids {
            let manifest_key = format!("manifests/{}", oid.to_hex());
            match self.storage.get(&manifest_key).await {
                Ok(data) => {
                    match bincode::deserialize::<ChunkManifest>(&data) {
                        Ok(manifest) => {
                            for chunk_ref in &manifest.chunks {
                                let chunk_key = format!("chunks/{}", chunk_ref.id.to_hex());
                                reachable_chunk_keys.insert(chunk_key);
                            }
                        }
                        Err(e) => {
                            debug!("Failed to deserialize manifest {}: {}", oid, e);
                        }
                    }
                }
                Err(e) => {
                    debug!("Failed to read manifest {}: {}", oid, e);
                }
            }
        }

        debug!("Found {} reachable chunk references", reachable_chunk_keys.len());

        // Step 3: Find orphan chunks
        let all_chunks = self.list_all_chunks().await?;
        let orphan_chunks: Vec<(String, u64)> = all_chunks
            .into_iter()
            .filter(|(key, _)| !reachable_chunk_keys.contains(key))
            .collect();

        debug!("Found {} orphan chunks", orphan_chunks.len());

        Ok((orphan_manifest_keys, orphan_chunks))
    }

    /// Delete orphaned manifests and chunks
    async fn delete_chunks_and_manifests(
        &self,
        orphan_manifests: &[String],
        orphan_chunks: &[(String, u64)],
        dry_run: bool,
        verbose: bool,
    ) -> Result<GcStats> {
        let mut stats = GcStats::default();
        let total_items = orphan_manifests.len() + orphan_chunks.len();

        if total_items == 0 {
            return Ok(stats);
        }

        let progress = if !dry_run && !verbose {
            let tracker = ProgressTracker::new(false);
            Some(tracker.object_bar("Cleaning chunks/manifests", total_items as u64))
        } else {
            None
        };

        // Delete orphan manifests
        for key in orphan_manifests {
            if dry_run {
                if verbose {
                    println!("[DRY RUN] Would delete manifest: {}", key);
                }
                stats.manifests_deleted += 1;
            } else {
                match self.storage.delete(key).await {
                    Ok(_) => {
                        if verbose {
                            println!("Deleted manifest: {}", key);
                        }
                        stats.manifests_deleted += 1;
                    }
                    Err(e) => {
                        let err_msg = format!("Failed to delete manifest {}: {}", key, e);
                        warn!("{}", err_msg);
                        stats.errors.push(err_msg);
                    }
                }
            }

            if let Some(ref pb) = progress {
                pb.inc(1);
            }
        }

        // Delete orphan chunks
        for (key, size) in orphan_chunks {
            if dry_run {
                if verbose {
                    println!("[DRY RUN] Would delete chunk: {} ({} bytes)", key, size);
                }
                stats.chunks_deleted += 1;
                stats.chunk_bytes_reclaimed += size;
            } else {
                match self.storage.delete(key).await {
                    Ok(_) => {
                        if verbose {
                            println!("Deleted chunk: {} ({} bytes)", key, size);
                        }
                        stats.chunks_deleted += 1;
                        stats.chunk_bytes_reclaimed += size;
                    }
                    Err(e) => {
                        let err_msg = format!("Failed to delete chunk {}: {}", key, e);
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
            pb.finish_with_message("Chunk cleanup complete");
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

        // Load storage backend
        let storage_path = repo_root.join(".mediagit");
        let storage = create_storage_backend(&repo_root).await?;

        let gc = GarbageCollector::new(storage.clone(), &storage_path);
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

        // Even if no unreachable loose objects, still check chunks/manifests
        let has_unreachable_objects = !unreachable.is_empty();

        // Calculate total size to reclaim
        let total_size: u64 = unreachable.iter().map(|(_, size)| size).sum();

        // Skip pruning if --no-prune flag is set
        if self.no_prune {
            if !self.quiet {
                println!(
                    "{} Found {} unreachable objects ({}) - skipping prune (--no-prune)",
                    style("ℹ").blue(),
                    unreachable.len(),
                    GcStats::format_bytes(total_size)
                );
            }
            stats.unreachable_objects = unreachable.len() as u64;
            stats.duration_secs = start.elapsed().as_secs_f64();
            stats.print_summary(self.quiet);
            return Ok(());
        }

        if has_unreachable_objects {
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
        }

        // Step 5: Chunk & manifest garbage collection
        if !self.quiet {
            println!("\n{} Scanning for orphaned chunks and manifests...", style("→").cyan());
        }

        let (orphan_manifests, orphan_chunks) = gc.find_orphan_chunks_and_manifests(&reachable).await?;

        if orphan_manifests.is_empty() && orphan_chunks.is_empty() {
            if !self.quiet {
                println!("{} No orphaned chunks or manifests found.", style("✓").green());
            }
        } else {
            let chunk_total_size: u64 = orphan_chunks.iter().map(|(_, size)| size).sum();

            if !self.quiet {
                println!(
                    "{} Found {} orphan manifests and {} orphan chunks ({})",
                    style("ℹ").blue(),
                    orphan_manifests.len(),
                    orphan_chunks.len(),
                    GcStats::format_bytes(chunk_total_size)
                );
            }

            if self.dry_run {
                if self.verbose {
                    for key in &orphan_manifests {
                        println!("  [DRY RUN] Would delete manifest: {}", key);
                    }
                    for (key, size) in &orphan_chunks {
                        println!("  [DRY RUN] Would delete chunk: {} ({} bytes)", key, size);
                    }
                }
                stats.manifests_deleted = orphan_manifests.len() as u64;
                stats.chunks_deleted = orphan_chunks.len() as u64;
                stats.chunk_bytes_reclaimed = chunk_total_size;
            } else {
                let chunk_stats = gc.delete_chunks_and_manifests(
                    &orphan_manifests,
                    &orphan_chunks,
                    false,
                    self.verbose,
                ).await?;

                stats.manifests_deleted = chunk_stats.manifests_deleted;
                stats.chunks_deleted = chunk_stats.chunks_deleted;
                stats.chunk_bytes_reclaimed = chunk_stats.chunk_bytes_reclaimed;
                stats.errors.extend(chunk_stats.errors);

                if !self.quiet {
                    println!(
                        "{} Deleted {} manifests + {} chunks, reclaimed {}",
                        style("✓").green(),
                        stats.manifests_deleted,
                        stats.chunks_deleted,
                        GcStats::format_bytes(stats.chunk_bytes_reclaimed)
                    );
                }
            }
        }

        if !has_unreachable_objects && orphan_manifests.is_empty() && orphan_chunks.is_empty() {
            println!("{} Repository is clean — no unreachable data found.", style("✓").green());
        }

        // Step 5: Repack loose objects if requested
        if self.repack {
            if !self.quiet {
                println!("\n{} Repacking loose objects...", style("→").cyan());
            }

            // Create ODB for repack operation
            use mediagit_versioning::ObjectDatabase;
            let odb = ObjectDatabase::new(storage.clone(), 1000);

            match odb.repack(self.max_pack_size, !self.dry_run).await {
                Ok(repack_stats) => {
                    if !self.quiet {
                        println!(
                            "{} Packed {} objects into pack file ({} deltas)",
                            style("✓").green(),
                            repack_stats.objects_packed,
                            repack_stats.delta_objects
                        );
                        println!(
                            "   Pack size: {}, Saved: {}",
                            GcStats::format_bytes(repack_stats.pack_size),
                            GcStats::format_bytes(repack_stats.bytes_saved)
                        );
                        if repack_stats.loose_objects_removed > 0 {
                            println!(
                                "   Removed {} loose objects",
                                repack_stats.loose_objects_removed
                            );
                        }
                    }
                }
                Err(e) => {
                    if !self.quiet {
                        println!("{} Repack failed: {}", style("✗").red(), e);
                    }
                    stats.errors.push(format!("Repack error: {}", e));
                }
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
