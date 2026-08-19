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

//! DC-7/D2: where a repository's at-rest encryption key lives, and how it is
//! unlocked.
//!
//! # Shape
//!
//! Two keys, not one:
//!
//! * a **repo key** — 32 random bytes, generated once by `mediagit key init`,
//!   the key every object in this repository is sealed under;
//! * a **master key** — never stored beside the repo, used only to wrap the
//!   repo key.
//!
//! The indirection buys the thing a single key cannot: the master key can be
//! held somewhere the repository directory is not (an OS keychain, a keyfile on
//! a removable volume), so copying `.mediagit/` copies nothing usable.
//!
//! # On-disk format — `.mediagit/encryption-key`
//!
//! TOML, and deliberately not part of `config.toml`: config is edited, diffed,
//! and pasted into bug reports, and key material has no business travelling
//! with it.
//!
//! ```toml
//! version     = 1
//! master      = "keyfile" | "keychain" | "passphrase"   # how it was wrapped
//! salt        = "<32 hex chars>"                        # passphrase master only
//! wrapped     = "<hex>"     # MGEN envelope over the 32-byte repo key
//! recovery    = "<hex>"     # optional: the same repo key, under the recovery code
//! fingerprint = "<64 hex chars>"   # BLAKE3(repo key), checked after every unwrap
//! ```
//!
//! `wrapped` is an ordinary [`mediagit_security::envelope`] envelope, so a
//! wrong master key fails GCM authentication rather than yielding 32 bytes of
//! garbage that would then be used to "decrypt" the whole repository. It is
//! sealed under a subkey *derived* from the master key
//! ([`EncryptionKey::derive_subkey`]), not the master key itself — see that
//! method's doc for why.
//!
//! `fingerprint` is a second, independent check on top of GCM: AES-GCM is not
//! key-committing, so on its own it does not guarantee a ciphertext can only
//! authenticate under one key. Recording `BLAKE3(repo key)` and checking it
//! after every unwrap closes that gap regardless of which slot produced the
//! key. `#[serde(default)]`, so key files written before this field existed
//! still parse — they simply have nothing to check against.
//!
//! # The recovery slot
//!
//! `recovery` is a *second* envelope over the *same* repo key, wrapped under a
//! random 32-byte secret shown to the user once at `key init` — the recovery
//! code. Two slots, one key: losing the master key stops being total data loss,
//! because the recovery code re-derives the repo key and `key recover` re-wraps
//! it under whatever master source the machine has now.
//!
//! The code is already 256 bits of OS randomness, so it is used as the wrapping
//! key directly — Argon2 over it would be stretching something that has nothing
//! left to stretch.
//!
//! The field is `#[serde(default)]` and stays at `version = 1`: a key file
//! written before recovery slots existed parses and unlocks exactly as it did,
//! it simply has no second slot.
//!
//! **The absence of this file means the repo is not encrypted, and that is
//! silent.** No warning, no prompt — encryption is opt-in and every existing
//! repository must behave exactly as it did before DC-7.
//!
//! # Master-key precedence
//!
//! `MEDIAGIT_ENCRYPTION_KEYFILE` > OS keychain > interactive passphrase.
//!
//! Candidates are tried in that order and each attempt is authenticated, so a
//! source that is present but wrong simply falls through to the next. The
//! passphrase prompt is only ever reached when the file records a `salt` —
//! i.e. when the repo really was passphrase-wrapped — so a keychain-wrapped
//! repo on a machine with a locked keychain fails with a message instead of
//! prompting for a passphrase that never existed.

use anyhow::{Context, Result, anyhow, bail};
use mediagit_security::encryption::EncryptionKey;
use mediagit_security::kdf::{Argon2Params, Salt, derive_key};
use mediagit_security::{SecretString, Zeroizing, envelope};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Environment variable naming a file that holds the master key.
pub const KEYFILE_ENV: &str = "MEDIAGIT_ENCRYPTION_KEYFILE";

/// Keychain account (under [`crate::repo::KEYRING_SERVICE`]) holding the
/// master key.
///
/// Machine-wide rather than per-repo on purpose: each repository has its own
/// random repo key, so one master key unlocking several of them leaks nothing
/// between them, and one keychain entry is one thing for a user to protect.
const KEYCHAIN_ACCOUNT: &str = "encryption-master";

/// File name under `.mediagit/`.
const KEY_FILE: &str = "encryption-key";

/// Repo/master key length — AES-256.
const KEY_LEN: usize = 32;

/// [`EncryptionKey::derive_subkey`] context for wrapping the repo key. Fixed
/// and never reused for anything else — that's the entire point of a
/// domain-separation context.
const WRAP_CONTEXT: &str = "mediagit/wrap/v2";

/// Which master-key source unlocks (or wrapped) a repo key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MasterSource {
    /// A file named by [`KEYFILE_ENV`].
    Keyfile,
    /// The OS keychain.
    Keychain,
    /// An interactive passphrase, stretched with Argon2id.
    Passphrase,
}

impl MasterSource {
    /// Stable spelling for the `master` field and for `key status`.
    fn as_str(self) -> &'static str {
        match self {
            Self::Keyfile => "keyfile",
            Self::Keychain => "keychain",
            Self::Passphrase => "passphrase",
        }
    }

    /// Human-facing description — where the user's key actually is.
    pub fn describe(self) -> &'static str {
        match self {
            Self::Keyfile => "key file named by MEDIAGIT_ENCRYPTION_KEYFILE",
            Self::Keychain => "OS keychain",
            Self::Passphrase => "interactive passphrase (Argon2id)",
        }
    }
}

/// The wrapped repo key as it sits on disk. Contains no usable key material
/// on its own; `wrapped` is ciphertext under a master key stored elsewhere.
#[derive(Debug, Serialize, Deserialize)]
struct StoredKey {
    version: u32,
    master: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    salt: Option<String>,
    wrapped: String,
    /// The second slot. `default` so key files written before recovery codes
    /// existed still parse — they simply have no slot to fall back on.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    recovery: Option<String>,
    /// `BLAKE3(repo key)`, hex-encoded. Checked by [`verify_fingerprint`]
    /// after every unwrap — see the module doc's "key commitment" note.
    /// `default` so key files written before this field existed still parse;
    /// they simply have nothing to check the unwrapped key against.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    fingerprint: Option<String>,
}

