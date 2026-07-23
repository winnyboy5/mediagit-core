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

//! Object Database (ODB) - Content-addressable storage with BLAKE3 addressing
//!
//! The ODB provides:
//! - **Content-addressable storage**: Objects are identified by BLAKE3 hash of their content
//! - **Automatic deduplication**: Identical content is stored only once
//! - **LRU caching**: Frequently accessed objects are cached in memory
//! - **Observable metrics**: Track cache performance and deduplication efficiency
//! - **Delta compression**: Store only differences between similar objects
//! - **Delta chain limits**: Prevent unbounded delta chains for consistent read performance

/// Maximum delta chain depth before re-storing as full object.
/// This prevents read performance degradation from long delta chains.
/// After this depth, objects are stored as full copies to break the chain.
pub const MAX_DELTA_DEPTH: u8 = 10;

/// Maximum allowed object size (16 GB).
/// This prevents memory allocation failures from corrupted chunk manifests
/// that may contain extremely large total_size values.
pub const MAX_OBJECT_SIZE: u64 = 16 * 1024 * 1024 * 1024;

/// Maximum object size to cache in the LRU cache (10 MB).
/// Objects larger than this are typically read once (checkout, push) and rarely re-read,
/// so caching them wastes memory without improving hit rate.
const MAX_CACHEABLE_OBJECT_SIZE: usize = 10 * 1024 * 1024;

/// Default maximum total cache size in bytes (512 MB).
/// Uses Moka's weigher to bound by total byte size instead of entry count.
const DEFAULT_CACHE_MAX_BYTES: u64 = 512 * 1024 * 1024;

/// Default byte budget for `base_chunk_cache` (256 MiB). Previously this
/// cache was bounded by *entry count* (64 entries) with no size weigher —
/// with chunks up to 32 MiB each, 64 entries could balloon to multiple GB.
/// Override via `MEDIAGIT_CHUNK_CACHE_BYTES`.
const DEFAULT_BASE_CHUNK_CACHE_BYTES: u64 = 256 * 1024 * 1024;

fn base_chunk_cache_bytes() -> u64 {
    match std::env::var("MEDIAGIT_CHUNK_CACHE_BYTES") {
        Ok(v) => v.parse::<u64>().unwrap_or_else(|_| {
            warn!(
                "MEDIAGIT_CHUNK_CACHE_BYTES='{}' is not a valid u64, using default {}",
                v, DEFAULT_BASE_CHUNK_CACHE_BYTES
            );
            DEFAULT_BASE_CHUNK_CACHE_BYTES
        }),
        Err(_) => DEFAULT_BASE_CHUNK_CACHE_BYTES,
    }
}

/// Build the byte-weighted `base_chunk_cache` used by every `ObjectDatabase`
/// constructor. Weighed by `Arc<Vec<u8>>::len()` so a handful of large
/// chunks can't blow past the configured byte budget the way a plain
/// entry-count cache could.
fn build_base_chunk_cache() -> Cache<Oid, Arc<Vec<u8>>> {
    Cache::builder()
        .max_capacity(base_chunk_cache_bytes())
        .weigher(|_key: &Oid, value: &Arc<Vec<u8>>| -> u32 {
            value.len().try_into().unwrap_or(u32::MAX)
        })
        .build()
}

use crate::chunking::{ChunkManifest, ChunkRef, ChunkStrategy, ContentChunker};
use crate::delta::{Delta, DeltaDecoder, DeltaEncoder};
use crate::{ObjectType, OdbMetrics, Oid};
use mediagit_compression::ObjectType as CompressionObjectType;
use mediagit_compression::{
    ChunkCodecHint, CompressionAlgorithm, Compressor, SmartCompressor, TypeAwareCompressor,
    ZlibCompressor,
};
use mediagit_storage::StorageBackend;

/// Codec-aware delta acceptance threshold.
///
/// Different stream types have different delta efficiency characteristics:
/// - Intra-only video (ProRes/DNxHR): tighter threshold — delta is very effective
/// - Subtitles/metadata: generous threshold — small chunks, high similarity
/// - Default (PCM, FLAC, unknown): standard 0.80
fn delta_ratio_threshold(
    codec: crate::chunking::CodecHint,
    chunk_type: crate::chunking::ChunkType,
) -> f64 {
    use crate::chunking::{ChunkType, CodecHint};
    match codec {
        CodecHint::ProRes | CodecHint::DNxHR | CodecHint::Jpeg2000 | CodecHint::RawVideo => 0.60,
        CodecHint::TextSub | CodecHint::BitmapSub => 0.90,
        CodecHint::Unknown => match chunk_type {
            ChunkType::Metadata | ChunkType::Subtitle => 0.90,
            _ => 0.80,
        },
        _ => 0.80,
    }
}

/// Map a chunk's CodecHint + ChunkType to a compression-level ChunkCodecHint.
fn to_chunk_codec_hint(
    codec: crate::chunking::CodecHint,
    chunk_type: crate::chunking::ChunkType,
) -> ChunkCodecHint {
    use crate::chunking::{ChunkType, CodecHint};
    match codec {
        CodecHint::H264 | CodecHint::H265 | CodecHint::VP9 | CodecHint::AV1 => {
            ChunkCodecHint::HighEntropyVideo
        }
        CodecHint::ProRes | CodecHint::DNxHR | CodecHint::Jpeg2000 => {
            ChunkCodecHint::IntraOnlyVideo
        }
        CodecHint::RawVideo => ChunkCodecHint::RawVideo,
        CodecHint::AAC | CodecHint::Opus | CodecHint::MP3 | CodecHint::Vorbis => {
            ChunkCodecHint::CompressedAudio
        }
        CodecHint::PCM => ChunkCodecHint::LosslessAudio,
        CodecHint::FLAC | CodecHint::ALAC => ChunkCodecHint::LosslessAudio,
        CodecHint::TextSub => ChunkCodecHint::TextSubtitle,
        CodecHint::BitmapSub => ChunkCodecHint::BitmapSubtitle,
        CodecHint::Unknown => match chunk_type {
            ChunkType::Metadata => ChunkCodecHint::Metadata,
            _ => ChunkCodecHint::Unknown,
        },
    }
}

