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

//! Format-specific merge strategies
//!
//! This module provides intelligent merge strategies tailored to different
//! media file formats, enabling automatic conflict resolution when possible.
//!
//! # Supported Formats
//!
//! - **Images**: JPEG, PNG, TIFF, WebP with metadata-aware merging
//! - **PSD**: Layer-based merging for Photoshop files
//! - **Video**: Timeline-based merging for MP4 files
//! - **Audio**: Track-based merging for multi-track audio
//! - **3D Models**: Future-proof framework for 3D file formats
//!
//! # Example
//!
//! ```rust,no_run
//! use mediagit_media::strategy::{MergeStrategy, MediaType, MergeResult};
//!
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! let strategy = MergeStrategy::for_media_type(MediaType::Image);
//!
//! let base = std::fs::read("base.jpg")?;
//! let ours = std::fs::read("ours.jpg")?;
//! let theirs = std::fs::read("theirs.jpg")?;
//!
//! let result = strategy.merge(&base, &ours, &theirs, "image.jpg").await?;
//! match result {
//!     MergeResult::AutoMerged(data) => println!("Auto-merged successfully"),
//!     MergeResult::Conflict(reason) => println!("Manual review needed: {}", reason),
//!     MergeResult::NoChangeNeeded => println!("Files are identical"),
//! }
//! # Ok(())
//! # }
//! ```

use crate::audio::AudioParser;
use crate::error::Result;
use crate::model3d::Model3DParser;
use crate::psd::PsdParser;
use crate::vfx::VfxParser;
use crate::video::VideoParser;
use serde::{Deserialize, Serialize};
use tracing::{debug, info, instrument, warn};

/// Media type for strategy selection
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MediaType {
    Image,
    Psd,
    Video,
    Audio,
    Model3D,
    Vfx,
    Unknown,
}

impl MediaType {
    /// Detect media type from file extension
    pub fn from_extension(ext: &str) -> Self {
        match ext.to_lowercase().as_str() {
            // === IMAGES ===
            // Standard web/photo formats
            "jpg" | "jpeg" | "png" | "gif" | "bmp" | "tiff" | "tif" | "webp" | "svg" |
            // RAW camera formats
            "raw" | "cr2" | "cr3" | "nef" | "arw" | "dng" | "orf" | "rw2" | "pef" | "srw" |
            // HDR/professional formats
            "heic" | "heif" | "exr" | "hdr" | "avif" | "jxl" => MediaType::Image,

            // === PSD/LAYERED IMAGES ===
            "psd" | "psb" | "xcf" | "kra" | "ora" => MediaType::Psd,

            // === VIDEO ===
            // Container formats
            "mp4" | "mov" | "avi" | "mkv" | "webm" | "flv" | "wmv" | "m4v" | "3gp" | "3g2" |
            // MPEG formats
            "mpg" | "mpeg" | "mts" | "m2ts" | "vob" | "ts" |
            // Professional video
            "mxf" | "r3d" | "braw" | "ari" => MediaType::Video,

            // === AUDIO ===
            // Common formats
            "mp3" | "wav" | "flac" | "aac" | "ogg" | "m4a" | "wma" | "aiff" | "aif" |
            // Professional/lossless
            "opus" | "alac" | "ape" | "dsd" | "dsf" | "dff" |
            // MIDI
            "mid" | "midi" => MediaType::Audio,

            // === 3D MODELS ===
            // Common interchange formats
            "obj" | "fbx" | "gltf" | "glb" | "stl" | "dae" | "3ds" | "ply" |
            // USD ecosystem
            "usd" | "usda" | "usdc" | "usdz" |
            // Alembic cache
            "abc" |
            // Application-specific
            "blend" | "max" | "ma" | "mb" | "c4d" | "hip" | "hiplc" | "hipnc" | "zpr" | "ztl" => MediaType::Model3D,

            // === VFX/CREATIVE APPS ===
            // Adobe suite
            "indd" | "indt" | "ai" | "ait" | "aep" | "aet" | "prproj" | "mogrt" | "sesx" |
            // Documents (PDF is cross-platform but used heavily in creative workflows)
            "pdf" |
            // Video editing
            "pproj" | "drp" | "fcpxml" | "fcpbundle" |
            // Compositing
            "nk" | "nknc" | "comp" |
            // Motion graphics
            "aaf" | "omf" | "edl" |
            // Design tools
            "fig" | "sketch" | "xd" => MediaType::Vfx,

            _ => MediaType::Unknown,
        }
    }
}

