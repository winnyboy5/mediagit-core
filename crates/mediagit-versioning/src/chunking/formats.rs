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

        // Pre-pass: scan hdrl→strl→strh/strf (a few KB of metadata) for the
        // dominant codec of the movi payload. Never touches movi bytes.
        let dominant_hint = if codec_detect_enabled() {
            avi_dominant_codec_hint(data)
        } else {
            CodecHint::Unknown
        };

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
                    dominant_hint,
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
        dominant_hint: CodecHint,
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
                        codec_hint: dominant_hint,
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
                            sub.codec_hint = dominant_hint;
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
                            codec_hint: dominant_hint,
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

        // Pre-pass: scan moov→trak→mdia→minf→stbl→stsd for sample-entry FourCCs
        // (a few KB of metadata) to determine the dominant codec for the mdat
        // payload. Never touches mdat itself.
        let dominant_hint = if codec_detect_enabled() {
            atoms
                .iter()
                .find(|a| &a.atom_type == b"moov")
                .and_then(|moov| {
                    let start = (moov.offset as usize).checked_add(8)?;
                    let end = (moov.offset as usize).checked_add(moov.size as usize)?;
                    if start <= end && end <= data.len() {
                        let hints: Vec<CodecHint> = mp4_stsd_fourccs(&data[start..end])
                            .iter()
                            .map(mp4_fourcc_to_codec_hint)
                            .collect();
                        Some(dominant_codec_hint(&hints))
                    } else {
                        None
                    }
                })
                .unwrap_or(CodecHint::Unknown)
        } else {
            CodecHint::Unknown
        };

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
                            codec_hint: dominant_hint,
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
                            sub.codec_hint = dominant_hint;
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
                            codec_hint: dominant_hint,
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
        // Pre-pass: scan the Tracks element (a few KB of metadata) for each
        // TrackEntry's CodecID to determine the dominant codec for Cluster
        // payloads. Never touches Cluster bytes.
        let dominant_hint = if codec_detect_enabled() {
            elements
                .iter()
                .find(|e| e.id == TRACKS_ID)
                .and_then(|tracks_elem| {
                    let start = tracks_elem.offset as usize + tracks_elem.header_size as usize;
                    let end = if tracks_elem.data_size == u64::MAX {
                        data.len()
                    } else {
                        (tracks_elem.offset as usize
                            + tracks_elem.header_size as usize
                            + tracks_elem.data_size as usize)
                            .min(data.len())
                    };
                    if start < end {
                        let hints: Vec<CodecHint> = mkv_track_codec_ids(&data[start..end])
                            .iter()
                            .map(|s| mkv_codec_id_to_hint(s))
                            .collect();
                        Some(dominant_codec_hint(&hints))
                    } else {
                        None
                    }
                })
                .unwrap_or(CodecHint::Unknown)
        } else {
            CodecHint::Unknown
        };

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
                        codec_hint: dominant_hint,
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
                            sub.codec_hint = dominant_hint;
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
                            codec_hint: dominant_hint,
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
                let avg: usize = 1024 * 1024; // 1 MB
                let min: usize = 512 * 1024; // 512 KB
                let max: usize = 4 * 1024 * 1024; // 4 MB
                let cdc = FastCDC::with_level_and_seed(
                    full_chunk_data,
                    min,
                    avg,
                    max,
                    fastcdc::v2020::Normalization::Level1,
                    self.seed,
                );
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
    /// FBX binary files have a node-based structure: a flat list of top-level
    /// node records, each starting with an `EndOffset` field that points
    /// directly at the start of the next node. Walking `EndOffset -> EndOffset`
    /// lets us emit one chunk per top-level node (coalescing small nodes and
    /// splitting oversized ones) without parsing property lists or names.
    /// Falls back to CDC for ASCII FBX or any structure that fails sanity
    /// checks (see [`Self::chunk_fbx_legacy`] for the `MEDIAGIT_CHUNK_FBX=0`
    /// path, which reproduces the pre-P3a header+CDC behavior exactly).
    pub(super) async fn chunk_fbx(&self, data: &[u8]) -> Result<Vec<ContentChunk>> {
        // FBX binary magic: "Kaydara FBX Binary  \x00"
        const FBX_MAGIC: &[u8] = b"Kaydara FBX Binary  \x00";
        const HEADER_LEN: usize = 27;

        if data.len() < HEADER_LEN || &data[0..21] != FBX_MAGIC {
            // ASCII FBX or invalid - use rolling CDC
            debug!("FBX file is ASCII or invalid, using rolling CDC");
            let (avg, min, max) = get_chunk_params(data.len() as u64);
            return self.chunk_fastcdc(data, avg, min, max).await;
        }

        if !chunk_fbx_enabled() {
            return self.chunk_fbx_legacy(data, HEADER_LEN).await;
        }

        self.chunk_fbx_walker(data).await
    }

    /// EndOffset node walker for binary FBX. Default-off (opt in with
    /// `MEDIAGIT_CHUNK_FBX=1`, see `chunk_fbx_enabled`); kept fully tested so
    /// it can be re-enabled by default once it shows a harness gain. Callers
    /// must have already validated the binary-FBX magic and minimum length.
    pub(super) async fn chunk_fbx_walker(&self, data: &[u8]) -> Result<Vec<ContentChunk>> {
        const HEADER_LEN: usize = 27;

        if data.len() < HEADER_LEN {
            let (avg, min, max) = get_chunk_params(data.len() as u64);
            return self.chunk_fastcdc(data, avg, min, max).await;
        }

        // Parse FBX version (bytes 23-26, little-endian). Versions >= 7500
        // (FBX 7.5+) widen EndOffset/NumProperties/PropertyListLen to u64.
        let version = u32::from_le_bytes([data[23], data[24], data[25], data[26]]);
        let off_width: usize = if version >= 7500 { 8 } else { 4 };

        // Walk top-level node records by jumping EndOffset -> EndOffset.
        // EndOffset == 0 is the NULL record that terminates the node list;
        // everything from there on (including trailing footer bytes) is
        // treated as a single trailing chunk.
        let mut node_ranges: Vec<(usize, usize)> = Vec::new();
        let mut pos = HEADER_LEN;
        let mut footer_start = data.len();
        let mut parse_ok = true;

        loop {
            if pos + off_width > data.len() {
                footer_start = pos;
                break;
            }
            let end_offset = if off_width == 8 {
                u64::from_le_bytes(data[pos..pos + 8].try_into().unwrap()) as usize
            } else {
                u32::from_le_bytes(data[pos..pos + 4].try_into().unwrap()) as usize
            };

            if end_offset == 0 {
                footer_start = pos;
                break;
            }

            // Offsets must be monotonically increasing and stay in-bounds.
            if end_offset <= pos || end_offset > data.len() {
                parse_ok = false;
                break;
            }

            node_ranges.push((pos, end_offset));
            pos = end_offset;
        }

        if !parse_ok {
            // Malformed node structure under a valid magic — fall back to the
            // same header+CDC treatment the pre-P3a implementation always used.
            debug!("FBX node offsets failed sanity check, using legacy header+CDC");
            return self.chunk_fbx_legacy(data, HEADER_LEN).await;
        }

        let mut chunks = Vec::new();
        let header = &data[0..HEADER_LEN];
        chunks.push(ContentChunk {
            id: Oid::hash(header),
            data: header.to_vec(),
            offset: 0,
            size: HEADER_LEN,
            chunk_type: ChunkType::Metadata,
            perceptual_hash: None,
            codec_hint: CodecHint::Unknown,
        });

        chunks.extend(
            self.emit_coalesced_node_chunks(data, &node_ranges, 256 * 1024)
                .await?,
        );

        if footer_start < data.len() {
            let footer = &data[footer_start..];
            chunks.push(ContentChunk {
                id: Oid::hash(footer),
                data: footer.to_vec(),
                offset: footer_start as u64,
                size: footer.len(),
                chunk_type: ChunkType::Metadata,
                perceptual_hash: None,
                codec_hint: CodecHint::Unknown,
            });
        }

        info!(
            chunks = chunks.len(),
            nodes = node_ranges.len(),
            total_size = data.len(),
            "FBX EndOffset chunking complete"
        );

        Ok(chunks)
    }

    /// Pre-P3a FBX chunking: 27-byte header as a single Metadata chunk,
    /// followed by rolling CDC over the remainder. Preserved verbatim so
    /// `MEDIAGIT_CHUNK_FBX=0` restores the exact legacy chunk boundaries.
    pub(super) async fn chunk_fbx_legacy(
        &self,
        data: &[u8],
        header_len: usize,
    ) -> Result<Vec<ContentChunk>> {
        let mut chunks = Vec::new();
        let header = &data[0..header_len];
        chunks.push(ContentChunk {
            id: Oid::hash(header),
            data: header.to_vec(),
            offset: 0,
            size: header_len,
            chunk_type: ChunkType::Metadata,
            perceptual_hash: None,
            codec_hint: CodecHint::Unknown,
        });

        if data.len() > header_len {
            let content = &data[header_len..];
            let (avg, min, max) = get_chunk_params(content.len() as u64);
            let sub_chunks = self.chunk_fastcdc(content, avg, min, max).await?;

            for mut chunk in sub_chunks {
                chunk.offset += header_len as u64;
                chunks.push(chunk);
            }
        }

        Ok(chunks)
    }

    /// Coalesce leading small structural node/block ranges (each under
    /// `coalesce_threshold`) into one metadata group, then treat the first
    /// node/block that stands alone (i.e. already at or past the threshold)
    /// *and everything after it* as a single continuous span. Seeded FastCDC
    /// (1 MB avg / 512 KB min / 4 MB max) then runs once per emitted group.
    /// Shared by the FBX EndOffset walker and the Blender BHEAD walker,
    /// which both walk a flat list of contiguous `(start, end)` byte ranges.
    ///
    /// Two earlier revisions were tried and measured against the
    /// `dedup_report` corpus (which includes v1/v2 pairs edited by inserting
    /// bytes mid-file — the common case for a real content edit):
    ///  - CDC only when a group exceeded 4 MB, else one opaque chunk: let a
    ///    single content-bearing node in the 256 KB-4 MB range (e.g. FBX's
    ///    `Objects` on a several-MB model) become one monolithic chunk — a
    ///    single edit anywhere inside invalidated the whole chunk.
    ///  - Always CDC each coalesced group independently (forcing a boundary
    ///    reset at every node start): regressed FBX dedup below baseline.
    ///    FBX's `EndOffset` chain is absolute, not self-relative, so any
    ///    size-changing edit inside a non-final node stales every
    ///    downstream `EndOffset` — the edited file then fails the walk
    ///    entirely and falls back to one continuous legacy CDC scan, while
    ///    the unedited file still used the new per-node multi-scan. Forcing
    ///    a reset at each node boundary in one but not the other misaligns
    ///    chunk boundaries for the whole content region, even though the
    ///    underlying bytes are otherwise unchanged.
    ///
    /// Treating everything from the first large node onward as one
    /// continuous scan matches what the legacy fallback does for that same
    /// region, so both sides of a pair align the same way regardless of
    /// which one succeeds the structural walk — while still separating out
    /// the (edit-immune, since it's always before any real content) leading
    /// metadata run as its own chunk.
    async fn emit_coalesced_node_chunks(
        &self,
        data: &[u8],
        ranges: &[(usize, usize)],
        coalesce_threshold: usize,
    ) -> Result<Vec<ContentChunk>> {
        let mut chunks = Vec::new();
        let mut i = 0;
        while i < ranges.len() {
            let start = ranges[i].0;
            let mut end = ranges[i].1;
            i += 1;
            while end - start < coalesce_threshold && i < ranges.len() {
                end = ranges[i].1;
                i += 1;
            }
            if i < ranges.len() {
                // This group reached the threshold on its own with more
                // ranges remaining: swallow everything else into one final
                // continuous span rather than resetting the CDC scan at
                // every subsequent node/block boundary.
                end = ranges[ranges.len() - 1].1;
                i = ranges.len();
            }

            let seg = &data[start..end];
            if !seg.is_empty() {
                let sub_chunks = self
                    .chunk_fastcdc(seg, 1024 * 1024, 512 * 1024, 4 * 1024 * 1024)
                    .await?;
                for mut sub in sub_chunks {
                    sub.offset += start as u64;
                    chunks.push(sub);
                }
            }
        }
        Ok(chunks)
    }

    /// Blender `.blend` scene file chunking — BHEAD block walker.
    ///
    /// Uncompressed `.blend` files start with a 12-byte header (`BLENDER` +
    /// pointer-size char + endianness char + 3-digit version), followed by a
    /// flat sequence of BHEAD file-blocks: `code[4] + len(u32) + old-ptr(4 or
    /// 8 bytes) + SDNAnr(u32) + nr(u32)`, then `len` bytes of payload. Walking
    /// block-to-block (using each block's declared `len`) lets us emit one
    /// chunk per block, coalescing small ones and splitting oversized ones.
    /// `ENDB` terminates the block list.
    ///
    /// Blender >= 3.0 default-saves `.blend` files zstd-compressed (older
    /// versions used gzip); a missing `BLENDER` magic — including either
    /// compressed form — falls back to generic CDC rather than decompressing.
    /// Big-endian files and any structure that fails sanity checks also fall
    /// back to CDC.
    pub(super) async fn chunk_blend(&self, data: &[u8]) -> Result<Vec<ContentChunk>> {
        const MAGIC: &[u8] = b"BLENDER";
        const HEADER_LEN: usize = 12;

        if !chunk_blend_enabled() || data.len() < HEADER_LEN || &data[0..7] != MAGIC {
            debug!("Not an uncompressed .blend (or disabled), using rolling CDC");
            let (avg, min, max) = get_chunk_params(data.len() as u64);
            return self.chunk_fastcdc(data, avg, min, max).await;
        }

        let ptr_size: usize = match data[7] {
            b'_' => 4,
            b'-' => 8,
            _ => {
                debug!("Unrecognized .blend pointer-size marker, using rolling CDC");
                let (avg, min, max) = get_chunk_params(data.len() as u64);
                return self.chunk_fastcdc(data, avg, min, max).await;
            }
        };
        if data[8] != b'v' {
            // Big-endian .blend - fall back rather than byte-swapping.
            debug!("Big-endian .blend, using rolling CDC");
            let (avg, min, max) = get_chunk_params(data.len() as u64);
            return self.chunk_fastcdc(data, avg, min, max).await;
        }

        let bhead_len = 4 + 4 + ptr_size + 4 + 4; // code + len + old + SDNAnr + nr

        let mut block_ranges: Vec<(usize, usize)> = Vec::new();
        let mut pos = HEADER_LEN;
        let mut parse_ok = true;

        while pos + bhead_len <= data.len() {
            let code = &data[pos..pos + 4];
            let body_len = u32::from_le_bytes(data[pos + 4..pos + 8].try_into().unwrap()) as usize;
            let block_end = pos.saturating_add(bhead_len).saturating_add(body_len);

            if block_end > data.len() || block_end <= pos {
                parse_ok = false;
                break;
            }

            block_ranges.push((pos, block_end));
            let is_endb = code == b"ENDB";
            pos = block_end;
            if is_endb {
                break;
            }
        }

        if !parse_ok {
            debug!("Blend BHEAD structure failed sanity check, using rolling CDC");
            let (avg, min, max) = get_chunk_params(data.len() as u64);
            return self.chunk_fastcdc(data, avg, min, max).await;
        }

        let mut chunks = Vec::new();
        let header = &data[0..HEADER_LEN];
        chunks.push(ContentChunk {
            id: Oid::hash(header),
            data: header.to_vec(),
            offset: 0,
            size: HEADER_LEN,
            chunk_type: ChunkType::Metadata,
            perceptual_hash: None,
            codec_hint: CodecHint::Unknown,
        });

        chunks.extend(
            self.emit_coalesced_node_chunks(data, &block_ranges, 256 * 1024)
                .await?,
        );

        if pos < data.len() {
            let footer = &data[pos..];
            chunks.push(ContentChunk {
                id: Oid::hash(footer),
                data: footer.to_vec(),
                offset: pos as u64,
                size: footer.len(),
                chunk_type: ChunkType::Metadata,
                perceptual_hash: None,
                codec_hint: CodecHint::Unknown,
            });
        }

        info!(
            chunks = chunks.len(),
            blocks = block_ranges.len(),
            total_size = data.len(),
            "Blend BHEAD chunking complete"
        );

        Ok(chunks)
    }

    /// Binary STL chunking — content-defined cuts over the triangle array.
    ///
    /// Binary STL is `80-byte header + u32 triangle count + count * 50-byte
    /// records` with no other structure. Validates `84 + count*50 ==
    /// file_len`; anything else (ASCII STL, truncated/malformed binary STL)
    /// falls back to [`Self::chunk_3d_text`] — the pre-P3a routing for
    /// `.stl`, which itself CDC-falls-back for non-text data.
    /// `MEDIAGIT_CHUNK_STL=0` takes the same fallback.
    ///
    /// The triangle array is CDC-subdivided (seeded FastCDC, 1 MB avg / 512
    /// KB min / 4 MB max) rather than cut at fixed 1 MB/triangle-count
    /// boundaries: fixed-position cuts do not re-sync after a mid-file
    /// insertion (every chunk after the edit shifts and stops matching), so
    /// they cannot recover any dedup on the unedited remainder — exactly the
    /// scenario a byte-inserted STL edit exercises. FastCDC re-syncs, at the
    /// cost of no longer guaranteeing a cut never splits a 50-byte record.
    /// The header is folded into the first emitted chunk.
    pub(super) async fn chunk_stl(&self, data: &[u8]) -> Result<Vec<ContentChunk>> {
        const STL_HEADER_LEN: usize = 80;
        const COUNT_LEN: usize = 4;
        const TRIANGLE_STRIDE: usize = 50;

        if !chunk_stl_enabled() || data.len() < STL_HEADER_LEN + COUNT_LEN {
            return self.chunk_3d_text(data).await;
        }

        let count = u32::from_le_bytes(
            data[STL_HEADER_LEN..STL_HEADER_LEN + COUNT_LEN]
                .try_into()
                .unwrap(),
        ) as usize;
        let header_len = STL_HEADER_LEN + COUNT_LEN; // 84
        let expected_len = header_len + count * TRIANGLE_STRIDE;

        if expected_len != data.len() {
            // ASCII STL (starts "solid ...") or malformed binary STL.
            debug!("Not a valid binary STL (size mismatch), using pre-P3a text/CDC routing");
            return self.chunk_3d_text(data).await;
        }

        let triangle_data = &data[header_len..];
        let mut chunks = self
            .chunk_fastcdc(triangle_data, 1024 * 1024, 512 * 1024, 4 * 1024 * 1024)
            .await?;
        for c in &mut chunks {
            c.offset += header_len as u64;
        }

        match chunks.first_mut() {
            Some(first) => {
                let mut merged = Vec::with_capacity(header_len + first.data.len());
                merged.extend_from_slice(&data[0..header_len]);
                merged.extend_from_slice(&first.data);
                first.id = Oid::hash(&merged);
                first.size = merged.len();
                first.data = merged;
                first.offset = 0;
            }
            None => {
                // count == 0: no triangle data, just the header.
                let header = &data[0..header_len];
                chunks.push(ContentChunk {
                    id: Oid::hash(header),
                    data: header.to_vec(),
                    offset: 0,
                    size: header.len(),
                    chunk_type: ChunkType::Generic,
                    perceptual_hash: None,
                    codec_hint: CodecHint::Unknown,
                });
            }
        }

        info!(
            chunks = chunks.len(),
            triangles = count,
            total_size = data.len(),
            "STL content-defined chunking complete"
        );

        Ok(chunks)
    }

    /// PLY chunking — header parse + element-aligned content-defined cuts.
    ///
    /// Only `format binary_little_endian` PLY files with a fixed-size (no
    /// `list` property) `vertex` element as the first element get
    /// structure-aware treatment: the vertex block and everything after it
    /// (face/edge blocks, which typically use variable-length `list`
    /// properties) are each separately CDC-subdivided (seeded FastCDC, 1 MB
    /// avg / 512 KB min / 4 MB max) — separated so an edit in one block
    /// can't shift boundaries in the other. ASCII PLY, big-endian PLY, or
    /// any parse doubt falls back to [`Self::chunk_3d_text`] — the pre-P3a
    /// routing for `.ply`, which itself CDC-falls-back for non-text data.
    /// `MEDIAGIT_CHUNK_PLY=0` takes the same fallback.
    pub(super) async fn chunk_ply(&self, data: &[u8]) -> Result<Vec<ContentChunk>> {
        if chunk_ply_enabled()
            && let Some(info) = parse_ply_binary_header(data)
        {
            let vertex_block_len = info.vertex_count * info.vertex_stride;
            let vertex_block_end = info.header_end + vertex_block_len;
            if vertex_block_end <= data.len() {
                return self.chunk_ply_binary(data, &info).await;
            }
        }

        debug!("Not a fixed-stride binary_little_endian PLY, using pre-P3a text/CDC routing");
        self.chunk_3d_text(data).await
    }

    async fn chunk_ply_binary(
        &self,
        data: &[u8],
        info: &PlyHeaderInfo,
    ) -> Result<Vec<ContentChunk>> {
        let mut chunks = Vec::new();
        let header = &data[0..info.header_end];
        chunks.push(ContentChunk {
            id: Oid::hash(header),
            data: header.to_vec(),
            offset: 0,
            size: header.len(),
            chunk_type: ChunkType::Metadata,
            perceptual_hash: None,
            codec_hint: CodecHint::Unknown,
        });

        let vertex_start = info.header_end;
        let vertex_block_len = info.vertex_count * info.vertex_stride;
        let vertex_end = vertex_start + vertex_block_len;

        // Content-defined (FastCDC) rather than fixed-position cuts: fixed
        // cuts don't re-sync after a mid-block insertion/edit, losing dedup
        // on the entire unedited remainder (see chunk_stl for the measured
        // regression this caused when first implemented with fixed cuts).
        if vertex_block_len > 0 {
            let vertex_block = &data[vertex_start..vertex_end];
            let sub_chunks = self
                .chunk_fastcdc(vertex_block, 1024 * 1024, 512 * 1024, 4 * 1024 * 1024)
                .await?;
            for mut sub in sub_chunks {
                sub.offset += vertex_start as u64;
                chunks.push(sub);
            }
        }

        let rest = &data[vertex_end..];
        if !rest.is_empty() {
            let sub_chunks = self
                .chunk_fastcdc(rest, 1024 * 1024, 512 * 1024, 4 * 1024 * 1024)
                .await?;
            for mut sub in sub_chunks {
                sub.offset += vertex_end as u64;
                chunks.push(sub);
            }
        }

        info!(
            chunks = chunks.len(),
            vertices = info.vertex_count,
            total_size = data.len(),
            "PLY element-aligned chunking complete"
        );

        Ok(chunks)
    }
}

