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
struct Mp4Atom {
    /// 4-byte FourCC type (e.g., b"ftyp", b"moov", b"mdat")
    atom_type: [u8; 4],
    /// Offset in data where atom starts
    offset: u64,
    /// Total size including header (8 or 16 bytes header + data)
    size: u64,
    /// Header size (8 for standard, 16 for extended)
    header_size: u8,
}

/// EBML Element header parsed from Matroska/WebM data
/// Used internally for MKV/WebM/MKA element-based chunking
#[derive(Debug, Clone)]
struct EbmlElement {
    /// Element ID (includes VINT marker, up to 4 bytes for Matroska)
    id: u32,
    /// Offset in data where element starts
    offset: u64,
    /// Size of ID + Size fields
    header_size: u8,
    /// Content size (u64::MAX = unknown size)
    data_size: u64,
}

// Matroska Element IDs (with VINT marker included)
const EBML_ID: u32 = 0x1A45DFA3; // EBML Header
const SEGMENT_ID: u32 = 0x18538067; // Segment container
const SEEKHEAD_ID: u32 = 0x114D9B74; // SeekHead (index)
const INFO_ID: u32 = 0x1549A966; // Segment Info
const TRACKS_ID: u32 = 0x1654AE6B; // Track definitions
const CLUSTER_ID: u32 = 0x1F43B675; // Cluster (media data)
const CUES_ID: u32 = 0x1C53BB6B; // Cues (seek index)
const CHAPTERS_ID: u32 = 0x1043A770; // Chapters
const TAGS_ID: u32 = 0x1254C367; // Tags (metadata)
const ATTACHMENTS_ID: u32 = 0x1941A469; // Attachments
const VOID_ID: u32 = 0xEC; // Void (padding, skip)
const CRC32_ID: u32 = 0xBF; // CRC-32 (skip)
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

/// Content-based chunker
pub struct ContentChunker {
    strategy: ChunkStrategy,
}

impl ContentChunker {
    /// Create a new content chunker with the specified strategy
    pub fn new(strategy: ChunkStrategy) -> Self {
        Self { strategy }
    }

    /// Chunk data according to the configured strategy
    pub async fn chunk(&self, data: &[u8], filename: &str) -> Result<Vec<ContentChunk>> {
        match self.strategy {
            ChunkStrategy::Fixed { size } => self.chunk_fixed(data, size).await,
            ChunkStrategy::Rolling {
                avg_size,
                min_size,
                max_size,
            } => self.chunk_fastcdc(data, avg_size, min_size, max_size).await,
            ChunkStrategy::MediaAware => self.chunk_media_aware(data, filename).await,
        }
    }

    /// Chunk a file with streaming callback pattern (constant memory)
    ///
    /// This method processes large files without loading them entirely into memory.
    /// Each chunk is passed to the callback immediately and can be processed/stored,
    /// then dropped to free memory.
    ///
    /// # Arguments
    ///
    /// * `path` - Path to the file to chunk
    /// * `on_chunk` - Async callback called for each chunk. The chunk should be
    ///   processed (e.g., written to storage) within the callback.
    ///
    /// # Returns
    ///
    /// Vector of chunk OIDs (only the identifiers, not the chunk data)
    ///
    /// # Memory Usage
    ///
    /// Memory is bounded by the chunk size (~8MB max for TB+ files) regardless
    /// of total file size.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # use mediagit_versioning::chunking::{ContentChunker, ChunkStrategy};
    /// # async fn example() -> anyhow::Result<()> {
    /// let chunker = ContentChunker::new(ChunkStrategy::MediaAware);
    /// let chunk_oids = chunker.chunk_file_streaming(
    ///     "large_video.mp4",
    ///     |chunk| async move {
    ///         // Process chunk (e.g., write to storage)
    ///         println!("Got chunk: {} bytes", chunk.size);
    ///         Ok(())
    ///     }
    /// ).await?;
    /// println!("Created {} chunks", chunk_oids.len());
    /// # Ok(())
    /// # }
    /// ```
    pub async fn chunk_file_streaming<P, F, Fut>(
        &self,
        path: P,
        mut on_chunk: F,
    ) -> Result<Vec<Oid>>
    where
        P: AsRef<std::path::Path>,
        F: FnMut(ContentChunk) -> Fut,
        Fut: std::future::Future<Output = Result<()>>,
    {
        let path = path.as_ref();
        let file_size = std::fs::metadata(path)
            .map_err(|e| anyhow::anyhow!("Failed to get file metadata: {}", e))?
            .len();

        let mut chunk_oids = Vec::new();

        // Tier 1: Small files (< 10MB): Load into memory for fastest processing
        if file_size < 10 * 1024 * 1024 {
            let data = tokio::fs::read(path)
                .await
                .map_err(|e| anyhow::anyhow!("Failed to read file: {}", e))?;
            let filename = path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("unknown");

            let chunks = self.chunk(&data, filename).await?;
            for chunk in chunks {
                let oid = chunk.id;
                on_chunk(chunk).await?; // Process immediately
                chunk_oids.push(oid); // Only store the OID
            }
            return Ok(chunk_oids);
        }

        // Tier 2: Medium files (10-100MB): Load into memory but process with streaming callback
        // Still loads file, but processes chunks immediately to reduce peak memory
        if file_size < 100 * 1024 * 1024 {
            debug!(
                size = file_size,
                "Medium file: loading then streaming chunks"
            );
            let data = tokio::fs::read(path)
                .await
                .map_err(|e| anyhow::anyhow!("Failed to read file: {}", e))?;
            let filename = path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("unknown");

            // Use MediaAware chunking for better boundaries
            let chunks = self.chunk(&data, filename).await?;
            for chunk in chunks {
                let oid = chunk.id;
                on_chunk(chunk).await?; // Process immediately (allows GC of chunk data)
                chunk_oids.push(oid);
            }
            return Ok(chunk_oids);
        }

        // Large files: Stream with FastCDC content-defined chunking
        // FastCDC's StreamCDC uses gear table for O(1) boundary detection - fast enough for streaming
        let (avg_size, min_size, max_size) = get_chunk_params(file_size);
        info!(
            "FastCDC streaming: file_size={}, avg_chunk={}KB",
            file_size,
            avg_size / 1024
        );

        // Read file into memory for FastCDC (required by current API)
        // Note: FastCDC StreamCDC requires Read trait, but for truly streaming
        // we need to buffer chunks. Memory usage is bounded by max_chunk_size.
        let file =
            std::fs::File::open(path).map_err(|e| anyhow::anyhow!("Failed to open file: {}", e))?;

        let chunker =
            fastcdc::v2020::StreamCDC::new(file, min_size as u32, avg_size as u32, max_size as u32);

        for result in chunker {
            let entry = result.map_err(|e| anyhow::anyhow!("FastCDC stream error: {}", e))?;
            let id = Oid::hash(&entry.data);

            let chunk = ContentChunk {
                id,
                data: entry.data,
                offset: entry.offset,
                size: entry.length,
                chunk_type: ChunkType::Generic,
                perceptual_hash: None,
                codec_hint: CodecHint::Unknown,
            };

            on_chunk(chunk).await?;
            chunk_oids.push(id);
        }

        info!(
            "FastCDC streaming complete: {} chunks created",
            chunk_oids.len()
        );
        Ok(chunk_oids)
    }

