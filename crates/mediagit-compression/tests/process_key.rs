// SPDX-License-Identifier: BUSL-1.1
// Copyright (C) 2025-2026 Aswin Krishnamoorthy

//! DC-7/D3: the process-global at-rest key, exercised end to end.
//!
//! Deliberately an *integration* test, and deliberately one single `#[test]`:
//! the thing under test is a set-once process global, so it cannot be reset
//! between cases and cannot be observed from two cases running in parallel.
//! A separate test binary keeps it out of the unit tests' process, where
//! `without_a_key_output_is_byte_identical` depends on the global being unset;
//! one ordered function keeps the before/after halves from racing each other.

#![allow(clippy::unwrap_used)]

use mediagit_compression::{
    CompressionLevel, CompressionStrategy, ObjectType, SmartCompressor, TypeAwareCompressor,
    ensure_key_scope, open_at_rest, process_key, seal_at_rest, set_process_key,
};
use mediagit_security::encryption::EncryptionKey;
use mediagit_security::envelope::is_sealed;
use std::path::PathBuf;

fn key_of(byte: u8) -> EncryptionKey {
    EncryptionKey::from_bytes(vec![byte; 32]).unwrap()
}

/// A real directory, since the scope check canonicalizes what it is given.
fn scratch_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir()
        .join("mediagit-process-key-test")
        .join(name);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Content that exercises both compress exits (ordinary, and the
/// expand-to-Store fallback) across the strategies the ODB actually picks.
fn samples() -> Vec<(ObjectType, Vec<u8>)> {
    vec![
        (ObjectType::Text, b"highly compressible text ".repeat(200)),
        (ObjectType::Jpeg, (0u8..=255).cycle().take(4096).collect()),
        (ObjectType::Tiff, vec![0xAB; 8192]),
        (ObjectType::Unknown, b"mixed payload ".repeat(64)),
    ]
}

#[test]
fn the_process_key_is_off_by_default_and_settable_exactly_once() {
    // ---- before any key is installed ----
    assert!(
        process_key().is_none(),
        "the global must default to None, or every existing repo becomes encrypted by accident"
    );

    // The format-freeze guarantee, restated against the global rather than the
    // per-compressor key: a process with no key installed writes exactly the
    // bytes it wrote before DC-7 existed.
    let plain = SmartCompressor::new();
    let mut legacy = Vec::new();
    for (obj_type, data) in samples() {
        let out = plain.compress_typed_with_size(&data, obj_type).unwrap();
        assert!(
            !is_sealed(&out),
            "a process with no global key must never emit an envelope ({obj_type:?})"
        );
        assert_eq!(plain.decompress_typed(&out).unwrap(), data);
        legacy.push((data, out));
    }

    // With no key installed the at-rest helpers are the identity, which is
    // what lets manifests, pack deltas and ingested chunks keep their frozen
    // bytes on every unencrypted repo.
    let mine = scratch_dir("mine");
    let other = scratch_dir("other");
    assert_eq!(&*seal_at_rest(b"frozen").unwrap(), b"frozen");
    assert!(
        ensure_key_scope(&other).is_ok(),
        "with no key installed, no repository is out of scope"
    );

    // ---- install ----
    set_process_key(&mine, key_of(7)).expect("the first install must succeed");
    assert!(process_key().is_some());

    // Re-installing the *same* key for the same repo is a no-op, so a startup
    // path that runs twice (or a test harness that re-enters it) is harmless.
    set_process_key(&mine, key_of(7))
        .expect("installing the identical key again must be idempotent, not an error");

    // The key belongs to ONE repository. `find_repo_root()` walks upward, so
    // without this a `clone` into a subdirectory of an encrypted repo would
    // seal the new repo under a key it has no record of (F3).
    ensure_key_scope(&mine).expect("the repo the key was installed for must be allowed");
    ensure_key_scope(&mine.join(".").join("..").join("mine"))
        .expect("a different spelling of the same directory is the same repository");
    let scope_err = ensure_key_scope(&other)
        .expect_err("a different repository must be refused, not silently sealed");
    assert!(
        format!("{scope_err}").contains("different repository"),
        "the refusal must say why, got: {scope_err}"
    );

    // Payloads that never reach the compressor still get sealed, and open
    // symmetrically. Unsealed input stays readable so a repo that adopts a key
    // mid-life still reads what it wrote before.
    let sealed_at_rest = seal_at_rest(b"manifest bytes").unwrap();
    assert!(is_sealed(&sealed_at_rest), "seal_at_rest must seal");
    assert_eq!(&*open_at_rest(&sealed_at_rest).unwrap(), b"manifest bytes");
    assert_eq!(
        &*open_at_rest(b"legacy plaintext").unwrap(),
        b"legacy plaintext"
    );

    // A *different* key is a hard error, never a silent swap: objects already
    // written in this process used the first key and nothing records which.
    let err = set_process_key(&mine, key_of(9))
        .expect_err("a second, differing key must be refused, not silently swapped in");
    assert!(
        format!("{err}").contains("already installed"),
        "the refusal must say why, got: {err}"
    );
    assert!(
        !format!("{err}").contains("0707"),
        "an error about keys must never contain key material"
    );

    // ---- after install ----
    // Every fresh compressor adopts the key with no call-site change. This is
    // the whole mechanism: the ~20 `SmartCompressor::new()` sites need no edit,
    // so none of them can be the one that was missed.
    let adopted = SmartCompressor::new();
    assert!(adopted.is_encrypted());

    for (obj_type, data) in samples() {
        let out = adopted.compress_typed_with_size(&data, obj_type).unwrap();
        assert!(
            is_sealed(&out),
            "with a global key installed every write must be sealed ({obj_type:?})"
        );
        assert_eq!(
            adopted.decompress_typed(&out).unwrap(),
            data,
            "sealed object did not round-trip ({obj_type:?})"
        );
    }

    // Enabling encryption must not orphan what the repo already holds: the
    // objects written above, before the key existed, still read back.
    for (data, out) in legacy {
        assert_eq!(
            adopted.decompress_typed(&out).unwrap(),
            data,
            "a keyed process must still read objects written before the key existed"
        );
    }

    // And the fail-closed direction still holds for a compressor explicitly
    // built without the key, which is what a wrong-key/no-key process sees.
    let sealed = adopted
        .compress_typed(b"secret", ObjectType::Jpeg)
        .expect("seal must not fail");
    let wrong = SmartCompressor::new().with_key(key_of(9));
    assert!(
        wrong.decompress_typed(&sealed).is_err(),
        "a wrong key must error, never fall through to the codec sniffer"
    );

    // Strategy coverage for the sealed path, including Store's separate exit.
    for strategy in [
        CompressionStrategy::Store,
        CompressionStrategy::Zstd(CompressionLevel::Default),
        CompressionStrategy::Brotli(CompressionLevel::Best),
    ] {
        let obj_type = match strategy {
            CompressionStrategy::Store => ObjectType::Jpeg,
            CompressionStrategy::Brotli(_) => ObjectType::Text,
            _ => ObjectType::AdobePhotoshop,
        };
        let data = b"strategy coverage payload ".repeat(40);
        let out = adopted.compress_typed(&data, obj_type).unwrap();
        assert!(is_sealed(&out), "{strategy:?} exit must seal");
        assert_eq!(adopted.decompress_typed(&out).unwrap(), data);
    }
}