/// Parsed subset of a binary PLY header needed for element-aligned chunking.
pub(super) struct PlyHeaderInfo {
    /// Byte offset right after the `end_header\n` line.
    pub(super) header_end: usize,
    /// `element vertex <N>` count.
    pub(super) vertex_count: usize,
    /// Sum of the fixed-size vertex properties' byte widths.
    pub(super) vertex_stride: usize,
}

/// Parse a PLY header looking for `format binary_little_endian` with a
/// `vertex` element (as the first element) whose properties are all
/// fixed-size (no `list` property — variable-stride vertex blocks aren't
/// supported). Returns `None` on ASCII/big-endian format, a missing/absent
/// `end_header` marker within the scan window, or any other parse doubt —
/// callers must fall back to generic CDC in that case.
///
/// Only the header bytes (guaranteed ASCII by the PLY spec) are decoded as
/// UTF-8; the binary payload after `end_header` is never touched.
pub(super) fn parse_ply_binary_header(data: &[u8]) -> Option<PlyHeaderInfo> {
    const MAX_HEADER_SCAN: usize = 64 * 1024;
    let scan_limit = data.len().min(MAX_HEADER_SCAN);
    let marker = b"end_header\n";
    let end_idx = data[..scan_limit]
        .windows(marker.len())
        .position(|w| w == marker)?;
    let header_end = end_idx + marker.len();
    let header_text = std::str::from_utf8(&data[..header_end]).ok()?;
    if !header_text.starts_with("ply") {
        return None;
    }

    let mut format_binary_le = false;
    let mut in_vertex = false;
    let mut saw_vertex_element = false;
    let mut vertex_count: Option<usize> = None;
    let mut stride = 0usize;

    for line in header_text.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("format ") {
            format_binary_le = rest.trim_start().starts_with("binary_little_endian");
        } else if let Some(rest) = line.strip_prefix("element ") {
            let mut parts = rest.split_whitespace();
            let name = parts.next().unwrap_or("");
            let count: usize = parts.next().and_then(|s| s.parse().ok()).unwrap_or(0);
            if name == "vertex" && !saw_vertex_element {
                in_vertex = true;
                saw_vertex_element = true;
                vertex_count = Some(count);
            } else {
                in_vertex = false;
            }
        } else if let Some(rest) = line.strip_prefix("property ")
            && in_vertex
        {
            let rest = rest.trim_start();
            if rest.starts_with("list") {
                return None; // variable-stride vertex block not supported
            }
            let type_name = rest.split_whitespace().next().unwrap_or("");
            stride += ply_type_size(type_name)?;
        }
    }

    if !format_binary_le {
        return None;
    }
    let vertex_count = vertex_count?;
    if vertex_count > 0 && stride == 0 {
        return None;
    }

    Some(PlyHeaderInfo {
        header_end,
        vertex_count,
        vertex_stride: stride,
    })
}

