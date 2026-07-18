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

//! Shared JSONL persistence helpers for auth stores (users, API keys).
//!
//! Mirrors the pattern used for server-side lock files
//! (`mediagit-server/src/locks.rs`): a `{"v":1}` header line followed by one
//! JSON record per line, written via tmp+rename full rewrite on every
//! mutation. Store sets are small (human-scale user/API-key counts), so a
//! full rewrite per mutation is fine — no append log needed.
//!
//! Unlike locks.rs, a corrupt/truncated file here is a **hard error** at
//! load rather than a silently-skipped line: auth state must never start
//! empty when a file exists but can't be read, since that would look like
//! "no users registered" instead of "storage is broken".

use serde::{de::DeserializeOwned, Serialize};
use std::path::Path;

use super::AuthError;

#[derive(serde::Serialize, serde::Deserialize)]
struct FileHeader {
    v: u32,
}

/// Current on-disk version for auth store JSONL headers (`{"v":N}`).
/// A header with a different version is a hard error at load — never
/// silently accepted (format-freeze policy).
const CURRENT_VERSION: u32 = 1;

/// Returns `true` unless `MEDIAGIT_AUTH_PERSIST` is explicitly set to `"0"`.
/// Lets tests (and operators) force pure in-memory behavior even when a
/// store directory is supplied.
pub(crate) fn persist_enabled() -> bool {
    std::env::var("MEDIAGIT_AUTH_PERSIST").as_deref() != Ok("0")
}

/// `cargo test` runs tests in the same process on multiple threads, and
/// `MEDIAGIT_AUTH_PERSIST` is a process-wide env var — a test in
/// `credentials.rs` that flips it to `"0"` would otherwise race a
/// concurrently-running test in `apikey.rs` that expects persistence ON.
/// Tests that mutate the var take the write lock for the duration of the
/// mutation; every other test that calls `load_or_new` takes the read lock
/// so it can't observe a torn value (readers run concurrently with each
/// other, just not with a writer).
#[cfg(test)]
pub(crate) static ENV_LOCK: std::sync::RwLock<()> = std::sync::RwLock::new(());

/// Load records from a JSONL file. A missing file means a fresh store
/// (first boot) and returns an empty `Vec`. An existing-but-corrupt file
/// (bad header, unparsable line) is a hard error.
pub(crate) fn load_jsonl<T: DeserializeOwned>(path: &Path) -> Result<Vec<T>, AuthError> {
    let content = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => {
            return Err(AuthError::Internal(anyhow::anyhow!(
                "failed to read auth store file {}: {}",
                path.display(),
                e
            )))
        }
    };

    let mut lines = content.lines();
    match lines.next() {
        Some(header_line) if !header_line.trim().is_empty() => {
            let header = serde_json::from_str::<FileHeader>(header_line).map_err(|e| {
                AuthError::Internal(anyhow::anyhow!(
                    "corrupt auth store file {} (bad header line): {}",
                    path.display(),
                    e
                ))
            })?;
            if header.v != CURRENT_VERSION {
                return Err(AuthError::Internal(anyhow::anyhow!(
                    "unsupported auth store version {} in {}, this build supports v{}",
                    header.v,
                    path.display(),
                    CURRENT_VERSION
                )));
            }
        }
        _ => {
            return Err(AuthError::Internal(anyhow::anyhow!(
                "corrupt auth store file {}: missing header line",
                path.display()
            )));
        }
    }

    let mut records = Vec::new();
    for (i, line) in lines.enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let record = serde_json::from_str::<T>(line).map_err(|e| {
            AuthError::Internal(anyhow::anyhow!(
                "corrupt auth store file {} at line {}: {}",
                path.display(),
                i + 2,
                e
            ))
        })?;
        records.push(record);
    }
    Ok(records)
}

/// Overwrite the JSONL file at `path` with `records` (tmp+rename full
/// rewrite), creating parent directories as needed.
pub(crate) async fn save_jsonl<T: Serialize>(path: &Path, records: &[T]) -> Result<(), AuthError> {
    let mut content = serde_json::to_string(&FileHeader { v: CURRENT_VERSION })
        .map_err(|e| AuthError::Internal(anyhow::anyhow!(e)))?;
    content.push('\n');
    for r in records {
        content.push_str(
            &serde_json::to_string(r).map_err(|e| AuthError::Internal(anyhow::anyhow!(e)))?,
        );
        content.push('\n');
    }
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|e| AuthError::Internal(anyhow::anyhow!(e)))?;
    }
    let tmp = path.with_extension("jsonl.tmp");
    tokio::fs::write(&tmp, content.as_bytes())
        .await
        .map_err(|e| AuthError::Internal(anyhow::anyhow!(e)))?;
    tokio::fs::rename(&tmp, path)
        .await
        .map_err(|e| AuthError::Internal(anyhow::anyhow!(e)))?;
    Ok(())
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[derive(serde::Serialize, serde::Deserialize, PartialEq, Debug)]
    struct Rec {
        id: u32,
    }

    #[test]
    fn load_jsonl_rejects_future_version_header() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("store.jsonl");
        std::fs::write(&path, "{\"v\":2}\n").unwrap();
        let result: Result<Vec<Rec>, AuthError> = load_jsonl(&path);
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn load_jsonl_roundtrips_current_version() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("store.jsonl");
        let records = vec![Rec { id: 1 }, Rec { id: 2 }];
        save_jsonl(&path, &records).await.unwrap();
        let loaded: Vec<Rec> = load_jsonl(&path).unwrap();
        assert_eq!(loaded, records);
    }
}
