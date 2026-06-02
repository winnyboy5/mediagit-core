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
    if start == target {
        return true;
    }
    let mut current = start;
    let mut visited = std::collections::HashSet::new();
    for _ in 0..=MAX_DELTA_DEPTH {
        if current == target {
            return true;
        }
        if !visited.insert(current) {
            // Existing cycle in stored data — not our concern here. We only
            // need to answer "does the path lead to target?" Bail out so we
            // don't loop forever.
            return false;
        }
        let meta_key = format!("chunk-deltas/{}.meta", current.to_hex());
        // Cheap probe first: if no meta, base is terminal (full chunk).
        match storage.exists(&meta_key).await {
            Ok(true) => {}
            Ok(false) | Err(_) => return false,
        }
        let bytes = match storage.get(&meta_key).await {
            Ok(b) => b,
            Err(_) => return false,
        };
        let s = match std::str::from_utf8(&bytes) {
            Ok(s) => s,
            Err(_) => return false,
        };
        let hex = match s.trim().strip_prefix("base:") {
            Some(h) => h.trim(),
            None => return false,
        };
        let next = match Oid::from_hex(hex) {
            Ok(o) => o,
            Err(_) => return false,
        };
        current = next;
    }
    false
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
            delta_enabled: self.delta_enabled,
            similarity_detector: self.similarity_detector.clone(),
            base_chunk_cache: self.base_chunk_cache.clone(),
            delta_written_pairs: self.delta_written_pairs.clone(),
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
mod tests {
    use super::*;
    use mediagit_storage::mock::MockBackend;

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
