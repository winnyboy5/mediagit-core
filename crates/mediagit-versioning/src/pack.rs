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

//! Pack file implementation for efficient multi-object storage
//!
//! Pack files provide efficient storage of multiple objects with optional
//! delta compression for similar objects.
//!
//! # Format
//!
//! ```text
//! [Header: 12 bytes]
//!   - Signature: "PACK" (4 bytes)
//!   - Version: u32 (4 bytes, currently 2)
//!   - Object count: u32 (4 bytes)
//! [Objects: variable]
//!   - Object entries (variable size)
//! [Index: variable]
//!   - OID -> (offset, size) mapping
//! [Checksum: 32 bytes]
//!   - SHA-256 of pack content
//! ```

use crate::{ObjectType, Oid};
use crate::delta::{Delta, DeltaDecoder};
use serde::{Deserialize, Serialize};
use sha2::Digest;
use std::collections::BTreeMap;
use std::io;
use tracing::{debug, info, warn};

/// Magic bytes for delta-encoded objects in pack files
const DELTA_MAGIC: &[u8; 5] = b"DELTA";

const PACK_SIGNATURE: &[u8; 4] = b"PACK";
const PACK_VERSION: u32 = 2;
const CHECKSUM_SIZE: usize = 32;

/// Pack file header
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackHeader {
    /// Pack format version
    pub version: u32,
    /// Total number of objects in pack
    pub object_count: u32,
}

impl PackHeader {
    /// Create a new pack header
    pub fn new(object_count: u32) -> Self {
        Self {
            version: PACK_VERSION,
            object_count,
        }
    }

    /// Serialize header to bytes
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(12);
        bytes.extend_from_slice(PACK_SIGNATURE);
        bytes.extend_from_slice(&self.version.to_le_bytes());
        bytes.extend_from_slice(&self.object_count.to_le_bytes());
        bytes
    }

    /// Deserialize header from bytes
    pub fn from_bytes(data: &[u8]) -> io::Result<Self> {
        if data.len() < 12 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Pack header too short",
            ));
        }

        if &data[0..4] != PACK_SIGNATURE {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Invalid pack signature",
            ));
        }

        let version = u32::from_le_bytes([data[4], data[5], data[6], data[7]]);
        let object_count = u32::from_le_bytes([data[8], data[9], data[10], data[11]]);

        if version != PACK_VERSION {
            warn!(
                expected = PACK_VERSION,
                actual = version,
                "Pack version mismatch"
            );
        }

        Ok(Self {
            version,
            object_count,
        })
    }
}

/// Pack object entry metadata
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackObjectEntry {
    /// Object identifier
    pub oid: Oid,
    /// Object type
    pub object_type: ObjectType,
    /// Offset in pack file
    pub offset: u64,
    /// Size of object data (compressed or delta)
    pub size: u32,
    /// Optional base OID for delta-encoded objects
    pub base_oid: Option<Oid>,
}

/// Pack object index for fast lookup
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PackIndex {
    /// Map from OID to (offset, size)
    entries: BTreeMap<Oid, (u64, u32)>,
}

impl PackIndex {
    /// Create a new empty pack index
    pub fn new() -> Self {
        Self {
            entries: BTreeMap::new(),
        }
    }

    /// Add an entry to the index
    pub fn insert(&mut self, oid: Oid, offset: u64, size: u32) {
        self.entries.insert(oid, (offset, size));
    }

    /// Look up an object in the index
    ///
    /// Returns (offset, size) if found
    pub fn lookup(&self, oid: &Oid) -> Option<(u64, u32)> {
        self.entries.get(oid).copied()
    }

    /// Get the number of entries in the index
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Check if the index is empty
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Iterate over all entries
    pub fn iter(&self) -> impl Iterator<Item = (&Oid, &(u64, u32))> {
        self.entries.iter()
    }

    /// Serialize index to bytes (simple format: count, then OID, offset, size triplets)
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut bytes = Vec::new();

