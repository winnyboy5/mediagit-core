// SPDX-License-Identifier: BUSL-1.1
// Copyright (C) 2025-2026 Aswin Krishnamoorthy

//! Stat cache for tracked working-tree files (#94).
//!
//! `status` and the other dirty checks used to re-read and hash every
//! committed file on every run, so their cost grew with total bytes in the
//! working tree. This cache remembers `(size, mtime, file id) -> oid` for each
//! tracked path so an unchanged file is not re-hashed.
//!
//! Safety rules:
//! - The cache only answers "what is the content hash of this file", never
//!   "is this file clean". Callers still compare the oid against HEAD, so a
//!   stale cache cannot hide a change against a new HEAD.
//! - An entry is trusted only if size, mtime and file id all match AND the
//!   entry is not racy: it was recorded at least [`RACY_MARGIN_NS`] after the
//!   file's mtime. An edit that lands in the same timestamp tick as the record
//!   (or within the coarse 2 s granularity of FAT/exFAT and some network
//!   shares) therefore always forces a re-hash.
//! - A missing, corrupt or wrong-version cache file loads as empty, which is
//!   the old behaviour: hash everything.
//! - Identity is size + mtime + file id (inode, Unix) + change time (ctime on
//!   Unix, creation time on Windows), so an editor that saves via a temp file
//!   and rename is caught even if it preserves size and mtime.
//! - Callers pass `symlink_metadata`; symlinks and other non-regular files
//!   are never cached.
//! - `MEDIAGIT_STAT_CACHE=0` disables the cache entirely.

use std::collections::BTreeMap;
use std::fs::Metadata;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::Oid;
use crate::atomic_write::write_atomic;

const FORMAT_VERSION: u8 = 1;
const FILE_NAME: &str = "stat-cache";

/// An entry recorded less than this long after the file's mtime is racy and
/// never trusted. 2 s covers FAT/exFAT mtime granularity.
pub const RACY_MARGIN_NS: u128 = 2_000_000_000;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct StatEntry {
    size: u64,
    mtime_ns: u128,
    file_id: u64,
    change_ns: u128,
    oid: Oid,
    recorded_ns: u128,
}

/// Per-repository stat cache stored at `.mediagit/stat-cache`.
#[derive(Debug)]
pub struct StatCache {
    path: PathBuf,
    entries: BTreeMap<String, StatEntry>,
    enabled: bool,
    dirty: bool,
}

impl StatCache {
    /// Load the cache for the repository rooted at `repo_root`. Never fails:
    /// any read or decode problem yields an empty cache.
    pub fn load(repo_root: &Path) -> Self {
        let path = repo_root.join(".mediagit").join(FILE_NAME);
        let enabled = std::env::var("MEDIAGIT_STAT_CACHE").map_or(true, |v| v != "0");
        let entries = if enabled {
            std::fs::read(&path)
                .ok()
                .and_then(|bytes| decode(&bytes))
                .unwrap_or_default()
        } else {
            BTreeMap::new()
        };
        Self {
            path,
            entries,
            enabled,
            dirty: false,
        }
    }

    /// The cached oid for `rel_path`, only if the entry is still trustworthy
    /// for the file's current metadata.
    pub fn lookup(&self, rel_path: &str, meta: &Metadata) -> Option<Oid> {
        let entry = self.entries.get(&normalize(rel_path))?;
        let mtime_ns = mtime_ns(meta)?;
        let trusted = entry.size == meta.len()
            && entry.mtime_ns == mtime_ns
            && entry.file_id == file_id(meta)
            && entry.change_ns == change_ns(meta)
            && meta.is_file()
            && mtime_ns.saturating_add(RACY_MARGIN_NS) < entry.recorded_ns;
        trusted.then_some(entry.oid)
    }

    /// Record that the file at `rel_path` with metadata `meta` hashes to `oid`.
    pub fn record(&mut self, rel_path: &str, meta: &Metadata, oid: Oid) {
        if !self.enabled || !meta.is_file() {
            return;
        }
        let Some(mtime_ns) = mtime_ns(meta) else {
            return;
        };
        let entry = StatEntry {
            size: meta.len(),
            mtime_ns,
            file_id: file_id(meta),
            change_ns: change_ns(meta),
            oid,
            recorded_ns: now_ns(),
        };
        if self
            .entries
            .insert(normalize(rel_path), entry.clone())
            .as_ref()
            != Some(&entry)
        {
            self.dirty = true;
        }
    }

