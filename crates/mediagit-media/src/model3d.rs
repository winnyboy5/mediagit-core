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

//! 3D model parsing and analysis
//!
//! This module provides 3D model file parsing and metadata extraction
//! to enable intelligent merging of non-conflicting mesh modifications.
//!
//! # Supported Formats
//!
//! - **OBJ**: Wavefront OBJ (text-based, widely supported)
//! - **FBX**: Autodesk FBX (binary format, industry standard)
//! - **Blend**: Blender native format (binary format)
//! - **GLTF/GLB**: GL Transmission Format (JSON-based)
//!
//! # Features
//!
//! - Mesh metadata extraction (vertex count, face count, material count)
//! - Bounding box calculation for spatial conflict detection
//! - Material and texture tracking
//! - Conflict detection for overlapping mesh modifications
//!
//! # Example
//!
//! ```rust,no_run
//! use mediagit_media::model3d::Model3DParser;
//!
//! # async fn example() -> anyhow::Result<()> {
//! let model_data = std::fs::read("model.obj")?;
//! let parser = Model3DParser::new();
//! let info = parser.parse(&model_data, "model.obj").await?;
//!
//! println!("Vertices: {}, Faces: {}", info.vertex_count, info.face_count);
//! println!("Materials: {}", info.materials.len());
//! # Ok(())
//! # }
//! ```

use crate::error::{MediaError, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use tracing::{debug, info, instrument, warn};

/// Complete 3D model file information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Model3DInfo {
    /// Model format
    pub format: Model3DFormat,

    /// Number of vertices
    pub vertex_count: u64,

    /// Number of faces/polygons
    pub face_count: u64,

    /// Number of objects/meshes
    pub object_count: u32,

    /// Materials used
    pub materials: Vec<MaterialInfo>,

    /// Textures referenced
    pub textures: Vec<String>,

    /// Bounding box (min and max coordinates)
    pub bounding_box: Option<BoundingBox>,

    /// File size in bytes
    pub file_size: u64,

    /// Animation data present
    pub has_animations: bool,

    /// Rigging/skeleton data present
    pub has_rigging: bool,
}

/// 3D model format enumeration
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Model3DFormat {
    Obj,
    Fbx,
    Blend,
    Gltf,
    Glb,
    Stl,
    Usd,
    Ply,
    Unknown,
}

impl Model3DFormat {
    /// Detect format from file extension
    pub fn from_extension(ext: &str) -> Self {
        match ext.to_lowercase().as_str() {
            "obj" => Model3DFormat::Obj,
            "fbx" => Model3DFormat::Fbx,
            "blend" => Model3DFormat::Blend,
            "gltf" => Model3DFormat::Gltf,
            "glb" => Model3DFormat::Glb,
            "stl" => Model3DFormat::Stl,
            "usd" | "usda" | "usdc" | "usdz" => Model3DFormat::Usd,
            "ply" => Model3DFormat::Ply,
            _ => Model3DFormat::Unknown,
        }
    }
}

/// Material information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MaterialInfo {
    /// Material name
    pub name: String,

    /// Diffuse color (R, G, B)
    pub diffuse_color: Option<(f32, f32, f32)>,

    /// Specular color (R, G, B)
    pub specular_color: Option<(f32, f32, f32)>,

    /// Texture maps referenced
    pub texture_maps: Vec<String>,

    /// Transparency/opacity
    pub transparency: Option<f32>,
}

/// 3D bounding box
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct BoundingBox {
    /// Minimum coordinates (x, y, z)
    pub min: (f32, f32, f32),

    /// Maximum coordinates (x, y, z)
    pub max: (f32, f32, f32),
}

impl BoundingBox {
    /// Create new bounding box
    pub fn new() -> Self {
        BoundingBox {
            min: (f32::MAX, f32::MAX, f32::MAX),
            max: (f32::MIN, f32::MIN, f32::MIN),
        }
    }

    /// Expand bounding box to include a point
    pub fn expand(&mut self, x: f32, y: f32, z: f32) {
        self.min.0 = self.min.0.min(x);
        self.min.1 = self.min.1.min(y);
        self.min.2 = self.min.2.min(z);
        self.max.0 = self.max.0.max(x);
        self.max.1 = self.max.1.max(y);
        self.max.2 = self.max.2.max(z);
    }

