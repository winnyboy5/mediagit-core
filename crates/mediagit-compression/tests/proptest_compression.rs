// Copyright (C) 2026  winnyboy5
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU Affero General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU Affero General Public License for more details.
//
// You should have received a copy of the GNU Affero General Public License
// along with this program.  If not, see <https://www.gnu.org/licenses/>.
#![allow(clippy::unwrap_used)]
//! Property-Based Tests for Compression
//!
//! Uses proptest to verify compression properties with random data:
//! - Roundtrip correctness (compress then decompress = original)
//! - Idempotence (compressing compressed data)
//! - Empty data handling
//! - Large data handling

use mediagit_compression::{BrotliCompressor, Compressor, CompressionLevel, ZstdCompressor};
use proptest::prelude::*;

/// Generate random binary data for testing
fn arb_binary_data() -> impl Strategy<Value = Vec<u8>> {
    prop::collection::vec(any::<u8>(), 0..10000)  // Reduced from 50KB to 10KB for memory efficiency
}

/// Generate small binary data
fn arb_small_data() -> impl Strategy<Value = Vec<u8>> {
    prop::collection::vec(any::<u8>(), 0..1000)
}

/// Generate text-like data (compressible)
fn arb_text_data() -> impl Strategy<Value = Vec<u8>> {
    prop::collection::vec(prop::char::range('a', 'z').prop_map(|c| c as u8), 100..10000)
}

// ============================================================================
// Zstd Property Tests
// ============================================================================

#[test]
fn proptest_zstd_roundtrip() {
    proptest!(|(data in arb_binary_data())| {
        let compressor = ZstdCompressor::new(CompressionLevel::Default);

        let compressed = compressor.compress(&data).unwrap();
        let decompressed = compressor.decompress(&compressed).unwrap();

        prop_assert_eq!(data, decompressed);
    });
}

#[test]
fn proptest_zstd_empty_data() {
    proptest!(|(_unit in any::<()>())| {
        let compressor = ZstdCompressor::new(CompressionLevel::Default);

        let compressed = compressor.compress(&[]).unwrap();
        prop_assert_eq!(compressed.len(), 0);

        let decompressed = compressor.decompress(&[]).unwrap();
        prop_assert_eq!(decompressed.len(), 0);
    });
}

#[test]
fn proptest_zstd_determinism() {
    proptest!(|(data in arb_small_data())| {
        let compressor = ZstdCompressor::new(CompressionLevel::Default);

        let compressed1 = compressor.compress(&data).unwrap();
        let compressed2 = compressor.compress(&data).unwrap();

        // Same input should produce same output
        prop_assert_eq!(compressed1, compressed2);
    });
}

#[test]
fn proptest_zstd_idempotence() {
    proptest!(|(data in arb_small_data())| {
        let compressor = ZstdCompressor::new(CompressionLevel::Default);

        let compressed = compressor.compress(&data).unwrap();
        let decompressed = compressor.decompress(&compressed).unwrap();

        // Decompressing compressed data should be identity
        prop_assert_eq!(data.clone(), decompressed);

        // Compressing again should be deterministic
        let compressed_again = compressor.compress(&data).unwrap();
        prop_assert_eq!(compressed, compressed_again);
    });
}

#[test]
fn proptest_zstd_compression_levels() {
    proptest!(|(data in arb_text_data())| {
        let fast = ZstdCompressor::fast();
        let default = ZstdCompressor::default_level();
        let best = ZstdCompressor::best();

        let fast_compressed = fast.compress(&data).unwrap();
        let default_compressed = default.compress(&data).unwrap();
        let best_compressed = best.compress(&data).unwrap();

        // All should decompress correctly
        prop_assert_eq!(data.clone(), fast.decompress(&fast_compressed).unwrap());
        prop_assert_eq!(data.clone(), default.decompress(&default_compressed).unwrap());
        prop_assert_eq!(data.clone(), best.decompress(&best_compressed).unwrap());

        // Better compression should generally produce smaller output for compressible data
        // (allowing some tolerance for very small data)
        if data.len() > 1000 {
            prop_assert!(best_compressed.len() <= default_compressed.len() + 100);
        }
    });
}

#[test]
fn proptest_zstd_uncompressed_passthrough() {
    proptest!(|(data in arb_small_data())| {
        let compressor = ZstdCompressor::new(CompressionLevel::Default);

        // Data without zstd magic bytes should pass through
        if !data.starts_with(b"\x28\xb5\x2f\xfd") {
            let result = compressor.decompress(&data).unwrap();
            prop_assert_eq!(data, result);
        }
    });
}

// ============================================================================
// Brotli Property Tests
// ============================================================================

#[test]
fn proptest_brotli_roundtrip() {
    proptest!(|(data in arb_binary_data())| {
        let compressor = BrotliCompressor::new(CompressionLevel::Default);

        let compressed = compressor.compress(&data).unwrap();
        let decompressed = compressor.decompress(&compressed).unwrap();

        prop_assert_eq!(data, decompressed);
    });
}

#[test]
fn proptest_brotli_empty_data() {
    proptest!(|(_unit in any::<()>())| {
        let compressor = BrotliCompressor::new(CompressionLevel::Default);

        let compressed = compressor.compress(&[]).unwrap();
        prop_assert_eq!(compressed.len(), 0);

        let decompressed = compressor.decompress(&[]).unwrap();
        prop_assert_eq!(decompressed.len(), 0);
    });
}

