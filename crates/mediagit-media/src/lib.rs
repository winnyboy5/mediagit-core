// SPDX-License-Identifier: BUSL-1.1
// Copyright (C) 2025-2026 Aswin Krishnamoorthy

#![allow(missing_docs)]
//! Media-aware merge intelligence for MediaGit.
//!
//! This crate provides format-specific metadata extraction and merge strategies
//! for binary media files, enabling semantically correct three-way merges
//! instead of byte-level conflicts.
//!
//! # Capabilities
//!
//! - **Image**: EXIF, IPTC, XMP metadata parsing; perceptual hash similarity
//! - **PSD**: Photoshop layer detection and layer-aware merging
//! - **Video**: Timeline parsing for MOV, MP4, MXF, R3D formats
//! - **Audio**: Track merging for WAV, AIFF, FLAC, MP3
//! - **3D Models**: GLB, FBX, OBJ, USD scene graph analysis
//! - **Merge Strategies**: Per-format `MergeStrategy` selected by file extension
//!
//! # Key Types
//!
//! - [`strategy::MergeStrategy`] — enum of all supported merge strategies;
//!   use `MergeStrategy::from_extension("psd")` to select the right one
//! - [`error::MediaError`] — unified error type for all parse failures

pub mod audio;
pub mod error;
pub mod image;
pub mod model3d;
pub mod phash;
pub mod psd;
pub mod strategy;
pub mod vfx;
pub mod video;

// Re-export commonly used types
pub use audio::{AudioInfo, AudioParser, AudioTrack};
pub use error::{MediaError, Result};
pub use image::{ImageMetadata, ImageMetadataParser};
pub use model3d::{BoundingBox, MaterialInfo, Model3DFormat, Model3DInfo, Model3DParser};
pub use phash::{PerceptualHash, PerceptualHasher};
pub use psd::{LayerInfo, PsdInfo, PsdParser};
pub use strategy::{MediaType, MergeResult, MergeStrategy};
pub use vfx::{VfxFormat, VfxInfo, VfxParser};
pub use video::{TimelineSegment, TrackInfo, VideoInfo, VideoParser};

#[cfg(test)]
mod tests {
    #[test]
    fn media_compiles() {
        // Foundation test
    }
}
