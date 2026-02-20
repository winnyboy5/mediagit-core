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

use std::time::{Duration, Instant};
use serde::{Serialize, Deserialize};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

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
        self.throughput_mbps = if seconds > 0.0 {
            mb / seconds
        } else {
            0.0
        };

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
                self.avg_compression_ratio * (1.0 - weight) +
                self.compression_ratio * weight;
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
            self.algorithm, self.level, self.compression_ratio,
            self.algorithm, self.level, self.throughput_mbps,
            self.algorithm, self.level, self.space_saved,
            self.algorithm, self.level, self.total_operations,
            self.algorithm, self.level, self.total_bytes_processed,
            self.algorithm, self.level, self.avg_compression_ratio
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

/// Timer for measuring compression operations
pub struct CompressionTimer {
    start: Instant,
}

impl CompressionTimer {
    /// Create and start a new timer
    pub fn start() -> Self {
        CompressionTimer {
            start: Instant::now(),
        }
    }

    /// Stop timer and return elapsed duration
    pub fn stop(self) -> Duration {
        self.start.elapsed()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        assert_eq!(metrics.decompression_time.unwrap(), Duration::from_millis(5));
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
    fn test_timer() {
        let timer = CompressionTimer::start();
        std::thread::sleep(Duration::from_millis(10));
        let elapsed = timer.stop();

        assert!(elapsed >= Duration::from_millis(10));
    }

    #[test]
    fn test_metrics_overhead() {
        // Ensure metrics collection overhead is minimal
        let iterations = 1000;
        let start = Instant::now();

        for _ in 0..iterations {
            let mut metrics = CompressionMetrics::new();
            metrics.record_compression(
                &vec![0u8; 100],
                &vec![0u8; 50],
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
        assert!(per_op < 100, "Metrics overhead too high: {}μs per operation", per_op);
    }
}

/// Global metrics aggregator for compression statistics
///
/// Provides system-wide statistics across all compression operations,
/// with support for per-algorithm and per-level breakdowns.
#[derive(Debug, Clone)]
pub struct MetricsAggregator {
    /// Per-algorithm statistics
    per_algorithm: Arc<Mutex<HashMap<CompressionAlgorithm, AggregatedStats>>>,
    /// Per-level statistics
    per_level: Arc<Mutex<HashMap<CompressionLevel, AggregatedStats>>>,
    /// Overall global statistics
    global: Arc<Mutex<AggregatedStats>>,
}

/// Aggregated statistics for a group of compressions
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AggregatedStats {
    /// Total number of operations
    pub operations: usize,
    /// Total bytes processed
    pub total_bytes_processed: usize,
    /// Total bytes compressed
    pub total_bytes_compressed: usize,
    /// Total compression time
    pub total_time_ms: u64,
    /// Minimum compression ratio seen
    pub min_ratio: f64,
    /// Maximum compression ratio seen
    pub max_ratio: f64,
    /// Average compression ratio
    pub avg_ratio: f64,
}

impl AggregatedStats {
    /// Record a new compression operation
    pub fn record(&mut self, metrics: &CompressionMetrics) {
        self.operations += 1;
        self.total_bytes_processed += metrics.original_size;
        self.total_bytes_compressed += metrics.compressed_size;
        self.total_time_ms += metrics.compression_time.as_millis() as u64;

        // Update min/max
        if self.operations == 1 {
            self.min_ratio = metrics.compression_ratio;
            self.max_ratio = metrics.compression_ratio;
        } else {
            self.min_ratio = self.min_ratio.min(metrics.compression_ratio);
            self.max_ratio = self.max_ratio.max(metrics.compression_ratio);
        }

        // Update rolling average
        let weight = 1.0 / self.operations as f64;
        self.avg_ratio = self.avg_ratio * (1.0 - weight) + metrics.compression_ratio * weight;
    }

    /// Get overall compression ratio
    pub fn compression_ratio(&self) -> f64 {
        if self.total_bytes_processed == 0 {
            1.0
        } else {
            self.total_bytes_compressed as f64 / self.total_bytes_processed as f64
        }
    }

    /// Get average throughput (MB/s)
    pub fn avg_throughput_mbps(&self) -> f64 {
        if self.total_time_ms == 0 {
            0.0
        } else {
            let mb = self.total_bytes_processed as f64 / 1_048_576.0;
            let seconds = self.total_time_ms as f64 / 1000.0;
            mb / seconds
        }
    }

    /// Get total space saved
    pub fn total_space_saved(&self) -> i64 {
        self.total_bytes_processed as i64 - self.total_bytes_compressed as i64
    }

    /// Get space saved percentage
    pub fn space_saved_percent(&self) -> f64 {
        if self.total_bytes_processed == 0 {
            0.0
        } else {
            self.total_space_saved() as f64 / self.total_bytes_processed as f64 * 100.0
        }
    }
}

impl MetricsAggregator {
    /// Create a new metrics aggregator
    pub fn new() -> Self {
        MetricsAggregator {
            per_algorithm: Arc::new(Mutex::new(HashMap::new())),
            per_level: Arc::new(Mutex::new(HashMap::new())),
            global: Arc::new(Mutex::new(AggregatedStats::default())),
        }
    }

    /// Record a compression operation
    pub fn record(&self, metrics: &CompressionMetrics) {
        // Update global stats
        if let Ok(mut global) = self.global.lock() {
            global.record(metrics);
        }

        // Update per-algorithm stats
        if let Ok(mut per_algo) = self.per_algorithm.lock() {
            per_algo
                .entry(metrics.algorithm)
                .or_insert_with(AggregatedStats::default)
                .record(metrics);
        }

        // Update per-level stats
        if let Ok(mut per_level) = self.per_level.lock() {
            per_level
                .entry(metrics.level)
                .or_insert_with(AggregatedStats::default)
                .record(metrics);
        }
    }

    /// Get global statistics
    pub fn global_stats(&self) -> AggregatedStats {
        self.global.lock().ok().map(|s| s.clone()).unwrap_or_default()
    }

    /// Get statistics for a specific algorithm
    pub fn algorithm_stats(&self, algorithm: CompressionAlgorithm) -> Option<AggregatedStats> {
        self.per_algorithm.lock().ok()?.get(&algorithm).cloned()
    }

    /// Get statistics for a specific level
    pub fn level_stats(&self, level: CompressionLevel) -> Option<AggregatedStats> {
        self.per_level.lock().ok()?.get(&level).cloned()
    }

    /// Get all algorithm statistics
    pub fn all_algorithm_stats(&self) -> HashMap<CompressionAlgorithm, AggregatedStats> {
        self.per_algorithm.lock().ok().map(|s| s.clone()).unwrap_or_default()
    }

    /// Get all level statistics
    pub fn all_level_stats(&self) -> HashMap<CompressionLevel, AggregatedStats> {
        self.per_level.lock().ok().map(|s| s.clone()).unwrap_or_default()
    }

    /// Reset all statistics
    pub fn reset(&self) {
        if let Ok(mut global) = self.global.lock() {
            *global = AggregatedStats::default();
        }
        if let Ok(mut per_algo) = self.per_algorithm.lock() {
            per_algo.clear();
        }
        if let Ok(mut per_level) = self.per_level.lock() {
            per_level.clear();
        }
    }

    /// Export metrics in Prometheus format
    pub fn to_prometheus_metrics(&self) -> String {
        let mut output = String::new();

        // Global metrics
        let global = self.global_stats();
        output.push_str("# HELP mediagit_compression_global_ratio Global compression ratio\n");
        output.push_str("# TYPE mediagit_compression_global_ratio gauge\n");
        output.push_str(&format!("mediagit_compression_global_ratio {}\n\n", global.avg_ratio));

        output.push_str("# HELP mediagit_compression_global_throughput Global throughput (MB/s)\n");
        output.push_str("# TYPE mediagit_compression_global_throughput gauge\n");
        output.push_str(&format!("mediagit_compression_global_throughput {}\n\n", global.avg_throughput_mbps()));

        output.push_str("# HELP mediagit_compression_global_operations Total operations\n");
        output.push_str("# TYPE mediagit_compression_global_operations counter\n");
        output.push_str(&format!("mediagit_compression_global_operations {}\n\n", global.operations));

        // Per-algorithm metrics
        output.push_str("# HELP mediagit_compression_algorithm_ratio Compression ratio by algorithm\n");
        output.push_str("# TYPE mediagit_compression_algorithm_ratio gauge\n");
        for (algo, stats) in self.all_algorithm_stats() {
            output.push_str(&format!(
                "mediagit_compression_algorithm_ratio{{algorithm=\"{:?}\"}} {}\n",
                algo, stats.avg_ratio
            ));
        }
        output.push('\n');

        // Per-level metrics
        output.push_str("# HELP mediagit_compression_level_ratio Compression ratio by level\n");
        output.push_str("# TYPE mediagit_compression_level_ratio gauge\n");
        for (level, stats) in self.all_level_stats() {
            output.push_str(&format!(
                "mediagit_compression_level_ratio{{level=\"{:?}\"}} {}\n",
                level, stats.avg_ratio
            ));
        }

        output
    }

    /// Export metrics as JSON
    pub fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "global": self.global_stats(),
            "by_algorithm": self.all_algorithm_stats(),
            "by_level": self.all_level_stats(),
        })
    }
}

