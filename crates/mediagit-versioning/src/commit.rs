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

//! Commit object representing snapshots in version control
//!
//! A Commit object captures a moment in time with metadata about changes,
//! references to the tree snapshot, and parent commits for history tracking.

use crate::{ObjectType, Oid, ObjectDatabase};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fmt;
use std::sync::Arc;

/// Author or committer information
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Signature {
    /// Name of the author or committer
    pub name: String,

    /// Email address
    pub email: String,

    /// Timestamp of the signature
    pub timestamp: DateTime<Utc>,
}

impl Signature {
    /// Create a new signature
    ///
    /// # Arguments
    ///
    /// * `name` - Author or committer name
    /// * `email` - Email address
    /// * `timestamp` - When the action occurred (defaults to now if not specified)
    ///
    /// # Examples
    ///
    /// ```
    /// use mediagit_versioning::Signature;
    /// use chrono::Utc;
    ///
    /// let sig = Signature::new(
    ///     "Alice Developer".to_string(),
    ///     "alice@example.com".to_string(),
    ///     Utc::now()
    /// );
    /// assert_eq!(sig.name, "Alice Developer");
    /// ```
    pub fn new(name: String, email: String, timestamp: DateTime<Utc>) -> Self {
        Self {
            name,
            email,
            timestamp,
        }
    }

    /// Create a signature with current timestamp
    pub fn now(name: String, email: String) -> Self {
        Self {
            name,
            email,
            timestamp: Utc::now(),
        }
    }
}

impl fmt::Display for Signature {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} <{}> {}",
            self.name,
            self.email,
            self.timestamp.timestamp()
        )
    }
}

/// Commit object representing a snapshot in version control history
///
/// A commit captures:
/// - A snapshot of the repository (tree OID)
/// - Parent commits (for history)
/// - Metadata about the change (author, committer, message, timestamp)
///
/// # Examples
///
/// ```no_run
/// use mediagit_versioning::{Commit, Signature, Oid, ObjectDatabase, ObjectType, Tree};
/// use mediagit_storage::LocalBackend;
/// use chrono::Utc;
/// use std::sync::Arc;
///
/// #[tokio::main]
/// async fn main() -> anyhow::Result<()> {
///     let storage = Arc::new(LocalBackend::new("/tmp/odb")?);
///     let odb = ObjectDatabase::new(storage, 100);
///
///     // Create a tree and write it
///     let tree = Tree::new();
///     let tree_oid = tree.write(&odb).await?;
///
///     // Create a commit
///     let author = Signature::now(
///         "Alice".to_string(),
///         "alice@example.com".to_string()
///     );
///     let mut commit = Commit::new(
///         tree_oid,
///         author.clone(),
///         author,
///         "Initial commit".to_string()
///     );
///
///     // Write commit
///     let commit_oid = commit.write(&odb).await?;
///     println!("Commit OID: {}", commit_oid);
///
///     // Load commit
///     let loaded = Commit::read(&odb, &commit_oid).await?;
///     assert_eq!(loaded.message, "Initial commit");
///
///     Ok(())
/// }
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Commit {
    /// OID of the tree this commit points to
    pub tree: Oid,

    /// OIDs of parent commits
    pub parents: Vec<Oid>,

    /// Author information
    pub author: Signature,

    /// Committer information
    pub committer: Signature,

    /// Commit message
    pub message: String,
}

impl Commit {
    /// Create a new commit
    ///
    /// # Arguments
    ///
    /// * `tree` - OID of the tree snapshot
    /// * `author` - Author signature
    /// * `committer` - Committer signature
    /// * `message` - Commit message
    ///
    /// # Examples
    ///
    /// ```
    /// use mediagit_versioning::{Commit, Signature, Oid};
    /// use chrono::Utc;
    ///
    /// let tree = Oid::hash(b"tree");
    /// let sig = Signature::now("Alice".to_string(), "alice@example.com".to_string());
    /// let commit = Commit::new(
    ///     tree,
    ///     sig.clone(),
    ///     sig,
    ///     "Initial commit".to_string()
    /// );
    /// assert_eq!(commit.message, "Initial commit");
    /// assert_eq!(commit.parents.len(), 0);
    /// ```
    pub fn new(
        tree: Oid,
        author: Signature,
        committer: Signature,
        message: String,
    ) -> Self {
        Self {
            tree,
            parents: Vec::new(),
            author,
            committer,
            message,
        }
    }