        // Write entry count
        bytes.extend_from_slice(&(self.entries.len() as u32).to_le_bytes());

        // Write each entry
        for (oid, (offset, size)) in &self.entries {
            bytes.extend_from_slice(oid.as_bytes());
            bytes.extend_from_slice(&offset.to_le_bytes());
            bytes.extend_from_slice(&size.to_le_bytes());
        }

        bytes
    }

    /// Deserialize index from bytes
    pub fn from_bytes(data: &[u8]) -> io::Result<Self> {
        if data.len() < 4 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Index too short",
            ));
        }

        let count = u32::from_le_bytes([data[0], data[1], data[2], data[3]]) as usize;
        let expected_len = 4 + count * (32 + 8 + 4); // count + (OID + offset + size) * count

        if data.len() < expected_len {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Index data too short for entry count",
            ));
        }

        let mut entries = BTreeMap::new();
        let mut pos = 4;

        for _ in 0..count {
            let mut oid_bytes = [0u8; 32];
            oid_bytes.copy_from_slice(&data[pos..pos + 32]);
            pos += 32;

            let offset = u64::from_le_bytes([
                data[pos], data[pos + 1], data[pos + 2], data[pos + 3],
                data[pos + 4], data[pos + 5], data[pos + 6], data[pos + 7],
            ]);
            pos += 8;

            let size = u32::from_le_bytes([data[pos], data[pos + 1], data[pos + 2], data[pos + 3]]);
            pos += 4;

            entries.insert(Oid::from(oid_bytes), (offset, size));
        }

        Ok(Self { entries })
    }
}

/// Pack file metadata and statistics
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackMetadata {
    /// Total size of pack file in bytes
    pub total_size: u64,
    /// Number of objects in pack
    pub object_count: u32,
    /// Number of delta-encoded objects
    pub delta_count: u32,
    /// Total size of original objects
    pub uncompressed_size: u64,
    /// Compression ratio (compressed / uncompressed)
    pub compression_ratio: f64,
}

/// Determine if an object should be included in pack based on size and type
///
/// This provides intelligent pack selection to avoid packing objects that:
/// - Are too small (metadata overhead outweighs benefit)
/// - Are already heavily compressed (no delta opportunity)
/// - Have poor delta compression potential
fn should_pack_object(size: usize, object_type: ObjectType, filename: Option<&str>) -> bool {
    const MIN_PACK_SIZE: usize = 1024; // 1KB minimum

    match object_type {
        ObjectType::Blob => {
            // Skip tiny blobs (metadata, small configs) - packing overhead too high
            if size < MIN_PACK_SIZE {
                return false;
            }

            // Type-based packing decision for blobs
            if let Some(name) = filename {
                // Extract file extension for intelligent decision
                let ext = std::path::Path::new(name)
                    .extension()
                    .and_then(|e| e.to_str())
                    .unwrap_or("");

                match ext.to_lowercase().as_str() {
                    // Large uncompressed media: Always pack (good delta opportunity)
                    "psd" | "tif" | "tiff" | "bmp" | "wav" | "aiff" | "avi" | "mov" => true,
                    // Compressed images: Pack only if large (limited delta benefit)
                    "jpg" | "jpeg" | "png" | "webp" => size > 100 * 1024, // 100KB
                    // Compressed video: Pack only if very large
                    "mp4" | "mkv" | "flv" | "wmv" => size > 100 * 1024, // 100KB
                    // Text/code: Always pack (excellent delta opportunity)
                    "txt" | "md" | "json" | "xml" | "html" | "css" | "js" | "ts" |
                    "py" | "rs" | "go" | "java" | "c" | "cpp" | "h" | "hpp" => true,
                    // Archives: Don't pack (already compressed, no delta benefit)
                    "zip" | "gz" | "bz2" | "7z" | "rar" => false,
                    // Unknown: Pack if above minimum size
                    _ => size > MIN_PACK_SIZE,
                }
            } else {
                // No filename: pack if above minimum
                size > MIN_PACK_SIZE
            }
        }
        // Always pack commits and trees (small, critical metadata)
        ObjectType::Commit | ObjectType::Tree => true,
    }
}

