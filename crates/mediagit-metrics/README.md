# MediaGit Metrics

Prometheus-based metrics collection and HTTP endpoint for monitoring MediaGit operations.

## Features

- ✅ **Prometheus Integration**: Standard Prometheus metrics with text exposition format
- ✅ **HTTP Endpoint**: Axum-based `/metrics` endpoint (port 9090 by default)
- ✅ **Low Overhead**: <1% performance impact on operations
- ✅ **Grafana Compatible**: Ready for Grafana dashboards
- ✅ **Comprehensive Metrics**: Dedup, compression, cache, operations, backends

## Quick Start

### Basic Usage

```rust
use mediagit_metrics::{MetricsRegistry, MetricsServer};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Initialize metrics registry
    let registry = MetricsRegistry::new()?;

    // Start metrics server on port 9090
    let server = MetricsServer::new(registry.clone(), 9090);
    tokio::spawn(async move {
        server.serve().await.expect("metrics server failed");
    });

    // Record metrics
    registry.record_dedup_write(1024, true);
    registry.record_cache_hit();

    Ok(())
}
```

### Configuration

```rust
use mediagit_metrics::types::MetricsConfig;

let config = MetricsConfig {
    port: 9090,
    enabled: true,
    bind_address: "127.0.0.1".to_string(),
};

let server = MetricsServer::with_config(registry, config);
```

## Metrics Collected

### Deduplication Metrics

- `mediagit_dedup_bytes_written_total` - Total bytes written including duplicates
- `mediagit_dedup_bytes_stored_total` - Total bytes actually stored after deduplication
- `mediagit_dedup_writes_avoided_total` - Number of duplicate writes avoided
- `mediagit_dedup_ratio` - Current deduplication ratio (0.0-1.0)

### Compression Metrics

- `mediagit_compression_ratio{algorithm}` - Compression ratio by algorithm
- `mediagit_compression_bytes_saved_total{algorithm}` - Bytes saved through compression
- `mediagit_compression_original_bytes_total{algorithm}` - Original bytes before compression
- `mediagit_compression_compressed_bytes_total{algorithm}` - Compressed bytes

### Cache Metrics

- `mediagit_cache_hits_total` - Number of cache hits
- `mediagit_cache_misses_total` - Number of cache misses
- `mediagit_cache_hit_rate` - Cache hit rate (0.0-1.0)

### Operation Timing Metrics

- `mediagit_operation_duration_seconds{operation,backend}` - Operation duration histogram
- `mediagit_operation_total{operation,backend,status}` - Total number of operations
- `mediagit_operation_errors_total{operation,backend,error_type}` - Operation errors

### Storage Backend Metrics

- `mediagit_backend_latency_seconds{backend,operation}` - Backend latency histogram
- `mediagit_backend_throughput_bytes_per_second{backend,operation}` - Backend throughput

## HTTP Endpoints

### GET /metrics

Returns all metrics in Prometheus text exposition format.

```bash
curl http://localhost:9090/metrics
```

### GET /health

Health check endpoint.

```bash
curl http://localhost:9090/health
```

## Labels

### Storage Backends

- `filesystem` - Local filesystem storage
- `s3` - AWS S3
- `azure_blob` - Azure Blob Storage
- `gcs` - Google Cloud Storage
- `minio` - MinIO (S3-compatible)
- `b2` - Backblaze B2
- `do_spaces` - DigitalOcean Spaces

### Compression Algorithms

- `none` - No compression
- `zstd` - Zstandard compression
- `brotli` - Brotli compression

### Operation Types

- `store` - Store operation
- `retrieve` - Retrieve operation
- `delete` - Delete operation
- `list` - List operation

## Performance

The metrics system is designed for <1% overhead:

```bash
# Run performance benchmarks
cargo bench -p mediagit-metrics

# Example results:
# - Deduplication: 0.02% overhead
# - Compression: 0.05% overhead
# - Cache: 0.01% overhead
# - Operations: 0.03% overhead
# - Overall: <0.15% overhead
```

## Integration with Grafana

### Prometheus Configuration

```yaml
scrape_configs:
  - job_name: 'mediagit'
    static_configs:
      - targets: ['localhost:9090']
```

### Example Grafana Queries

**Deduplication Savings**:
```promql
rate(mediagit_dedup_bytes_written_total[5m]) - rate(mediagit_dedup_bytes_stored_total[5m])
```

**Compression Ratio by Algorithm**:
```promql
mediagit_compression_ratio
```

**Cache Hit Rate**:
```promql
mediagit_cache_hit_rate
```

**P95 Operation Latency**:
```promql
histogram_quantile(0.95, rate(mediagit_operation_duration_seconds_bucket[5m]))
```

**Backend Throughput**:
```promql
mediagit_backend_throughput_bytes_per_second
```

## Environment Variables

- `MEDIAGIT_METRICS_PORT` - Override default metrics port (default: 9090)
- `MEDIAGIT_METRICS_BIND` - Override bind address (default: 127.0.0.1)
- `MEDIAGIT_METRICS_ENABLED` - Enable/disable metrics (default: false)

## Testing

```bash
# Run unit tests
cargo test -p mediagit-metrics

# Run with output
cargo test -p mediagit-metrics -- --nocapture

# Run specific test
cargo test -p mediagit-metrics test_dedup_metrics
```

## Examples

### Recording Deduplication

```rust
// New object written
registry.record_dedup_write(1024, true);

// Duplicate object (dedup avoided)
registry.record_dedup_write(1024, false);
```

### Recording Compression

```rust
use mediagit_metrics::types::CompressionAlgorithm;

registry.record_compression(
    CompressionAlgorithm::Zstd,
    10000, // original size
    6000,  // compressed size
);
```

### Recording Cache Operations

```rust
// Cache hit
registry.record_cache_hit();

// Cache miss
registry.record_cache_miss();
```

### Recording Operations

```rust
use mediagit_metrics::types::{OperationType, StorageBackend};

registry.record_operation_duration(
    OperationType::Store,
    StorageBackend::S3,
    0.05, // duration in seconds
);

registry.record_operation_complete(
    OperationType::Store,
    StorageBackend::S3,
    true, // success
);
```

### Recording Backend Performance

```rust
registry.record_backend_latency(
    StorageBackend::S3,
    OperationType::Store,
    0.05,
);

registry.record_backend_throughput(
    StorageBackend::S3,
    OperationType::Store,
    10_485_760.0, // 10 MB/s
);
```

## Architecture

```
mediagit-metrics/
├── src/
│   ├── lib.rs          # Public API
│   ├── registry.rs     # Metrics registry with all metrics
│   ├── collector.rs    # Prometheus collector implementation
│   ├── server.rs       # Axum HTTP server for /metrics endpoint
│   └── types.rs        # Configuration and enum types
├── benches/
│   └── metrics_overhead.rs  # Performance overhead benchmarks
└── Cargo.toml
```

## Dependencies

- `prometheus 0.13` - Metrics library
- `axum 0.7` - HTTP server framework
- `tokio` - Async runtime
- `tracing` - Structured logging

## License

AGPL-3.0

## Contributors

MediaGit Contributors
