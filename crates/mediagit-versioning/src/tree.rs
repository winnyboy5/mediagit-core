// SPDX-License-Identifier: BUSL-1.1
// Copyright (C) 2025-2026 Aswin Krishnamoorthy

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
///     let storage: Arc<dyn mediagit_storage::StorageBackend> =
///         Arc::new(LocalBackend::new("/tmp/odb").await?);
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

/// Refuse a tracked path that is absolute, drive-rooted or contains `..`.
///
/// Delegates to `mediagit_storage::validate_object_key`, the same validator
/// that guards object keys.
pub fn check_tracked_path(path: &str) -> anyhow::Result<()> {
    check_tracked_path_for(path, cfg!(windows))
}

/// Refuse a tree that gains a pair of paths differing only by letter case
/// (BUG-011), compared with the parent versions' trees.
///
/// Such a version cannot be checked out on Windows or macOS. A pair that ANY
/// parent already holds is allowed to stay, so repositories made on a case-sensitive
/// system keep working and a commit that removes one of the pair always
/// succeeds. Same folding rule as checkout: [`crate::case_only_collisions`].
pub fn check_no_new_case_collisions(tree: &Tree, parents: &[Tree]) -> anyhow::Result<()> {
    check_pairs_are_not_new(crate::case_only_collisions(tree.entries.keys()), parents)
}

/// [`check_no_new_case_collisions`] for pairs already found, so a caller can
/// skip loading the parents when there are none.
pub(crate) fn check_pairs_are_not_new(
    pairs: Vec<(std::path::PathBuf, std::path::PathBuf)>,
    parents: &[Tree],
) -> anyhow::Result<()> {
    let new_pairs: Vec<_> = pairs
        .into_iter()
        .filter(|(a, b)| {
            !parents.iter().any(|p| {
                p.entries.contains_key(a.to_string_lossy().as_ref())
                    && p.entries.contains_key(b.to_string_lossy().as_ref())
            })
        })
        .collect();
    if new_pairs.is_empty() {
        return Ok(());
    }
    let detail = new_pairs
        .iter()
        .map(|(a, b)| format!("'{}' and '{}'", a.display(), b.display()))
        .collect::<Vec<_>>()
        .join(", ");
    anyhow::bail!(
        "these paths differ only by letter case: {detail}. They would collide on Windows and \
         macOS (one overwrites the other), so this version cannot be saved. Rename one of \
         them, then run 'mediagit add --all' and save again."
    )
}

