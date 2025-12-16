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

//! VFX and design file format parsing
//!
//! This module provides metadata extraction for professional VFX and design tools:
//! - Adobe InDesign (.indd)
//! - Adobe Illustrator (.ai)
//! - Adobe After Effects (.aep)
//! - Adobe Premiere Pro (.prproj)
//!
//! These are complex proprietary binary formats. This implementation focuses on:
//! - Format detection and validation
//! - Basic metadata extraction (version, page count, dimensions)
//! - Intelligent conflict detection based on structural changes
//!
//! # Example
//!
//! ```rust,no_run
//! use mediagit_media::vfx::VfxParser;
//!
//! # async fn example() -> anyhow::Result<()> {
//! let file_data = std::fs::read("document.indd")?;
//! let parser = VfxParser::new();
//! let info = parser.parse(&file_data, "document.indd").await?;
//!
//! println!("Format: {:?}, Version: {:?}", info.format, info.version);
//! println!("Pages: {:?}", info.page_count);
//! # Ok(())
//! # }
//! ```

use crate::error::{MediaError, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use tracing::{debug, info, instrument, warn};

/// Complete VFX file information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VfxInfo {
    /// VFX format type
    pub format: VfxFormat,

    /// Application version
    pub version: Option<String>,

    /// Page/artboard count (for layout apps)
    pub page_count: Option<u32>,

    /// Composition/timeline duration in seconds (for video/animation)
    pub duration_seconds: Option<f64>,

    /// Document dimensions (width, height) in points
    pub dimensions: Option<(f32, f32)>,

    /// Layer count (if applicable)
    pub layer_count: Option<u32>,

    /// Linked assets/resources
    pub linked_assets: Vec<String>,

    /// Embedded fonts
    pub fonts: Vec<String>,

    /// Color mode (RGB, CMYK, etc.)
    pub color_mode: Option<String>,

    /// File size in bytes
    pub file_size: u64,

    /// Additional metadata fields
    pub metadata: HashMap<String, String>,
}

/// VFX file format enumeration
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum VfxFormat {
    /// Adobe InDesign document
    InDesign,
    /// Adobe Illustrator file
    Illustrator,
    /// Adobe After Effects project
    AfterEffects,
    /// Adobe Premiere Pro project
    Premiere,
    /// Unknown VFX format
    Unknown,
}

impl VfxFormat {
    /// Detect format from file extension
    pub fn from_extension(ext: &str) -> Self {
        match ext.to_lowercase().as_str() {
            "indd" | "indt" => VfxFormat::InDesign,
            "ai" | "ait" => VfxFormat::Illustrator,
            "aep" | "aet" => VfxFormat::AfterEffects,
            "prproj" => VfxFormat::Premiere,
            _ => VfxFormat::Unknown,
        }
    }

    /// Get human-readable format name
    pub fn name(&self) -> &str {
        match self {
            VfxFormat::InDesign => "Adobe InDesign",
            VfxFormat::Illustrator => "Adobe Illustrator",
            VfxFormat::AfterEffects => "Adobe After Effects",
            VfxFormat::Premiere => "Adobe Premiere Pro",
            VfxFormat::Unknown => "Unknown",
        }
    }
}

/// VFX file parser
#[derive(Debug)]
pub struct VfxParser;

impl VfxParser {
    /// Create a new VFX parser
    pub fn new() -> Self {
        VfxParser
    }

    /// Parse VFX file from bytes
    #[instrument(skip(data), fields(size = data.len(), filename = %filename))]
    pub async fn parse(&self, data: &[u8], filename: &str) -> Result<VfxInfo> {
        info!("Parsing VFX file: {}", filename);

        let format = Self::detect_format(filename, data)?;
        debug!("Detected VFX format: {:?}", format);

        match format {
            VfxFormat::InDesign => self.parse_indesign(data).await,
            VfxFormat::Illustrator => self.parse_illustrator(data).await,
            VfxFormat::AfterEffects => self.parse_after_effects(data).await,
            VfxFormat::Premiere => self.parse_premiere(data).await,
            VfxFormat::Unknown => Err(MediaError::UnsupportedFormat(
                "Unknown VFX format".to_string(),
            )),
        }
    }

