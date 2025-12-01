//! Prometheus metrics collector for MediaGit
//!
//! Provides a custom Prometheus collector that gathers metrics from MediaGit operations.

use prometheus::{core::Collector, proto::MetricFamily};
use std::sync::Arc;
use tracing::debug;

use crate::MetricsRegistry;

/// Custom Prometheus collector for MediaGit operations
///
/// This collector wraps the MetricsRegistry and implements the Prometheus
/// Collector trait for integration with the Prometheus ecosystem.
pub struct MediaGitCollector {
    registry: Arc<MetricsRegistry>,
}

impl MediaGitCollector {
    /// Create a new collector wrapping the given metrics registry
    pub fn new(registry: MetricsRegistry) -> Self {
        Self {
            registry: Arc::new(registry),
        }
    }

    /// Get reference to the underlying metrics registry
    pub fn registry(&self) -> &MetricsRegistry {
        &self.registry
    }
}

impl Collector for MediaGitCollector {
    fn desc(&self) -> Vec<&prometheus::core::Desc> {
        // Return descriptors for all metrics
        // This is called by Prometheus to understand what metrics are available
        vec![]
    }

    fn collect(&self) -> Vec<MetricFamily> {
        // Gather all metrics from the registry
        debug!("Collecting MediaGit metrics");

        let mut families = Vec::new();
        let gatherer = self.registry.registry();

        // Gather metrics from the Prometheus registry
        match gatherer.gather() {
            metrics => {
                families.extend(metrics);
            }
        }

        families
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{CompressionAlgorithm, OperationType, StorageBackend};

    #[test]
    fn test_collector_creation() {
        let registry = MetricsRegistry::new().unwrap();
        let collector = MediaGitCollector::new(registry);

        // Verify we can create a collector
        assert!(Arc::strong_count(&collector.registry) == 1);
    }

    #[test]
    fn test_collector_gather_metrics() {
        let registry = MetricsRegistry::new().unwrap();

        // Record some metrics
        registry.record_dedup_write(1000, true);
        registry.record_compression(CompressionAlgorithm::Zstd, 1000, 600);
        registry.record_cache_hit();

        let collector = MediaGitCollector::new(registry);

        // Collect metrics
        let families = collector.collect();

        // Should have metrics families
        assert!(!families.is_empty());

        // Verify we have expected metric names
        let metric_names: Vec<String> = families
            .iter()
            .map(|f| f.get_name().to_string())
            .collect();

        // Check for key metrics
        assert!(metric_names.iter().any(|n| n.contains("dedup")));
        assert!(metric_names.iter().any(|n| n.contains("compression")));
        assert!(metric_names.iter().any(|n| n.contains("cache")));
    }

    #[test]
    fn test_collector_multiple_operations() {
        let registry = MetricsRegistry::new().unwrap();

        // Record various operations
        for _ in 0..10 {
            registry.record_operation_duration(
                OperationType::Store,
                StorageBackend::Filesystem,
                0.05,
            );
            registry.record_operation_complete(
                OperationType::Store,
                StorageBackend::Filesystem,
                true,
            );
        }

        for _ in 0..5 {
            registry.record_backend_latency(
                StorageBackend::S3,
                OperationType::Retrieve,
                0.1,
            );
        }

        let collector = MediaGitCollector::new(registry);
        let families = collector.collect();

        // Verify metrics were collected
        assert!(!families.is_empty());

        // Check for operation and backend metrics
        let metric_names: Vec<String> = families
            .iter()
            .map(|f| f.get_name().to_string())
            .collect();

        assert!(metric_names.iter().any(|n| n.contains("operation")));
        assert!(metric_names.iter().any(|n| n.contains("backend")));
    }
}
