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

//! Compression metrics and statistics

use serde::{Deserialize, Serialize};
use std::time::Duration;

/// Compression algorithm identifier (copy to avoid circular dependency)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
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

/// Compression level configuration
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum CompressionLevel {
    /// Fast compression, larger output
    Fast,
    /// Default balance
    Default,
    /// Best compression, slower
    Best,
}

/// Compression metrics for tracking performance and effectiveness
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompressionMetrics {
    // Size metrics
    /// Original data size in bytes
    pub original_size: usize,
    /// Compressed data size in bytes
    pub compressed_size: usize,
    /// Compression ratio (original/compressed)
    pub compression_ratio: f64,
    /// Space saved in bytes
    pub space_saved: usize,
    /// Space saved as percentage
    pub space_saved_percent: f64,

    // Performance metrics
    /// Compression duration
    pub compression_time: Duration,
    /// Decompression duration (optional)
    pub decompression_time: Option<Duration>,
    /// Throughput in MB/s
    pub throughput_mbps: f64,

    // Algorithm info
    /// Compression algorithm used
    pub algorithm: CompressionAlgorithm,
    /// Compression level used
    pub level: CompressionLevel,

    // Accumulated metrics
    /// Total number of compression operations
    pub total_operations: u64,
    /// Total bytes processed
    pub total_bytes_processed: u64,
    /// Average compression ratio across all operations
    pub avg_compression_ratio: f64,
}

impl CompressionMetrics {
    /// Create new metrics instance
    pub fn new() -> Self {
        Self::default()
    }

    /// Create metrics from size information (backward compatibility)
    pub fn from_sizes(original_size: usize, compressed_size: usize) -> Self {
        let compression_ratio = if original_size == 0 {
            1.0
        } else {
            original_size as f64 / compressed_size as f64
        };

        let space_saved = original_size.saturating_sub(compressed_size);
        let space_saved_percent = if original_size == 0 {
            0.0
        } else {
            (space_saved as f64 / original_size as f64) * 100.0
        };

        CompressionMetrics {
            original_size,
            compressed_size,
            compression_ratio,
            space_saved,
            space_saved_percent,
            compression_time: Duration::from_secs(0),
            decompression_time: None,
            throughput_mbps: 0.0,
            algorithm: CompressionAlgorithm::None,
            level: CompressionLevel::Default,
            total_operations: 1,
            total_bytes_processed: original_size as u64,
            avg_compression_ratio: compression_ratio,
        }
    }

    /// Record a compression operation
    pub fn record_compression(
        &mut self,
        original: &[u8],
        compressed: &[u8],
        duration: Duration,
        algorithm: CompressionAlgorithm,
        level: CompressionLevel,
    ) {
        self.original_size = original.len();
        self.compressed_size = compressed.len();

        // Calculate compression ratio (original/compressed, higher is better)
        self.compression_ratio = if compressed.is_empty() {
            f64::INFINITY
        } else {
            original.len() as f64 / compressed.len() as f64
        };

        self.space_saved = original.len().saturating_sub(compressed.len());
        self.space_saved_percent = if original.is_empty() {
            0.0
        } else {
            (self.space_saved as f64 / original.len() as f64) * 100.0
        };

        self.compression_time = duration;
        self.algorithm = algorithm;
        self.level = level;

        // Calculate throughput (MB/s)
        let mb = original.len() as f64 / 1_048_576.0;
        let seconds = duration.as_secs_f64();
        self.throughput_mbps = if seconds > 0.0 { mb / seconds } else { 0.0 };

        // Update accumulated metrics
        self.total_operations += 1;
        self.total_bytes_processed += original.len() as u64;
        self.update_avg_ratio();
    }

    /// Record a decompression operation
    pub fn record_decompression(&mut self, duration: Duration) {
        self.decompression_time = Some(duration);
    }