    /// Calculate volume of bounding box
    pub fn volume(&self) -> f32 {
        let dx = self.max.0 - self.min.0;
        let dy = self.max.1 - self.min.1;
        let dz = self.max.2 - self.min.2;
        dx * dy * dz
    }

    /// Check if this bounding box overlaps with another
    pub fn overlaps(&self, other: &BoundingBox) -> bool {
        !(self.max.0 < other.min.0
            || self.min.0 > other.max.0
            || self.max.1 < other.min.1
            || self.min.1 > other.max.1
            || self.max.2 < other.min.2
            || self.min.2 > other.max.2)
    }
}

impl Default for BoundingBox {
    fn default() -> Self {
        Self::new()
    }
}

/// 3D model parser
#[derive(Debug)]
pub struct Model3DParser;

impl Model3DParser {
    /// Create a new 3D model parser
    pub fn new() -> Self {
        Model3DParser
    }

    /// Parse 3D model file from bytes
    #[instrument(skip(data), fields(size = data.len(), filename = %filename))]
    pub async fn parse(&self, data: &[u8], filename: &str) -> Result<Model3DInfo> {
        info!("Parsing 3D model file: {}", filename);

        let format = Self::detect_format(filename);
        debug!("Detected 3D format: {:?}", format);

        match format {
            Model3DFormat::Obj => self.parse_obj(data).await,
            Model3DFormat::Fbx => self.parse_fbx(data).await,
            Model3DFormat::Blend => self.parse_blend(data).await,
            Model3DFormat::Gltf | Model3DFormat::Glb => self.parse_gltf(data).await,
            Model3DFormat::Stl => self.parse_stl(data).await,
            Model3DFormat::Usd => self.parse_usd(data, filename).await,
            Model3DFormat::Ply => self.parse_ply(data).await,
            Model3DFormat::Unknown => Err(MediaError::UnsupportedFormat(
                "Unknown 3D model format".to_string(),
            )),
        }
    }

    /// Detect 3D model format from filename
    fn detect_format(filename: &str) -> Model3DFormat {
        filename
            .split('.')
            .next_back()
            .map(Model3DFormat::from_extension)
            .unwrap_or(Model3DFormat::Unknown)
    }

    /// Parse Wavefront OBJ file
    #[instrument(skip(data))]
    async fn parse_obj(&self, data: &[u8]) -> Result<Model3DInfo> {
        debug!("Parsing OBJ file");

        let content = std::str::from_utf8(data)
            .map_err(|e| MediaError::InvalidStructure(format!("Invalid UTF-8 in OBJ: {}", e)))?;

        let mut vertex_count = 0u64;
        let mut face_count = 0u64;
        let mut materials = HashSet::new();
        let mut textures = HashSet::new();
        let mut object_count = 0u32;
        let mut bounding_box = BoundingBox::new();

        for line in content.lines() {
            let trimmed = line.trim();

            if trimmed.starts_with("v ") {
                // Vertex
                vertex_count += 1;

                // Parse vertex coordinates for bounding box
                let parts: Vec<&str> = trimmed.split_whitespace().collect();
                if parts.len() >= 4 {
                    if let (Ok(x), Ok(y), Ok(z)) = (
                        parts[1].parse::<f32>(),
                        parts[2].parse::<f32>(),
                        parts[3].parse::<f32>(),
                    ) {
                        bounding_box.expand(x, y, z);
                    }
                }
            } else if trimmed.starts_with("f ") {
                // Face
                face_count += 1;
            } else if trimmed.starts_with("usemtl ") {
                // Material usage
                if let Some(mat_name) = trimmed.split_whitespace().nth(1) {
                    materials.insert(mat_name.to_string());
                }
            } else if trimmed.starts_with("mtllib ") {
                // Material library reference
                if let Some(mtl_file) = trimmed.split_whitespace().nth(1) {
                    textures.insert(mtl_file.to_string());
                }
            } else if trimmed.starts_with("o ") || trimmed.starts_with("g ") {
                // Object or group
                object_count += 1;
            }
        }

        let material_infos: Vec<MaterialInfo> = materials
            .into_iter()
            .map(|name| MaterialInfo {
                name,
                diffuse_color: None,
                specular_color: None,
                texture_maps: Vec::new(),
                transparency: None,
            })
            .collect();

        debug!(
            "Parsed OBJ: vertices={}, faces={}, materials={}, objects={}",
            vertex_count,
            face_count,
            material_infos.len(),
            object_count
        );

        Ok(Model3DInfo {
            format: Model3DFormat::Obj,
            vertex_count,
            face_count,
            object_count: object_count.max(1), // At least 1 object
            materials: material_infos,
            textures: textures.into_iter().collect(),
            bounding_box: Some(bounding_box),
            file_size: data.len() as u64,
            has_animations: false, // OBJ doesn't support animations
            has_rigging: false,     // OBJ doesn't support rigging
        })
    }

