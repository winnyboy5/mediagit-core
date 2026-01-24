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

//! Reference (ref) abstraction for branches and tags
//!
//! References provide a human-readable way to refer to commits without memorizing OIDs.
//! This module implements:
//! - **Ref types**: Direct refs (branches/tags) and symbolic refs (HEAD)
//! - **Ref namespaces**: heads/, tags/, remotes/ for organization
//! - **Atomic updates**: Safe ref updates with validation
//! - **Symbolic references**: Support for HEAD pointing to current branch

use crate::Oid;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use tracing::debug;

/// Ref types in the reference database
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RefType {
    /// Direct reference to a commit OID (branches, tags)
    Direct,
    /// Symbolic reference pointing to another ref (HEAD)
    Symbolic,
}

/// A reference to a commit or another reference
///
/// References provide human-readable names for commits. They can be:
/// - **Direct refs**: Point directly to a commit OID
/// - **Symbolic refs**: Point to another reference (typically HEAD pointing to current branch)
///
/// # Reference Paths
///
/// References are organized in namespaces:
/// - `refs/heads/main` - Main branch
/// - `refs/heads/feature/auth` - Feature branch
/// - `refs/tags/v1.0.0` - Release tag
/// - `refs/remotes/origin/main` - Remote tracking branch
/// - `HEAD` - Symbolic ref pointing to current branch
///
/// # Examples
///
/// ```
/// use mediagit_versioning::{Ref, RefType, Oid};
///
/// // Create a direct reference
/// let oid = Oid::hash(b"commit data");
/// let branch_ref = Ref::new_direct("refs/heads/main".to_string(), oid);
/// assert_eq!(branch_ref.name, "refs/heads/main");
/// assert_eq!(branch_ref.ref_type, RefType::Direct);
///
/// // Create a symbolic reference
/// let head_ref = Ref::new_symbolic("HEAD".to_string(), "refs/heads/main".to_string());
/// assert_eq!(head_ref.name, "HEAD");
/// assert_eq!(head_ref.ref_type, RefType::Symbolic);
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Ref {
    /// Reference name (e.g., "refs/heads/main", "HEAD")
    pub name: String,

    /// Type of reference (Direct or Symbolic)
    pub ref_type: RefType,

    /// For direct refs: OID of the referenced commit
    pub oid: Option<Oid>,

    /// For symbolic refs: Name of the referenced ref
    pub target: Option<String>,
}

impl Ref {
    /// Create a new direct reference pointing to an OID
    ///
    /// # Arguments
    ///
    /// * `name` - Reference name (e.g., "refs/heads/main")
    /// * `oid` - OID of the target commit
    ///
    /// # Examples
    ///
    /// ```
    /// use mediagit_versioning::{Ref, Oid};
    ///
    /// let oid = Oid::hash(b"data");
    /// let r = Ref::new_direct("refs/heads/main".to_string(), oid);
    /// assert_eq!(r.oid, Some(oid));
    /// assert_eq!(r.target, None);
    /// ```
    pub fn new_direct(name: String, oid: Oid) -> Self {
        Self {
            name,
            ref_type: RefType::Direct,
            oid: Some(oid),
            target: None,
        }
    }

    /// Create a new symbolic reference pointing to another ref
    ///
    /// # Arguments
    ///
    /// * `name` - Reference name (typically "HEAD")
    /// * `target` - Name of the target reference (e.g., "refs/heads/main")
    ///
    /// # Examples
    ///
    /// ```
    /// use mediagit_versioning::Ref;
    ///
    /// let r = Ref::new_symbolic("HEAD".to_string(), "refs/heads/main".to_string());
    /// assert_eq!(r.target, Some("refs/heads/main".to_string()));
    /// assert_eq!(r.oid, None);
    /// ```
    pub fn new_symbolic(name: String, target: String) -> Self {
        Self {
            name,
            ref_type: RefType::Symbolic,
            oid: None,
            target: Some(target),
        }
    }

    /// Get the namespace (e.g., "heads", "tags", "remotes")
    ///
    /// # Examples
    ///
    /// ```
    /// use mediagit_versioning::{Ref, Oid};
    ///
    /// let r = Ref::new_direct("refs/heads/main".to_string(), Oid::hash(b"x"));
    /// assert_eq!(r.namespace(), Some("heads"));
    /// ```
    pub fn namespace(&self) -> Option<&str> {
        if let Some(after_refs) = self.name.strip_prefix("refs/") {
            after_refs.split('/').next()
        } else {
            None
        }
    }

