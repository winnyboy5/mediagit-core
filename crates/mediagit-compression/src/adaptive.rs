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

//! Adaptive compression based on file characteristics
//!
//! This module implements intelligent compression algorithm selection by analyzing:
//! - File size (Tiny <1KB, Small <100KB, Medium <10MB, Large <100MB, Huge >100MB)
//! - Shannon entropy (High >7.5 bits/byte = random/compressed, Low <6.0 = text)
//! - Pattern detection (text, binary, already-compressed, media formats)
//!
//! The adaptive strategy:
//! - High entropy → Store (already compressed or random)
//! - Low entropy small files → Brotli(Best) for maximum compression
//! - Large files → Zstd(Fast) for speed
//! - Text files → Brotli for better ratios
//! - Binary with patterns → Zstd for balanced performance

use crate::{
    BrotliCompressor, Compressor, CompressionAlgorithm, CompressionLevel, CompressionResult,
    ZlibCompressor, ZstdCompressor,
};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// File size classification
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SizeClass {
    /// Tiny files (<1KB) - optimize for compression ratio
    Tiny,
    /// Small files (<100KB) - balance ratio and speed
    Small,
    /// Medium files (<10MB) - prefer speed
    Medium,
    /// Large files (<100MB) - prioritize speed
    Large,
    /// Huge files (>100MB) - fastest compression
    Huge,
}

impl SizeClass {
    /// Classify size in bytes
    pub fn classify(size: usize) -> Self {
        match size {
            0..=1_024 => SizeClass::Tiny,
            1_025..=102_400 => SizeClass::Small,
            102_401..=10_485_760 => SizeClass::Medium,
            10_485_761..=104_857_600 => SizeClass::Large,
            _ => SizeClass::Huge,
        }
    }
}

/// Entropy classification based on Shannon entropy
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EntropyClass {
    /// Very low entropy (<4.0 bits/byte) - highly repetitive
    VeryLow,
    /// Low entropy (<6.0 bits/byte) - typical text
    Low,
    /// Medium entropy (<7.5 bits/byte) - mixed content
    Medium,
    /// High entropy (≥7.5 bits/byte) - random/compressed
    High,
}

impl EntropyClass {
    /// Classify entropy value (0.0 to 8.0 bits per byte)
    pub fn classify(entropy: f64) -> Self {
        match entropy {
            e if e < 4.0 => EntropyClass::VeryLow,
            e if e < 6.0 => EntropyClass::Low,
            e if e < 7.5 => EntropyClass::Medium,
            _ => EntropyClass::High,
        }
    }
}

/// Content pattern classification
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PatternClass {
    /// Plain text (ASCII/UTF-8)
    Text,
    /// Binary data with patterns
    Binary,
    /// Already compressed (ZIP, GZIP, etc.)
    AlreadyCompressed,
    /// Media formats (JPEG, PNG, MP4, etc.)
    Media,
}

impl PatternClass {
    /// Detect pattern from content
    pub fn detect(data: &[u8]) -> Self {
        if data.is_empty() {
            return PatternClass::Binary;
        }

        // Check for common compressed/media format magic bytes
        if Self::is_compressed_format(data) {
            return PatternClass::AlreadyCompressed;
        }
        if Self::is_media_format(data) {
            return PatternClass::Media;
        }

        // Check text ratio
        let text_chars = data
            .iter()
            .take(4096)
            .filter(|&&b| {
                b == b'\n'
                    || b == b'\r'
                    || b == b'\t'
                    || (b >= 0x20 && b < 0x7F)
                    || b >= 0x80
            })
            .count();

        let sample_size = data.len().min(4096);
        let text_ratio = text_chars as f64 / sample_size as f64;

        if text_ratio > 0.85 {
            PatternClass::Text
        } else {
            PatternClass::Binary
        }
    }

