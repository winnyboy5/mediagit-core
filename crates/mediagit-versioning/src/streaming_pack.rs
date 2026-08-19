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

use crate::hash::Hasher;
use crate::pack::{PACK_HEADER_SIZE, PackHeader, PackKind};
use crate::streaming_index::StreamingPackIndex;
use crate::{ObjectType, Oid};
use std::io;
use std::path::{Path, PathBuf};
use tempfile::NamedTempFile;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncSeekExt, AsyncWrite, AsyncWriteExt};
use tracing::{debug, trace};

const DELTA_MAGIC: &[u8; 5] = b"DELTA";

/// Maximum allowed size for a single pack object (2 GB).
/// Prevents OOM from corrupted or malicious pack data advertising huge sizes.
const MAX_PACK_OBJECT_SIZE: usize = 2 * 1024 * 1024 * 1024;

/// VC-7: reject an object the 4-byte pack size field cannot describe.
///
/// `write_object` stored `data.len() as u32` unchecked. An object at or above
/// 4 GiB wrapped, so the header understated its length — and since the reader
/// uses that field to find the *next* object, every subsequent object in the
/// pack was misparsed. The pack stayed structurally plausible while decoding
/// to wrong bytes, which is worse than a hard failure.
///
/// Takes a length rather than the slice so the bound is testable without
/// allocating multiple gigabytes.
fn ensure_writable_object_size(len: usize, oid: &Oid) -> io::Result<()> {
    if len > MAX_PACK_OBJECT_SIZE {
        return Err(io::Error::other(format!(
            "pack object too large: {} bytes exceeds the {} byte limit (object {}). \
             Writing it would truncate the 4-byte size field and corrupt every \
             following object in the pack.",
            len, MAX_PACK_OBJECT_SIZE, oid
        )));
    }
    Ok(())
}

/// Streaming pack reader that processes objects incrementally
pub struct StreamingPackReader<R: AsyncRead + Unpin> {
    reader: R,
    header: Option<PackHeader>,
    objects_processed: u32,
    expected_count: u32,
    hasher: Hasher,
}

impl<R: AsyncRead + Unpin> StreamingPackReader<R> {
    /// Create new streaming pack reader
    pub async fn new(mut reader: R) -> io::Result<Self> {
        let mut header_buf = vec![0u8; 13];
        reader.read_exact(&mut header_buf).await?;

        let header = PackHeader::from_bytes(&header_buf)?;
        let mut hasher = Hasher::new();
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
        self.hasher.update(&header_buf);

        let type_byte = header_buf[0];
        let size = u32::from_le_bytes([header_buf[1], header_buf[2], header_buf[3], header_buf[4]])
            as usize;

        // Validate size before allocation to prevent OOM
        if size > MAX_PACK_OBJECT_SIZE {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "Pack object size {} exceeds maximum {} bytes",
                    size, MAX_PACK_OBJECT_SIZE
                ),
            ));
        }

        // Use try_reserve to handle allocation failure gracefully
        let mut obj_data = Vec::new();
        obj_data.try_reserve(size).map_err(|e| {
            io::Error::new(
                io::ErrorKind::OutOfMemory,
                format!("Failed to allocate {} bytes for pack object: {}", size, e),
            )
        })?;
        obj_data.resize(size, 0);
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
        let obj_type = ObjectType::from_u8(type_byte).ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("Invalid object type: {}", type_byte),
            )
        })?;

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

/// Result returned by `StreamingPackWriter::finalize_cloud`.
pub struct CloudPackResult {
    /// 32-byte BLAKE3 hash — the content address of the completed pack.
    pub pack_oid: Vec<u8>,
    /// Total byte length of the pack file.
    pub byte_len: u64,
    /// Path to the completed tempfile on disk (caller must upload then drop).
    pub temp_path: PathBuf,
    /// Chunk index entries sorted by offset.
    pub index: Vec<CloudChunkLoc>,
}

