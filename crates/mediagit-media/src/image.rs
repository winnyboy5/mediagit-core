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

//! Image metadata extraction and analysis
//!
//! Supports:
//! - EXIF metadata (camera settings, GPS, timestamps)
//! - IPTC metadata (copyright, keywords, captions)
//! - XMP metadata (extended metadata)
//! - Format detection (PNG, JPEG, TIFF, WebP)
//! - Perceptual hashing for visual similarity detection

use crate::error::{MediaError, Result};
use image::{GenericImageView, ImageFormat};
use img_hash::{HashAlg, HasherConfig};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::io::Cursor;
use std::path::Path;
use tracing::{debug, info, instrument, warn};

/// Image format enumeration
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SupportedImageFormat {
    Png,
    Jpeg,
    Tiff,
    WebP,
}

impl SupportedImageFormat {
    /// Detect format from file extension
    pub fn from_extension(ext: &str) -> Option<Self> {
        match ext.to_lowercase().as_str() {
            "png" => Some(Self::Png),
            "jpg" | "jpeg" => Some(Self::Jpeg),
            "tif" | "tiff" => Some(Self::Tiff),
            "webp" => Some(Self::WebP),
            _ => None,
        }
    }

    /// Convert to image crate format
    pub fn to_image_format(self) -> ImageFormat {
        match self {
            Self::Png => ImageFormat::Png,
            Self::Jpeg => ImageFormat::Jpeg,
            Self::Tiff => ImageFormat::Tiff,
            Self::WebP => ImageFormat::WebP,
        }
    }
}

/// Complete image metadata including EXIF, IPTC, XMP, and perceptual hash
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImageMetadata {
    /// Image dimensions
    pub width: u32,
    pub height: u32,

    /// Image format
    pub format: SupportedImageFormat,

    /// File size in bytes
    pub file_size: u64,

    /// EXIF metadata
    pub exif: Option<ExifMetadata>,

    /// IPTC metadata (keywords, copyright, etc.)
    pub iptc: Option<IptcMetadata>,

    /// XMP metadata (extended metadata)
    pub xmp: Option<XmpMetadata>,

    /// Color space information
    pub color_space: Option<String>,

    /// Bits per channel
    pub bit_depth: Option<u8>,

    /// Perceptual hash for visual similarity detection
    pub perceptual_hash: PerceptualHash,
}

/// Perceptual hash for image similarity detection
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PerceptualHash {
    /// Hash algorithm used
    pub algorithm: HashAlgorithm,

    /// Hash value as base64 string
    pub hash_value: String,

    /// Hash bit size
    pub hash_size: u32,
}

impl PerceptualHash {
    /// Calculate similarity score (0.0 = completely different, 1.0 = identical)
    pub fn similarity(&self, other: &PerceptualHash) -> Option<f64> {
        if self.algorithm != other.algorithm || self.hash_size != other.hash_size {
            return None;
        }

        // img_hash uses base64 encoding, calculate Hamming distance
        let hamming_distance = img_hash::ImageHash::<Vec<u8>>::dist(
            &img_hash::ImageHash::from_base64(&self.hash_value).ok()?,
            &img_hash::ImageHash::from_base64(&other.hash_value).ok()?,
        );

        let max_distance = (self.hash_size * self.hash_size) as u32;
        Some(1.0 - (hamming_distance as f64 / max_distance as f64))
    }
}

/// Hash algorithm used for perceptual hashing
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum HashAlgorithm {
    /// Mean hash (average hash)
    Mean,
    /// Gradient hash
    Gradient,
    /// Double gradient hash (most robust for photos)
    DoubleGradient,
    /// Vertical gradient hash
    VertGradient,
    /// Block hash
    Blockhash,
}

