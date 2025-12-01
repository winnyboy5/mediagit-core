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

//! Conflict detection for 3-way merge operations
//!
//! This module detects conflicts when merging two branches by comparing their
//! changes against a common base commit. It categorizes conflicts and determines
//! which changes can be auto-merged.

use crate::{ObjectDatabase, Oid, Tree, TreeEntry};
use anyhow::{anyhow, Result};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use tracing::{debug, instrument, trace};

/// Type of merge conflict detected
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConflictType {
    /// Both sides modified the same file with different content
    ModifyModify,

    /// Both sides added the same path with different content
    AddAdd,

    /// One side deleted while the other modified
    DeleteModify,

    /// One side modified while the other deleted
    ModifyDelete,
}

/// One side of a conflict (base, ours, or theirs)
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConflictSide {
    /// Object ID of the content
    pub oid: Oid,

    /// File mode
    pub mode: u32,
}

impl ConflictSide {
    /// Create a new conflict side from a tree entry
    pub fn from_entry(entry: &TreeEntry) -> Self {
        Self {
            oid: entry.oid,
            mode: entry.mode.as_u32(),
        }
    }
}

/// A detected merge conflict
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Conflict {
    /// Path to the conflicting file
    pub path: String,

    /// Type of conflict
    pub conflict_type: ConflictType,

    /// Base version (None if file didn't exist in base)
    pub base: Option<ConflictSide>,

    /// Our version (None if we deleted it)
    pub ours: Option<ConflictSide>,

    /// Their version (None if they deleted it)
    pub theirs: Option<ConflictSide>,
}

impl Conflict {
    /// Create a ModifyModify conflict
    pub fn modify_modify(
        path: String,
        base: ConflictSide,
        ours: ConflictSide,
        theirs: ConflictSide,
    ) -> Self {
        Self {
            path,
            conflict_type: ConflictType::ModifyModify,
            base: Some(base),
            ours: Some(ours),
            theirs: Some(theirs),
        }
    }

    /// Create an AddAdd conflict
    pub fn add_add(path: String, ours: ConflictSide, theirs: ConflictSide) -> Self {
        Self {
            path,
            conflict_type: ConflictType::AddAdd,
            base: None,
            ours: Some(ours),
            theirs: Some(theirs),
        }
    }

    /// Create a DeleteModify conflict
    pub fn delete_modify(path: String, base: ConflictSide, modifier: ConflictSide) -> Self {
        Self {
            path,
            conflict_type: ConflictType::DeleteModify,
            base: Some(base),
            ours: None,
            theirs: Some(modifier),
        }
    }

    /// Create a ModifyDelete conflict
    pub fn modify_delete(path: String, base: ConflictSide, modifier: ConflictSide) -> Self {
        Self {
            path,
            conflict_type: ConflictType::ModifyDelete,
            base: Some(base),
            ours: Some(modifier),
            theirs: None,
        }
    }
}

/// Detects conflicts in 3-way merge operations
pub struct ConflictDetector {
    #[allow(dead_code)]
    odb: Arc<ObjectDatabase>,
}

impl ConflictDetector {
    /// Create a new conflict detector
    pub fn new(odb: Arc<ObjectDatabase>) -> Self {
        Self { odb }
    }

