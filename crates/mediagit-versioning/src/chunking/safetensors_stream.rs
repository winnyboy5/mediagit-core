// SPDX-License-Identifier: BUSL-1.1
// Copyright (C) 2025-2026 Aswin Krishnamoorthy

//! Fixed-chunk walker for `.safetensors` (knob `MEDIAGIT_TENSOR_DELTA=1`).
//!
//! Layout: `u64 LE header_len | JSON header | tensor data`. Chunk 0 is the
//! prefix `[0, 8 + header_len)` ([`ChunkType::Metadata`]); the data region is
//! then cut into fixed 1 MiB chunks counted from the data start, so a header
//! that grows or shrinks never shifts a data chunk. Version N's chunk at data
//! offset X is therefore the natural XOR-planes delta base for version N+1's
//! chunk at the same X. Any parse problem declines (nothing emitted) and the
//! caller uses its existing path.

use super::{ChunkType, CodecHint, ContentChunk};
use crate::Oid;
use anyhow::{Result, bail};
use std::collections::HashMap;
use std::io::Read;

/// Fixed data-chunk size.
pub(crate) const CHUNK_SIZE: usize = 1024 * 1024;
/// safetensors itself caps the header at 100 MB.
const MAX_HEADER: u64 = 100 * 1024 * 1024;

/// `MEDIAGIT_TENSOR_DELTA=1` turns the walker and tensor XOR-planes delta on.
pub fn tensor_delta_enabled() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| {
        tensor_delta_from_env(std::env::var("MEDIAGIT_TENSOR_DELTA").ok().as_deref())
    })
}

/// Split out so tests can check parsing without the process-wide `OnceLock`.
fn tensor_delta_from_env(var: Option<&str>) -> bool {
    var == Some("1")
}

/// Positional delta bases for one `.safetensors` path: previous version's
/// data-chunk ids keyed by offset relative to the data start, plus the layout
/// fingerprint of that version's header. Passed explicitly to the writer
/// (not ODB state) so parallel adds cannot race on it.
#[derive(Debug, Clone)]
pub struct PositionalBase {
    /// Data-region-relative chunk offset -> previous version's chunk id.
    pub map: HashMap<u64, Oid>,
    /// Fingerprint of the previous header's tensor name/dtype/shape/offsets.
    pub layout: Oid,
}

/// Parsed header: where data starts, the sorted dtype table and the layout
/// fingerprint.
#[derive(Debug, Clone)]
pub(crate) struct TensorLayout {
    /// `8 + header_len`: size of chunk 0 and absolute start of the data region.
    pub data_start: u64,
    /// `(rel_start, rel_end, element width)` sorted by `rel_start`.
    pub table: Vec<(u64, u64, u8)>,
    /// Hash of the sorted `(name, dtype, shape, start, end)` list.
    pub fingerprint: Oid,
}

fn dtype_width(dtype: &str) -> u8 {
    match dtype {
        "F32" | "I32" | "U32" => 4,
        "BF16" | "F16" | "I16" => 2,
        "F64" | "I64" => 8,
        _ => 1,
    }
}