    /// Collect chunks from a file synchronously, sending each through a `tokio::sync::mpsc` channel.
    ///
    /// Designed for use inside `tokio::task::spawn_blocking` to avoid blocking the tokio
    /// executor with synchronous FastCDC file I/O.  The caller should spawn a blocking task,
    /// then receive from the corresponding `tokio::sync::mpsc::Receiver` in async context.
    ///
    /// Uses `memmap2` to memory-map the file so the existing format-aware parsers
    /// (`chunk_mp4`, `chunk_matroska`, `chunk_avi`) can operate on files of any size,
    /// not just those below the 100 MB in-memory threshold.  Falls back to `StreamCDC`
    /// when mmap is unavailable (network filesystems, permission issues, 32-bit targets
    /// with very large files) or when format parsing returns an error.
    ///
    /// # Errors
    /// Returns an error if the file cannot be opened, read, or if the receiver has been dropped.
    pub fn collect_file_chunks_blocking<P: AsRef<std::path::Path>>(
        &self,
        path: P,
        sender: tokio::sync::mpsc::Sender<ContentChunk>,
    ) -> anyhow::Result<()> {
        let path = path.as_ref();
        let file_size = std::fs::metadata(path)
            .map_err(|e| {
                anyhow::anyhow!("Failed to read file metadata '{}': {}", path.display(), e)
            })?
            .len();

        if file_size == 0 {
            return Ok(());
        }

        let filename = path.file_name().and_then(|n| n.to_str()).unwrap_or("");

        // Only use mmap + format-aware chunking for extensions that have a dedicated
        // structure-aware parser (container formats where splitting at structural
        // boundaries genuinely improves deduplication).  All other files go straight
        // to StreamCDC so that chunk boundaries stay content-defined and
        // delta-compressible — matching the behaviour of v0.2.5-beta.1.
        let has_structure_parser = {
            let ext = std::path::Path::new(filename)
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("")
                .to_lowercase();
            matches!(
                ext.as_str(),
                // Video containers with dedicated parsers
                "avi" | "riff" | "mp4" | "mov" | "m4v" | "m4a" | "3gp"
                    | "mkv" | "webm" | "mka" | "mk3d"
                    // 3D model containers with dedicated parsers
                    | "glb" | "gltf" | "obj" | "stl" | "ply" | "fbx"
            )
        };

        if has_structure_parser {
            let file = std::fs::File::open(path)
                .map_err(|e| anyhow::anyhow!("Failed to open file '{}': {}", path.display(), e))?;

            // Attempt memory-mapped I/O so the format-aware chunkers run on large files.
            // Safety: the file is opened read-only and the mapping is read-only.
            // On 32-bit targets, skip mmap for files whose size would overflow usize.
            #[cfg(target_pointer_width = "32")]
            let mmap_attempt: Option<memmap2::Mmap> = if file_size <= usize::MAX as u64 {
                unsafe { memmap2::Mmap::map(&file).ok() }
            } else {
                None
            };
            #[cfg(not(target_pointer_width = "32"))]
            let mmap_attempt: Option<memmap2::Mmap> = unsafe { memmap2::Mmap::map(&file).ok() };

            if let Some(mmap) = mmap_attempt {
                // Spin up a minimal current-thread tokio runtime.  We cannot reuse the
                // parent runtime because we're inside spawn_blocking.
                match tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                {
                    Ok(rt) => match rt.block_on(self.chunk(&mmap[..], filename)) {
                        Ok(chunks) => {
                            debug!(
                                path = %path.display(),
                                chunks = chunks.len(),
                                "mmap format-aware chunking complete"
                            );
                            for chunk in chunks {
                                sender.blocking_send(chunk).map_err(|_| {
                                    anyhow::anyhow!("Chunk worker channel closed unexpectedly")
                                })?;
                            }
                            return Ok(());
                        }
                        Err(e) => {
                            warn!(
                                path = %path.display(),
                                error = %e,
                                "Format-aware chunking failed, falling back to StreamCDC"
                            );
                        }
                    },
                    Err(e) => {
                        warn!(
                            "Failed to build tokio runtime for mmap chunking ({}), \
                             falling back to StreamCDC",
                            e
                        );
                    }
                }
            } else {
                debug!(path = %path.display(), "mmap unavailable for container format, using StreamCDC");
            }
        }

        // StreamCDC: content-defined chunking (constant memory, all formats).
        // This is the primary path for non-container files and the fallback for
        // container files when mmap or format-aware parsing fails.
        let (avg_size, min_size, max_size) = get_chunk_params(file_size);
        let file = std::fs::File::open(path)
            .map_err(|e| anyhow::anyhow!("Failed to open file '{}': {}", path.display(), e))?;
        let stream_cdc =
            fastcdc::v2020::StreamCDC::new(file, min_size as u32, avg_size as u32, max_size as u32);

        for result in stream_cdc {
            let entry = result.map_err(|e| anyhow::anyhow!("FastCDC streaming error: {}", e))?;
            let id = Oid::hash(&entry.data);
            let chunk = ContentChunk {
                id,
                data: entry.data,
                offset: entry.offset,
                size: entry.length,
                chunk_type: ChunkType::Generic,
                perceptual_hash: None,
                codec_hint: CodecHint::Unknown,
            };
            sender
                .blocking_send(chunk)
                .map_err(|_| anyhow::anyhow!("Chunk worker channel closed unexpectedly"))?;
        }

        Ok(())
    }

    /// Fixed-size chunking
    async fn chunk_fixed(&self, data: &[u8], chunk_size: usize) -> Result<Vec<ContentChunk>> {
        let mut chunks = Vec::new();
        let mut offset = 0u64;

        for chunk_data in data.chunks(chunk_size) {
            let id = Oid::hash(chunk_data);

            chunks.push(ContentChunk {
                id,
                data: chunk_data.to_vec(),
                offset,
                size: chunk_data.len(),
                chunk_type: ChunkType::Generic,
                perceptual_hash: None,
                codec_hint: CodecHint::Unknown,
            });

            offset += chunk_data.len() as u64;
        }

        debug!(
            chunks = chunks.len(),
            chunk_size = chunk_size,
            total_size = data.len(),
            "Fixed-size chunking complete"
        );

        Ok(chunks)
    }

    /// Content-defined chunking using FastCDC algorithm
    ///
    /// Uses gear table-based hashing for O(1) boundary detection per byte,
    /// approximately 10x faster than traditional rolling hash implementations.
    async fn chunk_fastcdc(
        &self,
        data: &[u8],
        avg_size: usize,
        min_size: usize,
        max_size: usize,
    ) -> Result<Vec<ContentChunk>> {
        use fastcdc::v2020::FastCDC;

        let chunker = FastCDC::new(data, min_size as u32, avg_size as u32, max_size as u32);
        let mut chunks = Vec::new();

        for entry in chunker {
            let chunk_data = &data[entry.offset..entry.offset + entry.length];
            let id = Oid::hash(chunk_data);

            chunks.push(ContentChunk {
                id,
                data: chunk_data.to_vec(),
                offset: entry.offset as u64,
                size: entry.length,
                chunk_type: ChunkType::Generic,
                perceptual_hash: None,
                codec_hint: CodecHint::Unknown,
            });
        }

        debug!(
            chunks = chunks.len(),
            avg_size = avg_size,
            total_size = data.len(),
            "FastCDC chunking complete"
        );

        Ok(chunks)
    }