/// Byte width of a PLY scalar property type name. `None` for anything
/// unrecognized (caller treats this as a parse failure -> fallback).
fn ply_type_size(type_name: &str) -> Option<usize> {
    Some(match type_name {
        "char" | "uchar" | "int8" | "uint8" => 1,
        "short" | "ushort" | "int16" | "uint16" => 2,
        "int" | "uint" | "int32" | "uint32" | "float" | "float32" => 4,
        "double" | "float64" => 8,
        _ => return None,
    })
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

/// Return the content (body, after the atom header) of every immediate child
/// atom of `container_body` matching `atom_type`. Bounds-checked; a
/// malformed/truncated child atom is skipped rather than causing a panic.
fn child_atom_bodies<'a>(container_body: &'a [u8], atom_type: &[u8; 4]) -> Vec<&'a [u8]> {
    parse_mp4_atoms(container_body)
        .into_iter()
        .filter(|a| &a.atom_type == atom_type)
        .filter_map(|a| {
            let start = (a.offset as usize).checked_add(a.header_size as usize)?;
            let end = (a.offset as usize).checked_add(a.size as usize)?;
            if start <= end && end <= container_body.len() {
                Some(&container_body[start..end])
            } else {
                None
            }
        })
        .collect()
}

/// Walk `moov → trak → mdia → minf → stbl → stsd` to collect every
/// sample-entry FourCC in the file. `moov_body` is the moov atom's content
/// *after* its own 8-byte atom header (matching the slicing `chunk_mp4`
/// already uses for nested-atom parsing). Never touches mdat; malformed or
/// truncated boxes are skipped, never panicked on.
fn mp4_stsd_fourccs(moov_body: &[u8]) -> Vec<[u8; 4]> {
    let mut out = Vec::new();
    for trak in child_atom_bodies(moov_body, b"trak") {
        for mdia in child_atom_bodies(trak, b"mdia") {
            for minf in child_atom_bodies(mdia, b"minf") {
                for stbl in child_atom_bodies(minf, b"stbl") {
                    for stsd in child_atom_bodies(stbl, b"stsd") {
                        out.extend(parse_stsd_sample_entries(stsd));
                    }
                }
            }
        }
    }
    out
}

