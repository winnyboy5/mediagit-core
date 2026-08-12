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
