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

//! Streaming pack file implementation for memory-efficient transfers
//!
//! This module provides streaming pack reader/writer that process objects
//! incrementally without loading entire packs into memory.

use crate::{ObjectType, Oid};
use crate::pack::PackHeader;
use crate::streaming_index::StreamingPackIndex;
use sha2::{Digest, Sha256};
use std::io;
use std::path::Path;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tracing::{debug, trace};

const DELTA_MAGIC: &[u8; 5] = b"DELTA";
#[allow(dead_code)]
const CHECKSUM_SIZE: usize = 32;

/// Streaming pack reader that processes objects incrementally
pub struct StreamingPackReader<R: AsyncRead + Unpin> {
    reader: R,
    header: Option<PackHeader>,
    objects_processed: u32,
    expected_count: u32,
    hasher: Sha256,
    #[allow(dead_code)]
    buffer: Vec<u8>,
}

impl<R: AsyncRead + Unpin> StreamingPackReader<R> {
    /// Create new streaming pack reader
    pub async fn new(mut reader: R) -> io::Result<Self> {
        let mut header_buf = vec![0u8; 12];
        reader.read_exact(&mut header_buf).await?;

        let header = PackHeader::from_bytes(&header_buf)?;
        let mut hasher = Sha256::new();
        hasher.update(&header_buf);

        debug!(
            version = header.version,
            object_count = header.object_count,
            "Streaming pack reader initialized"
        );

        Ok(Self {
            reader,
            header: Some(header.clone()),
            objects_processed: 0,
            expected_count: header.object_count,
            hasher,
            buffer: Vec::with_capacity(8192),
        })
    }

    /// Read next object from pack stream
    /// Returns None when all objects have been read
    pub async fn next_object(&mut self) -> Option<io::Result<(Oid, ObjectType, Vec<u8>)>> {
        if self.objects_processed >= self.expected_count {
            return None;
        }

        match self.read_object_internal().await {
            Ok((oid, obj_type, data)) => {
                self.objects_processed += 1;
                trace!(
                    oid = %oid,
                    obj_type = ?obj_type,
                    size = data.len(),
                    progress = self.objects_processed,
                    total = self.expected_count,
                    "Read object from pack stream"
                );
                Some(Ok((oid, obj_type, data)))
            }
            Err(e) => Some(Err(e)),
        }
    }

    async fn read_object_internal(&mut self) -> io::Result<(Oid, ObjectType, Vec<u8>)> {
        // Read object header: type (1 byte) + size (4 bytes)
        let mut header_buf = [0u8; 5];
        self.reader.read_exact(&mut header_buf).await?;
        self.hasher.update(header_buf);

        let type_byte = header_buf[0];
        let size = u32::from_le_bytes([
            header_buf[1],
            header_buf[2],
            header_buf[3],
            header_buf[4],
        ]) as usize;

        // Read object data
        let mut obj_data = vec![0u8; size];
        self.reader.read_exact(&mut obj_data).await?;
        self.hasher.update(&obj_data);

        // Check for delta encoding
        if obj_data.len() >= 5 && &obj_data[0..5] == DELTA_MAGIC {
            // Delta object - extract base OID and delta data
            if obj_data.len() < 37 {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "Delta object too small",
                ));
            }

            // Extract base OID (32 bytes after DELTA magic)
            let mut base_bytes = [0u8; 32];
            base_bytes.copy_from_slice(&obj_data[5..37]);
            let base_oid = Oid::from_bytes(base_bytes);
            let _delta_data = &obj_data[37..];

            // Note: Delta reconstruction requires base object lookup
            // For streaming, we'll need to handle this differently
            // For now, return error - will be addressed in Epic 1.1.4
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                format!("Delta objects require base lookup: {}", base_oid),
            ));
        }

        // Parse object type
        let obj_type = match type_byte {
            1 => ObjectType::Blob,
            2 => ObjectType::Tree,
            3 => ObjectType::Commit,
            _ => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("Invalid object type: {}", type_byte),
                ))
            }
        };

        // Calculate OID
        let oid = Oid::hash(&obj_data);

        Ok((oid, obj_type, obj_data))
    }

    /// Verify pack checksum after reading all objects
    pub fn verify_checksum(&mut self) -> io::Result<()> {
        // In a complete implementation, we would:
        // 1. Read the final checksum bytes from stream
        // 2. Compare with accumulated hasher result
        // For now, return Ok (will be implemented in Epic 1.1.5)
        debug!(
            objects_read = self.objects_processed,
            "Pack checksum verification (placeholder)"
        );
        Ok(())
    }

    /// Get number of objects processed so far
    pub fn objects_processed(&self) -> u32 {
        self.objects_processed
    }

    /// Get pack header
    pub fn header(&self) -> Option<&PackHeader> {
        self.header.as_ref()
    }
}

/// Streaming pack writer that generates pack data incrementally
///
/// Uses `StreamingPackIndex` for O(1) memory regardless of object count.
pub struct StreamingPackWriter<W: AsyncWrite + Unpin> {
    writer: W,
    objects_written: u32,
    expected_count: u32,
    hasher: Sha256,
    index: Option<StreamingPackIndex>,
    current_offset: u64,
    #[allow(dead_code)]
    header_written: bool,
}