/// Parse an `stsd` box body (version+flags(4) + entry_count(4) + entries)
/// and return each sample entry's format FourCC. Stops without panicking on
/// any malformed/truncated entry.
fn parse_stsd_sample_entries(stsd_body: &[u8]) -> Vec<[u8; 4]> {
    let mut out = Vec::new();
    if stsd_body.len() < 8 {
        return out;
    }
    let entry_count = u32::from_be_bytes([stsd_body[4], stsd_body[5], stsd_body[6], stsd_body[7]]);
    let mut pos = 8usize;
    for _ in 0..entry_count {
        if pos + 8 > stsd_body.len() {
            break;
        }
        let entry_size = u32::from_be_bytes([
            stsd_body[pos],
            stsd_body[pos + 1],
            stsd_body[pos + 2],
            stsd_body[pos + 3],
        ]) as usize;
        let mut fourcc = [0u8; 4];
        fourcc.copy_from_slice(&stsd_body[pos + 4..pos + 8]);
        out.push(fourcc);
        if entry_size < 8 || pos + entry_size > stsd_body.len() {
            break;
        }
        pos += entry_size;
    }
    out
}

/// Map an MP4/MOV sample-entry FourCC (from `stsd`) to a `CodecHint`.
fn mp4_fourcc_to_codec_hint(fourcc: &[u8; 4]) -> CodecHint {
    match fourcc {
        b"avc1" | b"avc3" => CodecHint::H264,
        b"hvc1" | b"hev1" | b"dvh1" | b"dvhe" => CodecHint::H265,
        b"vp09" => CodecHint::VP9,
        b"av01" => CodecHint::AV1,
        b"apch" | b"apcn" | b"apcs" | b"apco" | b"ap4h" | b"ap4x" => CodecHint::ProRes,
        b"AVdn" | b"AVdh" => CodecHint::DNxHR,
        b"mjp2" => CodecHint::Jpeg2000,
        b"v210" | b"2vuy" | b"yuv2" | b"raw " => CodecHint::RawVideo,
        b"mp4a" => CodecHint::AAC,
        b"Opus" => CodecHint::Opus,
        b"fLaC" => CodecHint::FLAC,
        b"alac" => CodecHint::ALAC,
        b"tx3g" | b"wvtt" => CodecHint::TextSub,
        _ => CodecHint::Unknown,
    }
}