    /// Create a commit with parents
    pub fn with_parents(
        tree: Oid,
        parents: Vec<Oid>,
        author: Signature,
        committer: Signature,
        message: String,
    ) -> Self {
        Self {
            tree,
            parents,
            author,
            committer,
            message,
        }
    }

    /// Add a parent commit
    ///
    /// Used when building a merge commit or continuing from a previous commit.
    pub fn add_parent(&mut self, parent_oid: Oid) {
        self.parents.push(parent_oid);
    }

    /// Check if this is an initial commit (no parents)
    pub fn is_initial(&self) -> bool {
        self.parents.is_empty()
    }

    /// Check if this is a merge commit (multiple parents)
    pub fn is_merge(&self) -> bool {
        self.parents.len() > 1
    }

    /// Get the first parent (primary parent in merge commits)
    pub fn first_parent(&self) -> Option<&Oid> {
        self.parents.first()
    }

    /// Get parent count
    pub fn parent_count(&self) -> usize {
        self.parents.len()
    }

    /// Get primary parent OID (first parent or None)
    pub fn parent(&self) -> Option<&Oid> {
        self.parents.first()
    }

    /// Serialize commit to bytes
    ///
    /// Uses bincode for efficient serialization.
    pub fn serialize(&self) -> anyhow::Result<Vec<u8>> {
        bincode::serialize(self)
            .map_err(|e| anyhow::anyhow!("Commit serialization failed: {}", e))
    }

    /// Deserialize commit from bytes
    pub fn deserialize(data: &[u8]) -> anyhow::Result<Self> {
        bincode::deserialize(data)
            .map_err(|e| anyhow::anyhow!("Commit deserialization failed: {}", e))
    }

    /// Write commit to object database and return its OID
    ///
    /// # Arguments
    ///
    /// * `odb` - Object database instance
    ///
    /// # Returns
    ///
    /// The OID of the written commit
    pub async fn write(
        &self,
        odb: &crate::ObjectDatabase,
    ) -> anyhow::Result<Oid> {
        let data = self.serialize()?;
        odb.write(ObjectType::Commit, &data).await
    }

    /// Read commit from object database by OID
    ///
    /// # Arguments
    ///
    /// * `odb` - Object database instance
    /// * `oid` - Object ID of the commit
    ///
    /// # Returns
    ///
    /// The deserialized commit object
    pub async fn read(
        odb: &crate::ObjectDatabase,
        oid: &Oid,
    ) -> anyhow::Result<Self> {
        let data = odb.read(oid).await?;
        Self::deserialize(&data)
    }

    /// Get a summary of the commit (first line of message)
    pub fn summary(&self) -> &str {
        self.message.lines().next().unwrap_or("")
    }

    /// Get the full message with empty line handling
    pub fn full_message(&self) -> &str {
        &self.message
    }
}

impl fmt::Display for Commit {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.summary())
    }
}

/// Result of a shallow walk operation
///
/// Contains the commits within the depth limit and the shallow boundaries
/// (commits at the depth limit whose parents are not included).
#[derive(Debug, Clone)]
pub struct ShallowWalkResult {
    /// Commits within the depth limit (includes boundary commits)
    pub commits: Vec<Oid>,

    /// Commits at the depth limit (shallow boundaries)
    pub shallow_boundaries: HashSet<Oid>,
}