/// Path of the wrapped-key file for `repo_root`.
pub fn key_file_path(repo_root: &Path) -> PathBuf {
    repo_root.join(".mediagit").join(KEY_FILE)
}

/// Does this repository have at-rest encryption configured?
///
/// A pure filesystem check with no unlocking and no prompting, so it is safe
/// on any path that only needs to *know* (`key status`, the push interlock).
pub fn has_key(repo_root: &Path) -> bool {
    key_file_path(repo_root).exists()
}

/// The master source that would be used to unlock this repo, if any is
/// currently available. `None` means the repo is encrypted but nothing on this
/// machine can open it right now.
pub fn available_master_source(repo_root: &Path) -> Result<Option<MasterSource>> {
    let stored = read_stored(repo_root)?;
    Ok(candidates(&stored).first().copied())
}

/// Unwrap this repository's key, or `None` if it has none.
///
/// Silent for an unencrypted repo — that is the overwhelmingly common case and
/// must cost nothing and say nothing.
pub fn load_repo_key(repo_root: &Path) -> Result<Option<EncryptionKey>> {
    if !has_key(repo_root) {
        return Ok(None);
    }
    let stored = read_stored(repo_root)?;
    let (bytes, _master) = unwrap_repo_key_bytes(&stored)?;
    let key = EncryptionKey::from_bytes(bytes.to_vec())
        .map_err(|e| anyhow!("encryption-key: unwrapped key is unusable: {e}"))?;
    Ok(Some(key))
}

/// This repository's key as raw bytes, for escrowing it with a remote.
///
/// Separate from [`load_repo_key`] because `EncryptionKey` deliberately offers
/// no way back out to its contents — and the bytes are exactly what the escrow
/// endpoint needs. `Zeroizing`, so a push that fails partway does not leave
/// the repository's key in a dropped buffer.
pub fn load_repo_key_bytes(repo_root: &Path) -> Result<Option<Zeroizing<Vec<u8>>>> {
    if !has_key(repo_root) {
        return Ok(None);
    }
    let stored = read_stored(repo_root)?;
    let (bytes, _master) = unwrap_repo_key_bytes(&stored)?;
    Ok(Some(bytes))
}

/// Refuse, before any object moves, when this repository's encryption state
/// and the remote's cannot both be true.
///
/// Every transfer command needs this question answered and each one used to
/// answer it differently. `push` asked only when the LOCAL repository held a
/// key, so an unencrypted clone pushing to a keyed remote skipped the check
/// entirely and uploaded plaintext into a repository the server considered
/// encrypted -- silently, because reads pass unsealed bytes straight through
/// (`process_key::open_at_rest`). `fetch`, `pull` and `download` never asked at
/// all, so a sealed object arrived and failed deep in the compressor talking
/// about MGEN envelopes. One function now, so the answer cannot drift apart
/// again.
///
/// Deliberately does NOT adopt a key the way [`adopt_repo_key`] does for
/// `clone`. Clone owns a directory it just created -- the same empty-repository
/// precondition `key init` enforces. Every other command runs against a
/// repository that may already hold plaintext objects, and keying that one
/// would manufacture exactly the half-sealed state the empty-repo refusal
/// exists to prevent. It would also fail outright on the second call, because
/// `adopt_repo_key` refuses once a key file is present -- so a repeat `pull`
/// on an encrypted repository would break, which is the most ordinary
/// workflow there is.
///
/// Returns this repository's key when it holds one the remote does not; only
/// `push` acts on that, by escrowing it.
pub async fn verify_remote_key_compatible(
    repo_root: &Path,
    client: &mediagit_protocol::ProtocolClient,
) -> Result<Option<Zeroizing<Vec<u8>>>> {
    use mediagit_protocol::client::escrow::EscrowedKey;

    let local = load_repo_key_bytes(repo_root)?;

    // A repository that holds a key must know the remote's state: getting it
    // wrong writes objects nothing can read back. Without one the check is
    // advisory, and a transport failure must not become an encryption-shaped
    // error -- before this existed, an unencrypted transfer touched the network
    // for the first time when it moved an object, and that is where the useful
    // message lives. A remote we cannot reach cannot be transferred with
    // either way, so nothing is lost by carrying on.
    let remote = match client.get_encryption_key().await {
        Ok(remote) => remote,
        Err(e) if local.is_none() => {
            tracing::debug!("encryption-key pre-flight check skipped: {e:#}");
            return Ok(None);
        }
        Err(e) => return Err(e),
    };

    match (local, remote) {
        (Some(local), EscrowedKey::Present(remote)) => {
            if local.as_slice() != remote.as_slice() {
                bail!(
                    "the remote holds a DIFFERENT encryption key for this repository. \
                        Its objects are sealed under that key, so mixing the two would \
                        produce a repository nothing can read end to end. Nothing was \
                        transferred."
                );
            }
            Ok(None)
        }
        // Absent, or a remote with no escrow route at all. The caller decides:
        // `push` offers the key, read paths carry on.
        (Some(local), _) => Ok(Some(local)),
        (None, EscrowedKey::Present(_)) => bail!(
            "the remote holds an encryption key for this repository, but this one has \
                none. Its objects are sealed, so nothing here could read them, and pushing \
                from here would mix plaintext into a repository that reports itself as \
                encrypted. Clone the repository again to receive the key. Nothing was \
                transferred."
        ),
        (None, _) => Ok(None),
    }
}

