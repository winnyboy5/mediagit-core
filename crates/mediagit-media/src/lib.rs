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

//! Media-aware merge intelligence for MediaGit
//!
//! This crate provides:
//! - Image metadata parsing (EXIF, IPTC, XMP)
//! - PSD layer detection and merging
//! - Video timeline parsing
//! - Audio track merging
//! - Perceptual hashing for similarity detection
//! - Format-specific merge strategies

pub mod audio;
pub mod error;
pub mod image;
pub mod phash;
pub mod psd;
pub mod strategy;
pub mod video;

// Re-export commonly used types
pub use audio::{AudioInfo, AudioParser, AudioTrack};
pub use error::{MediaError, Result};
pub use image::{ImageMetadata, ImageMetadataParser};
pub use phash::{PerceptualHash, PerceptualHasher};
pub use psd::{LayerInfo, PsdInfo, PsdParser};
pub use strategy::{MediaType, MergeResult, MergeStrategy};
pub use video::{TimelineSegment, TrackInfo, VideoInfo, VideoParser};

#[cfg(test)]
mod tests {
    #[test]
    fn media_compiles() {
        // Foundation test
    }
}