    /// Media-aware chunking (parse file structure)
    async fn chunk_media_aware(&self, data: &[u8], filename: &str) -> Result<Vec<ContentChunk>> {
        // Detect file type from extension
        let extension = std::path::Path::new(filename)
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("");

        match extension.to_lowercase().as_str() {
            // Media - Structure-aware chunking
            "avi" | "riff" => self.chunk_avi(data).await,
            // WAV is RIFF-based audio but not interleaved A/V — use rolling CDC
            "wav" => {
                let (avg, min, max) = get_chunk_params(data.len() as u64);
                self.chunk_fastcdc(data, avg, min, max).await
            }
            "mp4" | "mov" | "m4v" | "m4a" | "3gp" => self.chunk_mp4(data).await,
            "mkv" | "webm" | "mka" | "mk3d" => self.chunk_matroska(data).await,
            "mpg" | "mpeg" | "vob" | "mts" | "m2ts" => {
                // MPEG Program/Transport Streams - use rolling CDC
                let (avg, min, max) = get_chunk_params(data.len() as u64);
                self.chunk_fastcdc(data, avg, min, max).await
            }

            // 3D Models - Structure-aware chunking
            "glb" | "gltf" => self.chunk_glb(data).await,
            "obj" | "stl" | "ply" => self.chunk_3d_text(data).await,
            "fbx" => self.chunk_fbx(data).await,
            "usd" | "usda" | "usdc" | "usdz" => {
                // USD ecosystem - rolling CDC for scene graph dedup
                let (avg, min, max) = get_chunk_params(data.len() as u64);
                self.chunk_fastcdc(data, avg, min, max).await
            }
            "abc" => {
                // Alembic cache - rolling CDC for animation dedup
                let (avg, min, max) = get_chunk_params(data.len() as u64);
                self.chunk_fastcdc(data, avg, min, max).await
            }
            "blend" | "max" | "ma" | "mb" | "c4d" | "hip" | "zpr" | "ztl" => {
                // Application-specific 3D formats - rolling CDC
                let (avg, min, max) = get_chunk_params(data.len() as u64);
                self.chunk_fastcdc(data, avg, min, max).await
            }

            // Text/Code - Rolling CDC for incremental dedup (Brotli compression)
            "csv" | "tsv" | "json" | "xml" | "html" | "txt" | "md" | "rst" | "rs" | "py" | "js"
            | "ts" | "go" | "java" | "c" | "cpp" | "h" | "yaml" | "yml" | "toml" | "ini"
            | "cfg" | "sql" | "graphql" | "proto" => {
                let (avg, min, max) = get_chunk_params(data.len() as u64);
                self.chunk_fastcdc(data, avg, min, max).await
            }

            // ML Data formats - Rolling CDC for dataset versioning
            "parquet" | "arrow" | "feather" | "orc" | "avro" | "hdf5" | "h5" | "nc" | "netcdf"
            | "npy" | "npz" | "tfrecords" | "petastorm" => {
                let (avg, min, max) = get_chunk_params(data.len() as u64);
                self.chunk_fastcdc(data, avg, min, max).await
            }

            // ML Models - Rolling CDC for fine-tuning dedup
            "pt" | "pth" | "ckpt" | "pb" | "safetensors" | "bin" | "pkl" | "joblib" => {
                let (avg, min, max) = get_chunk_params(data.len() as u64);
                self.chunk_fastcdc(data, avg, min, max).await
            }

            // ML Deployment - Rolling CDC for model versioning
            "onnx" | "gguf" | "ggml" | "tflite" | "mlmodel" | "coreml" | "keras" | "pte"
            | "mleap" | "pmml" | "llamafile" => {
                let (avg, min, max) = get_chunk_params(data.len() as u64);
                self.chunk_fastcdc(data, avg, min, max).await
            }

            // Documents - Rolling CDC
            "pdf" | "svg" | "eps" | "ai" => {
                let (avg, min, max) = get_chunk_params(data.len() as u64);
                self.chunk_fastcdc(data, avg, min, max).await
            }

            // Design tools - Rolling CDC for incremental design changes
            "fig" | "sketch" | "xd" | "indd" | "indt" => {
                let (avg, min, max) = get_chunk_params(data.len() as u64);
                self.chunk_fastcdc(data, avg, min, max).await
            }

            // Lossless audio - Rolling CDC
            "flac" | "aiff" | "alac" => {
                let (avg, min, max) = get_chunk_params(data.len() as u64);
                self.chunk_fastcdc(data, avg, min, max).await
            }

            // Compressed formats - Fixed chunking (already compressed, replaced entirely)
            "jpg" | "jpeg" | "png" | "gif" | "webp" | "avif" | "heic" | "mp3" | "aac" | "ogg"
            | "opus" | "zip" | "7z" | "rar" | "gz" | "xz" | "bz2" => {
                self.chunk_fixed(data, 4 * 1024 * 1024).await
            }

            // Unknown - Rolling CDC as safe default for dedup
            _ => {
                debug!(
                    extension = extension,
                    "Unknown type, using rolling (CDC) chunking for dedup"
                );
                let (avg, min, max) = get_chunk_params(data.len() as u64);
                self.chunk_fastcdc(data, avg, min, max).await
            }
        }
    }

    /// AVI/RIFF format chunking.
    ///
    /// Walks the file at the RIFF-block level, handling both standard AVI 1.0
    /// (`RIFF/AVI `) and OpenDML AVI 2.0 (`RIFF/AVIX`) extension blocks.
    /// Inside each block, descends into `LIST/movi` and emits every video frame
    /// (`NNdc`/`NNdb`) and audio sample (`NNwb`) as its own `ContentChunk`.
    ///
    /// This allows deduplication of the video stream between files that share the
    /// same video encode but differ only in audio (e.g. stereo vs surround remux).
    async fn chunk_avi(&self, data: &[u8]) -> Result<Vec<ContentChunk>> {
        let mut chunks = Vec::new();
        let mut offset = 0u64;

        if data.len() < 12 || &data[0..4] != b"RIFF" {
            debug!("Not a valid RIFF file, using fixed chunking");
            return self.chunk_fixed(data, 4 * 1024 * 1024).await;
        }

        // Walk at the top-level RIFF-block level.
        // AVI 1.0: one RIFF/AVI  block contains everything.
        // AVI 2.0: RIFF/AVI  (headers + first movi) followed by one or more
        //          RIFF/AVIX blocks each containing a LIST/movi continuation.
        let mut file_pos = 0;
        while file_pos + 12 <= data.len() {
            let fourcc = &data[file_pos..file_pos + 4];
            let block_size = u32::from_le_bytes([
                data[file_pos + 4],
                data[file_pos + 5],
                data[file_pos + 6],
                data[file_pos + 7],
            ]) as usize;
            let form_type = &data[file_pos + 8..file_pos + 12];
            let block_end = file_pos
                .saturating_add(8)
                .saturating_add(block_size)
                .min(data.len());

            if fourcc == b"RIFF" && (form_type == b"AVI " || form_type == b"AVIX") {
                // Emit the 12-byte RIFF block header as Metadata.
                let blk_hdr = &data[file_pos..file_pos + 12];
                chunks.push(ContentChunk {
                    id: Oid::hash(blk_hdr),
                    data: blk_hdr.to_vec(),
                    offset,
                    size: blk_hdr.len(),
                    chunk_type: ChunkType::Metadata,
                    perceptual_hash: None,
                    codec_hint: CodecHint::Unknown,
                });
                offset += blk_hdr.len() as u64;

                // Process inner chunks (hdrl, LIST/movi, idx1, …)
                self.parse_avi_block_chunks(
                    data,
                    file_pos + 12,
                    block_end,
                    &mut offset,
                    &mut chunks,
                )
                .await?;
            } else {
                // Unknown top-level block → store as-is
                let blk_data = &data[file_pos..block_end];
                chunks.push(ContentChunk {
                    id: Oid::hash(blk_data),
                    data: blk_data.to_vec(),
                    offset,
                    size: blk_data.len(),
                    chunk_type: ChunkType::Generic,
                    perceptual_hash: None,
                    codec_hint: CodecHint::Unknown,
                });
                offset += blk_data.len() as u64;
            }

            file_pos = block_end;
        }

        // Trailing bytes at end of file
        if file_pos < data.len() {
            let rem = &data[file_pos..];
            chunks.push(ContentChunk {
                id: Oid::hash(rem),
                data: rem.to_vec(),
                offset,
                size: rem.len(),
                chunk_type: ChunkType::Generic,
                perceptual_hash: None,
                codec_hint: CodecHint::Unknown,
            });
        }

        fill_coverage_gaps(data, &mut chunks);

        info!(
            chunks = chunks.len(),
            video_chunks = chunks
                .iter()
                .filter(|c| c.chunk_type == ChunkType::VideoStream)
                .count(),
            audio_chunks = chunks
                .iter()
                .filter(|c| c.chunk_type == ChunkType::AudioStream)
                .count(),
            "AVI chunking complete"
        );

        Ok(chunks)
    }

