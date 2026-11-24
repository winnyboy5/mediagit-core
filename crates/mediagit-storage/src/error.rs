// SPDX-License-Identifier: BUSL-1.1
// Copyright (C) 2025-2026 Aswin Krishnamoorthy

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

    /// The bucket (or container) does not exist. Backends never create one.
    #[error("bucket '{0}' does not exist")]
    BucketNotFound(String),

    /// Permission denied for the requested operation
    #[error("permission denied: {0}")]
    PermissionDenied(String),

    /// A failure whose text is already one plain sentence a person can act on
    /// (bad key file, bad connection string, inaccessible bucket). It never
    /// holds a secret; callers show it as it is.
    #[error("{0}")]
    UserFacing(String),

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

/// The bucket exists but these credentials cannot use it (wrong account or region).
pub(crate) fn bucket_inaccessible(bucket: &str) -> StorageError {
    StorageError::UserFacing(format!(
        "The bucket '{bucket}' cannot be accessed with these credentials. It may belong to another account; check the bucket name and the keys."
    ))
}

/// A service-account key file that is missing, unreadable or not a key.
pub(crate) fn key_file_error(path: &std::path::Path, problem: &str) -> StorageError {
    StorageError::UserFacing(format!(
        "The key file {} {problem}. Check the path and that the file is a service-account key.",
        path.display()
    ))
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