    /// Detect VFX format from filename and magic bytes
    fn detect_format(filename: &str, data: &[u8]) -> Result<VfxFormat> {
        // First try extension
        let ext_format = filename
            .split('.')
            .last()
            .map(VfxFormat::from_extension)
            .unwrap_or(VfxFormat::Unknown);

        if ext_format != VfxFormat::Unknown {
            // Validate with magic bytes if possible
            let validated = Self::validate_magic_bytes(data, ext_format);
            if validated {
                return Ok(ext_format);
            }
        }

        // Try to detect from magic bytes alone
        Self::detect_from_magic_bytes(data)
    }

    /// Validate magic bytes for a given format
    fn validate_magic_bytes(data: &[u8], format: VfxFormat) -> bool {
        if data.len() < 16 {
            return false;
        }

        match format {
            VfxFormat::InDesign => {
                // InDesign uses compound document format
                data.starts_with(&[0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1])
            }
            VfxFormat::Illustrator => {
                // AI files are often PDF-based or PostScript
                data.starts_with(b"%PDF") || data.starts_with(b"%!PS-Adobe")
            }
            VfxFormat::AfterEffects => {
                // AEP uses RIFF format
                data.starts_with(b"RIFX") || data.starts_with(b"RIFF")
            }
            VfxFormat::Premiere => {
                // Premiere projects are XML-based
                data.starts_with(b"<?xml")
            }
            VfxFormat::Unknown => false,
        }
    }

    /// Detect format from magic bytes alone
    fn detect_from_magic_bytes(data: &[u8]) -> Result<VfxFormat> {
        if data.len() < 16 {
            return Err(MediaError::InvalidStructure("File too small".to_string()));
        }

        // Check for InDesign (compound document)
        if data.starts_with(&[0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1]) {
            return Ok(VfxFormat::InDesign);
        }

        // Check for Illustrator (PDF/PostScript)
        if data.starts_with(b"%PDF") || data.starts_with(b"%!PS-Adobe") {
            return Ok(VfxFormat::Illustrator);
        }

        // Check for After Effects (RIFF)
        if data.starts_with(b"RIFX") || data.starts_with(b"RIFF") {
            return Ok(VfxFormat::AfterEffects);
        }

        // Check for Premiere (XML)
        if data.starts_with(b"<?xml") {
            return Ok(VfxFormat::Premiere);
        }

        Err(MediaError::UnsupportedFormat("Unknown VFX format".to_string()))
    }

    /// Parse Adobe InDesign file
    #[instrument(skip(data))]
    async fn parse_indesign(&self, data: &[u8]) -> Result<VfxInfo> {
        debug!("Parsing InDesign file");

        // InDesign files are compound documents (OLE/CFB format)
        // Full parsing would require OLE2 parser - we'll extract basic metadata

        let mut metadata = HashMap::new();

        // Try to find version information in the binary
        let text = String::from_utf8_lossy(data);

        // Look for version markers
        if text.contains("InDesign") {
            metadata.insert("application".to_string(), "Adobe InDesign".to_string());
        }

        Ok(VfxInfo {
            format: VfxFormat::InDesign,
            version: None, // Would require OLE2 parsing
            page_count: None, // Would require full document parsing
            duration_seconds: None,
            dimensions: None,
            layer_count: None,
            linked_assets: Vec::new(),
            fonts: Vec::new(),
            color_mode: Some("CMYK".to_string()), // InDesign default
            file_size: data.len() as u64,
            metadata,
        })
    }

