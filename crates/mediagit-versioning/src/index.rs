// SPDX-License-Identifier: BUSL-1.1
// Copyright (C) 2025-2026 Aswin Krishnamoorthy

//! Staging area index management.
//!
//! The index (staging area) tracks files that have been staged for the next commit.
//! It maps file paths to their object IDs (OIDs) in the object database.

use crate::Oid;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

/// True when a path is a leftover `::stageN` merge-conflict key. These are
/// write-only debris (no reader anywhere) and must never reach a committed tree.
pub fn is_stage_debris_key(path: &str) -> bool {
    path.ends_with("::stage1") || path.ends_with("::stage2") || path.ends_with("::stage3")
}

/// An entry in the staging area index
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct IndexEntry {
    /// Path to the file relative to repository root
    pub path: PathBuf,
    /// Object ID of the staged content
    pub oid: Oid,
    /// File mode/permissions
    pub mode: u32,
    /// File size in bytes
    pub size: u64,
    /// File modification time (seconds since UNIX epoch) for stat-cache
    #[serde(default)]
    pub mtime: Option<u64>,
}

impl IndexEntry {
    /// Create a new index entry
    pub fn new(path: PathBuf, oid: Oid, mode: u32, size: u64, mtime: Option<u64>) -> Self {
        Self {
            path,
            oid,
            mode,
            size,
            mtime,
        }
    }
}

/// The staging area index
///
/// The index tracks which files have been staged for the next commit.
/// It is persisted to `.mediagit/index` as a JSON file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Index {
    /// Map of file paths to index entries
    entries: BTreeMap<PathBuf, IndexEntry>,
    /// Files marked for deletion (to be removed from tree at commit time)
    #[serde(default)]
    deleted_entries: HashSet<PathBuf>,
    /// WT-9: paths left unresolved by a conflicting merge/rebase/revert.
    ///
    /// The index previously had no way to say "this path is conflicted", so
    /// `--continue` could not tell a reviewed resolution from an untouched
    /// one. The only available signal was scanning the file for `<<<<<<<`
    /// markers — which is a *text* signal, and MediaGit is a VCS for media
    /// and binary files. A conflicting PSD never gets markers (inlining them
    /// would corrupt it); the resolver checks out one side provisionally
    /// instead. Marker-scanning therefore reported "resolved" for exactly the
    /// file types this system exists to version.
    ///
    /// This records the conflict directly. `add`-ing a path clears it, so the
    /// user's acknowledgement is an affirmative act that works identically
    /// whether they edited a text file or accepted the side chosen for a
    /// binary one.
    ///
    /// Deliberately not a git-style stage 1/2/3 triple: for a 74 MB PSD there
    /// is no meaningful "both sides in the index at once", so the useful state
    /// is simply *conflicted and awaiting acknowledgement*.
    #[serde(default)]
    unresolved: HashSet<PathBuf>,
    /// Version of the index format
    version: u32,
}

impl Index {
    /// Create a new empty index
    pub fn new() -> Self {
        Self {
            entries: BTreeMap::new(),
            deleted_entries: HashSet::new(),
            unresolved: HashSet::new(),
            version: 1,
        }
    }

    /// Load index from the repository
    pub fn load(repo_root: &Path) -> Result<Self> {
        let index_path = repo_root.join(".mediagit/index");

        if !index_path.exists() {
            // No index file yet, return empty index
            return Ok(Self::new());
        }

        let contents = fs::read_to_string(&index_path)
            .with_context(|| format!("Failed to read index file: {}", index_path.display()))?;

        let index: Index = serde_json::from_str(&contents).context("Failed to parse index file")?;

        Ok(index)
    }

    /// Save index to the repository
    pub fn save(&self, repo_root: &Path) -> Result<()> {
        let index_path = repo_root.join(".mediagit/index");

        let contents = serde_json::to_string_pretty(self).context("Failed to serialize index")?;

        // VC-4: atomic replace. A plain `fs::write` truncates first, so a
        // crash or ENOSPC mid-write left a half-written index — and
        // `Index::load` has no recovery path, so every later command failed
        // until the file was deleted by hand, discarding whatever was staged.
        // This is the highest-frequency write in the tool (every add/commit).
        crate::atomic_write::write_atomic(&index_path, contents.as_bytes())
            .with_context(|| format!("Failed to write index file: {}", index_path.display()))?;

        Ok(())
    }

    /// Add or update an entry in the index.
    ///
    /// WT-9: staging a path also clears any unresolved-conflict marker on it.
    /// `mediagit add <path>` is the user's affirmative "I have dealt with
    /// this", and it works the same whether they edited a text file or
    /// accepted the side the resolver checked out for a binary one.
    pub fn add_entry(&mut self, entry: IndexEntry) {
        self.unresolved.remove(&entry.path);
        self.entries.insert(entry.path.clone(), entry);
    }

    /// WT-9: mark `path` as conflicted and awaiting acknowledgement.
    pub fn mark_unresolved(&mut self, path: PathBuf) {
        self.unresolved.insert(path);
    }

    /// Paths still awaiting conflict acknowledgement, sorted for stable output.
    pub fn unresolved_paths(&self) -> Vec<PathBuf> {
        let mut v: Vec<PathBuf> = self.unresolved.iter().cloned().collect();
        v.sort();
        v
    }

