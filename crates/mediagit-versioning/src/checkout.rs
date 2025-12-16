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

//! Checkout operations for restoring working directory from commits
//!
//! This module provides functionality to update the working directory
//! to match a specific commit's tree structure.

use crate::{Commit, FileMode, ObjectDatabase, Oid, Tree};
use anyhow::{Context, Result};
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use tracing::{debug, info};

/// Checkout manager for working directory operations
pub struct CheckoutManager<'a> {
    odb: &'a ObjectDatabase,
    repo_root: PathBuf,
}

impl<'a> CheckoutManager<'a> {
    /// Create a new checkout manager
    pub fn new(odb: &'a ObjectDatabase, repo_root: impl Into<PathBuf>) -> Self {
        Self {
            odb,
            repo_root: repo_root.into(),
        }
    }

    /// Checkout a commit, updating the working directory to match its tree
    ///
    /// This operation:
    /// 1. Removes files not in the target commit
    /// 2. Writes all files from the target commit's tree
    /// 3. Preserves the .mediagit directory
    ///
    /// # Arguments
    ///
    /// * `commit_oid` - The commit to checkout
    ///
    /// # Returns
    ///
    /// Number of files updated
    pub async fn checkout_commit(&self, commit_oid: &Oid) -> Result<usize> {
        info!("Checking out commit: {}", commit_oid);

        // Read the commit
        let commit = Commit::read(self.odb, commit_oid).await?;

        debug!("Commit tree: {}", commit.tree);

        // Get all files that should exist from the tree
        let target_files = self.get_tree_files(&commit.tree, Path::new("")).await?;
        debug!("Target files: {} entries", target_files.len());

        // Clean working directory (remove files not in target)
        self.clean_working_directory(&target_files)?;

        // Checkout all files from the tree
        let files_updated = self.checkout_tree(&commit.tree, Path::new("")).await?;

        info!("Checked out {} files", files_updated);
        Ok(files_updated)
    }

    /// Get all file paths from a tree recursively
    fn get_tree_files<'b>(
        &'b self,
        tree_oid: &'b Oid,
        prefix: &'b Path,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<HashSet<PathBuf>>> + 'b>> {
        Box::pin(async move {
            let tree = Tree::read(self.odb, tree_oid).await?;

            let mut files = HashSet::new();

            for entry in tree.iter() {
                let entry_path = prefix.join(&entry.name);

                match entry.mode {
                    FileMode::Regular | FileMode::Executable | FileMode::Symlink => {
                        files.insert(entry_path);
                    }
                    FileMode::Directory => {
                        // Recursively get files from subdirectory
                        let subdir_files = self.get_tree_files(&entry.oid, &entry_path).await?;
                        files.extend(subdir_files);
                    }
                }
            }

            Ok(files)
        })
    }

    /// Clean working directory, removing files not in target set
    fn clean_working_directory(&self, target_files: &HashSet<PathBuf>) -> Result<()> {
        debug!("Cleaning working directory");

        // Get all files in working directory (excluding .mediagit)
        let existing_files = self.list_working_directory_files()?;

        for file in existing_files {
            if !target_files.contains(&file) {
                let file_path = self.repo_root.join(&file);
                debug!("Removing file not in target: {}", file.display());

                if file_path.exists() {
                    fs::remove_file(&file_path)
                        .with_context(|| format!("Failed to remove file: {}", file_path.display()))?;
                }
            }
        }

        // Remove empty directories
        self.remove_empty_directories()?;

        Ok(())
    }

    /// List all files in working directory (excluding .mediagit)
    fn list_working_directory_files(&self) -> Result<Vec<PathBuf>> {
        let mut files = Vec::new();
        self.list_files_recursive(&self.repo_root, &mut files)?;
        Ok(files)
    }

    /// Recursively list files in a directory
    fn list_files_recursive(&self, dir: &Path, files: &mut Vec<PathBuf>) -> Result<()> {
        if !dir.exists() || !dir.is_dir() {
            return Ok(());
        }

        for entry in fs::read_dir(dir)? {
            let entry = entry?;
            let path = entry.path();

            // Skip .mediagit directory
            if path.file_name().and_then(|n| n.to_str()) == Some(".mediagit") {
                continue;
            }

            if path.is_file() {
                // Store relative path from repo root
                let rel_path = path.strip_prefix(&self.repo_root)
                    .context("Failed to compute relative path")?;
                files.push(rel_path.to_path_buf());
            } else if path.is_dir() {
                self.list_files_recursive(&path, files)?;
            }
        }

        Ok(())
    }

    /// Remove empty directories in working tree
    fn remove_empty_directories(&self) -> Result<()> {
        // Simple implementation: try to remove all directories
        // Non-empty ones will fail to remove, which is fine
        self.try_remove_empty_dirs(&self.repo_root)?;
        Ok(())
    }

    /// Try to remove empty directories recursively
    fn try_remove_empty_dirs(&self, dir: &Path) -> Result<()> {
        if !dir.exists() || !dir.is_dir() || dir == self.repo_root {
            return Ok(());
        }

        for entry in fs::read_dir(dir)? {
            let entry = entry?;
            let path = entry.path();

            // Skip .mediagit
            if path.file_name().and_then(|n| n.to_str()) == Some(".mediagit") {
                continue;
            }

            if path.is_dir() {
                self.try_remove_empty_dirs(&path)?;
            }
        }

        // Try to remove this directory (will only succeed if empty)
        let _ = fs::remove_dir(dir);

        Ok(())
    }

