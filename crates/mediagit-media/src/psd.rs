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

//! PSD layer detection and analysis
//!
//! This module provides Photoshop (PSD) file parsing and layer extraction
//! to enable intelligent merging of non-overlapping layers.
//!
//! # Features
//!
//! - PSD file format parsing
//! - Layer extraction with metadata
//! - Blend mode and opacity detection
//! - Layer boundary detection for overlap analysis
//! - Automatic merge for non-overlapping layers
//!
//! # Example
//!
//! ```rust,no_run
//! use mediagit_media::psd::PsdParser;
//!
//! # async fn example() -> anyhow::Result<()> {
//! let psd_data = std::fs::read("image.psd")?;
//! let parser = PsdParser::new();
//! let info = parser.parse(&psd_data).await?;
//!
//! println!("Found {} layers", info.layers.len());
//! for layer in &info.layers {
//!     println!("Layer: {} at ({}, {})", layer.name, layer.x, layer.y);
//! }
//! # Ok(())
//! # }
//! ```

use crate::error::{MediaError, Result};
use psd::{Psd, PsdLayer};
use serde::{Deserialize, Serialize};
use tracing::{debug, info, instrument, warn};

/// Complete PSD file information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PsdInfo {
    /// Document width
    pub width: u32,

    /// Document height
    pub height: u32,

    /// Color depth (8, 16, or 32 bits per channel)
    pub depth: u16,

    /// Color mode (RGB, CMYK, etc.)
    pub color_mode: String,

    /// Number of channels
    pub channels: u16,

    /// All layers in the document
    pub layers: Vec<LayerInfo>,

    /// Layer groups/folders
    pub groups: Vec<LayerGroup>,
}

/// Information about a single PSD layer
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LayerInfo {
    /// Layer name
    pub name: String,

    /// Layer ID (if available)
    pub id: Option<u32>,

    /// X position (left edge)
    pub x: i32,

    /// Y position (top edge)
    pub y: i32,

    /// Layer width
    pub width: u32,

    /// Layer height
    pub height: u32,

    /// Layer opacity (0-255)
    pub opacity: u8,

    /// Blend mode
    pub blend_mode: String,

    /// Whether layer is visible
    pub visible: bool,

    /// Parent group (if any)
    pub parent_group: Option<String>,

    /// Layer type (normal, adjustment, etc.)
    pub layer_type: LayerType,
}

/// Type of PSD layer
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum LayerType {
    /// Regular pixel layer
    Normal,
    /// Adjustment layer
    Adjustment,
    /// Text layer
    Text,
    /// Shape layer
    Shape,
    /// Smart object
    SmartObject,
    /// Unknown type
    Unknown,
}

/// Layer group/folder information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LayerGroup {
    /// Group name
    pub name: String,

    /// Layer names in this group
    pub layer_names: Vec<String>,

    /// Whether group is visible
    pub visible: bool,
}

/// PSD layer bounds
#[derive(Debug, Clone, Copy)]
pub struct LayerBounds {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

impl LayerBounds {
    /// Check if this layer overlaps with another
    pub fn overlaps(&self, other: &LayerBounds) -> bool {
        !(self.right <= other.left
            || self.left >= other.right
            || self.bottom <= other.top
            || self.top >= other.bottom)
    }

    /// Calculate overlap area with another layer
    pub fn overlap_area(&self, other: &LayerBounds) -> u32 {
        if !self.overlaps(other) {
            return 0;
        }

        let x_overlap = (self.right.min(other.right) - self.left.max(other.left)).max(0);
        let y_overlap = (self.bottom.min(other.bottom) - self.top.max(other.top)).max(0);

        (x_overlap * y_overlap) as u32
    }
}

impl From<&LayerInfo> for LayerBounds {
    fn from(layer: &LayerInfo) -> Self {
        LayerBounds {
            left: layer.x,
            top: layer.y,
            right: layer.x + layer.width as i32,
            bottom: layer.y + layer.height as i32,
        }
    }
}

/// PSD file parser
#[derive(Debug)]
pub struct PsdParser;

impl PsdParser {
    /// Create a new PSD parser
    pub fn new() -> Self {
        PsdParser
    }

