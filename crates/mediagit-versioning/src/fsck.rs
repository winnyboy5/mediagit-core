// SPDX-License-Identifier: BUSL-1.1
// Copyright (C) 2025-2026 Aswin Krishnamoorthy

//! File System Check (FSCK) - Repository integrity verification and repair
//!
//! This module provides comprehensive repository integrity checking:
//! - **Checksum verification**: Verify BLAKE3 hashes match object content
//! - **Reference validation**: Ensure all refs point to valid commits
//! - **Missing object detection**: Find referenced but missing objects
//! - **Commit graph validation**: Verify parent and tree relationships
//! - **Repair mode**: Automatically fix common corruption issues
//!
//! # Examples
//!
//! ```no_run
//! use mediagit_versioning::fsck::{FsckChecker, FsckOptions};
//! use mediagit_storage::LocalBackend;
//! use std::sync::Arc;
//!
//! #[tokio::main]
//! async fn main() -> anyhow::Result<()> {
//!     let storage: Arc<dyn mediagit_storage::StorageBackend> =
//!         Arc::new(LocalBackend::new("/path/to/repo").await?);
//!     let checker = FsckChecker::new(storage);
//!
//!     // Run full integrity check
//!     let options = FsckOptions::default();
//!     let report = checker.check(options).await?;
//!
//!     println!("Issues found: {}", report.total_issues());
//!     if report.has_errors() {
//!         println!("Critical errors detected!");
//!     }
//!
//!     Ok(())
//! }
//! ```

use crate::odb::ObjectDatabase;
use crate::{
    Commit, FileMode, Index, ObjectType, Oid, Ref, RefDatabase, RefType, Reflog, Tag, Tree,
};
use mediagit_storage::StorageBackend;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;
use tracing::{debug, info, warn};

/// Severity level of an FSCK issue
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum IssueSeverity {
    /// Informational only, not a problem
    Info,
    /// Warning - potential issue but repository is functional
    Warning,
    /// Error - critical issue that may cause data loss or corruption
    Error,
}

/// Category of FSCK issue
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum IssueCategory {
    /// Object checksum mismatch
    ChecksumMismatch,
    /// Referenced object is missing
    MissingObject,
    /// Reference points to non-existent object
    BrokenReference,
    /// Circular reference in commit graph
    CircularReference,
    /// Dangling object (unreferenced)
    DanglingObject,
    /// Invalid object format
    InvalidFormat,
    /// Orphaned reference
    OrphanedRef,
    /// Chunk-delta chain deeper than `MAX_DELTA_DEPTH`. The chunk is
    /// unreadable (`get_chunk` refuses to reconstruct it), which makes the
    /// repository unpushable and unclonable. Repaired by flattening.
    DeepDeltaChain,
}

/// An issue detected during FSCK
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FsckIssue {
    /// Severity level
    pub severity: IssueSeverity,

    /// Issue category
    pub category: IssueCategory,

    /// Human-readable description
    pub message: String,

    /// Object ID involved (if applicable)
    pub oid: Option<Oid>,

    /// Reference name involved (if applicable)
    pub ref_name: Option<String>,

    /// Whether this issue can be automatically repaired
    pub repairable: bool,
}

impl FsckIssue {
    /// Create a new FSCK issue
    pub fn new(severity: IssueSeverity, category: IssueCategory, message: String) -> Self {
        Self {
            severity,
            category,
            message,
            oid: None,
            ref_name: None,
            repairable: false,
        }
    }

    /// Set the OID associated with this issue
    pub fn with_oid(mut self, oid: Oid) -> Self {
        self.oid = Some(oid);
        self
    }

    /// Set the ref name associated with this issue
    pub fn with_ref(mut self, ref_name: String) -> Self {
        self.ref_name = Some(ref_name);
        self
    }

    /// Mark this issue as repairable
    pub fn repairable(mut self) -> Self {
        self.repairable = true;
        self
    }
}

/// Comprehensive FSCK report
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FsckReport {
    /// All issues found during verification
    pub issues: Vec<FsckIssue>,

    /// Total objects checked
    pub objects_checked: u64,

    /// Total references checked
    pub refs_checked: u64,

    /// Objects with integrity issues
    pub corrupted_objects: u64,

    /// Broken references
    pub broken_refs: u64,

    /// Missing objects
    pub missing_objects: u64,

    /// Dangling objects (unreferenced)
    pub dangling_objects: u64,
}

impl FsckReport {
    /// Create a new empty report
    pub fn new() -> Self {
        Self {
            issues: Vec::new(),
            objects_checked: 0,
            refs_checked: 0,
            corrupted_objects: 0,
            broken_refs: 0,
            missing_objects: 0,
            dangling_objects: 0,
        }
    }

    /// Add an issue to the report
    pub fn add_issue(&mut self, issue: FsckIssue) {
        match issue.category {
            IssueCategory::ChecksumMismatch | IssueCategory::InvalidFormat => {
                self.corrupted_objects += 1;
            }
            IssueCategory::BrokenReference | IssueCategory::OrphanedRef => {
                self.broken_refs += 1;
            }
            IssueCategory::MissingObject => {
                self.missing_objects += 1;
            }
            IssueCategory::DanglingObject => {
                self.dangling_objects += 1;
            }
            _ => {}
        }
        self.issues.push(issue);
    }

    /// Get total number of issues
    pub fn total_issues(&self) -> usize {
        self.issues.len()
    }

    /// Check if there are any critical errors
    pub fn has_errors(&self) -> bool {
        self.issues
            .iter()
            .any(|i| i.severity == IssueSeverity::Error)
    }

    /// Get issues by severity
    pub fn issues_by_severity(&self, severity: IssueSeverity) -> Vec<&FsckIssue> {
        self.issues
            .iter()
            .filter(|i| i.severity == severity)
            .collect()
    }

    /// Get repairable issues
    pub fn repairable_issues(&self) -> Vec<&FsckIssue> {
        self.issues.iter().filter(|i| i.repairable).collect()
    }
}

impl Default for FsckReport {
    fn default() -> Self {
        Self::new()
    }
}

/// Options for FSCK checking
#[derive(Debug, Clone)]
pub struct FsckOptions {
    /// Check object integrity (checksums)
    pub check_objects: bool,

    /// Validate references
    pub check_refs: bool,

    /// Check commit graph connectivity
    pub check_connectivity: bool,

    /// Detect dangling objects
    pub check_dangling: bool,

    /// Validate chunk-delta chains (cycles, broken bases, excessive depth).
    /// Cheap: reads only the tiny `chunk-deltas/*.meta` sidecars.
    pub check_chunk_deltas: bool,

    /// Maximum objects to check (0 = unlimited)
    pub max_objects: u64,

    /// Verbose output
    pub verbose: bool,
}

impl Default for FsckOptions {
    fn default() -> Self {
        Self {
            check_objects: true,
            check_refs: true,
            check_connectivity: true,
            check_dangling: false, // Expensive operation
            check_chunk_deltas: true,
            max_objects: 0,
            verbose: false,
        }
    }
}

impl FsckOptions {
    /// Create options for a full comprehensive check
    pub fn full() -> Self {
        Self {
            check_objects: true,
            check_refs: true,
            check_connectivity: true,
            check_dangling: true,
            check_chunk_deltas: true,
            max_objects: 0,
            verbose: true,
        }
    }

    /// Create options for a quick check (objects, refs and the existence walk;
    /// no dangling or chunk-delta analysis). Connectivity stays on: it only
    /// tests existence, never reads blobs, and without it a deleted object is
    /// invisible.
    pub fn quick() -> Self {
        Self {
            check_objects: true,
            check_refs: true,
            check_connectivity: true,
            check_dangling: false,
            check_chunk_deltas: false,
            max_objects: 0,
            verbose: false,
        }
    }
}