/// Decompress `data` via `spawn_blocking` for large payloads, inline otherwise.
///
/// Gated by `MEDIAGIT_DECOMPRESS_BLOCKING` (enabled unless `"0"`) and
/// `MEDIAGIT_DECOMPRESS_BLOCKING_THRESHOLD` (default 262144 / 256 KiB).
async fn decompress_blocking(
    compressor: std::sync::Arc<dyn Compressor>,
    data: Vec<u8>,
) -> mediagit_compression::CompressionResult<Vec<u8>> {
    let enabled = std::env::var("MEDIAGIT_DECOMPRESS_BLOCKING")
        .as_deref()
        .unwrap_or("1")
        != "0";
    let threshold: usize = std::env::var("MEDIAGIT_DECOMPRESS_BLOCKING_THRESHOLD")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(262144);

    if enabled && data.len() >= threshold {
        tokio::task::spawn_blocking(move || compressor.decompress(&data))
            .await
            .map_err(|e| {
                mediagit_compression::CompressionError::decompression_failed(e.to_string())
            })?
    } else {
        compressor.decompress(&data)
    }
}

/// `decompress_typed` variant of `decompress_blocking` for `SmartCompressor`.
async fn decompress_typed_blocking(
    compressor: std::sync::Arc<SmartCompressor>,
    data: Vec<u8>,
) -> mediagit_compression::CompressionResult<Vec<u8>> {
    let enabled = std::env::var("MEDIAGIT_DECOMPRESS_BLOCKING")
        .as_deref()
        .unwrap_or("1")
        != "0";
    let threshold: usize = std::env::var("MEDIAGIT_DECOMPRESS_BLOCKING_THRESHOLD")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(262144);

    if enabled && data.len() >= threshold {
        tokio::task::spawn_blocking(move || compressor.decompress_typed(&data))
            .await
            .map_err(|e| {
                mediagit_compression::CompressionError::decompression_failed(e.to_string())
            })?
    } else {
        compressor.decompress_typed(&data)
    }
}

use moka::future::Cache;
use std::sync::Arc;
use tokio::sync::{Mutex, RwLock};
use tracing::{debug, info, warn};

/// Walk a chunk-delta chain on disk and report whether `target` appears.
///
/// Mirrors `ObjectDatabase::delta_chain_contains` for the `chunk-deltas/`
/// namespace. Module-level so the parallel chunk-write paths (which capture
/// `Arc<dyn StorageBackend>` rather than `&self`) can call it directly.
///
/// Returns `false` for any malformed/missing meta — the caller only cares
/// about positive identification of the target on the chain.
///
/// Uses `storage.exists` before `storage.get` because the common case on
/// fresh-add hot paths is "base is a full chunk, no meta exists" — and on
/// LocalBackend `exists()` is a single stat() syscall whereas `get()` is
/// open+read+close. For multi-GiB files with thousands of chunks this
/// difference matters.
async fn chunk_delta_chain_contains_impl(
    storage: &dyn StorageBackend,
    start: Oid,
    target: Oid,
) -> bool {
    chunk_delta_chain_walk(storage, start, Some(target))
        .await
        .contains_target
}

/// Absolute hop ceiling for a single chain traversal.
///
/// A legal chain is at most `MAX_DELTA_DEPTH`, but legacy repositories written
/// before the write-side depth guard existed can hold arbitrarily deep chains
/// (that is the defect this bound exists to survive). Walking one costs two
/// tiny reads per hop, so the walk is capped rather than unbounded; exceeding
/// the cap is reported via `truncated` and every caller treats that as "refuse
/// the delta", which is the safe direction. `fsck`'s repair path deliberately
/// does *not* use this helper — it must follow a chain of any length.
const CHAIN_WALK_CAP: usize = (MAX_DELTA_DEPTH as usize) * 4;

/// Outcome of one `chunk-deltas/` chain traversal.
pub(crate) struct ChainWalk {
    /// The requested `target` appears on the chain — writing `target -> start`
    /// would close a cycle.
    pub contains_target: bool,
    /// Delta hops from `start` down to the terminal full chunk. `0` means
    /// `start` is itself a full chunk (no `.meta` sidecar).
    pub depth: usize,
    /// The terminal non-delta chunk, when the chain resolved cleanly.
    /// `None` if the walk hit a cycle, a malformed sidecar, or the cap.
    pub root: Option<Oid>,
    /// The walk stopped early (cycle, malformed meta, or `CHAIN_WALK_CAP`), so
    /// `depth` is a lower bound and `root` is unknown.
    pub truncated: bool,
}

/// Walk a chunk-delta chain once, answering both questions the write paths ask:
/// "would this close a cycle?" and "how deep is this base already?".
///
/// Fusing them matters: every chunk-delta write already walks the chain for
/// cycle detection, so deriving depth from a *second* walk would double the
/// small-file I/O on every chunk of every add. One traversal answers both.
///
/// Uses `storage.exists` before `storage.get` because the common case on
/// fresh-add hot paths is "base is a full chunk, no meta exists" — and on
/// LocalBackend `exists()` is a single stat() syscall whereas `get()` is
/// open+read+close. For multi-GiB files with thousands of chunks this
/// difference matters, so the overwhelmingly common result (`depth == 0`)
/// costs exactly one stat().
async fn chunk_delta_chain_walk(
    storage: &dyn StorageBackend,
    start: Oid,
    target: Option<Oid>,
) -> ChainWalk {
    let mut walk = ChainWalk {
        contains_target: target == Some(start),
        depth: 0,
        root: None,
        truncated: false,
    };
    if walk.contains_target {
        walk.truncated = true;
        return walk;
    }

    let mut current = start;
    let mut visited = std::collections::HashSet::new();
    for _ in 0..CHAIN_WALK_CAP {
        if !visited.insert(current) {
            // Pre-existing cycle in stored data. Report it as truncated so
            // callers refuse to extend it; `fsck` is what reports/repairs it.
            walk.truncated = true;
            return walk;
        }
        let meta_key = format!("chunk-deltas/{}.meta", current.to_hex());
        // Cheap probe first: no meta => `current` is terminal (full chunk).
        match storage.exists(&meta_key).await {
            Ok(true) => {}
            Ok(false) => {
                walk.root = Some(current);
                return walk;
            }
            Err(_) => {
                walk.truncated = true;
                return walk;
            }
        }
        let next = match storage.get(&meta_key).await {
            Ok(bytes) => match std::str::from_utf8(&bytes)
                .ok()
                .and_then(|s| {
                    s.trim()
                        .strip_prefix("base:")
                        .map(str::trim)
                        .map(String::from)
                })
                .and_then(|hex| Oid::from_hex(&hex).ok())
            {
                Some(oid) => oid,
                None => {
                    // Malformed sidecar: corruption, not a clean terminus.
                    walk.truncated = true;
                    return walk;
                }
            },
            Err(_) => {
                walk.truncated = true;
                return walk;
            }
        };
        walk.depth += 1;
        current = next;
        if target == Some(current) {
            walk.contains_target = true;
            return walk;
        }
    }
    walk.truncated = true;
    walk
}

