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

//! XAES-256-GCM encryption for object storage
//!
//! This module provides authenticated encryption using AES-256-GCM with:
//! - 256-bit keys (32 bytes)
//! - 192-bit nonces (24 bytes) - randomly generated per encryption, extended
//!   via the [XAES-256-GCM] derivation below
//! - 128-bit authentication tags (16 bytes)
//! - Stream encryption for large objects (64KB chunks)
//!
//! # Security Features
//!
//! - **Authenticated Encryption**: Provides both confidentiality and integrity
//! - **Extended nonces**: 192 bits of OS randomness per encryption, not 96 —
//!   see "Why XAES" below
//! - **Bound framing**: the envelope magic and version byte are part of the
//!   AEAD associated data, so a flipped version byte fails the tag check
//!   rather than being silently misinterpreted
//! - **Stream Support**: Handles large objects efficiently with chunked encryption
//!
//! # Why XAES, not plain AES-256-GCM
//!
//! NIST SP 800-38D caps random-96-bit-nonce AES-GCM at 2^32 encryptions under
//! one key before the birthday bound on nonce collisions becomes a real risk.
//! A MediaGit repo key never rotates — it seals every object for the life of
//! the repository — so a random 96-bit nonce is not a comfortable margin for
//! a large, long-lived repo. [XAES-256-GCM] (C2SP) fixes this by spending a
//! 192-bit nonce: 96 bits pick a per-message AES-256 subkey (three extra
//! block encryptions, [`xaes_derive`]), and the other 96 bits are the ordinary
//! GCM nonce under that subkey. The random-nonce collision bound moves to
//! 2^80 messages, which a repo key will never approach.
//!
//! [XAES-256-GCM]: https://github.com/C2SP/C2SP/blob/main/XAES-256-GCM.md
//!
//! # Format
//!
//! Envelope v2 (written by this build):
//! ```text
//! [version=2:1][nonce:24][ciphertext:N][tag:16]
//! ```
//!
//! Envelope v1 (read-only — objects sealed by earlier builds must keep
//! opening; see [`decrypt`]'s version dispatch):
//! ```text
//! [version=1:1][nonce:12][ciphertext:N][tag:16]
//! ```
//!
//! For stream encryption (objects > 64KB), the per-object nonce (or, for v2,
//! the derived `Nx`) is the *base* nonce; each chunk's nonce is that base
//! XORed with its little-endian chunk index:
//! ```text
//! [version][nonce][chunk1_cipher][chunk1_tag][chunk2_cipher][chunk2_tag]...
//! ```

// Suppress deprecation warning for GenericArray::from_slice
// This is a dependency warning from aes-gcm 0.10 using generic-array 0.14
// Will be resolved when upgrading to aes-gcm with generic-array 1.x support
#![allow(deprecated)]

use aes_gcm::{
    Aes256Gcm, Nonce,
    aead::{Aead, KeyInit, Payload},
};
use secrecy::{ExposeSecret, SecretBox};
use thiserror::Error;
use tracing::debug;
use zeroize::{ZeroizeOnDrop, Zeroizing};

/// Current encryption version — the only version this build *writes*.
const ENCRYPTION_VERSION: u8 = 2;

/// Legacy version — still *readable* (see [`decrypt`]'s dispatch), never
/// written. Objects sealed before the XAES-256-GCM migration must keep
/// opening; dropping v1 support would make every existing encrypted repo
/// unreadable.
const LEGACY_VERSION: u8 = 1;

/// AES-GCM nonce size in bytes (96 bits) — the size of the *derived* nonce
/// `Nx` under v2, and of the full nonce under legacy v1.
const NONCE_SIZE: usize = 12;

/// v2 envelope nonce size in bytes (192 bits) — the XAES-256-GCM input nonce
/// `N`, split by [`xaes_derive`] into a subkey selector and `Nx`.
const XNONCE_SIZE: usize = 24;

/// Authentication tag size in bytes (128 bits)
const TAG_SIZE: usize = 16;

/// Key size in bytes (256 bits for AES-256)
pub const KEY_SIZE: usize = 32;

/// Chunk size for stream encryption (64KB)
const CHUNK_SIZE: usize = 64 * 1024;

/// Threshold for stream encryption (same as chunk size)
const STREAM_THRESHOLD: usize = CHUNK_SIZE;

/// Encryption errors
#[derive(Error, Debug)]
pub enum EncryptionError {
    /// AES-GCM encryption operation failed.
    #[error("Encryption failed: {0}")]
    EncryptionFailed(String),

    /// AES-GCM decryption or authentication tag verification failed.
    #[error("Decryption failed: {0}")]
    DecryptionFailed(String),

