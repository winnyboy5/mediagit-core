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

//! Content-based chunking for efficient media storage
//!
//! This module provides content-aware chunking strategies for media files,
//! enabling deduplication at the chunk level rather than file level.
//!
//! # Features
//!
//! - Media-aware chunking (separate video/audio streams)
//! - Fixed-size chunking with configurable boundaries
//! - Rolling hash-based chunking for similar content
//! - Chunk-level deduplication and reference counting
//! - Perceptual similarity detection for near-duplicate chunks
//!
//! # Example
//!
//! ```rust,no_run
//! use mediagit_versioning::chunking::{ChunkStrategy, ContentChunker};
//!
//! # async fn example() -> anyhow::Result<()> {
//! let chunker = ContentChunker::new(ChunkStrategy::MediaAware);
//!
//! let file_data = std::fs::read("video.avi")?;
//! let chunks = chunker.chunk(&file_data, "video.avi").await?;
//!
//! println!("Split into {} chunks", chunks.len());
//! for (i, chunk) in chunks.iter().enumerate() {
//!     println!("Chunk {}: {} bytes, hash: {}", i, chunk.data.len(), chunk.id);
//! }
//! # Ok(())
//! # }
//! ```

use crate::Oid;
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use tracing::{debug, info, warn};

pub(crate) mod chunker;
pub(crate) mod formats;

/// Chunk identifier (SHA-256 hash of chunk content)
pub type ChunkId = Oid;

/// Chunking strategy selection
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ChunkStrategy {
    /// Fixed-size chunks (simple, predictable)
    Fixed { size: usize },
    /// Rolling hash chunking (content-defined boundaries)
    Rolling {
        avg_size: usize,
        min_size: usize,
        max_size: usize,
    },
    /// Media-aware chunking (parse structure, separate streams)
    #[default]
    MediaAware,
}

/// Content chunk with metadata
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContentChunk {
    /// Unique chunk identifier (content hash)
    pub id: ChunkId,

    /// Chunk data
    #[serde(skip)]
    pub data: Vec<u8>,

    /// Offset in original file
    pub offset: u64,

    /// Chunk size in bytes
    pub size: usize,

    /// Chunk type (for media-aware chunking)
    pub chunk_type: ChunkType,

    /// Perceptual hash (for similarity detection)
    pub perceptual_hash: Option<Vec<u8>>,

    /// Codec hint for per-chunk compression/delta strategy
    #[serde(default)]
    pub codec_hint: CodecHint,
}

/// Chunk type classification
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ChunkType {
    /// Generic data chunk
    Generic,
    /// Video stream data
    VideoStream,
    /// Audio stream data
    AudioStream,
    /// Metadata/header data
    Metadata,
    /// Subtitle/caption data
    Subtitle,
}

/// Codec hint for per-chunk compression and delta strategy.
///
/// Detected during container parsing (MP4 stsd, MKV CodecID, AVI strf).
/// Enables stream-aware storage: compressed codecs → Store, uncompressed → Zstd,
/// text subtitles → Brotli, metadata → Zstd.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum CodecHint {
    // Video — lossy compressed (high entropy, Store)
    /// H.264/AVC
    H264,
    /// H.265/HEVC
    H265,
    /// VP9
    VP9,
    /// AV1
    AV1,
    // Video — intra-frame / mezzanine (medium entropy, Zstd compressible)
    /// Apple ProRes (all profiles)
    ProRes,
    /// Avid DNxHR/DNxHD
    DNxHR,
    /// JPEG 2000 (wavelet-compressed, Store)
    Jpeg2000,
    /// Raw/uncompressed video (v210, YUV, RGB)
    RawVideo,
    // Audio — lossy compressed (high entropy, Store)
    /// AAC
    AAC,
    /// Opus
    Opus,
    /// MP3
    MP3,
    /// Vorbis
    Vorbis,
    // Audio — uncompressed (medium entropy, Zstd compressible)
    /// PCM (all sample formats)
    PCM,
    /// FLAC (lossless compressed, Store)
    FLAC,
    /// ALAC (lossless compressed, Store)
    ALAC,
    // Subtitle
    /// Text-based subtitles (SRT, ASS, tx3g, WebVTT)
    TextSub,
    /// Bitmap-based subtitles (PGS, VobSub)
    BitmapSub,
    /// Unknown or undetected codec
    #[default]
    Unknown,
}

/// MP4 Atom header parsed from data
/// Used internally for MP4/MOV/M4V atom-based chunking
#[derive(Debug, Clone)]
pub(super) struct Mp4Atom {
    /// 4-byte FourCC type (e.g., b"ftyp", b"moov", b"mdat")
    pub(super) atom_type: [u8; 4],
    /// Offset in data where atom starts
    pub(super) offset: u64,
    /// Total size including header (8 or 16 bytes header + data)
    pub(super) size: u64,
    /// Header size (8 for standard, 16 for extended)
    pub(super) header_size: u8,
}

/// EBML Element header parsed from Matroska/WebM data
/// Used internally for MKV/WebM/MKA element-based chunking
#[derive(Debug, Clone)]
pub(super) struct EbmlElement {
    /// Element ID (includes VINT marker, up to 4 bytes for Matroska)
    pub(super) id: u32,
    /// Offset in data where element starts
    pub(super) offset: u64,
    /// Size of ID + Size fields
    pub(super) header_size: u8,
    /// Content size (u64::MAX = unknown size)
    pub(super) data_size: u64,
}

// Matroska Element IDs (with VINT marker included)
pub(super) const EBML_ID: u32 = 0x1A45DFA3; // EBML Header
pub(super) const SEGMENT_ID: u32 = 0x18538067; // Segment container
pub(super) const SEEKHEAD_ID: u32 = 0x114D9B74; // SeekHead (index)
pub(super) const INFO_ID: u32 = 0x1549A966; // Segment Info
pub(super) const TRACKS_ID: u32 = 0x1654AE6B; // Track definitions
pub(super) const CLUSTER_ID: u32 = 0x1F43B675; // Cluster (media data)
pub(super) const CUES_ID: u32 = 0x1C53BB6B; // Cues (seek index)
pub(super) const CHAPTERS_ID: u32 = 0x1043A770; // Chapters
pub(super) const TAGS_ID: u32 = 0x1254C367; // Tags (metadata)
pub(super) const ATTACHMENTS_ID: u32 = 0x1941A469; // Attachments
pub(super) const VOID_ID: u32 = 0xEC; // Void (padding, skip)
pub(super) const CRC32_ID: u32 = 0xBF; // CRC-32 (skip)
pub(super) const TRACK_ENTRY_ID: u32 = 0xAE; // TrackEntry (child of Tracks)
pub(super) const CODEC_ID_ID: u32 = 0x86; // CodecID string (child of TrackEntry)

/// Get optimal chunk parameters based on file size
///
/// Returns (avg_size, min_size, max_size) tuned for the file size.
/// Larger files use larger chunks to reduce manifest overhead.
///
/// # Memory Efficiency
/// - Files < 100MB: 1MB avg chunks (faster for small files)
/// - Files 100MB-10GB: 2MB avg chunks
/// - Files 10GB-100GB: 4MB avg chunks
/// - Files > 100GB: 8MB avg chunks (optimal for TB+ files)
///
/// # Examples
/// ```ignore
/// // Internal function, example for documentation only
/// let (avg, min, max) = get_chunk_params(50_000_000_000); // 50GB
/// assert_eq!(avg, 4 * 1024 * 1024); // 4MB average
/// ```
fn get_chunk_params(file_size: u64) -> (usize, usize, usize) {
    const MB: usize = 1024 * 1024;
    match file_size {
        0..=100_000_000 => (MB, 512 * 1024, 4 * MB), // < 100MB: 1MB avg
        100_000_001..=10_000_000_000 => (2 * MB, MB, 8 * MB), // 100MB-10GB: 2MB avg
        10_000_000_001..=100_000_000_000 => (4 * MB, MB, 16 * MB), // 10GB-100GB: 4MB avg
        _ => (8 * MB, MB, 32 * MB),                  // > 100GB: 8MB avg
    }
}

/// Chunk params for creative-container formats (AI/PSD/INDD/EPS/PDF).
///
/// These files embed zlib-compressed streams and an xref/directory table at
/// the end. Minor content changes (adding a layer/object) insert new streams
/// mid-file and rewrite the trailing table, which byte-shifts everything
/// after the insertion point. FastCDC re-syncs via rolling hash, but with
/// tier-2+ params (1–8 MB chunks) the re-sync window is too wide — large
/// chunks cross shifted boundaries and hash differently.
///
/// Capping at tier-1 params (avg 1 MB, max 4 MB) keeps chunks small enough
/// to realign quickly after a shift, recovering dedup on the unchanged
/// portions of the file. This matters most for delta_ai_lg /
/// delta_psd / delta_indd at 100 MB+.
fn get_creative_chunk_params(_file_size: u64) -> (usize, usize, usize) {
    const MB: usize = 1024 * 1024;
    (MB, 512 * 1024, 4 * MB)
}

/// Chunk params for the audio tier.
///
/// Compressed audio benefits from smaller chunks than the generic size
/// tiers: 256 KB avg / 64 KB min / 1 MB max keeps boundaries tight enough to
/// re-sync after small edits (trims, metadata tag rewrites) without the
/// manifest overhead of per-sample chunking.
///
/// Spec'd for WAV/AIFF/FLAC/OGG/MP3, but currently wired to MP3/OGG only
/// (see the deviation comment on the `flac` match arm in chunker.rs):
/// measured on the dedup_report corpus, applying this tier to WAV/FLAC
/// regressed total add_ms by ~140% for <1pp dedup gain, driven by
/// SmartCompressor's large fixed per-call cost multiplied by ~4x more
/// unique chunks on those two large-by-volume formats.
fn get_audio_chunk_params(_file_size: u64) -> (usize, usize, usize) {
    (256 * 1024, 64 * 1024, 1024 * 1024)
}

/// Content-based chunker
pub struct ContentChunker {
    pub(super) strategy: ChunkStrategy,
    /// Per-repo CDC seed for gear-hash mixing. `0` reproduces the original
    /// (pre-seed) chunk boundaries exactly — this is the legacy/default value.
    pub(super) seed: u64,
}

/// Resolve the effective CDC seed for chunking.
///
/// Priority: `MEDIAGIT_CDC_SEED` env var (parsed as u64; overrides everything,
/// including `0` which forces legacy unseeded boundaries) > `repo_seed` (the
/// repo's persisted `cdc_seed` config, `0` if absent) > `0`.
///
/// A seed mismatch between two clones of the same repo only degrades
/// deduplication (different chunk boundaries) — it never affects correctness,
/// since chunk storage remains content-addressed.
pub fn resolve_cdc_seed(repo_seed: u64) -> u64 {
    std::env::var("MEDIAGIT_CDC_SEED")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(repo_seed)
}

static CODEC_DETECT_ENABLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();

/// Parse the `MEDIAGIT_CODEC_DETECT` value into an enabled/disabled flag.
/// Split out from [`codec_detect_enabled`] so tests can exercise the parsing
/// logic directly without touching the process-wide `OnceLock` (which, once
/// initialized by any test in the same process, can no longer be changed).
fn codec_detect_enabled_from_env(var: Option<&str>) -> bool {
    var.map(|v| v != "0").unwrap_or(true)
}

/// Whether container-aware codec detection is enabled.
///
/// Controlled by `MEDIAGIT_CODEC_DETECT` (default: enabled). Set to `0` to
/// force every `CodecHint` to `Unknown` — byte-for-byte the pre-detection
/// chunking behavior. Read once and cached (never per-chunk/per-byte).
fn codec_detect_enabled() -> bool {
    *CODEC_DETECT_ENABLED.get_or_init(|| {
        codec_detect_enabled_from_env(std::env::var("MEDIAGIT_CODEC_DETECT").ok().as_deref())
    })
}