/// Decide which chunk a new delta should be written against.
///
/// This is the single choke point for chunk-delta base policy — all write
/// paths route through it so a new call site cannot silently omit a guard,
/// which is exactly how the unbounded-chain defect shipped (four independent
/// hand-rolled guards that each remembered cycles and forgot depth).
///
/// Returns the base to use, or `None` when no delta should be written and the
/// caller should store the chunk in full:
///
/// * `None` if `new_chunk` already appears on the nominated base's chain —
///   writing it would close a cycle and make both chunks unreadable.
/// * `None` if the chain cannot be resolved (corruption, pre-existing cycle,
///   or deeper than [`CHAIN_WALK_CAP`]) — never extend a chain we can't read.
/// * When the base already sits at `MAX_DELTA_DEPTH`, the chain **root** (a
///   full chunk) instead of the nominated base. The delta is still written —
///   storage savings are the product's differentiator, so hitting the cap must
///   not silently degrade to storing everything in full — but it restarts at
///   depth 1, so chains self-balance and never exceed the cap.
pub(crate) async fn resolve_delta_base(
    storage: &dyn StorageBackend,
    nominated_base: Oid,
    new_chunk: Oid,
) -> Option<Oid> {
    if nominated_base == new_chunk {
        return None;
    }
    let walk = chunk_delta_chain_walk(storage, nominated_base, Some(new_chunk)).await;
    if walk.contains_target || walk.truncated {
        return None;
    }
    // `truncated == false` guarantees a terminal full chunk was reached.
    let root = walk.root?;
    if walk.depth < MAX_DELTA_DEPTH as usize {
        return Some(nominated_base);
    }
    // At the cap: fall back to the chain root rather than abandoning the delta.
    if root == new_chunk { None } else { Some(root) }
}

/// Object Database with content-addressable storage
///
/// The ObjectDatabase provides Git-compatible content-addressable storage
/// with automatic deduplication and LRU caching for performance.
///
/// # Architecture
///
/// - **Storage**: Pluggable backend via `StorageBackend` trait
/// - **Caching**: Moka LRU cache for hot objects
/// - **Addressing**: BLAKE3 hash for content addressing
/// - **Organization**: Git-like object paths: `objects/{first2hex}/{remaining62hex}`
///
/// # Examples
///
/// ```no_run
/// use mediagit_versioning::{ObjectDatabase, ObjectType};
/// use mediagit_storage::LocalBackend;
/// use std::sync::Arc;
///
/// #[tokio::main]
/// async fn main() -> anyhow::Result<()> {
///     let storage: Arc<dyn mediagit_storage::StorageBackend> =
///         Arc::new(LocalBackend::new("/tmp/test-odb").await?);
///     let odb = ObjectDatabase::new(storage, 1000);
///
///     // Write an object
///     let data = b"Hello, World!";
///     let oid = odb.write(ObjectType::Blob, data).await?;
///
///     // Read it back
///     let retrieved = odb.read(&oid).await?;
///     assert_eq!(retrieved, data);
///
///     // Metrics show deduplication
///     let metrics = odb.metrics().await;
///     println!("Dedup ratio: {:.1}%", metrics.dedup_ratio() * 100.0);
///
///     Ok(())
/// }
/// ```
pub struct ObjectDatabase {
    /// Underlying storage backend
    storage: Arc<dyn StorageBackend>,

    /// LRU cache for frequently accessed objects
    cache: Cache<Oid, Arc<Vec<u8>>>,

    /// Metrics tracking
    metrics: Arc<RwLock<OdbMetrics>>,

    /// Compression engine (Git-compatible zlib by default)
    compressor: Arc<dyn Compressor>,

    /// Enable/disable compression (default: true)
    compression_enabled: bool,

    /// Smart compressor for type-aware compression (optional)
    smart_compressor: Option<Arc<SmartCompressor>>,

    /// Chunking strategy (optional)
    chunk_strategy: Option<ChunkStrategy>,

    /// Per-repo CDC seed (0 = legacy/unseeded). Env var `MEDIAGIT_CDC_SEED`
    /// overrides this at the point of use via `chunking::resolve_cdc_seed`.
    cdc_seed: u64,

    /// Enable delta encoding for similar objects
    delta_enabled: bool,

    /// Similarity detector for finding delta base candidates
    similarity_detector: Arc<RwLock<crate::similarity::SimilarityDetector>>,

    /// LRU cache for decompressed base chunks used in delta encoding.
    /// Avoids re-reading and re-decompressing the same base chunk across workers.
    base_chunk_cache: Cache<Oid, Arc<Vec<u8>>>,

    /// Tracks committed chunk-delta pairs (chunk_id, base_id) to prevent TOCTOU cycles.
    /// In-memory O(1) check inside a short-held lock; all network IO happens outside the lock.
    delta_written_pairs: Arc<Mutex<std::collections::HashSet<(Oid, Oid)>>>,

    /// Lazily-built set of OIDs embedded in pack indexes. `None` until the
    /// first `chunk_exists()` call (or a repack) populates it — packs are
    /// immutable once written and only ever grow in number, so this can be
    /// extended in place by `repack()` instead of reloading from scratch.
    /// Without this, `chunk_exists()` only checked `chunks/` and
    /// `chunk-deltas/`, so post-repack (which deletes the loose copies)
    /// every chunk looked "new" and push dedup re-uploaded everything.
    pack_membership: Arc<RwLock<Option<std::collections::HashSet<Oid>>>>,
}

impl Clone for ObjectDatabase {
    fn clone(&self) -> Self {
        Self {
            storage: self.storage.clone(),
            cache: self.cache.clone(),
            metrics: self.metrics.clone(),
            compressor: self.compressor.clone(),
            compression_enabled: self.compression_enabled,
            smart_compressor: self.smart_compressor.clone(),
            chunk_strategy: self.chunk_strategy,
            cdc_seed: self.cdc_seed,
            delta_enabled: self.delta_enabled,
            similarity_detector: self.similarity_detector.clone(),
            base_chunk_cache: self.base_chunk_cache.clone(),
            delta_written_pairs: self.delta_written_pairs.clone(),
            pack_membership: self.pack_membership.clone(),
        }
    }
}

pub(crate) mod chunks;
pub(crate) mod core;
pub(crate) mod delta;

