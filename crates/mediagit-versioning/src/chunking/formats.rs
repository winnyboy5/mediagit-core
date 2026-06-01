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

use super::*;

impl ContentChunker {
    /// AVI/RIFF format chunking.
    ///
    /// Walks the file at the RIFF-block level, handling both standard AVI 1.0
    /// (`RIFF/AVI `) and OpenDML AVI 2.0 (`RIFF/AVIX`) extension blocks.
    /// Inside each block, descends into `LIST/movi` and emits every video frame
    /// (`NNdc`/`NNdb`) and audio sample (`NNwb`) as its own `ContentChunk`.
    ///
    /// This allows deduplication of the video stream between files that share the
    /// same video encode but differ only in audio (e.g. stereo vs surround remux).
    pub(super) async fn chunk_avi(&self, data: &[u8]) -> Result<Vec<ContentChunk>> {
        let mut chunks = Vec::new();
        let mut offset = 0u64;

        if data.len() < 12 || &data[0..4] != b"RIFF" {
            debug!("Not a valid RIFF file, using fixed chunking");
            return self.chunk_fixed(data, 4 * 1024 * 1024).await;
        }

        // Walk at the top-level RIFF-block level.
        // AVI 1.0: one RIFF/AVI  block contains everything.
        // AVI 2.0: RIFF/AVI  (headers + first movi) followed by one or more
        //          RIFF/AVIX blocks each containing a LIST/movi continuation.
        let mut file_pos = 0;
        while file_pos + 12 <= data.len() {
            let fourcc = &data[file_pos..file_pos + 4];
            let block_size = u32::from_le_bytes([
                data[file_pos + 4],
                data[file_pos + 5],
                data[file_pos + 6],
                data[file_pos + 7],
            ]) as usize;
            let form_type = &data[file_pos + 8..file_pos + 12];
            let block_end = file_pos
                .saturating_add(8)
                .saturating_add(block_size)
                .min(data.len());

            if fourcc == b"RIFF" && (form_type == b"AVI " || form_type == b"AVIX") {
                // Emit the 12-byte RIFF block header as Metadata.
                let blk_hdr = &data[file_pos..file_pos + 12];
                chunks.push(ContentChunk {
                    id: Oid::hash(blk_hdr),
                    data: blk_hdr.to_vec(),
                    offset,
                    size: blk_hdr.len(),
                    chunk_type: ChunkType::Metadata,
                    perceptual_hash: None,
                    codec_hint: CodecHint::Unknown,
                });
                offset += blk_hdr.len() as u64;

                // Process inner chunks (hdrl, LIST/movi, idx1, …)
                self.parse_avi_block_chunks(
                    data,
                    file_pos + 12,
                    block_end,
                    &mut offset,
                    &mut chunks,
                )
                .await?;
            } else {
                // Unknown top-level block → store as-is
                let blk_data = &data[file_pos..block_end];
                chunks.push(ContentChunk {
                    id: Oid::hash(blk_data),
                    data: blk_data.to_vec(),
                    offset,
                    size: blk_data.len(),
                    chunk_type: ChunkType::Generic,
                    perceptual_hash: None,
                    codec_hint: CodecHint::Unknown,
                });
                offset += blk_data.len() as u64;
            }

            file_pos = block_end;
        }

        // Trailing bytes at end of file
        if file_pos < data.len() {
            let rem = &data[file_pos..];
            chunks.push(ContentChunk {
                id: Oid::hash(rem),
                data: rem.to_vec(),
                offset,
                size: rem.len(),
                chunk_type: ChunkType::Generic,
                perceptual_hash: None,
                codec_hint: CodecHint::Unknown,
            });
        }

        fill_coverage_gaps(data, &mut chunks);

        info!(
            chunks = chunks.len(),
            video_chunks = chunks
                .iter()
                .filter(|c| c.chunk_type == ChunkType::VideoStream)
                .count(),
            audio_chunks = chunks
                .iter()
                .filter(|c| c.chunk_type == ChunkType::AudioStream)
                .count(),
            "AVI chunking complete"
        );

        Ok(chunks)
    }