    /// Forget `rel_path` (file deleted or no longer tracked).
    pub fn remove(&mut self, rel_path: &str) {
        if self.entries.remove(&normalize(rel_path)).is_some() {
            self.dirty = true;
        }
    }

    /// Drop entries whose path `keep` rejects (no longer tracked).
    pub fn retain(&mut self, mut keep: impl FnMut(&str) -> bool) {
        let before = self.entries.len();
        self.entries.retain(|path, _| keep(path));
        self.dirty |= self.entries.len() != before;
    }

    /// Write the cache if anything changed. Callers may ignore the error:
    /// a cache that fails to save only costs a re-hash next time.
    pub fn save(&mut self) -> io::Result<()> {
        if !self.enabled || !self.dirty {
            return Ok(());
        }
        let mut bytes = vec![FORMAT_VERSION];
        bytes.extend(
            postcard::to_allocvec(&self.entries)
                .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?,
        );
        write_atomic(&self.path, &bytes)?;
        self.dirty = false;
        Ok(())
    }
}

fn decode(bytes: &[u8]) -> Option<BTreeMap<String, StatEntry>> {
    let (&version, body) = bytes.split_first()?;
    if version != FORMAT_VERSION {
        return None;
    }
    postcard::from_bytes(body).ok()
}

fn normalize(rel_path: &str) -> String {
    rel_path.replace('\\', "/")
}

fn mtime_ns(meta: &Metadata) -> Option<u128> {
    meta.modified()
        .ok()?
        .duration_since(UNIX_EPOCH)
        .ok()
        .map(|d| d.as_nanos())
}

fn now_ns() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos())
}

/// Status-change time on Unix (updated by every write, cannot be set by
/// tools that preserve mtime); creation time on Windows (new for a file
/// replaced via save-to-temp-and-rename). 0 if unavailable.
#[cfg(unix)]
fn change_ns(meta: &Metadata) -> u128 {
    use std::os::unix::fs::MetadataExt;
    (meta.ctime().max(0) as u128) * 1_000_000_000 + meta.ctime_nsec().max(0) as u128
}

#[cfg(not(unix))]
fn change_ns(meta: &Metadata) -> u128 {
    meta.created()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_nanos())
}

#[cfg(unix)]
fn file_id(meta: &Metadata) -> u64 {
    use std::os::unix::fs::MetadataExt;
    meta.ino()
}