    fn is_compressed_format(data: &[u8]) -> bool {
        if data.len() < 4 {
            return false;
        }

        // Check common compression signatures
        matches!(
            &data[0..4],
            [0x1f, 0x8b, _, _]         // GZIP
            | [0x50, 0x4b, 0x03, 0x04] // ZIP
            | [0x50, 0x4b, 0x05, 0x06] // ZIP empty
            | [0x50, 0x4b, 0x07, 0x08] // ZIP spanned
            | [0x42, 0x5a, 0x68, _]    // BZIP2
            | [0xfd, 0x37, 0x7a, 0x58] // XZ
            | [0x28, 0xb5, 0x2f, 0xfd] // Zstd
        ) || (data.len() >= 2 && data[0] == 0x78) // Zlib
    }

    fn is_media_format(data: &[u8]) -> bool {
        if data.len() < 4 {
            return false;
        }

        // Check common media format signatures
        matches!(
            &data[0..4],
            [0xff, 0xd8, 0xff, _]      // JPEG
            | [0x89, 0x50, 0x4e, 0x47] // PNG
            | [0x47, 0x49, 0x46, 0x38] // GIF
            | [0x42, 0x4d, _, _]       // BMP
            | [0x66, 0x74, 0x79, 0x70] // MP4/MOV (ftyp)
            | [0x52, 0x49, 0x46, 0x46] // RIFF (WAV/AVI/WebP)
        ) || (data.len() >= 12 && &data[4..12] == b"ftypMSNV") // MP4
            || (data.len() >= 12 && &data[4..12] == b"ftypisom") // MP4
    }
}

/// File profile for compression strategy selection
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FileProfile {
    /// Size classification
    pub size: SizeClass,
    /// Entropy classification
    pub entropy: EntropyClass,
    /// Content pattern classification
    pub pattern: PatternClass,
}

impl FileProfile {
    /// Create profile from data
    pub fn analyze(data: &[u8]) -> Self {
        let size = SizeClass::classify(data.len());
        let entropy_value = calculate_entropy(data);
        let entropy = EntropyClass::classify(entropy_value);
        let pattern = PatternClass::detect(data);

        FileProfile {
            size,
            entropy,
            pattern,
        }
    }
}

/// Calculate Shannon entropy of data (bits per byte)
///
/// Shannon entropy H = -Σ(p(x) * log2(p(x)))
/// where p(x) is probability of byte value x
///
/// Returns value between 0.0 (all same byte) and 8.0 (perfectly random)
pub fn calculate_entropy(data: &[u8]) -> f64 {
    if data.is_empty() {
        return 0.0;
    }

    // Sample first 4KB for efficiency
    let sample = &data[..data.len().min(4096)];

    // Count byte frequencies
    let mut frequencies = [0u32; 256];
    for &byte in sample {
        frequencies[byte as usize] += 1;
    }

    // Calculate entropy
    let len = sample.len() as f64;
    let mut entropy = 0.0;

    for &count in &frequencies {
        if count > 0 {
            let probability = count as f64 / len;
            entropy -= probability * probability.log2();
        }
    }

    entropy
}

/// Compression strategy selection
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CompressionStrategy {
    /// Selected algorithm
    pub algorithm: CompressionAlgorithm,
    /// Selected level
    pub level: CompressionLevel,
}

impl CompressionStrategy {
    /// Select strategy based on file profile
    pub fn select(profile: FileProfile) -> Self {
        use CompressionAlgorithm::*;
        use CompressionLevel::*;

        // Already compressed or media → Store (no compression)
        if matches!(
            profile.pattern,
            PatternClass::AlreadyCompressed | PatternClass::Media
        ) {
            return CompressionStrategy {
                algorithm: None,
                level: Fast,
            };
        }

        // High entropy → likely compressed/random → Store
        if profile.entropy == EntropyClass::High {
            return CompressionStrategy {
                algorithm: None,
                level: Fast,
            };
        }

        match (profile.size, profile.entropy, profile.pattern) {
            // Tiny files with low entropy → Brotli(Best) for max compression
            (SizeClass::Tiny, EntropyClass::VeryLow | EntropyClass::Low, _) => {
                CompressionStrategy {
                    algorithm: Brotli,
                    level: Best,
                }
            }

            // Small text files → Brotli(Best) for excellent ratios
            (SizeClass::Small, EntropyClass::VeryLow | EntropyClass::Low, PatternClass::Text) => {
                CompressionStrategy {
                    algorithm: Brotli,
                    level: Best,
                }
            }

            // Small binary with low entropy → Brotli(Default)
            (
                SizeClass::Small,
                EntropyClass::VeryLow | EntropyClass::Low,
                PatternClass::Binary,
            ) => CompressionStrategy {
                algorithm: Brotli,
                level: Default,
            },

            // Medium files with very low entropy → Brotli for compression
            (SizeClass::Medium, EntropyClass::VeryLow, _) => CompressionStrategy {
                algorithm: Brotli,
                level: Default,
            },

            // Medium files with low entropy → Zstd(Default)
            (SizeClass::Medium, EntropyClass::Low, _) => CompressionStrategy {
                algorithm: Zstd,
                level: Default,
            },

            // Large/Huge files → Zstd(Fast) for speed
            (SizeClass::Large | SizeClass::Huge, _, _) => CompressionStrategy {
                algorithm: Zstd,
                level: Fast,
            },

            // Default: Zstd(Default) for balanced performance
            _ => CompressionStrategy {
                algorithm: Zstd,
                level: Default,
            },
        }
    }
}

