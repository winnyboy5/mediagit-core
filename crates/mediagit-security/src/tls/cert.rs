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
//! Certificate generation and management
//!
//! Handles self-signed certificate generation for development
//! and certificate loading from PEM files for production.

use super::{TlsError, TlsResult};
use std::path::Path;
use thiserror::Error;

#[cfg(feature = "tls")]
use rcgen::{CertificateParams, DistinguishedName, DnType};

/// Certificate error types
#[derive(Debug, Error)]
pub enum CertificateError {
    /// Generation failed
    #[error("Certificate generation failed: {0}")]
    GenerationFailed(String),

    /// Loading failed
    #[error("Certificate loading failed: {0}")]
    LoadingFailed(String),

    /// Validation failed
    #[error("Certificate validation failed: {0}")]
    ValidationFailed(String),

    /// Certificate expired
    #[error("Certificate expired")]
    Expired,
}

/// Certificate representation
#[derive(Debug, Clone)]
pub struct Certificate {
    /// Certificate in PEM format
    pub cert_pem: String,

    /// Private key in PEM format
    pub key_pem: String,

    /// Subject common name
    pub common_name: String,

    /// Organization name
    pub organization: Option<String>,

    /// Certificate validity period in days
    pub validity_days: u32,
}

impl Certificate {
    /// Create a new certificate from PEM strings
    pub fn new(cert_pem: String, key_pem: String, common_name: String) -> Self {
        Self {
            cert_pem,
            key_pem,
            common_name,
            organization: None,
            validity_days: 365,
        }
    }

    /// Load certificate from PEM files
    #[cfg(feature = "tls")]
    pub fn from_pem_files<P: AsRef<Path>>(
        cert_path: P,
        key_path: P,
    ) -> TlsResult<Self> {
        let cert_pem = std::fs::read_to_string(cert_path.as_ref())
            .map_err(|e| TlsError::CertificateLoading(e.to_string()))?;

        let key_pem = std::fs::read_to_string(key_path.as_ref())
            .map_err(|e| TlsError::CertificateLoading(e.to_string()))?;

        // Extract common name from certificate (simplified - in production would use x509-parser)
        let common_name = "loaded-certificate".to_string();

        Ok(Self {
            cert_pem,
            key_pem,
            common_name,
            organization: None,
            validity_days: 365,
        })
    }

    /// Save certificate to PEM files
    pub fn save_to_files<P: AsRef<Path>>(
        &self,
        cert_path: P,
        key_path: P,
    ) -> TlsResult<()> {
        std::fs::write(cert_path.as_ref(), &self.cert_pem)
            .map_err(TlsError::Io)?;

        std::fs::write(key_path.as_ref(), &self.key_pem)
            .map_err(TlsError::Io)?;

        Ok(())
    }

    /// Validate certificate (basic validation)
    pub fn validate(&self) -> TlsResult<()> {
        if self.cert_pem.is_empty() || self.key_pem.is_empty() {
            return Err(TlsError::CertificateValidation(
                "Certificate or key is empty".to_string(),
            ));
        }

        // Basic PEM format validation
        if !self.cert_pem.contains("-----BEGIN CERTIFICATE-----") {
            return Err(TlsError::InvalidPemFormat(
                "Missing certificate header".to_string(),
            ));
        }

        if !self.key_pem.contains("-----BEGIN") {
            return Err(TlsError::InvalidPemFormat(
                "Missing key header".to_string(),
            ));
        }

        Ok(())
    }
}

/// Certificate builder for creating custom certificates
pub struct CertificateBuilder {
    common_name: String,
    organization: Option<String>,
    country: Option<String>,
    validity_days: u32,
    san_dns_names: Vec<String>,
    san_ip_addresses: Vec<String>,
}

impl CertificateBuilder {
    /// Create a new certificate builder
    pub fn new(common_name: impl Into<String>) -> Self {
        Self {
            common_name: common_name.into(),
            organization: None,
            country: None,
            validity_days: 365,
            san_dns_names: Vec::new(),
            san_ip_addresses: Vec::new(),
        }
    }

    /// Set organization name
    pub fn organization(mut self, org: impl Into<String>) -> Self {
        self.organization = Some(org.into());
        self
    }

    /// Set country code
    pub fn country(mut self, country: impl Into<String>) -> Self {
        self.country = Some(country.into());
        self
    }