// Per-format kill knobs for the P3a structure-aware walkers (FBX/blend/STL/PLY)
// and the audio chunk tier. Each reads its env var once (OnceLock) and shares
// the same "0 disables, anything else (including unset) enables" parsing as
// `codec_detect_enabled_from_env` above — value `0` restores exact pre-P3a
// chunking behavior for that format.
static CHUNK_FBX_ENABLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
static CHUNK_BLEND_ENABLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
static CHUNK_STL_ENABLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
static CHUNK_PLY_ENABLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
static AUDIO_TIER_ENABLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();

fn chunk_fbx_enabled() -> bool {
    // Default OFF, unlike the other walkers. Two measured strikes:
    // (1) byte-insert pair: LOST 3.79pp dedup vs generic CDC (the insert
    //     breaks the EndOffset chain, so v1/v2 take different cut paths);
    // (2) fair trial on a structure-VALID edit (duplicated subtree, all
    //     offsets fixed up): +0.003pp — statistically nothing, because real
    //     FBX files are one giant `Objects` node (~98% of bytes) and the
    //     top-level walker collapses to whole-span CDC anyway.
    // Beating CDC would require descending INTO Objects (per-Model/Geometry
    // cuts) — a different design. Until someone builds that, this stays off;
    // opt in with MEDIAGIT_CHUNK_FBX=1.
    *CHUNK_FBX_ENABLED.get_or_init(|| {
        matches!(
            std::env::var("MEDIAGIT_CHUNK_FBX").ok().as_deref(),
            Some("1")
        )
    })
}

fn chunk_blend_enabled() -> bool {
    *CHUNK_BLEND_ENABLED.get_or_init(|| {
        codec_detect_enabled_from_env(std::env::var("MEDIAGIT_CHUNK_BLEND").ok().as_deref())
    })
}

fn chunk_stl_enabled() -> bool {
    *CHUNK_STL_ENABLED.get_or_init(|| {
        codec_detect_enabled_from_env(std::env::var("MEDIAGIT_CHUNK_STL").ok().as_deref())
    })
}

fn chunk_ply_enabled() -> bool {
    *CHUNK_PLY_ENABLED.get_or_init(|| {
        codec_detect_enabled_from_env(std::env::var("MEDIAGIT_CHUNK_PLY").ok().as_deref())
    })
}

fn audio_tier_enabled() -> bool {
    *AUDIO_TIER_ENABLED.get_or_init(|| {
        codec_detect_enabled_from_env(std::env::var("MEDIAGIT_AUDIO_TIER").ok().as_deref())
    })
}

/// Video-group codec hints (matches the classification `odb::to_chunk_codec_hint`
/// and `odb::delta_ratio_threshold` use for compression/delta strategy).
fn is_video_codec_hint(hint: CodecHint) -> bool {
    matches!(
        hint,
        CodecHint::H264
            | CodecHint::H265
            | CodecHint::VP9
            | CodecHint::AV1
            | CodecHint::ProRes
            | CodecHint::DNxHR
            | CodecHint::Jpeg2000
            | CodecHint::RawVideo
    )
}

/// Audio-group codec hints.
fn is_audio_codec_hint(hint: CodecHint) -> bool {
    matches!(
        hint,
        CodecHint::AAC
            | CodecHint::Opus
            | CodecHint::MP3
            | CodecHint::Vorbis
            | CodecHint::PCM
            | CodecHint::FLAC
            | CodecHint::ALAC
    )
}

/// Pick the "dominant" codec hint out of the per-track/per-sample-entry hints
/// found while scanning container metadata (MP4 `stsd`, MKV `Tracks`, AVI
/// `strl`). Video dominates a container's byte content (mdat/Cluster/movi),
/// so the first video hint wins; falls back to the first audio hint when the
/// file has no video track. Empty/subtitle-only input yields `Unknown`.
pub(super) fn dominant_codec_hint(hints: &[CodecHint]) -> CodecHint {
    hints
        .iter()
        .copied()
        .find(|h| is_video_codec_hint(*h))
        .or_else(|| hints.iter().copied().find(|h| is_audio_codec_hint(*h)))
        .unwrap_or(CodecHint::Unknown)
}

/// Overwrite `codec_hint` on every chunk, honoring the `MEDIAGIT_CODEC_DETECT`
/// kill switch and leaving chunks untouched when `hint` is `Unknown`.
pub(super) fn apply_codec_hint(chunks: Vec<ContentChunk>, hint: CodecHint) -> Vec<ContentChunk> {
    apply_codec_hint_if(chunks, hint, codec_detect_enabled())
}

/// Same as [`apply_codec_hint`] but takes the enabled flag explicitly, so
/// tests can exercise both branches without racing the shared `OnceLock`.
fn apply_codec_hint_if(
    mut chunks: Vec<ContentChunk>,
    hint: CodecHint,
    enabled: bool,
) -> Vec<ContentChunk> {
    if enabled && hint != CodecHint::Unknown {
        for chunk in &mut chunks {
            chunk.codec_hint = hint;
        }
    }
    chunks
}

/// Chunk store for managing chunk-level deduplication
pub struct ChunkStore {
    /// Chunk reference counts
    ref_counts: HashMap<ChunkId, usize>,

    /// Chunk metadata
    chunk_metadata: HashMap<ChunkId, ChunkMetadata>,
}

#[derive(Debug, Clone)]
struct ChunkMetadata {
    size: usize,
}

impl ChunkStore {
    /// Create a new chunk store
    pub fn new() -> Self {
        Self {
            ref_counts: HashMap::new(),
            chunk_metadata: HashMap::new(),
        }
    }

    /// Register a chunk (increment reference count)
    pub fn add_chunk(&mut self, chunk: &ContentChunk) {
        *self.ref_counts.entry(chunk.id).or_insert(0) += 1;

        self.chunk_metadata
            .entry(chunk.id)
            .or_insert_with(|| ChunkMetadata { size: chunk.size });
    }

    /// Remove a chunk reference (decrement reference count)
    pub fn remove_chunk(&mut self, chunk_id: &ChunkId) -> bool {
        if let Some(count) = self.ref_counts.get_mut(chunk_id) {
            *count -= 1;
            if *count == 0 {
                self.ref_counts.remove(chunk_id);
                self.chunk_metadata.remove(chunk_id);
                return true; // Chunk can be deleted
            }
        }
        false // Chunk still referenced
    }

    /// Check if a chunk exists
    pub fn contains(&self, chunk_id: &ChunkId) -> bool {
        self.ref_counts.contains_key(chunk_id)
    }

    /// Get chunk reference count
    pub fn ref_count(&self, chunk_id: &ChunkId) -> usize {
        self.ref_counts.get(chunk_id).copied().unwrap_or(0)
    }

    /// Calculate deduplication ratio
    pub fn dedup_ratio(&self) -> f64 {
        if self.ref_counts.is_empty() {
            return 0.0;
        }

        let total_refs: usize = self.ref_counts.values().sum();
        let unique_chunks = self.ref_counts.len();

        1.0 - (unique_chunks as f64 / total_refs as f64)
    }

    /// Get storage statistics
    pub fn stats(&self) -> ChunkStoreStats {
        let unique_chunks = self.ref_counts.len();
        let total_refs: usize = self.ref_counts.values().sum();
        let total_size: usize = self.chunk_metadata.values().map(|m| m.size).sum();

        ChunkStoreStats {
            unique_chunks,
            total_references: total_refs,
            total_size_bytes: total_size,
            dedup_ratio: self.dedup_ratio(),
        }
    }
}

impl Default for ChunkStore {
    fn default() -> Self {
        Self::new()
    }
}

/// Chunk store statistics
#[derive(Debug, Clone)]
pub struct ChunkStoreStats {
    pub unique_chunks: usize,
    pub total_references: usize,
    pub total_size_bytes: usize,
    pub dedup_ratio: f64,
}

/// Chunk reference in manifest (minimal metadata for reconstruction)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChunkRef {
    /// Chunk identifier (SHA-256 hash)
    pub id: ChunkId,
    /// Offset in original file
    pub offset: u64,
    /// Chunk size in bytes
    pub size: usize,
    /// Chunk type classification
    pub chunk_type: ChunkType,
    /// Codec hint for per-chunk compression/delta decisions
    #[serde(default)]
    pub codec_hint: CodecHint,
}

/// Chunk manifest for reconstructing chunked objects
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChunkManifest {
    /// List of chunks in order
    pub chunks: Vec<ChunkRef>,
    /// Total size of reconstructed object
    pub total_size: u64,
    /// Original filename (optional, for type detection)
    pub filename: Option<String>,
}

impl ChunkManifest {
    /// Create manifest from chunks
    pub fn from_chunks(chunks: Vec<ContentChunk>, filename: Option<String>) -> Self {
        let total_size = chunks.iter().map(|c| c.size as u64).sum();
        let chunk_refs = chunks
            .into_iter()
            .map(|c| ChunkRef {
                id: c.id,
                offset: c.offset,
                size: c.size,
                chunk_type: c.chunk_type,
                codec_hint: c.codec_hint,
            })
            .collect();

        Self {
            chunks: chunk_refs,
            total_size,
            filename,
        }
    }

    /// Get total number of chunks
    pub fn chunk_count(&self) -> usize {
        self.chunks.len()
    }
}

#[cfg(test)]
mod tests {
    use super::formats::{
        mkv_codec_id_to_hint, parse_ebml_elements, parse_mp4_atoms, read_ebml_id, read_ebml_size,
    };
    use super::*;

    #[tokio::test]
    async fn test_fixed_chunking() {
        let chunker = ContentChunker::new(ChunkStrategy::Fixed { size: 1024 });
        let data = vec![0u8; 3000];

        let chunks = chunker.chunk(&data, "test.bin").await.unwrap();

        assert_eq!(chunks.len(), 3);
        assert_eq!(chunks[0].size, 1024);
        assert_eq!(chunks[1].size, 1024);
        assert_eq!(chunks[2].size, 952);
    }

    #[tokio::test]
    async fn test_chunk_store() {
        let mut store = ChunkStore::new();

        let chunk1 = ContentChunk {
            id: Oid::hash(b"test1"),
            data: b"test1".to_vec(),
            offset: 0,
            size: 5,
            chunk_type: ChunkType::Generic,
            perceptual_hash: None,
            codec_hint: CodecHint::Unknown,
        };

        let chunk2 = ContentChunk {
            id: Oid::hash(b"test1"), // Same content
            data: b"test1".to_vec(),
            offset: 0,
            size: 5,
            chunk_type: ChunkType::Generic,
            perceptual_hash: None,
            codec_hint: CodecHint::Unknown,
        };

        store.add_chunk(&chunk1);
        store.add_chunk(&chunk2);

        assert_eq!(store.ref_count(&chunk1.id), 2);
        assert!(store.contains(&chunk1.id));

        store.remove_chunk(&chunk1.id);
        assert_eq!(store.ref_count(&chunk1.id), 1);

        store.remove_chunk(&chunk1.id);
        assert_eq!(store.ref_count(&chunk1.id), 0);
        assert!(!store.contains(&chunk1.id));
    }

    #[tokio::test]
    async fn test_fastcdc_handles_input_smaller_than_min_size() {
        // Depended on by emit_coalesced_node_chunks/chunk_stl/chunk_ply,
        // which always run FastCDC regardless of segment size: confirms it
        // degrades to a single whole-input chunk rather than panicking or
        // misbehaving when data.len() < min_size.
        let chunker = ContentChunker::new(ChunkStrategy::MediaAware);
        let data = vec![7u8; 1000]; // 1000 bytes, well below min_size
        let chunks = chunker
            .chunk_fastcdc(&data, 1024 * 1024, 512 * 1024, 4 * 1024 * 1024)
            .await
            .unwrap();
        assert_eq!(chunks.iter().map(|c| c.size).sum::<usize>(), 1000);
    }