    /// Walk the inner chunks of a `RIFF/AVI ` or `RIFF/AVIX` block.
    /// Handles `LIST/movi` descent and emits structural chunks as Metadata.
    async fn parse_avi_block_chunks(
        &self,
        data: &[u8],
        start: usize,
        end: usize,
        offset: &mut u64,
        chunks: &mut Vec<ContentChunk>,
    ) -> Result<()> {
        let mut pos = start;
        while pos + 8 <= end {
            let fourcc = &data[pos..pos + 4];
            let chunk_size =
                u32::from_le_bytes([data[pos + 4], data[pos + 5], data[pos + 6], data[pos + 7]])
                    as usize;

            let data_end = pos.saturating_add(8).saturating_add(chunk_size).min(end);
            let needs_padding = !chunk_size.is_multiple_of(2) && data_end < end;
            let chunk_end = if needs_padding {
                data_end + 1
            } else {
                data_end
            }
            .min(end);

            if fourcc == b"LIST" && pos + 12 <= end {
                let list_type = &data[pos + 8..pos + 12];
                if list_type == b"movi" {
                    // Emit the 12-byte LIST/movi header as Metadata.
                    let list_hdr = &data[pos..pos + 12];
                    chunks.push(ContentChunk {
                        id: Oid::hash(list_hdr),
                        data: list_hdr.to_vec(),
                        offset: *offset,
                        size: list_hdr.len(),
                        chunk_type: ChunkType::Metadata,
                        perceptual_hash: None,
                        codec_hint: CodecHint::Unknown,
                    });
                    *offset += list_hdr.len() as u64;

                    // CDC sub-chunking for byte-exact reconstruction.
                    // Per-stream batching (parse_avi_movi_subchunks) is intentionally
                    // NOT used: it accumulates non-contiguous bytes causing size
                    // mismatch during chunk reconstruction.
                    let movi_content = &data[pos + 12..chunk_end];
                    let movi_offset = *offset;
                    if movi_content.len() > 4 * 1024 * 1024 {
                        let sub_chunks = self
                            .chunk_fastcdc(
                                movi_content,
                                2 * 1024 * 1024,
                                1024 * 1024,
                                8 * 1024 * 1024,
                            )
                            .await?;
                        for mut sub in sub_chunks {
                            sub.offset += movi_offset;
                            sub.chunk_type = ChunkType::VideoStream;
                            chunks.push(sub);
                        }
                    } else if !movi_content.is_empty() {
                        chunks.push(ContentChunk {
                            id: Oid::hash(movi_content),
                            data: movi_content.to_vec(),
                            offset: movi_offset,
                            size: movi_content.len(),
                            chunk_type: ChunkType::VideoStream,
                            perceptual_hash: None,
                            codec_hint: CodecHint::Unknown,
                        });
                    }
                    *offset += movi_content.len() as u64;
                } else {
                    // hdrl, INFO, strl, etc. → single Metadata chunk
                    let chunk_data = &data[pos..chunk_end];
                    chunks.push(ContentChunk {
                        id: Oid::hash(chunk_data),
                        data: chunk_data.to_vec(),
                        offset: *offset,
                        size: chunk_data.len(),
                        chunk_type: ChunkType::Metadata,
                        perceptual_hash: None,
                        codec_hint: CodecHint::Unknown,
                    });
                    *offset += chunk_data.len() as u64;
                }
            } else {
                let chunk_data = &data[pos..chunk_end];
                let chunk_type = match fourcc {
                    b"idx1" | b"JUNK" | b"IDIT" | b"indx" => ChunkType::Metadata,
                    _ => ChunkType::Generic,
                };
                chunks.push(ContentChunk {
                    id: Oid::hash(chunk_data),
                    data: chunk_data.to_vec(),
                    offset: *offset,
                    size: chunk_data.len(),
                    chunk_type,
                    perceptual_hash: None,
                    codec_hint: CodecHint::Unknown,
                });
                *offset += chunk_data.len() as u64;
            }

            pos = chunk_end;
        }

        // Trailing bytes inside this RIFF block
        if pos < end {
            let rem = &data[pos..end];
            chunks.push(ContentChunk {
                id: Oid::hash(rem),
                data: rem.to_vec(),
                offset: *offset,
                size: rem.len(),
                chunk_type: ChunkType::Generic,
                perceptual_hash: None,
                codec_hint: CodecHint::Unknown,
            });
            *offset += rem.len() as u64;
        }

        Ok(())
    }

