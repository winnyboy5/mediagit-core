// MediaGit - Git for Media Files
// Copyright (C) 2025 MediaGit Contributors
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU Affero General Public License as published
// by the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE. See the
// GNU Affero General Public License for more details.

use crate::progress::ProgressTracker;
use crate::repo::create_storage_backend;
use anyhow::Result;
use clap::Parser;
use console::style;
use dialoguer::Confirm;
use mediagit_storage::StorageBackend;
use mediagit_versioning::{
    BranchManager, ChunkManifest, Commit, FileMode, Index, ObjectType, Oid, RefDatabase, RefType,
    Reflog, Tag, Tree,
};
use std::collections::HashSet;
use std::path::Path;
use std::sync::Arc;
use std::time::Instant;
use tracing::{debug, info, warn};

/// Default horizon (in days) for protecting reflog-referenced commits from
/// gc. Entries older than this are no longer roots, matching git's
/// `gc.reflogExpire` behavior. `0` disables reflog roots entirely (old
/// behavior). Override via `MEDIAGIT_GC_REFLOG_HORIZON_DAYS`.
const DEFAULT_GC_REFLOG_HORIZON_DAYS: i64 = 90;

fn gc_reflog_horizon_days() -> i64 {
    std::env::var("MEDIAGIT_GC_REFLOG_HORIZON_DAYS")
        .ok()
        .and_then(|v| {
            v.parse::<i64>().ok().or_else(|| {
                tracing::warn!(
                    "MEDIAGIT_GC_REFLOG_HORIZON_DAYS='{}' is not a valid i64, using default {}",
                    v,
                    DEFAULT_GC_REFLOG_HORIZON_DAYS
                );
                None
            })
        })
        .unwrap_or(DEFAULT_GC_REFLOG_HORIZON_DAYS)
}

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

/// Reusable GC options struct.
///
/// Mirrors the CLI flags in `GcCmd` so that other code paths (e.g. auto-gc
/// triggered after `commit` / `pull` / `clone`) can invoke gc without going
/// through clap.
#[derive(Debug, Clone, Default)]
pub struct GcOptions {
    /// Reserved for future "aggressive" mode (currently a no-op CLI flag).
    /// Kept on the struct so the flag is wired end-to-end the moment its
    /// behavior is implemented; suppress the unused warning until then.
    #[allow(dead_code)]
    pub aggressive: bool,
    pub no_prune: bool,
    pub auto: bool,
    pub dry_run: bool,
    pub yes: bool,
    pub quiet: bool,
    pub verbose: bool,
    pub repack: bool,
    pub max_pack_size: usize,
}

impl From<&GcCmd> for GcOptions {
    fn from(cmd: &GcCmd) -> Self {
        Self {
            aggressive: cmd.aggressive,
            no_prune: cmd.no_prune,
            auto: cmd.auto,
            dry_run: cmd.dry_run,
            yes: cmd.yes,
            quiet: cmd.quiet,
            verbose: cmd.verbose,
            repack: cmd.repack,
            max_pack_size: cmd.max_pack_size,
        }
    }
}

/// Auto-gc threshold: minimum reclaimable bytes before auto-mode proceeds.
/// Below this, auto-gc exits silently (the work isn't worth the IO).
const AUTO_GC_MIN_RECLAIM_BYTES: u64 = 50 * 1024 * 1024; // 50 MiB

