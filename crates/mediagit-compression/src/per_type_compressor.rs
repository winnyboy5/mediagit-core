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

//! Per-object-type compression with configurable profiles
//!
//! This module provides a compressor that maintains separate compression
//! strategies for each object type, with configurable profiles for different
//! use cases (speed, balanced, maximum compression).

use crate::{
    BrotliCompressor, CompressionResult, Compressor,
    ZlibCompressor, ZstdCompressor,
};
use crate::smart_compressor::{CompressionStrategy, ObjectCategory, ObjectType, TypeAwareCompressor};
use crate::metrics::{CompressionMetrics, CompressionAlgorithm as MetricsAlgorithm, CompressionLevel as MetricsLevel};
use crate::{CompressionAlgorithm, CompressionLevel};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Instant;

/// Compression profile for different optimization goals
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CompressionProfile {
    /// Speed-optimized: Fast compression, lower ratios
    Speed,
    /// Balanced: Good compression with acceptable speed
    Balanced,
    /// Maximum compression: Best ratios, slower
    MaxCompression,
    /// Custom: User-defined strategy mappings
    Custom,
}

impl CompressionProfile {
    /// Get compression strategy for object type based on profile
    pub fn strategy_for_type(&self, obj_type: ObjectType) -> CompressionStrategy {
        match self {
            CompressionProfile::Speed => Self::speed_strategy(obj_type),
            CompressionProfile::Balanced => CompressionStrategy::for_object_type(obj_type),
            CompressionProfile::MaxCompression => Self::max_compression_strategy(obj_type),
            CompressionProfile::Custom => CompressionStrategy::for_object_type(obj_type),
        }
    }

    /// Speed-optimized strategy
    fn speed_strategy(obj_type: ObjectType) -> CompressionStrategy {
        if obj_type.is_already_compressed() {
            return CompressionStrategy::Store;
        }

        match obj_type.category() {
            ObjectCategory::Image | ObjectCategory::Audio => {
                CompressionStrategy::Zstd(CompressionLevel::Fast)
            }
            ObjectCategory::Text | ObjectCategory::Document => {
                CompressionStrategy::Zstd(CompressionLevel::Fast)
            }
            ObjectCategory::GitObject => {
                CompressionStrategy::Zlib(CompressionLevel::Fast)
            }
            ObjectCategory::Archive if obj_type == ObjectType::Tar => {
                CompressionStrategy::Zstd(CompressionLevel::Fast)
            }
            _ => CompressionStrategy::Zstd(CompressionLevel::Fast),
        }
    }

    /// Maximum compression strategy
    fn max_compression_strategy(obj_type: ObjectType) -> CompressionStrategy {
        if obj_type.is_already_compressed() {
            return CompressionStrategy::Store;
        }

        match obj_type.category() {
            ObjectCategory::Image | ObjectCategory::Audio => {
                CompressionStrategy::Zstd(CompressionLevel::Best)
            }
            ObjectCategory::Text => {
                CompressionStrategy::Brotli(CompressionLevel::Best)
            }
            ObjectCategory::Document => {
                CompressionStrategy::Brotli(CompressionLevel::Best)
            }
            ObjectCategory::GitObject => {
                CompressionStrategy::Zlib(CompressionLevel::Best)
            }
            ObjectCategory::Archive if obj_type == ObjectType::Tar => {
                CompressionStrategy::Zstd(CompressionLevel::Best)
            }
            _ => CompressionStrategy::Zstd(CompressionLevel::Best),
        }
    }
}

/// Per-type compression statistics
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PerTypeStats {
    /// Total compressions for this type
    pub compressions: usize,
    /// Total bytes processed
    pub bytes_processed: usize,
    /// Total bytes compressed
    pub bytes_compressed: usize,
    /// Total compression time (milliseconds)
    pub total_time_ms: u64,
    /// Average compression ratio
    pub avg_ratio: f64,
}

impl PerTypeStats {
    /// Record a compression operation
    pub fn record(&mut self, metrics: &CompressionMetrics) {
        self.compressions += 1;
        self.bytes_processed += metrics.original_size;
        self.bytes_compressed += metrics.compressed_size;
        self.total_time_ms += metrics.compression_time.as_millis() as u64;

        // Update rolling average
        let weight = 1.0 / self.compressions as f64;
        self.avg_ratio = self.avg_ratio * (1.0 - weight) + metrics.compression_ratio * weight;
    }