/// Commit graph walker with depth-limited traversal support
///
/// Provides efficient commit graph traversal with optional depth limiting
/// for shallow clone support. Tracks visited commits to avoid redundant work.
///
/// # Examples
///
/// ```no_run
/// use mediagit_versioning::{CommitWalker, ObjectDatabase, Oid};
/// use mediagit_storage::LocalBackend;
/// use std::sync::Arc;
///
/// #[tokio::main]
/// async fn main() -> anyhow::Result<()> {
///     let storage = Arc::new(LocalBackend::new("/tmp/odb")?);
///     let odb = Arc::new(ObjectDatabase::new(storage, 100));
///
///     let mut walker = CommitWalker::new(odb);
///     let head_oid = Oid::hash(b"head");
///
///     // Walk with depth limit of 10
///     let result = walker.walk_shallow(&head_oid, 10).await?;
///     println!("Found {} commits, {} boundaries",
///              result.commits.len(),
///              result.shallow_boundaries.len());
///
///     Ok(())
/// }
/// ```
pub struct CommitWalker {
    odb: Arc<ObjectDatabase>,
    visited: HashSet<Oid>,
}

impl CommitWalker {
    /// Create a new commit walker
    ///
    /// # Arguments
    ///
    /// * `odb` - Object database for reading commits
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use mediagit_versioning::{CommitWalker, ObjectDatabase};
    /// use mediagit_storage::LocalBackend;
    /// use std::sync::Arc;
    ///
    /// # async fn example() -> anyhow::Result<()> {
    /// let storage = Arc::new(LocalBackend::new("/tmp/odb")?);
    /// let odb = Arc::new(ObjectDatabase::new(storage, 100));
    /// let walker = CommitWalker::new(odb);
    /// # Ok(())
    /// # }
    /// ```
    pub fn new(odb: Arc<ObjectDatabase>) -> Self {
        Self {
            odb,
            visited: HashSet::new(),
        }
    }

    /// Walk commit graph with depth limit (for shallow clones)
    ///
    /// Traverses the commit graph starting from `start`, limiting traversal
    /// to `depth` commits. Returns all commits within the depth and marks
    /// commits at the depth limit as shallow boundaries.
    ///
    /// # Arguments
    ///
    /// * `start` - Starting commit OID (typically HEAD)
    /// * `depth` - Maximum depth to traverse (0 = only start commit)
    ///
    /// # Returns
    ///
    /// `ShallowWalkResult` containing:
    /// - `commits`: All commits within depth limit
    /// - `shallow_boundaries`: Commits at depth limit (boundary commits)
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use mediagit_versioning::{CommitWalker, ObjectDatabase, Oid};
    /// use mediagit_storage::LocalBackend;
    /// use std::sync::Arc;
    ///
    /// #[tokio::main]
    /// async fn main() -> anyhow::Result<()> {
    ///     let storage = Arc::new(LocalBackend::new("/tmp/odb")?);
    ///     let odb = Arc::new(ObjectDatabase::new(storage, 100));
    ///     let mut walker = CommitWalker::new(odb);
    ///
    ///     let head = Oid::hash(b"head");
    ///
    ///     // Clone with depth 1 (only HEAD commit)
    ///     let result = walker.walk_shallow(&head, 0).await?;
    ///     assert_eq!(result.commits.len(), 1);
    ///     assert_eq!(result.shallow_boundaries.len(), 1);
    ///
    ///     Ok(())
    /// }
    /// ```
    pub async fn walk_shallow(
        &mut self,
        start: &Oid,
        depth: usize,
    ) -> anyhow::Result<ShallowWalkResult> {
        let mut commits = Vec::new();
        let mut boundaries = HashSet::new();

        // Reset visited set for new walk
        self.visited.clear();

        // Perform depth-limited recursive walk
        self.walk_recursive(start, 0, depth, &mut commits, &mut boundaries)
            .await?;

        Ok(ShallowWalkResult {
            commits,
            shallow_boundaries: boundaries,
        })
    }