    /// MP4/ISO BMFF chunking based on atom structure
    ///
    /// Parses MP4 atoms (ftyp, moov, mdat, etc.) and creates chunks at atom boundaries.
    /// For large mdat atoms, uses CDC sub-chunking. For moov, parses nested atoms.
    async fn chunk_mp4(&self, data: &[u8]) -> Result<Vec<ContentChunk>> {
        // Validate MP4 signature (ftyp should be first atom)
        if data.len() < 8 || &data[4..8] != b"ftyp" {
            debug!("Not a valid MP4 file (no ftyp), using fixed chunking");
            return self.chunk_fixed(data, 4 * 1024 * 1024).await;
        }

        let atoms = parse_mp4_atoms(data);
        if atoms.is_empty() {
            debug!("Failed to parse MP4 atoms, using fixed chunking");
            return self.chunk_fixed(data, 4 * 1024 * 1024).await;
        }

        let mut chunks = Vec::new();

        for atom in atoms {
            let atom_start = atom.offset as usize;
            let atom_end = (atom.offset + atom.size) as usize;

            if atom_end > data.len() {
                // Atom size extends past EOF (common for MOV mdat with size=0 or
                // extended-size atoms).  Emit remaining bytes as a Generic chunk so
                // reconstruction integrity is preserved, then stop.
                let remaining = &data[atom_start..];
                if !remaining.is_empty() {
                    chunks.push(ContentChunk {
                        id: Oid::hash(remaining),
                        data: remaining.to_vec(),
                        offset: atom.offset,
                        size: remaining.len(),
                        chunk_type: ChunkType::Generic,
                        perceptual_hash: None,
                        codec_hint: CodecHint::Unknown,
                    });
                }
                debug!(
                    atom_type = %String::from_utf8_lossy(&atom.atom_type),
                    remaining = remaining.len(),
                    "Truncated atom — emitted remaining bytes as Generic chunk"
                );
                break;
            }

            let atom_data = &data[atom_start..atom_end];
            let atom_type_str = String::from_utf8_lossy(&atom.atom_type);

            match &atom.atom_type {
                b"ftyp" => {
                    // File type - always small, single chunk
                    chunks.push(ContentChunk {
                        id: Oid::hash(atom_data),
                        data: atom_data.to_vec(),
                        offset: atom.offset,
                        size: atom_data.len(),
                        chunk_type: ChunkType::Metadata,
                        perceptual_hash: None,
                        codec_hint: CodecHint::Unknown,
                    });
                    debug!(atom_type = %atom_type_str, size = atom.size, "Parsed ftyp atom");
                }
                b"moov" => {
                    // Movie metadata container - parse nested atoms for finer deduplication
                    if atom.size > 8 {
                        let nested = parse_mp4_atoms(&atom_data[8..]); // Skip moov header

                        if nested.is_empty() {
                            // Fallback: keep as single chunk
                            chunks.push(ContentChunk {
                                id: Oid::hash(atom_data),
                                data: atom_data.to_vec(),
                                offset: atom.offset,
                                size: atom_data.len(),
                                chunk_type: ChunkType::Metadata,
                                perceptual_hash: None,
                                codec_hint: CodecHint::Unknown,
                            });
                        } else {
                            // Emit moov header (8 bytes) as separate chunk
                            let header = &atom_data[..8];
                            chunks.push(ContentChunk {
                                id: Oid::hash(header),
                                data: header.to_vec(),
                                offset: atom.offset,
                                size: 8,
                                chunk_type: ChunkType::Metadata,
                                perceptual_hash: None,
                                codec_hint: CodecHint::Unknown,
                            });

                            // Emit each nested atom as separate chunk.
                            // Clamp nested_end to atom_data.len() so that a nested
                            // atom whose declared size extends past the moov boundary
                            // still gets emitted rather than being silently dropped.
                            for nested_atom in &nested {
                                let nested_start = 8 + nested_atom.offset as usize;
                                let nested_end =
                                    (nested_start + nested_atom.size as usize).min(atom_data.len());
                                if nested_start < atom_data.len() {
                                    let nested_data = &atom_data[nested_start..nested_end];
                                    if !nested_data.is_empty() {
                                        let nested_type =
                                            String::from_utf8_lossy(&nested_atom.atom_type);
                                        chunks.push(ContentChunk {
                                            id: Oid::hash(nested_data),
                                            data: nested_data.to_vec(),
                                            offset: atom.offset + 8 + nested_atom.offset,
                                            size: nested_data.len(),
                                            chunk_type: ChunkType::Metadata,
                                            perceptual_hash: None,
                                            codec_hint: CodecHint::Unknown,
                                        });
                                        debug!(
                                            atom_type = %nested_type,
                                            size = nested_atom.size,
                                            "Parsed nested moov atom"
                                        );
                                    }
                                }
                            }
                        }
                    } else {
                        chunks.push(ContentChunk {
                            id: Oid::hash(atom_data),
                            data: atom_data.to_vec(),
                            offset: atom.offset,
                            size: atom_data.len(),
                            chunk_type: ChunkType::Metadata,
                            perceptual_hash: None,
                            codec_hint: CodecHint::Unknown,
                        });
                    }
                    debug!(atom_type = %atom_type_str, size = atom.size, "Parsed moov atom");
                }
                b"mdat" => {
                    // Media data — CDC sub-chunking for byte-exact reconstruction.
                    // Track-aware per-stream batching is intentionally NOT used here:
                    // it accumulates non-contiguous sample bytes into a single chunk,
                    // which causes reconstruction to produce the wrong total size when
                    // paired with gap-filling logic.  CDC produces contiguous, ordered
                    // chunks that always reconstruct the exact original bytes.
                    if atom.size > 4 * 1024 * 1024 {
                        // Always emit the mdat header as a Metadata chunk
                        let header = &atom_data[..atom.header_size as usize];
                        chunks.push(ContentChunk {
                            id: Oid::hash(header),
                            data: header.to_vec(),
                            offset: atom.offset,
                            size: header.len(),
                            chunk_type: ChunkType::Metadata,
                            perceptual_hash: None,
                            codec_hint: CodecHint::Unknown,
                        });

                        let mdat_content = &atom_data[atom.header_size as usize..];
                        let mdat_content_offset = atom.offset + atom.header_size as u64;

                        let sub_chunks = self
                            .chunk_fastcdc(
                                mdat_content,
                                2 * 1024 * 1024, // 2MB average (video-optimised)
                                1024 * 1024,     // 1MB minimum
                                8 * 1024 * 1024, // 8MB maximum
                            )
                            .await?;

                        let before = chunks.len();
                        for mut sub in sub_chunks {
                            sub.offset += mdat_content_offset;
                            sub.chunk_type = ChunkType::VideoStream;
                            chunks.push(sub);
                        }
                        debug!(
                            atom_type = %atom_type_str,
                            size = atom.size,
                            sub_chunks = chunks.len() - before,
                            "Parsed large mdat with CDC sub-chunking"
                        );
                    } else {
                        // Small mdat: single chunk
                        chunks.push(ContentChunk {
                            id: Oid::hash(atom_data),
                            data: atom_data.to_vec(),
                            offset: atom.offset,
                            size: atom_data.len(),
                            chunk_type: ChunkType::VideoStream,
                            perceptual_hash: None,
                            codec_hint: CodecHint::Unknown,
                        });
                        debug!(atom_type = %atom_type_str, size = atom.size, "Parsed small mdat atom");
                    }
                }
                b"moof" => {
                    // Movie fragment (fMP4) - treat as metadata
                    chunks.push(ContentChunk {
                        id: Oid::hash(atom_data),
                        data: atom_data.to_vec(),
                        offset: atom.offset,
                        size: atom_data.len(),
                        chunk_type: ChunkType::Metadata,
                        perceptual_hash: None,
                        codec_hint: CodecHint::Unknown,
                    });
                    debug!(atom_type = %atom_type_str, size = atom.size, "Parsed moof atom");
                }
                b"free" | b"skip" | b"wide" => {
                    // Free space / padding - still chunk it for reconstruction
                    chunks.push(ContentChunk {
                        id: Oid::hash(atom_data),
                        data: atom_data.to_vec(),
                        offset: atom.offset,
                        size: atom_data.len(),
                        chunk_type: ChunkType::Generic,
                        perceptual_hash: None,
                        codec_hint: CodecHint::Unknown,
                    });
                    debug!(atom_type = %atom_type_str, size = atom.size, "Parsed padding atom");
                }
                _ => {
                    // Other atoms: single chunk as generic
                    chunks.push(ContentChunk {
                        id: Oid::hash(atom_data),
                        data: atom_data.to_vec(),
                        offset: atom.offset,
                        size: atom_data.len(),
                        chunk_type: ChunkType::Generic,
                        perceptual_hash: None,
                        codec_hint: CodecHint::Unknown,
                    });
                    debug!(atom_type = %atom_type_str, size = atom.size, "Parsed unknown atom");
                }
            }
        }

        fill_coverage_gaps(data, &mut chunks);

        info!(
            chunks = chunks.len(),
            metadata_chunks = chunks
                .iter()
                .filter(|c| c.chunk_type == ChunkType::Metadata)
                .count(),
            video_chunks = chunks
                .iter()
                .filter(|c| c.chunk_type == ChunkType::VideoStream)
                .count(),
            total_size = data.len(),
            "MP4 atom-based chunking complete"
        );

        Ok(chunks)
    }