    /// Parse Adobe Illustrator file
    #[instrument(skip(data))]
    async fn parse_illustrator(&self, data: &[u8]) -> Result<VfxInfo> {
        debug!("Parsing Illustrator file");

        let mut metadata = HashMap::new();
        let mut fonts = Vec::new();
        let mut linked_assets = Vec::new();

        // Illustrator files are often PDF-based or PostScript
        if data.starts_with(b"%PDF") {
            // PDF-based AI file
            metadata.insert("format".to_string(), "PDF-based".to_string());

            // Try to extract version from PDF header
            if let Some(version_start) = data.windows(5).position(|w| w == b"%PDF-") {
                if data.len() > version_start + 8 {
                    let version_bytes = &data[version_start + 5..version_start + 8];
                    if let Ok(version) = String::from_utf8(version_bytes.to_vec()) {
                        metadata.insert("pdf_version".to_string(), version);
                    }
                }
            }
        } else if data.starts_with(b"%!PS-Adobe") {
            // PostScript-based AI file
            metadata.insert("format".to_string(), "PostScript-based".to_string());
        }

        // Try to parse as text for some metadata
        let content = String::from_utf8_lossy(data);

        // Count layers (simplified - look for layer markers)
        let layer_count = content.matches("/Layer").count() as u32;

        // Extract font references
        for line in content.lines() {
            if line.contains("/FontName") || line.contains("/BaseFont") {
                if let Some(font_start) = line.find('/') {
                    if let Some(font_end) = line[font_start..].find(char::is_whitespace) {
                        let font_name = &line[font_start + 1..font_start + font_end];
                        if !font_name.is_empty() && !fonts.contains(&font_name.to_string()) {
                            fonts.push(font_name.to_string());
                        }
                    }
                }
            }

            // Look for linked images
            if line.contains("/ImageFile") || line.contains("(*.jpg)") || line.contains("(*.png)") {
                // Extract filename if possible
                if let Some(start) = line.find('(') {
                    if let Some(end) = line[start..].find(')') {
                        let asset = &line[start + 1..start + end];
                        if !asset.is_empty() {
                            linked_assets.push(asset.to_string());
                        }
                    }
                }
            }
        }

        debug!("Parsed AI: layers={}, fonts={}", layer_count, fonts.len());

        Ok(VfxInfo {
            format: VfxFormat::Illustrator,
            version: None,
            page_count: Some(1), // AI files typically have one artboard (simplified)
            duration_seconds: None,
            dimensions: None, // Would need to parse BoundingBox
            layer_count: Some(layer_count),
            linked_assets,
            fonts,
            color_mode: Some("RGB".to_string()),
            file_size: data.len() as u64,
            metadata,
        })
    }

    /// Parse Adobe After Effects project
    #[instrument(skip(data))]
    async fn parse_after_effects(&self, data: &[u8]) -> Result<VfxInfo> {
        debug!("Parsing After Effects project");

        // AEP files use RIFF format
        if !data.starts_with(b"RIFX") && !data.starts_with(b"RIFF") {
            return Err(MediaError::InvalidStructure(
                "Not a valid After Effects project".to_string(),
            ));
        }

        let mut metadata = HashMap::new();
        let endian = if data.starts_with(b"RIFX") {
            "big"
        } else {
            "little"
        };

        metadata.insert("endian".to_string(), endian.to_string());

        Ok(VfxInfo {
            format: VfxFormat::AfterEffects,
            version: None, // Would require RIFF chunk parsing
            page_count: None,
            duration_seconds: None, // Would need composition analysis
            dimensions: None,        // Would need composition analysis
            layer_count: None,       // Would need composition analysis
            linked_assets: Vec::new(),
            fonts: Vec::new(),
            color_mode: Some("RGB".to_string()),
            file_size: data.len() as u64,
            metadata,
        })
    }

