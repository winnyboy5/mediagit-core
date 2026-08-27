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
use crate::{Commit, ObjectType, Oid, Ref, RefDatabase, RefType, Tag, Tree};
use mediagit_storage::StorageBackend;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
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

    /// Create options for a quick check (objects and refs only)
    pub fn quick() -> Self {
        Self {
            check_objects: true,
            check_refs: true,
            check_connectivity: false,
            check_dangling: false,
            check_chunk_deltas: false,
            max_objects: 0,
            verbose: false,
        }
    }
}

/// FSCK integrity checker
pub struct FsckChecker {
    /// Storage backend for file operations
    storage: Arc<dyn StorageBackend>,
    /// Object database for reading and verifying objects
    odb: Arc<ObjectDatabase>,
    /// Reference database for listing refs
    refdb: Option<RefDatabase>,
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
        }
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
        match self.odb.read(oid).await {
            Ok(_data) => {
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
        let mut visited = HashSet::new();
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
            visited.len(),
            referenced_objects.len()
        );

        Ok(())
    }

    /// Traverse commit graph from a commit
    fn traverse_commit<'a>(
        &'a self,
        oid: &'a Oid,
        visited: &'a mut HashSet<Oid>,
        referenced_objects: &'a mut HashSet<Oid>,
        report: &'a mut FsckReport,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = anyhow::Result<()>> + 'a>> {
        Box::pin(async move {
            // Detect circular references
            if visited.contains(oid) {
                return Ok(());
            }
            visited.insert(*oid);
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

            // Check tree exists
            referenced_objects.insert(commit.tree);
            if !self.object_exists(&commit.tree).await? {
                report.add_issue(
                    FsckIssue::new(
                        IssueSeverity::Error,
                        IssueCategory::MissingObject,
                        format!("Commit {} references missing tree {}", oid, commit.tree),
                    )
                    .with_oid(*oid),
                );
            }

            // Traverse parent commits
            for parent in &commit.parents {
                referenced_objects.insert(*parent);
                self.traverse_commit(parent, visited, referenced_objects, report)
                    .await?;
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
        visited: &mut HashSet<Oid>,
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

        visited.insert(oid);
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

        // Find dangling objects
        for oid in all_objects {
            if !referenced.contains(&oid) {
                report.add_issue(
                    FsckIssue::new(
                        IssueSeverity::Info,
                        IssueCategory::DanglingObject,
                        format!("Object {} is not referenced by any commit", oid),
                    )
                    .with_oid(oid)
                    .repairable(),
                );
            }
        }

        Ok(())
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
                match crate::format::deserialize::<Commit>(&data) {
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
                        match crate::format::deserialize::<Tag>(&data) {
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
                                if let Ok(tree) = crate::format::deserialize::<Tree>(&data) {
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
}

impl FsckRepair {
    /// Create a new FSCK repair tool
    ///
    /// Repairs that need to reconstruct content (see
    /// [`FsckRepair::with_odb`]) are unavailable on an instance built this
    /// way — `repair` reports them instead of claiming success.
    pub fn new(storage: Arc<dyn StorageBackend>) -> Self {
        Self { storage, odb: None }
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
    /// Number of issues successfully repaired
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
    ///     let fixed = repair.repair(&report, false).await?;
    ///     println!("Repaired {} issues", fixed);
    /// }
    /// # Ok(())
    /// # }
    /// ```
    pub async fn repair(&self, report: &FsckReport, dry_run: bool) -> anyhow::Result<u64> {
        info!(
            dry_run = dry_run,
            repairable = report.repairable_issues().len(),
            "Starting FSCK repair"
        );

        let mut repaired = 0;

        for issue in report.repairable_issues() {
            match issue.category {
                IssueCategory::ChecksumMismatch => {
                    if let Some(oid) = issue.oid
                        && self.repair_corrupted_object(&oid, dry_run).await?
                    {
                        repaired += 1;
                    }
                }
                IssueCategory::BrokenReference => {
                    if let Some(ref_name) = &issue.ref_name
                        && self.repair_broken_reference(ref_name, dry_run).await?
                    {
                        repaired += 1;
                    }
                }
                IssueCategory::DanglingObject => {
                    if let Some(oid) = issue.oid
                        && self.remove_dangling_object(&oid, dry_run).await?
                    {
                        repaired += 1;
                    }
                }
                IssueCategory::DeepDeltaChain => {
                    if let Some(oid) = issue.oid
                        && self.flatten_deep_chunk_chain(&oid, dry_run).await?
                    {
                        repaired += 1;
                    }
                }
                _ => {
                    warn!("No repair strategy for category: {:?}", issue.category);
                }
            }
        }

        info!(repaired = repaired, "FSCK repair complete");
        Ok(repaired)
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

    /// Repair a corrupted object by removing it
    ///
    /// `storage.delete(&key)` only touches the loose object path. If `oid`
    /// isn't present there — e.g. it lives inside a pack — the delete is a
    /// silent no-op, but the old code still returned `Ok(true)` and the
    /// caller counted it as repaired (QA-007: "Successfully repaired 67
    /// issues" while nothing changed, because the objects were packed).
    /// Now: only claim success when a loose file actually existed to remove.
    async fn repair_corrupted_object(&self, oid: &Oid, dry_run: bool) -> anyhow::Result<bool> {
        // Use oid.to_hex() - LocalBackend handles "objects/" prefix and sharding
        let key = oid.to_hex();

        // FS-2: a corrupt *chunk* is reported under its own OID, and chunks
        // live at `chunks/<hex>`, not the loose object path. Handle that first
        // — otherwise the loose probe below misses, and the operator is told
        // the chunk is "likely packed" when it is sitting right there.
        let chunk_key = format!("chunks/{}", oid.to_hex());
        if self.storage.exists(&chunk_key).await.unwrap_or(false) {
            if dry_run {
                info!("[DRY RUN] Would remove corrupted chunk: {}", oid);
                return Ok(true);
            }
            // Removing the bad bytes is what lets the next fetch/pull supply a
            // good copy; keeping them guarantees every future read fails the
            // same way. Any delta sidecar routing *to* this id goes with it,
            // or the chunk would resolve back to the payload just deleted.
            warn!("Removing corrupted chunk: {}", oid);
            self.storage.delete(&chunk_key).await?;
            let _ = self
                .storage
                .delete(&format!("chunk-deltas/{}.meta", oid.to_hex()))
                .await;
            let _ = self
                .storage
                .delete(&format!("chunk-deltas/{}", oid.to_hex()))
                .await;
            return Ok(true);
        }

        if !self.storage.exists(&key).await.unwrap_or(false) {
            // Distinguish the two reasons the loose probe misses. Reporting a
            // chunked manifest as "likely packed" sent operators to
            // `gc --repack`, which cannot help.
            let manifest_key = format!("manifests/{}", oid.to_hex());
            if self.storage.exists(&manifest_key).await.unwrap_or(false) {
                warn!(
                    "Cannot remove corrupted object {}: it is a chunked-blob manifest. \
                     The damage is in its chunks — re-run fsck so they are reported \
                     individually, or re-fetch the object.",
                    oid
                );
            } else {
                warn!(
                    "Cannot remove corrupted object {}: not present as a loose file \
                     (likely packed); requires a repack, not a targeted delete",
                    oid
                );
            }
            return Ok(false);
        }

        if dry_run {
            info!("[DRY RUN] Would remove corrupted object: {}", oid);
            return Ok(true);
        }

        warn!("Removing corrupted object: {}", oid);
        self.storage.delete(&key).await?;
        Ok(true)
    }

    /// Repair a broken reference by removing it
    async fn repair_broken_reference(&self, ref_name: &str, dry_run: bool) -> anyhow::Result<bool> {
        if dry_run {
            info!("[DRY RUN] Would remove broken reference: {}", ref_name);
            return Ok(true);
        }

        warn!("Removing broken reference: {}", ref_name);
        self.storage.delete(ref_name).await?;
        Ok(true)
    }

    /// Remove a dangling object
    ///
    /// Same packed-object honesty check as `repair_corrupted_object`: a
    /// dangling packed object can't be removed with a bare loose delete, so
    /// don't claim it was repaired.
    async fn remove_dangling_object(&self, oid: &Oid, dry_run: bool) -> anyhow::Result<bool> {
        // Use oid.to_hex() - LocalBackend handles "objects/" prefix and sharding
        let key = oid.to_hex();

        if !self.storage.exists(&key).await.unwrap_or(false) {
            warn!(
                "Cannot remove dangling object {}: not present as a loose file \
                 (likely packed); requires a repack, not a targeted delete",
                oid
            );
            return Ok(false);
        }

        if dry_run {
            info!("[DRY RUN] Would remove dangling object: {}", oid);
            return Ok(true);
        }

        info!("Removing dangling object: {}", oid);
        self.storage.delete(&key).await?;
        Ok(true)
    }
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
        assert!(repaired > 0, "repair must report at least one fix");

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
            repaired, 0,
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
        assert!(!quick.check_connectivity);
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
            repaired, 0,
            "a healthy repo has nothing to repair, got report: {:?}",
            report.issues
        );
        assert_eq!(
            before, after,
            "fsck --repair must not delete anything from a healthy repo"
        );
    }

    #[tokio::test]
    async fn test_fsck_repair_removes_only_genuine_dangling_blob() {
        let storage = Arc::new(MockBackend::new());
        let (blob_oid, tree_oid, commit_oid) = write_referenced_chain(&storage).await;

        // Genuinely dangling: written, never referenced by any tree/commit.
        let orphan_data = b"nobody points at me";
        let orphan_oid = Oid::hash(orphan_data);
        storage
            .put(&orphan_oid.to_hex(), orphan_data)
            .await
            .unwrap();

        let checker = FsckChecker::new(storage.clone());
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

        let repair = FsckRepair::new(storage.clone());
        let repaired = repair.repair(&report, false).await.unwrap();
        assert_eq!(
            repaired, 1,
            "exactly the orphan must be repaired, got report: {:?}",
            report.issues
        );

        assert!(
            !storage.exists(&orphan_oid.to_hex()).await.unwrap(),
            "orphan blob must be deleted"
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
            repaired, 0,
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
    async fn fsck_repair_removes_the_corrupt_chunk_and_counts_it() {
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

        let repair = FsckRepair::new(storage.clone()).with_odb(checker.odb());
        let repaired = repair.repair(&report, false).await.unwrap();

        assert!(
            repaired > 0,
            "a corrupt loose chunk is removable, so repair must not report 0; \
             reporting 0 here is the '✅ repaired 0 issues' failure"
        );
        assert!(
            !storage.exists(&chunk_key).await.unwrap(),
            "the corrupt chunk must be gone so a later fetch can supply a good \
             copy; leaving it means every future read fails identically"
        );
    }
}