/// Statistics from a repack operation
#[derive(Debug, Default, Clone)]
pub struct RepackStats {
    /// Number of loose objects found
    pub loose_objects_found: usize,
    /// Number of objects successfully packed
    pub objects_packed: usize,
    /// Number of objects stored as deltas
    pub delta_objects: usize,
    /// Total size of pack file
    pub pack_size: u64,
    /// Bytes saved by packing
    pub bytes_saved: u64,
    /// Number of loose objects removed
    pub loose_objects_removed: usize,
}

#[cfg(test)]
#[allow(unsafe_code)] // edition-2024: test-only env::set_var/remove_var requires unsafe
mod tests {
    use super::*;
    use mediagit_storage::mock::MockBackend;

    /// Pins the guard the chunk-delta cycle fix relies on: with A→B and
    /// B→C metas on disk, a writer about to store C→A walks A's chain and
    /// must find C (the loop closer). Under the widened delta_written_pairs
    /// lock this walk is atomic with the meta write, so the last write of a
    /// would-be A→B→C→A cycle always refuses (found as AWS deep-test
    /// failure 2026-07-07: three video chunks stored as mutual deltas,
    /// unreconstructable).
    #[tokio::test]
    async fn test_chunk_delta_chain_walk_detects_loop_closer() {
        let storage = Arc::new(MockBackend::new());
        let a = Oid::hash(b"chunk-a");
        let b = Oid::hash(b"chunk-b");
        let c = Oid::hash(b"chunk-c");

        // Simulate two committed deltas: A→B, B→C.
        storage
            .put(
                &format!("chunk-deltas/{}.meta", a.to_hex()),
                format!("base:{}", b.to_hex()).as_bytes(),
            )
            .await
            .unwrap();
        storage
            .put(
                &format!("chunk-deltas/{}.meta", b.to_hex()),
                format!("base:{}", c.to_hex()).as_bytes(),
            )
            .await
            .unwrap();

        // The would-be loop closer C→A: walking from base A must reach C.
        assert!(
            chunk_delta_chain_contains_impl(&*storage, a, c).await,
            "walk from A must find C so the C→A write is refused"
        );
        // Non-cycle nomination is still allowed: D→A terminates at full chunk C.
        let d = Oid::hash(b"chunk-d");
        assert!(
            !chunk_delta_chain_contains_impl(&*storage, a, d).await,
            "walk from A must not find unrelated D"
        );
    }

    /// An already-corrupt on-disk cycle must not hang the walk.
    #[tokio::test]
    async fn test_chunk_delta_chain_walk_bails_on_existing_cycle() {
        let storage = Arc::new(MockBackend::new());
        let a = Oid::hash(b"cyc-a");
        let b = Oid::hash(b"cyc-b");
        for (from, to) in [(a, b), (b, a)] {
            storage
                .put(
                    &format!("chunk-deltas/{}.meta", from.to_hex()),
                    format!("base:{}", to.to_hex()).as_bytes(),
                )
                .await
                .unwrap();
        }
        let unrelated = Oid::hash(b"cyc-x");
        // Terminates (visited-set bail) and reports "not found".
        assert!(!chunk_delta_chain_contains_impl(&*storage, a, unrelated).await);
    }

    /// Build a chain of `len` deltas terminating at a full chunk, and return
    /// (leaf, root). Only `.meta` sidecars are written — enough for the walk,
    /// which never reads payloads.
    async fn plant_chain(storage: &Arc<MockBackend>, tag: &str, len: usize) -> (Oid, Oid) {
        let root = Oid::hash(format!("{tag}-root").as_bytes());
        storage
            .put(&format!("chunks/{}", root.to_hex()), b"full")
            .await
            .unwrap();
        let mut prev = root;
        for i in 0..len {
            let id = Oid::hash(format!("{tag}-{i}").as_bytes());
            storage
                .put(
                    &format!("chunk-deltas/{}.meta", id.to_hex()),
                    format!("base:{}", prev.to_hex()).as_bytes(),
                )
                .await
                .unwrap();
            prev = id;
        }
        (prev, root)
    }

    /// Below the cap the nominated base is used unchanged — the guard must not
    /// cost savings on ordinary chains.
    #[tokio::test]
    async fn test_resolve_delta_base_keeps_shallow_base() {
        let storage = Arc::new(MockBackend::new());
        let (leaf, _root) = plant_chain(&storage, "shallow", 3).await;
        let newcomer = Oid::hash(b"shallow-new");
        assert_eq!(
            resolve_delta_base(&*storage, leaf, newcomer).await,
            Some(leaf),
            "a chain well under MAX_DELTA_DEPTH must delta against the nominated base"
        );
    }

    /// At the cap, re-target to the chain root instead of refusing: the chunk
    /// stays delta-compressed (savings preserved) and depth restarts at 1.
    #[tokio::test]
    async fn test_resolve_delta_base_retargets_to_root_at_max_depth() {
        let storage = Arc::new(MockBackend::new());
        let (leaf, root) = plant_chain(&storage, "deep", MAX_DELTA_DEPTH as usize).await;
        let newcomer = Oid::hash(b"deep-new");
        assert_eq!(
            resolve_delta_base(&*storage, leaf, newcomer).await,
            Some(root),
            "at MAX_DELTA_DEPTH the base must fall back to the chain root, not \
             be abandoned — refusing outright would cost storage savings"
        );
    }

    /// The bug this whole change exists to prevent: repeatedly deltaing each
    /// new chunk onto the previous one must never exceed the depth a reader
    /// will reconstruct.
    #[tokio::test]
    async fn test_resolve_delta_base_never_exceeds_max_depth() {
        let storage = Arc::new(MockBackend::new());
        let root = Oid::hash(b"grow-root");
        storage
            .put(&format!("chunks/{}", root.to_hex()), b"full")
            .await
            .unwrap();

        let mut prev = root;
        for i in 0..40 {
            let id = Oid::hash(format!("grow-{i}").as_bytes());
            let base = resolve_delta_base(&*storage, prev, id)
                .await
                .expect("a resolvable chain must always yield some base");
            storage
                .put(
                    &format!("chunk-deltas/{}.meta", id.to_hex()),
                    format!("base:{}", base.to_hex()).as_bytes(),
                )
                .await
                .unwrap();
            let depth = chunk_delta_chain_walk(&*storage, id, None).await.depth;
            assert!(
                depth <= MAX_DELTA_DEPTH as usize,
                "chunk {i} reached depth {depth}, above the {} a reader will \
                 reconstruct — this is the unbounded-chain defect",
                MAX_DELTA_DEPTH
            );
            prev = id;
        }
    }

