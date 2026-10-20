// SPDX-License-Identifier: BUSL-1.1
// Copyright (C) 2025-2026 Aswin Krishnamoorthy

//! Delta compression using zstd dictionary mode
//!
//! Uses the base object as a zstd dictionary to compress the target object.
//! This produces compact deltas for similar binary data (media file chunks)
//! while being significantly faster than suffix-array approaches.
//!
//! # Wire Format
//!
//! ```text
//! [0x5A, 0x44] magic ("ZD")
//! varint(base_size)
//! varint(result_size)
//! [zstd-compressed target using base as raw dictionary]
//! ```

/// Magic bytes identifying zstd-dict delta format
const ZSTD_DICT_MAGIC: [u8; 2] = [0x5A, 0x44]; // "ZD"

/// Default zstd compression level for delta encoding.
///
/// The comment here used to read "excellent ratio for dictionary mode without
/// extreme CPU cost". The second half was never measured and is false: level 19
/// costs **238 ms per 1 MB chunk**, which on a 357 MB PSD is 85 s of the 86 s
/// `add` wall at one worker. zstd's own default is 3, and 19 is two steps off
/// the 22 maximum.
///
/// Left at 19 because storage savings are this project's differentiator and
/// lowering it silently would trade them away. It is now a knob so the
/// ratio-versus-CPU curve can be measured on real data instead of asserted.
const DEFAULT_ZSTD_DICT_LEVEL: i32 = 19;

/// `MEDIAGIT_DELTA_LEVEL` overrides the delta compression level (1-22).
///
/// Read once. Out-of-range or unparseable values fall back to the default
/// rather than failing: a bad knob must not make a repo unwritable, and a
/// silently clamped level would misreport what a measurement actually ran at.
fn zstd_dict_level() -> i32 {
    static LEVEL: std::sync::OnceLock<i32> = std::sync::OnceLock::new();
    *LEVEL.get_or_init(|| {
        match std::env::var("MEDIAGIT_DELTA_LEVEL")
            .ok()
            .and_then(|v| v.trim().parse::<i32>().ok())
        {
            Some(n) if (1..=22).contains(&n) => n,
            Some(bad) => {
                tracing::warn!(
                    "MEDIAGIT_DELTA_LEVEL={bad} is outside 1-22; using {DEFAULT_ZSTD_DICT_LEVEL}"
                );
                DEFAULT_ZSTD_DICT_LEVEL
            }
            None => DEFAULT_ZSTD_DICT_LEVEL,
        }
    })
}

/// Magic bytes identifying the XOR-planes delta format
const XOR_PLANES_MAGIC: [u8; 2] = [0x58, 0x50]; // "XP"

/// Only XP wire version this build reads and writes.
const XOR_PLANES_VERSION: u8 = 1;

/// zstd level for the XP payload. The XOR residual is mostly zeros, so a
/// cheap level is enough.
const XOR_PLANES_ZSTD_LEVEL: i32 = 3;

/// Which codec produced a delta's payload.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DeltaCodec {
    /// zstd with the base as a raw dictionary ("ZD")
    ZstdDict,
    /// `target ^ base`, byte-plane split with stride `w` at `phase`, zstd ("XP")
    XorPlanes { w: u8, phase: u8 },
}

/// Delta encoding result
#[derive(Debug, Clone)]
pub struct Delta {
    /// Base object size
    pub base_size: usize,
    /// Resulting object size after applying delta
    pub result_size: usize,
    /// Compression ratio of delta vs original
    pub compression_ratio: f64,
    /// Codec that produced `zstd_data`
    codec: DeltaCodec,
    /// Compressed payload (zstd-dict for ZD, zstd of planes(xor) for XP)
    zstd_data: Vec<u8>,
}