/// LRU cache entry
#[derive(Debug, Clone)]
struct CacheEntry {
    strategy: CompressionStrategy,
    hits: usize,
}

/// LRU cache for strategy memoization
#[derive(Debug)]
struct StrategyCache {
    cache: HashMap<FileProfile, CacheEntry>,
    max_size: usize,
}

impl StrategyCache {
    fn new(max_size: usize) -> Self {
        StrategyCache {
            cache: HashMap::new(),
            max_size,
        }
    }

    fn get(&mut self, profile: &FileProfile) -> Option<CompressionStrategy> {
        if let Some(entry) = self.cache.get_mut(profile) {
            entry.hits += 1;
            Some(entry.strategy)
        } else {
            None
        }
    }

    fn insert(&mut self, profile: FileProfile, strategy: CompressionStrategy) {
        // Evict least used if at capacity
        if self.cache.len() >= self.max_size {
            if let Some((&least_used, _)) =
                self.cache.iter().min_by_key(|(_, entry)| entry.hits)
            {
                self.cache.remove(&least_used);
            }
        }

        self.cache.insert(
            profile,
            CacheEntry {
                strategy,
                hits: 1,
            },
        );
    }
}

/// Performance statistics
#[derive(Debug, Clone, Default)]
pub struct PerformanceStats {
    /// Total compressions performed
    pub total_compressions: usize,
    /// Total bytes processed
    pub total_bytes_processed: usize,
    /// Total bytes compressed output
    pub total_bytes_compressed: usize,
    /// Cache hits
    pub cache_hits: usize,
    /// Cache misses
    pub cache_misses: usize,
}

impl PerformanceStats {
    /// Calculate compression ratio
    pub fn compression_ratio(&self) -> f64 {
        if self.total_bytes_processed == 0 {
            return 1.0;
        }
        self.total_bytes_compressed as f64 / self.total_bytes_processed as f64
    }

    /// Calculate cache hit rate
    pub fn cache_hit_rate(&self) -> f64 {
        let total = self.cache_hits + self.cache_misses;
        if total == 0 {
            return 0.0;
        }
        self.cache_hits as f64 / total as f64
    }
}

/// Adaptive compressor with intelligent algorithm selection
#[derive(Debug)]
pub struct AdaptiveCompressor {
    zstd: ZstdCompressor,
    brotli: BrotliCompressor,
    zlib: ZlibCompressor,
    cache: Arc<Mutex<StrategyCache>>,
    stats: Arc<Mutex<PerformanceStats>>,
}

impl AdaptiveCompressor {
    /// Create new adaptive compressor with default cache size
    pub fn new() -> Self {
        Self::with_cache_size(1000)
    }

    /// Create with specific cache size
    pub fn with_cache_size(cache_size: usize) -> Self {
        AdaptiveCompressor {
            zstd: ZstdCompressor::new(CompressionLevel::Default),
            brotli: BrotliCompressor::new(CompressionLevel::Default),
            zlib: ZlibCompressor::new(CompressionLevel::Default),
            cache: Arc::new(Mutex::new(StrategyCache::new(cache_size))),
            stats: Arc::new(Mutex::new(PerformanceStats::default())),
        }
    }

