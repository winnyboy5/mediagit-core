// SPDX-License-Identifier: BUSL-1.1
// Copyright (C) 2025-2026 Aswin Krishnamoorthy

//! Reads of chunks that only live inside packs (after `gc --repack`) must be
//! one ranged read each, not a whole-pack fetch + checksum per chunk.

// edition-2024: test-only env::set_var/remove_var requires unsafe
#![allow(unsafe_code)]
// ENV_LOCK is held across awaits on purpose: each #[tokio::test] has its own
// runtime, and the lock only serialises the process-global env vars.
#![allow(clippy::await_holding_lock)]

use mediagit_storage::{StorageBackend, mock::MockBackend};
use mediagit_versioning::{ChunkStrategy, ObjectDatabase, ObjectType};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

/// `MEDIAGIT_PACK_CHUNKS` / `MEDIAGIT_REPACK_CHUNKS` / `MEDIAGIT_VIDEO_CHUNKER`
/// are process-global; every test holds this for its whole body.
static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Counts whole-object `get`s of `packs/` keys and ranged reads of them.
#[derive(Debug)]
struct CountingBackend {
    inner: Arc<MockBackend>,
    pack_gets: Arc<AtomicUsize>,
    pack_ranges: Arc<AtomicUsize>,
}

#[async_trait::async_trait]
impl StorageBackend for CountingBackend {
    async fn get(&self, key: &str) -> anyhow::Result<Vec<u8>> {
        if key.starts_with("packs/") {
            self.pack_gets.fetch_add(1, Ordering::SeqCst);
        }
        self.inner.get(key).await
    }
    async fn get_range(&self, key: &str, offset: u64, len: u64) -> anyhow::Result<Vec<u8>> {
        if key.starts_with("packs/") {
            self.pack_ranges.fetch_add(1, Ordering::SeqCst);
        }
        self.inner.get_range(key, offset, len).await
    }
    async fn put(&self, key: &str, data: &[u8]) -> anyhow::Result<()> {
        self.inner.put(key, data).await
    }
    async fn exists(&self, key: &str) -> anyhow::Result<bool> {
        self.inner.exists(key).await
    }
    async fn delete(&self, key: &str) -> anyhow::Result<()> {
        self.inner.delete(key).await
    }
    async fn list_objects(&self, prefix: &str) -> anyhow::Result<Vec<String>> {
        self.inner.list_objects(prefix).await
    }
    async fn head(&self, key: &str) -> anyhow::Result<Option<u64>> {
        self.inner.head(key).await
    }
}

struct Fixture {
    mock: Arc<MockBackend>,
    pack_gets: Arc<AtomicUsize>,
    pack_ranges: Arc<AtomicUsize>,
}

impl Fixture {
    fn new() -> Self {
        Fixture {
            mock: Arc::new(MockBackend::new()),
            pack_gets: Arc::new(AtomicUsize::new(0)),
            pack_ranges: Arc::new(AtomicUsize::new(0)),
        }
    }

    /// A fresh ODB (empty caches, empty pack locator) over the shared storage.
    fn odb(&self) -> ObjectDatabase {
        let storage: Arc<dyn StorageBackend> = Arc::new(CountingBackend {
            inner: self.mock.clone(),
            pack_gets: self.pack_gets.clone(),
            pack_ranges: self.pack_ranges.clone(),
        });
        ObjectDatabase::with_optimizations(
            storage,
            100,
            Some(ChunkStrategy::Fixed { size: 64 * 1024 }),
            false,
            0,
        )
    }
}

fn payload(seed: usize, len: usize) -> Vec<u8> {
    (0..len)
        .map(|i| ((i.wrapping_mul(seed) ^ (i >> 3)) % 251) as u8)
        .collect()
}