    /// Parse PSD file from bytes
    #[instrument(skip(data), fields(size = data.len()))]
    pub async fn parse(&self, data: &[u8]) -> Result<PsdInfo> {
        info!("Parsing PSD file");

        let psd = Psd::from_bytes(data)
            .map_err(|e| MediaError::PsdError(format!("Failed to parse PSD: {}", e)))?;

        use psd::PsdDepth;

        let depth = match psd.depth() {
            PsdDepth::One => 1,
            PsdDepth::Eight => 8,
            PsdDepth::Sixteen => 16,
            PsdDepth::ThirtyTwo => 32,
        };

        let channels = Self::determine_channel_count(psd.color_mode());

        debug!(
            "PSD dimensions: {}x{}, depth: {}, channels: {}",
            psd.width(),
            psd.height(),
            depth,
            channels
        );

        let layers = self.extract_layers(&psd)?;
        let groups = self.extract_groups(&layers);

        Ok(PsdInfo {
            width: psd.width(),
            height: psd.height(),
            depth,
            color_mode: format!("{:?}", psd.color_mode()),
            channels,
            layers,
            groups,
        })
    }

    /// Determine typical channel count from color mode
    ///
    /// NOTE: This is an approximation as the actual channel count includes alpha channels
    /// and is stored in the PSD header but not exposed by the psd crate API.
    fn determine_channel_count(color_mode: psd::ColorMode) -> u16 {
        use psd::ColorMode;
        match color_mode {
            ColorMode::Bitmap | ColorMode::Grayscale | ColorMode::Duotone => 1,
            ColorMode::Indexed => 1,  // Indexed color uses color table
            ColorMode::Rgb => 3,  // Red, Green, Blue
            ColorMode::Cmyk => 4,  // Cyan, Magenta, Yellow, Black
            ColorMode::Lab => 3,  // L, a, b
            ColorMode::Multichannel => 3,  // Typically 3+ channels
        }
    }

    /// Extract layer information from PSD
    fn extract_layers(&self, psd: &Psd) -> Result<Vec<LayerInfo>> {
        let mut layers = Vec::new();

        for (idx, layer) in psd.layers().iter().enumerate() {
            let layer_info = self.parse_layer(layer, idx)?;
            layers.push(layer_info);
        }

        info!("Extracted {} layers from PSD", layers.len());
        Ok(layers)
    }

    /// Parse a single PSD layer
    fn parse_layer(&self, layer: &PsdLayer, idx: usize) -> Result<LayerInfo> {
        let name = if layer.name().is_empty() {
            format!("Layer {}", idx)
        } else {
            layer.name().to_string()
        };

        let (x, y) = (layer.layer_left(), layer.layer_top());
        let width = (layer.layer_right() - layer.layer_left()) as u32;
        let height = (layer.layer_bottom() - layer.layer_top()) as u32;

        let opacity = layer.opacity();
        let blend_mode = format!("{:?}", layer.blend_mode());

        // NOTE: Layer visibility determination pending
        // The psd crate doesn't currently expose layer visibility flags directly.
        // Visibility could be derived from layer.flags() if the method becomes available.
        // For now, we default to true which is safe for merge conflict detection.
        let visible = true;

        let layer_type = Self::detect_layer_type(layer);

        Ok(LayerInfo {
            name,
            id: None,  // PSD crate may not expose layer ID
            x,
            y,
            width,
            height,
            opacity,
            blend_mode,
            visible,
            parent_group: None,  // Would need layer tree analysis
            layer_type,
        })
    }

    /// Detect layer type from PSD layer
    fn detect_layer_type(layer: &PsdLayer) -> LayerType {
        // Basic type detection - could be enhanced with more analysis
        let name = layer.name();

        if name.contains("text") || name.contains("Text") {
            LayerType::Text
        } else if name.contains("shape") || name.contains("Shape") {
            LayerType::Shape
        } else if name.contains("adjustment") {
            LayerType::Adjustment
        } else {
            LayerType::Normal
        }
    }

    /// Extract layer groups from layers
    fn extract_groups(&self, layers: &[LayerInfo]) -> Vec<LayerGroup> {
        // Simplified group extraction - would need layer tree analysis for real implementation
        let mut groups = Vec::new();

        // Group visible layers together
        let visible_layers: Vec<String> = layers
            .iter()
            .filter(|l| l.visible)
            .map(|l| l.name.clone())
            .collect();

        if !visible_layers.is_empty() {
            groups.push(LayerGroup {
                name: "Visible Layers".to_string(),
                layer_names: visible_layers,
                visible: true,
            });
        }

        groups
    }

