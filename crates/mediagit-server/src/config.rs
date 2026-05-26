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

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[cfg(feature = "tls")]
use mediagit_security::TlsConfig;

/// Server configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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

    /// TTL (seconds) for presigned PUT URLs issued to clients for direct-to-bucket uploads.
    /// 12 hours by default; may need lowering if credentials use short-lived STS sessions.
    #[serde(default = "default_presigned_url_ttl")]
    pub presigned_url_ttl_seconds: u64,

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

fn default_presigned_url_ttl() -> u64 {
    43200 // 12 hours
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
            presigned_url_ttl_seconds: default_presigned_url_ttl(),
            enable_rate_limiting: false,
            rate_limit_rps: default_rate_limit_rps(),
            rate_limit_burst: default_rate_limit_burst(),
        }
    }
}

impl ServerConfig {
    /// Load configuration from the given path, or use defaults if the file does not exist.
    ///
    /// If `config_path` differs from the default ("mediagit-server.toml") and the file
    /// is missing, return an error instead of silently falling back — this prevents
    /// operators from thinking their S3/TLS/auth config is wired when it isn't.
    pub fn load(config_path: &str) -> Result<Self> {
        let path = PathBuf::from(config_path);
        let is_default = config_path == "mediagit-server.toml";

        if path.exists() {
            let content = std::fs::read_to_string(&path).context("Failed to read config file")?;

            toml::from_str(&content).with_context(|| {
                format!(
                    "Failed to parse config file {} (unknown keys are rejected; \
                     check for typos or deprecated sections)",
                    path.display()
                )
            })
        } else if is_default {
            tracing::warn!(
                "No config file at '{}'; using built-in defaults \
                 (port=3000, host=127.0.0.1, auth=off, rate_limit=off)",
                path.display()
            );
            Ok(Self::default())
        } else {
            anyhow::bail!(
                "config file not found: {} (specified via --config)",
                path.display()
            )
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
            let cert_path = self
                .tls_cert_path
                .as_ref()
                .context("TLS certificate path is required when not using self-signed")?;
            let key_path = self
                .tls_key_path
                .as_ref()
                .context("TLS key path is required when not using self-signed")?;

            builder = builder.certificate_paths(cert_path, key_path);
        }

        builder.build().context("Failed to build TLS configuration")
    }

    /// Build TlsConfig (stub for non-TLS builds)
    #[cfg(not(feature = "tls"))]
    pub fn build_tls_config(&self) -> Result<()> {
        if self.enable_tls {
            anyhow::bail!(
                "TLS is enabled in configuration but not compiled in. Rebuild with --features tls"
            );
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rejects_unknown_top_level_keys() {
        // Guard for BUG-004: unknown keys/sections must not be silently dropped.
        let toml_str = r#"
            port = 5061
            [storage]
            backend = "s3"
        "#;
        let err = toml::from_str::<ServerConfig>(toml_str)
            .expect_err("unknown [storage] section must not parse");
        let msg = err.to_string();
        assert!(
            msg.contains("storage") || msg.contains("unknown"),
            "expected unknown-field rejection, got: {msg}"
        );
    }

    #[test]
    fn test_rejects_unknown_leaf_key() {
        let toml_str = r#"
            port = 5061
            enable_s3 = true
        "#;
        let err = toml::from_str::<ServerConfig>(toml_str)
            .expect_err("unknown `enable_s3` must not parse");
        let msg = err.to_string();
        assert!(
            msg.contains("enable_s3") || msg.contains("unknown"),
            "expected unknown-field rejection, got: {msg}"
        );
    }

    #[test]
    fn test_load_explicit_path_missing_bails() {
        let err = ServerConfig::load("nonexistent-config-for-bug-004.toml")
            .expect_err("explicit missing path must bail");
        let msg = err.to_string();
        assert!(
            msg.contains("not found") || msg.contains("nonexistent-config-for-bug-004"),
            "expected missing-path error, got: {msg}"
        );
    }

    #[test]
    fn test_load_default_path_missing_uses_defaults() {
        // Default path "mediagit-server.toml" is allowed to be missing
        // so first-run users get sensible defaults. Run from a temp dir
        // to guarantee the default file is not present.
        let tmp = tempfile::tempdir().expect("tempdir");
        let cwd = std::env::current_dir().expect("cwd");
        std::env::set_current_dir(tmp.path()).expect("chdir");
        let result = ServerConfig::load("mediagit-server.toml");
        std::env::set_current_dir(cwd).expect("restore cwd");
        let cfg = result.expect("default path missing must fall back to defaults");
        assert_eq!(cfg.port, 3000);
        assert!(!cfg.enable_auth);
    }
}