    /// Matroska/WebM chunking
    ///
    /// Parses EBML elements and creates content-aware chunks:
    /// - Each metadata element (Info, Tracks, Tags, Cues, etc.) gets its own chunk
    ///   for granular dedup (changing tags won't invalidate tracks)
    /// - Segment container emits header-only; children get individual chunks
    /// - Clusters > 4MB are CDC-subdivided (1MB avg) as VideoStream chunks
    /// - Small Clusters become single VideoStream chunks
    /// - Attachments become separate Generic chunks
    async fn chunk_matroska(&self, data: &[u8]) -> Result<Vec<ContentChunk>> {
        // LEVEL 1: Parse EBML elements
        let elements = parse_ebml_elements(data);

        if elements.is_empty() {
            warn!("No valid EBML elements found, falling back to fixed chunking");
            return self.chunk_fixed(data, 4 * 1024 * 1024).await;
        }

        // LEVEL 2: Validate EBML header exists
        if !elements.iter().any(|e| e.id == EBML_ID) {
            warn!("EBML header not found, not a valid Matroska file");
            return self.chunk_fixed(data, 4 * 1024 * 1024).await;
        }
        let mut chunks = Vec::new();

        for element in &elements {
            let elem_start = element.offset as usize;
            let elem_size = if element.data_size == u64::MAX {
                // Unknown size: extends to end of data (from element start, including header)
                data.len().saturating_sub(elem_start)
            } else {
                element.header_size as usize + element.data_size as usize
            };
            let elem_end = (elem_start + elem_size).min(data.len());

            if elem_end <= elem_start {
                continue;
            }

            match element.id {
                CLUSTER_ID => {
                    let cluster_data = &data[elem_start..elem_end];
                    let header_size = element.header_size as usize;
                    let cluster_content_start = header_size;
                    let base_offset = element.offset + header_size as u64;

                    // Always emit the Cluster header as a Metadata chunk —
                    // it contains the Timecode which marks the start of the cluster.
                    let header = &cluster_data[..header_size.min(cluster_data.len())];
                    chunks.push(ContentChunk {
                        id: Oid::hash(header),
                        data: header.to_vec(),
                        offset: element.offset,
                        size: header.len(),
                        chunk_type: ChunkType::Metadata,
                        perceptual_hash: None,
                        codec_hint: CodecHint::Unknown,
                    });

                    if cluster_data.len() <= cluster_content_start {
                        continue;
                    }
                    let cluster_content = &cluster_data[cluster_content_start..];

                    // CDC sub-chunking for byte-exact reconstruction.
                    // Per-stream batching (chunk_cluster_by_tracks) is intentionally
                    // NOT used: it accumulates non-contiguous bytes causing size
                    // mismatch during chunk reconstruction.
                    if cluster_content.len() > 4 * 1024 * 1024 {
                        let sub_chunks = self
                            .chunk_fastcdc(
                                cluster_content,
                                2 * 1024 * 1024,
                                1024 * 1024,
                                8 * 1024 * 1024,
                            )
                            .await?;

                        for mut sub in sub_chunks {
                            sub.offset += base_offset;
                            sub.chunk_type = ChunkType::VideoStream;
                            chunks.push(sub);
                        }
                    } else {
                        chunks.push(ContentChunk {
                            id: Oid::hash(cluster_content),
                            data: cluster_content.to_vec(),
                            offset: base_offset,
                            size: cluster_content.len(),
                            chunk_type: ChunkType::VideoStream,
                            perceptual_hash: None,
                            codec_hint: CodecHint::Unknown,
                        });
                    }
                }
                ATTACHMENTS_ID => {
                    // Attachments as separate Generic chunk
                    let attach_data = &data[elem_start..elem_end];
                    chunks.push(ContentChunk {
                        id: Oid::hash(attach_data),
                        data: attach_data.to_vec(),
                        offset: element.offset,
                        size: attach_data.len(),
                        chunk_type: ChunkType::Generic,
                        perceptual_hash: None,
                        codec_hint: CodecHint::Unknown,
                    });
                }
                // Segment is a container — emit its header as a small metadata chunk
                // so child elements get their own chunks
                SEGMENT_ID => {
                    let header_size = element.header_size as usize;
                    let header = &data[elem_start..elem_start + header_size];
                    chunks.push(ContentChunk {
                        id: Oid::hash(header),
                        data: header.to_vec(),
                        offset: element.offset,
                        size: header.len(),
                        chunk_type: ChunkType::Metadata,
                        perceptual_hash: None,
                        codec_hint: CodecHint::Unknown,
                    });
                }
                // Each metadata element gets its own chunk for granular dedup:
                // changing Tags won't invalidate Tracks, editing Chapters won't
                // invalidate Cues, etc.
                EBML_ID | SEEKHEAD_ID | INFO_ID | TRACKS_ID | CUES_ID | CHAPTERS_ID | TAGS_ID => {
                    let elem_data = &data[elem_start..elem_end];
                    chunks.push(ContentChunk {
                        id: Oid::hash(elem_data),
                        data: elem_data.to_vec(),
                        offset: element.offset,
                        size: elem_data.len(),
                        chunk_type: ChunkType::Metadata,
                        perceptual_hash: None,
                        codec_hint: CodecHint::Unknown,
                    });
                }
                _ => {
                    // Unknown elements: emit as individual metadata chunks
                    let elem_data = &data[elem_start..elem_end];
                    chunks.push(ContentChunk {
                        id: Oid::hash(elem_data),
                        data: elem_data.to_vec(),
                        offset: element.offset,
                        size: elem_data.len(),
                        chunk_type: ChunkType::Metadata,
                        perceptual_hash: None,
                        codec_hint: CodecHint::Unknown,
                    });
                }
            }
        }

        // LEVEL 3: Ensure we produced chunks
        if chunks.is_empty() {
            warn!("No chunks created from Matroska parsing, falling back");
            return self.chunk_fixed(data, 4 * 1024 * 1024).await;
        }

        fill_coverage_gaps(data, &mut chunks);

        info!(
            chunks = chunks.len(),
            metadata = chunks
                .iter()
                .filter(|c| c.chunk_type == ChunkType::Metadata)
                .count(),
            clusters = chunks
                .iter()
                .filter(|c| c.chunk_type == ChunkType::VideoStream)
                .count(),
            total_size = data.len(),
            "Matroska EBML chunking complete"
        );

        Ok(chunks)
    }

    /// GLB (binary glTF) format chunking
    ///
    /// GLB structure:
    /// - 12-byte header: magic (4) + version (4) + total length (4)
    /// - JSON chunk: length (4) + type "JSON" (4) + JSON data
    /// - Binary chunk: length (4) + type "BIN\0" (4) + binary data
    async fn chunk_glb(&self, data: &[u8]) -> Result<Vec<ContentChunk>> {
        let mut chunks = Vec::new();

        // GLB magic: "glTF" (0x46546C67)
        const GLB_MAGIC: &[u8] = b"glTF";
        const JSON_CHUNK_TYPE: u32 = 0x4E4F534A; // "JSON"
        const BIN_CHUNK_TYPE: u32 = 0x004E4942; // "BIN\0"

        // Validate GLB header
        if data.len() < 12 || &data[0..4] != GLB_MAGIC {
            debug!("Not a valid GLB file, using fixed chunking");
            return self.chunk_fixed(data, 4 * 1024 * 1024).await;
        }

        let version = u32::from_le_bytes([data[4], data[5], data[6], data[7]]);
        let _total_length = u32::from_le_bytes([data[8], data[9], data[10], data[11]]) as usize;

        if version != 2 {
            debug!(version, "Unsupported GLB version, using fixed chunking");
            return self.chunk_fixed(data, 4 * 1024 * 1024).await;
        }

        // Header chunk (12 bytes)
        let header_data = &data[0..12];
        chunks.push(ContentChunk {
            id: Oid::hash(header_data),
            data: header_data.to_vec(),
            offset: 0,
            size: 12,
            chunk_type: ChunkType::Metadata,
            perceptual_hash: None,
            codec_hint: CodecHint::Unknown,
        });

        let mut pos = 12;

        // Parse JSON and BIN chunks
        while pos + 8 <= data.len() {
            let chunk_length =
                u32::from_le_bytes([data[pos], data[pos + 1], data[pos + 2], data[pos + 3]])
                    as usize;
            let chunk_type =
                u32::from_le_bytes([data[pos + 4], data[pos + 5], data[pos + 6], data[pos + 7]]);

            let chunk_start = pos;
            let chunk_data_start = pos + 8;
            let chunk_end = (chunk_data_start + chunk_length).min(data.len());

            // Include chunk header (8 bytes) as a tiny metadata separator so that
            // the BIN data start offset is stable even when sub-chunked.
            let header_bytes = &data[chunk_start..chunk_start + 8];
            let (ct, label) = match chunk_type {
                JSON_CHUNK_TYPE => (ChunkType::Metadata, "JSON"),
                BIN_CHUNK_TYPE => (ChunkType::Generic, "BIN"),
                _ => (ChunkType::Generic, "Unknown"),
            };

            // Emit the 8-byte chunk header always as a metadata marker.
            chunks.push(ContentChunk {
                id: Oid::hash(header_bytes),
                data: header_bytes.to_vec(),
                offset: chunk_start as u64,
                size: 8,
                chunk_type: ChunkType::Metadata,
                perceptual_hash: None,
                codec_hint: CodecHint::Unknown,
            });

            let full_chunk_data = &data[chunk_data_start..chunk_end];

            // BIN payloads > 4 MB: CDC-subdivide for better dedup granularity.
            // Matches the MKV large-Cluster strategy (1 MB avg / 512 KB min / 4 MB max).
            // GLB BIN buffers store vertex arrays, textures, and animation data. Large
            // buffers are common (scanned meshes, terrain, photogrammetry) and often share
            // sub-regions across model revisions.
            const GLB_BIN_SUBCHUNK_THRESHOLD: usize = 4 * 1024 * 1024; // 4 MB
            if chunk_type == BIN_CHUNK_TYPE && full_chunk_data.len() > GLB_BIN_SUBCHUNK_THRESHOLD {
                use fastcdc::v2020::FastCDC;
                let avg: u32 = 1024 * 1024; // 1 MB
                let min: u32 = 512 * 1024; // 512 KB
                let max: u32 = 4 * 1024 * 1024; // 4 MB
                let cdc = FastCDC::new(full_chunk_data, min, avg, max);
                let base_offset = chunk_data_start as u64;
                for entry in cdc {
                    let sub = &full_chunk_data[entry.offset..entry.offset + entry.length];
                    chunks.push(ContentChunk {
                        id: Oid::hash(sub),
                        data: sub.to_vec(),
                        offset: base_offset + entry.offset as u64,
                        size: entry.length,
                        chunk_type: ChunkType::Generic,
                        perceptual_hash: None,
                        codec_hint: CodecHint::Unknown,
                    });
                }
                debug!(
                    chunk_type = label,
                    offset = chunk_start,
                    size = full_chunk_data.len(),
                    "GLB BIN chunk (sub-chunked via FastCDC)"
                );
            } else {
                chunks.push(ContentChunk {
                    id: Oid::hash(full_chunk_data),
                    data: full_chunk_data.to_vec(),
                    offset: chunk_data_start as u64,
                    size: full_chunk_data.len(),
                    chunk_type: ct,
                    perceptual_hash: None,
                    codec_hint: CodecHint::Unknown,
                });
                debug!(
                    chunk_type = label,
                    offset = chunk_start,
                    size = full_chunk_data.len(),
                    "GLB chunk"
                );
            }

            pos = chunk_end;
        }

        // Handle remaining data if any
        if pos < data.len() {
            let remaining = &data[pos..];
            chunks.push(ContentChunk {
                id: Oid::hash(remaining),
                data: remaining.to_vec(),
                offset: pos as u64,
                size: remaining.len(),
                chunk_type: ChunkType::Generic,
                perceptual_hash: None,
                codec_hint: CodecHint::Unknown,
            });
        }

        if chunks.is_empty() {
            warn!("No chunks created from GLB parsing, falling back");
            return self.chunk_fixed(data, 4 * 1024 * 1024).await;
        }

        info!(
            chunks = chunks.len(),
            total_size = data.len(),
            "GLB chunking complete"
        );

        Ok(chunks)
    }

