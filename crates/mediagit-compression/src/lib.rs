// SPDX-License-Identifier: BUSL-1.1
// Copyright (C) 2025-2026 Aswin Krishnamoorthy

#![allow(missing_docs)]
//! Intelligent compression for MediaGit
//!
//! This crate provides compression abstractions and implementations:
//! - **Smart compression**: Automatic per-object-type strategy selection
//! - **Zstd compression**: Fast, good compression ratios (default)
//! - **Brotli compression**: Higher compression, slower (for archival)
//! - **Auto-detection**: Transparently handle compressed vs uncompressed data
//! - **Compression levels**: Configurable speed vs compression trade-offs
//!
//! # Quick Start
//!
//! ## Basic Compression
//!
//! ```rust,no_run
//! use mediagit_compression::{Compressor, CompressionLevel, ZstdCompressor};
//!
//! fn main() -> anyhow::Result<()> {
//!     let compressor = ZstdCompressor::new(CompressionLevel::Default);
//!
//!     let original = b"Hello, World!";
//!     let compressed = compressor.compress(original)?;
//!     let decompressed = compressor.decompress(&compressed)?;
//!
//!     assert_eq!(original, &decompressed[..]);
//!     println!("Original: {} bytes, Compressed: {} bytes",
//!         original.len(), compressed.len());
//!
//!     Ok(())
//! }
//! ```
//!
//! ## Smart Type-Aware Compression
//!
//! ```rust,no_run
//! use mediagit_compression::{SmartCompressor, TypeAwareCompressor, ObjectType};
//!
//! fn main() -> anyhow::Result<()> {
//!     let compressor = SmartCompressor::new();
//!
//!     // Automatically selects best strategy based on file type
//!     let text_data = b"Some text content...";
//!     let compressed = compressor.compress_typed(text_data, ObjectType::Text)?;
//!
//!     // Already compressed formats stored without recompression (with Store prefix)
//!     let jpeg_data = vec![0xFF, 0xD8, 0xFF, 0xE0];
//!     let stored = compressor.compress_typed(&jpeg_data, ObjectType::Jpeg)?;
//!     assert_eq!(stored[0], 0x00); // Store mode magic byte
//!     assert_eq!(&stored[1..], &jpeg_data[..]); // No recompression overhead
//!
//!     Ok(())
//! }
//! ```
//!
//! # Compression Algorithms
//!
//! ## Zstd (Default)
//! - Fast compression and decompression
//! - Good compression ratios (typical 2-4x for text/structured data)
//! - Low memory usage
//! - Ideal for frequently accessed data
//!
//! ## Brotli
//! - Higher compression ratios (typical 3-8x for text/structured data)
//! - Slower compression but faster decompression than gzip
//! - Good for archival and infrequently accessed data
//! - Higher memory usage during compression
//!
//! # Per-Object Type Strategies
//!
//! `SmartCompressor` automatically selects optimal compression:
//!
//! - **Already compressed** (JPEG, PNG, MP4, ZIP): Store without recompression
//! - **Uncompressed images** (TIFF, BMP, PSD, RAW): Zstd Best compression
//! - **Text/Code** (TXT, JSON, XML, YAML): Brotli Best for maximum text compression
//! - **Documents** (PDF, SVG): Zstd Default for balanced performance
//! - **Unknown/Binary**: Zstd Default as safe fallback

pub mod adaptive;
pub mod brotli_compressor;
pub mod error;
pub mod metrics;
pub mod per_type_compressor;
pub mod process_key;
pub mod smart_compressor;
pub mod zlib_compressor;
pub mod zstd_compressor;

use std::fmt::Debug;