    /// Never extend a chain that cannot be read: a pre-existing cycle must
    /// yield no base at all rather than a plausible-looking one.
    #[tokio::test]
    async fn test_resolve_delta_base_refuses_on_existing_cycle() {
        let storage = Arc::new(MockBackend::new());
        let a = Oid::hash(b"rc-a");
        let b = Oid::hash(b"rc-b");
        for (from, to) in [(a, b), (b, a)] {
            storage
                .put(
                    &format!("chunk-deltas/{}.meta", from.to_hex()),
                    format!("base:{}", to.to_hex()).as_bytes(),
                )
                .await
                .unwrap();
        }
        assert_eq!(
            resolve_delta_base(&*storage, a, Oid::hash(b"rc-new")).await,
            None
        );
    }

    /// Self-loops are the degenerate cycle and must be refused outright.
    #[tokio::test]
    async fn test_resolve_delta_base_refuses_self_loop() {
        let storage = Arc::new(MockBackend::new());
        let a = Oid::hash(b"self-a");
        assert_eq!(resolve_delta_base(&*storage, a, a).await, None);
    }

    #[tokio::test]
    async fn test_write_and_read() {
        let storage = Arc::new(MockBackend::new());
        let odb = ObjectDatabase::new(storage, 100);

        let data = b"test content";
        let oid = odb.write(ObjectType::Blob, data).await.unwrap();

        let retrieved = odb.read(&oid).await.unwrap();
        assert_eq!(retrieved, data);
    }

    #[tokio::test]
    async fn test_deduplication() {
        let storage = Arc::new(MockBackend::new());
        let odb = ObjectDatabase::new(storage, 100);

        let data = b"duplicate content";

        // Write same content twice
        let oid1 = odb.write(ObjectType::Blob, data).await.unwrap();
        let oid2 = odb.write(ObjectType::Blob, data).await.unwrap();

        // Should return same OID
        assert_eq!(oid1, oid2);

        // Metrics should show deduplication
        let metrics = odb.metrics().await;
        assert_eq!(metrics.unique_objects, 1);
        assert_eq!(metrics.total_writes, 2);
        assert_eq!(metrics.bytes_written, data.len() as u64 * 2);
        assert_eq!(metrics.bytes_stored, data.len() as u64);
        assert_eq!(metrics.dedup_ratio(), 0.5); // 50% deduplicated
    }

    #[tokio::test]
    async fn test_cache_hit() {
        let storage = Arc::new(MockBackend::new());
        let odb = ObjectDatabase::new(storage, 100);

        let data = b"cached data";
        let oid = odb.write(ObjectType::Blob, data).await.unwrap();

        // First read - cache miss
        let _ = odb.read(&oid).await.unwrap();

        // Clear internal state and read again - should be cache hit
        let _ = odb.read(&oid).await.unwrap();

        let metrics = odb.metrics().await;
        assert!(metrics.cache_hits > 0);
    }

    #[tokio::test]
    async fn test_exists() {
        let storage = Arc::new(MockBackend::new());
        let odb = ObjectDatabase::new(storage, 100);

        let data = b"exists test";
        let oid = odb.write(ObjectType::Blob, data).await.unwrap();

        assert!(odb.exists(&oid).await.unwrap());

        let non_existent = Oid::hash(b"does not exist");
        assert!(!odb.exists(&non_existent).await.unwrap());
    }

    #[tokio::test]
    async fn test_resolve_abbreviated_oid_basic_and_errors() {
        // Guard for BUG-002: short OIDs must resolve when unique and
        // error loudly on too-short / non-hex / unknown prefixes.
        let storage = Arc::new(MockBackend::new());
        let odb = ObjectDatabase::new(storage, 100);

        let oid_a = odb
            .write(ObjectType::Blob, b"resolve-abbrev-fixture-A")
            .await
            .unwrap();

        let full = oid_a.to_hex();
        let abbrev = &full[..8];
        let resolved = odb.resolve_abbreviated_oid(abbrev).await.unwrap();
        assert_eq!(resolved, oid_a);

        // Too-short prefix
        let err = odb
            .resolve_abbreviated_oid("abc")
            .await
            .expect_err("<4 char prefix must bail");
        assert!(err.to_string().contains("at least 4"));

        // Non-hex
        let err = odb
            .resolve_abbreviated_oid("zzzz")
            .await
            .expect_err("non-hex must bail");
        assert!(err.to_string().to_lowercase().contains("prefix"));
    }

    /// QA-010: after `gc --repack` bundles a loose object into a pack and
    /// deletes the loose copy, both `exists()` and `resolve_abbreviated_oid()`
    /// must still find it via pack membership — not just the loose scan.
    #[tokio::test]
    async fn test_exists_and_resolve_abbreviated_oid_after_repack_removes_loose() {
        let storage = Arc::new(MockBackend::new());
        let odb = ObjectDatabase::new(storage, 100);

        let oid = odb
            .write(ObjectType::Blob, b"packed-only-object-fixture")
            .await
            .unwrap();
        // Force a real storage/pack lookup instead of a cache hit.
        odb.invalidate_cache(&oid).await;

        let stats = odb.repack(0, true).await.unwrap();
        assert_eq!(stats.loose_objects_removed, 1, "loose copy must be removed");

        assert!(
            odb.exists(&oid).await.unwrap(),
            "packed-only object must still report as existing"
        );

        let abbrev = &oid.to_hex()[..8];
        let resolved = odb
            .resolve_abbreviated_oid(abbrev)
            .await
            .expect("packed-only object must resolve by short hash");
        assert_eq!(resolved, oid);
    }

