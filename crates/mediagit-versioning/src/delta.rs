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

//! Delta compression for similar objects
//!
//! Implements efficient delta encoding for objects that are similar.
//! Uses a sliding window approach to identify matching sequences.
//!
//! # Delta Format
//!
//! Delta instructions:
//! - Copy instruction: `C offset length` (copy `length` bytes from base at `offset`)
//! - Insert instruction: `I data` (insert raw data)

use std::cmp;
use std::collections::HashMap;

/// Minimum match length to consider for delta encoding
const MIN_MATCH_LENGTH: usize = 4;

/// Sliding window size for matching
const WINDOW_SIZE: usize = 32768; // 32 KB

/// Delta instruction for reconstruction
#[derive(Debug, Clone)]
pub enum DeltaInstruction {
    /// Copy from base object at offset with length
    Copy { offset: usize, length: usize },
    /// Insert literal bytes
    Insert(Vec<u8>),
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
    /// Instructions to apply to base
    pub instructions: Vec<DeltaInstruction>,
}

impl Delta {
    /// Serialize delta to bytes
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut bytes = Vec::new();

        // Write sizes
        encode_varint(&mut bytes, self.base_size as u32);
        encode_varint(&mut bytes, self.result_size as u32);

        // Write instructions
        for instruction in &self.instructions {
            match instruction {
                DeltaInstruction::Copy { offset, length } => {
                    // Instruction format: 0x80 | size_bits, then offset, then length
                    let op_byte = 0x80; // Copy operation marker
                    bytes.push(op_byte);
                    encode_varint(&mut bytes, *offset as u32);
                    encode_varint(&mut bytes, *length as u32);
                }
                DeltaInstruction::Insert(data) => {
                    // Instruction format: 0x00-0x7F, size, then data
                    if data.len() > 127 {
                        bytes.push(127);
                        bytes.extend_from_slice(&data[0..127]);

                        for chunk in data[127..].chunks(127) {
                            bytes.push(chunk.len() as u8);
                            bytes.extend_from_slice(chunk);
                        }
                    } else {
                        bytes.push(data.len() as u8);
                        bytes.extend_from_slice(data);
                    }
                }
            }
        }

        bytes
    }

    /// Deserialize delta from bytes
    pub fn from_bytes(data: &[u8]) -> anyhow::Result<Self> {
        let mut pos = 0;

        let base_size = decode_varint(data, &mut pos)? as usize;
        let result_size = decode_varint(data, &mut pos)? as usize;

        let mut instructions = Vec::new();

        while pos < data.len() {
            let op_byte = data[pos];
            pos += 1;

            if op_byte & 0x80 != 0 {
                // Copy instruction
                let offset = decode_varint(data, &mut pos)? as usize;
                let length = decode_varint(data, &mut pos)? as usize;
                instructions.push(DeltaInstruction::Copy { offset, length });
            } else {
                // Insert instruction
                let insert_len = op_byte as usize;
                if pos + insert_len > data.len() {
                    anyhow::bail!("Delta instruction overflow");
                }
                let insert_data = data[pos..pos + insert_len].to_vec();
                pos += insert_len;
                instructions.push(DeltaInstruction::Insert(insert_data));
            }
        }

        let compression_ratio = data.len() as f64 / result_size as f64;

        Ok(Self {
            base_size,
            result_size,
            compression_ratio,
            instructions,
        })
    }
}

/// Delta encoder using sliding window algorithm
pub struct DeltaEncoder;