/// EXIF metadata extracted from image
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExifMetadata {
    /// Camera make
    pub make: Option<String>,

    /// Camera model
    pub model: Option<String>,

    /// Lens information
    pub lens_model: Option<String>,

    /// ISO speed
    pub iso: Option<u32>,

    /// Exposure time (shutter speed)
    pub exposure_time: Option<String>,

    /// F-number (aperture)
    pub f_number: Option<f64>,

    /// Focal length
    pub focal_length: Option<f64>,

    /// Date/time original
    pub datetime_original: Option<String>,

    /// GPS latitude
    pub gps_latitude: Option<f64>,

    /// GPS longitude
    pub gps_longitude: Option<f64>,

    /// GPS altitude
    pub gps_altitude: Option<f64>,

    /// Image orientation
    pub orientation: Option<u32>,

    /// Software used
    pub software: Option<String>,

    /// Additional raw fields
    pub raw_fields: HashMap<String, String>,
}

/// IPTC metadata for image cataloging
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IptcMetadata {
    /// Keywords/tags
    pub keywords: Vec<String>,

    /// Caption/description
    pub caption: Option<String>,

    /// Copyright notice
    pub copyright: Option<String>,

    /// Creator/photographer
    pub creator: Option<String>,

    /// Credit line
    pub credit: Option<String>,

    /// Source
    pub source: Option<String>,

    /// City
    pub city: Option<String>,

    /// Country
    pub country: Option<String>,
}

/// XMP metadata (extensible metadata)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct XmpMetadata {
    /// Rating (0-5 stars typically)
    pub rating: Option<u8>,

    /// Color label
    pub color_label: Option<String>,

    /// Edit history
    pub history: Vec<String>,

    /// Additional XMP fields
    pub raw_fields: HashMap<String, String>,
}

/// Image metadata parser
pub struct ImageMetadataParser;

impl ImageMetadataParser {
    /// Parse complete image metadata from file
    #[instrument(skip(data), fields(size = data.len()))]
    pub async fn parse(data: &[u8], filename: &str) -> Result<ImageMetadata> {
        debug!("Parsing image metadata for {}", filename);

        // Detect format
        let format = Self::detect_format(data, filename)?;
        debug!("Detected format: {:?}", format);

        // Parse basic image information
        let img = image::load_from_memory(data)
            .map_err(|e| MediaError::ImageError(e.to_string()))?;

        let (width, height) = img.dimensions();
        let file_size = data.len() as u64;

        // Calculate perceptual hash
        info!("Calculating perceptual hash for {}", filename);
        let perceptual_hash = Self::calculate_perceptual_hash(&img)?;
        debug!("Perceptual hash calculated: {} (algorithm: {:?})",
               perceptual_hash.hash_value, perceptual_hash.algorithm);

        // Extract EXIF if present
        let exif = Self::extract_exif(data).await.ok();

        // Extract IPTC if present
        let iptc = Self::extract_iptc(data).await.ok();

        // Extract XMP if present
        let xmp = Self::extract_xmp(data).await.ok();

        // Determine color space
        let color_space = Some(format!("{:?}", img.color()));

        // Bit depth (simplified)
        let bit_depth = Some(8); // Most common, could be detected more accurately

        Ok(ImageMetadata {
            width,
            height,
            format,
            file_size,
            exif,
            iptc,
            xmp,
            color_space,
            bit_depth,
            perceptual_hash,
        })
    }

    /// Calculate perceptual hash for an image
    fn calculate_perceptual_hash(img: &image::DynamicImage) -> Result<PerceptualHash> {
        // Use DoubleGradient (DCT-like) for best robustness
        let hasher = HasherConfig::new()
            .hash_size(8, 8) // 8x8 = 64-bit hash
            .hash_alg(HashAlg::DoubleGradient)
            .to_hasher();

        // With image 0.23 matching img_hash 3.2, DynamicImage works directly
        let hash = hasher.hash_image(img);
        let hash_value = hash.to_base64();

        Ok(PerceptualHash {
            algorithm: HashAlgorithm::DoubleGradient,
            hash_value,
            hash_size: 8,
        })
    }

    /// Detect image format from data
    fn detect_format(data: &[u8], filename: &str) -> Result<SupportedImageFormat> {
        // Try extension first
        if let Some(ext) = Path::new(filename).extension() {
            if let Some(format) = SupportedImageFormat::from_extension(
                ext.to_str().unwrap_or("")
            ) {
                return Ok(format);
            }
        }

        // Detect from magic bytes
        let format = image::guess_format(data)
            .map_err(|e| MediaError::ImageError(e.to_string()))?;

        match format {
            ImageFormat::Png => Ok(SupportedImageFormat::Png),
            ImageFormat::Jpeg => Ok(SupportedImageFormat::Jpeg),
            ImageFormat::Tiff => Ok(SupportedImageFormat::Tiff),
            ImageFormat::WebP => Ok(SupportedImageFormat::WebP),
            _ => Err(MediaError::UnsupportedFormat(format!("{:?}", format))),
        }
    }