    /// Get compression ratio
    pub fn compression_ratio(&self) -> f64 {
        if self.bytes_processed == 0 {
            1.0
        } else {
            self.bytes_compressed as f64 / self.bytes_processed as f64
        }
    }

    /// Get average throughput (MB/s)
    pub fn avg_throughput_mbps(&self) -> f64 {
        if self.total_time_ms == 0 {
            0.0
        } else {
            let mb = self.bytes_processed as f64 / 1_048_576.0;
            let seconds = self.total_time_ms as f64 / 1000.0;
            mb / seconds
        }
    }
}

/// Per-object-type compressor with configurable profiles
#[derive(Debug)]
pub struct PerObjectTypeCompressor {
    /// Compression profile
    profile: CompressionProfile,
    /// Custom strategy overrides (used when profile is Custom)
    custom_strategies: HashMap<ObjectType, CompressionStrategy>,
    /// Per-type statistics
    stats: Arc<Mutex<HashMap<ObjectType, PerTypeStats>>>,
    /// Compressor instances
    zlib: ZlibCompressor,
    zstd_fast: ZstdCompressor,
    zstd_default: ZstdCompressor,
    zstd_best: ZstdCompressor,
    brotli_fast: BrotliCompressor,
    brotli_default: BrotliCompressor,
    brotli_best: BrotliCompressor,
}

impl PerObjectTypeCompressor {
    /// Create new compressor with default balanced profile
    pub fn new() -> Self {
        Self::with_profile(CompressionProfile::Balanced)
    }

    /// Create compressor with specific profile
    pub fn with_profile(profile: CompressionProfile) -> Self {
        PerObjectTypeCompressor {
            profile,
            custom_strategies: HashMap::new(),
            stats: Arc::new(Mutex::new(HashMap::new())),
            zlib: ZlibCompressor::new(CompressionLevel::Default),
            zstd_fast: ZstdCompressor::new(CompressionLevel::Fast),
            zstd_default: ZstdCompressor::new(CompressionLevel::Default),
            zstd_best: ZstdCompressor::new(CompressionLevel::Best),
            brotli_fast: BrotliCompressor::new(CompressionLevel::Fast),
            brotli_default: BrotliCompressor::new(CompressionLevel::Default),
            brotli_best: BrotliCompressor::new(CompressionLevel::Best),
        }
    }

    /// Set custom strategy for a specific object type
    pub fn set_strategy(&mut self, obj_type: ObjectType, strategy: CompressionStrategy) {
        self.custom_strategies.insert(obj_type, strategy);
        self.profile = CompressionProfile::Custom;
    }

    /// Get compression strategy for object type
    pub fn strategy_for_type(&self, obj_type: ObjectType) -> CompressionStrategy {
        // Check custom strategies first
        if let Some(&strategy) = self.custom_strategies.get(&obj_type) {
            return strategy;
        }
        self.profile.strategy_for_type(obj_type)
    }

    /// Compress with specific strategy
    fn compress_with_strategy(
        &self,
        data: &[u8],
        strategy: CompressionStrategy,
    ) -> CompressionResult<Vec<u8>> {
        match strategy {
            CompressionStrategy::Store => Ok(data.to_vec()),
            CompressionStrategy::Zlib(level) => {
                let compressor = ZlibCompressor::new(level);
                compressor.compress(data)
            }
            CompressionStrategy::Zstd(level) => {
                let compressor = match level {
                    CompressionLevel::Fast => &self.zstd_fast,
                    CompressionLevel::Default => &self.zstd_default,
                    CompressionLevel::Best => &self.zstd_best,
                };
                compressor.compress(data)
            }
            CompressionStrategy::Brotli(level) => {
                let compressor = match level {
                    CompressionLevel::Fast => &self.brotli_fast,
                    CompressionLevel::Default => &self.brotli_default,
                    CompressionLevel::Best => &self.brotli_best,
                };
                compressor.compress(data)
            }
            CompressionStrategy::Delta => {
                // Delta requires base - fall back to Zstd
                self.zstd_default.compress(data)
            }
        }
    }

    /// Get statistics for a specific object type
    pub fn stats_for_type(&self, obj_type: ObjectType) -> Option<PerTypeStats> {
        self.stats.lock().unwrap().get(&obj_type).cloned()
    }

    /// Get statistics for all object types
    pub fn all_stats(&self) -> HashMap<ObjectType, PerTypeStats> {
        self.stats.lock().unwrap().clone()
    }

    /// Reset all statistics
    pub fn reset_stats(&self) {
        self.stats.lock().unwrap().clear();
    }