    /// Get the short name without the refs/ prefix
    ///
    /// # Examples
    ///
    /// ```
    /// use mediagit_versioning::{Ref, Oid};
    ///
    /// let r = Ref::new_direct("refs/heads/feature/auth".to_string(), Oid::hash(b"x"));
    /// assert_eq!(r.short_name(), "heads/feature/auth");
    /// ```
    pub fn short_name(&self) -> String {
        self.name
            .strip_prefix("refs/")
            .unwrap_or(&self.name)
            .to_string()
    }

    /// Get the branch name (last component of path)
    ///
    /// # Examples
    ///
    /// ```
    /// use mediagit_versioning::{Ref, Oid};
    ///
    /// let r = Ref::new_direct("refs/heads/feature/auth".to_string(), Oid::hash(b"x"));
    /// assert_eq!(r.branch_name(), "auth");
    /// ```
    pub fn branch_name(&self) -> String {
        self.name
            .split('/')
            .last()
            .unwrap_or(&self.name)
            .to_string()
    }

    /// Check if this is a branch reference
    pub fn is_branch(&self) -> bool {
        self.namespace() == Some("heads")
    }

    /// Check if this is a tag reference
    pub fn is_tag(&self) -> bool {
        self.namespace() == Some("tags")
    }

    /// Check if this is a remote tracking reference
    pub fn is_remote(&self) -> bool {
        self.namespace() == Some("remotes")
    }

    /// Check if this is HEAD
    pub fn is_head(&self) -> bool {
        self.name == "HEAD"
    }

    /// Validate the reference structure
    ///
    /// Ensures:
    /// - Direct refs have an OID and no target
    /// - Symbolic refs have a target and no OID
    /// - Ref name is not empty
    pub fn validate(&self) -> anyhow::Result<()> {
        if self.name.is_empty() {
            anyhow::bail!("Reference name cannot be empty");
        }

        match self.ref_type {
            RefType::Direct => {
                if self.oid.is_none() {
                    anyhow::bail!("Direct reference must have an OID");
                }
                if self.target.is_some() {
                    anyhow::bail!("Direct reference must not have a target");
                }
            }
            RefType::Symbolic => {
                if self.target.is_none() {
                    anyhow::bail!("Symbolic reference must have a target");
                }
                if self.oid.is_some() {
                    anyhow::bail!("Symbolic reference must not have an OID");
                }
            }
        }

        Ok(())
    }

    /// Serialize reference to bytes (plain text format)
    ///
    /// Format:
    /// - Direct ref: <hex-oid>\n
    /// - Symbolic ref: ref: <target>\n
    pub fn serialize(&self) -> anyhow::Result<Vec<u8>> {
        let content = match self.ref_type {
            RefType::Direct => {
                let oid = self.oid.ok_or_else(|| anyhow::anyhow!("Direct ref missing OID"))?;
                format!("{}\n", oid.to_hex())
            }
            RefType::Symbolic => {
                let target = self.target.as_ref().ok_or_else(|| anyhow::anyhow!("Symbolic ref missing target"))?;
                format!("ref: {}\n", target)
            }
        };
        Ok(content.into_bytes())
    }

    /// Deserialize reference from bytes (plain text format)
    ///
    /// Supports both:
    /// - Direct ref: <hex-oid>\n
    /// - Symbolic ref: ref: <target>\n
    pub fn deserialize(data: &[u8]) -> anyhow::Result<Self> {
        use crate::Oid;

        let content = std::str::from_utf8(data)
            .map_err(|e| anyhow::anyhow!("Invalid UTF-8 in ref file: {}", e))?
            .trim();

        if let Some(target) = content.strip_prefix("ref: ") {
            // Symbolic reference
            Ok(Self {
                name: String::new(), // Name will be set by caller
                ref_type: RefType::Symbolic,
                oid: None,
                target: Some(target.to_string()),
            })
        } else {
            // Direct reference - parse as hex OID
            let oid = Oid::from_hex(content)
                .map_err(|e| anyhow::anyhow!("Invalid OID in ref file: {}", e))?;
            Ok(Self {
                name: String::new(), // Name will be set by caller
                ref_type: RefType::Direct,
                oid: Some(oid),
                target: None,
            })
        }
    }
}