    /// Extract EXIF metadata
    #[instrument(skip(data))]
    async fn extract_exif(data: &[u8]) -> Result<ExifMetadata> {
        let mut cursor = Cursor::new(data);
        let exif_reader = exif::Reader::new();

        let exif = exif_reader
            .read_from_container(&mut cursor)
            .map_err(|e| MediaError::ExifError(e.to_string()))?;

        let mut metadata = ExifMetadata {
            make: None,
            model: None,
            lens_model: None,
            iso: None,
            exposure_time: None,
            f_number: None,
            focal_length: None,
            datetime_original: None,
            gps_latitude: None,
            gps_longitude: None,
            gps_altitude: None,
            orientation: None,
            software: None,
            raw_fields: HashMap::new(),
        };

        // Extract common fields
        for field in exif.fields() {
            let tag = field.tag;
            let value = field.display_value().to_string();

            match tag {
                exif::Tag::Make => metadata.make = Some(value),
                exif::Tag::Model => metadata.model = Some(value),
                exif::Tag::LensModel => metadata.lens_model = Some(value),
                exif::Tag::PhotographicSensitivity => {
                    metadata.iso = value.parse().ok();
                }
                exif::Tag::ExposureTime => metadata.exposure_time = Some(value),
                exif::Tag::FNumber => {
                    metadata.f_number = value.parse().ok();
                }
                exif::Tag::FocalLength => {
                    metadata.focal_length = value.parse().ok();
                }
                exif::Tag::DateTimeOriginal => metadata.datetime_original = Some(value),
                exif::Tag::Orientation => {
                    metadata.orientation = value.parse().ok();
                }
                exif::Tag::Software => metadata.software = Some(value),
                _ => {
                    // Store other fields
                    metadata.raw_fields.insert(
                        format!("{:?}", tag),
                        value,
                    );
                }
            }
        }

        debug!("Extracted EXIF metadata with {} raw fields", metadata.raw_fields.len());
        Ok(metadata)
    }

    /// Extract IPTC metadata
    #[instrument(skip(_data))]
    async fn extract_iptc(_data: &[u8]) -> Result<IptcMetadata> {
        // IPTC parsing would go here
        // For now, return empty metadata as a placeholder
        warn!("IPTC extraction not yet fully implemented");

        Ok(IptcMetadata {
            keywords: Vec::new(),
            caption: None,
            copyright: None,
            creator: None,
            credit: None,
            source: None,
            city: None,
            country: None,
        })
    }

    /// Extract XMP metadata
    #[instrument(skip(_data))]
    async fn extract_xmp(_data: &[u8]) -> Result<XmpMetadata> {
        // XMP parsing would go here
        // For now, return empty metadata as a placeholder
        warn!("XMP extraction not yet fully implemented");

        Ok(XmpMetadata {
            rating: None,
            color_label: None,
            history: Vec::new(),
            raw_fields: HashMap::new(),
        })
    }

    /// Compare two image metadata for merge intelligence
    pub fn compare_metadata(
        base: &ImageMetadata,
        ours: &ImageMetadata,
        theirs: &ImageMetadata,
    ) -> MetadataComparison {
        MetadataComparison {
            dimensions_changed: (ours.width, ours.height) != (base.width, base.height)
                || (theirs.width, theirs.height) != (base.width, base.height),
            format_changed: ours.format != base.format || theirs.format != base.format,
            size_delta_ours: ours.file_size as i64 - base.file_size as i64,
            size_delta_theirs: theirs.file_size as i64 - base.file_size as i64,
            both_modified_exif: base.exif.is_some()
                && ours.exif.is_some()
                && theirs.exif.is_some(),
        }
    }

