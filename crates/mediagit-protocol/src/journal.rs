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

    /// Sweep stale journal files (`.mediagit/upload/*.journal`) older than
    /// `max_age`. Age-based only — a journal younger than `max_age` is left
    /// alone even if its push has already completed (it'll be cleaned up by
    /// `delete` on the next successful push, or by a later sweep once it
    /// ages out). Never touches anything but `*.journal` files in that one
    /// directory, so it can't collide with live/fresh uploads. Idempotent:
    /// running it twice in a row sweeps nothing the second time. Returns the
    /// number of files removed; best-effort — a single file's stat/remove
    /// failure doesn't abort the sweep of the rest.
    pub fn sweep_stale(repo_dir: &Path, max_age: std::time::Duration) -> usize {
        let upload_dir = repo_dir.join(".mediagit").join("upload");
        let entries = match std::fs::read_dir(&upload_dir) {
            Ok(e) => e,
            Err(_) => return 0, // no upload dir yet — nothing to sweep
        };

        let now = std::time::SystemTime::now();
        let mut swept = 0;
        for entry in entries.filter_map(|e| e.ok()) {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("journal") {
                continue;
            }
            let Ok(metadata) = entry.metadata() else {
                continue;
            };
            let Ok(modified) = metadata.modified() else {
                continue;
            };
            let Ok(age) = now.duration_since(modified) else {
                continue; // clock skew (mtime in the future) — leave it alone
            };
            if age > max_age && std::fs::remove_file(&path).is_ok() {
                swept += 1;
            }
        }
        swept
    }
}

/// Default staleness threshold for [`UploadJournal::sweep_stale`]: journals
/// older than this are assumed abandoned (the push that created them either
/// completed via `delete()` or failed permanently long ago).
pub const DEFAULT_JOURNAL_MAX_AGE: std::time::Duration =
    std::time::Duration::from_secs(7 * 24 * 3600);

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
    fn sweep_stale_removes_old_keeps_fresh() {
        let dir = std::env::temp_dir().join("mediagit-journal-sweep-test");
        let upload_dir = dir.join(".mediagit").join("upload");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&upload_dir).unwrap();

        let stale = upload_dir.join("stale-push.journal");
        let fresh = upload_dir.join("fresh-push.journal");
        std::fs::write(&stale, b"{}").unwrap();
        std::fs::write(&fresh, b"{}").unwrap();

        // Backdate the "stale" file's mtime by 8 days (stdlib only —
        // `File::set_modified` was stabilized in Rust 1.75).
        let old_time = std::time::SystemTime::now() - std::time::Duration::from_secs(8 * 24 * 3600);
        std::fs::OpenOptions::new()
            .write(true)
            .open(&stale)
            .unwrap()
            .set_modified(old_time)
            .unwrap();

        let swept = UploadJournal::sweep_stale(&dir, DEFAULT_JOURNAL_MAX_AGE);
        assert_eq!(swept, 1);
        assert!(!stale.exists(), "stale journal must be removed");
        assert!(fresh.exists(), "fresh journal must survive");

        // Idempotent: sweeping again finds nothing left to remove.
        let swept_again = UploadJournal::sweep_stale(&dir, DEFAULT_JOURNAL_MAX_AGE);
        assert_eq!(swept_again, 0);
        assert!(fresh.exists());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn sweep_stale_missing_upload_dir_is_noop() {
        let dir = std::env::temp_dir().join("mediagit-journal-sweep-missing-xyz");
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(UploadJournal::sweep_stale(&dir, DEFAULT_JOURNAL_MAX_AGE), 0);
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
