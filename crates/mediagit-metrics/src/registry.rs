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
//! Metrics registry for tracking MediaGit operations

use prometheus::{
    Counter, CounterVec, Gauge, GaugeVec, HistogramOpts, HistogramVec, Opts, Registry,
};
use std::sync::Arc;
use tracing::warn;

use crate::types::{CompressionAlgorithm, OperationType, StorageBackend};

/// Central metrics registry for MediaGit operations
///
/// Thread-safe registry that can be cloned and shared across async tasks.
#[derive(Clone)]
pub struct MetricsRegistry {
    inner: Arc<MetricsRegistryInner>,
}

struct MetricsRegistryInner {
    /// Prometheus registry
    registry: Registry,

    // Deduplication metrics
    /// Total bytes written (including duplicates)
    dedup_bytes_written: Counter,
    /// Total bytes actually stored (after deduplication)
    dedup_bytes_stored: Counter,
    /// Number of duplicate writes avoided
    dedup_writes_avoided: Counter,
    /// Current deduplication ratio (0.0-1.0)
    dedup_ratio: Gauge,

    // Compression metrics
    /// Compression ratio by algorithm
    compression_ratio: GaugeVec,
    /// Bytes saved by compression
    compression_bytes_saved: CounterVec,
    /// Original size before compression
    compression_original_bytes: CounterVec,
    /// Compressed size
    compression_compressed_bytes: CounterVec,

    // Cache metrics
    /// Cache hits
    cache_hits: Counter,
    /// Cache misses
    cache_misses: Counter,
    /// Cache hit rate (0.0-1.0)
    cache_hit_rate: Gauge,

    // Operation timing metrics
    /// Operation duration histogram (seconds)
    operation_duration: HistogramVec,
    /// Operation count
    operation_total: CounterVec,
    /// Operation errors
    operation_errors: CounterVec,

    // Storage backend metrics
    /// Backend operation latency
    backend_latency: HistogramVec,
    /// Backend throughput (bytes/second)
    backend_throughput: GaugeVec,
}

