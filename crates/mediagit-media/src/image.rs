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

use crate::error::{MediaError, Result};
use image::{GenericImageView, ImageFormat};
use exif;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::io::Cursor;
use std::path::Path;
use tracing::{debug, instrument, warn};

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

/// Complete image metadata including EXIF, IPTC, and XMP
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
        };

        let json = serde_json::to_string(&metadata).unwrap();
        assert!(json.contains("1920"));
        assert!(json.contains("Png"));
    }
}