/// One entry in the cloud chunk index.
pub struct CloudChunkLoc {
    pub chunk_oid: Oid,
    pub offset: u64,
    pub length: u32,
}

/// Returns the maximum number of concurrent pack-builder tasks.
///
/// Reads `MEDIAGIT_PACK_BUILDER_CONCURRENCY` env var; defaults to `2`.
#[allow(dead_code)]
pub fn pack_builder_concurrency() -> usize {
    std::env::var("MEDIAGIT_PACK_BUILDER_CONCURRENCY")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(2)
}

/// Streaming pack writer that generates pack data incrementally
///
/// Uses `StreamingPackIndex` for O(1) memory regardless of object count.
pub struct StreamingPackWriter<W: AsyncWrite + Unpin> {
    writer: W,
    objects_written: u32,
    expected_count: u32,
    hasher: Hasher,
    index: Option<StreamingPackIndex>,
    current_offset: u64,
    kind: PackKind,
    /// Kept alive so the tempfile is not deleted until `finalize_cloud` consumes it.
    temp_named_file: Option<NamedTempFile>,
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

        let mut hasher = Hasher::new();
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
            current_offset: PACK_HEADER_SIZE as u64, // After pack header (PACK + version u32 + count u32 + kind u8)
            kind: PackKind::Local,
            temp_named_file: None,
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

        // Write object header. `ObjectType::to_u8`/`from_u8` are the single
        // source of truth for the wire byte value (matches `pack.rs`).
        let type_byte: u8 = obj_type.to_u8();

        // VC-7: refuse oversized objects instead of silently truncating.
        //
        // The size field is 4 bytes, and this was an unchecked `as u32`. An
        // object at or above 4 GiB wrapped, so the header understated its
        // length — and because the reader uses that field to find the *next*
        // object, every subsequent object in the pack was misparsed. The pack
        // stayed structurally plausible while decoding to wrong bytes, which
        // is worse than a hard failure.
        //
        // The reader already refuses anything over `MAX_PACK_OBJECT_SIZE`
        // (2 GiB), so enforcing the same bound here keeps writer and reader
        // agreeing rather than producing packs this build cannot read back.
        ensure_writable_object_size(data.len(), &oid)?;

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

        // Current offset is where the chunk_index will start (after header + objects)
        let chunk_index_offset: u64 = self.current_offset;

        // Finalize streaming index to get serialized bytes (count u32 + entries)
        let index_bytes = match self.index.take() {
            Some(index) => index.finalize().await?,
            _ => {
                // Empty index still needs 4-byte count prefix
                vec![0, 0, 0, 0]
            }
        };

        // Write index bytes
        self.writer.write_all(&index_bytes).await?;
        self.hasher.update(&index_bytes);

        // Write chunk_index_offset as u64 LE immediately before checksum
        let index_offset_bytes = chunk_index_offset.to_le_bytes();
        self.writer.write_all(&index_offset_bytes).await?;
        self.hasher.update(&index_offset_bytes);

        // Write final checksum
        let checksum: [u8; 32] = self.hasher.finalize();
        self.writer.write_all(&checksum).await?;

        // Flush to ensure all data is written
        self.writer.flush().await?;

        debug!(
            objects_written = self.objects_written,
            chunk_index_offset = chunk_index_offset,
            "Pack finalized with index and checksum"
        );

        Ok(())
    }

    /// Get number of objects written so far
    pub fn objects_written(&self) -> u32 {
        self.objects_written
    }
}

