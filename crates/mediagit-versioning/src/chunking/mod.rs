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

/// Content-based chunker
pub struct ContentChunker {
    pub(super) strategy: ChunkStrategy,
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
    use super::formats::{parse_ebml_elements, parse_mp4_atoms, read_ebml_id, read_ebml_size};
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
}
