// SPDX-License-Identifier: BUSL-1.1
// Copyright (C) 2025-2026 Aswin Krishnamoorthy

//! DC-7/D4: two repositories, two different keys, one process.
//!
//! This is the case the rest of the encryption suite structurally cannot
//! express. Every other keyed test drives `SmartCompressor` through
//! `process_key`, a set-once `OnceLock` bound to one repo root — fine for the
//! CLI, useless for a server holding many repositories open at once. So the
//! server carries the key per repository and hands it to
//! `ObjectDatabase::with_at_rest_key`, and that is what these pin.
//!
//! No process key is set anywhere in this file. If sealing happened here, it
//! happened because the database was handed a key.

use mediagit_compression::EncryptionKey;
use mediagit_storage::StorageBackend;
use mediagit_storage::mock::MockBackend;
use mediagit_versioning::{ObjectDatabase, ObjectType};
use std::sync::Arc;

fn key(byte: u8) -> EncryptionKey {
    EncryptionKey::from_bytes(vec![byte; 32]).expect("32 bytes is a key")
}

/// Every object in the backend, with whether it carries the `MGEN` magic.
///
/// Asserts it saw something: otherwise "nothing is sealed" and "I looked in
/// the wrong place" are the same answer.
async fn sealed_flags(storage: &Arc<MockBackend>) -> Vec<bool> {
    let keys = storage.list_objects("").await.unwrap_or_default();
    assert!(!keys.is_empty(), "no objects at all; measuring nothing");
    let mut out = Vec::new();
    for k in keys {
        let bytes = storage.get(&k).await.expect("stored object reads back");
        out.push(mediagit_security::envelope::is_sealed(&bytes));
    }
    out
}