/// Pack file writer for creating pack files
pub struct PackWriter {
    /// Current data buffer
    data: Vec<u8>,
    /// Index for objects
    index: PackIndex,
    /// Object entries metadata
    entries: Vec<PackObjectEntry>,
}

impl PackWriter {
    /// Create a new pack writer
    pub fn new() -> Self {
        Self {
            data: Vec::new(),
            index: PackIndex::new(),
            entries: Vec::new(),
        }
    }

    /// Add an object to the pack
    ///
    /// # Arguments
    ///
    /// * `oid` - Object identifier
    /// * `object_type` - Type of object
    /// * `object_data` - Object data
    ///
    /// # Returns
    ///
    /// Offset of the object in the pack
    pub fn add_object(
        &mut self,
        oid: Oid,
        object_type: ObjectType,
        object_data: &[u8],
    ) -> u64 {
        let offset = self.data.len() as u64;

        // Write simple header: 1 byte type + 4 bytes size
        let type_byte = match object_type {
            ObjectType::Blob => 1u8,
            ObjectType::Tree => 2u8,
            ObjectType::Commit => 3u8,
        };
        self.data.push(type_byte);
        self.data.extend_from_slice(&(object_data.len() as u32).to_le_bytes());

        // Write object data
        let size = object_data.len() as u32;
        self.data.extend_from_slice(object_data);

        // Record entry - adjust for header size (5 bytes)
        self.index.insert(oid, offset, size + 5);
        self.entries.push(PackObjectEntry {
            oid,
            object_type,
            offset,
            size,
            base_oid: None,
        });

        offset
    }

    /// Add an object to the pack with intelligent selection
    ///
    /// Uses type-aware logic to determine if object should be packed.
    /// Returns Ok(Some(offset)) if packed, Ok(None) if skipped.
    ///
    /// # Arguments
    ///
    /// * `oid` - Object identifier
    /// * `object_type` - Type of object
    /// * `object_data` - Object data
    /// * `filename` - Optional filename for type inference
    ///
    /// # Returns
    ///
    /// Option with offset if packed, None if skipped
    pub fn smart_add_object(
        &mut self,
        oid: Oid,
        object_type: ObjectType,
        object_data: &[u8],
        filename: Option<&str>,
    ) -> Option<u64> {
        // Check if object should be packed
        if should_pack_object(object_data.len(), object_type, filename) {
            Some(self.add_object(oid, object_type, object_data))
        } else {
            debug!(
                oid = %oid,
                size = object_data.len(),
                filename = ?filename,
                "Skipping object from pack (too small or poor compression candidate)"
            );
            None
        }
    }

    /// Add a delta-encoded object to the pack
    ///
    /// # Arguments
    ///
    /// * `oid` - Object identifier
    /// * `base_oid` - OID of the base object
    /// * `delta_data` - Delta instructions
    ///
    /// # Returns
    ///
    /// Offset of the object in the pack
    pub fn add_delta_object(&mut self, oid: Oid, base_oid: Oid, delta_data: &[u8]) -> u64 {
        let offset = self.data.len() as u64;

        // Write delta header with base OID reference
        self.data.extend_from_slice(b"DELTA");
        self.data.extend_from_slice(base_oid.as_bytes());

        // Write delta data
        let size = delta_data.len() as u32;
        self.data.extend_from_slice(delta_data);

        // Record entry
        self.index.insert(oid, offset, size);
        self.entries.push(PackObjectEntry {
            oid,
            object_type: ObjectType::Blob, // Delta objects are stored as blobs
            offset,
            size,
            base_oid: Some(base_oid),
        });

        offset
    }