    /// Parse Autodesk FBX file (binary format)
    #[instrument(skip(data))]
    async fn parse_fbx(&self, data: &[u8]) -> Result<Model3DInfo> {
        warn!("FBX parsing using metadata extraction approach");

        // FBX is a complex binary format - for MVP we'll extract basic metadata
        // Check FBX magic bytes: "Kaydara FBX Binary"
        if data.len() < 23 {
            return Err(MediaError::InvalidStructure("File too small for FBX".to_string()));
        }

        let magic = &data[0..18];
        if magic != b"Kaydara FBX Binary" {
            // Try ASCII FBX
            if data.len() < 20 || !data[0..20].starts_with(b"; FBX") {
                return Err(MediaError::InvalidStructure("Not a valid FBX file".to_string()));
            }
            return self.parse_fbx_ascii(data).await;
        }

        // For binary FBX, we'll provide basic metadata estimation
        // Full parsing would require complex binary structure analysis
        debug!("Binary FBX detected - using estimation approach");

        Ok(Model3DInfo {
            format: Model3DFormat::Fbx,
            vertex_count: 0, // Would require full parsing
            face_count: 0,    // Would require full parsing
            object_count: 1,
            materials: Vec::new(),
            textures: Vec::new(),
            bounding_box: None,
            file_size: data.len() as u64,
            has_animations: true, // FBX typically has animations
            has_rigging: true,     // FBX typically has rigging
        })
    }

    /// Parse ASCII FBX file
    #[instrument(skip(data))]
    async fn parse_fbx_ascii(&self, data: &[u8]) -> Result<Model3DInfo> {
        let content = std::str::from_utf8(data)
            .map_err(|e| MediaError::InvalidStructure(format!("Invalid UTF-8 in FBX: {}", e)))?;

        let mut vertex_count = 0u64;
        let mut object_count = 0u32;
        let mut materials = HashSet::new();

        for line in content.lines() {
            let trimmed = line.trim();

            if trimmed.contains("Vertices:") {
                // Count vertices in the next lines
                vertex_count += 1;
            } else if trimmed.starts_with("Model:") {
                object_count += 1;
            } else if trimmed.starts_with("Material:") {
                if let Some(mat_name) = trimmed.split('"').nth(1) {
                    materials.insert(mat_name.to_string());
                }
            }
        }

        let material_infos: Vec<MaterialInfo> = materials
            .into_iter()
            .map(|name| MaterialInfo {
                name,
                diffuse_color: None,
                specular_color: None,
                texture_maps: Vec::new(),
                transparency: None,
            })
            .collect();

        debug!("Parsed ASCII FBX: vertices~{}, objects={}", vertex_count, object_count);

        Ok(Model3DInfo {
            format: Model3DFormat::Fbx,
            vertex_count,
            face_count: 0, // Would need deeper parsing
            object_count: object_count.max(1),
            materials: material_infos,
            textures: Vec::new(),
            bounding_box: None,
            file_size: data.len() as u64,
            has_animations: true,
            has_rigging: true,
        })
    }