    /// Text-based 3D model chunking (OBJ, STL ASCII, PLY ASCII)
    ///
    /// These formats are line-oriented text files that benefit from
    /// structure-aware chunking at logical boundaries:
    /// - OBJ: groups (g), objects (o), material uses (usemtl)
    /// - STL: facet boundaries
    /// - PLY: header vs data sections
    async fn chunk_3d_text(&self, data: &[u8]) -> Result<Vec<ContentChunk>> {
        // Check if data is valid UTF-8 text
        if !data
            .iter()
            .take(1024)
            .all(|&b| (..128).contains(&b) || b >= 0xC0)
        {
            // Binary format - use rolling CDC
            let (avg, min, max) = get_chunk_params(data.len() as u64);
            return self.chunk_fastcdc(data, avg, min, max).await;
        }

        let mut chunks = Vec::new();
        let mut chunk_start = 0;
        let min_chunk_size = 256 * 1024; // 256KB minimum chunk
        let max_chunk_size = 4 * 1024 * 1024; // 4MB maximum

        // Parse as text, split at logical boundaries
        let text = String::from_utf8_lossy(data);
        let lines: Vec<&str> = text.lines().collect();
        let mut current_pos = 0;

        for line in lines.iter() {
            let line_len = line.len() + 1; // +1 for newline
            let chunk_size = current_pos - chunk_start;

            // Check for logical boundaries (OBJ groups/objects, STL facets)
            let is_boundary = line.starts_with("g ")
                || line.starts_with("o ")
                || line.starts_with("usemtl ")
                || line.starts_with("facet ")
                || line.starts_with("end_header");

            // Create chunk at boundary if size is acceptable
            if is_boundary && chunk_size >= min_chunk_size {
                let chunk_data = &data[chunk_start..current_pos];
                chunks.push(ContentChunk {
                    id: Oid::hash(chunk_data),
                    data: chunk_data.to_vec(),
                    offset: chunk_start as u64,
                    size: chunk_data.len(),
                    chunk_type: ChunkType::Generic,
                    perceptual_hash: None,
                    codec_hint: CodecHint::Unknown,
                });
                chunk_start = current_pos;
            }

            // Force chunk if we exceed max size
            if chunk_size >= max_chunk_size {
                let chunk_data = &data[chunk_start..current_pos];
                chunks.push(ContentChunk {
                    id: Oid::hash(chunk_data),
                    data: chunk_data.to_vec(),
                    offset: chunk_start as u64,
                    size: chunk_data.len(),
                    chunk_type: ChunkType::Generic,
                    perceptual_hash: None,
                    codec_hint: CodecHint::Unknown,
                });
                chunk_start = current_pos;
            }

            current_pos += line_len;

            // Prevent going past data length due to line ending differences
            if current_pos > data.len() {
                current_pos = data.len();
            }
        }

        // Final chunk
        if chunk_start < data.len() {
            let chunk_data = &data[chunk_start..];
            chunks.push(ContentChunk {
                id: Oid::hash(chunk_data),
                data: chunk_data.to_vec(),
                offset: chunk_start as u64,
                size: chunk_data.len(),
                chunk_type: ChunkType::Generic,
                perceptual_hash: None,
                codec_hint: CodecHint::Unknown,
            });
        }

        // Fallback if no chunks created
        if chunks.is_empty() {
            return self.chunk_fixed(data, 4 * 1024 * 1024).await;
        }

        info!(
            chunks = chunks.len(),
            total_size = data.len(),
            "3D text model chunking complete"
        );

        Ok(chunks)
    }

    /// FBX binary format chunking
    ///
    /// FBX binary files have a node-based structure that can be parsed
    /// for structure-aware chunking. Falls back to CDC for ASCII FBX.
    async fn chunk_fbx(&self, data: &[u8]) -> Result<Vec<ContentChunk>> {
        // FBX binary magic: "Kaydara FBX Binary  \x00"
        const FBX_MAGIC: &[u8] = b"Kaydara FBX Binary  \x00";

        if data.len() < 27 || &data[0..21] != FBX_MAGIC {
            // ASCII FBX or invalid - use rolling CDC
            debug!("FBX file is ASCII or invalid, using rolling CDC");
            let (avg, min, max) = get_chunk_params(data.len() as u64);
            return self.chunk_fastcdc(data, avg, min, max).await;
        }

        // Parse FBX version (bytes 23-26, little-endian)
        let _version = u32::from_le_bytes([data[23], data[24], data[25], data[26]]);

        let mut chunks = Vec::new();

        // Header chunk (first 27 bytes)
        let header = &data[0..27];
        chunks.push(ContentChunk {
            id: Oid::hash(header),
            data: header.to_vec(),
            offset: 0,
            size: 27,
            chunk_type: ChunkType::Metadata,
            perceptual_hash: None,
            codec_hint: CodecHint::Unknown,
        });

        // For FBX, use adaptive rolling CDC on the rest of the data
        // Full FBX node parsing is complex; CDC provides good dedup
        if data.len() > 27 {
            let content = &data[27..];
            let (avg, min, max) = get_chunk_params(content.len() as u64);
            let sub_chunks = self.chunk_fastcdc(content, avg, min, max).await?;

            for mut chunk in sub_chunks {
                chunk.offset += 27;
                chunks.push(chunk);
            }
        }

        info!(
            chunks = chunks.len(),
            total_size = data.len(),
            "FBX chunking complete"
        );

        Ok(chunks)
    }
}

