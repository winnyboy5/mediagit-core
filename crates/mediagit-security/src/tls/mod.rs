//! TLS/SSL certificate management module
//!
//! Provides certificate generation, loading, and validation
//! for secure HTTPS communication.
//!
//! # Features
//! - Self-signed certificate generation for development
//! - PEM file loading for production certificates
//! - Certificate validation and expiry checking
//! - TLS configuration management

pub mod cert;
pub mod config;

pub use cert::{Certificate, CertificateBuilder, CertificateError};
pub use config::{TlsConfig, TlsConfigBuilder};

use thiserror::Error;

/// TLS module errors
#[derive(Debug, Error)]
pub enum TlsError {
    /// Certificate generation failed
    #[error("Certificate generation failed: {0}")]
    CertificateGeneration(String),

    /// Certificate loading failed
    #[error("Certificate loading failed: {0}")]
    CertificateLoading(String),

    /// Certificate validation failed
    #[error("Certificate validation failed: {0}")]
    CertificateValidation(String),

    /// Certificate expired
    #[error("Certificate expired")]
    CertificateExpired,

    /// Invalid PEM file format
    #[error("Invalid PEM file format: {0}")]
    InvalidPemFormat(String),

    /// I/O error
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    /// Internal error
    #[error("TLS error: {0}")]
    Internal(#[from] anyhow::Error),
}

pub type TlsResult<T> = Result<T, TlsError>;