/// Scan an AVI/RIFF file's `hdrl → strl → strh/strf` metadata (a few KB) for
/// the dominant video (else audio) codec hint. Never reads `movi` payload
/// bytes — their size is used only to skip past them.
fn avi_dominant_codec_hint(data: &[u8]) -> CodecHint {
    let mut hints = Vec::new();
    if data.len() < 12 || &data[0..4] != b"RIFF" {
        return CodecHint::Unknown;
    }

    let mut file_pos = 0usize;
    while file_pos + 12 <= data.len() {
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

        if &data[file_pos..file_pos + 4] == b"RIFF"
            && (form_type == b"AVI " || form_type == b"AVIX")
        {
            collect_avi_strl_hints(data, file_pos + 12, block_end, &mut hints);
        }

        if block_end <= file_pos {
            break; // malformed size — avoid an infinite loop
        }
        file_pos = block_end;
    }

    dominant_codec_hint(&hints)
}

/// Recursively walk RIFF sub-chunks looking for `LIST/strl` blocks, skipping
/// `LIST/movi` payload entirely (only its size is used to advance past it).
fn collect_avi_strl_hints(data: &[u8], start: usize, end: usize, hints: &mut Vec<CodecHint>) {
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
            if list_type == b"strl" {
                if let Some(h) = strl_codec_hint(&data[pos + 12..chunk_end]) {
                    hints.push(h);
                }
            } else if list_type != b"movi" {
                // hdrl and similar containers — descend for nested strl.
                collect_avi_strl_hints(data, pos + 12, chunk_end, hints);
            }
            // movi: skip — its payload is never scanned here.
        }

        if chunk_end <= pos {
            break; // malformed size — avoid an infinite loop
        }
        pos = chunk_end;
    }
}