    /// Check if two images can be auto-merged based on perceptual similarity
    ///
    /// Auto-merge is possible when:
    /// - Images are visually identical (high perceptual similarity)
    /// - Only metadata has changed (EXIF, IPTC, XMP)
    /// - Dimensions remain the same
    ///
    /// Similarity threshold: 0.95 (95% similar for auto-merge)
    pub fn can_auto_merge(
        base: &ImageMetadata,
        ours: &ImageMetadata,
        theirs: &ImageMetadata,
    ) -> MergeDecision {
        const SIMILARITY_THRESHOLD: f64 = 0.95;

        // Check dimensions - must match for auto-merge
        if (ours.width, ours.height) != (base.width, base.height)
            || (theirs.width, theirs.height) != (base.width, base.height)
        {
            warn!("Image dimensions changed - manual review required");
            return MergeDecision::ManualReview(vec![
                "Image dimensions changed - dimensions must match for auto-merge".to_string()
            ]);
        }

        // Calculate perceptual similarity
        let ours_similarity = ours
            .perceptual_hash
            .similarity(&base.perceptual_hash)
            .unwrap_or(0.0);

        let theirs_similarity = theirs
            .perceptual_hash
            .similarity(&base.perceptual_hash)
            .unwrap_or(0.0);

        debug!(
            "Perceptual similarity - ours: {:.2}%, theirs: {:.2}%",
            ours_similarity * 100.0,
            theirs_similarity * 100.0
        );

        // If both versions are visually similar to base (only metadata changed)
        if ours_similarity >= SIMILARITY_THRESHOLD
            && theirs_similarity >= SIMILARITY_THRESHOLD
        {
            // Check if the visual content is the same in both versions
            let ours_theirs_similarity = ours
                .perceptual_hash
                .similarity(&theirs.perceptual_hash)
                .unwrap_or(0.0);

            if ours_theirs_similarity >= SIMILARITY_THRESHOLD {
                info!(
                    "Images are visually identical ({:.2}% similarity) - auto-merge metadata",
                    ours_theirs_similarity * 100.0
                );
                return MergeDecision::AutoMerge;
            }
        }

        // If either version has significant visual changes, require manual review
        let mut conflicts = Vec::new();

        if ours_similarity < SIMILARITY_THRESHOLD {
            conflicts.push(format!(
                "Ours version has significant visual changes ({:.2}% similarity to base)",
                ours_similarity * 100.0
            ));
        }

        if theirs_similarity < SIMILARITY_THRESHOLD {
            conflicts.push(format!(
                "Theirs version has significant visual changes ({:.2}% similarity to base)",
                theirs_similarity * 100.0
            ));
        }

        if conflicts.is_empty() {
            conflicts.push("Images are visually different - manual review required".to_string());
        }

        warn!("Image conflicts detected: {:?}", conflicts);
        MergeDecision::ManualReview(conflicts)
    }