    /// Detect conflicts between base, ours, and theirs trees
    ///
    /// Returns a list of conflicts that cannot be auto-merged.
    /// Changes that can be auto-merged are not included.
    ///
    /// # Auto-merge Rules
    ///
    /// - Same change on both sides → auto-merge (no conflict)
    /// - Change on one side only → auto-merge (no conflict)
    /// - Different changes to same file → conflict
    /// - Delete vs modify → conflict
    #[instrument(skip(self, base, ours, theirs))]
    pub async fn detect_conflicts(
        &self,
        base: &Tree,
        ours: &Tree,
        theirs: &Tree,
    ) -> Result<Vec<Conflict>> {
        debug!("Detecting conflicts in 3-way merge");

        let mut conflicts = Vec::new();

        // Build maps for efficient lookup
        let base_map: HashMap<&str, &TreeEntry> =
            base.entries.iter().map(|(k, v)| (k.as_str(), v)).collect();
        let ours_map: HashMap<&str, &TreeEntry> =
            ours.entries.iter().map(|(k, v)| (k.as_str(), v)).collect();
        let theirs_map: HashMap<&str, &TreeEntry> =
            theirs.entries.iter().map(|(k, v)| (k.as_str(), v)).collect();

        // Collect all paths that appear in any tree
        let mut all_paths = HashSet::new();
        all_paths.extend(base_map.keys().copied());
        all_paths.extend(ours_map.keys().copied());
        all_paths.extend(theirs_map.keys().copied());

        // Check each path for conflicts
        for path in all_paths {
            let base_entry = base_map.get(path).copied();
            let ours_entry = ours_map.get(path).copied();
            let theirs_entry = theirs_map.get(path).copied();

            trace!("Checking path: {}", path);

            // Categorize the change
            match (base_entry, ours_entry, theirs_entry) {
                // All three present
                (Some(base), Some(ours), Some(theirs)) => {
                    let ours_changed = base.oid != ours.oid;
                    let theirs_changed = base.oid != theirs.oid;

                    if ours_changed && theirs_changed {
                        // Both modified
                        if ours.oid == theirs.oid {
                            // Same change on both sides → auto-merge
                            trace!("Same change on both sides: {}", path);
                        } else {
                            // Different changes → conflict
                            debug!("ModifyModify conflict: {}", path);
                            conflicts.push(Conflict::modify_modify(
                                path.to_string(),
                                ConflictSide::from_entry(base),
                                ConflictSide::from_entry(ours),
                                ConflictSide::from_entry(theirs),
                            ));
                        }
                    }
                    // If only one side changed, it's an auto-merge (no conflict)
                }

                // Base and ours present, theirs deleted
                (Some(base), Some(ours), None) => {
                    let ours_changed = base.oid != ours.oid;

                    if ours_changed {
                        // We modified, they deleted → conflict
                        debug!("ModifyDelete conflict: {}", path);
                        conflicts.push(Conflict::modify_delete(
                            path.to_string(),
                            ConflictSide::from_entry(base),
                            ConflictSide::from_entry(ours),
                        ));
                    }
                    // If we didn't change it, they just deleted it → auto-merge
                }

                // Base and theirs present, ours deleted
                (Some(base), None, Some(theirs)) => {
                    let theirs_changed = base.oid != theirs.oid;

                    if theirs_changed {
                        // They modified, we deleted → conflict
                        debug!("DeleteModify conflict: {}", path);
                        conflicts.push(Conflict::delete_modify(
                            path.to_string(),
                            ConflictSide::from_entry(base),
                            ConflictSide::from_entry(theirs),
                        ));
                    }
                    // If they didn't change it, we just deleted it → auto-merge
                }

                // Only ours and theirs present (both added)
                (None, Some(ours), Some(theirs)) => {
                    if ours.oid == theirs.oid {
                        // Same content added on both sides → auto-merge
                        trace!("Same addition on both sides: {}", path);
                    } else {
                        // Different content → conflict
                        debug!("AddAdd conflict: {}", path);
                        conflicts.push(Conflict::add_add(
                            path.to_string(),
                            ConflictSide::from_entry(ours),
                            ConflictSide::from_entry(theirs),
                        ));
                    }
                }

                // Only in base (both deleted) → auto-merge
                (Some(_), None, None) => {
                    trace!("Both deleted: {}", path);
                }

                // Only in ours (we added) → auto-merge
                (None, Some(_), None) => {
                    trace!("We added: {}", path);
                }

                // Only in theirs (they added) → auto-merge
                (None, None, Some(_)) => {
                    trace!("They added: {}", path);
                }

                // Should never happen (no entry in any tree)
                (None, None, None) => {
                    return Err(anyhow!("Impossible state: path exists in none of the trees"));
                }
            }
        }

        debug!("Detected {} conflicts", conflicts.len());
        Ok(conflicts)
    }