/// Write `data` chunked, repack everything into small packs, and return its oid.
async fn write_and_repack(
    odb: &ObjectDatabase,
    data: &[u8],
    name: &str,
) -> mediagit_versioning::Oid {
    let oid = odb
        .write_chunked(ObjectType::Blob, data, name)
        .await
        .expect("write_chunked");
    odb.repack(0, true).await.expect("repack");
    oid
}

async fn restore(odb: &ObjectDatabase, oid: &mediagit_versioning::Oid) -> anyhow::Result<Vec<u8>> {
    let dir = tempfile::TempDir::new()?;
    let dest = dir.path().join("out.bin");
    odb.read_to_file(oid, &dest).await?;
    Ok(std::fs::read(&dest)?)
}

/// Cost regression: restoring N packed chunks must not fetch whole packs.
#[tokio::test]
async fn packed_restore_uses_ranged_reads_not_whole_pack_gets() {
    let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    // SAFETY: serialised by ENV_LOCK.
    unsafe {
        std::env::remove_var("MEDIAGIT_REPACK_CHUNKS");
        std::env::set_var("MEDIAGIT_PACK_CHUNKS", "4");
    }

    let fx = Fixture::new();
    let data = payload(7, 2 * 1024 * 1024); // 32 chunks -> 8 packs of 4
    let oid = write_and_repack(&fx.odb(), &data, "big.bin").await;

    let pack_keys: Vec<String> = fx
        .mock
        .list_objects("packs/")
        .await
        .unwrap()
        .into_iter()
        .filter(|k| !k.ends_with(".jsonl"))
        .collect();
    assert!(pack_keys.len() > 1, "fixture must span several packs");

    fx.pack_gets.store(0, Ordering::SeqCst);
    let restored = restore(&fx.odb(), &oid).await.unwrap();
    assert_eq!(restored, data, "restore must be byte-identical");
    assert_eq!(
        fx.pack_gets.load(Ordering::SeqCst),
        0,
        "no whole-pack get may be needed to restore packed chunks"
    );
    assert!(fx.pack_ranges.load(Ordering::SeqCst) > 0);

    unsafe { std::env::remove_var("MEDIAGIT_PACK_CHUNKS") };
}

/// Integrity: a flipped byte inside one packed entry fails that chunk with a
/// hash error, while the other chunks of the same pack still read.
#[tokio::test]
async fn corrupt_packed_entry_fails_only_that_chunk() {
    let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    unsafe {
        std::env::remove_var("MEDIAGIT_REPACK_CHUNKS");
        std::env::set_var("MEDIAGIT_PACK_CHUNKS", "16");
    }

    let fx = Fixture::new();
    let data = payload(11, 1024 * 1024); // 16 chunks, one pack
    let writer = fx.odb();
    let oid = write_and_repack(&writer, &data, "c.bin").await;
    let manifest = writer.get_chunk_manifest(&oid).await.unwrap().unwrap();
    assert_eq!(manifest.chunks.len(), 16);

    let pack_key = fx
        .mock
        .list_objects("packs/")
        .await
        .unwrap()
        .into_iter()
        .find(|k| !k.ends_with(".jsonl"))
        .expect("a pack");
    let mut pack = fx.mock.get(&pack_key).await.unwrap();
    // 13-byte header, then entries: flip a byte inside the first entry's payload.
    pack[13 + 5 + 16] ^= 0xFF;
    fx.mock.put(&pack_key, &pack).await.unwrap();

    let odb = fx.odb();
    let mut failed = 0;
    let mut ok = 0;
    for chunk in &manifest.chunks {
        match odb.get_chunk(&chunk.id).await {
            Ok(bytes) => {
                assert_eq!(mediagit_versioning::Oid::hash(&bytes), chunk.id);
                ok += 1;
            }
            Err(e) => {
                assert!(
                    e.to_string().to_lowercase().contains("integrity")
                        || e.to_string().to_lowercase().contains("hash"),
                    "expected a hash error, got: {e}"
                );
                failed += 1;
            }
        }
    }
    assert_eq!(failed, 1, "exactly the corrupted entry must fail");
    assert_eq!(ok, 15, "other chunks of the same pack must still restore");
    assert!(restore(&fx.odb(), &oid).await.is_err());

    unsafe { std::env::remove_var("MEDIAGIT_PACK_CHUNKS") };
}