impl StreamingPackWriter<tokio::fs::File> {
    /// Create an open-ended streaming pack writer that streams to a temporary file.
    ///
    /// Unlike `new`, this constructor does not require knowing the object count upfront.
    /// Use `finalize_cloud` (not `finalize`) to complete the pack.
    ///
    /// # Arguments
    /// * `kind` - Pack storage kind (e.g. `PackKind::CloudObject`)
    /// * `temp_dir` - Directory where the temporary pack file is created
    pub async fn new_open_ended(kind: PackKind, temp_dir: &Path) -> io::Result<Self> {
        // Create the named tempfile synchronously then open it async
        let named_tf = NamedTempFile::new_in(temp_dir).map_err(|e: std::io::Error| {
            io::Error::new(e.kind(), format!("Failed to create temp pack file: {}", e))
        })?;
        let path = named_tf.path().to_path_buf();

        // Open the tempfile path for async read+write
        let file: tokio::fs::File = tokio::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(&path)
            .await?;

        // Write 13-byte placeholder header (will be patched in finalize_cloud)
        let mut writer = file;
        writer.write_all(&[0u8; PACK_HEADER_SIZE]).await?;

        // Create streaming index
        let index = StreamingPackIndex::new(temp_dir).await?;

        debug!(
            kind = ?kind,
            temp_dir = %temp_dir.display(),
            "Open-ended streaming pack writer initialized"
        );

        Ok(Self {
            writer,
            objects_written: 0,
            expected_count: 0, // sentinel: open-ended
            hasher: Hasher::new(),
            index: Some(index),
            current_offset: PACK_HEADER_SIZE as u64,
            kind,
            temp_named_file: Some(named_tf),
        })
    }

    /// Finalize the open-ended pack: patch the header, write the trailer, compute BLAKE3.
    ///
    /// Returns a `CloudPackResult` with the temp file path left on disk.
    /// The caller is responsible for uploading and then dropping the file.
    pub async fn finalize_cloud(mut self) -> io::Result<CloudPackResult> {
        let actual_count = self.objects_written;
        let chunk_index_offset: u64 = self.current_offset;

        // Finalize the streaming index to get [count u32][entries 44B×N]
        let index_bytes = match self.index.take() {
            Some(index) => index.finalize().await?,
            _ => {
                vec![0, 0, 0, 0]
            }
        };

        // Write index bytes then chunk_index_offset pointer (no BLAKE3 yet)
        self.writer.write_all(&index_bytes).await?;
        self.writer
            .write_all(&chunk_index_offset.to_le_bytes())
            .await?;

        // --- Patch the 13-byte header at offset 0 ---
        let mut real_header = PackHeader::new(actual_count);
        real_header.kind = self.kind;
        let header_bytes = real_header.to_bytes();

        self.writer.seek(std::io::SeekFrom::Start(0)).await?;
        self.writer.write_all(&header_bytes).await?;

        // --- Stream-read the entire file to compute BLAKE3 ---
        self.writer.seek(std::io::SeekFrom::Start(0)).await?;
        self.writer.flush().await?;

        let mut hash_hasher = Hasher::new();
        let mut buf = vec![0u8; 64 * 1024];
        loop {
            let n = self.writer.read(&mut buf).await?;
            if n == 0 {
                break;
            }
            hash_hasher.update(&buf[..n]);
        }
        let checksum: [u8; 32] = hash_hasher.finalize();

        // Append checksum at end
        self.writer.seek(std::io::SeekFrom::End(0)).await?;
        self.writer.write_all(&checksum).await?;
        self.writer.flush().await?;

        // Total file size
        let byte_len = self.writer.seek(std::io::SeekFrom::End(0)).await?;

        // --- Build Vec<CloudChunkLoc> from index_bytes ---
        // Format: [count: u32 LE][OID 32B | offset u64 LE | length u32 LE] × count
        let mut cloud_index = Vec::with_capacity(actual_count as usize);
        if index_bytes.len() >= 4 {
            let count = u32::from_le_bytes([
                index_bytes[0],
                index_bytes[1],
                index_bytes[2],
                index_bytes[3],
            ]) as usize;
            let entries_data = &index_bytes[4..];
            const ENTRY_SIZE: usize = 44; // 32 OID + 8 offset + 4 length
            for i in 0..count {
                let base = i * ENTRY_SIZE;
                if base + ENTRY_SIZE > entries_data.len() {
                    break;
                }
                let mut oid_bytes = [0u8; 32];
                oid_bytes.copy_from_slice(&entries_data[base..base + 32]);
                let offset =
                    u64::from_le_bytes(entries_data[base + 32..base + 40].try_into().unwrap());
                let length =
                    u32::from_le_bytes(entries_data[base + 40..base + 44].try_into().unwrap());
                cloud_index.push(CloudChunkLoc {
                    chunk_oid: Oid::from_bytes(oid_bytes),
                    offset,
                    length,
                });
            }
        }

        // Persist the tempfile so the caller can upload it before dropping.
        let temp_path: PathBuf = match self.temp_named_file.take() {
            Some(named_tf) => {
                let path = named_tf.path().to_path_buf();
                // Prevent auto-delete: forget the NamedTempFile without running its destructor.
                std::mem::forget(named_tf);
                path
            }
            _ => {
                return Err(io::Error::other(
                    "Missing temp file handle in open-ended writer",
                ));
            }
        };

        debug!(
            objects_written = actual_count,
            chunk_index_offset = chunk_index_offset,
            byte_len = byte_len,
            "Cloud pack finalized"
        );

        Ok(CloudPackResult {
            pack_oid: checksum.to_vec(),
            byte_len,
            temp_path,
            index: cloud_index,
        })
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
            let writer = StreamingPackWriter::new(file, 0, temp_dir.path())
                .await
                .unwrap();
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
        let mut reader = StreamingPackReader::new(file).await.unwrap();

        assert_eq!(reader.objects_processed(), 0);
        assert!(reader.next_object().await.is_none());
    }

