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

/// Compression metrics for tracking performance and effectiveness
#[derive(Debug, Clone)]
pub struct CompressionMetrics {
    /// Original data size in bytes
    pub original_size: usize,
    /// Compressed data size in bytes
    pub compressed_size: usize,
    /// Compression duration
    pub compression_duration: Option<Duration>,
    /// Decompression duration
    pub decompression_duration: Option<Duration>,
}

impl CompressionMetrics {
    /// Create metrics from size information
    pub fn from_sizes(original_size: usize, compressed_size: usize) -> Self {
        CompressionMetrics {
            original_size,
            compressed_size,
            compression_duration: None,
            decompression_duration: None,
        }
    }

    /// Calculate compression ratio (compressed / original)
    pub fn compression_ratio(&self) -> f64 {
        if self.original_size == 0 {
            1.0
        } else {
            self.compressed_size as f64 / self.original_size as f64
        }
    }

    /// Calculate bytes saved
    pub fn bytes_saved(&self) -> i64 {
        self.original_size as i64 - self.compressed_size as i64
    }

    /// Calculate savings percentage
    pub fn savings_percentage(&self) -> f64 {
        if self.original_size == 0 {
            0.0
        } else {
            (self.bytes_saved() as f64 / self.original_size as f64) * 100.0
        }
    }

    /// Calculate compression throughput (bytes/second)
    pub fn compression_throughput(&self) -> Option<f64> {
        self.compression_duration.map(|duration| {
            let secs = duration.as_secs_f64();
            if secs > 0.0 {
                self.original_size as f64 / secs
            } else {
                0.0
            }
        })
    }

    /// Calculate decompression throughput (bytes/second)
    pub fn decompression_throughput(&self) -> Option<f64> {
        self.decompression_duration.map(|duration| {
            let secs = duration.as_secs_f64();
            if secs > 0.0 {
                self.compressed_size as f64 / secs
            } else {
                0.0
            }
        })
    }

    /// Format metrics as a readable string
    pub fn format_summary(&self) -> String {
        let ratio = self.compression_ratio();
        let savings = self.savings_percentage();

        let throughput = self
            .compression_throughput()
            .map(|tp| format!(" ({:.2} MB/s)", tp / (1024.0 * 1024.0)))
            .unwrap_or_default();

        format!(
            "Original: {} bytes, Compressed: {} bytes, Ratio: {:.2}x, Saved: {:.1}%{}",
            self.original_size, self.compressed_size, ratio, savings, throughput
        )
    }
}

impl Default for CompressionMetrics {
    fn default() -> Self {
        CompressionMetrics {
            original_size: 0,
            compressed_size: 0,
            compression_duration: None,
            decompression_duration: None,
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
    fn test_compression_ratio() {
        let metrics = CompressionMetrics {
            original_size: 1000,
            compressed_size: 500,
            compression_duration: None,
            decompression_duration: None,
        };

        assert_eq!(metrics.compression_ratio(), 0.5);
    }

    #[test]
    fn test_bytes_saved() {
        let metrics = CompressionMetrics {
            original_size: 1000,
            compressed_size: 600,
            compression_duration: None,
            decompression_duration: None,
        };

        assert_eq!(metrics.bytes_saved(), 400);
    }

    #[test]
    fn test_savings_percentage() {
        let metrics = CompressionMetrics {
            original_size: 1000,
            compressed_size: 750,
            compression_duration: None,
            decompression_duration: None,
        };

        assert_eq!(metrics.savings_percentage(), 25.0);
    }

    #[test]
    fn test_savings_percentage_no_compression() {
        let metrics = CompressionMetrics {
            original_size: 1000,
            compressed_size: 1000,
            compression_duration: None,
            decompression_duration: None,
        };

        assert_eq!(metrics.savings_percentage(), 0.0);
    }

    #[test]
    fn test_compression_ratio_zero_size() {
        let metrics = CompressionMetrics {
            original_size: 0,
            compressed_size: 0,
            compression_duration: None,
            decompression_duration: None,
        };

        assert_eq!(metrics.compression_ratio(), 1.0);
    }

    #[test]
    fn test_format_summary() {
        let metrics = CompressionMetrics {
            original_size: 1000,
            compressed_size: 500,
            compression_duration: None,
            decompression_duration: None,
        };

        let summary = metrics.format_summary();
        assert!(summary.contains("Original: 1000"));
        assert!(summary.contains("Compressed: 500"));
        assert!(summary.contains("Ratio: 0.50x"));
        assert!(summary.contains("Saved: 50.0%"));
    }

    #[test]
    fn test_compression_throughput() {
        let metrics = CompressionMetrics {
            original_size: 1000,
            compressed_size: 500,
            compression_duration: Some(Duration::from_millis(100)),
            decompression_duration: None,
        };

        let throughput = metrics.compression_throughput().unwrap();
        // 1000 bytes in 100ms = 10,000 bytes/sec = ~10KB/s
        assert!(throughput > 9_000.0 && throughput < 11_000.0);
    }

    #[test]
    fn test_timer() {
        let timer = CompressionTimer::start();
        std::thread::sleep(Duration::from_millis(10));
        let elapsed = timer.stop();

        assert!(elapsed >= Duration::from_millis(10));
    }
}