    #[tokio::test]
    async fn test_fastcdc_deterministic() {
        // FastCDC should produce the same chunk boundaries for the same data
        let data = (0..50_000).map(|i| (i % 256) as u8).collect::<Vec<u8>>();

        // Use Rolling strategy with appropriate chunk sizes for test data
        let chunker = ContentChunker::new(ChunkStrategy::Rolling {
            avg_size: 8192,
            min_size: 4096,
            max_size: 16384,
        });

        // Chunk the same data twice
        let chunks1 = chunker.chunk(&data, "test.bin").await.unwrap();
        let chunks2 = chunker.chunk(&data, "test.bin").await.unwrap();

        // Should produce identical chunks
        assert_eq!(
            chunks1.len(),
            chunks2.len(),
            "FastCDC should be deterministic"
        );
        for (c1, c2) in chunks1.iter().zip(chunks2.iter()) {
            assert_eq!(c1.id, c2.id, "Chunk IDs should match for same content");
        }

        // Different data should produce different chunks
        let different = (0..50_000)
            .map(|i| ((i + 1) % 256) as u8)
            .collect::<Vec<u8>>();
        let chunks3 = chunker.chunk(&different, "test.bin").await.unwrap();
        assert_ne!(
            chunks1[0].id, chunks3[0].id,
            "Different content should produce different chunks"
        );
    }

    /// Deterministic pseudo-random bytes (LCG) — avoids the periodicity of a
    /// simple `i % 256` ramp, which repeats every 256 bytes and can make
    /// same-length chunk boundaries hash identically regardless of seed.
    fn pseudo_random_bytes(len: usize, mut state: u32) -> Vec<u8> {
        (0..len)
            .map(|_| {
                state = state.wrapping_mul(1_103_515_245).wrapping_add(12_345);
                (state >> 16) as u8
            })
            .collect()
    }

    #[tokio::test]
    async fn test_cdc_seed_changes_boundaries() {
        // Same data, seed=0 vs. a nonzero seed, should produce different chunk
        // boundaries (gear table is XORed with the seed before hashing).
        let data = pseudo_random_bytes(50_000, 42);

        let unseeded = ContentChunker::with_seed(
            ChunkStrategy::Rolling {
                avg_size: 8192,
                min_size: 4096,
                max_size: 16384,
            },
            0,
        );
        let seeded = ContentChunker::with_seed(
            ChunkStrategy::Rolling {
                avg_size: 8192,
                min_size: 4096,
                max_size: 16384,
            },
            0xDEAD_BEEF,
        );

        let chunks_unseeded = unseeded.chunk(&data, "test.bin").await.unwrap();
        let chunks_seeded = seeded.chunk(&data, "test.bin").await.unwrap();

        assert!(
            chunks_unseeded.len() > 3,
            "test data should produce more than 3 chunks"
        );

        let ids_unseeded: std::collections::HashSet<_> =
            chunks_unseeded.iter().map(|c| c.id).collect();
        let ids_seeded: std::collections::HashSet<_> = chunks_seeded.iter().map(|c| c.id).collect();
        assert_ne!(
            ids_unseeded, ids_seeded,
            "seed=0 and seed=0xDEADBEEF should produce different chunk boundaries"
        );
    }

    #[tokio::test]
    async fn test_cdc_seed_deterministic_across_instances() {
        // Same data, same nonzero seed, two separate chunker instances should
        // produce byte-identical boundaries (seed only affects the gear table,
        // not any per-instance state).
        let data = pseudo_random_bytes(50_000, 42);

        let chunker1 = ContentChunker::with_seed(
            ChunkStrategy::Rolling {
                avg_size: 8192,
                min_size: 4096,
                max_size: 16384,
            },
            0x1234_5678,
        );
        let chunker2 = ContentChunker::with_seed(
            ChunkStrategy::Rolling {
                avg_size: 8192,
                min_size: 4096,
                max_size: 16384,
            },
            0x1234_5678,
        );

        let chunks1 = chunker1.chunk(&data, "test.bin").await.unwrap();
        let chunks2 = chunker2.chunk(&data, "test.bin").await.unwrap();

        assert_eq!(chunks1.len(), chunks2.len());
        for (c1, c2) in chunks1.iter().zip(chunks2.iter()) {
            assert_eq!(c1.id, c2.id, "Same seed should be deterministic");
        }
    }

    #[test]
    fn test_parse_mp4_atoms() {
        // Create minimal MP4 structure
        let mut mp4 = Vec::new();

        // ftyp atom (20 bytes)
        mp4.extend_from_slice(&[0, 0, 0, 20]); // size = 20
        mp4.extend_from_slice(b"ftyp"); // type
        mp4.extend_from_slice(b"isom"); // brand
        mp4.extend_from_slice(&[0, 0, 0, 1]); // version
        mp4.extend_from_slice(b"isom"); // compatible brand

        // moov atom (16 bytes: 8 header + 8 mvhd)
        mp4.extend_from_slice(&[0, 0, 0, 16]); // size = 16
        mp4.extend_from_slice(b"moov"); // type
        mp4.extend_from_slice(&[0, 0, 0, 8]); // mvhd size
        mp4.extend_from_slice(b"mvhd"); // mvhd type

        let atoms = parse_mp4_atoms(&mp4);

        assert_eq!(atoms.len(), 2);
        assert_eq!(&atoms[0].atom_type, b"ftyp");
        assert_eq!(atoms[0].size, 20);
        assert_eq!(atoms[0].offset, 0);
        assert_eq!(atoms[0].header_size, 8);

        assert_eq!(&atoms[1].atom_type, b"moov");
        assert_eq!(atoms[1].size, 16);
        assert_eq!(atoms[1].offset, 20);
    }

    #[test]
    fn test_parse_mp4_extended_size() {
        // Test extended size atom (size == 1)
        let mut mp4 = Vec::new();
        mp4.extend_from_slice(&[0, 0, 0, 1]); // size = 1 (extended)
        mp4.extend_from_slice(b"mdat"); // type
        mp4.extend_from_slice(&[0, 0, 0, 0, 0, 0, 0, 24]); // 64-bit size = 24
        mp4.extend_from_slice(&[0; 8]); // 8 bytes of data

        let atoms = parse_mp4_atoms(&mp4);

        assert_eq!(atoms.len(), 1);
        assert_eq!(&atoms[0].atom_type, b"mdat");
        assert_eq!(atoms[0].size, 24);
        assert_eq!(atoms[0].header_size, 16);
    }

    #[test]
    fn test_parse_mp4_nested_moov() {
        // Create moov with nested atoms
        let mut moov_content = Vec::new();

        // mvhd (8 bytes header only for test)
        moov_content.extend_from_slice(&[0, 0, 0, 8]);
        moov_content.extend_from_slice(b"mvhd");

        // trak (8 bytes header only)
        moov_content.extend_from_slice(&[0, 0, 0, 8]);
        moov_content.extend_from_slice(b"trak");

        // udta (8 bytes header only)
        moov_content.extend_from_slice(&[0, 0, 0, 8]);
        moov_content.extend_from_slice(b"udta");

        let nested = parse_mp4_atoms(&moov_content);

        assert_eq!(nested.len(), 3);
        assert_eq!(&nested[0].atom_type, b"mvhd");
        assert_eq!(&nested[1].atom_type, b"trak");
        assert_eq!(&nested[2].atom_type, b"udta");
    }

    #[tokio::test]
    async fn test_chunk_mp4_basic() {
        let chunker = ContentChunker::new(ChunkStrategy::MediaAware);

        // Create minimal valid MP4
        let mut mp4 = Vec::new();

        // ftyp (20 bytes)
        mp4.extend_from_slice(&[0, 0, 0, 20]);
        mp4.extend_from_slice(b"ftyp");
        mp4.extend_from_slice(b"isom");
        mp4.extend_from_slice(&[0, 0, 0, 1]);
        mp4.extend_from_slice(b"isom");

        // moov (16 bytes with mvhd)
        mp4.extend_from_slice(&[0, 0, 0, 16]);
        mp4.extend_from_slice(b"moov");
        mp4.extend_from_slice(&[0, 0, 0, 8]);
        mp4.extend_from_slice(b"mvhd");

        // small mdat (20 bytes)
        mp4.extend_from_slice(&[0, 0, 0, 20]);
        mp4.extend_from_slice(b"mdat");
        mp4.extend_from_slice(&[0; 12]); // data

        let chunks = chunker.chunk(&mp4, "test.mp4").await.unwrap();

        // Should have: ftyp(1) + moov header(1) + mvhd(1) + mdat(1) = 4 chunks
        assert!(chunks.len() >= 3);

        // First chunk should be ftyp
        assert_eq!(chunks[0].chunk_type, ChunkType::Metadata);
        assert_eq!(chunks[0].size, 20);
    }

    #[tokio::test]
    #[ignore] // Requires test-files directory
    async fn test_chunk_mp4_real_file() {
        // Test with actual MP4 file
        let mp4_path = std::path::Path::new("../../test-files/101394-video-720.mp4");
        if !mp4_path.exists() {
            eprintln!("Skipping test: MP4 file not found at {:?}", mp4_path);
            return;
        }

        let data = std::fs::read(mp4_path).expect("Failed to read MP4 file");
        println!("Read {} bytes from MP4 file", data.len());

        let chunker = ContentChunker::new(ChunkStrategy::MediaAware);
        let chunks = chunker.chunk(&data, "101394-video-720.mp4").await.unwrap();

        println!("Parsed into {} chunks", chunks.len());

        // Count by type
        let metadata = chunks
            .iter()
            .filter(|c| c.chunk_type == ChunkType::Metadata)
            .count();
        let video = chunks
            .iter()
            .filter(|c| c.chunk_type == ChunkType::VideoStream)
            .count();
        let generic = chunks
            .iter()
            .filter(|c| c.chunk_type == ChunkType::Generic)
            .count();

        println!(
            "Chunk breakdown: Metadata={}, VideoStream={}, Generic={}",
            metadata, video, generic
        );

        // Verify first chunk is ftyp (Metadata)
        assert_eq!(chunks[0].chunk_type, ChunkType::Metadata);
        assert!(chunks[0].size <= 100, "ftyp should be small");

        // Should have at least ftyp + moov parts + mdat parts
        assert!(chunks.len() >= 3, "Expected at least 3 chunks");
        assert!(
            metadata >= 2,
            "Expected at least 2 metadata chunks (ftyp, moov header)"
        );

        // Verify total size matches file size
        let total_size: usize = chunks.iter().map(|c| c.size).sum();
        assert_eq!(
            total_size,
            data.len(),
            "Chunk sizes should sum to file size"
        );

        println!("✓ Real MP4 file test passed!");
    }

    // ==================== EBML/Matroska Tests ====================

    #[test]
    fn test_read_ebml_id_1byte() {
        // 1-byte ID: 0xBF (CRC-32)
        assert_eq!(read_ebml_id(&[0xBF], 0), Some((0xBF, 1)));
        // 1-byte ID: 0xEC (Void)
        assert_eq!(read_ebml_id(&[0xEC], 0), Some((0xEC, 1)));
    }

    #[test]
    fn test_read_ebml_id_4byte() {
        // 4-byte ID: EBML header (0x1A45DFA3)
        assert_eq!(
            read_ebml_id(&[0x1A, 0x45, 0xDF, 0xA3], 0),
            Some((0x1A45DFA3, 4))
        );
        // 4-byte ID: Segment (0x18538067)
        assert_eq!(
            read_ebml_id(&[0x18, 0x53, 0x80, 0x67], 0),
            Some((0x18538067, 4))
        );
        // 4-byte ID: Cluster (0x1F43B675)
        assert_eq!(
            read_ebml_id(&[0x1F, 0x43, 0xB6, 0x75], 0),
            Some((0x1F43B675, 4))
        );
    }

    #[test]
    fn test_read_ebml_id_invalid() {
        // Invalid: all zeros
        assert_eq!(read_ebml_id(&[0x00], 0), None);
        // Invalid: empty
        assert_eq!(read_ebml_id(&[], 0), None);
        // Invalid: truncated 4-byte ID
        assert_eq!(read_ebml_id(&[0x1A, 0x45], 0), None);
    }

    #[test]
    fn test_read_ebml_size_1byte() {
        // 1-byte size: 50 (0x80 | 50 = 0xB2)
        assert_eq!(read_ebml_size(&[0x80 | 50], 0), Some((50, 1)));
        // 1-byte size: 0 (0x80)
        assert_eq!(read_ebml_size(&[0x80], 0), Some((0, 1)));
        // 1-byte size: 127 - but 0xFF is unknown!
        assert_eq!(read_ebml_size(&[0x80 | 126], 0), Some((126, 1)));
    }