/// Parse a `strl` block's `strh`/`strf` sub-chunks and classify the stream.
/// Returns `None` if `fccType` is neither `vids` nor `auds` (e.g. subtitle
/// streams) or the block is too short to read.
fn strl_codec_hint(strl_body: &[u8]) -> Option<CodecHint> {
    let mut fcc_type: Option<[u8; 4]> = None;
    let mut fcc_handler: Option<[u8; 4]> = None;
    let mut strf_body: Option<&[u8]> = None;

    let mut pos = 0usize;
    while pos + 8 <= strl_body.len() {
        let fourcc = &strl_body[pos..pos + 4];
        let size = u32::from_le_bytes([
            strl_body[pos + 4],
            strl_body[pos + 5],
            strl_body[pos + 6],
            strl_body[pos + 7],
        ]) as usize;
        let data_start = pos + 8;
        let data_end = data_start.saturating_add(size).min(strl_body.len());

        if fourcc == b"strh" && data_end.saturating_sub(data_start) >= 8 {
            let mut t = [0u8; 4];
            t.copy_from_slice(&strl_body[data_start..data_start + 4]);
            let mut h = [0u8; 4];
            h.copy_from_slice(&strl_body[data_start + 4..data_start + 8]);
            fcc_type = Some(t);
            fcc_handler = Some(h);
        } else if fourcc == b"strf" {
            strf_body = Some(&strl_body[data_start..data_end]);
        }

        let padded_end = if !size.is_multiple_of(2) {
            (data_end + 1).min(strl_body.len())
        } else {
            data_end
        };
        if padded_end <= pos {
            break; // malformed size — avoid an infinite loop
        }
        pos = padded_end;
    }

    match fcc_type {
        Some(t) if &t == b"vids" => Some(avi_video_codec_hint(fcc_handler, strf_body)),
        Some(t) if &t == b"auds" => Some(avi_audio_codec_hint(strf_body)),
        _ => None,
    }
}

