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

//! Compression error types

use thiserror::Error;

/// Result type alias for compression operations
pub type CompressionResult<T> = Result<T, CompressionError>;

/// Errors that can occur during compression operations
#[derive(Error, Debug)]
pub enum CompressionError {
    /// Compression operation failed
    #[error("compression failed: {0}")]
    CompressionFailed(String),

    /// Decompression operation failed
    #[error("decompression failed: {0}")]
    DecompressionFailed(String),

    /// Invalid input data
    #[error("invalid input: {0}")]
    InvalidInput(String),

    /// Unsupported compression algorithm
    #[error("unsupported algorithm: {0}")]
    UnsupportedAlgorithm(String),

    /// I/O error occurred
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    /// Zstd-specific error
    #[error("zstd error: {0}")]
    ZstdError(String),

    /// Brotli-specific error
    #[error("brotli error: {0}")]
    BrotliError(String),

    /// Generic error for opaque error types
    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

impl CompressionError {
    /// Create a compression failed error
    pub fn compression_failed<S: Into<String>>(msg: S) -> Self {
        CompressionError::CompressionFailed(msg.into())
    }

    /// Create a decompression failed error
    pub fn decompression_failed<S: Into<String>>(msg: S) -> Self {
        CompressionError::DecompressionFailed(msg.into())
    }

    /// Create an invalid input error
    pub fn invalid_input<S: Into<String>>(msg: S) -> Self {
        CompressionError::InvalidInput(msg.into())
    }

    /// Create an unsupported algorithm error
    pub fn unsupported_algorithm<S: Into<String>>(algorithm: S) -> Self {
        CompressionError::UnsupportedAlgorithm(algorithm.into())
    }

    /// Create a zstd-specific error
    pub fn zstd_error<S: Into<String>>(msg: S) -> Self {
        CompressionError::ZstdError(msg.into())
    }

    /// Create a brotli-specific error
    pub fn brotli_error<S: Into<String>>(msg: S) -> Self {
        CompressionError::BrotliError(msg.into())
    }

    /// Check if this is a compression failed error
    pub fn is_compression_failed(&self) -> bool {
        matches!(self, CompressionError::CompressionFailed(_))
    }

    /// Check if this is a decompression failed error
    pub fn is_decompression_failed(&self) -> bool {
        matches!(self, CompressionError::DecompressionFailed(_))
    }

    /// Check if this is an I/O error
    pub fn is_io(&self) -> bool {
        matches!(self, CompressionError::Io(_))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_compression_error_creation() {
        let err = CompressionError::compression_failed("test compression");
        assert!(err.is_compression_failed());
        assert_eq!(err.to_string(), "compression failed: test compression");
    }

    #[test]
    fn test_decompression_error_creation() {
        let err = CompressionError::decompression_failed("test decompression");
        assert!(err.is_decompression_failed());
        assert_eq!(err.to_string(), "decompression failed: test decompression");
    }

    #[test]
    fn test_invalid_input_error() {
        let err = CompressionError::invalid_input("empty data");
        assert_eq!(err.to_string(), "invalid input: empty data");
    }

    #[test]
    fn test_unsupported_algorithm_error() {
        let err = CompressionError::unsupported_algorithm("gzip");
        assert_eq!(err.to_string(), "unsupported algorithm: gzip");
    }

    #[test]
    fn test_io_error_conversion() {
        let io_err = std::io::Error::other("read failed");
        let comp_err = CompressionError::from(io_err);
        assert!(comp_err.is_io());
    }
}
