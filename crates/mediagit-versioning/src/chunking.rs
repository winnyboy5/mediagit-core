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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChunkStrategy {
    /// Fixed-size chunks (simple, predictable)
    Fixed { size: usize },
    /// Rolling hash chunking (content-defined boundaries)
    Rolling { avg_size: usize, min_size: usize, max_size: usize },
    /// Media-aware chunking (parse structure, separate streams)
    MediaAware,
}

impl Default for ChunkStrategy {
    fn default() -> Self {
        ChunkStrategy::MediaAware
    }
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
}

/// Chunk type classification
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
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
const EBML_ID: u32 = 0x1A45DFA3;        // EBML Header
const SEGMENT_ID: u32 = 0x18538067;     // Segment container
const SEEKHEAD_ID: u32 = 0x114D9B74;    // SeekHead (index)
const INFO_ID: u32 = 0x1549A966;        // Segment Info
const TRACKS_ID: u32 = 0x1654AE6B;      // Track definitions
const CLUSTER_ID: u32 = 0x1F43B675;     // Cluster (media data)
const CUES_ID: u32 = 0x1C53BB6B;        // Cues (seek index)
const CHAPTERS_ID: u32 = 0x1043A770;    // Chapters
const TAGS_ID: u32 = 0x1254C367;        // Tags (metadata)
const ATTACHMENTS_ID: u32 = 0x1941A469; // Attachments
const VOID_ID: u32 = 0xEC;              // Void (padding, skip)
const CRC32_ID: u32 = 0xBF;             // CRC-32 (skip)
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
        0..=100_000_000 => (1 * MB, 512 * 1024, 4 * MB),           // < 100MB: 1MB avg
        100_000_001..=10_000_000_000 => (2 * MB, 1 * MB, 8 * MB),  // 100MB-10GB: 2MB avg
        10_000_000_001..=100_000_000_000 => (4 * MB, 2 * MB, 16 * MB), // 10GB-100GB: 4MB avg
        _ => (8 * MB, 4 * MB, 32 * MB),                           // > 100GB: 8MB avg
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
            ChunkStrategy::Rolling { avg_size, min_size, max_size } => {
                self.chunk_rolling(data, avg_size, min_size, max_size).await
            }
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
        use tokio::io::AsyncReadExt;
        
        let path = path.as_ref();
        let file_size = std::fs::metadata(path)
            .map_err(|e| anyhow::anyhow!("Failed to get file metadata: {}", e))?
            .len();
        
        let mut chunk_oids = Vec::new();
        
        // Tier 1: Small files (< 10MB): Load into memory for fastest processing
        if file_size < 10 * 1024 * 1024 {
            let data = tokio::fs::read(path).await
                .map_err(|e| anyhow::anyhow!("Failed to read file: {}", e))?;
            let filename = path.file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("unknown");
            
            let chunks = self.chunk(&data, filename).await?;
            for chunk in chunks {
                let oid = chunk.id;
                on_chunk(chunk).await?;  // Process immediately
                chunk_oids.push(oid);    // Only store the OID
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
            let data = tokio::fs::read(path).await
                .map_err(|e| anyhow::anyhow!("Failed to read file: {}", e))?;
            let filename = path.file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("unknown");
            
            // Use MediaAware chunking for better boundaries
            let chunks = self.chunk(&data, filename).await?;
            for chunk in chunks {
                let oid = chunk.id;
                on_chunk(chunk).await?;  // Process immediately (allows GC of chunk data)
                chunk_oids.push(oid);
            }
            return Ok(chunk_oids);
        }
        
        // Large files: Stream with adaptive chunk sizes
        let (avg_size, _min_size, _max_size) = get_chunk_params(file_size);
        info!(
            "Streaming chunking: file_size={}, avg_chunk={}KB",
            file_size,
            avg_size / 1024
        );
        
        // Open file for streaming
        let mut file = tokio::fs::File::open(path).await
            .map_err(|e| anyhow::anyhow!("Failed to open file: {}", e))?;
        
        // For streaming large files, use FIXED-SIZE chunking (much faster)
        // Rolling hash CDC is too slow for streaming (O(n) byte-by-byte checks)
        let chunk_size = avg_size;
        let mut buffer = vec![0u8; chunk_size];
        let mut file_offset = 0u64;
        
        loop {
            // Read exactly one chunk worth of data
            let mut bytes_in_chunk = 0;
            loop {
                let bytes_read = file.read(&mut buffer[bytes_in_chunk..]).await?;
                if bytes_read == 0 {
                    break; // EOF
                }
                bytes_in_chunk += bytes_read;
                if bytes_in_chunk >= chunk_size {
                    break; // Full chunk
                }
            }
            
            if bytes_in_chunk == 0 {
                break; // EOF
            }
            
            // Create chunk from data (may be less than chunk_size at EOF)
            let chunk_data = &buffer[..bytes_in_chunk];
            let id = Oid::hash(chunk_data);
            
            let chunk = ContentChunk {
                id,
                data: chunk_data.to_vec(),
                offset: file_offset,
                size: chunk_data.len(),
                chunk_type: ChunkType::Generic,
                perceptual_hash: None,
            };
            
            on_chunk(chunk).await?;
            chunk_oids.push(id);
            
            file_offset += bytes_in_chunk as u64;
            
            // If we read less than chunk_size, we hit EOF
            if bytes_in_chunk < chunk_size {
                break;
            }
        }
        
        info!("Streaming chunking complete: {} chunks created", chunk_oids.len());
        Ok(chunk_oids)
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

    /// Rolling hash chunking with content-defined boundaries
    async fn chunk_rolling(
        &self,
        data: &[u8],
        avg_size: usize,
        min_size: usize,
        max_size: usize,
    ) -> Result<Vec<ContentChunk>> {
        let mut chunks = Vec::new();
        let mut offset = 0u64;
        let mut start = 0;

        // Rolling hash window
        const WINDOW_SIZE: usize = 48;
        let mask = (1 << (avg_size.trailing_zeros())) - 1;

        let mut i = min_size;
        while i < data.len() {
            // Calculate rolling hash for boundary detection
            let window_end = (i + WINDOW_SIZE).min(data.len());
            let hash = rolling_hash(&data[i..window_end]);

            // Check if we hit a boundary or max size
            let is_boundary = (hash & mask) == 0;
            let chunk_size = i - start;

            if is_boundary || chunk_size >= max_size || i + WINDOW_SIZE >= data.len() {
                let chunk_data = &data[start..i];
                let id = Oid::hash(chunk_data);

                chunks.push(ContentChunk {
                    id,
                    data: chunk_data.to_vec(),
                    offset,
                    size: chunk_data.len(),
                    chunk_type: ChunkType::Generic,
                    perceptual_hash: None,
                });

                offset += chunk_data.len() as u64;
                start = i;
                i += min_size;
            } else {
                i += 1;
            }
        }

        // Handle remaining data
        if start < data.len() {
            let chunk_data = &data[start..];
            let id = Oid::hash(chunk_data);

            chunks.push(ContentChunk {
                id,
                data: chunk_data.to_vec(),
                offset,
                size: chunk_data.len(),
                chunk_type: ChunkType::Generic,
                perceptual_hash: None,
            });
        }

        debug!(
            chunks = chunks.len(),
            avg_size = avg_size,
            total_size = data.len(),
            "Rolling hash chunking complete"
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
            "avi" | "riff" | "wav" => self.chunk_avi(data).await,
            "mp4" | "mov" | "m4v" | "m4a" | "3gp" => self.chunk_mp4(data).await,
            "mkv" | "webm" | "mka" | "mk3d" => self.chunk_matroska(data).await,
            "mpg" | "mpeg" | "vob" | "mts" | "m2ts" => {
                // MPEG Program/Transport Streams - use rolling CDC
                let (avg, min, max) = get_chunk_params(data.len() as u64);
                self.chunk_rolling(data, avg, min, max).await
            }

            // 3D Models - Structure-aware chunking
            "glb" | "gltf" => self.chunk_glb(data).await,
            "obj" | "stl" | "ply" => self.chunk_3d_text(data).await,
            "fbx" => self.chunk_fbx(data).await,
            "usd" | "usda" | "usdc" | "usdz" => {
                // USD ecosystem - rolling CDC for scene graph dedup
                let (avg, min, max) = get_chunk_params(data.len() as u64);
                self.chunk_rolling(data, avg, min, max).await
            }
            "abc" => {
                // Alembic cache - rolling CDC for animation dedup
                let (avg, min, max) = get_chunk_params(data.len() as u64);
                self.chunk_rolling(data, avg, min, max).await
            }
            "blend" | "max" | "ma" | "mb" | "c4d" | "hip" | "zpr" | "ztl" => {
                // Application-specific 3D formats - rolling CDC
                let (avg, min, max) = get_chunk_params(data.len() as u64);
                self.chunk_rolling(data, avg, min, max).await
            }

            // Text/Code - Rolling CDC for incremental dedup (Brotli compression)
            "csv" | "tsv" | "json" | "xml" | "html" | "txt" | "md" | "rst" |
            "rs" | "py" | "js" | "ts" | "go" | "java" | "c" | "cpp" | "h" |
            "yaml" | "yml" | "toml" | "ini" | "cfg" |
            "sql" | "graphql" | "proto" => {
                let (avg, min, max) = get_chunk_params(data.len() as u64);
                self.chunk_rolling(data, avg, min, max).await
            }
            
            // ML Data formats - Rolling CDC for dataset versioning
            "parquet" | "arrow" | "feather" | "orc" | "avro" |
            "hdf5" | "h5" | "nc" | "netcdf" | "npy" | "npz" | "tfrecords" | "petastorm" => {
                let (avg, min, max) = get_chunk_params(data.len() as u64);
                self.chunk_rolling(data, avg, min, max).await
            }
            
            // ML Models - Rolling CDC for fine-tuning dedup
            "pt" | "pth" | "ckpt" | "pb" | "safetensors" | "bin" |
            "pkl" | "joblib" => {
                let (avg, min, max) = get_chunk_params(data.len() as u64);
                self.chunk_rolling(data, avg, min, max).await
            }
            
            // ML Deployment - Rolling CDC for model versioning
            "onnx" | "gguf" | "ggml" | "tflite" | "mlmodel" | "coreml" |
            "keras" | "pte" | "mleap" | "pmml" | "llamafile" => {
                let (avg, min, max) = get_chunk_params(data.len() as u64);
                self.chunk_rolling(data, avg, min, max).await
            }
            
            // Documents - Rolling CDC
            "pdf" | "svg" | "eps" | "ai" => {
                let (avg, min, max) = get_chunk_params(data.len() as u64);
                self.chunk_rolling(data, avg, min, max).await
            }

            // Design tools - Rolling CDC for incremental design changes
            "fig" | "sketch" | "xd" | "indd" | "indt" => {
                let (avg, min, max) = get_chunk_params(data.len() as u64);
                self.chunk_rolling(data, avg, min, max).await
            }
            
            // Lossless audio - Rolling CDC
            "flac" | "aiff" | "alac" => {
                let (avg, min, max) = get_chunk_params(data.len() as u64);
                self.chunk_rolling(data, avg, min, max).await
            }
            
            // Compressed formats - Fixed chunking (already compressed, replaced entirely)
            "jpg" | "jpeg" | "png" | "gif" | "webp" | "avif" | "heic" |
            "mp3" | "aac" | "ogg" | "opus" |
            "zip" | "7z" | "rar" | "gz" | "xz" | "bz2" => {
                self.chunk_fixed(data, 4 * 1024 * 1024).await
            }
            
            // Unknown - Rolling CDC as safe default for dedup
            _ => {
                debug!(extension = extension, "Unknown type, using rolling (CDC) chunking for dedup");
                let (avg, min, max) = get_chunk_params(data.len() as u64);
                self.chunk_rolling(data, avg, min, max).await
            }
        }
    }

    /// AVI/RIFF format chunking
    async fn chunk_avi(&self, data: &[u8]) -> Result<Vec<ContentChunk>> {
        let mut chunks = Vec::new();
        let mut offset = 0u64;

        // Simple RIFF parser - find LIST chunks and hdrl/movi/idx1
        if data.len() < 12 || &data[0..4] != b"RIFF" {
            debug!("Not a valid RIFF file, using fixed chunking");
            return self.chunk_fixed(data, 4 * 1024 * 1024).await;
        }

        // IMPORTANT: Include the RIFF header (first 12 bytes) as a chunk
        // This is needed for correct reconstruction of the original file
        let header_data = &data[0..12];
        let header_id = Oid::hash(header_data);
        chunks.push(ContentChunk {
            id: header_id,
            data: header_data.to_vec(),
            offset,
            size: 12,
            chunk_type: ChunkType::Metadata,
            perceptual_hash: None,
        });
        offset = 12;

        // Parse RIFF structure starting after header
        let mut pos = 12;

        while pos < data.len() {
            if pos + 8 > data.len() {
                // Handle remaining bytes that don't form a complete chunk header
                if pos < data.len() {
                    let remaining_data = &data[pos..];
                    let remaining_id = Oid::hash(remaining_data);
                    chunks.push(ContentChunk {
                        id: remaining_id,
                        data: remaining_data.to_vec(),
                        offset,
                        size: remaining_data.len(),
                        chunk_type: ChunkType::Generic,
                        perceptual_hash: None,
                    });
                }
                break;
            }

            let fourcc = &data[pos..pos + 4];
            let chunk_size = u32::from_le_bytes([
                data[pos + 4],
                data[pos + 5],
                data[pos + 6],
                data[pos + 7],
            ]) as usize;

            // Calculate chunk end, including padding byte if needed for RIFF alignment
            let data_end = (pos + 8 + chunk_size).min(data.len());
            let needs_padding = chunk_size % 2 != 0 && data_end < data.len();
            let chunk_end = if needs_padding { data_end + 1 } else { data_end };
            let chunk_end = chunk_end.min(data.len());
            
            // Include the padding byte in the chunk data for correct reconstruction
            let chunk_data = &data[pos..chunk_end];

            // Determine chunk type
            let chunk_type = match fourcc {
                b"hdrl" | b"avih" => ChunkType::Metadata,
                b"movi" => ChunkType::VideoStream, // Contains interleaved A/V
                b"idx1" => ChunkType::Metadata,
                _ if fourcc.starts_with(b"00") || fourcc.starts_with(b"01") => {
                    // Video stream chunks
                    ChunkType::VideoStream
                }
                _ if fourcc.starts_with(b"02") || fourcc.starts_with(b"03") => {
                    // Audio stream chunks
                    ChunkType::AudioStream
                }
                _ => ChunkType::Generic,
            };

            let id = Oid::hash(chunk_data);

            chunks.push(ContentChunk {
                id,
                data: chunk_data.to_vec(),
                offset,
                size: chunk_data.len(),
                chunk_type,
                perceptual_hash: None,
            });

            offset += chunk_data.len() as u64;
            pos = chunk_end;
        }

        info!(
            chunks = chunks.len(),
            video_chunks = chunks.iter().filter(|c| c.chunk_type == ChunkType::VideoStream).count(),
            audio_chunks = chunks.iter().filter(|c| c.chunk_type == ChunkType::AudioStream).count(),
            "AVI chunking complete"
        );

        Ok(chunks)
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
                debug!(
                    atom_type = %String::from_utf8_lossy(&atom.atom_type),
                    "Truncated atom, stopping parse"
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
                            });
                            
                            // Emit each nested atom as separate chunk
                            for nested_atom in &nested {
                                let nested_start = 8 + nested_atom.offset as usize;
                                let nested_end = nested_start + nested_atom.size as usize;
                                if nested_end <= atom_data.len() {
                                    let nested_data = &atom_data[nested_start..nested_end];
                                    let nested_type = String::from_utf8_lossy(&nested_atom.atom_type);
                                    chunks.push(ContentChunk {
                                        id: Oid::hash(nested_data),
                                        data: nested_data.to_vec(),
                                        offset: atom.offset + 8 + nested_atom.offset,
                                        size: nested_data.len(),
                                        chunk_type: ChunkType::Metadata,
                                        perceptual_hash: None,
                                    });
                                    debug!(
                                        atom_type = %nested_type,
                                        size = nested_atom.size,
                                        "Parsed nested moov atom"
                                    );
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
                        });
                    }
                    debug!(atom_type = %atom_type_str, size = atom.size, "Parsed moov atom");
                }
                b"mdat" => {
                    // Media data - potentially huge, use CDC sub-chunking for large atoms
                    if atom.size > 4 * 1024 * 1024 {
                        // Large mdat: keep header, sub-chunk data with CDC
                        let header = &atom_data[..atom.header_size as usize];
                        chunks.push(ContentChunk {
                            id: Oid::hash(header),
                            data: header.to_vec(),
                            offset: atom.offset,
                            size: header.len(),
                            chunk_type: ChunkType::Metadata,
                            perceptual_hash: None,
                        });

                        let mdat_content = &atom_data[atom.header_size as usize..];
                        let base_offset = atom.offset + atom.header_size as u64;

                        let sub_chunks = self.chunk_rolling(
                            mdat_content,
                            1 * 1024 * 1024,  // 1MB average
                            512 * 1024,       // 512KB minimum
                            4 * 1024 * 1024,  // 4MB maximum
                        ).await?;

                        // Adjust offsets for sub-chunks
                        for mut sub in sub_chunks {
                            sub.offset += base_offset;
                            sub.chunk_type = ChunkType::VideoStream;
                            chunks.push(sub);
                        }
                        debug!(
                            atom_type = %atom_type_str,
                            size = atom.size,
                            sub_chunks = chunks.len(),
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
                    });
                    debug!(atom_type = %atom_type_str, size = atom.size, "Parsed unknown atom");
                }
            }
        }

        info!(
            chunks = chunks.len(),
            metadata_chunks = chunks.iter().filter(|c| c.chunk_type == ChunkType::Metadata).count(),
            video_chunks = chunks.iter().filter(|c| c.chunk_type == ChunkType::VideoStream).count(),
            total_size = data.len(),
            "MP4 atom-based chunking complete"
        );

        Ok(chunks)
    }

    /// Matroska/WebM chunking
    /// 
    /// Parses EBML elements and creates content-aware chunks:
    /// - Metadata (EBML, Info, Tracks, SeekHead, Cues, etc.) grouped together
    /// - Each Cluster (media data) becomes a separate VideoStream chunk
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
        let mut metadata_start: Option<usize> = None;
        let mut metadata_end: usize = 0;
        
        for element in &elements {
            let elem_start = element.offset as usize;
            let elem_size = if element.data_size == u64::MAX {
                // Unknown size: extends to end of data
                data.len().saturating_sub(elem_start).saturating_sub(element.header_size as usize)
            } else {
                element.header_size as usize + element.data_size as usize
            };
            let elem_end = (elem_start + elem_size).min(data.len());
            
            match element.id {
                CLUSTER_ID => {
                    // Flush accumulated metadata before Cluster
                    if let Some(start) = metadata_start {
                        if metadata_end > start {
                            let meta_data = &data[start..metadata_end];
                            chunks.push(ContentChunk {
                                id: Oid::hash(meta_data),
                                data: meta_data.to_vec(),
                                offset: start as u64,
                                size: meta_data.len(),
                                chunk_type: ChunkType::Metadata,
                                perceptual_hash: None,
                            });
                        }
                        metadata_start = None;
                    }
                    
                    // Create Cluster chunk (VideoStream)
                    if elem_end > elem_start {
                        let cluster_data = &data[elem_start..elem_end];
                        chunks.push(ContentChunk {
                            id: Oid::hash(cluster_data),
                            data: cluster_data.to_vec(),
                            offset: element.offset,
                            size: cluster_data.len(),
                            chunk_type: ChunkType::VideoStream,
                            perceptual_hash: None,
                        });
                    }
                }
                ATTACHMENTS_ID => {
                    // Flush metadata before attachments
                    if let Some(start) = metadata_start {
                        if metadata_end > start {
                            let meta_data = &data[start..metadata_end];
                            chunks.push(ContentChunk {
                                id: Oid::hash(meta_data),
                                data: meta_data.to_vec(),
                                offset: start as u64,
                                size: meta_data.len(),
                                chunk_type: ChunkType::Metadata,
                                perceptual_hash: None,
                            });
                        }
                        metadata_start = None;
                    }
                    
                    // Attachments as separate Generic chunk
                    if elem_end > elem_start {
                        let attach_data = &data[elem_start..elem_end];
                        chunks.push(ContentChunk {
                            id: Oid::hash(attach_data),
                            data: attach_data.to_vec(),
                            offset: element.offset,
                            size: attach_data.len(),
                            chunk_type: ChunkType::Generic,
                            perceptual_hash: None,
                        });
                    }
                }
                EBML_ID | SEGMENT_ID | SEEKHEAD_ID | INFO_ID | TRACKS_ID |
                CUES_ID | CHAPTERS_ID | TAGS_ID => {
                    // Accumulate metadata elements
                    if metadata_start.is_none() {
                        metadata_start = Some(elem_start);
                    }
                    metadata_end = metadata_end.max(elem_end);
                }
                _ => {
                    // Unknown elements: include in metadata grouping
                    if metadata_start.is_none() {
                        metadata_start = Some(elem_start);
                    }
                    metadata_end = metadata_end.max(elem_end);
                }
            }
        }
        
        // Final metadata chunk (for trailing Cues, Tags, etc.)
        if let Some(start) = metadata_start {
            if metadata_end > start {
                let meta_data = &data[start..metadata_end];
                chunks.push(ContentChunk {
                    id: Oid::hash(meta_data),
                    data: meta_data.to_vec(),
                    offset: start as u64,
                    size: meta_data.len(),
                    chunk_type: ChunkType::Metadata,
                    perceptual_hash: None,
                });
            }
        }
        
        // LEVEL 3: Ensure we produced chunks
        if chunks.is_empty() {
            warn!("No chunks created from Matroska parsing, falling back");
            return self.chunk_fixed(data, 4 * 1024 * 1024).await;
        }
        
        info!(
            chunks = chunks.len(),
            metadata = chunks.iter().filter(|c| c.chunk_type == ChunkType::Metadata).count(),
            clusters = chunks.iter().filter(|c| c.chunk_type == ChunkType::VideoStream).count(),
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
        const BIN_CHUNK_TYPE: u32 = 0x004E4942;  // "BIN\0"
        
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
        });
        
        let mut pos = 12;
        
        // Parse JSON and BIN chunks
        while pos + 8 <= data.len() {
            let chunk_length = u32::from_le_bytes([
                data[pos], data[pos + 1], data[pos + 2], data[pos + 3]
            ]) as usize;
            let chunk_type = u32::from_le_bytes([
                data[pos + 4], data[pos + 5], data[pos + 6], data[pos + 7]
            ]);
            
            let chunk_start = pos;
            let chunk_data_start = pos + 8;
            let chunk_end = (chunk_data_start + chunk_length).min(data.len());
            
            let (ct, label) = match chunk_type {
                JSON_CHUNK_TYPE => (ChunkType::Metadata, "JSON"),
                BIN_CHUNK_TYPE => (ChunkType::Generic, "BIN"),
                _ => (ChunkType::Generic, "Unknown"),
            };
            
            // Include chunk header + data as single chunk
            let full_chunk_data = &data[chunk_start..chunk_end];
            chunks.push(ContentChunk {
                id: Oid::hash(full_chunk_data),
                data: full_chunk_data.to_vec(),
                offset: chunk_start as u64,
                size: full_chunk_data.len(),
                chunk_type: ct,
                perceptual_hash: None,
            });
            
            debug!(
                chunk_type = label,
                offset = chunk_start,
                size = full_chunk_data.len(),
                "GLB chunk"
            );
            
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
        if !data.iter().take(1024).all(|&b| b < 128 || b >= 0xC0) {
            // Binary format - use rolling CDC
            let (avg, min, max) = get_chunk_params(data.len() as u64);
            return self.chunk_rolling(data, avg, min, max).await;
        }

        let mut chunks = Vec::new();
        let mut chunk_start = 0;
        let min_chunk_size = 256 * 1024;  // 256KB minimum chunk
        let max_chunk_size = 4 * 1024 * 1024;  // 4MB maximum

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
            return self.chunk_rolling(data, avg, min, max).await;
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
        });

        // For FBX, use adaptive rolling CDC on the rest of the data
        // Full FBX node parsing is complex; CDC provides good dedup
        if data.len() > 27 {
            let content = &data[27..];
            let (avg, min, max) = get_chunk_params(content.len() as u64);
            let sub_chunks = self.chunk_rolling(content, avg, min, max).await?;

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

/// Calculate rolling hash for boundary detection (Rabin fingerprint)
fn rolling_hash(window: &[u8]) -> u64 {
    let mut hash = 0u64;
    for &byte in window {
        hash = hash.wrapping_mul(31).wrapping_add(byte as u64);
    }
    hash
}

/// Parse MP4 atoms from data
/// 
/// Returns a vector of Mp4Atom headers describing the atom structure.
/// Handles standard 4-byte size, extended 8-byte size (size == 1),
/// and size == 0 (extends to end of data).
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
        let is_valid_type = atom_type.iter().all(|&b| {
            (b >= 0x20 && b <= 0x7E) || b == 0x00
        });
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
    let mask = 0xFFu8 >> len;
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
    #[allow(dead_code)]
    chunk_type: ChunkType,
    #[allow(dead_code)]
    first_seen: std::time::SystemTime,
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

        self.chunk_metadata.entry(chunk.id).or_insert_with(|| ChunkMetadata {
            size: chunk.size,
            chunk_type: chunk.chunk_type,
            first_seen: std::time::SystemTime::now(),
        });
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
        };

        let chunk2 = ContentChunk {
            id: Oid::hash(b"test1"), // Same content
            data: b"test1".to_vec(),
            offset: 0,
            size: 5,
            chunk_type: ChunkType::Generic,
            perceptual_hash: None,
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
    async fn test_rolling_hash() {
        let window = b"test data for hashing";
        let hash1 = rolling_hash(window);
        let hash2 = rolling_hash(window);

        assert_eq!(hash1, hash2); // Deterministic

        let different = b"different data here!!";
        let hash3 = rolling_hash(different);

        assert_ne!(hash1, hash3); // Different content
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
        let metadata = chunks.iter().filter(|c| c.chunk_type == ChunkType::Metadata).count();
        let video = chunks.iter().filter(|c| c.chunk_type == ChunkType::VideoStream).count();
        let generic = chunks.iter().filter(|c| c.chunk_type == ChunkType::Generic).count();
        
        println!("Chunk breakdown: Metadata={}, VideoStream={}, Generic={}", metadata, video, generic);
        
        // Verify first chunk is ftyp (Metadata)
        assert_eq!(chunks[0].chunk_type, ChunkType::Metadata);
        assert!(chunks[0].size <= 100, "ftyp should be small");
        
        // Should have at least ftyp + moov parts + mdat parts
        assert!(chunks.len() >= 3, "Expected at least 3 chunks");
        assert!(metadata >= 2, "Expected at least 2 metadata chunks (ftyp, moov header)");
        
        // Verify total size matches file size
        let total_size: usize = chunks.iter().map(|c| c.size).sum();
        assert_eq!(total_size, data.len(), "Chunk sizes should sum to file size");
        
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
        assert_eq!(read_ebml_id(&[0x1A, 0x45, 0xDF, 0xA3], 0), Some((0x1A45DFA3, 4)));
        // 4-byte ID: Segment (0x18538067)
        assert_eq!(read_ebml_id(&[0x18, 0x53, 0x80, 0x67], 0), Some((0x18538067, 4)));
        // 4-byte ID: Cluster (0x1F43B675)
        assert_eq!(read_ebml_id(&[0x1F, 0x43, 0xB6, 0x75], 0), Some((0x1F43B675, 4)));
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
        let chunks = chunker.chunk(&[0x00, 0x01, 0x02, 0x03], "test.mkv").await.unwrap();
        
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
        
        println!("Created {} chunks", chunks.len());
        for (i, chunk) in chunks.iter().enumerate() {
            println!("  Chunk {}: type={:?}, size={}", i, chunk.chunk_type, chunk.size);
        }
        
        // Should have at least 2 chunks: Metadata + VideoStream (Cluster)
        assert!(chunks.len() >= 2, "Expected at least 2 chunks, got {}", chunks.len());
        
        // Should have at least one VideoStream chunk (Cluster)
        let video_count = chunks.iter().filter(|c| c.chunk_type == ChunkType::VideoStream).count();
        assert!(video_count >= 1, "Expected at least 1 VideoStream chunk");
    }
}