    /// Key byte length was not `KEY_SIZE` (32 bytes).
    #[error("Invalid key size: expected {KEY_SIZE}, got {0}")]
    InvalidKeySize(usize),

    /// Ciphertext is malformed or truncated (missing nonce, tag, or version byte).
    #[error("Invalid ciphertext: {0}")]
    InvalidCiphertext(String),

    /// Ciphertext was produced by a newer version of this encryption format.
    #[error("Unsupported encryption version: {0}")]
    UnsupportedVersion(u8),
}

/// Encryption key wrapper with secure memory handling
pub struct EncryptionKey {
    key: SecretBox<Vec<u8>>,
}

impl Clone for EncryptionKey {
    fn clone(&self) -> Self {
        Self {
            key: SecretBox::new(Box::new(self.key.expose_secret().clone())),
        }
    }
}

// `SecretBox<Vec<u8>>` already zeroizes its contents on drop (it requires
// `Vec<u8>: Zeroize`, which the `zeroize` crate provides, and implements
// `Drop` + `ZeroizeOnDrop` unconditionally on top of that). This impl makes
// that guarantee a checkable property of `EncryptionKey` itself rather than
// something a reader has to trace through `secrecy`'s internals — and
// `tests::key_is_zeroize_on_drop` fails to compile if it ever stops holding.
impl ZeroizeOnDrop for EncryptionKey {}

impl EncryptionKey {
    /// Create a new encryption key from bytes
    ///
    /// # Security
    ///
    /// The key bytes are securely stored and will be zeroized on drop.
    pub fn from_bytes(bytes: Vec<u8>) -> Result<Self, EncryptionError> {
        if bytes.len() != KEY_SIZE {
            return Err(EncryptionError::InvalidKeySize(bytes.len()));
        }

        Ok(Self {
            key: SecretBox::new(Box::new(bytes)),
        })
    }

    /// Generate a new random encryption key
    ///
    /// Uses the OS CSPRNG directly (`getrandom`), not `rand`'s thread-local
    /// RNG: a forked child process inherits `ThreadRng`'s state and would
    /// repeat it, which for key/nonce material means repeating an AES-GCM
    /// nonce under the same key — catastrophic for GCM.
    pub fn generate() -> Result<Self, EncryptionError> {
        let mut key_bytes = vec![0u8; KEY_SIZE];
        getrandom::fill(&mut key_bytes)
            .map_err(|e| EncryptionError::EncryptionFailed(format!("OS RNG unavailable: {e}")))?;

        Ok(Self {
            key: SecretBox::new(Box::new(key_bytes)),
        })
    }

    /// Get the key bytes (use with caution)
    ///
    /// # Security Warning
    ///
    /// This exposes the key material. Only use when necessary and ensure
    /// the exposed bytes are properly zeroized after use.
    pub(crate) fn expose_key(&self) -> &[u8] {
        self.key.expose_secret().as_slice()
    }

    /// Derive a new key from this one via BLAKE3's domain-separated KDF.
    ///
    /// For wrapping keys specifically: a master/recovery key used *directly*
    /// as an AES-GCM key to wrap a repo key is sharing a primitive and nonce
    /// space with every other use of "an AES-GCM key" in this codebase,
    /// separated only by the convention that nobody reuses it for anything
    /// else. Deriving a role-specific subkey here — rather than wrapping with
    /// the master/recovery key raw — makes that separation structural: even
    /// if the same key material were ever handed to a second role, the two
    /// roles would use cryptographically unrelated keys.
    ///
    /// `context` should be a stable, unique string identifying the purpose
    /// (`blake3::derive_key`'s convention, e.g. `"mediagit/wrap/v2"`) — never
    /// reuse a context across two different derivations of the same key.
    pub fn derive_subkey(&self, context: &str) -> Result<Self, EncryptionError> {
        let derived = Zeroizing::new(blake3::derive_key(context, self.expose_key()));
        Self::from_bytes(derived.to_vec())
    }
}

/// Constant-time equality, so callers outside this crate can ask "is this the
/// same key?" without [`EncryptionKey::expose_key`] having to become public.
///
/// DC-7's process-global key registry needs exactly this and nothing more: it
/// distinguishes "installed twice, same key" (idempotent, fine) from
/// "installed twice, different key" (a hard error). Keeping the comparison
/// here rather than handing out bytes keeps key material inside this module.
impl PartialEq for EncryptionKey {
    fn eq(&self, other: &Self) -> bool {
        let (a, b) = (self.expose_key(), other.expose_key());
        // Both operands are fixed-size, but fold rather than short-circuit so
        // the timing carries no information about where they first differ.
        a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
    }
}

impl Eq for EncryptionKey {}

impl std::fmt::Debug for EncryptionKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EncryptionKey")
            .field("key", &"<redacted>")
            .finish()
    }
}

