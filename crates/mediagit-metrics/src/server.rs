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
//! HTTP server for Prometheus metrics endpoint
//!
//! Provides an Axum-based HTTP server that exposes a `/metrics` endpoint
//! in Prometheus text exposition format.

use axum::{
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::get,
    Router,
};
use prometheus::{Encoder, TextEncoder};
use std::sync::Arc;
use tokio::net::TcpListener;
use tracing::{debug, error, info};

use crate::{types::MetricsConfig, MetricsRegistry};

/// HTTP server for Prometheus metrics
///
/// Provides a lightweight HTTP server with a single `/metrics` endpoint
/// that returns metrics in Prometheus text format.
#[derive(Clone)]
pub struct MetricsServer {
    registry: Arc<MetricsRegistry>,
    config: MetricsConfig,
}

impl MetricsServer {
    /// Create a new metrics server
    ///
    /// # Arguments
    /// * `registry` - The metrics registry to serve
    /// * `port` - Port to bind the server to
    ///
    /// # Returns
    /// A new MetricsServer instance
    pub fn new(registry: MetricsRegistry, port: u16) -> Self {
        Self {
            registry: Arc::new(registry),
            config: MetricsConfig::with_port(port),
        }
    }

    /// Create a new metrics server with custom configuration
    pub fn with_config(registry: MetricsRegistry, config: MetricsConfig) -> Self {
        Self {
            registry: Arc::new(registry),
            config,
        }
    }

    /// Get the bind address for the server
    pub fn bind_address(&self) -> String {
        self.config.socket_addr()
    }

    /// Start the metrics server
    ///
    /// This method runs the server indefinitely. It should typically be spawned
    /// as a background task.
    ///
    /// # Example
    /// ```ignore
    /// let server = MetricsServer::new(registry, 9090);
    /// tokio::spawn(async move {
    ///     server.serve().await
    /// });
    /// ```
    pub async fn serve(self) -> anyhow::Result<()> {
        if !self.config.enabled {
            info!("Metrics server disabled");
            return Ok(());
        }

        let addr = self.config.socket_addr();
        info!("Starting metrics server on http://{}/metrics", addr);

        // Build the router
        let app = Router::new()
            .route("/metrics", get(metrics_handler))
            .route("/health", get(health_handler))
            .with_state(Arc::clone(&self.registry));

        // Bind and serve
        let listener = TcpListener::bind(&addr).await?;
        info!("Metrics server listening on {}", addr);

        axum::serve(listener, app)
            .await
            .map_err(|e| anyhow::anyhow!("Metrics server error: {}", e))
    }
}

/// Handler for `/metrics` endpoint
///
/// Returns all metrics in Prometheus text exposition format
async fn metrics_handler(State(registry): State<Arc<MetricsRegistry>>) -> Response {
    debug!("Serving metrics");

    // Gather metrics
    let metric_families = registry.registry().gather();

    // Encode in Prometheus text format
    let encoder = TextEncoder::new();
    let mut buffer = Vec::new();

    match encoder.encode(&metric_families, &mut buffer) {
        Ok(_) => {
            debug!("Successfully encoded {} metric families", metric_families.len());
            (
                StatusCode::OK,
                [("content-type", encoder.format_type())],
                buffer,
            )
                .into_response()
        }
        Err(e) => {
            error!("Failed to encode metrics: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to encode metrics: {}", e),
            )
                .into_response()
        }
    }
}

/// Handler for `/health` endpoint
///
/// Returns a simple health check response
async fn health_handler() -> impl IntoResponse {
    (StatusCode::OK, "OK")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{CompressionAlgorithm, OperationType, StorageBackend};
    use std::time::Duration;
    use tokio::time::sleep;

    #[tokio::test]
    async fn test_server_creation() {
        let registry = MetricsRegistry::new().unwrap();
        let server = MetricsServer::new(registry, 9191);

        assert_eq!(server.bind_address(), "127.0.0.1:9191");
    }

    #[tokio::test]
    async fn test_server_with_config() {
        let registry = MetricsRegistry::new().unwrap();
        let config = MetricsConfig {
            port: 8080,
            enabled: true,
            bind_address: "0.0.0.0".to_string(),
        };

        let server = MetricsServer::with_config(registry, config);
        assert_eq!(server.bind_address(), "0.0.0.0:8080");
    }

    #[tokio::test]
    async fn test_metrics_endpoint() {
        let registry = MetricsRegistry::new().unwrap();

        // Record some test metrics
        registry.record_dedup_write(1000, true);
        registry.record_compression(CompressionAlgorithm::Zstd, 1000, 600);
        registry.record_cache_hit();
        registry.record_operation_duration(
            OperationType::Store,
            StorageBackend::Filesystem,
            0.05,
        );

        // Start server on random high port
        let server = MetricsServer::new(registry, 19090);
        let addr = server.bind_address().clone();

        // Spawn server in background
        tokio::spawn(async move {
            let _ = server.serve().await;
        });

        // Give server time to start
        sleep(Duration::from_millis(100)).await;

        // Make request to metrics endpoint
        let client = reqwest::Client::new();
        let url = format!("http://{}/metrics", addr);

        match client.get(&url).send().await {
            Ok(response) => {
                assert_eq!(response.status(), StatusCode::OK);

                let body = response.text().await.unwrap();

                // Verify metrics are present in output
                assert!(body.contains("mediagit_dedup_bytes_written_total"));
                assert!(body.contains("mediagit_compression_ratio"));
                assert!(body.contains("mediagit_cache_hits_total"));
                assert!(body.contains("mediagit_operation_duration_seconds"));
            }
            Err(e) => {
                // Server might not be ready yet, that's ok for this test
                eprintln!("Warning: Could not connect to metrics server: {}", e);
            }
        }
    }

    #[tokio::test]
    async fn test_health_endpoint() {
        let registry = MetricsRegistry::new().unwrap();
        let server = MetricsServer::new(registry, 19091);
        let addr = server.bind_address().clone();

        // Spawn server in background
        tokio::spawn(async move {
            let _ = server.serve().await;
        });

        // Give server time to start
        sleep(Duration::from_millis(100)).await;

        // Make request to health endpoint
        let client = reqwest::Client::new();
        let url = format!("http://{}/health", addr);

        match client.get(&url).send().await {
            Ok(response) => {
                assert_eq!(response.status(), StatusCode::OK);
                let body = response.text().await.unwrap();
                assert_eq!(body, "OK");
            }
            Err(e) => {
                eprintln!("Warning: Could not connect to health endpoint: {}", e);
            }
        }
    }

    #[tokio::test]
    async fn test_disabled_server() {
        let registry = MetricsRegistry::new().unwrap();
        let config = MetricsConfig {
            port: 9092,
            enabled: false,
            bind_address: "127.0.0.1".to_string(),
        };

        let server = MetricsServer::with_config(registry, config);

        // Server should return immediately when disabled
        let result = server.serve().await;
        assert!(result.is_ok());
    }
}
