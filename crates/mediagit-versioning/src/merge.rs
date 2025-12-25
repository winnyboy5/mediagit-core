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

//! Merge engine for 3-way merge operations
//!
//! This module orchestrates LCA finding, tree diffing, and conflict detection
//! to perform complete merge operations with various strategies.

use crate::{
    Commit, Conflict, ConflictDetector, LcaFinder, ObjectDatabase, Oid, Tree, TreeDiffer,
};
use anyhow::{anyhow, Result};
use std::sync::Arc;
use tracing::{debug, instrument, trace};

/// Merge strategy to use when conflicts are detected
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MergeStrategy {
    /// Recursive 3-way merge (default)
    /// Automatically merges non-conflicting changes, reports conflicts
    Recursive,

    /// Always take our version on conflict
    Ours,

    /// Always take their version on conflict
    Theirs,
}

impl Default for MergeStrategy {
    fn default() -> Self {
        MergeStrategy::Recursive
    }
}

/// Fast-forward merge information
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FastForwardInfo {
    /// The commit being fast-forwarded from
    pub from: Oid,

    /// The commit being fast-forwarded to
    pub to: Oid,

    /// Whether this is a fast-forward merge
    pub is_fast_forward: bool,
}

/// Result of a merge operation
#[derive(Debug, Clone)]
pub struct MergeResult {
    /// Result tree OID (if successful or using conflict resolution strategy)
    pub tree_oid: Option<Oid>,

    /// Conflicts detected (empty if no conflicts or strategy resolved them)
    pub conflicts: Vec<Conflict>,

    /// Whether the merge was fully successful without conflicts
    pub success: bool,

    /// Fast-forward information (if applicable)
    pub fast_forward: Option<FastForwardInfo>,

    /// Merge strategy used
    pub strategy: MergeStrategy,
}

impl MergeResult {
    /// Check if the merge has unresolved conflicts
    pub fn has_conflicts(&self) -> bool {
        !self.conflicts.is_empty()
    }

    /// Check if this was a fast-forward merge
    pub fn is_fast_forward(&self) -> bool {
        self.fast_forward
            .as_ref()
            .map(|ff| ff.is_fast_forward)
            .unwrap_or(false)
    }
}

/// Merge engine orchestrating the complete merge process
pub struct MergeEngine {
    odb: Arc<ObjectDatabase>,
    lca_finder: LcaFinder,
    __differ: TreeDiffer,
    conflict_detector: ConflictDetector,
}

impl MergeEngine {
    /// Create a new merge engine
    pub fn new(odb: Arc<ObjectDatabase>) -> Self {
        Self {
            lca_finder: LcaFinder::new(Arc::clone(&odb)),
            __differ: TreeDiffer::new(Arc::clone(&odb)),
            conflict_detector: ConflictDetector::new(Arc::clone(&odb)),
            odb,
        }
    }

    /// Perform a merge operation between two commits
    ///
    /// This is the main entry point for merge operations. It:
    /// 1. Finds the merge base (LCA)
    /// 2. Checks for fast-forward possibility
    /// 3. Performs 3-way merge if needed
    /// 4. Detects and handles conflicts based on strategy
    /// 5. Builds the result tree
    #[instrument(level = "debug", skip(self, ours, theirs))]
    pub async fn merge(
        &self,
        ours: &Oid,
        theirs: &Oid,
        strategy: MergeStrategy,
    ) -> Result<MergeResult> {
        debug!("Starting merge: ours={}, theirs={}", ours, theirs);

        // Trivial case: same commit
        if ours == theirs {
            debug!("Trivial merge: commits are identical");
            let commit = Commit::read(&self.odb, ours).await?;
            return Ok(MergeResult {
                tree_oid: Some(commit.tree),
                conflicts: Vec::new(),
                success: true,
                fast_forward: None,
                strategy,
            });
        }

        // Check for fast-forward
        if let Some(ff_result) = self.check_fast_forward(ours, theirs, strategy).await? {
            return Ok(ff_result);
        }

        // Find merge base (LCA)
        let merge_bases = self.lca_finder.find_merge_base(ours, theirs).await?;
        if merge_bases.is_empty() {
            return Err(anyhow!("No common ancestor found between commits"));
        }

        // Use first merge base (handle multiple bases by using first one)
        let base_oid = &merge_bases[0];
        debug!("Merge base: {}", base_oid);

        // Load trees
        let base_commit = Commit::read(&self.odb, base_oid).await?;
        let ours_commit = Commit::read(&self.odb, ours).await?;
        let theirs_commit = Commit::read(&self.odb, theirs).await?;

        let base_tree = Tree::read(&self.odb, &base_commit.tree).await?;
        let ours_tree = Tree::read(&self.odb, &ours_commit.tree).await?;
        let theirs_tree = Tree::read(&self.odb, &theirs_commit.tree).await?;

        // Perform 3-way merge
        self.three_way_merge(&base_tree, &ours_tree, &theirs_tree, strategy)
            .await
    }