/// Every 64-char hex run in `text`, parsed as an OID.
///
/// Handles JSON, newline-delimited and bare-OID state files uniformly without
/// knowing any of their schemas. That is deliberate: a state file whose format
/// changes must not silently stop protecting the operation it describes.
///
/// Shared by `fsck`'s dangling detection and the CLI's `gc`, which must agree
/// on which objects are rooted — the two having drifted apart on exactly that
/// question is what let `fsck --repair` delete staged work.
pub fn scan_hex_oids(text: &str) -> Vec<Oid> {
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

/// Ids already handled during one graph walk. Trees are flat (full path
/// keys), so without the blob and chunk sets every commit would re-probe every
/// file it shares with its parent. Separate sets: a blob id and a chunk id can
/// coincide (a file equal to one chunk of another) yet need different probes.
#[derive(Default)]
struct Seen {
    /// Commits and trees.
    oids: HashSet<Oid>,
    blobs: HashSet<Oid>,
    chunks: HashSet<Oid>,
}

/// FSCK integrity checker
pub struct FsckChecker {
    /// Storage backend for file operations
    storage: Arc<dyn StorageBackend>,
    /// Object database for reading and verifying objects
    odb: Arc<ObjectDatabase>,
    /// Reference database for listing refs
    refdb: Option<RefDatabase>,
    /// Repository root: the directory *containing* `.mediagit`.
    ///
    /// Needed only by dangling detection, which must protect the roots that
    /// live outside the ref graph — the staging index, stashes, in-progress
    /// operation state and the reflog. Optional because `FsckChecker` is also
    /// built in server-side and test contexts that have a storage backend and
    /// nothing else; when it is absent, the working-state roots are not
    /// protected and dangling reports may include staged or stashed objects.
    repo_root: Option<PathBuf>,
}

impl FsckChecker {
    /// Create a new FSCK checker with optional RefDatabase
    pub fn new_with_refdb(storage: Arc<dyn StorageBackend>, refdb: Option<RefDatabase>) -> Self {
        // Create ODB with smart compression to handle all compression types
        let odb = ObjectDatabase::with_smart_compression(storage.clone(), 10_000_000); // 10MB cache
        Self {
            storage,
            odb: Arc::new(odb),
            refdb,
            repo_root: None,
        }
    }

    /// Point the checker at the repository root (the directory containing
    /// `.mediagit`), enabling dangling detection to protect staged, stashed,
    /// mid-operation and reflog-referenced objects.
    ///
    /// Without this, `check_dangling` sees only the ref graph and would report
    /// every such object as unreferenced.
    pub fn with_repo_root(mut self, repo_root: impl Into<PathBuf>) -> Self {
        self.repo_root = Some(repo_root.into());
        self
    }

    /// Create a new FSCK checker (backward compat, no refdb)
    pub fn new(storage: Arc<dyn StorageBackend>) -> Self {
        Self::new_with_refdb(storage, None)
    }

    /// The ODB this checker reads through.
    ///
    /// Exposed so `FsckRepair::with_odb` can reuse the *same* instance rather
    /// than building a second one: repairs that re-store content must use an
    /// identically-configured compressor, or they would write chunks the
    /// checker cannot read back.
    pub fn odb(&self) -> Arc<ObjectDatabase> {
        Arc::clone(&self.odb)
    }

    /// Run comprehensive integrity check
    ///
    /// # Arguments
    ///
    /// * `options` - FSCK options controlling what to check
    ///
    /// # Returns
    ///
    /// A comprehensive report of all issues found
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # use mediagit_versioning::fsck::{FsckChecker, FsckOptions};
    /// # use mediagit_storage::LocalBackend;
    /// # use std::sync::Arc;
    /// # #[tokio::main]
    /// # async fn main() -> anyhow::Result<()> {
    /// # let storage: Arc<dyn mediagit_storage::StorageBackend> =
    /// #     Arc::new(LocalBackend::new("/tmp").await?);
    /// let checker = FsckChecker::new(storage);
    /// let report = checker.check(FsckOptions::full()).await?;
    /// println!("Found {} issues", report.total_issues());
    /// # Ok(())
    /// # }
    /// ```
    pub async fn check(&self, options: FsckOptions) -> anyhow::Result<FsckReport> {
        info!("Starting FSCK integrity check");
        let mut report = FsckReport::new();

        // Step 1: Check object integrity
        if options.check_objects {
            info!("Checking object integrity...");
            self.check_objects(&mut report, &options).await?;
        }

        // Step 2: Validate references
        if options.check_refs {
            info!("Validating references...");
            self.check_references(&mut report).await?;
        }

        // Step 3: Check commit graph connectivity
        if options.check_connectivity {
            info!("Checking commit graph connectivity...");
            self.check_connectivity(&mut report).await?;
        }

        // Step 4: Detect dangling objects
        if options.check_dangling {
            info!("Detecting dangling objects...");
            self.check_dangling(&mut report).await?;
        }

        // Step 5: Validate chunk-delta chains
        if options.check_chunk_deltas {
            info!("Validating chunk-delta chains...");
            self.check_chunk_deltas(&mut report).await?;
        }

        info!(
            objects_checked = report.objects_checked,
            refs_checked = report.refs_checked,
            issues = report.total_issues(),
            "FSCK check complete"
        );

        Ok(report)
    }

    /// Check integrity of all objects in storage
    async fn check_objects(
        &self,
        report: &mut FsckReport,
        options: &FsckOptions,
    ) -> anyhow::Result<()> {
        debug!("Enumerating objects in storage");

        // List all objects in storage
        let objects = self.list_all_objects(report).await?;
        info!("Found {} objects to check", objects.len());

        let max_check = if options.max_objects > 0 {
            std::cmp::min(objects.len(), options.max_objects as usize)
        } else {
            objects.len()
        };

        for (idx, oid) in objects.iter().take(max_check).enumerate() {
            if options.verbose && (idx + 1) % 100 == 0 {
                debug!("Checked {}/{} objects", idx + 1, max_check);
            }

            self.verify_object(oid, report).await?;
            report.objects_checked += 1;
        }

        Ok(())
    }

    /// FS-2: which chunks of a chunked object actually fail their hash.
    ///
    /// Returns empty for a non-chunked object (no manifest), for an unreadable
    /// or unparseable manifest, and — deliberately — when every chunk verifies:
    /// in that case the corruption is in the manifest or the reassembly rather
    /// than in any one chunk, and the caller falls back to blaming the object
    /// itself. Never guesses; an empty result means "no chunk was proven bad",
    /// which is what keeps `repair` from deleting a healthy chunk.
    async fn identify_corrupt_chunks(&self, oid: &Oid) -> Vec<Oid> {
        // Through the ODB, not `storage.get`: on a keyed repo the manifest is
        // sealed, and a raw read would parse ciphertext and silently report
        // "not chunked" for every chunked object in the repository.
        let Ok(Some(manifest)) = self.odb.get_chunk_manifest(oid).await else {
            return Vec::new();
        };

        let mut corrupt = Vec::new();
        for chunk_ref in &manifest.chunks {
            // A chunk id *is* the BLAKE3 of its content, so the expected digest
            // needs no external record. `get_chunk` reconstructs deltas, so
            // this judges the bytes a reader would actually get, not the bytes
            // on disk.
            match self.odb.get_chunk(&chunk_ref.id).await {
                Ok(data) if Oid::hash(&data) == chunk_ref.id => {}
                _ => corrupt.push(chunk_ref.id),
            }
        }
        corrupt
    }

    /// Verify a single object's integrity
    async fn verify_object(&self, oid: &Oid, report: &mut FsckReport) -> anyhow::Result<()> {
        // Use ObjectDatabase's read method, which handles:
        // - Decompression (smart, zlib, or uncompressed)
        // - Checksum verification (returns error if checksum doesn't match)
        // - Chunk reconstruction if needed
        //
        // Streamed into a sink, not `read()`: a chunked blob can be 100+ GB
        // and `read()` would reassemble it in RAM (and refuse above
        // MAX_OBJECT_SIZE). Same per-chunk and whole-object verification.
        match self.odb.stream_object_to(oid, &mut tokio::io::sink()).await {
            Ok(_len) => {
                // Object read successfully, checksum verified by ODB
                debug!(oid = %oid, "Object verified successfully");
                Ok(())
            }
            Err(e) => {
                let error_msg = e.to_string();

                // Classify the error based on error message
                // Match "integrity check", not "integrity check failed": the
                // ODB phrases these both ways ("Chunk integrity check failed
                // for ..." but "base chunk {} failed integrity check: ..."),
                // and the stricter substring silently classified the second
                // form as InvalidFormat — non-repairable — so chunk corruption
                // detected via the delta-base guard could never be repaired.
                if error_msg.contains("integrity check") {
                    // FS-2: when the corruption is in a *chunk*, the failing
                    // object here is the manifest that references it. Tagging
                    // the issue with the manifest OID sent `repair` at the
                    // wrong object — and since a chunked blob has no loose
                    // file at its own OID (it lives at `manifests/<oid>`), the
                    // repair found nothing, blamed "likely packed", and
                    // reported no repair while the corrupt chunk stayed put.
                    //
                    // Identify the offending chunks from the manifest instead
                    // of parsing them out of the error text: the text is not a
                    // contract, and one read failure surfaces only the *first*
                    // bad chunk while a damaged file may have several.
                    let corrupt_chunks = self.identify_corrupt_chunks(oid).await;
                    if corrupt_chunks.is_empty() {
                        report.add_issue(
                            FsckIssue::new(
                                IssueSeverity::Error,
                                IssueCategory::ChecksumMismatch,
                                format!("Checksum mismatch: {}", e),
                            )
                            .with_oid(*oid)
                            .repairable(),
                        );
                    } else {
                        for chunk_id in corrupt_chunks {
                            report.add_issue(
                                FsckIssue::new(
                                    IssueSeverity::Error,
                                    IssueCategory::ChecksumMismatch,
                                    format!(
                                        "Chunk {} of object {} fails its integrity check",
                                        chunk_id, oid
                                    ),
                                )
                                .with_oid(chunk_id)
                                .repairable(),
                            );
                        }
                    }
                } else if error_msg.contains("not found") || error_msg.contains("No such file") {
                    // Object file is missing
                    report.add_issue(
                        FsckIssue::new(
                            IssueSeverity::Error,
                            IssueCategory::MissingObject,
                            format!("Object file missing: {}", oid),
                        )
                        .with_oid(*oid),
                    );
                } else {
                    // Other error (decompression failure, invalid format, etc.)
                    report.add_issue(
                        FsckIssue::new(
                            IssueSeverity::Error,
                            IssueCategory::InvalidFormat,
                            format!("Failed to read object {}: {}", oid, e),
                        )
                        .with_oid(*oid),
                    );
                }
                Ok(())
            }
        }
    }

    /// Validate all references
    async fn check_references(&self, report: &mut FsckReport) -> anyhow::Result<()> {
        debug!("Checking references");

        let refs = self.list_all_refs().await?;
        info!("Found {} references to check", refs.len());

        for r in refs {
            report.refs_checked += 1;

            match r.ref_type {
                RefType::Direct => {
                    if let Some(oid) = r.oid {
                        // Verify the referenced commit exists
                        if !self.object_exists(&oid).await? {
                            report.add_issue(
                                FsckIssue::new(
                                    IssueSeverity::Error,
                                    IssueCategory::BrokenReference,
                                    format!(
                                        "Reference {} points to missing commit {}",
                                        r.name, oid
                                    ),
                                )
                                .with_ref(r.name.clone())
                                .with_oid(oid)
                                .repairable(),
                            );
                        }
                    } else {
                        report.add_issue(
                            FsckIssue::new(
                                IssueSeverity::Error,
                                IssueCategory::InvalidFormat,
                                format!("Direct reference {} has no OID", r.name),
                            )
                            .with_ref(r.name.clone()),
                        );
                    }
                }
                RefType::Symbolic => {
                    if let Some(target) = &r.target {
                        // Verify the target reference exists
                        if !self.ref_exists(target).await? {
                            report.add_issue(
                                FsckIssue::new(
                                    IssueSeverity::Warning,
                                    IssueCategory::BrokenReference,
                                    format!(
                                        "Symbolic reference {} points to missing ref {}",
                                        r.name, target
                                    ),
                                )
                                .with_ref(r.name.clone())
                                .repairable(),
                            );
                        }
                    } else {
                        report.add_issue(
                            FsckIssue::new(
                                IssueSeverity::Error,
                                IssueCategory::InvalidFormat,
                                format!("Symbolic reference {} has no target", r.name),
                            )
                            .with_ref(r.name.clone()),
                        );
                    }
                }
            }
        }

        Ok(())
    }

    /// Check commit graph connectivity
    async fn check_connectivity(&self, report: &mut FsckReport) -> anyhow::Result<()> {
        debug!("Checking commit graph connectivity");

        let refs = self.list_all_refs().await?;
        let mut visited = Seen::default();
        let mut referenced_objects = HashSet::new();

        // Traverse from all branch heads
        for r in refs {
            if let Some(oid) = r.oid {
                let is_annotated_tag = r.namespace() == Some("tags")
                    && self
                        .check_and_traverse_tag(
                            &r,
                            oid,
                            &mut visited,
                            &mut referenced_objects,
                            report,
                        )
                        .await?;
                if is_annotated_tag {
                    continue;
                }
                self.traverse_commit(&oid, &mut visited, &mut referenced_objects, report)
                    .await?;
            }
        }

        info!(
            "Connectivity check complete, visited {} commits, {} total objects referenced",
            visited.oids.len(),
            referenced_objects.len()
        );

        Ok(())
    }

    /// Traverse commit graph from a commit
    fn traverse_commit<'a>(
        &'a self,
        oid: &'a Oid,
        visited: &'a mut Seen,
        referenced_objects: &'a mut HashSet<Oid>,
        report: &'a mut FsckReport,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = anyhow::Result<()>> + 'a>> {
        Box::pin(async move {
            // Detect circular references
            if visited.oids.contains(oid) {
                return Ok(());
            }
            visited.oids.insert(*oid);
            referenced_objects.insert(*oid);

            // Read via the ODB (not raw storage) so compressed/smart-compressed
            // commit objects are decompressed before deserialization — a direct
            // `self.storage.get()` here previously fed compressed bytes straight
            // into `format::deserialize`, corrupting every connectivity check on
            // a repo written through the normal (compressed) ODB write path.
            let data = match self.odb.read(oid).await {
                Ok(d) => d,
                Err(_) => {
                    report.add_issue(
                        FsckIssue::new(
                            IssueSeverity::Error,
                            IssueCategory::MissingObject,
                            format!("Commit {} is missing", oid),
                        )
                        .with_oid(*oid),
                    );
                    return Ok(());
                }
            };

            // Deserialize commit
            let commit: Commit = match crate::format::deserialize(&data) {
                Ok(c) => c,
                Err(e) => {
                    report.add_issue(
                        FsckIssue::new(
                            IssueSeverity::Error,
                            IssueCategory::InvalidFormat,
                            format!("Failed to deserialize commit {}: {}", oid, e),
                        )
                        .with_oid(*oid),
                    );
                    return Ok(());
                }
            };

            // Check tree exists, then walk INTO it. Stopping at the tree's own
            // existence was the whole gap: `check_objects` enumerates what is
            // in storage and verifies that, so it is structurally unable to
            // notice an object that is *gone*. Only a walk down from the refs
            // can, and this one stopped one level short — so deleting every
            // blob in the repo left `fsck` reporting a clean bill of health.
            self.check_commit_tree(oid, &commit.tree, visited, referenced_objects, report)
                .await?;

            // Traverse parent commits
            for parent in &commit.parents {
                referenced_objects.insert(*parent);
                self.traverse_commit(parent, visited, referenced_objects, report)
                    .await?;
            }

            Ok(())
        })
    }

    /// Tree-existence step shared by [`Self::traverse_commit`] and
    /// [`Self::check_commits`].
    async fn check_commit_tree(
        &self,
        commit_oid: &Oid,
        tree: &Oid,
        visited: &mut Seen,
        referenced_objects: &mut HashSet<Oid>,
        report: &mut FsckReport,
    ) -> anyhow::Result<()> {
        referenced_objects.insert(*tree);
        if !self.object_exists(tree).await? {
            report.add_issue(
                FsckIssue::new(
                    IssueSeverity::Error,
                    IssueCategory::MissingObject,
                    format!("Commit {} references missing tree {}", commit_oid, tree),
                )
                .with_oid(*commit_oid),
            );
        } else {
            self.check_tree_entries(tree, visited, referenced_objects, report)
                .await?;
        }
        Ok(())
    }

    /// Existence-check the contents (tree, blobs, manifests, chunks) of the
    /// given commits only, without walking parents. For callers that already
    /// selected a commit range (`verify <COMMIT>` / `--start` / `--end`).
    /// Commits that are themselves missing or unparseable are skipped: the
    /// caller reports those.
    pub async fn check_commits(&self, commits: &[Oid]) -> anyhow::Result<FsckReport> {
        let mut report = FsckReport::new();
        let mut visited = Seen::default();
        let mut referenced = HashSet::new();
        for oid in commits {
            let Ok(data) = self.odb.read(oid).await else {
                continue;
            };
            let Ok(commit) = crate::format::deserialize::<Commit>(&data) else {
                continue;
            };
            self.check_commit_tree(
                oid,
                &commit.tree,
                &mut visited,
                &mut referenced,
                &mut report,
            )
            .await?;
        }
        Ok(report)
    }

    /// Verify every object a reachable tree points at is actually present.
    ///
    /// Existence is tested with [`ObjectDatabase::exists`] (loose | manifest |
    /// pack), deliberately NOT `read`: a chunked blob's `read` reassembles the
    /// entire file, which would turn a default-on connectivity check into a
    /// full-repo download on exactly the multi-GB media repos this VCS exists
    /// for. Whether the bytes are *correct* stays `check_objects`' job — it
    /// already reads and BLAKE3-verifies every object it enumerates, including
    /// per-chunk verification for manifests. This walk answers only the
    /// question enumeration cannot: is anything referenced but absent —
    /// blobs, trees, manifests, and (via the manifest) every chunk, so a
    /// missing chunk is reported even when `check_objects` is off or
    /// truncated by `max_objects`.
    ///
    /// Shares `visited` with [`Self::traverse_commit`] so trees reused across
    /// commits are walked once; OIDs are content hashes, so a tree OID can
    /// never collide with a commit OID in that set.
    fn check_tree_entries<'a>(
        &'a self,
        tree_oid: &'a Oid,
        visited: &'a mut Seen,
        referenced_objects: &'a mut HashSet<Oid>,
        report: &'a mut FsckReport,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = anyhow::Result<()>> + 'a>> {
        Box::pin(async move {
            if !visited.oids.insert(*tree_oid) {
                return Ok(());
            }

            // The caller already established this tree exists; an unreadable
            // or unparseable one is corruption, which `check_objects` reports
            // against the object itself. Nothing to add here.
            let Ok(data) = self.odb.read(tree_oid).await else {
                return Ok(());
            };
            let Ok(tree) = crate::format::deserialize::<Tree>(&data) else {
                return Ok(());
            };

            for entry in tree.entries.values() {
                referenced_objects.insert(entry.oid);

                // Already judged via an earlier commit or path: skip before
                // any I/O (the first verdict, including "missing", stands).
                if !visited.blobs.insert(entry.oid) {
                    continue;
                }

                // `?`: a storage error aborts the walk on purpose; an unknown
                // answer must never be read as "present" or as "missing".
                // `object_exists`, not `odb.exists`: the latter does not know
                // delta-stored (`deltas/<oid>`) blobs.
                if !self.object_exists(&entry.oid).await? {
                    let issue = FsckIssue::new(
                        IssueSeverity::Error,
                        IssueCategory::MissingObject,
                        format!("Tree {} references missing object {}", tree_oid, entry.oid),
                    )
                    .with_oid(entry.oid);
                    // A missing file can be restored; a missing subtree cannot.
                    report.add_issue(if entry.mode == FileMode::Directory {
                        issue
                    } else {
                        issue.repairable()
                    });
                    continue;
                }

                // A chunked blob's manifest can exist while a chunk is gone.
                // Metadata only: read the (small) manifest, then test each
                // chunk id's existence; no chunk bytes are read. Not left to
                // `check_objects`, which `max_objects` can truncate. An
                // unreadable manifest is corruption, reported there.
                if let Ok(Some(manifest)) = self.odb.get_chunk_manifest(&entry.oid).await {
                    for chunk in &manifest.chunks {
                        referenced_objects.insert(chunk.id);
                        if !visited.chunks.insert(chunk.id) {
                            continue;
                        }
                        // `?` aborts on a storage error, intentionally (see above).
                        // `chunk_exists` covers loose, delta-sidecar and packed chunks.
                        if !self.odb.chunk_exists(&chunk.id).await? {
                            report.add_issue(
                                FsckIssue::new(
                                    IssueSeverity::Error,
                                    IssueCategory::MissingObject,
                                    format!(
                                        "Blob {} references missing chunk {}",
                                        entry.oid, chunk.id
                                    ),
                                )
                                .with_oid(chunk.id)
                                .repairable(),
                            );
                        }
                    }
                }

                // Trees in this VCS are normally single-level (entries keyed
                // by full relative path), but `FileMode::Directory` entries
                // are produced on some paths (see the server's tree builder),
                // and `gc`'s walker recurses through them — so this one must
                // too, or fsck and gc would disagree about what is reachable.
                if entry.mode == FileMode::Directory {
                    self.check_tree_entries(&entry.oid, visited, referenced_objects, report)
                        .await?;
                }
            }

            Ok(())
        })
    }

    /// Validate and traverse a `refs/tags/*` ref's target.
    ///
    /// Returns `Ok(true)` if `oid` deserialized as a [`Tag`] object
    /// (annotated tag) — in that case this method already did all the
    /// necessary work (structure validation + walking through to the
    /// target) and the caller must not also call `traverse_commit(oid, ..)`
    /// on it. Returns `Ok(false)` for a lightweight tag (oid is a commit
    /// directly), leaving the caller to fall back to `traverse_commit`.
    async fn check_and_traverse_tag(
        &self,
        r: &Ref,
        oid: Oid,
        visited: &mut Seen,
        referenced_objects: &mut HashSet<Oid>,
        report: &mut FsckReport,
    ) -> anyhow::Result<bool> {
        // Via the ODB, not raw storage, so a compressed Tag object decompresses
        // before deserialization (same fix as traverse_commit below).
        let Ok(data) = self.odb.read(&oid).await else {
            // Missing object: let the generic traverse_commit path below
            // report it via its own "missing" issue for a uniform message.
            return Ok(false);
        };
        let Ok(tag) = Tag::deserialize(&data) else {
            // Not a Tag object -> lightweight tag, oid is a commit.
            return Ok(false);
        };

        visited.oids.insert(oid);
        referenced_objects.insert(oid);

        // Validate: target exists.
        if !self.object_exists(&tag.target).await? {
            report.add_issue(
                FsckIssue::new(
                    IssueSeverity::Error,
                    IssueCategory::BrokenReference,
                    format!("Tag {} ({}) target {} is missing", r.name, oid, tag.target),
                )
                .with_ref(r.name.clone())
                .with_oid(oid),
            );
            return Ok(true);
        }
        referenced_objects.insert(tag.target);

        // Validate: declared target_type matches the actual target object.
        if let Ok(target_data) = self.odb.read(&tag.target).await {
            let type_matches = match tag.target_type {
                ObjectType::Commit => Commit::deserialize(&target_data).is_ok(),
                ObjectType::Tree => Tree::deserialize(&target_data).is_ok(),
                ObjectType::Tag => Tag::deserialize(&target_data).is_ok(),
                // Any bytes are a valid blob; nothing to falsify.
                ObjectType::Blob => true,
            };
            if !type_matches {
                report.add_issue(
                    FsckIssue::new(
                        IssueSeverity::Error,
                        IssueCategory::InvalidFormat,
                        format!(
                            "Tag {} ({}) declares target_type {} but target {} does not match",
                            r.name, oid, tag.target_type, tag.target
                        ),
                    )
                    .with_ref(r.name.clone())
                    .with_oid(oid),
                );
            }
        }

        // Walk through to the target so its own closure is checked too.
        match tag.target_type {
            ObjectType::Commit => {
                self.traverse_commit(&tag.target, visited, referenced_objects, report)
                    .await?;
            }
            // Tree/Blob/Tag targets: existence and type already validated
            // above; check_connectivity's scope (matching its pre-existing
            // behavior for other refs) only descends into commit graphs.
            ObjectType::Tree | ObjectType::Blob | ObjectType::Tag => {}
        }

        Ok(true)
    }

    /// Validate chunk-delta chains: every `chunk-deltas/<id>.meta` must lead,
    /// via `base:` links, to a full chunk within `MAX_DELTA_DEPTH` hops —
    /// never to a cycle (unreconstructable, data-loss; see the 2026-07-07
    /// A→B→C→A incident) and never to a missing base.
    ///
    /// One `list_objects` pass loads all metas into a map; chains are then
    /// walked in memory, so cost is O(number of chunk deltas), independent of
    /// chunk sizes.
    async fn check_chunk_deltas(&self, report: &mut FsckReport) -> anyhow::Result<()> {
        let keys = self.storage.list_objects("chunk-deltas/").await?;

        // delta chunk id -> base chunk id
        let mut bases: std::collections::HashMap<Oid, Oid> = std::collections::HashMap::new();
        for key in &keys {
            let Some(hex) = key
                .strip_prefix("chunk-deltas/")
                .and_then(|k| k.strip_suffix(".meta"))
            else {
                continue;
            };
            let Ok(id) = Oid::from_hex(hex) else {
                report.add_issue(FsckIssue::new(
                    IssueSeverity::Warning,
                    IssueCategory::InvalidFormat,
                    format!("chunk-delta meta with unparseable id: {}", key),
                ));
                continue;
            };
            let Ok(bytes) = self.storage.get(key).await else {
                continue;
            };
            let base = std::str::from_utf8(&bytes)
                .ok()
                .and_then(|s| s.trim().strip_prefix("base:"))
                // Tolerate the extended "base:<hex>:depth:<n>" form.
                .map(|rest| rest.split(':').next().unwrap_or(rest).trim())
                .and_then(|hex| Oid::from_hex(hex).ok());
            match base {
                Some(b) => {
                    bases.insert(id, b);
                }
                None => {
                    report.add_issue(
                        FsckIssue::new(
                            IssueSeverity::Error,
                            IssueCategory::InvalidFormat,
                            format!("chunk-delta meta is malformed: {}", key),
                        )
                        .with_oid(id),
                    );
                }
            }
        }

        for &id in bases.keys() {
            let mut visited = std::collections::HashSet::new();
            let mut current = id;
            loop {
                if !visited.insert(current) {
                    report.add_issue(
                        FsckIssue::new(
                            IssueSeverity::Error,
                            IssueCategory::CircularReference,
                            format!(
                                "chunk-delta cycle: chain from {} revisits {} — chunks on this \
                                 loop have no full copy and cannot be reconstructed",
                                id.to_hex(),
                                current.to_hex()
                            ),
                        )
                        .with_oid(id),
                    );
                    break;
                }
                match bases.get(&current) {
                    Some(&next) => current = next,
                    None => {
                        // Chain terminates: base must exist as a full chunk.
                        // Routed through the ODB's `chunk_exists` (pack-aware)
                        // rather than raw `storage.exists`, which only sees
                        // loose chunks and false-positives "missing base
                        // chunk" after `gc --repack` moves chunks into a
                        // pack (QA-005).
                        if !self.odb.chunk_exists(&current).await.unwrap_or(false) {
                            report.add_issue(
                                FsckIssue::new(
                                    IssueSeverity::Error,
                                    IssueCategory::MissingObject,
                                    format!(
                                        "chunk-delta chain from {} ends at missing base chunk {}",
                                        id.to_hex(),
                                        current.to_hex()
                                    ),
                                )
                                .with_oid(id),
                            );
                        } else if visited.len().saturating_sub(1)
                            > crate::odb::MAX_DELTA_DEPTH as usize
                        {
                            // FS-1: compare *edges*, not nodes. `visited`
                            // holds every delta node **and** the terminal
                            // full chunk, so its length is depth + 1. The
                            // bare `visited.len() > MAX_DELTA_DEPTH` fired at
                            // depth 10 — precisely the deepest chain
                            // `resolve_delta_base` is allowed to build and
                            // one `get_chunk` reconstructs without complaint.
                            // Every such repo reported "11 hops deep (max
                            // 10)" for chains that were correct, which is how
                            // a healthy repo produced a screenful of warnings
                            // that no repair could ever clear.
                            //
                            // Repairable: `--repair` flattens the chain by
                            // reconstructing the chunk and re-storing it in
                            // full. Until that landed this was a dead-end
                            // warning on a repo that could no longer be
                            // pushed or cloned, because `get_chunk` refuses
                            // to reconstruct a chain this deep.
                            report.add_issue(
                                FsckIssue::new(
                                    IssueSeverity::Warning,
                                    IssueCategory::DeepDeltaChain,
                                    format!(
                                        "chunk-delta chain from {} is {} hops deep (max {})",
                                        id.to_hex(),
                                        visited.len().saturating_sub(1),
                                        crate::odb::MAX_DELTA_DEPTH
                                    ),
                                )
                                .with_oid(id)
                                .repairable(),
                            );
                        }
                        break;
                    }
                }
            }
        }

        debug!(
            chunk_deltas = bases.len(),
            "chunk-delta chain check complete"
        );
        Ok(())
    }

    /// Detect dangling (unreferenced) objects
    async fn check_dangling(&self, report: &mut FsckReport) -> anyhow::Result<()> {
        debug!("Detecting dangling objects");

        // Get all objects
        let all_objects = self.list_all_objects(report).await?;

        // Get all referenced objects
        let refs = self.list_all_refs().await?;
        let mut referenced = HashSet::new();

        for r in refs {
            if let Some(oid) = r.oid {
                let mut visited = HashSet::new();
                self.collect_referenced_objects(&oid, &mut visited, &mut referenced, report)
                    .await?;
            }
        }

        // Roots that live outside the ref graph. Without these, an object
        // that is staged, stashed or held by an in-progress rebase is
        // "unreferenced by any commit" — true, and exactly the wrong
        // conclusion, because `--repair` then deletes the user's live work.
        self.protect_working_roots(&mut referenced, report).await?;

        // Informational only: `gc` owns unreferenced objects, `--repair`
        // never touches them.
        for oid in all_objects {
            if !referenced.contains(&oid) {
                report.add_issue(
                    FsckIssue::new(
                        IssueSeverity::Info,
                        IssueCategory::DanglingObject,
                        format!("Object {} is not referenced by any commit", oid),
                    )
                    .with_oid(oid),
                );
            }
        }

        Ok(())
    }

    /// Protect objects rooted outside the ref graph: the staging index,
    /// stashes, in-progress operation state and the reflog.
    ///
    /// `gc` roots reachability from all of these (see `build_reachability_set`
    /// in the CLI's `gc` command, which added each one in response to a
    /// separate data-loss report). `check_dangling` rooted from refs alone, so
    /// every staged, stashed or mid-rebase object was reported dangling and
    /// `fsck --full --repair` deleted it.
    ///
    /// fsck is deliberately *more* conservative than gc here: it protects the
    /// entire reflog rather than gc's `gc.reflogExpire` horizon. The two
    /// errors are not symmetric — protecting something gc would collect just
    /// leaves an object for the next gc to remove, while condemning something
    /// gc protects destroys work the user can still reach.
    async fn protect_working_roots(
        &self,
        referenced: &mut HashSet<Oid>,
        report: &mut FsckReport,
    ) -> anyhow::Result<()> {
        let Some(repo_root) = self.repo_root.clone() else {
            return Ok(());
        };

        // Staged blobs. Inserted directly rather than routed through
        // `collect_referenced_objects`, which would `odb.read` each one —
        // and reading a chunked blob reassembles the whole file to learn
        // nothing that the manifest lookup below does not already say.
        if let Ok(index) = Index::load(&repo_root) {
            for entry in index.entries() {
                if referenced.insert(entry.oid)
                    && let Ok(Some(manifest)) = self.odb.get_chunk_manifest(&entry.oid).await
                {
                    for chunk in &manifest.chunks {
                        referenced.insert(chunk.id);
                    }
                }
            }
        }

        // Commit-shaped roots get their full closure walked.
        for oid in Self::working_root_commits(&repo_root).await {
            let mut visited = HashSet::new();
            self.collect_referenced_objects(&oid, &mut visited, referenced, report)
                .await?;
        }

        Ok(())
    }

    /// Commit OIDs held by stashes, in-progress operation state and the
    /// reflog.
    ///
    /// State files are scanned for 64-char hex runs rather than parsed:
    /// deliberately format-agnostic, so a state file whose schema changes
    /// cannot silently stop protecting the operation it describes. Mirrors
    /// the same choice in the CLI's `gc`.
    async fn working_root_commits(repo_root: &Path) -> Vec<Oid> {
        let mediagit = repo_root.join(".mediagit");
        let mut roots = Vec::new();

        for name in [
            "STASH_LIST",
            "rebase-apply/state.json",
            "CHERRY_PICK_STATE",
            "REVERT_STATE",
            "BISECT_STATE",
            "MERGE_HEAD",
            "ORIG_HEAD",
        ] {
            if let Ok(text) = std::fs::read_to_string(mediagit.join(name)) {
                roots.extend(scan_hex_oids(&text));
            }
        }

        let reflog = Reflog::new(&mediagit);
        let zero_oid = Oid::from_bytes([0u8; 32]);
        if let Ok(ref_names) = reflog.list_refs().await {
            for ref_name in ref_names {
                if let Ok(entries) = reflog.read(&ref_name, None).await {
                    for entry in entries {
                        roots.extend(
                            [entry.old_oid, entry.new_oid]
                                .into_iter()
                                .filter(|o| *o != zero_oid),
                        );
                    }
                }
            }
        }

        roots.sort_by_key(|o| o.to_hex());
        roots.dedup();
        roots
    }

    /// Collect all objects referenced from a commit
    ///
    /// Previously this only recognized `Commit` and `Tag` objects, so a
    /// commit's `Tree` was never deserialized — every blob and chunk
    /// manifest it points to was left out of `referenced`, and
    /// `check_dangling`/`FsckRepair::remove_dangling_object` would then
    /// delete every live file in the repo as "dangling" (QA-007 data loss).
    /// Trees in this VCS are single-level (`BTreeMap<full relative path,
    /// TreeEntry>`, no subtrees), so a flat scan of `entries` is enough —
    /// no recursion needed here.
    fn collect_referenced_objects<'a>(
        &'a self,
        oid: &'a Oid,
        visited: &'a mut HashSet<Oid>,
        referenced: &'a mut HashSet<Oid>,
        report: &'a mut FsckReport,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = anyhow::Result<()>> + 'a>> {
        Box::pin(async move {
            if visited.contains(oid) {
                return Ok(());
            }
            visited.insert(*oid);
            referenced.insert(*oid);

            // Routed through the ODB (pack-aware, decompresses) rather than raw
            // storage, so dangling detection isn't fooled by packed objects.
            if let Ok(data) = self.odb.read(oid).await {
                match crate::format::deserialize_exact::<Commit>(&data) {
                    Ok(commit) => {
                        referenced.insert(commit.tree);
                        // Recurse into the tree itself (not just record its oid)
                        // so the Tree arm below actually runs and walks its
                        // entries -- otherwise every blob is still never marked
                        // referenced (QA-007).
                        self.collect_referenced_objects(&commit.tree, visited, referenced, report)
                            .await?;
                        for parent in commit.parents {
                            self.collect_referenced_objects(&parent, visited, referenced, report)
                                .await?;
                        }
                    }
                    _ => {
                        match crate::format::deserialize_exact::<Tag>(&data) {
                            Ok(tag) => {
                                referenced.insert(tag.target);
                                if tag.target_type == ObjectType::Commit {
                                    self.collect_referenced_objects(
                                        &tag.target,
                                        visited,
                                        referenced,
                                        report,
                                    )
                                    .await?;
                                }
                            }
                            _ => {
                                if let Ok(tree) = crate::format::deserialize_exact::<Tree>(&data) {
                                    for (name, entry) in &tree.entries {
                                        // A chunked blob's pack members are its chunks, which are
                                        // referenced via the blob's manifest — expand it here or
                                        // every packed chunk shows up as a dangling-object info.
                                        // insert() returning true = first sighting of this blob.
                                        if referenced.insert(entry.oid)
                                            && let Ok(Some(manifest)) =
                                                self.odb.get_chunk_manifest(&entry.oid).await
                                        {
                                            for chunk in &manifest.chunks {
                                                referenced.insert(chunk.id);
                                            }
                                        }
                                        if crate::is_stage_debris_key(name) {
                                            report.add_issue(
                                    FsckIssue::new(
                                        IssueSeverity::Warning,
                                        IssueCategory::InvalidFormat,
                                        format!(
                                            "tree {} entry '{}' is merge-stage debris; re-commit",
                                            oid, name
                                        ),
                                    )
                                    .with_oid(entry.oid),
                                );
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }

            Ok(())
        })
    }

    /// List all objects in storage, including objects packed into `packs/*.pack`.
    ///
    /// Previously this only collected bare loose OIDs, so after `gc --repack`
    /// (which deletes loose objects, leaving only packs) fsck/verify checked 0
    /// objects and reported the repo as clean. Packed objects are enumerated
    /// via `PackReader::list_objects()` (same reader the ODB's `read_from_packs`
    /// uses), not a hand-rolled parser.
    ///
    /// A pack that fails to read or fails its own checksum verification is
    /// corruption in its own right (e.g. a flipped byte in the trailing
    /// checksum) and is reported as a `ChecksumMismatch` issue rather than
    /// silently skipped — silently skipping would just reproduce the
    /// "0 objects, PERFECT" blind spot for a corrupted pack.
    async fn list_all_objects(&self, report: &mut FsckReport) -> anyhow::Result<Vec<Oid>> {
        use crate::pack::PackReader;

        let mut objects = HashSet::new();

        // Loose objects (LocalBackend already operates within objects/ directory,
        // returning bare hex OIDs with no "objects/" prefix).
        let object_keys = self.storage.list_objects("").await?;
        for key in object_keys {
            if key.len() == 64
                && let Ok(oid) = Oid::from_hex(&key)
            {
                objects.insert(oid);
            }
        }

        // Chunked-blob manifests: a chunked blob's OID exists only as
        // `manifests/<oid>` (no bare loose key), so without this it was
        // never added to `objects` and its content never reached
        // `verify_object` -> `odb.read` -> `read_chunked`'s per-chunk
        // BLAKE3 verification. At-rest chunk corruption was invisible
        // (QA-006a).
        let manifest_keys = self.storage.list_objects("manifests/").await?;
        for key in manifest_keys {
            if let Some(hex) = key.strip_prefix("manifests/")
                && let Ok(oid) = Oid::from_hex(hex)
            {
                objects.insert(oid);
            }
        }

        // Packed objects: read each pack and list the OIDs it contains.
        // Covers both legacy `gc --repack` packs (`packs/<id>.pack`) and
        // Track F cloud packs (`packs/<oid_hex>`, no extension) — same
        // envelope, PackReader parses either (see odb list_pack_files).
        let pack_keys = self.storage.list_objects("packs/").await?;
        for pack_key in pack_keys
            .iter()
            .filter(|k| k.ends_with(".pack") || !k.rsplit('/').next().unwrap_or("").contains('.'))
        {
            let Ok(pack_data) = self.storage.get(pack_key).await else {
                report.add_issue(FsckIssue::new(
                    IssueSeverity::Error,
                    IssueCategory::ChecksumMismatch,
                    format!("Pack file unreadable: {}", pack_key),
                ));
                continue;
            };
            match PackReader::new(pack_data) {
                Ok(reader) => objects.extend(reader.list_objects()),
                Err(e) => {
                    report.add_issue(FsckIssue::new(
                        IssueSeverity::Error,
                        IssueCategory::ChecksumMismatch,
                        format!("Pack file corrupt: {}: {}", pack_key, e),
                    ));
                }
            }
        }

        Ok(objects.into_iter().collect())
    }

    /// List all references
    async fn list_all_refs(&self) -> anyhow::Result<Vec<Ref>> {
        // If RefDatabase is available, use it (more reliable than storage for refs)
        if let Some(ref refdb) = self.refdb {
            let mut refs = Vec::new();

            // List branches, tags, remotes via refdb namespaces
            for namespace in ["heads", "remotes", "tags"] {
                if let Ok(ref_names) = refdb.list(namespace).await {
                    for name in ref_names {
                        if let Ok(r) = refdb.read(&name).await {
                            refs.push(r);
                        }
                    }
                }
            }

            // Also add HEAD
            if let Ok(head) = refdb.read("HEAD").await {
                refs.push(head);
            }

            return Ok(refs);
        }

        // Fallback: List all ref files from storage
        let mut refs = Vec::new();

        // List all ref files
        let ref_keys = self.storage.list_objects("refs/").await?;

        for key in ref_keys {
            if let Ok(data) = self.storage.get(&key).await
                && let Ok(r) = crate::format::deserialize::<Ref>(&data)
            {
                refs.push(r);
            }
        }

        // Also check HEAD
        if let Ok(head_data) = self.storage.get("HEAD").await
            && let Ok(head_ref) = crate::format::deserialize::<Ref>(&head_data)
        {
            refs.push(head_ref);
        }

        Ok(refs)
    }

    /// Check if an object exists
    ///
    /// Routed through the ODB's `read` (pack-aware, falls back to
    /// `read_from_packs`) rather than raw loose storage, so packed objects
    /// aren't reported missing after `gc --repack`.
    async fn object_exists(&self, oid: &Oid) -> anyhow::Result<bool> {
        // Presence check, not a verified read: `read()` of a chunked blob
        // reassembles it in RAM. Integrity of present objects is judged by
        // `verify_object`. `exists()` does not know legacy `deltas/<oid>.meta`
        // objects, so a miss still falls back to a read.
        if self.odb.exists(oid).await? {
            return Ok(true);
        }
        Ok(self.odb.read(oid).await.is_ok())
    }

    /// Check if a reference exists
    async fn ref_exists(&self, ref_name: &str) -> anyhow::Result<bool> {
        // Refs live in the RefDatabase (postcard-encoded), not as raw storage
        // keys — a raw exists() check false-positives "missing ref" warnings.
        if let Some(refdb) = &self.refdb {
            return refdb.exists(ref_name).await;
        }
        self.storage.exists(ref_name).await
    }
}

/// Repair functionality for fixing common issues
pub struct FsckRepair {
    storage: Arc<dyn StorageBackend>,
    /// Needed by repairs that must *reconstruct* content rather than just
    /// delete it (chunk-delta chain flattening). Optional so existing
    /// storage-only callers keep working; those simply cannot perform
    /// ODB-level repairs and say so rather than silently skipping.
    odb: Option<Arc<ObjectDatabase>>,
    /// Refs are files managed by `RefDatabase`, not storage keys; broken-ref
    /// repair and the ref walk behind [`FsckRepair::paths_referencing`] need it.
    refdb: Option<RefDatabase>,
    /// Directory containing `.mediagit`. Source of the working files an
    /// object can be restored from, and home of `.mediagit/quarantine`.
    /// Without it nothing is moved or restored: repair never deletes.
    repo_root: Option<PathBuf>,
}

/// A file in some reachable tree that needs a damaged object.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReferencingFile {
    /// Path as stored in the tree (relative to the repository root).
    pub path: String,
    /// The tree entry's blob id (the chunk's owner, for a chunk).
    pub blob: Oid,
}

/// What a repair run did. Counts are the run's own claims until
/// [`RepairOutcome::confirm_against`] filters them with a fresh check.
#[derive(Debug, Clone, Default)]
pub struct RepairOutcome {
    /// `--dry-run`: everything below is what *would* happen.
    pub dry_run: bool,
    /// Damaged objects rewritten from a good copy.
    pub restored: Vec<Oid>,
    /// Damaged objects moved to `.mediagit/quarantine` (never deleted).
    pub quarantined: Vec<Oid>,
    /// Broken refs removed through `RefDatabase`.
    pub refs_removed: Vec<String>,
    /// Over-deep chunk-delta chains rewritten as full chunks.
    pub flattened: Vec<Oid>,
    /// Files that need a quarantined object and so cannot be restored.
    pub unrestorable_files: Vec<String>,
    /// (damaged or missing object, file that needs it) behind `unrestorable_files`.
    pub lost: Vec<(Oid, String)>,
}

impl RepairOutcome {
    /// Issues actually fixed (quarantining is containment, not a fix).
    pub fn repaired(&self) -> u64 {
        (self.restored.len() + self.refs_removed.len() + self.flattened.len()) as u64
    }

    /// The distinct objects no source could restore yet.
    pub fn lost_oids(&self) -> Vec<Oid> {
        let mut v: Vec<Oid> = self.lost.iter().map(|(o, _)| *o).collect();
        v.sort_unstable();
        v.dedup();
        v
    }

    /// A later source (the remote) restored these objects: move them from
    /// "quarantined"/"lost" to "restored".
    pub fn mark_restored(&mut self, healthy: &[Oid]) {
        for oid in healthy {
            self.quarantined.retain(|q| q != oid);
            if self.lost.iter().any(|(o, _)| o == oid) && !self.restored.contains(oid) {
                self.restored.push(*oid);
            }
            self.lost.retain(|(o, _)| o != oid);
        }
        self.refresh_unrestorable_files();
    }

    fn refresh_unrestorable_files(&mut self) {
        self.unrestorable_files = self.lost.iter().map(|(_, p)| p.clone()).collect();
        self.unrestorable_files.sort();
        self.unrestorable_files.dedup();
    }

    /// Keep only the fixes a fresh check agrees with: an object, ref or chain
    /// that still shows up as an issue was not repaired, whatever the repair
    /// step believed.
    pub fn confirm_against(&mut self, recheck: &FsckReport) {
        let still_bad = |oid: &Oid| recheck.issues.iter().any(|i| i.oid == Some(*oid));
        self.restored.retain(|o| !still_bad(o));
        self.flattened.retain(|o| !still_bad(o));
        self.refs_removed.retain(|r| {
            !recheck
                .issues
                .iter()
                .any(|i| i.ref_name.as_deref() == Some(r))
        });
    }
}

impl FsckRepair {
    /// Create a new FSCK repair tool
    ///
    /// Repairs that need to reconstruct content (see
    /// [`FsckRepair::with_odb`]) are unavailable on an instance built this
    /// way — `repair` reports them instead of claiming success.
    pub fn new(storage: Arc<dyn StorageBackend>) -> Self {
        Self {
            storage,
            odb: None,
            refdb: None,
            repo_root: None,
        }
    }

    /// Attach the ref database (broken-ref repair, ref walk).
    pub fn with_refdb(mut self, refdb: RefDatabase) -> Self {
        self.refdb = Some(refdb);
        self
    }

    /// Attach the repository root (the directory containing `.mediagit`).
    pub fn with_repo_root(mut self, repo_root: impl Into<PathBuf>) -> Self {
        self.repo_root = Some(repo_root.into());
        self
    }

    /// Attach an ODB, enabling repairs that reconstruct content.
    ///
    /// Required for `IssueCategory::DeepDeltaChain`: flattening an over-deep
    /// chain means applying every delta down to the base chunk, which is ODB
    /// work and cannot be done through the raw storage handle.
    pub fn with_odb(mut self, odb: Arc<ObjectDatabase>) -> Self {
        self.odb = Some(odb);
        self
    }

    /// Attempt to repair issues found in an FSCK report
    ///
    /// # Arguments
    ///
    /// * `report` - FSCK report with issues to repair
    /// * `dry_run` - If true, only simulate repairs without making changes
    ///
    /// # Returns
    ///
    /// A [`RepairOutcome`]: what was restored, quarantined, removed or
    /// flattened, and which files can no longer be restored. Damaged objects
    /// are restored from the working tree when a file there hashes to the
    /// tree's id; otherwise they are moved to `.mediagit/quarantine`.
    ///
    /// Pack and remote restore sources do not exist yet; a damaged object
    /// that only a pack or remote could heal is quarantined.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # use mediagit_versioning::fsck::{FsckChecker, FsckRepair, FsckOptions};
    /// # use mediagit_storage::LocalBackend;
    /// # use std::sync::Arc;
    /// # #[tokio::main]
    /// # async fn main() -> anyhow::Result<()> {
    /// # let storage: Arc<dyn mediagit_storage::StorageBackend> =
    /// #     Arc::new(LocalBackend::new("/tmp").await?);
    /// let checker = FsckChecker::new(storage.clone());
    /// let report = checker.check(FsckOptions::full()).await?;
    ///
    /// if report.has_errors() {
    ///     let repair = FsckRepair::new(storage);
    ///     let outcome = repair.repair(&report, false).await?;
    ///     println!("Repaired {} issues", outcome.repaired());
    /// }
    /// # Ok(())
    /// # }
    /// ```
    pub async fn repair(
        &self,
        report: &FsckReport,
        dry_run: bool,
    ) -> anyhow::Result<RepairOutcome> {
        info!(
            dry_run = dry_run,
            repairable = report.repairable_issues().len(),
            "Starting FSCK repair"
        );

        let mut out = RepairOutcome {
            dry_run,
            ..RepairOutcome::default()
        };

        let mut damaged: Vec<Oid> = report
            .repairable_issues()
            .iter()
            .filter(|i| {
                matches!(
                    i.category,
                    IssueCategory::ChecksumMismatch | IssueCategory::MissingObject
                )
            })
            .filter_map(|i| i.oid)
            .collect();
        damaged.sort_unstable();
        damaged.dedup();
        let referencing = self.paths_referencing(&damaged).await?;

        let mut handled = HashSet::new();
        for issue in report.repairable_issues() {
            match issue.category {
                IssueCategory::ChecksumMismatch | IssueCategory::MissingObject => {
                    if let Some(oid) = issue.oid
                        && handled.insert(oid)
                    {
                        let files = referencing.get(&oid).map_or(&[][..], |v| v.as_slice());
                        let missing = issue.category == IssueCategory::MissingObject;
                        self.repair_damaged_object(&oid, files, missing, dry_run, &mut out)
                            .await?;
                    }
                }
                IssueCategory::BrokenReference => {
                    if let Some(ref_name) = &issue.ref_name
                        && self.repair_broken_reference(ref_name, dry_run).await?
                    {
                        out.refs_removed.push(ref_name.clone());
                    }
                }
                IssueCategory::DeepDeltaChain => {
                    if let Some(oid) = issue.oid
                        && self.flatten_deep_chunk_chain(&oid, dry_run).await?
                    {
                        out.flattened.push(oid);
                    }
                }
                _ => {
                    warn!("No repair strategy for category: {:?}", issue.category);
                }
            }
        }

        out.refresh_unrestorable_files();
        info!(
            restored = out.restored.len(),
            quarantined = out.quarantined.len(),
            refs_removed = out.refs_removed.len(),
            "FSCK repair complete"
        );
        Ok(out)
    }

    /// Flatten an over-deep chunk-delta chain by re-storing the chunk in full.
    ///
    /// `get_chunk` refuses to reconstruct a chain deeper than `MAX_DELTA_DEPTH`,
    /// so a repository holding one cannot be pushed or cloned — and this is the
    /// one code path allowed to walk past that limit, because it is what makes
    /// such a repository readable again.
    ///
    /// Safety properties, in order:
    ///
    /// 1. **Verify before mutate.** The reconstructed bytes must hash to
    ///    `chunk_id`; on mismatch nothing is written and the repair fails
    ///    loudly. Flattening is the one repair that *writes content*, so a
    ///    silent mis-reconstruction here would be indistinguishable from
    ///    corruption.
    /// 2. **Write the full chunk before deleting the delta.** A crash then
    ///    leaves an unreachable delta (collected by `gc`), never a chunk with
    ///    no payload. This ordering is also required on Windows, where an open
    ///    handle makes a delete fail — the reverse order could remove the delta
    ///    and then fail to write the replacement, destroying the only copy.
    /// 3. **Only report success when it happened** — the lesson from
    ///    `repair_corrupted_object` below, which used to count no-op deletes.
    async fn flatten_deep_chunk_chain(
        &self,
        chunk_id: &Oid,
        dry_run: bool,
    ) -> anyhow::Result<bool> {
        let Some(odb) = self.odb.as_ref() else {
            warn!(
                "Cannot flatten chunk-delta chain for {}: repair was constructed \
                 without an ODB handle (use FsckRepair::with_odb)",
                chunk_id
            );
            return Ok(false);
        };

        if dry_run {
            info!(
                "[DRY RUN] Would flatten over-deep chunk-delta chain at {}",
                chunk_id
            );
            return Ok(true);
        }

        // Reconstruct by walking the whole chain, however deep it goes.
        let data = match odb.reconstruct_chunk_unbounded(chunk_id).await {
            Ok(d) => d,
            Err(e) => {
                warn!(
                    "Cannot flatten chunk-delta chain for {}: reconstruction failed: {}",
                    chunk_id, e
                );
                return Ok(false);
            }
        };

        // (1) Verify before mutating anything.
        let actual = Oid::hash(&data);
        if actual != *chunk_id {
            warn!(
                "Refusing to flatten {}: reconstructed content hashes to {} \
                 — the chain is corrupt, not merely deep",
                chunk_id, actual
            );
            return Ok(false);
        }

        // (2) Write the replacement first, then drop the delta.
        odb.write_full_chunk(chunk_id, &data).await?;

        let meta_key = format!("chunk-deltas/{}.meta", chunk_id.to_hex());
        let delta_key = format!("chunk-deltas/{}", chunk_id.to_hex());
        let _ = self.storage.delete(&meta_key).await;
        let _ = self.storage.delete(&delta_key).await;

        info!(
            "Flattened over-deep chunk-delta chain at {} ({} bytes re-stored in full)",
            chunk_id,
            data.len()
        );
        Ok(true)
    }

    /// Every file that needs one of `oids`, found by walking the trees of all
    /// commits reachable from local refs: a tree entry whose blob id is in
    /// `oids`, or whose chunk manifest lists a chunk in `oids`. Empty when the
    /// ref database or ODB is not attached.
    ///
    /// Annotated-tag objects are not followed (the commit they point at is
    /// normally reachable from a branch too).
    pub async fn paths_referencing(
        &self,
        oids: &[Oid],
    ) -> anyhow::Result<HashMap<Oid, Vec<ReferencingFile>>> {
        let mut found: HashMap<Oid, Vec<ReferencingFile>> = HashMap::new();
        let (Some(refdb), Some(odb)) = (&self.refdb, &self.odb) else {
            return Ok(found);
        };
        if oids.is_empty() {
            return Ok(found);
        }
        let wanted: HashSet<Oid> = oids.iter().copied().collect();

        let mut commits = Vec::new();
        for namespace in ["heads", "remotes", "tags"] {
            for name in refdb.list(namespace).await.unwrap_or_default() {
                if let Ok(r) = refdb.read(&name).await
                    && let Some(oid) = r.oid
                {
                    commits.push(oid);
                }
            }
        }

        let mut seen_commits = HashSet::new();
        let mut seen_trees = HashSet::new();
        // blob id -> the wanted chunk ids its manifest lists (one manifest read per blob)
        let mut chunks_of: HashMap<Oid, Vec<Oid>> = HashMap::new();
        while let Some(commit_oid) = commits.pop() {
            if !seen_commits.insert(commit_oid) {
                continue;
            }
            let Ok(data) = odb.read(&commit_oid).await else {
                continue;
            };
            let Ok(commit) = crate::format::deserialize::<Commit>(&data) else {
                continue;
            };
            commits.extend(commit.parents.iter().copied());

            let mut trees = vec![(commit.tree, String::new())];
            while let Some((tree_oid, prefix)) = trees.pop() {
                // ponytail: a tree reused at a second directory is walked once
                if !seen_trees.insert(tree_oid) {
                    continue;
                }
                let Ok(data) = odb.read(&tree_oid).await else {
                    continue;
                };
                let Ok(tree) = crate::format::deserialize::<Tree>(&data) else {
                    continue;
                };
                for (key, entry) in &tree.entries {
                    let path = if prefix.is_empty() {
                        key.clone()
                    } else {
                        format!("{prefix}/{key}")
                    };
                    if entry.mode == FileMode::Directory {
                        trees.push((entry.oid, path));
                        continue;
                    }
                    let mut hits = Vec::new();
                    if wanted.contains(&entry.oid) {
                        hits.push(entry.oid);
                    }
                    let listed = match chunks_of.get(&entry.oid) {
                        Some(listed) => listed.clone(),
                        None => {
                            let listed: Vec<Oid> = match odb.get_chunk_manifest(&entry.oid).await {
                                Ok(Some(m)) => m
                                    .chunks
                                    .iter()
                                    .map(|c| c.id)
                                    .filter(|id| wanted.contains(id))
                                    .collect(),
                                _ => Vec::new(),
                            };
                            chunks_of.insert(entry.oid, listed.clone());
                            listed
                        }
                    };
                    hits.extend(listed);
                    for hit in hits {
                        let file = ReferencingFile {
                            path: path.clone(),
                            blob: entry.oid,
                        };
                        let files = found.entry(hit).or_default();
                        if !files.contains(&file) {
                            files.push(file);
                        }
                    }
                }
            }
        }
        Ok(found)
    }

    /// Does `oid` read back intact right now (cache cleared)? Works for whole
    /// objects and chunks alike.
    pub async fn is_healthy(&self, oid: &Oid) -> bool {
        let Some(odb) = &self.odb else {
            return false;
        };
        odb.invalidate_cache(oid).await;
        odb.stream_object_to(oid, &mut tokio::io::sink())
            .await
            .is_ok()
            || matches!(odb.get_chunk(oid).await, Ok(d) if Oid::hash(&d) == *oid)
    }

    /// The ODB repairs write through (and `is_healthy` reads through).
    pub fn odb(&self) -> Option<Arc<ObjectDatabase>> {
        self.odb.clone()
    }

    /// `<root>/.mediagit/quarantine`
    fn quarantine_dir(&self) -> Option<PathBuf> {
        self.repo_root
            .as_ref()
            .map(|r| r.join(".mediagit").join("quarantine"))
    }

    /// The stored pieces of a damaged object, as (quarantine kind, storage key).
    /// A chunk wins over a same-id loose object, as it always did here.
    async fn stored_pieces(&self, oid: &Oid) -> Vec<(&'static str, String)> {
        let hex = oid.to_hex();
        let mut pieces = Vec::new();
        for (kind, key) in [
            ("chunk", format!("chunks/{hex}")),
            ("chunk-delta-meta", format!("chunk-deltas/{hex}.meta")),
            ("chunk-delta", format!("chunk-deltas/{hex}")),
        ] {
            if self.storage.exists(&key).await.unwrap_or(false) {
                pieces.push((kind, key));
            }
        }
        if pieces.is_empty() && self.storage.exists(&hex).await.unwrap_or(false) {
            pieces.push(("loose", hex));
        }
        pieces
    }

    /// Move one stored key to `.mediagit/quarantine/<kind>/<oid>[.n]`.
    /// The original is removed only after the copy is on disk at full length,
    /// and an existing quarantine file is never overwritten.
    async fn quarantine(&self, kind: &str, key: &str, oid: &Oid) -> anyhow::Result<()> {
        use tokio::io::AsyncWriteExt;

        let dir = self
            .quarantine_dir()
            .ok_or_else(|| anyhow::anyhow!("no repository root, cannot quarantine"))?
            .join(kind);
        tokio::fs::create_dir_all(&dir).await?;
        let bytes = self.storage.get(key).await?;

        let hex = oid.to_hex();
        let mut n = 0u32;
        let dest = loop {
            let name = if n == 0 {
                hex.clone()
            } else {
                format!("{hex}.{n}")
            };
            let path = dir.join(name);
            match tokio::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&path)
                .await
            {
                Ok(mut f) => {
                    f.write_all(&bytes).await?;
                    f.sync_all().await?;
                    break path;
                }
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => n += 1,
                Err(e) => return Err(e.into()),
            }
        };
        anyhow::ensure!(
            tokio::fs::metadata(&dest).await?.len() == bytes.len() as u64,
            "quarantine copy {} is short; original kept",
            dest.display()
        );
        warn!("Quarantined damaged {} {} at {}", kind, oid, dest.display());
        // Safe: the bytes are in quarantine, verified above.
        self.storage.delete(key).await?;
        Ok(())
    }

    /// A good copy of `oid` in the working tree, if any file needing it has one.
    async fn find_source(
        &self,
        oid: &Oid,
        is_chunk: bool,
        files: &[ReferencingFile],
    ) -> Option<Source> {
        let root = self.repo_root.as_ref()?;
        let odb = self.odb.as_ref()?;
        for f in files {
            let rel = Path::new(&f.path);
            // Tree paths are repository content: never follow `..` or absolute ones.
            if !rel.components().all(|c| matches!(c, Component::Normal(_))) {
                continue;
            }
            let path = root.join(rel);
            if !path.is_file() {
                continue;
            }
            if is_chunk {
                let Ok(Some(manifest)) = odb.get_chunk_manifest(&f.blob).await else {
                    continue;
                };
                let Some(c) = manifest.chunks.iter().find(|c| c.id == *oid) else {
                    continue;
                };
                let (offset, size) = (c.offset, c.size);
                let read = tokio::task::spawn_blocking(move || {
                    use std::io::{Read, Seek, SeekFrom};
                    let mut file = std::fs::File::open(path)?;
                    file.seek(SeekFrom::Start(offset))?;
                    let mut buf = vec![0u8; size];
                    file.read_exact(&mut buf)?;
                    std::io::Result::Ok(buf)
                })
                .await;
                if let Ok(Ok(data)) = read
                    && Oid::hash(&data) == *oid
                {
                    return Some(Source::Chunk(data));
                }
            } else if f.blob == *oid && Oid::from_file_async(&path).await.ok() == Some(*oid) {
                return Some(Source::File {
                    path,
                    name: f.path.clone(),
                });
            }
        }
        // Next best: a pack still holds an intact copy (a crash in the middle
        // of `repack` leaves objects both loose and packed). `read` prefers
        // the loose file, so damage there hides the good packed one.
        if odb.packed_copy_is_intact(oid).await {
            return Some(Source::Pack);
        }
        None
    }

    /// Write `source` back and read it back through the ODB (cache cleared, so
    /// the bytes on disk are what is judged).
    async fn restore(
        &self,
        odb: &ObjectDatabase,
        oid: &Oid,
        source: Source,
        is_chunk: bool,
    ) -> bool {
        let written = match source {
            // Nothing to write: with the damaged loose file quarantined, reads
            // fall through to the pack. The read-back below proves it.
            Source::Pack => true,
            Source::Chunk(data) => odb.write_full_chunk(oid, &data).await.is_ok(),
            Source::File { path, name } => match tokio::fs::read(&path).await {
                Ok(data) => matches!(
                    odb.write_with_path(ObjectType::Blob, &data, &name).await,
                    Ok(written) if written == *oid
                ),
                Err(_) => false,
            },
        };
        if !written {
            return false;
        }
        odb.invalidate_cache(oid).await;
        if is_chunk {
            matches!(odb.get_chunk(oid).await, Ok(d) if Oid::hash(&d) == *oid)
        } else {
            odb.stream_object_to(oid, &mut tokio::io::sink())
                .await
                .is_ok()
        }
    }

    /// Restore a damaged object from the working tree, or quarantine it.
    /// Nothing is ever deleted: the bad bytes go to `.mediagit/quarantine`
    /// first, and only then is a good copy written in their place.
    async fn repair_damaged_object(
        &self,
        oid: &Oid,
        files: &[ReferencingFile],
        missing: bool,
        dry_run: bool,
        out: &mut RepairOutcome,
    ) -> anyhow::Result<()> {
        let pieces = self.stored_pieces(oid).await;
        // `missing`: the walk found nothing stored under this id, so there is
        // nothing to quarantine, only a good copy to look for. A "missing"
        // object that is in fact stored is left to the next check.
        if missing && (!pieces.is_empty() || files.is_empty()) {
            return Ok(());
        }
        if pieces.is_empty() && !missing {
            // Distinguish the two reasons the probe misses. Reporting a
            // chunked manifest as "likely packed" sent operators to
            // `gc --repack`, which cannot help.
            if self
                .storage
                .exists(&format!("manifests/{}", oid.to_hex()))
                .await
                .unwrap_or(false)
            {
                warn!(
                    "Cannot repair damaged object {}: it is a chunked-blob manifest. \
                     The damage is in its chunks — re-run fsck so they are reported \
                     individually, or re-fetch the object.",
                    oid
                );
            } else {
                warn!(
                    "Cannot repair damaged object {}: not present as a loose file \
                     (likely packed); requires a repack",
                    oid
                );
            }
            return Ok(());
        }
        let (Some(odb), Some(_root)) = (&self.odb, &self.repo_root) else {
            warn!(
                "Cannot repair damaged object {}: repair was built without an ODB or \
                 repository root (use FsckRepair::with_odb and with_repo_root)",
                oid
            );
            return Ok(());
        };

        let is_chunk = if missing {
            files.iter().all(|f| f.blob != *oid)
        } else {
            pieces.iter().any(|(kind, _)| *kind != "loose")
        };
        let source = self.find_source(oid, is_chunk, files).await;
        let unrestorable = |out: &mut RepairOutcome| {
            if !missing {
                out.quarantined.push(*oid);
            }
            out.lost
                .extend(files.iter().map(|f| (*oid, f.path.clone())));
        };

        if dry_run {
            info!("[DRY RUN] Would repair damaged object {}", oid);
            if source.is_some() {
                out.restored.push(*oid);
            } else {
                unrestorable(out);
            }
            return Ok(());
        }

        for (kind, key) in &pieces {
            if let Err(e) = self.quarantine(kind, key, oid).await {
                warn!(
                    "Cannot quarantine damaged {} {}: {}; left in place",
                    kind, oid, e
                );
                return Ok(());
            }
        }
        odb.invalidate_cache(oid).await;

        let restored = match source {
            Some(source) => self.restore(odb, oid, source, is_chunk).await,
            None => false,
        };
        if restored {
            out.restored.push(*oid);
            return Ok(());
        }
        // A rewrite that failed its read-back must not stay behind live.
        for (kind, key) in self.stored_pieces(oid).await {
            if let Err(e) = self.quarantine(kind, &key, oid).await {
                warn!("Cannot quarantine bad rewrite of {}: {}", oid, e);
            }
        }
        unrestorable(out);
        Ok(())
    }

    /// Remove a broken reference through `RefDatabase` (refs are files, not
    /// storage keys). `HEAD` is never removed: it names an unborn branch in
    /// every fresh repository.
    async fn repair_broken_reference(&self, ref_name: &str, dry_run: bool) -> anyhow::Result<bool> {
        let Some(refdb) = &self.refdb else {
            warn!(
                "Cannot remove broken reference {}: repair was built without a RefDatabase \
                 (use FsckRepair::with_refdb)",
                ref_name
            );
            return Ok(false);
        };
        if ref_name == "HEAD" {
            warn!("Not removing HEAD: it points at a branch that has no commits yet");
            return Ok(false);
        }
        if dry_run {
            info!("[DRY RUN] Would remove broken reference: {}", ref_name);
            return Ok(true);
        }

        warn!("Removing broken reference: {}", ref_name);
        if let Err(e) = refdb.delete(ref_name).await {
            warn!("Cannot remove broken reference {}: {}", ref_name, e);
            return Ok(false);
        }
        Ok(true)
    }
}

/// What is left in `.mediagit/quarantine` after a sweep.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct QuarantineStatus {
    /// Copies deleted by this sweep because their object is healthy again.
    pub removed: u64,
    /// Copies still held.
    pub count: u64,
    /// Total size of the copies still held.
    pub bytes: u64,
}

/// One file in `.mediagit/quarantine`.
#[derive(Debug, Clone)]
pub struct QuarantineEntry {
    pub path: PathBuf,
    /// `loose`, `chunk`, `chunk-delta-meta` or `chunk-delta`.
    pub kind: String,
    /// `None` when the file name is not `<oid>[.n]`.
    pub oid: Option<Oid>,
    pub bytes: u64,
}

/// Every file in `.mediagit/quarantine` (empty when the folder is absent).
pub async fn list_quarantine(repo_root: &Path) -> anyhow::Result<Vec<QuarantineEntry>> {
    let root = repo_root.join(".mediagit").join("quarantine");
    let mut out = Vec::new();
    let Ok(mut kinds) = tokio::fs::read_dir(&root).await else {
        return Ok(out);
    };
    while let Some(kind_dir) = kinds.next_entry().await? {
        if !kind_dir.file_type().await?.is_dir() {
            continue;
        }
        let kind = kind_dir.file_name().to_string_lossy().into_owned();
        let mut files = tokio::fs::read_dir(kind_dir.path()).await?;
        while let Some(f) = files.next_entry().await? {
            let name = f.file_name().to_string_lossy().into_owned();
            // `<oid>` or `<oid>.<n>`
            let oid = name.split('.').next().and_then(|h| Oid::from_hex(h).ok());
            out.push(QuarantineEntry {
                path: f.path(),
                kind: kind.clone(),
                oid,
                bytes: f.metadata().await?.len(),
            });
        }
    }
    Ok(out)
}

/// Drop quarantined copies whose object now verifies in the store, and count
/// what stays. This is the only automatic deletion from quarantine: a copy of
/// an object that is still missing or still damaged is never touched.
/// `delete = false` (dry run) only counts. `odb` should have a cold cache.
pub async fn sweep_quarantine(
    odb: &ObjectDatabase,
    repo_root: &Path,
    delete: bool,
) -> anyhow::Result<QuarantineStatus> {
    let mut status = QuarantineStatus::default();
    for entry in list_quarantine(repo_root).await? {
        let healthy = match entry.oid {
            Some(oid) => {
                odb.invalidate_cache(&oid).await;
                if entry.kind == "loose" {
                    odb.stream_object_to(&oid, &mut tokio::io::sink())
                        .await
                        .is_ok()
                } else {
                    matches!(odb.get_chunk(&oid).await, Ok(d) if Oid::hash(&d) == oid)
                }
            }
            None => false,
        };
        if healthy && delete {
            tokio::fs::remove_file(&entry.path).await?;
            status.removed += 1;
        } else {
            status.count += 1;
            status.bytes += entry.bytes;
        }
    }
    Ok(status)
}

/// Where a good copy of a damaged object comes from.
enum Source {
    /// The exact bytes of one chunk, cut from a working file.
    Chunk(Vec<u8>),
    /// An intact copy inside a pack file.
    Pack,
    /// A working file that hashes to the object id.
    File { path: PathBuf, name: String },
}

#[cfg(test)]
mod tests {
    use super::*;
    use mediagit_storage::mock::MockBackend;

    async fn checker_with_metas(entries: &[(&Oid, &Oid)], full_chunks: &[&Oid]) -> FsckChecker {
        let storage = Arc::new(MockBackend::new());
        for (id, base) in entries {
            storage
                .put(
                    &format!("chunk-deltas/{}.meta", id.to_hex()),
                    format!("base:{}", base.to_hex()).as_bytes(),
                )
                .await
                .unwrap();
        }
        for id in full_chunks {
            storage
                .put(&format!("chunks/{}", id.to_hex()), b"full chunk data")
                .await
                .unwrap();
        }
        FsckChecker::new(storage)
    }

    #[tokio::test]
    async fn test_fsck_detects_chunk_delta_cycle() {
        // The 2026-07-07 AWS incident shape: A→B→C→A, no full copy anywhere.
        let a = Oid::hash(b"fsck-a");
        let b = Oid::hash(b"fsck-b");
        let c = Oid::hash(b"fsck-c");
        let checker = checker_with_metas(&[(&a, &b), (&b, &c), (&c, &a)], &[]).await;

        let mut report = FsckReport::new();
        checker.check_chunk_deltas(&mut report).await.unwrap();

        let cycles: Vec<_> = report
            .issues
            .iter()
            .filter(|i| i.category == IssueCategory::CircularReference)
            .collect();
        assert!(
            !cycles.is_empty(),
            "A→B→C→A chunk-delta cycle must be reported"
        );
        assert!(report.has_errors());
    }

    #[tokio::test]
    async fn test_fsck_detects_missing_chunk_delta_base() {
        let a = Oid::hash(b"fsck-orphan");
        let missing = Oid::hash(b"fsck-missing-base");
        let checker = checker_with_metas(&[(&a, &missing)], &[]).await;

        let mut report = FsckReport::new();
        checker.check_chunk_deltas(&mut report).await.unwrap();

        assert!(
            report
                .issues
                .iter()
                .any(|i| i.category == IssueCategory::MissingObject),
            "chain ending at a missing base chunk must be reported"
        );
    }

    #[tokio::test]
    async fn test_fsck_healthy_chunk_delta_chain_is_clean() {
        let a = Oid::hash(b"fsck-h1");
        let b = Oid::hash(b"fsck-h2");
        let full = Oid::hash(b"fsck-full");
        let checker = checker_with_metas(&[(&a, &b), (&b, &full)], &[&full]).await;

        let mut report = FsckReport::new();
        checker.check_chunk_deltas(&mut report).await.unwrap();

        assert_eq!(
            report.total_issues(),
            0,
            "healthy A→B→full chain must produce no issues, got: {:?}",
            report.issues
        );
    }

    #[tokio::test]
    async fn test_fsck_detects_over_deep_chunk_delta_chain() {
        // Chain longer than MAX_DELTA_DEPTH hops, terminating at a valid
        // full chunk (not a cycle, not missing) — must still surface as a
        // warning so operators know to repack before a depth-bounded client
        // walk (MAX_DELTA_DEPTH-capped) fails to resolve it.
        let full = Oid::hash(b"fsck-deep-full");
        let chain: Vec<Oid> = (0..12)
            .map(|i| Oid::hash(format!("fsck-deep-{i}").as_bytes()))
            .collect();
        let mut entries: Vec<(&Oid, &Oid)> = (0..chain.len() - 1)
            .map(|i| (&chain[i], &chain[i + 1]))
            .collect();
        entries.push((&chain[chain.len() - 1], &full));
        let checker = checker_with_metas(&entries, &[&full]).await;

        let mut report = FsckReport::new();
        checker.check_chunk_deltas(&mut report).await.unwrap();

        let deep: Vec<_> = report
            .issues
            .iter()
            .filter(|i| i.category == IssueCategory::DeepDeltaChain)
            .collect();
        assert!(
            !deep.is_empty(),
            "chain deeper than MAX_DELTA_DEPTH must be reported, got: {:?}",
            report.issues
        );
        assert!(
            deep.iter().all(|i| i.repairable),
            "an over-deep chain leaves the repo unpushable, so it must be \
             marked repairable — otherwise `fsck --repair` silently skips it \
             and the operator has no way forward"
        );
        // FS-1: depth must be reported in hops (edges), not visited nodes.
        // The longest chain here is 12 deltas terminating at a full chunk —
        // 12 hops, though `visited` holds 13 entries. Asserted as "nothing
        // exceeds 12" rather than "something equals 12": fsck reports a chain
        // from *every* delta node, so the shorter suffixes mean an
        // `any(== 12)` check passes even with the node/edge bug present.
        assert!(
            !deep.iter().any(|i| i.message.contains("is 13 hops deep")),
            "depth must be reported as hops, not visited-node count, got: {:?}",
            deep.iter().map(|i| &i.message).collect::<Vec<_>>()
        );
    }

    /// FS-1 regression: a chain at *exactly* `MAX_DELTA_DEPTH` is legal.
    ///
    /// `resolve_delta_base` permits a nominated base at depth
    /// `MAX_DELTA_DEPTH - 1`, so the deepest chain a writer can produce has
    /// exactly `MAX_DELTA_DEPTH` delta nodes, and `get_chunk` reconstructs it
    /// without complaint. `visited` also holds the terminal full chunk, so
    /// the old node-count comparison flagged this healthy chain as over-deep
    /// — every warning reading "11 hops deep (max 10)" on a repo where
    /// nothing was actually wrong and no repair could clear it.
    #[tokio::test]
    async fn chain_at_exactly_max_delta_depth_is_not_reported_as_too_deep() {
        let depth = crate::odb::MAX_DELTA_DEPTH as usize;
        let full = Oid::hash(b"fsck-atcap-full");
        let chain: Vec<Oid> = (0..depth)
            .map(|i| Oid::hash(format!("fsck-atcap-{i}").as_bytes()))
            .collect();
        let mut entries: Vec<(&Oid, &Oid)> = (0..chain.len() - 1)
            .map(|i| (&chain[i], &chain[i + 1]))
            .collect();
        entries.push((&chain[chain.len() - 1], &full));
        let checker = checker_with_metas(&entries, &[&full]).await;

        let mut report = FsckReport::new();
        checker.check_chunk_deltas(&mut report).await.unwrap();

        let deep: Vec<_> = report
            .issues
            .iter()
            .filter(|i| i.category == IssueCategory::DeepDeltaChain)
            .collect();
        assert!(
            deep.is_empty(),
            "a chain of exactly MAX_DELTA_DEPTH ({depth}) hops is what the \
             writer is allowed to build and the reader accepts, so fsck must \
             stay silent; got: {:?}",
            deep.iter().map(|i| &i.message).collect::<Vec<_>>()
        );
    }

    /// Building a genuine over-deep chain requires real delta payloads, so
    /// this drives the ODB directly rather than fabricating `.meta` files:
    /// the point is to prove the *reconstruct* path recovers, which fake
    /// sidecars would not exercise.
    #[tokio::test]
    async fn test_fsck_repair_flattens_over_deep_chunk_delta_chain() {
        use crate::delta::DeltaEncoder;
        use mediagit_compression::{
            ObjectType as CompObjectType, SmartCompressor, TypeAwareCompressor,
        };
        use mediagit_storage::LocalBackend;

        let tmp = tempfile::tempdir().unwrap();
        let storage: Arc<dyn StorageBackend> =
            Arc::new(LocalBackend::new(tmp.path().to_path_buf()).await.unwrap());
        let odb = Arc::new(ObjectDatabase::with_smart_compression(
            storage.clone(),
            10_000_000,
        ));
        // Same compressor the ODB builds internally, so the payloads we plant
        // are byte-for-byte what it would have written.
        let smart = SmartCompressor::new();

        // Base full chunk, then MAX+2 successive deltas, each a small edit of
        // the last — the shape a run of similar chunks produces.
        let mut payloads: Vec<Vec<u8>> = Vec::new();
        let mut cur = vec![7u8; 4096];
        payloads.push(cur.clone());
        for i in 0..(crate::odb::MAX_DELTA_DEPTH as usize + 2) {
            // `| 0x80` keeps every written value clear of the 7 fill byte, so
            // each edit is guaranteed to change the payload. (Writing a plain
            // `i` silently no-ops at i == 7, producing two identical payloads,
            // one OID, and a self-referencing chain.)
            cur[i * 8] = (i as u8) | 0x80;
            payloads.push(cur.clone());
        }

        let base_id = Oid::hash(&payloads[0]);
        odb.write_full_chunk(&base_id, &payloads[0]).await.unwrap();

        // Write the chain by hand so it exceeds what the guard now permits —
        // this is a legacy repository, not something the writer can produce.
        let mut prev_id = base_id;
        let mut ids = vec![base_id];
        for payload in &payloads[1..] {
            let id = Oid::hash(payload);
            let delta = DeltaEncoder::encode(&payloads[0], payload);
            let compressed = smart
                .compress_typed(&delta.to_bytes(), CompObjectType::Unknown)
                .unwrap();
            storage
                .put(&format!("chunk-deltas/{}", id.to_hex()), &compressed)
                .await
                .unwrap();
            storage
                .put(
                    &format!("chunk-deltas/{}.meta", id.to_hex()),
                    format!("base:{}", prev_id.to_hex()).as_bytes(),
                )
                .await
                .unwrap();
            ids.push(id);
            prev_id = id;
        }

        let leaf = *ids.last().unwrap();

        // Precondition: the leaf is unreadable — this is the reported bug.
        assert!(
            odb.get_chunk(&leaf).await.is_err(),
            "an over-deep chain must be unreadable before repair, otherwise \
             this test is not reproducing the defect"
        );

        let checker = FsckChecker::new(storage.clone());
        let mut report = FsckReport::new();
        checker.check_chunk_deltas(&mut report).await.unwrap();
        assert!(
            report
                .repairable_issues()
                .iter()
                .any(|i| i.category == IssueCategory::DeepDeltaChain),
            "over-deep chain must be reported as repairable, got: {:?}",
            report.issues
        );

        let repaired = FsckRepair::new(storage.clone())
            .with_odb(Arc::clone(&odb))
            .repair(&report, false)
            .await
            .unwrap();
        assert!(
            repaired.repaired() > 0,
            "repair must report at least one fix"
        );

        // The chunk reads back, and byte-identically.
        let recovered = odb
            .get_chunk(&leaf)
            .await
            .expect("chain must be readable after flattening");
        assert_eq!(
            recovered,
            *payloads.last().unwrap(),
            "flattened chunk must be byte-identical to the original content"
        );
    }

    #[tokio::test]
    async fn test_fsck_repair_without_odb_does_not_claim_success() {
        // A storage-only FsckRepair cannot reconstruct anything. It must say
        // so rather than counting an unfixed issue as repaired (the QA-007
        // "repaired 67 issues while nothing changed" failure mode).
        let full = Oid::hash(b"noodb-full");
        let chain: Vec<Oid> = (0..12)
            .map(|i| Oid::hash(format!("noodb-{i}").as_bytes()))
            .collect();
        let mut entries: Vec<(&Oid, &Oid)> = (0..chain.len() - 1)
            .map(|i| (&chain[i], &chain[i + 1]))
            .collect();
        entries.push((&chain[chain.len() - 1], &full));
        let checker = checker_with_metas(&entries, &[&full]).await;

        let mut report = FsckReport::new();
        checker.check_chunk_deltas(&mut report).await.unwrap();

        let storage = Arc::new(MockBackend::new());
        let repaired = FsckRepair::new(storage)
            .repair(&report, false)
            .await
            .unwrap();
        assert_eq!(
            repaired.repaired(),
            0,
            "without an ODB the flatten repair must report 0, not a false success"
        );
    }

    #[test]
    fn test_fsck_issue_creation() {
        let issue = FsckIssue::new(
            IssueSeverity::Error,
            IssueCategory::ChecksumMismatch,
            "Test issue".to_string(),
        );

        assert_eq!(issue.severity, IssueSeverity::Error);
        assert_eq!(issue.category, IssueCategory::ChecksumMismatch);
        assert!(!issue.repairable);
    }

    #[test]
    fn test_fsck_issue_builder() {
        let oid = Oid::hash(b"test");
        let issue = FsckIssue::new(
            IssueSeverity::Warning,
            IssueCategory::BrokenReference,
            "Test".to_string(),
        )
        .with_oid(oid)
        .with_ref("refs/heads/main".to_string())
        .repairable();

        assert_eq!(issue.oid, Some(oid));
        assert_eq!(issue.ref_name, Some("refs/heads/main".to_string()));
        assert!(issue.repairable);
    }

    #[test]
    fn test_fsck_report() {
        let mut report = FsckReport::new();

        report.add_issue(FsckIssue::new(
            IssueSeverity::Error,
            IssueCategory::ChecksumMismatch,
            "Test".to_string(),
        ));

        assert_eq!(report.total_issues(), 1);
        assert_eq!(report.corrupted_objects, 1);
        assert!(report.has_errors());
    }

    #[test]
    fn test_fsck_options() {
        let full = FsckOptions::full();
        assert!(full.check_dangling);
        assert!(full.verbose);

        let quick = FsckOptions::quick();
        assert!(!quick.check_dangling);
        // The existence walk is what sees deleted objects; quick keeps it.
        assert!(quick.check_connectivity);
    }

    async fn write_ref(storage: &Arc<MockBackend>, name: &str, oid: Oid) {
        let r = Ref::new_direct(name.to_string(), oid);
        let bytes = crate::format::serialize(&r).unwrap();
        storage.put(name, &bytes).await.unwrap();
    }

    #[tokio::test]
    async fn test_fsck_flags_tag_with_missing_target() {
        let storage = Arc::new(MockBackend::new());

        let bogus_target = Oid::hash(b"fsck-tag-missing-target");
        let author = crate::Signature::now("t".to_string(), "t@e".to_string());
        let tag = Tag::new(
            bogus_target,
            ObjectType::Commit,
            "v1.0.0".to_string(),
            author,
            "release".to_string(),
        );
        let tag_bytes = tag.serialize().unwrap();
        let tag_oid = Oid::hash(&tag_bytes);
        storage.put(&tag_oid.to_hex(), &tag_bytes).await.unwrap();
        write_ref(&storage, "refs/tags/v1.0.0", tag_oid).await;

        let checker = FsckChecker::new(storage);
        let mut report = FsckReport::new();
        checker.check_connectivity(&mut report).await.unwrap();

        assert!(
            report
                .issues
                .iter()
                .any(|i| i.category == IssueCategory::BrokenReference
                    && i.message.contains("target")),
            "missing tag target must be reported, got: {:?}",
            report.issues
        );
    }

    #[tokio::test]
    async fn test_fsck_flags_tag_with_wrong_target_type() {
        let storage = Arc::new(MockBackend::new());

        // Target actually stored is a blob, but the tag declares Commit.
        let blob_data = b"just a blob, not a commit";
        let blob_oid = Oid::hash(blob_data);
        storage.put(&blob_oid.to_hex(), blob_data).await.unwrap();

        let author = crate::Signature::now("t".to_string(), "t@e".to_string());
        let tag = Tag::new(
            blob_oid,
            ObjectType::Commit,
            "v1.0.0".to_string(),
            author,
            "release".to_string(),
        );
        let tag_bytes = tag.serialize().unwrap();
        let tag_oid = Oid::hash(&tag_bytes);
        storage.put(&tag_oid.to_hex(), &tag_bytes).await.unwrap();
        write_ref(&storage, "refs/tags/v1.0.0", tag_oid).await;

        let checker = FsckChecker::new(storage);
        let mut report = FsckReport::new();
        checker.check_connectivity(&mut report).await.unwrap();

        assert!(
            report
                .issues
                .iter()
                .any(|i| i.category == IssueCategory::InvalidFormat
                    && i.message.contains("target_type")),
            "target_type mismatch must be reported, got: {:?}",
            report.issues
        );
    }

    #[tokio::test]
    async fn test_fsck_healthy_annotated_tag_is_clean_and_walks_target() {
        let storage = Arc::new(MockBackend::new());

        // Build a real commit -> tree -> blob chain.
        let blob_data = b"file contents";
        let blob_oid = Oid::hash(blob_data);
        storage.put(&blob_oid.to_hex(), blob_data).await.unwrap();

        let mut tree = Tree::new();
        tree.add_entry(crate::TreeEntry::new(
            "a.txt".to_string(),
            crate::FileMode::Regular,
            blob_oid,
        ));
        let tree_bytes = crate::format::serialize(&tree).unwrap();
        let tree_oid = Oid::hash(&tree_bytes);
        storage.put(&tree_oid.to_hex(), &tree_bytes).await.unwrap();

        let author = crate::Signature::now("t".to_string(), "t@e".to_string());
        let commit = Commit::new(tree_oid, author.clone(), author.clone(), "msg".to_string());
        let commit_bytes = crate::format::serialize(&commit).unwrap();
        let commit_oid = Oid::hash(&commit_bytes);
        storage
            .put(&commit_oid.to_hex(), &commit_bytes)
            .await
            .unwrap();

        let tag = Tag::new(
            commit_oid,
            ObjectType::Commit,
            "v1.0.0".to_string(),
            author,
            "release".to_string(),
        );
        let tag_bytes = tag.serialize().unwrap();
        let tag_oid = Oid::hash(&tag_bytes);
        storage.put(&tag_oid.to_hex(), &tag_bytes).await.unwrap();
        write_ref(&storage, "refs/tags/v1.0.0", tag_oid).await;

        let checker = FsckChecker::new(storage);
        let mut report = FsckReport::new();
        checker.check_connectivity(&mut report).await.unwrap();

        assert_eq!(
            report.total_issues(),
            0,
            "healthy annotated tag must produce no issues, got: {:?}",
            report.issues
        );
    }

    /// Builds a minimal but real commit -> tree -> blob chain (one tracked
    /// file "a.txt") with `refs/heads/main` pointing at the commit. Returns
    /// (blob_oid, tree_oid, commit_oid).
    async fn write_referenced_chain(storage: &Arc<MockBackend>) -> (Oid, Oid, Oid) {
        let blob_data = b"referenced contents";
        let blob_oid = Oid::hash(blob_data);
        storage.put(&blob_oid.to_hex(), blob_data).await.unwrap();

        let mut tree = Tree::new();
        tree.add_entry(crate::TreeEntry::new(
            "a.txt".to_string(),
            crate::FileMode::Regular,
            blob_oid,
        ));
        let tree_bytes = crate::format::serialize(&tree).unwrap();
        let tree_oid = Oid::hash(&tree_bytes);
        storage.put(&tree_oid.to_hex(), &tree_bytes).await.unwrap();

        let author = crate::Signature::now("t".to_string(), "t@e".to_string());
        let commit = Commit::new(tree_oid, author.clone(), author, "msg".to_string());
        let commit_bytes = crate::format::serialize(&commit).unwrap();
        let commit_oid = Oid::hash(&commit_bytes);
        storage
            .put(&commit_oid.to_hex(), &commit_bytes)
            .await
            .unwrap();
        write_ref(storage, "refs/heads/main", commit_oid).await;

        (blob_oid, tree_oid, commit_oid)
    }

    // QA-007 regression: `collect_referenced_objects` previously only
    // recognized Commit/Tag objects, never Tree, so every blob reachable
    // only via a tree entry (i.e. every normal tracked file) was wrongly
    // classified dangling and `fsck --repair` deleted it. These four tests
    // pin the fix: a healthy repo loses nothing, a genuine orphan is
    // removed exactly, a packed object is never claimed "repaired", and
    // at-rest chunk corruption (QA-006a) is now detected.

    #[tokio::test]
    async fn test_fsck_repair_leaves_healthy_repo_untouched() {
        let storage = Arc::new(MockBackend::new());
        write_referenced_chain(&storage).await;

        let before: HashSet<String> = storage
            .list_objects("")
            .await
            .unwrap()
            .into_iter()
            .collect();

        let checker = FsckChecker::new(storage.clone());
        let report = checker.check(FsckOptions::full()).await.unwrap();

        let repair = FsckRepair::new(storage.clone());
        let repaired = repair.repair(&report, false).await.unwrap();

        let after: HashSet<String> = storage
            .list_objects("")
            .await
            .unwrap()
            .into_iter()
            .collect();

        assert_eq!(
            repaired.repaired(),
            0,
            "a healthy repo has nothing to repair, got report: {:?}",
            report.issues
        );
        assert_eq!(
            before, after,
            "fsck --repair must not delete anything from a healthy repo"
        );
    }

    #[tokio::test]
    async fn test_fsck_repair_leaves_dangling_blob_to_gc() {
        let storage = Arc::new(MockBackend::new());
        let (blob_oid, tree_oid, commit_oid) = write_referenced_chain(&storage).await;

        // Genuinely dangling: written, never referenced by any tree/commit.
        let orphan_data = b"nobody points at me";
        let orphan_oid = Oid::hash(orphan_data);
        storage
            .put(&orphan_oid.to_hex(), orphan_data)
            .await
            .unwrap();

        // A repair-capable checker is one that knows its repo root: only then
        // can it tell a genuine orphan from a staged/stashed/mid-rebase object
        // and mark findings repairable. The directory is empty of repo state,
        // so every working root resolves to nothing and this test's orphan is
        // judged exactly as before.
        let repo = tempfile::TempDir::new().unwrap();
        let checker = FsckChecker::new(storage.clone()).with_repo_root(repo.path());
        let report = checker.check(FsckOptions::full()).await.unwrap();

        assert!(
            report
                .issues
                .iter()
                .any(|i| i.category == IssueCategory::DanglingObject && i.oid == Some(orphan_oid)),
            "orphan blob must be flagged dangling, got: {:?}",
            report.issues
        );
        assert!(
            !report
                .issues
                .iter()
                .any(|i| i.category == IssueCategory::DanglingObject
                    && (i.oid == Some(blob_oid)
                        || i.oid == Some(tree_oid)
                        || i.oid == Some(commit_oid))),
            "referenced blob/tree/commit must NOT be flagged dangling, got: {:?}",
            report.issues
        );

        // Unreferenced objects belong to `gc`: repair neither plans nor
        // performs anything for them.
        let repair = FsckRepair::new(storage.clone());
        let outcome = repair.repair(&report, false).await.unwrap();
        assert_eq!(
            outcome.repaired(),
            0,
            "repair must leave dangling objects to gc, got report: {:?}",
            report.issues
        );

        assert!(
            storage.exists(&orphan_oid.to_hex()).await.unwrap(),
            "orphan blob must survive repair"
        );
        assert!(
            storage.exists(&blob_oid.to_hex()).await.unwrap(),
            "referenced blob must survive repair"
        );
        assert!(
            storage.exists(&tree_oid.to_hex()).await.unwrap(),
            "tree must survive repair"
        );
        assert!(
            storage.exists(&commit_oid.to_hex()).await.unwrap(),
            "commit must survive repair"
        );
    }

    #[tokio::test]
    async fn test_fsck_repair_refuses_to_delete_packed_object() {
        let storage = Arc::new(MockBackend::new());
        // Simulate a ChecksumMismatch issue for an object that is NOT
        // present as a loose file (i.e. it lives only inside a pack).
        // `storage.delete()` only ever touches the loose path, so repair
        // must not claim success for something it can't actually remove.
        let packed_oid = Oid::hash(b"lives-only-in-a-pack");

        let mut report = FsckReport::new();
        report.add_issue(
            FsckIssue::new(
                IssueSeverity::Error,
                IssueCategory::ChecksumMismatch,
                "simulated corruption in a packed object".to_string(),
            )
            .with_oid(packed_oid)
            .repairable(),
        );

        let repair = FsckRepair::new(storage.clone());
        let repaired = repair.repair(&report, false).await.unwrap();

        assert_eq!(
            repaired.repaired(),
            0,
            "a packed (non-loose) object must never be reported as repaired"
        );
    }

    #[tokio::test]
    async fn test_fsck_detects_flipped_byte_in_loose_chunk() {
        use crate::chunking::ChunkStrategy;

        let storage = Arc::new(MockBackend::new());
        let writer = ObjectDatabase::with_optimizations(
            storage.clone(),
            100,
            Some(ChunkStrategy::Fixed { size: 256 * 1024 }),
            false,
            0,
        );

        // 2MB of varied content so it actually chunks (> 1MB threshold).
        let data: Vec<u8> = (0..2 * 1024 * 1024).map(|i| (i % 251) as u8).collect();
        let blob_oid = writer
            .write_chunked(ObjectType::Blob, &data, "big.bin")
            .await
            .expect("write_chunked should succeed");

        let manifest_key = format!("manifests/{}", blob_oid.to_hex());
        let manifest_bytes = storage.get(&manifest_key).await.unwrap();
        let manifest: crate::chunking::ChunkManifest =
            crate::chunking::ChunkManifest::from_bytes(&manifest_bytes).unwrap();
        let victim = &manifest.chunks[0];
        let chunk_key = format!("chunks/{}", victim.id.to_hex());

        let mut bytes = storage.get(&chunk_key).await.unwrap();
        let mid = bytes.len() / 2;
        bytes[mid] ^= 0xFF;
        storage.put(&chunk_key, &bytes).await.unwrap();

        // Pre-fix, list_all_objects never enumerated manifests/, so
        // blob_oid never reached verify_object and this corruption was
        // invisible (QA-006a).
        let checker = FsckChecker::new(storage);
        let mut report = FsckReport::new();
        checker
            .check_objects(&mut report, &FsckOptions::default())
            .await
            .unwrap();

        assert!(
            report.issues.iter().any(|i| matches!(
                i.category,
                IssueCategory::ChecksumMismatch | IssueCategory::InvalidFormat
            )),
            "corrupted chunk content must be detected by fsck, got: {:?}",
            report.issues
        );

        // FS-2: the issue must name the corrupt *chunk*, not the manifest
        // that references it. Tagged with `blob_oid`, `repair` looked for a
        // loose file at the manifest's OID, found none, and reported the
        // object as "likely packed" — so the corruption was detected and
        // then never repairable.
        let victim_id = victim.id;
        assert!(
            report
                .issues
                .iter()
                .any(|i| i.oid == Some(victim_id) && i.repairable),
            "fsck must report the corrupt chunk {} as a repairable issue, got: {:?}",
            victim_id,
            report.issues
        );
    }

    /// FS-2: detection is only half of it — the reported issue has to be one
    /// `repair` can actually act on, and the count it returns has to be real.
    #[tokio::test]
    async fn fsck_repair_quarantines_the_corrupt_chunk() {
        use crate::chunking::ChunkStrategy;

        let storage = Arc::new(MockBackend::new());
        let writer = ObjectDatabase::with_optimizations(
            storage.clone(),
            100,
            Some(ChunkStrategy::Fixed { size: 256 * 1024 }),
            false,
            0,
        );

        let data: Vec<u8> = (0..2 * 1024 * 1024).map(|i| (i % 251) as u8).collect();
        let blob_oid = writer
            .write_chunked(ObjectType::Blob, &data, "big.bin")
            .await
            .expect("write_chunked should succeed");

        let manifest_bytes = storage
            .get(&format!("manifests/{}", blob_oid.to_hex()))
            .await
            .unwrap();
        let manifest = crate::chunking::ChunkManifest::from_bytes(&manifest_bytes).unwrap();
        let victim_id = manifest.chunks[0].id;
        let chunk_key = format!("chunks/{}", victim_id.to_hex());

        let mut bytes = storage.get(&chunk_key).await.unwrap();
        let mid = bytes.len() / 2;
        bytes[mid] ^= 0xFF;
        storage.put(&chunk_key, &bytes).await.unwrap();

        let checker = FsckChecker::new(storage.clone());
        let mut report = FsckReport::new();
        checker
            .check_objects(&mut report, &FsckOptions::default())
            .await
            .unwrap();

        let repo = tempfile::TempDir::new().unwrap();
        let repair = FsckRepair::new(storage.clone())
            .with_odb(checker.odb())
            .with_repo_root(repo.path());
        let outcome = repair.repair(&report, false).await.unwrap();

        assert_eq!(outcome.quarantined, vec![victim_id]);
        assert!(
            !storage.exists(&chunk_key).await.unwrap(),
            "the corrupt chunk must leave the store so a later fetch can supply a \
             good copy; leaving it means every future read fails identically"
        );
        let kept = repo
            .path()
            .join(".mediagit/quarantine/chunk")
            .join(victim_id.to_hex());
        assert_eq!(
            std::fs::read(kept).unwrap(),
            bytes,
            "the damaged bytes must be kept, not deleted"
        );
    }

    /// "Repaired" is only claimed for what a fresh check no longer reports.
    #[test]
    fn repair_outcome_keeps_only_confirmed_fixes() {
        let (fixed, still_bad) = (Oid::hash(b"fixed"), Oid::hash(b"still bad"));
        let mut outcome = RepairOutcome {
            restored: vec![fixed, still_bad],
            refs_removed: vec!["refs/heads/gone".into(), "refs/heads/stuck".into()],
            quarantined: vec![still_bad],
            ..RepairOutcome::default()
        };
        let mut recheck = FsckReport::new();
        recheck.add_issue(
            FsckIssue::new(
                IssueSeverity::Error,
                IssueCategory::MissingObject,
                "still".to_string(),
            )
            .with_oid(still_bad),
        );
        recheck.add_issue(
            FsckIssue::new(
                IssueSeverity::Error,
                IssueCategory::BrokenReference,
                "stuck".to_string(),
            )
            .with_ref("refs/heads/stuck".to_string()),
        );

        outcome.confirm_against(&recheck);

        assert_eq!(outcome.restored, vec![fixed]);
        assert_eq!(outcome.refs_removed, vec!["refs/heads/gone".to_string()]);
        assert_eq!(outcome.quarantined, vec![still_bad], "containment stands");
        assert_eq!(outcome.repaired(), 2);
    }

    #[tokio::test]
    async fn fsck_repair_without_repo_root_deletes_nothing() {
        let storage = Arc::new(MockBackend::new());
        let oid = Oid::hash(b"damaged");
        storage
            .put(&oid.to_hex(), b"not the real bytes")
            .await
            .unwrap();
        let mut report = FsckReport::new();
        report.add_issue(
            FsckIssue::new(
                IssueSeverity::Error,
                IssueCategory::ChecksumMismatch,
                "damaged".to_string(),
            )
            .with_oid(oid)
            .repairable(),
        );

        let outcome = FsckRepair::new(storage.clone())
            .repair(&report, false)
            .await
            .unwrap();

        assert!(outcome.quarantined.is_empty() && outcome.restored.is_empty());
        assert!(
            storage.exists(&oid.to_hex()).await.unwrap(),
            "with nowhere to quarantine, the object must stay where it is"
        );
    }

    /// Builds a 2 MB chunked blob, then lets the caller damage one chunk
    /// before running `check_objects`. Guards the streaming `verify_object`
    /// against the "fsck is blind to a damaged blob" failure mode.
    async fn fsck_report_after_damaging_chunk(
        damage: impl FnOnce(Arc<MockBackend>, String) -> futures::future::BoxFuture<'static, ()>,
    ) -> FsckReport {
        use crate::chunking::ChunkStrategy;

        let storage = Arc::new(MockBackend::new());
        let writer = ObjectDatabase::with_optimizations(
            storage.clone(),
            100,
            Some(ChunkStrategy::Fixed { size: 256 * 1024 }),
            false,
            0,
        );
        let data: Vec<u8> = (0..2 * 1024 * 1024).map(|i| (i % 251) as u8).collect();
        let blob_oid = writer
            .write_chunked(ObjectType::Blob, &data, "big.bin")
            .await
            .expect("write_chunked should succeed");
        let manifest_bytes = storage
            .get(&format!("manifests/{}", blob_oid.to_hex()))
            .await
            .unwrap();
        let manifest = crate::chunking::ChunkManifest::from_bytes(&manifest_bytes).unwrap();
        let chunk_key = format!("chunks/{}", manifest.chunks[0].id.to_hex());
        damage(storage.clone(), chunk_key).await;

        let checker = FsckChecker::new(storage);
        let mut report = FsckReport::new();
        checker
            .check_objects(&mut report, &FsckOptions::default())
            .await
            .unwrap();
        report
    }

    #[tokio::test]
    async fn fsck_reports_corrupt_chunk_of_chunked_blob() {
        let report = fsck_report_after_damaging_chunk(|storage, key| {
            Box::pin(async move {
                let mut bytes = storage.get(&key).await.unwrap();
                let mid = bytes.len() / 2;
                bytes[mid] ^= 0xFF;
                storage.put(&key, &bytes).await.unwrap();
            })
        })
        .await;
        assert!(report.has_errors(), "corrupt chunk went unreported");
    }

    #[tokio::test]
    async fn fsck_reports_missing_chunk_of_chunked_blob() {
        let report = fsck_report_after_damaging_chunk(|storage, key| {
            Box::pin(async move {
                storage.delete(&key).await.unwrap();
            })
        })
        .await;
        assert!(report.has_errors(), "missing chunk went unreported");
    }

    /// Build a one-commit repo on a LocalBackend and return
    /// (checker, blob_oid, tree_oid, commit_oid, storage).
    async fn repo_with_one_commit(
        tmp: &tempfile::TempDir,
    ) -> (FsckChecker, Oid, Oid, Oid, Arc<dyn StorageBackend>) {
        use crate::{Commit, FileMode, ObjectType, Signature, Tree, TreeEntry};
        use mediagit_storage::LocalBackend;

        let storage: Arc<dyn StorageBackend> =
            Arc::new(LocalBackend::new(tmp.path()).await.unwrap());
        let odb = ObjectDatabase::with_smart_compression(storage.clone(), 1000);

        let blob = odb.write(ObjectType::Blob, b"payload bytes").await.unwrap();
        let mut tree = Tree::new();
        tree.add_entry(TreeEntry::new("a.txt".to_string(), FileMode::Regular, blob));
        let tree_oid = tree.write(&odb).await.unwrap();
        let sig = Signature::now("t".to_string(), "t@e".to_string());
        let commit = Commit::new(tree_oid, sig.clone(), sig, "msg".to_string());
        let commit_oid = commit.write(&odb).await.unwrap();

        let refdb = RefDatabase::new(tmp.path());
        refdb
            .write(&Ref::new_direct("refs/heads/main".to_string(), commit_oid))
            .await
            .unwrap();

        let checker =
            FsckChecker::new_with_refdb(storage.clone(), Some(refdb)).with_repo_root(tmp.path());
        (checker, blob, tree_oid, commit_oid, storage)
    }

    /// A blob deleted from storage must be reported as missing.
    ///
    /// `check_connectivity` walked commits -> parents and checked only that
    /// `commit.tree` existed; it never walked the tree's entries. And
    /// `check_objects` enumerates what IS in storage, so it can never see an
    /// absence. Net effect: deleting every blob in the repo left `fsck`
    /// reporting a clean bill of health.
    #[tokio::test]
    async fn test_fsck_detects_missing_blob_referenced_by_tree() {
        let tmp = tempfile::TempDir::new().unwrap();
        let (checker, blob, _tree, _commit, storage) = repo_with_one_commit(&tmp).await;

        storage.delete(&blob.to_hex()).await.unwrap();
        assert!(
            !storage.exists(&blob.to_hex()).await.unwrap(),
            "precondition: the blob must actually be gone from storage"
        );

        // Default options — NOT `full()`. The hole was reachable on the path
        // `mediagit fsck` takes with no flags.
        let report = checker.check(FsckOptions::default()).await.unwrap();

        let missing: Vec<_> = report
            .issues
            .iter()
            .filter(|i| i.category == IssueCategory::MissingObject)
            .collect();
        assert!(
            !missing.is_empty(),
            "a blob referenced by a reachable tree but absent from storage must \
             be reported, got issues: {:?}",
            report.issues
        );
        assert!(
            missing.iter().any(|i| i.oid == Some(blob)),
            "the report must name the missing blob {}, got: {:?}",
            blob,
            missing
        );
        assert!(report.has_errors());
    }

    /// The healthy-repo half of the guard above: an intact repo must stay
    /// clean. Without this, "report a missing object" could be satisfied by
    /// reporting one unconditionally.
    #[tokio::test]
    async fn test_fsck_intact_repo_reports_no_missing_objects() {
        let tmp = tempfile::TempDir::new().unwrap();
        let (checker, _blob, _tree, _commit, _storage) = repo_with_one_commit(&tmp).await;

        let report = checker.check(FsckOptions::default()).await.unwrap();

        let missing: Vec<_> = report
            .issues
            .iter()
            .filter(|i| i.category == IssueCategory::MissingObject)
            .collect();
        assert!(
            missing.is_empty(),
            "an intact repo must report no missing objects, got: {:?}",
            missing
        );
    }

    /// `gc` protects staged (index) objects from collection; `fsck --full`
    /// classifies them as dangling and marks them repairable, so
    /// `fsck --full --repair` deletes the user's staged work.
    ///
    /// This is the QA-007 data-loss shape with a different root set: `gc`
    /// roots from HEAD + branches + tags + reflog + remotes + stash +
    /// in-progress state + the index, while `fsck` roots from refs alone.
    #[tokio::test]
    async fn test_fsck_dangling_does_not_condemn_staged_objects() {
        use crate::ObjectType;
        use crate::index::{Index, IndexEntry};

        let tmp = tempfile::TempDir::new().unwrap();
        let (checker, _blob, _tree, _commit, storage) = repo_with_one_commit(&tmp).await;

        // Stage a NEW file: `mediagit add` writes the blob and records it in
        // the index, but no commit references it yet.
        let odb = ObjectDatabase::with_smart_compression(storage.clone(), 1000);
        let staged = odb
            .write(ObjectType::Blob, b"staged but uncommitted")
            .await
            .unwrap();
        let mut index = Index::load(tmp.path()).unwrap_or_default();
        index.add_entry(IndexEntry::new(
            std::path::PathBuf::from("staged.txt"),
            staged,
            0o100644,
            22,
            None,
        ));
        index.save(tmp.path()).unwrap();

        let report = checker.check(FsckOptions::full()).await.unwrap();

        let condemned: Vec<_> = report
            .issues
            .iter()
            .filter(|i| i.category == IssueCategory::DanglingObject)
            .filter(|i| i.oid == Some(staged))
            .collect();
        assert!(
            condemned.is_empty(),
            "a staged-but-uncommitted object is protected work, not dangling: \
             `gc` protects it too. Got: {:?}",
            condemned
        );
    }

    /// The other half of the guard above. Protecting the working roots must
    /// not degenerate into protecting everything — a genuinely orphaned
    /// object still has to be found and still has to be repairable, or
    /// `fsck --full --repair` becomes a no-op and the previous test passes
    /// vacuously.
    #[tokio::test]
    async fn test_fsck_still_condemns_genuinely_orphaned_objects() {
        use crate::ObjectType;

        let tmp = tempfile::TempDir::new().unwrap();
        let (checker, _blob, _tree, _commit, storage) = repo_with_one_commit(&tmp).await;

        // Written straight to the ODB: no ref, no tree, no index entry, no
        // stash, no reflog — garbage by every root set.
        let odb = ObjectDatabase::with_smart_compression(storage.clone(), 1000);
        let orphan = odb
            .write(ObjectType::Blob, b"referenced by absolutely nothing")
            .await
            .unwrap();

        let report = checker.check(FsckOptions::full()).await.unwrap();

        let condemned: Vec<_> = report
            .issues
            .iter()
            .filter(|i| i.category == IssueCategory::DanglingObject && i.oid == Some(orphan))
            .collect();
        assert!(
            !condemned.is_empty(),
            "a truly unreferenced object must still be reported dangling, else \
             the working-root protection has swallowed the whole check"
        );
        assert!(
            condemned.iter().all(|i| !i.repairable),
            "dangling findings are for `gc`; `--repair` must never act on them, got: {:?}",
            condemned
        );
    }

    /// A checker built without a repo root cannot tell staged work from
    /// garbage, so it must report dangling objects without marking them
    /// repairable — `--repair` must not be able to act on a root set known
    /// to be narrower than gc's.
    #[tokio::test]
    async fn test_fsck_without_repo_root_reports_but_never_condemns() {
        use crate::ObjectType;

        let tmp = tempfile::TempDir::new().unwrap();
        let (_checker, _blob, _tree, _commit, storage) = repo_with_one_commit(&tmp).await;

        let odb = ObjectDatabase::with_smart_compression(storage.clone(), 1000);
        let orphan = odb.write(ObjectType::Blob, b"orphan").await.unwrap();

        // Same repo, but the checker is given no repo root.
        let refdb = RefDatabase::new(tmp.path());
        let blind = FsckChecker::new_with_refdb(storage.clone(), Some(refdb));
        let report = blind.check(FsckOptions::full()).await.unwrap();

        let found: Vec<_> = report
            .issues
            .iter()
            .filter(|i| i.category == IssueCategory::DanglingObject && i.oid == Some(orphan))
            .collect();
        assert!(
            !found.is_empty(),
            "the finding must still be reported — it is the deletion that is unsafe, \
             not the observation"
        );
        assert!(
            found.iter().all(|i| !i.repairable),
            "without a repo root, staged/stashed objects are indistinguishable from \
             garbage, so nothing may be marked repairable, got: {:?}",
            found
        );
    }

    fn missing_oids(report: &FsckReport) -> Vec<Oid> {
        report
            .issues
            .iter()
            .filter(|i| i.category == IssueCategory::MissingObject)
            .filter_map(|i| i.oid)
            .collect()
    }

    /// A reachable commit whose tree object is gone: `fsck` with default
    /// options must name the tree's commit as having a missing tree.
    #[tokio::test]
    async fn test_fsck_detects_missing_tree_of_commit() {
        let tmp = tempfile::TempDir::new().unwrap();
        let (checker, _blob, tree, commit, storage) = repo_with_one_commit(&tmp).await;

        storage.delete(&tree.to_hex()).await.unwrap();
        assert!(!storage.exists(&tree.to_hex()).await.unwrap());

        let report = checker.check(FsckOptions::default()).await.unwrap();

        assert!(
            report
                .issues
                .iter()
                .any(|i| i.category == IssueCategory::MissingObject
                    && i.oid == Some(commit)
                    && i.message.contains(&tree.to_string())),
            "a missing tree must be reported against its commit, got: {:?}",
            report.issues
        );
        assert!(report.has_errors());
    }

    /// One-commit repo whose only file is a 2 MB chunked blob (manifest +
    /// chunks). Returns (checker, blob_oid, first_chunk_key, storage).
    async fn repo_with_chunked_blob(
        tmp: &tempfile::TempDir,
    ) -> (FsckChecker, Oid, String, Arc<dyn StorageBackend>) {
        use crate::chunking::{ChunkManifest, ChunkStrategy};
        use crate::{Commit, FileMode, Signature, Tree, TreeEntry};
        use mediagit_storage::LocalBackend;

        let storage: Arc<dyn StorageBackend> =
            Arc::new(LocalBackend::new(tmp.path()).await.unwrap());
        let odb = ObjectDatabase::with_optimizations(
            storage.clone(),
            100,
            Some(ChunkStrategy::Fixed { size: 256 * 1024 }),
            false,
            0,
        );
        let data: Vec<u8> = (0..2 * 1024 * 1024).map(|i| (i % 251) as u8).collect();
        let blob = odb
            .write_chunked(ObjectType::Blob, &data, "big.bin")
            .await
            .unwrap();
        let manifest_bytes = storage
            .get(&format!("manifests/{}", blob.to_hex()))
            .await
            .unwrap();
        let manifest = ChunkManifest::from_bytes(&manifest_bytes).unwrap();
        let chunk_key = format!("chunks/{}", manifest.chunks[0].id.to_hex());

        let mut tree = Tree::new();
        tree.add_entry(TreeEntry::new(
            "big.bin".to_string(),
            FileMode::Regular,
            blob,
        ));
        let tree_oid = tree.write(&odb).await.unwrap();
        let sig = Signature::now("t".to_string(), "t@e".to_string());
        let commit = Commit::new(tree_oid, sig.clone(), sig, "msg".to_string());
        let commit_oid = commit.write(&odb).await.unwrap();

        let refdb = RefDatabase::new(tmp.path());
        refdb
            .write(&Ref::new_direct("refs/heads/main".to_string(), commit_oid))
            .await
            .unwrap();
        let checker = FsckChecker::new_with_refdb(storage.clone(), Some(refdb));
        (checker, blob, chunk_key, storage)
    }

    /// Intact chunked repo stays clean, so the two tests below cannot pass
    /// by reporting unconditionally.
    #[tokio::test]
    async fn test_fsck_intact_chunked_repo_is_clean() {
        let tmp = tempfile::TempDir::new().unwrap();
        let (checker, _blob, _chunk, _storage) = repo_with_chunked_blob(&tmp).await;
        let report = checker.check(FsckOptions::default()).await.unwrap();
        assert!(!report.has_errors(), "got: {:?}", report.issues);
    }

    /// A chunked blob's manifest is its identity: delete it and the tree
    /// entry points at nothing.
    #[tokio::test]
    async fn test_fsck_detects_missing_manifest_of_chunked_blob() {
        let tmp = tempfile::TempDir::new().unwrap();
        let (checker, blob, _chunk, storage) = repo_with_chunked_blob(&tmp).await;

        storage
            .delete(&format!("manifests/{}", blob.to_hex()))
            .await
            .unwrap();

        let report = checker.check(FsckOptions::default()).await.unwrap();

        assert!(
            missing_oids(&report).contains(&blob),
            "a chunked blob with no manifest must be reported missing, got: {:?}",
            report.issues
        );
    }

    /// Assert the report names `hex` as a missing object.
    fn assert_names_missing(report: &FsckReport, hex: &str) {
        assert!(
            report
                .issues
                .iter()
                .any(|i| i.category == IssueCategory::MissingObject
                    && i.severity == IssueSeverity::Error
                    && i.oid.is_some_and(|o| o.to_hex() == hex)
                    && i.message.contains(hex)),
            "expected a MissingObject error naming {hex}, got: {:?}",
            report.issues
        );
    }

    /// With the manifest intact but a chunk gone, the default `check` must
    /// fail and name that chunk.
    #[tokio::test]
    async fn test_fsck_detects_missing_chunk_of_chunked_blob_end_to_end() {
        let tmp = tempfile::TempDir::new().unwrap();
        let (checker, _blob, chunk_key, storage) = repo_with_chunked_blob(&tmp).await;

        storage.delete(&chunk_key).await.unwrap();
        assert!(!storage.exists(&chunk_key).await.unwrap());

        let report = checker.check(FsckOptions::default()).await.unwrap();

        assert!(report.has_errors());
        assert_names_missing(&report, chunk_key.strip_prefix("chunks/").unwrap());
    }

    /// The walk, not `check_objects`, owns chunk existence: with objects off
    /// (and with a tiny `max_objects`) a missing chunk is still reported.
    #[tokio::test]
    async fn test_fsck_missing_chunk_found_without_check_objects() {
        for (check_objects, max_objects) in [(false, 0), (true, 1)] {
            let tmp = tempfile::TempDir::new().unwrap();
            let (checker, _blob, chunk_key, storage) = repo_with_chunked_blob(&tmp).await;
            storage.delete(&chunk_key).await.unwrap();

            let options = FsckOptions {
                check_objects,
                max_objects,
                ..FsckOptions::default()
            };
            let report = checker.check(options).await.unwrap();
            assert_names_missing(&report, chunk_key.strip_prefix("chunks/").unwrap());
        }
    }

    /// `quick()` must see a deleted blob, tree and manifest: it used to turn
    /// the walk off, so every one of these passed.
    #[tokio::test]
    async fn test_fsck_quick_detects_missing_blob() {
        let tmp = tempfile::TempDir::new().unwrap();
        let (checker, blob, _tree, _commit, storage) = repo_with_one_commit(&tmp).await;
        storage.delete(&blob.to_hex()).await.unwrap();

        let report = checker.check(FsckOptions::quick()).await.unwrap();
        assert_names_missing(&report, &blob.to_hex());
    }

    #[tokio::test]
    async fn test_fsck_quick_detects_missing_tree() {
        let tmp = tempfile::TempDir::new().unwrap();
        let (checker, _blob, tree, _commit, storage) = repo_with_one_commit(&tmp).await;
        storage.delete(&tree.to_hex()).await.unwrap();

        let report = checker.check(FsckOptions::quick()).await.unwrap();
        assert!(
            report
                .issues
                .iter()
                .any(|i| i.category == IssueCategory::MissingObject
                    && i.message.contains(&tree.to_string())),
            "got: {:?}",
            report.issues
        );
    }

    #[tokio::test]
    async fn test_fsck_quick_detects_missing_commit() {
        let tmp = tempfile::TempDir::new().unwrap();
        let (checker, _blob, _tree, commit, storage) = repo_with_one_commit(&tmp).await;
        storage.delete(&commit.to_hex()).await.unwrap();

        let report = checker.check(FsckOptions::quick()).await.unwrap();
        assert!(report.has_errors(), "got: {:?}", report.issues);
    }

    #[tokio::test]
    async fn test_fsck_quick_detects_missing_manifest() {
        let tmp = tempfile::TempDir::new().unwrap();
        let (checker, blob, _chunk, storage) = repo_with_chunked_blob(&tmp).await;
        storage
            .delete(&format!("manifests/{}", blob.to_hex()))
            .await
            .unwrap();

        let report = checker.check(FsckOptions::quick()).await.unwrap();
        assert_names_missing(&report, &blob.to_hex());
    }

    /// Commit `entries` (path, blob) on `storage`, return a cold checker.
    async fn commit_entries(
        storage: &Arc<dyn StorageBackend>,
        odb: &ObjectDatabase,
        tmp: &tempfile::TempDir,
        entries: &[(&str, Oid)],
    ) -> FsckChecker {
        use crate::{FileMode, Signature, TreeEntry};
        let mut tree = Tree::new();
        for (name, blob) in entries {
            tree.add_entry(TreeEntry::new(name.to_string(), FileMode::Regular, *blob));
        }
        let tree_oid = tree.write(odb).await.unwrap();
        let sig = Signature::now("t".to_string(), "t@e".to_string());
        let commit = Commit::new(tree_oid, sig.clone(), sig, "m".to_string());
        let commit_oid = commit.write(odb).await.unwrap();
        let refdb = RefDatabase::new(tmp.path());
        refdb
            .write(&Ref::new_direct("refs/heads/main".to_string(), commit_oid))
            .await
            .unwrap();
        FsckChecker::new_with_refdb(storage.clone(), Some(refdb))
    }

    fn assert_no_missing(report: &FsckReport) {
        assert!(
            !report
                .issues
                .iter()
                .any(|i| i.category == IssueCategory::MissingObject),
            "false 'missing' on a present object: {:?}",
            report.issues
        );
    }

    /// A blob stored as `deltas/<oid>` + `.meta` has no bare key and no
    /// manifest; `ObjectDatabase::exists` does not know it (core.rs `exists`
    /// probes cache, loose key, manifest, packs only). The walk must still
    /// find it.
    #[tokio::test]
    async fn test_fsck_walk_sees_delta_stored_blob() {
        use mediagit_storage::LocalBackend;
        let tmp = tempfile::TempDir::new().unwrap();
        let storage: Arc<dyn StorageBackend> =
            Arc::new(LocalBackend::new(tmp.path()).await.unwrap());
        let odb = ObjectDatabase::new(storage.clone(), 100);

        let mut base = vec![b'a'; 10_000];
        base.extend_from_slice(b"UNIQUE_TAIL_BASE");
        let base_oid = odb.write(ObjectType::Blob, &base).await.unwrap();
        let mut target = vec![b'a'; 10_000];
        target.extend_from_slice(b"UNIQUE_TAIL_TARG");
        let delta_oid = odb
            .write_delta_against_base(ObjectType::Blob, &target, "f.bin", base_oid)
            .await
            .unwrap()
            .expect("fixture: delta must be accepted");
        assert!(
            storage
                .exists(&format!("deltas/{}.meta", delta_oid.to_hex()))
                .await
                .unwrap()
                && !storage.exists(&delta_oid.to_hex()).await.unwrap(),
            "fixture: blob must be delta-stored only"
        );

        let checker = commit_entries(
            &storage,
            &odb,
            &tmp,
            &[("base.bin", base_oid), ("target.bin", delta_oid)],
        )
        .await;
        for options in [FsckOptions::default(), FsckOptions::quick()] {
            assert_no_missing(&checker.check(options).await.unwrap());
        }
    }

    /// After `repack(remove_loose)` every blob, manifest-owned chunk and tree
    /// lives only in a pack; a clean repository must stay clean.
    #[tokio::test]
    async fn test_fsck_walk_clean_on_packed_only_repo() {
        use crate::chunking::ChunkStrategy;
        use mediagit_storage::LocalBackend;
        let tmp = tempfile::TempDir::new().unwrap();
        let storage: Arc<dyn StorageBackend> =
            Arc::new(LocalBackend::new(tmp.path()).await.unwrap());
        let odb = ObjectDatabase::with_optimizations(
            storage.clone(),
            100,
            Some(ChunkStrategy::Fixed { size: 256 * 1024 }),
            false,
            0,
        );
        let small = odb.write(ObjectType::Blob, b"small").await.unwrap();
        let data: Vec<u8> = (0..2 * 1024 * 1024).map(|i| (i % 251) as u8).collect();
        let big = odb
            .write_chunked(ObjectType::Blob, &data, "big.bin")
            .await
            .unwrap();
        let checker =
            commit_entries(&storage, &odb, &tmp, &[("s.txt", small), ("big.bin", big)]).await;

        odb.repack(10_000, true).await.unwrap();
        let manifest = odb.get_chunk_manifest(&big).await.unwrap().unwrap();
        assert!(
            !storage
                .exists(&format!("chunks/{}", manifest.chunks[0].id.to_hex()))
                .await
                .unwrap(),
            "fixture: chunks must be packed-only"
        );

        for options in [FsckOptions::default(), FsckOptions::quick()] {
            let report = checker.check(options).await.unwrap();
            assert!(!report.has_errors(), "got: {:?}", report.issues);
        }
    }

    /// Counts `exists` probes per key; everything else is the mock.
    #[derive(Debug)]
    struct CountingBackend {
        inner: MockBackend,
        exists_calls: std::sync::Mutex<std::collections::HashMap<String, usize>>,
    }

    #[async_trait::async_trait]
    impl StorageBackend for CountingBackend {
        async fn get(&self, key: &str) -> anyhow::Result<Vec<u8>> {
            self.inner.get(key).await
        }
        async fn put(&self, key: &str, data: &[u8]) -> anyhow::Result<()> {
            self.inner.put(key, data).await
        }
        async fn exists(&self, key: &str) -> anyhow::Result<bool> {
            *self
                .exists_calls
                .lock()
                .unwrap()
                .entry(key.to_string())
                .or_default() += 1;
            self.inner.exists(key).await
        }
        async fn delete(&self, key: &str) -> anyhow::Result<()> {
            self.inner.delete(key).await
        }
        async fn list_objects(&self, prefix: &str) -> anyhow::Result<Vec<String>> {
            self.inner.list_objects(prefix).await
        }
        async fn head(&self, key: &str) -> anyhow::Result<Option<u64>> {
            self.inner.head(key).await
        }
    }

    /// Many commits that share the same files must cost one probe per
    /// distinct blob and chunk, not one per commit.
    #[tokio::test]
    async fn test_fsck_walk_probes_each_blob_and_chunk_once() {
        use crate::chunking::ChunkStrategy;
        use crate::{FileMode, Signature, TreeEntry};
        let backend = Arc::new(CountingBackend {
            inner: MockBackend::new(),
            exists_calls: Default::default(),
        });
        let storage: Arc<dyn StorageBackend> = backend.clone();
        let odb = ObjectDatabase::with_optimizations(
            storage.clone(),
            100,
            Some(ChunkStrategy::Fixed { size: 256 * 1024 }),
            false,
            0,
        );
        let a = odb.write(ObjectType::Blob, b"shared a").await.unwrap();
        let b = odb.write(ObjectType::Blob, b"shared b").await.unwrap();
        let data: Vec<u8> = (0..2 * 1024 * 1024).map(|i| (i % 251) as u8).collect();
        let big = odb
            .write_chunked(ObjectType::Blob, &data, "big.bin")
            .await
            .unwrap();
        // Shares every chunk but the last with `big`: chunk dedup must work
        // across different blobs, not only across commits.
        let mut data2 = data.clone();
        *data2.last_mut().unwrap() ^= 0xFF;
        let big2 = odb
            .write_chunked(ObjectType::Blob, &data2, "big2.bin")
            .await
            .unwrap();
        let chunk_ids: Vec<Oid> = odb
            .get_chunk_manifest(&big)
            .await
            .unwrap()
            .unwrap()
            .chunks
            .iter()
            .map(|c| c.id)
            .collect();
        assert!(chunk_ids.len() > 2, "fixture: file must be chunked");

        let sig = Signature::now("t".to_string(), "t@e".to_string());
        let mut parent: Vec<Oid> = Vec::new();
        for i in 0..6 {
            let unique = odb
                .write(ObjectType::Blob, format!("unique {i}").as_bytes())
                .await
                .unwrap();
            let mut tree = Tree::new();
            for (name, oid) in [
                ("a", a),
                ("b", b),
                ("big.bin", big),
                ("big2.bin", big2),
                ("u", unique),
            ] {
                tree.add_entry(TreeEntry::new(name.to_string(), FileMode::Regular, oid));
            }
            let tree_oid = tree.write(&odb).await.unwrap();
            let commit = Commit::with_parents(
                tree_oid,
                parent.clone(),
                sig.clone(),
                sig.clone(),
                "m".into(),
            );
            parent = vec![commit.write(&odb).await.unwrap()];
        }
        let tmp = tempfile::TempDir::new().unwrap();
        let refdb = RefDatabase::new(tmp.path());
        refdb
            .write(&Ref::new_direct("refs/heads/main".to_string(), parent[0]))
            .await
            .unwrap();
        let checker = FsckChecker::new_with_refdb(storage, Some(refdb));
        backend.exists_calls.lock().unwrap().clear();

        let report = checker
            .check(FsckOptions {
                check_objects: false,
                check_refs: false,
                check_dangling: false,
                check_chunk_deltas: false,
                check_connectivity: true,
                ..FsckOptions::default()
            })
            .await
            .unwrap();
        assert!(!report.has_errors(), "got: {:?}", report.issues);

        let calls = backend.exists_calls.lock().unwrap();
        for blob in [a, b] {
            assert_eq!(calls.get(&blob.to_hex()), Some(&1), "blob {blob}");
        }
        for chunk in chunk_ids {
            assert_eq!(
                calls.get(&format!("chunks/{}", chunk.to_hex())),
                Some(&1),
                "chunk {chunk}"
            );
        }
    }
}