    /// Walk the inner chunks of a `RIFF/AVI ` or `RIFF/AVIX` block.
    /// Handles `LIST/movi` descent and emits structural chunks as Metadata.
    async fn parse_avi_block_chunks(
        &self,
        data: &[u8],
        start: usize,
        end: usize,
        offset: &mut u64,
        chunks: &mut Vec<ContentChunk>,
    ) -> Result<()> {
        let mut pos = start;
        while pos + 8 <= end {
            let fourcc = &data[pos..pos + 4];
            let chunk_size =
                u32::from_le_bytes([data[pos + 4], data[pos + 5], data[pos + 6], data[pos + 7]])
                    as usize;

            let data_end = pos.saturating_add(8).saturating_add(chunk_size).min(end);
            let needs_padding = !chunk_size.is_multiple_of(2) && data_end < end;
            let chunk_end = if needs_padding {
                data_end + 1
            } else {
                data_end
            }
            .min(end);

            if fourcc == b"LIST" && pos + 12 <= end {
                let list_type = &data[pos + 8..pos + 12];
                if list_type == b"movi" {
                    // Emit the 12-byte LIST/movi header as Metadata.
                    let list_hdr = &data[pos..pos + 12];
                    chunks.push(ContentChunk {
                        id: Oid::hash(list_hdr),
                        data: list_hdr.to_vec(),
                        offset: *offset,
                        size: list_hdr.len(),
                        chunk_type: ChunkType::Metadata,
                        perceptual_hash: None,
                        codec_hint: CodecHint::Unknown,
                    });
                    *offset += list_hdr.len() as u64;

                    // CDC sub-chunking for byte-exact reconstruction.
                    // Per-stream batching (parse_avi_movi_subchunks) is intentionally
                    // NOT used: it accumulates non-contiguous bytes causing size
                    // mismatch during chunk reconstruction.
                    let movi_content = &data[pos + 12..chunk_end];
                    let movi_offset = *offset;
                    if movi_content.len() > 4 * 1024 * 1024 {
                        let sub_chunks = self
                            .chunk_fastcdc(
                                movi_content,
                                2 * 1024 * 1024,
                                1024 * 1024,
                                8 * 1024 * 1024,
                            )
                            .await?;
                        for mut sub in sub_chunks {
                            sub.offset += movi_offset;
                            sub.chunk_type = ChunkType::VideoStream;
                            chunks.push(sub);
                        }
                    } else if !movi_content.is_empty() {
                        chunks.push(ContentChunk {
                            id: Oid::hash(movi_content),
                            data: movi_content.to_vec(),
                            offset: movi_offset,
                            size: movi_content.len(),
                            chunk_type: ChunkType::VideoStream,
                            perceptual_hash: None,
                            codec_hint: CodecHint::Unknown,
                        });
                    }
                    *offset += movi_content.len() as u64;
                } else {
                    // hdrl, INFO, strl, etc. → single Metadata chunk
                    let chunk_data = &data[pos..chunk_end];
                    chunks.push(ContentChunk {
                        id: Oid::hash(chunk_data),
                        data: chunk_data.to_vec(),
                        offset: *offset,
                        size: chunk_data.len(),
                        chunk_type: ChunkType::Metadata,
                        perceptual_hash: None,
                        codec_hint: CodecHint::Unknown,
                    });
                    *offset += chunk_data.len() as u64;
                }
            } else {
                let chunk_data = &data[pos..chunk_end];
                let chunk_type = match fourcc {
                    b"idx1" | b"JUNK" | b"IDIT" | b"indx" => ChunkType::Metadata,
                    _ => ChunkType::Generic,
                };
                chunks.push(ContentChunk {
                    id: Oid::hash(chunk_data),
                    data: chunk_data.to_vec(),
                    offset: *offset,
                    size: chunk_data.len(),
                    chunk_type,
                    perceptual_hash: None,
                    codec_hint: CodecHint::Unknown,
                });
                *offset += chunk_data.len() as u64;
            }

            pos = chunk_end;
        }

        // Trailing bytes inside this RIFF block
        if pos < end {
            let rem = &data[pos..end];
            chunks.push(ContentChunk {
                id: Oid::hash(rem),
                data: rem.to_vec(),
                offset: *offset,
                size: rem.len(),
                chunk_type: ChunkType::Generic,
                perceptual_hash: None,
                codec_hint: CodecHint::Unknown,
            });
            *offset += rem.len() as u64;
        }

        Ok(())
    }