/// `MGEN || version`, bound into the AEAD tag as associated data so a
/// tampered version byte fails GCM authentication instead of merely being
/// misrouted to the wrong decode path. `crate::envelope` owns the `MGEN`
/// framing; referencing its constant here (rather than duplicating the
/// literal) keeps the two in lockstep.
fn build_aad(version: u8) -> [u8; 5] {
    // 4-byte magic + 1-byte version. `envelope::tests::overhead_is_what_the_constant_claims`
    // pins the magic's length, so this literal can't silently drift from it.
    let mut aad = [0u8; 5];
    aad[..4].copy_from_slice(crate::envelope::MGEN_MAGIC.as_slice());
    aad[4] = version;
    aad
}

/// XAES-256-GCM key/nonce derivation, implemented directly on the `aes`
/// crate's raw block cipher (three AES-256 block encryptions) rather than
/// adding a dependency on a wrapper crate. Verified against the C2SP spec's
/// published test vectors — see `tests::xaes_test_vector_*` below.
///
/// <https://github.com/C2SP/C2SP/blob/main/XAES-256-GCM.md>
fn xaes_derive(
    key: &[u8; KEY_SIZE],
    nonce: &[u8; XNONCE_SIZE],
) -> (Zeroizing<[u8; KEY_SIZE]>, [u8; NONCE_SIZE]) {
    use aes::Aes256;
    use aes::cipher::{BlockEncrypt, KeyInit as _, generic_array::GenericArray};

    let cipher = Aes256::new(GenericArray::from_slice(key));

    // Steps 1-2: CMAC subkey K1, from L = AES_K(0^16) (NIST SP 800-38B §6.1
    // subkey generation — the "double" is a big-endian 128-bit left shift
    // with conditional XOR of the GCM reduction constant 0x87).
    let mut l_block = GenericArray::clone_from_slice(&[0u8; 16]);
    cipher.encrypt_block(&mut l_block);
    let mut l = [0u8; 16];
    l.copy_from_slice(l_block.as_slice());
    let k1 = Zeroizing::new(double_block(&l));
    l.fill(0);

    // Steps 3-4: M1/M2 = 0x00 || counter(1|2) || 'X' (0x58) || 0x00 || N[:12].
    let mut m1 = [0u8; 16];
    m1[1] = 0x01;
    m1[2] = 0x58;
    m1[4..].copy_from_slice(&nonce[..12]);
    let mut m2 = m1;
    m2[1] = 0x02;

    // Step 5: Kx = AES_K(M1 ^ K1) || AES_K(M2 ^ K1).
    let mut b1 = GenericArray::clone_from_slice(&xor16(&m1, &k1));
    cipher.encrypt_block(&mut b1);
    let mut b2 = GenericArray::clone_from_slice(&xor16(&m2, &k1));
    cipher.encrypt_block(&mut b2);

    let mut kx = Zeroizing::new([0u8; KEY_SIZE]);
    kx.as_mut_slice()[..16].copy_from_slice(&b1);
    kx.as_mut_slice()[16..].copy_from_slice(&b2);
    // `GenericArray` isn't `zeroize::Zeroize` here (that impl is behind a
    // feature this workspace doesn't enable), so clear it by hand.
    b1.as_mut_slice().fill(0);
    b2.as_mut_slice().fill(0);

    // Step 6: Nx = N[12:].
    let mut nx = [0u8; NONCE_SIZE];
    nx.copy_from_slice(&nonce[12..]);

    (kx, nx)
}

fn xor16(a: &[u8; 16], b: &[u8; 16]) -> [u8; 16] {
    let mut out = [0u8; 16];
    for i in 0..16 {
        out[i] = a[i] ^ b[i];
    }
    out
}

/// Big-endian 128-bit left shift with the standard AES-CMAC/GCM doubling
/// reduction: if the vacated top bit was 1, XOR the constant `0x87` into the
/// low byte. NIST SP 800-38B §6.1; XAES-256-GCM spec steps 1-2.
fn double_block(l: &[u8; 16]) -> [u8; 16] {
    let msb_set = l[0] & 0x80 != 0;
    let mut out = [0u8; 16];
    for i in 0..16 {
        out[i] = l[i] << 1;
        if i + 1 < 16 && (l[i + 1] & 0x80) != 0 {
            out[i] |= 1;
        }
    }
    if msb_set {
        out[15] ^= 0x87;
    }
    out
}

/// Chunk `idx`'s nonce: `base` XORed with the chunk index as little-endian
/// bytes. Shared by stream encryption and stream decryption so the two can
/// never derive different per-chunk nonces from the same base.
fn chunk_nonce(base: &[u8; NONCE_SIZE], idx: usize) -> [u8; NONCE_SIZE] {
    let mut nonce = *base;
    for (i, &byte) in (idx as u64).to_le_bytes().iter().enumerate() {
        if i < NONCE_SIZE {
            nonce[i] ^= byte;
        }
    }
    nonce
}

