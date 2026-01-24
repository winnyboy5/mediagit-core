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

//! Branch operations and management
//!
//! This module provides high-level branch management functionality:
//! - Creating and deleting branches
//! - Switching branches with detached HEAD support
//! - Fast-forward and force updates
//! - Branch listing with metadata
//! - Validation and safety checks

use crate::{Oid, Ref, RefDatabase, RefType};
use std::path::Path;
use tracing::{debug, info, warn};

/// Branch operations manager
///
/// Provides high-level operations for branch management, built on top of RefDatabase.
/// Supports:
/// - Creating branches at specific commits
/// - Deleting branches with safety checks
/// - Switching branches (changing HEAD)
/// - Detached HEAD state
/// - Fast-forward and force updates
///
/// # Examples
///
/// ```no_run
/// use mediagit_versioning::{BranchManager, Oid};
///
/// #[tokio::main]
/// async fn main() -> anyhow::Result<()> {
///     let branch_mgr = BranchManager::new("/tmp/mediagit");
///
///     let commit_oid = Oid::hash(b"commit data");
///
///     // Create a new branch
///     branch_mgr.create("feature/auth", commit_oid).await?;
///
///     // Switch to the branch
///     branch_mgr.switch_to("feature/auth").await?;
///
///     // List all branches
///     let branches = branch_mgr.list().await?;
///     println!("Branches: {:?}", branches);
///
///     // Delete the branch
///     branch_mgr.delete("feature/auth").await?;
///
///     Ok(())
/// }
/// ```
pub struct BranchManager {
    refdb: RefDatabase,
}

/// Information about a branch
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BranchInfo {
    /// Branch name (e.g., "main", "feature/auth")
    pub name: String,

    /// Full ref path (e.g., "refs/heads/main")
    pub ref_path: String,

    /// OID of the commit the branch points to
    pub oid: Oid,

    /// Whether this is the current branch
    pub is_current: bool,
}

/// Detached HEAD state information
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DetachedHead {
    /// The OID that HEAD points to
    pub commit_oid: Oid,

    /// The ref that was previously checked out (if any)
    pub previous_branch: Option<String>,
}

impl BranchManager {
    /// Create a new branch manager
    ///
    /// # Arguments
    ///
    /// * `root` - Root directory path (e.g., .mediagit)
    pub fn new<P: AsRef<Path>>(root: P) -> Self {
        Self {
            refdb: RefDatabase::new(root),
        }
    }

    /// Create a new branch pointing to a commit
    ///
    /// # Arguments
    ///
    /// * `branch_name` - Name of the branch (e.g., "feature/auth", "main")
    /// * `commit_oid` - OID of the commit the branch should point to
    ///
    /// # Returns
    ///
    /// Error if branch already exists or name is invalid
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # use mediagit_versioning::{BranchManager, Oid};
    /// # use mediagit_storage::LocalBackend;
    /// # use std::sync::Arc;
    /// # #[tokio::main]
    /// # async fn main() -> anyhow::Result<()> {
    /// # let storage: Arc<dyn mediagit_storage::StorageBackend> =
    /// #     Arc::new(LocalBackend::new("/tmp/test").await?);
    /// # let storage_path = std::path::PathBuf::from("/tmp/test");
    /// let branch_mgr = BranchManager::new(&storage_path);
    /// let oid = Oid::hash(b"commit");
    /// branch_mgr.create("develop", oid).await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn create(&self, branch_name: &str, commit_oid: Oid) -> anyhow::Result<()> {
        self.validate_branch_name(branch_name)?;

        let ref_path = format!("refs/heads/{}", branch_name);

        if self.refdb.exists(&ref_path).await? {
            anyhow::bail!("Branch already exists: {}", branch_name);
        }

        let r = Ref::new_direct(ref_path, commit_oid);
        self.refdb.write(&r).await?;