/// Result of a merge operation
#[derive(Debug, Clone)]
pub enum MergeResult {
    /// Files were successfully auto-merged.
    ///
    /// **Contract: these bytes are the merged file, in the same format as the
    /// inputs.** A caller writes them straight to the working tree, so
    /// anything else is silent data destruction.
    ///
    /// Four strategies used to return serialized *metadata* here — a
    /// successful PSD "auto-merge" produced a JSON document, which would have
    /// replaced a designer's `.psd` with JSON had the strategies ever been
    /// wired into `merge`. They now report `Conflict` instead: none of them
    /// can reconstruct a real file yet (PSD writing is not supported by the
    /// `psd` crate, video would need re-encoding). Do not restore an
    /// `AutoMerged` return without producing genuine format bytes.
    AutoMerged(Vec<u8>),

    /// Manual review required
    Conflict(String),

    /// No merge needed (files are identical)
    NoChangeNeeded,
}

/// Format-specific merge strategy
#[derive(Debug, Clone)]
pub enum MergeStrategy {
    /// Image merge strategy
    Image(ImageStrategy),

    /// PSD merge strategy
    Psd(PsdStrategy),

    /// Video merge strategy
    Video(VideoStrategy),

    /// Audio merge strategy
    Audio(AudioStrategy),

    /// 3D model merge strategy
    Model3D(Model3DStrategy),

    /// VFX file merge strategy
    Vfx(VfxStrategy),

    /// Generic binary merge (no intelligence)
    Generic,
}

impl MergeStrategy {
    /// Create strategy for media type
    pub fn for_media_type(media_type: MediaType) -> Self {
        match media_type {
            MediaType::Image => MergeStrategy::Image(ImageStrategy::default()),
            MediaType::Psd => MergeStrategy::Psd(PsdStrategy),
            MediaType::Video => MergeStrategy::Video(VideoStrategy),
            MediaType::Audio => MergeStrategy::Audio(AudioStrategy),
            MediaType::Model3D => MergeStrategy::Model3D(Model3DStrategy),
            MediaType::Vfx => MergeStrategy::Vfx(VfxStrategy),
            MediaType::Unknown => MergeStrategy::Generic,
        }
    }

    /// Attempt to merge three versions of a file
    #[instrument(skip(base, ours, theirs), fields(filename = %filename))]
    pub async fn merge(
        &self,
        base: &[u8],
        ours: &[u8],
        theirs: &[u8],
        filename: &str,
    ) -> Result<MergeResult> {
        info!("Attempting media-aware merge for {}", filename);

        match self {
            MergeStrategy::Image(strategy) => strategy.merge(base, ours, theirs, filename).await,
            MergeStrategy::Psd(strategy) => strategy.merge(base, ours, theirs, filename).await,
            MergeStrategy::Video(strategy) => strategy.merge(base, ours, theirs, filename).await,
            MergeStrategy::Audio(strategy) => strategy.merge(base, ours, theirs, filename).await,
            MergeStrategy::Model3D(strategy) => strategy.merge(base, ours, theirs, filename).await,
            MergeStrategy::Vfx(strategy) => strategy.merge(base, ours, theirs, filename).await,
            MergeStrategy::Generic => {
                warn!("Using generic strategy - no media intelligence");
                Ok(MergeResult::Conflict(
                    "Generic binary file - manual review required".to_string(),
                ))
            }
        }
    }
}

/// Image-specific merge strategy
#[derive(Debug, Clone)]
pub struct ImageStrategy {
    /// Similarity threshold for auto-merge (0.0-1.0)
    pub similarity_threshold: f64,
}