    #[test]
    fn test_read_ebml_size_unknown() {
        // Unknown size (1-byte): 0xFF (all data bits = 1)
        assert_eq!(read_ebml_size(&[0xFF], 0), Some((u64::MAX, 1)));
        // Unknown size (2-byte): 0x7FFF
        assert_eq!(read_ebml_size(&[0x7F, 0xFF], 0), Some((u64::MAX, 2)));
    }

    #[test]
    fn test_read_ebml_size_2byte() {
        // 2-byte size: 0x4000 = size 0
        assert_eq!(read_ebml_size(&[0x40, 0x00], 0), Some((0, 2)));
        // 2-byte size: 0x4001 = size 1
        assert_eq!(read_ebml_size(&[0x40, 0x01], 0), Some((1, 2)));
    }

    #[test]
    fn test_parse_ebml_elements_empty() {
        let elements = parse_ebml_elements(&[]);
        assert!(elements.is_empty());
    }

    #[test]
    fn test_parse_ebml_elements_ebml_header() {
        // EBML header (0x1A45DFA3) + size 3 (0x83) + 3 bytes data
        let data = [0x1A, 0x45, 0xDF, 0xA3, 0x83, 0x00, 0x00, 0x00];
        let elements = parse_ebml_elements(&data);

        assert_eq!(elements.len(), 1);
        assert_eq!(elements[0].id, EBML_ID);
        assert_eq!(elements[0].offset, 0);
        assert_eq!(elements[0].header_size, 5); // 4 bytes ID + 1 byte size
        assert_eq!(elements[0].data_size, 3);
    }

    #[test]
    fn test_parse_ebml_elements_void_skipped() {
        // Void element (0xEC) should be skipped
        // EBML + Void + EBML
        let mut data = vec![];
        // First EBML header + size 0
        data.extend_from_slice(&[0x1A, 0x45, 0xDF, 0xA3, 0x80]);
        // Void + size 2 + 2 bytes padding
        data.extend_from_slice(&[0xEC, 0x82, 0x00, 0x00]);

        let elements = parse_ebml_elements(&data);

        // Should have 1 element (EBML), Void is skipped
        assert_eq!(elements.len(), 1);
        assert_eq!(elements[0].id, EBML_ID);
    }

    #[tokio::test]
    async fn test_chunk_matroska_fallback_on_invalid() {
        let chunker = ContentChunker::new(ChunkStrategy::MediaAware);

        // Invalid data (not EBML)
        let chunks = chunker
            .chunk(&[0x00, 0x01, 0x02, 0x03], "test.mkv")
            .await
            .unwrap();

        // Should fall back to fixed chunking (1 chunk for small data)
        assert!(!chunks.is_empty());
    }

    #[tokio::test]
    async fn test_chunk_matroska_basic() {
        let chunker = ContentChunker::new(ChunkStrategy::MediaAware);

        // Create minimal valid Matroska
        let mut mkv = vec![];

        // EBML header (0x1A45DFA3) + size 0
        mkv.extend_from_slice(&[0x1A, 0x45, 0xDF, 0xA3, 0x80]);

        // Segment (0x18538067) + unknown size (0xFF)
        mkv.extend_from_slice(&[0x18, 0x53, 0x80, 0x67, 0xFF]);

        // Info (0x1549A966) + size 0
        mkv.extend_from_slice(&[0x15, 0x49, 0xA9, 0x66, 0x80]);

        // Cluster (0x1F43B675) + size 4 + 4 bytes data
        mkv.extend_from_slice(&[0x1F, 0x43, 0xB6, 0x75, 0x84, 0x00, 0x00, 0x00, 0x00]);

        let chunks = chunker.chunk(&mkv, "test.mkv").await.unwrap();

        // Per-element chunking: EBML(1) + Segment header(1) + Info(1) + Cluster(1) = 4 chunks
        assert!(
            chunks.len() >= 3,
            "Expected at least 3 chunks (EBML + Info + Cluster), got {}",
            chunks.len()
        );

        // Should have at least one VideoStream chunk (Cluster)
        let video_count = chunks
            .iter()
            .filter(|c| c.chunk_type == ChunkType::VideoStream)
            .count();
        assert!(video_count >= 1, "Expected at least 1 VideoStream chunk");

        // Each metadata element should be its own chunk
        let meta_count = chunks
            .iter()
            .filter(|c| c.chunk_type == ChunkType::Metadata)
            .count();
        assert!(
            meta_count >= 3,
            "Expected at least 3 Metadata chunks (EBML + Segment header + Info), got {}",
            meta_count
        );
    }

    #[tokio::test]
    async fn test_chunk_matroska_metadata_splitting() {
        let chunker = ContentChunker::new(ChunkStrategy::MediaAware);

        let mut mkv = vec![];

        // EBML header + size 2 + 2 bytes content
        mkv.extend_from_slice(&[0x1A, 0x45, 0xDF, 0xA3, 0x82, 0xAA, 0xBB]);

        // Segment + unknown size
        mkv.extend_from_slice(&[0x18, 0x53, 0x80, 0x67, 0xFF]);

        // Info + size 3 + content
        mkv.extend_from_slice(&[0x15, 0x49, 0xA9, 0x66, 0x83, 0x01, 0x02, 0x03]);

        // Tracks + size 2 + content
        mkv.extend_from_slice(&[0x16, 0x54, 0xAE, 0x6B, 0x82, 0x04, 0x05]);

        // Tags + size 2 + content
        mkv.extend_from_slice(&[0x12, 0x54, 0xC3, 0x67, 0x82, 0x06, 0x07]);

        // Cluster + size 4 + content
        mkv.extend_from_slice(&[0x1F, 0x43, 0xB6, 0x75, 0x84, 0x10, 0x11, 0x12, 0x13]);

        let chunks = chunker.chunk(&mkv, "test.mkv").await.unwrap();

        // Should have separate chunks: EBML, Segment hdr, Info, Tracks, Tags, Cluster
        let meta_chunks: Vec<_> = chunks
            .iter()
            .filter(|c| c.chunk_type == ChunkType::Metadata)
            .collect();
        let video_chunks: Vec<_> = chunks
            .iter()
            .filter(|c| c.chunk_type == ChunkType::VideoStream)
            .collect();

        assert_eq!(video_chunks.len(), 1, "Should have exactly 1 Cluster chunk");
        assert!(
            meta_chunks.len() >= 5,
            "Expected at least 5 metadata chunks (EBML + Segment hdr + Info + Tracks + Tags), got {}",
            meta_chunks.len()
        );

        // Verify that Info, Tracks, and Tags are separate — changing one doesn't
        // invalidate others (the core dedup improvement)
        let info_chunk = chunks.iter().find(|c| {
            c.chunk_type == ChunkType::Metadata && c.data.starts_with(&[0x15, 0x49, 0xA9, 0x66])
        });
        let tracks_chunk = chunks.iter().find(|c| {
            c.chunk_type == ChunkType::Metadata && c.data.starts_with(&[0x16, 0x54, 0xAE, 0x6B])
        });
        let tags_chunk = chunks.iter().find(|c| {
            c.chunk_type == ChunkType::Metadata && c.data.starts_with(&[0x12, 0x54, 0xC3, 0x67])
        });

        assert!(info_chunk.is_some(), "Info should be its own chunk");
        assert!(tracks_chunk.is_some(), "Tracks should be its own chunk");
        assert!(tags_chunk.is_some(), "Tags should be its own chunk");

        // Verify they have different hashes (different content = different IDs)
        assert_ne!(info_chunk.unwrap().id, tracks_chunk.unwrap().id);
        assert_ne!(tracks_chunk.unwrap().id, tags_chunk.unwrap().id);
    }

    #[tokio::test]
    async fn test_chunk_matroska_large_cluster_subdivision() {
        let chunker = ContentChunker::new(ChunkStrategy::MediaAware);

        let mut mkv = vec![];

        // EBML header + size 0
        mkv.extend_from_slice(&[0x1A, 0x45, 0xDF, 0xA3, 0x80]);

        // Segment + unknown size
        mkv.extend_from_slice(&[0x18, 0x53, 0x80, 0x67, 0xFF]);

        // Large Cluster: header + 5MB of data (triggers CDC subdivision at >4MB)
        let cluster_content_size: usize = 5 * 1024 * 1024;
        mkv.extend_from_slice(&[0x1F, 0x43, 0xB6, 0x75]); // Cluster ID
                                                          // Encode size as 4-byte VINT: marker bit in first byte
        let size_val = cluster_content_size as u32;
        mkv.push(0x10 | ((size_val >> 24) & 0x0F) as u8); // 4-byte VINT marker
        mkv.push((size_val >> 16) as u8);
        mkv.push((size_val >> 8) as u8);
        mkv.push(size_val as u8);
        // Fill with pseudo-random data for CDC to find boundaries
        let mut rng_val: u32 = 0xDEADBEEF;
        for _ in 0..cluster_content_size {
            rng_val = rng_val.wrapping_mul(1103515245).wrapping_add(12345);
            mkv.push((rng_val >> 16) as u8);
        }

        let chunks = chunker.chunk(&mkv, "test.mkv").await.unwrap();

        // Should have: EBML(1) + Segment hdr(1) + Cluster header(1) + CDC sub-chunks(multiple)
        let video_chunks: Vec<_> = chunks
            .iter()
            .filter(|c| c.chunk_type == ChunkType::VideoStream)
            .collect();

        assert!(
            video_chunks.len() > 1,
            "Large cluster should be subdivided into multiple VideoStream chunks, got {}",
            video_chunks.len()
        );

        // Verify total video data size matches original cluster content
        let total_video_size: usize = video_chunks.iter().map(|c| c.size).sum();
        assert_eq!(
            total_video_size, cluster_content_size,
            "CDC sub-chunks should cover entire cluster content"
        );
    }

    // Build a minimal RIFF/AVI blob with a movi LIST containing video and audio sub-chunks.
    // `video_payload` is the raw data inside the 00dc chunk.
    // `audio_payload` is the raw data inside the 01wb chunk.
    fn make_avi(video_payload: &[u8], audio_payload: &[u8]) -> Vec<u8> {
        let mut movi_content = Vec::new();
        // 00dc video chunk
        movi_content.extend_from_slice(b"00dc");
        let vlen = video_payload.len() as u32;
        movi_content.extend_from_slice(&vlen.to_le_bytes());
        movi_content.extend_from_slice(video_payload);
        if !video_payload.len().is_multiple_of(2) {
            movi_content.push(0); // RIFF padding
        }
        // 01wb audio chunk
        movi_content.extend_from_slice(b"01wb");
        let alen = audio_payload.len() as u32;
        movi_content.extend_from_slice(&alen.to_le_bytes());
        movi_content.extend_from_slice(audio_payload);
        if !audio_payload.len().is_multiple_of(2) {
            movi_content.push(0);
        }

        // LIST/movi: "LIST" + size(4) + "movi" + content
        let movi_size = (4 + movi_content.len()) as u32; // includes "movi" type
        let mut list_movi = Vec::new();
        list_movi.extend_from_slice(b"LIST");
        list_movi.extend_from_slice(&movi_size.to_le_bytes());
        list_movi.extend_from_slice(b"movi");
        list_movi.extend_from_slice(&movi_content);

        // Minimal hdrl placeholder (just a JUNK chunk)
        let mut hdrl: Vec<u8> = Vec::new();
        hdrl.extend_from_slice(b"JUNK");
        hdrl.extend_from_slice(&4u32.to_le_bytes());
        hdrl.extend_from_slice(&[0u8; 4]);

        // RIFF/AVI header: "RIFF" + file_size + "AVI "
        let body_size = (hdrl.len() + list_movi.len()) as u32;
        let file_size = 4 + body_size; // "AVI " + body
        let mut out = Vec::new();
        out.extend_from_slice(b"RIFF");
        out.extend_from_slice(&file_size.to_le_bytes());
        out.extend_from_slice(b"AVI ");
        out.extend_from_slice(&hdrl);
        out.extend_from_slice(&list_movi);
        out
    }