    /// Finalize the pack and get the complete pack data
    ///
    /// # Returns
    ///
    /// Complete pack file data with header, objects, index, and checksum
    pub fn finalize(self) -> Vec<u8> {
        let mut pack_data = Vec::new();

        // Write header
        let header = PackHeader::new(self.entries.len() as u32);
        pack_data.extend_from_slice(&header.to_bytes());

        // The object data starts after the header
        let objects_start = pack_data.len();

        // Write objects
        pack_data.extend_from_slice(&self.data);

        // Now adjust index offsets to be relative to pack file start
        let mut adjusted_index = PackIndex::new();
        for (oid, (offset, size)) in &self.index.entries {
            adjusted_index.insert(*oid, objects_start as u64 + offset, *size);
        }

        // Write index
        let index_bytes = adjusted_index.to_bytes();
        let index_offset = pack_data.len() as u32;
        pack_data.extend_from_slice(&index_bytes);

        // Write index offset (helps us find the index when reading)
        pack_data.extend_from_slice(&index_offset.to_le_bytes());

        // Calculate and write checksum for content (excluding checksum itself)
        let checksum = sha2::Sha256::digest(&pack_data[..]);
        pack_data.extend_from_slice(&checksum);

        debug!(
            size = pack_data.len(),
            objects = self.entries.len(),
            "Pack file finalized"
        );

        pack_data
    }

}

impl Default for PackWriter {
    fn default() -> Self {
        Self::new()
    }
}

/// Pack file reader for extracting objects from packs
pub struct PackReader {
    data: Vec<u8>,
    index: PackIndex,
    _object_data_end: usize,
}