    /// Parse Blender file (binary format)
    #[instrument(skip(data))]
    async fn parse_blend(&self, data: &[u8]) -> Result<Model3DInfo> {
        warn!("Blender file parsing using metadata extraction approach");

        // .blend is a complex binary format - for MVP we'll provide basic info
        // Check .blend magic bytes: "BLENDER"
        if data.len() < 12 {
            return Err(MediaError::InvalidStructure(
                "File too small for Blender".to_string(),
            ));
        }

        if &data[0..7] != b"BLENDER" {
            return Err(MediaError::InvalidStructure("Not a valid Blender file".to_string()));
        }

        // Extract version from header (e.g., "v280" for Blender 2.80)
        let version = String::from_utf8_lossy(&data[7..10]).to_string();
        debug!("Detected Blender version: {}", version);

        Ok(Model3DInfo {
            format: Model3DFormat::Blend,
            vertex_count: 0, // Would require full parsing
            face_count: 0,    // Would require full parsing
            object_count: 1,
            materials: Vec::new(),
            textures: Vec::new(),
            bounding_box: None,
            file_size: data.len() as u64,
            has_animations: true, // Blender files typically have animations
            has_rigging: true,     // Blender files typically have rigging
        })
    }

    /// Parse GLTF/GLB file
    #[instrument(skip(data))]
    async fn parse_gltf(&self, data: &[u8]) -> Result<Model3DInfo> {
        warn!("GLTF/GLB parsing using metadata extraction approach");

        // Check if it's GLB (binary) or GLTF (JSON)
        let is_glb = data.len() >= 4 && &data[0..4] == b"glTF";

        if is_glb {
            // GLB binary format
            debug!("Binary GLB detected");

            Ok(Model3DInfo {
                format: Model3DFormat::Glb,
                vertex_count: 0,
                face_count: 0,
                object_count: 1,
                materials: Vec::new(),
                textures: Vec::new(),
                bounding_box: None,
                file_size: data.len() as u64,
                has_animations: true,
                has_rigging: true,
            })
        } else {
            // GLTF JSON format
            let content = std::str::from_utf8(data).map_err(|e| {
                MediaError::InvalidStructure(format!("Invalid UTF-8 in GLTF: {}", e))
            })?;

            // Basic JSON parsing to extract mesh count
            let mesh_count = content.matches("\"meshes\"").count() as u32;
            let material_count = content.matches("\"materials\"").count() as u32;

            debug!("Parsed GLTF: meshes={}, materials={}", mesh_count, material_count);

            Ok(Model3DInfo {
                format: Model3DFormat::Gltf,
                vertex_count: 0, // Would need JSON parsing
                face_count: 0,
                object_count: mesh_count.max(1),
                materials: Vec::new(),
                textures: Vec::new(),
                bounding_box: None,
                file_size: data.len() as u64,
                has_animations: true,
                has_rigging: true,
            })
        }
    }

    /// Parse STL file (ASCII or binary)
    #[instrument(skip(data))]
    async fn parse_stl(&self, data: &[u8]) -> Result<Model3DInfo> {
        debug!("Parsing STL file");

        // Check if ASCII STL (starts with "solid")
        let is_ascii = data.len() >= 5 && &data[0..5] == b"solid";

        if is_ascii {
            let content = String::from_utf8_lossy(data);
            let face_count = content.matches("facet normal").count() as u64;
            let vertex_count = face_count * 3; // Each facet has 3 vertices

            Ok(Model3DInfo {
                format: Model3DFormat::Stl,
                vertex_count,
                face_count,
                object_count: 1,
                materials: Vec::new(),
                textures: Vec::new(),
                bounding_box: None,
                file_size: data.len() as u64,
                has_animations: false,
                has_rigging: false,
            })
        } else if data.len() >= 84 {
            // Binary STL: 80-byte header + 4-byte triangle count
            let face_count = u32::from_le_bytes([data[80], data[81], data[82], data[83]]) as u64;
            let vertex_count = face_count * 3;

            Ok(Model3DInfo {
                format: Model3DFormat::Stl,
                vertex_count,
                face_count,
                object_count: 1,
                materials: Vec::new(),
                textures: Vec::new(),
                bounding_box: None,
                file_size: data.len() as u64,
                has_animations: false,
                has_rigging: false,
            })
        } else {
            Err(MediaError::InvalidStructure("File too small for STL".to_string()))
        }
    }