    #[tokio::test]
    async fn test_chunk_avi_movi_descends_into_subchunks() {
        let chunker = ContentChunker::new(ChunkStrategy::MediaAware);
        let video = vec![0xAAu8; 64];
        let audio = vec![0xBBu8; 32];
        let avi = make_avi(&video, &audio);

        let chunks = chunker.chunk(&avi, "test.avi").await.unwrap();

        // CDC on the movi region produces VideoStream-typed chunks (no per-stream
        // type detection; byte-exact reconstruction is the primary goal).
        let video_chunks: Vec<_> = chunks
            .iter()
            .filter(|c| c.chunk_type == ChunkType::VideoStream)
            .collect();
        let audio_chunks: Vec<_> = chunks
            .iter()
            .filter(|c| c.chunk_type == ChunkType::AudioStream)
            .collect();

        assert!(
            !video_chunks.is_empty(),
            "movi content must produce at least one VideoStream chunk"
        );
        assert_eq!(
            audio_chunks.len(),
            0,
            "CDC does not emit AudioStream chunks (no per-stream type detection)"
        );

        // Reconstruction integrity: concatenating chunks in offset order must
        // reproduce the exact original bytes.
        let mut sorted = chunks.clone();
        sorted.sort_unstable_by_key(|c| c.offset);
        let reconstructed: Vec<u8> = sorted.iter().flat_map(|c| c.data.iter().copied()).collect();
        assert_eq!(
            reconstructed, avi,
            "chunk reconstruction must be byte-exact"
        );
    }

    // =========================================================
    // S5: GLB BIN sub-chunking tests
    // =========================================================

    /// Build a minimal valid GLB file from a JSON payload and a BIN payload.
    ///
    /// GLB structure:
    ///   [magic(4)] [version(4)] [total_len(4)]
    ///   [json_len(4)] [0x4E4F534A = "JSON"(4)] [json_data]
    ///   [bin_len(4)]  [0x004E4942 = "BIN\0"(4)] [bin_data]
    fn make_glb(json: &[u8], bin: &[u8]) -> Vec<u8> {
        let mut buf = Vec::new();
        // Header: "glTF" + version 2 + total length (computed below, patched)
        buf.extend_from_slice(b"glTF");
        buf.extend_from_slice(&2u32.to_le_bytes());
        let total_len_offset = buf.len();
        buf.extend_from_slice(&0u32.to_le_bytes()); // placeholder

        // JSON chunk
        buf.extend_from_slice(&(json.len() as u32).to_le_bytes());
        buf.extend_from_slice(&0x4E4F534Au32.to_le_bytes()); // "JSON"
        buf.extend_from_slice(json);

        // BIN chunk (only emit if non-empty)
        if !bin.is_empty() {
            buf.extend_from_slice(&(bin.len() as u32).to_le_bytes());
            buf.extend_from_slice(&0x004E4942u32.to_le_bytes()); // "BIN\0"
            buf.extend_from_slice(bin);
        }

        // Patch total length
        let total = buf.len() as u32;
        buf[total_len_offset..total_len_offset + 4].copy_from_slice(&total.to_le_bytes());
        buf
    }

    #[tokio::test]
    async fn test_glb_small_bin_single_chunk() {
        // BIN payload < 4 MB → must stay as a single Generic chunk.
        let json = b"{}";
        let bin = vec![0xBBu8; 1024]; // 1 KB — well below 4 MB threshold
        let data = make_glb(json, &bin);

        let chunker = ContentChunker::new(ChunkStrategy::MediaAware);
        let chunks = chunker.chunk_glb(&data).await.unwrap();

        // Must produce: GLB header (Metadata) + JSON header (Metadata) + JSON data (Metadata)
        //             + BIN header (Metadata) + BIN data (Generic, single chunk)
        let generic: Vec<_> = chunks
            .iter()
            .filter(|c| c.chunk_type == ChunkType::Generic)
            .collect();
        assert_eq!(generic.len(), 1, "Small BIN must be a single Generic chunk");
        assert_eq!(
            generic[0].size,
            bin.len(),
            "Single BIN chunk size must match full payload"
        );
    }

    #[tokio::test]
    async fn test_glb_large_bin_is_subdivided() {
        // BIN payload > 4 MB → must be CDC-subdivided into multiple Generic chunks.
        let json = b"{}";
        let bin = vec![0xAAu8; 6 * 1024 * 1024]; // 6 MB — above 4 MB threshold
        let data = make_glb(json, &bin);

        let chunker = ContentChunker::new(ChunkStrategy::MediaAware);
        let chunks = chunker.chunk_glb(&data).await.unwrap();

        let generic: Vec<_> = chunks
            .iter()
            .filter(|c| c.chunk_type == ChunkType::Generic)
            .collect();
        assert!(
            generic.len() > 1,
            "Large BIN (6 MB) must be split into multiple Generic chunks via FastCDC, got {}",
            generic.len()
        );

        // The sub-chunks must cover the full BIN payload in total
        let total: usize = generic.iter().map(|c| c.size).sum();
        assert_eq!(
            total,
            bin.len(),
            "Sum of BIN sub-chunk sizes must equal full BIN payload"
        );
    }

    #[tokio::test]
    async fn test_glb_json_chunk_always_single_metadata() {
        // JSON chunk must always be emitted as a single Metadata chunk regardless of BIN.
        let json = b"{\"asset\":{\"version\":\"2.0\"}}";
        let bin = vec![0u8; 512]; // small BIN
        let data = make_glb(json, &bin);

        let chunker = ContentChunker::new(ChunkStrategy::MediaAware);
        let chunks = chunker.chunk_glb(&data).await.unwrap();

        let metadata: Vec<_> = chunks
            .iter()
            .filter(|c| c.chunk_type == ChunkType::Metadata)
            .collect();
        // Metadata chunks: GLB header (12B) + JSON section header (8B) + JSON data + BIN header (8B)
        assert!(
            !metadata.is_empty(),
            "Must produce at least one Metadata chunk"
        );
        // Verify JSON payload is somewhere in a metadata chunk
        let json_in_metadata = metadata.iter().any(|c| c.data == json);
        assert!(
            json_in_metadata,
            "JSON payload must appear in a Metadata chunk"
        );
    }

    #[tokio::test]
    async fn test_glb_large_bin_different_data_different_chunk_ids() {
        // Two GLBs with same JSON but different BIN → their sub-chunks must NOT share OIDs.
        // Regression: verifies the sub-chunk hash covers only the unique data.
        let json = b"{}";
        let bin_a = vec![0xAAu8; 6 * 1024 * 1024];
        let bin_b = vec![0xBBu8; 6 * 1024 * 1024];

        let chunker = ContentChunker::new(ChunkStrategy::MediaAware);
        let chunks_a = chunker.chunk_glb(&make_glb(json, &bin_a)).await.unwrap();
        let chunks_b = chunker.chunk_glb(&make_glb(json, &bin_b)).await.unwrap();

        let ids_a: std::collections::HashSet<_> = chunks_a
            .iter()
            .filter(|c| c.chunk_type == ChunkType::Generic)
            .map(|c| c.id)
            .collect();
        let ids_b: std::collections::HashSet<_> = chunks_b
            .iter()
            .filter(|c| c.chunk_type == ChunkType::Generic)
            .map(|c| c.id)
            .collect();

        let shared: Vec<_> = ids_a.intersection(&ids_b).collect();
        assert!(
            shared.is_empty(),
            "Completely different BIN payloads must produce zero shared chunk OIDs"
        );
    }

    #[tokio::test]
    async fn test_glb_no_bin_sections_ok() {
        // A GLB with only a JSON chunk (no BIN) must not panic and must return chunks.
        let json = b"{}";
        let data = make_glb(json, &[]);

        let chunker = ContentChunker::new(ChunkStrategy::MediaAware);
        let chunks = chunker.chunk_glb(&data).await.unwrap();
        assert!(
            !chunks.is_empty(),
            "GLB with no BIN must still produce chunks"
        );
    }

    #[tokio::test]
    async fn test_glb_invalid_data_falls_back() {
        // Non-GLB data passed to chunk_glb → must fall back (no panic, returns non-empty).
        let data = vec![0u8; 64]; // no glTF magic
        let chunker = ContentChunker::new(ChunkStrategy::MediaAware);
        let chunks = chunker.chunk_glb(&data).await.unwrap();
        assert!(
            !chunks.is_empty(),
            "Fallback must still produce at least one chunk"
        );
    }

    #[test]
    fn test_creative_chunk_params_stable_across_tiers() {
        // Guards the BUG-003 / v6 fix: creative containers (AI/PDF/PSD/INDD)
        // must use the small-chunk params (1 MB avg, 512 KB min, 4 MB max)
        // regardless of file size. Drifting back to tier-2+ params would
        // regress delta_psd past the 35 % workspace target
        // (see dev-tests/standalone-deep-v6/reports/verification-plan-v6.md).
        const MB: usize = 1024 * 1024;
        for &size in &[
            1u64,
            50 * 1024 * 1024,         // tier-1
            500 * 1024 * 1024,        // tier-2 territory
            50 * 1024 * 1024 * 1024,  // tier-3 territory
            500 * 1024 * 1024 * 1024, // tier-4 territory
        ] {
            let (avg, min, max) = get_creative_chunk_params(size);
            assert_eq!(avg, MB, "creative avg drifted at size={}", size);
            assert_eq!(min, 512 * 1024, "creative min drifted at size={}", size);
            assert_eq!(max, 4 * MB, "creative max drifted at size={}", size);
        }
    }

    // ==================== P2: Codec Detection Tests ====================

    /// Build a `size(4 BE) + fourcc(4) + content` MP4-style atom.
    fn mp4_atom_bytes(fourcc: &[u8; 4], content: &[u8]) -> Vec<u8> {
        let size = (8 + content.len()) as u32;
        let mut out = Vec::with_capacity(size as usize);
        out.extend_from_slice(&size.to_be_bytes());
        out.extend_from_slice(fourcc);
        out.extend_from_slice(content);
        out
    }

    /// Build a minimal MP4 (ftyp + moov→trak→mdia→minf→stbl→stsd + mdat) whose
    /// `stsd` has a single sample entry with `entry_count` claimed vs. `entries`
    /// actually present — pass `entries` empty to simulate a truncated/garbage
    /// stsd (claims entries, has none).
    fn make_mp4_with_stsd(entry_count: u32, entries: &[[u8; 4]]) -> Vec<u8> {
        let mut stsd_content = vec![0, 0, 0, 0]; // version + flags
        stsd_content.extend_from_slice(&entry_count.to_be_bytes());
        for fourcc in entries {
            stsd_content.extend_from_slice(&mp4_atom_bytes(fourcc, &[]));
        }
        let stsd = mp4_atom_bytes(b"stsd", &stsd_content);
        let stbl = mp4_atom_bytes(b"stbl", &stsd);
        let minf = mp4_atom_bytes(b"minf", &stbl);
        let mdia = mp4_atom_bytes(b"mdia", &minf);
        let trak = mp4_atom_bytes(b"trak", &mdia);
        let moov = mp4_atom_bytes(b"moov", &trak);
        let ftyp = mp4_atom_bytes(b"ftyp", b"isom\x00\x00\x00\x01isom");
        let mdat = mp4_atom_bytes(b"mdat", &[0xABu8; 32]);

        let mut data = Vec::new();
        data.extend_from_slice(&ftyp);
        data.extend_from_slice(&moov);
        data.extend_from_slice(&mdat);
        data
    }

    #[tokio::test]
    async fn test_mp4_avc1_stsd_yields_h264_on_mdat() {
        let data = make_mp4_with_stsd(1, &[*b"avc1"]);
        let chunker = ContentChunker::new(ChunkStrategy::MediaAware);
        let chunks = chunker.chunk(&data, "test.mp4").await.unwrap();

        let mdat_chunk = chunks
            .iter()
            .find(|c| c.chunk_type == ChunkType::VideoStream)
            .expect("mdat should produce a VideoStream chunk");
        assert_eq!(mdat_chunk.codec_hint, CodecHint::H264);

        // Boundaries must be unaffected by codec detection.
        let total: usize = chunks.iter().map(|c| c.size).sum();
        assert_eq!(
            total,
            data.len(),
            "chunking must still fully cover the file"
        );
    }