/// [`check_tracked_path`] with the platform as a parameter, so both
/// behaviours are testable on any host.
pub(crate) fn check_tracked_path_for(path: &str, windows: bool) -> anyhow::Result<()> {
    // The validator rejects `X:name` as a drive path on every platform. Off
    // Windows that is a legal file name (checkout's `worktree_path` accepts
    // it), so judge it with a neutral first letter. `X:/...` and `X:\...` stay
    // rejected everywhere.
    let b = path.as_bytes();
    let plain_colon_name = !windows
        && b.len() >= 3
        && b[0].is_ascii_alphabetic()
        && b[1] == b':'
        && b[2] != b'/'
        && b[2] != b'\\';
    let probe;
    let judged = if plain_colon_name {
        probe = format!("_{}", &path[1..]);
        probe.as_str()
    } else {
        path
    };
    mediagit_storage::validate_object_key(judged).map_err(|_| {
        anyhow::anyhow!(
            "refusing to track unsafe path '{path}': absolute or '..' paths are not allowed. \
             To get out: run 'mediagit add --all' (it drops the bad entry and re-adds the \
             file under its repo-relative path), or 'mediagit reset -- <path>' for an \
             entry that is only in the index"
        )
    })
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
    pub fn serialize(&self) -> anyhow::Result<Vec<u8>> {
        // BUG-008: every tree writer serializes here; an absolute or `..`
        // name would make every later checkout of this tree fail.
        for name in self.entries.keys() {
            check_tracked_path(name)?;
        }
        crate::format::serialize(self)
            .map_err(|e| anyhow::anyhow!("Tree serialization failed: {}", e))
    }

    /// Deserialize tree from bytes
    pub fn deserialize(data: &[u8]) -> anyhow::Result<Self> {
        crate::format::deserialize_exact(data)
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
    pub async fn write(&self, odb: &crate::ObjectDatabase) -> anyhow::Result<Oid> {
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
    pub async fn read(odb: &crate::ObjectDatabase, oid: &Oid) -> anyhow::Result<Self> {
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

    fn tree_of(names: &[&str]) -> Tree {
        let mut t = Tree::new();
        for n in names {
            t.add_entry(TreeEntry::new(
                n.to_string(),
                FileMode::Regular,
                Oid::hash(b"x"),
            ));
        }
        t
    }

    /// BUG-011: a NEW pair is refused with both paths, the platforms and the
    /// way out; a pair the parent already holds may stay; removing one of the
    /// pair always passes. Pure, so it runs on every platform.
    #[test]
    fn new_case_pairs_are_refused_existing_ones_are_grandfathered() {
        let both = tree_of(&["art/Hero.txt", "art/hero.txt"]);
        let msg = check_no_new_case_collisions(&both, &[])
            .unwrap_err()
            .to_string();
        for want in [
            "art/Hero.txt",
            "art/hero.txt",
            "Windows and",
            "macOS",
            "Rename",
        ] {
            assert!(msg.contains(want), "missing {want:?} in: {msg}");
        }
        assert!(!msg.contains("  "), "no runs of spaces: {msg}");
        // parent without the pair: refused
        let one = tree_of(&["art/hero.txt"]);
        assert!(check_no_new_case_collisions(&both, std::slice::from_ref(&one)).is_err());
        // parent with the pair: allowed to stay
        assert!(check_no_new_case_collisions(&both, std::slice::from_ref(&both)).is_ok());
        // a third colliding name is new even when two of the three were there
        let three = tree_of(&["art/Hero.txt", "art/hero.txt", "art/HERO.txt"]);
        assert!(check_no_new_case_collisions(&three, std::slice::from_ref(&both)).is_err());
        // removing one of the pair passes
        assert!(check_no_new_case_collisions(&one, std::slice::from_ref(&both)).is_ok());
    }

    /// Folder-level and non-ASCII pairs fold with the same rule.
    #[test]
    fn case_pairs_cover_folders_and_unicode() {
        assert!(check_no_new_case_collisions(&tree_of(&["Art/x.txt", "art/x.txt"]), &[]).is_err());
        assert!(check_no_new_case_collisions(&tree_of(&["\u{e9}/x", "\u{c9}/x"]), &[]).is_err());
        assert!(check_no_new_case_collisions(&tree_of(&["a/x", "a/y", "b/x"]), &[]).is_ok());
    }

    /// `serialize` itself no longer refuses a pair (the writers that know the
    /// parent do): a repository made on Linux must stay readable and writable.
    #[test]
    fn serialize_accepts_a_case_pair() {
        assert!(tree_of(&["Makefile", "makefile"]).serialize().is_ok());
    }

    fn tree_with(name: &str) -> Tree {
        let mut t = Tree::new();
        t.add_entry(TreeEntry::new(
            name.to_string(),
            FileMode::Regular,
            Oid::hash(b"x"),
        ));
        t
    }

    #[test]
    fn test_tree_serialize_refuses_absolute_and_parent_names() {
        for bad in [
            "//?/D:/work/repo/a.txt",
            "D:/work/a.txt",
            "/etc/passwd",
            "../a.txt",
            "dir/../../a.txt",
        ] {
            let err = tree_with(bad).serialize().unwrap_err();
            assert!(
                format!("{err:#}").contains(bad),
                "error must name {bad}: {err:#}"
            );
        }
    }

    /// `a:b.txt` is a legal file name off Windows; `a:/b` never is. The
    /// platform is a parameter so both halves run on any host.
    #[test]
    fn test_check_tracked_path_colon_names_per_platform() {
        assert!(check_tracked_path_for("dir/x:y", true).is_ok());
        for ok in ["a:b.txt", "v:take1.psd"] {
            assert!(check_tracked_path_for(ok, false).is_ok(), "unix {ok}");
            assert!(check_tracked_path_for(ok, true).is_err(), "windows {ok}");
        }
        for bad in ["a:/b", "a:\\b", "C:/x", "/x", "../x"] {
            assert!(check_tracked_path_for(bad, false).is_err(), "unix {bad}");
            assert!(check_tracked_path_for(bad, true).is_err(), "windows {bad}");
        }
    }

    #[test]
    fn test_check_tracked_path_error_names_path_and_way_out() {
        let msg = check_tracked_path_for("//?/C:/x/a.txt", true)
            .unwrap_err()
            .to_string();
        assert!(msg.contains("//?/C:/x/a.txt"), "{msg}");
        assert!(msg.contains("mediagit add --all"), "{msg}");
        assert!(msg.contains("mediagit reset --"), "{msg}");
    }

    #[test]
    fn test_tree_serialize_accepts_legitimate_names() {
        let deep = vec!["d"; 40].join("/");
        for ok in [
            "plain.txt",
            "my dir/with space.txt",
            "caf\u{e9}/\u{65e5}\u{672c}.psd",
            "a..b/c.txt",
            deep.as_str(),
        ] {
            tree_with(ok)
                .serialize()
                .unwrap_or_else(|e| panic!("{ok}: {e:#}"));
        }
    }

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
        tree.add_entry(TreeEntry::new(
            "file.txt".to_string(),
            FileMode::Regular,
            oid,
        ));

        let serialized = tree.serialize().unwrap();
        let deserialized = Tree::deserialize(&serialized).unwrap();

        assert_eq!(tree, deserialized);
    }

    #[test]
    fn test_tree_file_and_dir_count() {
        let mut tree = Tree::new();
        let oid = Oid::hash(b"content");

        tree.add_entry(TreeEntry::new(
            "file1.txt".to_string(),
            FileMode::Regular,
            oid,
        ));
        tree.add_entry(TreeEntry::new(
            "file2.txt".to_string(),
            FileMode::Executable,
            oid,
        ));
        tree.add_entry(TreeEntry::new(
            "subdir".to_string(),
            FileMode::Directory,
            oid,
        ));

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
        tree.add_entry(TreeEntry::new(
            "file.txt".to_string(),
            FileMode::Regular,
            oid,
        ));

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
