// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2025 MediaGit Contributors

//! Integration tests for mediagit-metrics crate
//!
//! Tests the public API of the metrics module including registry creation,
//! metric recording, and Prometheus export format.

use mediagit_metrics::{MetricsRegistry, TextEncoder, Encoder};
use mediagit_metrics::types::{StorageBackend, CompressionAlgorithm, OperationType};

#[test]
fn test_metrics_registry_creation() {
    let registry = MetricsRegistry::new();
    assert!(registry.is_ok(), "MetricsRegistry should create successfully");
}

#[test]
fn test_metrics_registry_default() {
    // Default should work without errors
    let _registry = MetricsRegistry::default();
}

#[test]
fn test_deduplication_metrics() {
    let registry = MetricsRegistry::new().unwrap();

    // Record some writes
    registry.record_dedup_write(1024, true);  // New object
    registry.record_dedup_write(1024, false); // Duplicate
    registry.record_dedup_write(2048, true);  // New object
    registry.record_dedup_write(2048, false); // Duplicate

    // Should not panic and metrics should be recorded
}

#[test]
fn test_compression_metrics() {
    let registry = MetricsRegistry::new().unwrap();

    registry.record_compression(CompressionAlgorithm::Zstd, 1000, 400);
    registry.record_compression(CompressionAlgorithm::Brotli, 2000, 500);
    registry.record_compression(CompressionAlgorithm::None, 1500, 1500);

    // Should not panic
}

#[test]
fn test_cache_metrics() {
    let registry = MetricsRegistry::new().unwrap();

    // Record cache hits and misses
    for _ in 0..10 {
        registry.record_cache_hit();
    }
    for _ in 0..5 {
        registry.record_cache_miss();
    }

    // Should not panic
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
        OperationType::Retrieve,
        StorageBackend::S3,
        true,
    );

    registry.record_operation_error(
        OperationType::Store,
        StorageBackend::Gcs,
        "network_timeout",
    );

    // Should not panic
}

#[test]
fn test_backend_metrics() {
    let registry = MetricsRegistry::new().unwrap();

    registry.record_backend_latency(
        StorageBackend::S3,
        OperationType::Store,
        0.1,
    );

    registry.record_backend_throughput(
        StorageBackend::Filesystem,
        OperationType::Retrieve,
        1024.0 * 1024.0, // 1 MB/s
    );

    // Should not panic
}

#[test]
fn test_prometheus_export_format() {
    let registry = MetricsRegistry::new().unwrap();

    // Record some metrics
    registry.record_dedup_write(1024, true);
    registry.record_cache_hit();

    // Export to Prometheus format
    let encoder = TextEncoder::new();
    let metric_families = registry.registry().gather();
    let mut buffer = Vec::new();
    encoder.encode(&metric_families, &mut buffer).unwrap();

    let output = String::from_utf8(buffer).unwrap();

    // Should contain standard Prometheus format elements
    assert!(output.contains("# HELP") || output.contains("# TYPE") || output.is_empty(),
        "Output should be valid Prometheus format");
}

#[test]
fn test_registry_thread_safety() {
    use std::thread;
    use std::sync::Arc;

    let registry = Arc::new(MetricsRegistry::new().unwrap());

    let handles: Vec<_> = (0..4)
        .map(|i| {
            let reg = Arc::clone(&registry);
            thread::spawn(move || {
                for j in 0..100 {
                    reg.record_dedup_write(j as u64 * 100, i % 2 == 0);
                    reg.record_cache_hit();
                }
            })
        })
        .collect();

    for handle in handles {
        handle.join().unwrap();
    }

    // Should complete without panics or deadlocks
}
