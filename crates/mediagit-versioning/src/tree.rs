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

//! Tree object representing directory structures
//!
//! A Tree object contains references to files (blobs) and subdirectories (other trees).
//! Trees are Git-compatible and provide the structure for snapshots in commits.

use crate::{ObjectType, Oid};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fmt;

/// File mode in a tree entry (Unix-like permission bits)
///
/// Represents the type and permissions of a file in the tree.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum FileMode {
    /// Regular file (100644)
    Regular = 0o100644,
    /// Executable file (100755)
    Executable = 0o100755,
    /// Symlink (120000)
    Symlink = 0o120000,
    /// Directory/tree (40000)
    Directory = 0o040000,
}

impl FileMode {
    /// Get the file mode from an integer value
    pub fn from_u32(mode: u32) -> anyhow::Result<Self> {
        match mode {
            0o100644 => Ok(FileMode::Regular),
            0o100755 => Ok(FileMode::Executable),
            0o120000 => Ok(FileMode::Symlink),
            0o040000 => Ok(FileMode::Directory),
            _ => anyhow::bail!("Unknown file mode: {:o}", mode),
        }
    }

    /// Convert to u32 representation
    pub fn as_u32(&self) -> u32 {
        *self as u32
    }

    /// Determine object type based on file mode
    pub fn object_type(&self) -> ObjectType {
        match self {
            FileMode::Directory => ObjectType::Tree,
            _ => ObjectType::Blob,
        }
    }
}

impl fmt::Display for FileMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:06o}", self.as_u32())
    }
}

/// Entry in a tree (file or subdirectory)
///
/// # Examples
///
/// ```
/// use mediagit_versioning::{TreeEntry, FileMode, Oid};
///
/// let oid = Oid::hash(b"content");
/// let entry = TreeEntry::new(
///     "README.md".to_string(),
///     FileMode::Regular,
///     oid
/// );
/// assert_eq!(entry.name, "README.md");
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TreeEntry {
    /// File name (not full path)
    pub name: String,

    /// File mode (permissions and type)
    pub mode: FileMode,

    /// Object ID (OID) of the blob or tree
    pub oid: Oid,
}

impl TreeEntry {
    /// Create a new tree entry
    ///
    /// # Arguments
    ///
    /// * `name` - File or directory name
    /// * `mode` - File mode (regular, executable, symlink, or directory)
    /// * `oid` - OID of the blob or tree
    pub fn new(name: String, mode: FileMode, oid: Oid) -> Self {
        Self { name, mode, oid }
    }

    /// Check if this entry points to a tree (directory)
    pub fn is_tree(&self) -> bool {
        self.mode == FileMode::Directory
    }

    /// Check if this entry is a blob (file)
    pub fn is_blob(&self) -> bool {
        self.mode != FileMode::Directory
    }
}

/// Tree object representing a directory snapshot
///
/// Trees contain entries sorted by name, providing a canonical structure
/// for version control snapshots.
///
/// # Examples
///
/// ```no_run
/// use mediagit_versioning::{Tree, TreeEntry, FileMode, Oid, ObjectDatabase, ObjectType};
/// use mediagit_storage::LocalBackend;
/// use std::sync::Arc;
///
/// #[tokio::main]
/// async fn main() -> anyhow::Result<()> {
///     let storage = Arc::new(LocalBackend::new("/tmp/odb")?);
///     let odb = ObjectDatabase::new(storage, 100);
///
///     // Create a tree with two files
///     let mut tree = Tree::new();
///
///     let file_oid = odb.write(ObjectType::Blob, b"file content").await?;
///     tree.add_entry(TreeEntry::new(
///         "file.txt".to_string(),
///         FileMode::Regular,
///         file_oid
///     ));
///
///     // Store tree and get its OID
///     let tree_oid = tree.write(&odb).await?;
///     println!("Tree OID: {}", tree_oid);
///
///     // Load tree back from OID
///     let loaded_tree = Tree::read(&odb, &tree_oid).await?;
///     assert_eq!(loaded_tree.entries.len(), 1);
///
///     Ok(())
/// }
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Tree {
    /// Tree entries, sorted by name for canonical representation
    pub entries: BTreeMap<String, TreeEntry>,
}

impl Tree {
    /// Create a new empty tree
    pub fn new() -> Self {
        Self {
            entries: BTreeMap::new(),
        }
    }