/// Recursively collect all reference files in a directory
async fn collect_refs_recursive(
    dir_path: &std::path::Path,
    refs_root: &std::path::Path,
    refs: &mut Vec<String>,
) -> anyhow::Result<()> {
    use tokio::fs;

    let mut entries = fs::read_dir(dir_path).await?;

    while let Some(entry) = entries.next_entry().await? {
        let path = entry.path();
        let file_type = entry.file_type().await?;

        if file_type.is_file() {
            // Convert absolute path to ref name: refs/heads/feat/bunny-audio
            if let Ok(rel) = path.strip_prefix(refs_root) {
                let ref_name = format!("refs/{}", rel.display());
                refs.push(ref_name);
            }
        } else if file_type.is_dir() {
            // Recursively scan subdirectories
            Box::pin(collect_refs_recursive(&path, refs_root, refs)).await?;
        }
    }

    Ok(())
}

/// Reference database providing ref management
///
/// Manages references atomically and safely, supporting:
/// - Direct and symbolic references
/// - Multiple namespaces
/// - Atomic updates
/// - Validation and safety checks
///
/// # Examples
///
/// ```no_run
/// use mediagit_versioning::{RefDatabase, Ref, Oid};
/// use std::path::PathBuf;
///
/// #[tokio::main]
/// async fn main() -> anyhow::Result<()> {
///     let refdb = RefDatabase::new(PathBuf::from("/tmp/mediagit"));
///
///     // Create a branch reference
///     let oid = Oid::hash(b"commit");
///     let r = Ref::new_direct("refs/heads/main".to_string(), oid);
///     refdb.write(&r).await?;
///
///     // Set HEAD to point to main
///     let head = Ref::new_symbolic("HEAD".to_string(), "refs/heads/main".to_string());
///     refdb.write(&head).await?;
///
///     // Read the reference back
///     let branch = refdb.read("refs/heads/main").await?;
///     assert_eq!(branch.oid, Some(oid));
///
///     Ok(())
/// }
/// ```
pub struct RefDatabase {
    /// Root path (e.g., .mediagit directory)
    root: PathBuf,
}

impl RefDatabase {
    /// Create a new reference database
    ///
    /// # Arguments
    ///
    /// * `root` - Root directory path (e.g., .mediagit)
    pub fn new<P: AsRef<Path>>(root: P) -> Self {
        Self {
            root: root.as_ref().to_path_buf(),
        }
    }

    /// Get the absolute file path for a reference name
    fn ref_path(&self, ref_name: &str) -> PathBuf {
        // Refs are stored at: .mediagit/HEAD or .mediagit/refs/heads/main
        self.root.join(ref_name)
    }