#[tokio::test(flavor = "multi_thread")]
async fn two_repositories_under_two_keys_stay_separate_in_one_process() {
    let payload = b"the object graph is as sensitive as the objects".repeat(40);

    let store_a = Arc::new(MockBackend::new());
    let store_b = Arc::new(MockBackend::new());
    let odb_a = ObjectDatabase::with_smart_compression(store_a.clone(), 100)
        .with_at_rest_key(Some(key(0xa1)));
    let odb_b = ObjectDatabase::with_smart_compression(store_b.clone(), 100)
        .with_at_rest_key(Some(key(0xb2)));

    let oid_a = odb_a.write(ObjectType::Blob, &payload).await.unwrap();
    let oid_b = odb_b.write(ObjectType::Blob, &payload).await.unwrap();

    // Same content, same plaintext hash — dedup keys on plaintext, so keying
    // must not change the OID. If it did, an encrypted repo would lose every
    // bit of the dedup that is the product.
    assert_eq!(
        oid_a, oid_b,
        "the OID is a plaintext hash and must not move"
    );

    assert!(
        sealed_flags(&store_a).await.iter().all(|s| *s),
        "every object in a keyed repository must be sealed"
    );
    assert!(sealed_flags(&store_b).await.iter().all(|s| *s));

    // Each opens its own.
    assert_eq!(odb_a.read(&oid_a).await.unwrap(), payload);
    assert_eq!(odb_b.read(&oid_b).await.unwrap(), payload);

    // Neither opens the other's. This is the whole point of per-repo keying:
    // one process, two tenants, and B's key must not read A's bytes.
    let crossed = ObjectDatabase::with_smart_compression(store_a.clone(), 100)
        .with_at_rest_key(Some(key(0xb2)));
    assert!(
        crossed.read(&oid_a).await.is_err(),
        "the wrong key must fail closed, never hand back bytes"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn an_unkeyed_database_cannot_read_a_keyed_one_and_says_so() {
    let payload = b"sealed bytes reaching the codec sniffer is silent corruption".repeat(20);

    let storage = Arc::new(MockBackend::new());
    let keyed =
        ObjectDatabase::with_smart_compression(storage.clone(), 100).with_at_rest_key(Some(key(7)));
    let oid = keyed.write(ObjectType::Blob, &payload).await.unwrap();

    // A server that lost its master, or never loaded one, gets an error — not
    // ciphertext classified as "uncompressed" and returned as content.
    let bare = ObjectDatabase::with_smart_compression(storage, 100);
    assert!(bare.read(&oid).await.is_err());
}

#[tokio::test(flavor = "multi_thread")]
async fn no_key_means_no_change() {
    let payload = b"the default path must stay byte-for-byte what it was".repeat(20);

    let storage = Arc::new(MockBackend::new());
    // `None` is the whole unencrypted world; it must be a no-op, not a
    // different-but-equivalent path.
    let odb = ObjectDatabase::with_smart_compression(storage.clone(), 100).with_at_rest_key(None);
    assert!(!odb.is_at_rest_encrypted());

    let oid = odb.write(ObjectType::Blob, &payload).await.unwrap();
    assert!(
        sealed_flags(&storage).await.iter().all(|s| !*s),
        "nothing may be sealed without a key"
    );
    assert_eq!(odb.read(&oid).await.unwrap(), payload);
}

/// Bytes that arrive already sealed must not be sealed a second time.
///
/// This is the shape that broke encrypted clone. `put_compressed_chunk` used
/// to assume its input was plaintext "because the server holds no key" -- true
/// before DC-7/D4, false after it, since the server now holds the repository
/// key and hands back exactly the bytes the client uploaded. Wrapping them
/// again produced a double envelope: the read unseals once, finds another
/// envelope, the codec sniffer recognises no magic and calls it uncompressed,
/// and the chunk fails its own hash check. Every cloned chunk, silently, with
/// the whole unit suite green.
#[tokio::test(flavor = "multi_thread")]
async fn a_chunk_that_arrives_sealed_is_stored_once_not_twice() {
    let payload = b"the bytes a clone pulls back down are already wrapped".repeat(30);

    // Producer: a keyed repo, exactly what the uploading client had.
    let origin_store = Arc::new(MockBackend::new());
    let origin = ObjectDatabase::with_smart_compression(origin_store.clone(), 100)
        .with_at_rest_key(Some(key(3)));
    let oid = origin.write(ObjectType::Blob, &payload).await.unwrap();

    // The wire bytes: what the server hands back is what the client stored.
    let wire = {
        let keys = origin_store.list_objects("").await.unwrap();
        let k = keys
            .iter()
            .find(|k| k.contains(&oid.to_hex()))
            .expect("the object we just wrote");
        origin_store.get(k).await.unwrap()
    };
    assert!(
        mediagit_security::envelope::is_sealed(&wire),
        "the premise of this test is that the wire bytes are sealed"
    );

    // Consumer: the cloning repo, same key (escrow hands back the same one).
    let clone_store = Arc::new(MockBackend::new());
    let clone = ObjectDatabase::with_smart_compression(clone_store.clone(), 100)
        .with_at_rest_key(Some(key(3)));
    clone.put_compressed_chunk(&oid, &wire).await.unwrap();

    // Stored exactly as received -- one envelope, not two.
    let stored = clone_store
        .get(&format!("chunks/{}", oid.to_hex()))
        .await
        .unwrap();
    assert_eq!(
        stored, wire,
        "already-sealed input must be stored verbatim; a second envelope is unreadable"
    );

    // And it reads back. This is what actually failed in the field.
    assert_eq!(clone.get_chunk(&oid).await.unwrap(), payload);
}

/// The other direction still works: plaintext off the wire gets sealed.
#[tokio::test(flavor = "multi_thread")]
async fn a_chunk_that_arrives_plaintext_is_sealed_on_the_way_in() {
    let payload = b"an unencrypted remote into a keyed clone".repeat(30);

    let plain_store = Arc::new(MockBackend::new());
    let plain = ObjectDatabase::with_smart_compression(plain_store.clone(), 100);
    let oid = plain.write(ObjectType::Blob, &payload).await.unwrap();
    let wire = {
        let keys = plain_store.list_objects("").await.unwrap();
        let k = keys.iter().find(|k| k.contains(&oid.to_hex())).unwrap();
        plain_store.get(k).await.unwrap()
    };
    assert!(!mediagit_security::envelope::is_sealed(&wire));

    let clone_store = Arc::new(MockBackend::new());
    let clone = ObjectDatabase::with_smart_compression(clone_store.clone(), 100)
        .with_at_rest_key(Some(key(4)));
    clone.put_compressed_chunk(&oid, &wire).await.unwrap();

    let stored = clone_store
        .get(&format!("chunks/{}", oid.to_hex()))
        .await
        .unwrap();
    assert!(
        mediagit_security::envelope::is_sealed(&stored),
        "plaintext must not land in the clear inside a keyed repository"
    );
    assert_eq!(clone.get_chunk(&oid).await.unwrap(), payload);
}