    /// Update average compression ratio
    fn update_avg_ratio(&mut self) {
        if self.total_operations == 0 {
            self.avg_compression_ratio = 1.0;
        } else {
            // Weighted average: current avg + (new_ratio - avg) / total_ops
            let weight = 1.0 / self.total_operations as f64;
            self.avg_compression_ratio =
                self.avg_compression_ratio * (1.0 - weight) + self.compression_ratio * weight;
        }
    }

    /// Export metrics in Prometheus format
    pub fn to_prometheus_metrics(&self) -> String {
        format!(
            "# HELP mediagit_compression_ratio Compression ratio (original/compressed)\n\
             # TYPE mediagit_compression_ratio gauge\n\
             mediagit_compression_ratio{{algorithm=\"{:?}\",level=\"{:?}\"}} {}\n\
             \n\
             # HELP mediagit_compression_throughput_mbps Compression throughput in MB/s\n\
             # TYPE mediagit_compression_throughput_mbps gauge\n\
             mediagit_compression_throughput_mbps{{algorithm=\"{:?}\",level=\"{:?}\"}} {}\n\
             \n\
             # HELP mediagit_compression_space_saved_bytes Space saved in bytes\n\
             # TYPE mediagit_compression_space_saved_bytes counter\n\
             mediagit_compression_space_saved_bytes{{algorithm=\"{:?}\",level=\"{:?}\"}} {}\n\
             \n\
             # HELP mediagit_compression_total_operations Total compression operations\n\
             # TYPE mediagit_compression_total_operations counter\n\
             mediagit_compression_total_operations{{algorithm=\"{:?}\",level=\"{:?}\"}} {}\n\
             \n\
             # HELP mediagit_compression_total_bytes_processed Total bytes processed\n\
             # TYPE mediagit_compression_total_bytes_processed counter\n\
             mediagit_compression_total_bytes_processed{{algorithm=\"{:?}\",level=\"{:?}\"}} {}\n\
             \n\
             # HELP mediagit_compression_avg_ratio Average compression ratio\n\
             # TYPE mediagit_compression_avg_ratio gauge\n\
             mediagit_compression_avg_ratio{{algorithm=\"{:?}\",level=\"{:?}\"}} {}\n",
            self.algorithm,
            self.level,
            self.compression_ratio,
            self.algorithm,
            self.level,
            self.throughput_mbps,
            self.algorithm,
            self.level,
            self.space_saved,
            self.algorithm,
            self.level,
            self.total_operations,
            self.algorithm,
            self.level,
            self.total_bytes_processed,
            self.algorithm,
            self.level,
            self.avg_compression_ratio
        )
    }

    /// Export metrics as JSON
    pub fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "size_metrics": {
                "original_size": self.original_size,
                "compressed_size": self.compressed_size,
                "compression_ratio": self.compression_ratio,
                "space_saved": self.space_saved,
                "space_saved_percent": self.space_saved_percent
            },
            "performance_metrics": {
                "compression_time_ms": self.compression_time.as_millis(),
                "decompression_time_ms": self.decompression_time.map(|d| d.as_millis()),
                "throughput_mbps": self.throughput_mbps
            },
            "algorithm": {
                "name": format!("{:?}", self.algorithm),
                "level": format!("{:?}", self.level)
            },
            "accumulated_metrics": {
                "total_operations": self.total_operations,
                "total_bytes_processed": self.total_bytes_processed,
                "avg_compression_ratio": self.avg_compression_ratio
            }
        })
    }

    /// Generate human-readable summary
    pub fn summary(&self) -> String {
        format!(
            "Compressed {} → {} bytes ({:.1}% reduction, {:.2}x ratio) in {:.2}ms ({:.1} MB/s)",
            self.original_size,
            self.compressed_size,
            self.space_saved_percent,
            self.compression_ratio,
            self.compression_time.as_millis(),
            self.throughput_mbps
        )
    }

    /// Calculate compression ratio (compressed / original) - backward compatibility
    pub fn compression_ratio_legacy(&self) -> f64 {
        if self.original_size == 0 {
            1.0
        } else {
            self.compressed_size as f64 / self.original_size as f64
        }
    }

    /// Calculate bytes saved - backward compatibility
    pub fn bytes_saved(&self) -> i64 {
        self.space_saved as i64
    }

    /// Calculate savings percentage - backward compatibility
    pub fn savings_percentage(&self) -> f64 {
        self.space_saved_percent
    }

    /// Calculate compression throughput (bytes/second) - backward compatibility
    pub fn compression_throughput(&self) -> Option<f64> {
        let secs = self.compression_time.as_secs_f64();
        if secs > 0.0 {
            Some(self.original_size as f64 / secs)
        } else {
            None
        }
    }

    /// Calculate decompression throughput (bytes/second) - backward compatibility
    pub fn decompression_throughput(&self) -> Option<f64> {
        self.decompression_time.map(|duration| {
            let secs = duration.as_secs_f64();
            if secs > 0.0 {
                self.compressed_size as f64 / secs
            } else {
                0.0
            }
        })
    }

    /// Format metrics as a readable string - backward compatibility
    pub fn format_summary(&self) -> String {
        self.summary()
    }
}