/// Adopt `key` as this repository's key, wrapped under a local master.
///
/// For clone: the repository key comes from the remote, and this machine has
/// to be able to open it again tomorrow without asking anyone. Master source
/// follows the same precedence as `key init` — keyfile, then keychain, then a
/// keychain entry provisioned on the spot — so cloning never prompts.
///
/// Refuses if the repository already has a key file. Overwriting one would
/// orphan whatever is already sealed under it.
pub fn adopt_repo_key(repo_root: &Path, key: &[u8]) -> Result<MasterSource> {
    if has_key(repo_root) {
        bail!(
            "{} already exists; refusing to overwrite this repository's encryption key",
            key_file_path(repo_root).display()
        );
    }
    if key.len() != KEY_LEN {
        bail!(
            "an encryption key must be {KEY_LEN} bytes, got {}",
            key.len()
        );
    }

    let (master, source, salt) = provision_master()?;
    let wrap_key = master
        .derive_subkey(WRAP_CONTEXT)
        .map_err(|e| anyhow!("deriving the wrapping subkey: {e}"))?;

    let stored = StoredKey {
        version: 1,
        master: source.as_str().to_string(),
        salt,
        wrapped: hex::encode(
            envelope::seal(&wrap_key, key).map_err(|e| anyhow!("wrapping the repo key: {e}"))?,
        ),
        // No recovery slot. A clone's recovery path is the remote it came
        // from, which still holds this key; minting a second code here would
        // be one more secret to lose for no gain the original does not
        // already cover.
        recovery: None,
        fingerprint: Some(blake3::hash(key).to_hex().to_string()),
    };
    write_new(&key_file_path(repo_root), &stored)?;
    Ok(source)
}

/// Open the `wrapped` slot with whichever master source this machine can
/// supply, and hand back the raw repo key.
///
/// Separate from [`load_repo_key`] because [`rotate_master_key`] needs the
/// bytes themselves to re-seal them, and `EncryptionKey` deliberately offers no
/// way back out to its contents.
fn unwrap_repo_key_bytes(stored: &StoredKey) -> Result<(Zeroizing<Vec<u8>>, EncryptionKey)> {
    let wrapped =
        hex::decode(stored.wrapped.trim()).context("encryption-key: `wrapped` is not valid hex")?;

    let candidates = candidates(stored);
    if candidates.is_empty() {
        bail!(
            "this repository is encrypted, but no master key is available on this machine \
             (it was wrapped with the {}). Set {KEYFILE_ENV}, or restore the keychain entry, \
             then retry.",
            stored.master
        );
    }

    let mut tried = Vec::new();
    for source in candidates {
        let Some(master) = master_key(source, stored)? else {
            // A candidate that turned out to be unusable is still a candidate
            // that was *tried*: without this the final message can name no
            // source at all, which reads like the code never looked.
            tried.push(source.describe());
            continue;
        };
        let wrap_key = master
            .derive_subkey(WRAP_CONTEXT)
            .map_err(|e| anyhow!("deriving the wrapping subkey: {e}"))?;
        // Authenticated: a wrong master key is a GCM failure here, never 32
        // bytes of garbage that would go on to "decrypt" the whole repository.
        if let Ok(bytes) = envelope::open(&wrap_key, &wrapped) {
            // Zeroized on every exit from here, not just the happy path: the
            // fingerprint check below can bail! before the caller ever takes
            // ownership of these bytes.
            let bytes = Zeroizing::new(bytes);
            verify_fingerprint(stored, &bytes)?;
            return Ok((bytes, master));
        }
        tried.push(source.describe());
    }

    bail!(
        "could not unlock this repository's encryption key; tried: {}. \
         The master key is wrong or has been replaced — MediaGit cannot read the \
         repository's objects without it.",
        tried.join(", ")
    )
}

/// The repository whose key this process will unlock when it first needs one.
///
/// Recorded at startup, used later: `find_repo_root()` answers from the
/// directory the user ran the command in, and by the time
/// [`install_armed_key`] runs a command may have *created* a nested repository
/// — resolving the root there would silently pick that one instead.
static ARMED_ROOT: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();

/// Note the repository to unlock, without unlocking it. Cheap and silent.
pub fn arm_process_key(repo_root: PathBuf) {
    let _ = ARMED_ROOT.set(repo_root);
}

/// Unlock the armed repository's key and install it process-wide.
///
/// Called from `repo::create_storage_backend`, which is where every local ODB
/// gets its storage — and therefore the last moment before any
/// `SmartCompressor::new()` captures the key. Deferred to here rather than run
/// at startup because unlocking can cost an OS-keychain hit or a passphrase
/// prompt, and `config get`, `remote -v` and `auth login` touch no object at
/// all; they must not be made to pay for a key they never use.
pub fn install_armed_key() -> Result<()> {
    let Some(root) = ARMED_ROOT.get() else {
        return Ok(());
    };
    // Already installed: the key is set once per process, and a second unlock
    // here would re-prompt on every `create_storage_backend` in a command that
    // builds storage more than once.
    if mediagit_compression::process_key().is_some() || !has_key(root) {
        return Ok(());
    }
    if let Some(key) = load_repo_key(root)? {
        mediagit_compression::set_process_key(root, key)
            .context("installing this repository's at-rest encryption key")?;
    }
    Ok(())
}

/// Does this repository have a recovery slot? Reported by `key status` so a
/// user can tell, before they need it, whether they have a second way in.
pub fn has_recovery_slot(repo_root: &Path) -> Result<bool> {
    Ok(read_stored(repo_root)?.recovery.is_some())
}

/// What [`init_repo_key`] produced. The recovery code exists in exactly one
/// place after this returns — the caller's terminal.
pub struct InitOutcome {
    /// Which master source wrapped the repo key.
    pub source: MasterSource,
    /// The one-time recovery code, for display and nowhere else. Wiped when
    /// this struct drops: it is a transcribable secret that unlocks every
    /// object in the repository, and it has no business outliving the print.
    pub recovery_code: Zeroizing<String>,
}

/// Directories under `.mediagit` that hold object data. If any of them holds a
/// file, the repository has content that a later `key init` could not have
/// sealed.
const OBJECT_DIRS: [&str; 4] = ["objects", "chunks", "packs", "chunk-deltas"];