impl<W: AsyncWrite + Unpin> StreamingPackWriter<W> {
    /// Create new streaming pack writer with disk-based index
    ///
    /// # Arguments
    /// * `writer` - Output stream for pack data
    /// * `expected_count` - Expected number of objects
    /// * `temp_dir` - Directory for temporary index file
    pub async fn new(mut writer: W, expected_count: u32, temp_dir: &Path) -> io::Result<Self> {
        // Write pack header
        let header = PackHeader::new(expected_count);
        let header_bytes = header.to_bytes();
        writer.write_all(&header_bytes).await?;

        let mut hasher = Sha256::new();
        hasher.update(&header_bytes);

        // Create streaming index for O(1) memory
        let index = StreamingPackIndex::new(temp_dir).await?;

        debug!(
            expected_count = expected_count,
            temp_dir = %temp_dir.display(),
            "Streaming pack writer initialized with disk-based index"
        );

        Ok(Self {
            writer,
            objects_written: 0,
            expected_count,
            hasher,
            index: Some(index),
            current_offset: 12, // After header
            header_written: true,
        })
    }

    /// Write object to pack stream
    pub async fn write_object(
        &mut self,
        oid: Oid,
        obj_type: ObjectType,
        data: &[u8],
    ) -> io::Result<()> {
        let entry_offset = self.current_offset;

        // Write object header
        let type_byte: u8 = match obj_type {
            ObjectType::Blob => 1,
            ObjectType::Tree => 2,
            ObjectType::Commit => 3,
        };

        let size = data.len() as u32;
        let mut header = Vec::with_capacity(5);
        header.push(type_byte);
        header.extend_from_slice(&size.to_le_bytes());

        self.writer.write_all(&header).await?;
        self.hasher.update(&header);
        self.current_offset += 5;

        // Write object data
        self.writer.write_all(data).await?;
        self.hasher.update(data);
        self.current_offset += data.len() as u64;

        // Record index entry to streaming index
        // Note: size includes the 5-byte header to match PackWriter behavior
        if let Some(index) = self.index.as_mut() {
            index.add_entry(oid, entry_offset, size + 5).await?;
        }
        self.objects_written += 1;

        trace!(
            oid = %oid,
            obj_type = ?obj_type,
            size = data.len(),
            offset = entry_offset,
            progress = self.objects_written,
            total = self.expected_count,
            "Wrote object to pack stream"
        );

        Ok(())
    }

    /// Finalize pack by writing index and checksum
    pub async fn finalize(mut self) -> io::Result<()> {
        if self.objects_written != self.expected_count {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!(
                    "Object count mismatch: expected {}, wrote {}",
                    self.expected_count, self.objects_written
                ),
            ));
        }

        // Current offset is where the index will start (after header + objects)
        let index_offset = self.current_offset as u32;

        // Finalize streaming index to get serialized bytes
        let index_bytes = if let Some(index) = self.index.take() {
            index.finalize().await?
        } else {
            // Empty index still needs 4-byte count prefix
            vec![0, 0, 0, 0]
        };

        // Write index bytes
        self.writer.write_all(&index_bytes).await?;
        self.hasher.update(&index_bytes);

        // Write index offset (the position where the index starts in the pack file)
        let index_offset_bytes = index_offset.to_le_bytes();
        self.writer.write_all(&index_offset_bytes).await?;
        self.hasher.update(index_offset_bytes);

        // Write final checksum
        let checksum = self.hasher.finalize();
        self.writer.write_all(&checksum).await?;

        // Flush to ensure all data is written
        self.writer.flush().await?;

        debug!(
            objects_written = self.objects_written,
            index_offset = index_offset,
            "Pack finalized with index and checksum"
        );

        Ok(())
    }

    /// Get number of objects written so far
    pub fn objects_written(&self) -> u32 {
        self.objects_written
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;
    use tokio::fs::File;

    #[tokio::test]
    async fn test_streaming_pack_empty() {
        let temp_dir = TempDir::new().unwrap();
        let pack_path = temp_dir.path().join("test.pack");

        {
            eprintln!("Creating file: {:?}", pack_path);
            let file = File::create(&pack_path).await.unwrap();
            eprintln!("Creating writer");
            let writer = StreamingPackWriter::new(file, 0, temp_dir.path()).await.unwrap();
            eprintln!("Finalizing");
            match writer.finalize().await {
                Ok(_) => eprintln!("Finalize succeeded"),
                Err(e) => {
                    eprintln!("Finalize failed: {:?}", e);
                    panic!("Finalize error: {}", e);
                }
            }
        }

        // Verify file exists and has content
        let metadata = tokio::fs::metadata(&pack_path).await.unwrap();
        eprintln!("Pack file size: {}", metadata.len());

        // Read back from file
        let file = File::open(&pack_path).await.unwrap();
        let mut reader = StreamingPackReader::new(file)
            .await
            .unwrap();

        assert_eq!(reader.objects_processed(), 0);
        assert!(reader.next_object().await.is_none());
    }

    #[tokio::test]
    async fn test_streaming_pack_single_object() {
        let temp_dir = TempDir::new().unwrap();
        let pack_path = temp_dir.path().join("test.pack");

        {
            let file = File::create(&pack_path).await.unwrap();
            let mut writer = StreamingPackWriter::new(file, 1, temp_dir.path()).await.unwrap();

            let test_data = b"Hello, streaming world!";
            let oid = Oid::hash(test_data);

            writer
                .write_object(oid, ObjectType::Blob, test_data)
                .await
                .unwrap();
            writer.finalize().await.unwrap();
        }

        // Read back from file
        let file = File::open(&pack_path).await.unwrap();
        let mut reader = StreamingPackReader::new(file)
            .await
            .unwrap();

        let (read_oid, read_type, read_data) = reader.next_object().await.unwrap().unwrap();
        let test_data = b"Hello, streaming world!";
        let oid = Oid::hash(test_data);

        assert_eq!(read_oid, oid);
        assert_eq!(read_type, ObjectType::Blob);
        assert_eq!(read_data, test_data);
        assert!(reader.next_object().await.is_none());
    }
}