    /// Parse USD/USDA/USDC/USDZ file
    #[instrument(skip(data))]
    async fn parse_usd(&self, data: &[u8], filename: &str) -> Result<Model3DInfo> {
        debug!("Parsing USD file: {}", filename);

        // USDZ is a zip archive; USDC is binary crate format; USDA is text
        let ext = filename.split('.').next_back().unwrap_or("");
        let is_usdz = ext.eq_ignore_ascii_case("usdz");
        let is_usda = ext.eq_ignore_ascii_case("usda");

        // Basic metadata extraction
        let object_count = if is_usda {
            let content = String::from_utf8_lossy(data);
            content.matches("def Xform").count() as u32
                + content.matches("def Mesh").count() as u32
        } else {
            1
        };

        Ok(Model3DInfo {
            format: Model3DFormat::Usd,
            vertex_count: 0,
            face_count: 0,
            object_count: object_count.max(1),
            materials: Vec::new(),
            textures: Vec::new(),
            bounding_box: None,
            file_size: data.len() as u64,
            has_animations: !is_usdz, // USDZ is typically static
            has_rigging: false,
        })
    }

    /// Parse PLY file (ASCII or binary)
    #[instrument(skip(data))]
    async fn parse_ply(&self, data: &[u8]) -> Result<Model3DInfo> {
        debug!("Parsing PLY file");

        let header_end = data.windows(11)
            .position(|w| w == b"end_header\n")
            .or_else(|| data.windows(12).position(|w| w == b"end_header\r\n"));

        if header_end.is_none() {
            return Err(MediaError::InvalidStructure("No PLY header found".to_string()));
        }

        let header = String::from_utf8_lossy(&data[..header_end.unwrap()]);
        let mut vertex_count = 0u64;
        let mut face_count = 0u64;

        for line in header.lines() {
            if line.starts_with("element vertex ") {
                vertex_count = line.split_whitespace().nth(2)
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(0);
            } else if line.starts_with("element face ") {
                face_count = line.split_whitespace().nth(2)
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(0);
            }
        }

        Ok(Model3DInfo {
            format: Model3DFormat::Ply,
            vertex_count,
            face_count,
            object_count: 1,
            materials: Vec::new(),
            textures: Vec::new(),
            bounding_box: None,
            file_size: data.len() as u64,
            has_animations: false,
            has_rigging: false,
        })
    }

    /// Check if two 3D models can be auto-merged
    pub fn can_auto_merge(
        base: &Model3DInfo,
        ours: &Model3DInfo,
        theirs: &Model3DInfo,
    ) -> MergeDecision {
        // Basic conflict detection based on metadata changes

        // Check if formats match
        if ours.format != base.format || theirs.format != base.format {
            warn!("Model format changed between versions");
            return MergeDecision::ManualReview(vec![
                "3D model format changed - manual review required".to_string()
            ]);
        }

        let mut conflicts = Vec::new();

        // Check for significant vertex count changes
        let ours_vertex_delta = (ours.vertex_count as i64 - base.vertex_count as i64).abs();
        let theirs_vertex_delta = (theirs.vertex_count as i64 - base.vertex_count as i64).abs();

        if ours_vertex_delta > 0 && theirs_vertex_delta > 0 {
            conflicts.push(format!(
                "Both branches modified mesh geometry (ours: {:+} vertices, theirs: {:+} vertices)",
                ours.vertex_count as i64 - base.vertex_count as i64,
                theirs.vertex_count as i64 - base.vertex_count as i64
            ));
        }

        // Check for material conflicts
        let ours_material_delta = ours.materials.len() as i32 - base.materials.len() as i32;
        let theirs_material_delta = theirs.materials.len() as i32 - base.materials.len() as i32;

        if ours_material_delta != 0 && theirs_material_delta != 0 {
            conflicts.push(format!(
                "Both branches modified materials (ours: {:+} materials, theirs: {:+} materials)",
                ours_material_delta, theirs_material_delta
            ));
        }

        // Check bounding box overlap if available
        if let (Some(our_bbox), Some(their_bbox)) = (&ours.bounding_box, &theirs.bounding_box) {
            if let Some(base_bbox) = &base.bounding_box {
                let ours_changed = our_bbox.volume() != base_bbox.volume();
                let theirs_changed = their_bbox.volume() != base_bbox.volume();

                if ours_changed && theirs_changed && our_bbox.overlaps(their_bbox) {
                    conflicts.push(
                        "Both branches modified overlapping spatial regions".to_string()
                    );
                }
            }
        }

        if conflicts.is_empty() {
            info!("No 3D model conflicts detected - can auto-merge");
            MergeDecision::AutoMerge
        } else {
            warn!("Found {} 3D model conflicts", conflicts.len());
            MergeDecision::ManualReview(conflicts)
        }
    }
}