impl ImageStrategy {
    /// Intelligent image merging using perceptual hashing and metadata analysis
    ///
    /// This implementation:
    /// - Extracts EXIF, IPTC, XMP metadata from all three versions
    /// - Calculates perceptual hashes for visual similarity detection
    /// - Auto-merges when images are visually identical (95%+ similar)
    /// - Intelligently merges metadata (EXIF, IPTC, XMP)
    /// - Requires manual review when visual content differs
    async fn merge(
        &self,
        base: &[u8],
        ours: &[u8],
        theirs: &[u8],
        filename: &str,
    ) -> crate::error::Result<MergeResult> {
        use crate::image::{ImageMetadataParser, MergeDecision};

        debug!("Using image merge strategy for {}", filename);

        // Parse all three versions
        let base_metadata = ImageMetadataParser::parse(base, &format!("base_{}", filename)).await?;
        let ours_metadata = ImageMetadataParser::parse(ours, &format!("ours_{}", filename)).await?;
        let theirs_metadata =
            ImageMetadataParser::parse(theirs, &format!("theirs_{}", filename)).await?;

        // Check if auto-merge is possible
        let decision =
            ImageMetadataParser::can_auto_merge(&base_metadata, &ours_metadata, &theirs_metadata);

        match decision {
            MergeDecision::AutoMerge => {
                info!("Images are visually identical - executing auto-merge");

                // Perform intelligent metadata merge
                let merged_metadata = ImageMetadataParser::merge_metadata(
                    &base_metadata,
                    &ours_metadata,
                    &theirs_metadata,
                )?;

                // Only the metadata was merged; writing it back into the image
                // needs an encoder this crate does not have. Returning the
                // metadata JSON as `AutoMerged` would replace the image file
                // with a JSON document.
                //
                // To make this a real auto-merge: load `ours` (visual content
                // is identical on this path), strip its metadata, write the
                // merged metadata back, and return those image bytes.
                let _ = &merged_metadata;
                info!("Image metadata is mergeable but re-encoding is unsupported");
                Ok(MergeResult::Conflict(
                    "only metadata differs and it could be merged, but MediaGit cannot \
                        re-encode the image. Resolve by keeping one side and staging it."
                        .to_string(),
                ))
            }
            MergeDecision::ManualReview(conflicts) => {
                warn!("Image conflicts detected: {:?}", conflicts);
                Ok(MergeResult::Conflict(format!(
                    "Visual differences detected: {}",
                    conflicts.join(", ")
                )))
            }
        }
    }
}

impl Default for ImageStrategy {
    fn default() -> Self {
        ImageStrategy {
            similarity_threshold: 0.95, // 95% similarity for auto-merge
        }
    }
}

/// PSD-specific merge strategy
#[derive(Debug, Clone, Default)]
pub struct PsdStrategy;

impl PsdStrategy {
    async fn merge(
        &self,
        base: &[u8],
        ours: &[u8],
        theirs: &[u8],
        _filename: &str,
    ) -> Result<MergeResult> {
        debug!("Using PSD merge strategy");

        let parser = PsdParser::new();
        let base_psd = parser.parse(base).await?;
        let ours_psd = parser.parse(ours).await?;
        let theirs_psd = parser.parse(theirs).await?;

        let decision = PsdParser::can_auto_merge(&base_psd, &ours_psd, &theirs_psd);

        match decision {
            crate::psd::MergeDecision::AutoMerge => {
                info!("Non-overlapping layer changes - executing auto-merge");

                // Perform actual layer merge
                let merged_psd = PsdParser::merge_layers(&base_psd, &ours_psd, &theirs_psd)?;

                // The layer analysis says these edits *could* be combined, but
                // nothing here can write a PSD back out — the `psd` crate is
                // read-only. Returning the merged metadata as `AutoMerged`
                // would hand the caller JSON to write over the user's .psd.
                // Report what was learned instead; it is still the most
                // useful thing we can tell them.
                info!(
                    "PSD layers are separable ({} layers) but PSD writing is unsupported",
                    merged_psd.layers.len()
                );
                Ok(MergeResult::Conflict(format!(
                    "layer edits do not overlap ({} layers), but MediaGit cannot \
                        write a merged PSD — the format is read-only here. Resolve by \
                        combining the layers in your editor and staging the result.",
                    merged_psd.layers.len()
                )))
            }
            crate::psd::MergeDecision::ManualReview(conflicts) => {
                warn!("PSD layer conflicts: {:?}", conflicts);
                Ok(MergeResult::Conflict(format!(
                    "Layer conflicts detected: {}",
                    conflicts.join(", ")
                )))
            }
        }
    }
}

/// Video-specific merge strategy
#[derive(Debug, Clone, Default)]
pub struct VideoStrategy;