    /// Select compression strategy for data
    pub fn select_strategy(&self, data: &[u8]) -> CompressionStrategy {
        let profile = FileProfile::analyze(data);

        // Check cache first
        if let Ok(mut cache) = self.cache.lock() {
            if let Some(strategy) = cache.get(&profile) {
                if let Ok(mut stats) = self.stats.lock() {
                    stats.cache_hits += 1;
                }
                return strategy;
            }
        }

        // Compute strategy
        let strategy = CompressionStrategy::select(profile);

        // Update cache
        if let Ok(mut cache) = self.cache.lock() {
            cache.insert(profile, strategy);
        }
        if let Ok(mut stats) = self.stats.lock() {
            stats.cache_misses += 1;
        }

        strategy
    }

    /// Get performance statistics
    pub fn stats(&self) -> PerformanceStats {
        self.stats.lock().ok().map(|s| s.clone()).unwrap_or_default()
    }

    /// Reset statistics
    pub fn reset_stats(&self) {
        if let Ok(mut stats) = self.stats.lock() {
            *stats = PerformanceStats::default();
        }
    }
}

impl Default for AdaptiveCompressor {
    fn default() -> Self {
        Self::new()
    }
}

impl Compressor for AdaptiveCompressor {
    fn compress(&self, data: &[u8]) -> CompressionResult<Vec<u8>> {
        let strategy = self.select_strategy(data);

        let result = match strategy.algorithm {
            CompressionAlgorithm::None => Ok(data.to_vec()),
            CompressionAlgorithm::Zlib => {
                let compressor = ZlibCompressor::new(strategy.level);
                compressor.compress(data)
            }
            CompressionAlgorithm::Zstd => {
                let compressor = ZstdCompressor::new(strategy.level);
                compressor.compress(data)
            }
            CompressionAlgorithm::Brotli => {
                let compressor = BrotliCompressor::new(strategy.level);
                compressor.compress(data)
            }
        };

        // Update stats
        if let Ok(compressed) = &result {
            if let Ok(mut stats) = self.stats.lock() {
                stats.total_compressions += 1;
                stats.total_bytes_processed += data.len();
                stats.total_bytes_compressed += compressed.len();
            }
        }

        result
    }