/// Bookkeeping files that live among the object directories without being
/// objects. `mediagit init` writes `objects/<namespace>/LAYOUT` on a
/// completely empty repository, so counting it would make every repository
/// look non-empty and this gate would refuse everything.
const NON_OBJECT_FILES: [&str; 1] = ["LAYOUT"];

/// Does this repository already hold objects?
///
/// A shallow existence walk, not a count: the answer is only ever used as a
/// yes/no gate, and a repository with a large ODB is exactly the case where
/// counting would be slowest and least useful.
fn repo_has_objects(repo_root: &Path) -> bool {
    fn any_object(dir: &Path) -> bool {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return false;
        };
        entries.flatten().any(|e| match e.file_type() {
            Ok(t) if t.is_dir() => any_object(&e.path()),
            Ok(t) if t.is_file() => !NON_OBJECT_FILES
                .iter()
                .any(|n| e.file_name().to_str() == Some(n)),
            _ => false,
        })
    }
    let base = repo_root.join(".mediagit");
    OBJECT_DIRS.iter().any(|d| any_object(&base.join(d)))
}

/// Generate this repository's key, wrap it under both slots, and store it.
///
/// Refuses if a key already exists: overwriting one orphans every object
/// already sealed under it, and nothing in the repository records which key an
/// object used, so the damage would be silent and total.
///
/// Refuses, too, if the repository already holds objects. Encryption is
/// enabled at creation or not at all: sealing what is already there means
/// rewriting every object and every pack, and the guarantee people read into
/// the word "encrypted" is not one a half-converted repository can make.
/// Lifting this needs the re-seal pass, which is deferred.
pub fn init_repo_key(repo_root: &Path) -> Result<InitOutcome> {
    let path = key_file_path(repo_root);
    if path.exists() {
        bail!(
            "this repository already has an encryption key ({}). Overwriting it would \
             make every object already written unreadable, so `key init` will not do it.",
            path.display()
        );
    }
    if repo_has_objects(repo_root) {
        bail!(
            "this repository already contains objects, and at-rest encryption can only \
             be enabled on an empty one.\n\
             \n\
             Everything already committed here was written in the clear. Enabling a key \
             now would encrypt only what comes next, leaving a repository that reports \
             itself as encrypted while most of its contents are not. Encrypting an \
             existing repository is not supported yet.\n\
             \n\
             To get an encrypted repository: run `mediagit key init` in a fresh one, \
             then add your files. Nothing was changed here."
        );
    }

    let (master, source, salt) = provision_master()?;
    let wrap_key = master
        .derive_subkey(WRAP_CONTEXT)
        .map_err(|e| anyhow!("deriving the wrapping subkey: {e}"))?;

    let repo_key = random_key_bytes().context("generating the repository key")?;
    let recovery_secret = random_key_bytes().context("generating the recovery code")?;
    let recovery_key = EncryptionKey::from_bytes(recovery_secret.to_vec())
        .map_err(|e| anyhow!("recovery code is unusable: {e}"))?;
    let recovery_wrap_key = recovery_key
        .derive_subkey(WRAP_CONTEXT)
        .map_err(|e| anyhow!("deriving the recovery wrapping subkey: {e}"))?;

    let fingerprint = blake3::hash(&repo_key[..]).to_hex().to_string();

    let stored = StoredKey {
        version: 1,
        master: source.as_str().to_string(),
        salt,
        wrapped: hex::encode(
            envelope::seal(&wrap_key, &repo_key[..])
                .map_err(|e| anyhow!("wrapping the repo key: {e}"))?,
        ),
        // Same key, second slot. Sealed independently, so neither slot's
        // ciphertext tells you anything about the other's wrapping key.
        recovery: Some(hex::encode(
            envelope::seal(&recovery_wrap_key, &repo_key[..])
                .map_err(|e| anyhow!("wrapping the recovery slot: {e}"))?,
        )),
        fingerprint: Some(fingerprint),
    };
    write_new(&path, &stored)?;

    Ok(InitOutcome {
        source,
        recovery_code: encode_recovery_code(&recovery_secret),
    })
}

/// Unlock the repo key with `code` and re-wrap it under whatever master source
/// this machine has now, rewriting the key file.
///
/// The recovery slot itself is carried over untouched: the user still holds
/// that code, and silently invalidating it would turn one recovery into the
/// last one they get.
pub fn recover_repo_key(repo_root: &Path, code: &str) -> Result<MasterSource> {
    let mut stored = read_stored(repo_root)?;
    let Some(recovery_hex) = stored.recovery.as_deref() else {
        bail!(
            "this repository has no recovery slot ({}). It was created before recovery \
             codes existed, so the master key is the only way in.",
            key_file_path(repo_root).display()
        );
    };
    let recovery_blob =
        hex::decode(recovery_hex.trim()).context("encryption-key: `recovery` is not valid hex")?;

    let recovery_key = decode_recovery_code(code)?;
    let recovery_wrap_key = recovery_key
        .derive_subkey(WRAP_CONTEXT)
        .map_err(|e| anyhow!("deriving the recovery wrapping subkey: {e}"))?;
    // Fails closed on a mistyped code: GCM authentication rejects it rather
    // than handing back 32 bytes that would then "decrypt" every object.
    let repo_key = envelope::open(&recovery_wrap_key, &recovery_blob).map_err(|_| {
        anyhow!(
            "that recovery code does not open this repository. Check it for transcription \
             errors — nothing has been changed."
        )
    })?;
    // Zeroized on every exit from here: `repo_key` is re-sealed by reference
    // below, not consumed, so nothing else would wipe this copy.
    let repo_key = Zeroizing::new(repo_key);
    verify_fingerprint(&stored, &repo_key)?;

    let (master, source, salt) = provision_master()?;
    rewrap_under_new_master(repo_root, &mut stored, &repo_key, master, source, salt)
}