impl Default for MetricsAggregator {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod aggregator_tests {
    use super::*;

    #[test]
    fn test_aggregator_basic() {
        let aggregator = MetricsAggregator::new();

        let mut metrics1 = CompressionMetrics::default();
        metrics1.record_compression(
            &vec![0u8; 1000],
            &vec![0u8; 500],
            Duration::from_millis(10),
            CompressionAlgorithm::Zstd,
            CompressionLevel::Default,
        );

        aggregator.record(&metrics1);

        let global = aggregator.global_stats();
        assert_eq!(global.operations, 1);
        assert_eq!(global.total_bytes_processed, 1000);
        assert_eq!(global.total_bytes_compressed, 500);
    }

    #[test]
    fn test_aggregator_multiple_operations() {
        let aggregator = MetricsAggregator::new();

        for _ in 0..10 {
            let mut metrics = CompressionMetrics::default();
            metrics.record_compression(
                &vec![0u8; 1000],
                &vec![0u8; 500],
                Duration::from_millis(10),
                CompressionAlgorithm::Zstd,
                CompressionLevel::Default,
            );
            aggregator.record(&metrics);
        }

        let global = aggregator.global_stats();
        assert_eq!(global.operations, 10);
        assert_eq!(global.total_bytes_processed, 10000);
        assert_eq!(global.total_bytes_compressed, 5000);
        assert!((global.avg_ratio - 2.0).abs() < 0.01);
    }