    /// Check if a fast-forward merge is possible
    async fn check_fast_forward(
        &self,
        ours: &Oid,
        theirs: &Oid,
        strategy: MergeStrategy,
    ) -> Result<Option<MergeResult>> {
        // Check if theirs is ancestor of ours (already up to date)
        if self.lca_finder.is_ancestor(theirs, ours).await? {
            debug!("Already up to date: theirs is ancestor of ours");
            let commit = Commit::read(&self.odb, ours).await?;
            return Ok(Some(MergeResult {
                tree_oid: Some(commit.tree),
                conflicts: Vec::new(),
                success: true,
                fast_forward: Some(FastForwardInfo {
                    from: *ours,
                    to: *ours,
                    is_fast_forward: false, // No actual fast-forward needed
                }),
                strategy,
            }));
        }

        // Check if ours is ancestor of theirs (can fast-forward)
        if self.lca_finder.is_ancestor(ours, theirs).await? {
            debug!("Fast-forward possible: ours is ancestor of theirs");
            let commit = Commit::read(&self.odb, theirs).await?;
            return Ok(Some(MergeResult {
                tree_oid: Some(commit.tree),
                conflicts: Vec::new(),
                success: true,
                fast_forward: Some(FastForwardInfo {
                    from: *ours,
                    to: *theirs,
                    is_fast_forward: true,
                }),
                strategy,
            }));
        }

        Ok(None)
    }

    /// Perform 3-way merge between base, ours, and theirs trees
    #[instrument(level = "debug", skip(self, base, ours, theirs))]
    async fn three_way_merge(
        &self,
        base: &Tree,
        ours: &Tree,
        theirs: &Tree,
        strategy: MergeStrategy,
    ) -> Result<MergeResult> {
        debug!("Performing 3-way merge with strategy: {:?}", strategy);

        // Detect conflicts
        let conflicts = self
            .conflict_detector
            .detect_conflicts(base, ours, theirs)
            .await?;

        debug!("Detected {} conflicts", conflicts.len());

        // Build merged tree based on strategy
        let (tree_oid, final_conflicts, success) = match strategy {
            MergeStrategy::Recursive => {
                if conflicts.is_empty() {
                    // No conflicts - build clean merged tree
                    let tree = self.build_merged_tree(base, ours, theirs, &[]).await?;
                    let tree_oid = tree.write(&self.odb).await?;
                    (Some(tree_oid), Vec::new(), true)
                } else {
                    // Has conflicts - report them without building tree
                    (None, conflicts, false)
                }
            }
            MergeStrategy::Ours => {
                // Always use our version, resolve conflicts in our favor
                let tree = self.build_merged_tree_ours(base, ours, theirs).await?;
                let tree_oid = tree.write(&self.odb).await?;
                (Some(tree_oid), Vec::new(), true)
            }
            MergeStrategy::Theirs => {
                // Always use their version, resolve conflicts in their favor
                let tree = self
                    .build_merged_tree_theirs(base, ours, theirs)
                    .await?;
                let tree_oid = tree.write(&self.odb).await?;
                (Some(tree_oid), Vec::new(), true)
            }
        };

        Ok(MergeResult {
            tree_oid,
            conflicts: final_conflicts,
            success,
            fast_forward: None,
            strategy,
        })
    }

