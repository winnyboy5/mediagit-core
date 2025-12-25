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

//! Lowest Common Ancestor (LCA) algorithm for merge base detection
//!
//! Implements merge base finding for 3-way merge operations.
//! Uses breadth-first search to identify common ancestors efficiently.
//!
//! # Algorithm
//!
//! 1. Build commit graphs from both commits
//! 2. Use BFS to traverse ancestors
//! 3. Identify all common ancestors
//! 4. Filter to find lowest (most recent) common ancestors
//! 5. Handle criss-cross merge scenarios
//!
//! # Examples
//!
//! ```no_run
//! use mediagit_versioning::{LcaFinder, ObjectDatabase, Oid};
//! use mediagit_storage::LocalBackend;
//! use std::sync::Arc;
//!
//! #[tokio::main]
//! async fn main() -> anyhow::Result<()> {
//!     let storage = Arc::new(LocalBackend::new("/tmp/mediagit")?);
//!     let odb = Arc::new(ObjectDatabase::new(storage, 100));
//!     let lca_finder = LcaFinder::new(odb);
//!
//!     let oid1 = Oid::hash(b"commit1");
//!     let oid2 = Oid::hash(b"commit2");
//!
//!     // Find merge base(s)
//!     let merge_bases = lca_finder.find_merge_base(&oid1, &oid2).await?;
//!     println!("Merge bases: {:?}", merge_bases);
//!
//!     Ok(())
//! }
//! ```

use crate::{Commit, ObjectDatabase, Oid};
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;
use tracing::debug;

/// Lowest Common Ancestor finder for merge operations
///
/// Provides merge base detection using BFS algorithm.
/// Handles complex scenarios including criss-cross merges.
pub struct LcaFinder {
    odb: Arc<ObjectDatabase>,
}

/// Result of LCA search with metadata
#[derive(Debug, Clone)]
pub struct LcaResult {
    /// Merge base commits (usually one, sometimes multiple)
    pub merge_bases: Vec<Oid>,

    /// Distance from oid1 to merge base
    pub distance_from_oid1: usize,

    /// Distance from oid2 to merge base
    pub distance_from_oid2: usize,

    /// Whether this is a criss-cross merge (multiple bases)
    pub is_criss_cross: bool,
}

impl LcaFinder {
    /// Create a new LCA finder
    ///
    /// # Arguments
    ///
    /// * `odb` - Object database for reading commits
    pub fn new(odb: Arc<ObjectDatabase>) -> Self {
        Self { odb }
    }

    /// Find merge base(s) between two commits
    ///
    /// Returns one or more merge bases. Multiple bases indicate a criss-cross merge.
    ///
    /// # Arguments
    ///
    /// * `oid1` - First commit OID
    /// * `oid2` - Second commit OID
    ///
    /// # Returns
    ///
    /// Vector of merge base OIDs (usually one, sometimes multiple)
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # use mediagit_versioning::{LcaFinder, ObjectDatabase, Oid};
    /// # use mediagit_storage::LocalBackend;
    /// # use std::sync::Arc;
    /// # #[tokio::main]
    /// # async fn main() -> anyhow::Result<()> {
    /// # let storage = Arc::new(LocalBackend::new("/tmp/test")?);
    /// # let odb = Arc::new(ObjectDatabase::new(storage, 100));
    /// let lca_finder = LcaFinder::new(odb);
    /// let base = lca_finder.find_merge_base(&oid1, &oid2).await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn find_merge_base(&self, oid1: &Oid, oid2: &Oid) -> anyhow::Result<Vec<Oid>> {
        debug!(oid1 = %oid1, oid2 = %oid2, "Finding merge base");

        // Same commit is trivial merge base
        if oid1 == oid2 {
            debug!("Same commit - trivial merge base");
            return Ok(vec![*oid1]);
        }

        // Check if one is ancestor of the other (fast-forward)
        if self.is_ancestor(oid1, oid2).await? {
            debug!(ancestor = %oid1, descendant = %oid2, "Fast-forward: oid1 is ancestor of oid2");
            return Ok(vec![*oid1]);
        }

        if self.is_ancestor(oid2, oid1).await? {
            debug!(ancestor = %oid2, descendant = %oid1, "Fast-forward: oid2 is ancestor of oid1");
            return Ok(vec![*oid2]);
        }

        // Find common ancestors using BFS
        let common_ancestors = self.find_common_ancestors(oid1, oid2).await?;

        if common_ancestors.is_empty() {
            anyhow::bail!("No common ancestor found between {} and {}", oid1, oid2);
        }

        // Filter to lowest common ancestors
        let lca = self.filter_to_lca(&common_ancestors).await?;