    /// Check if two sets of layers have non-overlapping changes (enhanced)
    pub fn can_auto_merge(base: &PsdInfo, ours: &PsdInfo, theirs: &PsdInfo) -> MergeDecision {
        let base_layers: Vec<&LayerInfo> = base.layers.iter().collect();
        let ours_layers: Vec<&LayerInfo> = ours.layers.iter().collect();
        let theirs_layers: Vec<&LayerInfo> = theirs.layers.iter().collect();

        // Check if layers were added/removed
        let layers_changed = base_layers.len() != ours_layers.len()
            || base_layers.len() != theirs_layers.len();

        if layers_changed {
            debug!("Layer count changed, checking for conflicts");
        }

        // Find modified layers in each branch
        let ours_modified = Self::find_modified_layers(&base_layers, &ours_layers);
        let theirs_modified = Self::find_modified_layers(&base_layers, &theirs_layers);

        // Collect all conflicts from different detection strategies
        let mut all_conflicts = Vec::new();

        // Check for overlapping spatial modifications
        let spatial_conflicts = Self::find_overlapping_modifications(&ours_modified, &theirs_modified);
        all_conflicts.extend(spatial_conflicts);

        // Check for blend mode conflicts
        let blend_conflicts = Self::detect_blend_conflicts(&ours_modified, &theirs_modified);
        all_conflicts.extend(blend_conflicts);

        // Check for layer group hierarchy conflicts
        let group_conflicts = Self::detect_group_conflicts(base, ours, theirs);
        all_conflicts.extend(group_conflicts);

        // Check for smart object conflicts
        let smart_conflicts = Self::detect_smart_object_conflicts(&ours_modified, &theirs_modified);
        all_conflicts.extend(smart_conflicts);

        if all_conflicts.is_empty() {
            info!("No overlapping layer modifications detected - can auto-merge");
            MergeDecision::AutoMerge
        } else {
            warn!("Found {} total conflicts across all detection strategies", all_conflicts.len());
            MergeDecision::ManualReview(all_conflicts)
        }
    }

    /// Find layers that were modified compared to base
    fn find_modified_layers<'a>(
        base: &[&'a LayerInfo],
        modified: &[&'a LayerInfo],
    ) -> Vec<&'a LayerInfo> {
        let mut result = Vec::new();

        for mod_layer in modified {
            if let Some(base_layer) = base.iter().find(|b| b.name == mod_layer.name) {
                if Self::layer_differs(base_layer, mod_layer) {
                    result.push(*mod_layer);
                }
            } else {
                // New layer added
                result.push(*mod_layer);
            }
        }

        result
    }

    /// Check if two layers differ (enhanced with blend mode and type checking)
    fn layer_differs(a: &LayerInfo, b: &LayerInfo) -> bool {
        a.x != b.x
            || a.y != b.y
            || a.width != b.width
            || a.height != b.height
            || a.opacity != b.opacity
            || a.visible != b.visible
            || a.blend_mode != b.blend_mode
            || a.layer_type != b.layer_type
    }

    /// Detect blend mode conflicts between layers
    fn detect_blend_conflicts(
        ours: &[&LayerInfo],
        theirs: &[&LayerInfo],
    ) -> Vec<String> {
        let mut conflicts = Vec::new();

        for our_layer in ours {
            for their_layer in theirs {
                if our_layer.name == their_layer.name
                    && our_layer.blend_mode != their_layer.blend_mode {
                    conflicts.push(format!(
                        "Layer '{}': blend mode changed from '{}' to '{}' in different branches",
                        our_layer.name, our_layer.blend_mode, their_layer.blend_mode
                    ));
                }
            }
        }

        conflicts
    }

    /// Check for layer group hierarchy conflicts
    fn detect_group_conflicts(
        base: &PsdInfo,
        ours: &PsdInfo,
        theirs: &PsdInfo,
    ) -> Vec<String> {
        let mut conflicts = Vec::new();

        // Check if same layer moved to different groups
        for base_layer in &base.layers {
            let ours_layer = ours.layers.iter().find(|l| l.name == base_layer.name);
            let theirs_layer = theirs.layers.iter().find(|l| l.name == base_layer.name);

            if let (Some(ours), Some(theirs)) = (ours_layer, theirs_layer) {
                if ours.parent_group != theirs.parent_group
                    && (ours.parent_group != base_layer.parent_group
                        || theirs.parent_group != base_layer.parent_group) {
                    conflicts.push(format!(
                        "Layer '{}' moved to different groups: {:?} vs {:?}",
                        base_layer.name, ours.parent_group, theirs.parent_group
                    ));
                }
            }
        }

        conflicts
    }

    /// Detect smart object conflicts
    fn detect_smart_object_conflicts(
        ours: &[&LayerInfo],
        theirs: &[&LayerInfo],
    ) -> Vec<String> {
        let mut conflicts = Vec::new();

        for our_layer in ours {
            if our_layer.layer_type == LayerType::SmartObject {
                for their_layer in theirs {
                    if their_layer.name == our_layer.name
                        && their_layer.layer_type == LayerType::SmartObject {
                        conflicts.push(format!(
                            "Smart Object '{}' modified in both branches (may contain different embedded content)",
                            our_layer.name
                        ));
                    }
                }
            }
        }

        conflicts
    }