/// Patch any uncovered byte ranges with Generic chunks.
///
/// Format-aware parsers can miss bytes due to EBML padding, atom-size edge
/// cases, Void elements, CRC-32 elements, or other format quirks.  This
/// helper sorts chunks by offset, finds uncovered ranges, and emits a Generic
/// chunk for each gap so that concatenating all chunks reconstructs the exact
/// original file.
///
/// Safe to call when chunks come from CDC (`chunk_fastcdc`) because CDC always
/// produces contiguous, non-overlapping slices — no duplication risk.
fn fill_coverage_gaps(data: &[u8], chunks: &mut Vec<ContentChunk>) {
    if data.is_empty() {
        return;
    }
    chunks.sort_unstable_by_key(|c| c.offset);

    let mut gap_chunks: Vec<ContentChunk> = Vec::new();
    let mut covered_up_to: u64 = 0;

    for chunk in chunks.iter() {
        if chunk.offset > covered_up_to {
            let gap_start = covered_up_to as usize;
            let gap_end = chunk.offset as usize;
            if gap_end <= data.len() && gap_start < gap_end {
                let gap_data = &data[gap_start..gap_end];
                gap_chunks.push(ContentChunk {
                    id: Oid::hash(gap_data),
                    data: gap_data.to_vec(),
                    offset: covered_up_to,
                    size: gap_data.len(),
                    chunk_type: ChunkType::Generic,
                    perceptual_hash: None,
                    codec_hint: CodecHint::Unknown,
                });
            }
        }
        let chunk_end = chunk.offset + chunk.size as u64;
        if chunk_end > covered_up_to {
            covered_up_to = chunk_end;
        }
    }

    if (covered_up_to as usize) < data.len() {
        let trail = &data[covered_up_to as usize..];
        gap_chunks.push(ContentChunk {
            id: Oid::hash(trail),
            data: trail.to_vec(),
            offset: covered_up_to,
            size: trail.len(),
            chunk_type: ChunkType::Generic,
            perceptual_hash: None,
            codec_hint: CodecHint::Unknown,
        });
    }

    if !gap_chunks.is_empty() {
        warn!(
            gaps = gap_chunks.len(),
            total_gap_bytes = gap_chunks.iter().map(|c| c.size).sum::<usize>(),
            "Coverage gaps patched with Generic chunks"
        );
        chunks.extend(gap_chunks);
        chunks.sort_unstable_by_key(|c| c.offset);
    }
}

fn parse_mp4_atoms(data: &[u8]) -> Vec<Mp4Atom> {
    let mut atoms = Vec::new();
    let mut pos = 0u64;
    let data_len = data.len() as u64;

    while pos + 8 <= data_len {
        let offset = pos as usize;

        // Read size (4 bytes, big-endian)
        let size = u32::from_be_bytes([
            data[offset],
            data[offset + 1],
            data[offset + 2],
            data[offset + 3],
        ]) as u64;

        // Read type (4 bytes FourCC)
        let mut atom_type = [0u8; 4];
        atom_type.copy_from_slice(&data[offset + 4..offset + 8]);

        // Validate atom type (should be printable ASCII or known types)
        let is_valid_type = atom_type
            .iter()
            .all(|&b| (0x20..=0x7E).contains(&b) || b == 0x00);
        if !is_valid_type {
            // Invalid atom type, stop parsing
            break;
        }

        let (actual_size, header_size): (u64, u8) = match size {
            0 => {
                // Size 0 = extends to EOF
                (data_len - pos, 8)
            }
            1 => {
                // Extended size (8-byte size after type)
                if offset + 16 > data.len() {
                    break;
                }
                let ext_size = u64::from_be_bytes([
                    data[offset + 8],
                    data[offset + 9],
                    data[offset + 10],
                    data[offset + 11],
                    data[offset + 12],
                    data[offset + 13],
                    data[offset + 14],
                    data[offset + 15],
                ]);
                (ext_size, 16)
            }
            _ => (size, 8),
        };

        // Sanity check: atom shouldn't extend beyond data
        if pos + actual_size > data_len {
            // Truncated atom - include what we have
            atoms.push(Mp4Atom {
                atom_type,
                offset: pos,
                size: data_len - pos,
                header_size,
            });
            break;
        }

        atoms.push(Mp4Atom {
            atom_type,
            offset: pos,
            size: actual_size,
            header_size,
        });

        pos += actual_size;
    }

    atoms
}

/// Read EBML VINT for Element ID (marker bit KEPT in value)
///
/// Returns (id, bytes_consumed) or None if invalid.
/// Element IDs include the VINT marker bit as part of the value.
fn read_ebml_id(data: &[u8], pos: usize) -> Option<(u32, u8)> {
    if pos >= data.len() {
        return None;
    }

    let first = data[pos];
    if first == 0 {
        return None; // Invalid: leading zeros not allowed in shortest form
    }

    // Count leading zeros to determine byte count
    let len = (first.leading_zeros() + 1) as usize;
    if len > 4 || pos + len > data.len() {
        return None; // Matroska limits IDs to 4 bytes
    }

    // Build ID value (marker bit is kept)
    let mut id = first as u32;
    for i in 1..len {
        id = (id << 8) | data[pos + i] as u32;
    }

    Some((id, len as u8))
}

/// Read EBML VINT for Element Size (marker bit REMOVED from value)
///
/// Returns (size, bytes_consumed) or None if invalid.
/// Size u64::MAX indicates "unknown size" (all data bits = 1).
fn read_ebml_size(data: &[u8], pos: usize) -> Option<(u64, u8)> {
    if pos >= data.len() {
        return None;
    }

    let first = data[pos];
    if first == 0 {
        return None; // Invalid: all zeros
    }

    // Count leading zeros to determine byte count
    let len = (first.leading_zeros() + 1) as usize;
    if len > 8 || pos + len > data.len() {
        return None;
    }

    // Clear marker bit and build size value
    // When len==8 (leading_zeros==7), the entire first byte is header — no data bits.
    let mask = if len >= 8 { 0u8 } else { 0xFFu8 >> len };
    let mut size = (first & mask) as u64;
    for i in 1..len {
        size = (size << 8) | data[pos + i] as u64;
    }

    // Check for "unknown size" (all data bits = 1)
    // For 1-byte: 0x7F, 2-byte: 0x3FFF, etc.
    let unknown_marker = (1u64 << (7 * len)) - 1;
    if size == unknown_marker {
        return Some((u64::MAX, len as u8));
    }

    Some((size, len as u8))
}

/// Parse EBML elements from Matroska/WebM data
///
/// Returns a vector of EbmlElement headers. Enters Segment containers
/// to parse their children (Clusters, Info, Tracks, etc.).
/// Stops on unknown size elements or parse errors.
fn parse_ebml_elements(data: &[u8]) -> Vec<EbmlElement> {
    let mut elements = Vec::new();
    let mut pos = 0usize;
    let data_len = data.len();

    while pos + 2 <= data_len {
        // Read Element ID
        let (id, id_len) = match read_ebml_id(data, pos) {
            Some(v) => v,
            None => break,
        };

        // Read Element Size
        let (size, size_len) = match read_ebml_size(data, pos + id_len as usize) {
            Some(v) => v,
            None => break,
        };

        let header_size = id_len + size_len;

        // Skip Void and CRC-32 elements (padding/checksum)
        if id == VOID_ID || id == CRC32_ID {
            if size != u64::MAX {
                pos += header_size as usize + size as usize;
            } else {
                break; // Cannot skip unknown size
            }
            continue;
        }

        let element = EbmlElement {
            id,
            offset: pos as u64,
            header_size,
            data_size: size,
        };
        elements.push(element);

        // Handle Segment: parse its children, don't skip over it
        if id == SEGMENT_ID {
            pos += header_size as usize; // Enter Segment
        } else if size == u64::MAX {
            // Unknown size: can't skip, stop parsing
            break;
        } else {
            // Skip to next element
            let next_pos = pos + header_size as usize + size as usize;
            if next_pos > data_len {
                break; // Element extends beyond data
            }
            pos = next_pos;
        }
    }

    elements
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
}