    /// MP4/ISO BMFF chunking based on atom structure
    ///
    /// Parses MP4 atoms (ftyp, moov, mdat, etc.) and creates chunks at atom boundaries.
    /// For large mdat atoms, uses CDC sub-chunking. For moov, parses nested atoms.
    pub(super) async fn chunk_mp4(&self, data: &[u8]) -> Result<Vec<ContentChunk>> {
        // Validate MP4 signature (ftyp should be first atom)
        if data.len() < 8 || &data[4..8] != b"ftyp" {
            debug!("Not a valid MP4 file (no ftyp), using fixed chunking");
            return self.chunk_fixed(data, 4 * 1024 * 1024).await;
        }

        let atoms = parse_mp4_atoms(data);
        if atoms.is_empty() {
            debug!("Failed to parse MP4 atoms, using fixed chunking");
            return self.chunk_fixed(data, 4 * 1024 * 1024).await;
        }

        let mut chunks = Vec::new();

        for atom in atoms {
            let atom_start = atom.offset as usize;
            let atom_end = (atom.offset + atom.size) as usize;

            if atom_end > data.len() {
                // Atom size extends past EOF (common for MOV mdat with size=0 or
                // extended-size atoms).  Emit remaining bytes as a Generic chunk so
                // reconstruction integrity is preserved, then stop.
                let remaining = &data[atom_start..];
                if !remaining.is_empty() {
                    chunks.push(ContentChunk {
                        id: Oid::hash(remaining),
                        data: remaining.to_vec(),
                        offset: atom.offset,
                        size: remaining.len(),
                        chunk_type: ChunkType::Generic,
                        perceptual_hash: None,
                        codec_hint: CodecHint::Unknown,
                    });
                }
                debug!(
                    atom_type = %String::from_utf8_lossy(&atom.atom_type),
                    remaining = remaining.len(),
                    "Truncated atom — emitted remaining bytes as Generic chunk"
                );
                break;
            }

            let atom_data = &data[atom_start..atom_end];
            let atom_type_str = String::from_utf8_lossy(&atom.atom_type);

            match &atom.atom_type {
                b"ftyp" => {
                    // File type - always small, single chunk
                    chunks.push(ContentChunk {
                        id: Oid::hash(atom_data),
                        data: atom_data.to_vec(),
                        offset: atom.offset,
                        size: atom_data.len(),
                        chunk_type: ChunkType::Metadata,
                        perceptual_hash: None,
                        codec_hint: CodecHint::Unknown,
                    });
                    debug!(atom_type = %atom_type_str, size = atom.size, "Parsed ftyp atom");
                }
                b"moov" => {
                    // Movie metadata container - parse nested atoms for finer deduplication
                    if atom.size > 8 {
                        let nested = parse_mp4_atoms(&atom_data[8..]); // Skip moov header

                        if nested.is_empty() {
                            // Fallback: keep as single chunk
                            chunks.push(ContentChunk {
                                id: Oid::hash(atom_data),
                                data: atom_data.to_vec(),
                                offset: atom.offset,
                                size: atom_data.len(),
                                chunk_type: ChunkType::Metadata,
                                perceptual_hash: None,
                                codec_hint: CodecHint::Unknown,
                            });
                        } else {
                            // Emit moov header (8 bytes) as separate chunk
                            let header = &atom_data[..8];
                            chunks.push(ContentChunk {
                                id: Oid::hash(header),
                                data: header.to_vec(),
                                offset: atom.offset,
                                size: 8,
                                chunk_type: ChunkType::Metadata,
                                perceptual_hash: None,
                                codec_hint: CodecHint::Unknown,
                            });

                            // Emit each nested atom as separate chunk.
                            // Clamp nested_end to atom_data.len() so that a nested
                            // atom whose declared size extends past the moov boundary
                            // still gets emitted rather than being silently dropped.
                            for nested_atom in &nested {
                                let nested_start = 8 + nested_atom.offset as usize;
                                let nested_end =
                                    (nested_start + nested_atom.size as usize).min(atom_data.len());
                                if nested_start < atom_data.len() {
                                    let nested_data = &atom_data[nested_start..nested_end];
                                    if !nested_data.is_empty() {
                                        let nested_type =
                                            String::from_utf8_lossy(&nested_atom.atom_type);
                                        chunks.push(ContentChunk {
                                            id: Oid::hash(nested_data),
                                            data: nested_data.to_vec(),
                                            offset: atom.offset + 8 + nested_atom.offset,
                                            size: nested_data.len(),
                                            chunk_type: ChunkType::Metadata,
                                            perceptual_hash: None,
                                            codec_hint: CodecHint::Unknown,
                                        });
                                        debug!(
                                            atom_type = %nested_type,
                                            size = nested_atom.size,
                                            "Parsed nested moov atom"
                                        );
                                    }
                                }
                            }
                        }
                    } else {
                        chunks.push(ContentChunk {
                            id: Oid::hash(atom_data),
                            data: atom_data.to_vec(),
                            offset: atom.offset,
                            size: atom_data.len(),
                            chunk_type: ChunkType::Metadata,
                            perceptual_hash: None,
                            codec_hint: CodecHint::Unknown,
                        });
                    }
                    debug!(atom_type = %atom_type_str, size = atom.size, "Parsed moov atom");
                }
                b"mdat" => {
                    // Media data — CDC sub-chunking for byte-exact reconstruction.
                    // Track-aware per-stream batching is intentionally NOT used here:
                    // it accumulates non-contiguous sample bytes into a single chunk,
                    // which causes reconstruction to produce the wrong total size when
                    // paired with gap-filling logic.  CDC produces contiguous, ordered
                    // chunks that always reconstruct the exact original bytes.
                    if atom.size > 4 * 1024 * 1024 {
                        // Always emit the mdat header as a Metadata chunk
                        let header = &atom_data[..atom.header_size as usize];
                        chunks.push(ContentChunk {
                            id: Oid::hash(header),
                            data: header.to_vec(),
                            offset: atom.offset,
                            size: header.len(),
                            chunk_type: ChunkType::Metadata,
                            perceptual_hash: None,
                            codec_hint: CodecHint::Unknown,
                        });

                        let mdat_content = &atom_data[atom.header_size as usize..];
                        let mdat_content_offset = atom.offset + atom.header_size as u64;

                        let sub_chunks = self
                            .chunk_fastcdc(
                                mdat_content,
                                2 * 1024 * 1024, // 2MB average (video-optimised)
                                1024 * 1024,     // 1MB minimum
                                8 * 1024 * 1024, // 8MB maximum
                            )
                            .await?;

                        let before = chunks.len();
                        for mut sub in sub_chunks {
                            sub.offset += mdat_content_offset;
                            sub.chunk_type = ChunkType::VideoStream;
                            chunks.push(sub);
                        }
                        debug!(
                            atom_type = %atom_type_str,
                            size = atom.size,
                            sub_chunks = chunks.len() - before,
                            "Parsed large mdat with CDC sub-chunking"
                        );
                    } else {
                        // Small mdat: single chunk
                        chunks.push(ContentChunk {
                            id: Oid::hash(atom_data),
                            data: atom_data.to_vec(),
                            offset: atom.offset,
                            size: atom_data.len(),
                            chunk_type: ChunkType::VideoStream,
                            perceptual_hash: None,
                            codec_hint: CodecHint::Unknown,
                        });
                        debug!(atom_type = %atom_type_str, size = atom.size, "Parsed small mdat atom");
                    }
                }
                b"moof" => {
                    // Movie fragment (fMP4) - treat as metadata
                    chunks.push(ContentChunk {
                        id: Oid::hash(atom_data),
                        data: atom_data.to_vec(),
                        offset: atom.offset,
                        size: atom_data.len(),
                        chunk_type: ChunkType::Metadata,
                        perceptual_hash: None,
                        codec_hint: CodecHint::Unknown,
                    });
                    debug!(atom_type = %atom_type_str, size = atom.size, "Parsed moof atom");
                }
                b"free" | b"skip" | b"wide" => {
                    // Free space / padding - still chunk it for reconstruction
                    chunks.push(ContentChunk {
                        id: Oid::hash(atom_data),
                        data: atom_data.to_vec(),
                        offset: atom.offset,
                        size: atom_data.len(),
                        chunk_type: ChunkType::Generic,
                        perceptual_hash: None,
                        codec_hint: CodecHint::Unknown,
                    });
                    debug!(atom_type = %atom_type_str, size = atom.size, "Parsed padding atom");
                }
                _ => {
                    // Other atoms: single chunk as generic
                    chunks.push(ContentChunk {
                        id: Oid::hash(atom_data),
                        data: atom_data.to_vec(),
                        offset: atom.offset,
                        size: atom_data.len(),
                        chunk_type: ChunkType::Generic,
                        perceptual_hash: None,
                        codec_hint: CodecHint::Unknown,
                    });
                    debug!(atom_type = %atom_type_str, size = atom.size, "Parsed unknown atom");
                }
            }
        }

        fill_coverage_gaps(data, &mut chunks);

        info!(
            chunks = chunks.len(),
            metadata_chunks = chunks
                .iter()
                .filter(|c| c.chunk_type == ChunkType::Metadata)
                .count(),
            video_chunks = chunks
                .iter()
                .filter(|c| c.chunk_type == ChunkType::VideoStream)
                .count(),
            total_size = data.len(),
            "MP4 atom-based chunking complete"
        );

        Ok(chunks)
    }