        info!(branch_name = %branch_name, commit_oid = %commit_oid, "Branch created");
        Ok(())
    }

    /// Delete a branch
    ///
    /// # Arguments
    ///
    /// * `branch_name` - Name of the branch to delete
    ///
    /// # Returns
    ///
    /// Error if branch doesn't exist or is the current branch
    pub async fn delete(&self, branch_name: &str) -> anyhow::Result<()> {
        let ref_path = format!("refs/heads/{}", branch_name);

        if !self.refdb.exists(&ref_path).await? {
            anyhow::bail!("Branch does not exist: {}", branch_name);
        }

        // Check if it's the current branch
        if let Ok(head) = self.refdb.read("HEAD").await {
            if head.ref_type == RefType::Symbolic {
                if let Some(target) = head.target {
                    if target == ref_path {
                        anyhow::bail!("Cannot delete current branch: {}", branch_name);
                    }
                }
            }
        }

        self.refdb.delete(&ref_path).await?;
        info!(branch_name = %branch_name, "Branch deleted");
        Ok(())
    }

    /// List all branches
    ///
    /// # Returns
    ///
    /// Vector of branch information including name and current status
    pub async fn list(&self) -> anyhow::Result<Vec<BranchInfo>> {
        let branch_paths = self.refdb.list_branches().await?;

        let current_branch = self.current_branch().await.ok().flatten();

        let mut branches = Vec::new();

        for ref_path in branch_paths {
            if let Ok(r) = self.refdb.read(&ref_path).await {
                if let Some(oid) = r.oid {
                    // Extract the short branch name from the path
                    let branch_name = ref_path
                        .strip_prefix("refs/heads/")
                        .unwrap_or(&ref_path)
                        .to_string();

                    let is_current = current_branch
                        .as_ref()
                        .map(|cb| cb == &branch_name)
                        .unwrap_or(false);

                    branches.push(BranchInfo {
                        name: branch_name,
                        ref_path,
                        oid,
                        is_current,
                    });
                }
            }
        }

        Ok(branches)
    }

    /// Get information about a specific branch
    ///
    /// # Arguments
    ///
    /// * `branch_name` - Name of the branch
    pub async fn get_info(&self, branch_name: &str) -> anyhow::Result<BranchInfo> {
        let ref_path = format!("refs/heads/{}", branch_name);

        if !self.refdb.exists(&ref_path).await? {
            anyhow::bail!("Branch does not exist: {}", branch_name);
        }

        let r = self.refdb.read(&ref_path).await?;

        let oid = r
            .oid
            .ok_or_else(|| anyhow::anyhow!("Branch has no OID: {}", branch_name))?;

        let current_branch = self.current_branch().await.ok().flatten();
        let is_current = current_branch
            .as_ref()
            .map(|cb| cb == &ref_path)
            .unwrap_or(false);

        Ok(BranchInfo {
            name: branch_name.to_string(),
            ref_path,
            oid,
            is_current,
        })
    }

    /// Check if a branch exists
    pub async fn exists(&self, branch_name: &str) -> anyhow::Result<bool> {
        let ref_path = format!("refs/heads/{}", branch_name);
        self.refdb.exists(&ref_path).await
    }

    /// Rename a branch
    ///
    /// # Arguments
    ///
    /// * `old_name` - Current branch name
    /// * `new_name` - New branch name
    pub async fn rename(&self, old_name: &str, new_name: &str) -> anyhow::Result<()> {
        self.validate_branch_name(new_name)?;

        let old_ref = format!("refs/heads/{}", old_name);
        let new_ref = format!("refs/heads/{}", new_name);

        if !self.refdb.exists(&old_ref).await? {
            anyhow::bail!("Branch does not exist: {}", old_name);
        }

        if self.refdb.exists(&new_ref).await? {
            anyhow::bail!("Branch already exists: {}", new_name);
        }

        let r = self.refdb.read(&old_ref).await?;

        // Update ref to new path
        let mut updated_ref = r;
        updated_ref.name = new_ref.clone();
        self.refdb.write(&updated_ref).await?;

        // Delete old ref
        self.refdb.delete(&old_ref).await?;

        // If it was current branch, update HEAD
        if let Ok(head) = self.refdb.read("HEAD").await {
            if head.ref_type == RefType::Symbolic {
                if let Some(target) = head.target {
                    if target == old_ref {
                        self.refdb.update_symbolic("HEAD", &new_ref).await?;
                    }
                }
            }
        }

        info!(old_name = %old_name, new_name = %new_name, "Branch renamed");
        Ok(())
    }

    /// Switch to a branch
    ///
    /// Updates HEAD to point to the specified branch.
    ///
    /// # Arguments
    ///
    /// * `branch_name` - Name of the branch to switch to
    pub async fn switch_to(&self, branch_name: &str) -> anyhow::Result<()> {
        let ref_path = format!("refs/heads/{}", branch_name);

        if !self.refdb.exists(&ref_path).await? {
            anyhow::bail!("Branch does not exist: {}", branch_name);
        }

        self.refdb.update_symbolic("HEAD", &ref_path).await?;
        info!(branch_name = %branch_name, "Switched to branch");
        Ok(())
    }

    /// Get the current branch
    ///
    /// # Returns
    ///
    /// The name of the current branch, or None if HEAD is detached
    pub async fn current_branch(&self) -> anyhow::Result<Option<String>> {
        match self.refdb.read("HEAD").await {
            Ok(head) => {
                if head.ref_type == RefType::Symbolic {
                    Ok(head.target.and_then(|target| {
                        target
                            .strip_prefix("refs/heads/")
                            .map(|s| s.to_string())
                    }))
                } else {
                    Ok(None) // Detached HEAD
                }
            }
            Err(_) => {
                warn!("HEAD reference not found - initializing to default");
                Ok(None)
            }
        }
    }

    /// Set HEAD to detached state pointing to a commit
    ///
    /// # Arguments
    ///
    /// * `commit_oid` - OID to point to
    pub async fn detach_head(&self, commit_oid: Oid) -> anyhow::Result<DetachedHead> {
        let previous_branch = self.current_branch().await.ok().flatten();

        let head = Ref::new_direct("HEAD".to_string(), commit_oid);
        self.refdb.write(&head).await?;

        info!(commit_oid = %commit_oid, "HEAD detached");

        Ok(DetachedHead {
            commit_oid,
            previous_branch,
        })
    }

    /// Check if HEAD is detached
    ///
    /// # Returns
    ///
    /// true if HEAD points directly to a commit, false if it points to a branch
    pub async fn is_detached(&self) -> anyhow::Result<bool> {
        match self.refdb.read("HEAD").await {
            Ok(head) => Ok(head.ref_type == RefType::Direct),
            Err(_) => Ok(false),
        }
    }

    /// Get the commit OID that HEAD points to
    ///
    /// Works whether HEAD is attached or detached.
    pub async fn head_commit(&self) -> anyhow::Result<Oid> {
        self.refdb.resolve("HEAD").await
    }

    /// Update a branch to point to a new commit (fast-forward safe)
    ///
    /// # Arguments
    ///
    /// * `branch_name` - Name of the branch to update
    /// * `new_oid` - New commit OID
    /// * `force` - If true, allow non-fast-forward updates
    ///
    /// # Returns
    ///
    /// Error if the update would not be a fast-forward and force=false
    pub async fn update_to(
        &self,
        branch_name: &str,
        new_oid: Oid,
        force: bool,
    ) -> anyhow::Result<()> {
        let ref_path = format!("refs/heads/{}", branch_name);

        if !self.refdb.exists(&ref_path).await? {
            anyhow::bail!("Branch does not exist: {}", branch_name);
        }

        self.refdb.update(&ref_path, new_oid, force).await?;

        if force {
            info!(branch_name = %branch_name, new_oid = %new_oid, "Force updated branch");
        } else {
            info!(branch_name = %branch_name, new_oid = %new_oid, "Fast-forward updated branch");
        }

        Ok(())
    }

    /// Create a tag at a specific commit
    ///
    /// # Arguments
    ///
    /// * `tag_name` - Name of the tag (e.g., "v1.0.0")
    /// * `commit_oid` - OID of the commit to tag
    pub async fn create_tag(&self, tag_name: &str, commit_oid: Oid) -> anyhow::Result<()> {
        self.validate_tag_name(tag_name)?;

        let ref_path = format!("refs/tags/{}", tag_name);

        if self.refdb.exists(&ref_path).await? {
            anyhow::bail!("Tag already exists: {}", tag_name);
        }

        let r = Ref::new_direct(ref_path, commit_oid);
        self.refdb.write(&r).await?;

        info!(tag_name = %tag_name, commit_oid = %commit_oid, "Tag created");
        Ok(())
    }

    /// Delete a tag
    pub async fn delete_tag(&self, tag_name: &str) -> anyhow::Result<()> {
        let ref_path = format!("refs/tags/{}", tag_name);

        if !self.refdb.exists(&ref_path).await? {
            anyhow::bail!("Tag does not exist: {}", tag_name);
        }

        self.refdb.delete(&ref_path).await?;
        info!(tag_name = %tag_name, "Tag deleted");
        Ok(())
    }

    /// List all tags
    pub async fn list_tags(&self) -> anyhow::Result<Vec<String>> {
        let tag_paths = self.refdb.list_tags().await?;
        Ok(tag_paths
            .into_iter()
            .map(|path| {
                path.strip_prefix("refs/tags/")
                    .unwrap_or(&path)
                    .to_string()
            })
            .collect())
    }

    /// Initialize a new repository with default refs
    ///
    /// Creates the HEAD reference pointing to main branch (or creates main branch if needed)
    pub async fn initialize(&self, initial_commit: Option<Oid>) -> anyhow::Result<()> {
        debug!("Initializing branch database");

        if let Some(commit_oid) = initial_commit {
            // Create main branch
            let main_ref = Ref::new_direct("refs/heads/main".to_string(), commit_oid);
            self.refdb.write(&main_ref).await?;

            // Point HEAD to main
            let head = Ref::new_symbolic("HEAD".to_string(), "refs/heads/main".to_string());
            self.refdb.write(&head).await?;

            info!("Branch database initialized with main branch");
        } else {
            // Create HEAD pointing to non-existent main (orphaned)
            let head = Ref::new_symbolic("HEAD".to_string(), "refs/heads/main".to_string());
            self.refdb.write(&head).await?;

            info!("Branch database initialized (orphaned state)");
        }

        Ok(())
    }

    /// Validate branch name
    fn validate_branch_name(&self, name: &str) -> anyhow::Result<()> {
        if name.is_empty() {
            anyhow::bail!("Branch name cannot be empty");
        }

        if name.starts_with('-') || name.starts_with('.') {
            anyhow::bail!("Branch name cannot start with '-' or '.'");
        }

        if name.ends_with('.') {
            anyhow::bail!("Branch name cannot end with '.'");
        }

        if name.contains("..") || name.contains("//") || name.contains("\\") {
            anyhow::bail!("Branch name contains invalid sequences");
        }

        Ok(())
    }

    /// Validate tag name
    fn validate_tag_name(&self, name: &str) -> anyhow::Result<()> {
        if name.is_empty() {
            anyhow::bail!("Tag name cannot be empty");
        }

        if name.starts_with('-') || name.starts_with('.') {
            anyhow::bail!("Tag name cannot start with '-' or '.'");
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[tokio::test]
    async fn test_branch_create() {
        let temp_dir = tempdir().unwrap();
        let storage_path = temp_dir.path();
        let mgr = BranchManager::new(storage_path);

        let oid = Oid::hash(b"commit");
        mgr.create("main", oid).await.unwrap();

        assert!(mgr.exists("main").await.unwrap());
    }

    #[tokio::test]
    async fn test_branch_create_duplicate() {
        let temp_dir = tempdir().unwrap();
        let storage_path = temp_dir.path();
        let mgr = BranchManager::new(storage_path);

        let oid = Oid::hash(b"commit");
        mgr.create("main", oid).await.unwrap();
        assert!(mgr.create("main", oid).await.is_err());
    }

    #[tokio::test]
    async fn test_branch_delete() {
        let temp_dir = tempdir().unwrap();
        let storage_path = temp_dir.path();
        let mgr = BranchManager::new(storage_path);

        let oid = Oid::hash(b"commit");
        mgr.create("main", oid).await.unwrap();
        mgr.delete("main").await.unwrap();

        assert!(!mgr.exists("main").await.unwrap());
    }

    #[tokio::test]
    async fn test_branch_switch() {
        let temp_dir = tempdir().unwrap();
        let storage_path = temp_dir.path();
        let mgr = BranchManager::new(storage_path);

        let oid = Oid::hash(b"commit");
        mgr.create("main", oid).await.unwrap();
        mgr.create("develop", oid).await.unwrap();

        mgr.switch_to("main").await.unwrap();
        let current = mgr.current_branch().await.unwrap();
        assert_eq!(current, Some("main".to_string()));

        mgr.switch_to("develop").await.unwrap();
        let current = mgr.current_branch().await.unwrap();
        assert_eq!(current, Some("develop".to_string()));
    }

    #[tokio::test]
    async fn test_branch_list() {
        let temp_dir = tempdir().unwrap();
        let storage_path = temp_dir.path();
        let mgr = BranchManager::new(storage_path);

        let oid = Oid::hash(b"commit");
        mgr.create("main", oid).await.unwrap();
        mgr.create("develop", oid).await.unwrap();

        let branches = mgr.list().await.unwrap();
        assert_eq!(branches.len(), 2);
    }

    #[tokio::test]
    async fn test_branch_rename() {
        let temp_dir = tempdir().unwrap();
        let storage_path = temp_dir.path();
        let mgr = BranchManager::new(storage_path);

        let oid = Oid::hash(b"commit");
        mgr.create("main", oid).await.unwrap();
        mgr.rename("main", "master").await.unwrap();

        assert!(!mgr.exists("main").await.unwrap());
        assert!(mgr.exists("master").await.unwrap());
    }

    #[tokio::test]
    async fn test_branch_get_info() {
        let temp_dir = tempdir().unwrap();
        let storage_path = temp_dir.path();
        let mgr = BranchManager::new(storage_path);

        let oid = Oid::hash(b"commit");
        mgr.create("main", oid).await.unwrap();

        let info = mgr.get_info("main").await.unwrap();
        assert_eq!(info.name, "main");
        assert_eq!(info.oid, oid);
    }

    #[tokio::test]
    async fn test_detach_head() {
        let temp_dir = tempdir().unwrap();
        let storage_path = temp_dir.path();
        let mgr = BranchManager::new(storage_path);

        let oid = Oid::hash(b"commit");
        mgr.create("main", oid).await.unwrap();
        mgr.switch_to("main").await.unwrap();

        let detached = mgr.detach_head(oid).await.unwrap();
        assert_eq!(detached.commit_oid, oid);

        assert!(mgr.is_detached().await.unwrap());
    }

    #[tokio::test]
    async fn test_head_commit() {
        let temp_dir = tempdir().unwrap();
        let storage_path = temp_dir.path();
        let mgr = BranchManager::new(storage_path);

        let oid = Oid::hash(b"commit");
        mgr.create("main", oid).await.unwrap();
        mgr.switch_to("main").await.unwrap();

        let head_oid = mgr.head_commit().await.unwrap();
        assert_eq!(head_oid, oid);
    }

    #[tokio::test]
    async fn test_update_branch() {
        let temp_dir = tempdir().unwrap();
        let storage_path = temp_dir.path();
        let mgr = BranchManager::new(storage_path);

        let oid1 = Oid::hash(b"commit1");
        let oid2 = Oid::hash(b"commit2");

        mgr.create("main", oid1).await.unwrap();
        // Use force=true since we don't have real commit history
        mgr.update_to("main", oid2, true).await.unwrap();

        let info = mgr.get_info("main").await.unwrap();
        assert_eq!(info.oid, oid2);
    }

    #[tokio::test]
    async fn test_create_tag() {
        let temp_dir = tempdir().unwrap();
        let storage_path = temp_dir.path();
        let mgr = BranchManager::new(storage_path);

        let oid = Oid::hash(b"commit");
        mgr.create_tag("v1.0.0", oid).await.unwrap();

        let tags = mgr.list_tags().await.unwrap();
        assert!(tags.iter().any(|t| t.ends_with("v1.0.0")));
    }

    #[tokio::test]
    async fn test_delete_tag() {
        let temp_dir = tempdir().unwrap();
        let storage_path = temp_dir.path();
        let mgr = BranchManager::new(storage_path);

        let oid = Oid::hash(b"commit");
        mgr.create_tag("v1.0.0", oid).await.unwrap();
        mgr.delete_tag("v1.0.0").await.unwrap();

        let tags = mgr.list_tags().await.unwrap();
        assert!(!tags.iter().any(|t| t.ends_with("v1.0.0")));
    }

    #[tokio::test]
    async fn test_initialize() {
        let temp_dir = tempdir().unwrap();
        let storage_path = temp_dir.path();
        let mgr = BranchManager::new(storage_path);

        let oid = Oid::hash(b"commit");
        mgr.initialize(Some(oid)).await.unwrap();

        let current = mgr.current_branch().await.unwrap();
        assert_eq!(current, Some("main".to_string()));
    }

    #[test]
    fn test_validate_branch_name_empty() {
        let temp_dir = tempdir().unwrap();
        let storage_path = temp_dir.path();
        let mgr = BranchManager::new(storage_path);
        assert!(mgr.validate_branch_name("").is_err());
    }

    #[test]
    fn test_validate_branch_name_starts_with_dash() {
        let temp_dir = tempdir().unwrap();
        let storage_path = temp_dir.path();
        let mgr = BranchManager::new(storage_path);
        assert!(mgr.validate_branch_name("-invalid").is_err());
    }

    #[test]
    fn test_validate_branch_name_valid() {
        let temp_dir = tempdir().unwrap();
        let storage_path = temp_dir.path();
        let mgr = BranchManager::new(storage_path);
        assert!(mgr.validate_branch_name("feature/auth").is_ok());
        assert!(mgr.validate_branch_name("main").is_ok());
        assert!(mgr.validate_branch_name("develop").is_ok());
    }
}