impl PackReader {
    /// Create a pack reader from pack data
    ///
    /// # Errors
    ///
    /// Returns error if pack format is invalid
    pub fn new(data: Vec<u8>) -> io::Result<Self> {
        if data.len() < 12 + CHECKSUM_SIZE + 4 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Pack file too short",
            ));
        }

        // Verify header
        PackHeader::from_bytes(&data[0..12])?;

        // Verify checksum (at end)
        let checksum_offset = data.len() - CHECKSUM_SIZE;
        let expected_checksum = &data[checksum_offset..];
        let actual_checksum = sha2::Sha256::digest(&data[0..checksum_offset]);

        if actual_checksum[..] != expected_checksum[..] {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Pack checksum verification failed",
            ));
        }

        // Read index offset (located right before index offset marker and checksum)
        let index_offset_pos = data.len() - CHECKSUM_SIZE - 4;
        let index_offset =
            u32::from_le_bytes([data[index_offset_pos], data[index_offset_pos + 1], data[index_offset_pos + 2], data[index_offset_pos + 3]]) as usize;

        if index_offset < 12 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Invalid pack index offset",
            ));
        }

        // Parse index
        let index = PackIndex::from_bytes(&data[index_offset..index_offset_pos])?;

        info!(
            object_count = index.len(),
            "Pack file loaded successfully"
        );

        Ok(Self {
            data,
            index,
            _object_data_end: index_offset,
        })
    }

    /// Get object data by OID
    ///
    /// # Errors
    ///
    /// Returns error if object not found or data is corrupted
    pub fn get_object(&self, oid: &Oid) -> io::Result<Vec<u8>> {
        let (_, data) = self.get_object_with_type(oid)?;
        Ok(data)
    }

    /// Get object data and type by OID
    ///
    /// Handles both regular objects and delta-encoded objects.
    /// For delta objects, recursively retrieves the base object and applies the delta.
    ///
    /// # Errors
    ///
    /// Returns error if object not found or data is corrupted
    pub fn get_object_with_type(&self, oid: &Oid) -> io::Result<(ObjectType, Vec<u8>)> {
        self.get_object_with_type_depth(oid, 0)
    }

    /// Internal method with depth tracking to prevent infinite recursion in delta chains
    fn get_object_with_type_depth(&self, oid: &Oid, depth: usize) -> io::Result<(ObjectType, Vec<u8>)> {
        const MAX_DELTA_CHAIN_DEPTH: usize = 10;

        if depth > MAX_DELTA_CHAIN_DEPTH {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("Delta chain too deep (> {})", MAX_DELTA_CHAIN_DEPTH),
            ));
        }

        let (offset, total_size) = self
            .index
            .lookup(oid)
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "Object not found in pack"))?;

        let offset = offset as usize;
        let total_size = total_size as usize;

        if offset + total_size > self.data.len() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Object data corrupted",
            ));
        }

        // Check if this is a delta-encoded object (starts with "DELTA" magic)
        if total_size >= DELTA_MAGIC.len() && &self.data[offset..offset + DELTA_MAGIC.len()] == DELTA_MAGIC {
            return self.read_delta_object(oid, offset, total_size, depth);
        }

        // Regular object: read type from 1-byte header
        let type_byte = self.data[offset];
        let object_type = ObjectType::from_u8(type_byte)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "Invalid object type"))?;

        // Skip the 5-byte header (1 byte type + 4 bytes size)
        let header_size = 5;
        if total_size < header_size {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Object size too small for header",
            ));
        }

        let data_size = total_size - header_size;
        let data = self.data[offset + header_size..offset + header_size + data_size].to_vec();

        Ok((object_type, data))
    }

    /// Read and reconstruct a delta-encoded object from the pack
    fn read_delta_object(&self, oid: &Oid, offset: usize, total_size: usize, depth: usize) -> io::Result<(ObjectType, Vec<u8>)> {
        // Delta format: "DELTA" (5 bytes) + base_oid (32 bytes) + delta_data
        const BASE_OID_SIZE: usize = 32;
        let min_delta_size = DELTA_MAGIC.len() + BASE_OID_SIZE;

        if total_size < min_delta_size {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Delta object too small",
            ));
        }

        // Read base OID
        let base_oid_start = offset + DELTA_MAGIC.len();
        let mut base_oid_bytes = [0u8; 32];
        base_oid_bytes.copy_from_slice(&self.data[base_oid_start..base_oid_start + BASE_OID_SIZE]);
        let base_oid = Oid::from(base_oid_bytes);

        // Read delta data
        let delta_data_start = base_oid_start + BASE_OID_SIZE;
        let delta_data = &self.data[delta_data_start..offset + total_size];

        debug!(
            oid = %oid,
            base_oid = %base_oid,
            delta_size = delta_data.len(),
            "Reading delta object from pack"
        );

        // Get base object (may be another delta, so use depth tracking)
        let (base_type, base_data) = self.get_object_with_type_depth(&base_oid, depth + 1)?;

        // Parse and apply delta
        let delta = Delta::from_bytes(delta_data)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, format!("Failed to parse delta: {}", e)))?;

        let reconstructed = DeltaDecoder::apply(&base_data, &delta)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, format!("Failed to apply delta: {}", e)))?;

        debug!(
            oid = %oid,
            base_size = base_data.len(),
            result_size = reconstructed.len(),
            "Reconstructed delta object"
        );

        // Delta objects inherit the type from their base object
        Ok((base_type, reconstructed))
    }

    /// Get the index reference
    pub fn index(&self) -> &PackIndex {
        &self.index
    }

    /// List all objects in the pack
    pub fn list_objects(&self) -> Vec<Oid> {
        self.index.iter().map(|(oid, _)| *oid).collect()
    }

    /// Get pack statistics
    pub fn stats(&self) -> PackMetadata {
        let object_count = self.index.len() as u32;
        let total_size = self.data.len() as u64;
        let uncompressed_size = self
            .index
            .iter()
            .map(|(_, (_, size))| *size as u64)
            .sum();

        let compression_ratio = if uncompressed_size > 0 {
            total_size as f64 / uncompressed_size as f64
        } else {
            1.0
        };

        PackMetadata {
            total_size,
            object_count,
            delta_count: 0, // Placeholder
            uncompressed_size,
            compression_ratio,
        }
    }
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pack_header_roundtrip() {
        let header = PackHeader::new(42);
        let bytes = header.to_bytes();
        assert_eq!(bytes.len(), 12);
        assert_eq!(&bytes[0..4], PACK_SIGNATURE);

        let decoded = PackHeader::from_bytes(&bytes).unwrap();
        assert_eq!(decoded.object_count, 42);
    }

    #[test]
    fn test_pack_writer_add_object() {
        let mut writer = PackWriter::new();
        let oid = Oid::hash(b"test object");
        let data = b"test data";

        writer.add_object(oid, ObjectType::Blob, data);

        assert_eq!(writer.index.len(), 1);
        assert!(writer.index.lookup(&oid).is_some());
    }

    #[test]
    fn test_pack_writer_finalize() {
        let mut writer = PackWriter::new();
        let oid1 = Oid::hash(b"object1");
        let oid2 = Oid::hash(b"object2");

        writer.add_object(oid1, ObjectType::Blob, b"data1");
        writer.add_object(oid2, ObjectType::Tree, b"data2");

        let pack_data = writer.finalize();

        // Verify header
        assert_eq!(&pack_data[0..4], PACK_SIGNATURE);

        // Verify we have checksum at end
        assert!(pack_data.len() > 12 + CHECKSUM_SIZE);
    }

    #[test]
    fn test_pack_reader_verification() {
        let mut writer = PackWriter::new();
        let oid = Oid::hash(b"test");
        let data = b"hello world";

        writer.add_object(oid, ObjectType::Blob, data);
        let pack_data = writer.finalize();

        // Reader should successfully load the pack
        let reader = PackReader::new(pack_data).unwrap();
        assert_eq!(reader.index.len(), 1);

        // Should be able to retrieve the object
        let retrieved = reader.get_object(&oid).unwrap();
        assert_eq!(retrieved, data);
    }

    #[test]
    fn test_pack_index_operations() {
        let mut index = PackIndex::new();
        let oid = Oid::hash(b"test");

        index.insert(oid, 100, 50);
        assert_eq!(index.lookup(&oid), Some((100, 50)));

        let other_oid = Oid::hash(b"other");
        assert_eq!(index.lookup(&other_oid), None);
    }

    #[test]
    fn test_pack_reader_object_not_found() {
        let mut writer = PackWriter::new();
        let oid = Oid::hash(b"test");

        writer.add_object(oid, ObjectType::Blob, b"data");
        let pack_data = writer.finalize();

        let reader = PackReader::new(pack_data).unwrap();
        let missing_oid = Oid::hash(b"missing");

        assert!(reader.get_object(&missing_oid).is_err());
    }

    #[test]
    fn test_pack_reader_list_objects() {
        let mut writer = PackWriter::new();
        let oid1 = Oid::hash(b"first");
        let oid2 = Oid::hash(b"second");

        writer.add_object(oid1, ObjectType::Blob, b"data1");
        writer.add_object(oid2, ObjectType::Tree, b"data2");
        let pack_data = writer.finalize();

        let reader = PackReader::new(pack_data).unwrap();
        let objects = reader.list_objects();

        assert_eq!(objects.len(), 2);
        assert!(objects.contains(&oid1));
        assert!(objects.contains(&oid2));
    }

    #[test]
    fn test_invalid_pack_signature() {
        let mut bad_data = vec![0u8; 12];
        bad_data[0..4].copy_from_slice(b"XXXX");

        assert!(PackReader::new(bad_data).is_err());
    }

    #[test]
    fn test_pack_stats() {
        let mut writer = PackWriter::new();
        let oid1 = Oid::hash(b"obj1");
        let oid2 = Oid::hash(b"obj2");

        writer.add_object(oid1, ObjectType::Blob, &vec![0u8; 100]);
        writer.add_object(oid2, ObjectType::Blob, &vec![0u8; 200]);
        let pack_data = writer.finalize();

        let reader = PackReader::new(pack_data).unwrap();
        let stats = reader.stats();

        assert_eq!(stats.object_count, 2);
        // Uncompressed size includes headers: 100 + 5 + 200 + 5 = 310
        assert_eq!(stats.uncompressed_size, 310);
    }
}