impl MetricsRegistry {
    /// Create new metrics registry
    pub fn new() -> anyhow::Result<Self> {
        let registry = Registry::new();

        // Deduplication metrics
        let dedup_bytes_written = Counter::with_opts(Opts::new(
            "mediagit_dedup_bytes_written_total",
            "Total bytes written including duplicates",
        ))?;
        registry.register(Box::new(dedup_bytes_written.clone()))?;

        let dedup_bytes_stored = Counter::with_opts(Opts::new(
            "mediagit_dedup_bytes_stored_total",
            "Total bytes actually stored after deduplication",
        ))?;
        registry.register(Box::new(dedup_bytes_stored.clone()))?;

        let dedup_writes_avoided = Counter::with_opts(Opts::new(
            "mediagit_dedup_writes_avoided_total",
            "Number of duplicate writes avoided",
        ))?;
        registry.register(Box::new(dedup_writes_avoided.clone()))?;

        let dedup_ratio = Gauge::with_opts(Opts::new(
            "mediagit_dedup_ratio",
            "Current deduplication ratio (bytes saved / bytes written)",
        ))?;
        registry.register(Box::new(dedup_ratio.clone()))?;

        // Compression metrics
        let compression_ratio = GaugeVec::new(
            Opts::new(
                "mediagit_compression_ratio",
                "Compression ratio by algorithm (compressed / original)",
            ),
            &["algorithm"],
        )?;
        registry.register(Box::new(compression_ratio.clone()))?;

        let compression_bytes_saved = CounterVec::new(
            Opts::new(
                "mediagit_compression_bytes_saved_total",
                "Bytes saved through compression",
            ),
            &["algorithm"],
        )?;
        registry.register(Box::new(compression_bytes_saved.clone()))?;

        let compression_original_bytes = CounterVec::new(
            Opts::new(
                "mediagit_compression_original_bytes_total",
                "Original bytes before compression",
            ),
            &["algorithm"],
        )?;
        registry.register(Box::new(compression_original_bytes.clone()))?;

        let compression_compressed_bytes = CounterVec::new(
            Opts::new(
                "mediagit_compression_compressed_bytes_total",
                "Compressed bytes",
            ),
            &["algorithm"],
        )?;
        registry.register(Box::new(compression_compressed_bytes.clone()))?;

        // Cache metrics
        let cache_hits = Counter::with_opts(Opts::new(
            "mediagit_cache_hits_total",
            "Number of cache hits",
        ))?;
        registry.register(Box::new(cache_hits.clone()))?;

        let cache_misses = Counter::with_opts(Opts::new(
            "mediagit_cache_misses_total",
            "Number of cache misses",
        ))?;
        registry.register(Box::new(cache_misses.clone()))?;

        let cache_hit_rate = Gauge::with_opts(Opts::new(
            "mediagit_cache_hit_rate",
            "Cache hit rate (hits / total accesses)",
        ))?;
        registry.register(Box::new(cache_hit_rate.clone()))?;

        // Operation timing metrics
        let operation_duration = HistogramVec::new(
            HistogramOpts::new(
                "mediagit_operation_duration_seconds",
                "Operation duration in seconds",
            )
            .buckets(vec![
                0.001, 0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0,
            ]),
            &["operation", "backend"],
        )?;
        registry.register(Box::new(operation_duration.clone()))?;

        let operation_total = CounterVec::new(
            Opts::new(
                "mediagit_operation_total",
                "Total number of operations",
            ),
            &["operation", "backend", "status"],
        )?;
        registry.register(Box::new(operation_total.clone()))?;

        let operation_errors = CounterVec::new(
            Opts::new(
                "mediagit_operation_errors_total",
                "Total number of operation errors",
            ),
            &["operation", "backend", "error_type"],
        )?;
        registry.register(Box::new(operation_errors.clone()))?;

        // Backend metrics
        let backend_latency = HistogramVec::new(
            HistogramOpts::new(
                "mediagit_backend_latency_seconds",
                "Storage backend latency in seconds",
            )
            .buckets(vec![
                0.001, 0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0,
            ]),
            &["backend", "operation"],
        )?;
        registry.register(Box::new(backend_latency.clone()))?;

        let backend_throughput = GaugeVec::new(
            Opts::new(
                "mediagit_backend_throughput_bytes_per_second",
                "Storage backend throughput in bytes per second",
            ),
            &["backend", "operation"],
        )?;
        registry.register(Box::new(backend_throughput.clone()))?;

        Ok(Self {
            inner: Arc::new(MetricsRegistryInner {
                registry,
                dedup_bytes_written,
                dedup_bytes_stored,
                dedup_writes_avoided,
                dedup_ratio,
                compression_ratio,
                compression_bytes_saved,
                compression_original_bytes,
                compression_compressed_bytes,
                cache_hits,
                cache_misses,
                cache_hit_rate,
                operation_duration,
                operation_total,
                operation_errors,
                backend_latency,
                backend_throughput,
            }),
        })
    }

    /// Get reference to Prometheus registry for gathering metrics
    pub fn registry(&self) -> &Registry {
        &self.inner.registry
    }

    // Deduplication metrics

    /// Record a write operation for deduplication tracking
    ///
    /// # Arguments
    /// * `bytes` - Size of data written
    /// * `is_new` - Whether this is a new object (true) or duplicate (false)
    pub fn record_dedup_write(&self, bytes: u64, is_new: bool) {
        self.inner.dedup_bytes_written.inc_by(bytes as f64);

        if is_new {
            self.inner.dedup_bytes_stored.inc_by(bytes as f64);
        } else {
            self.inner.dedup_writes_avoided.inc();
        }

        self.update_dedup_ratio();
    }

    /// Update deduplication ratio gauge
    fn update_dedup_ratio(&self) {
        let written = self.inner.dedup_bytes_written.get();
        if written > 0.0 {
            let stored = self.inner.dedup_bytes_stored.get();
            let ratio = (written - stored) / written;
            self.inner.dedup_ratio.set(ratio);
        }
    }

    // Compression metrics