impl Delta {
    /// Serialize delta to bytes
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(8 + self.zstd_data.len());
        match self.codec {
            DeltaCodec::ZstdDict => bytes.extend_from_slice(&ZSTD_DICT_MAGIC),
            DeltaCodec::XorPlanes { w, phase } => {
                bytes.extend_from_slice(&XOR_PLANES_MAGIC);
                bytes.extend_from_slice(&[XOR_PLANES_VERSION, w, phase]);
            }
        }
        encode_varint(&mut bytes, self.base_size as u64);
        encode_varint(&mut bytes, self.result_size as u64);
        bytes.extend_from_slice(&self.zstd_data);
        bytes
    }

    /// Deserialize delta from bytes
    pub fn from_bytes(data: &[u8]) -> anyhow::Result<Self> {
        let codec;
        let mut pos = 2; // skip magic
        if data.len() >= 2 && data[..2] == ZSTD_DICT_MAGIC {
            codec = DeltaCodec::ZstdDict;
        } else if data.len() >= 2 && data[..2] == XOR_PLANES_MAGIC {
            if data.len() < 5 {
                anyhow::bail!("Invalid XP delta: truncated header");
            }
            if data[2] != XOR_PLANES_VERSION {
                anyhow::bail!("Unsupported XP delta version {}", data[2]);
            }
            let (w, phase) = (data[3], data[4]);
            if !(1..=8).contains(&w) || phase >= w {
                anyhow::bail!("Invalid XP delta parameters: w={w} phase={phase}");
            }
            codec = DeltaCodec::XorPlanes { w, phase };
            pos = 5;
        } else {
            anyhow::bail!("Invalid delta format: missing ZD or XP magic bytes");
        }

        let base_size = decode_varint(data, &mut pos)? as usize;
        let result_size = decode_varint(data, &mut pos)? as usize;
        let zstd_data = data[pos..].to_vec();
        let compression_ratio = data.len() as f64 / result_size.max(1) as f64;

        Ok(Self {
            base_size,
            result_size,
            compression_ratio,
            codec,
            zstd_data,
        })
    }
}

/// `target ^ base` over the overlap; target bytes past `base.len()` kept as-is.
fn xor_residual(base: &[u8], target: &[u8]) -> Vec<u8> {
    let mut out = target.to_vec();
    for (o, b) in out.iter_mut().zip(base) {
        *o ^= *b;
    }
    out
}

/// Stable byte-plane split: byte `j` goes to plane `(j + phase) % w`.
fn planes(data: &[u8], w: usize, phase: usize) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len());
    for p in 0..w {
        // first j with (j + phase) % w == p
        let start = (p + w - phase % w) % w;
        out.extend(data.iter().skip(start).step_by(w));
    }
    out
}

/// Exact inverse of [`planes`].
fn unplanes(data: &[u8], w: usize, phase: usize) -> Vec<u8> {
    let mut out = vec![0u8; data.len()];
    let mut src = data.iter();
    for p in 0..w {
        let start = (p + w - phase % w) % w;
        for slot in out.iter_mut().skip(start).step_by(w) {
            // lengths match by construction
            *slot = *src.next().expect("plane length invariant");
        }
    }
    out
}

/// Delta encoder using zstd dictionary compression
pub struct DeltaEncoder;

impl DeltaEncoder {
    /// Encode a target object relative to a base object using zstd dictionary mode.
    ///
    /// Uses the base as a raw zstd dictionary to compress the target.
    pub fn encode(base: &[u8], target: &[u8]) -> Delta {
        if target.is_empty() {
            return Delta {
                base_size: base.len(),
                result_size: 0,
                compression_ratio: 0.0,
                codec: DeltaCodec::ZstdDict,
                zstd_data: Vec::new(),
            };
        }

        let zstd_data = match Self::compress_with_dict(base, target) {
            Ok(data) => data,
            Err(_) => {
                // Fallback: store target as-is (uncompressed)
                target.to_vec()
            }
        };

        let compression_ratio = zstd_data.len() as f64 / target.len() as f64;

        Delta {
            base_size: base.len(),
            result_size: target.len(),
            compression_ratio,
            codec: DeltaCodec::ZstdDict,
            zstd_data,
        }
    }