    /// Build merged tree for clean merge (no conflicts)
    async fn build_merged_tree(
        &self,
        base: &Tree,
        ours: &Tree,
        theirs: &Tree,
        _conflicts: &[Conflict],
    ) -> Result<Tree> {
        let mut merged = Tree::new();

        // Get all unique paths across all trees
        let mut all_paths = std::collections::HashSet::new();
        all_paths.extend(base.entries.keys().map(|s| s.as_str()));
        all_paths.extend(ours.entries.keys().map(|s| s.as_str()));
        all_paths.extend(theirs.entries.keys().map(|s| s.as_str()));

        for path in all_paths {
            let base_entry = base.entries.get(path);
            let ours_entry = ours.entries.get(path);
            let theirs_entry = theirs.entries.get(path);

            trace!("Merging path: {}", path);

            // Determine which version to use
            let entry = match (base_entry, ours_entry, theirs_entry) {
                // All three present
                (Some(base), Some(ours), Some(theirs)) => {
                    if ours.oid == theirs.oid {
                        // Same content on both sides
                        Some(ours.clone())
                    } else if base.oid == ours.oid {
                        // We didn't change, they did
                        Some(theirs.clone())
                    } else if base.oid == theirs.oid {
                        // They didn't change, we did
                        Some(ours.clone())
                    } else {
                        // Both changed differently - this should be a conflict
                        // For clean merge, this shouldn't happen
                        return Err(anyhow!(
                            "Unexpected conflict in clean merge for path: {}",
                            path
                        ));
                    }
                }

                // File deleted on one or both sides
                (Some(base), Some(ours), None) => {
                    if base.oid == ours.oid {
                        // We didn't change it, they deleted it - accept deletion
                        None
                    } else {
                        // We changed it, they deleted it - this is a conflict
                        return Err(anyhow!(
                            "Unexpected conflict (modify/delete) in clean merge for path: {}",
                            path
                        ));
                    }
                }
                (Some(base), None, Some(theirs)) => {
                    if base.oid == theirs.oid {
                        // They didn't change it, we deleted it - accept deletion
                        None
                    } else {
                        // They changed it, we deleted it - this is a conflict
                        return Err(anyhow!(
                            "Unexpected conflict (delete/modify) in clean merge for path: {}",
                            path
                        ));
                    }
                }
                (Some(_), None, None) => {
                    // Both deleted - accept deletion
                    None
                }

                // File added on one or both sides
                (None, Some(ours), Some(theirs)) => {
                    if ours.oid == theirs.oid {
                        // Same addition on both sides
                        Some(ours.clone())
                    } else {
                        // Different additions - this is a conflict
                        return Err(anyhow!(
                            "Unexpected conflict (add/add) in clean merge for path: {}",
                            path
                        ));
                    }
                }
                (None, Some(ours), None) => {
                    // We added it
                    Some(ours.clone())
                }
                (None, None, Some(theirs)) => {
                    // They added it
                    Some(theirs.clone())
                }

                // Impossible case
                (None, None, None) => None,
            };

            if let Some(entry) = entry {
                merged.add_entry(entry);
            }
        }

        Ok(merged)
    }

    /// Build merged tree using "ours" strategy
    async fn build_merged_tree_ours(&self, base: &Tree, ours: &Tree, theirs: &Tree) -> Result<Tree> {
        let mut merged = Tree::new();

        // Get all unique paths
        let mut all_paths = std::collections::HashSet::new();
        all_paths.extend(base.entries.keys().map(|s| s.as_str()));
        all_paths.extend(ours.entries.keys().map(|s| s.as_str()));
        all_paths.extend(theirs.entries.keys().map(|s| s.as_str()));

        for path in all_paths {
            let base_entry = base.entries.get(path);
            let ours_entry = ours.entries.get(path);
            let theirs_entry = theirs.entries.get(path);

            // "Ours" strategy: prefer our version in conflicts
            let entry = match (base_entry, ours_entry, theirs_entry) {
                // If we have it, use ours
                (_, Some(ours), _) => Some(ours.clone()),

                // If we don't have it but they do, check if we deleted it
                (Some(_), None, Some(_theirs)) => {
                    // We deleted it - honor our deletion
                    None
                }
                (None, None, Some(theirs)) => {
                    // They added it and we didn't - take theirs
                    Some(theirs.clone())
                }

                // We don't have it and they don't either
                _ => None,
            };

            if let Some(entry) = entry {
                merged.add_entry(entry);
            }
        }

        Ok(merged)
    }