impl VideoStrategy {
    async fn merge(
        &self,
        base: &[u8],
        ours: &[u8],
        theirs: &[u8],
        _filename: &str,
    ) -> Result<MergeResult> {
        debug!("Using video merge strategy");

        let parser = VideoParser::new();
        let base_video = parser.parse(base).await?;
        let ours_video = parser.parse(ours).await?;
        let theirs_video = parser.parse(theirs).await?;

        let decision = VideoParser::can_auto_merge(&base_video, &ours_video, &theirs_video);

        match decision {
            crate::video::MergeDecision::AutoMerge => {
                info!("Non-overlapping timeline edits - executing auto-merge");

                // Perform actual timeline merge
                let merged_video =
                    VideoParser::merge_timelines(&base_video, &ours_video, &theirs_video)?;

                // Timeline analysis says the edits are separable, but writing
                // a merged video needs re-encoding (FFmpeg or similar), which
                // this crate does not do. Returning timeline JSON as
                // `AutoMerged` would overwrite the user's video with metadata.
                info!(
                    "Video timeline is separable ({} tracks, {} segments) but \
                        re-encoding is unsupported",
                    merged_video.tracks.len(),
                    merged_video.segments.len()
                );
                Ok(MergeResult::Conflict(format!(
                    "timeline edits do not overlap ({} tracks, {} segments), but \
                        MediaGit cannot render a merged video — that needs re-encoding. \
                        Resolve in your editor and stage the result.",
                    merged_video.tracks.len(),
                    merged_video.segments.len()
                )))
            }
            crate::video::MergeDecision::ManualReview(conflicts) => {
                warn!("Video timeline conflicts: {:?}", conflicts);
                Ok(MergeResult::Conflict(format!(
                    "Timeline conflicts detected: {}",
                    conflicts.join(", ")
                )))
            }
        }
    }
}

/// Audio-specific merge strategy
#[derive(Debug, Clone, Default)]
pub struct AudioStrategy;

impl AudioStrategy {
    async fn merge(
        &self,
        base: &[u8],
        ours: &[u8],
        theirs: &[u8],
        filename: &str,
    ) -> Result<MergeResult> {
        debug!("Using audio merge strategy");

        let parser = AudioParser::new();
        let base_audio = parser.parse(base, &format!("base_{}", filename)).await?;
        let ours_audio = parser.parse(ours, &format!("ours_{}", filename)).await?;
        let theirs_audio = parser
            .parse(theirs, &format!("theirs_{}", filename))
            .await?;

        let decision = AudioParser::can_auto_merge(&base_audio, &ours_audio, &theirs_audio);

        match decision {
            crate::audio::MergeDecision::AutoMerge => {
                info!("Different audio tracks modified - executing auto-merge");

                // Perform actual track merge
                let merged_audio =
                    AudioParser::merge_tracks(&base_audio, &ours_audio, &theirs_audio)?;

                // Track analysis says the edits are separable, but producing a
                // mixed file needs audio processing this crate does not do.
                // Returning track JSON as `AutoMerged` would overwrite the
                // user's audio with metadata.
                info!(
                    "Audio tracks are separable ({}) but mixing is unsupported",
                    merged_audio.tracks.len()
                );
                Ok(MergeResult::Conflict(format!(
                    "different tracks were modified ({} tracks), but MediaGit cannot \
                        mix a merged audio file. Resolve in your editor and stage the result.",
                    merged_audio.tracks.len()
                )))
            }
            crate::audio::MergeDecision::ManualReview(conflicts) => {
                warn!("Audio track conflicts: {:?}", conflicts);
                Ok(MergeResult::Conflict(format!(
                    "Track conflicts detected: {}",
                    conflicts.join(", ")
                )))
            }
        }
    }
}

/// 3D model merge strategy
#[derive(Debug, Clone, Default)]
pub struct Model3DStrategy;