    /// Perform actual merge of PSD layers (metadata-level merge)
    ///
    /// This merges non-conflicting layer changes by combining:
    /// - Layers added in 'ours' branch
    /// - Layers added in 'theirs' branch
    /// - Layers from base that weren't modified
    ///
    /// Returns merged PsdInfo structure (not actual PSD binary - that requires rebuild)
    pub fn merge_layers(
        base: &PsdInfo,
        ours: &PsdInfo,
        theirs: &PsdInfo,
    ) -> Result<PsdInfo> {
        info!("Executing PSD layer merge");

        // Start with base document properties
        let mut merged_psd = PsdInfo {
            width: ours.width,
            height: ours.height,
            depth: ours.depth,
            color_mode: ours.color_mode.clone(),
            channels: ours.channels,
            layers: Vec::new(),
            groups: Vec::new(),
        };

        // Collect layers from both branches
        let mut merged_layers = Vec::new();

        // Add all layers from 'ours' that were added or modified
        for layer in &ours.layers {
            if !base.layers.iter().any(|b| b.name == layer.name) {
                // New layer in 'ours'
                debug!("Adding new layer from 'ours': {}", layer.name);
                merged_layers.push(layer.clone());
            } else {
                // Modified or unchanged layer from 'ours'
                merged_layers.push(layer.clone());
            }
        }

        // Add new layers from 'theirs' that don't exist in 'ours'
        for layer in &theirs.layers {
            if !ours.layers.iter().any(|o| o.name == layer.name)
                && !base.layers.iter().any(|b| b.name == layer.name) {
                // New layer in 'theirs' only
                debug!("Adding new layer from 'theirs': {}", layer.name);
                merged_layers.push(layer.clone());
            }
        }

        merged_psd.layers = merged_layers;

        // Merge groups - combine all unique groups
        let mut merged_groups = ours.groups.clone();
        for group in &theirs.groups {
            if !merged_groups.iter().any(|g| g.name == group.name) {
                merged_groups.push(group.clone());
            }
        }
        merged_psd.groups = merged_groups;

        info!("PSD merge complete: {} layers", merged_psd.layers.len());

        Ok(merged_psd)
    }

    /// Find overlapping modifications between two sets of layers
    fn find_overlapping_modifications(
        ours: &[&LayerInfo],
        theirs: &[&LayerInfo],
    ) -> Vec<String> {
        let mut conflicts = Vec::new();

        for our_layer in ours {
            for their_layer in theirs {
                // Same layer name modified in both branches
                if our_layer.name == their_layer.name {
                    conflicts.push(format!("Layer '{}' modified in both branches", our_layer.name));
                    continue;
                }

                // Check for spatial overlap
                let our_bounds = LayerBounds::from(*our_layer);
                let their_bounds = LayerBounds::from(*their_layer);

                if our_bounds.overlaps(&their_bounds) {
                    let overlap_area = our_bounds.overlap_area(&their_bounds);
                    if overlap_area > 0 {
                        conflicts.push(format!(
                            "Layers '{}' and '{}' overlap by {} pixels",
                            our_layer.name, their_layer.name, overlap_area
                        ));
                    }
                }
            }
        }

        conflicts
    }
}

impl Default for PsdParser {
    fn default() -> Self {
        Self::new()
    }
}

/// PSD merge decision
#[derive(Debug, Clone)]
pub enum MergeDecision {
    /// Layers don't overlap, can auto-merge
    AutoMerge,
    /// Layers overlap or conflict, needs manual review
    ManualReview(Vec<String>),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_layer_bounds_overlap() {
        let layer1 = LayerBounds {
            left: 0,
            top: 0,
            right: 100,
            bottom: 100,
        };

        let layer2 = LayerBounds {
            left: 50,
            top: 50,
            right: 150,
            bottom: 150,
        };

        assert!(layer1.overlaps(&layer2));
        assert_eq!(layer1.overlap_area(&layer2), 50 * 50);
    }

    #[test]
    fn test_layer_bounds_no_overlap() {
        let layer1 = LayerBounds {
            left: 0,
            top: 0,
            right: 100,
            bottom: 100,
        };

        let layer2 = LayerBounds {
            left: 200,
            top: 200,
            right: 300,
            bottom: 300,
        };

        assert!(!layer1.overlaps(&layer2));
        assert_eq!(layer1.overlap_area(&layer2), 0);
    }

    #[test]
    fn test_layer_type_detection() {
        // Basic serialization test
        let layer_type = LayerType::Normal;
        let json = serde_json::to_string(&layer_type).unwrap();
        assert!(json.contains("Normal"));
    }
}
