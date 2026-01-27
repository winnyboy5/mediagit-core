use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Top-level configuration structure
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Config {
    /// Application metadata
    pub app: AppConfig,

    /// Storage backend configuration
    pub storage: StorageConfig,

    /// Compression settings
    pub compression: CompressionConfig,

    /// Performance tuning
    pub performance: PerformanceConfig,

    /// Observability settings
    pub observability: ObservabilityConfig,

    /// Security settings
    pub security: SecurityConfig,

    /// Remote repositories configuration
    #[serde(default)]
    pub remotes: HashMap<String, RemoteConfig>,

    /// Branch tracking configuration (upstream branches)
    #[serde(default)]
    pub branches: HashMap<String, BranchConfig>,

    /// Branch protection rules
    #[serde(default)]
    pub protected_branches: HashMap<String, BranchProtection>,

    /// Custom user-defined settings
    #[serde(default)]
    pub custom: HashMap<String, serde_json::Value>,
}

impl Config {
    /// Get remote URL by name
    pub fn get_remote_url(&self, remote_name: &str) -> Result<String, String> {
        self.remotes
            .get(remote_name)
            .map(|r| r.url.clone())
            .ok_or_else(|| format!("Remote '{}' not found in configuration", remote_name))
    }

    /// Add or update a remote
    pub fn set_remote(&mut self, name: impl Into<String>, url: impl Into<String>) {
        self.remotes
            .insert(name.into(), RemoteConfig::new(url.into()));
    }

    /// Remove a remote
    pub fn remove_remote(&mut self, name: &str) -> Option<RemoteConfig> {
        self.remotes.remove(name)
    }

    /// List all remote names
    pub fn list_remotes(&self) -> Vec<String> {
        self.remotes.keys().cloned().collect()
    }

    /// Load config from repository root
    pub async fn load(repo_root: impl AsRef<std::path::Path>) -> anyhow::Result<Self> {
        use crate::ConfigLoader;
        let config_path = repo_root.as_ref().join(".mediagit/config.toml");

        if !config_path.exists() {
            // Return default config if file doesn't exist
            return Ok(Self::default());
        }

        let loader = ConfigLoader::new();
        Ok(loader.load_file(&config_path).await?)
    }

    /// Save config to repository root
    pub fn save(&self, repo_root: impl AsRef<std::path::Path>) -> anyhow::Result<()> {
        let config_path = repo_root.as_ref().join(".mediagit/config.toml");
        
        // Create .mediagit directory if it doesn't exist
        if let Some(parent) = config_path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let toml_str = toml::to_string_pretty(self)?;
        std::fs::write(&config_path, toml_str)?;
        Ok(())
    }

    /// Get upstream tracking for a branch
    /// Returns (remote_name, remote_branch) if tracked
    pub fn get_branch_upstream(&self, branch: &str) -> Option<(&str, &str)> {
        self.branches.get(branch).map(|bc| (bc.remote.as_str(), bc.merge.as_str()))
    }

    /// Set upstream tracking for a branch
    pub fn set_branch_upstream(&mut self, branch: impl Into<String>, remote: impl Into<String>, merge: impl Into<String>) {
        self.branches.insert(branch.into(), BranchConfig::new(remote, merge));
    }

    /// Remove upstream tracking for a branch
    pub fn remove_branch_upstream(&mut self, branch: &str) -> Option<BranchConfig> {
        self.branches.remove(branch)
    }

    /// Check if a branch is protected
    pub fn is_branch_protected(&self, branch: &str) -> bool {
        self.protected_branches.contains_key(branch)
    }

    /// Get protection rules for a branch
    pub fn get_branch_protection(&self, branch: &str) -> Option<&BranchProtection> {
        self.protected_branches.get(branch)
    }

    /// Protect a branch with default rules
    pub fn protect_branch(&mut self, branch: impl Into<String>) {
        self.protected_branches.insert(branch.into(), BranchProtection::default_protection());
    }

    /// Protect a branch with custom rules
    pub fn protect_branch_with(&mut self, branch: impl Into<String>, protection: BranchProtection) {
        self.protected_branches.insert(branch.into(), protection);
    }

    /// Unprotect a branch
    pub fn unprotect_branch(&mut self, branch: &str) -> Option<BranchProtection> {
        self.protected_branches.remove(branch)
    }