/// Encrypt data using XAES-256-GCM (envelope v2).
///
/// # Arguments
///
/// * `key` - Encryption key (32 bytes)
/// * `plaintext` - Data to encrypt
///
/// # Returns
///
/// Encrypted data with format: `[version=2:1][nonce:24][ciphertext:N][tag:16]`
///
/// # Examples
///
/// ```no_run
/// use mediagit_security::encryption::{EncryptionKey, encrypt};
///
/// let key = EncryptionKey::generate().unwrap();
/// let plaintext = b"sensitive data";
/// let ciphertext = encrypt(&key, plaintext).unwrap();
/// ```
pub fn encrypt(key: &EncryptionKey, plaintext: &[u8]) -> Result<Vec<u8>, EncryptionError> {
    debug!(size = plaintext.len(), "Encrypting data");

    // 192 bits of OS randomness per encryption — see the module doc for why
    // this, and not a 96-bit AES-GCM nonce, is what gets generated here.
    let mut nonce24 = [0u8; XNONCE_SIZE];
    getrandom::fill(&mut nonce24)
        .map_err(|e| EncryptionError::EncryptionFailed(format!("OS RNG unavailable: {e}")))?;

    let key_bytes: [u8; KEY_SIZE] = key
        .expose_key()
        .try_into()
        .expect("EncryptionKey::from_bytes enforces KEY_SIZE");
    let (kx, nx) = xaes_derive(&key_bytes, &nonce24);

    let cipher = Aes256Gcm::new_from_slice(kx.as_slice())
        .map_err(|e| EncryptionError::EncryptionFailed(e.to_string()))?;
    let aad = build_aad(ENCRYPTION_VERSION);

    let mut output = Vec::with_capacity(1 + XNONCE_SIZE + plaintext.len() + TAG_SIZE);
    output.push(ENCRYPTION_VERSION);
    output.extend_from_slice(&nonce24);

    if plaintext.len() > STREAM_THRESHOLD {
        output.extend_from_slice(&encrypt_stream(&cipher, &nx, &aad, plaintext)?);
    } else {
        let nonce = Nonce::from_slice(&nx);
        let ciphertext = cipher
            .encrypt(
                nonce,
                Payload {
                    msg: plaintext,
                    aad: &aad,
                },
            )
            .map_err(|e| EncryptionError::EncryptionFailed(e.to_string()))?;
        output.extend_from_slice(&ciphertext);
    }

    debug!(
        plaintext_size = plaintext.len(),
        ciphertext_size = output.len(),
        overhead = output.len() - plaintext.len(),
        "Encryption complete"
    );

    Ok(output)
}

/// Decrypt data produced by [`encrypt`] — dispatches on the version byte so
/// v1 objects (sealed before the XAES-256-GCM migration) keep opening
/// alongside v2 ones.
///
/// # Arguments
///
/// * `key` - Decryption key (32 bytes)
/// * `ciphertext` - Encrypted data from `encrypt()`
///
/// # Returns
///
/// Original plaintext data
///
/// # Examples
///
/// ```no_run
/// use mediagit_security::encryption::{EncryptionKey, encrypt, decrypt};
///
/// let key = EncryptionKey::generate().unwrap();
/// let plaintext = b"sensitive data";
/// let ciphertext = encrypt(&key, plaintext).unwrap();
/// let decrypted = decrypt(&key, &ciphertext).unwrap();
/// assert_eq!(decrypted, plaintext);
/// ```
pub fn decrypt(key: &EncryptionKey, ciphertext: &[u8]) -> Result<Vec<u8>, EncryptionError> {
    debug!(size = ciphertext.len(), "Decrypting data");

    let Some(&version) = ciphertext.first() else {
        return Err(EncryptionError::InvalidCiphertext(
            "Ciphertext too short".to_string(),
        ));
    };

    match version {
        LEGACY_VERSION => decrypt_v1(key, ciphertext),
        ENCRYPTION_VERSION => decrypt_v2(key, ciphertext),
        other => Err(EncryptionError::UnsupportedVersion(other)),
    }
}