        debug!(
            merge_bases = ?lca,
            count = lca.len(),
            "Found merge base(s)"
        );

        Ok(lca)
    }

    /// Check if one commit is an ancestor of another
    ///
    /// # Arguments
    ///
    /// * `ancestor` - Potential ancestor commit
    /// * `descendant` - Potential descendant commit
    ///
    /// # Returns
    ///
    /// true if ancestor is in the history of descendant
    pub async fn is_ancestor(&self, ancestor: &Oid, descendant: &Oid) -> anyhow::Result<bool> {
        // Removed trace logging from hot path for performance

        if ancestor == descendant {
            return Ok(true);
        }

        let mut visited = HashSet::new();
        let mut queue = VecDeque::new();
        queue.push_back(*descendant);

        while let Some(current) = queue.pop_front() {
            if current == *ancestor {
                return Ok(true);
            }

            if !visited.insert(current) {
                continue;
            }

            // Load commit and traverse parents
            if let Ok(commit) = Commit::read(&self.odb, &current).await {
                for parent in &commit.parents {
                    if !visited.contains(parent) {
                        queue.push_back(*parent);
                    }
                }
            }
        }

        Ok(false)
    }

    /// Find all common ancestors between two commits using BFS
    async fn find_common_ancestors(&self, oid1: &Oid, oid2: &Oid) -> anyhow::Result<Vec<Oid>> {
        // BFS from oid1 to mark all ancestors
        let ancestors1 = self.get_all_ancestors(oid1).await?;

        // BFS from oid2 to find intersections
        let mut common = Vec::new();
        let mut visited = HashSet::new();
        let mut queue = VecDeque::new();
        queue.push_back(*oid2);

        while let Some(current) = queue.pop_front() {
            if !visited.insert(current) {
                continue;
            }

            // Check if this commit is in oid1's ancestry
            if ancestors1.contains(&current) {
                common.push(current);
            }

            // Continue traversing
            if let Ok(commit) = Commit::read(&self.odb, &current).await {
                for parent in &commit.parents {
                    if !visited.contains(parent) {
                        queue.push_back(*parent);
                    }
                }
            }
        }

        debug!(count = common.len(), "Found common ancestors");
        Ok(common)
    }

    /// Get all ancestors of a commit
    async fn get_all_ancestors(&self, oid: &Oid) -> anyhow::Result<HashSet<Oid>> {
        let mut ancestors = HashSet::new();
        let mut queue = VecDeque::new();
        queue.push_back(*oid);

        while let Some(current) = queue.pop_front() {
            if !ancestors.insert(current) {
                continue;
            }

            if let Ok(commit) = Commit::read(&self.odb, &current).await {
                for parent in &commit.parents {
                    if !ancestors.contains(parent) {
                        queue.push_back(*parent);
                    }
                }
            }
        }

        Ok(ancestors)
    }

    /// Filter common ancestors to find lowest (most recent) ones
    ///
    /// A commit is a lowest common ancestor if no other common ancestor
    /// is a descendant of it.
    async fn filter_to_lca(&self, common: &[Oid]) -> anyhow::Result<Vec<Oid>> {
        if common.len() <= 1 {
            return Ok(common.to_vec());
        }

        // For each common ancestor, check if it's an ancestor of any other
        let mut lca = Vec::new();

        for candidate in common {
            let mut is_lca = true;

            for other in common {
                if candidate == other {
                    continue;
                }

                // If candidate is ancestor of other, it's not LCA
                if self.is_ancestor(candidate, other).await? {
                    is_lca = false;
                    break;
                }
            }

            if is_lca {
                lca.push(*candidate);
            }
        }

        Ok(lca)
    }

    /// Find detailed merge base result with metadata
    pub async fn find_merge_base_detailed(
        &self,
        oid1: &Oid,
        oid2: &Oid,
    ) -> anyhow::Result<LcaResult> {
        let merge_bases = self.find_merge_base(oid1, oid2).await?;
        let is_criss_cross = merge_bases.len() > 1;

        // Calculate distances (simplified - just for metadata)
        let distance_from_oid1 = if merge_bases.is_empty() {
            0
        } else {
            self.distance_between(oid1, &merge_bases[0]).await?
        };

        let distance_from_oid2 = if merge_bases.is_empty() {
            0
        } else {
            self.distance_between(oid2, &merge_bases[0]).await?
        };

        Ok(LcaResult {
            merge_bases,
            distance_from_oid1,
            distance_from_oid2,
            is_criss_cross,
        })
    }

    /// Calculate distance between two commits (number of commits in path)
    ///
    /// Finds shortest path from ancestor to descendant or vice versa.
    async fn distance_between(&self, from: &Oid, to: &Oid) -> anyhow::Result<usize> {
        if from == to {
            return Ok(0);
        }

        // First try traversing parents from 'from' (assuming from is descendant)
        let mut distances: HashMap<Oid, usize> = HashMap::new();
        let mut queue = VecDeque::new();
        queue.push_back((*from, 0));
        distances.insert(*from, 0);

        while let Some((current, dist)) = queue.pop_front() {
            if current == *to {
                return Ok(dist);
            }

            if let Ok(commit) = Commit::read(&self.odb, &current).await {
                for parent in &commit.parents {
                    if !distances.contains_key(parent) {
                        distances.insert(*parent, dist + 1);
                        queue.push_back((*parent, dist + 1));
                    }
                }
            }
        }

        // Not found traversing parents, try reverse (from is ancestor, to is descendant)
        // In this case we need to traverse from 'to' towards 'from'
        let mut distances: HashMap<Oid, usize> = HashMap::new();
        let mut queue = VecDeque::new();
        queue.push_back((*to, 0));
        distances.insert(*to, 0);

        while let Some((current, dist)) = queue.pop_front() {
            if current == *from {
                return Ok(dist);
            }

            if let Ok(commit) = Commit::read(&self.odb, &current).await {
                for parent in &commit.parents {
                    if !distances.contains_key(parent) {
                        distances.insert(*parent, dist + 1);
                        queue.push_back((*parent, dist + 1));
                    }
                }
            }
        }

        // Not found in either direction
        Ok(usize::MAX)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Signature;
    use mediagit_storage::mock::MockBackend;

    async fn create_commit(
        odb: &Arc<ObjectDatabase>,
        message: &str,
        parents: Vec<Oid>,
    ) -> anyhow::Result<Oid> {
        let tree = crate::Tree::new();
        let tree_oid = tree.write(odb).await?;

        let sig = Signature::now("Test".to_string(), "test@example.com".to_string());
        let mut commit = Commit::new(tree_oid, sig.clone(), sig, message.to_string());

        for parent in parents {
            commit.add_parent(parent);
        }

        commit.write(odb).await
    }

    #[tokio::test]
    async fn test_lca_same_commit() {
        let storage = Arc::new(MockBackend::new());
        let odb = Arc::new(ObjectDatabase::new(storage, 100));
        let lca_finder = LcaFinder::new(odb.clone());

        let commit = create_commit(&odb, "initial", vec![]).await.unwrap();

        let bases = lca_finder.find_merge_base(&commit, &commit).await.unwrap();
        assert_eq!(bases.len(), 1);
        assert_eq!(bases[0], commit);
    }

    #[tokio::test]
    async fn test_lca_linear_history() {
        let storage = Arc::new(MockBackend::new());
        let odb = Arc::new(ObjectDatabase::new(storage, 100));
        let lca_finder = LcaFinder::new(odb.clone());

        // A -> B -> C
        let a = create_commit(&odb, "A", vec![]).await.unwrap();
        let b = create_commit(&odb, "B", vec![a]).await.unwrap();
        let c = create_commit(&odb, "C", vec![b]).await.unwrap();

        let bases = lca_finder.find_merge_base(&a, &c).await.unwrap();
        assert_eq!(bases.len(), 1);
        assert_eq!(bases[0], a);
    }

    #[tokio::test]
    async fn test_lca_diverged_branches() {
        let storage = Arc::new(MockBackend::new());
        let odb = Arc::new(ObjectDatabase::new(storage, 100));
        let lca_finder = LcaFinder::new(odb.clone());

        //     B   D
        //      \ /
        //       A
        //      / \
        //     C   E

        let a = create_commit(&odb, "A", vec![]).await.unwrap();
        let b = create_commit(&odb, "B", vec![a]).await.unwrap();
        let c = create_commit(&odb, "C", vec![a]).await.unwrap();

        let bases = lca_finder.find_merge_base(&b, &c).await.unwrap();
        assert_eq!(bases.len(), 1);
        assert_eq!(bases[0], a);
    }

    #[tokio::test]
    async fn test_is_ancestor_true() {
        let storage = Arc::new(MockBackend::new());
        let odb = Arc::new(ObjectDatabase::new(storage, 100));
        let lca_finder = LcaFinder::new(odb.clone());

        // A -> B -> C
        let a = create_commit(&odb, "A", vec![]).await.unwrap();
        let b = create_commit(&odb, "B", vec![a]).await.unwrap();
        let c = create_commit(&odb, "C", vec![b]).await.unwrap();

        assert!(lca_finder.is_ancestor(&a, &c).await.unwrap());
        assert!(lca_finder.is_ancestor(&b, &c).await.unwrap());
        assert!(lca_finder.is_ancestor(&a, &b).await.unwrap());
    }

    #[tokio::test]
    async fn test_is_ancestor_false() {
        let storage = Arc::new(MockBackend::new());
        let odb = Arc::new(ObjectDatabase::new(storage, 100));
        let lca_finder = LcaFinder::new(odb.clone());

        // A -> B
        // A -> C
        let a = create_commit(&odb, "A", vec![]).await.unwrap();
        let b = create_commit(&odb, "B", vec![a]).await.unwrap();
        let c = create_commit(&odb, "C", vec![a]).await.unwrap();

        assert!(!lca_finder.is_ancestor(&b, &c).await.unwrap());
        assert!(!lca_finder.is_ancestor(&c, &b).await.unwrap());
    }

    #[tokio::test]
    async fn test_lca_merge_commit() {
        let storage = Arc::new(MockBackend::new());
        let odb = Arc::new(ObjectDatabase::new(storage, 100));
        let lca_finder = LcaFinder::new(odb.clone());

        //     B   C
        //      \ /
        //       A
        //       |
        //       D (merge of B and C)

        let a = create_commit(&odb, "A", vec![]).await.unwrap();
        let b = create_commit(&odb, "B", vec![a]).await.unwrap();
        let c = create_commit(&odb, "C", vec![a]).await.unwrap();
        let _d = create_commit(&odb, "D (merge)", vec![b, c]).await.unwrap();

        let bases = lca_finder.find_merge_base(&b, &c).await.unwrap();
        assert_eq!(bases.len(), 1);
        assert_eq!(bases[0], a);
    }

    #[tokio::test]
    async fn test_lca_distance() {
        let storage = Arc::new(MockBackend::new());
        let odb = Arc::new(ObjectDatabase::new(storage, 100));
        let lca_finder = LcaFinder::new(odb.clone());

        // A -> B -> C
        let a = create_commit(&odb, "A", vec![]).await.unwrap();
        let b = create_commit(&odb, "B", vec![a]).await.unwrap();
        let c = create_commit(&odb, "C", vec![b]).await.unwrap();

        assert_eq!(lca_finder.distance_between(&a, &a).await.unwrap(), 0);
        assert_eq!(lca_finder.distance_between(&a, &b).await.unwrap(), 1);
        assert_eq!(lca_finder.distance_between(&a, &c).await.unwrap(), 2);
        assert_eq!(lca_finder.distance_between(&b, &c).await.unwrap(), 1);
    }

    #[tokio::test]
    async fn test_lca_detailed_result() {
        let storage = Arc::new(MockBackend::new());
        let odb = Arc::new(ObjectDatabase::new(storage, 100));
        let lca_finder = LcaFinder::new(odb.clone());

        //     B   C
        //      \ /
        //       A

        let a = create_commit(&odb, "A", vec![]).await.unwrap();
        let b = create_commit(&odb, "B", vec![a]).await.unwrap();
        let c = create_commit(&odb, "C", vec![a]).await.unwrap();

        let result = lca_finder.find_merge_base_detailed(&b, &c).await.unwrap();

        assert_eq!(result.merge_bases.len(), 1);
        assert_eq!(result.merge_bases[0], a);
        assert!(!result.is_criss_cross);
        assert_eq!(result.distance_from_oid1, 1);
        assert_eq!(result.distance_from_oid2, 1);
    }

    #[tokio::test]
    async fn test_lca_complex_graph() {
        let storage = Arc::new(MockBackend::new());
        let odb = Arc::new(ObjectDatabase::new(storage, 100));
        let lca_finder = LcaFinder::new(odb.clone());

        //       F   G
        //        \ /
        //     B   E
        //      \ / \
        //       A   C
        //            \
        //             D

        let a = create_commit(&odb, "A", vec![]).await.unwrap();
        let b = create_commit(&odb, "B", vec![a]).await.unwrap();
        let c = create_commit(&odb, "C", vec![a]).await.unwrap();
        let d = create_commit(&odb, "D", vec![c]).await.unwrap();
        let e = create_commit(&odb, "E", vec![b, c]).await.unwrap();
        let f = create_commit(&odb, "F", vec![e]).await.unwrap();
        let g = create_commit(&odb, "G", vec![e, d]).await.unwrap();

        let bases = lca_finder.find_merge_base(&f, &g).await.unwrap();
        assert_eq!(bases.len(), 1);
        assert_eq!(bases[0], e);
    }
}