    /// Matroska/WebM chunking
    ///
    /// Parses EBML elements and creates content-aware chunks:
    /// - Each metadata element (Info, Tracks, Tags, Cues, etc.) gets its own chunk
    ///   for granular dedup (changing tags won't invalidate tracks)
    /// - Segment container emits header-only; children get individual chunks
    /// - Clusters > 4MB are CDC-subdivided (1MB avg) as VideoStream chunks
    /// - Small Clusters become single VideoStream chunks
    /// - Attachments become separate Generic chunks
    pub(super) async fn chunk_matroska(&self, data: &[u8]) -> Result<Vec<ContentChunk>> {
        // LEVEL 1: Parse EBML elements
        let elements = parse_ebml_elements(data);

        if elements.is_empty() {
            warn!("No valid EBML elements found, falling back to fixed chunking");
            return self.chunk_fixed(data, 4 * 1024 * 1024).await;
        }

        // LEVEL 2: Validate EBML header exists
        if !elements.iter().any(|e| e.id == EBML_ID) {
            warn!("EBML header not found, not a valid Matroska file");
            return self.chunk_fixed(data, 4 * 1024 * 1024).await;
        }
        let mut chunks = Vec::new();

        for element in &elements {
            let elem_start = element.offset as usize;
            let elem_size = if element.data_size == u64::MAX {
                // Unknown size: extends to end of data (from element start, including header)
                data.len().saturating_sub(elem_start)
            } else {
                element.header_size as usize + element.data_size as usize
            };
            let elem_end = (elem_start + elem_size).min(data.len());

            if elem_end <= elem_start {
                continue;
            }

            match element.id {
                CLUSTER_ID => {
                    let cluster_data = &data[elem_start..elem_end];
                    let header_size = element.header_size as usize;
                    let cluster_content_start = header_size;
                    let base_offset = element.offset + header_size as u64;

                    // Always emit the Cluster header as a Metadata chunk —
                    // it contains the Timecode which marks the start of the cluster.
                    let header = &cluster_data[..header_size.min(cluster_data.len())];
                    chunks.push(ContentChunk {
                        id: Oid::hash(header),
                        data: header.to_vec(),
                        offset: element.offset,
                        size: header.len(),
                        chunk_type: ChunkType::Metadata,
                        perceptual_hash: None,
                        codec_hint: CodecHint::Unknown,
                    });

                    if cluster_data.len() <= cluster_content_start {
                        continue;
                    }
                    let cluster_content = &cluster_data[cluster_content_start..];

                    // CDC sub-chunking for byte-exact reconstruction.
                    // Per-stream batching (chunk_cluster_by_tracks) is intentionally
                    // NOT used: it accumulates non-contiguous bytes causing size
                    // mismatch during chunk reconstruction.
                    if cluster_content.len() > 4 * 1024 * 1024 {
                        let sub_chunks = self
                            .chunk_fastcdc(
                                cluster_content,
                                2 * 1024 * 1024,
                                1024 * 1024,
                                8 * 1024 * 1024,
                            )
                            .await?;

                        for mut sub in sub_chunks {
                            sub.offset += base_offset;
                            sub.chunk_type = ChunkType::VideoStream;
                            chunks.push(sub);
                        }
                    } else {
                        chunks.push(ContentChunk {
                            id: Oid::hash(cluster_content),
                            data: cluster_content.to_vec(),
                            offset: base_offset,
                            size: cluster_content.len(),
                            chunk_type: ChunkType::VideoStream,
                            perceptual_hash: None,
                            codec_hint: CodecHint::Unknown,
                        });
                    }
                }
                ATTACHMENTS_ID => {
                    // Attachments as separate Generic chunk
                    let attach_data = &data[elem_start..elem_end];
                    chunks.push(ContentChunk {
                        id: Oid::hash(attach_data),
                        data: attach_data.to_vec(),
                        offset: element.offset,
                        size: attach_data.len(),
                        chunk_type: ChunkType::Generic,
                        perceptual_hash: None,
                        codec_hint: CodecHint::Unknown,
                    });
                }
                // Segment is a container — emit its header as a small metadata chunk
                // so child elements get their own chunks
                SEGMENT_ID => {
                    let header_size = element.header_size as usize;
                    let header = &data[elem_start..elem_start + header_size];
                    chunks.push(ContentChunk {
                        id: Oid::hash(header),
                        data: header.to_vec(),
                        offset: element.offset,
                        size: header.len(),
                        chunk_type: ChunkType::Metadata,
                        perceptual_hash: None,
                        codec_hint: CodecHint::Unknown,
                    });
                }
                // Each metadata element gets its own chunk for granular dedup:
                // changing Tags won't invalidate Tracks, editing Chapters won't
                // invalidate Cues, etc.
                EBML_ID | SEEKHEAD_ID | INFO_ID | TRACKS_ID | CUES_ID | CHAPTERS_ID | TAGS_ID => {
                    let elem_data = &data[elem_start..elem_end];
                    chunks.push(ContentChunk {
                        id: Oid::hash(elem_data),
                        data: elem_data.to_vec(),
                        offset: element.offset,
                        size: elem_data.len(),
                        chunk_type: ChunkType::Metadata,
                        perceptual_hash: None,
                        codec_hint: CodecHint::Unknown,
                    });
                }
                _ => {
                    // Unknown elements: emit as individual metadata chunks
                    let elem_data = &data[elem_start..elem_end];
                    chunks.push(ContentChunk {
                        id: Oid::hash(elem_data),
                        data: elem_data.to_vec(),
                        offset: element.offset,
                        size: elem_data.len(),
                        chunk_type: ChunkType::Metadata,
                        perceptual_hash: None,
                        codec_hint: CodecHint::Unknown,
                    });
                }
            }
        }

        // LEVEL 3: Ensure we produced chunks
        if chunks.is_empty() {
            warn!("No chunks created from Matroska parsing, falling back");
            return self.chunk_fixed(data, 4 * 1024 * 1024).await;
        }

        fill_coverage_gaps(data, &mut chunks);

        info!(
            chunks = chunks.len(),
            metadata = chunks
                .iter()
                .filter(|c| c.chunk_type == ChunkType::Metadata)
                .count(),
            clusters = chunks
                .iter()
                .filter(|c| c.chunk_type == ChunkType::VideoStream)
                .count(),
            total_size = data.len(),
            "Matroska EBML chunking complete"
        );

        Ok(chunks)
    }