/// Re-wrap the repository's existing key under a newly provisioned master,
/// rewriting the key file.
///
/// The repo key itself is unchanged, so every object stays readable and
/// nothing needs rewriting — this swaps only the lock on the key, not the key.
/// Changing the repo key would mean re-encrypting the entire repository, which
/// is a different and much larger operation.
///
/// Use when the master key is compromised or simply inconvenient: a stolen
/// laptop, a passphrase to change, a move from the OS keychain to a keyfile.
pub fn rotate_master_key(repo_root: &Path, new_keyfile: Option<&Path>) -> Result<MasterSource> {
    if !has_key(repo_root) {
        bail!(
            "this repository does not have at-rest encryption enabled, so there is no \
             master key to rotate. Run `mediagit key init` in an empty repository to \
             enable it."
        );
    }
    let mut stored = read_stored(repo_root)?;
    // Unwrapping under the *current* master is what authorises the rotation:
    // someone who cannot open the key file has no business re-locking it.
    let (repo_key, old_master) = unwrap_repo_key_bytes(&stored)?;

    let (new_master, source, salt) = match new_keyfile {
        Some(path) => (
            keyfile_master_at(path)?,
            MasterSource::Keyfile,
            // A keyfile master is used as-is, so there is no salt to record.
            // Leaving a stale one behind would describe the wrong derivation.
            None,
        ),
        None => provision_master()?,
    };

    // Both the unwrap above and `provision_master` consult the same sources, so
    // without this a rotation that named no new destination would cheerfully
    // re-wrap under the master it started with and report success. Someone
    // rotating because their master leaked would walk away believing they had
    // fixed it.
    if new_master == old_master {
        bail!(
            "that would re-wrap the repository under the master key it already uses, \
             which changes nothing.\n\
             \n\
             Point `--new-keyfile` at a different key file, or change the source the \
             new master comes from ({KEYFILE_ENV}, the OS keychain, or a passphrase). \
             Nothing was changed."
        );
    }

    rewrap_under_new_master(repo_root, &mut stored, &repo_key, new_master, source, salt)
}

/// Provision a fresh master, re-seal `repo_key` under it, and write the key
/// file. Shared by [`recover_repo_key`] and [`rotate_master_key`], which differ
/// only in how they get the repo key back — the recovery slot versus the
/// current master.
///
/// The recovery slot is carried over untouched. It wraps the same repo key
/// under a secret the user wrote down, so a new master does not invalidate it,
/// and silently dropping it here would turn one recovery into the last one they
/// get.
fn rewrap_under_new_master(
    repo_root: &Path,
    stored: &mut StoredKey,
    repo_key: &[u8],
    master: EncryptionKey,
    source: MasterSource,
    salt: Option<String>,
) -> Result<MasterSource> {
    let wrap_key = master
        .derive_subkey(WRAP_CONTEXT)
        .map_err(|e| anyhow!("deriving the wrapping subkey: {e}"))?;
    stored.master = source.as_str().to_string();
    stored.salt = salt;
    stored.wrapped = hex::encode(
        envelope::seal(&wrap_key, repo_key)
            .map_err(|e| anyhow!("re-wrapping the repo key: {e}"))?,
    );
    write_over(&key_file_path(repo_root), stored)?;
    Ok(source)
}

/// Verify the just-unwrapped repo key against the recorded fingerprint, if
/// any. See the module doc's "key commitment" note for why this exists
/// alongside GCM authentication rather than instead of it. Older key files
/// have no fingerprint (`#[serde(default)]`) and skip the check.
fn verify_fingerprint(stored: &StoredKey, repo_key: &[u8]) -> Result<()> {
    let Some(expected) = stored.fingerprint.as_deref() else {
        return Ok(());
    };
    let actual = blake3::hash(repo_key).to_hex();
    if actual.as_str() != expected {
        bail!(
            "encryption-key: the unwrapped repo key does not match its recorded fingerprint \
             — the key file may be corrupted or tampered with"
        );
    }
    Ok(())
}

/// 32 bytes from the OS RNG, wiped when the caller drops them. Every caller
/// copies these bytes into an [`EncryptionKey`] or a code shown once, so the
/// buffer itself is the copy nothing else would ever clear.
fn random_key_bytes() -> Result<Zeroizing<[u8; KEY_LEN]>> {
    let mut bytes = Zeroizing::new([0u8; KEY_LEN]);
    if getrandom::fill(&mut *bytes).is_err() {
        bail!("OS random number generator unavailable");
    }
    Ok(bytes)
}

/// Render 32 bytes as a code a human can read off a screen and type back:
/// lowercase hex in eight dash-separated groups of eight.
///
/// ponytail: hex, not base32/bip39. It has no case to get wrong and no
/// alphabet to look up, and [`decode_recovery_code`] strips the dashes and
/// whitespace back out, so a code retyped with different grouping — or none —
/// still works. Upgrade path: a checksummed alphabet (Crockford base32) if
/// transcription errors ever show up in support traffic; today a wrong code is
/// already caught by GCM, it just cannot say *which* character was wrong.
fn encode_recovery_code(bytes: &[u8; KEY_LEN]) -> Zeroizing<String> {
    let hexed = Zeroizing::new(hex::encode(bytes));
    // Built in place, at exact capacity: an intermediate `Vec<String>` of
    // groups (or a realloc) would leave copies of the code on the heap that
    // nothing wipes.
    let mut out = Zeroizing::new(String::with_capacity(KEY_LEN * 2 + 7));
    for (i, c) in hexed.chars().enumerate() {
        if i > 0 && i.is_multiple_of(8) {
            out.push('-');
        }
        out.push(c);
    }
    out
}

/// Parse a code back into its key, tolerating the dashes, spaces, line breaks
/// and stray case a transcription picks up.
fn decode_recovery_code(code: &str) -> Result<EncryptionKey> {
    let cleaned = Zeroizing::new(
        code.chars()
            .filter(|c| !c.is_whitespace() && *c != '-')
            .flat_map(char::to_lowercase)
            .collect::<String>(),
    );
    let bytes =
        Zeroizing::new(hex::decode(&*cleaned).map_err(|_| {
            anyhow!("that does not look like a recovery code (expected hex digits)")
        })?);
    if bytes.len() != KEY_LEN {
        bail!(
            "a recovery code is {} hex digits; got {}",
            KEY_LEN * 2,
            cleaned.len()
        );
    }
    EncryptionKey::from_bytes(bytes.to_vec()).map_err(|e| anyhow!("recovery code is unusable: {e}"))
}