    /// Whether any path is still conflicted.
    pub fn has_unresolved(&self) -> bool {
        !self.unresolved.is_empty()
    }

    /// Drop all unresolved markers (used when an operation is aborted).
    pub fn clear_unresolved(&mut self) {
        self.unresolved.clear();
    }

    /// Remove an entry from the index
    pub fn remove_entry(&mut self, path: &Path) -> Option<IndexEntry> {
        self.entries.remove(path)
    }

    /// Get an entry from the index
    pub fn get_entry(&self, path: &Path) -> Option<&IndexEntry> {
        self.entries.get(path)
    }

    /// Check if the index contains a path
    pub fn contains(&self, path: &Path) -> bool {
        self.entries.contains_key(path)
    }

    /// Get all entries in the index
    pub fn entries(&self) -> impl Iterator<Item = &IndexEntry> {
        self.entries.values()
    }

    /// Get the number of entries in the index
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Check if the index is empty (no staged files or deletions)
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty() && self.deleted_entries.is_empty()
    }

    /// Clear all entries from the index (both additions and deletions)
    pub fn clear(&mut self) {
        self.entries.clear();
        self.deleted_entries.clear();
        // A commit resolves whatever the operation was; conflict markers must
        // not survive into the next one.
        self.unresolved.clear();
    }

    /// Get all staged file paths
    pub fn staged_paths(&self) -> Vec<PathBuf> {
        self.entries.keys().cloned().collect()
    }

    /// Get staged files as (path, oid) pairs
    pub fn staged_files(&self) -> Vec<(PathBuf, Oid)> {
        self.entries
            .values()
            .map(|entry| (entry.path.clone(), entry.oid))
            .collect()
    }

    // ===== Deletion tracking methods =====

    /// Mark a file as deleted (to be removed from tree at commit time)
    pub fn mark_deleted(&mut self, path: PathBuf) {
        // If file was staged for addition, remove it from entries
        self.entries.remove(&path);
        // Add to deleted entries
        self.deleted_entries.insert(path);
    }

    /// Check if a file is marked for deletion
    pub fn is_deleted(&self, path: &Path) -> bool {
        self.deleted_entries.contains(path)
    }

    /// Get all files marked for deletion
    pub fn deleted_paths(&self) -> impl Iterator<Item = &PathBuf> {
        self.deleted_entries.iter()
    }

    /// Get the number of files marked for deletion
    pub fn deleted_count(&self) -> usize {
        self.deleted_entries.len()
    }

    /// Check if any files are marked for deletion
    pub fn has_deletions(&self) -> bool {
        !self.deleted_entries.is_empty()
    }
}

impl Default for Index {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_index_new() {
        let index = Index::new();
        assert!(index.is_empty());
        assert_eq!(index.len(), 0);
        assert_eq!(index.version, 1);
    }

    #[test]
    fn test_index_add_entry() {
        let mut index = Index::new();
        let oid = Oid::hash(b"test content");
        let entry = IndexEntry::new(PathBuf::from("test.txt"), oid, 0o100644, 12, None);

        index.add_entry(entry.clone());
        assert_eq!(index.len(), 1);
        assert!(index.contains(Path::new("test.txt")));
        assert_eq!(index.get_entry(Path::new("test.txt")), Some(&entry));
    }

    #[test]
    fn test_index_remove_entry() {
        let mut index = Index::new();
        let oid = Oid::hash(b"test content");
        let entry = IndexEntry::new(PathBuf::from("test.txt"), oid, 0o100644, 12, None);

        index.add_entry(entry.clone());
        let removed = index.remove_entry(Path::new("test.txt"));

        assert_eq!(removed, Some(entry));
        assert!(index.is_empty());
        assert!(!index.contains(Path::new("test.txt")));
    }

    #[test]
    fn test_index_save_load() {
        let temp_dir = TempDir::new().unwrap();
        let repo_root = temp_dir.path();
        fs::create_dir(repo_root.join(".mediagit")).unwrap();

        let mut index = Index::new();
        let oid = Oid::hash(b"test content");
        let entry = IndexEntry::new(PathBuf::from("test.txt"), oid, 0o100644, 12, None);
        index.add_entry(entry.clone());

        // Save and load
        index.save(repo_root).unwrap();
        let loaded = Index::load(repo_root).unwrap();

        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded.get_entry(Path::new("test.txt")), Some(&entry));
    }

    #[test]
    fn test_index_load_nonexistent() {
        let temp_dir = TempDir::new().unwrap();
        let repo_root = temp_dir.path();
        fs::create_dir(repo_root.join(".mediagit")).unwrap();

        let index = Index::load(repo_root).unwrap();
        assert!(index.is_empty());
    }

    #[test]
    fn test_index_staged_paths() {
        let mut index = Index::new();

        let oid1 = Oid::hash(b"content1");
        let oid2 = Oid::hash(b"content2");

        index.add_entry(IndexEntry::new(
            PathBuf::from("file1.txt"),
            oid1,
            0o100644,
            8,
            None,
        ));
        index.add_entry(IndexEntry::new(
            PathBuf::from("file2.txt"),
            oid2,
            0o100644,
            8,
            None,
        ));

        let paths = index.staged_paths();
        assert_eq!(paths.len(), 2);
        assert!(paths.contains(&PathBuf::from("file1.txt")));
        assert!(paths.contains(&PathBuf::from("file2.txt")));
    }
}