// ponytail: Windows file index needs an unstable std API or a new dependency;
// size + 100 ns NTFS mtime + racy margin carry the check there. Add a file id
// if a replace-with-identical-size-and-mtime case ever shows up.
#[cfg(not(unix))]
fn file_id(_meta: &Metadata) -> u64 {
    0
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::time::Duration;

    fn set_mtime(path: &Path, t: SystemTime) {
        fs::File::options()
            .write(true)
            .open(path)
            .unwrap()
            .set_modified(t)
            .unwrap();
    }

    /// A file whose mtime is safely in the past, so a fresh record is trusted.
    fn old_file(dir: &Path, name: &str, content: &[u8]) -> PathBuf {
        let p = dir.join(name);
        fs::write(&p, content).unwrap();
        set_mtime(&p, SystemTime::now() - Duration::from_secs(60));
        p
    }

    fn repo() -> tempfile::TempDir {
        let d = tempfile::tempdir().unwrap();
        fs::create_dir(d.path().join(".mediagit")).unwrap();
        d
    }

    #[test]
    fn hit_after_record_and_reload() {
        let d = repo();
        let p = old_file(d.path(), "a.bin", b"hello");
        let oid = Oid::hash(b"hello");
        let mut c = StatCache::load(d.path());
        c.record("a.bin", &fs::metadata(&p).unwrap(), oid);
        c.save().unwrap();
        let c = StatCache::load(d.path());
        assert_eq!(c.lookup("a.bin", &fs::metadata(&p).unwrap()), Some(oid));
    }

    #[test]
    fn backslash_and_slash_paths_are_the_same_key() {
        let d = repo();
        fs::create_dir(d.path().join("dir")).unwrap();
        let p = old_file(d.path(), "dir/a.bin", b"x");
        let mut c = StatCache::load(d.path());
        c.record("dir\\a.bin", &fs::metadata(&p).unwrap(), Oid::hash(b"x"));
        assert!(c.lookup("dir/a.bin", &fs::metadata(&p).unwrap()).is_some());
    }

    #[test]
    fn size_change_misses() {
        let d = repo();
        let p = old_file(d.path(), "a.bin", b"hello");
        let mut c = StatCache::load(d.path());
        c.record("a.bin", &fs::metadata(&p).unwrap(), Oid::hash(b"hello"));
        let t = fs::metadata(&p).unwrap().modified().unwrap();
        fs::write(&p, b"hello world").unwrap();
        set_mtime(&p, t);
        assert_eq!(c.lookup("a.bin", &fs::metadata(&p).unwrap()), None);
    }

    #[test]
    fn mtime_change_with_same_size_misses() {
        let d = repo();
        let p = old_file(d.path(), "a.bin", b"hello");
        let mut c = StatCache::load(d.path());
        c.record("a.bin", &fs::metadata(&p).unwrap(), Oid::hash(b"hello"));
        fs::write(&p, b"HELLO").unwrap();
        set_mtime(&p, SystemTime::now() - Duration::from_secs(30));
        assert_eq!(c.lookup("a.bin", &fs::metadata(&p).unwrap()), None);
    }

    #[test]
    fn racy_entry_is_not_trusted() {
        // File written "now" and recorded immediately: an edit in the same
        // tick would keep size and mtime, so the entry must not be trusted.
        let d = repo();
        let p = d.path().join("a.bin");
        fs::write(&p, b"hello").unwrap();
        let mut c = StatCache::load(d.path());
        c.record("a.bin", &fs::metadata(&p).unwrap(), Oid::hash(b"hello"));
        assert_eq!(c.lookup("a.bin", &fs::metadata(&p).unwrap()), None);
    }

    #[test]
    fn corrupt_or_wrong_version_loads_empty() {
        let d = repo();
        let p = old_file(d.path(), "a.bin", b"hello");
        let file = d.path().join(".mediagit").join(FILE_NAME);
        for bad in [&b"\x01garbage"[..], &b"\x63"[..], &b""[..]] {
            fs::write(&file, bad).unwrap();
            let c = StatCache::load(d.path());
            assert_eq!(c.lookup("a.bin", &fs::metadata(&p).unwrap()), None);
        }
    }

    #[test]
    fn replaced_by_rename_with_same_size_and_mtime_misses() {
        // Editors and copy tools often write a temp file and rename it over
        // the original, sometimes preserving the old mtime. Size and mtime
        // then match; the change time must still expose the replacement.
        let d = repo();
        let p = old_file(d.path(), "a.bin", b"hello");
        let mut c = StatCache::load(d.path());
        c.record(
            "a.bin",
            &fs::symlink_metadata(&p).unwrap(),
            Oid::hash(b"hello"),
        );
        let t = fs::metadata(&p).unwrap().modified().unwrap();
        std::thread::sleep(Duration::from_millis(20));
        let tmp = d.path().join("a.tmp");
        fs::write(&tmp, b"HELLO").unwrap();
        set_mtime(&tmp, t);
        fs::rename(&tmp, &p).unwrap();
        assert_eq!(c.lookup("a.bin", &fs::symlink_metadata(&p).unwrap()), None);
    }

    #[test]
    fn retain_prunes_untracked_paths() {
        let d = repo();
        let p = old_file(d.path(), "a.bin", b"hello");
        let mut c = StatCache::load(d.path());
        c.record("a.bin", &fs::metadata(&p).unwrap(), Oid::hash(b"hello"));
        c.retain(|path| path != "a.bin");
        assert_eq!(c.lookup("a.bin", &fs::metadata(&p).unwrap()), None);
    }

    #[test]
    fn remove_forgets_entry() {
        let d = repo();
        let p = old_file(d.path(), "a.bin", b"hello");
        let mut c = StatCache::load(d.path());
        c.record("a.bin", &fs::metadata(&p).unwrap(), Oid::hash(b"hello"));
        c.remove("a.bin");
        assert_eq!(c.lookup("a.bin", &fs::metadata(&p).unwrap()), None);
    }
}