    /// Build merged tree using "theirs" strategy
    async fn build_merged_tree_theirs(
        &self,
        base: &Tree,
        ours: &Tree,
        theirs: &Tree,
    ) -> Result<Tree> {
        let mut merged = Tree::new();

        // Get all unique paths
        let mut all_paths = std::collections::HashSet::new();
        all_paths.extend(base.entries.keys().map(|s| s.as_str()));
        all_paths.extend(ours.entries.keys().map(|s| s.as_str()));
        all_paths.extend(theirs.entries.keys().map(|s| s.as_str()));

        for path in all_paths {
            let base_entry = base.entries.get(path);
            let ours_entry = ours.entries.get(path);
            let theirs_entry = theirs.entries.get(path);

            // "Theirs" strategy: prefer their version in conflicts
            let entry = match (base_entry, ours_entry, theirs_entry) {
                // If they have it, use theirs
                (_, _, Some(theirs)) => Some(theirs.clone()),

                // If they don't have it but we do, check if they deleted it
                (Some(_), Some(_ours), None) => {
                    // They deleted it - honor their deletion
                    None
                }
                (None, Some(ours), None) => {
                    // We added it and they didn't - take ours
                    Some(ours.clone())
                }

                // They don't have it and we don't either
                _ => None,
            };

            if let Some(entry) = entry {
                merged.add_entry(entry);
            }
        }

        Ok(merged)
    }

    /// Check if fast-forward merge is possible from 'from' to 'to'
    pub async fn can_fast_forward(&self, from: &Oid, to: &Oid) -> Result<bool> {
        if from == to {
            return Ok(true);
        }
        self.lca_finder.is_ancestor(from, to).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Commit, FileMode, Signature, Tree, TreeEntry};
    use chrono::Utc;
    use mediagit_storage::mock::MockBackend;

    fn create_test_odb() -> Arc<ObjectDatabase> {
        let storage = Arc::new(MockBackend::new());
        Arc::new(ObjectDatabase::new(storage, 100))
    }

    fn create_signature() -> Signature {
        Signature {
            name: "Test User".to_string(),
            email: "test@example.com".to_string(),
            timestamp: Utc::now(),
        }
    }

    async fn create_tree(odb: &Arc<ObjectDatabase>, entries: Vec<(&str, &[u8])>) -> Oid {
        let mut tree = Tree::new();
        for (name, content) in entries {
            let oid = Oid::hash(content);
            let entry = TreeEntry::new(name.to_string(), FileMode::Regular, oid);
            tree.add_entry(entry);
        }
        tree.write(odb).await.unwrap()
    }

    async fn create_commit(
        odb: &Arc<ObjectDatabase>,
        tree: Oid,
        parents: Vec<Oid>,
        message: &str,
    ) -> Oid {
        let commit = Commit {
            tree,
            parents,
            author: create_signature(),
            committer: create_signature(),
            message: message.to_string(),
        };
        commit.write(odb).await.unwrap()
    }

    #[tokio::test]
    async fn test_merge_same_commit() {
        let odb = create_test_odb();
        let engine = MergeEngine::new(Arc::clone(&odb));

        let tree = create_tree(&odb, vec![("file.txt", b"content")]).await;
        let commit = create_commit(&odb, tree, vec![], "Initial").await;

        let result = engine
            .merge(&commit, &commit, MergeStrategy::Recursive)
            .await
            .unwrap();

        assert!(result.success);
        assert_eq!(result.tree_oid, Some(tree));
        assert_eq!(result.conflicts.len(), 0);
    }

    #[tokio::test]
    async fn test_fast_forward_merge() {
        let odb = create_test_odb();
        let engine = MergeEngine::new(Arc::clone(&odb));

        // Create base commit
        let tree1 = create_tree(&odb, vec![("file.txt", b"v1")]).await;
        let commit1 = create_commit(&odb, tree1, vec![], "Base").await;

        // Create descendant commit
        let tree2 = create_tree(&odb, vec![("file.txt", b"v2")]).await;
        let commit2 = create_commit(&odb, tree2, vec![commit1], "Update").await;

        let result = engine
            .merge(&commit1, &commit2, MergeStrategy::Recursive)
            .await
            .unwrap();

        assert!(result.success);
        assert!(result.is_fast_forward());
        assert_eq!(result.tree_oid, Some(tree2));
        assert_eq!(result.conflicts.len(), 0);
    }