    /// Recursive helper for depth-limited traversal
    ///
    /// # Arguments
    ///
    /// * `oid` - Current commit OID
    /// * `current_depth` - Current depth in traversal (0 = start)
    /// * `max_depth` - Maximum allowed depth
    /// * `commits` - Accumulator for all commits within depth
    /// * `boundaries` - Accumulator for boundary commits at depth limit
    fn walk_recursive<'a>(
        &'a mut self,
        oid: &'a Oid,
        current_depth: usize,
        max_depth: usize,
        commits: &'a mut Vec<Oid>,
        boundaries: &'a mut HashSet<Oid>,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = anyhow::Result<()>> + 'a>> {
        Box::pin(async move {
            // Already visited? (handles merge commits and DAG structure)
            if !self.visited.insert(*oid) {
                return Ok(());
            }

            // Add commit to result
            commits.push(*oid);

            // At depth limit?
            if current_depth >= max_depth {
                // This is a boundary commit - its parents won't be included
                boundaries.insert(*oid);
                return Ok(());
            }

            // Read commit to get parents
            let commit_data = self.odb.read(oid).await?;
            let commit: Commit = bincode::deserialize(&commit_data)
                .map_err(|e| anyhow::anyhow!("Failed to deserialize commit: {}", e))?;

            // Recurse to all parents (depth-first traversal)
            for parent in &commit.parents {
                self.walk_recursive(
                    parent,
                    current_depth + 1,
                    max_depth,
                    commits,
                    boundaries,
                )
                .await?;
            }

            Ok(())
        })
    }

    /// Reset the visited set (useful for multiple walks)
    pub fn reset(&mut self) {
        self.visited.clear();
    }

