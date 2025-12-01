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

use crate::{ObjectType, Oid};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::fmt;

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
}