    /// Merge image metadata (prefer 'ours' version with intelligent merging)
    pub fn merge_metadata(
        _base: &ImageMetadata,
        ours: &ImageMetadata,
        theirs: &ImageMetadata,
    ) -> Result<ImageMetadata> {
        info!("Merging image metadata - preferring 'ours' version with intelligent merge");

        // Start with 'ours' as base
        let mut merged = ours.clone();

        // Merge EXIF metadata intelligently
        if let (Some(ours_exif), Some(theirs_exif)) =
            (&ours.exif, &theirs.exif)
        {
            let mut merged_exif = ours_exif.clone();

            // Prefer non-None values from theirs if ours is None
            if merged_exif.make.is_none() {
                merged_exif.make = theirs_exif.make.clone();
            }
            if merged_exif.model.is_none() {
                merged_exif.model = theirs_exif.model.clone();
            }
            if merged_exif.lens_model.is_none() {
                merged_exif.lens_model = theirs_exif.lens_model.clone();
            }
            if merged_exif.software.is_none() {
                merged_exif.software = theirs_exif.software.clone();
            }

            // Merge raw_fields - combine both
            for (key, value) in &theirs_exif.raw_fields {
                merged_exif.raw_fields.entry(key.clone()).or_insert_with(|| value.clone());
            }

            merged.exif = Some(merged_exif);
        } else if theirs.exif.is_some() {
            // If ours has no EXIF but theirs does, use theirs
            merged.exif = theirs.exif.clone();
        }

        // Merge IPTC metadata
        if let (Some(ours_iptc), Some(theirs_iptc)) = (&ours.iptc, &theirs.iptc) {
            let mut merged_iptc = ours_iptc.clone();

            // Merge keywords - combine both
            let mut keywords_set: std::collections::HashSet<String> =
                ours_iptc.keywords.iter().cloned().collect();
            keywords_set.extend(theirs_iptc.keywords.iter().cloned());
            merged_iptc.keywords = keywords_set.into_iter().collect();

            // Prefer non-None values
            if merged_iptc.caption.is_none() {
                merged_iptc.caption = theirs_iptc.caption.clone();
            }
            if merged_iptc.copyright.is_none() {
                merged_iptc.copyright = theirs_iptc.copyright.clone();
            }
            if merged_iptc.creator.is_none() {
                merged_iptc.creator = theirs_iptc.creator.clone();
            }

            merged.iptc = Some(merged_iptc);
        } else if theirs.iptc.is_some() {
            merged.iptc = theirs.iptc.clone();
        }

        // Merge XMP metadata
        if let (Some(ours_xmp), Some(theirs_xmp)) = (&ours.xmp, &theirs.xmp) {
            let mut merged_xmp = ours_xmp.clone();

            // Prefer ours rating unless theirs is higher
            if let Some(theirs_rating) = theirs_xmp.rating {
                if let Some(ours_rating) = ours_xmp.rating {
                    merged_xmp.rating = Some(ours_rating.max(theirs_rating));
                } else {
                    merged_xmp.rating = Some(theirs_rating);
                }
            }

            // Combine edit history
            let mut history_set: std::collections::HashSet<String> =
                ours_xmp.history.iter().cloned().collect();
            history_set.extend(theirs_xmp.history.iter().cloned());
            merged_xmp.history = history_set.into_iter().collect();

            // Merge raw_fields
            for (key, value) in &theirs_xmp.raw_fields {
                merged_xmp.raw_fields.entry(key.clone()).or_insert_with(|| value.clone());
            }

            merged.xmp = Some(merged_xmp);
        } else if theirs.xmp.is_some() {
            merged.xmp = theirs.xmp.clone();
        }

        Ok(merged)
    }
}

/// Image merge decision
#[derive(Debug, Clone)]
pub enum MergeDecision {
    /// Images are visually identical, can auto-merge metadata
    AutoMerge,
    /// Images have visual differences, needs manual review
    ManualReview(Vec<String>),
}

/// Result of metadata comparison for merge decisions
#[derive(Debug, Clone)]
pub struct MetadataComparison {
    /// Whether image dimensions changed
    pub dimensions_changed: bool,

    /// Whether format changed
    pub format_changed: bool,

    /// Size change in ours branch
    pub size_delta_ours: i64,

    /// Size change in theirs branch
    pub size_delta_theirs: i64,

    /// Both sides modified EXIF
    pub both_modified_exif: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_detection() {
        assert_eq!(
            SupportedImageFormat::from_extension("png"),
            Some(SupportedImageFormat::Png)
        );
        assert_eq!(
            SupportedImageFormat::from_extension("jpg"),
            Some(SupportedImageFormat::Jpeg)
        );
        assert_eq!(
            SupportedImageFormat::from_extension("webp"),
            Some(SupportedImageFormat::WebP)
        );
    }

    #[tokio::test]
    async fn test_metadata_structure() {
        // Test that metadata structures serialize properly
        let metadata = ImageMetadata {
            width: 1920,
            height: 1080,
            format: SupportedImageFormat::Png,
            file_size: 1024,
            exif: None,
            iptc: None,
            xmp: None,
            color_space: Some("RGB".to_string()),
            bit_depth: Some(8),
            perceptual_hash: PerceptualHash {
                algorithm: HashAlgorithm::DoubleGradient,
                hash_value: "test_hash_value".to_string(),
                hash_size: 8,
            },
        };

        let json = serde_json::to_string(&metadata).unwrap();
        assert!(json.contains("1920"));
        assert!(json.contains("Png"));
        assert!(json.contains("DoubleGradient"));
    }
}