/// Auto-gc threshold: minimum orphan object count before auto-mode proceeds.
/// Catches "many small orphans" cases that don't trip the byte threshold.
const AUTO_GC_MIN_ORPHAN_COUNT: usize = 100;

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

    /// Orphan blob-level deltas deleted
    blob_deltas_deleted: u64,

    /// Orphan blob-level delta bytes reclaimed
    blob_delta_bytes_reclaimed: u64,

    /// Orphan chunk-level deltas deleted
    chunk_deltas_deleted: u64,

    /// Orphan chunk-level delta bytes reclaimed
    chunk_delta_bytes_reclaimed: u64,

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
        println!(
            "{:<25} {}",
            "Objects scanned:",
            style(self.objects_scanned).yellow()
        );
        println!(
            "{:<25} {}",
            "Reachable objects:",
            style(self.reachable_objects).green()
        );
        println!(
            "{:<25} {}",
            "Unreachable objects:",
            style(self.unreachable_objects).red()
        );
        println!(
            "{:<25} {}",
            "Objects deleted:",
            style(self.objects_deleted).red().bold()
        );
        println!(
            "{:<25} {}",
            "Space reclaimed:",
            style(Self::format_bytes(self.bytes_reclaimed))
                .yellow()
                .bold()
        );

        if self.manifests_deleted > 0 || self.chunks_deleted > 0 {
            println!(
                "{:<25} {}",
                "Manifests deleted:",
                style(self.manifests_deleted).red()
            );
            println!(
                "{:<25} {}",
                "Chunks deleted:",
                style(self.chunks_deleted).red()
            );
            println!(
                "{:<25} {}",
                "Chunk space reclaimed:",
                style(Self::format_bytes(self.chunk_bytes_reclaimed))
                    .yellow()
                    .bold()
            );
        }

        if self.blob_deltas_deleted > 0 {
            println!(
                "{:<25} {}",
                "Blob deltas deleted:",
                style(self.blob_deltas_deleted).red()
            );
            println!(
                "{:<25} {}",
                "Blob delta reclaimed:",
                style(Self::format_bytes(self.blob_delta_bytes_reclaimed))
                    .yellow()
                    .bold()
            );
        }

        if self.chunk_deltas_deleted > 0 {
            println!(
                "{:<25} {}",
                "Chunk deltas deleted:",
                style(self.chunk_deltas_deleted).red()
            );
            println!(
                "{:<25} {}",
                "Chunk delta reclaimed:",
                style(Self::format_bytes(self.chunk_delta_bytes_reclaimed))
                    .yellow()
                    .bold()
            );
        }

        println!("{:<25} {:.2}s", "Time taken:", self.duration_secs);

        if !self.errors.is_empty() {
            println!(
                "\n{}",
                style(format!("⚠ {} errors encountered", self.errors.len())).yellow()
            );
        }
    }
}

/// Garbage collector for unreferenced objects
struct GarbageCollector {
    storage: Arc<dyn StorageBackend>,
    odb: mediagit_versioning::ObjectDatabase,
    refdb: RefDatabase,
    branch_mgr: BranchManager,
    reflog: Reflog,
}

impl GarbageCollector {
    fn new(storage: Arc<dyn StorageBackend>, root_path: &Path) -> Self {
        // Create ODB for reading objects (including from pack files)
        let odb =
            mediagit_versioning::ObjectDatabase::with_smart_compression(storage.clone(), 1000);

        Self {
            storage: storage.clone(),
            odb,
            refdb: RefDatabase::new(root_path),
            branch_mgr: BranchManager::new(root_path),
            reflog: Reflog::new(root_path),
        }
    }

    /// Build reachability graph from all branch refs AND current index
    async fn build_reachability_set(&self, repo_root: &Path) -> Result<HashSet<Oid>> {
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
            self.traverse_commit_chain(&branch.oid, &mut reachable)
                .await?;
        }

        // Get all tags
        let tags = self.branch_mgr.list_tags().await?;
        debug!("Found {} tags to traverse", tags.len());

        for tag_name in tags {
            if let Ok(tag_ref) = self.refdb.read(&format!("refs/tags/{}", tag_name)).await {
                if let Some(oid) = tag_ref.oid {
                    // `oid` is either a lightweight tag (points straight at a
                    // commit) or an annotated tag (points at a Tag object).
                    // Protect the ref target itself either way, then walk
                    // THROUGH a Tag object to its target — otherwise a
                    // commit reachable only via an annotated tag would be
                    // collected as garbage.
                    reachable.insert(oid);
                    let tag_obj = match self.odb.read(&oid).await {
                        Ok(data) => Tag::deserialize(&data).ok(),
                        Err(_) => None,
                    };
                    match tag_obj {
                        Some(tag) => match tag.target_type {
                            ObjectType::Commit => {
                                self.traverse_commit_chain(&tag.target, &mut reachable)
                                    .await?;
                            }
                            ObjectType::Tree => {
                                self.traverse_tree(&tag.target, &mut reachable).await?;
                            }
                            ObjectType::Blob | ObjectType::Tag => {
                                // Leaf or tag-of-a-tag target: existence is
                                // all gc protects for non-commit targets.
                                reachable.insert(tag.target);
                            }
                        },
                        None => {
                            // Lightweight tag: oid IS the commit directly.
                            self.traverse_commit_chain(&oid, &mut reachable).await?;
                        }
                    }
                }
            }
        }

