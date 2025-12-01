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

//! Tree diffing for 3-way merge operations
//!
//! Implements algorithms for comparing tree snapshots and detecting changes.
//! Essential for merge conflict detection and resolution.
//!
//! # Diff Types
//!
//! - **Two-way diff**: Compare two trees directly (A vs B)
//! - **Three-way diff**: Compare base → ours and base → theirs
//!
//! # Change Classification
//!
//! - **Added**: File exists in target but not in source
//! - **Deleted**: File exists in source but not in target
//! - **Modified**: File exists in both but with different content

use crate::{ObjectDatabase, Oid, Tree, TreeEntry};
use std::collections::HashMap;
use std::sync::Arc;
use tracing::{debug, trace};

/// Two-way tree diff result
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TreeDiff {
    /// Files added in target (not in source)
    pub added: Vec<TreeEntry>,

    /// Files deleted from source (not in target)
    pub deleted: Vec<TreeEntry>,

    /// Files modified (exist in both with different OIDs)
    pub modified: Vec<ModifiedEntry>,
}

/// Modified file entry with before/after states
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModifiedEntry {
    /// File path
    pub path: String,

    /// Source (before) entry
    pub source: TreeEntry,

    /// Target (after) entry
    pub target: TreeEntry,
}

/// Three-way diff result for merge operations
#[derive(Debug, Clone)]
pub struct ThreeWayDiff {
    /// Changes from base to ours
    pub ours_changes: TreeDiff,

    /// Changes from base to theirs
    pub theirs_changes: TreeDiff,

    /// Files modified on both sides (potential conflicts)
    pub both_modified: Vec<String>,

    /// Files only modified by us
    pub only_ours: Vec<String>,

    /// Files only modified by them
    pub only_theirs: Vec<String>,

    /// Files modified identically on both sides (auto-merge)
    pub same_changes: Vec<String>,
}

/// Tree differ for comparing snapshots
pub struct TreeDiffer {
    odb: Arc<ObjectDatabase>,
}

impl TreeDiffer {
    /// Create a new tree differ
    ///
    /// # Arguments
    ///
    /// * `odb` - Object database for reading trees
    pub fn new(odb: Arc<ObjectDatabase>) -> Self {
        Self { odb }
    }

    /// Diff two trees
    ///
    /// Compares source tree to target tree and identifies added, deleted,
    /// and modified files.
    ///
    /// # Arguments
    ///
    /// * `source_oid` - Source tree OID (before)
    /// * `target_oid` - Target tree OID (after)
    ///
    /// # Returns
    ///
    /// TreeDiff with categorized changes
    pub async fn diff_trees(
        &self,
        source_oid: &Oid,
        target_oid: &Oid,
    ) -> anyhow::Result<TreeDiff> {
        debug!(source = %source_oid, target = %target_oid, "Diffing trees");

        // Same tree = no changes
        if source_oid == target_oid {
            return Ok(TreeDiff {
                added: Vec::new(),
                deleted: Vec::new(),
                modified: Vec::new(),
            });
        }

        let source_tree = Tree::read(&self.odb, source_oid).await?;
        let target_tree = Tree::read(&self.odb, target_oid).await?;

        let mut added = Vec::new();
        let mut deleted = Vec::new();
        let mut modified = Vec::new();

        // Build maps for efficient lookup
        let source_map: HashMap<String, &TreeEntry> = source_tree
            .entries
            .values()
            .map(|e| (e.name.clone(), e))
            .collect();

        let target_map: HashMap<String, &TreeEntry> = target_tree
            .entries
            .values()
            .map(|e| (e.name.clone(), e))
            .collect();

        // Find added and modified files
        for (name, target_entry) in &target_map {
            match source_map.get(name) {
                None => {
                    // File added
                    added.push((*target_entry).clone());
                    trace!(file = %name, "Added");
                }
                Some(source_entry) => {
                    // File exists in both - check if modified
                    if source_entry.oid != target_entry.oid
                        || source_entry.mode != target_entry.mode
                    {
                        modified.push(ModifiedEntry {
                            path: name.clone(),
                            source: (*source_entry).clone(),
                            target: (*target_entry).clone(),
                        });
                        trace!(file = %name, "Modified");
                    }
                }
            }
        }

        // Find deleted files
        for (name, source_entry) in &source_map {
            if !target_map.contains_key(name) {
                deleted.push((*source_entry).clone());
                trace!(file = %name, "Deleted");
            }
        }

        debug!(
            added = added.len(),
            deleted = deleted.len(),
            modified = modified.len(),
            "Diff complete"
        );

        Ok(TreeDiff {
            added,
            deleted,
            modified,
        })
    }