impl TensorLayout {
    /// Parse `prefix` = the whole `[0, 8 + header_len)` byte range.
    pub(crate) fn parse(prefix: &[u8]) -> Result<Self> {
        if prefix.len() < 9 {
            bail!("safetensors prefix too short");
        }
        let hl = u64::from_le_bytes(prefix[..8].try_into()?);
        if hl == 0 || hl > MAX_HEADER || prefix.len() as u64 != 8 + hl {
            bail!("safetensors header length {hl} invalid");
        }
        let json: serde_json::Value = serde_json::from_slice(&prefix[8..])?;
        let obj = json
            .as_object()
            .ok_or_else(|| anyhow::anyhow!("safetensors header is not an object"))?;
        let mut entries: Vec<(&str, &str, String, u64, u64)> = Vec::new();
        for (name, v) in obj {
            if name == "__metadata__" {
                continue;
            }
            let dtype = v["dtype"].as_str();
            let shape = v["shape"].as_array();
            let off = v["data_offsets"].as_array();
            let (Some(dtype), Some(shape), Some(off)) = (dtype, shape, off) else {
                bail!("safetensors tensor '{name}' malformed");
            };
            let (Some(s), Some(e)) = (
                off.first().and_then(|x| x.as_u64()),
                off.get(1).and_then(|x| x.as_u64()),
            ) else {
                bail!("safetensors tensor '{name}' bad data_offsets");
            };
            if s > e {
                bail!("safetensors tensor '{name}' inverted data_offsets");
            }
            let shape: Vec<String> = shape.iter().map(|d| d.to_string()).collect();
            entries.push((name, dtype, shape.join(","), s, e));
        }
        entries.sort();
        let mut canon = String::new();
        for (name, dtype, shape, s, e) in &entries {
            canon.push_str(&format!("{name}|{dtype}|{shape}|{s}|{e}\n"));
        }
        let mut table: Vec<(u64, u64, u8)> = entries
            .iter()
            .map(|(_, dtype, _, s, e)| (*s, *e, dtype_width(dtype)))
            .collect();
        table.sort();
        Ok(Self {
            data_start: prefix.len() as u64,
            table,
            fingerprint: Oid::hash(canon.as_bytes()),
        })
    }

    /// Read and parse the header from `r` (positioned at the file start).
    /// Returns the layout and the prefix bytes (chunk 0).
    pub(crate) fn read<R: Read>(r: &mut R, file_size: u64) -> Result<(Self, Vec<u8>)> {
        let mut len = [0u8; 8];
        r.read_exact(&mut len)?;
        let hl = u64::from_le_bytes(len);
        if hl == 0 || hl > MAX_HEADER || 8 + hl > file_size {
            bail!("safetensors header length {hl} out of range");
        }
        let mut prefix = vec![0u8; (8 + hl) as usize];
        prefix[..8].copy_from_slice(&len);
        r.read_exact(&mut prefix[8..])?;
        Ok((Self::parse(&prefix)?, prefix))
    }

    /// `(w, phase)` for the chunk starting at data-relative offset `rel`:
    /// element width of the tensor covering `rel`, and `rel`'s position inside
    /// that element grid. Uncovered offsets (padding) get `(1, 0)`.
    pub(crate) fn plane_params(&self, rel: u64) -> (u8, u8) {
        let i = self.table.partition_point(|t| t.0 <= rel);
        if i > 0 {
            let (s, e, w) = self.table[i - 1];
            if rel < e {
                return (w, ((rel - s) % w as u64) as u8);
            }
        }
        (1, 0)
    }
}

fn data_chunk(data: Vec<u8>, offset: u64) -> ContentChunk {
    ContentChunk {
        id: Oid::hash(&data),
        size: data.len(),
        data,
        offset,
        chunk_type: ChunkType::Generic,
        perceptual_hash: None,
        codec_hint: CodecHint::Unknown,
    }
}

fn header_chunk(prefix: Vec<u8>) -> ContentChunk {
    ContentChunk {
        id: Oid::hash(&prefix),
        size: prefix.len(),
        data: prefix,
        offset: 0,
        chunk_type: ChunkType::Metadata,
        perceptual_hash: None,
        codec_hint: CodecHint::Unknown,
    }
}

/// Stream the file through `sink`. `Err` before the first `sink` call means
/// the walker declined (parse failure); the caller falls back.
pub(crate) fn chunk_file(
    file: &mut std::fs::File,
    file_size: u64,
    sink: &mut dyn FnMut(ContentChunk) -> Result<()>,
) -> Result<()> {
    let (layout, prefix) = TensorLayout::read(file, file_size)?;
    sink(header_chunk(prefix))?;
    let mut offset = layout.data_start;
    while offset < file_size {
        let want = CHUNK_SIZE.min((file_size - offset) as usize);
        let mut buf = vec![0u8; want];
        file.read_exact(&mut buf)?;
        sink(data_chunk(buf, offset))?;
        offset += want as u64;
    }
    Ok(())
}

