// SPDX-License-Identifier: BUSL-1.1
// Copyright (C) 2025-2026 Aswin Krishnamoorthy

//! Crash-safe, collision-free file replacement (VC-3, VC-4).
//!
//! Two distinct bugs motivated this:
//!
//! - **Torn writes.** `Index::save`, the reflog rewriters and the upload
//!   journal all used a plain `fs::write`, which truncates first and then
//!   fills. A crash or `ENOSPC` mid-write left a half-written file, and
//!   `Index::load` has no recovery path — every subsequent command failed
//!   until the file was deleted by hand, discarding whatever was staged.
//!
//! - **Shared temp names.** The writers that *were* atomic derived their temp
//!   path deterministically from the target (`path.with_extension("tmp")`,
//!   `<name>.mgtmp`). Two processes replacing the same file therefore opened
//!   the *same* temp path: the second `File::create` truncated the first
//!   process's in-flight temp file, and whichever `rename` ran last won —
//!   silently discarding the other write.
//!
//! [`tmp_path_for`] makes the temp name unique per process, thread and call,
//! so concurrent writers can never share one. It stays in the target's own
//! directory so the final `rename` is same-filesystem and therefore atomic.

use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

/// Monotonic discriminator so two writes from one thread never collide.
static COUNTER: AtomicU64 = AtomicU64::new(0);

/// A temp path in the same directory as `path`, unique to this process,
/// thread and call.
///
/// Appends to the *full* file name rather than using `Path::with_extension`,
/// which replaces the extension and would map `a.psd` and `a.txt` onto the
/// same temp file.
pub fn tmp_path_for(path: &Path) -> io::Result<PathBuf> {
    let file_name = path.file_name().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("path has no file name: {}", path.display()),
        )
    })?;

    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let mut tmp = file_name.to_os_string();
    tmp.push(format!(
        ".{}.{:?}.{}.mgtmp",
        std::process::id(),
        std::thread::current().id(),
        n
    ));
    Ok(path.with_file_name(tmp))
}

/// Replace `path` with `data` atomically.
///
/// Writes to a unique temp file, fsyncs it so the bytes are durable before
/// they are visible, then renames over the target. A crash leaves either the
/// old file or the new one — never a truncated mix. On any failure the temp
/// file is removed, so a failed save never litters the repo.
pub fn write_atomic(path: &Path, data: &[u8]) -> io::Result<()> {
    use std::io::Write;

    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent)?;
    }

    let tmp = tmp_path_for(path)?;

    let write_result = (|| -> io::Result<()> {
        let mut file = std::fs::File::create(&tmp)?;
        file.write_all(data)?;
        file.sync_all()?;
        Ok(())
    })();

    if let Err(e) = write_result {
        let _ = std::fs::remove_file(&tmp);
        return Err(e);
    }

    if let Err(first) = std::fs::rename(&tmp, path) {
        // Windows can transiently refuse a rename when the destination is
        // held open by an AV scanner or a concurrent reader. One
        // remove-and-retry, then give up cleanly.
        let _ = std::fs::remove_file(path);
        if let Err(retry) = std::fs::rename(&tmp, path) {
            let _ = std::fs::remove_file(&tmp);
            return Err(io::Error::other(format!(
                "failed to rename {} -> {}: {first} (retry: {retry})",
                tmp.display(),
                path.display()
            )));
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn tmp_paths_are_unique_across_calls() {
        let target = Path::new("/tmp/repo/.mediagit/index");
        let mut seen = HashSet::new();
        for _ in 0..1000 {
            assert!(
                seen.insert(tmp_path_for(target).unwrap()),
                "tmp_path_for produced a duplicate — concurrent writers would collide"
            );
        }
    }

    #[test]
    fn tmp_path_stays_in_target_directory() {
        let target = Path::new("/tmp/repo/.mediagit/index");
        let tmp = tmp_path_for(target).unwrap();
        assert_eq!(
            tmp.parent(),
            target.parent(),
            "temp file must share the target's directory or the rename is not atomic"
        );
    }

    /// `with_extension` would map both of these onto one temp path.
    #[test]
    fn tmp_path_does_not_collide_on_shared_stem() {
        let a = tmp_path_for(Path::new("/tmp/a.psd")).unwrap();
        let b = tmp_path_for(Path::new("/tmp/a.txt")).unwrap();
        assert_ne!(a, b);
        assert!(a.file_name().unwrap().to_str().unwrap().contains("a.psd"));
        assert!(b.file_name().unwrap().to_str().unwrap().contains("a.txt"));
    }

    #[test]
    fn write_atomic_replaces_content_and_leaves_no_temp_files() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("index");

        write_atomic(&target, b"first").unwrap();
        assert_eq!(std::fs::read(&target).unwrap(), b"first");

        write_atomic(&target, b"second").unwrap();
        assert_eq!(std::fs::read(&target).unwrap(), b"second");

        let leftovers: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().contains("mgtmp"))
            .collect();
        assert!(
            leftovers.is_empty(),
            "temp files left behind: {leftovers:?}"
        );
    }

    #[test]
    fn write_atomic_creates_missing_parent_directories() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("nested").join("deep").join("index");
        write_atomic(&target, b"x").unwrap();
        assert_eq!(std::fs::read(&target).unwrap(), b"x");
    }

    /// The point of the whole module: a reader must never observe a partial
    /// file. Concurrent writers each publish a complete, valid document.
    #[test]
    fn concurrent_writers_never_produce_a_torn_file() {
        let dir = tempfile::tempdir().unwrap();
        let target = std::sync::Arc::new(dir.path().join("index"));

        let payloads: Vec<Vec<u8>> = (0..8).map(|i| vec![b'a' + i as u8; 64 * 1024]).collect();

        std::thread::scope(|s| {
            for p in &payloads {
                let target = std::sync::Arc::clone(&target);
                s.spawn(move || {
                    for _ in 0..20 {
                        write_atomic(&target, p).unwrap();
                    }
                });
            }
        });

        // Whatever landed last, it must be exactly one payload — not a blend.
        let final_bytes = std::fs::read(target.as_path()).unwrap();
        assert!(
            payloads.contains(&final_bytes),
            "file is a torn mix of concurrent writes (len {})",
            final_bytes.len()
        );
    }
}
