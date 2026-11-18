// SPDX-License-Identifier: BUSL-1.1
// Copyright (C) 2025-2026 Aswin Krishnamoorthy

//! SEC-5: `write_chunk_delta` takes its payload from a remote. A delta that
//! does not rebuild the chunk it is filed under must never become visible.

use mediagit_compression::{SmartCompressor, TypeAwareCompressor};
use mediagit_storage::StorageBackend;
use mediagit_storage::mock::MockBackend;
use mediagit_versioning::{DeltaEncoder, ObjectDatabase, Oid};
use std::sync::Arc;

fn payload(base: &[u8], target: &[u8]) -> Vec<u8> {
    SmartCompressor::new()
        .compress_typed(
            &DeltaEncoder::encode(base, target).to_bytes(),
            mediagit_compression::ObjectType::Unknown,
        )
        .unwrap()
}

async fn setup() -> (ObjectDatabase, Arc<MockBackend>, Vec<u8>, Oid) {
    let storage = Arc::new(MockBackend::new());
    let odb = ObjectDatabase::with_smart_compression(storage.clone(), 100);
    let base = b"base chunk bytes, long enough to delta against. ".repeat(64);
    let base_id = Oid::hash(&base);
    odb.write_full_chunk(&base_id, &base).await.unwrap();
    (odb, storage, base, base_id)
}

async fn nothing_stored(storage: &MockBackend, id: &Oid) -> bool {
    let k = format!("chunk-deltas/{}", id.to_hex());
    !storage.exists(&k).await.unwrap() && !storage.exists(&format!("{k}.meta")).await.unwrap()
}

#[tokio::test]
async fn delta_that_rebuilds_the_wrong_bytes_is_refused_and_not_stored() {
    let (odb, storage, base, base_id) = setup().await;
    let mut target = base.clone();
    target.extend_from_slice(b" tail");
    let id = Oid::hash(&target);
    // A well-formed delta, but to other bytes than `id` names.
    let mut wrong = base.clone();
    wrong.extend_from_slice(b" evil");
    let err = odb
        .write_chunk_delta(&id, &base_id, &payload(&base, &wrong))
        .await
        .expect_err("a bad delta must be refused");
    assert!(err.to_string().contains("rebuilds"), "{err}");
    assert!(nothing_stored(&storage, &id).await, "bad delta went live");
}

#[tokio::test]
async fn delta_that_rebuilds_the_chunk_is_stored() {
    let (odb, _storage, base, base_id) = setup().await;
    let mut target = base.clone();
    target.extend_from_slice(b" tail");
    let id = Oid::hash(&target);
    odb.write_chunk_delta(&id, &base_id, &payload(&base, &target))
        .await
        .unwrap();
    assert_eq!(odb.get_chunk(&id).await.unwrap(), target);
}

#[tokio::test]
async fn delta_against_an_unreadable_base_is_refused_and_not_stored() {
    let (odb, storage, base, _) = setup().await;
    let mut target = base.clone();
    target.extend_from_slice(b" tail");
    let id = Oid::hash(&target);
    let missing = Oid::hash(b"no such base");
    assert!(
        odb.write_chunk_delta(&id, &missing, &payload(&base, &target))
            .await
            .is_err()
    );
    assert!(nothing_stored(&storage, &id).await);
}