    /// List all protected branches
    pub fn list_protected_branches(&self) -> Vec<&String> {
        self.protected_branches.keys().collect()
    }
}

/// Application metadata
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AppConfig {
    /// Application name
    #[serde(default = "default_app_name")]
    pub name: String,

    /// Application version
    #[serde(default = "default_app_version")]
    pub version: String,

    /// Environment (development, staging, production)
    #[serde(default = "default_environment")]
    pub environment: String,

    /// API server port
    #[serde(default = "default_port")]
    pub port: u16,

    /// API server host
    #[serde(default = "default_host")]
    pub host: String,

    /// Enable debug mode
    #[serde(default)]
    pub debug: bool,
}

/// Storage backend configuration
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "backend")]
pub enum StorageConfig {
    /// Filesystem storage
    #[serde(rename = "filesystem")]
    FileSystem(FileSystemStorage),

    /// AWS S3 storage
    #[serde(rename = "s3")]
    S3(S3Storage),

    /// Azure Blob Storage
    #[serde(rename = "azure")]
    Azure(AzureStorage),

    /// Google Cloud Storage
    #[serde(rename = "gcs")]
    GCS(GCSStorage),

    /// Multi-backend configuration
    #[serde(rename = "multi")]
    Multi(MultiBackendStorage),
}

/// Filesystem storage configuration
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FileSystemStorage {
    /// Base directory path
    pub base_path: String,

    /// Create directories if they don't exist
    #[serde(default = "default_true")]
    pub create_dirs: bool,

    /// Sync writes to disk
    #[serde(default)]
    pub sync: bool,

    /// File permissions (octal string like "0755")
    #[serde(default = "default_file_permissions")]
    pub file_permissions: String,
}

/// AWS S3 storage configuration
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct S3Storage {
    /// S3 bucket name
    pub bucket: String,

    /// AWS region
    pub region: String,

    /// AWS access key ID (can be overridden via env)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub access_key_id: Option<String>,

    /// AWS secret access key (can be overridden via env)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub secret_access_key: Option<String>,

    /// S3 endpoint (for S3-compatible services)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub endpoint: Option<String>,

    /// Object prefix
    #[serde(default)]
    pub prefix: String,

    /// Enable server-side encryption
    #[serde(default)]
    pub encryption: bool,

    /// Encryption algorithm (AES256, aws:kms)
    #[serde(default = "default_encryption_algorithm")]
    pub encryption_algorithm: String,
}

/// Azure Blob Storage configuration
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AzureStorage {
    /// Storage account name
    pub account_name: String,

    /// Storage account key (can be overridden via env)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub account_key: Option<String>,

    /// Container name
    pub container: String,

    /// Blob path prefix
    #[serde(default)]
    pub prefix: String,

    /// Connection string (alternative to account_name/account_key)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub connection_string: Option<String>,
}

/// Google Cloud Storage configuration
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct GCSStorage {
    /// GCS bucket name
    pub bucket: String,

    /// Project ID
    pub project_id: String,

    /// Credentials file path (can be overridden via env)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub credentials_path: Option<String>,

    /// Object prefix
    #[serde(default)]
    pub prefix: String,
}

/// Multi-backend storage configuration
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MultiBackendStorage {
    /// Primary backend name
    pub primary: String,

    /// Replica backends
    #[serde(default)]
    pub replicas: Vec<String>,

    /// Individual backend configurations
    pub backends: HashMap<String, serde_json::Value>,
}

/// Compression configuration
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CompressionConfig {
    /// Enable compression by default
    #[serde(default = "default_true")]
    pub enabled: bool,

    /// Default compression algorithm
    #[serde(default = "default_algorithm")]
    pub algorithm: CompressionAlgorithm,

    /// Default compression level (1-22 for zstd, 1-11 for brotli)
    #[serde(default = "default_level")]
    pub level: u32,

    /// Minimum file size for compression (in bytes)
    #[serde(default = "default_min_size")]
    pub min_size: u64,

    /// Algorithm-specific settings
    #[serde(default)]
    pub algorithms: HashMap<String, AlgorithmConfig>,
}

/// Supported compression algorithms
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "lowercase")]
pub enum CompressionAlgorithm {
    Zstd,
    Brotli,
    None,
}