    /// Encode with the XP codec: `zstd(planes(target ^ base, w, phase))`.
    ///
    /// Invalid `w` (not 1..=8) or `phase >= w` falls back to `w = 1, phase = 0`
    /// (plain XOR) instead of failing, matching `encode`'s infallible signature.
    pub fn encode_xor_planes(base: &[u8], target: &[u8], w: u8, phase: u8) -> Delta {
        let (w, phase) = if (1..=8).contains(&w) && phase < w {
            (w, phase)
        } else {
            (1, 0)
        };
        let codec = DeltaCodec::XorPlanes { w, phase };
        if target.is_empty() {
            return Delta {
                base_size: base.len(),
                result_size: 0,
                compression_ratio: 0.0,
                codec,
                zstd_data: Vec::new(),
            };
        }

        let split = planes(&xor_residual(base, target), w as usize, phase as usize);
        let zstd_data = zstd::bulk::compress(&split, XOR_PLANES_ZSTD_LEVEL)
            .expect("in-memory zstd compression cannot fail");
        let compression_ratio = zstd_data.len() as f64 / target.len() as f64;

        Delta {
            base_size: base.len(),
            result_size: target.len(),
            compression_ratio,
            codec,
            zstd_data,
        }
    }

    /// Compress target using base as zstd dictionary
    fn compress_with_dict(base: &[u8], target: &[u8]) -> anyhow::Result<Vec<u8>> {
        let mut encoder = zstd::bulk::Compressor::with_dictionary(zstd_dict_level(), base)?;
        let compressed = encoder.compress(target)?;
        Ok(compressed)
    }
}

/// Delta decoder for reconstructing objects from deltas
pub struct DeltaDecoder;

impl DeltaDecoder {
    /// Apply delta to base object to reconstruct target.
    pub fn apply(base: &[u8], delta: &Delta) -> anyhow::Result<Vec<u8>> {
        const MAX_DELTA_RESULT_SIZE: usize = 16 * 1024 * 1024 * 1024; // 16 GB
        if delta.result_size > MAX_DELTA_RESULT_SIZE {
            anyhow::bail!(
                "Delta result_size {} exceeds maximum {} bytes",
                delta.result_size,
                MAX_DELTA_RESULT_SIZE
            );
        }

        if delta.zstd_data.is_empty() {
            return Ok(Vec::new());
        }

        if let DeltaCodec::XorPlanes { w, phase } = delta.codec {
            if delta.base_size != base.len() {
                anyhow::bail!(
                    "XP delta base size mismatch: delta expects {}, got {}",
                    delta.base_size,
                    base.len()
                );
            }
            let split = zstd::bulk::decompress(&delta.zstd_data, delta.result_size)
                .map_err(|e| anyhow::anyhow!("Failed to decompress XP delta: {}", e))?;
            if split.len() != delta.result_size {
                anyhow::bail!(
                    "Delta reconstruction size mismatch: {} != {}",
                    split.len(),
                    delta.result_size
                );
            }
            let mut out = unplanes(&split, w as usize, phase as usize);
            for (o, b) in out.iter_mut().zip(base) {
                *o ^= *b;
            }
            return Ok(out);
        }

        let mut decoder = zstd::bulk::Decompressor::with_dictionary(base)?;
        let result = decoder
            .decompress(&delta.zstd_data, delta.result_size)
            .map_err(|e| anyhow::anyhow!("Failed to decompress zstd-dict delta: {}", e))?;

        if result.len() != delta.result_size {
            anyhow::bail!(
                "Delta reconstruction size mismatch: {} != {}",
                result.len(),
                delta.result_size
            );
        }

        Ok(result)
    }
}

/// Helper function to encode variable-length integer (u64 to support objects > 4GB)
fn encode_varint(bytes: &mut Vec<u8>, mut value: u64) {
    loop {
        let mut byte = (value & 0x7f) as u8;
        value >>= 7;

        if value != 0 {
            byte |= 0x80;
        }

        bytes.push(byte);

        if value == 0 {
            break;
        }
    }
}

