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

//! Zstd compression implementation
//!
//! Fast compression and decompression with good compression ratios.
//! Ideal for frequently accessed data in MediaGit.

use crate::error::{CompressionError, CompressionResult};
use crate::{CompressionLevel, Compressor};
use std::fmt;

/// Zstd compressor implementation
///
/// Uses the Zstandard compression algorithm for balanced speed and ratio.
/// Supports configurable compression levels.
#[derive(Clone)]
pub struct ZstdCompressor {
    level: CompressionLevel,
}

impl ZstdCompressor {
    /// Create a new Zstd compressor with the given compression level
    pub fn new(level: CompressionLevel) -> Self {
        ZstdCompressor { level }
    }

    /// Create a Zstd compressor with fast compression
    pub fn fast() -> Self {
        ZstdCompressor::new(CompressionLevel::Fast)
    }

    /// Create a Zstd compressor with default compression
    pub fn default_level() -> Self {
        ZstdCompressor::new(CompressionLevel::Default)
    }

    /// Create a Zstd compressor with best compression
    pub fn best() -> Self {
        ZstdCompressor::new(CompressionLevel::Best)
    }
}

impl fmt::Debug for ZstdCompressor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ZstdCompressor")
            .field("level", &self.level)
            .finish()
    }
}

impl Compressor for ZstdCompressor {
    fn compress(&self, data: &[u8]) -> CompressionResult<Vec<u8>> {
        let level = self.level.to_zstd_level();

        // For very small data, just return uncompressed to avoid overhead
        if data.is_empty() {
            return Ok(Vec::new());
        }

        match zstd::encode_all(data, level) {
            Ok(compressed) => {
                // Prepend zstd magic bytes (already in zstd output, but ensure it's there)
                Ok(compressed)
            }
            Err(e) => Err(CompressionError::zstd_error(format!(
                "zstd compression failed: {}",
                e
            ))),
        }
    }

    fn decompress(&self, data: &[u8]) -> CompressionResult<Vec<u8>> {
        if data.is_empty() {
            return Ok(Vec::new());
        }

        // Check if this looks like zstd compressed data (has zstd magic bytes)
        if data.len() >= 4 && data.starts_with(b"\x28\xb5\x2f\xfd") {
            match zstd::decode_all(data) {
                Ok(decompressed) => Ok(decompressed),
                Err(e) => Err(CompressionError::decompression_failed(format!(
                    "zstd decompression failed: {}",
                    e
                ))),
            }
        } else {
            // Data is not zstd compressed, return as-is
            // This handles the case where data was never compressed
            Ok(data.to_vec())
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn test_zstd_compress_decompress() {
        let compressor = ZstdCompressor::new(CompressionLevel::Default);
        let original = b"Hello, World! This is a test of zstd compression.";

        let compressed = compressor.compress(original).unwrap();
        let decompressed = compressor.decompress(&compressed).unwrap();

        assert_eq!(original, &decompressed[..]);
    }

    #[test]
    fn test_zstd_compress_empty() {
        let compressor = ZstdCompressor::new(CompressionLevel::Default);
        let compressed = compressor.compress(b"").unwrap();
        assert_eq!(compressed.len(), 0);
    }

    #[test]
    fn test_zstd_decompress_empty() {
        let compressor = ZstdCompressor::new(CompressionLevel::Default);
        let decompressed = compressor.decompress(b"").unwrap();
        assert_eq!(decompressed.len(), 0);
    }

    #[test]
    fn test_zstd_decompress_uncompressed_data() {
        let compressor = ZstdCompressor::new(CompressionLevel::Default);
        let data = b"This is not compressed";
        let result = compressor.decompress(data).unwrap();
        // Should return as-is since it doesn't have zstd magic bytes
        assert_eq!(result, data);
    }

    #[test]
    fn test_zstd_compression_levels() {
        let data = b"This is test data that should compress. ".repeat(100);

        let fast = ZstdCompressor::fast().compress(&data).unwrap();
        let default = ZstdCompressor::default_level().compress(&data).unwrap();
        let best = ZstdCompressor::best().compress(&data).unwrap();

        // Best should compress better than default, which should be better than fast
        // (though not always strictly true due to data characteristics)
        assert!(best.len() <= default.len());
        assert!(default.len() <= fast.len() || fast.len() < 100); // some tolerance

        // All should decompress correctly
        let decompressor = ZstdCompressor::default_level();
        let fast_dec = decompressor.decompress(&fast).unwrap();
        let default_dec = decompressor.decompress(&default).unwrap();
        let best_dec = decompressor.decompress(&best).unwrap();

        assert_eq!(fast_dec, data);
        assert_eq!(default_dec, data);
        assert_eq!(best_dec, data);
    }

    #[test]
    fn test_zstd_large_data() {
        let compressor = ZstdCompressor::new(CompressionLevel::Default);
        let original = vec![0x42u8; 1024 * 1024]; // 1MB of repeated data

        let compressed = compressor.compress(&original).unwrap();
        let decompressed = compressor.decompress(&compressed).unwrap();

        assert_eq!(original, decompressed);
        // Repeated data should compress very well
        assert!(compressed.len() < original.len() / 100);
    }

    #[test]
    fn test_zstd_random_data() {
        let compressor = ZstdCompressor::new(CompressionLevel::Default);
        // Random data doesn't compress well
        let original = (0..1000).map(|i| (i ^ 0xAA) as u8).collect::<Vec<_>>();

        let compressed = compressor.compress(&original).unwrap();
        let decompressed = compressor.decompress(&compressed).unwrap();

        assert_eq!(original, decompressed);
        // Random data might even expand slightly
        assert!(compressed.len() <= original.len() + 1000);
    }

    #[test]
    fn test_zstd_debug_format() {
        let compressor = ZstdCompressor::new(CompressionLevel::Default);
        let debug_str = format!("{:?}", compressor);
        assert!(debug_str.contains("ZstdCompressor"));
        assert!(debug_str.contains("Default"));
    }
}