/// Algorithm-specific configuration
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AlgorithmConfig {
    /// Compression level
    #[serde(skip_serializing_if = "Option::is_none")]
    pub level: Option<u32>,

    /// Additional options (algorithm-specific)
    #[serde(default)]
    pub options: HashMap<String, serde_json::Value>,
}

/// Performance tuning configuration
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PerformanceConfig {
    /// Maximum concurrent operations
    #[serde(default = "default_max_concurrency")]
    pub max_concurrency: usize,

    /// Buffer size for I/O operations (in bytes)
    #[serde(default = "default_buffer_size")]
    pub buffer_size: usize,

    /// Cache configuration
    pub cache: CacheConfig,

    /// Connection pool settings
    pub connection_pool: ConnectionPoolConfig,

    /// Timeout settings (in seconds)
    pub timeouts: TimeoutConfig,
}

/// Cache configuration
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CacheConfig {
    /// Enable caching
    #[serde(default = "default_true")]
    pub enabled: bool,

    /// Cache type (memory, disk, redis)
    #[serde(default = "default_cache_type")]
    pub cache_type: String,

    /// Maximum cache size (in bytes)
    #[serde(default = "default_cache_size")]
    pub max_size: u64,

    /// Cache TTL (in seconds)
    #[serde(default = "default_cache_ttl")]
    pub ttl: u64,

    /// Enable compression in cache
    #[serde(default)]
    pub compression: bool,
}

/// Connection pool configuration
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ConnectionPoolConfig {
    /// Minimum pool size
    #[serde(default = "default_min_connections")]
    pub min_connections: usize,

    /// Maximum pool size
    #[serde(default = "default_max_connections")]
    pub max_connections: usize,

    /// Connection timeout (in seconds)
    #[serde(default = "default_connection_timeout")]
    pub timeout: u64,

    /// Idle connection timeout (in seconds)
    #[serde(default = "default_idle_timeout")]
    pub idle_timeout: u64,
}

/// Timeout configuration
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TimeoutConfig {
    /// Request timeout (in seconds)
    #[serde(default = "default_request_timeout")]
    pub request: u64,

    /// Read timeout (in seconds)
    #[serde(default = "default_read_timeout")]
    pub read: u64,

    /// Write timeout (in seconds)
    #[serde(default = "default_write_timeout")]
    pub write: u64,

    /// Connection timeout (in seconds)
    #[serde(default = "default_connection_timeout")]
    pub connection: u64,
}

/// Observability configuration
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ObservabilityConfig {
    /// Logging level
    #[serde(default = "default_log_level")]
    pub log_level: String,

    /// Log format (json, text)
    #[serde(default = "default_log_format")]
    pub log_format: String,

    /// Enable tracing
    #[serde(default = "default_true")]
    pub tracing_enabled: bool,

    /// Trace sample rate (0.0 to 1.0)
    #[serde(default = "default_sample_rate")]
    pub sample_rate: f64,

    /// Metrics configuration
    pub metrics: MetricsConfig,
}

/// Metrics configuration
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MetricsConfig {
    /// Enable metrics collection
    #[serde(default = "default_true")]
    pub enabled: bool,

    /// Metrics port
    #[serde(default = "default_metrics_port")]
    pub port: u16,

    /// Metrics endpoint path
    #[serde(default = "default_metrics_endpoint")]
    pub endpoint: String,

    /// Metrics collection interval (in seconds)
    #[serde(default = "default_metrics_interval")]
    pub interval: u64,
}

/// Security configuration
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SecurityConfig {
    /// Enable HTTPS
    #[serde(default)]
    pub https_enabled: bool,

    /// TLS certificate path
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tls_cert_path: Option<String>,

    /// TLS key path
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tls_key_path: Option<String>,

    /// API key for authentication
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_key: Option<String>,

    /// Enable authentication
    #[serde(default)]
    pub auth_enabled: bool,

    /// CORS allowed origins
    #[serde(default)]
    pub cors_origins: Vec<String>,

    /// Enable encryption at rest
    #[serde(default)]
    pub encryption_at_rest: bool,

    /// Encryption key path (can be overridden via env)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub encryption_key_path: Option<String>,

    /// Rate limiting configuration
    pub rate_limiting: RateLimitConfig,
}