    /// Record compression operation
    pub fn record_compression(
        &self,
        algorithm: CompressionAlgorithm,
        original_size: u64,
        compressed_size: u64,
    ) {
        let algo_label = algorithm.as_label();

        self.inner
            .compression_original_bytes
            .with_label_values(&[algo_label])
            .inc_by(original_size as f64);

        self.inner
            .compression_compressed_bytes
            .with_label_values(&[algo_label])
            .inc_by(compressed_size as f64);

        let saved = original_size.saturating_sub(compressed_size);
        self.inner
            .compression_bytes_saved
            .with_label_values(&[algo_label])
            .inc_by(saved as f64);

        if original_size > 0 {
            let ratio = compressed_size as f64 / original_size as f64;
            self.inner
                .compression_ratio
                .with_label_values(&[algo_label])
                .set(ratio);
        }
    }

    // Cache metrics

    /// Record cache hit
    pub fn record_cache_hit(&self) {
        self.inner.cache_hits.inc();
        self.update_cache_hit_rate();
    }

    /// Record cache miss
    pub fn record_cache_miss(&self) {
        self.inner.cache_misses.inc();
        self.update_cache_hit_rate();
    }

    /// Update cache hit rate gauge
    fn update_cache_hit_rate(&self) {
        let hits = self.inner.cache_hits.get();
        let misses = self.inner.cache_misses.get();
        let total = hits + misses;

        if total > 0.0 {
            let rate = hits / total;
            self.inner.cache_hit_rate.set(rate);
        }
    }

    // Operation timing metrics

    /// Record operation duration
    pub fn record_operation_duration(
        &self,
        operation: OperationType,
        backend: StorageBackend,
        duration_secs: f64,
    ) {
        self.inner
            .operation_duration
            .with_label_values(&[operation.as_label(), backend.as_label()])
            .observe(duration_secs);
    }

    /// Record operation completion
    pub fn record_operation_complete(
        &self,
        operation: OperationType,
        backend: StorageBackend,
        success: bool,
    ) {
        let status = if success { "success" } else { "error" };
        self.inner
            .operation_total
            .with_label_values(&[operation.as_label(), backend.as_label(), status])
            .inc();
    }

    /// Record operation error
    pub fn record_operation_error(
        &self,
        operation: OperationType,
        backend: StorageBackend,
        error_type: &str,
    ) {
        self.inner
            .operation_errors
            .with_label_values(&[operation.as_label(), backend.as_label(), error_type])
            .inc();
    }

    // Backend metrics

    /// Record backend latency
    pub fn record_backend_latency(
        &self,
        backend: StorageBackend,
        operation: OperationType,
        latency_secs: f64,
    ) {
        self.inner
            .backend_latency
            .with_label_values(&[backend.as_label(), operation.as_label()])
            .observe(latency_secs);
    }

    /// Record backend throughput
    pub fn record_backend_throughput(
        &self,
        backend: StorageBackend,
        operation: OperationType,
        bytes_per_second: f64,
    ) {
        self.inner
            .backend_throughput
            .with_label_values(&[backend.as_label(), operation.as_label()])
            .set(bytes_per_second);
    }
}

