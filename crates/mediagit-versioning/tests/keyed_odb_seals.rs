// SPDX-License-Identifier: BUSL-1.1
// Copyright (C) 2025-2026 Aswin Krishnamoorthy

//! DC-7: an `ObjectDatabase` built by a *plain* constructor must still seal
//! once the process holds an encryption key.
//!
//! `ObjectDatabase::new` and `without_compression` leave `smart_compressor`
//! as `None`, and sealing only happens through `SmartCompressor` on the
//! compression path — so before the guard in `odb/core.rs` these wrote
//! **plaintext** into a repository whose owner had run `mediagit key init`.
//! No production call site builds one today, which is exactly why this needed
//! a test rather than a comment: the trap is latent, so nothing else would
//! notice it being re-opened.
//!
//! Its own test binary because the process key is a set-once global; a test
//! that sets it would otherwise leak into every other test in the crate.
//! The unkeyed half runs FIRST, inside the same test function, so ordering is
//! sequential and guaranteed rather than left to the harness.

use mediagit_storage::StorageBackend;
use mediagit_storage::mock::MockBackend;
use mediagit_versioning::{ChunkStrategy, ObjectDatabase, ObjectType};
use std::sync::Arc;

/// Does any object in the backend carry the `MGEN` envelope magic?
///
/// Asserts it actually saw objects. Without that, "nothing is sealed" and
/// "I looked in the wrong place" are the same answer — and the unkeyed half
/// of this test would pass for the wrong reason.
async fn any_sealed(storage: &Arc<MockBackend>) -> bool {
    let keys = storage.list_objects("").await.unwrap_or_default();
    assert!(
        !keys.is_empty(),
        "detector found no objects at all; it is measuring nothing"
    );
    for key in keys {
        if let Ok(bytes) = storage.get(&key).await
            && mediagit_security::envelope::is_sealed(&bytes)
        {
            return true;
        }
    }
    false
}