    /// Perform 3-way diff for merge operations
    ///
    /// Compares base → ours and base → theirs to identify:
    /// - Changes made only by us
    /// - Changes made only by them
    /// - Changes made by both (potential conflicts)
    /// - Identical changes (auto-mergeable)
    ///
    /// # Arguments
    ///
    /// * `base_oid` - Common ancestor tree OID
    /// * `ours_oid` - Our tree OID (current branch)
    /// * `theirs_oid` - Their tree OID (branch to merge)
    ///
    /// # Returns
    ///
    /// ThreeWayDiff with categorized changes
    pub async fn three_way_diff(
        &self,
        base_oid: &Oid,
        ours_oid: &Oid,
        theirs_oid: &Oid,
    ) -> anyhow::Result<ThreeWayDiff> {
        debug!(
            base = %base_oid,
            ours = %ours_oid,
            theirs = %theirs_oid,
            "Performing 3-way diff"
        );

        // Get diffs from base to each side
        let ours_changes = self.diff_trees(base_oid, ours_oid).await?;
        let theirs_changes = self.diff_trees(base_oid, theirs_oid).await?;

        // Categorize changes
        let mut both_modified = Vec::new();
        let mut only_ours = Vec::new();
        let mut only_theirs = Vec::new();
        let mut same_changes = Vec::new();

        // Build sets of modified file paths
        let ours_modified: HashMap<String, &ModifiedEntry> = ours_changes
            .modified
            .iter()
            .map(|e| (e.path.clone(), e))
            .collect();

        let theirs_modified: HashMap<String, &ModifiedEntry> = theirs_changes
            .modified
            .iter()
            .map(|e| (e.path.clone(), e))
            .collect();

        // Check for files modified by both sides
        for (path, ours_entry) in &ours_modified {
            if let Some(theirs_entry) = theirs_modified.get(path) {
                // Both sides modified this file
                if ours_entry.target.oid == theirs_entry.target.oid {
                    // Same modification - auto-merge
                    same_changes.push(path.clone());
                    trace!(file = %path, "Same change on both sides");
                } else {
                    // Different modifications - conflict
                    both_modified.push(path.clone());
                    trace!(file = %path, "Modified on both sides (conflict)");
                }
            }
        }

        // Find files only modified by us
        for path in ours_modified.keys() {
            if !theirs_modified.contains_key(path) && !same_changes.contains(path) {
                only_ours.push(path.clone());
                trace!(file = %path, "Only modified by us");
            }
        }

        // Find files only modified by them
        for path in theirs_modified.keys() {
            if !ours_modified.contains_key(path) && !same_changes.contains(path) {
                only_theirs.push(path.clone());
                trace!(file = %path, "Only modified by them");
            }
        }

        // Handle additions and deletions
        let ours_added: HashMap<String, &TreeEntry> = ours_changes
            .added
            .iter()
            .map(|e| (e.name.clone(), e))
            .collect();

        let theirs_added: HashMap<String, &TreeEntry> = theirs_changes
            .added
            .iter()
            .map(|e| (e.name.clone(), e))
            .collect();

        for (name, ours_entry) in &ours_added {
            if let Some(theirs_entry) = theirs_added.get(name) {
                // Both added same file
                if ours_entry.oid == theirs_entry.oid {
                    same_changes.push(name.clone());
                } else {
                    both_modified.push(name.clone());
                }
            } else {
                only_ours.push(name.clone());
            }
        }

        for name in theirs_added.keys() {
            if !ours_added.contains_key(name) {
                only_theirs.push(name.clone());
            }
        }

        // Handle deletions
        let ours_deleted: Vec<String> = ours_changes.deleted.iter().map(|e| e.name.clone()).collect();
        let theirs_deleted: Vec<String> = theirs_changes.deleted.iter().map(|e| e.name.clone()).collect();

        for name in &ours_deleted {
            if theirs_deleted.contains(name) {
                same_changes.push(name.clone());
            } else {
                only_ours.push(name.clone());
            }
        }

        for name in &theirs_deleted {
            if !ours_deleted.contains(name) {
                only_theirs.push(name.clone());
            }
        }

        debug!(
            both_modified = both_modified.len(),
            only_ours = only_ours.len(),
            only_theirs = only_theirs.len(),
            same_changes = same_changes.len(),
            "3-way diff complete"
        );

        Ok(ThreeWayDiff {
            ours_changes,
            theirs_changes,
            both_modified,
            only_ours,
            only_theirs,
            same_changes,
        })
    }

