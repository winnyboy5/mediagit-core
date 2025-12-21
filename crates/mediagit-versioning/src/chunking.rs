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
use tracing::{debug, info};

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
            "avi" | "riff" => self.chunk_avi(data).await,
            "mp4" | "mov" | "m4v" | "m4a" => self.chunk_mp4(data).await,
            "mkv" | "webm" => self.chunk_matroska(data).await,
            _ => {
                // Fall back to fixed chunking for unknown types
                debug!(extension = extension, "Unknown media type, using fixed chunking");
                self.chunk_fixed(data, 4 * 1024 * 1024).await // 4MB chunks
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

    /// MP4/ISO BMFF chunking
    async fn chunk_mp4(&self, data: &[u8]) -> Result<Vec<ContentChunk>> {
        // TODO: Implement MP4 atom-based chunking
        // For now, use fixed chunking
        debug!("MP4 chunking not yet implemented, using fixed chunking");
        self.chunk_fixed(data, 4 * 1024 * 1024).await
    }

    /// Matroska/WebM chunking
    async fn chunk_matroska(&self, data: &[u8]) -> Result<Vec<ContentChunk>> {
        // TODO: Implement Matroska EBML-based chunking
        // For now, use fixed chunking
        debug!("Matroska chunking not yet implemented, using fixed chunking");
        self.chunk_fixed(data, 4 * 1024 * 1024).await
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
    chunk_type: ChunkType,
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
}
