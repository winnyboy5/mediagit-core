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
//! TLS configuration management
//!
//! Manages TLS settings, certificate paths, and security parameters.

use super::{Certificate, TlsError, TlsResult};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// TLS configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TlsConfig {
    /// Enable TLS/HTTPS
    pub enabled: bool,

    /// Certificate file path (PEM format)
    pub cert_path: Option<PathBuf>,

    /// Private key file path (PEM format)
    pub key_path: Option<PathBuf>,

    /// Use self-signed certificate for development
    pub use_self_signed: bool,

    /// Self-signed certificate common name
    pub self_signed_common_name: String,

    /// Minimum TLS version (1.2 or 1.3)
    pub min_tls_version: TlsVersion,

    /// Require client certificates (mTLS)
    pub require_client_cert: bool,

    /// Client CA certificate path for mTLS
    pub client_ca_path: Option<PathBuf>,
}

impl Default for TlsConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            cert_path: None,
            key_path: None,
            use_self_signed: false,
            self_signed_common_name: "localhost".to_string(),
            min_tls_version: TlsVersion::V1_3,
            require_client_cert: false,
            client_ca_path: None,
        }
    }
}

/// TLS version
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum TlsVersion {
    /// TLS 1.2
    V1_2,
    /// TLS 1.3
    V1_3,
}

impl TlsConfig {
    /// Create a new TLS configuration
    pub fn new() -> Self {
        Self::default()
    }

    /// Enable TLS
    pub fn enable(&mut self) -> &mut Self {
        self.enabled = true;
        self
    }

    /// Disable TLS
    pub fn disable(&mut self) -> &mut Self {
        self.enabled = false;
        self
    }

    /// Set certificate paths
    pub fn with_certificate_paths(
        &mut self,
        cert_path: impl Into<PathBuf>,
        key_path: impl Into<PathBuf>,
    ) -> &mut Self {
        self.cert_path = Some(cert_path.into());
        self.key_path = Some(key_path.into());
        self.use_self_signed = false;
        self
    }

    /// Use self-signed certificate for development
    pub fn with_self_signed(&mut self, common_name: impl Into<String>) -> &mut Self {
        self.use_self_signed = true;
        self.self_signed_common_name = common_name.into();
        self.cert_path = None;
        self.key_path = None;
        self
    }

    /// Set minimum TLS version
    pub fn with_min_tls_version(&mut self, version: TlsVersion) -> &mut Self {
        self.min_tls_version = version;
        self
    }

    /// Enable mutual TLS (mTLS) with client certificates
    pub fn with_mtls(&mut self, client_ca_path: impl Into<PathBuf>) -> &mut Self {
        self.require_client_cert = true;
        self.client_ca_path = Some(client_ca_path.into());
        self
    }

    /// Validate configuration
    pub fn validate(&self) -> TlsResult<()> {
        if !self.enabled {
            return Ok(());
        }

        if self.use_self_signed {
            // Self-signed is always valid for development
            return Ok(());
        }

        // For production certificates, require paths
        if self.cert_path.is_none() || self.key_path.is_none() {
            return Err(TlsError::CertificateValidation(
                "Certificate and key paths required when not using self-signed".to_string(),
            ));
        }

        // Verify files exist
        if let Some(cert_path) = &self.cert_path {
            if !cert_path.exists() {
                return Err(TlsError::CertificateLoading(
                    format!("Certificate file not found: {}", cert_path.display()),
                ));
            }
        }

        if let Some(key_path) = &self.key_path {
            if !key_path.exists() {
                return Err(TlsError::CertificateLoading(
                    format!("Key file not found: {}", key_path.display()),
                ));
            }
        }

        // If mTLS enabled, verify CA certificate exists
        if self.require_client_cert {
            if let Some(ca_path) = &self.client_ca_path {
                if !ca_path.exists() {
                    return Err(TlsError::CertificateLoading(
                        format!("Client CA file not found: {}", ca_path.display()),
                    ));
                }
            } else {
                return Err(TlsError::CertificateValidation(
                    "Client CA path required for mTLS".to_string(),
                ));
            }
        }

        Ok(())
    }

