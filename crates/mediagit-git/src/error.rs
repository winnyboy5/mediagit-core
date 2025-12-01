// SPDX-License-Identifier: AGPL-3.0
// Copyright (C) 2025 MediaGit Contributors

//! Error types for Git integration

use thiserror::Error;

/// Result type for Git operations
pub type GitResult<T> = Result<T, GitError>;

/// Error types for Git integration operations
#[derive(Debug, Error)]
pub enum GitError {
    /// Error parsing pointer file
    #[error("Failed to parse pointer file: {0}")]
    PointerParse(String),

    /// Invalid pointer file format
    #[error("Invalid pointer file format: {0}")]
    InvalidPointerFormat(String),

    /// Missing required field in pointer file
    #[error("Missing required field in pointer file: {0}")]
    MissingPointerField(String),

    /// Invalid OID format
    #[error("Invalid OID format: {0}")]
    InvalidOid(String),

    /// Git2 library error
    #[error("Git error: {0}")]
    Git2(#[from] git2::Error),

    /// IO error
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    /// Filter driver not configured
    #[error("Filter driver not configured: {0}")]
    FilterNotConfigured(String),

    /// Filter operation failed
    #[error("Filter operation failed: {0}")]
    FilterFailed(String),

    /// .gitattributes configuration error
    #[error("Failed to configure .gitattributes: {0}")]
    GitattributesConfig(String),

    /// Repository not initialized
    #[error("Repository not initialized at path: {0}")]
    RepositoryNotFound(String),

    /// Invalid repository state
    #[error("Invalid repository state: {0}")]
    InvalidRepositoryState(String),
}
