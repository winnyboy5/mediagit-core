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

//! Storage error types and utilities

use std::io;
use thiserror::Error;

/// Result type alias for storage operations
pub type StorageResult<T> = Result<T, StorageError>;

/// Errors that can occur during storage operations
#[derive(Error, Debug)]
pub enum StorageError {
    /// Object not found in storage
    #[error("object not found: {0}")]
    NotFound(String),

    /// Permission denied for the requested operation
    #[error("permission denied: {0}")]
    PermissionDenied(String),

    /// I/O error occurred
    #[error("I/O error: {0}")]
    Io(#[from] io::Error),

    /// Invalid key format (empty, contains invalid characters, etc.)
    #[error("invalid key: {0}")]
    InvalidKey(String),

    /// Storage backend not available or misconfigured
    #[error("storage backend error: {0}")]
    Backend(String),

    /// Operation timed out
    #[error("operation timed out: {0}")]
    Timeout(String),

    /// Transparent error delegation for wrapped error types
    ///
    /// This variant allows wrapping other error types (like anyhow::Error)
    /// while forwarding their Display and source implementations transparently.
    /// Useful for catch-all error handling and opaque error types.
    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

impl StorageError {
    /// Create a NotFound error with the given key
    pub fn not_found<S: Into<String>>(key: S) -> Self {
        StorageError::NotFound(key.into())
    }

    /// Create a PermissionDenied error with context
    pub fn permission_denied<S: Into<String>>(msg: S) -> Self {
        StorageError::PermissionDenied(msg.into())
    }

    /// Create an InvalidKey error with context
    pub fn invalid_key<S: Into<String>>(msg: S) -> Self {
        StorageError::InvalidKey(msg.into())
    }

    /// Create a Backend error with context
    pub fn backend<S: Into<String>>(msg: S) -> Self {
        StorageError::Backend(msg.into())
    }

    /// Create a Timeout error with context
    pub fn timeout<S: Into<String>>(msg: S) -> Self {
        StorageError::Timeout(msg.into())
    }

    /// Create a generic error from any error type that can convert to anyhow::Error
    pub fn other<E: Into<anyhow::Error>>(error: E) -> Self {
        StorageError::Other(error.into())
    }

    /// Check if this is a NotFound error
    pub fn is_not_found(&self) -> bool {
        matches!(self, StorageError::NotFound(_))
    }

    /// Check if this is a PermissionDenied error
    pub fn is_permission_denied(&self) -> bool {
        matches!(self, StorageError::PermissionDenied(_))
    }

    /// Check if this is an InvalidKey error
    pub fn is_invalid_key(&self) -> bool {
        matches!(self, StorageError::InvalidKey(_))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_creation() {
        let err = StorageError::not_found("test_key");
        assert!(err.is_not_found());
        assert_eq!(err.to_string(), "object not found: test_key");
    }

    #[test]
    fn test_permission_denied_error() {
        let err = StorageError::permission_denied("bucket locked");
        assert!(err.is_permission_denied());
    }

    #[test]
    fn test_invalid_key_error() {
        let err = StorageError::invalid_key("empty key");
        assert!(err.is_invalid_key());
    }

    #[test]
    fn test_io_error_conversion() {
        let io_err = io::Error::other("read failed");
        let storage_err = StorageError::from(io_err);
        assert!(matches!(storage_err, StorageError::Io(_)));
    }
}