        // Protect reflog-referenced commits from gc, so `reflog`-based
        // recovery of a branch reset/rebase/etc. keeps working. Only
        // entries newer than the horizon are roots (matches git's
        // gc.reflogExpire); a horizon of 0 disables this entirely.
        let horizon_days = gc_reflog_horizon_days();
        if horizon_days != 0 {
            let cutoff = (horizon_days > 0)
                .then(|| chrono::Utc::now() - chrono::Duration::days(horizon_days));
            let zero_oid = Oid::from_bytes([0u8; 32]);

            let reflog_refs = self.reflog.list_refs().await.unwrap_or_default();
            let mut reflog_roots = 0usize;
            for ref_name in reflog_refs {
                let entries = match self.reflog.read(&ref_name, None).await {
                    Ok(entries) => entries,
                    Err(_) => continue,
                };
                for entry in entries {
                    // Entries with no parseable timestamp already never
                    // reach here (Reflog::read drops unparseable lines) —
                    // treat any entry we do get as in-horizon by default.
                    let in_horizon = cutoff.is_none_or(|c| entry.committer.timestamp >= c);
                    if !in_horizon {
                        continue;
                    }
                    for oid in [entry.old_oid, entry.new_oid] {
                        if oid == zero_oid {
                            continue;
                        }
                        // Skip oids the ODB no longer has (pruned by a
                        // prior gc) without erroring.
                        if self.odb.read(&oid).await.is_err() {
                            continue;
                        }
                        if !reachable.contains(&oid) {
                            reflog_roots += 1;
                        }
                        self.traverse_commit_chain(&oid, &mut reachable).await?;
                    }
                }
            }
            if reflog_roots > 0 {
                debug!(
                    "Protected {} reflog-referenced commits from gc",
                    reflog_roots
                );
            }
        }

        // Protect currently-staged index entries from gc.
        // Without this, running gc between `add` and `commit` would delete
        // the staged objects, corrupting the next commit.
        if let Ok(index) = Index::load(repo_root) {
            let index_count = index.len();
            for entry in index.entries() {
                reachable.insert(entry.oid);
            }
            if index_count > 0 {
                debug!("Protected {} staged index entries from gc", index_count);
            }
        }

        info!(
            "Reachability analysis complete: {} objects reachable",
            reachable.len()
        );
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
            if let Ok(commit) = mediagit_versioning::format::deserialize::<Commit>(&data) {
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
            let tree = match mediagit_versioning::format::deserialize::<Tree>(&data) {
                Ok(t) => t,
                Err(e) => {
                    debug!("Failed to deserialize tree {}: {}", tree_oid, e);
                    return Ok(());
                }
            };

            // Mark all entries as reachable and traverse subtrees
            for entry in tree.entries.values() {
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
                Ok(data) => match ChunkManifest::from_bytes(&data) {
                    Ok(manifest) => {
                        for chunk_ref in &manifest.chunks {
                            let chunk_key = format!("chunks/{}", chunk_ref.id.to_hex());
                            reachable_chunk_keys.insert(chunk_key);
                        }
                    }
                    Err(e) => {
                        debug!("Failed to deserialize manifest {}: {}", oid, e);
                    }
                },
                Err(e) => {
                    debug!("Failed to read manifest {}: {}", oid, e);
                }
            }
        }

        debug!(
            "Found {} reachable chunk references",
            reachable_chunk_keys.len()
        );

        // Step 3: Find orphan chunks
        let all_chunks = self.list_all_chunks().await?;
        let orphan_chunks: Vec<(String, u64)> = all_chunks
            .into_iter()
            .filter(|(key, _)| !reachable_chunk_keys.contains(key))
            .collect();