    fn decompress(&self, data: &[u8]) -> CompressionResult<Vec<u8>> {
        // Auto-detect algorithm and delegate
        let algorithm = CompressionAlgorithm::detect(data);

        match algorithm {
            CompressionAlgorithm::None => Ok(data.to_vec()),
            CompressionAlgorithm::Zlib => self.zlib.decompress(data),
            CompressionAlgorithm::Zstd => self.zstd.decompress(data),
            CompressionAlgorithm::Brotli => self.brotli.decompress(data),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_size_classification() {
        assert_eq!(SizeClass::classify(500), SizeClass::Tiny);
        assert_eq!(SizeClass::classify(1024), SizeClass::Tiny);
        assert_eq!(SizeClass::classify(1025), SizeClass::Small);
        assert_eq!(SizeClass::classify(50_000), SizeClass::Small);
        assert_eq!(SizeClass::classify(102_400), SizeClass::Small);
        assert_eq!(SizeClass::classify(102_401), SizeClass::Medium);
        assert_eq!(SizeClass::classify(5_000_000), SizeClass::Medium);
        assert_eq!(SizeClass::classify(10_485_760), SizeClass::Medium);
        assert_eq!(SizeClass::classify(10_485_761), SizeClass::Large);
        assert_eq!(SizeClass::classify(50_000_000), SizeClass::Large);
        assert_eq!(SizeClass::classify(104_857_600), SizeClass::Large);
        assert_eq!(SizeClass::classify(104_857_601), SizeClass::Huge);
    }

    #[test]
    fn test_entropy_classification() {
        assert_eq!(EntropyClass::classify(3.5), EntropyClass::VeryLow);
        assert_eq!(EntropyClass::classify(3.99), EntropyClass::VeryLow);
        assert_eq!(EntropyClass::classify(4.0), EntropyClass::Low);
        assert_eq!(EntropyClass::classify(5.5), EntropyClass::Low);
        assert_eq!(EntropyClass::classify(5.99), EntropyClass::Low);
        assert_eq!(EntropyClass::classify(6.0), EntropyClass::Medium);
        assert_eq!(EntropyClass::classify(7.0), EntropyClass::Medium);
        assert_eq!(EntropyClass::classify(7.49), EntropyClass::Medium);
        assert_eq!(EntropyClass::classify(7.5), EntropyClass::High);
        assert_eq!(EntropyClass::classify(8.0), EntropyClass::High);
    }

    #[test]
    fn test_entropy_calculation() {
        // All same byte → entropy = 0
        let uniform = vec![0u8; 1000];
        let entropy = calculate_entropy(&uniform);
        assert!(entropy < 0.01);

        // Random bytes → entropy ≈ 8
        let random: Vec<u8> = (0..1000).map(|i| (i % 256) as u8).collect();
        let entropy = calculate_entropy(&random);
        assert!(entropy > 5.0); // Should be high but not necessarily 8.0

        // Text → entropy 4-6
        let text = b"Hello, World! This is a test of Shannon entropy calculation.";
        let entropy = calculate_entropy(text);
        assert!(entropy > 3.0 && entropy < 6.0);
    }

    #[test]
    fn test_pattern_detection_text() {
        let text = b"This is plain text content with newlines\nand tabs\t.";
        assert_eq!(PatternClass::detect(text), PatternClass::Text);
    }

    #[test]
    fn test_pattern_detection_compressed() {
        let gzip = b"\x1f\x8b\x08\x00\x00\x00\x00\x00";
        assert_eq!(
            PatternClass::detect(gzip),
            PatternClass::AlreadyCompressed
        );

        let zip = b"\x50\x4b\x03\x04\x00\x00\x00\x00";
        assert_eq!(
            PatternClass::detect(zip),
            PatternClass::AlreadyCompressed
        );

        let zstd = b"\x28\xb5\x2f\xfd\x00\x00\x00\x00";
        assert_eq!(
            PatternClass::detect(zstd),
            PatternClass::AlreadyCompressed
        );
    }

    #[test]
    fn test_pattern_detection_media() {
        let jpeg = b"\xff\xd8\xff\xe0\x00\x10JFIF";
        assert_eq!(PatternClass::detect(jpeg), PatternClass::Media);

        let png = b"\x89\x50\x4e\x47\x0d\x0a\x1a\x0a";
        assert_eq!(PatternClass::detect(png), PatternClass::Media);

        let gif = b"\x47\x49\x46\x38\x39\x61";
        assert_eq!(PatternClass::detect(gif), PatternClass::Media);
    }

    #[test]
    fn test_strategy_selection_high_entropy() {
        // High entropy → Store
        let profile = FileProfile {
            size: SizeClass::Medium,
            entropy: EntropyClass::High,
            pattern: PatternClass::Binary,
        };
        let strategy = CompressionStrategy::select(profile);
        assert_eq!(strategy.algorithm, CompressionAlgorithm::None);
    }

    #[test]
    fn test_strategy_selection_compressed() {
        // Already compressed → Store
        let profile = FileProfile {
            size: SizeClass::Small,
            entropy: EntropyClass::Low,
            pattern: PatternClass::AlreadyCompressed,
        };
        let strategy = CompressionStrategy::select(profile);
        assert_eq!(strategy.algorithm, CompressionAlgorithm::None);
    }

    #[test]
    fn test_strategy_selection_tiny_text() {
        // Tiny text with low entropy → Brotli(Best)
        let profile = FileProfile {
            size: SizeClass::Tiny,
            entropy: EntropyClass::Low,
            pattern: PatternClass::Text,
        };
        let strategy = CompressionStrategy::select(profile);
        assert_eq!(strategy.algorithm, CompressionAlgorithm::Brotli);
        assert_eq!(strategy.level, CompressionLevel::Best);
    }

    #[test]
    fn test_strategy_selection_large_file() {
        // Large file → Zstd(Fast)
        let profile = FileProfile {
            size: SizeClass::Large,
            entropy: EntropyClass::Low,
            pattern: PatternClass::Binary,
        };
        let strategy = CompressionStrategy::select(profile);
        assert_eq!(strategy.algorithm, CompressionAlgorithm::Zstd);
        assert_eq!(strategy.level, CompressionLevel::Fast);
    }

    #[test]
    fn test_file_profile_analysis() {
        // Text file
        let text = b"Hello, World! This is a test.";
        let profile = FileProfile::analyze(text);
        assert_eq!(profile.size, SizeClass::Tiny);
        assert_eq!(profile.pattern, PatternClass::Text);

        // JPEG file
        let jpeg = b"\xff\xd8\xff\xe0\x00\x10JFIF\x00";
        let profile = FileProfile::analyze(jpeg);
        assert_eq!(profile.pattern, PatternClass::Media);
    }

    #[test]
    fn test_adaptive_compressor_text() {
        let compressor = AdaptiveCompressor::new();
        let text = b"Hello, World! ".repeat(100); // ~1.4KB

        let compressed = compressor.compress(&text).unwrap();
        let decompressed = compressor.decompress(&compressed).unwrap();

        assert_eq!(&text[..], &decompressed[..]);
        assert!(compressed.len() < text.len());
    }

    #[test]
    fn test_adaptive_compressor_random() {
        let compressor = AdaptiveCompressor::new();
        // High entropy data (pseudo-random)
        let random: Vec<u8> = (0..1000).map(|i| ((i * 7 + 13) % 256) as u8).collect();

        let compressed = compressor.compress(&random).unwrap();
        let decompressed = compressor.decompress(&compressed).unwrap();

        assert_eq!(&random[..], &decompressed[..]);
        // High entropy may not compress well
    }

    #[test]
    fn test_adaptive_compressor_cache() {
        let compressor = AdaptiveCompressor::new();
        let text = b"Test data";

        // First compression → cache miss
        compressor.compress(text).unwrap();
        let stats1 = compressor.stats();
        assert_eq!(stats1.cache_misses, 1);
        assert_eq!(stats1.cache_hits, 0);

        // Second compression of similar profile → cache hit
        compressor.compress(text).unwrap();
        let stats2 = compressor.stats();
        assert_eq!(stats2.cache_hits, 1);
    }

    #[test]
    fn test_performance_stats() {
        let compressor = AdaptiveCompressor::new();
        let data1 = b"Hello, World!".repeat(10);
        let data2 = b"Test data".repeat(20);

        compressor.compress(&data1).unwrap();
        compressor.compress(&data2).unwrap();

        let stats = compressor.stats();
        assert_eq!(stats.total_compressions, 2);
        assert!(stats.total_bytes_processed > 0);
        assert!(stats.total_bytes_compressed > 0);
        assert!(stats.compression_ratio() < 1.0); // Should compress
    }

    #[test]
    fn test_strategy_cache_lru_eviction() {
        let mut cache = StrategyCache::new(2);

        let profile1 = FileProfile {
            size: SizeClass::Tiny,
            entropy: EntropyClass::Low,
            pattern: PatternClass::Text,
        };
        let profile2 = FileProfile {
            size: SizeClass::Small,
            entropy: EntropyClass::Medium,
            pattern: PatternClass::Binary,
        };
        let profile3 = FileProfile {
            size: SizeClass::Large,
            entropy: EntropyClass::High,
            pattern: PatternClass::Binary,
        };

        let strategy = CompressionStrategy {
            algorithm: CompressionAlgorithm::Zstd,
            level: CompressionLevel::Default,
        };

        // Insert two entries
        cache.insert(profile1, strategy);
        cache.insert(profile2, strategy);
        assert_eq!(cache.cache.len(), 2);

        // Access profile1 to increase its hit count
        cache.get(&profile1);

        // Insert third entry → should evict profile2 (least used)
        cache.insert(profile3, strategy);
        assert_eq!(cache.cache.len(), 2);
        assert!(cache.cache.contains_key(&profile1));
        assert!(cache.cache.contains_key(&profile3));
        assert!(!cache.cache.contains_key(&profile2));
    }
}