    #[tokio::test]
    async fn test_clean_merge_no_conflicts() {
        let odb = create_test_odb();
        let engine = MergeEngine::new(Arc::clone(&odb));

        // Base commit
        let base_tree = create_tree(&odb, vec![("file1.txt", b"base1"), ("file2.txt", b"base2")])
            .await;
        let base_commit = create_commit(&odb, base_tree, vec![], "Base").await;

        // Ours: modify file1
        let ours_tree = create_tree(&odb, vec![("file1.txt", b"ours1"), ("file2.txt", b"base2")])
            .await;
        let ours_commit = create_commit(&odb, ours_tree, vec![base_commit], "Ours").await;

        // Theirs: modify file2
        let theirs_tree =
            create_tree(&odb, vec![("file1.txt", b"base1"), ("file2.txt", b"theirs2")]).await;
        let theirs_commit = create_commit(&odb, theirs_tree, vec![base_commit], "Theirs").await;

        let result = engine
            .merge(&ours_commit, &theirs_commit, MergeStrategy::Recursive)
            .await
            .unwrap();

        assert!(result.success);
        assert!(result.tree_oid.is_some());
        assert_eq!(result.conflicts.len(), 0);

        // Verify merged tree has both changes
        let merged_tree = Tree::read(&odb, &result.tree_oid.unwrap()).await.unwrap();
        assert_eq!(merged_tree.entries.len(), 2);
        assert_eq!(merged_tree.entries.get("file1.txt").unwrap().oid, Oid::hash(b"ours1"));
        assert_eq!(
            merged_tree.entries.get("file2.txt").unwrap().oid,
            Oid::hash(b"theirs2")
        );
    }

    #[tokio::test]
    async fn test_merge_with_conflict() {
        let odb = create_test_odb();
        let engine = MergeEngine::new(Arc::clone(&odb));

        // Base commit
        let base_tree = create_tree(&odb, vec![("file.txt", b"base")]).await;
        let base_commit = create_commit(&odb, base_tree, vec![], "Base").await;

        // Ours: modify file
        let ours_tree = create_tree(&odb, vec![("file.txt", b"ours")]).await;
        let ours_commit = create_commit(&odb, ours_tree, vec![base_commit], "Ours").await;

        // Theirs: modify file differently
        let theirs_tree = create_tree(&odb, vec![("file.txt", b"theirs")]).await;
        let theirs_commit = create_commit(&odb, theirs_tree, vec![base_commit], "Theirs").await;

        let result = engine
            .merge(&ours_commit, &theirs_commit, MergeStrategy::Recursive)
            .await
            .unwrap();

        assert!(!result.success);
        assert!(result.tree_oid.is_none());
        assert_eq!(result.conflicts.len(), 1);
        assert_eq!(result.conflicts[0].path, "file.txt");
    }

    #[tokio::test]
    async fn test_merge_strategy_ours() {
        let odb = create_test_odb();
        let engine = MergeEngine::new(Arc::clone(&odb));

        // Base commit
        let base_tree = create_tree(&odb, vec![("file.txt", b"base")]).await;
        let base_commit = create_commit(&odb, base_tree, vec![], "Base").await;

        // Ours: modify file
        let ours_tree = create_tree(&odb, vec![("file.txt", b"ours")]).await;
        let ours_commit = create_commit(&odb, ours_tree, vec![base_commit], "Ours").await;

        // Theirs: modify file differently
        let theirs_tree = create_tree(&odb, vec![("file.txt", b"theirs")]).await;
        let theirs_commit = create_commit(&odb, theirs_tree, vec![base_commit], "Theirs").await;

        let result = engine
            .merge(&ours_commit, &theirs_commit, MergeStrategy::Ours)
            .await
            .unwrap();

        assert!(result.success);
        assert!(result.tree_oid.is_some());
        assert_eq!(result.conflicts.len(), 0);

        // Verify we got our version
        let merged_tree = Tree::read(&odb, &result.tree_oid.unwrap()).await.unwrap();
        assert_eq!(merged_tree.entries.get("file.txt").unwrap().oid, Oid::hash(b"ours"));
    }

