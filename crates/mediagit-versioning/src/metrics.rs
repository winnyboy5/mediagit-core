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

//! Metrics tracking for object database operations

use serde::{Deserialize, Serialize};

/// Metrics for object database operations and deduplication efficiency
///
/// Tracks performance and space savings from content-addressable storage.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct OdbMetrics {
    /// Number of cache hits (object found in cache)
    pub cache_hits: u64,

    /// Number of cache misses (object read from storage)
    pub cache_misses: u64,

    /// Number of unique objects stored
    pub unique_objects: u64,

    /// Total number of write operations (including duplicates)
    pub total_writes: u64,

    /// Total bytes stored (actual storage used)
    pub bytes_stored: u64,

    /// Total bytes written (including duplicates that were deduplicated)
    pub bytes_written: u64,
}

impl OdbMetrics {
    /// Create new metrics with zero values
    pub fn new() -> Self {
        Self::default()
    }

    /// Calculate cache hit rate
    ///
    /// Returns ratio of hits to total accesses (hits + misses)
    ///
    /// # Examples
    ///
    /// ```
    /// use mediagit_versioning::OdbMetrics;
    ///
    /// let mut metrics = OdbMetrics::new();
    /// metrics.cache_hits = 75;
    /// metrics.cache_misses = 25;
    /// assert_eq!(metrics.hit_rate(), 0.75);
    /// ```
    pub fn hit_rate(&self) -> f64 {
        let total = self.cache_hits + self.cache_misses;
        if total == 0 {
            0.0
        } else {
            self.cache_hits as f64 / total as f64
        }
    }

    /// Calculate deduplication ratio
    ///
    /// Returns ratio of bytes saved through deduplication
    ///
    /// # Examples
    ///
    /// ```
    /// use mediagit_versioning::OdbMetrics;
    ///
    /// let mut metrics = OdbMetrics::new();
    /// metrics.bytes_written = 1000;
    /// metrics.bytes_stored = 600;
    /// assert_eq!(metrics.dedup_ratio(), 0.4); // 40% saved
    /// ```
    pub fn dedup_ratio(&self) -> f64 {
        if self.bytes_written == 0 {
            0.0
        } else {
            (self.bytes_written - self.bytes_stored) as f64 / self.bytes_written as f64
        }
    }

    /// Calculate bytes saved through deduplication
    pub fn bytes_saved(&self) -> u64 {
        self.bytes_written.saturating_sub(self.bytes_stored)
    }

    /// Record a cache hit
    pub fn record_cache_hit(&mut self) {
        self.cache_hits += 1;
    }

    /// Record a cache miss
    pub fn record_cache_miss(&mut self) {
        self.cache_misses += 1;
    }

    /// Record a new object write
    pub fn record_write(&mut self, size: u64, is_new: bool) {
        self.total_writes += 1;
        self.bytes_written += size;

        if is_new {
            self.unique_objects += 1;
            self.bytes_stored += size;
        }
    }

    /// Record object deletion
    pub fn record_delete(&mut self, size: u64) {
        if self.unique_objects > 0 {
            self.unique_objects -= 1;
        }
        self.bytes_stored = self.bytes_stored.saturating_sub(size);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_metrics() {
        let metrics = OdbMetrics::new();
        assert_eq!(metrics.cache_hits, 0);
        assert_eq!(metrics.cache_misses, 0);
        assert_eq!(metrics.unique_objects, 0);
        assert_eq!(metrics.hit_rate(), 0.0);
        assert_eq!(metrics.dedup_ratio(), 0.0);
    }

    #[test]
    fn test_hit_rate() {
        let mut metrics = OdbMetrics::new();
        metrics.cache_hits = 80;
        metrics.cache_misses = 20;
        assert_eq!(metrics.hit_rate(), 0.8);
    }

    #[test]
    fn test_dedup_ratio() {
        let mut metrics = OdbMetrics::new();
        metrics.bytes_written = 10000;
        metrics.bytes_stored = 3000;
        assert_eq!(metrics.dedup_ratio(), 0.7); // 70% saved
        assert_eq!(metrics.bytes_saved(), 7000);
    }

    #[test]
    fn test_record_cache_hit() {
        let mut metrics = OdbMetrics::new();
        metrics.record_cache_hit();
        metrics.record_cache_hit();
        assert_eq!(metrics.cache_hits, 2);
    }

    #[test]
    fn test_record_write_new() {
        let mut metrics = OdbMetrics::new();
        metrics.record_write(1000, true);
        assert_eq!(metrics.total_writes, 1);
        assert_eq!(metrics.unique_objects, 1);
        assert_eq!(metrics.bytes_written, 1000);
        assert_eq!(metrics.bytes_stored, 1000);
    }

    #[test]
    fn test_record_write_duplicate() {
        let mut metrics = OdbMetrics::new();
        metrics.record_write(1000, true); // New object
        metrics.record_write(1000, false); // Duplicate
        assert_eq!(metrics.total_writes, 2);
        assert_eq!(metrics.unique_objects, 1);
        assert_eq!(metrics.bytes_written, 2000);
        assert_eq!(metrics.bytes_stored, 1000);
        assert_eq!(metrics.dedup_ratio(), 0.5); // 50% saved
    }

    #[test]
    fn test_record_delete() {
        let mut metrics = OdbMetrics::new();
        metrics.record_write(1000, true);
        metrics.record_delete(1000);
        assert_eq!(metrics.unique_objects, 0);
        assert_eq!(metrics.bytes_stored, 0);
    }
}