    /// Checkout a tree recursively, writing all files to working directory
    fn checkout_tree<'b>(
        &'b self,
        tree_oid: &'b Oid,
        prefix: &'b Path,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<usize>> + 'b>> {
        Box::pin(async move {
            let tree = Tree::read(self.odb, tree_oid).await?;

            let mut files_updated = 0;

            for entry in tree.iter() {
                let entry_path = prefix.join(&entry.name);
                let full_path = self.repo_root.join(&entry_path);

                match entry.mode {
                    FileMode::Regular | FileMode::Executable => {
                        // Read blob data
                        let blob_data = self.odb.read(&entry.oid).await
                            .with_context(|| format!("Failed to read blob: {}", entry.oid))?;

                        // Ensure parent directory exists
                        if let Some(parent) = full_path.parent() {
                            fs::create_dir_all(parent)
                                .with_context(|| format!("Failed to create directory: {}", parent.display()))?;
                        }

                        // Write file
                        fs::write(&full_path, blob_data)
                            .with_context(|| format!("Failed to write file: {}", full_path.display()))?;

                        // Set executable permission if needed
                        #[cfg(unix)]
                        if entry.mode == FileMode::Executable {
                            use std::os::unix::fs::PermissionsExt;
                            let mut perms = fs::metadata(&full_path)?.permissions();
                            perms.set_mode(0o755);
                            fs::set_permissions(&full_path, perms)?;
                        }

                        debug!("Checked out file: {}", entry_path.display());
                        files_updated += 1;
                    }
                    FileMode::Symlink => {
                        // Read symlink target
                        let target_data = self.odb.read(&entry.oid).await?;
                        let target = String::from_utf8(target_data)
                            .context("Symlink target is not valid UTF-8")?;

                        // Ensure parent directory exists
                        if let Some(parent) = full_path.parent() {
                            fs::create_dir_all(parent)?;
                        }

                        // Create symlink
                        #[cfg(unix)]
                        {
                            use std::os::unix::fs::symlink;
                            // Remove existing file/link if present
                            let _ = fs::remove_file(&full_path);
                            symlink(&target, &full_path)
                                .with_context(|| format!("Failed to create symlink: {}", full_path.display()))?;
                        }

                        #[cfg(not(unix))]
                        {
                            debug!("Symlinks not supported on this platform, skipping: {}", entry_path.display());
                        }

                        files_updated += 1;
                    }
                    FileMode::Directory => {
                        // Recursively checkout subdirectory
                        let subdir_count = self.checkout_tree(&entry.oid, &entry_path).await?;
                        files_updated += subdir_count;
                    }
                }
            }

            Ok(files_updated)
        })
    }

    /// Checkout to an empty working directory
    ///
    /// Useful for initial clone or reset operations
    pub async fn checkout_fresh(&self, commit_oid: &Oid) -> Result<usize> {
        info!("Performing fresh checkout of commit: {}", commit_oid);

        // Read the commit
        let commit = Commit::read(self.odb, commit_oid).await?;

        // Checkout tree without cleaning (assume empty directory)
        self.checkout_tree(&commit.tree, Path::new("")).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ObjectType, Signature, TreeEntry};
    use mediagit_storage::LocalBackend;
    use std::sync::Arc;
    use tempfile::TempDir;

    #[tokio::test]
    async fn test_checkout_commit() -> Result<()> {
        let temp_dir = TempDir::new()?;
        let repo_root = temp_dir.path();
        let storage_path = repo_root.join(".mediagit");
        fs::create_dir_all(&storage_path)?;

        let storage = Arc::new(LocalBackend::new(&storage_path).await?);
        let odb = ObjectDatabase::new(storage, 100);

        // Create a test commit with a file
        let file_data = b"Hello, MediaGit!";
        let blob_oid = odb.write(ObjectType::Blob, file_data).await?;

        let mut tree = Tree::new();
        tree.add_entry(TreeEntry::new(
            "README.md".to_string(),
            FileMode::Regular,
            blob_oid,
        ));

        let tree_data = serde_json::to_vec(&tree)?;
        let tree_oid = odb.write(ObjectType::Tree, &tree_data).await?;

        let commit = Commit::new(
            tree_oid,
            Signature::now("Test".to_string(), "test@example.com".to_string()),
            Signature::now("Test".to_string(), "test@example.com".to_string()),
            "Initial commit".to_string(),
        );

        let commit_data = serde_json::to_vec(&commit)?;
        let commit_oid = odb.write(ObjectType::Commit, &commit_data).await?;

        // Checkout the commit
        let checkout_mgr = CheckoutManager::new(&odb, repo_root);
        let files_updated = checkout_mgr.checkout_commit(&commit_oid).await?;

        assert_eq!(files_updated, 1);

        // Verify file exists
        let file_path = repo_root.join("README.md");
        assert!(file_path.exists());

        let contents = fs::read(&file_path)?;
        assert_eq!(contents, file_data);

        Ok(())
    }
}