    /// Check if two trees are identical
    pub async fn are_trees_equal(&self, oid1: &Oid, oid2: &Oid) -> anyhow::Result<bool> {
        if oid1 == oid2 {
            return Ok(true);
        }

        let diff = self.diff_trees(oid1, oid2).await?;
        Ok(diff.added.is_empty() && diff.deleted.is_empty() && diff.modified.is_empty())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{FileMode, Oid};
    use mediagit_storage::mock::MockBackend;

    async fn create_tree(odb: &Arc<ObjectDatabase>, entries: Vec<(&str, &[u8])>) -> Oid {
        let mut tree = Tree::new();
        for (name, content) in entries {
            let oid = Oid::hash(content);
            let entry = TreeEntry::new(name.to_string(), FileMode::Regular, oid);
            tree.add_entry(entry);
        }
        tree.write(odb).await.unwrap()
    }

    #[tokio::test]
    async fn test_diff_identical_trees() {
        let storage = Arc::new(MockBackend::new());
        let odb = Arc::new(ObjectDatabase::new(storage, 100));
        let differ = TreeDiffer::new(odb.clone());

        let tree_oid = create_tree(&odb, vec![("file.txt", b"content")]).await;

        let diff = differ.diff_trees(&tree_oid, &tree_oid).await.unwrap();

        assert_eq!(diff.added.len(), 0);
        assert_eq!(diff.deleted.len(), 0);
        assert_eq!(diff.modified.len(), 0);
    }

    #[tokio::test]
    async fn test_diff_added_file() {
        let storage = Arc::new(MockBackend::new());
        let odb = Arc::new(ObjectDatabase::new(storage, 100));
        let differ = TreeDiffer::new(odb.clone());

        let source = create_tree(&odb, vec![("file1.txt", b"content1")]).await;
        let target = create_tree(
            &odb,
            vec![("file1.txt", b"content1"), ("file2.txt", b"content2")],
        )
        .await;

        let diff = differ.diff_trees(&source, &target).await.unwrap();

        assert_eq!(diff.added.len(), 1);
        assert_eq!(diff.added[0].name, "file2.txt");
        assert_eq!(diff.deleted.len(), 0);
        assert_eq!(diff.modified.len(), 0);
    }

    #[tokio::test]
    async fn test_diff_deleted_file() {
        let storage = Arc::new(MockBackend::new());
        let odb = Arc::new(ObjectDatabase::new(storage, 100));
        let differ = TreeDiffer::new(odb.clone());

        let source = create_tree(
            &odb,
            vec![("file1.txt", b"content1"), ("file2.txt", b"content2")],
        )
        .await;
        let target = create_tree(&odb, vec![("file1.txt", b"content1")]).await;

        let diff = differ.diff_trees(&source, &target).await.unwrap();

        assert_eq!(diff.added.len(), 0);
        assert_eq!(diff.deleted.len(), 1);
        assert_eq!(diff.deleted[0].name, "file2.txt");
        assert_eq!(diff.modified.len(), 0);
    }

    #[tokio::test]
    async fn test_diff_modified_file() {
        let storage = Arc::new(MockBackend::new());
        let odb = Arc::new(ObjectDatabase::new(storage, 100));
        let differ = TreeDiffer::new(odb.clone());

        let source = create_tree(&odb, vec![("file.txt", b"original")]).await;
        let target = create_tree(&odb, vec![("file.txt", b"modified")]).await;

        let diff = differ.diff_trees(&source, &target).await.unwrap();

        assert_eq!(diff.added.len(), 0);
        assert_eq!(diff.deleted.len(), 0);
        assert_eq!(diff.modified.len(), 1);
        assert_eq!(diff.modified[0].path, "file.txt");
    }

    #[tokio::test]
    async fn test_three_way_diff_no_conflict() {
        let storage = Arc::new(MockBackend::new());
        let odb = Arc::new(ObjectDatabase::new(storage, 100));
        let differ = TreeDiffer::new(odb.clone());

        let base = create_tree(&odb, vec![("file.txt", b"base")]).await;
        let ours = create_tree(&odb, vec![("file.txt", b"base"), ("ours.txt", b"our change")]).await;
        let theirs = create_tree(&odb, vec![("file.txt", b"base"), ("theirs.txt", b"their change")]).await;

        let diff = differ.three_way_diff(&base, &ours, &theirs).await.unwrap();

        assert!(diff.both_modified.is_empty());
        assert_eq!(diff.only_ours.len(), 1);
        assert_eq!(diff.only_theirs.len(), 1);
        assert_eq!(diff.same_changes.len(), 0);
    }

    #[tokio::test]
    async fn test_three_way_diff_same_change() {
        let storage = Arc::new(MockBackend::new());
        let odb = Arc::new(ObjectDatabase::new(storage, 100));
        let differ = TreeDiffer::new(odb.clone());

        let base = create_tree(&odb, vec![("file.txt", b"original")]).await;
        let ours = create_tree(&odb, vec![("file.txt", b"modified")]).await;
        let theirs = create_tree(&odb, vec![("file.txt", b"modified")]).await;

        let diff = differ.three_way_diff(&base, &ours, &theirs).await.unwrap();

        assert_eq!(diff.both_modified.len(), 0);
        assert_eq!(diff.same_changes.len(), 1);
        assert!(diff.same_changes.contains(&"file.txt".to_string()));
    }

    #[tokio::test]
    async fn test_three_way_diff_conflict() {
        let storage = Arc::new(MockBackend::new());
        let odb = Arc::new(ObjectDatabase::new(storage, 100));
        let differ = TreeDiffer::new(odb.clone());

        let base = create_tree(&odb, vec![("file.txt", b"original")]).await;
        let ours = create_tree(&odb, vec![("file.txt", b"our modification")]).await;
        let theirs = create_tree(&odb, vec![("file.txt", b"their modification")]).await;

        let diff = differ.three_way_diff(&base, &ours, &theirs).await.unwrap();

        assert_eq!(diff.both_modified.len(), 1);
        assert!(diff.both_modified.contains(&"file.txt".to_string()));
        assert_eq!(diff.same_changes.len(), 0);
    }

    #[tokio::test]
    async fn test_three_way_diff_add_add_same() {
        let storage = Arc::new(MockBackend::new());
        let odb = Arc::new(ObjectDatabase::new(storage, 100));
        let differ = TreeDiffer::new(odb.clone());

        let base = create_tree(&odb, vec![]).await;
        let ours = create_tree(&odb, vec![("new.txt", b"same content")]).await;
        let theirs = create_tree(&odb, vec![("new.txt", b"same content")]).await;

        let diff = differ.three_way_diff(&base, &ours, &theirs).await.unwrap();

        assert_eq!(diff.both_modified.len(), 0);
        assert_eq!(diff.same_changes.len(), 1);
        assert!(diff.same_changes.contains(&"new.txt".to_string()));
    }

    #[tokio::test]
    async fn test_three_way_diff_add_add_different() {
        let storage = Arc::new(MockBackend::new());
        let odb = Arc::new(ObjectDatabase::new(storage, 100));
        let differ = TreeDiffer::new(odb.clone());

        let base = create_tree(&odb, vec![]).await;
        let ours = create_tree(&odb, vec![("new.txt", b"our content")]).await;
        let theirs = create_tree(&odb, vec![("new.txt", b"their content")]).await;

        let diff = differ.three_way_diff(&base, &ours, &theirs).await.unwrap();

        assert_eq!(diff.both_modified.len(), 1);
        assert!(diff.both_modified.contains(&"new.txt".to_string()));
    }

    #[tokio::test]
    async fn test_are_trees_equal() {
        let storage = Arc::new(MockBackend::new());
        let odb = Arc::new(ObjectDatabase::new(storage, 100));
        let differ = TreeDiffer::new(odb.clone());

        let tree1 = create_tree(&odb, vec![("file.txt", b"content")]).await;
        let tree2 = create_tree(&odb, vec![("file.txt", b"content")]).await;
        let tree3 = create_tree(&odb, vec![("file.txt", b"different")]).await;

        assert!(differ.are_trees_equal(&tree1, &tree1).await.unwrap());
        assert!(differ.are_trees_equal(&tree1, &tree2).await.unwrap());
        assert!(!differ.are_trees_equal(&tree1, &tree3).await.unwrap());
    }

    #[tokio::test]
    async fn test_complex_three_way_diff() {
        let storage = Arc::new(MockBackend::new());
        let odb = Arc::new(ObjectDatabase::new(storage, 100));
        let differ = TreeDiffer::new(odb.clone());

        let base = create_tree(
            &odb,
            vec![
                ("file1.txt", b"original1"),
                ("file2.txt", b"original2"),
                ("file3.txt", b"original3"),
            ],
        )
        .await;

        let ours = create_tree(
            &odb,
            vec![
                ("file1.txt", b"modified1"),  // Modified by us
                ("file2.txt", b"original2"),  // Unchanged
                ("ours_new.txt", b"new file"), // Added by us
            ],
        )
        .await;

        let theirs = create_tree(
            &odb,
            vec![
                ("file1.txt", b"their_mod1"),    // Modified differently
                ("file3.txt", b"original3"),    // Unchanged
                ("theirs_new.txt", b"their new"), // Added by them
            ],
        )
        .await;

        let diff = differ.three_way_diff(&base, &ours, &theirs).await.unwrap();

        // file1.txt modified by both (conflict)
        assert!(diff.both_modified.contains(&"file1.txt".to_string()));

        // file2.txt deleted by them only
        assert!(diff.only_theirs.contains(&"file2.txt".to_string()));

        // file3.txt deleted by us only
        assert!(diff.only_ours.contains(&"file3.txt".to_string()));

        // ours_new.txt added by us only
        assert!(diff.only_ours.contains(&"ours_new.txt".to_string()));

        // theirs_new.txt added by them only
        assert!(diff.only_theirs.contains(&"theirs_new.txt".to_string()));
    }
}
