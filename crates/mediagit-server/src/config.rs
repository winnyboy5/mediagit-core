use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[cfg(feature = "tls")]
use mediagit_security::TlsConfig;

/// Server configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerConfig {
    /// Port to listen on (HTTP)
    #[serde(default = "default_port")]
    pub port: u16,

    /// Directory containing repositories
    #[serde(default = "default_repos_dir")]
    pub repos_dir: PathBuf,

    /// Host to bind to
    #[serde(default = "default_host")]
    pub host: String,

    /// Enable HTTPS/TLS
    #[serde(default)]
    pub enable_tls: bool,

    /// HTTPS port (when TLS is enabled)
    #[serde(default = "default_tls_port")]
    pub tls_port: u16,

    /// TLS certificate file path (PEM format)
    pub tls_cert_path: Option<PathBuf>,

    /// TLS private key file path (PEM format)
    pub tls_key_path: Option<PathBuf>,

    /// Use self-signed certificate for development
    #[serde(default)]
    pub tls_self_signed: bool,

    /// Enable authentication
    #[serde(default)]
    pub enable_auth: bool,

    /// JWT secret key (required when enable_auth = true)
    pub jwt_secret: Option<String>,

    /// Enable rate limiting
    #[serde(default)]
    pub enable_rate_limiting: bool,

    /// Rate limiting: requests per second
    #[serde(default = "default_rate_limit_rps")]
    pub rate_limit_rps: u64,

    /// Rate limiting: burst size
    #[serde(default = "default_rate_limit_burst")]
    pub rate_limit_burst: u32,
}

fn default_port() -> u16 {
    3000
}

fn default_tls_port() -> u16 {
    3443
}

fn default_host() -> String {
    "127.0.0.1".to_string()
}

fn default_repos_dir() -> PathBuf {
    PathBuf::from("./repos")
}

fn default_rate_limit_rps() -> u64 {
    10 // 10 requests per second
}

fn default_rate_limit_burst() -> u32 {
    20 // Allow bursts up to 20 requests
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            port: default_port(),
            repos_dir: default_repos_dir(),
            host: default_host(),
            enable_tls: false,
            tls_port: default_tls_port(),
            tls_cert_path: None,
            tls_key_path: None,
            tls_self_signed: false,
            enable_auth: false,
            jwt_secret: None,
            enable_rate_limiting: false,
            rate_limit_rps: default_rate_limit_rps(),
            rate_limit_burst: default_rate_limit_burst(),
        }
    }
}

impl ServerConfig {
    /// Load configuration from the given path, or use defaults if the file does not exist
    pub fn load(config_path: &str) -> Result<Self> {
        let config_path = PathBuf::from(config_path);

        if config_path.exists() {
            let content = std::fs::read_to_string(&config_path)
                .context("Failed to read config file")?;

            toml::from_str(&content).context("Failed to parse config file")
        } else {
            // Use defaults
            tracing::info!("No config file found, using defaults");
            Ok(Self::default())
        }
    }

    /// Get the full bind address
    pub fn bind_addr(&self) -> String {
        format!("{}:{}", self.host, self.port)
    }

    /// Get the full TLS bind address
    pub fn tls_bind_addr(&self) -> String {
        format!("{}:{}", self.host, self.tls_port)
    }

    /// Build TlsConfig from server configuration
    #[cfg(feature = "tls")]
    pub fn build_tls_config(&self) -> Result<TlsConfig> {
        use mediagit_security::TlsConfigBuilder;

        if !self.enable_tls {
            return Ok(TlsConfig::default());
        }

        let mut builder = TlsConfigBuilder::new().enable();

        if self.tls_self_signed {
            // Use self-signed certificate for development
            builder = builder.self_signed("localhost");
        } else {
            // Use provided certificate paths
            let cert_path = self.tls_cert_path.as_ref()
                .context("TLS certificate path is required when not using self-signed")?;
            let key_path = self.tls_key_path.as_ref()
                .context("TLS key path is required when not using self-signed")?;

            builder = builder.certificate_paths(cert_path, key_path);
        }

        builder.build().context("Failed to build TLS configuration")
    }

    /// Build TlsConfig (stub for non-TLS builds)
    #[cfg(not(feature = "tls"))]
    pub fn build_tls_config(&self) -> Result<()> {
        if self.enable_tls {
            anyhow::bail!("TLS is enabled in configuration but not compiled in. Rebuild with --features tls");
        }
        Ok(())
    }
}