        debug!("Found {} orphan chunks", orphan_chunks.len());

        Ok((orphan_manifest_keys, orphan_chunks))
    }

    /// List all blob-level delta keys in storage
    ///
    /// Blob deltas are stored as `deltas/{oid_hex}` with metadata at `deltas/{oid_hex}.meta`.
    /// Returns (oid, delta_key, meta_key, total_size) tuples.
    async fn list_all_blob_deltas(&self) -> Result<Vec<(Oid, String, String, u64)>> {
        let all_keys = self.storage.list_objects("").await?;
        let mut deltas = Vec::new();

        for key in &all_keys {
            // Match `deltas/{64-char hex}` but not `.meta` files
            if let Some(hex) = key.strip_prefix("deltas/") {
                if hex.len() == 64 && !hex.contains('.') {
                    if let Ok(oid) = Oid::from_hex(hex) {
                        let meta_key = format!("deltas/{}.meta", hex);
                        let delta_size = match self.storage.get(key).await {
                            Ok(data) => data.len() as u64,
                            Err(_) => 0,
                        };
                        let meta_size = match self.storage.get(&meta_key).await {
                            Ok(data) => data.len() as u64,
                            Err(_) => 0,
                        };
                        deltas.push((oid, key.clone(), meta_key, delta_size + meta_size));
                    }
                }
            }
        }

        debug!("Found {} blob-level deltas in storage", deltas.len());
        Ok(deltas)
    }

    /// List all chunk-level delta keys in storage
    ///
    /// Chunk deltas are stored as `chunk-deltas/{chunk_hex}` with metadata at
    /// `chunk-deltas/{chunk_hex}.meta`.
    /// Returns (delta_key, meta_key, total_size) tuples.
    async fn list_all_chunk_delta_keys(&self) -> Result<Vec<(String, String, u64)>> {
        let all_keys = self.storage.list_objects("").await?;
        let mut chunk_deltas = Vec::new();

        for key in &all_keys {
            if let Some(hex) = key.strip_prefix("chunk-deltas/") {
                if hex.len() == 64 && !hex.contains('.') {
                    let meta_key = format!("chunk-deltas/{}.meta", hex);
                    let delta_size = match self.storage.get(key).await {
                        Ok(data) => data.len() as u64,
                        Err(_) => 0,
                    };
                    let meta_size = match self.storage.get(&meta_key).await {
                        Ok(data) => data.len() as u64,
                        Err(_) => 0,
                    };
                    chunk_deltas.push((key.clone(), meta_key, delta_size + meta_size));
                }
            }
        }

        debug!("Found {} chunk-level deltas in storage", chunk_deltas.len());
        Ok(chunk_deltas)
    }

    /// Find orphaned blob-level deltas.
    ///
    /// A blob delta is orphaned if its OID is not in the reachability set.
    async fn find_orphan_blob_deltas(
        &self,
        reachable: &HashSet<Oid>,
    ) -> Result<Vec<(String, String, u64)>> {
        let all_blob_deltas = self.list_all_blob_deltas().await?;
        let orphans: Vec<(String, String, u64)> = all_blob_deltas
            .into_iter()
            .filter(|(oid, _, _, _)| !reachable.contains(oid))
            .map(|(_, delta_key, meta_key, size)| (delta_key, meta_key, size))
            .collect();

        info!("Found {} orphan blob-level deltas", orphans.len());
        Ok(orphans)
    }

    /// Find orphaned chunk-level deltas.
    ///
    /// A chunk delta is orphaned if its chunk ID is NOT referenced by any
    /// reachable manifest. We collect reachable chunk IDs from reachable
    /// manifests and check against all chunk-deltas.
    async fn find_orphan_chunk_deltas(
        &self,
        reachable: &HashSet<Oid>,
    ) -> Result<Vec<(String, String, u64)>> {
        // Collect all chunk IDs referenced by reachable manifests
        let all_manifests = self.list_all_manifests().await?;
        let mut reachable_chunk_ids: HashSet<String> = HashSet::new();

        for (oid, _) in &all_manifests {
            if reachable.contains(oid) {
                let manifest_key = format!("manifests/{}", oid.to_hex());
                if let Ok(data) = self.storage.get(&manifest_key).await {
                    if let Ok(manifest) = ChunkManifest::from_bytes(&data) {
                        for chunk_ref in &manifest.chunks {
                            reachable_chunk_ids.insert(chunk_ref.id.to_hex());
                        }
                    }
                }
            }
        }

        // Find chunk-deltas whose chunk ID is NOT in reachable set
        let all_chunk_deltas = self.list_all_chunk_delta_keys().await?;
        let orphans: Vec<(String, String, u64)> = all_chunk_deltas
            .into_iter()
            .filter(|(key, _, _)| {
                // Extract chunk ID hex from "chunk-deltas/{hex}"
                if let Some(hex) = key.strip_prefix("chunk-deltas/") {
                    !reachable_chunk_ids.contains(hex)
                } else {
                    true // Unknown format, treat as orphan
                }
            })
            .collect();

        info!("Found {} orphan chunk-level deltas", orphans.len());
        Ok(orphans)
    }

    /// Delete orphaned delta files (both blob-level and chunk-level)
    async fn delete_deltas(
        &self,
        orphan_deltas: &[(String, String, u64)],
        dry_run: bool,
        verbose: bool,
    ) -> Result<(u64, u64)> {
        let mut deleted = 0u64;
        let mut bytes_reclaimed = 0u64;

        for (delta_key, meta_key, size) in orphan_deltas {
            if dry_run {
                if verbose {
                    println!(
                        "[DRY RUN] Would delete delta: {} ({} bytes)",
                        delta_key, size
                    );
                }
                deleted += 1;
                bytes_reclaimed += size;
            } else {
                // Delete delta data
                if let Err(e) = self.storage.delete(delta_key).await {
                    warn!("Failed to delete delta {}: {}", delta_key, e);
                }
                // Delete delta metadata
                if let Err(e) = self.storage.delete(meta_key).await {
                    // Meta file may not exist for all deltas; don't warn
                    debug!("Delta meta {} not found or delete failed: {}", meta_key, e);
                }
                if verbose {
                    println!("Deleted delta: {} ({} bytes)", delta_key, size);
                }
                deleted += 1;
                bytes_reclaimed += size;
            }
        }

        Ok((deleted, bytes_reclaimed))
    }

    /// Bitmap maintenance (M3, #2b): regenerate the reachability bitmap for
    /// every current branch tip, and prune bitmaps belonging to commits no
    /// longer in `reachable`.
    ///
    /// Bitmaps are derived data (see `mediagit_versioning::bitmap`) — a
    /// regeneration or prune failure here is never fatal to `gc`; callers
    /// log and continue. The `bitmaps/` namespace is never touched by the
    /// orphan sweeps above (they only match `chunks/`, `manifests/`,
    /// `deltas/`, `chunk-deltas/` prefixes), so this is the one place gc
    /// actively manages it.
    ///
    /// Returns (regenerated_count, pruned_count).
    async fn regenerate_and_prune_bitmaps(
        &self,
        reachable: &HashSet<Oid>,
    ) -> Result<(usize, usize)> {
        let mut regenerated = 0usize;

        let branches = self.branch_mgr.list().await?;
        for branch in &branches {
            match mediagit_versioning::ReachabilityBitmap::generate(&self.odb, branch.oid).await {
                Ok(bitmap) => match bitmap.serialize() {
                    Ok(bytes) => {
                        let key = mediagit_versioning::bitmap_key(&branch.oid);
                        if let Err(e) = self.storage.put(&key, &bytes).await {
                            warn!(
                                "Failed to persist bitmap for branch '{}': {}",
                                branch.name, e
                            );
                        } else {
                            regenerated += 1;
                        }
                    }
                    Err(e) => warn!(
                        "Failed to serialize bitmap for branch '{}': {}",
                        branch.name, e
                    ),
                },
                Err(e) => warn!(
                    "Failed to regenerate bitmap for branch '{}': {}",
                    branch.name, e
                ),
            }
        }

        let mut pruned = 0usize;
        let all_bitmap_keys = self.storage.list_objects("bitmaps").await?;
        for key in all_bitmap_keys {
            let Some(hex) = key
                .strip_prefix("bitmaps/")
                .and_then(|s| s.strip_suffix(".bitmap"))
            else {
                continue;
            };
            let Ok(oid) = Oid::from_hex(hex) else {
                continue;
            };
            if !reachable.contains(&oid) {
                if let Err(e) = self.storage.delete(&key).await {
                    warn!("Failed to prune orphaned bitmap {}: {}", key, e);
                } else {
                    pruned += 1;
                }
            }
        }

        Ok((regenerated, pruned))
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
        run_gc(&GcOptions::from(self)).await
    }
}

