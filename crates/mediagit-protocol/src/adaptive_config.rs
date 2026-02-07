// MediaGit - Git for Media Files
// Copyright (C) 2025 MediaGit Contributors
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU Affero General Public License as published
// by the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.

//! Adaptive Transfer Configuration for TB-Scale Operations
//!
//! Provides configurable timeouts, chunk sizes, and keepalive mechanisms
//! optimized for long-running TB-scale transfers.
//!
//! # Background
//!
//! Standard HTTP clients use fixed timeouts that cause failures on slow
//! networks or for large files. TB-scale transfers (1TB+) may take hours
//! and need:
//! - Infinite or very long timeouts
//! - Larger chunk sizes for efficiency
//! - Periodic keepalive to prevent proxy/firewall timeouts
//! - Checkpointing for resumable transfers
//!
//! # Example
//!
//! ```rust
//! use mediagit_protocol::adaptive_config::AdaptiveTransferConfig;
//! use std::time::Duration;
//!
//! // Use default config for GB-scale
//! let config = AdaptiveTransferConfig::default();
//! assert_eq!(config.base_chunk_size, 4 * 1024 * 1024);
//!
//! // Use TB-scale config for large files
//! let config = AdaptiveTransferConfig::tb_scale();
//! assert!(config.read_timeout.is_none()); // Infinite timeout
//! ```

use std::time::Duration;

/// Threshold for switching to TB-scale mode (100GB)
pub const TB_SCALE_THRESHOLD: u64 = 100 * 1024 * 1024 * 1024;

/// Adaptive configuration for TB-scale transfers.
///
/// Provides optimal settings for both GB-scale and TB-scale operations
/// with automatic selection based on transfer size.
#[derive(Debug, Clone)]
pub struct AdaptiveTransferConfig {
    /// Base chunk size for streaming (default: 4MB for GB-scale)
    pub base_chunk_size: usize,

    /// Maximum chunk size (64MB for TB-scale operations)
    pub max_chunk_size: usize,

    /// Threshold for switching to TB-scale mode (default: 100GB)
    pub tb_scale_threshold: u64,

    /// Connection timeout (default: 30 seconds)
    pub connect_timeout: Duration,

    /// Read timeout per chunk (None = infinite)
    pub read_timeout: Option<Duration>,

    /// Write timeout per chunk (None = infinite)
    pub write_timeout: Option<Duration>,

    /// Keepalive interval for long transfers (default: 30 seconds)
    pub keepalive_interval: Duration,

    /// Progress checkpoint interval for resumable state (default: 60 seconds)
    pub checkpoint_interval: Duration,

    /// Maximum retries on transient failures
    pub max_retries: u32,

    /// Base delay between retries (exponential backoff applied)
    pub retry_base_delay: Duration,
}

impl Default for AdaptiveTransferConfig {
    /// Default configuration optimized for GB-scale operations.
    fn default() -> Self {
        Self {
            base_chunk_size: 4 * 1024 * 1024,              // 4MB
            max_chunk_size: 64 * 1024 * 1024,              // 64MB
            tb_scale_threshold: TB_SCALE_THRESHOLD,        // 100GB
            connect_timeout: Duration::from_secs(30),
            read_timeout: Some(Duration::from_secs(300)),  // 5 min per chunk
            write_timeout: Some(Duration::from_secs(300)), // 5 min per chunk
            keepalive_interval: Duration::from_secs(30),
            checkpoint_interval: Duration::from_secs(60),
            max_retries: 3,
            retry_base_delay: Duration::from_secs(1),
        }
    }
}

