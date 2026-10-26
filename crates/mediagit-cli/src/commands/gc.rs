// SPDX-License-Identifier: BUSL-1.1
// Copyright (C) 2025-2026 Aswin Krishnamoorthy

use crate::progress::ProgressTracker;
use crate::repo::create_storage_backend;
use anyhow::Result;
use clap::Parser;
use console::style;
use dialoguer::Confirm;
use mediagit_storage::StorageBackend;
use mediagit_versioning::{
    BranchManager, Commit, FileMode, Index, ObjectType, Oid, RefDatabase, RefType, Reflog, Tag,
    Tree,
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

/// How recently an object must have been written to be spared by gc.
///
/// Default one hour. The window only has to exceed the realistic gap between
/// "chunk uploaded" and "the commit referencing it lands" — minutes, even for a
/// 10 GB push — so an hour is generous without letting real garbage accumulate.
/// Real systems in this class use anywhere from an hour to several days.
///
/// `MEDIAGIT_GC_GRACE_SECS` overrides it. `0` disables the protection, which is
/// the pre-VC-2 behaviour and is a deliberate data-loss risk on any repo with
/// concurrent writers.
fn grace_period() -> std::time::Duration {
    const DEFAULT_GRACE_SECS: u64 = 3600;
    let secs = std::env::var("MEDIAGIT_GC_GRACE_SECS")
        .ok()
        .and_then(|v| v.trim().parse::<u64>().ok())
        .unwrap_or(DEFAULT_GRACE_SECS);
    std::time::Duration::from_secs(secs)
}

/// Clean up repository and optimize storage
#[derive(Parser, Debug)]
pub struct GcCmd {
    /// Aggressive optimization (not yet implemented)
    #[arg(long, hide = true)]
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
            if let Ok(tag_ref) = self.refdb.read(&format!("refs/tags/{}", tag_name)).await
                && let Some(oid) = tag_ref.oid
            {
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

        // Remote-tracking branches (VC-1). `branch_mgr.list()` only walks
        // `refs/heads`, so a commit fetched but not yet merged into any local
        // branch had no root at all and was collected — silently undoing the
        // fetch and breaking the next `merge origin/<branch>`.
        match self.refdb.list("remotes").await {
            Ok(remote_refs) => {
                let mut rooted = 0usize;
                for name in &remote_refs {
                    // `list` yields ref names; resolve each to its commit.
                    if let Ok(oid) = self.refdb.resolve(name).await {
                        self.traverse_commit_chain(&oid, &mut reachable).await?;
                        rooted += 1;
                    }
                }
                if rooted > 0 {
                    debug!("Protected {} remote-tracking ref(s) from gc", rooted);
                }
            }
            Err(e) => debug!("No remote-tracking refs to protect: {}", e),
        }

        // Stashes (WT-6). Stash entries live only in `.mediagit/STASH_LIST`
        // and are never refs, so nothing rooted them — `gc` after a
        // `stash push` deleted the stashed tree and blobs, and the later
        // `stash pop` failed with "object not found".
        let stash_roots = self.stash_roots(repo_root);
        for oid in &stash_roots {
            self.traverse_commit_chain(oid, &mut reachable).await?;
        }
        if !stash_roots.is_empty() {
            debug!(
                "Protected {} stash entry/entries from gc",
                stash_roots.len()
            );
        }

        // In-progress operation state. auto-gc fires after `add` and `commit`
        // (add.rs / commit.rs), and a rebase commits on every applied step —
        // so a gc could land mid-rebase and collect `commits_remaining`, i.e.
        // the user's own not-yet-replayed commits, which no ref points at
        // once the branch has moved. Same exposure for cherry-pick, revert,
        // merge and bisect state.
        let op_roots = self.in_progress_roots(repo_root);
        for oid in &op_roots {
            self.traverse_commit_chain(oid, &mut reachable).await?;
        }
        if !op_roots.is_empty() {
            debug!(
                "Protected {} in-progress operation commit(s) from gc",
                op_roots.len()
            );
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

    /// Commit OIDs referenced by stash entries (`.mediagit/STASH_LIST`).
    ///
    /// Parsed leniently on purpose: an unreadable or malformed stash list must
    /// never cause gc to protect *less*, but it also must not abort gc. Any
    /// hex-looking `commit_oid` is taken as a root.
    fn stash_roots(&self, repo_root: &Path) -> Vec<Oid> {
        let path = repo_root.join(".mediagit").join("STASH_LIST");
        let Ok(text) = std::fs::read_to_string(&path) else {
            return Vec::new();
        };
        let Ok(value) = serde_json::from_str::<serde_json::Value>(&text) else {
            warn!("STASH_LIST is unparseable; stashed objects cannot be protected from gc");
            return Vec::new();
        };
        value
            .as_array()
            .map(|entries| {
                entries
                    .iter()
                    .filter_map(|e| e.get("commit_oid")?.as_str())
                    .filter_map(|s| Oid::from_hex(s).ok())
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Commit OIDs held by in-progress operation state files.
    ///
    /// Covers rebase (`rebase-apply/state.json`), cherry-pick, revert, merge
    /// (`MERGE_HEAD`/`ORIG_HEAD`) and bisect. These reference commits that a
    /// user can still `--continue` or `--abort` back to, but which no ref
    /// points at while the operation is mid-flight.
    ///
    /// Deliberately format-agnostic: every value that parses as a 64-char hex
    /// OID is treated as a root. A state file whose schema changes must not
    /// silently stop protecting the operation it describes.
    fn in_progress_roots(&self, repo_root: &Path) -> Vec<Oid> {
        let dir = repo_root.join(".mediagit");
        let candidates = [
            dir.join("rebase-apply").join("state.json"),
            dir.join("CHERRY_PICK_STATE"),
            dir.join("REVERT_STATE"),
            dir.join("BISECT_STATE"),
            dir.join("MERGE_HEAD"),
            dir.join("ORIG_HEAD"),
        ];

        let mut oids = Vec::new();
        for path in candidates {
            let Ok(text) = std::fs::read_to_string(&path) else {
                continue;
            };
            oids.extend(Self::scan_hex_oids(&text));
        }
        oids.sort_by_key(|o| o.to_hex());
        oids.dedup();
        oids
    }

    /// Every 64-char hex run in `text`, parsed as an OID. Handles JSON, the
    /// newline-delimited REVERT_STATE format, and bare-OID files uniformly.
    fn scan_hex_oids(text: &str) -> Vec<Oid> {
        let bytes = text.as_bytes();
        let mut out = Vec::new();
        let mut start = 0usize;
        let mut run = 0usize;
        for (i, b) in bytes.iter().enumerate() {
            if b.is_ascii_hexdigit() {
                if run == 0 {
                    start = i;
                }
                run += 1;
            } else {
                if run == 64
                    && let Ok(oid) = Oid::from_hex(&text[start..i])
                {
                    out.push(oid);
                }
                run = 0;
            }
        }
        if run == 64
            && let Ok(oid) = Oid::from_hex(&text[start..])
        {
            out.push(oid);
        }
        out
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
    async fn list_all_objects(&self) -> Result<Vec<(Oid, u64, String)>> {
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
                if hex.len() == 64
                    && let Ok(oid) = Oid::from_hex(&hex)
                {
                    // Size only — `head()` stats, `get()` would read and decompress
                    // the whole object just to call `.len()`. This scan runs on
                    // EVERY add and commit via auto-gc, so reading the full corpus
                    // here is what pegged the CPU as history grew (measured
                    // 2026-08-03: 500-commit churn went 125s -> 1620s per 100).
                    let size = self.storage.head(&key).await.ok().flatten().unwrap_or(0);
                    // Key carried so the grace-period check below can ask the
                    // backend for this object's age; it cannot be rederived
                    // from the oid without duplicating backend layout rules.
                    objects.push((oid, size, key.clone()));
                }
            }
        }

        debug!("Found {} objects in storage", objects.len());
        Ok(objects)
    }

    /// Identify unreferenced objects.
    ///
    /// Objects the grace period keeps become extra roots: everything they
    /// name is added to `reachable`, so a writer that dedups against an old
    /// unreachable object (a fresh tree naming an old blob, a fresh commit
    /// naming an old tree) does not lose it. The later manifest, chunk and
    /// delta passes read the same `reachable`, so a chunked blob named by a
    /// young tree keeps its manifest and chunks too.
    async fn find_unreachable_objects(
        &self,
        reachable: &mut HashSet<Oid>,
    ) -> Result<Vec<(Oid, u64)>> {
        let all_objects = self.list_all_objects().await?;

        let candidates: Vec<(Oid, u64, String)> = all_objects
            .into_iter()
            .filter(|(oid, _, _)| !reachable.contains(oid))
            .collect();

        let old: Vec<(Oid, u64, String)> = self
            .past_grace(candidates.clone(), |(_, _, key)| key, "object")
            .await;
        if old.len() < candidates.len() {
            let old_oids: HashSet<Oid> = old.iter().map(|(oid, _, _)| *oid).collect();
            for (oid, size, _) in &candidates {
                if !old_oids.contains(oid) {
                    self.walk_grace_kept(oid, *size, reachable).await?;
                }
            }
        }

        let unreachable: Vec<(Oid, u64)> = old
            .into_iter()
            .filter(|(oid, _, _)| !reachable.contains(oid))
            .map(|(oid, size, _)| (oid, size))
            .collect();

        info!("Found {} unreachable objects", unreachable.len());
        Ok(unreachable)
    }

    /// Add everything a grace-kept object names to `reachable` (#37).
    ///
    /// The store does not record an object's type, and postcard ignores
    /// trailing bytes, so one successful parse proves nothing: try every
    /// shape and keep the union. A blob that happens to parse only adds
    /// unknown oids, which keeps more, never less.
    async fn walk_grace_kept(
        &self,
        oid: &Oid,
        size: u64,
        reachable: &mut HashSet<Oid>,
    ) -> Result<()> {
        // ponytail: commits, trees and tags are small, but loose blobs can be
        // hundreds of MB, and reading one just to learn it is a blob is
        // waste. Objects above this stored size are treated as leaves. A tree
        // that big would hold millions of entries; raise the cap if one exists.
        const MAX_WALK_BYTES: u64 = 64 * 1024 * 1024;
        if size > MAX_WALK_BYTES {
            return Ok(());
        }
        let Ok(data) = self.odb.read(oid).await else {
            return Ok(());
        };
        if let Ok(commit) = mediagit_versioning::format::deserialize::<Commit>(&data) {
            self.traverse_tree(&commit.tree, reachable).await?;
            for parent in &commit.parents {
                self.traverse_commit_chain(parent, reachable).await?;
            }
        }
        if let Ok(tree) = mediagit_versioning::format::deserialize::<Tree>(&data) {
            for entry in tree.entries.values() {
                if entry.mode == FileMode::Directory {
                    self.traverse_tree(&entry.oid, reachable).await?;
                } else {
                    reachable.insert(entry.oid);
                }
            }
        }
        if let Ok(tag) = Tag::deserialize(&data) {
            reachable.insert(tag.target);
            match tag.target_type {
                ObjectType::Commit => self.traverse_commit_chain(&tag.target, reachable).await?,
                ObjectType::Tree => self.traverse_tree(&tag.target, reachable).await?,
                ObjectType::Blob | ObjectType::Tag => {}
            }
        }
        Ok(())
    }

    /// VC-2: prune grace period. Keeps only the candidates whose age is
    /// proven to be at least `grace_period()`; everything else is spared.
    ///
    /// Rooting is inherently racy against a concurrent writer. A push or an
    /// `add` writes chunks, deltas and manifests before the ref that reaches
    /// them, so in that window they are orphans and look collectible. The
    /// victim cannot recover: push dedups unconditionally and never
    /// re-verifies, so the loss only surfaces later as a terminal 404 on
    /// clone. Every delete list goes through here, not only loose objects.
    ///
    /// Unknown age (`modified_at` returns `None` or fails) is NOT old: the
    /// object is kept and counted in a warning. A future mtime (clock skew) is
    /// kept too. `MEDIAGIT_GC_GRACE_SECS=0` skips the check entirely.
    ///
    /// Age is queried only for candidates, never for the whole corpus: this
    /// scan runs on every add and commit via auto-gc, and statting everything
    /// is what pegged the CPU before (see `list_all_objects`).
    async fn past_grace<T>(
        &self,
        candidates: Vec<T>,
        key: impl Fn(&T) -> &str,
        what: &str,
    ) -> Vec<T> {
        let grace = grace_period();
        if grace.is_zero() {
            return candidates;
        }
        let now = std::time::SystemTime::now();
        let mut protected = 0usize;
        let mut unknown_age = 0usize;
        let mut old = Vec::with_capacity(candidates.len());

        for c in candidates {
            match self.storage.modified_at(key(&c)).await {
                Ok(Some(mtime)) => match now.duration_since(mtime) {
                    Ok(age) if age >= grace => old.push(c),
                    Ok(age) => {
                        protected += 1;
                        debug!(
                            "gc: protecting {} (age {}s < grace {}s)",
                            key(&c),
                            age.as_secs(),
                            grace.as_secs()
                        );
                    }
                    Err(_) => {
                        protected += 1;
                        debug!("gc: protecting {} (mtime in the future)", key(&c));
                    }
                },
                Ok(None) => unknown_age += 1,
                Err(e) => {
                    unknown_age += 1;
                    debug!("gc: mtime lookup failed for {}: {}", key(&c), e);
                }
            }
        }

        if protected > 0 {
            info!(
                "Protected {} recently-written {}(s) from collection (grace {}s)",
                protected,
                what,
                grace.as_secs()
            );
        }
        if unknown_age > 0 {
            warn!(
                "Kept {} unreachable {}(s) whose age could not be read: the {}s prune \
                 grace period cannot prove them old. Set MEDIAGIT_GC_GRACE_SECS=0 to \
                 collect them anyway (unsafe with concurrent writers).",
                unknown_age,
                what,
                grace.as_secs()
            );
        }
        old
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
            if let Some(hex) = key.strip_prefix("manifests/")
                && hex.len() == 64
                && let Ok(oid) = Oid::from_hex(hex)
            {
                manifests.push((oid, key));
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
                // Size only — see `list_all_objects`: stat, don't read.
                let size = self.storage.head(&key).await.ok().flatten().unwrap_or(0);
                chunks.push((key, size));
            }
        }

        debug!("Found {} chunks in storage", chunks.len());
        Ok(chunks)
    }

    /// Chunk IDs that reachable data needs: every chunk named by a reachable
    /// manifest, plus every base along each one's chunk-delta `.meta` chain.
    /// A base is named only by its delta's `.meta`, so once the version that
    /// introduced it is unreachable no manifest keeps it alive. Any read
    /// failure aborts: a chunk we could not account for would be deleted.
    async fn reachable_chunk_ids(&self, manifest_oids: &[Oid]) -> Result<HashSet<Oid>> {
        let mut ids: HashSet<Oid> = HashSet::new();
        for oid in manifest_oids {
            // Through the ODB, never `storage.get`: on a keyed repo the manifest
            // is sealed, and a raw read would fail to parse — leaving every
            // chunk it references looking unreachable, i.e. deletable.
            // Never skip an unreadable or vanished manifest: its chunks would
            // look unreachable and be deleted.
            match self.odb.get_chunk_manifest(oid).await {
                Ok(Some(manifest)) => ids.extend(manifest.chunks.iter().map(|c| c.id)),
                Ok(None) => {
                    return Err(anyhow::anyhow!(
                        "reachable chunk manifest {oid} vanished between listing and read; aborting gc without deleting anything"
                    ));
                }
                Err(e) => {
                    return Err(anyhow::anyhow!(
                        "cannot read reachable chunk manifest {oid}: {e}; aborting gc without deleting anything"
                    ));
                }
            }
        }

        // One listing tells which chunks are stored as deltas; only those
        // have a `.meta` to follow. `ids.insert` doubles as the cycle guard.
        let delta_ids: HashSet<String> = self
            .storage
            .list_objects("chunk-deltas/")
            .await?
            .iter()
            .filter_map(|k| k.strip_prefix("chunk-deltas/")?.strip_suffix(".meta"))
            .map(str::to_owned)
            .collect();
        let mut stack: Vec<Oid> = ids.iter().copied().collect();
        while let Some(id) = stack.pop() {
            let hex = id.to_hex();
            if !delta_ids.contains(&hex) {
                continue;
            }
            let base = self
                .storage
                .get(&format!("chunk-deltas/{hex}.meta"))
                .await
                .ok()
                .and_then(|b| {
                    let s = String::from_utf8(b).ok()?;
                    Oid::from_hex(s.trim().strip_prefix("base:")?.trim()).ok()
                })
                .ok_or_else(|| {
                    anyhow::anyhow!(
                        "cannot read base of reachable chunk-delta {hex}; aborting gc without deleting anything"
                    )
                })?;
            if ids.insert(base) {
                stack.push(base);
            }
        }
        Ok(ids)
    }

    /// Find orphaned manifests and chunks using the reachability set.
    ///
    /// Algorithm:
    /// 1. List all manifests → those whose blob OID is NOT reachable are orphan
    /// 2. Orphans younger than the grace period are kept, and stay LIVE
    /// 3. Read all live manifests → collect referenced chunk IDs (+ delta bases)
    /// 4. List all chunks → any chunk NOT live and past grace is orphan
    ///
    /// Step 2 matters for dedup: a push writing a new manifest can reuse an
    /// old orphan chunk instead of uploading it. Only the new manifest is
    /// young, so the chunk is protected only by being live through it.
    ///
    /// Returns (orphan_manifest_keys, orphan_chunk_keys_with_sizes,
    /// live_chunk_ids); the live set feeds `find_orphan_chunk_deltas`.
    async fn find_orphan_chunks_and_manifests(
        &self,
        reachable: &HashSet<Oid>,
    ) -> Result<(Vec<String>, Vec<(String, u64)>, HashSet<Oid>)> {
        // Step 1: Classify manifests as reachable or orphan
        let all_manifests = self.list_all_manifests().await?;
        let mut orphan_manifests = Vec::new();
        let mut live_manifest_oids = Vec::new();

        for (oid, key) in all_manifests {
            if reachable.contains(&oid) {
                live_manifest_oids.push(oid);
            } else {
                orphan_manifests.push((oid, key));
            }
        }

        debug!(
            "Manifests: {} reachable, {} orphaned",
            live_manifest_oids.len(),
            orphan_manifests.len()
        );

        // Step 2: grace. Kept orphans are live for chunk accounting.
        let orphan_count = orphan_manifests.len();
        let old_orphans = self
            .past_grace(orphan_manifests.clone(), |(_, key)| key, "manifest")
            .await;
        if old_orphans.len() < orphan_count {
            let old: HashSet<Oid> = old_orphans.iter().map(|(oid, _)| *oid).collect();
            live_manifest_oids.extend(
                orphan_manifests
                    .iter()
                    .map(|(oid, _)| *oid)
                    .filter(|oid| !old.contains(oid)),
            );
        }
        let orphan_manifest_keys: Vec<String> =
            old_orphans.into_iter().map(|(_, key)| key).collect();

        // Step 3: Collect chunk IDs live data needs (manifest chunks plus
        // their chunk-delta bases).
        let live_chunk_ids = self.reachable_chunk_ids(&live_manifest_oids).await?;
        let live_chunk_keys: HashSet<String> = live_chunk_ids
            .iter()
            .map(|id| format!("chunks/{}", id.to_hex()))
            .collect();

        debug!("Found {} live chunk references", live_chunk_keys.len());

        // Step 4: Find orphan chunks
        let all_chunks = self.list_all_chunks().await?;
        let orphan_chunks: Vec<(String, u64)> = all_chunks
            .into_iter()
            .filter(|(key, _)| !live_chunk_keys.contains(key))
            .collect();
        let orphan_chunks = self
            .past_grace(orphan_chunks, |(key, _)| key, "chunk")
            .await;

        debug!("Found {} orphan chunks", orphan_chunks.len());

        Ok((orphan_manifest_keys, orphan_chunks, live_chunk_ids))
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
            if let Some(hex) = key.strip_prefix("deltas/")
                && hex.len() == 64
                && !hex.contains('.')
                && let Ok(oid) = Oid::from_hex(hex)
            {
                let meta_key = format!("deltas/{}.meta", hex);
                // Size only — see `list_all_objects`: stat, don't read.
                let delta_size = self.storage.head(key).await.ok().flatten().unwrap_or(0);
                let meta_size = self
                    .storage
                    .head(&meta_key)
                    .await
                    .ok()
                    .flatten()
                    .unwrap_or(0);
                deltas.push((oid, key.clone(), meta_key, delta_size + meta_size));
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
            if let Some(hex) = key.strip_prefix("chunk-deltas/")
                && hex.len() == 64
                && !hex.contains('.')
            {
                let meta_key = format!("chunk-deltas/{}.meta", hex);
                // Size only — see `list_all_objects`: stat, don't read.
                let delta_size = self.storage.head(key).await.ok().flatten().unwrap_or(0);
                let meta_size = self
                    .storage
                    .head(&meta_key)
                    .await
                    .ok()
                    .flatten()
                    .unwrap_or(0);
                chunk_deltas.push((key.clone(), meta_key, delta_size + meta_size));
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
        let orphans = self
            .past_grace(orphans, |(key, _, _)| key, "blob delta")
            .await;

        info!("Found {} orphan blob-level deltas", orphans.len());
        Ok(orphans)
    }

    /// Find orphaned chunk-level deltas.
    ///
    /// A chunk delta is orphaned if its chunk ID is not live: not referenced
    /// by a reachable or grace-protected manifest, nor a delta base of one.
    /// `live_chunk_ids` comes from `find_orphan_chunks_and_manifests`, so both
    /// passes judge chunks against the same set.
    async fn find_orphan_chunk_deltas(
        &self,
        live_chunk_ids: &HashSet<Oid>,
    ) -> Result<Vec<(String, String, u64)>> {
        let reachable_chunk_ids: HashSet<String> =
            live_chunk_ids.iter().map(|id| id.to_hex()).collect();

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
        let orphans = self
            .past_grace(orphans, |(key, _, _)| key, "chunk delta")
            .await;

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
                        match self.storage.put(&key, &bytes).await {
                            Err(e) => {
                                warn!(
                                    "Failed to persist bitmap for branch '{}': {}",
                                    branch.name, e
                                );
                            }
                            _ => {
                                regenerated += 1;
                            }
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
                match self.storage.delete(&key).await {
                    Err(e) => {
                        warn!("Failed to prune orphaned bitmap {}: {}", key, e);
                    }
                    _ => {
                        pruned += 1;
                    }
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
        // UX-5: declared but never read; the struct comment already admitted
        // it was "a no-op CLI flag".
        if self.aggressive {
            anyhow::bail!("gc --aggressive is not yet implemented.");
        }
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
    let mut reachable = gc.build_reachability_set(&repo_root).await?;
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
    let unreachable = gc.find_unreachable_objects(&mut reachable).await?;
    stats.unreachable_objects = unreachable.len() as u64;

    // Even if no unreachable loose objects, still check chunks/manifests
    let has_unreachable_objects = !unreachable.is_empty();

    // Calculate total size to reclaim
    let total_size: u64 = unreachable.iter().map(|(_, size)| size).sum();

    // Pre-scan orphan chunks/manifests/deltas for both auto-gating and the
    // normal flow below. We do this once so auto mode can decide whether
    // to proceed without paying the cost twice.
    let (orphan_manifests, orphan_chunks, live_chunk_ids) =
        gc.find_orphan_chunks_and_manifests(&reachable).await?;
    let chunk_total_size: u64 = orphan_chunks.iter().map(|(_, size)| size).sum();

    let orphan_blob_deltas = gc.find_orphan_blob_deltas(&reachable).await?;
    let blob_delta_total_size: u64 = orphan_blob_deltas.iter().map(|(_, _, s)| s).sum();

    let orphan_chunk_deltas = gc.find_orphan_chunk_deltas(&live_chunk_ids).await?;
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

#[cfg(test)]
mod tests {
    use super::*;

    /// A reachable manifest that is listed but gone by the time it is read
    /// (`Ok(None)`) must abort like an unreadable one: skipping it leaves its
    /// chunks looking unreachable, i.e. deletable.
    #[tokio::test]
    async fn vanished_reachable_manifest_aborts() {
        let tmp = tempfile::TempDir::new().unwrap();
        let storage: Arc<dyn StorageBackend> = Arc::new(mediagit_storage::mock::MockBackend::new());
        let gc = GarbageCollector::new(storage, tmp.path());
        let missing = Oid::hash(b"manifest that vanished");
        let err = gc.reachable_chunk_ids(&[missing]).await.unwrap_err();
        assert!(err.to_string().contains("aborting gc"), "{err}");
    }

    /// A backend that cannot report age (every cloud backend before 0.6.1)
    /// must not have its unreachable objects deleted: unknown age is not old.
    #[tokio::test]
    async fn unknown_age_object_is_kept() {
        let tmp = tempfile::TempDir::new().unwrap();
        let mock = mediagit_storage::mock::MockBackend::new();
        let oid = Oid::hash(b"object a push just uploaded");
        mock.put(&oid.to_hex(), b"x").await.unwrap();
        let gc = GarbageCollector::new(Arc::new(mock), tmp.path());
        let doomed = gc
            .find_unreachable_objects(&mut HashSet::new())
            .await
            .unwrap();
        assert!(
            doomed.is_empty(),
            "deleted objects of unknown age: {doomed:?}"
        );
    }

    /// #37: a writer that dedups against an OLD unreachable object (a fresh
    /// tree naming an old blob, a fresh commit naming an old tree) must not
    /// lose it: the young object is kept by the grace period, so everything it
    /// names is kept too. An old object nothing names is still collected.
    #[tokio::test]
    async fn fresh_objects_keep_the_old_objects_they_reference() {
        let tmp = tempfile::TempDir::new().unwrap();
        let objects = tmp.path().join("objects");
        let local = mediagit_storage::LocalBackend::new(&objects).await.unwrap();
        let gc = GarbageCollector::new(Arc::new(local), tmp.path());

        let old_blob = gc
            .odb
            .write(ObjectType::Blob, b"old blob a fresh tree reuses")
            .await
            .unwrap();
        let old_leaf = gc
            .odb
            .write(ObjectType::Blob, b"old blob under an old tree")
            .await
            .unwrap();
        let mut old_tree = Tree::new();
        old_tree.add_entry(mediagit_versioning::TreeEntry::new(
            "leaf".into(),
            FileMode::Regular,
            old_leaf,
        ));
        let old_tree = gc
            .odb
            .write(ObjectType::Tree, &old_tree.serialize().unwrap())
            .await
            .unwrap();
        let garbage = gc
            .odb
            .write(ObjectType::Blob, b"old blob nothing names")
            .await
            .unwrap();
        for oid in [old_blob, old_leaf, old_tree, garbage] {
            age_files(&objects, &oid.to_hex());
        }

        let mut fresh_tree = Tree::new();
        fresh_tree.add_entry(mediagit_versioning::TreeEntry::new(
            "a".into(),
            FileMode::Regular,
            old_blob,
        ));
        gc.odb
            .write(ObjectType::Tree, &fresh_tree.serialize().unwrap())
            .await
            .unwrap();
        let sig = mediagit_versioning::Signature::new(
            "t".into(),
            "t@example.com".into(),
            chrono::Utc::now(),
        );
        let fresh_commit = Commit::new(old_tree, sig.clone(), sig, "fresh".into());
        gc.odb
            .write(ObjectType::Commit, &fresh_commit.serialize().unwrap())
            .await
            .unwrap();

        let mut reachable = HashSet::new();
        let doomed = gc.find_unreachable_objects(&mut reachable).await.unwrap();
        assert_eq!(
            doomed.iter().map(|(oid, _)| *oid).collect::<Vec<_>>(),
            vec![garbage],
            "only the old object nothing names may be collected"
        );
    }

    /// Push the mtime of every stored file whose name contains `needle` two
    /// hours into the past, so it is older than the default grace period.
    fn age_files(dir: &Path, needle: &str) {
        let old = std::time::SystemTime::now() - std::time::Duration::from_secs(7200);
        for e in std::fs::read_dir(dir).unwrap().flatten() {
            let p = e.path();
            if p.is_dir() {
                age_files(&p, needle);
            } else if p.to_string_lossy().contains(needle) {
                let f = std::fs::File::options().write(true).open(&p).unwrap();
                f.set_modified(old).unwrap();
            }
        }
    }

    /// A push uploads chunks and deltas before the manifest and ref that
    /// reach them. Freshly written orphans must survive the grace period on
    /// every path, not only loose objects. And a fresh manifest that dedups
    /// against an OLD orphan chunk must keep that chunk and its chunk delta
    /// alive, while an old chunk nothing names is still collected.
    #[tokio::test]
    async fn fresh_orphans_are_kept_and_keep_what_they_reference() {
        let tmp = tempfile::TempDir::new().unwrap();
        let objects = tmp.path().join("objects");
        let local = mediagit_storage::LocalBackend::new(&objects).await.unwrap();
        let fresh = Oid::hash(b"fresh").to_hex();
        let reused = Oid::hash(b"old chunk a new manifest reuses");
        let garbage = Oid::hash(b"old chunk nothing names").to_hex();
        let manifest = mediagit_versioning::ChunkManifest {
            chunks: vec![mediagit_versioning::ChunkRef {
                id: reused,
                offset: 0,
                size: 1,
                chunk_type: mediagit_versioning::ChunkType::Generic,
                codec_hint: Default::default(),
            }],
            total_size: 1,
            filename: None,
        };
        local
            .put(&format!("manifests/{fresh}"), &manifest.to_bytes().unwrap())
            .await
            .unwrap();
        let reused = reused.to_hex();
        for key in [
            format!("chunks/{fresh}"),
            format!("chunks/{reused}"),
            format!("chunks/{garbage}"),
            format!("deltas/{fresh}"),
            format!("deltas/{fresh}.meta"),
            format!("chunk-deltas/{fresh}"),
            format!("chunk-deltas/{fresh}.meta"),
            format!("chunk-deltas/{reused}"),
        ] {
            local.put(&key, b"x").await.unwrap();
        }
        age_files(&objects, &reused);
        age_files(&objects, &garbage);

        let gc = GarbageCollector::new(Arc::new(local), tmp.path());
        let none = HashSet::new();
        let (manifests, chunks, live) = gc.find_orphan_chunks_and_manifests(&none).await.unwrap();
        assert!(
            manifests.is_empty(),
            "fresh manifests deleted: {manifests:?}"
        );
        assert_eq!(
            chunks,
            vec![(format!("chunks/{garbage}"), 1)],
            "only the old unnamed chunk may be collected"
        );
        let blob_deltas = gc.find_orphan_blob_deltas(&none).await.unwrap();
        assert!(
            blob_deltas.is_empty(),
            "fresh blob deltas deleted: {blob_deltas:?}"
        );
        let chunk_deltas = gc.find_orphan_chunk_deltas(&live).await.unwrap();
        assert!(
            chunk_deltas.is_empty(),
            "live or fresh chunk deltas deleted: {chunk_deltas:?}"
        );
    }
}