    /// Set validity period in days
    pub fn validity_days(mut self, days: u32) -> Self {
        self.validity_days = days;
        self
    }

    /// Add subject alternative name (DNS)
    pub fn add_san_dns(mut self, dns_name: impl Into<String>) -> Self {
        self.san_dns_names.push(dns_name.into());
        self
    }

    /// Add subject alternative name (IP address)
    pub fn add_san_ip(mut self, ip: impl Into<String>) -> Self {
        self.san_ip_addresses.push(ip.into());
        self
    }

    /// Generate self-signed certificate
    #[cfg(feature = "tls")]
    #[allow(clippy::unwrap_used)]
    pub fn generate_self_signed(self) -> TlsResult<Certificate> {
        // Create certificate parameters
        let mut params = CertificateParams::default();

        // Set subject alt names
        params.subject_alt_names = self.san_dns_names.iter()
            .map(|name| rcgen::SanType::DnsName(name.clone().try_into().unwrap()))
            .collect();

        // Set distinguished name
        let mut dn = DistinguishedName::new();
        dn.push(DnType::CommonName, &self.common_name);

        if let Some(org) = &self.organization {
            dn.push(DnType::OrganizationName, org);
        }

        if let Some(country) = &self.country {
            dn.push(DnType::CountryName, country);
        }

        params.distinguished_name = dn;

        // Set validity period using time crate (not chrono)
        use time::OffsetDateTime;
        let now = OffsetDateTime::now_utc();
        params.not_before = now - time::Duration::days(1);
        params.not_after = now + time::Duration::days(self.validity_days as i64);

        // Generate key pair first
        let key_pair = rcgen::KeyPair::generate()
            .map_err(|e| TlsError::CertificateGeneration(e.to_string()))?;

        // Store key PEM before consuming key_pair
        let key_pem = key_pair.serialize_pem();

        // Generate certificate
        let cert = params.self_signed(&key_pair)
            .map_err(|e| TlsError::CertificateGeneration(e.to_string()))?;

        let cert_pem = cert.pem();

        Ok(Certificate {
            cert_pem,
            key_pem,
            common_name: self.common_name,
            organization: self.organization,
            validity_days: self.validity_days,
        })
    }

    /// Generate self-signed certificate (stub for non-TLS feature)
    #[cfg(not(feature = "tls"))]
    pub fn generate_self_signed(self) -> TlsResult<Certificate> {
        Err(TlsError::CertificateGeneration(
            "TLS feature not enabled".to_string(),
        ))
    }
}

#[cfg(all(test, feature = "tls"))]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn test_certificate_builder() {
        let cert = CertificateBuilder::new("localhost")
            .organization("MediaGit")
            .country("US")
            .validity_days(30)
            .add_san_dns("localhost")
            .add_san_dns("127.0.0.1")
            .generate_self_signed()
            .unwrap();

        assert_eq!(cert.common_name, "localhost");
        assert_eq!(cert.organization, Some("MediaGit".to_string()));
        assert_eq!(cert.validity_days, 30);
        assert!(cert.cert_pem.contains("-----BEGIN CERTIFICATE-----"));
        assert!(cert.key_pem.contains("-----BEGIN PRIVATE KEY-----"));
    }

    #[test]
    fn test_certificate_validation() {
        let cert = CertificateBuilder::new("test.example.com")
            .generate_self_signed()
            .unwrap();

        assert!(cert.validate().is_ok());
    }

    #[test]
    fn test_invalid_certificate() {
        let cert = Certificate::new(
            String::new(),
            String::new(),
            "invalid".to_string(),
        );

        assert!(cert.validate().is_err());
    }

    #[test]
    fn test_certificate_save_load() {
        use tempfile::TempDir;

        let temp_dir = TempDir::new().unwrap();
        let cert_path = temp_dir.path().join("cert.pem");
        let key_path = temp_dir.path().join("key.pem");

        // Generate and save certificate
        let cert = CertificateBuilder::new("save-test.example.com")
            .generate_self_signed()
            .unwrap();

        cert.save_to_files(&cert_path, &key_path).unwrap();

        // Load certificate
        let loaded_cert = Certificate::from_pem_files(&cert_path, &key_path).unwrap();

        assert_eq!(loaded_cert.cert_pem, cert.cert_pem);
        assert_eq!(loaded_cert.key_pem, cert.key_pem);
    }
}
