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

//! DC-7/D3: the process-global at-rest encryption key.
//!
//! [`SmartCompressor::with_key`](crate::SmartCompressor::with_key) already
//! carries the key that seals and opens objects. What it does not solve is
//! *delivery*: there are 20-odd `SmartCompressor::new()` call sites, and
//! threading a key through every one by hand is the exact shape of the "ODB
//! bypass" defect this codebase has shipped six-plus times — one path that
//! skipped the shared entry point. A missed WRITE site there would be an
//! unencrypted object; a missed READ site would be an unreadable repository.
//!
//! So the key is installed once, for the process, and every
//! `SmartCompressor::new()` picks it up automatically. Adding a new call site
//! cannot forget it, because there is nothing at the call site to remember.
//!
//! Two properties make that safe:
//!
//! * **Defaults to `None`.** With no key installed, every byte written is
//!   what it was before DC-7 existed — the §12 frozen format is untouched.
//! * **Set once.** A second, *differing* [`set_process_key`] is a hard error
//!   rather than a silent swap, because a swap mid-process would mean objects
//!   written under two different keys with nothing recording which is which.
//!
//! **This is a CLI/local-ODB mechanism only.** The server hosts many
//! repositories in one process, so a single process-wide key is meaningless
//! there and must not be used; per-repo server-side sealing is D4's problem.

use crate::{CompressionError, CompressionResult};
use mediagit_security::encryption::EncryptionKey;
use std::borrow::Cow;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// Installed at most once, at process startup, before any ODB exists.
///
/// The repository root travels *with* the key. A key is a per-repository
/// secret sitting in a process-wide slot, and without the root there is
/// nothing to check it against: `find_repo_root()` walks upward, so a command
/// run inside an encrypted repo that then opens a different repository would
/// seal the second repo's objects under the first repo's key — and the second
/// repo has no key file, so nothing could ever open them again. See
/// [`ensure_key_scope`].
static PROCESS_KEY: OnceLock<(PathBuf, EncryptionKey)> = OnceLock::new();

/// Refusal from [`set_process_key`] or [`ensure_key_scope`].
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ProcessKeyError {
    /// A *different* key was already installed. Deliberately not a silent
    /// overwrite: objects already written in this process used the first key.
    #[error(
        "an at-rest encryption key is already installed for this process and a \
         different one was supplied; the key must be set once, before any object \
         is read or written"
    )]
    AlreadySet,

    /// The process holds a key for one repository and is being asked to open
    /// another. Fatal rather than best-effort: the alternative is sealing the
    /// second repository under a key it has no record of.
    #[error(
        "this process holds the at-rest encryption key of the repository at {installed}, \
         but {requested} is a different repository. Objects written here would be \
         encrypted under a key that repository has no record of, and nothing could \
         open them again. Run this command from outside the encrypted repository."
    )]
    WrongRepository {
        /// Root the installed key belongs to.
        installed: PathBuf,
        /// Root that was about to be opened.
        requested: PathBuf,
    },
}

/// The key every [`SmartCompressor::new`](crate::SmartCompressor::new) will
/// adopt, or `None` when this process has no at-rest encryption.
pub fn process_key() -> Option<&'static EncryptionKey> {
    PROCESS_KEY.get().map(|(_, key)| key)
}

/// Install the process-wide at-rest key for the repository at `repo_root`.
/// Call once, at startup, **before** any ODB is constructed — an ODB built
/// earlier holds a compressor that already captured `None` and would happily
/// write plaintext into an encrypted repo.
///
/// Idempotent for the same repo and key (so a startup path that runs twice is
/// harmless); [`ProcessKeyError::AlreadySet`] for anything else.
pub fn set_process_key(repo_root: &Path, key: EncryptionKey) -> Result<(), ProcessKeyError> {
    let rejected = match PROCESS_KEY.set((normalize(repo_root), key)) {
        Ok(()) => return Ok(()),
        Err(rejected) => rejected,
    };
    // `EncryptionKey: PartialEq` is constant-time and does not hand out key
    // bytes — the comparison lives in mediagit-security for that reason.
    match PROCESS_KEY.get() {
        Some(current) if *current == rejected => Ok(()),
        _ => Err(ProcessKeyError::AlreadySet),
    }
}

/// Refuse to open a repository the installed key does not belong to.
///
/// Call at the point a repository's storage is built, before any I/O against
/// it. A no-op for the overwhelming majority of processes, which hold no key.
pub fn ensure_key_scope(repo_root: &Path) -> Result<(), ProcessKeyError> {
    let Some((installed, _)) = PROCESS_KEY.get() else {
        return Ok(());
    };
    let requested = normalize(repo_root);
    if *installed == requested {
        return Ok(());
    }
    Err(ProcessKeyError::WrongRepository {
        installed: installed.clone(),
        requested,
    })
}

/// One spelling per directory, so `.`, a relative path and a symlinked path
/// all compare equal. Falls back to the path as given when the filesystem
/// cannot resolve it — a comparison against the raw path is still better than
/// skipping the check.
fn normalize(root: &Path) -> PathBuf {
    std::fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf())
}

/// Seal `data` under the process key, or hand it back untouched when this
/// process has none.
///
/// For payloads that are stored *beside* the compression path rather than
/// through it — chunk manifests, pack-embedded deltas, chunks ingested
/// pre-compressed from a remote. Those never reach
/// [`SmartCompressor::compress_typed`](crate::SmartCompressor::compress_typed),
/// so without this they would land on disk in the clear inside an otherwise
/// encrypted repository.
///
/// Borrowing on the unkeyed path is deliberate: with no key the bytes written
/// are exactly the bytes passed in, with no copy and no envelope, which is
/// what keeps the §12 frozen format frozen.
pub fn seal_at_rest(data: &[u8]) -> CompressionResult<Cow<'_, [u8]>> {
    match process_key() {
        None => Ok(Cow::Borrowed(data)),
        Some(key) => mediagit_security::envelope::seal(key, data)
            .map(Cow::Owned)
            .map_err(|e| CompressionError::compression_failed(format!("seal at rest: {e}"))),
    }
}

/// Open `data` if it carries an envelope, otherwise hand it back untouched.
///
/// Tolerant in one direction only, matching
/// [`SmartCompressor`](crate::SmartCompressor): unsealed input passes through
/// (so a repo that adopts a key mid-life still reads what it wrote before),
/// but sealed input with no key is a hard error rather than ciphertext handed
/// on to a parser.
pub fn open_at_rest(data: &[u8]) -> CompressionResult<Cow<'_, [u8]>> {
    if !mediagit_security::envelope::is_sealed(data) {
        return Ok(Cow::Borrowed(data));
    }
    let Some(key) = process_key() else {
        return Err(CompressionError::decompression_failed(
            "data is encrypted (MGEN envelope) but no encryption key is configured \
             for this process",
        ));
    };
    mediagit_security::envelope::open(key, data)
        .map(Cow::Owned)
        .map_err(|e| CompressionError::decompression_failed(format!("open at rest: {e}")))
}
