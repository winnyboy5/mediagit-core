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

//! Error types for media processing operations

use thiserror::Error;

/// Media processing errors
#[derive(Debug, Error)]
pub enum MediaError {
    /// Image processing error
    #[error("Image processing error: {0}")]
    ImageError(String),

    /// EXIF parsing error
    #[error("EXIF parsing error: {0}")]
    ExifError(String),

    /// PSD parsing error
    #[error("PSD parsing error: {0}")]
    PsdError(String),

    /// Video parsing error
    #[error("Video parsing error: {0}")]
    VideoError(String),

    /// Audio parsing error
    #[error("Audio parsing error: {0}")]
    AudioError(String),

    /// Unsupported format
    #[error("Unsupported media format: {0}")]
    UnsupportedFormat(String),

    /// IO error
    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),

    /// Invalid file structure
    #[error("Invalid file structure: {0}")]
    InvalidStructure(String),

    /// Perceptual hash error
    #[error("Perceptual hash error: {0}")]
    HashError(String),

    /// Metadata extraction error
    #[error("Metadata extraction error: {0}")]
    MetadataError(String),
}

/// Result type for media operations
pub type Result<T> = std::result::Result<T, MediaError>;