impl Default for Model3DParser {
    fn default() -> Self {
        Self::new()
    }
}

/// 3D model merge decision
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
        assert_eq!(Model3DFormat::from_extension("obj"), Model3DFormat::Obj);
        assert_eq!(Model3DFormat::from_extension("fbx"), Model3DFormat::Fbx);
        assert_eq!(Model3DFormat::from_extension("blend"), Model3DFormat::Blend);
        assert_eq!(Model3DFormat::from_extension("gltf"), Model3DFormat::Gltf);
        assert_eq!(Model3DFormat::from_extension("glb"), Model3DFormat::Glb);
        assert_eq!(Model3DFormat::from_extension("stl"), Model3DFormat::Stl);
        assert_eq!(Model3DFormat::from_extension("usdz"), Model3DFormat::Usd);
        assert_eq!(Model3DFormat::from_extension("usda"), Model3DFormat::Usd);
        assert_eq!(Model3DFormat::from_extension("usdc"), Model3DFormat::Usd);
        assert_eq!(Model3DFormat::from_extension("usd"), Model3DFormat::Usd);
        assert_eq!(Model3DFormat::from_extension("ply"), Model3DFormat::Ply);
        assert_eq!(Model3DFormat::from_extension("unknown"), Model3DFormat::Unknown);
    }

    #[test]
    fn test_bounding_box_expand() {
        let mut bbox = BoundingBox::new();
        bbox.expand(1.0, 2.0, 3.0);
        bbox.expand(-1.0, -2.0, -3.0);

        assert_eq!(bbox.min, (-1.0, -2.0, -3.0));
        assert_eq!(bbox.max, (1.0, 2.0, 3.0));
    }

    #[test]
    fn test_bounding_box_volume() {
        let bbox = BoundingBox {
            min: (0.0, 0.0, 0.0),
            max: (2.0, 3.0, 4.0),
        };
        assert_eq!(bbox.volume(), 24.0);
    }

    #[test]
    fn test_bounding_box_overlap() {
        let bbox1 = BoundingBox {
            min: (0.0, 0.0, 0.0),
            max: (2.0, 2.0, 2.0),
        };

        let bbox2 = BoundingBox {
            min: (1.0, 1.0, 1.0),
            max: (3.0, 3.0, 3.0),
        };

        assert!(bbox1.overlaps(&bbox2));
    }

    #[test]
    fn test_bounding_box_no_overlap() {
        let bbox1 = BoundingBox {
            min: (0.0, 0.0, 0.0),
            max: (1.0, 1.0, 1.0),
        };

        let bbox2 = BoundingBox {
            min: (5.0, 5.0, 5.0),
            max: (6.0, 6.0, 6.0),
        };

        assert!(!bbox1.overlaps(&bbox2));
    }

    #[tokio::test]
    async fn test_simple_obj_parsing() {
        let obj_data = b"# Simple OBJ file
v 0.0 0.0 0.0
v 1.0 0.0 0.0
v 1.0 1.0 0.0
v 0.0 1.0 0.0
f 1 2 3 4
usemtl Material_001
";

        let parser = Model3DParser::new();
        let result = parser.parse(obj_data, "test.obj").await;

        assert!(result.is_ok());
        let info = result.unwrap();
        assert_eq!(info.format, Model3DFormat::Obj);
        assert_eq!(info.vertex_count, 4);
        assert_eq!(info.face_count, 1);
    }
}