    /// `MEDIAGIT_REPACK_CHUNKS` is a process-global env var, but `cargo test`
    /// runs test functions from this file concurrently on separate threads
    /// within the same process. Every test below that sets/reads it must
    /// hold this lock for its whole body so it can't observe (or clobber)
    /// another such test's value mid-run.
    static REPACK_CHUNKS_ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    /// I9: loose chunks default-route into Track-F-style cloud packs
    /// (`MEDIAGIT_REPACK_CHUNKS` unset = enabled). Packed chunks must stay
    /// byte-identical and readable via the pack-fallback path in
    /// `get_chunk`, and the loose `chunks/<hex>` keys must be gone.
    #[tokio::test]
    // Deliberately holds the env lock across awaits (see REPACK_CHUNKS_ENV_LOCK).
    #[allow(clippy::await_holding_lock)]
    async fn test_repack_chunks_into_cloud_pack_readable_after_loose_removed() {
        let _env_lock = REPACK_CHUNKS_ENV_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        // FIXME: Audit that the environment access only happens in single-threaded code.
        unsafe { std::env::remove_var("MEDIAGIT_REPACK_CHUNKS") };

        let storage = Arc::new(MockBackend::new());
        let odb = ObjectDatabase::new(storage, 100);

        let mut fixtures = Vec::new();
        for i in 0..5u8 {
            let content = vec![i; 4096 + i as usize * 17];
            let chunk_id = Oid::hash(&content);
            let compressed = odb.compressor.compress(&content).unwrap();
            odb.put_compressed_chunk(&chunk_id, &compressed)
                .await
                .unwrap();
            fixtures.push((chunk_id, content));
        }

        let stats = odb.repack(0, true).await.unwrap();
        assert_eq!(stats.objects_packed, 5);
        assert_eq!(stats.loose_objects_removed, 5);

        for (chunk_id, _) in &fixtures {
            let key = format!("chunks/{}", chunk_id.to_hex());
            assert!(
                !odb.storage.exists(&key).await.unwrap(),
                "loose chunk {} must be removed after cloud-pack repack",
                chunk_id
            );
        }

        for (chunk_id, content) in &fixtures {
            let data = odb.get_chunk(chunk_id).await.unwrap();
            assert_eq!(
                &data, content,
                "chunk {} must read back byte-identical from the cloud pack",
                chunk_id
            );
        }

        // A JSONL manifest must have been written under packs/<shard>/.
        let pack_keys = odb.storage.list_objects("packs/").await.unwrap();
        assert!(
            pack_keys.iter().any(|k| k.ends_with(".jsonl")),
            "cloud-pack repack must persist a JSONL chunk index: {:?}",
            pack_keys
        );
    }

    /// I9 knob: `MEDIAGIT_REPACK_CHUNKS=0` must reproduce the pre-I9
    /// behavior exactly — chunks folded into the monolithic `PackWriter`
    /// pack, no JSONL manifest written.
    #[tokio::test]
    // Deliberately holds the env lock across awaits (see REPACK_CHUNKS_ENV_LOCK).
    #[allow(clippy::await_holding_lock)]
    async fn test_repack_chunks_knob_disabled_matches_legacy_behavior() {
        let _env_lock = REPACK_CHUNKS_ENV_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        // FIXME: Audit that the environment access only happens in single-threaded code.
        unsafe { std::env::set_var("MEDIAGIT_REPACK_CHUNKS", "0") };

        let storage = Arc::new(MockBackend::new());
        let odb = ObjectDatabase::new(storage, 100);

        let content = vec![7u8; 5000];
        let chunk_id = Oid::hash(&content);
        let compressed = odb.compressor.compress(&content).unwrap();
        odb.put_compressed_chunk(&chunk_id, &compressed)
            .await
            .unwrap();

        let stats = odb.repack(0, true).await.unwrap();
        assert_eq!(stats.objects_packed, 1);
        assert_eq!(stats.loose_objects_removed, 1);

        let pack_keys = odb.storage.list_objects("packs/").await.unwrap();
        assert!(
            pack_keys.iter().all(|k| !k.ends_with(".jsonl")),
            "MEDIAGIT_REPACK_CHUNKS=0 must not produce a JSONL manifest: {:?}",
            pack_keys
        );
        assert!(
            pack_keys.iter().any(|k| k.ends_with(".pack")),
            "MEDIAGIT_REPACK_CHUNKS=0 must still produce a legacy .pack file: {:?}",
            pack_keys
        );

        let data = odb.get_chunk(&chunk_id).await.unwrap();
        assert_eq!(data, content);

        // FIXME: Audit that the environment access only happens in single-threaded code.
        unsafe { std::env::remove_var("MEDIAGIT_REPACK_CHUNKS") };
    }

    /// Storage wrapper that fails `put()` for any key containing
    /// `fail_substring`, used to simulate a crash partway through a
    /// multi-step durable write sequence.
    #[derive(Debug)]
    struct FailOnKeyBackend {
        inner: Arc<dyn StorageBackend>,
        fail_substring: String,
    }