    /// Export metrics in Prometheus format
    pub fn to_prometheus_metrics(&self) -> String {
        let stats = self.stats.lock().unwrap();
        let mut output = String::new();

        output.push_str("# HELP mediagit_compression_per_type_ratio Compression ratio by object type\n");
        output.push_str("# TYPE mediagit_compression_per_type_ratio gauge\n");
        for (obj_type, stat) in stats.iter() {
            output.push_str(&format!(
                "mediagit_compression_per_type_ratio{{type=\"{:?}\"}} {}\n",
                obj_type,
                stat.avg_ratio
            ));
        }

        output.push_str("\n# HELP mediagit_compression_per_type_throughput Throughput by object type (MB/s)\n");
        output.push_str("# TYPE mediagit_compression_per_type_throughput gauge\n");
        for (obj_type, stat) in stats.iter() {
            output.push_str(&format!(
                "mediagit_compression_per_type_throughput{{type=\"{:?}\"}} {}\n",
                obj_type,
                stat.avg_throughput_mbps()
            ));
        }

        output.push_str("\n# HELP mediagit_compression_per_type_operations Total operations by object type\n");
        output.push_str("# TYPE mediagit_compression_per_type_operations counter\n");
        for (obj_type, stat) in stats.iter() {
            output.push_str(&format!(
                "mediagit_compression_per_type_operations{{type=\"{:?}\"}} {}\n",
                obj_type,
                stat.compressions
            ));
        }

        output
    }
}

impl Default for PerObjectTypeCompressor {
    fn default() -> Self {
        Self::new()
    }
}

impl TypeAwareCompressor for PerObjectTypeCompressor {
    fn compress_typed(&self, data: &[u8], obj_type: ObjectType) -> CompressionResult<Vec<u8>> {
        let start = Instant::now();
        let strategy = self.strategy_for_type(obj_type);
        let result = self.compress_with_strategy(data, strategy)?;
        let duration = start.elapsed();

        // Record metrics
        let mut metrics = CompressionMetrics::default();
        let algorithm = match strategy {
            CompressionStrategy::Store => MetricsAlgorithm::None,
            CompressionStrategy::Zlib(_) => MetricsAlgorithm::Zlib,
            CompressionStrategy::Zstd(_) => MetricsAlgorithm::Zstd,
            CompressionStrategy::Brotli(_) => MetricsAlgorithm::Brotli,
            CompressionStrategy::Delta => MetricsAlgorithm::Zstd,
        };
        let level = match strategy {
            CompressionStrategy::Store => MetricsLevel::Fast,
            CompressionStrategy::Zlib(l) | CompressionStrategy::Zstd(l) | CompressionStrategy::Brotli(l) => {
                match l {
                    CompressionLevel::Fast => MetricsLevel::Fast,
                    CompressionLevel::Default => MetricsLevel::Default,
                    CompressionLevel::Best => MetricsLevel::Best,
                }
            }
            CompressionStrategy::Delta => MetricsLevel::Default,
        };
        metrics.record_compression(data, &result, duration, algorithm, level);

        // Update per-type stats
        if let Ok(mut stats) = self.stats.lock() {
            stats.entry(obj_type).or_insert_with(PerTypeStats::default).record(&metrics);
        }

        Ok(result)
    }

    fn decompress_typed(&self, data: &[u8]) -> CompressionResult<Vec<u8>> {
        // Auto-detect compression algorithm
        let algorithm = CompressionAlgorithm::detect(data);

        match algorithm {
            CompressionAlgorithm::None => Ok(data.to_vec()),
            CompressionAlgorithm::Zlib => self.zlib.decompress(data),
            CompressionAlgorithm::Zstd => self.zstd_default.decompress(data),
            CompressionAlgorithm::Brotli => self.brotli_default.decompress(data),
        }
    }

    fn strategy_for_type(&self, obj_type: ObjectType) -> CompressionStrategy {
        self.strategy_for_type(obj_type)
    }
}

impl Compressor for PerObjectTypeCompressor {
    fn compress(&self, data: &[u8]) -> CompressionResult<Vec<u8>> {
        // Default to Zstd when no type information available
        self.compress_typed(data, ObjectType::Unknown)
    }