    /// Write a reference to the database
    ///
    /// Performs validation and atomically stores the reference.
    /// For symbolic refs, also validates that the target is valid.
    pub async fn write(&self, r: &Ref) -> anyhow::Result<()> {
        use tokio::fs;
        use tokio::io::AsyncWriteExt;

        r.validate()?;

        debug!(
            ref_name = %r.name,
            ref_type = ?r.ref_type,
            "Writing reference"
        );

        let data = r.serialize()?;
        let path = self.ref_path(&r.name);

        // Create parent directory if needed
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).await?;
        }

        // Write atomically using temp file + rename
        let temp_path = path.with_extension("tmp");
        let mut file = fs::File::create(&temp_path).await?;
        file.write_all(&data).await?;
        file.sync_all().await?;
        drop(file);

        fs::rename(&temp_path, &path).await?;

        debug!(ref_name = %r.name, "Reference written successfully");
        Ok(())
    }

    /// Read a reference from the database
    ///
    /// # Arguments
    ///
    /// * `ref_name` - Name of the reference (e.g., "refs/heads/main", "HEAD")
    ///
    /// # Returns
    ///
    /// The reference if it exists
    pub async fn read(&self, ref_name: &str) -> anyhow::Result<Ref> {
        use tokio::fs;

        let path = self.ref_path(ref_name);

        match fs::read(&path).await {
            Ok(data) => {
                debug!(ref_name = %ref_name, "Read reference");
                let mut r = Ref::deserialize(&data)?;
                r.name = ref_name.to_string(); // Set name from the file path
                Ok(r)
            }
            Err(_) => {
                anyhow::bail!("Reference not found: {}", ref_name)
            }
        }
    }

    /// Check if a reference exists
    pub async fn exists(&self, ref_name: &str) -> anyhow::Result<bool> {
        use tokio::fs;

        let path = self.ref_path(ref_name);
        Ok(fs::metadata(&path).await.is_ok())
    }

    /// Delete a reference
    ///
    /// # Arguments
    ///
    /// * `ref_name` - Name of the reference to delete
    pub async fn delete(&self, ref_name: &str) -> anyhow::Result<()> {
        use tokio::fs;

        debug!(ref_name = %ref_name, "Deleting reference");

        let path = self.ref_path(ref_name);
        fs::remove_file(&path).await?;

        debug!(ref_name = %ref_name, "Reference deleted");
        Ok(())
    }

    /// List all references in a namespace
    ///
    /// # Arguments
    ///
    /// * `namespace` - Namespace to list (e.g., "heads", "tags")
    ///
    /// # Returns
    ///
    /// Vector of reference names in the namespace
    pub async fn list(&self, namespace: &str) -> anyhow::Result<Vec<String>> {
        use tokio::fs;

        let prefix = format!("refs/{}", namespace);
        let dir_path = self.root.join(&prefix);

        debug!(namespace = %namespace, "Listing references");

        if !fs::metadata(&dir_path).await.is_ok() {
            return Ok(Vec::new());
        }

        let mut refs = Vec::new();
        let refs_root = self.root.join("refs");
        collect_refs_recursive(&dir_path, &refs_root, &mut refs).await?;

        debug!(
            namespace = %namespace,
            count = refs.len(),
            "Listed references"
        );

        Ok(refs)
    }

    /// List all branches
    pub async fn list_branches(&self) -> anyhow::Result<Vec<String>> {
        self.list("heads").await
    }

    /// List all tags
    pub async fn list_tags(&self) -> anyhow::Result<Vec<String>> {
        self.list("tags").await
    }

    /// Resolve a symbolic reference to its direct OID
    ///
    /// If the reference is symbolic (like HEAD), follows the target until
    /// reaching a direct reference. Returns the OID of the final target.
    ///
    /// # Arguments
    ///
    /// * `ref_name` - Name of the reference to resolve
    ///
    /// # Returns
    ///
    /// The OID of the final target
    pub async fn resolve(&self, ref_name: &str) -> anyhow::Result<Oid> {
        let mut current = ref_name.to_string();
        let mut depth = 0;
        const MAX_DEPTH: usize = 10;

        loop {
            if depth >= MAX_DEPTH {
                anyhow::bail!("Circular reference detected in: {}", ref_name);
            }

            let r = self.read(&current).await?;

            match r.ref_type {
                RefType::Direct => {
                    if let Some(oid) = r.oid {
                        debug!(ref_name = %ref_name, resolved_oid = %oid, "Resolved reference");
                        return Ok(oid);
                    } else {
                        anyhow::bail!("Direct reference has no OID: {}", current);
                    }
                }
                RefType::Symbolic => {
                    if let Some(target) = r.target {
                        current = target;
                        depth += 1;
                    } else {
                        anyhow::bail!("Symbolic reference has no target: {}", current);
                    }
                }
            }
        }
    }

    /// Update a reference with a new OID (for direct refs only)
    ///
    /// # Arguments
    ///
    /// * `ref_name` - Name of the reference
    /// * `new_oid` - New OID to set
    /// * `force` - If true, update even if ref exists with different OID
    pub async fn update(&self, ref_name: &str, new_oid: Oid, force: bool) -> anyhow::Result<()> {
        if let Ok(existing) = self.read(ref_name).await {
            if existing.ref_type != RefType::Direct {
                anyhow::bail!(
                    "Cannot update symbolic reference: {}. Use update_symbolic instead.",
                    ref_name
                );
            }

            if !force && existing.oid != Some(new_oid) {
                debug!(
                    ref_name = %ref_name,
                    old_oid = %existing.oid.unwrap(),
                    new_oid = %new_oid,
                    "Fast-forward check failed"
                );

                // If we got here with force=false, this is a non-fast-forward update
                anyhow::bail!(
                    "Non-fast-forward update to {}: {} -> {}",
                    ref_name,
                    existing.oid.unwrap(),
                    new_oid
                );
            }
        }

        let r = Ref::new_direct(ref_name.to_string(), new_oid);
        self.write(&r).await?;

        debug!(ref_name = %ref_name, new_oid = %new_oid, "Updated reference");
        Ok(())
    }

    /// Update a symbolic reference
    ///
    /// # Arguments
    ///
    /// * `ref_name` - Name of the symbolic reference (typically "HEAD")
    /// * `target` - New target reference
    pub async fn update_symbolic(&self, ref_name: &str, target: &str) -> anyhow::Result<()> {
        let r = Ref::new_symbolic(ref_name.to_string(), target.to_string());
        self.write(&r).await?;

        debug!(
            ref_name = %ref_name,
            target = %target,
            "Updated symbolic reference"
        );
        Ok(())
    }
}