/// Classify a `vids` stream from `strh`'s `fccHandler` or `strf`'s
/// `biCompression` (BITMAPINFOHEADER offset 16..20).
fn avi_video_codec_hint(fcc_handler: Option<[u8; 4]>, strf: Option<&[u8]>) -> CodecHint {
    let is_h264 = |f: &[u8; 4]| matches!(f, b"H264" | b"h264" | b"X264" | b"x264" | b"avc1");
    let is_raw = |f: &[u8; 4]| f == b"DIB " || *f == [0u8; 4];

    if let Some(h) = fcc_handler {
        if is_h264(&h) {
            return CodecHint::H264;
        }
        if is_raw(&h) {
            return CodecHint::RawVideo;
        }
    }
    if let Some(strf) = strf
        && strf.len() >= 20
    {
        let mut comp = [0u8; 4];
        comp.copy_from_slice(&strf[16..20]);
        if is_h264(&comp) {
            return CodecHint::H264;
        }
        if is_raw(&comp) {
            return CodecHint::RawVideo;
        }
    }
    CodecHint::Unknown
}

/// Classify an `auds` stream from `strf`'s `wFormatTag` (WAVEFORMATEX offset 0..2, LE).
fn avi_audio_codec_hint(strf: Option<&[u8]>) -> CodecHint {
    let Some(strf) = strf else {
        return CodecHint::Unknown;
    };
    if strf.len() < 2 {
        return CodecHint::Unknown;
    }
    match u16::from_le_bytes([strf[0], strf[1]]) {
        0x0001 => CodecHint::PCM,
        0x0055 => CodecHint::MP3,
        0x00FF => CodecHint::AAC,
        0x674F | 0x6771 => CodecHint::Vorbis,
        _ => CodecHint::Unknown,
    }
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

/// Walk a Matroska `Tracks` element body for each `TrackEntry`'s `CodecID`
/// string. `tracks_body` is the Tracks element's content *after* its own
/// EBML header (offset + header_size), matching the slicing `chunk_matroska`
/// already uses for other elements. Malformed/truncated entries are skipped,
/// never panicked on.
fn mkv_track_codec_ids(tracks_body: &[u8]) -> Vec<String> {
    let mut out = Vec::new();
    for entry in parse_ebml_elements(tracks_body)
        .iter()
        .filter(|e| e.id == TRACK_ENTRY_ID)
    {
        let entry_start = entry.offset as usize + entry.header_size as usize;
        let entry_end = if entry.data_size == u64::MAX {
            tracks_body.len()
        } else {
            (entry.offset as usize + entry.header_size as usize + entry.data_size as usize)
                .min(tracks_body.len())
        };
        if entry_start >= entry_end || entry_end > tracks_body.len() {
            continue;
        }
        let entry_body = &tracks_body[entry_start..entry_end];

        for codec in parse_ebml_elements(entry_body)
            .iter()
            .filter(|e| e.id == CODEC_ID_ID)
        {
            let cs = codec.offset as usize + codec.header_size as usize;
            let ce = if codec.data_size == u64::MAX {
                entry_body.len()
            } else {
                (codec.offset as usize + codec.header_size as usize + codec.data_size as usize)
                    .min(entry_body.len())
            };
            if cs < ce
                && let Ok(s) = std::str::from_utf8(&entry_body[cs..ce])
            {
                out.push(s.trim_end_matches('\0').to_string());
            }
        }
    }
    out
}

/// Map a Matroska `CodecID` string to a `CodecHint`.
pub(super) fn mkv_codec_id_to_hint(codec_id: &str) -> CodecHint {
    if codec_id.starts_with("V_MPEG4/ISO/AVC") {
        CodecHint::H264
    } else if codec_id.starts_with("V_MPEGH/ISO/HEVC") {
        CodecHint::H265
    } else if codec_id == "V_VP9" {
        CodecHint::VP9
    } else if codec_id == "V_AV1" {
        CodecHint::AV1
    } else if codec_id.starts_with("V_PRORES") {
        CodecHint::ProRes
    } else if codec_id == "V_UNCOMPRESSED" {
        CodecHint::RawVideo
    } else if codec_id.starts_with("A_AAC") {
        CodecHint::AAC
    } else if codec_id == "A_OPUS" {
        CodecHint::Opus
    } else if codec_id == "A_VORBIS" {
        CodecHint::Vorbis
    } else if codec_id == "A_FLAC" {
        CodecHint::FLAC
    } else if codec_id.starts_with("A_PCM") {
        CodecHint::PCM
    } else if codec_id == "A_MPEG/L3" {
        CodecHint::MP3
    } else if codec_id == "A_ALAC" {
        CodecHint::ALAC
    } else if codec_id.starts_with("S_TEXT") {
        CodecHint::TextSub
    } else if codec_id == "S_HDMV/PGS" || codec_id == "S_VOBSUB" {
        CodecHint::BitmapSub
    } else {
        CodecHint::Unknown
    }
}
