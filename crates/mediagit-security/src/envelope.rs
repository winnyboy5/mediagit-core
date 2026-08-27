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

//! DC-7: the `MGEN` envelope — a self-identifying frame for an encrypted object.
//!
//! [`encrypt`](crate::encryption::encrypt) already produces
//! `[version:1][nonce:12][ciphertext][tag:16]`, which is enough to *decrypt* a
//! blob you already know is encrypted. It is not enough to *recognise* one, and
//! recognition is the whole problem here.
//!
//! Object bytes in this project are read back by content-sniffing: the smart
//! compressor identifies its codec from a leading byte (`0x00` Store, `0x78`
//! zlib, `0x28B52FFD` zstd, `"BRT\x01"` brotli). AES-GCM output is
//! indistinguishable from random, so there is nothing to sniff and no byte that
//! is safe to assume. Without a magic, an encrypted object would be fed to
//! whichever codec its first random byte happened to resemble.
//!
//! Hence a four-byte magic, checked **before** any sniffing:
//!
//! ```text
//! [ "MGEN" : 4 ][ version=2 : 1 ][ nonce : 24 ][ ciphertext ][ GCM tag : 16 ]
//!  \___ this module ___/ \________ crate::encryption::encrypt output _______/
//! ```
//!
//! 45 bytes of overhead (up from 33 in envelope v1 — see
//! `crate::encryption`'s module doc for why the nonce grew to 192 bits).
//! Reading stays version-dispatched: v1 objects sealed by earlier builds
//! still open (`crate::encryption::decrypt` picks the framing from the
//! version byte), this module just never *writes* v1 again.
//!
//! The magic cannot collide with any codec this project emits (`M` is
//! `0x4D`), so the discrimination is exact in both directions: a sealed
//! object is never mistaken for a compressed one, and — the property that
//! matters for a frozen format — an object written **without** a key is
//! byte-for-byte what it was before this module existed.
//!
//! Modelled on the `MGCM` manifest envelope, the one existing precedent in the
//! codebase, rather than inventing a scheme.

use crate::encryption::{EncryptionError, EncryptionKey, decrypt, encrypt};

/// Magic prefix identifying a sealed object. Four bytes, like `MGCM`.
pub const MGEN_MAGIC: &[u8; 4] = b"MGEN";

/// Bytes added to a payload by [`seal`]: magic + version + nonce + GCM tag.
/// Envelope v2 (24-byte nonce); a v1 object read back by [`open`] carries 12
/// fewer bytes of overhead, but this crate never writes v1 again.
pub const ENVELOPE_OVERHEAD: usize = MGEN_MAGIC.len() + 1 + 24 + 16;

/// Is this a sealed object?
///
/// Cheap enough to call on every read, which is the point: it is the branch
/// that keeps an unencrypted repo on exactly its old code path.
pub fn is_sealed(data: &[u8]) -> bool {
    data.len() >= MGEN_MAGIC.len() && &data[..MGEN_MAGIC.len()] == MGEN_MAGIC
}

/// Wrap `plaintext` in an `MGEN` envelope.
///
/// `plaintext` here is whatever the caller wants stored — for the object
/// database that is the *compressed* bytes, so compression still happens and
/// still pays before the payload becomes incompressible.
pub fn seal(key: &EncryptionKey, plaintext: &[u8]) -> Result<Vec<u8>, EncryptionError> {
    let body = encrypt(key, plaintext)?;
    let mut out = Vec::with_capacity(MGEN_MAGIC.len() + body.len());
    out.extend_from_slice(MGEN_MAGIC);
    out.extend_from_slice(&body);
    Ok(out)
}