    /// Parse Adobe Premiere Pro project
    #[instrument(skip(data))]
    async fn parse_premiere(&self, data: &[u8]) -> Result<VfxInfo> {
        debug!("Parsing Premiere Pro project");

        // Premiere projects are XML-based
        let content = std::str::from_utf8(data).map_err(|e| {
            MediaError::InvalidStructure(format!("Invalid UTF-8 in Premiere project: {}", e))
        })?;

        let mut metadata = HashMap::new();
        let mut linked_assets = Vec::new();

        // Extract version from XML
        if let Some(version_start) = content.find("Version=\"") {
            if let Some(version_end) = content[version_start + 9..].find('"') {
                let version = &content[version_start + 9..version_start + 9 + version_end];
                metadata.insert("version".to_string(), version.to_string());
            }
        }

        // Count sequences (simplified)
        let sequence_count = content.matches("<Sequence").count() as u32;
        metadata.insert("sequences".to_string(), sequence_count.to_string());

        // Extract linked media files
        for line in content.lines() {
            if line.contains("pathurl=") || line.contains("FilePath=") {
                if let Some(start) = line.find('"') {
                    if let Some(end) = line[start + 1..].find('"') {
                        let asset = &line[start + 1..start + 1 + end];
                        if !asset.is_empty() {
                            linked_assets.push(asset.to_string());
                        }
                    }
                }
            }
        }

        debug!("Parsed Premiere: sequences={}, assets={}", sequence_count, linked_assets.len());

        Ok(VfxInfo {
            format: VfxFormat::Premiere,
            version: metadata.get("version").cloned(),
            page_count: None,
            duration_seconds: None, // Would need timeline analysis
            dimensions: None,
            layer_count: Some(sequence_count), // Using sequences as "layers"
            linked_assets,
            fonts: Vec::new(),
            color_mode: Some("RGB".to_string()),
            file_size: data.len() as u64,
            metadata,
        })
    }

    /// Check if two VFX files can be auto-merged
    pub fn can_auto_merge(base: &VfxInfo, ours: &VfxInfo, theirs: &VfxInfo) -> MergeDecision {
        // Basic conflict detection based on metadata changes

        // Check if formats match
        if ours.format != base.format || theirs.format != base.format {
            warn!("VFX format changed between versions");
            return MergeDecision::ManualReview(vec![
                "VFX file format changed - manual review required".to_string()
            ]);
        }

        let mut conflicts = Vec::new();

        // Check for page count changes (layout files)
        if let (Some(base_pages), Some(ours_pages), Some(theirs_pages)) =
            (base.page_count, ours.page_count, theirs.page_count)
        {
            if ours_pages != base_pages && theirs_pages != base_pages {
                conflicts.push(format!(
                    "Both branches modified page count (ours: {} pages, theirs: {} pages)",
                    ours_pages, theirs_pages
                ));
            }
        }

        // Check for layer count changes
        if let (Some(base_layers), Some(ours_layers), Some(theirs_layers)) =
            (base.layer_count, ours.layer_count, theirs.layer_count)
        {
            if ours_layers != base_layers && theirs_layers != base_layers {
                conflicts.push(format!(
                    "Both branches modified layer count (ours: {} layers, theirs: {} layers)",
                    ours_layers, theirs_layers
                ));
            }
        }

        // Check for duration changes (video/animation files)
        if let (Some(base_dur), Some(ours_dur), Some(theirs_dur)) =
            (base.duration_seconds, ours.duration_seconds, theirs.duration_seconds)
        {
            let ours_changed = (ours_dur - base_dur).abs() > 0.1;
            let theirs_changed = (theirs_dur - base_dur).abs() > 0.1;

            if ours_changed && theirs_changed {
                conflicts.push(format!(
                    "Both branches modified duration (ours: {:.2}s, theirs: {:.2}s)",
                    ours_dur, theirs_dur
                ));
            }
        }

        // Check for linked asset changes
        let ours_assets_changed = ours.linked_assets.len() != base.linked_assets.len();
        let theirs_assets_changed = theirs.linked_assets.len() != base.linked_assets.len();

        if ours_assets_changed && theirs_assets_changed {
            conflicts.push(format!(
                "Both branches modified linked assets (ours: {} assets, theirs: {} assets)",
                ours.linked_assets.len(),
                theirs.linked_assets.len()
            ));
        }

        if conflicts.is_empty() {
            info!("No VFX file conflicts detected - can auto-merge");
            MergeDecision::AutoMerge
        } else {
            warn!("Found {} VFX file conflicts", conflicts.len());
            MergeDecision::ManualReview(conflicts)
        }
    }
}

