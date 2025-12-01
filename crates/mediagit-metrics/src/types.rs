//! Common types for metrics collection

use serde::{Deserialize, Serialize};

/// Configuration for metrics server
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetricsConfig {
    /// Port for metrics HTTP server
    pub port: u16,

    /// Enable metrics collection
    pub enabled: bool,

    /// Bind address (default: 127.0.0.1)
    pub bind_address: String,
}

impl Default for MetricsConfig {
    fn default() -> Self {
        Self {
            port: 9090,
            enabled: false,
            bind_address: "127.0.0.1".to_string(),
        }
    }
}

impl MetricsConfig {
    /// Create new config with port
    pub fn with_port(port: u16) -> Self {
        Self {
            port,
            enabled: true,
            ..Default::default()
        }
    }

    /// Get bind address with port
    pub fn socket_addr(&self) -> String {
        format!("{}:{}", self.bind_address, self.port)
    }
}

/// Storage backend types for labeling metrics
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum StorageBackend {
    /// Local filesystem storage
    Filesystem,
    /// AWS S3
    S3,
    /// Azure Blob Storage
    AzureBlob,
    /// Google Cloud Storage
    Gcs,
    /// MinIO (S3-compatible)
    MinIO,
    /// Backblaze B2
    B2,
    /// DigitalOcean Spaces
    DoSpaces,
}

impl StorageBackend {
    /// Get string label for Prometheus
    pub fn as_label(&self) -> &'static str {
        match self {
            StorageBackend::Filesystem => "filesystem",
            StorageBackend::S3 => "s3",
            StorageBackend::AzureBlob => "azure_blob",
            StorageBackend::Gcs => "gcs",
            StorageBackend::MinIO => "minio",
            StorageBackend::B2 => "b2",
            StorageBackend::DoSpaces => "do_spaces",
        }
    }
}

/// Compression algorithm types for labeling metrics
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum CompressionAlgorithm {
    /// No compression
    None,
    /// Zstandard compression
    Zstd,
    /// Brotli compression
    Brotli,
}

impl CompressionAlgorithm {
    /// Get string label for Prometheus
    pub fn as_label(&self) -> &'static str {
        match self {
            CompressionAlgorithm::None => "none",
            CompressionAlgorithm::Zstd => "zstd",
            CompressionAlgorithm::Brotli => "brotli",
        }
    }
}

/// Operation types for timing metrics
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum OperationType {
    /// Store operation
    Store,
    /// Retrieve operation
    Retrieve,
    /// Delete operation
    Delete,
    /// List operation
    List,
}

impl OperationType {
    /// Get string label for Prometheus
    pub fn as_label(&self) -> &'static str {
        match self {
            OperationType::Store => "store",
            OperationType::Retrieve => "retrieve",
            OperationType::Delete => "delete",
            OperationType::List => "list",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_metrics_config_default() {
        let config = MetricsConfig::default();
        assert_eq!(config.port, 9090);
        assert!(!config.enabled);
        assert_eq!(config.bind_address, "127.0.0.1");
    }

    #[test]
    fn test_metrics_config_with_port() {
        let config = MetricsConfig::with_port(8080);
        assert_eq!(config.port, 8080);
        assert!(config.enabled);
        assert_eq!(config.socket_addr(), "127.0.0.1:8080");
    }

    #[test]
    fn test_storage_backend_labels() {
        assert_eq!(StorageBackend::Filesystem.as_label(), "filesystem");
        assert_eq!(StorageBackend::S3.as_label(), "s3");
        assert_eq!(StorageBackend::AzureBlob.as_label(), "azure_blob");
    }

    #[test]
    fn test_compression_algorithm_labels() {
        assert_eq!(CompressionAlgorithm::None.as_label(), "none");
        assert_eq!(CompressionAlgorithm::Zstd.as_label(), "zstd");
        assert_eq!(CompressionAlgorithm::Brotli.as_label(), "brotli");
    }

    #[test]
    fn test_operation_type_labels() {
        assert_eq!(OperationType::Store.as_label(), "store");
        assert_eq!(OperationType::Retrieve.as_label(), "retrieve");
        assert_eq!(OperationType::Delete.as_label(), "delete");
    }
}