    /// Check if a specific path can be auto-merged
    ///
    /// Returns true if the changes can be merged automatically without conflict.
    pub async fn can_auto_merge(
        &self,
        path: &str,
        base: &Tree,
        ours: &Tree,
        theirs: &Tree,
    ) -> Result<bool> {
        let base_entry = base.entries.get(path);
        let ours_entry = ours.entries.get(path);
        let theirs_entry = theirs.entries.get(path);

        match (base_entry, ours_entry, theirs_entry) {
            // All three present
            (Some(base), Some(ours), Some(theirs)) => {
                let ours_changed = base.oid != ours.oid;
                let theirs_changed = base.oid != theirs.oid;

                if ours_changed && theirs_changed {
                    // Both changed - can auto-merge only if same change
                    Ok(ours.oid == theirs.oid)
                } else {
                    // Only one or neither changed - can auto-merge
                    Ok(true)
                }
            }

            // One side deleted
            (Some(base), Some(ours), None) => {
                // Theirs deleted - can auto-merge only if we didn't change it
                Ok(base.oid == ours.oid)
            }
            (Some(base), None, Some(theirs)) => {
                // Ours deleted - can auto-merge only if they didn't change it
                Ok(base.oid == theirs.oid)
            }

            // Both added
            (None, Some(ours), Some(theirs)) => {
                // Can auto-merge only if same content
                Ok(ours.oid == theirs.oid)
            }

            // All other cases are auto-mergeable
            _ => Ok(true),
        }
    }

    /// Get statistics about conflicts
    pub fn conflict_stats(&self, conflicts: &[Conflict]) -> ConflictStats {
        let mut stats = ConflictStats::default();

        for conflict in conflicts {
            match conflict.conflict_type {
                ConflictType::ModifyModify => stats.modify_modify += 1,
                ConflictType::AddAdd => stats.add_add += 1,
                ConflictType::DeleteModify => stats.delete_modify += 1,
                ConflictType::ModifyDelete => stats.modify_delete += 1,
            }
        }

        stats.total = conflicts.len();
        stats
    }
}