    /// GLB (binary glTF) format chunking
    ///
    /// GLB structure:
    /// - 12-byte header: magic (4) + version (4) + total length (4)
    /// - JSON chunk: length (4) + type "JSON" (4) + JSON data
    /// - Binary chunk: length (4) + type "BIN\0" (4) + binary data
    pub(super) async fn chunk_glb(&self, data: &[u8]) -> Result<Vec<ContentChunk>> {
        let mut chunks = Vec::new();

        // GLB magic: "glTF" (0x46546C67)
        const GLB_MAGIC: &[u8] = b"glTF";
        const JSON_CHUNK_TYPE: u32 = 0x4E4F534A; // "JSON"
        const BIN_CHUNK_TYPE: u32 = 0x004E4942; // "BIN\0"

        // Validate GLB header
        if data.len() < 12 || &data[0..4] != GLB_MAGIC {
            debug!("Not a valid GLB file, using fixed chunking");
            return self.chunk_fixed(data, 4 * 1024 * 1024).await;
        }

        let version = u32::from_le_bytes([data[4], data[5], data[6], data[7]]);
        let _total_length = u32::from_le_bytes([data[8], data[9], data[10], data[11]]) as usize;

        if version != 2 {
            debug!(version, "Unsupported GLB version, using fixed chunking");
            return self.chunk_fixed(data, 4 * 1024 * 1024).await;
        }

        // Header chunk (12 bytes)
        let header_data = &data[0..12];
        chunks.push(ContentChunk {
            id: Oid::hash(header_data),
            data: header_data.to_vec(),
            offset: 0,
            size: 12,
            chunk_type: ChunkType::Metadata,
            perceptual_hash: None,
            codec_hint: CodecHint::Unknown,
        });

        let mut pos = 12;

        // Parse JSON and BIN chunks
        while pos + 8 <= data.len() {
            let chunk_length =
                u32::from_le_bytes([data[pos], data[pos + 1], data[pos + 2], data[pos + 3]])
                    as usize;
            let chunk_type =
                u32::from_le_bytes([data[pos + 4], data[pos + 5], data[pos + 6], data[pos + 7]]);

            let chunk_start = pos;
            let chunk_data_start = pos + 8;
            let chunk_end = (chunk_data_start + chunk_length).min(data.len());

            // Include chunk header (8 bytes) as a tiny metadata separator so that
            // the BIN data start offset is stable even when sub-chunked.
            let header_bytes = &data[chunk_start..chunk_start + 8];
            let (ct, label) = match chunk_type {
                JSON_CHUNK_TYPE => (ChunkType::Metadata, "JSON"),
                BIN_CHUNK_TYPE => (ChunkType::Generic, "BIN"),
                _ => (ChunkType::Generic, "Unknown"),
            };

            // Emit the 8-byte chunk header always as a metadata marker.
            chunks.push(ContentChunk {
                id: Oid::hash(header_bytes),
                data: header_bytes.to_vec(),
                offset: chunk_start as u64,
                size: 8,
                chunk_type: ChunkType::Metadata,
                perceptual_hash: None,
                codec_hint: CodecHint::Unknown,
            });

            let full_chunk_data = &data[chunk_data_start..chunk_end];

            // BIN payloads > 4 MB: CDC-subdivide for better dedup granularity.
            // Matches the MKV large-Cluster strategy (1 MB avg / 512 KB min / 4 MB max).
            // GLB BIN buffers store vertex arrays, textures, and animation data. Large
            // buffers are common (scanned meshes, terrain, photogrammetry) and often share
            // sub-regions across model revisions.
            const GLB_BIN_SUBCHUNK_THRESHOLD: usize = 4 * 1024 * 1024; // 4 MB
            if chunk_type == BIN_CHUNK_TYPE && full_chunk_data.len() > GLB_BIN_SUBCHUNK_THRESHOLD {
                use fastcdc::v2020::FastCDC;
                let avg: u32 = 1024 * 1024; // 1 MB
                let min: u32 = 512 * 1024; // 512 KB
                let max: u32 = 4 * 1024 * 1024; // 4 MB
                let cdc = FastCDC::new(full_chunk_data, min, avg, max);
                let base_offset = chunk_data_start as u64;
                for entry in cdc {
                    let sub = &full_chunk_data[entry.offset..entry.offset + entry.length];
                    chunks.push(ContentChunk {
                        id: Oid::hash(sub),
                        data: sub.to_vec(),
                        offset: base_offset + entry.offset as u64,
                        size: entry.length,
                        chunk_type: ChunkType::Generic,
                        perceptual_hash: None,
                        codec_hint: CodecHint::Unknown,
                    });
                }
                debug!(
                    chunk_type = label,
                    offset = chunk_start,
                    size = full_chunk_data.len(),
                    "GLB BIN chunk (sub-chunked via FastCDC)"
                );
            } else {
                chunks.push(ContentChunk {
                    id: Oid::hash(full_chunk_data),
                    data: full_chunk_data.to_vec(),
                    offset: chunk_data_start as u64,
                    size: full_chunk_data.len(),
                    chunk_type: ct,
                    perceptual_hash: None,
                    codec_hint: CodecHint::Unknown,
                });
                debug!(
                    chunk_type = label,
                    offset = chunk_start,
                    size = full_chunk_data.len(),
                    "GLB chunk"
                );
            }

            pos = chunk_end;
        }

        // Handle remaining data if any
        if pos < data.len() {
            let remaining = &data[pos..];
            chunks.push(ContentChunk {
                id: Oid::hash(remaining),
                data: remaining.to_vec(),
                offset: pos as u64,
                size: remaining.len(),
                chunk_type: ChunkType::Generic,
                perceptual_hash: None,
                codec_hint: CodecHint::Unknown,
            });
        }

        if chunks.is_empty() {
            warn!("No chunks created from GLB parsing, falling back");
            return self.chunk_fixed(data, 4 * 1024 * 1024).await;
        }

        info!(
            chunks = chunks.len(),
            total_size = data.len(),
            "GLB chunking complete"
        );

        Ok(chunks)
    }