    /// Load certificate based on configuration
    #[cfg(feature = "tls")]
    pub fn load_certificate(&self) -> TlsResult<Certificate> {
        use super::CertificateBuilder;

        if self.use_self_signed {
            // Generate self-signed certificate
            CertificateBuilder::new(&self.self_signed_common_name)
                .add_san_dns("localhost")
                .add_san_dns("127.0.0.1")
                .add_san_ip("127.0.0.1")
                .add_san_ip("::1")
                .generate_self_signed()
        } else {
            // Load from files
            let cert_path = self.cert_path.as_ref().ok_or_else(|| {
                TlsError::CertificateLoading("Certificate path not set".to_string())
            })?;

            let key_path = self.key_path.as_ref().ok_or_else(|| {
                TlsError::CertificateLoading("Key path not set".to_string())
            })?;

            Certificate::from_pem_files(cert_path, key_path)
        }
    }

    /// Load certificate (stub for non-TLS feature)
    #[cfg(not(feature = "tls"))]
    pub fn load_certificate(&self) -> TlsResult<Certificate> {
        Err(TlsError::CertificateGeneration(
            "TLS feature not enabled".to_string(),
        ))
    }
}

/// TLS configuration builder
pub struct TlsConfigBuilder {
    config: TlsConfig,
}

impl TlsConfigBuilder {
    /// Create a new builder
    pub fn new() -> Self {
        Self {
            config: TlsConfig::default(),
        }
    }

    /// Enable TLS
    pub fn enable(mut self) -> Self {
        self.config.enabled = true;
        self
    }

    /// Set certificate paths
    pub fn certificate_paths(
        mut self,
        cert_path: impl Into<PathBuf>,
        key_path: impl Into<PathBuf>,
    ) -> Self {
        self.config.cert_path = Some(cert_path.into());
        self.config.key_path = Some(key_path.into());
        self.config.use_self_signed = false;
        self
    }

    /// Use self-signed certificate
    pub fn self_signed(mut self, common_name: impl Into<String>) -> Self {
        self.config.use_self_signed = true;
        self.config.self_signed_common_name = common_name.into();
        self
    }

    /// Set minimum TLS version
    pub fn min_tls_version(mut self, version: TlsVersion) -> Self {
        self.config.min_tls_version = version;
        self
    }

    /// Enable mTLS
    pub fn mtls(mut self, client_ca_path: impl Into<PathBuf>) -> Self {
        self.config.require_client_cert = true;
        self.config.client_ca_path = Some(client_ca_path.into());
        self
    }

    /// Build the configuration
    pub fn build(self) -> TlsResult<TlsConfig> {
        self.config.validate()?;
        Ok(self.config)
    }
}

impl Default for TlsConfigBuilder {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = TlsConfig::default();
        assert!(!config.enabled);
        assert!(!config.use_self_signed);
        assert_eq!(config.min_tls_version, TlsVersion::V1_3);
    }

    #[test]
    fn test_self_signed_config() {
        let config = TlsConfig::new().with_self_signed("test.example.com").clone();
        assert!(config.use_self_signed);
        assert_eq!(config.self_signed_common_name, "test.example.com");
        assert!(config.validate().is_ok());
    }

    #[test]
    fn test_config_builder() {
        let config = TlsConfigBuilder::new()
            .enable()
            .self_signed("builder-test.example.com")
            .min_tls_version(TlsVersion::V1_2)
            .build()
            .unwrap();

        assert!(config.enabled);
        assert!(config.use_self_signed);
        assert_eq!(config.self_signed_common_name, "builder-test.example.com");
        assert_eq!(config.min_tls_version, TlsVersion::V1_2);
    }

    #[test]
    fn test_validation_requires_paths() {
        let mut config = TlsConfig::new();
        config.enable();
        // Not using self-signed, no paths provided
        assert!(config.validate().is_err());
    }

    #[test]
    #[cfg(feature = "tls")]
    fn test_load_self_signed_certificate() {
        let config = TlsConfigBuilder::new()
            .enable()
            .self_signed("load-test.example.com")
            .build()
            .unwrap();

        let cert = config.load_certificate().unwrap();
        assert!(cert.cert_pem.contains("-----BEGIN CERTIFICATE-----"));
        assert!(cert.key_pem.contains("-----BEGIN PRIVATE KEY-----"));
    }
}
