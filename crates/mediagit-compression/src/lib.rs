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

//! Intelligent compression for MediaGit
//!
//! This crate provides compression abstractions and implementations:
//! - **Zstd compression**: Fast, good compression ratios (default)
//! - **Brotli compression**: Higher compression, slower (for archival)
//! - **Auto-detection**: Transparently handle compressed vs uncompressed data
//! - **Compression levels**: Configurable speed vs compression trade-offs
//!
//! # Quick Start
//!
//! ```rust
//! use mediagit_compression::{Compressor, CompressionLevel, ZstdCompressor};
//!
//! #[tokio::main]
//! async fn main() -> anyhow::Result<()> {
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

pub mod brotli_compressor;
pub mod delta;
pub mod error;
pub mod metrics;
pub mod zstd_compressor;

use std::fmt::Debug;

pub use brotli_compressor::BrotliCompressor;
pub use error::{CompressionError, CompressionResult};
pub use metrics::CompressionMetrics;
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
    /// Best compression, slower (level 22 for zstd, 11 for brotli)
    Best,
}

impl CompressionLevel {
    /// Convert to zstd compression level (1-22)
    pub fn to_zstd_level(self) -> i32 {
        match self {
            CompressionLevel::Fast => 1,
            CompressionLevel::Default => 3,
            CompressionLevel::Best => 22,
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
    /// Zstd compression
    Zstd = 1,
    /// Brotli compression
    Brotli = 2,
}

impl CompressionAlgorithm {
    /// Get magic bytes that identify this algorithm
    pub fn magic_bytes(self) -> &'static [u8] {
        match self {
            CompressionAlgorithm::None => b"",
            CompressionAlgorithm::Zstd => b"\x28\xb5\x2f\xfd", // Zstd frame magic
            CompressionAlgorithm::Brotli => b"BRT\x01",         // Custom marker for brotli
        }
    }

    /// Detect compression algorithm from data
    pub fn detect(data: &[u8]) -> Self {
        if data.len() >= 4 {
            if data.starts_with(b"\x28\xb5\x2f\xfd") {
                return CompressionAlgorithm::Zstd;
            }
            if data.starts_with(b"BRT\x01") {
                return CompressionAlgorithm::Brotli;
            }
        }
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

    /// Get compression metrics for data
    fn metrics(&self, original: &[u8], compressed: &[u8]) -> CompressionMetrics {
        CompressionMetrics::from_sizes(original.len(), compressed.len())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compression_level_conversions() {
        assert_eq!(CompressionLevel::Fast.to_zstd_level(), 1);
        assert_eq!(CompressionLevel::Default.to_zstd_level(), 3);
        assert_eq!(CompressionLevel::Best.to_zstd_level(), 22);

        assert_eq!(CompressionLevel::Fast.to_brotli_level(), 4);
        assert_eq!(CompressionLevel::Default.to_brotli_level(), 9);
        assert_eq!(CompressionLevel::Best.to_brotli_level(), 11);
    }

    #[test]
    fn algorithm_detection() {
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
    }

    #[test]
    fn compression_level_debug() {
        assert_eq!(format!("{:?}", CompressionLevel::Fast), "Fast");
        assert_eq!(format!("{:?}", CompressionLevel::Default), "Default");
        assert_eq!(format!("{:?}", CompressionLevel::Best), "Best");
    }
}