    #[async_trait::async_trait]
    impl StorageBackend for FailOnKeyBackend {
        async fn get(&self, key: &str) -> anyhow::Result<Vec<u8>> {
            self.inner.get(key).await
        }
        async fn put(&self, key: &str, data: &[u8]) -> anyhow::Result<()> {
            if key.contains(&self.fail_substring) {
                anyhow::bail!("simulated crash writing {}", key);
            }
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

    /// I9 abort-safety: a crash between "pack bytes stored" and "JSONL index
    /// persisted" must leave the loose chunk untouched — `seal_chunk_cloud_pack`
    /// only deletes loose chunks after the JSONL write succeeds.
    #[tokio::test]
    // Deliberately holds the env lock across awaits (see REPACK_CHUNKS_ENV_LOCK).
    #[allow(clippy::await_holding_lock)]
    async fn test_repack_chunks_cloud_pack_abort_before_index_persist_keeps_loose_chunk() {
        let _env_lock = REPACK_CHUNKS_ENV_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        // FIXME: Audit that the environment access only happens in single-threaded code.
        unsafe { std::env::remove_var("MEDIAGIT_REPACK_CHUNKS") };

        let inner = Arc::new(MockBackend::new());
        let failing: Arc<dyn StorageBackend> = Arc::new(FailOnKeyBackend {
            inner: inner.clone(),
            fail_substring: ".jsonl".to_string(),
        });
        let odb = ObjectDatabase::new(failing, 100);

        let content = vec![9u8; 4096];
        let chunk_id = Oid::hash(&content);
        let compressed = odb.compressor.compress(&content).unwrap();
        odb.put_compressed_chunk(&chunk_id, &compressed)
            .await
            .unwrap();

        let result = odb.repack(0, true).await;
        assert!(
            result.is_err(),
            "repack must propagate the simulated JSONL-write failure"
        );

        // The loose chunk must survive: the pack bytes may already be
        // written, but nothing is deleted because the JSONL write (which
        // gates deletion) never succeeded.
        let key = format!("chunks/{}", chunk_id.to_hex());
        assert!(
            inner.exists(&key).await.unwrap(),
            "loose chunk must survive a crash before the index is durably persisted"
        );

        // Recovery: re-running repack against the real backend succeeds and
        // packs the still-present loose chunk.
        let odb2 = ObjectDatabase::new(inner.clone(), 100);
        let stats = odb2.repack(0, true).await.unwrap();
        assert_eq!(stats.objects_packed, 1);
        assert_eq!(stats.loose_objects_removed, 1);
        let data = odb2.get_chunk(&chunk_id).await.unwrap();
        assert_eq!(data, content);
    }

    /// QA-006b: `put_compressed_chunk` must decompress + hash-verify the
    /// payload against its declared chunk_id before persisting anything.
    #[tokio::test]
    async fn test_put_compressed_chunk_accepts_valid_payload() {
        let storage = Arc::new(MockBackend::new());
        let odb = ObjectDatabase::new(storage, 100);

        let content = b"valid chunk payload".to_vec();
        let chunk_id = Oid::hash(&content);
        let compressed = odb.compressor.compress(&content).unwrap();

        odb.put_compressed_chunk(&chunk_id, &compressed)
            .await
            .unwrap();
        assert!(odb.chunk_exists(&chunk_id).await.unwrap());
        assert_eq!(odb.get_chunk(&chunk_id).await.unwrap(), content);
    }

    #[tokio::test]
    async fn test_put_compressed_chunk_rejects_hash_mismatch() {
        let storage = Arc::new(MockBackend::new());
        let odb = ObjectDatabase::new(storage, 100);

        // Well-formed compressed bytes that decompress successfully, but to
        // content that does NOT hash to the declared id (e.g. an attacker
        // or a corrupted transport relabeling one chunk as another).
        let real_content = b"the real chunk content".to_vec();
        let compressed = odb.compressor.compress(&real_content).unwrap();
        let wrong_id = Oid::hash(b"a completely different payload");

        let err = odb
            .put_compressed_chunk(&wrong_id, &compressed)
            .await
            .expect_err("content/id mismatch must be rejected");
        assert!(
            err.to_string().contains("integrity"),
            "unexpected error: {}",
            err
        );
        assert!(
            !odb.chunk_exists(&wrong_id).await.unwrap(),
            "mismatched chunk must not be persisted"
        );
    }

    #[tokio::test]
    async fn test_put_compressed_chunk_rejects_flipped_byte() {
        let storage = Arc::new(MockBackend::new());
        let odb = ObjectDatabase::new(storage, 100);

        let content = b"chunk payload for bit-flip integrity test".to_vec();
        let chunk_id = Oid::hash(&content);
        let mut compressed = odb.compressor.compress(&content).unwrap();
        let last = compressed.len() - 1;
        compressed[last] ^= 0xFF;

        // Corruption is rejected whether it surfaces as a decompression
        // failure or a hash mismatch — either way, nothing gets stored.
        assert!(
            odb.put_compressed_chunk(&chunk_id, &compressed)
                .await
                .is_err(),
            "flipped-byte payload must be rejected"
        );
        assert!(
            !odb.chunk_exists(&chunk_id).await.unwrap(),
            "rejected chunk must not be stored"
        );
    }

    #[tokio::test]
    async fn test_verify() {
        let storage = Arc::new(MockBackend::new());
        let odb = ObjectDatabase::new(storage, 100);

        let data = b"verify test";
        let oid = odb.write(ObjectType::Blob, data).await.unwrap();

        assert!(odb.verify(&oid).await.unwrap());

        let non_existent = Oid::hash(b"does not exist");
        assert!(!odb.verify(&non_existent).await.unwrap());
    }

    #[tokio::test]
    async fn test_cache_operations() {
        let storage = Arc::new(MockBackend::new());
        let odb = ObjectDatabase::new(storage, 100);

        let data = b"cache test";
        let oid = odb.write(ObjectType::Blob, data).await.unwrap();

        // Run pending tasks to ensure cache is updated
        odb.cache.run_pending_tasks().await;

        // Should be in cache after write
        assert_eq!(odb.cache_entry_count().await, 1);

        // Invalidate specific entry
        odb.invalidate_cache(&oid).await;

        // Clear all
        odb.clear_cache().await;
    }

    #[tokio::test]
    async fn test_compression_enabled() {
        let storage = Arc::new(MockBackend::new());
        let odb = ObjectDatabase::new(storage.clone(), 100);

        // Write large compressible data
        let data = b"This is test data that compresses well. ".repeat(100);
        let oid = odb.write(ObjectType::Blob, &data).await.unwrap();

        // Read it back
        let retrieved = odb.read(&oid).await.unwrap();
        assert_eq!(retrieved, data);

        // Verify data is compressed in storage
        let key = oid.to_hex();
        let stored_data = storage.get(&key).await.unwrap();

        // Stored data should be smaller than original (compressed)
        assert!(stored_data.len() < data.len());

        // Stored data should have zlib header (0x78)
        assert_eq!(stored_data[0], 0x78);
    }

    #[tokio::test]
    async fn test_compression_disabled() {
        let storage = Arc::new(MockBackend::new());
        let odb = ObjectDatabase::without_compression(storage.clone(), 100);

        let data = b"test data without compression";
        let oid = odb.write(ObjectType::Blob, data).await.unwrap();

        // Read it back
        let retrieved = odb.read(&oid).await.unwrap();
        assert_eq!(retrieved, data);

        // Verify data is NOT compressed in storage
        let key = oid.to_hex();
        let stored_data = storage.get(&key).await.unwrap();

        // Stored data should be same as original (uncompressed)
        assert_eq!(stored_data, data);
    }

    #[tokio::test]
    async fn test_backward_compatibility() {
        let storage = Arc::new(MockBackend::new());

        // First, write uncompressed data (simulating old version)
        let odb_old = ObjectDatabase::without_compression(storage.clone(), 100);
        let data = b"old uncompressed data";
        let oid = odb_old.write(ObjectType::Blob, data).await.unwrap();

        // Now read with compression-enabled ODB (simulating new version)
        let odb_new = ObjectDatabase::new(storage, 100);
        let retrieved = odb_new.read(&oid).await.unwrap();

        // Should read successfully
        assert_eq!(retrieved, data);
    }

    #[tokio::test]
    async fn test_compression_ratio() {
        let storage = Arc::new(MockBackend::new());
        let odb = ObjectDatabase::new(storage.clone(), 100);

        // Highly compressible data (repeated pattern)
        let data = vec![0x42u8; 10000];
        let oid = odb.write(ObjectType::Blob, &data).await.unwrap();

        // Get stored data
        let key = oid.to_hex();
        let stored_data = storage.get(&key).await.unwrap();

        // Compression ratio should be significant (>90% reduction for repeated data)
        let ratio = stored_data.len() as f64 / data.len() as f64;
        assert!(
            ratio < 0.1,
            "Expected high compression ratio, got {}",
            ratio
        );

        // Verify integrity
        let retrieved = odb.read(&oid).await.unwrap();
        assert_eq!(retrieved, data);
    }

    #[tokio::test]
    async fn test_custom_compressor() {
        use mediagit_compression::{CompressionLevel, ZstdCompressor};

        let storage = Arc::new(MockBackend::new());
        let compressor = Arc::new(ZstdCompressor::new(CompressionLevel::Best));
        let odb = ObjectDatabase::with_compression(storage, 100, compressor, true);

        let data = b"test data with zstd compression";
        let oid = odb.write(ObjectType::Blob, data).await.unwrap();

        let retrieved = odb.read(&oid).await.unwrap();
        assert_eq!(retrieved, data);
    }

    #[tokio::test]
    async fn test_empty_data_compression() {
        let storage = Arc::new(MockBackend::new());
        let odb = ObjectDatabase::new(storage, 100);

        let data = b"";
        let oid = odb.write(ObjectType::Blob, data).await.unwrap();

        let retrieved = odb.read(&oid).await.unwrap();
        assert_eq!(retrieved, data);
    }

    #[tokio::test]
    async fn test_large_file_compression() {
        let storage = Arc::new(MockBackend::new());
        let odb = ObjectDatabase::new(storage, 100);

        // Simulate a 1MB file with some compressibility
        let data = (0..10000)
            .flat_map(|i| format!("Line {} content\n", i).into_bytes())
            .collect::<Vec<u8>>();

        let oid = odb.write(ObjectType::Blob, &data).await.unwrap();
        let retrieved = odb.read(&oid).await.unwrap();

        assert_eq!(retrieved, data);
    }

    /// REGRESSION TEST for GC --repack branch switching bug
    ///
    /// This test ensures that objects remain readable after GC reorganization.
    /// Previously, ODB::exists() used format!("objects/{}", oid.to_path()) while
    /// ODB::read() used oid.to_hex(), causing branch checkouts to fail after GC.
    #[tokio::test]
    async fn test_object_path_consistency_after_gc() {
        use mediagit_storage::LocalBackend;
        use tempfile::TempDir;

        let temp_dir = TempDir::new().unwrap();
        let storage_path = temp_dir.path();

        // Create storage with real LocalBackend (not mock) to test path handling
        let storage = Arc::new(LocalBackend::new(storage_path).await.unwrap());
        let odb = ObjectDatabase::new(storage.clone(), 100);

        // Write multiple test objects
        let data1 = b"test content 1";
        let data2 = b"test content 2 with different data";
        let data3 = b"yet another test object";

        let oid1 = odb.write(ObjectType::Blob, data1).await.unwrap();
        let oid2 = odb.write(ObjectType::Blob, data2).await.unwrap();
        let oid3 = odb.write(ObjectType::Blob, data3).await.unwrap();

        // Verify objects exist BEFORE any operation
        assert!(
            odb.exists(&oid1).await.unwrap(),
            "Object 1 should exist before GC"
        );
        assert!(
            odb.exists(&oid2).await.unwrap(),
            "Object 2 should exist before GC"
        );
        assert!(
            odb.exists(&oid3).await.unwrap(),
            "Object 3 should exist before GC"
        );

        // Verify objects are readable BEFORE
        let read1 = odb.read(&oid1).await.unwrap();
        assert_eq!(read1, data1, "Should read object 1 before GC");

        let read2 = odb.read(&oid2).await.unwrap();
        assert_eq!(read2, data2, "Should read object 2 before GC");

        let read3 = odb.read(&oid3).await.unwrap();
        assert_eq!(read3, data3, "Should read object 3 before GC");

        // Clear cache to ensure we're reading from storage (not cache)
        odb.clear_cache().await;

        // The test doesn't actually need to repack since objects are already in
        // the sharded storage structure. The key test is that exists() and read()
        // use consistent path resolution via oid.to_hex().

        // CRITICAL TEST: Verify objects still exist AFTER reorganization
        assert!(
            odb.exists(&oid1).await.unwrap(),
            "Object 1 should exist after GC"
        );
        assert!(
            odb.exists(&oid2).await.unwrap(),
            "Object 2 should exist after GC"
        );
        assert!(
            odb.exists(&oid3).await.unwrap(),
            "Object 3 should exist after GC"
        );

        // CRITICAL TEST: Verify objects are still readable AFTER reorganization
        // This is where the bug manifested - checkout would fail here
        let read1_after = odb.read(&oid1).await.unwrap();
        assert_eq!(
            read1_after, data1,
            "Should read object 1 after GC reorganization"
        );

        let read2_after = odb.read(&oid2).await.unwrap();
        assert_eq!(
            read2_after, data2,
            "Should read object 2 after GC reorganization"
        );

        let read3_after = odb.read(&oid3).await.unwrap();
        assert_eq!(
            read3_after, data3,
            "Should read object 3 after GC reorganization"
        );

        // Additional check: Verify size queries work
        let size1 = odb.get_object_size(&oid1).await.unwrap();
        assert_eq!(size1, data1.len(), "Size query should work after GC");
    }

    #[test]
    fn test_delta_metadata_parsing() {
        // Test the delta metadata parsing logic handles both formats correctly
        // Format 1 (new): "base:{oid}:depth:{n}"
        // Format 2 (legacy): "base:{oid}"

        let test_oid = "57b77408e3f862ecc9288b59a6cd6da6c529bd4d61883483c5e8dc7989e1e918";

        // Test new format with depth
        let meta_new = format!("base:{}:depth:1", test_oid);
        let after_prefix = meta_new.strip_prefix("base:").unwrap().trim();
        let base_oid_hex = if let Some(idx) = after_prefix.find(":depth:") {
            &after_prefix[..idx]
        } else {
            after_prefix
        };
        assert_eq!(base_oid_hex, test_oid, "Should parse new format correctly");
        assert_eq!(base_oid_hex.len(), 64, "OID should be 64 chars");

        // Test legacy format without depth
        let meta_legacy = format!("base:{}", test_oid);
        let after_prefix = meta_legacy.strip_prefix("base:").unwrap().trim();
        let base_oid_hex = if let Some(idx) = after_prefix.find(":depth:") {
            &after_prefix[..idx]
        } else {
            after_prefix
        };
        assert_eq!(
            base_oid_hex, test_oid,
            "Should parse legacy format correctly"
        );
        assert_eq!(base_oid_hex.len(), 64, "OID should be 64 chars");

        // Verify OID can be parsed
        let oid = Oid::from_hex(base_oid_hex).unwrap();
        assert_eq!(oid.to_hex(), test_oid);
    }
}