/// Staleness: packs added after the locator was built (by this ODB or another
/// process) are found, and a second repack with new pack keys still restores.
#[tokio::test]
async fn locator_sees_packs_added_after_it_was_built() {
    let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    unsafe {
        std::env::remove_var("MEDIAGIT_REPACK_CHUNKS");
        std::env::set_var("MEDIAGIT_PACK_CHUNKS", "4");
    }

    let fx = Fixture::new();
    let a = payload(3, 1024 * 1024);
    let b = payload(5, 1024 * 1024);
    let c = payload(13, 1024 * 1024);

    let odb1 = fx.odb();
    let oid_a = write_and_repack(&odb1, &a, "a.bin").await;
    assert_eq!(restore(&odb1, &oid_a).await.unwrap(), a); // builds the locator

    // Same ODB: repack again (new pack keys) -> locator is reset and rebuilt.
    let oid_b = write_and_repack(&odb1, &b, "b.bin").await;
    assert_eq!(restore(&odb1, &oid_b).await.unwrap(), b);
    assert_eq!(restore(&odb1, &oid_a).await.unwrap(), a);

    // Another ODB (another process) adds packs behind odb1's back.
    let odb2 = fx.odb();
    let oid_c = write_and_repack(&odb2, &c, "c.bin").await;
    assert_eq!(
        restore(&odb1, &oid_c).await.unwrap(),
        c,
        "miss must refresh"
    );
    assert_eq!(restore(&odb1, &oid_a).await.unwrap(), a);

    unsafe { std::env::remove_var("MEDIAGIT_PACK_CHUNKS") };
}

/// Byte-identical restore after repack for a synthetic MP4 with both video
/// chunkers (frame falls back to CDC if the synthetic file isn't parseable).
#[tokio::test]
async fn synthetic_mp4_restores_byte_identical_after_repack() {
    let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    unsafe {
        std::env::remove_var("MEDIAGIT_REPACK_CHUNKS");
        std::env::set_var("MEDIAGIT_PACK_CHUNKS", "2");
    }

    // ftyp + mdat (2 MiB of pseudo-random "frames") + empty moov.
    let mut mp4 = Vec::new();
    mp4.extend_from_slice(&24u32.to_be_bytes());
    mp4.extend_from_slice(b"ftypisom");
    mp4.extend_from_slice(&0u32.to_be_bytes());
    mp4.extend_from_slice(b"isomiso2");
    let body = payload(17, 2 * 1024 * 1024);
    mp4.extend_from_slice(&((body.len() as u32 + 8).to_be_bytes()));
    mp4.extend_from_slice(b"mdat");
    mp4.extend_from_slice(&body);
    mp4.extend_from_slice(&8u32.to_be_bytes());
    mp4.extend_from_slice(b"moov");

    for mode in ["frame", "cdc"] {
        unsafe { std::env::set_var("MEDIAGIT_VIDEO_CHUNKER", mode) };
        let fx = Fixture::new();
        let odb = fx.odb();
        let oid = write_and_repack(&odb, &mp4, "clip.mp4").await;
        let restored = restore(&fx.odb(), &oid).await.unwrap();
        assert_eq!(
            restored, mp4,
            "chunker={mode}: restore must be byte-identical"
        );
    }

    unsafe {
        std::env::remove_var("MEDIAGIT_VIDEO_CHUNKER");
        std::env::remove_var("MEDIAGIT_PACK_CHUNKS");
    }
}