impl AdaptiveTransferConfig {
    /// Create configuration optimized for TB-scale transfers.
    ///
    /// Uses infinite timeouts, larger chunks, and more frequent keepalives.
    pub fn tb_scale() -> Self {
        Self {
            base_chunk_size: 16 * 1024 * 1024,             // 16MB
            max_chunk_size: 64 * 1024 * 1024,              // 64MB
            tb_scale_threshold: TB_SCALE_THRESHOLD,
            connect_timeout: Duration::from_secs(60),
            read_timeout: None,                             // Infinite
            write_timeout: None,                            // Infinite
            keepalive_interval: Duration::from_secs(15),    // More frequent
            checkpoint_interval: Duration::from_secs(30),   // More frequent checkpoints
            max_retries: 5,                                 // More retries
            retry_base_delay: Duration::from_secs(2),
        }
    }

    /// Create configuration for fast local transfers.
    ///
    /// Smaller timeouts for quick failure detection.
    pub fn fast_local() -> Self {
        Self {
            base_chunk_size: 8 * 1024 * 1024,              // 8MB
            max_chunk_size: 32 * 1024 * 1024,              // 32MB
            tb_scale_threshold: TB_SCALE_THRESHOLD,
            connect_timeout: Duration::from_secs(5),
            read_timeout: Some(Duration::from_secs(30)),
            write_timeout: Some(Duration::from_secs(30)),
            keepalive_interval: Duration::from_secs(60),
            checkpoint_interval: Duration::from_secs(120),
            max_retries: 2,
            retry_base_delay: Duration::from_millis(500),
        }
    }

    /// Select optimal chunk size based on transfer size.
    ///
    /// Large transfers use larger chunks for efficiency.
    pub fn chunk_size_for(&self, total_size: u64) -> usize {
        if total_size >= self.tb_scale_threshold {
            self.max_chunk_size
        } else if total_size >= 1024 * 1024 * 1024 {
            // 1GB+ files use intermediate size
            (self.base_chunk_size + self.max_chunk_size) / 2
        } else {
            self.base_chunk_size
        }
    }

    /// Estimate transfer duration based on size and bandwidth.
    ///
    /// # Arguments
    ///
    /// * `total_size` - Total transfer size in bytes
    /// * `bandwidth_bps` - Network bandwidth in bytes per second
    ///
    /// # Returns
    ///
    /// Estimated duration for the transfer
    pub fn estimated_duration(&self, total_size: u64, bandwidth_bps: u64) -> Duration {
        let seconds = total_size / bandwidth_bps.max(1);
        Duration::from_secs(seconds)
    }

    /// Check if transfer size qualifies as TB-scale.
    pub fn is_tb_scale(&self, size: u64) -> bool {
        size >= self.tb_scale_threshold
    }

    /// Calculate retry delay with exponential backoff.
    ///
    /// # Arguments
    ///
    /// * `attempt` - Current retry attempt (0-based)
    ///
    /// # Returns
    ///
    /// Delay before next retry attempt
    pub fn retry_delay(&self, attempt: u32) -> Duration {
        let multiplier = 2u64.pow(attempt.min(5)); // Cap at 32x
        self.retry_base_delay * multiplier as u32
    }

    /// Get effective read timeout for a given transfer size.
    ///
    /// Returns infinite timeout for TB-scale operations.
    pub fn effective_read_timeout(&self, transfer_size: Option<u64>) -> Option<Duration> {
        match transfer_size {
            Some(size) if size >= self.tb_scale_threshold => None,
            _ => self.read_timeout,
        }
    }

    /// Get effective write timeout for a given transfer size.
    pub fn effective_write_timeout(&self, transfer_size: Option<u64>) -> Option<Duration> {
        match transfer_size {
            Some(size) if size >= self.tb_scale_threshold => None,
            _ => self.write_timeout,
        }
    }
}

/// Builder for AdaptiveTransferConfig.
#[derive(Debug, Default)]
pub struct AdaptiveTransferConfigBuilder {
    config: AdaptiveTransferConfig,
}