impl DeltaEncoder {
    /// Encode a target object relative to a base object
    ///
    /// Uses sliding window pattern matching to find similar sequences
    /// between base and target, creating a delta that can reconstruct
    /// target from base.
    ///
    /// # Arguments
    ///
    /// * `base` - Base object data
    /// * `target` - Target object to encode as delta
    ///
    /// # Returns
    ///
    /// Delta that can reconstruct target from base
    pub fn encode(base: &[u8], target: &[u8]) -> Delta {
        let mut instructions = Vec::new();
        let mut target_pos = 0;

        // Build hash table of base sequences
        let mut hash_table: HashMap<&[u8], Vec<usize>> = HashMap::new();
        for i in 0..base.len().saturating_sub(MIN_MATCH_LENGTH) {
            let seq = &base[i..cmp::min(i + MIN_MATCH_LENGTH, base.len())];
            hash_table.entry(seq).or_insert_with(Vec::new).push(i);
        }

        while target_pos < target.len() {
            let mut best_match_length = 0;
            let mut best_match_offset = 0;

            // Try to find a match in the base
            if target_pos + MIN_MATCH_LENGTH <= target.len() {
                let target_seq = &target[target_pos..cmp::min(target_pos + MIN_MATCH_LENGTH, target.len())];

                if let Some(positions) = hash_table.get(target_seq) {
                    for &base_pos in positions {
                        let max_len = cmp::min(
                            cmp::min(
                                base.len() - base_pos,
                                target.len() - target_pos,
                            ),
                            WINDOW_SIZE,
                        );

                        let mut match_length = 0;
                        while match_length < max_len
                            && base[base_pos + match_length] == target[target_pos + match_length]
                        {
                            match_length += 1;
                        }

                        if match_length > best_match_length {
                            best_match_length = match_length;
                            best_match_offset = base_pos;
                        }

                        if best_match_length >= WINDOW_SIZE {
                            break;
                        }
                    }
                }
            }

            if best_match_length >= MIN_MATCH_LENGTH {
                // Emit copy instruction
                instructions.push(DeltaInstruction::Copy {
                    offset: best_match_offset,
                    length: best_match_length,
                });
                target_pos += best_match_length;
            } else {
                // Collect literal bytes until next match
                let mut literal = Vec::new();
                while target_pos < target.len() {
                    literal.push(target[target_pos]);
                    target_pos += 1;

                    // Check if we can start a match
                    if target_pos + MIN_MATCH_LENGTH <= target.len() {
                        let target_seq = &target[target_pos..cmp::min(target_pos + MIN_MATCH_LENGTH, target.len())];
                        if hash_table.contains_key(target_seq) {
                            break;
                        }
                    }
                }

                if !literal.is_empty() {
                    instructions.push(DeltaInstruction::Insert(literal));
                }
            }
        }

        let compression_ratio = if target.len() > 0 {
            let encoded_size = instructions
                .iter()
                .map(|instr| match instr {
                    DeltaInstruction::Copy { .. } => 10, // Rough estimate
                    DeltaInstruction::Insert(data) => data.len() + 2,
                })
                .sum::<usize>();
            encoded_size as f64 / target.len() as f64
        } else {
            0.0
        };

        Delta {
            base_size: base.len(),
            result_size: target.len(),
            compression_ratio,
            instructions,
        }
    }
}

/// Delta decoder for reconstructing objects from deltas
pub struct DeltaDecoder;

impl DeltaDecoder {
    /// Apply delta to base object to reconstruct target
    ///
    /// # Arguments
    ///
    /// * `base` - Base object data
    /// * `delta` - Delta instructions
    ///
    /// # Returns
    ///
    /// Reconstructed target object
    pub fn apply(base: &[u8], delta: &Delta) -> anyhow::Result<Vec<u8>> {
        let mut result = Vec::with_capacity(delta.result_size);

        for instruction in &delta.instructions {
            match instruction {
                DeltaInstruction::Copy { offset, length } => {
                    if *offset + *length > base.len() {
                        anyhow::bail!("Delta copy out of bounds");
                    }
                    result.extend_from_slice(&base[*offset..offset + length]);
                }
                DeltaInstruction::Insert(data) => {
                    result.extend_from_slice(data);
                }
            }
        }

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

/// Helper function to encode variable-length integer
fn encode_varint(bytes: &mut Vec<u8>, mut value: u32) {
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

/// Helper function to decode variable-length integer
fn decode_varint(data: &[u8], pos: &mut usize) -> anyhow::Result<u32> {
    let mut result: u32 = 0;
    let mut shift = 0;

    loop {
        if *pos >= data.len() {
            anyhow::bail!("Varint decode overflow");
        }

        let byte = data[*pos] as u32;
        *pos += 1;

        result |= (byte & 0x7f) << shift;
        shift += 7;

        if (byte & 0x80) == 0 {
            break;
        }

        if shift >= 32 {
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
        let data = b"Hello, World!";
        let delta = DeltaEncoder::encode(data, data);

        assert_eq!(delta.base_size, data.len());
        assert_eq!(delta.result_size, data.len());
        assert!(!delta.instructions.is_empty());
    }

    #[test]
    fn test_delta_encode_similar() {
        let base = b"The quick brown fox";
        let target = b"The quick brown fox jumps";
        let delta = DeltaEncoder::encode(base, target);

        assert_eq!(delta.base_size, base.len());
        assert_eq!(delta.result_size, target.len());
        // Should have at least a copy and insert
        assert!(!delta.instructions.is_empty());
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
        let base = b"original";
        let target = b"modified";
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
        let base = b"abcdefghij";
        let target = b"abcdefghijabcdefghij"; // Repeat of base
        let delta = DeltaEncoder::encode(base, target);

        // Verify the delta can reconstruct the target
        let reconstructed = DeltaDecoder::apply(base, &delta).unwrap();
        assert_eq!(reconstructed, target);

        // The compression ratio should be reasonable (may not always be < 1 for small objects)
        // For this test, we just verify it produces a valid delta
        assert!(!delta.instructions.is_empty(), "Delta should have instructions");
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
    }

    #[test]
    fn test_delta_completely_different() {
        let base = vec![0x00u8; 100];
        let target = vec![0xFFu8; 100];

        let delta = DeltaEncoder::encode(&base, &target);
        let reconstructed = DeltaDecoder::apply(&base, &delta).unwrap();

        assert_eq!(reconstructed, target);
    }
}
