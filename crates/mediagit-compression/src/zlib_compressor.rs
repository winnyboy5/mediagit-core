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

//! Zlib compression implementation for Git compatibility
//!
//! Git uses zlib (deflate) compression for all objects in the object database.
//! This implementation provides Git-compatible compression and decompression.

use crate::error::{CompressionError, CompressionResult};
use crate::{CompressionLevel, Compressor};
use flate2::read::{ZlibDecoder, ZlibEncoder};
use flate2::Compression;
use std::fmt;
use std::io::Read;

/// Zlib compressor implementation
///
/// Uses the zlib (deflate) compression algorithm for Git compatibility.
/// Supports configurable compression levels.
#[derive(Clone)]
pub struct ZlibCompressor {
    level: CompressionLevel,
}

impl ZlibCompressor {
    /// Create a new Zlib compressor with the given compression level
    pub fn new(level: CompressionLevel) -> Self {
        ZlibCompressor { level }
    }

    /// Create a Zlib compressor with fast compression
    pub fn fast() -> Self {
        ZlibCompressor::new(CompressionLevel::Fast)
    }

    /// Create a Zlib compressor with default compression (Git default: level 6)
    pub fn default_level() -> Self {
        ZlibCompressor::new(CompressionLevel::Default)
    }

    /// Create a Zlib compressor with best compression
    pub fn best() -> Self {
        ZlibCompressor::new(CompressionLevel::Best)
    }

    /// Get the flate2 compression level
    fn get_compression(&self) -> Compression {
        match self.level {
            CompressionLevel::Fast => Compression::fast(),      // Level 1
            CompressionLevel::Default => Compression::new(6),    // Git default
            CompressionLevel::Best => Compression::best(),       // Level 9
        }
    }
}

impl fmt::Debug for ZlibCompressor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ZlibCompressor")
            .field("level", &self.level)
            .finish()
    }
}

impl Compressor for ZlibCompressor {
    fn compress(&self, data: &[u8]) -> CompressionResult<Vec<u8>> {
        if data.is_empty() {
            return Ok(Vec::new());
        }

        let mut encoder = ZlibEncoder::new(data, self.get_compression());
        let mut compressed = Vec::new();

        encoder
            .read_to_end(&mut compressed)
            .map_err(|e| CompressionError::zstd_error(format!("zlib compression failed: {}", e)))?;

        Ok(compressed)
    }

    fn decompress(&self, data: &[u8]) -> CompressionResult<Vec<u8>> {
        if data.is_empty() {
            return Ok(Vec::new());
        }

        // Check if this looks like zlib compressed data
        // Zlib header: CMF (0x78) + FLG where (CMF * 256 + FLG) % 31 == 0
        // Valid headers: 0x789C (default), 0x78DA (best), 0x7801 (no compression)
        let is_zlib = data.len() >= 2 && data[0] == 0x78 && {
            let cmf = data[0] as u16;
            let flg = data[1] as u16;
            (cmf * 256 + flg).is_multiple_of(31)
        };

        if is_zlib {
            let mut decoder = ZlibDecoder::new(data);
            let mut decompressed = Vec::new();

            decoder
                .read_to_end(&mut decompressed)
                .map_err(|e| {
                    CompressionError::decompression_failed(format!("zlib decompression failed: {}", e))
                })?;

            Ok(decompressed)
        } else {
            // Data is not zlib compressed, return as-is
            // This handles backward compatibility with uncompressed data
            Ok(data.to_vec())
        }
    }
}


#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn test_zlib_compress_decompress() {
        let compressor = ZlibCompressor::new(CompressionLevel::Default);
        let original = b"Hello, World! This is a test of zlib compression.";

        let compressed = compressor.compress(original).unwrap();
        let decompressed = compressor.decompress(&compressed).unwrap();

        assert_eq!(original, &decompressed[..]);
    }

    #[test]
    fn test_zlib_compress_empty() {
        let compressor = ZlibCompressor::new(CompressionLevel::Default);
        let compressed = compressor.compress(b"").unwrap();
        assert_eq!(compressed.len(), 0);
    }

    #[test]
    fn test_zlib_decompress_empty() {
        let compressor = ZlibCompressor::new(CompressionLevel::Default);
        let decompressed = compressor.decompress(b"").unwrap();
        assert_eq!(decompressed.len(), 0);
    }

    #[test]
    fn test_zlib_decompress_uncompressed_data() {
        let compressor = ZlibCompressor::new(CompressionLevel::Default);
        let data = b"This is not compressed";
        let result = compressor.decompress(data).unwrap();
        // Should return as-is since it doesn't have zlib magic bytes
        assert_eq!(result, data);
    }

    #[test]
    fn test_zlib_compression_levels() {
        let data = b"This is test data that should compress. ".repeat(100);

        let fast = ZlibCompressor::fast().compress(&data).unwrap();
        let default = ZlibCompressor::default_level().compress(&data).unwrap();
        let best = ZlibCompressor::best().compress(&data).unwrap();

        // Best should compress better than default, which should be better than fast
        assert!(best.len() <= default.len());
        assert!(default.len() <= fast.len() + 50); // some tolerance

        // All should decompress correctly
        let decompressor = ZlibCompressor::default_level();
        let fast_dec = decompressor.decompress(&fast).unwrap();
        let default_dec = decompressor.decompress(&default).unwrap();
        let best_dec = decompressor.decompress(&best).unwrap();

        assert_eq!(fast_dec, data);
        assert_eq!(default_dec, data);
        assert_eq!(best_dec, data);
    }

    #[test]
    fn test_zlib_large_data() {
        let compressor = ZlibCompressor::new(CompressionLevel::Default);
        let original = vec![0x42u8; 1024 * 1024]; // 1MB of repeated data

        let compressed = compressor.compress(&original).unwrap();
        let decompressed = compressor.decompress(&compressed).unwrap();

        assert_eq!(original, decompressed);
        // Repeated data should compress very well
        assert!(compressed.len() < original.len() / 100);
    }

    #[test]
    fn test_zlib_random_data() {
        let compressor = ZlibCompressor::new(CompressionLevel::Default);
        // Semi-random data
        let original = (0..1000).map(|i| (i ^ 0xAA) as u8).collect::<Vec<_>>();

        let compressed = compressor.compress(&original).unwrap();
        let decompressed = compressor.decompress(&compressed).unwrap();

        assert_eq!(original, decompressed);
    }

    #[test]
    fn test_zlib_debug_format() {
        let compressor = ZlibCompressor::new(CompressionLevel::Default);
        let debug_str = format!("{:?}", compressor);
        assert!(debug_str.contains("ZlibCompressor"));
        assert!(debug_str.contains("Default"));
    }

    #[test]
    fn test_zlib_git_compatibility() {
        // Git uses zlib compression with level 6 by default
        let compressor = ZlibCompressor::default_level();
        let git_blob = b"blob 13\0Hello, World!";

        let compressed = compressor.compress(git_blob).unwrap();

        // Verify zlib header (0x78 + compression level bits)
        assert_eq!(compressed[0], 0x78);

        // Decompress and verify
        let decompressed = compressor.decompress(&compressed).unwrap();
        assert_eq!(decompressed, git_blob);
    }
}
