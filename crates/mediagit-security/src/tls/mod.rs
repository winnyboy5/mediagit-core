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