impl Default for VfxParser {
    fn default() -> Self {
        Self::new()
    }
}

/// VFX merge decision
#[derive(Debug, Clone)]
pub enum MergeDecision {
    /// No conflicts, can auto-merge
    AutoMerge,
    /// Conflicts detected, needs manual review
    ManualReview(Vec<String>),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_detection() {
        assert_eq!(VfxFormat::from_extension("indd"), VfxFormat::InDesign);
        assert_eq!(VfxFormat::from_extension("ai"), VfxFormat::Illustrator);
        assert_eq!(VfxFormat::from_extension("aep"), VfxFormat::AfterEffects);
        assert_eq!(VfxFormat::from_extension("prproj"), VfxFormat::Premiere);
        assert_eq!(VfxFormat::from_extension("unknown"), VfxFormat::Unknown);
    }

    #[test]
    fn test_format_names() {
        assert_eq!(VfxFormat::InDesign.name(), "Adobe InDesign");
        assert_eq!(VfxFormat::Illustrator.name(), "Adobe Illustrator");
        assert_eq!(VfxFormat::AfterEffects.name(), "Adobe After Effects");
        assert_eq!(VfxFormat::Premiere.name(), "Adobe Premiere Pro");
    }

    #[test]
    fn test_magic_bytes_indesign() {
        let mut data = vec![0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1];
        data.extend_from_slice(&[0; 8]); // Pad to 16 bytes
        assert!(VfxParser::validate_magic_bytes(&data, VfxFormat::InDesign));
    }

    #[test]
    fn test_magic_bytes_illustrator_pdf() {
        let mut data = b"%PDF-1.4\n".to_vec();
        data.extend_from_slice(&[0; 7]); // Pad to 16 bytes
        assert!(VfxParser::validate_magic_bytes(&data, VfxFormat::Illustrator));
    }

    #[test]
    fn test_magic_bytes_illustrator_ps() {
        let mut data = b"%!PS-Adobe-3.0\n".to_vec();
        data.push(0); // Pad to 16 bytes
        assert!(VfxParser::validate_magic_bytes(&data, VfxFormat::Illustrator));
    }

    #[test]
    fn test_magic_bytes_after_effects() {
        let mut data = b"RIFX".to_vec();
        data.extend_from_slice(&[0; 12]); // Pad to 16 bytes
        assert!(VfxParser::validate_magic_bytes(&data, VfxFormat::AfterEffects));
    }

    #[test]
    fn test_magic_bytes_premiere() {
        let mut data = b"<?xml version=\"1.0\"?>".to_vec();
        assert!(VfxParser::validate_magic_bytes(&data, VfxFormat::Premiere));
    }

    #[tokio::test]
    async fn test_premiere_xml_parsing() {
        let xml_data = br#"<?xml version="1.0"?>
<PremiereData Version="3">
    <Sequence>Main Timeline</Sequence>
    <Sequence>Intro</Sequence>
    <Media FilePath="video.mp4"/>
    <Media FilePath="audio.wav"/>
</PremiereData>"#;

        let parser = VfxParser::new();
        let result = parser.parse(xml_data, "project.prproj").await;

        assert!(result.is_ok());
        let info = result.unwrap();
        assert_eq!(info.format, VfxFormat::Premiere);
        assert_eq!(info.layer_count, Some(2)); // 2 sequences
        assert_eq!(info.linked_assets.len(), 2); // 2 media files
    }
}