    #[tokio::test]
    async fn test_mp4_truncated_stsd_no_panic_unknown_hint() {
        // entry_count claims 5 entries but zero are actually present.
        let data = make_mp4_with_stsd(5, &[]);
        let chunker = ContentChunker::new(ChunkStrategy::MediaAware);
        let chunks = chunker.chunk(&data, "test.mp4").await.unwrap();

        let mdat_chunk = chunks
            .iter()
            .find(|c| c.chunk_type == ChunkType::VideoStream)
            .expect("mdat should still produce a VideoStream chunk");
        assert_eq!(mdat_chunk.codec_hint, CodecHint::Unknown);

        let total: usize = chunks.iter().map(|c| c.size).sum();
        assert_eq!(
            total,
            data.len(),
            "chunking must still fully cover the file"
        );
    }

    #[test]
    fn test_mkv_codec_id_to_hint_mapping() {
        assert_eq!(mkv_codec_id_to_hint("V_MPEG4/ISO/AVC"), CodecHint::H264);
        assert_eq!(mkv_codec_id_to_hint("V_MPEGH/ISO/HEVC"), CodecHint::H265);
        assert_eq!(mkv_codec_id_to_hint("V_VP9"), CodecHint::VP9);
        assert_eq!(mkv_codec_id_to_hint("V_AV1"), CodecHint::AV1);
        assert_eq!(mkv_codec_id_to_hint("V_PRORES"), CodecHint::ProRes);
        assert_eq!(mkv_codec_id_to_hint("V_UNCOMPRESSED"), CodecHint::RawVideo);
        assert_eq!(mkv_codec_id_to_hint("A_AAC"), CodecHint::AAC);
        assert_eq!(mkv_codec_id_to_hint("A_AAC/MPEG4/LC"), CodecHint::AAC);
        assert_eq!(mkv_codec_id_to_hint("A_OPUS"), CodecHint::Opus);
        assert_eq!(mkv_codec_id_to_hint("A_VORBIS"), CodecHint::Vorbis);
        assert_eq!(mkv_codec_id_to_hint("A_FLAC"), CodecHint::FLAC);
        assert_eq!(mkv_codec_id_to_hint("A_PCM/INT/LIT"), CodecHint::PCM);
        assert_eq!(mkv_codec_id_to_hint("A_MPEG/L3"), CodecHint::MP3);
        assert_eq!(mkv_codec_id_to_hint("A_ALAC"), CodecHint::ALAC);
        assert_eq!(mkv_codec_id_to_hint("S_TEXT/UTF8"), CodecHint::TextSub);
        assert_eq!(mkv_codec_id_to_hint("S_HDMV/PGS"), CodecHint::BitmapSub);
        assert_eq!(mkv_codec_id_to_hint("S_VOBSUB"), CodecHint::BitmapSub);
        assert_eq!(mkv_codec_id_to_hint("X_UNKNOWN_CODEC"), CodecHint::Unknown);
    }

    #[tokio::test]
    async fn test_mkv_truncated_tracks_no_panic_unknown_hint() {
        let mut mkv = vec![];
        mkv.extend_from_slice(&[0x1A, 0x45, 0xDF, 0xA3, 0x80]); // EBML header, size 0
        mkv.extend_from_slice(&[0x18, 0x53, 0x80, 0x67, 0xFF]); // Segment, unknown size

        // Tracks → TrackEntry claims data_size=10 but only 2 bytes of body
        // actually follow — garbage the CodecID scan must not choke on.
        let tracks_content: Vec<u8> = vec![0xAE, 0x8A, 0x01, 0x02];
        mkv.extend_from_slice(&[0x16, 0x54, 0xAE, 0x6B]); // Tracks ID
        mkv.push(0x80 | tracks_content.len() as u8); // Tracks size
        mkv.extend_from_slice(&tracks_content);

        // Cluster + size 4 + 4 bytes content
        mkv.extend_from_slice(&[0x1F, 0x43, 0xB6, 0x75, 0x84, 0x00, 0x00, 0x00, 0x00]);

        let chunker = ContentChunker::new(ChunkStrategy::MediaAware);
        let chunks = chunker.chunk(&mkv, "test.mkv").await.unwrap();

        let cluster_chunk = chunks
            .iter()
            .find(|c| c.chunk_type == ChunkType::VideoStream)
            .expect("Cluster should still produce a VideoStream chunk");
        assert_eq!(cluster_chunk.codec_hint, CodecHint::Unknown);

        let total: usize = chunks.iter().map(|c| c.size).sum();
        assert_eq!(total, mkv.len(), "chunking must still fully cover the file");
    }

    /// Build a minimal AVI with a `hdrl→strl→strh(+strf)` describing a single
    /// `vids` stream, plus a small `movi` payload.
    fn make_avi_with_strh(fcc_handler: &[u8; 4], strf: Option<&[u8]>) -> Vec<u8> {
        let mut strh_content = Vec::new();
        strh_content.extend_from_slice(b"vids");
        strh_content.extend_from_slice(fcc_handler);
        strh_content.extend_from_slice(&[0u8; 40]); // rest of AVISTREAMHEADER (unused by parser)

        let mut strl_content = Vec::new();
        strl_content.extend_from_slice(b"strh");
        strl_content.extend_from_slice(&(strh_content.len() as u32).to_le_bytes());
        strl_content.extend_from_slice(&strh_content);

        if let Some(strf_data) = strf {
            strl_content.extend_from_slice(b"strf");
            strl_content.extend_from_slice(&(strf_data.len() as u32).to_le_bytes());
            strl_content.extend_from_slice(strf_data);
            if !strf_data.len().is_multiple_of(2) {
                strl_content.push(0);
            }
        }

        let mut list_strl = Vec::new();
        list_strl.extend_from_slice(b"LIST");
        list_strl.extend_from_slice(&((4 + strl_content.len()) as u32).to_le_bytes());
        list_strl.extend_from_slice(b"strl");
        list_strl.extend_from_slice(&strl_content);

        let mut list_hdrl = Vec::new();
        list_hdrl.extend_from_slice(b"LIST");
        list_hdrl.extend_from_slice(&((4 + list_strl.len()) as u32).to_le_bytes());
        list_hdrl.extend_from_slice(b"hdrl");
        list_hdrl.extend_from_slice(&list_strl);

        let mut movi_content = Vec::new();
        movi_content.extend_from_slice(b"00dc");
        let video_payload = vec![0xAAu8; 16];
        movi_content.extend_from_slice(&(video_payload.len() as u32).to_le_bytes());
        movi_content.extend_from_slice(&video_payload);

        let mut list_movi = Vec::new();
        list_movi.extend_from_slice(b"LIST");
        list_movi.extend_from_slice(&((4 + movi_content.len()) as u32).to_le_bytes());
        list_movi.extend_from_slice(b"movi");
        list_movi.extend_from_slice(&movi_content);

        let body_size = (list_hdrl.len() + list_movi.len()) as u32;
        let file_size = 4 + body_size;
        let mut out = Vec::new();
        out.extend_from_slice(b"RIFF");
        out.extend_from_slice(&file_size.to_le_bytes());
        out.extend_from_slice(b"AVI ");
        out.extend_from_slice(&list_hdrl);
        out.extend_from_slice(&list_movi);
        out
    }

    #[tokio::test]
    async fn test_avi_unrecognized_fcc_handler_no_strf_yields_unknown_hint() {
        // fccType=vids but fccHandler is unrecognized and there is no strf —
        // must fall back to Unknown, never panic.
        let avi = make_avi_with_strh(b"XXXX", None);
        let chunker = ContentChunker::new(ChunkStrategy::MediaAware);
        let chunks = chunker.chunk(&avi, "test.avi").await.unwrap();

        let video_chunk = chunks
            .iter()
            .find(|c| c.chunk_type == ChunkType::VideoStream)
            .expect("movi should still produce a VideoStream chunk");
        assert_eq!(video_chunk.codec_hint, CodecHint::Unknown);

        let mut sorted = chunks.clone();
        sorted.sort_unstable_by_key(|c| c.offset);
        let reconstructed: Vec<u8> = sorted.iter().flat_map(|c| c.data.iter().copied()).collect();
        assert_eq!(
            reconstructed, avi,
            "chunk reconstruction must be byte-exact"
        );
    }

    #[tokio::test]
    async fn test_avi_h264_fcc_handler_yields_h264_hint() {
        let avi = make_avi_with_strh(b"H264", None);
        let chunker = ContentChunker::new(ChunkStrategy::MediaAware);
        let chunks = chunker.chunk(&avi, "test.avi").await.unwrap();

        let video_chunk = chunks
            .iter()
            .find(|c| c.chunk_type == ChunkType::VideoStream)
            .expect("movi should still produce a VideoStream chunk");
        assert_eq!(video_chunk.codec_hint, CodecHint::H264);
    }

    #[test]
    fn test_codec_detect_env_parsing() {
        assert!(
            !codec_detect_enabled_from_env(Some("0")),
            "\"0\" must disable detection"
        );
        assert!(codec_detect_enabled_from_env(Some("1")));
        assert!(
            codec_detect_enabled_from_env(None),
            "unset env var defaults to enabled"
        );
    }

    #[test]
    fn test_apply_codec_hint_if_respects_kill_switch() {
        let chunk = ContentChunk {
            id: Oid::hash(b"x"),
            data: b"x".to_vec(),
            offset: 0,
            size: 1,
            chunk_type: ChunkType::Generic,
            perceptual_hash: None,
            codec_hint: CodecHint::Unknown,
        };

        let disabled = apply_codec_hint_if(vec![chunk.clone()], CodecHint::PCM, false);
        assert_eq!(
            disabled[0].codec_hint,
            CodecHint::Unknown,
            "disabled must leave hints as Unknown (exact pre-detection behavior)"
        );

        let enabled = apply_codec_hint_if(vec![chunk], CodecHint::PCM, true);
        assert_eq!(
            enabled[0].codec_hint,
            CodecHint::PCM,
            "enabled must apply the hint"
        );
    }

    #[tokio::test]
    #[ignore] // Requires test-files directory
    async fn test_chunk_mkv_real_fixture_roundtrip_h264() {
        let path = std::path::Path::new("../../test-files/video-variants/bbb-5s-h264.mkv");
        if !path.exists() {
            eprintln!("Skipping test: fixture not found at {:?}", path);
            return;
        }
        let data = std::fs::read(path).expect("Failed to read MKV fixture");

        let chunker = ContentChunker::new(ChunkStrategy::MediaAware);
        let chunks = chunker
            .chunk(&data, "bbb-5s-h264.mkv")
            .await
            .expect("chunking must succeed");

        let mut sorted = chunks.clone();
        sorted.sort_unstable_by_key(|c| c.offset);
        let reconstructed: Vec<u8> = sorted.iter().flat_map(|c| c.data.iter().copied()).collect();
        assert_eq!(
            reconstructed, data,
            "chunk reconstruction must be byte-exact"
        );

        let has_h264 = chunks.iter().any(|c| c.codec_hint == CodecHint::H264);
        assert!(has_h264, "expected at least one chunk with H264 codec hint");
    }

    // ---- P3a structure-aware walkers: FBX / .blend / STL / PLY / audio tier ----

    /// Sort chunks by offset and assert: (1) concat(chunks) == original bytes,
    /// (2) coverage is contiguous with no gap/overlap (every chunk's offset
    /// equals the running total of preceding sizes), and (3) sum(sizes)
    /// equals the original length. This is the integrity contract every new
    /// walker below must satisfy.
    fn assert_concat_and_coverage(chunks: &[ContentChunk], original: &[u8]) {
        let mut sorted = chunks.to_vec();
        sorted.sort_unstable_by_key(|c| c.offset);

        let mut covered = 0u64;
        for c in &sorted {
            assert_eq!(
                c.offset, covered,
                "gap or overlap: expected next chunk at offset {covered}, found {}",
                c.offset
            );
            covered += c.size as u64;
        }
        assert_eq!(
            covered,
            original.len() as u64,
            "sum(sizes) must equal original length"
        );

        let reconstructed: Vec<u8> = sorted.iter().flat_map(|c| c.data.iter().copied()).collect();
        assert_eq!(
            reconstructed, original,
            "concat(chunks) must equal original file bytes"
        );
    }