// ---------------------------------------------------------------------------
// internals
// ---------------------------------------------------------------------------

fn read_stored(repo_root: &Path) -> Result<StoredKey> {
    let path = key_file_path(repo_root);
    let text =
        std::fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
    let stored: StoredKey =
        toml::from_str(&text).with_context(|| format!("parsing {}", path.display()))?;
    if stored.version != 1 {
        bail!(
            "{} was written by a newer MediaGit (format version {}); upgrade to read it",
            path.display(),
            stored.version
        );
    }
    Ok(stored)
}

/// Master sources worth trying for `stored`, in precedence order, filtered to
/// those that could actually produce a key on this machine.
fn candidates(stored: &StoredKey) -> Vec<MasterSource> {
    let mut out = Vec::new();
    if std::env::var_os(KEYFILE_ENV).is_some() {
        out.push(MasterSource::Keyfile);
    }
    if keychain_master().is_some() {
        out.push(MasterSource::Keychain);
    }
    // Only when the repo really was passphrase-wrapped: prompting for a
    // passphrase that was never set is a dead end dressed up as a question.
    if stored.salt.is_some() {
        out.push(MasterSource::Passphrase);
    }
    out
}

/// Produce the master key for `source`, or `None` if it turns out to be
/// unavailable after all (a keychain entry that vanished between the
/// availability check and here).
fn master_key(source: MasterSource, stored: &StoredKey) -> Result<Option<EncryptionKey>> {
    match source {
        // A keyfile that cannot be read or parsed is exactly "a source that is
        // present but wrong" — a stale `MEDIAGIT_ENCRYPTION_KEYFILE` pointing
        // at a deleted file — so it falls through to the keychain like an
        // absent one instead of aborting the whole unlock. Only *shape*
        // failures land here; a well-formed keyfile holding the wrong 32 bytes
        // still returns `Ok`, and is rejected by GCM at the caller.
        MasterSource::Keyfile => Ok(keyfile_master()
            .inspect_err(|e| tracing::warn!(error = %format!("{e:#}"), "ignoring {KEYFILE_ENV}"))
            .ok()),
        MasterSource::Keychain => Ok(keychain_master()),
        MasterSource::Passphrase => {
            let salt_hex = stored
                .salt
                .as_deref()
                .ok_or_else(|| anyhow!("encryption-key: passphrase master with no salt"))?;
            let salt =
                Salt::from_hex(salt_hex).map_err(|e| anyhow!("encryption-key: bad salt: {e}"))?;
            let pw = prompt_passphrase(false)?;
            derive_key(&pw, &salt, Argon2Params::default())
                .map(Some)
                .map_err(|e| anyhow!("deriving the master key: {e}"))
        }
    }
}

/// Read the master key from the file named by [`KEYFILE_ENV`]. Accepts 64 hex
/// characters or exactly 32 raw bytes — the two shapes a `head -c 32
/// /dev/urandom` or an `openssl rand -hex 32` actually produces.
fn keyfile_master() -> Result<EncryptionKey> {
    let path = std::env::var_os(KEYFILE_ENV).ok_or_else(|| anyhow!("{KEYFILE_ENV} is not set"))?;
    keyfile_master_at(&PathBuf::from(path))
}

/// Read a master key out of a named file.
///
/// Split from [`keyfile_master`] because rotation needs to name the *new*
/// keyfile explicitly: unwrapping the old key and provisioning the new one both
/// consult `MEDIAGIT_ENCRYPTION_KEYFILE`, so without this a keyfile-to-keyfile
/// rotation could only ever re-wrap under the master it started with.
fn keyfile_master_at(path: &Path) -> Result<EncryptionKey> {
    // Zeroized regardless of which arm below fires: the hex arm's `raw` is
    // the master key spelled out in ASCII (equally sensitive to the raw-bytes
    // form), and the raw-bytes arm's `raw` *is* the key.
    let raw = Zeroizing::new(
        std::fs::read(path).with_context(|| format!("reading key file at {}", path.display()))?,
    );

    let bytes = match std::str::from_utf8(&raw).map(str::trim) {
        Ok(text) if text.len() == KEY_LEN * 2 => Zeroizing::new(
            hex::decode(text).with_context(|| format!("{} is not valid hex", path.display()))?,
        ),
        _ if raw.len() == KEY_LEN => Zeroizing::new(raw.to_vec()),
        _ => bail!(
            "{} must contain a 32-byte key, either as {} hex characters or {KEY_LEN} raw bytes",
            path.display(),
            KEY_LEN * 2
        ),
    };
    EncryptionKey::from_bytes(bytes.to_vec()).map_err(|e| anyhow!("{}: {e}", path.display()))
}

/// True when `MEDIAGIT_NO_KEYRING` is set, in which case the OS-keychain tier
/// is skipped for at-rest keys too.
///
/// It gated only `repo.rs`'s credential keyring at first, which meant a
/// `key init` with no `MEDIAGIT_ENCRYPTION_KEYFILE` fell through to the OS
/// keychain and wrote there anyway — so a test suite that had opted out still
/// touched the developer's real keychain. Both tiers use the same
/// `KEYRING_SERVICE`, so one opt-out has to cover both.
fn keyring_disabled() -> bool {
    std::env::var_os("MEDIAGIT_NO_KEYRING").is_some()
}

/// The OS-keychain master key, or `None` for any failure at all — missing
/// entry, locked keychain, unreadable payload. Mirrors the credential
/// keychain's rule in `repo.rs`: a keyring that is having a bad day degrades
/// to the next tier rather than breaking the command.
fn keychain_master() -> Option<EncryptionKey> {
    if keyring_disabled() {
        return None;
    }
    let entry = keyring::Entry::new(crate::repo::KEYRING_SERVICE, KEYCHAIN_ACCOUNT).ok()?;
    // The keychain hands back the master key as plain hex; both it and the
    // decoded bytes are wiped here, since from here on the only copy that
    // should exist is the one inside `EncryptionKey`.
    let hex_key = Zeroizing::new(entry.get_password().ok()?);
    let bytes = Zeroizing::new(hex::decode(hex_key.trim()).ok()?);
    EncryptionKey::from_bytes(bytes.to_vec()).ok()
}