    /// Text-based 3D model chunking (OBJ, STL ASCII, PLY ASCII)
    ///
    /// These formats are line-oriented text files that benefit from
    /// structure-aware chunking at logical boundaries:
    /// - OBJ: groups (g), objects (o), material uses (usemtl)
    /// - STL: facet boundaries
    /// - PLY: header vs data sections
    pub(super) async fn chunk_3d_text(&self, data: &[u8]) -> Result<Vec<ContentChunk>> {
        // Check if data is valid UTF-8 text
        if !data
            .iter()
            .take(1024)
            .all(|&b| (..128).contains(&b) || b >= 0xC0)
        {
            // Binary format - use rolling CDC
            let (avg, min, max) = get_chunk_params(data.len() as u64);
            return self.chunk_fastcdc(data, avg, min, max).await;
        }

        let mut chunks = Vec::new();
        let mut chunk_start = 0;
        let min_chunk_size = 256 * 1024; // 256KB minimum chunk
        let max_chunk_size = 4 * 1024 * 1024; // 4MB maximum

        // Parse as text, split at logical boundaries
        let text = String::from_utf8_lossy(data);
        let lines: Vec<&str> = text.lines().collect();
        let mut current_pos = 0;

        for line in lines.iter() {
            let line_len = line.len() + 1; // +1 for newline
            let chunk_size = current_pos - chunk_start;

            // Check for logical boundaries (OBJ groups/objects, STL facets)
            let is_boundary = line.starts_with("g ")
                || line.starts_with("o ")
                || line.starts_with("usemtl ")
                || line.starts_with("facet ")
                || line.starts_with("end_header");

            // Create chunk at boundary if size is acceptable
            if is_boundary && chunk_size >= min_chunk_size {
                let chunk_data = &data[chunk_start..current_pos];
                chunks.push(ContentChunk {
                    id: Oid::hash(chunk_data),
                    data: chunk_data.to_vec(),
                    offset: chunk_start as u64,
                    size: chunk_data.len(),
                    chunk_type: ChunkType::Generic,
                    perceptual_hash: None,
                    codec_hint: CodecHint::Unknown,
                });
                chunk_start = current_pos;
            }

            // Force chunk if we exceed max size
            if chunk_size >= max_chunk_size {
                let chunk_data = &data[chunk_start..current_pos];
                chunks.push(ContentChunk {
                    id: Oid::hash(chunk_data),
                    data: chunk_data.to_vec(),
                    offset: chunk_start as u64,
                    size: chunk_data.len(),
                    chunk_type: ChunkType::Generic,
                    perceptual_hash: None,
                    codec_hint: CodecHint::Unknown,
                });
                chunk_start = current_pos;
            }

            current_pos += line_len;

            // Prevent going past data length due to line ending differences
            if current_pos > data.len() {
                current_pos = data.len();
            }
        }

        // Final chunk
        if chunk_start < data.len() {
            let chunk_data = &data[chunk_start..];
            chunks.push(ContentChunk {
                id: Oid::hash(chunk_data),
                data: chunk_data.to_vec(),
                offset: chunk_start as u64,
                size: chunk_data.len(),
                chunk_type: ChunkType::Generic,
                perceptual_hash: None,
                codec_hint: CodecHint::Unknown,
            });
        }

        // Fallback if no chunks created
        if chunks.is_empty() {
            return self.chunk_fixed(data, 4 * 1024 * 1024).await;
        }

        info!(
            chunks = chunks.len(),
            total_size = data.len(),
            "3D text model chunking complete"
        );

        Ok(chunks)
    }