    /// Add an entry to the tree
    ///
    /// Entries are automatically sorted by name.
    pub fn add_entry(&mut self, entry: TreeEntry) {
        self.entries.insert(entry.name.clone(), entry);
    }

    /// Remove an entry from the tree
    pub fn remove_entry(&mut self, name: &str) -> Option<TreeEntry> {
        self.entries.remove(name)
    }

    /// Get an entry by name
    pub fn get_entry(&self, name: &str) -> Option<&TreeEntry> {
        self.entries.get(name)
    }

    /// Check if tree contains an entry by name
    pub fn has_entry(&self, name: &str) -> bool {
        self.entries.contains_key(name)
    }

    /// Get all entries in sorted order
    pub fn iter(&self) -> impl Iterator<Item = &TreeEntry> {
        self.entries.values()
    }

    /// Get number of entries
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Check if tree is empty
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Serialize tree to bytes
    ///
    /// Uses bincode for efficient serialization.
    pub fn serialize(&self) -> anyhow::Result<Vec<u8>> {
        bincode::serialize(self).map_err(|e| anyhow::anyhow!("Tree serialization failed: {}", e))
    }

    /// Deserialize tree from bytes
    pub fn deserialize(data: &[u8]) -> anyhow::Result<Self> {
        bincode::deserialize(data)
            .map_err(|e| anyhow::anyhow!("Tree deserialization failed: {}", e))
    }

    /// Write tree to object database and return its OID
    ///
    /// # Arguments
    ///
    /// * `odb` - Object database instance
    ///
    /// # Returns
    ///
    /// The OID of the written tree
    pub async fn write(
        &self,
        odb: &crate::ObjectDatabase,
    ) -> anyhow::Result<Oid> {
        let data = self.serialize()?;
        odb.write(ObjectType::Tree, &data).await
    }

    /// Read tree from object database by OID
    ///
    /// # Arguments
    ///
    /// * `odb` - Object database instance
    /// * `oid` - Object ID of the tree
    ///
    /// # Returns
    ///
    /// The deserialized tree object
    pub async fn read(
        odb: &crate::ObjectDatabase,
        oid: &Oid,
    ) -> anyhow::Result<Self> {
        let data = odb.read(oid).await?;
        Self::deserialize(&data)
    }

    /// Count files (blobs) in tree (not recursively)
    pub fn file_count(&self) -> usize {
        self.entries.values().filter(|e| e.is_blob()).count()
    }

    /// Count subdirectories (trees) in tree (not recursively)
    pub fn dir_count(&self) -> usize {
        self.entries.values().filter(|e| e.is_tree()).count()
    }
}

impl Default for Tree {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_file_mode_values() {
        assert_eq!(FileMode::Regular.as_u32(), 0o100644);
        assert_eq!(FileMode::Executable.as_u32(), 0o100755);
        assert_eq!(FileMode::Symlink.as_u32(), 0o120000);
        assert_eq!(FileMode::Directory.as_u32(), 0o040000);
    }

    #[test]
    fn test_file_mode_from_u32() {
        assert_eq!(FileMode::from_u32(0o100644).unwrap(), FileMode::Regular);
        assert_eq!(FileMode::from_u32(0o100755).unwrap(), FileMode::Executable);
        assert_eq!(FileMode::from_u32(0o120000).unwrap(), FileMode::Symlink);
        assert_eq!(FileMode::from_u32(0o040000).unwrap(), FileMode::Directory);
        assert!(FileMode::from_u32(0o777).is_err());
    }

    #[test]
    fn test_file_mode_object_type() {
        assert_eq!(FileMode::Regular.object_type(), ObjectType::Blob);
        assert_eq!(FileMode::Executable.object_type(), ObjectType::Blob);
        assert_eq!(FileMode::Symlink.object_type(), ObjectType::Blob);
        assert_eq!(FileMode::Directory.object_type(), ObjectType::Tree);
    }

    #[test]
    fn test_tree_entry_creation() {
        let oid = Oid::hash(b"test");
        let entry = TreeEntry::new("file.txt".to_string(), FileMode::Regular, oid);

        assert_eq!(entry.name, "file.txt");
        assert_eq!(entry.mode, FileMode::Regular);
        assert_eq!(entry.oid, oid);
        assert!(entry.is_blob());
        assert!(!entry.is_tree());
    }

    #[test]
    fn test_tree_add_entry() {
        let mut tree = Tree::new();
        let oid = Oid::hash(b"content");
        let entry = TreeEntry::new("file.txt".to_string(), FileMode::Regular, oid);

        tree.add_entry(entry.clone());
        assert_eq!(tree.len(), 1);
        assert_eq!(tree.get_entry("file.txt"), Some(&entry));
        assert!(tree.has_entry("file.txt"));
    }