/// Remote repository configuration
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RemoteConfig {
    /// Remote URL (e.g., "http://localhost:3000/repo-name")
    pub url: String,

    /// Fetch URL (if different from url)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fetch: Option<String>,

    /// Push URL (if different from url)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub push: Option<String>,

    /// Default fetch flag
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_fetch: Option<bool>,
}

impl RemoteConfig {
    /// Create a new remote configuration
    pub fn new(url: impl Into<String>) -> Self {
        Self {
            url: url.into(),
            fetch: None,
            push: None,
            default_fetch: Some(true),
        }
    }

    /// Get the effective fetch URL
    pub fn fetch_url(&self) -> &str {
        self.fetch.as_deref().unwrap_or(&self.url)
    }

    /// Get the effective push URL
    pub fn push_url(&self) -> &str {
        self.push.as_deref().unwrap_or(&self.url)
    }
}

/// Rate limiting configuration
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RateLimitConfig {
    /// Enable rate limiting
    #[serde(default)]
    pub enabled: bool,

    /// Requests per second
    #[serde(default = "default_rps")]
    pub requests_per_second: u32,

    /// Burst size
    #[serde(default = "default_burst")]
    pub burst_size: u32,
}

/// Branch tracking configuration (similar to Git's branch.<name>.remote and branch.<name>.merge)
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BranchConfig {
    /// The remote to push/pull from by default
    pub remote: String,

    /// The remote branch to merge from (e.g., "refs/heads/main")
    pub merge: String,
}

impl BranchConfig {
    /// Create a new branch tracking config
    pub fn new(remote: impl Into<String>, merge: impl Into<String>) -> Self {
        Self {
            remote: remote.into(),
            merge: merge.into(),
        }
    }
}

/// Branch protection rules
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct BranchProtection {
    /// Prevent force-push to this branch
    #[serde(default = "default_true")]
    pub prevent_force_push: bool,

    /// Prevent deletion of this branch
    #[serde(default = "default_true")]
    pub prevent_deletion: bool,

    /// Require pull request reviews before merge
    #[serde(default)]
    pub require_reviews: bool,

    /// Minimum number of approvals required (if require_reviews is true)
    #[serde(default = "default_min_approvals")]
    pub min_approvals: u32,
}

impl BranchProtection {
    /// Create default protection (prevent force-push and deletion)
    pub fn default_protection() -> Self {
        Self {
            prevent_force_push: true,
            prevent_deletion: true,
            require_reviews: false,
            min_approvals: 1,
        }
    }

    /// Create protection with review requirement
    pub fn with_reviews(min_approvals: u32) -> Self {
        Self {
            prevent_force_push: true,
            prevent_deletion: true,
            require_reviews: true,
            min_approvals,
        }
    }
}

fn default_min_approvals() -> u32 {
    1
}

// Default value functions
fn default_true() -> bool {
    true
}

fn default_app_name() -> String {
    "mediagit".to_string()
}

fn default_app_version() -> String {
    "0.1.0".to_string()
}

fn default_environment() -> String {
    "development".to_string()
}

fn default_port() -> u16 {
    8080
}

fn default_host() -> String {
    "127.0.0.1".to_string()
}

fn default_algorithm() -> CompressionAlgorithm {
    CompressionAlgorithm::Zstd
}

fn default_level() -> u32 {
    3
}

fn default_min_size() -> u64 {
    1024 // 1KB
}

fn default_file_permissions() -> String {
    "0644".to_string()
}

fn default_encryption_algorithm() -> String {
    "AES256".to_string()
}

fn default_max_concurrency() -> usize {
    num_cpus::get().max(4)
}

fn default_buffer_size() -> usize {
    65536 // 64KB
}

fn default_cache_type() -> String {
    "memory".to_string()
}

fn default_cache_size() -> u64 {
    536870912 // 512MB
}

fn default_cache_ttl() -> u64 {
    3600 // 1 hour
}

fn default_min_connections() -> usize {
    1
}

fn default_max_connections() -> usize {
    10
}

fn default_connection_timeout() -> u64 {
    30
}

fn default_idle_timeout() -> u64 {
    600
}

fn default_request_timeout() -> u64 {
    60
}

fn default_read_timeout() -> u64 {
    30
}