    /// FBX binary format chunking
    ///
    /// FBX binary files have a node-based structure that can be parsed
    /// for structure-aware chunking. Falls back to CDC for ASCII FBX.
    pub(super) async fn chunk_fbx(&self, data: &[u8]) -> Result<Vec<ContentChunk>> {
        // FBX binary magic: "Kaydara FBX Binary  \x00"
        const FBX_MAGIC: &[u8] = b"Kaydara FBX Binary  \x00";

        if data.len() < 27 || &data[0..21] != FBX_MAGIC {
            // ASCII FBX or invalid - use rolling CDC
            debug!("FBX file is ASCII or invalid, using rolling CDC");
            let (avg, min, max) = get_chunk_params(data.len() as u64);
            return self.chunk_fastcdc(data, avg, min, max).await;
        }

        // Parse FBX version (bytes 23-26, little-endian)
        let _version = u32::from_le_bytes([data[23], data[24], data[25], data[26]]);

        let mut chunks = Vec::new();

        // Header chunk (first 27 bytes)
        let header = &data[0..27];
        chunks.push(ContentChunk {
            id: Oid::hash(header),
            data: header.to_vec(),
            offset: 0,
            size: 27,
            chunk_type: ChunkType::Metadata,
            perceptual_hash: None,
            codec_hint: CodecHint::Unknown,
        });

        // For FBX, use adaptive rolling CDC on the rest of the data
        // Full FBX node parsing is complex; CDC provides good dedup
        if data.len() > 27 {
            let content = &data[27..];
            let (avg, min, max) = get_chunk_params(content.len() as u64);
            let sub_chunks = self.chunk_fastcdc(content, avg, min, max).await?;

            for mut chunk in sub_chunks {
                chunk.offset += 27;
                chunks.push(chunk);
            }
        }

        info!(
            chunks = chunks.len(),
            total_size = data.len(),
            "FBX chunking complete"
        );

        Ok(chunks)
    }
}

/// Patch any uncovered byte ranges with Generic chunks.
///
/// Format-aware parsers can miss bytes due to EBML padding, atom-size edge
/// cases, Void elements, CRC-32 elements, or other format quirks.  This
/// helper sorts chunks by offset, finds uncovered ranges, and emits a Generic
/// chunk for each gap so that concatenating all chunks reconstructs the exact
/// original file.
///
/// Safe to call when chunks come from CDC (`chunk_fastcdc`) because CDC always
/// produces contiguous, non-overlapping slices — no duplication risk.
pub(super) fn fill_coverage_gaps(data: &[u8], chunks: &mut Vec<ContentChunk>) {
    if data.is_empty() {
        return;
    }
    chunks.sort_unstable_by_key(|c| c.offset);

    let mut gap_chunks: Vec<ContentChunk> = Vec::new();
    let mut covered_up_to: u64 = 0;

    for chunk in chunks.iter() {
        if chunk.offset > covered_up_to {
            let gap_start = covered_up_to as usize;
            let gap_end = chunk.offset as usize;
            if gap_end <= data.len() && gap_start < gap_end {
                let gap_data = &data[gap_start..gap_end];
                gap_chunks.push(ContentChunk {
                    id: Oid::hash(gap_data),
                    data: gap_data.to_vec(),
                    offset: covered_up_to,
                    size: gap_data.len(),
                    chunk_type: ChunkType::Generic,
                    perceptual_hash: None,
                    codec_hint: CodecHint::Unknown,
                });
            }
        }
        let chunk_end = chunk.offset + chunk.size as u64;
        if chunk_end > covered_up_to {
            covered_up_to = chunk_end;
        }
    }

    if (covered_up_to as usize) < data.len() {
        let trail = &data[covered_up_to as usize..];
        gap_chunks.push(ContentChunk {
            id: Oid::hash(trail),
            data: trail.to_vec(),
            offset: covered_up_to,
            size: trail.len(),
            chunk_type: ChunkType::Generic,
            perceptual_hash: None,
            codec_hint: CodecHint::Unknown,
        });
    }

    if !gap_chunks.is_empty() {
        warn!(
            gaps = gap_chunks.len(),
            total_gap_bytes = gap_chunks.iter().map(|c| c.size).sum::<usize>(),
            "Coverage gaps patched with Generic chunks"
        );
        chunks.extend(gap_chunks);
        chunks.sort_unstable_by_key(|c| c.offset);
    }
}