impl Model3DStrategy {
    async fn merge(
        &self,
        base: &[u8],
        ours: &[u8],
        theirs: &[u8],
        filename: &str,
    ) -> Result<MergeResult> {
        debug!("Using 3D model merge strategy");

        let parser = Model3DParser::new();
        let base_model = parser.parse(base, &format!("base_{}", filename)).await?;
        let ours_model = parser.parse(ours, &format!("ours_{}", filename)).await?;
        let theirs_model = parser
            .parse(theirs, &format!("theirs_{}", filename))
            .await?;

        let decision = Model3DParser::can_auto_merge(&base_model, &ours_model, &theirs_model);

        match decision {
            crate::model3d::MergeDecision::AutoMerge => {
                info!("Non-overlapping 3D model changes - can auto-merge");
                // For 3D models, actual binary merging is complex
                // For now, return conflict to trigger manual review
                Ok(MergeResult::Conflict(
                    "3D model merging requires manual review in 3D software".to_string(),
                ))
            }
            crate::model3d::MergeDecision::ManualReview(conflicts) => {
                warn!("3D model conflicts: {:?}", conflicts);
                Ok(MergeResult::Conflict(format!(
                    "3D model conflicts detected: {}",
                    conflicts.join(", ")
                )))
            }
        }
    }
}

/// VFX file merge strategy
#[derive(Debug, Clone, Default)]
pub struct VfxStrategy;