fn raw_pack(
    entries: &[(mediagit_versioning::Oid, &[u8])],
    deltas: &[(mediagit_versioning::Oid, mediagit_versioning::Oid, Vec<u8>)],
) -> Vec<u8> {
    let mut w = mediagit_versioning::PackWriter::new();
    for (oid, data) in entries {
        w.add_object(*oid, ObjectType::Blob, data);
    }
    for (oid, base, delta) in deltas {
        w.add_delta_object(*oid, *base, delta);
    }
    w.finalize()
}

/// Delta entries (a chain: base <- d1 <- d2) restore through the locator with
/// ranged reads only: no whole-pack get, same bytes `PackReader` produces.
#[tokio::test]
async fn delta_entries_resolve_without_whole_pack_gets() {
    use mediagit_versioning::{DeltaEncoder, Oid};
    let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());

    let mut base = payload(19, 20_000);
    base[0] = 0xAB; // not a compression-framing byte: stored raw in the pack
    let mut d1 = base.clone();
    d1[5_000] ^= 0xFF;
    let mut d2 = d1.clone();
    d2[15_000] ^= 0xFF;
    let (b_oid, d1_oid, d2_oid) = (Oid::hash(&base), Oid::hash(&d1), Oid::hash(&d2));
    let pack = raw_pack(
        &[(b_oid, &base)],
        &[
            (d1_oid, b_oid, DeltaEncoder::encode(&base, &d1).to_bytes()),
            (d2_oid, d1_oid, DeltaEncoder::encode(&d1, &d2).to_bytes()),
        ],
    );

    // The pre-existing whole-pack reader must agree with the expected bytes.
    let reader = mediagit_versioning::PackReader::new(pack.clone()).unwrap();
    assert_eq!(reader.get_object(&d2_oid).unwrap(), d2);

    let fx = Fixture::new();
    fx.mock.put("packs/delta.pack", &pack).await.unwrap();
    let odb = fx.odb();
    assert_eq!(odb.read(&d1_oid).await.unwrap(), d1);
    assert_eq!(odb.read(&d2_oid).await.unwrap(), d2);
    assert_eq!(
        fx.pack_gets.load(Ordering::SeqCst),
        0,
        "delta resolution must not fetch the whole pack"
    );
}

/// The same oid packed twice: a corrupt first copy falls through to the second;
/// only when every copy is corrupt does the read fail, with a hash error.
#[tokio::test]
async fn duplicate_oid_in_two_packs_falls_back_to_intact_copy() {
    use mediagit_versioning::Oid;
    let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());

    let mut data = payload(23, 10_000);
    data[0] = 0xAB; // not a compression-framing byte: stored raw in the pack
    let oid = Oid::hash(&data);
    let pack = raw_pack(&[(oid, &data)], &[]);
    let corrupt = |mut p: Vec<u8>| {
        p[13 + 5 + 100] ^= 0xFF; // inside the entry payload
        p
    };

    // Whichever copy the locator tries first, the intact one must win.
    for (a, b) in [(true, false), (false, true)] {
        let fx = Fixture::new();
        fx.mock
            .put(
                "packs/a.pack",
                &if a {
                    corrupt(pack.clone())
                } else {
                    pack.clone()
                },
            )
            .await
            .unwrap();
        fx.mock
            .put(
                "packs/b.pack",
                &if b {
                    corrupt(pack.clone())
                } else {
                    pack.clone()
                },
            )
            .await
            .unwrap();
        assert_eq!(fx.odb().read(&oid).await.unwrap(), data);
    }

    let fx = Fixture::new();
    fx.mock
        .put("packs/a.pack", &corrupt(pack.clone()))
        .await
        .unwrap();
    fx.mock
        .put("packs/b.pack", &corrupt(pack.clone()))
        .await
        .unwrap();
    let err = fx
        .odb()
        .read(&oid)
        .await
        .unwrap_err()
        .to_string()
        .to_lowercase();
    assert!(
        err.contains("integrity"),
        "expected a hash error, got: {err}"
    );
}