/// Unwrap an `MGEN` envelope.
///
/// Fails closed on every abnormal input: not sealed, truncated, wrong key,
/// tampered byte. There is deliberately **no** fallback to returning the input
/// unchanged — that would turn a wrong key into silent corruption, handing the
/// caller ciphertext it would then try to decompress and store as if it were
/// content.
pub fn open(key: &EncryptionKey, data: &[u8]) -> Result<Vec<u8>, EncryptionError> {
    if !is_sealed(data) {
        return Err(EncryptionError::InvalidCiphertext(
            "not an MGEN envelope (missing magic)".to_string(),
        ));
    }
    decrypt(key, &data[MGEN_MAGIC.len()..])
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn key() -> EncryptionKey {
        EncryptionKey::generate().unwrap()
    }

    #[test]
    fn round_trips() {
        let k = key();
        let plain = b"the compressed bytes of a chunk".to_vec();
        let sealed = seal(&k, &plain).unwrap();
        assert!(is_sealed(&sealed));
        assert_eq!(open(&k, &sealed).unwrap(), plain);
    }

    /// An envelope v1 object — `MGEN` + `[version=1][nonce:12][ct+tag]`, as
    /// pre-XAES-256-GCM builds wrote and as existing encrypted repos still
    /// hold — must keep opening. `seal` only ever writes v2 now, so this is
    /// hand-built rather than produced by round-tripping through it.
    #[test]
    fn a_v1_sealed_object_still_opens() {
        use aes_gcm::{
            Aes256Gcm, Nonce,
            aead::{Aead, KeyInit},
        };

        let k = key();
        let plain = b"pre-XAES object, sealed by an earlier build".to_vec();

        let cipher = Aes256Gcm::new_from_slice(k.expose_key()).unwrap();
        let nonce_bytes = [9u8; 12];
        // v1 had no associated data at all.
        let body = cipher
            .encrypt(Nonce::from_slice(&nonce_bytes), plain.as_slice())
            .unwrap();

        let mut v1 = Vec::new();
        v1.extend_from_slice(MGEN_MAGIC);
        v1.push(1); // legacy version byte
        v1.extend_from_slice(&nonce_bytes);
        v1.extend_from_slice(&body);

        assert!(is_sealed(&v1));
        assert_eq!(open(&k, &v1).unwrap(), plain);
    }

    #[test]
    fn the_wrong_key_fails_closed() {
        // The single most important property. A wrong key must be an error, never
        // a partial read and never a pass-through: the caller's next move is to
        // decompress and store what it gets back, so "returns something" here
        // means silent corruption of a whole repository.
        let sealed = seal(&key(), b"secret").unwrap();
        assert!(open(&key(), &sealed).is_err());
    }

    #[test]
    fn a_flipped_byte_fails_closed() {
        let k = key();
        let mut sealed = seal(&k, b"secret payload").unwrap();
        let last = sealed.len() - 1;
        sealed[last] ^= 0xFF;
        assert!(
            open(&k, &sealed).is_err(),
            "GCM authentication must reject a tampered envelope"
        );
    }

    #[test]
    fn unsealed_input_is_rejected_not_passed_through() {
        assert!(open(&key(), b"plain zlib-ish bytes").is_err());
    }

    #[test]
    fn no_codec_magic_can_be_mistaken_for_an_envelope() {
        // The discrimination this module exists for. Every byte pattern the
        // smart compressor emits as a leading marker must read as NOT sealed,
        // or an ordinary compressed object would be routed to the decryptor.
        for probe in [
            &[0x00u8, 0x01, 0x02, 0x03][..], // Store
            &[0x78, 0xF9, 0x00, 0x00][..],   // zlib
            &[0x78, 0xDA, 0x00, 0x00][..],   // zlib
            &[0x28, 0xB5, 0x2F, 0xFD][..],   // zstd
            b"BRT\x01",                      // brotli
        ] {
            assert!(
                !is_sealed(probe),
                "codec magic must not look sealed: {probe:?}"
            );
        }
    }

    #[test]
    fn short_input_does_not_panic() {
        for n in 0..MGEN_MAGIC.len() {
            assert!(!is_sealed(&MGEN_MAGIC[..n]));
        }
    }

    #[test]
    fn overhead_is_what_the_constant_claims() {
        // FORMATS.md quotes this number; a drift between the constant and the
        // real framing would make the storage-overhead figure a lie.
        let sealed = seal(&key(), b"").unwrap();
        assert_eq!(sealed.len(), ENVELOPE_OVERHEAD);
    }
}