    // ---- FBX ----

    /// Build a synthetic FBX binary: header + a run of top-level node
    /// records (EndOffset walked, payload content otherwise opaque) + a NULL
    /// record terminator + trailing footer bytes.
    fn make_fbx_binary(version: u32, node_payloads: &[Vec<u8>], footer: &[u8]) -> Vec<u8> {
        let mut buf = Vec::new();
        buf.extend_from_slice(b"Kaydara FBX Binary  \x00"); // 21 bytes
        buf.extend_from_slice(&[0u8, 0u8]); // 2 reserved bytes -> 23
        buf.extend_from_slice(&version.to_le_bytes()); // -> 27 (HEADER_LEN)

        let off_width = if version >= 7500 { 8 } else { 4 };
        for payload in node_payloads {
            let record_start = buf.len();
            let end_offset = (record_start + off_width + payload.len()) as u64;
            if off_width == 8 {
                buf.extend_from_slice(&end_offset.to_le_bytes());
            } else {
                buf.extend_from_slice(&(end_offset as u32).to_le_bytes());
            }
            buf.extend_from_slice(payload);
        }

        // NULL record: the walker only reads the EndOffset field, so
        // off_width zero bytes is sufficient to signal termination.
        buf.extend(std::iter::repeat_n(0u8, off_width));
        buf.extend_from_slice(footer);
        buf
    }

    #[tokio::test]
    async fn test_fbx_synthetic_roundtrip_and_coverage() {
        let node_payloads = vec![
            vec![0xAAu8; 1000],            // small - coalesced with neighbors
            vec![0xBBu8; 2000],            // small - coalesced with neighbors
            vec![0xCCu8; 5 * 1024 * 1024], // > 4MB - FastCDC sub-split
            vec![0xDDu8; 100],             // small trailing node
        ];
        let footer = b"trailing-footer-bytes";
        let data = make_fbx_binary(7400, &node_payloads, footer);

        let chunker = ContentChunker::new(ChunkStrategy::MediaAware);
        let chunks = chunker.chunk_fbx_walker(&data).await.unwrap();

        assert_concat_and_coverage(&chunks, &data);
        assert!(
            chunks
                .iter()
                .any(|c| c.chunk_type == ChunkType::Metadata && c.offset == 0),
            "expected the 27-byte header as a Metadata chunk at offset 0"
        );
        // The 5MB node must have been split into more than one chunk.
        assert!(
            chunks.len() > 3,
            "expected the large node to be FastCDC-subdivided, got {} chunks",
            chunks.len()
        );
    }

    #[tokio::test]
    async fn test_fbx_wide_offsets_version_7500() {
        // Version >= 7500 uses 8-byte EndOffset fields.
        let node_payloads = vec![vec![0x11u8; 300], vec![0x22u8; 300]];
        let data = make_fbx_binary(7500, &node_payloads, b"");

        let chunker = ContentChunker::new(ChunkStrategy::MediaAware);
        let chunks = chunker.chunk_fbx_walker(&data).await.unwrap();
        assert_concat_and_coverage(&chunks, &data);
    }

    #[tokio::test]
    async fn test_fbx_corrupt_node_offsets_falls_back_no_panic() {
        // Valid magic + version, but the first "EndOffset" is garbage
        // (points backward), which must fail the monotonic sanity check and
        // fall back to legacy header+CDC rather than panicking.
        let mut data = Vec::new();
        data.extend_from_slice(b"Kaydara FBX Binary  \x00");
        data.extend_from_slice(&[0u8, 0u8]);
        data.extend_from_slice(&7400u32.to_le_bytes());
        data.extend_from_slice(&1u32.to_le_bytes()); // EndOffset < current pos (27) -> invalid
        data.extend_from_slice(&[0xFFu8; 500]); // garbage payload

        let chunker = ContentChunker::new(ChunkStrategy::MediaAware);
        let chunks = chunker.chunk_fbx_walker(&data).await.unwrap();
        assert_concat_and_coverage(&chunks, &data);
    }

    #[tokio::test]
    async fn test_fbx_truncated_short_data_no_panic() {
        let data = vec![0u8; 10]; // shorter than the 27-byte header
        let chunker = ContentChunker::new(ChunkStrategy::MediaAware);
        let chunks = chunker.chunk_fbx_walker(&data).await.unwrap();
        assert_concat_and_coverage(&chunks, &data);
    }

    #[tokio::test]
    async fn test_fbx_legacy_reproduces_header_plus_cdc() {
        // Pins the MEDIAGIT_CHUNK_FBX=0 behavior directly (bypassing the
        // process-wide OnceLock kill-switch, same rationale as
        // apply_codec_hint_if's test above): header must stay a standalone
        // 27-byte Metadata chunk, exactly like the pre-P3a implementation.
        let data = make_fbx_binary(7400, &[vec![0x55u8; 4096]], b"");
        let chunker = ContentChunker::new(ChunkStrategy::MediaAware);
        let chunks = chunker.chunk_fbx_legacy(&data, 27).await.unwrap();

        assert_concat_and_coverage(&chunks, &data);
        assert_eq!(chunks[0].offset, 0);
        assert_eq!(chunks[0].size, 27);
        assert_eq!(chunks[0].chunk_type, ChunkType::Metadata);
    }

    #[tokio::test]
    #[ignore] // Requires test-files directory
    async fn test_fbx_real_fixture_roundtrip() {
        let path = std::path::Path::new("../../test-files/56-fbx/fbx/Dragon 2.5_fbx.fbx");
        if !path.exists() {
            eprintln!("Skipping test: fixture not found at {:?}", path);
            return;
        }
        let data = std::fs::read(path).expect("Failed to read FBX fixture");
        let chunker = ContentChunker::new(ChunkStrategy::MediaAware);
        let chunks = chunker
            .chunk(&data, "Dragon 2.5_fbx.fbx")
            .await
            .expect("chunking must succeed");
        assert_concat_and_coverage(&chunks, &data);
    }

    // ---- Blender .blend ----

    /// Build a synthetic uncompressed little-endian .blend: 12-byte header +
    /// a run of BHEAD blocks + trailing footer bytes.
    fn make_blend(ptr_size: u8, blocks: &[(&[u8; 4], Vec<u8>)], footer: &[u8]) -> Vec<u8> {
        let mut buf = Vec::new();
        buf.extend_from_slice(b"BLENDER");
        buf.push(if ptr_size == 8 { b'-' } else { b'_' });
        buf.push(b'v'); // little-endian
        buf.extend_from_slice(b"300"); // version digits, unused by the walker

        for (code, body) in blocks {
            buf.extend_from_slice(*code);
            buf.extend_from_slice(&(body.len() as u32).to_le_bytes());
            buf.extend(std::iter::repeat_n(0u8, ptr_size as usize)); // old ptr
            buf.extend_from_slice(&0u32.to_le_bytes()); // SDNAnr
            buf.extend_from_slice(&0u32.to_le_bytes()); // nr
            buf.extend_from_slice(body);
        }
        buf.extend_from_slice(footer);
        buf
    }

    #[tokio::test]
    async fn test_blend_synthetic_roundtrip_and_coverage() {
        let blocks: Vec<(&[u8; 4], Vec<u8>)> = vec![
            (b"DATA", vec![0xABu8; 1000]),
            // > 4MB and non-constant: constant bytes give the gear hash no
            // variation, so FastCDC would only cut at max_size (2 sub-chunks).
            (b"DATA", pseudo_random_bytes(6 * 1024 * 1024, 7)),
            (b"DNA1", vec![0xEFu8; 200]),
            (b"ENDB", vec![]),
        ];
        let data = make_blend(4, &blocks, b"");

        let chunker = ContentChunker::new(ChunkStrategy::MediaAware);
        let chunks = chunker.chunk_blend(&data).await.unwrap();

        assert_concat_and_coverage(&chunks, &data);
        assert!(
            chunks
                .iter()
                .any(|c| c.chunk_type == ChunkType::Metadata && c.offset == 0),
            "expected the 12-byte header as a Metadata chunk at offset 0"
        );
        assert!(
            chunks.len() > 3,
            "expected the large block to be FastCDC-subdivided, got {} chunks",
            chunks.len()
        );
    }

    #[tokio::test]
    async fn test_blend_8byte_pointers_roundtrip() {
        let blocks: Vec<(&[u8; 4], Vec<u8>)> =
            vec![(b"DATA", vec![0x77u8; 500]), (b"ENDB", vec![])];
        let data = make_blend(8, &blocks, b"");

        let chunker = ContentChunker::new(ChunkStrategy::MediaAware);
        let chunks = chunker.chunk_blend(&data).await.unwrap();
        assert_concat_and_coverage(&chunks, &data);
    }

    #[tokio::test]
    async fn test_blend_non_blender_magic_falls_back() {
        // e.g. gzip-compressed .blend (Blender's older default save format).
        let data = vec![0x1Fu8, 0x8B, 0x08, 0x00, 0x00, 0x00, 0x00, 0x00, 0xAA, 0xBB];
        let chunker = ContentChunker::new(ChunkStrategy::MediaAware);
        let chunks = chunker.chunk_blend(&data).await.unwrap();
        assert_concat_and_coverage(&chunks, &data);
    }

    #[tokio::test]
    async fn test_blend_bigendian_falls_back() {
        let mut data = Vec::new();
        data.extend_from_slice(b"BLENDER");
        data.push(b'_');
        data.push(b'V'); // big-endian marker
        data.extend_from_slice(b"300");
        data.extend_from_slice(&[0u8; 100]);

        let chunker = ContentChunker::new(ChunkStrategy::MediaAware);
        let chunks = chunker.chunk_blend(&data).await.unwrap();
        assert_concat_and_coverage(&chunks, &data);
    }

    #[tokio::test]
    async fn test_blend_truncated_falls_back_no_panic() {
        // Valid header, but the declared block length runs past EOF.
        let mut data = Vec::new();
        data.extend_from_slice(b"BLENDER");
        data.push(b'_');
        data.push(b'v');
        data.extend_from_slice(b"300");
        data.extend_from_slice(b"DATA");
        data.extend_from_slice(&(1_000_000u32).to_le_bytes()); // body way past EOF
        data.extend_from_slice(&[0u8; 12]); // old-ptr(4) + SDNAnr(4) + nr(4)
        data.extend_from_slice(&[0xEEu8; 20]); // truncated body

        let chunker = ContentChunker::new(ChunkStrategy::MediaAware);
        let chunks = chunker.chunk_blend(&data).await.unwrap();
        assert_concat_and_coverage(&chunks, &data);
    }

    #[tokio::test]
    #[ignore] // Requires test-files directory
    async fn test_blend_real_fixture_roundtrip() {
        let path = std::path::Path::new(
            "../../test-files/27-blender/blender/Dragon_2.5_For_Animations.blend",
        );
        if !path.exists() {
            eprintln!("Skipping test: fixture not found at {:?}", path);
            return;
        }
        let data = std::fs::read(path).expect("Failed to read blend fixture");
        let chunker = ContentChunker::new(ChunkStrategy::MediaAware);
        let chunks = chunker
            .chunk(&data, "Dragon_2.5_For_Animations.blend")
            .await
            .expect("chunking must succeed");
        // This fixture is gzip-compressed (older Blender default save format),
        // so it must exercise the non-BLENDER-magic CDC fallback path.
        assert_concat_and_coverage(&chunks, &data);
    }

    // ---- STL ----

    fn make_stl_binary(triangle_count: usize) -> Vec<u8> {
        let mut buf = vec![0u8; 80];
        buf.extend_from_slice(&(triangle_count as u32).to_le_bytes());
        // Pseudo-random (not a periodic ramp) so FastCDC has real content
        // signal to find cut points on, matching pseudo_random_bytes' doc note.
        buf.extend(pseudo_random_bytes(triangle_count * 50, 99));
        buf
    }

