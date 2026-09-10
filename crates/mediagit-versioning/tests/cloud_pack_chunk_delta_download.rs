// SPDX-License-Identifier: BUSL-1.1
// Copyright (C) 2025-2026 Aswin Krishnamoorthy

//! Regression test: server-side chunk reads (`ObjectDatabase::get_chunk`) must
//! find a chunk-delta's BASE chunk when that base only exists inside a Track F
//! "cloud pack" object (`packs/<pack_oid>`, no `.pack` extension) rather than
//! as a loose `chunks/<id>` object or a legacy `gc --repack` `.pack` file.
//!
//! Bug: `ObjectDatabase::list_pack_files` (odb/chunks.rs) filtered storage
//! keys under `packs/` to only those ending in `.pack` — the legacy repack
//! format. Cloud packs (Track F, client push default `MEDIAGIT_CLOUD_PACKS=1`)
//! are stored at `packs/<pack_oid_hex>` with no extension, so they were
//! silently excluded from the pack search. `read_from_packs` then reported
//! "no pack files" even though the pack object was present, and any
//! server-side raw-file download whose delta chain based off a cloud-packed
//! chunk (e.g. `GET /{repo}/files/{path}`, handled by
//! `mediagit-server::handlers::browse::download_file_by_path`) failed mid
//! stream.
//!
//! This test drives the same `ObjectDatabase::get_chunk` codepath the download
//! handler uses, without needing an HTTP server: it stores a base chunk only
//! inside a hand-built cloud-pack object and a delta chunk referencing it, the
//! way a real push (`MEDIAGIT_CLOUD_PACKS=1`, the default) would leave a repo.

use mediagit_compression::{SmartCompressor, TypeAwareCompressor};
use mediagit_storage::StorageBackend;
use mediagit_storage::mock::MockBackend;
use mediagit_versioning::{
    Delta, DeltaEncoder, ObjectDatabase, ObjectType, Oid, PackKind, StreamingPackWriter,
};
use std::sync::Arc;

// `mediagit_compression::ObjectType` (used by SmartCompressor) and
// `mediagit_versioning::ObjectType` (used by StreamingPackWriter, imported
// above) are distinct types with the same name.
type CompObjectType = mediagit_compression::ObjectType;

#[tokio::test]
async fn get_chunk_finds_delta_base_inside_cloud_pack() {
    let storage: Arc<dyn StorageBackend> = Arc::new(MockBackend::new());
    let odb = ObjectDatabase::with_smart_compression(storage.clone(), 100);
    let smart = SmartCompressor::new();

    // ── Base chunk: lives ONLY inside a cloud pack, never loose ──────────
    let base_payload =
        b"BASE CHUNK: the quick brown fox jumps over the lazy dog, repeated. ".repeat(64);
    let base_id = Oid::hash(&base_payload);
    let compressed_base = smart
        .compress_typed(&base_payload, CompObjectType::Unknown)
        .expect("compress base");

    let temp_dir = tempfile::TempDir::new().unwrap();
    let mut writer = StreamingPackWriter::new_open_ended(PackKind::CloudObject, temp_dir.path())
        .await
        .expect("open cloud pack writer");
    writer
        .write_object(base_id, ObjectType::Blob, &compressed_base)
        .await
        .expect("write base chunk into pack");
    let result = writer.finalize_cloud().await.expect("finalize cloud pack");
    let pack_bytes = tokio::fs::read(&result.temp_path).await.unwrap();
    let pack_oid_hex: String = result
        .pack_oid
        .iter()
        .map(|b| format!("{:02x}", b))
        .collect();

    // Cloud-pack storage key: `packs/<pack_oid>` — no `.pack` extension.
    // (Legacy `gc --repack` packs use `packs/<id>.pack`.)
    storage
        .put(&format!("packs/{}", pack_oid_hex), &pack_bytes)
        .await
        .expect("upload cloud pack object");

    // ── Delta chunk: a real Delta against the base, compressed the same
    // way the ODB compresses any chunk-delta payload before storing ──────
    let leaf_payload = {
        let mut v = base_payload.clone();
        v.extend_from_slice(b" -- appended tail bytes for the leaf version");
        v
    };
    let delta: Delta = DeltaEncoder::encode(&base_payload, &leaf_payload);
    let delta_id = Oid::hash(&leaf_payload);
    let compressed_delta = smart
        .compress_typed(&delta.to_bytes(), CompObjectType::Unknown)
        .expect("compress delta");
    odb.write_chunk_delta(&delta_id, &base_id, &compressed_delta)
        .await
        .expect("write chunk delta");

    // Sanity: base is NOT loose.
    assert!(
        !storage
            .exists(&format!("chunks/{}", base_id.to_hex()))
            .await
            .unwrap(),
        "test setup invariant broken: base chunk must not be loose"
    );

    // ── The actual regression: get_chunk must reconstruct the leaf by
    // finding the base inside the cloud pack. ────────────────────────────
    let reconstructed = odb
        .get_chunk(&delta_id)
        .await
        .expect("get_chunk must locate the delta's base chunk inside the cloud pack");
    assert_eq!(
        reconstructed, leaf_payload,
        "reconstructed chunk bytes must match the original leaf payload"
    );
}