/// Choose (and if necessary create) the master key for a fresh `key init`,
/// following the same precedence the read side uses. The third element is the
/// Argon2 salt, present only on the passphrase branch — it is the one piece of
/// the chosen source that has to be recorded on disk.
fn provision_master() -> Result<(EncryptionKey, MasterSource, Option<String>)> {
    if std::env::var_os(KEYFILE_ENV).is_some() {
        return Ok((keyfile_master()?, MasterSource::Keyfile, None));
    }
    if let Some(existing) = keychain_master() {
        return Ok((existing, MasterSource::Keychain, None));
    }
    if let Some(fresh) = create_keychain_master()? {
        return Ok((fresh, MasterSource::Keychain, None));
    }
    // No keychain on this machine (headless Linux, locked service). Fall back
    // to a passphrase rather than refusing to encrypt at all.
    let salt = Salt::generate().map_err(|e| anyhow!("generating a salt: {e}"))?;
    let pw = prompt_passphrase(true)?;
    let key = derive_key(&pw, &salt, Argon2Params::default())
        .map_err(|e| anyhow!("deriving the master key: {e}"))?;
    Ok((key, MasterSource::Passphrase, Some(salt.to_hex())))
}

/// Generate a master key and store it in the OS keychain. `None` if the
/// keychain is unavailable, which is the caller's cue to fall back.
fn create_keychain_master() -> Result<Option<EncryptionKey>> {
    if keyring_disabled() {
        return Ok(None);
    }
    let bytes = random_key_bytes().context("generating a master key")?;
    let Ok(entry) = keyring::Entry::new(crate::repo::KEYRING_SERVICE, KEYCHAIN_ACCOUNT) else {
        return Ok(None);
    };
    let hexed = Zeroizing::new(hex::encode(*bytes));
    if entry.set_password(hexed.as_str()).is_err() {
        return Ok(None);
    }
    Ok(EncryptionKey::from_bytes(bytes.to_vec()).ok())
}

/// Prompt for the master passphrase. `confirm` adds the re-type check, used
/// only at `key init` — a typo there is unrecoverable, since nothing else on
/// the machine knows the passphrase.
fn prompt_passphrase(confirm: bool) -> Result<SecretString> {
    let mut prompt = dialoguer::Password::new().with_prompt("Encryption passphrase");
    if confirm {
        prompt = prompt.with_confirmation("Confirm passphrase", "Passphrases don't match");
    }
    let value = prompt
        .interact()
        .context("reading the encryption passphrase")?;
    if value.is_empty() {
        bail!("an empty passphrase is not accepted");
    }
    Ok(SecretString::from(value))
}

/// Write the key file, failing if anything is already there.
///
/// `create_new` rather than a check-then-write: two `key init` runs racing must
/// not both think they won, because the loser would silently replace a key that
/// objects had already been sealed under.
fn write_new(path: &Path, stored: &StoredKey) -> Result<()> {
    use std::io::Write;

    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .with_context(|| format!("creating {}", path.display()))?;
    file.write_all(render(stored)?.as_bytes())
        .with_context(|| format!("writing {}", path.display()))?;
    restrict_permissions(path);
    Ok(())
}

/// Replace an existing key file (`key recover`, the only caller).
///
/// Write-then-rename, because the file being replaced is the only copy of the
/// wrapping that makes the repository readable: a torn write here would leave a
/// repo that neither the old nor the new master key can open.
fn write_over(path: &Path, stored: &StoredKey) -> Result<()> {
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, render(stored)?).with_context(|| format!("writing {}", tmp.display()))?;
    restrict_permissions(&tmp);
    if let Err(e) = std::fs::rename(&tmp, path) {
        // A failed rename would otherwise leave `encryption-key.tmp` behind —
        // holding the same key material as `path`, but under whatever ACL the
        // parent directory happened to hand a freshly created file.
        let _ = std::fs::remove_file(&tmp);
        return Err(e).with_context(|| format!("replacing {}", path.display()));
    }
    Ok(())
}

/// The file's full text: the explanatory header plus the TOML body.
fn render(stored: &StoredKey) -> Result<String> {
    let body = toml::to_string_pretty(stored).context("serializing the encryption key file")?;
    Ok(format!(
        "# MediaGit at-rest encryption key (DC-7).\n\
         #\n\
         # `wrapped` is this repository's 32-byte data key inside an MGEN envelope,\n\
         # encrypted under a master key that is NOT stored here. `recovery`, when\n\
         # present, is the same data key under the one-time recovery code shown at\n\
         # `key init`. Lose both and the repository's objects are unrecoverable.\n\
         # This file is safe to back up; it is not safe to treat as a backup of the\n\
         # key itself.\n\
         {body}"
    ))
}

/// Owner-only permissions where the OS models them. Best-effort: the file
/// holds ciphertext, so a permissive mode is a weakened defence rather than an
/// exposed key — same posture as `config.toml`'s advisory check in `repo.rs`.
#[cfg(unix)]
fn restrict_permissions(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
}

/// Windows has no owner-only mode bit; a freshly created file just inherits
/// its parent directory's ACL. `icacls /inheritance:r` breaks that
/// inheritance and `/grant:r "<user>:F"` leaves only the current user with
/// access — the posture OpenSSH requires of private key files. Shells out to
/// the built-in `icacls.exe` rather than the raw ACL Win32 APIs: this crate
/// denies `unsafe_code`, and `icacls` needs none. Best-effort, same posture as
/// the unix branch: failure here weakens the file's protection, it doesn't
/// expose the key directly (the file holds ciphertext).
#[cfg(windows)]
fn restrict_permissions(path: &Path) {
    let Ok(user) = std::env::var("USERNAME") else {
        return;
    };
    let _ = std::process::Command::new("icacls")
        .arg(path)
        .arg("/inheritance:r")
        .arg("/grant:r")
        .arg(format!("{user}:F"))
        .output();
}