/// Run garbage collection with the given options.
///
/// Public so other commands (auto-gc after commit/pull/clone) can invoke it
/// without going through the clap CLI parser.
pub async fn run_gc(opts: &GcOptions) -> Result<()> {
    let start = Instant::now();

    // Determine repository root (canonicalize to match index paths)
    let repo_root = dunce::canonicalize(std::env::current_dir()?)
        .unwrap_or_else(|_| std::env::current_dir().expect("current dir"));

    if !repo_root.join(".mediagit").exists() {
        anyhow::bail!("Not a MediaGit repository (no .mediagit directory found)");
    }

    if opts.dry_run && !opts.quiet {
        println!(
            "{} Running in dry-run mode (no changes will be made)",
            style("ℹ").blue()
        );
    }

    // Load storage backend
    let storage_path = repo_root.join(".mediagit");
    let storage = create_storage_backend(&repo_root).await?;

    let gc = GarbageCollector::new(storage.clone(), &storage_path);
    let mut stats = GcStats::default();

    // Step 1: Build reachability graph
    if !opts.quiet {
        println!(
            "{} Building reachability graph from refs + index...",
            style("→").cyan()
        );
    }
    let reachable = gc.build_reachability_set(&repo_root).await?;
    stats.reachable_objects = reachable.len() as u64;

    // Step 2: List all objects
    if !opts.quiet {
        println!("{} Scanning object database...", style("→").cyan());
    }
    let all_objects = gc.list_all_objects().await?;
    stats.objects_scanned = all_objects.len() as u64;

    // Step 3: Identify unreachable objects
    if !opts.quiet {
        println!("{} Identifying unreachable objects...", style("→").cyan());
    }
    let unreachable = gc.find_unreachable_objects(&reachable).await?;
    stats.unreachable_objects = unreachable.len() as u64;

    // Even if no unreachable loose objects, still check chunks/manifests
    let has_unreachable_objects = !unreachable.is_empty();

    // Calculate total size to reclaim
    let total_size: u64 = unreachable.iter().map(|(_, size)| size).sum();

    // Pre-scan orphan chunks/manifests/deltas for both auto-gating and the
    // normal flow below. We do this once so auto mode can decide whether
    // to proceed without paying the cost twice.
    let (orphan_manifests, orphan_chunks) = gc.find_orphan_chunks_and_manifests(&reachable).await?;
    let chunk_total_size: u64 = orphan_chunks.iter().map(|(_, size)| size).sum();

    let orphan_blob_deltas = gc.find_orphan_blob_deltas(&reachable).await?;
    let blob_delta_total_size: u64 = orphan_blob_deltas.iter().map(|(_, _, s)| s).sum();

    let orphan_chunk_deltas = gc.find_orphan_chunk_deltas(&reachable).await?;
    let chunk_delta_total_size: u64 = orphan_chunk_deltas.iter().map(|(_, _, s)| s).sum();

    // Auto-mode threshold check: if reclaimable work is below threshold, exit
    // silently. Avoids paying delete-IO costs for trivial gains every commit.
    if opts.auto {
        let total_orphan_count = unreachable.len()
            + orphan_manifests.len()
            + orphan_chunks.len()
            + orphan_blob_deltas.len()
            + orphan_chunk_deltas.len();
        let total_orphan_bytes =
            total_size + chunk_total_size + blob_delta_total_size + chunk_delta_total_size;

        let exceeds_byte_threshold = total_orphan_bytes >= AUTO_GC_MIN_RECLAIM_BYTES;
        let exceeds_count_threshold = total_orphan_count >= AUTO_GC_MIN_ORPHAN_COUNT;

        if !exceeds_byte_threshold && !exceeds_count_threshold {
            if opts.verbose && !opts.quiet {
                println!(
                    "{} auto-gc: {} orphans / {} below thresholds (>= {} or >= {} objects); skipping",
                    style("ℹ").blue(),
                    total_orphan_count,
                    GcStats::format_bytes(total_orphan_bytes),
                    GcStats::format_bytes(AUTO_GC_MIN_RECLAIM_BYTES),
                    AUTO_GC_MIN_ORPHAN_COUNT,
                );
            }
            return Ok(());
        }

        if !opts.quiet {
            println!(
                "{} auto-gc: reclaiming {} orphans ({})",
                style("→").cyan(),
                total_orphan_count,
                GcStats::format_bytes(total_orphan_bytes)
            );
        }
    }

    // Skip pruning if --no-prune flag is set
    if opts.no_prune {
        if !opts.quiet {
            println!(
                "{} Found {} unreachable objects ({}) - skipping prune (--no-prune)",
                style("ℹ").blue(),
                unreachable.len(),
                GcStats::format_bytes(total_size)
            );
        }
        stats.unreachable_objects = unreachable.len() as u64;
        stats.duration_secs = start.elapsed().as_secs_f64();
        stats.print_summary(opts.quiet);
        return Ok(());
    }

    if has_unreachable_objects {
        if opts.dry_run {
            println!(
                "\n{} Would delete {} objects ({} total)",
                style("ℹ").blue(),
                unreachable.len(),
                GcStats::format_bytes(total_size)
            );

            if opts.verbose {
                println!("\nObjects to be deleted:");
                for (oid, size) in &unreachable {
                    println!("  {} ({} bytes)", oid, size);
                }
            }

            stats.objects_deleted = unreachable.len() as u64;
            stats.bytes_reclaimed = total_size;
        } else {
            // Confirmation required if >100 objects and not --yes flag.
            // Auto mode skips the prompt — it runs unattended.
            if unreachable.len() > 100 && !opts.yes && !opts.auto {
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
            if !opts.quiet {
                println!("{} Deleting unreachable objects...", style("→").cyan());
            }
            let delete_stats = gc.delete_objects(&unreachable, false, opts.verbose).await?;
            stats.objects_deleted = delete_stats.objects_deleted;
            stats.bytes_reclaimed = delete_stats.bytes_reclaimed;
            stats.errors = delete_stats.errors;

            if !opts.quiet {
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
    if !opts.quiet {
        println!(
            "\n{} Scanning for orphaned chunks and manifests...",
            style("→").cyan()
        );
    }

    if orphan_manifests.is_empty() && orphan_chunks.is_empty() {
        if !opts.quiet {
            println!(
                "{} No orphaned chunks or manifests found.",
                style("✓").green()
            );
        }
    } else {
        if !opts.quiet {
            println!(
                "{} Found {} orphan manifests and {} orphan chunks ({})",
                style("ℹ").blue(),
                orphan_manifests.len(),
                orphan_chunks.len(),
                GcStats::format_bytes(chunk_total_size)
            );
        }

        if opts.dry_run {
            if opts.verbose {
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
            let chunk_stats = gc
                .delete_chunks_and_manifests(&orphan_manifests, &orphan_chunks, false, opts.verbose)
                .await?;

            stats.manifests_deleted = chunk_stats.manifests_deleted;
            stats.chunks_deleted = chunk_stats.chunks_deleted;
            stats.chunk_bytes_reclaimed = chunk_stats.chunk_bytes_reclaimed;
            stats.errors.extend(chunk_stats.errors);

            if !opts.quiet {
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

    // Step 6: Blob-level delta garbage collection (deltas/ namespace)
    if !opts.quiet {
        println!(
            "\n{} Scanning for orphaned blob-level deltas...",
            style("→").cyan()
        );
    }

    if !orphan_blob_deltas.is_empty() {
        if !opts.quiet {
            println!(
                "{} Found {} orphan blob deltas ({})",
                style("ℹ").blue(),
                orphan_blob_deltas.len(),
                GcStats::format_bytes(blob_delta_total_size)
            );
        }

        if !opts.no_prune {
            let (del, reclaimed) = gc
                .delete_deltas(&orphan_blob_deltas, opts.dry_run, opts.verbose)
                .await?;
            stats.blob_deltas_deleted = del;
            stats.blob_delta_bytes_reclaimed = reclaimed;
        }
    }

    // Step 7: Chunk-level delta garbage collection (chunk-deltas/ namespace)
    if !opts.quiet {
        println!(
            "{} Scanning for orphaned chunk-level deltas...",
            style("→").cyan()
        );
    }

    if !orphan_chunk_deltas.is_empty() {
        if !opts.quiet {
            println!(
                "{} Found {} orphan chunk deltas ({})",
                style("ℹ").blue(),
                orphan_chunk_deltas.len(),
                GcStats::format_bytes(chunk_delta_total_size)
            );
        }

        if !opts.no_prune {
            let (del, reclaimed) = gc
                .delete_deltas(&orphan_chunk_deltas, opts.dry_run, opts.verbose)
                .await?;
            stats.chunk_deltas_deleted = del;
            stats.chunk_delta_bytes_reclaimed = reclaimed;
        }
    }

    let has_any_orphans = has_unreachable_objects
        || !orphan_manifests.is_empty()
        || !orphan_chunks.is_empty()
        || !orphan_blob_deltas.is_empty()
        || !orphan_chunk_deltas.is_empty();

    if !has_any_orphans && !opts.quiet {
        println!(
            "{} Repository is clean — no unreachable data found.",
            style("✓").green()
        );
    }

    // Step 5: Repack loose objects if requested
    if opts.repack {
        if !opts.quiet {
            println!("\n{} Repacking loose objects...", style("→").cyan());
        }

        // Create ODB for repack operation. Loose objects are written via
        // `with_smart_compression` (zstd/brotli/zlib) by add/commit, so the
        // repack reader must use the same compressor — `ObjectDatabase::new`
        // is plain-zlib-only and fails to decompress every zstd/brotli loose
        // object, silently packing 0 objects.
        use mediagit_versioning::ObjectDatabase;
        let odb = ObjectDatabase::with_smart_compression(storage.clone(), 1000);

        match odb.repack(opts.max_pack_size, !opts.dry_run).await {
            Ok(repack_stats) => {
                if !opts.quiet {
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
                if !opts.quiet {
                    println!("{} Repack failed: {}", style("✗").red(), e);
                }
                stats.errors.push(format!("Repack error: {}", e));
            }
        }
    }

    // Step 8: Bitmap maintenance (M3, #2b) — regenerate branch-tip bitmaps,
    // prune bitmaps for commits no longer reachable. Derived data: skipped
    // entirely under --dry-run (writes nothing) or when MEDIAGIT_BITMAP
    // disables it. A failure here never fails the gc run.
    if !opts.dry_run && mediagit_versioning::bitmap_enabled() {
        if !opts.quiet {
            println!("\n{} Updating reachability bitmaps...", style("→").cyan());
        }
        match gc.regenerate_and_prune_bitmaps(&reachable).await {
            Ok((regenerated, pruned)) => {
                if !opts.quiet && (regenerated > 0 || pruned > 0) {
                    println!(
                        "{} Regenerated {} bitmap(s), pruned {} orphaned bitmap(s)",
                        style("✓").green(),
                        regenerated,
                        pruned
                    );
                }
            }
            Err(e) => {
                warn!("Bitmap maintenance failed: {}", e);
            }
        }
    }

    stats.duration_secs = start.elapsed().as_secs_f64();
    stats.print_summary(opts.quiet);

    if !stats.errors.is_empty() {
        anyhow::bail!("{} errors occurred during GC", stats.errors.len());
    }

    Ok(())
}
