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

//! Per-push upload journal for crash-resume of presigned multipart uploads.
//!
//! The journal is a JSON file written to `.mediagit/upload/<push-id>.journal`
//! inside the local repository directory. It records per-chunk state so that
//! an interrupted push can resume without re-uploading already-completed parts.
//!
//! # State machine
//!
//! ```text
//! Pending → MpuInProgress { upload_id, completed_parts } → Done
//!         ↘                                               ↗
//!           → Done  (via single-PUT presigned path)
//! ```
//!
//! `MpuInProgress` entries survive across process restarts. On the next push
//! attempt the client checks the journal: if an entry is `MpuInProgress` with
//! a non-empty `completed_parts` list, it can resume by uploading only the
//! remaining parts and calling `mpu/complete` with the full set.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum ChunkState {
    Pending,
    MpuInProgress {
        upload_id: String,
        /// Ordered list of (part_number, etag) pairs already confirmed by S3/MinIO.
        completed_parts: Vec<(i32, String)>,
    },
    Done,
}

/// Per-push upload journal.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UploadJournal {
    pub push_id: String,
    #[serde(default)]
    pub chunks: HashMap<String, ChunkState>,
}

impl UploadJournal {
    pub fn new(push_id: impl Into<String>) -> Self {
        Self {
            push_id: push_id.into(),
            chunks: HashMap::new(),
        }
    }

    /// Load from disk, creating a new empty journal if the file does not exist.
    pub fn load_or_new(path: &Path, push_id: &str) -> anyhow::Result<Self> {
        if path.exists() {
            let data = std::fs::read_to_string(path)?;
            let j: Self = serde_json::from_str(&data)?;
            return Ok(j);
        }
        Ok(Self::new(push_id))
    }

    /// Persist the journal to `path`, creating parent directories as needed.
    pub fn save(&self, path: &Path) -> anyhow::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let data = serde_json::to_string(self)?;
        std::fs::write(path, data)?;
        Ok(())
    }

    /// Canonical journal path for a given repository root and push ID.
    pub fn journal_path(repo_dir: &Path, push_id: &str) -> PathBuf {
        repo_dir
            .join(".mediagit")
            .join("upload")
            .join(format!("{}.journal", push_id))
    }

    pub fn mark_done(&mut self, chunk_hex: &str) {
        self.chunks.insert(chunk_hex.to_string(), ChunkState::Done);
    }

    pub fn mark_mpu_in_progress(
        &mut self,
        chunk_hex: &str,
        upload_id: String,
        completed_parts: Vec<(i32, String)>,
    ) {
        self.chunks.insert(
            chunk_hex.to_string(),
            ChunkState::MpuInProgress {
                upload_id,
                completed_parts,
            },
        );
    }

    pub fn get_state(&self, chunk_hex: &str) -> Option<&ChunkState> {
        self.chunks.get(chunk_hex)
    }

    pub fn is_done(&self, chunk_hex: &str) -> bool {
        matches!(self.chunks.get(chunk_hex), Some(ChunkState::Done))
    }

    /// Remove the journal file if it exists. Call after a push completes
    /// successfully to avoid stale journals accumulating on disk.
    pub fn delete(path: &Path) {
        let _ = std::fs::remove_file(path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_empty() {
        let j = UploadJournal::new("push-001");
        let json = serde_json::to_string(&j).unwrap();
        let back: UploadJournal = serde_json::from_str(&json).unwrap();
        assert_eq!(back.push_id, "push-001");
        assert!(back.chunks.is_empty());
    }

    #[test]
    fn mark_and_query_states() {
        let mut j = UploadJournal::new("push-002");
        j.mark_done("aabb");
        j.mark_mpu_in_progress(
            "ccdd",
            "upload-id-1".to_string(),
            vec![(1, "etag1".to_string()), (2, "etag2".to_string())],
        );

        assert!(j.is_done("aabb"));
        assert!(!j.is_done("ccdd"));
        assert!(!j.is_done("unknown"));

        match j.get_state("ccdd") {
            Some(ChunkState::MpuInProgress {
                upload_id,
                completed_parts,
            }) => {
                assert_eq!(upload_id, "upload-id-1");
                assert_eq!(completed_parts.len(), 2);
            }
            _ => panic!("expected MpuInProgress"),
        }
    }

    #[test]
    fn save_and_load_from_disk() {
        let dir = std::env::temp_dir().join("mediagit-journal-test-save");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("test-push-003.journal");
        let _ = std::fs::remove_file(&path); // clean up from prior run

        let mut j = UploadJournal::new("push-003");
        j.mark_done("ff00");
        j.save(&path).unwrap();

        let loaded = UploadJournal::load_or_new(&path, "push-003").unwrap();
        assert!(loaded.is_done("ff00"));

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn load_or_new_creates_fresh_when_missing() {
        let path = std::env::temp_dir().join("mediagit-journal-test-nonexistent-xyz987.journal");
        let _ = std::fs::remove_file(&path);

        let j = UploadJournal::load_or_new(&path, "push-004").unwrap();
        assert_eq!(j.push_id, "push-004");
        assert!(j.chunks.is_empty());
    }

    #[test]
    fn journal_path_uses_mediagit_subdir() {
        let repo = std::path::Path::new("/repo/test");
        let p = UploadJournal::journal_path(repo, "abc123");
        assert_eq!(
            p,
            std::path::Path::new("/repo/test/.mediagit/upload/abc123.journal")
        );
    }
}