fn default_write_timeout() -> u64 {
    30
}

fn default_log_level() -> String {
    "info".to_string()
}

fn default_log_format() -> String {
    "json".to_string()
}

fn default_sample_rate() -> f64 {
    0.1
}

fn default_metrics_port() -> u16 {
    9090
}

fn default_metrics_endpoint() -> String {
    "/metrics".to_string()
}

fn default_metrics_interval() -> u64 {
    60
}

fn default_rps() -> u32 {
    100
}

fn default_burst() -> u32 {
    200
}

impl Default for Config {
    fn default() -> Self {
        Config {
            app: AppConfig::default(),
            storage: StorageConfig::FileSystem(FileSystemStorage::default()),
            compression: CompressionConfig::default(),
            performance: PerformanceConfig::default(),
            observability: ObservabilityConfig::default(),
            security: SecurityConfig::default(),
            remotes: HashMap::new(),
            branches: HashMap::new(),
            protected_branches: HashMap::new(),
            custom: HashMap::new(),
        }
    }
}

impl Default for AppConfig {
    fn default() -> Self {
        AppConfig {
            name: default_app_name(),
            version: default_app_version(),
            environment: default_environment(),
            port: default_port(),
            host: default_host(),
            debug: false,
        }
    }
}

impl Default for FileSystemStorage {
    fn default() -> Self {
        FileSystemStorage {
            base_path: "./data".to_string(),
            create_dirs: true,
            sync: false,
            file_permissions: "0644".to_string(),
        }
    }
}

impl Default for CompressionConfig {
    fn default() -> Self {
        CompressionConfig {
            enabled: true,
            algorithm: CompressionAlgorithm::Zstd,
            level: 3,
            min_size: 1024,
            algorithms: HashMap::new(),
        }
    }
}

impl Default for PerformanceConfig {
    fn default() -> Self {
        PerformanceConfig {
            max_concurrency: default_max_concurrency(),
            buffer_size: 65536,
            cache: CacheConfig::default(),
            connection_pool: ConnectionPoolConfig::default(),
            timeouts: TimeoutConfig::default(),
        }
    }
}

impl Default for CacheConfig {
    fn default() -> Self {
        CacheConfig {
            enabled: true,
            cache_type: "memory".to_string(),
            max_size: 536870912,
            ttl: 3600,
            compression: false,
        }
    }
}

impl Default for ConnectionPoolConfig {
    fn default() -> Self {
        ConnectionPoolConfig {
            min_connections: 1,
            max_connections: 10,
            timeout: 30,
            idle_timeout: 600,
        }
    }
}

impl Default for TimeoutConfig {
    fn default() -> Self {
        TimeoutConfig {
            request: 60,
            read: 30,
            write: 30,
            connection: 30,
        }
    }
}

impl Default for ObservabilityConfig {
    fn default() -> Self {
        ObservabilityConfig {
            log_level: "info".to_string(),
            log_format: "json".to_string(),
            tracing_enabled: true,
            sample_rate: 0.1,
            metrics: MetricsConfig::default(),
        }
    }
}

impl Default for MetricsConfig {
    fn default() -> Self {
        MetricsConfig {
            enabled: true,
            port: 9090,
            endpoint: "/metrics".to_string(),
            interval: 60,
        }
    }
}

impl Default for SecurityConfig {
    fn default() -> Self {
        SecurityConfig {
            https_enabled: false,
            tls_cert_path: None,
            tls_key_path: None,
            api_key: None,
            auth_enabled: false,
            cors_origins: vec!["http://localhost:3000".to_string()],
            encryption_at_rest: false,
            encryption_key_path: None,
            rate_limiting: RateLimitConfig::default(),
        }
    }
}

impl Default for RateLimitConfig {
    fn default() -> Self {
        RateLimitConfig {
            enabled: false,
            requests_per_second: 100,
            burst_size: 200,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = Config::default();
        assert_eq!(config.app.name, "mediagit");
        assert_eq!(config.app.port, 8080);
        assert!(config.compression.enabled);
    }

    #[test]
    fn test_serialization_roundtrip() {
        let config = Config::default();
        let json = serde_json::to_string(&config).unwrap();
        let deserialized: Config = serde_json::from_str(&json).unwrap();
        assert_eq!(config, deserialized);
    }
}