impl AdaptiveTransferConfigBuilder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn base_chunk_size(mut self, size: usize) -> Self {
        self.config.base_chunk_size = size;
        self
    }

    pub fn max_chunk_size(mut self, size: usize) -> Self {
        self.config.max_chunk_size = size;
        self
    }

    pub fn connect_timeout(mut self, timeout: Duration) -> Self {
        self.config.connect_timeout = timeout;
        self
    }

    pub fn read_timeout(mut self, timeout: Option<Duration>) -> Self {
        self.config.read_timeout = timeout;
        self
    }

    pub fn write_timeout(mut self, timeout: Option<Duration>) -> Self {
        self.config.write_timeout = timeout;
        self
    }

    pub fn keepalive_interval(mut self, interval: Duration) -> Self {
        self.config.keepalive_interval = interval;
        self
    }

    pub fn checkpoint_interval(mut self, interval: Duration) -> Self {
        self.config.checkpoint_interval = interval;
        self
    }

    pub fn max_retries(mut self, retries: u32) -> Self {
        self.config.max_retries = retries;
        self
    }

    pub fn build(self) -> AdaptiveTransferConfig {
        self.config
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = AdaptiveTransferConfig::default();

        assert_eq!(config.base_chunk_size, 4 * 1024 * 1024);
        assert_eq!(config.max_chunk_size, 64 * 1024 * 1024);
        assert!(config.read_timeout.is_some());
        assert_eq!(config.max_retries, 3);
    }

    #[test]
    fn test_tb_scale_config() {
        let config = AdaptiveTransferConfig::tb_scale();

        assert_eq!(config.base_chunk_size, 16 * 1024 * 1024);
        assert!(config.read_timeout.is_none()); // Infinite
        assert!(config.write_timeout.is_none());
        assert_eq!(config.max_retries, 5);
    }

    #[test]
    fn test_chunk_size_selection() {
        let config = AdaptiveTransferConfig::default();

        // Small file: base chunk
        let small = config.chunk_size_for(100 * 1024 * 1024); // 100MB
        assert_eq!(small, config.base_chunk_size);

        // Large file (1GB+): intermediate
        let large = config.chunk_size_for(2 * 1024 * 1024 * 1024); // 2GB
        assert!(large > config.base_chunk_size);

        // TB-scale: max chunk
        let tb = config.chunk_size_for(200 * 1024 * 1024 * 1024); // 200GB
        assert_eq!(tb, config.max_chunk_size);
    }

    #[test]
    fn test_retry_delay_backoff() {
        let config = AdaptiveTransferConfig::default();

        let delay0 = config.retry_delay(0);
        let delay1 = config.retry_delay(1);
        let delay2 = config.retry_delay(2);

        assert_eq!(delay0, config.retry_base_delay);
        assert_eq!(delay1, config.retry_base_delay * 2);
        assert_eq!(delay2, config.retry_base_delay * 4);

        // Capped at 32x
        let delay_high = config.retry_delay(10);
        assert_eq!(delay_high, config.retry_base_delay * 32);
    }

    #[test]
    fn test_estimated_duration() {
        let config = AdaptiveTransferConfig::default();

        // 1GB at 100MB/s = 10 seconds
        let duration = config.estimated_duration(
            1024 * 1024 * 1024,
            100 * 1024 * 1024,
        );
        assert_eq!(duration, Duration::from_secs(10));
    }

    #[test]
    fn test_effective_timeout() {
        let config = AdaptiveTransferConfig::default();

        // Normal size: use configured timeout
        let normal = config.effective_read_timeout(Some(1024 * 1024 * 1024)); // 1GB
        assert!(normal.is_some());

        // TB-scale: infinite timeout
        let tb = config.effective_read_timeout(Some(200 * 1024 * 1024 * 1024)); // 200GB
        assert!(tb.is_none());
    }

    #[test]
    fn test_builder() {
        let config = AdaptiveTransferConfigBuilder::new()
            .base_chunk_size(8 * 1024 * 1024)
            .max_retries(10)
            .read_timeout(None)
            .build();

        assert_eq!(config.base_chunk_size, 8 * 1024 * 1024);
        assert_eq!(config.max_retries, 10);
        assert!(config.read_timeout.is_none());
    }
}