/// v1: `[version=1:1][nonce:12][ciphertext+tag]`, raw key, no associated
/// data — exactly what pre-XAES builds wrote. Read-only; never produced by
/// [`encrypt`].
fn decrypt_v1(key: &EncryptionKey, ciphertext: &[u8]) -> Result<Vec<u8>, EncryptionError> {
    if ciphertext.len() < 1 + NONCE_SIZE + TAG_SIZE {
        return Err(EncryptionError::InvalidCiphertext(
            "Ciphertext too short".to_string(),
        ));
    }
    let nonce_bytes: [u8; NONCE_SIZE] = ciphertext[1..1 + NONCE_SIZE]
        .try_into()
        .expect("slice is exactly NONCE_SIZE bytes");
    let encrypted_data = &ciphertext[1 + NONCE_SIZE..];

    let cipher = Aes256Gcm::new_from_slice(key.expose_key())
        .map_err(|e| EncryptionError::DecryptionFailed(e.to_string()))?;

    let plaintext = decrypt_dispatch(&cipher, &nonce_bytes, &[], encrypted_data)?;
    debug!(
        ciphertext_size = ciphertext.len(),
        plaintext_size = plaintext.len(),
        "Decryption complete (v1)"
    );
    Ok(plaintext)
}

/// v2: `[version=2:1][nonce:24][ciphertext+tag]`, XAES-derived subkey, `MGEN
/// || version` as associated data.
fn decrypt_v2(key: &EncryptionKey, ciphertext: &[u8]) -> Result<Vec<u8>, EncryptionError> {
    if ciphertext.len() < 1 + XNONCE_SIZE + TAG_SIZE {
        return Err(EncryptionError::InvalidCiphertext(
            "Ciphertext too short".to_string(),
        ));
    }
    let nonce24: [u8; XNONCE_SIZE] = ciphertext[1..1 + XNONCE_SIZE]
        .try_into()
        .expect("slice is exactly XNONCE_SIZE bytes");
    let encrypted_data = &ciphertext[1 + XNONCE_SIZE..];

    let key_bytes: [u8; KEY_SIZE] = key
        .expose_key()
        .try_into()
        .expect("EncryptionKey::from_bytes enforces KEY_SIZE");
    let (kx, nx) = xaes_derive(&key_bytes, &nonce24);

    let cipher = Aes256Gcm::new_from_slice(kx.as_slice())
        .map_err(|e| EncryptionError::DecryptionFailed(e.to_string()))?;
    let aad = build_aad(ENCRYPTION_VERSION);

    let plaintext = decrypt_dispatch(&cipher, &nx, &aad, encrypted_data)?;
    debug!(
        ciphertext_size = ciphertext.len(),
        plaintext_size = plaintext.len(),
        "Decryption complete (v2)"
    );
    Ok(plaintext)
}

/// Regular vs. stream decryption, shared by v1 and v2 (they differ only in
/// nonce size, key, and associated data, all already resolved by the caller).
///
/// Detects stream-encrypted data from its size, same heuristic `encrypt`'s
/// counterpart has always used: stream output is a concatenation of
/// `CHUNK_SIZE + TAG_SIZE`-sized blocks, which single-block ciphertext only
/// coincidentally matches at implausible sizes.
fn decrypt_dispatch(
    cipher: &Aes256Gcm,
    nonce_base: &[u8; NONCE_SIZE],
    aad: &[u8],
    encrypted_data: &[u8],
) -> Result<Vec<u8>, EncryptionError> {
    let chunk_ciphertext_size = CHUNK_SIZE + TAG_SIZE;
    let could_be_stream = encrypted_data.len() > STREAM_THRESHOLD + TAG_SIZE
        || (encrypted_data.len() >= chunk_ciphertext_size
            && encrypted_data.len().is_multiple_of(chunk_ciphertext_size)
            && encrypted_data.len() > TAG_SIZE);

    if could_be_stream {
        match decrypt_stream(cipher, nonce_base, aad, encrypted_data) {
            Ok(plaintext) => return Ok(plaintext),
            Err(_) => {
                // Fall through to try regular decryption.
                debug!("Stream decryption failed, trying regular decryption");
            }
        }
    }

    let nonce = Nonce::from_slice(nonce_base);
    cipher
        .decrypt(
            nonce,
            Payload {
                msg: encrypted_data,
                aad,
            },
        )
        .map_err(|e| EncryptionError::DecryptionFailed(format!("Authentication failed: {}", e)))
}

/// Encrypt large data using stream encryption.
///
/// Splits `plaintext` into 64KB chunks, each encrypted with `cipher` under a
/// nonce derived from `base` by [`chunk_nonce`]. Returns just the
/// concatenated chunk ciphertexts — the caller prepends version and nonce.
fn encrypt_stream(
    cipher: &Aes256Gcm,
    base: &[u8; NONCE_SIZE],
    aad: &[u8],
    plaintext: &[u8],
) -> Result<Vec<u8>, EncryptionError> {
    debug!(
        size = plaintext.len(),
        chunks = plaintext.len().div_ceil(CHUNK_SIZE),
        "Stream encrypting large object"
    );

    let mut output =
        Vec::with_capacity(plaintext.len() + TAG_SIZE * plaintext.len().div_ceil(CHUNK_SIZE));

    for (chunk_idx, chunk) in plaintext.chunks(CHUNK_SIZE).enumerate() {
        let nonce_bytes = chunk_nonce(base, chunk_idx);
        let nonce = Nonce::from_slice(&nonce_bytes);
        let chunk_ciphertext = cipher
            .encrypt(nonce, Payload { msg: chunk, aad })
            .map_err(|e| EncryptionError::EncryptionFailed(e.to_string()))?;
        output.extend_from_slice(&chunk_ciphertext);
    }

    Ok(output)
}