/// Helper function to decode variable-length integer (u64 to support objects > 4GB)
fn decode_varint(data: &[u8], pos: &mut usize) -> anyhow::Result<u64> {
    let mut result: u64 = 0;
    let mut shift = 0;

    loop {
        if *pos >= data.len() {
            anyhow::bail!("Varint decode overflow");
        }

        let byte = data[*pos] as u64;
        *pos += 1;

        result |= (byte & 0x7f) << shift;
        shift += 7;

        if (byte & 0x80) == 0 {
            break;
        }

        if shift >= 64 {
            anyhow::bail!("Varint too large");
        }
    }

    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_delta_encode_identical() {
        let data: Vec<u8> = (0..4096).map(|i| (i % 256) as u8).collect();
        let delta = DeltaEncoder::encode(&data, &data);

        assert_eq!(delta.base_size, data.len());
        assert_eq!(delta.result_size, data.len());

        let bytes = delta.to_bytes();
        assert!(bytes.len() < data.len());
    }

    #[test]
    fn test_delta_encode_similar() {
        let base = b"The quick brown fox jumps over the lazy dog";
        let target = b"The quick brown cat jumps over the lazy dog today";
        let delta = DeltaEncoder::encode(base, target);

        assert_eq!(delta.base_size, base.len());
        assert_eq!(delta.result_size, target.len());
    }

    #[test]
    fn test_delta_roundtrip() {
        let base = b"The quick brown fox";
        let target = b"The quick brown fox jumps";
        let delta = DeltaEncoder::encode(base, target);

        let reconstructed = DeltaDecoder::apply(base, &delta).unwrap();
        assert_eq!(reconstructed, target);
    }

    #[test]
    fn test_delta_serialize_deserialize() {
        let base = b"original data here for testing purposes";
        let target = b"modified data here for testing purposes today";
        let delta = DeltaEncoder::encode(base, target);

        let bytes = delta.to_bytes();
        let decoded = Delta::from_bytes(&bytes).unwrap();

        assert_eq!(decoded.base_size, delta.base_size);
        assert_eq!(decoded.result_size, delta.result_size);

        let reconstructed = DeltaDecoder::apply(base, &decoded).unwrap();
        assert_eq!(reconstructed, target);
    }

    #[test]
    fn test_delta_compression_ratio() {
        let base = vec![42u8; 10000];
        let mut target = base.clone();
        target[5000] = 99;
        let delta = DeltaEncoder::encode(&base, &target);

        let reconstructed = DeltaDecoder::apply(&base, &delta).unwrap();
        assert_eq!(reconstructed, target);

        let bytes = delta.to_bytes();
        assert!(
            bytes.len() < target.len() / 2,
            "Delta should be < 50% of target, was {}%",
            bytes.len() * 100 / target.len()
        );
    }

    #[test]
    fn test_varint_encode_decode() {
        let mut bytes = Vec::new();
        encode_varint(&mut bytes, 127);
        encode_varint(&mut bytes, 128);
        encode_varint(&mut bytes, 16383);

        let mut pos = 0;
        assert_eq!(decode_varint(&bytes, &mut pos).unwrap(), 127);
        assert_eq!(decode_varint(&bytes, &mut pos).unwrap(), 128);
        assert_eq!(decode_varint(&bytes, &mut pos).unwrap(), 16383);
    }

    #[test]
    fn test_delta_large_objects() {
        let base = vec![0x42u8; 100000];
        let mut target = vec![0x42u8; 100000];
        target[50000] = 0x43;

        let delta = DeltaEncoder::encode(&base, &target);
        let reconstructed = DeltaDecoder::apply(&base, &delta).unwrap();
        assert_eq!(reconstructed, target);

        let bytes = delta.to_bytes();
        assert!(
            bytes.len() < 1000,
            "Delta of 100KB with 1 byte diff should be tiny, was {} bytes",
            bytes.len()
        );
    }

    #[test]
    fn test_delta_completely_different() {
        let base = vec![0x00u8; 100];
        let target = vec![0xFFu8; 100];

        let delta = DeltaEncoder::encode(&base, &target);
        let reconstructed = DeltaDecoder::apply(&base, &delta).unwrap();
        assert_eq!(reconstructed, target);
    }

    #[test]
    fn test_delta_empty_target() {
        let base = b"some data";
        let delta = DeltaEncoder::encode(base, b"");
        let reconstructed = DeltaDecoder::apply(base, &delta).unwrap();
        assert_eq!(reconstructed, b"");
    }

    #[test]
    fn test_delta_empty_base() {
        let delta = DeltaEncoder::encode(b"", b"some target");
        let reconstructed = DeltaDecoder::apply(b"", &delta).unwrap();
        assert_eq!(reconstructed, b"some target");
    }

    #[test]
    fn test_format_magic_bytes() {
        let base = b"test base data for format detection";
        let target = b"test target data for format detection";
        let delta = DeltaEncoder::encode(base, target);

        let bytes = delta.to_bytes();
        assert_eq!(bytes[0], 0x5A);
        assert_eq!(bytes[1], 0x44);

        let decoded = Delta::from_bytes(&bytes).unwrap();
        assert_eq!(decoded.base_size, base.len());
        assert_eq!(decoded.result_size, target.len());
    }

    #[test]
    fn test_zstd_dict_quality_scattered_matches() {
        let mut base = Vec::with_capacity(4096);
        let mut target = Vec::with_capacity(4096);

        for i in 0u8..64 {
            base.extend_from_slice(&[i; 32]);
            base.extend_from_slice(b"MATCH");
            base.extend_from_slice(&[i.wrapping_mul(7).wrapping_add(3); 26]);
        }

        for i in 0u8..64 {
            target.extend_from_slice(&[255u8.wrapping_sub(i); 20]);
            target.extend_from_slice(b"MATCH");
            target.extend_from_slice(&[i.wrapping_mul(13).wrapping_add(7); 38]);
        }

        let delta = DeltaEncoder::encode(&base, &target);
        let reconstructed = DeltaDecoder::apply(&base, &delta).unwrap();
        assert_eq!(reconstructed, target);

        let bytes = delta.to_bytes();
        assert!(
            bytes.len() < target.len(),
            "Delta should be smaller than target: {} vs {}",
            bytes.len(),
            target.len()
        );
    }

    #[test]
    fn test_zstd_dict_media_chunk_simulation() {
        let mut base = vec![0u8; 1024 * 1024]; // 1MB
        for (i, byte) in base.iter_mut().enumerate() {
            *byte = ((i * 31337 + 12345) % 256) as u8;
        }

        let mut target = base.clone();
        for i in (0..target.len()).step_by(20) {
            target[i] = target[i].wrapping_add(1);
        }

        let delta = DeltaEncoder::encode(&base, &target);
        let reconstructed = DeltaDecoder::apply(&base, &delta).unwrap();
        assert_eq!(reconstructed, target);

        let bytes = delta.to_bytes();
        let ratio = bytes.len() as f64 / target.len() as f64;
        assert!(
            ratio < 0.80,
            "Delta of 95% similar 1MB chunks should be < 80%, was {:.1}%",
            ratio * 100.0
        );
    }

    fn pseudo(len: usize, seed: u32) -> Vec<u8> {
        let mut s = seed.wrapping_mul(2654435761).wrapping_add(1);
        (0..len)
            .map(|_| {
                s ^= s << 13;
                s ^= s >> 17;
                s ^= s << 5;
                (s >> 8) as u8
            })
            .collect()
    }

    #[test]
    fn test_xp_roundtrip_matrix() {
        for w in [1u8, 2, 4, 8] {
            for phase in 0..w {
                let wu = w as usize;
                for len in [0, 1, wu - 1, wu + 1, 1001] {
                    let target = pseudo(len, len as u32 + 7);
                    let mut bases = vec![Vec::new(), pseudo(len, 99)];
                    bases.push(pseudo(len / 2, 5));
                    bases.push(pseudo(len + 13, 6));
                    for base in bases {
                        let d = DeltaEncoder::encode_xor_planes(&base, &target, w, phase);
                        let d = Delta::from_bytes(&d.to_bytes()).unwrap();
                        assert_eq!(
                            DeltaDecoder::apply(&base, &d).unwrap(),
                            target,
                            "w={w} phase={phase} len={len} base={}",
                            base.len()
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn test_xp_invalid_params_fall_back_to_plain_xor() {
        let (base, target) = (pseudo(100, 1), pseudo(100, 2));
        for (w, p) in [(0u8, 0u8), (9, 0), (4, 4)] {
            let d = DeltaEncoder::encode_xor_planes(&base, &target, w, p);
            let bytes = d.to_bytes();
            assert_eq!(&bytes[..5], &[b'X', b'P', 1, 1, 0]);
            assert_eq!(DeltaDecoder::apply(&base, &d).unwrap(), target);
        }
    }

    #[test]
    fn test_xp_rejects_wrong_base_size_and_bad_header() {
        let (base, target) = (pseudo(64, 1), pseudo(64, 2));
        let d = DeltaEncoder::encode_xor_planes(&base, &target, 4, 1);
        assert!(DeltaDecoder::apply(&base[..63], &d).is_err());

        let mut bytes = d.to_bytes();
        bytes[2] = 2; // unknown version
        assert!(Delta::from_bytes(&bytes).is_err());
        assert!(Delta::from_bytes(b"QQ\x00\x00\x00").is_err());
        assert!(Delta::from_bytes(b"XP").is_err());
    }

    /// Pre-change ZD-only parser, copied verbatim: old binaries must error on XP.
    fn old_zd_only_from_bytes(data: &[u8]) -> anyhow::Result<()> {
        if data.len() < 2 || data[0] != ZSTD_DICT_MAGIC[0] || data[1] != ZSTD_DICT_MAGIC[1] {
            anyhow::bail!("Invalid delta format: missing ZD magic bytes");
        }
        Ok(())
    }

    #[test]
    fn test_xp_rejected_by_pre_change_parser() {
        let d = DeltaEncoder::encode_xor_planes(b"abcd", b"abce", 2, 0);
        assert!(old_zd_only_from_bytes(&d.to_bytes()).is_err());
    }

    #[test]
    fn test_zd_wire_bytes_golden() {
        let delta = DeltaEncoder::encode(
            b"The quick brown fox jumps over the lazy dog",
            b"The quick brown cat jumps over the lazy dog today",
        );
        let expected: &[u8] = &[
            90, 68, 43, 49, 40, 181, 47, 253, 32, 49, 149, 0, 0, 80, 84, 99, 97, 116, 32, 116, 111,
            100, 97, 121, 2, 0, 0, 28, 215, 169, 43,
        ];
        assert_eq!(delta.to_bytes(), expected);
    }

    #[test]
    fn test_xp_beats_zd_on_perturbed_floats() {
        let base: Vec<u8> = (0..16384u32)
            .flat_map(|i| (1000.0f32 + (i as f32) * 0.37).to_le_bytes())
            .collect();
        let target: Vec<u8> = base
            .chunks(4)
            .enumerate()
            .flat_map(|(i, c)| {
                let v = f32::from_le_bytes([c[0], c[1], c[2], c[3]])
                    + (((i * 2654435761) % 1000) as f32) * 1e-6;
                v.to_le_bytes()
            })
            .collect();
        let xp = DeltaEncoder::encode_xor_planes(&base, &target, 4, 0).to_bytes();
        let zd = DeltaEncoder::encode(&base, &target).to_bytes();
        assert!(xp.len() < zd.len(), "xp {} vs zd {}", xp.len(), zd.len());
    }
}