    #[tokio::test]
    async fn test_streaming_pack_single_object() {
        let temp_dir = TempDir::new().unwrap();
        let pack_path = temp_dir.path().join("test.pack");

        {
            let file = File::create(&pack_path).await.unwrap();
            let mut writer = StreamingPackWriter::new(file, 1, temp_dir.path())
                .await
                .unwrap();

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
        let mut reader = StreamingPackReader::new(file).await.unwrap();

        let (read_oid, read_type, read_data) = reader.next_object().await.unwrap().unwrap();
        let test_data = b"Hello, streaming world!";
        let oid = Oid::hash(test_data);

        assert_eq!(read_oid, oid);
        assert_eq!(read_type, ObjectType::Blob);
        assert_eq!(read_data, test_data);
        assert!(reader.next_object().await.is_none());
    }

    /// Helper: re-hash all bytes of a file except the trailing 32-byte BLAKE3 checksum.
    async fn hash_file_minus_checksum(path: &std::path::Path) -> Vec<u8> {
        let mut file = tokio::fs::File::open(path).await.unwrap();
        let meta = file.metadata().await.unwrap();
        let total = meta.len();
        assert!(total >= 32, "file too small to contain checksum");
        let to_hash = total - 32;

        let mut hasher = Hasher::new();
        let mut remaining = to_hash;
        let mut buf = vec![0u8; 64 * 1024];
        while remaining > 0 {
            let want = (remaining as usize).min(buf.len());
            let n = file.read(&mut buf[..want]).await.unwrap();
            assert!(n > 0);
            hasher.update(&buf[..n]);
            remaining -= n as u64;
        }
        hasher.finalize().to_vec()
    }

    #[tokio::test]
    async fn test_open_ended_round_trip_1_chunk() {
        let temp_dir = TempDir::new().unwrap();

        let mut writer =
            StreamingPackWriter::new_open_ended(PackKind::CloudObject, temp_dir.path())
                .await
                .unwrap();

        let data = b"single chunk data";
        let oid = Oid::hash(data);
        writer
            .write_object(oid, ObjectType::Blob, data)
            .await
            .unwrap();

        let result = writer.finalize_cloud().await.unwrap();

        // BLAKE3 is 32 bytes
        assert_eq!(result.pack_oid.len(), 32);

        // Verify stored BLAKE3 matches re-computed hash
        let expected_hash = hash_file_minus_checksum(&result.temp_path).await;
        assert_eq!(result.pack_oid, expected_hash, "BLAKE3 mismatch");

        // Index has exactly 1 entry with the correct OID
        assert_eq!(result.index.len(), 1);
        assert_eq!(result.index[0].chunk_oid, oid);

        // Cleanup
        let _ = std::fs::remove_file(&result.temp_path);
    }

    #[tokio::test]
    async fn test_open_ended_round_trip_128_chunks() {
        let temp_dir = TempDir::new().unwrap();

        let mut writer =
            StreamingPackWriter::new_open_ended(PackKind::CloudObject, temp_dir.path())
                .await
                .unwrap();

        let mut expected_oids = Vec::with_capacity(128);
        for i in 0u32..128 {
            let data = format!("chunk-{}", i);
            let oid = Oid::hash(data.as_bytes());
            expected_oids.push(oid);
            writer
                .write_object(oid, ObjectType::Blob, data.as_bytes())
                .await
                .unwrap();
        }

        let result = writer.finalize_cloud().await.unwrap();

        assert_eq!(result.pack_oid.len(), 32);

        // Verify BLAKE3
        let expected_hash = hash_file_minus_checksum(&result.temp_path).await;
        assert_eq!(result.pack_oid, expected_hash, "BLAKE3 mismatch");

        // All 128 entries present
        assert_eq!(result.index.len(), 128);

        // OIDs match insertion order (streaming index preserves insertion order)
        for (i, entry) in result.index.iter().enumerate() {
            assert_eq!(
                entry.chunk_oid, expected_oids[i],
                "OID mismatch at index {}",
                i
            );
        }

        // Offsets are strictly increasing
        for w in result.index.windows(2) {
            assert!(w[1].offset > w[0].offset, "offsets not strictly increasing");
        }

        // Cleanup
        let _ = std::fs::remove_file(&result.temp_path);
    }

    /// VC-7: nothing the writer accepts may overflow the 4-byte size field.
    ///
    /// Asserted on the bound itself rather than by writing a 4 GiB object,
    /// which is not a runnable test. Together with the guard in
    /// `write_object` this is what makes truncation unreachable: raise
    /// `MAX_PACK_OBJECT_SIZE` past `u32::MAX` and this fails.
    #[test]
    fn accepted_object_sizes_always_fit_the_u32_size_field() {
        assert!(
            MAX_PACK_OBJECT_SIZE <= u32::MAX as usize,
            "MAX_PACK_OBJECT_SIZE ({MAX_PACK_OBJECT_SIZE}) exceeds u32::MAX, so an \
                object passing the size guard would still truncate its header"
        );
    }

    #[test]
    fn oversized_pack_object_is_rejected_rather_than_truncated() {
        let oid = Oid::hash(b"vc7");

        ensure_writable_object_size(MAX_PACK_OBJECT_SIZE, &oid)
            .expect("an object at exactly the limit must be writable");

        let err = ensure_writable_object_size(MAX_PACK_OBJECT_SIZE + 1, &oid)
            .expect_err("an object over the limit must be refused");
        assert!(
            err.to_string().contains("pack object too large"),
            "unexpected error: {err}"
        );

        // The size that actually motivated the guard: 4 GiB wraps to 0 under
        // `as u32`, so the header would claim an empty object and every
        // following object in the pack would be read from the wrong offset.
        let four_gib = 4usize * 1024 * 1024 * 1024;
        assert_eq!(four_gib as u32, 0, "premise of the guard");
        assert!(ensure_writable_object_size(four_gib, &oid).is_err());
    }
}