#[cfg(not(any(unix, windows)))]
fn restrict_permissions(_path: &Path) {}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    /// The load path must be silent and cheap for the overwhelming majority of
    /// repositories, which have no key at all.
    #[test]
    fn a_repo_with_no_key_file_loads_nothing() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join(".mediagit")).unwrap();
        assert!(!has_key(dir.path()));
        assert!(load_repo_key(dir.path()).unwrap().is_none());
    }

    /// A key file from a future format version must be a message, not a
    /// mis-parse that ends in "wrong key".
    #[test]
    fn a_future_format_version_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join(".mediagit")).unwrap();
        std::fs::write(
            key_file_path(dir.path()),
            "version = 99\nmaster = \"keyfile\"\nwrapped = \"00\"\n",
        )
        .unwrap();
        let err = load_repo_key(dir.path()).unwrap_err();
        assert!(
            format!("{err:#}").contains("newer MediaGit"),
            "got: {err:#}"
        );
    }

    fn stored_fixture(fingerprint: Option<String>) -> StoredKey {
        StoredKey {
            version: 1,
            master: "keyfile".to_string(),
            salt: None,
            wrapped: String::new(),
            recovery: None,
            fingerprint,
        }
    }

    /// A key file whose recorded fingerprint doesn't match its unwrapped repo
    /// key must be rejected — closes the AES-GCM-isn't-key-committing gap
    /// regardless of which master source produced the key.
    #[test]
    fn a_mismatched_fingerprint_is_rejected() {
        let stored = stored_fixture(Some(hex::encode([0xAAu8; 32])));
        assert!(verify_fingerprint(&stored, &[7u8; KEY_LEN]).is_err());
    }

    /// A matching fingerprint passes.
    #[test]
    fn a_matching_fingerprint_is_accepted() {
        let repo_key = [7u8; KEY_LEN];
        let stored = stored_fixture(Some(blake3::hash(&repo_key).to_hex().to_string()));
        assert!(verify_fingerprint(&stored, &repo_key).is_ok());
    }

    /// Key files written before the fingerprint field existed have nothing to
    /// check against — must not block loading.
    #[test]
    fn a_missing_fingerprint_is_not_checked() {
        let stored = stored_fixture(None);
        assert!(verify_fingerprint(&stored, &[7u8; KEY_LEN]).is_ok());
    }

    /// The stored file must never contain the repo key in the clear — the
    /// whole point of wrapping it.
    #[test]
    fn the_stored_file_holds_ciphertext_not_the_key() {
        let master = EncryptionKey::from_bytes(vec![3u8; KEY_LEN]).unwrap();
        let repo_key = [9u8; KEY_LEN];
        let wrapped = envelope::seal(&master, &repo_key).unwrap();
        let hexed = hex::encode(&wrapped);
        assert!(
            !hexed.contains(&hex::encode(repo_key)),
            "the wrapped blob must not contain the plaintext repo key"
        );
        assert_eq!(envelope::open(&master, &wrapped).unwrap(), repo_key);
    }

    /// The code has to survive a human: read off a screen, typed back with
    /// whatever grouping, spacing and case they used.
    #[test]
    fn a_recovery_code_round_trips_through_transcription() {
        let secret = [0x4Du8; KEY_LEN];
        let code = encode_recovery_code(&secret);
        assert_eq!(code.len(), KEY_LEN * 2 + 7, "8 groups of 8, 7 dashes");

        let canonical = decode_recovery_code(&code).unwrap();
        for retyped in [
            code.replace('-', ""),                     // dashes dropped
            code.replace('-', " "),                    // dashes as spaces
            code.to_uppercase(),                       // caps lock
            format!(" {}\n", code.replace('-', "\n")), // pasted across lines
        ] {
            assert_eq!(
                decode_recovery_code(&retyped).unwrap(),
                canonical,
                "a transcribed code must decode to the same key: {retyped:?}"
            );
        }
    }

    /// A mistyped code must fail closed at GCM, never yield 32 bytes that would
    /// then be used to "decrypt" the whole repository.
    #[test]
    fn a_wrong_recovery_code_fails_closed() {
        let secret = [0x11u8; KEY_LEN];
        let key = decode_recovery_code(&encode_recovery_code(&secret)).unwrap();
        let repo_key = [0x22u8; KEY_LEN];
        let slot = envelope::seal(&key, &repo_key).unwrap();

        // One character off: same shape, same length, different key.
        let mut wrong = secret;
        wrong[0] ^= 0x01;
        let wrong_key = decode_recovery_code(&encode_recovery_code(&wrong)).unwrap();
        assert!(
            envelope::open(&wrong_key, &slot).is_err(),
            "a near-miss recovery code must be rejected, not partially accepted"
        );

        // And malformed input is a message, not a panic or a silent zero key.
        assert!(decode_recovery_code("not a code at all").is_err());
        assert!(decode_recovery_code("dead-beef").is_err());
        assert!(decode_recovery_code("").is_err());
    }

    /// Backward compatibility: a key file written before recovery slots existed
    /// still parses, still reports version 1, and simply has no slot.
    #[test]
    fn a_key_file_without_a_recovery_field_still_parses() {
        let text = "version = 1\nmaster = \"keyfile\"\nwrapped = \"abcd\"\n";
        let stored: StoredKey = toml::from_str(text).unwrap();
        assert_eq!(stored.version, 1);
        assert!(stored.recovery.is_none());
        // No salt recorded, so the passphrase prompt is not a candidate — a
        // repo that was never passphrase-wrapped must never ask for one.
        assert!(!candidates(&stored).contains(&MasterSource::Passphrase));

        // And round-trips back out without inventing the field.
        assert!(!render(&stored).unwrap().contains("recovery ="));
    }
}