pub use adaptive::{
    AdaptiveCompressor, CompressionStrategy as AdaptiveStrategy, EntropyClass, FileProfile,
    PatternClass, PerformanceStats, SizeClass, calculate_entropy,
};
pub use brotli_compressor::BrotliCompressor;
pub use error::{CompressionError, CompressionResult};
/// The at-rest key type `SmartCompressor::with_key` takes.
///
/// Re-exported because that constructor is public but its parameter type was
/// not reachable from this crate, so no caller outside `mediagit-security`
/// could name it.
pub use mediagit_security::encryption::EncryptionKey;
/// Does `data` carry the MGEN envelope? Re-exported so callers can ask without
/// depending on `mediagit-security` directly.
pub use mediagit_security::envelope::is_sealed;
pub use metrics::CompressionMetrics;
pub use per_type_compressor::{CompressionProfile, PerObjectTypeCompressor, PerTypeStats};
pub use process_key::{
    ProcessKeyError, ensure_key_scope, open_at_rest, process_key, seal_at_rest, set_process_key,
};
pub use smart_compressor::{
    ChunkCodecHint, CompressionStrategy, ObjectCategory, ObjectType, SmartCompressor,
    TypeAwareCompressor,
};
pub use zlib_compressor::ZlibCompressor;
pub use zstd_compressor::ZstdCompressor;

/// Compression level configuration
///
/// Balances compression speed vs compression ratio.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompressionLevel {
    /// Fast compression, larger output (level 1 for zstd, 4 for brotli)
    Fast,
    /// Default balance (level 3 for zstd, 9 for brotli)
    Default,
    /// Best compression, slower (level 19 for zstd, 11 for brotli).
    /// zstd 20-22 ("ultra") need ~1 GB per compression context and OOM under
    /// parallel adds on 16 GB machines, for <0.5% extra ratio on media data
    /// (measured 2026-07-07: deep-test FLAC add hard-failed on the ultra
    /// path). Level 19 is the highest normal level (~128 MB per context).
    Best,
}

impl CompressionLevel {
    /// Convert to zstd compression level (1-19; ultra levels excluded, see `Best`)
    pub fn to_zstd_level(self) -> i32 {
        match self {
            CompressionLevel::Fast => 1,
            CompressionLevel::Default => 3,
            CompressionLevel::Best => 19,
        }
    }

    /// Convert to brotli compression level (0-11)
    pub fn to_brotli_level(self) -> u32 {
        match self {
            CompressionLevel::Fast => 4,
            CompressionLevel::Default => 9,
            CompressionLevel::Best => 11,
        }
    }
}

/// Compression algorithm identifier
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[repr(u8)]
pub enum CompressionAlgorithm {
    /// No compression (raw data)
    None = 0,
    /// Zlib compression (Git-compatible)
    Zlib = 1,
    /// Zstd compression
    Zstd = 2,
    /// Brotli compression
    Brotli = 3,
}

impl CompressionAlgorithm {
    /// Get magic bytes that identify this algorithm
    pub fn magic_bytes(self) -> &'static [u8] {
        match self {
            CompressionAlgorithm::None => b"",
            CompressionAlgorithm::Zlib => b"\x78", // Zlib header
            CompressionAlgorithm::Zstd => b"\x28\xb5\x2f\xfd", // Zstd frame magic
            CompressionAlgorithm::Brotli => b"BRT\x01", // Custom marker for brotli
        }
    }

    /// Detect compression algorithm from data
    pub fn detect(data: &[u8]) -> Self {
        if data.is_empty() {
            return CompressionAlgorithm::None;
        }

        // Check other formats first (they have more reliable magic bytes)
        if data.len() >= 4 {
            // Zstd magic: 0xFD2FB528 (little-endian)
            if data.starts_with(b"\x28\xb5\x2f\xfd") {
                return CompressionAlgorithm::Zstd;
            }
            // Brotli: No standard magic, but we use "BRT\x01" as custom marker
            if data.starts_with(b"BRT\x01") {
                return CompressionAlgorithm::Brotli;
            }
        }

        // Check zlib (Git compatibility) - requires proper header validation
        // Zlib header: CMF (0x78) + FLG where (CMF * 256 + FLG) % 31 == 0
        if data.len() >= 2 && data[0] == 0x78 {
            let cmf = data[0] as u16;
            let flg = data[1] as u16;
            let header_check = cmf * 256 + flg;
            // Valid zlib headers: 0x789C (default), 0x78DA (best), 0x7801 (no compression)
            if header_check.is_multiple_of(31) {
                return CompressionAlgorithm::Zlib;
            }
        }

        // No recognized compression format - treat as uncompressed (Store)
        CompressionAlgorithm::None
    }
}