    #[test]
    fn test_tree_multiple_entries() {
        let mut tree = Tree::new();
        let oid1 = Oid::hash(b"file1");
        let oid2 = Oid::hash(b"file2");

        tree.add_entry(TreeEntry::new("b.txt".to_string(), FileMode::Regular, oid2));
        tree.add_entry(TreeEntry::new("a.txt".to_string(), FileMode::Regular, oid1));

        // Entries should be sorted by name
        let names: Vec<&str> = tree.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, vec!["a.txt", "b.txt"]);
    }

    #[test]
    fn test_tree_remove_entry() {
        let mut tree = Tree::new();
        let oid = Oid::hash(b"content");
        let entry = TreeEntry::new("file.txt".to_string(), FileMode::Regular, oid);

        tree.add_entry(entry.clone());
        assert_eq!(tree.len(), 1);

        let removed = tree.remove_entry("file.txt");
        assert_eq!(removed, Some(entry));
        assert_eq!(tree.len(), 0);
    }

    #[test]
    fn test_tree_empty() {
        let tree = Tree::new();
        assert!(tree.is_empty());
        assert_eq!(tree.len(), 0);
    }

    #[test]
    fn test_tree_serialization() {
        let mut tree = Tree::new();
        let oid = Oid::hash(b"content");
        tree.add_entry(TreeEntry::new("file.txt".to_string(), FileMode::Regular, oid));

        let serialized = tree.serialize().unwrap();
        let deserialized = Tree::deserialize(&serialized).unwrap();

        assert_eq!(tree, deserialized);
    }

    #[test]
    fn test_tree_file_and_dir_count() {
        let mut tree = Tree::new();
        let oid = Oid::hash(b"content");

        tree.add_entry(TreeEntry::new("file1.txt".to_string(), FileMode::Regular, oid));
        tree.add_entry(TreeEntry::new("file2.txt".to_string(), FileMode::Executable, oid));
        tree.add_entry(TreeEntry::new("subdir".to_string(), FileMode::Directory, oid));

        assert_eq!(tree.file_count(), 2);
        assert_eq!(tree.dir_count(), 1);
    }

    #[tokio::test]
    async fn test_tree_odb_roundtrip() {
        use mediagit_storage::mock::MockBackend;
        use std::sync::Arc;

        let storage = Arc::new(MockBackend::new());
        let odb = crate::ObjectDatabase::new(storage, 100);

        let mut tree = Tree::new();
        let oid = Oid::hash(b"file content");
        tree.add_entry(TreeEntry::new("file.txt".to_string(), FileMode::Regular, oid));

        // Write tree
        let tree_oid = tree.write(&odb).await.unwrap();

        // Read tree back
        let loaded_tree = Tree::read(&odb, &tree_oid).await.unwrap();
        assert_eq!(tree, loaded_tree);
        assert_eq!(loaded_tree.len(), 1);
    }

    #[test]
    fn test_tree_default() {
        let tree = Tree::default();
        assert!(tree.is_empty());
    }

    #[tokio::test]
    async fn test_tree_complex_structure() {
        use mediagit_storage::mock::MockBackend;
        use std::sync::Arc;

        let storage = Arc::new(MockBackend::new());
        let odb = crate::ObjectDatabase::new(storage, 100);

        let mut tree = Tree::new();

        // Add multiple files with different modes
        tree.add_entry(TreeEntry::new(
            "README.md".to_string(),
            FileMode::Regular,
            Oid::hash(b"readme"),
        ));
        tree.add_entry(TreeEntry::new(
            "script.sh".to_string(),
            FileMode::Executable,
            Oid::hash(b"script"),
        ));
        tree.add_entry(TreeEntry::new(
            "link".to_string(),
            FileMode::Symlink,
            Oid::hash(b"link"),
        ));
        tree.add_entry(TreeEntry::new(
            "src".to_string(),
            FileMode::Directory,
            Oid::hash(b"subtree"),
        ));

        // Write and read back
        let tree_oid = tree.write(&odb).await.unwrap();
        let loaded = Tree::read(&odb, &tree_oid).await.unwrap();

        assert_eq!(loaded.file_count(), 3);
        assert_eq!(loaded.dir_count(), 1);
        assert_eq!(loaded.len(), 4);
    }
}