pub(super) fn parse_mp4_atoms(data: &[u8]) -> Vec<Mp4Atom> {
    let mut atoms = Vec::new();
    let mut pos = 0u64;
    let data_len = data.len() as u64;

    while pos + 8 <= data_len {
        let offset = pos as usize;

        // Read size (4 bytes, big-endian)
        let size = u32::from_be_bytes([
            data[offset],
            data[offset + 1],
            data[offset + 2],
            data[offset + 3],
        ]) as u64;

        // Read type (4 bytes FourCC)
        let mut atom_type = [0u8; 4];
        atom_type.copy_from_slice(&data[offset + 4..offset + 8]);

        // Validate atom type (should be printable ASCII or known types)
        let is_valid_type = atom_type
            .iter()
            .all(|&b| (0x20..=0x7E).contains(&b) || b == 0x00);
        if !is_valid_type {
            // Invalid atom type, stop parsing
            break;
        }

        let (actual_size, header_size): (u64, u8) = match size {
            0 => {
                // Size 0 = extends to EOF
                (data_len - pos, 8)
            }
            1 => {
                // Extended size (8-byte size after type)
                if offset + 16 > data.len() {
                    break;
                }
                let ext_size = u64::from_be_bytes([
                    data[offset + 8],
                    data[offset + 9],
                    data[offset + 10],
                    data[offset + 11],
                    data[offset + 12],
                    data[offset + 13],
                    data[offset + 14],
                    data[offset + 15],
                ]);
                (ext_size, 16)
            }
            _ => (size, 8),
        };

        // Sanity check: atom shouldn't extend beyond data
        if pos + actual_size > data_len {
            // Truncated atom - include what we have
            atoms.push(Mp4Atom {
                atom_type,
                offset: pos,
                size: data_len - pos,
                header_size,
            });
            break;
        }

        atoms.push(Mp4Atom {
            atom_type,
            offset: pos,
            size: actual_size,
            header_size,
        });

        pos += actual_size;
    }

    atoms
}

/// Read EBML VINT for Element ID (marker bit KEPT in value)
///
/// Returns (id, bytes_consumed) or None if invalid.
/// Element IDs include the VINT marker bit as part of the value.
pub(super) fn read_ebml_id(data: &[u8], pos: usize) -> Option<(u32, u8)> {
    if pos >= data.len() {
        return None;
    }

    let first = data[pos];
    if first == 0 {
        return None; // Invalid: leading zeros not allowed in shortest form
    }

    // Count leading zeros to determine byte count
    let len = (first.leading_zeros() + 1) as usize;
    if len > 4 || pos + len > data.len() {
        return None; // Matroska limits IDs to 4 bytes
    }

    // Build ID value (marker bit is kept)
    let mut id = first as u32;
    for i in 1..len {
        id = (id << 8) | data[pos + i] as u32;
    }

    Some((id, len as u8))
}

/// Read EBML VINT for Element Size (marker bit REMOVED from value)
///
/// Returns (size, bytes_consumed) or None if invalid.
/// Size u64::MAX indicates "unknown size" (all data bits = 1).
pub(super) fn read_ebml_size(data: &[u8], pos: usize) -> Option<(u64, u8)> {
    if pos >= data.len() {
        return None;
    }

    let first = data[pos];
    if first == 0 {
        return None; // Invalid: all zeros
    }

    // Count leading zeros to determine byte count
    let len = (first.leading_zeros() + 1) as usize;
    if len > 8 || pos + len > data.len() {
        return None;
    }

    // Clear marker bit and build size value
    // When len==8 (leading_zeros==7), the entire first byte is header — no data bits.
    let mask = if len >= 8 { 0u8 } else { 0xFFu8 >> len };
    let mut size = (first & mask) as u64;
    for i in 1..len {
        size = (size << 8) | data[pos + i] as u64;
    }

    // Check for "unknown size" (all data bits = 1)
    // For 1-byte: 0x7F, 2-byte: 0x3FFF, etc.
    let unknown_marker = (1u64 << (7 * len)) - 1;
    if size == unknown_marker {
        return Some((u64::MAX, len as u8));
    }

    Some((size, len as u8))
}

/// Parse EBML elements from Matroska/WebM data
///
/// Returns a vector of EbmlElement headers. Enters Segment containers
/// to parse their children (Clusters, Info, Tracks, etc.).
/// Stops on unknown size elements or parse errors.
pub(super) fn parse_ebml_elements(data: &[u8]) -> Vec<EbmlElement> {
    let mut elements = Vec::new();
    let mut pos = 0usize;
    let data_len = data.len();

    while pos + 2 <= data_len {
        // Read Element ID
        let (id, id_len) = match read_ebml_id(data, pos) {
            Some(v) => v,
            None => break,
        };

        // Read Element Size
        let (size, size_len) = match read_ebml_size(data, pos + id_len as usize) {
            Some(v) => v,
            None => break,
        };

        let header_size = id_len + size_len;

        // Skip Void and CRC-32 elements (padding/checksum)
        if id == VOID_ID || id == CRC32_ID {
            if size != u64::MAX {
                pos += header_size as usize + size as usize;
            } else {
                break; // Cannot skip unknown size
            }
            continue;
        }

        let element = EbmlElement {
            id,
            offset: pos as u64,
            header_size,
            data_size: size,
        };
        elements.push(element);

        // Handle Segment: parse its children, don't skip over it
        if id == SEGMENT_ID {
            pos += header_size as usize; // Enter Segment
        } else if size == u64::MAX {
            // Unknown size: can't skip, stop parsing
            break;
        } else {
            // Skip to next element
            let next_pos = pos + header_size as usize + size as usize;
            if next_pos > data_len {
                break; // Element extends beyond data
            }
            pos = next_pos;
        }
    }

    elements
}