/// Statistics about detected conflicts
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ConflictStats {
    /// Total number of conflicts
    pub total: usize,

    /// Number of ModifyModify conflicts
    pub modify_modify: usize,

    /// Number of AddAdd conflicts
    pub add_add: usize,

    /// Number of DeleteModify conflicts
    pub delete_modify: usize,

    /// Number of ModifyDelete conflicts
    pub modify_delete: usize,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::FileMode;
    use mediagit_storage::mock::MockBackend;

    fn create_test_odb() -> Arc<ObjectDatabase> {
        let storage = Arc::new(MockBackend::new());
        Arc::new(ObjectDatabase::new(storage, 100))
    }

    async fn create_blob(_odb: &ObjectDatabase, content: &[u8]) -> Oid {
        Oid::hash(content)
    }

    fn create_tree(entries: Vec<(&str, Oid)>) -> Tree {
        let mut tree = Tree::new();
        for (name, oid) in entries {
            let entry = TreeEntry::new(name.to_string(), FileMode::Regular, oid);
            tree.add_entry(entry);
        }
        tree
    }

    #[tokio::test]
    async fn test_no_conflicts_identical() {
        let odb = create_test_odb();
        let detector = ConflictDetector::new(Arc::clone(&odb));

        let file_oid = create_blob(&odb, b"content").await;
        let tree = create_tree(vec![("file.txt", file_oid)]);

        let conflicts = detector
            .detect_conflicts(&tree, &tree, &tree)
            .await
            .unwrap();

        assert_eq!(conflicts.len(), 0, "Identical trees should have no conflicts");
    }

    #[tokio::test]
    async fn test_no_conflict_one_side_change() {
        let odb = create_test_odb();
        let detector = ConflictDetector::new(Arc::clone(&odb));

        let base_content = create_blob(&odb, b"base").await;
        let ours_content = create_blob(&odb, b"modified").await;

        let base = create_tree(vec![("file.txt", base_content)]);
        let ours = create_tree(vec![("file.txt", ours_content)]);
        let theirs = create_tree(vec![("file.txt", base_content)]);

        let conflicts = detector.detect_conflicts(&base, &ours, &theirs).await.unwrap();

        assert_eq!(conflicts.len(), 0, "One-sided change should auto-merge");
    }

    #[tokio::test]
    async fn test_conflict_modify_modify() {
        let odb = create_test_odb();
        let detector = ConflictDetector::new(Arc::clone(&odb));

        let base_content = create_blob(&odb, b"base").await;
        let ours_content = create_blob(&odb, b"ours").await;
        let theirs_content = create_blob(&odb, b"theirs").await;

        let base = create_tree(vec![("file.txt", base_content)]);
        let ours = create_tree(vec![("file.txt", ours_content)]);
        let theirs = create_tree(vec![("file.txt", theirs_content)]);

        let conflicts = detector.detect_conflicts(&base, &ours, &theirs).await.unwrap();

        assert_eq!(conflicts.len(), 1);
        assert_eq!(conflicts[0].conflict_type, ConflictType::ModifyModify);
        assert_eq!(conflicts[0].path, "file.txt");
    }

    #[tokio::test]
    async fn test_no_conflict_same_modification() {
        let odb = create_test_odb();
        let detector = ConflictDetector::new(Arc::clone(&odb));

        let base_content = create_blob(&odb, b"base").await;
        let modified_content = create_blob(&odb, b"modified").await;

        let base = create_tree(vec![("file.txt", base_content)]);
        let ours = create_tree(vec![("file.txt", modified_content)]);
        let theirs = create_tree(vec![("file.txt", modified_content)]);

        let conflicts = detector.detect_conflicts(&base, &ours, &theirs).await.unwrap();

        assert_eq!(conflicts.len(), 0, "Same modification should auto-merge");
    }

    #[tokio::test]
    async fn test_conflict_add_add_different() {
        let odb = create_test_odb();
        let detector = ConflictDetector::new(Arc::clone(&odb));

        let ours_content = create_blob(&odb, b"ours").await;
        let theirs_content = create_blob(&odb, b"theirs").await;

        let base = create_tree(vec![]);
        let ours = create_tree(vec![("file.txt", ours_content)]);
        let theirs = create_tree(vec![("file.txt", theirs_content)]);

        let conflicts = detector.detect_conflicts(&base, &ours, &theirs).await.unwrap();

        assert_eq!(conflicts.len(), 1);
        assert_eq!(conflicts[0].conflict_type, ConflictType::AddAdd);
        assert_eq!(conflicts[0].path, "file.txt");
    }

    #[tokio::test]
    async fn test_no_conflict_add_add_same() {
        let odb = create_test_odb();
        let detector = ConflictDetector::new(Arc::clone(&odb));

        let content = create_blob(&odb, b"same").await;

        let base = create_tree(vec![]);
        let ours = create_tree(vec![("file.txt", content)]);
        let theirs = create_tree(vec![("file.txt", content)]);

        let conflicts = detector.detect_conflicts(&base, &ours, &theirs).await.unwrap();

        assert_eq!(conflicts.len(), 0, "Same addition should auto-merge");
    }

    #[tokio::test]
    async fn test_conflict_delete_modify() {
        let odb = create_test_odb();
        let detector = ConflictDetector::new(Arc::clone(&odb));

        let base_content = create_blob(&odb, b"base").await;
        let theirs_content = create_blob(&odb, b"modified").await;

        let base = create_tree(vec![("file.txt", base_content)]);
        let ours = create_tree(vec![]); // We deleted
        let theirs = create_tree(vec![("file.txt", theirs_content)]);

        let conflicts = detector.detect_conflicts(&base, &ours, &theirs).await.unwrap();

        assert_eq!(conflicts.len(), 1);
        assert_eq!(conflicts[0].conflict_type, ConflictType::DeleteModify);
        assert_eq!(conflicts[0].path, "file.txt");
    }

    #[tokio::test]
    async fn test_conflict_modify_delete() {
        let odb = create_test_odb();
        let detector = ConflictDetector::new(Arc::clone(&odb));

        let base_content = create_blob(&odb, b"base").await;
        let ours_content = create_blob(&odb, b"modified").await;

        let base = create_tree(vec![("file.txt", base_content)]);
        let ours = create_tree(vec![("file.txt", ours_content)]);
        let theirs = create_tree(vec![]); // They deleted

        let conflicts = detector.detect_conflicts(&base, &ours, &theirs).await.unwrap();

        assert_eq!(conflicts.len(), 1);
        assert_eq!(conflicts[0].conflict_type, ConflictType::ModifyDelete);
        assert_eq!(conflicts[0].path, "file.txt");
    }

    #[tokio::test]
    async fn test_no_conflict_both_deleted() {
        let odb = create_test_odb();
        let detector = ConflictDetector::new(Arc::clone(&odb));

        let base_content = create_blob(&odb, b"base").await;

        let base = create_tree(vec![("file.txt", base_content)]);
        let ours = create_tree(vec![]);
        let theirs = create_tree(vec![]);

        let conflicts = detector.detect_conflicts(&base, &ours, &theirs).await.unwrap();

        assert_eq!(conflicts.len(), 0, "Both deleted should auto-merge");
    }

    #[tokio::test]
    async fn test_no_conflict_delete_unchanged() {
        let odb = create_test_odb();
        let detector = ConflictDetector::new(Arc::clone(&odb));

        let content = create_blob(&odb, b"unchanged").await;

        let base = create_tree(vec![("file.txt", content)]);
        let ours = create_tree(vec![]); // We deleted
        let theirs = create_tree(vec![("file.txt", content)]); // They kept unchanged

        let conflicts = detector.detect_conflicts(&base, &ours, &theirs).await.unwrap();

        assert_eq!(conflicts.len(), 0, "Delete of unchanged file should auto-merge");
    }

    #[tokio::test]
    async fn test_complex_multi_file_conflicts() {
        let odb = create_test_odb();
        let detector = ConflictDetector::new(Arc::clone(&odb));

        // Create various blobs
        let base1 = create_blob(&odb, b"base1").await;
        let base2 = create_blob(&odb, b"base2").await;
        let ours1 = create_blob(&odb, b"ours1").await;
        let theirs1 = create_blob(&odb, b"theirs1").await;
        let same_modified = create_blob(&odb, b"same_modified").await;
        let added_both = create_blob(&odb, b"added_both").await;
        let added_ours = create_blob(&odb, b"added_ours").await;

        // Build trees
        let base = create_tree(vec![
            ("conflict.txt", base1),
            ("same_change.txt", base2),
            ("delete_modify.txt", base1),
        ]);

        let ours = create_tree(vec![
            ("conflict.txt", ours1),         // Modified differently
            ("same_change.txt", same_modified), // Same modification
            // delete_modify.txt - we deleted it but they modified
            ("add_both.txt", added_both),    // Same addition
            ("add_ours.txt", added_ours),    // We added
        ]);

        let theirs = create_tree(vec![
            ("conflict.txt", theirs1),       // Modified differently
            ("same_change.txt", same_modified), // Same modification
            ("delete_modify.txt", theirs1),  // They modified
            ("add_both.txt", added_both),    // Same addition
        ]);

        let conflicts = detector.detect_conflicts(&base, &ours, &theirs).await.unwrap();

        // Should have 2 conflicts: conflict.txt (ModifyModify), delete_modify.txt (DeleteModify)
        assert_eq!(conflicts.len(), 2);

        let conflict_paths: Vec<_> = conflicts.iter().map(|c| c.path.as_str()).collect();
        assert!(conflict_paths.contains(&"conflict.txt"));
        assert!(conflict_paths.contains(&"delete_modify.txt"));
    }

    #[tokio::test]
    async fn test_can_auto_merge() {
        let odb = create_test_odb();
        let detector = ConflictDetector::new(Arc::clone(&odb));

        let base_content = create_blob(&odb, b"base").await;
        let modified_content = create_blob(&odb, b"modified").await;

        let base = create_tree(vec![("file.txt", base_content)]);
        let ours = create_tree(vec![("file.txt", modified_content)]);
        let theirs = create_tree(vec![("file.txt", base_content)]);

        let can_merge = detector
            .can_auto_merge("file.txt", &base, &ours, &theirs)
            .await
            .unwrap();

        assert!(can_merge, "One-sided change should be auto-mergeable");
    }

    #[tokio::test]
    async fn test_conflict_stats() {
        let odb = create_test_odb();
        let detector = ConflictDetector::new(Arc::clone(&odb));

        let conflicts = vec![
            Conflict {
                path: "file1.txt".to_string(),
                conflict_type: ConflictType::ModifyModify,
                base: None,
                ours: None,
                theirs: None,
            },
            Conflict {
                path: "file2.txt".to_string(),
                conflict_type: ConflictType::ModifyModify,
                base: None,
                ours: None,
                theirs: None,
            },
            Conflict {
                path: "file3.txt".to_string(),
                conflict_type: ConflictType::AddAdd,
                base: None,
                ours: None,
                theirs: None,
            },
            Conflict {
                path: "file4.txt".to_string(),
                conflict_type: ConflictType::DeleteModify,
                base: None,
                ours: None,
                theirs: None,
            },
        ];

        let stats = detector.conflict_stats(&conflicts);

        assert_eq!(stats.total, 4);
        assert_eq!(stats.modify_modify, 2);
        assert_eq!(stats.add_add, 1);
        assert_eq!(stats.delete_modify, 1);
        assert_eq!(stats.modify_delete, 0);
    }
}