/// In-memory variant for `ContentChunker::chunk`.
pub(crate) fn chunk_slice(data: &[u8]) -> Result<Vec<ContentChunk>> {
    let (layout, prefix) = TensorLayout::read(&mut &data[..], data.len() as u64)?;
    let mut out = vec![header_chunk(prefix)];
    let mut offset = layout.data_start as usize;
    while offset < data.len() {
        let end = (offset + CHUNK_SIZE).min(data.len());
        out.push(data_chunk(data[offset..end].to_vec(), offset as u64));
        offset = end;
    }
    Ok(out)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// Build a safetensors file: tensors are `(name, dtype, raw bytes)`;
    /// `pad` extra spaces are appended to the JSON header.
    pub(crate) fn make_safetensors(tensors: &[(&str, &str, &[u8])], pad: usize) -> Vec<u8> {
        let mut hdr = serde_json::Map::new();
        let mut at = 0usize;
        for (name, dtype, bytes) in tensors {
            let w = dtype_width(dtype) as usize;
            hdr.insert(
                (*name).into(),
                serde_json::json!({
                    "dtype": dtype,
                    "shape": [bytes.len() / w],
                    "data_offsets": [at, at + bytes.len()],
                }),
            );
            at += bytes.len();
        }
        let mut json = serde_json::to_vec(&hdr).unwrap();
        json.extend(std::iter::repeat_n(b' ', pad));
        let mut out = (json.len() as u64).to_le_bytes().to_vec();
        out.extend_from_slice(&json);
        for (_, _, bytes) in tensors {
            out.extend_from_slice(bytes);
        }
        out
    }

    fn noise(n: usize, mut x: u64) -> Vec<u8> {
        (0..n)
            .map(|_| {
                x ^= x << 13;
                x ^= x >> 7;
                x ^= x << 17;
                (x >> 24) as u8
            })
            .collect()
    }

    fn collect(path: &std::path::Path, walker: bool) -> Vec<(Oid, u64)> {
        let (tx, mut rx) = tokio::sync::mpsc::channel(1024);
        let p = path.to_path_buf();
        let h = std::thread::spawn(move || {
            crate::ContentChunker::new(crate::ChunkStrategy::MediaAware)
                .collect_file_chunks_with(&p, tx, walker)
        });
        let mut out = Vec::new();
        while let Some(c) = rx.blocking_recv() {
            out.push((c.id, c.offset));
        }
        h.join().unwrap().unwrap();
        out
    }

    /// Knob off must take the exact StreamCDC path a non-container extension
    /// takes; knob on must differ (header chunk + fixed grid).
    #[test]
    fn knob_off_matches_stream_cdc_boundaries() {
        let a = noise(3 * CHUNK_SIZE + 777, 9);
        let f = make_safetensors(&[("a", "I64", &a[..a.len() / 8 * 8])], 0);
        let dir = tempfile::tempdir().unwrap();
        let st = dir.path().join("m.safetensors");
        let bin = dir.path().join("m.bin");
        std::fs::write(&st, &f).unwrap();
        std::fs::write(&bin, &f).unwrap();
        let off = collect(&st, false);
        assert_eq!(off, collect(&bin, false));
        assert_ne!(off, collect(&st, true));
    }

    #[test]
    fn knob_parsing() {
        assert!(tensor_delta_from_env(Some("1")));
        assert!(!tensor_delta_from_env(Some("0")));
        assert!(!tensor_delta_from_env(None));
    }

    #[test]
    fn walker_emits_header_then_fixed_chunks() {
        let a = noise(CHUNK_SIZE + 5000, 1);
        let b = noise(CHUNK_SIZE, 2);
        let f = make_safetensors(&[("a", "F32", &a[..a.len() / 4 * 4]), ("b", "BF16", &b)], 0);
        let chunks = chunk_slice(&f).unwrap();
        let hl = u64::from_le_bytes(f[..8].try_into().unwrap()) + 8;
        assert_eq!(chunks[0].chunk_type, ChunkType::Metadata);
        assert_eq!((chunks[0].offset, chunks[0].size as u64), (0, hl));
        assert!(
            chunks[1..]
                .iter()
                .all(|c| c.chunk_type == ChunkType::Generic)
        );
        assert!(
            chunks[1..chunks.len() - 1]
                .iter()
                .all(|c| c.size == CHUNK_SIZE)
        );
        let mut joined = Vec::new();
        for c in &chunks {
            assert_eq!(c.offset, joined.len() as u64);
            joined.extend_from_slice(&c.data);
        }
        assert_eq!(joined, f);
    }

    #[test]
    fn file_walker_matches_slice_walker() {
        let a = noise(3 * CHUNK_SIZE + 12, 3);
        let f = make_safetensors(&[("a", "I64", &a[..a.len() / 8 * 8])], 0);
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("m.safetensors");
        std::fs::write(&p, &f).unwrap();
        let mut got = Vec::new();
        let mut file = std::fs::File::open(&p).unwrap();
        chunk_file(&mut file, f.len() as u64, &mut |c| {
            got.push(c);
            Ok(())
        })
        .unwrap();
        let want = chunk_slice(&f).unwrap();
        assert_eq!(
            got.iter().map(|c| (c.id, c.offset)).collect::<Vec<_>>(),
            want.iter().map(|c| (c.id, c.offset)).collect::<Vec<_>>()
        );
    }

    #[test]
    fn data_chunks_stable_under_header_length_change() {
        let a = noise(2 * CHUNK_SIZE, 4);
        let f1 = make_safetensors(&[("a", "F32", &a)], 0);
        let f2 = make_safetensors(&[("a", "F32", &a)], 700);
        let (c1, c2) = (chunk_slice(&f1).unwrap(), chunk_slice(&f2).unwrap());
        assert_ne!(c1[0].size, c2[0].size);
        let rel = |cs: &[ContentChunk]| {
            cs[1..]
                .iter()
                .map(|c| (c.offset - cs[0].size as u64, c.id))
                .collect::<Vec<_>>()
        };
        assert_eq!(rel(&c1), rel(&c2));
    }

    #[test]
    fn corrupt_or_truncated_header_declines() {
        let a = noise(4096, 5);
        let f = make_safetensors(&[("a", "F32", &a)], 0);
        assert!(chunk_slice(&f[..20]).is_err()); // truncated inside header
        let mut huge = f.clone();
        huge[..8].copy_from_slice(&u64::MAX.to_le_bytes());
        assert!(chunk_slice(&huge).is_err());
        let mut junk = f.clone();
        junk[8] = b'!'; // break the JSON (opening brace)
        assert!(chunk_slice(&junk).is_err());
        assert!(chunk_slice(&[]).is_err());
    }

    #[test]
    fn plane_params_follow_dtype_table() {
        let a = vec![0u8; 4000]; // F32 tensor at rel 0..4000
        let b = vec![0u8; 1002]; // BF16 tensor at rel 4000..5002
        let f = make_safetensors(&[("a", "F32", &a), ("b", "BF16", &b)], 0);
        let hl = u64::from_le_bytes(f[..8].try_into().unwrap());
        let l = TensorLayout::parse(&f[..(8 + hl) as usize]).unwrap();
        assert_eq!(l.plane_params(0), (4, 0));
        assert_eq!(l.plane_params(3999), (4, 3));
        assert_eq!(l.plane_params(4001), (2, 1));
        assert_eq!(l.plane_params(9999), (1, 0));
    }

    #[test]
    fn fingerprint_ignores_padding_but_not_layout() {
        let a = noise(4096, 6);
        let l = |tensors: &[(&str, &str, &[u8])], pad| {
            let f = make_safetensors(tensors, pad);
            let hl = u64::from_le_bytes(f[..8].try_into().unwrap());
            TensorLayout::parse(&f[..(8 + hl) as usize])
                .unwrap()
                .fingerprint
        };
        assert_eq!(l(&[("a", "F32", &a)], 0), l(&[("a", "F32", &a)], 50));
        assert_ne!(l(&[("a", "F32", &a)], 0), l(&[("b", "F32", &a)], 0));
        assert_ne!(l(&[("a", "F32", &a)], 0), l(&[("a", "F32", &a[..2048])], 0));
    }
}