/// Decrypt stream-encrypted data. Reached from [`decrypt_dispatch`], never
/// called directly.
fn decrypt_stream(
    cipher: &Aes256Gcm,
    base: &[u8; NONCE_SIZE],
    aad: &[u8],
    encrypted_data: &[u8],
) -> Result<Vec<u8>, EncryptionError> {
    debug!(
        size = encrypted_data.len(),
        "Stream decrypting large object"
    );

    let mut plaintext = Vec::with_capacity(encrypted_data.len());
    let chunk_size = CHUNK_SIZE + TAG_SIZE;

    for (chunk_idx, chunk) in encrypted_data.chunks(chunk_size).enumerate() {
        let nonce_bytes = chunk_nonce(base, chunk_idx);
        let nonce = Nonce::from_slice(&nonce_bytes);
        let chunk_plaintext = cipher
            .decrypt(nonce, Payload { msg: chunk, aad })
            .map_err(|e| EncryptionError::DecryptionFailed(e.to_string()))?;
        plaintext.extend_from_slice(&chunk_plaintext);
    }

    Ok(plaintext)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn test_key_generation() {
        let key = EncryptionKey::generate().unwrap();
        assert_eq!(key.expose_key().len(), KEY_SIZE);
    }

    #[test]
    fn test_key_from_bytes() {
        let bytes = vec![42u8; KEY_SIZE];
        let key = EncryptionKey::from_bytes(bytes.clone()).unwrap();
        assert_eq!(key.expose_key(), &bytes[..]);
    }

    #[test]
    fn test_key_invalid_size() {
        let bytes = vec![42u8; 16]; // Wrong size
        assert!(EncryptionKey::from_bytes(bytes).is_err());
    }

    /// Structural, not behavioral: `EncryptionKey` wraps `SecretBox<Vec<u8>>`,
    /// which only implements `ZeroizeOnDrop` because `Vec<u8>: Zeroize` — this
    /// fails to *compile* if that chain ever breaks (e.g. the field is swapped
    /// for a type that doesn't zeroize), which is the only assurance available
    /// without reading freed memory, itself unsafe and forbidden in this crate.
    #[test]
    fn key_is_zeroize_on_drop() {
        fn assert_zeroize_on_drop<T: ZeroizeOnDrop>() {}
        assert_zeroize_on_drop::<EncryptionKey>();
    }

    #[test]
    fn derive_subkey_is_deterministic_and_context_separated() {
        let key = EncryptionKey::generate().unwrap();
        let a1 = key.derive_subkey("mediagit/wrap/v2").unwrap();
        let a2 = key.derive_subkey("mediagit/wrap/v2").unwrap();
        let b = key.derive_subkey("mediagit/other/v1").unwrap();

        assert_eq!(
            a1, a2,
            "same key + same context must derive the same subkey"
        );
        assert_ne!(
            a1, b,
            "different contexts must derive unrelated subkeys, not just different values"
        );
        assert_ne!(
            a1.expose_key(),
            key.expose_key(),
            "the subkey must not just be the parent key back out"
        );
    }

    #[test]
    fn test_encrypt_decrypt_small() {
        let key = EncryptionKey::generate().unwrap();
        let plaintext = b"Hello, World!";

        let ciphertext = encrypt(&key, plaintext).unwrap();
        let decrypted = decrypt(&key, &ciphertext).unwrap();

        assert_eq!(decrypted, plaintext);
        assert_ne!(ciphertext[1 + XNONCE_SIZE..], plaintext[..]); // Ensure it's encrypted
    }

    #[test]
    fn test_encrypt_decrypt_large() {
        let key = EncryptionKey::generate().unwrap();
        let plaintext = vec![42u8; 200_000]; // 200KB

        let ciphertext = encrypt(&key, &plaintext).unwrap();
        let decrypted = decrypt(&key, &ciphertext).unwrap();

        assert_eq!(decrypted, plaintext);
    }

    #[test]
    fn test_encrypt_empty() {
        let key = EncryptionKey::generate().unwrap();
        let plaintext = b"";

        let ciphertext = encrypt(&key, plaintext).unwrap();
        let decrypted = decrypt(&key, &ciphertext).unwrap();

        assert_eq!(decrypted, plaintext);
    }

    #[test]
    fn test_decrypt_invalid_ciphertext() {
        let key = EncryptionKey::generate().unwrap();
        let invalid = vec![1u8; 10]; // Too short

        assert!(decrypt(&key, &invalid).is_err());
    }

    #[test]
    fn test_decrypt_wrong_key() {
        let key1 = EncryptionKey::generate().unwrap();
        let key2 = EncryptionKey::generate().unwrap();
        let plaintext = b"secret";

        let ciphertext = encrypt(&key1, plaintext).unwrap();
        assert!(decrypt(&key2, &ciphertext).is_err());
    }

    #[test]
    fn test_decrypt_tampered_data() {
        let key = EncryptionKey::generate().unwrap();
        let plaintext = b"original data";

        let mut ciphertext = encrypt(&key, plaintext).unwrap();
        // Tamper with ciphertext
        let last = ciphertext.len() - 1;
        ciphertext[last] ^= 1;

        assert!(decrypt(&key, &ciphertext).is_err());
    }

    #[test]
    fn a_flipped_version_byte_fails_the_tag_not_just_the_dispatch() {
        // The AAD-binding property: corrupting the version byte must fail
        // AEAD authentication, not just route to a different (or rejected)
        // decode path. Flip it to another *valid* version's byte value so a
        // naive `UnsupportedVersion` short-circuit can't be the thing catching
        // this — the legacy-vs-current nonce sizes differ, but the point of
        // binding the version into the AAD is that even a same-shape version
        // confusion is caught by the tag, not by incidental field layout.
        let key = EncryptionKey::generate().unwrap();
        let mut ciphertext = encrypt(&key, b"payload").unwrap();
        assert_eq!(ciphertext[0], ENCRYPTION_VERSION);
        ciphertext[0] = LEGACY_VERSION;
        assert!(decrypt(&key, &ciphertext).is_err());
    }

    #[test]
    fn test_unique_nonces() {
        let key = EncryptionKey::generate().unwrap();
        let plaintext = b"same data";

        let ct1 = encrypt(&key, plaintext).unwrap();
        let ct2 = encrypt(&key, plaintext).unwrap();

        // Nonces should be different
        assert_ne!(&ct1[1..1 + XNONCE_SIZE], &ct2[1..1 + XNONCE_SIZE]);
        // But both should decrypt to same plaintext
        assert_eq!(decrypt(&key, &ct1).unwrap(), plaintext);
        assert_eq!(decrypt(&key, &ct2).unwrap(), plaintext);
    }

    #[test]
    fn test_overhead() {
        let key = EncryptionKey::generate().unwrap();
        let plaintext = vec![0u8; 1000];

        let ciphertext = encrypt(&key, &plaintext).unwrap();
        let overhead = ciphertext.len() - plaintext.len();

        // Overhead should be: version(1) + nonce(24) + tag(16) = 41 bytes
        assert_eq!(overhead, 1 + XNONCE_SIZE + TAG_SIZE);
    }

    #[test]
    fn test_stream_encryption_threshold() {
        let key = EncryptionKey::generate().unwrap();

        // Just under threshold
        let small = vec![0u8; STREAM_THRESHOLD - 1];
        let ct_small = encrypt(&key, &small).unwrap();

        // Just over threshold
        let large = vec![0u8; STREAM_THRESHOLD + 1];
        let ct_large = encrypt(&key, &large).unwrap();

        // Both should decrypt correctly
        assert_eq!(decrypt(&key, &ct_small).unwrap(), small);
        assert_eq!(decrypt(&key, &ct_large).unwrap(), large);
    }

    /// A v1-shaped ciphertext (as pre-XAES builds produced, and as still sits
    /// in existing encrypted repos) must keep opening under the current code.
    /// Hand-built rather than round-tripped through `encrypt`, since `encrypt`
    /// only ever writes v2 now.
    #[test]
    fn a_v1_sealed_object_still_opens() {
        let key = EncryptionKey::generate().unwrap();
        let plaintext = b"pre-XAES object";

        let cipher = Aes256Gcm::new_from_slice(key.expose_key()).unwrap();
        let nonce_bytes = [7u8; NONCE_SIZE];
        let nonce = Nonce::from_slice(&nonce_bytes);
        // v1 had no associated data at all.
        let body = cipher.encrypt(nonce, plaintext.as_slice()).unwrap();

        let mut v1_ciphertext = Vec::with_capacity(1 + NONCE_SIZE + body.len());
        v1_ciphertext.push(LEGACY_VERSION);
        v1_ciphertext.extend_from_slice(&nonce_bytes);
        v1_ciphertext.extend_from_slice(&body);

        assert_eq!(decrypt(&key, &v1_ciphertext).unwrap(), plaintext);
    }

    // ------------------------------------------------------------------
    // XAES-256-GCM: C2SP published test vectors
    // https://github.com/C2SP/C2SP/blob/main/XAES-256-GCM.md
    //
    // Fetched directly from the raw spec source (not summarized) and
    // cross-checked by hand against the derivation formula before being
    // committed here, per the instruction to stop rather than ship
    // unverified crypto if the vectors couldn't be obtained.
    // ------------------------------------------------------------------

    fn hex32(s: &str) -> [u8; 32] {
        hex::decode(s).unwrap().try_into().unwrap()
    }

    fn hex16(s: &str) -> [u8; 16] {
        hex::decode(s).unwrap().try_into().unwrap()
    }

    /// Vector 1: MSB1(L) = 0, empty AAD.
    #[test]
    fn xaes_test_vector_1() {
        let key = hex32("0101010101010101010101010101010101010101010101010101010101010101");
        let nonce: [u8; XNONCE_SIZE] = b"ABCDEFGHIJKLMNOPQRSTUVWX".to_owned();

        let expected_kx = hex32("c8612c9ed53fe43e8e005b828a1631a0bbcb6ab2f46514ec4f439fcfd0fa969b");
        let expected_nx: [u8; NONCE_SIZE] = hex::decode("4d4e4f505152535455565758")
            .unwrap()
            .try_into()
            .unwrap();

        let (kx, nx) = xaes_derive(&key, &nonce);
        assert_eq!(*kx, expected_kx, "Kx mismatch");
        assert_eq!(nx, expected_nx, "Nx mismatch");

        // Full AEAD: plaintext "XAES-256-GCM", empty AAD.
        let cipher = Aes256Gcm::new_from_slice(&expected_kx).unwrap();
        let ct = cipher
            .encrypt(
                Nonce::from_slice(&expected_nx),
                Payload {
                    msg: b"XAES-256-GCM",
                    aad: b"",
                },
            )
            .unwrap();
        assert_eq!(
            hex::encode(&ct),
            "ce546ef63c9cc60765923609b33a9a1974e96e52daf2fcf7075e2271"
        );
    }

    /// Vector 2: MSB1(L) = 1, non-empty AAD.
    #[test]
    fn xaes_test_vector_2() {
        let key = hex32("0303030303030303030303030303030303030303030303030303030303030303");
        let nonce: [u8; XNONCE_SIZE] = b"ABCDEFGHIJKLMNOPQRSTUVWX".to_owned();

        let expected_kx = hex32("e9c621d4cdd9b11b00a6427ad7e559aeedd66b3857646677748f8ca796cb3fd8");
        let expected_nx: [u8; NONCE_SIZE] = hex::decode("4d4e4f505152535455565758")
            .unwrap()
            .try_into()
            .unwrap();

        let (kx, nx) = xaes_derive(&key, &nonce);
        assert_eq!(*kx, expected_kx, "Kx mismatch");
        assert_eq!(nx, expected_nx, "Nx mismatch");

        let cipher = Aes256Gcm::new_from_slice(&expected_kx).unwrap();
        let ct = cipher
            .encrypt(
                Nonce::from_slice(&expected_nx),
                Payload {
                    msg: b"XAES-256-GCM",
                    aad: b"c2sp.org/XAES-256-GCM",
                },
            )
            .unwrap();
        assert_eq!(
            hex::encode(&ct),
            "986ec1832593df5443a179437fd083bf3fdb41abd740a21f71eb769d"
        );
    }

    /// Cross-checks the CMAC subkey (`L`, `K1`) the spec's vectors publish
    /// alongside `Kx`/`Nx`, so a bug in [`double_block`] specifically (as
    /// opposed to the two AES calls around it) would show up here rather
    /// than only as a wrong final `Kx`.
    #[test]
    fn xaes_subkey_generation_matches_spec() {
        use aes::Aes256;
        use aes::cipher::{BlockEncrypt, KeyInit as _, generic_array::GenericArray};

        let key = hex32("0101010101010101010101010101010101010101010101010101010101010101");
        let expected_l = hex16("7298caa565031eadc6ce23d23ea66378");
        let expected_k1 = hex16("e531954aca063d5b8d9c47a47d4cc6f0");

        let cipher = Aes256::new(GenericArray::from_slice(&key));
        let mut l_block = GenericArray::clone_from_slice(&[0u8; 16]);
        cipher.encrypt_block(&mut l_block);
        assert_eq!(l_block.as_slice(), &expected_l[..]);

        let k1 = double_block(&expected_l);
        assert_eq!(k1, expected_k1);
    }
}
