//! MediaGit Metrics Module
//!
//! Prometheus-based metrics collection and HTTP endpoint for monitoring MediaGit operations.
//!
//! # Features
//!
//! - **Prometheus Integration**: Standard Prometheus metrics with text exposition format
//! - **HTTP Endpoint**: Axum-based `/metrics` endpoint for scraping
//! - **Low Overhead**: <1% performance impact on operations
//! - **Grafana Compatible**: Ready for Grafana dashboards
//!
//! # Key Metrics
//!
//! - Deduplication ratio (from object database)
//! - Compression ratios by algorithm
//! - Operation timing (store/retrieve)
//! - Cache hit/miss rates
//! - Storage backend performance
//!
//! # Example
//!
//! ```ignore
//! use mediagit_metrics::{MetricsRegistry, MetricsServer};
//!
//! #[tokio::main]
//! async fn main() -> anyhow::Result<()> {
//!     // Initialize metrics registry
//!     let registry = MetricsRegistry::new();
//!
//!     // Start metrics server
//!     let server = MetricsServer::new(registry.clone(), 9090)?;
//!     tokio::spawn(async move {
//!         server.serve().await
//!     });
//!
//!     // Record metrics
//!     registry.record_dedup_write(1024, true);
//!     registry.record_cache_hit();
//!
//!     Ok(())
//! }
//! ```

pub mod collector;
pub mod registry;
pub mod server;
pub mod types;

pub use collector::MediaGitCollector;
pub use registry::MetricsRegistry;
pub use server::MetricsServer;
pub use types::{MetricsConfig, StorageBackend, CompressionAlgorithm};

// Re-export prometheus types for convenience
pub use prometheus::{Encoder, TextEncoder};