    /// Get the number of visited commits
    pub fn visited_count(&self) -> usize {
        self.visited.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    #[test]
    fn test_signature_creation() {
        let timestamp = Utc::now();
        let sig = Signature::new(
            "Alice".to_string(),
            "alice@example.com".to_string(),
            timestamp,
        );

        assert_eq!(sig.name, "Alice");
        assert_eq!(sig.email, "alice@example.com");
        assert_eq!(sig.timestamp, timestamp);
    }

    #[test]
    fn test_signature_now() {
        let sig = Signature::now("Bob".to_string(), "bob@example.com".to_string());
        assert_eq!(sig.name, "Bob");
        assert_eq!(sig.email, "bob@example.com");
        // Timestamp should be very recent
        let now = Utc::now();
        let diff = (now - sig.timestamp).num_seconds();
        assert!(diff.abs() < 5); // Within 5 seconds
    }

    #[test]
    fn test_signature_display() {
        let timestamp = Utc::now();
        let sig = Signature::new(
            "Alice".to_string(),
            "alice@example.com".to_string(),
            timestamp,
        );
        let display = format!("{}", sig);
        assert!(display.contains("Alice"));
        assert!(display.contains("alice@example.com"));
    }

    #[test]
    fn test_commit_creation() {
        let tree = Oid::hash(b"tree");
        let sig = Signature::now("Alice".to_string(), "alice@example.com".to_string());
        let commit = Commit::new(
            tree,
            sig.clone(),
            sig,
            "Initial commit".to_string(),
        );

        assert_eq!(commit.tree, tree);
        assert_eq!(commit.message, "Initial commit");
        assert!(commit.is_initial());
        assert!(!commit.is_merge());
        assert_eq!(commit.parent_count(), 0);
    }

    #[test]
    fn test_commit_with_parent() {
        let tree = Oid::hash(b"tree");
        let parent = Oid::hash(b"parent");
        let sig = Signature::now("Alice".to_string(), "alice@example.com".to_string());

        let mut commit = Commit::new(tree, sig.clone(), sig, "Second commit".to_string());
        commit.add_parent(parent);

        assert_eq!(commit.parent_count(), 1);
        assert!(!commit.is_initial());
        assert!(!commit.is_merge());
        assert_eq!(commit.first_parent(), Some(&parent));
        assert_eq!(commit.parent(), Some(&parent));
    }

    #[test]
    fn test_commit_merge() {
        let tree = Oid::hash(b"tree");
        let parent1 = Oid::hash(b"parent1");
        let parent2 = Oid::hash(b"parent2");
        let sig = Signature::now("Alice".to_string(), "alice@example.com".to_string());

        let commit = Commit::with_parents(
            tree,
            vec![parent1, parent2],
            sig.clone(),
            sig,
            "Merge commit".to_string(),
        );

        assert_eq!(commit.parent_count(), 2);
        assert!(!commit.is_initial());
        assert!(commit.is_merge());
        assert_eq!(commit.first_parent(), Some(&parent1));
    }

    #[test]
    fn test_commit_summary() {
        let tree = Oid::hash(b"tree");
        let sig = Signature::now("Alice".to_string(), "alice@example.com".to_string());

        let commit = Commit::new(
            tree,
            sig.clone(),
            sig,
            "First line\nSecond line\nThird line".to_string(),
        );

        assert_eq!(commit.summary(), "First line");
        assert_eq!(commit.full_message(), "First line\nSecond line\nThird line");
    }

    #[test]
    fn test_commit_serialization() {
        let tree = Oid::hash(b"tree");
        let parent = Oid::hash(b"parent");
        let sig = Signature::now("Alice".to_string(), "alice@example.com".to_string());

        let mut commit = Commit::new(tree, sig.clone(), sig, "Test commit".to_string());
        commit.add_parent(parent);

        let serialized = commit.serialize().unwrap();
        let deserialized = Commit::deserialize(&serialized).unwrap();

        assert_eq!(commit, deserialized);
        assert_eq!(deserialized.message, "Test commit");
        assert_eq!(deserialized.parent_count(), 1);
    }

    #[test]
    fn test_commit_display() {
        let tree = Oid::hash(b"tree");
        let sig = Signature::now("Alice".to_string(), "alice@example.com".to_string());
        let commit = Commit::new(
            tree,
            sig.clone(),
            sig,
            "Feature: Add support for media".to_string(),
        );

        let display = format!("{}", commit);
        assert_eq!(display, "Feature: Add support for media");
    }

    #[tokio::test]
    async fn test_commit_odb_roundtrip() {
        use mediagit_storage::mock::MockBackend;
        use std::sync::Arc;

        let storage = Arc::new(MockBackend::new());
        let odb = crate::ObjectDatabase::new(storage, 100);

        let tree = Oid::hash(b"tree");
        let parent = Oid::hash(b"parent");
        let sig = Signature::now("Alice".to_string(), "alice@example.com".to_string());

        let mut commit = Commit::new(tree, sig.clone(), sig, "Test commit".to_string());
        commit.add_parent(parent);

        // Write commit
        let commit_oid = commit.write(&odb).await.unwrap();

        // Read commit back
        let loaded = Commit::read(&odb, &commit_oid).await.unwrap();
        assert_eq!(commit, loaded);
        assert_eq!(loaded.message, "Test commit");
        assert_eq!(loaded.parent_count(), 1);
    }

    #[test]
    fn test_commit_multiline_message() {
        let tree = Oid::hash(b"tree");
        let sig = Signature::now("Alice".to_string(), "alice@example.com".to_string());

        let message = "Refactor authentication system\n\nThis refactor improves security by:\n- Using bcrypt for password hashing\n- Implementing rate limiting\n- Adding audit logging";

        let commit = Commit::new(tree, sig.clone(), sig, message.to_string());

        assert_eq!(commit.summary(), "Refactor authentication system");
        assert!(commit.full_message().contains("bcrypt"));
        assert!(commit.full_message().contains("rate limiting"));
    }

    #[test]
    fn test_commit_empty_message() {
        let tree = Oid::hash(b"tree");
        let sig = Signature::now("Alice".to_string(), "alice@example.com".to_string());
        let commit = Commit::new(tree, sig.clone(), sig, "".to_string());

        assert_eq!(commit.summary(), "");
        assert_eq!(commit.full_message(), "");
    }

    #[tokio::test]
    async fn test_commit_chain() {
        use mediagit_storage::mock::MockBackend;
        use std::sync::Arc;

        let storage = Arc::new(MockBackend::new());
        let odb = crate::ObjectDatabase::new(storage, 100);

        let sig = Signature::now("Alice".to_string(), "alice@example.com".to_string());

        // Create initial commit
        let tree1 = Oid::hash(b"tree1");
        let commit1 = Commit::new(
            tree1,
            sig.clone(),
            sig.clone(),
            "Initial commit".to_string(),
        );
        let commit1_oid = commit1.write(&odb).await.unwrap();

        // Create second commit with first as parent
        let tree2 = Oid::hash(b"tree2");
        let mut commit2 = Commit::new(
            tree2,
            sig.clone(),
            sig.clone(),
            "Second commit".to_string(),
        );
        commit2.add_parent(commit1_oid);
        let commit2_oid = commit2.write(&odb).await.unwrap();

        // Verify chain
        let loaded1 = Commit::read(&odb, &commit1_oid).await.unwrap();
        let loaded2 = Commit::read(&odb, &commit2_oid).await.unwrap();

        assert!(loaded1.is_initial());
        assert_eq!(loaded2.parent(), Some(&commit1_oid));
        assert_eq!(loaded2.summary(), "Second commit");
    }

    #[tokio::test]
    async fn test_merge_commit() {
        use mediagit_storage::mock::MockBackend;
        use std::sync::Arc;

        let storage = Arc::new(MockBackend::new());
        let odb = crate::ObjectDatabase::new(storage, 100);

        let sig = Signature::now("Alice".to_string(), "alice@example.com".to_string());

        // Create two parent commits
        let tree1 = Oid::hash(b"tree1");
        let commit1 = Commit::new(
            tree1,
            sig.clone(),
            sig.clone(),
            "Feature A".to_string(),
        );
        let commit1_oid = commit1.write(&odb).await.unwrap();

        let tree2 = Oid::hash(b"tree2");
        let commit2 = Commit::new(
            tree2,
            sig.clone(),
            sig.clone(),
            "Feature B".to_string(),
        );
        let commit2_oid = commit2.write(&odb).await.unwrap();

        // Create merge commit
        let tree_merged = Oid::hash(b"tree_merged");
        let merge_commit = Commit::with_parents(
            tree_merged,
            vec![commit1_oid, commit2_oid],
            sig.clone(),
            sig,
            "Merge feature-a and feature-b".to_string(),
        );
        let merge_oid = merge_commit.write(&odb).await.unwrap();

        // Verify merge
        let loaded = Commit::read(&odb, &merge_oid).await.unwrap();
        assert!(loaded.is_merge());
        assert_eq!(loaded.parent_count(), 2);
        assert_eq!(loaded.first_parent(), Some(&commit1_oid));
    }

    // CommitWalker tests
    #[tokio::test]
    async fn test_walker_depth_zero() {
        use mediagit_storage::mock::MockBackend;
        use std::sync::Arc;

        let storage = Arc::new(MockBackend::new());
        let odb = Arc::new(ObjectDatabase::new(storage, 100));

        let sig = Signature::now("Alice".to_string(), "alice@example.com".to_string());

        // Create a single commit
        let tree = Oid::hash(b"tree");
        let commit = Commit::new(tree, sig.clone(), sig, "Initial commit".to_string());
        let commit_oid = commit.write(&odb).await.unwrap();

        // Walk with depth 0 (only the start commit)
        let mut walker = CommitWalker::new(odb);
        let result = walker.walk_shallow(&commit_oid, 0).await.unwrap();

        assert_eq!(result.commits.len(), 1);
        assert_eq!(result.commits[0], commit_oid);
        assert_eq!(result.shallow_boundaries.len(), 1);
        assert!(result.shallow_boundaries.contains(&commit_oid));
    }

    #[tokio::test]
    async fn test_walker_linear_chain() {
        use mediagit_storage::mock::MockBackend;
        use std::sync::Arc;

        let storage = Arc::new(MockBackend::new());
        let odb = Arc::new(ObjectDatabase::new(storage, 100));

        let sig = Signature::now("Alice".to_string(), "alice@example.com".to_string());

        // Create a chain: commit1 <- commit2 <- commit3 <- commit4
        let tree1 = Oid::hash(b"tree1");
        let commit1 = Commit::new(tree1, sig.clone(), sig.clone(), "Commit 1".to_string());
        let commit1_oid = commit1.write(&odb).await.unwrap();

        let tree2 = Oid::hash(b"tree2");
        let commit2 = Commit::with_parents(
            tree2,
            vec![commit1_oid],
            sig.clone(),
            sig.clone(),
            "Commit 2".to_string(),
        );
        let commit2_oid = commit2.write(&odb).await.unwrap();

        let tree3 = Oid::hash(b"tree3");
        let commit3 = Commit::with_parents(
            tree3,
            vec![commit2_oid],
            sig.clone(),
            sig.clone(),
            "Commit 3".to_string(),
        );
        let commit3_oid = commit3.write(&odb).await.unwrap();

        let tree4 = Oid::hash(b"tree4");
        let commit4 = Commit::with_parents(
            tree4,
            vec![commit3_oid],
            sig.clone(),
            sig.clone(),
            "Commit 4".to_string(),
        );
        let commit4_oid = commit4.write(&odb).await.unwrap();

        // Walk from commit4 with depth 2
        let mut walker = CommitWalker::new(odb);
        let result = walker.walk_shallow(&commit4_oid, 2).await.unwrap();

        // Should get: commit4 (depth 0), commit3 (depth 1), commit2 (depth 2)
        // commit2 is a boundary (at depth limit)
        assert_eq!(result.commits.len(), 3);
        assert!(result.commits.contains(&commit4_oid));
        assert!(result.commits.contains(&commit3_oid));
        assert!(result.commits.contains(&commit2_oid));

        // Boundary should be commit2
        assert_eq!(result.shallow_boundaries.len(), 1);
        assert!(result.shallow_boundaries.contains(&commit2_oid));
    }

    #[tokio::test]
    async fn test_walker_merge_commit() {
        use mediagit_storage::mock::MockBackend;
        use std::sync::Arc;

        let storage = Arc::new(MockBackend::new());
        let odb = Arc::new(ObjectDatabase::new(storage, 100));

        let sig = Signature::now("Alice".to_string(), "alice@example.com".to_string());

        // Create a merge structure:
        //     base
        //    /    \
        //  left  right
        //    \    /
        //     merge
        let tree_base = Oid::hash(b"tree_base");
        let base = Commit::new(
            tree_base,
            sig.clone(),
            sig.clone(),
            "Base".to_string(),
        );
        let base_oid = base.write(&odb).await.unwrap();

        let tree_left = Oid::hash(b"tree_left");
        let left = Commit::with_parents(
            tree_left,
            vec![base_oid],
            sig.clone(),
            sig.clone(),
            "Left".to_string(),
        );
        let left_oid = left.write(&odb).await.unwrap();

        let tree_right = Oid::hash(b"tree_right");
        let right = Commit::with_parents(
            tree_right,
            vec![base_oid],
            sig.clone(),
            sig.clone(),
            "Right".to_string(),
        );
        let right_oid = right.write(&odb).await.unwrap();

        let tree_merge = Oid::hash(b"tree_merge");
        let merge = Commit::with_parents(
            tree_merge,
            vec![left_oid, right_oid],
            sig.clone(),
            sig,
            "Merge".to_string(),
        );
        let merge_oid = merge.write(&odb).await.unwrap();

        // Walk from merge with depth 1
        let mut walker = CommitWalker::new(odb);
        let result = walker.walk_shallow(&merge_oid, 1).await.unwrap();

        // Should get: merge (depth 0), left (depth 1), right (depth 1)
        assert_eq!(result.commits.len(), 3);
        assert!(result.commits.contains(&merge_oid));
        assert!(result.commits.contains(&left_oid));
        assert!(result.commits.contains(&right_oid));

        // Both left and right are boundaries
        assert_eq!(result.shallow_boundaries.len(), 2);
        assert!(result.shallow_boundaries.contains(&left_oid));
        assert!(result.shallow_boundaries.contains(&right_oid));
    }

    #[tokio::test]
    async fn test_walker_reset() {
        use mediagit_storage::mock::MockBackend;
        use std::sync::Arc;

        let storage = Arc::new(MockBackend::new());
        let odb = Arc::new(ObjectDatabase::new(storage, 100));

        let sig = Signature::now("Alice".to_string(), "alice@example.com".to_string());

        let tree = Oid::hash(b"tree");
        let commit = Commit::new(tree, sig.clone(), sig, "Commit".to_string());
        let commit_oid = commit.write(&odb).await.unwrap();

        let mut walker = CommitWalker::new(odb);

        // First walk
        let _ = walker.walk_shallow(&commit_oid, 0).await.unwrap();
        assert_eq!(walker.visited_count(), 1);

        // Reset and walk again
        walker.reset();
        assert_eq!(walker.visited_count(), 0);

        let _ = walker.walk_shallow(&commit_oid, 0).await.unwrap();
        assert_eq!(walker.visited_count(), 1);
    }

    #[tokio::test]
    async fn test_walker_dag_structure() {
        use mediagit_storage::mock::MockBackend;
        use std::sync::Arc;

        let storage = Arc::new(MockBackend::new());
        let odb = Arc::new(ObjectDatabase::new(storage, 100));

        let sig = Signature::now("Alice".to_string(), "alice@example.com".to_string());

        // Create a DAG where commit is visited via two paths:
        //     c1
        //    /  \
        //   c2  c3
        //    \  /
        //     c4
        let tree1 = Oid::hash(b"tree1");
        let c1 = Commit::new(tree1, sig.clone(), sig.clone(), "C1".to_string());
        let c1_oid = c1.write(&odb).await.unwrap();

        let tree2 = Oid::hash(b"tree2");
        let c2 = Commit::with_parents(
            tree2,
            vec![c1_oid],
            sig.clone(),
            sig.clone(),
            "C2".to_string(),
        );
        let c2_oid = c2.write(&odb).await.unwrap();

        let tree3 = Oid::hash(b"tree3");
        let c3 = Commit::with_parents(
            tree3,
            vec![c1_oid],
            sig.clone(),
            sig.clone(),
            "C3".to_string(),
        );
        let c3_oid = c3.write(&odb).await.unwrap();

        let tree4 = Oid::hash(b"tree4");
        let c4 = Commit::with_parents(
            tree4,
            vec![c2_oid, c3_oid],
            sig.clone(),
            sig,
            "C4".to_string(),
        );
        let c4_oid = c4.write(&odb).await.unwrap();

        // Walk with depth 2 from c4
        let mut walker = CommitWalker::new(odb);
        let result = walker.walk_shallow(&c4_oid, 2).await.unwrap();

        // Should visit: c4 (depth 0), c2 (depth 1), c3 (depth 1), c1 (depth 2)
        // c1 should only be counted once even though reachable via two paths
        assert_eq!(result.commits.len(), 4);
        assert!(result.commits.contains(&c4_oid));
        assert!(result.commits.contains(&c2_oid));
        assert!(result.commits.contains(&c3_oid));
        assert!(result.commits.contains(&c1_oid));

        // c1 is the boundary
        assert_eq!(result.shallow_boundaries.len(), 1);
        assert!(result.shallow_boundaries.contains(&c1_oid));
    }
}