    #[tokio::test]
    async fn test_merge_strategy_theirs() {
        let odb = create_test_odb();
        let engine = MergeEngine::new(Arc::clone(&odb));

        // Base commit
        let base_tree = create_tree(&odb, vec![("file.txt", b"base")]).await;
        let base_commit = create_commit(&odb, base_tree, vec![], "Base").await;

        // Ours: modify file
        let ours_tree = create_tree(&odb, vec![("file.txt", b"ours")]).await;
        let ours_commit = create_commit(&odb, ours_tree, vec![base_commit], "Ours").await;

        // Theirs: modify file differently
        let theirs_tree = create_tree(&odb, vec![("file.txt", b"theirs")]).await;
        let theirs_commit = create_commit(&odb, theirs_tree, vec![base_commit], "Theirs").await;

        let result = engine
            .merge(&ours_commit, &theirs_commit, MergeStrategy::Theirs)
            .await
            .unwrap();

        assert!(result.success);
        assert!(result.tree_oid.is_some());
        assert_eq!(result.conflicts.len(), 0);

        // Verify we got their version
        let merged_tree = Tree::read(&odb, &result.tree_oid.unwrap()).await.unwrap();
        assert_eq!(
            merged_tree.entries.get("file.txt").unwrap().oid,
            Oid::hash(b"theirs")
        );
    }

    #[tokio::test]
    async fn test_merge_with_additions() {
        let odb = create_test_odb();
        let engine = MergeEngine::new(Arc::clone(&odb));

        // Base commit
        let base_tree = create_tree(&odb, vec![("base.txt", b"base")]).await;
        let base_commit = create_commit(&odb, base_tree, vec![], "Base").await;

        // Ours: add file1
        let ours_tree = create_tree(&odb, vec![("base.txt", b"base"), ("ours.txt", b"ours")]).await;
        let ours_commit = create_commit(&odb, ours_tree, vec![base_commit], "Ours").await;

        // Theirs: add file2
        let theirs_tree =
            create_tree(&odb, vec![("base.txt", b"base"), ("theirs.txt", b"theirs")]).await;
        let theirs_commit = create_commit(&odb, theirs_tree, vec![base_commit], "Theirs").await;

        let result = engine
            .merge(&ours_commit, &theirs_commit, MergeStrategy::Recursive)
            .await
            .unwrap();

        assert!(result.success);
        assert!(result.tree_oid.is_some());
        assert_eq!(result.conflicts.len(), 0);

        // Verify merged tree has all files
        let merged_tree = Tree::read(&odb, &result.tree_oid.unwrap()).await.unwrap();
        assert_eq!(merged_tree.entries.len(), 3);
        assert!(merged_tree.entries.contains_key("base.txt"));
        assert!(merged_tree.entries.contains_key("ours.txt"));
        assert!(merged_tree.entries.contains_key("theirs.txt"));
    }

    #[tokio::test]
    async fn test_merge_with_deletions() {
        let odb = create_test_odb();
        let engine = MergeEngine::new(Arc::clone(&odb));

        // Base commit with two files
        let base_tree = create_tree(&odb, vec![("file1.txt", b"content1"), ("file2.txt", b"content2")])
            .await;
        let base_commit = create_commit(&odb, base_tree, vec![], "Base").await;

        // Ours: delete file1
        let ours_tree = create_tree(&odb, vec![("file2.txt", b"content2")]).await;
        let ours_commit = create_commit(&odb, ours_tree, vec![base_commit], "Ours").await;

        // Theirs: delete file2
        let theirs_tree = create_tree(&odb, vec![("file1.txt", b"content1")]).await;
        let theirs_commit = create_commit(&odb, theirs_tree, vec![base_commit], "Theirs").await;

        let result = engine
            .merge(&ours_commit, &theirs_commit, MergeStrategy::Recursive)
            .await
            .unwrap();

        assert!(result.success);
        assert!(result.tree_oid.is_some());
        assert_eq!(result.conflicts.len(), 0);

        // Verify merged tree has no files (both deleted)
        let merged_tree = Tree::read(&odb, &result.tree_oid.unwrap()).await.unwrap();
        assert_eq!(merged_tree.entries.len(), 0);
    }