#[tokio::test(flavor = "multi_thread")]
async fn a_plain_object_database_seals_once_the_process_is_keyed() {
    let payload = b"delta savings are the product; plaintext on disk is not".repeat(20);

    // --- unkeyed control, first: nothing may be sealed, and this is also the
    // proof that `any_sealed` can return false (a detector that always says
    // "sealed" would make the second half pass vacuously).
    let plain_storage = Arc::new(MockBackend::new());
    let plain_odb = ObjectDatabase::new(plain_storage.clone(), 100);
    let plain_oid = plain_odb
        .write(ObjectType::Blob, &payload)
        .await
        .expect("unkeyed write");
    assert!(
        !any_sealed(&plain_storage).await,
        "no key is set, so nothing may be sealed"
    );
    assert_eq!(
        plain_odb.read(&plain_oid).await.expect("unkeyed read"),
        payload,
        "unkeyed round trip"
    );

    // Still unkeyed, and this is the only window in which it can be done:
    // capture what an *unencrypted* repository stores for a chunked object.
    // The manifest bytes are the frozen-format control, and the chunk bytes
    // are exactly the shape a keyless server hands to `put_compressed_chunk`
    // during a pull.
    let (remote_chunk_id, remote_chunk_bytes, plain_chunk) = {
        let storage = Arc::new(MockBackend::new());
        let odb = ObjectDatabase::with_optimizations(
            storage.clone(),
            100,
            Some(ChunkStrategy::Fixed { size: 256 * 1024 }),
            false,
            0,
        );
        let oid = odb
            .write_chunked(ObjectType::Blob, &chunked_payload(), "budget-2027.psd")
            .await
            .expect("unkeyed chunked write");

        let manifest_bytes = storage
            .get(&format!("manifests/{}", oid.to_hex()))
            .await
            .expect("unkeyed manifest");
        assert!(
            !mediagit_security::envelope::is_sealed(&manifest_bytes),
            "no key is installed, so the manifest must be the frozen plaintext bytes"
        );
        assert!(
            find_bytes(&manifest_bytes, b"budget-2027.psd"),
            "control: the filename really is in the clear when unencrypted, \
             so the keyed assertion below is measuring something"
        );

        let manifest = odb
            .get_chunk_manifest(&oid)
            .await
            .expect("unkeyed manifest read")
            .expect("object is chunked");
        let chunk_id = manifest.chunks[0].id;
        let bytes = storage
            .get(&format!("chunks/{}", chunk_id.to_hex()))
            .await
            .expect("unkeyed chunk");
        let plain = odb.get_chunk(&chunk_id).await.expect("unkeyed chunk read");
        (chunk_id, bytes, plain)
    };

    // --- now key the process and rebuild through the SAME plain constructor.
    let key = mediagit_security::encryption::EncryptionKey::from_bytes(vec![9u8; 32])
        .expect("32-byte key");
    mediagit_compression::set_process_key(&std::env::temp_dir(), key)
        .expect("first set must succeed");

    let keyed_storage = Arc::new(MockBackend::new());
    let keyed_odb = ObjectDatabase::new(keyed_storage.clone(), 100);
    let keyed_oid = keyed_odb
        .write(ObjectType::Blob, &payload)
        .await
        .expect("keyed write");

    assert!(
        any_sealed(&keyed_storage).await,
        "process is keyed, so `ObjectDatabase::new` must seal — a plain \
         constructor silently storing plaintext is the whole point of this test"
    );
    assert_eq!(
        keyed_odb.read(&keyed_oid).await.expect("keyed read"),
        payload,
        "sealed objects must still round trip"
    );

    // `without_compression` is the nastier one: it stores raw bytes, so
    // without the guard a keyed process would write plaintext with not even
    // zlib in the way.
    let raw_storage = Arc::new(MockBackend::new());
    let raw_odb = ObjectDatabase::without_compression(raw_storage.clone(), 100);
    let raw_oid = raw_odb
        .write(ObjectType::Blob, &payload)
        .await
        .expect("keyed uncompressed write");
    assert!(
        any_sealed(&raw_storage).await,
        "`without_compression` must not defeat encryption"
    );
    assert_eq!(
        raw_odb
            .read(&raw_oid)
            .await
            .expect("keyed uncompressed read"),
        payload,
        "round trip through the uncompressed keyed path"
    );

    // ---- F1: chunk manifests ----------------------------------------------
    // A manifest names the file and lists the plaintext hash of every chunk,
    // so leaving it in the clear hands over the filename and a confirmation
    // oracle for guessable content. Sealing is at the local storage boundary
    // only — `to_bytes`/`from_bytes` still speak the wire format the keyless
    // server stores and serves.
    let chunk_storage = Arc::new(MockBackend::new());
    let chunk_odb = ObjectDatabase::with_optimizations(
        chunk_storage.clone(),
        100,
        Some(ChunkStrategy::Fixed { size: 256 * 1024 }),
        false,
        0,
    );
    let chunked_payload = chunked_payload();
    let chunked_oid = chunk_odb
        .write_chunked(ObjectType::Blob, &chunked_payload, "budget-2027.psd")
        .await
        .expect("keyed chunked write");

    let manifest_key = format!("manifests/{}", chunked_oid.to_hex());
    let stored_manifest = chunk_storage
        .get(&manifest_key)
        .await
        .expect("keyed manifest");
    assert!(
        mediagit_security::envelope::is_sealed(&stored_manifest),
        "the manifest of a keyed repo must be sealed on disk"
    );
    for key in chunk_storage.list_objects("").await.unwrap_or_default() {
        let bytes = chunk_storage.get(&key).await.expect("stored object");
        assert!(
            !find_bytes(&bytes, b"budget-2027.psd"),
            "the filename leaked in the clear at {key}"
        );
    }
    // And every local reader still works, which is the other half of the fix:
    // a sealed manifest that only the writer understands is a broken repo.
    assert_eq!(
        chunk_odb
            .get_chunk_manifest(&chunked_oid)
            .await
            .expect("keyed manifest read")
            .expect("object is chunked")
            .total_size,
        chunked_payload.len() as u64
    );
    assert_eq!(
        chunk_odb
            .read(&chunked_oid)
            .await
            .expect("keyed chunked read"),
        chunked_payload,
        "a chunked object must reconstruct through the sealed manifest"
    );
    assert_eq!(
        chunk_odb
            .get_object_size(&chunked_oid)
            .await
            .expect("keyed size"),
        chunked_payload.len(),
        "get_object_size reads the manifest too"
    );

    // ---- F4: chunks ingested from a remote --------------------------------
    // `put_compressed_chunk` writes bytes that arrived over the wire, and the
    // wire has no key. Unsealed input passes `unseal` through untouched, so
    // before the fix a pull left the ODB half encrypted with nothing to say so.
    assert!(
        !mediagit_security::envelope::is_sealed(&remote_chunk_bytes),
        "control: the bytes a keyless server sends are unsealed, so storing them \
         verbatim is what the assertion below has to catch"
    );
    chunk_odb
        .put_compressed_chunk(&remote_chunk_id, &remote_chunk_bytes)
        .await
        .expect("ingesting a remote chunk");
    assert!(
        mediagit_security::envelope::is_sealed(
            &chunk_storage
                .get(&format!("chunks/{}", remote_chunk_id.to_hex()))
                .await
                .expect("ingested chunk")
        ),
        "a chunk pulled into a keyed repo must be sealed on ingest, not stored verbatim"
    );
    assert_eq!(
        chunk_odb
            .get_chunk(&remote_chunk_id)
            .await
            .expect("reading the ingested chunk"),
        plain_chunk,
        "sealing on ingest must not change what a reader gets back"
    );

    // ---- F2: `gc --repack` ------------------------------------------------
    // The delta branch of `repack` hands raw delta bytes to the pack writer
    // and then DELETES the loose sealed copy, so before the fix gc removed
    // encryption from delta-encoded objects permanently and silently.
    let pack_storage = Arc::new(MockBackend::new());
    let pack_odb = ObjectDatabase::new(pack_storage.clone(), 100);
    const SECRET: &[u8] = b"SALARY-BAND-CONFIDENTIAL";
    let base: Vec<u8> = b"quarterly report body, mostly unchanged. ".repeat(400);
    let mut revised = base.clone();
    revised.extend_from_slice(SECRET);
    let base_oid = pack_odb
        .write(ObjectType::Blob, &base)
        .await
        .expect("base write");
    let revised_oid = pack_odb
        .write(ObjectType::Blob, &revised)
        .await
        .expect("revised write");
    // `repack` only reaches its delta branch when the similarity detector
    // already knows the base; a plain `write` does not register one.
    pack_odb
        .seed_similarity_from_blob(&base_oid, "report.txt")
        .await
        .expect("seeding the similarity detector");

    let stats = pack_odb.repack(0, true).await.expect("repack");
    assert!(
        stats.delta_objects > 0,
        "this test is only meaningful if the delta branch ran; it packed {} objects with {} deltas",
        stats.objects_packed,
        stats.delta_objects
    );
    for key in pack_storage
        .list_objects("packs/")
        .await
        .unwrap_or_default()
    {
        let bytes = pack_storage.get(&key).await.expect("pack");
        assert!(
            !find_bytes(&bytes, SECRET),
            "gc --repack wrote plaintext into {key}: the delta payload is not sealed"
        );
    }
    assert_eq!(
        pack_odb.read(&base_oid).await.expect("packed base read"),
        base,
        "the packed base must still read back"
    );
    assert_eq!(
        pack_odb
            .read(&revised_oid)
            .await
            .expect("packed delta read"),
        revised,
        "the packed delta must still reconstruct, i.e. the pack reader unseals"
    );
}

/// Enough varied content to cross the chunking threshold and produce several
/// chunks. Varied so it does not collapse to a single run-length chunk.
fn chunked_payload() -> Vec<u8> {
    (0..2 * 1024 * 1024).map(|i| (i % 251) as u8).collect()
}

/// Is `needle` present anywhere in `haystack`?
///
/// The leak detector for the assertions above: sealing is only worth anything
/// if the plaintext it was hiding is genuinely absent from the stored bytes.
fn find_bytes(haystack: &[u8], needle: &[u8]) -> bool {
    haystack
        .windows(needle.len())
        .any(|window| window == needle)
}
