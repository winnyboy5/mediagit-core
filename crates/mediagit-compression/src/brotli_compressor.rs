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

//! Brotli compression implementation
//!
//! High compression ratios with slower compression speed.
//! Ideal for archival and infrequently accessed data.

use crate::error::{CompressionError, CompressionResult};
use crate::{CompressionLevel, Compressor};
use std::fmt;
use std::io::Write;

/// Brotli compressor implementation
///
/// Uses the Brotli compression algorithm for higher compression ratios
/// at the cost of slower compression speed.
#[derive(Clone)]
pub struct BrotliCompressor {
    level: CompressionLevel,
}

impl BrotliCompressor {
    /// Create a new Brotli compressor with the given compression level
    pub fn new(level: CompressionLevel) -> Self {
        BrotliCompressor { level }
    }

    /// Create a Brotli compressor with fast compression
    pub fn fast() -> Self {
        BrotliCompressor::new(CompressionLevel::Fast)
    }

    /// Create a Brotli compressor with default compression
    pub fn default_level() -> Self {
        BrotliCompressor::new(CompressionLevel::Default)
    }

    /// Create a Brotli compressor with best compression
    pub fn best() -> Self {
        BrotliCompressor::new(CompressionLevel::Best)
    }
}

impl fmt::Debug for BrotliCompressor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BrotliCompressor")
            .field("level", &self.level)
            .finish()
    }
}

impl Compressor for BrotliCompressor {
    fn compress(&self, data: &[u8]) -> CompressionResult<Vec<u8>> {
        if data.is_empty() {
            return Ok(Vec::new());
        }

        let level = self.level.to_brotli_level();
        let mut output = Vec::with_capacity(data.len() / 2);

        // Add custom marker prefix to identify brotli compressed data
        output.extend_from_slice(b"BRT\x01");

        // Compress using brotli in a scoped block to drop the writer
        {
            let mut compressor = brotli::CompressorWriter::new(
                &mut output,
                4096, // buffer size
                level,
                22, // window size (larger = better compression but more memory)
            );

            if let Err(e) = compressor.write_all(data) {
                return Err(CompressionError::brotli_error(format!(
                    "brotli compression failed: {}",
                    e
                )));
            }

            if let Err(e) = compressor.flush() {
                return Err(CompressionError::brotli_error(format!(
                    "brotli flush failed: {}",
                    e
                )));
            }
        } // compressor is dropped here, releasing the borrow

        Ok(output)
    }

    fn decompress(&self, data: &[u8]) -> CompressionResult<Vec<u8>> {
        if data.is_empty() {
            return Ok(Vec::new());
        }

        // Check for brotli marker
        if data.len() >= 4 && data.starts_with(b"BRT\x01") {
            // Skip the marker prefix
            let compressed_data = &data[4..];
            let mut output = Vec::with_capacity(data.len() * 2);

            match brotli::BrotliDecompress(
                &mut std::io::Cursor::new(compressed_data),
                &mut output,
            ) {
                Ok(_) => Ok(output),
                Err(e) => Err(CompressionError::decompression_failed(format!(
                    "brotli decompression failed: {}",
                    e
                ))),
            }
        } else {
            // Data is not brotli compressed, return as-is
            Ok(data.to_vec())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_brotli_compress_decompress() {
        let compressor = BrotliCompressor::new(CompressionLevel::Default);
        let original = b"Hello, World! This is a test of brotli compression.";

        let compressed = compressor.compress(original).unwrap();
        let decompressed = compressor.decompress(&compressed).unwrap();

        assert_eq!(original, &decompressed[..]);
    }

    #[test]
    fn test_brotli_has_marker() {
        let compressor = BrotliCompressor::new(CompressionLevel::Default);
        let original = b"Test data for brotli";

        let compressed = compressor.compress(original).unwrap();
        assert!(compressed.starts_with(b"BRT\x01"));
    }

    #[test]
    fn test_brotli_compress_empty() {
        let compressor = BrotliCompressor::new(CompressionLevel::Default);
        let compressed = compressor.compress(b"").unwrap();
        assert_eq!(compressed.len(), 0);
    }

    #[test]
    fn test_brotli_decompress_empty() {
        let compressor = BrotliCompressor::new(CompressionLevel::Default);
        let decompressed = compressor.decompress(b"").unwrap();
        assert_eq!(decompressed.len(), 0);
    }

    #[test]
    fn test_brotli_decompress_uncompressed_data() {
        let compressor = BrotliCompressor::new(CompressionLevel::Default);
        let data = b"This is not brotli compressed";
        let result = compressor.decompress(data).unwrap();
        // Should return as-is since it doesn't have brotli marker
        assert_eq!(result, data);
    }

    #[test]
    fn test_brotli_compression_levels() {
        let data = b"This is test data that should compress. ".repeat(100);

        let fast = BrotliCompressor::fast().compress(&data).unwrap();
        let default = BrotliCompressor::default_level().compress(&data).unwrap();
        let best = BrotliCompressor::best().compress(&data).unwrap();

        // Better compression levels should generally produce smaller output
        // (though this depends on data characteristics)
        assert!(best.len() <= default.len() || best.len() < 100);
        assert!(default.len() <= fast.len() || default.len() < 100);

        // All should decompress correctly
        let decompressor = BrotliCompressor::default_level();
        let fast_dec = decompressor.decompress(&fast).unwrap();
        let default_dec = decompressor.decompress(&default).unwrap();
        let best_dec = decompressor.decompress(&best).unwrap();

        assert_eq!(fast_dec, data);
        assert_eq!(default_dec, data);
        assert_eq!(best_dec, data);
    }

    #[test]
    fn test_brotli_large_data() {
        let compressor = BrotliCompressor::new(CompressionLevel::Default);
        let original = vec![0x42u8; 1024 * 100]; // 100KB of repeated data

        let compressed = compressor.compress(&original).unwrap();
        let decompressed = compressor.decompress(&compressed).unwrap();

        assert_eq!(original, decompressed);
        // Repeated data should compress very well
        assert!(compressed.len() < original.len() / 50);
    }

    #[test]
    fn test_brotli_debug_format() {
        let compressor = BrotliCompressor::new(CompressionLevel::Default);
        let debug_str = format!("{:?}", compressor);
        assert!(debug_str.contains("BrotliCompressor"));
        assert!(debug_str.contains("Default"));
    }
}