impl Default for CompressionMetrics {
    fn default() -> Self {
        CompressionMetrics {
            original_size: 0,
            compressed_size: 0,
            compression_ratio: 1.0,
            space_saved: 0,
            space_saved_percent: 0.0,
            compression_time: Duration::from_secs(0),
            decompression_time: None,
            throughput_mbps: 0.0,
            algorithm: CompressionAlgorithm::None,
            level: CompressionLevel::Default,
            total_operations: 0,
            total_bytes_processed: 0,
            avg_compression_ratio: 1.0,
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    // Only the overhead benchmark below needs a clock; `Instant` was a
    // production import until `CompressionTimer` was removed as dead code.
    use std::time::Instant;

    #[test]
    fn test_record_compression() {
        let mut metrics = CompressionMetrics::new();
        let original = vec![0u8; 1000];
        let compressed = vec![0u8; 400];

        metrics.record_compression(
            &original,
            &compressed,
            Duration::from_millis(50),
            CompressionAlgorithm::Zstd,
            CompressionLevel::Default,
        );

        assert_eq!(metrics.original_size, 1000);
        assert_eq!(metrics.compressed_size, 400);
        assert_eq!(metrics.space_saved, 600);
        assert_eq!(metrics.space_saved_percent, 60.0);
        assert!((metrics.compression_ratio - 2.5).abs() < 0.01); // 1000/400 = 2.5
        assert_eq!(metrics.total_operations, 1);
        assert_eq!(metrics.total_bytes_processed, 1000);
    }

    #[test]
    fn test_prometheus_export() {
        let mut metrics = CompressionMetrics::new();
        let original = vec![0u8; 1000];
        let compressed = vec![0u8; 500];

        metrics.record_compression(
            &original,
            &compressed,
            Duration::from_millis(10),
            CompressionAlgorithm::Zstd,
            CompressionLevel::Best,
        );

        let prometheus = metrics.to_prometheus_metrics();

        assert!(prometheus.contains("mediagit_compression_ratio"));
        assert!(prometheus.contains("mediagit_compression_throughput_mbps"));
        assert!(prometheus.contains("mediagit_compression_space_saved_bytes"));
        assert!(prometheus.contains("algorithm=\"Zstd\""));
        assert!(prometheus.contains("level=\"Best\""));
    }

    #[test]
    fn test_json_export() {
        let mut metrics = CompressionMetrics::new();
        let original = vec![0u8; 2000];
        let compressed = vec![0u8; 800];

        metrics.record_compression(
            &original,
            &compressed,
            Duration::from_millis(20),
            CompressionAlgorithm::Brotli,
            CompressionLevel::Fast,
        );

        let json = metrics.to_json();

        assert_eq!(json["size_metrics"]["original_size"], 2000);
        assert_eq!(json["size_metrics"]["compressed_size"], 800);
        assert_eq!(json["size_metrics"]["space_saved"], 1200);
        assert_eq!(json["algorithm"]["name"], "Brotli");
        assert_eq!(json["algorithm"]["level"], "Fast");
        assert!(json["performance_metrics"]["throughput_mbps"].is_number());
    }

    #[test]
    fn test_summary_output() {
        let mut metrics = CompressionMetrics::new();
        let original = vec![0u8; 1000];
        let compressed = vec![0u8; 250];

        metrics.record_compression(
            &original,
            &compressed,
            Duration::from_millis(5),
            CompressionAlgorithm::Zstd,
            CompressionLevel::Default,
        );

        let summary = metrics.summary();

        assert!(summary.contains("1000"));
        assert!(summary.contains("250"));
        assert!(summary.contains("75.0%")); // reduction
        assert!(summary.contains("4.00x")); // ratio
    }

    #[test]
    fn test_accumulated_metrics() {
        let mut metrics = CompressionMetrics::new();

        // First operation
        metrics.record_compression(
            &vec![0u8; 1000],
            &vec![0u8; 500],
            Duration::from_millis(10),
            CompressionAlgorithm::Zstd,
            CompressionLevel::Default,
        );

        // Second operation
        metrics.record_compression(
            &vec![0u8; 2000],
            &vec![0u8; 800],
            Duration::from_millis(20),
            CompressionAlgorithm::Zstd,
            CompressionLevel::Default,
        );

        assert_eq!(metrics.total_operations, 2);
        assert_eq!(metrics.total_bytes_processed, 3000);
        assert!(metrics.avg_compression_ratio > 2.0);
    }

    #[test]
    fn test_throughput_calculation() {
        let mut metrics = CompressionMetrics::new();
        let data = vec![0u8; 1_048_576]; // 1 MB
        let compressed = vec![0u8; 524_288]; // 0.5 MB

        metrics.record_compression(
            &data,
            &compressed,
            Duration::from_secs(1),
            CompressionAlgorithm::Zstd,
            CompressionLevel::Fast,
        );

        // 1 MB in 1 second = 1.0 MB/s
        assert!((metrics.throughput_mbps - 1.0).abs() < 0.01);
    }

    #[test]
    fn test_decompression_recording() {
        let mut metrics = CompressionMetrics::new();

        metrics.record_compression(
            &vec![0u8; 1000],
            &vec![0u8; 500],
            Duration::from_millis(10),
            CompressionAlgorithm::Zstd,
            CompressionLevel::Default,
        );

        metrics.record_decompression(Duration::from_millis(5));

        assert!(metrics.decompression_time.is_some());
        assert_eq!(
            metrics.decompression_time.unwrap(),
            Duration::from_millis(5)
        );
    }

    #[test]
    fn test_backward_compatibility() {
        let metrics = CompressionMetrics::from_sizes(1000, 500);

        assert_eq!(metrics.original_size, 1000);
        assert_eq!(metrics.compressed_size, 500);
        assert_eq!(metrics.bytes_saved(), 500);
        assert_eq!(metrics.savings_percentage(), 50.0);
        assert!((metrics.compression_ratio_legacy() - 0.5).abs() < 0.01);
    }

    #[test]
    fn test_zero_size_handling() {
        let metrics = CompressionMetrics::from_sizes(0, 0);

        assert_eq!(metrics.compression_ratio, 1.0);
        assert_eq!(metrics.space_saved_percent, 0.0);
    }

    #[test]
    fn test_metrics_overhead() {
        // Ensure metrics collection overhead is minimal
        let iterations = 1000;
        let start = Instant::now();

        for _ in 0..iterations {
            let mut metrics = CompressionMetrics::new();
            metrics.record_compression(
                &[0u8; 100],
                &[0u8; 50],
                Duration::from_micros(1),
                CompressionAlgorithm::Zstd,
                CompressionLevel::Fast,
            );
            let _ = metrics.to_prometheus_metrics();
            let _ = metrics.to_json();
        }

        let elapsed = start.elapsed();
        let per_op = elapsed.as_micros() / iterations;

        // Should be under 100 microseconds per operation
        assert!(
            per_op < 100,
            "Metrics overhead too high: {}μs per operation",
            per_op
        );
    }
}