/// Normalize a ref name to its full path
///
/// Handles both short branch names (e.g., "main") and full ref paths (e.g., "refs/heads/main").
/// This ensures consistent ref handling across push, pull, and other operations.
///
/// # Arguments
///
/// * `input` - The ref name to normalize (short or full path)
///
/// # Returns
///
/// The full ref path (e.g., "refs/heads/main")
///
/// # Examples
///
/// ```
/// use mediagit_versioning::normalize_ref_name;
///
/// assert_eq!(normalize_ref_name("main"), "refs/heads/main");
/// assert_eq!(normalize_ref_name("refs/heads/main"), "refs/heads/main");
/// assert_eq!(normalize_ref_name("feature/auth"), "refs/heads/feature/auth");
/// ```
pub fn normalize_ref_name(input: &str) -> String {
    if input.starts_with("refs/") {
        // Already a full ref path
        input.to_string()
    } else if input == "HEAD" {
        // HEAD is special - don't prefix it
        input.to_string()
    } else {
        // Assume it's a branch name - prefix with refs/heads/
        format!("refs/heads/{}", input)
    }
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ref_direct_creation() {
        let oid = Oid::hash(b"test");
        let r = Ref::new_direct("refs/heads/main".to_string(), oid);

        assert_eq!(r.name, "refs/heads/main");
        assert_eq!(r.ref_type, RefType::Direct);
        assert_eq!(r.oid, Some(oid));
        assert_eq!(r.target, None);
    }

    #[test]
    fn test_ref_symbolic_creation() {
        let r = Ref::new_symbolic("HEAD".to_string(), "refs/heads/main".to_string());

        assert_eq!(r.name, "HEAD");
        assert_eq!(r.ref_type, RefType::Symbolic);
        assert_eq!(r.oid, None);
        assert_eq!(r.target, Some("refs/heads/main".to_string()));
    }

    #[test]
    fn test_ref_namespace() {
        let r1 = Ref::new_direct("refs/heads/main".to_string(), Oid::hash(b"x"));
        assert_eq!(r1.namespace(), Some("heads"));

        let r2 = Ref::new_direct("refs/tags/v1.0.0".to_string(), Oid::hash(b"x"));
        assert_eq!(r2.namespace(), Some("tags"));

        let r3 = Ref::new_symbolic("HEAD".to_string(), "refs/heads/main".to_string());
        assert_eq!(r3.namespace(), None);
    }

    #[test]
    fn test_ref_short_name() {
        let r = Ref::new_direct("refs/heads/feature/auth".to_string(), Oid::hash(b"x"));
        assert_eq!(r.short_name(), "heads/feature/auth");
    }

    #[test]
    fn test_ref_branch_name() {
        let r = Ref::new_direct("refs/heads/feature/auth".to_string(), Oid::hash(b"x"));
        assert_eq!(r.branch_name(), "auth");

        let r2 = Ref::new_direct("refs/heads/main".to_string(), Oid::hash(b"x"));
        assert_eq!(r2.branch_name(), "main");
    }

    #[test]
    fn test_ref_is_branch() {
        let r1 = Ref::new_direct("refs/heads/main".to_string(), Oid::hash(b"x"));
        assert!(r1.is_branch());

        let r2 = Ref::new_direct("refs/tags/v1.0".to_string(), Oid::hash(b"x"));
        assert!(!r2.is_branch());
    }

    #[test]
    fn test_ref_is_tag() {
        let r = Ref::new_direct("refs/tags/v1.0".to_string(), Oid::hash(b"x"));
        assert!(r.is_tag());
    }

    #[test]
    fn test_ref_is_remote() {
        let r = Ref::new_direct("refs/remotes/origin/main".to_string(), Oid::hash(b"x"));
        assert!(r.is_remote());
    }

    #[test]
    fn test_ref_is_head() {
        let r = Ref::new_symbolic("HEAD".to_string(), "refs/heads/main".to_string());
        assert!(r.is_head());
    }

    #[test]
    fn test_ref_validate_direct() {
        let r = Ref::new_direct("refs/heads/main".to_string(), Oid::hash(b"x"));
        assert!(r.validate().is_ok());
    }

    #[test]
    fn test_ref_validate_symbolic() {
        let r = Ref::new_symbolic("HEAD".to_string(), "refs/heads/main".to_string());
        assert!(r.validate().is_ok());
    }

    #[test]
    fn test_ref_validate_empty_name() {
        let oid = Oid::hash(b"x");
        let mut r = Ref::new_direct("refs/heads/main".to_string(), oid);
        r.name = String::new();
        assert!(r.validate().is_err());
    }

    #[test]
    fn test_ref_validate_invalid_direct() {
        let mut r = Ref::new_direct("refs/heads/main".to_string(), Oid::hash(b"x"));
        r.oid = None;
        assert!(r.validate().is_err());
    }

    #[test]
    fn test_ref_validate_invalid_symbolic() {
        let mut r = Ref::new_symbolic("HEAD".to_string(), "refs/heads/main".to_string());
        r.target = None;
        assert!(r.validate().is_err());
    }

    #[test]
    fn test_ref_serialization() {
        let oid = Oid::hash(b"test");
        let r = Ref::new_direct("refs/heads/main".to_string(), oid);

        let serialized = r.serialize().unwrap();
        let mut deserialized = Ref::deserialize(&serialized).unwrap();
        deserialized.name = r.name.clone(); // Restore name as done in RefDatabase::read

        assert_eq!(r, deserialized);
    }

    #[test]
    fn test_ref_serialization_symbolic() {
        let r = Ref::new_symbolic("HEAD".to_string(), "refs/heads/main".to_string());

        let serialized = r.serialize().unwrap();
        let mut deserialized = Ref::deserialize(&serialized).unwrap();
        deserialized.name = r.name.clone(); // Restore name as done in RefDatabase::read

        assert_eq!(r, deserialized);
    }

    #[tokio::test]
    async fn test_refdb_write_and_read() {

        let temp_dir = tempfile::tempdir().unwrap();
        let refdb = RefDatabase::new(temp_dir.path());

        let oid = Oid::hash(b"commit");
        let r = Ref::new_direct("refs/heads/main".to_string(), oid);

        refdb.write(&r).await.unwrap();
        let read_r = refdb.read("refs/heads/main").await.unwrap();

        assert_eq!(r, read_r);
    }

    #[tokio::test]
    async fn test_refdb_symbolic_ref() {

        let temp_dir = tempfile::tempdir().unwrap();
        let refdb = RefDatabase::new(temp_dir.path());

        let oid = Oid::hash(b"commit");
        let branch = Ref::new_direct("refs/heads/main".to_string(), oid);
        refdb.write(&branch).await.unwrap();

        let head = Ref::new_symbolic("HEAD".to_string(), "refs/heads/main".to_string());
        refdb.write(&head).await.unwrap();

        let read_head = refdb.read("HEAD").await.unwrap();
        assert_eq!(read_head.ref_type, RefType::Symbolic);
        assert_eq!(read_head.target, Some("refs/heads/main".to_string()));
    }

    #[tokio::test]
    async fn test_refdb_resolve() {

        let temp_dir = tempfile::tempdir().unwrap();
        let refdb = RefDatabase::new(temp_dir.path());

        let oid = Oid::hash(b"commit");
        let branch = Ref::new_direct("refs/heads/main".to_string(), oid);
        refdb.write(&branch).await.unwrap();

        let head = Ref::new_symbolic("HEAD".to_string(), "refs/heads/main".to_string());
        refdb.write(&head).await.unwrap();

        let resolved = refdb.resolve("HEAD").await.unwrap();
        assert_eq!(resolved, oid);
    }

    #[tokio::test]
    async fn test_refdb_delete() {

        let temp_dir = tempfile::tempdir().unwrap();
        let refdb = RefDatabase::new(temp_dir.path());

        let oid = Oid::hash(b"commit");
        let r = Ref::new_direct("refs/heads/main".to_string(), oid);
        refdb.write(&r).await.unwrap();

        assert!(refdb.read("refs/heads/main").await.is_ok());

        refdb.delete("refs/heads/main").await.unwrap();

        assert!(refdb.read("refs/heads/main").await.is_err());
    }

    #[tokio::test]
    async fn test_refdb_exists() {

        let temp_dir = tempfile::tempdir().unwrap();
        let refdb = RefDatabase::new(temp_dir.path());

        assert!(!refdb.exists("refs/heads/main").await.unwrap());

        let oid = Oid::hash(b"commit");
        let r = Ref::new_direct("refs/heads/main".to_string(), oid);
        refdb.write(&r).await.unwrap();

        assert!(refdb.exists("refs/heads/main").await.unwrap());
    }

    #[tokio::test]
    async fn test_refdb_update() {

        let temp_dir = tempfile::tempdir().unwrap();
        let refdb = RefDatabase::new(temp_dir.path());

        let oid1 = Oid::hash(b"commit1");
        let r = Ref::new_direct("refs/heads/main".to_string(), oid1);
        refdb.write(&r).await.unwrap();

        let oid2 = Oid::hash(b"commit2");
        // Use force=true since we don't have real commit history
        refdb.update("refs/heads/main", oid2, true).await.unwrap();

        let updated = refdb.read("refs/heads/main").await.unwrap();
        assert_eq!(updated.oid, Some(oid2));
    }

    #[tokio::test]
    async fn test_refdb_update_symbolic() {

        let temp_dir = tempfile::tempdir().unwrap();
        let refdb = RefDatabase::new(temp_dir.path());

        let oid = Oid::hash(b"commit");
        let branch = Ref::new_direct("refs/heads/main".to_string(), oid);
        refdb.write(&branch).await.unwrap();

        let head = Ref::new_symbolic("HEAD".to_string(), "refs/heads/main".to_string());
        refdb.write(&head).await.unwrap();

        // Update HEAD to point to develop
        let develop = Ref::new_direct("refs/heads/develop".to_string(), oid);
        refdb.write(&develop).await.unwrap();

        refdb.update_symbolic("HEAD", "refs/heads/develop").await.unwrap();

        let updated_head = refdb.read("HEAD").await.unwrap();
        assert_eq!(updated_head.target, Some("refs/heads/develop".to_string()));
    }

    #[tokio::test]
    async fn test_refdb_list_branches() {

        let temp_dir = tempfile::tempdir().unwrap();
        let refdb = RefDatabase::new(temp_dir.path());

        let oid = Oid::hash(b"commit");

        let r1 = Ref::new_direct("refs/heads/main".to_string(), oid);
        refdb.write(&r1).await.unwrap();

        let r2 = Ref::new_direct("refs/heads/develop".to_string(), oid);
        refdb.write(&r2).await.unwrap();

        let branches = refdb.list_branches().await.unwrap();
        assert_eq!(branches.len(), 2);
        assert!(branches.iter().any(|b| b == "refs/heads/main"));
        assert!(branches.iter().any(|b| b == "refs/heads/develop"));
    }

    #[tokio::test]
    async fn test_refdb_list_tags() {

        let temp_dir = tempfile::tempdir().unwrap();
        let refdb = RefDatabase::new(temp_dir.path());

        let oid = Oid::hash(b"commit");

        let t1 = Ref::new_direct("refs/tags/v1.0.0".to_string(), oid);
        refdb.write(&t1).await.unwrap();

        let t2 = Ref::new_direct("refs/tags/v2.0.0".to_string(), oid);
        refdb.write(&t2).await.unwrap();

        let tags = refdb.list_tags().await.unwrap();
        assert_eq!(tags.len(), 2);
        assert!(tags.iter().any(|t| t == "refs/tags/v1.0.0"));
        assert!(tags.iter().any(|t| t == "refs/tags/v2.0.0"));
    }

    #[tokio::test]
    async fn test_refdb_circular_reference_detection() {

        let temp_dir = tempfile::tempdir().unwrap();
        let refdb = RefDatabase::new(temp_dir.path());

        // Create circular reference: HEAD -> main, main -> develop, develop -> HEAD
        let head = Ref::new_symbolic("HEAD".to_string(), "refs/heads/main".to_string());
        refdb.write(&head).await.unwrap();

        let main = Ref::new_symbolic("refs/heads/main".to_string(), "refs/heads/develop".to_string());
        refdb.write(&main).await.unwrap();

        let develop = Ref::new_symbolic("refs/heads/develop".to_string(), "HEAD".to_string());
        refdb.write(&develop).await.unwrap();

        // Resolution should fail
        assert!(refdb.resolve("HEAD").await.is_err());
    }
}