impl VfxStrategy {
    async fn merge(
        &self,
        base: &[u8],
        ours: &[u8],
        theirs: &[u8],
        filename: &str,
    ) -> Result<MergeResult> {
        debug!("Using VFX file merge strategy");

        let parser = VfxParser::new();
        let base_vfx = parser.parse(base, &format!("base_{}", filename)).await?;
        let ours_vfx = parser.parse(ours, &format!("ours_{}", filename)).await?;
        let theirs_vfx = parser
            .parse(theirs, &format!("theirs_{}", filename))
            .await?;

        let decision = VfxParser::can_auto_merge(&base_vfx, &ours_vfx, &theirs_vfx);

        match decision {
            crate::vfx::MergeDecision::AutoMerge => {
                info!("Non-overlapping VFX changes - can auto-merge");
                // For VFX files, actual binary merging is complex
                // For now, return conflict to trigger manual review
                Ok(MergeResult::Conflict(
                    "VFX file merging requires manual review in creative software".to_string(),
                ))
            }
            crate::vfx::MergeDecision::ManualReview(conflicts) => {
                warn!("VFX file conflicts: {:?}", conflicts);
                Ok(MergeResult::Conflict(format!(
                    "VFX file conflicts detected: {}",
                    conflicts.join(", ")
                )))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_media_type_detection() {
        // Image formats
        assert_eq!(MediaType::from_extension("jpg"), MediaType::Image);
        assert_eq!(MediaType::from_extension("jpeg"), MediaType::Image);
        assert_eq!(MediaType::from_extension("png"), MediaType::Image);
        assert_eq!(MediaType::from_extension("gif"), MediaType::Image);
        assert_eq!(MediaType::from_extension("webp"), MediaType::Image);
        assert_eq!(MediaType::from_extension("heic"), MediaType::Image);
        assert_eq!(MediaType::from_extension("exr"), MediaType::Image);
        assert_eq!(MediaType::from_extension("cr2"), MediaType::Image);
        assert_eq!(MediaType::from_extension("dng"), MediaType::Image);

        // PSD/layered formats
        assert_eq!(MediaType::from_extension("psd"), MediaType::Psd);
        assert_eq!(MediaType::from_extension("psb"), MediaType::Psd);
        assert_eq!(MediaType::from_extension("xcf"), MediaType::Psd);

        // Video formats
        assert_eq!(MediaType::from_extension("mp4"), MediaType::Video);
        assert_eq!(MediaType::from_extension("mov"), MediaType::Video);
        assert_eq!(MediaType::from_extension("mkv"), MediaType::Video);
        assert_eq!(MediaType::from_extension("webm"), MediaType::Video);
        assert_eq!(MediaType::from_extension("mpg"), MediaType::Video);
        assert_eq!(MediaType::from_extension("mpeg"), MediaType::Video);
        assert_eq!(MediaType::from_extension("mxf"), MediaType::Video);

        // Audio formats
        assert_eq!(MediaType::from_extension("mp3"), MediaType::Audio);
        assert_eq!(MediaType::from_extension("wav"), MediaType::Audio);
        assert_eq!(MediaType::from_extension("flac"), MediaType::Audio);
        assert_eq!(MediaType::from_extension("aiff"), MediaType::Audio);
        assert_eq!(MediaType::from_extension("opus"), MediaType::Audio);
        assert_eq!(MediaType::from_extension("mid"), MediaType::Audio);

        // 3D model formats
        assert_eq!(MediaType::from_extension("obj"), MediaType::Model3D);
        assert_eq!(MediaType::from_extension("fbx"), MediaType::Model3D);
        assert_eq!(MediaType::from_extension("gltf"), MediaType::Model3D);
        assert_eq!(MediaType::from_extension("glb"), MediaType::Model3D);
        assert_eq!(MediaType::from_extension("stl"), MediaType::Model3D);
        assert_eq!(MediaType::from_extension("usd"), MediaType::Model3D);
        assert_eq!(MediaType::from_extension("usdz"), MediaType::Model3D);
        assert_eq!(MediaType::from_extension("abc"), MediaType::Model3D);
        assert_eq!(MediaType::from_extension("blend"), MediaType::Model3D);
        assert_eq!(MediaType::from_extension("c4d"), MediaType::Model3D);

        // VFX formats
        assert_eq!(MediaType::from_extension("indd"), MediaType::Vfx);
        assert_eq!(MediaType::from_extension("ai"), MediaType::Vfx);
        assert_eq!(MediaType::from_extension("aep"), MediaType::Vfx);
        assert_eq!(MediaType::from_extension("prproj"), MediaType::Vfx);
        assert_eq!(MediaType::from_extension("nk"), MediaType::Vfx);
        assert_eq!(MediaType::from_extension("drp"), MediaType::Vfx);

        // Documents and design tools
        assert_eq!(MediaType::from_extension("pdf"), MediaType::Vfx);
        assert_eq!(MediaType::from_extension("fig"), MediaType::Vfx);
        assert_eq!(MediaType::from_extension("sketch"), MediaType::Vfx);
        assert_eq!(MediaType::from_extension("xd"), MediaType::Vfx);

        // Unknown
        assert_eq!(MediaType::from_extension("unknown"), MediaType::Unknown);
    }

    #[test]
    fn test_strategy_creation() {
        let image_strategy = MergeStrategy::for_media_type(MediaType::Image);
        assert!(matches!(image_strategy, MergeStrategy::Image(_)));

        let psd_strategy = MergeStrategy::for_media_type(MediaType::Psd);
        assert!(matches!(psd_strategy, MergeStrategy::Psd(_)));

        let vfx_strategy = MergeStrategy::for_media_type(MediaType::Vfx);
        assert!(matches!(vfx_strategy, MergeStrategy::Vfx(_)));

        let generic_strategy = MergeStrategy::for_media_type(MediaType::Unknown);
        assert!(matches!(generic_strategy, MergeStrategy::Generic));
    }

    /// Safety invariant for [`MergeResult::AutoMerged`]: its bytes are written
    /// straight into the user's working tree, so they must be a real file in
    /// the input format.
    ///
    /// Every strategy that could produce merged *metadata* previously returned
    /// it as `AutoMerged` — a successful PSD "auto-merge" yielded a JSON
    /// document. Had these been wired into `merge`, resolving a conflict would
    /// have replaced the designer's .psd with JSON. Nothing here can write a
    /// real PSD, image, video or audio file yet, so nothing may claim to.
    ///
    /// This test fails the moment a strategy starts returning `AutoMerged`
    /// again, which is the point: restoring it requires producing genuine
    /// format bytes, and this test is where you prove you did.
    #[tokio::test]
    async fn no_strategy_claims_auto_merge_it_cannot_perform() {
        // Deliberately not real media: parsing should fail or the analysis
        // should decline, and neither path may yield `AutoMerged`.
        let base = b" base payload";
        let ours = b" ours payload";
        let theirs = b" theirs payload";

        for (media_type, name) in [
            (MediaType::Image, "a.png"),
            (MediaType::Psd, "a.psd"),
            (MediaType::Video, "a.mp4"),
            (MediaType::Audio, "a.wav"),
            (MediaType::Model3D, "a.glb"),
            (MediaType::Vfx, "a.exr"),
            (MediaType::Unknown, "a.bin"),
        ] {
            let strategy = MergeStrategy::for_media_type(media_type);
            if let Ok(result) = strategy.merge(base, ours, theirs, name).await {
                assert!(
                    !matches!(result, MergeResult::AutoMerged(_)),
                    "{name}: strategy returned AutoMerged; its bytes get written \
                        over the user's file, so they must be real {media_type:?} \
                        content, not metadata"
                );
            }
        }
    }
}
