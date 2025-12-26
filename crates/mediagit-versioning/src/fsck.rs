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
//! - **Checksum verification**: Verify SHA-256 hashes match object content
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
//!     let storage = Arc::new(LocalBackend::new("/path/to/repo")?);
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

use crate::{Commit, Oid, Ref, RefType};
use crate::odb::ObjectDatabase;
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
    pub fn new(
        severity: IssueSeverity,
        category: IssueCategory,
        message: String,
    ) -> Self {
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
        self.issues.iter().any(|i| i.severity == IssueSeverity::Error)
    }

    /// Get issues by severity
    pub fn issues_by_severity(&self, severity: IssueSeverity) -> Vec<&FsckIssue> {
        self.issues.iter().filter(|i| i.severity == severity).collect()
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
}

impl FsckChecker {
    /// Create a new FSCK checker
    pub fn new(storage: Arc<dyn StorageBackend>) -> Self {
        // Create ODB with smart compression to handle all compression types
        let odb = ObjectDatabase::with_smart_compression(storage.clone(), 10_000_000); // 10MB cache
        Self {
            storage,
            odb: Arc::new(odb),
        }
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
    /// # let storage = Arc::new(LocalBackend::new("/tmp")?);
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
        let objects = self.list_all_objects().await?;
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
                if error_msg.contains("integrity check failed") {
                    // Checksum mismatch - object is corrupt
                    report.add_issue(
                        FsckIssue::new(
                            IssueSeverity::Error,
                            IssueCategory::ChecksumMismatch,
                            format!("Checksum mismatch: {}", e),
                        )
                        .with_oid(*oid)
                        .repairable(),
                    );
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
                                    format!("Reference {} points to missing commit {}", r.name, oid),
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
                                    format!("Symbolic reference {} points to missing ref {}", r.name, target),
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
                self.traverse_commit(
                    &oid,
                    &mut visited,
                    &mut referenced_objects,
                    report,
                )
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

        // Try to read commit
        // Use oid.to_hex() - LocalBackend handles "objects/" prefix and sharding
        let key = oid.to_hex();
        let data = match self.storage.get(&key).await {
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
        let commit: Commit = match bincode::deserialize(&data) {
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

    /// Detect dangling (unreferenced) objects
    async fn check_dangling(&self, report: &mut FsckReport) -> anyhow::Result<()> {
        debug!("Detecting dangling objects");

        // Get all objects
        let all_objects = self.list_all_objects().await?;

        // Get all referenced objects
        let refs = self.list_all_refs().await?;
        let mut referenced = HashSet::new();

        for r in refs {
            if let Some(oid) = r.oid {
                let mut visited = HashSet::new();
                self.collect_referenced_objects(&oid, &mut visited, &mut referenced)
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
    fn collect_referenced_objects<'a>(
        &'a self,
        oid: &'a Oid,
        visited: &'a mut HashSet<Oid>,
        referenced: &'a mut HashSet<Oid>,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = anyhow::Result<()>> + 'a>> {
        Box::pin(async move {
        if visited.contains(oid) {
            return Ok(());
        }
        visited.insert(*oid);
        referenced.insert(*oid);

        // Use oid.to_hex() - LocalBackend handles "objects/" prefix and sharding
        let key = oid.to_hex();
        if let Ok(data) = self.storage.get(&key).await {
            if let Ok(commit) = bincode::deserialize::<Commit>(&data) {
                referenced.insert(commit.tree);
                for parent in commit.parents {
                    self.collect_referenced_objects(&parent, visited, referenced)
                        .await?;
                }
            }
        }

        Ok(())
        })
    }

    /// List all objects in storage
    async fn list_all_objects(&self) -> anyhow::Result<Vec<Oid>> {
        let mut objects = Vec::new();

        // List all objects from storage
        // This assumes the storage backend provides a way to list objects
        // For now, we'll scan the objects directory structure
        // List all objects (LocalBackend already operates within objects/ directory)
        let object_keys = self.storage.list_objects("").await?;

        for key in object_keys {
            // LocalBackend returns hex OIDs directly (no "objects/" prefix)
            // The key is already the hex string
            if key.len() == 64 {
                if let Ok(oid) = Oid::from_hex(&key) {
                    objects.push(oid);
                }
            }
        }

        Ok(objects)
    }

    /// List all references
    async fn list_all_refs(&self) -> anyhow::Result<Vec<Ref>> {
        let mut refs = Vec::new();

        // List all ref files
        let ref_keys = self.storage.list_objects("refs/").await?;

        for key in ref_keys {
            if let Ok(data) = self.storage.get(&key).await {
                if let Ok(r) = bincode::deserialize::<Ref>(&data) {
                    refs.push(r);
                }
            }
        }

        // Also check HEAD
        if let Ok(head_data) = self.storage.get("HEAD").await {
            if let Ok(head_ref) = bincode::deserialize::<Ref>(&head_data) {
                refs.push(head_ref);
            }
        }

        Ok(refs)
    }

    /// Check if an object exists
    async fn object_exists(&self, oid: &Oid) -> anyhow::Result<bool> {
        // Use oid.to_hex() - LocalBackend handles "objects/" prefix and sharding
        let key = oid.to_hex();
        self.storage.exists(&key).await
    }

    /// Check if a reference exists
    async fn ref_exists(&self, ref_name: &str) -> anyhow::Result<bool> {
        self.storage.exists(ref_name).await
    }
}

/// Repair functionality for fixing common issues
pub struct FsckRepair {
    storage: Arc<dyn StorageBackend>,
}

impl FsckRepair {
    /// Create a new FSCK repair tool
    pub fn new(storage: Arc<dyn StorageBackend>) -> Self {
        Self { storage }
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
    /// # let storage = Arc::new(LocalBackend::new("/tmp")?);
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
                    if let Some(oid) = issue.oid {
                        if self.repair_corrupted_object(&oid, dry_run).await? {
                            repaired += 1;
                        }
                    }
                }
                IssueCategory::BrokenReference => {
                    if let Some(ref_name) = &issue.ref_name {
                        if self.repair_broken_reference(ref_name, dry_run).await? {
                            repaired += 1;
                        }
                    }
                }
                IssueCategory::DanglingObject => {
                    if let Some(oid) = issue.oid {
                        if self.remove_dangling_object(&oid, dry_run).await? {
                            repaired += 1;
                        }
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

    /// Repair a corrupted object by removing it
    async fn repair_corrupted_object(&self, oid: &Oid, dry_run: bool) -> anyhow::Result<bool> {
        // Use oid.to_hex() - LocalBackend handles "objects/" prefix and sharding
        let key = oid.to_hex();

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
    async fn remove_dangling_object(&self, oid: &Oid, dry_run: bool) -> anyhow::Result<bool> {
        // Use oid.to_hex() - LocalBackend handles "objects/" prefix and sharding
        let key = oid.to_hex();

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
}
