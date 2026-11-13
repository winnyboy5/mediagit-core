// SPDX-License-Identifier: BUSL-1.1
// Copyright (C) 2025-2026 Aswin Krishnamoorthy

//! Tracking refs whose commit was deliberately not downloaded (BUG-006).
//!
//! A clone downloads the default branch only. It still records the other
//! branches as `refs/remotes/<remote>/<branch>` and lists each one here, in
//! `.mediagit/lazy-refs`, as `<ref name> <commit id>` lines.
//!
//! `fsck` treats a missing commit as "not downloaded yet" only when the ref
//! is listed with the same commit id. Any other missing commit stays an
//! error, so a commit that was really lost never looks healthy.
//!
//! Whoever downloads a branch calls [`prune_present`], which drops entries
//! whose commit is now in the store. A stale entry could otherwise hide a
//! later loss of that commit.

use crate::{ObjectDatabase, Oid};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// ref name -> commit id that was not downloaded.
pub type LazyRefs = BTreeMap<String, Oid>;

fn file(repo_root: &Path) -> PathBuf {
    repo_root.join(".mediagit").join("lazy-refs")
}

/// Read the list. A missing file is an empty list. A line that does not parse
/// is skipped: fewer entries only makes `fsck` stricter.
pub fn load(repo_root: &Path) -> LazyRefs {
    let Ok(text) = std::fs::read_to_string(file(repo_root)) else {
        return LazyRefs::new();
    };
    text.lines()
        .filter_map(|line| {
            let (name, oid) = line.rsplit_once(' ')?;
            Some((name.to_string(), Oid::from_hex(oid.trim()).ok()?))
        })
        .collect()
}

/// Replace the list. An empty list removes the file.
pub fn save(repo_root: &Path, refs: &LazyRefs) -> std::io::Result<()> {
    let path = file(repo_root);
    if refs.is_empty() {
        return match std::fs::remove_file(&path) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
            _ => Ok(()),
        };
    }
    let mut text = String::new();
    for (name, oid) in refs {
        text.push_str(&format!("{name} {}\n", oid.to_hex()));
    }
    crate::atomic_write::write_atomic(&path, text.as_bytes())
}

/// Drop entries whose commit is now in the store. A failed probe keeps the
/// entry (it can only make `fsck` stricter, never looser).
pub async fn prune_present(repo_root: &Path, odb: &ObjectDatabase) -> std::io::Result<()> {
    let mut refs = load(repo_root);
    let before = refs.len();
    let mut present = Vec::new();
    for (name, oid) in &refs {
        if matches!(odb.exists(oid).await, Ok(true)) {
            present.push(name.clone());
        }
    }
    for name in present {
        refs.remove(&name);
    }
    if refs.len() == before {
        return Ok(());
    }
    save(repo_root, &refs)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_and_garbage_lines_are_skipped() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join(".mediagit")).unwrap();
        assert!(load(dir.path()).is_empty(), "missing file is empty");

        let oid = Oid::hash(b"x");
        let mut refs = LazyRefs::new();
        refs.insert("refs/remotes/origin/a b".to_string(), oid);
        save(dir.path(), &refs).unwrap();
        assert_eq!(load(dir.path()), refs);

        let path = file(dir.path());
        let mut text = std::fs::read_to_string(&path).unwrap();
        text.push_str("no-separator\nrefs/remotes/origin/z nothex\n");
        std::fs::write(&path, text).unwrap();
        assert_eq!(load(dir.path()), refs, "bad lines must be ignored");

        save(dir.path(), &LazyRefs::new()).unwrap();
        assert!(!path.exists(), "empty list removes the file");
    }
}