    #[tokio::test]
    async fn test_stl_binary_small_roundtrip_and_coverage() {
        let data = make_stl_binary(10);
        let chunker = ContentChunker::new(ChunkStrategy::MediaAware);
        let chunks = chunker.chunk_stl(&data).await.unwrap();
        assert_concat_and_coverage(&chunks, &data);
        assert_eq!(chunks.len(), 1, "small STL fits in a single chunk");
    }

    #[tokio::test]
    async fn test_stl_binary_large_is_content_defined_subdivided() {
        // ~50000 triangles * 50 bytes = ~2.4MB -> multiple ~1MB CDC chunks.
        let data = make_stl_binary(50_000);
        let chunker = ContentChunker::new(ChunkStrategy::MediaAware);
        let chunks = chunker.chunk_stl(&data).await.unwrap();

        assert_concat_and_coverage(&chunks, &data);
        assert!(
            chunks.len() > 1,
            "expected the triangle array to be CDC-subdivided into multiple chunks"
        );
        assert_eq!(
            chunks[0].offset, 0,
            "first chunk must start at offset 0 (header folded in)"
        );
    }

    #[tokio::test]
    async fn test_stl_edit_recovers_dedup_after_insertion() {
        // Regression pin for the fixed-position-cut bug this walker started
        // with: inserting bytes at ~25% offset must NOT invalidate every
        // chunk after the insertion point. Content-defined cuts re-sync, so
        // v1 and v2 must still share chunk IDs over the unedited remainder
        // (fixed-position cuts, as originally implemented, shared zero).
        // Operates on the triangle array directly (the same bytes chunk_stl
        // feeds to FastCDC) to isolate the re-sync property being pinned.
        let full_v1 = make_stl_binary(40_000); // ~2MB
        let triangle_v1 = &full_v1[84..];
        let insert_at = triangle_v1.len() / 4;
        let mut triangle_v2 = triangle_v1[..insert_at].to_vec();
        triangle_v2.extend(std::iter::repeat_n(0xEEu8, 4096));
        triangle_v2.extend_from_slice(&triangle_v1[insert_at..]);

        let chunker = ContentChunker::new(ChunkStrategy::MediaAware);
        let chunks_v1 = chunker
            .chunk_fastcdc(triangle_v1, 1024 * 1024, 512 * 1024, 4 * 1024 * 1024)
            .await
            .unwrap();
        let chunks_v2 = chunker
            .chunk_fastcdc(&triangle_v2, 1024 * 1024, 512 * 1024, 4 * 1024 * 1024)
            .await
            .unwrap();

        let ids_v1: std::collections::HashSet<_> = chunks_v1.iter().map(|c| c.id).collect();
        let shared = chunks_v2.iter().filter(|c| ids_v1.contains(&c.id)).count();
        assert!(
            shared > 0,
            "expected at least some chunk IDs to survive a mid-file insertion via CDC re-sync"
        );
    }

    #[tokio::test]
    async fn test_stl_ascii_falls_back_no_panic() {
        let data = b"solid test\nfacet normal 0 0 0\nendsolid test\n".to_vec();
        let chunker = ContentChunker::new(ChunkStrategy::MediaAware);
        let chunks = chunker.chunk_stl(&data).await.unwrap();
        assert_concat_and_coverage(&chunks, &data);
    }

    #[tokio::test]
    async fn test_stl_size_mismatch_falls_back_no_panic() {
        // Header claims 100 triangles but the file is truncated.
        let mut data = vec![0u8; 80];
        data.extend_from_slice(&100u32.to_le_bytes());
        data.extend_from_slice(&[0u8; 200]); // way short of 100*50 bytes

        let chunker = ContentChunker::new(ChunkStrategy::MediaAware);
        let chunks = chunker.chunk_stl(&data).await.unwrap();
        assert_concat_and_coverage(&chunks, &data);
    }

    #[tokio::test]
    #[ignore] // Requires test-files directory
    async fn test_stl_real_fixture_roundtrip() {
        let path = std::path::Path::new("../../test-files/39-stl/stl/Dragon 2.5_stl.stl");
        if !path.exists() {
            eprintln!("Skipping test: fixture not found at {:?}", path);
            return;
        }
        let data = std::fs::read(path).expect("Failed to read STL fixture");
        let chunker = ContentChunker::new(ChunkStrategy::MediaAware);
        let chunks = chunker
            .chunk(&data, "Dragon 2.5_stl.stl")
            .await
            .expect("chunking must succeed");
        assert_concat_and_coverage(&chunks, &data);
    }

    // ---- PLY ----

    /// vertex properties: float x, y, z (12-byte stride).
    fn make_ply_binary(vertex_count: usize, face_block: &[u8]) -> Vec<u8> {
        let header = format!(
            "ply\nformat binary_little_endian 1.0\nelement vertex {vertex_count}\nproperty float x\nproperty float y\nproperty float z\nelement face 0\nproperty list uchar int vertex_indices\nend_header\n"
        );
        let mut buf = header.into_bytes();
        for i in 0..vertex_count {
            let v = i as f32;
            buf.extend_from_slice(&v.to_le_bytes());
            buf.extend_from_slice(&v.to_le_bytes());
            buf.extend_from_slice(&v.to_le_bytes());
        }
        buf.extend_from_slice(face_block);
        buf
    }

    #[tokio::test]
    async fn test_ply_binary_small_roundtrip_and_coverage() {
        let data = make_ply_binary(100, &[0x01, 0x02, 0x03, 0x04]);
        let chunker = ContentChunker::new(ChunkStrategy::MediaAware);
        let chunks = chunker.chunk_ply(&data).await.unwrap();
        assert_concat_and_coverage(&chunks, &data);
    }

    #[tokio::test]
    async fn test_ply_large_vertex_block_is_content_defined_subdivided() {
        // 100_000 vertices * 12 bytes = ~1.14MB -> multiple CDC chunks.
        let data = make_ply_binary(100_000, &[]);
        let chunker = ContentChunker::new(ChunkStrategy::MediaAware);
        let chunks = chunker.chunk_ply(&data).await.unwrap();

        assert_concat_and_coverage(&chunks, &data);
        let generic_count = chunks
            .iter()
            .filter(|c| c.chunk_type == ChunkType::Generic)
            .count();
        assert!(
            generic_count > 1,
            "expected the vertex block to be CDC-subdivided into multiple chunks"
        );
    }

    #[tokio::test]
    async fn test_ply_large_face_block_is_subdivided() {
        let face_block = vec![0x99u8; 5 * 1024 * 1024]; // > 4MB
        let data = make_ply_binary(10, &face_block);
        let chunker = ContentChunker::new(ChunkStrategy::MediaAware);
        let chunks = chunker.chunk_ply(&data).await.unwrap();
        assert_concat_and_coverage(&chunks, &data);
        assert!(
            chunks.len() > 3,
            "expected the >4MB face block to be FastCDC-subdivided"
        );
    }

    #[tokio::test]
    async fn test_ply_ascii_falls_back_no_panic() {
        let data = b"ply\nformat ascii 1.0\nelement vertex 1\nproperty float x\nend_header\n1.0\n"
            .to_vec();
        let chunker = ContentChunker::new(ChunkStrategy::MediaAware);
        let chunks = chunker.chunk_ply(&data).await.unwrap();
        assert_concat_and_coverage(&chunks, &data);
    }

    #[tokio::test]
    async fn test_ply_list_property_in_vertex_falls_back_no_panic() {
        // A `list` property inside the vertex element makes the stride
        // variable - must fall back rather than mis-align cuts.
        let data = b"ply\nformat binary_little_endian 1.0\nelement vertex 1\nproperty list uchar int foo\nend_header\n\x01\x00\x00\x00\x00".to_vec();
        let chunker = ContentChunker::new(ChunkStrategy::MediaAware);
        let chunks = chunker.chunk_ply(&data).await.unwrap();
        assert_concat_and_coverage(&chunks, &data);
    }

    #[tokio::test]
    async fn test_ply_truncated_vertex_block_falls_back_no_panic() {
        // Header claims far more vertices than the file actually has.
        let header = "ply\nformat binary_little_endian 1.0\nelement vertex 1000000\nproperty float x\nproperty float y\nproperty float z\nend_header\n";
        let mut data = header.as_bytes().to_vec();
        data.extend_from_slice(&[0u8; 12]); // just one vertex's worth of bytes

        let chunker = ContentChunker::new(ChunkStrategy::MediaAware);
        let chunks = chunker.chunk_ply(&data).await.unwrap();
        assert_concat_and_coverage(&chunks, &data);
    }

    #[tokio::test]
    #[ignore] // Requires test-files directory
    async fn test_ply_real_fixture_roundtrip() {
        let path = std::path::Path::new("../../test-files/93-ply/ply/Dragon 2.5_ply.ply");
        if !path.exists() {
            eprintln!("Skipping test: fixture not found at {:?}", path);
            return;
        }
        let data = std::fs::read(path).expect("Failed to read PLY fixture");
        let chunker = ContentChunker::new(ChunkStrategy::MediaAware);
        let chunks = chunker
            .chunk(&data, "Dragon 2.5_ply.ply")
            .await
            .expect("chunking must succeed");
        // This fixture is ASCII PLY, so it must exercise the chunk_3d_text fallback.
        assert_concat_and_coverage(&chunks, &data);
    }

    // ---- Audio tier ----

    #[test]
    fn test_audio_chunk_params_values() {
        for &size in &[0u64, 1024, 10 * 1024 * 1024, 10 * 1024 * 1024 * 1024] {
            assert_eq!(
                get_audio_chunk_params(size),
                (256 * 1024, 64 * 1024, 1024 * 1024)
            );
        }
    }

    #[tokio::test]
    async fn test_wav_does_not_use_audio_tier_by_default() {
        // Deviation pin: WAV/FLAC/AIFF stay on the pre-P3a generic tier (see
        // the deviation comment on the "flac" arm in chunker.rs) — a measured
        // add_ms regression on the dedup_report corpus, not an oversight.
        let data = pseudo_random_bytes(2 * 1024 * 1024, 1);
        let chunker = ContentChunker::new(ChunkStrategy::MediaAware);
        let chunks = chunker.chunk(&data, "test.wav").await.unwrap();

        assert_concat_and_coverage(&chunks, &data);
        assert!(chunks.iter().all(|c| c.codec_hint == CodecHint::PCM));
        let avg = data.len() as f64 / chunks.len() as f64;
        assert!(
            avg > 600_000.0,
            "expected the generic tier (~1MB avg), got avg chunk size {avg} \
             (looks like the audio tier is being applied to wav again)"
        );
    }

    #[tokio::test]
    async fn test_mp3_routes_through_audio_tier_cdc_by_default() {
        let data = pseudo_random_bytes(2 * 1024 * 1024, 2);
        let chunker = ContentChunker::new(ChunkStrategy::MediaAware);
        let chunks = chunker.chunk(&data, "test.mp3").await.unwrap();

        assert_concat_and_coverage(&chunks, &data);
        assert!(chunks.iter().all(|c| c.codec_hint == CodecHint::MP3));
        assert!(
            chunks.len() > 1,
            "audio-tier CDC should split a 2MB mp3 into multiple chunks \
             (pre-P3a fixed-chunking produced exactly 1)"
        );
    }

    #[tokio::test]
    async fn test_ogg_routes_through_audio_tier_cdc_by_default() {
        let data = pseudo_random_bytes(2 * 1024 * 1024, 3);
        let chunker = ContentChunker::new(ChunkStrategy::MediaAware);
        let chunks = chunker.chunk(&data, "test.ogg").await.unwrap();

        assert_concat_and_coverage(&chunks, &data);
        assert!(chunks.iter().all(|c| c.codec_hint == CodecHint::Vorbis));
        assert!(
            chunks.len() > 1,
            "audio-tier CDC should split a 2MB ogg into multiple chunks \
             (pre-P3a fixed-chunking produced exactly 1)"
        );
    }
}