#[test]
fn proptest_brotli_marker_present() {
    proptest!(|(data in arb_small_data())| {
        prop_assume!(!data.is_empty());

        let compressor = BrotliCompressor::new(CompressionLevel::Default);
        let compressed = compressor.compress(&data).unwrap();

        // Compressed data should have brotli marker
        prop_assert!(compressed.starts_with(b"BRT\x01"));
    });
}

#[test]
fn proptest_brotli_determinism() {
    proptest!(|(data in arb_small_data())| {
        let compressor = BrotliCompressor::new(CompressionLevel::Default);

        let compressed1 = compressor.compress(&data).unwrap();
        let compressed2 = compressor.compress(&data).unwrap();

        // Same input should produce same output
        prop_assert_eq!(compressed1, compressed2);
    });
}

#[test]
fn proptest_brotli_idempotence() {
    proptest!(|(data in arb_small_data())| {
        let compressor = BrotliCompressor::new(CompressionLevel::Default);

        let compressed = compressor.compress(&data).unwrap();
        let decompressed = compressor.decompress(&compressed).unwrap();

        // Decompressing compressed data should be identity
        prop_assert_eq!(data.clone(), decompressed);

        // Compressing again should be deterministic
        let compressed_again = compressor.compress(&data).unwrap();
        prop_assert_eq!(compressed, compressed_again);
    });
}

#[test]
fn proptest_brotli_uncompressed_passthrough() {
    proptest!(|(data in arb_small_data())| {
        let compressor = BrotliCompressor::new(CompressionLevel::Default);

        // Data without brotli marker should pass through
        if !data.starts_with(b"BRT\x01") {
            let result = compressor.decompress(&data).unwrap();
            prop_assert_eq!(data, result);
        }
    });
}

// ============================================================================
// Cross-Compressor Property Tests
// ============================================================================

#[test]
fn proptest_compression_produces_valid_output() {
    proptest!(|(data in arb_binary_data())| {
        let zstd = ZstdCompressor::new(CompressionLevel::Default);
        let brotli = BrotliCompressor::new(CompressionLevel::Default);

        // Both compressors should produce valid output
        let zstd_compressed = zstd.compress(&data).unwrap();
        let brotli_compressed = brotli.compress(&data).unwrap();

        // Both should decompress correctly
        prop_assert_eq!(data.clone(), zstd.decompress(&zstd_compressed).unwrap());
        prop_assert_eq!(data.clone(), brotli.decompress(&brotli_compressed).unwrap());
    });
}

#[test]
fn proptest_compression_size_bounds() {
    proptest!(|(data in arb_binary_data())| {
        prop_assume!(!data.is_empty());

        let compressor = ZstdCompressor::new(CompressionLevel::Default);
        let compressed = compressor.compress(&data).unwrap();

        // Compressed size should be reasonable (not more than 2x original + overhead)
        // This accounts for incompressible random data
        prop_assert!(compressed.len() <= data.len() * 2 + 1000);
    });
}

#[test]
fn proptest_repeated_data_compresses_well() {
    proptest!(|(byte in any::<u8>(), count in 1000usize..10000)| {
        let data = vec![byte; count];

        let compressor = ZstdCompressor::new(CompressionLevel::Default);
        let compressed = compressor.compress(&data).unwrap();

        // Repeated data should compress to much less than 10% of original
        prop_assert!(compressed.len() < data.len() / 10);

        // Verify correctness
        let decompressed = compressor.decompress(&compressed).unwrap();
        prop_assert_eq!(data, decompressed);
    });
}

// ============================================================================
// Edge Case Property Tests
// ============================================================================

#[test]
fn proptest_single_byte_data() {
    proptest!(|(byte in any::<u8>())| {
        let data = vec![byte];
        let compressor = ZstdCompressor::new(CompressionLevel::Default);

        let compressed = compressor.compress(&data).unwrap();
        let decompressed = compressor.decompress(&compressed).unwrap();

        prop_assert_eq!(data, decompressed);
    });
}

#[test]
fn proptest_alternating_bytes() {
    proptest!(|(byte1 in any::<u8>(), byte2 in any::<u8>(), count in 100usize..1000)| {
        let data: Vec<u8> = (0..count)
            .map(|i| if i % 2 == 0 { byte1 } else { byte2 })
            .collect();

        let compressor = ZstdCompressor::new(CompressionLevel::Default);

        let compressed = compressor.compress(&data).unwrap();
        let decompressed = compressor.decompress(&compressed).unwrap();

        prop_assert_eq!(data.clone(), decompressed);

        // Alternating pattern should compress reasonably well
        if count > 200 {
            prop_assert!(compressed.len() < data.len());
        }
    });
}

#[test]
fn proptest_ascii_text() {
    proptest!(|(text in "[a-zA-Z0-9 ,.!?]{100,1000}")| {
        let data = text.as_bytes();
        let compressor = ZstdCompressor::new(CompressionLevel::Default);

        let compressed = compressor.compress(data).unwrap();
        let decompressed = compressor.decompress(&compressed).unwrap();

        prop_assert_eq!(data, &decompressed[..]);

        // ASCII text should compress well for longer strings (>500 chars)
        // For very short strings, compression overhead may exceed savings
        if data.len() > 500 {
            prop_assert!(compressed.len() < data.len());
        }
    });
}