    #[tokio::test]
    async fn test_merge_same_addition() {
        let odb = create_test_odb();
        let engine = MergeEngine::new(Arc::clone(&odb));

        // Base commit
        let base_tree = create_tree(&odb, vec![("base.txt", b"base")]).await;
        let base_commit = create_commit(&odb, base_tree, vec![], "Base").await;

        // Both add same file with same content
        let same_tree = create_tree(&odb, vec![("base.txt", b"base"), ("new.txt", b"same")]).await;
        let ours_commit = create_commit(&odb, same_tree, vec![base_commit], "Ours").await;
        let theirs_commit = create_commit(&odb, same_tree, vec![base_commit], "Theirs").await;

        let result = engine
            .merge(&ours_commit, &theirs_commit, MergeStrategy::Recursive)
            .await
            .unwrap();

        assert!(result.success);
        assert!(result.tree_oid.is_some());
        assert_eq!(result.conflicts.len(), 0);
    }

    #[tokio::test]
    async fn test_can_fast_forward() {
        let odb = create_test_odb();
        let engine = MergeEngine::new(Arc::clone(&odb));

        let tree1 = create_tree(&odb, vec![("file.txt", b"v1")]).await;
        let commit1 = create_commit(&odb, tree1, vec![], "Base").await;

        let tree2 = create_tree(&odb, vec![("file.txt", b"v2")]).await;
        let commit2 = create_commit(&odb, tree2, vec![commit1], "Update").await;

        // Can fast-forward from commit1 to commit2
        assert!(engine.can_fast_forward(&commit1, &commit2).await.unwrap());

        // Cannot fast-forward from commit2 to commit1
        assert!(!engine.can_fast_forward(&commit2, &commit1).await.unwrap());

        // Can "fast-forward" from commit to itself
        assert!(engine.can_fast_forward(&commit1, &commit1).await.unwrap());
    }

    #[tokio::test]
    async fn test_complex_merge_scenario() {
        let odb = create_test_odb();
        let engine = MergeEngine::new(Arc::clone(&odb));

        // Base: 3 files
        let base_tree = create_tree(
            &odb,
            vec![
                ("file1.txt", b"base1"),
                ("file2.txt", b"base2"),
                ("file3.txt", b"base3"),
            ],
        )
        .await;
        let base_commit = create_commit(&odb, base_tree, vec![], "Base").await;

        // Ours: modify file1, delete file2, add file4
        let ours_tree = create_tree(
            &odb,
            vec![
                ("file1.txt", b"ours1"),
                ("file3.txt", b"base3"),
                ("file4.txt", b"ours4"),
            ],
        )
        .await;
        let ours_commit = create_commit(&odb, ours_tree, vec![base_commit], "Ours").await;

        // Theirs: modify file3, delete file1, add file5
        let theirs_tree = create_tree(
            &odb,
            vec![
                ("file2.txt", b"base2"),
                ("file3.txt", b"theirs3"),
                ("file5.txt", b"theirs5"),
            ],
        )
        .await;
        let theirs_commit = create_commit(&odb, theirs_tree, vec![base_commit], "Theirs").await;

        let result = engine
            .merge(&ours_commit, &theirs_commit, MergeStrategy::Theirs)
            .await
            .unwrap();

        assert!(result.success);
        assert!(result.tree_oid.is_some());

        let merged_tree = Tree::read(&odb, &result.tree_oid.unwrap()).await.unwrap();

        // With Theirs strategy:
        // file1: we modified, they deleted - they win (deleted)
        assert!(!merged_tree.entries.contains_key("file1.txt"));

        // file2: we deleted, they kept - they win (kept)
        assert!(merged_tree.entries.contains_key("file2.txt"));
        assert_eq!(
            merged_tree.entries.get("file2.txt").unwrap().oid,
            Oid::hash(b"base2")
        );

        // file3: they modified, we kept - they win (modified)
        assert_eq!(
            merged_tree.entries.get("file3.txt").unwrap().oid,
            Oid::hash(b"theirs3")
        );

        // file4: we added, they didn't - included
        assert!(merged_tree.entries.contains_key("file4.txt"));

        // file5: they added, we didn't - included
        assert!(merged_tree.entries.contains_key("file5.txt"));
    }
}