/// Compressor trait for pluggable compression implementations
///
/// Implementations must support transparent compression and decompression
/// with configurable levels.
pub trait Compressor: Send + Sync + Debug {
    /// Compress data
    ///
    /// # Arguments
    ///
    /// * `data` - Raw data to compress
    ///
    /// # Returns
    ///
    /// Compressed data with algorithm identification prefix
    ///
    /// # Errors
    ///
    /// Returns `CompressionError` if compression fails
    fn compress(&self, data: &[u8]) -> CompressionResult<Vec<u8>>;

    /// Decompress data
    ///
    /// Automatically detects compression algorithm from data prefix
    ///
    /// # Arguments
    ///
    /// * `data` - Compressed data (or raw data without compression)
    ///
    /// # Returns
    ///
    /// Decompressed data
    ///
    /// # Errors
    ///
    /// Returns `CompressionError` if decompression fails
    fn decompress(&self, data: &[u8]) -> CompressionResult<Vec<u8>>;
}

/// Decompress `data` using `spawn_blocking` when the payload exceeds the configured threshold.
///
/// Gated by `MEDIAGIT_DECOMPRESS_BLOCKING` (enabled unless set to `"0"`) and
/// `MEDIAGIT_DECOMPRESS_BLOCKING_THRESHOLD` (default 262144 bytes / 256 KiB).
/// Below the threshold a direct synchronous call avoids the dispatch overhead.
pub async fn decompress_maybe_blocking(
    compressor: std::sync::Arc<dyn Compressor>,
    data: Vec<u8>,
) -> CompressionResult<Vec<u8>> {
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
            .map_err(|e| CompressionError::decompression_failed(e.to_string()))?
    } else {
        compressor.decompress(&data)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compression_level_conversions() {
        assert_eq!(CompressionLevel::Fast.to_zstd_level(), 1);
        assert_eq!(CompressionLevel::Default.to_zstd_level(), 3);
        assert_eq!(CompressionLevel::Best.to_zstd_level(), 19); // ultra (20-22) excluded: OOM class

        assert_eq!(CompressionLevel::Fast.to_brotli_level(), 4);
        assert_eq!(CompressionLevel::Default.to_brotli_level(), 9);
        assert_eq!(CompressionLevel::Best.to_brotli_level(), 11);
    }

    #[test]
    fn algorithm_detection() {
        // Zlib magic bytes
        let zlib_data = b"\x78\x9c\x00\x00\x00\x00";
        assert_eq!(
            CompressionAlgorithm::detect(zlib_data),
            CompressionAlgorithm::Zlib
        );

        // Zstd magic bytes
        let zstd_data = b"\x28\xb5\x2f\xfd\x00\x00\x00\x00";
        assert_eq!(
            CompressionAlgorithm::detect(zstd_data),
            CompressionAlgorithm::Zstd
        );

        // Brotli marker
        let brotli_data = b"BRT\x01some_data";
        assert_eq!(
            CompressionAlgorithm::detect(brotli_data),
            CompressionAlgorithm::Brotli
        );

        // Uncompressed
        let raw_data = b"Hello, World!";
        assert_eq!(
            CompressionAlgorithm::detect(raw_data),
            CompressionAlgorithm::None
        );

        // Empty data
        let empty_data = b"";
        assert_eq!(
            CompressionAlgorithm::detect(empty_data),
            CompressionAlgorithm::None
        );
    }

    #[test]
    fn compression_level_debug() {
        assert_eq!(format!("{:?}", CompressionLevel::Fast), "Fast");
        assert_eq!(format!("{:?}", CompressionLevel::Default), "Default");
        assert_eq!(format!("{:?}", CompressionLevel::Best), "Best");
    }
}