    #[test]
    fn test_aggregator_per_algorithm() {
        let aggregator = MetricsAggregator::new();

        // Record Zstd compression
        let mut metrics_zstd = CompressionMetrics::default();
        metrics_zstd.record_compression(
            &vec![0u8; 1000],
            &vec![0u8; 500],
            Duration::from_millis(10),
            CompressionAlgorithm::Zstd,
            CompressionLevel::Default,
        );
        aggregator.record(&metrics_zstd);

        // Record Brotli compression
        let mut metrics_brotli = CompressionMetrics::default();
        metrics_brotli.record_compression(
            &vec![0u8; 1000],
            &vec![0u8; 400],
            Duration::from_millis(20),
            CompressionAlgorithm::Brotli,
            CompressionLevel::Best,
        );
        aggregator.record(&metrics_brotli);

        // Check per-algorithm stats
        let zstd_stats = aggregator.algorithm_stats(CompressionAlgorithm::Zstd).unwrap();
        assert_eq!(zstd_stats.operations, 1);
        assert_eq!(zstd_stats.total_bytes_compressed, 500);

        let brotli_stats = aggregator.algorithm_stats(CompressionAlgorithm::Brotli).unwrap();
        assert_eq!(brotli_stats.operations, 1);
        assert_eq!(brotli_stats.total_bytes_compressed, 400);

        // Global stats should include both
        let global = aggregator.global_stats();
        assert_eq!(global.operations, 2);
    }

    #[test]
    fn test_aggregator_prometheus_export() {
        let aggregator = MetricsAggregator::new();

        let mut metrics = CompressionMetrics::default();
        metrics.record_compression(
            &vec![0u8; 1000],
            &vec![0u8; 500],
            Duration::from_millis(10),
            CompressionAlgorithm::Zstd,
            CompressionLevel::Default,
        );
        aggregator.record(&metrics);

        let prometheus = aggregator.to_prometheus_metrics();

        assert!(prometheus.contains("mediagit_compression_global_ratio"));
        assert!(prometheus.contains("mediagit_compression_algorithm_ratio"));
        assert!(prometheus.contains("algorithm=\"Zstd\""));
    }

    #[test]
    fn test_aggregator_reset() {
        let aggregator = MetricsAggregator::new();

        let mut metrics = CompressionMetrics::default();
        metrics.record_compression(
            &vec![0u8; 1000],
            &vec![0u8; 500],
            Duration::from_millis(10),
            CompressionAlgorithm::Zstd,
            CompressionLevel::Default,
        );
        aggregator.record(&metrics);

        assert_eq!(aggregator.global_stats().operations, 1);

        aggregator.reset();

        assert_eq!(aggregator.global_stats().operations, 0);
    }
}