impl Default for MetricsRegistry {
    fn default() -> Self {
        Self::new().unwrap_or_else(|e| {
            warn!("Failed to create metrics registry: {}", e);
            // Create a minimal fallback registry
            Self {
                inner: Arc::new(MetricsRegistryInner {
                    registry: Registry::new(),
                    dedup_bytes_written: Counter::new("fallback", "fallback").unwrap(),
                    dedup_bytes_stored: Counter::new("fallback", "fallback").unwrap(),
                    dedup_writes_avoided: Counter::new("fallback", "fallback").unwrap(),
                    dedup_ratio: Gauge::new("fallback", "fallback").unwrap(),
                    compression_ratio: GaugeVec::new(
                        Opts::new("fallback", "fallback"),
                        &["algorithm"],
                    )
                    .unwrap(),
                    compression_bytes_saved: CounterVec::new(
                        Opts::new("fallback", "fallback"),
                        &["algorithm"],
                    )
                    .unwrap(),
                    compression_original_bytes: CounterVec::new(
                        Opts::new("fallback", "fallback"),
                        &["algorithm"],
                    )
                    .unwrap(),
                    compression_compressed_bytes: CounterVec::new(
                        Opts::new("fallback", "fallback"),
                        &["algorithm"],
                    )
                    .unwrap(),
                    cache_hits: Counter::new("fallback", "fallback").unwrap(),
                    cache_misses: Counter::new("fallback", "fallback").unwrap(),
                    cache_hit_rate: Gauge::new("fallback", "fallback").unwrap(),
                    operation_duration: HistogramVec::new(
                        HistogramOpts::new("fallback", "fallback"),
                        &["operation", "backend"],
                    )
                    .unwrap(),
                    operation_total: CounterVec::new(
                        Opts::new("fallback", "fallback"),
                        &["operation", "backend", "status"],
                    )
                    .unwrap(),
                    operation_errors: CounterVec::new(
                        Opts::new("fallback", "fallback"),
                        &["operation", "backend", "error_type"],
                    )
                    .unwrap(),
                    backend_latency: HistogramVec::new(
                        HistogramOpts::new("fallback", "fallback"),
                        &["backend", "operation"],
                    )
                    .unwrap(),
                    backend_throughput: GaugeVec::new(
                        Opts::new("fallback", "fallback"),
                        &["backend", "operation"],
                    )
                    .unwrap(),
                }),
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_registry_creation() {
        let registry = MetricsRegistry::new();
        assert!(registry.is_ok());
    }

    #[test]
    fn test_dedup_metrics() {
        let registry = MetricsRegistry::new().unwrap();

        // Write new object
        registry.record_dedup_write(1000, true);
        assert_eq!(registry.inner.dedup_bytes_written.get(), 1000.0);
        assert_eq!(registry.inner.dedup_bytes_stored.get(), 1000.0);

        // Write duplicate
        registry.record_dedup_write(1000, false);
        assert_eq!(registry.inner.dedup_bytes_written.get(), 2000.0);
        assert_eq!(registry.inner.dedup_bytes_stored.get(), 1000.0);
        assert_eq!(registry.inner.dedup_writes_avoided.get(), 1.0);

        // Check dedup ratio: (2000 - 1000) / 2000 = 0.5
        assert_eq!(registry.inner.dedup_ratio.get(), 0.5);
    }

    #[test]
    fn test_compression_metrics() {
        let registry = MetricsRegistry::new().unwrap();

        registry.record_compression(CompressionAlgorithm::Zstd, 1000, 600);

        let original = registry
            .inner
            .compression_original_bytes
            .with_label_values(&["zstd"])
            .get();
        assert_eq!(original, 1000.0);

        let compressed = registry
            .inner
            .compression_compressed_bytes
            .with_label_values(&["zstd"])
            .get();
        assert_eq!(compressed, 600.0);

        let saved = registry
            .inner
            .compression_bytes_saved
            .with_label_values(&["zstd"])
            .get();
        assert_eq!(saved, 400.0);

        let ratio = registry
            .inner
            .compression_ratio
            .with_label_values(&["zstd"])
            .get();
        assert_eq!(ratio, 0.6);
    }

    #[test]
    fn test_cache_metrics() {
        let registry = MetricsRegistry::new().unwrap();

        registry.record_cache_hit();
        registry.record_cache_hit();
        registry.record_cache_hit();
        registry.record_cache_miss();

        assert_eq!(registry.inner.cache_hits.get(), 3.0);
        assert_eq!(registry.inner.cache_misses.get(), 1.0);
        assert_eq!(registry.inner.cache_hit_rate.get(), 0.75);
    }

    #[test]
    fn test_operation_metrics() {
        let registry = MetricsRegistry::new().unwrap();

        registry.record_operation_duration(
            OperationType::Store,
            StorageBackend::Filesystem,
            0.5,
        );

        registry.record_operation_complete(
            OperationType::Store,
            StorageBackend::Filesystem,
            true,
        );

        let count = registry
            .inner
            .operation_total
            .with_label_values(&["store", "filesystem", "success"])
            .get();
        assert_eq!(count, 1.0);
    }
}