    fn decompress(&self, data: &[u8]) -> CompressionResult<Vec<u8>> {
        self.decompress_typed(data)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_compression_profiles() {
        let speed = CompressionProfile::Speed;
        let balanced = CompressionProfile::Balanced;
        let max = CompressionProfile::MaxCompression;

        // Text files should use different strategies
        let text_strategy_speed = speed.strategy_for_type(ObjectType::Text);
        let text_strategy_balanced = balanced.strategy_for_type(ObjectType::Text);
        let text_strategy_max = max.strategy_for_type(ObjectType::Text);

        // Speed should use Zstd Fast
        assert_eq!(text_strategy_speed, CompressionStrategy::Zstd(CompressionLevel::Fast));
        // Balanced should use Zstd Default (delegated from for_object_type)
        assert_eq!(text_strategy_balanced, CompressionStrategy::Zstd(CompressionLevel::Default));
        // Max should use Brotli Best (maximum compression for text)
        assert_eq!(text_strategy_max, CompressionStrategy::Brotli(CompressionLevel::Best));
    }

    #[test]
    fn test_per_type_compressor_balanced() {
        let compressor = PerObjectTypeCompressor::new();
        let text_data = b"Hello, World! ".repeat(100);

        let compressed = compressor.compress_typed(&text_data, ObjectType::Text).unwrap();
        assert!(compressed.len() < text_data.len());

        let decompressed = compressor.decompress_typed(&compressed).unwrap();
        assert_eq!(text_data, &decompressed[..]);
    }

    #[test]
    fn test_per_type_compressor_speed() {
        let compressor = PerObjectTypeCompressor::with_profile(CompressionProfile::Speed);
        let image_data = vec![0x42u8; 10000];

        let compressed = compressor.compress_typed(&image_data, ObjectType::Tiff).unwrap();
        let decompressed = compressor.decompress_typed(&compressed).unwrap();
        assert_eq!(image_data, decompressed);
    }

    #[test]
    fn test_custom_strategy() {
        let mut compressor = PerObjectTypeCompressor::new();

        // Set custom strategy for JSON files
        compressor.set_strategy(ObjectType::Json, CompressionStrategy::Zstd(CompressionLevel::Fast));

        let json_data = b"{\"test\": \"data\"}";
        let compressed = compressor.compress_typed(json_data, ObjectType::Json).unwrap();

        // Should use Zstd Fast instead of Brotli Best
        let decompressed = compressor.decompress_typed(&compressed).unwrap();
        assert_eq!(json_data, &decompressed[..]);
    }

    #[test]
    fn test_already_compressed_types() {
        let compressor = PerObjectTypeCompressor::new();
        let jpeg_data = vec![0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x10];

        let compressed = compressor.compress_typed(&jpeg_data, ObjectType::Jpeg).unwrap();

        // Should store as-is (no compression)
        assert_eq!(compressed, jpeg_data);
    }

    #[test]
    fn test_per_type_stats() {
        let compressor = PerObjectTypeCompressor::new();

        // Compress some text data
        let text_data = b"Test data";
        compressor.compress_typed(text_data, ObjectType::Text).unwrap();
        compressor.compress_typed(text_data, ObjectType::Text).unwrap();

        // Compress some JSON data
        let json_data = b"{\"test\": \"data\"}";
        compressor.compress_typed(json_data, ObjectType::Json).unwrap();

        // Check stats
        let text_stats = compressor.stats_for_type(ObjectType::Text).unwrap();
        assert_eq!(text_stats.compressions, 2);

        let json_stats = compressor.stats_for_type(ObjectType::Json).unwrap();
        assert_eq!(json_stats.compressions, 1);
    }

    #[test]
    fn test_prometheus_export() {
        let compressor = PerObjectTypeCompressor::new();

        compressor.compress_typed(b"test data", ObjectType::Text).unwrap();

        let prometheus = compressor.to_prometheus_metrics();

        assert!(prometheus.contains("mediagit_compression_per_type_ratio"));
        assert!(prometheus.contains("mediagit_compression_per_type_throughput"));
        assert!(prometheus.contains("mediagit_compression_per_type_operations"));
        assert!(prometheus.contains("type=\"Text\""));
    }

    #[test]
    fn test_git_object_compression() {
        let compressor = PerObjectTypeCompressor::new();
        let blob_data = b"blob content";

        let compressed = compressor.compress_typed(blob_data, ObjectType::GitBlob).unwrap();
        let decompressed = compressor.decompress_typed(&compressed).unwrap();

        assert_eq!(blob_data, &decompressed[..]);
        // Git objects should use Zlib
        assert!(compressed.starts_with(&[0x78])); // Zlib magic
    }
}
