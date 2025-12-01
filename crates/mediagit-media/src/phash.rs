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

//! Perceptual hashing for image similarity detection
//!
//! This module provides perceptual hashing (pHash) algorithms for detecting
//! similar images even when they've been slightly modified, resized, or compressed.
//!
//! # Features
//!
//! - Multiple perceptual hash algorithms (average, difference, perceptual)
//! - Hamming distance calculation for similarity scoring
//! - Fast duplicate detection (<50ms per image)
//! - Robust to minor image modifications
//!
//! # Example
//!
//! ```rust,no_run
//! use mediagit_media::phash::PerceptualHasher;
//!
//! # async fn example() -> anyhow::Result<()> {
//! let hasher = PerceptualHasher::new();
//!
//! let image1 = std::fs::read("image1.jpg")?;
//! let image2 = std::fs::read("image2.jpg")?;
//!
//! let hash1 = hasher.hash(&image1).await?;
//! let hash2 = hasher.hash(&image2).await?;
//!
//! let similarity = hasher.similarity(&hash1, &hash2);
//! println!("Images are {:.1}% similar", similarity * 100.0);
//! # Ok(())
//! # }
//! ```

use crate::error::{MediaError, Result};
use image::{DynamicImage, GenericImageView};
use img_hash::HasherConfig;
use serde::{Deserialize, Serialize};
use std::time::Instant;
use tracing::{debug, info, instrument, trace};

/// Perceptual hash algorithm type
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum HashAlgorithm {
    /// Average hash (fast, good for exact duplicates)
    Average,
    /// Difference hash (fast, better for minor edits)
    Difference,
    /// Perceptual hash (slower, best for significant changes)
    Perceptual,
    /// Gradient hash (balanced performance and accuracy)
    Gradient,
}

/// Perceptual hash result
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PerceptualHash {
    /// Hash bytes
    pub hash: Vec<u8>,

    /// Algorithm used
    pub algorithm: HashAlgorithm,

    /// Hash size (in bits)
    pub hash_size: usize,

    /// Original image dimensions
    pub image_width: u32,
    pub image_height: u32,
}

impl PerceptualHash {
    /// Calculate Hamming distance to another hash
    ///
    /// Returns the number of differing bits
    pub fn hamming_distance(&self, other: &PerceptualHash) -> u32 {
        if self.algorithm != other.algorithm {
            // Different algorithms, can't compare
            return u32::MAX;
        }

        if self.hash.len() != other.hash.len() {
            return u32::MAX;
        }

        let mut distance = 0u32;
        for (a, b) in self.hash.iter().zip(other.hash.iter()) {
            distance += (a ^ b).count_ones();
        }

        distance
    }

    /// Calculate similarity score (0.0 to 1.0)
    ///
    /// 1.0 means identical, 0.0 means completely different
    pub fn similarity(&self, other: &PerceptualHash) -> f64 {
        let distance = self.hamming_distance(other);

        if distance == u32::MAX {
            return 0.0;
        }

        let max_distance = (self.hash_size) as f64;
        1.0 - (distance as f64 / max_distance)
    }
}

/// Perceptual image hasher
#[derive(Debug)]
pub struct PerceptualHasher {
    /// Hash algorithm to use
    algorithm: HashAlgorithm,

    /// Hash size (affects accuracy and performance)
    hash_size: u32,
}

impl PerceptualHasher {
    /// Create a new perceptual hasher with default settings
    ///
    /// Uses difference hash algorithm with 8x8 hash size (64 bits)
    pub fn new() -> Self {
        PerceptualHasher {
            algorithm: HashAlgorithm::Difference,
            hash_size: 8,
        }
    }

    /// Create a hasher with specific algorithm and size
    pub fn with_algorithm(algorithm: HashAlgorithm, hash_size: u32) -> Self {
        PerceptualHasher {
            algorithm,
            hash_size,
        }
    }

    /// Compute perceptual hash for an image
    #[instrument(skip(data), fields(size = data.len()))]
    pub async fn hash(&self, data: &[u8]) -> Result<PerceptualHash> {
        let start = Instant::now();

        // Load image
        let img = image::load_from_memory(data)
            .map_err(|e| MediaError::ImageError(e.to_string()))?;

        let (width, height) = img.dimensions();
        trace!("Loaded image: {}x{}", width, height);

        // Compute hash
        let hash_bytes = self.compute_hash(&img)?;

        let elapsed = start.elapsed();
        debug!(
            "Computed {:?} hash in {:?} for {}x{} image",
            self.algorithm, elapsed, width, height
        );

        Ok(PerceptualHash {
            hash: hash_bytes,
            algorithm: self.algorithm,
            hash_size: (self.hash_size * self.hash_size) as usize,
            image_width: width,
            image_height: height,
        })
    }

    /// Compute hash using img-hash crate
    fn compute_hash(&self, img: &DynamicImage) -> Result<Vec<u8>> {
        let hasher = HasherConfig::new()
            .hash_size(self.hash_size, self.hash_size)
            .hash_alg(self.algorithm_to_img_hash())
            .to_hasher();

        // img_hash::HasherConfig uses its own image re-export
        // We need to convert from our image::DynamicImage to img_hash's image type
        use img_hash::image as img_hash_image;

        // Convert to image bytes and reload with img_hash's image type
        let mut bytes = Vec::new();
        let mut cursor = std::io::Cursor::new(&mut bytes);
        img.write_to(&mut cursor, image::ImageFormat::Png)
            .map_err(|e| MediaError::ImageError(e.to_string()))?;

        let img_hash_img = img_hash_image::load_from_memory(&bytes)
            .map_err(|e| MediaError::ImageError(e.to_string()))?;

        let image_hash = hasher.hash_image(&img_hash_img);

        Ok(image_hash.as_bytes().to_vec())
    }

    /// Convert algorithm enum to img-hash algorithm
    fn algorithm_to_img_hash(&self) -> img_hash::HashAlg {
        match self.algorithm {
            HashAlgorithm::Average => img_hash::HashAlg::Mean,
            HashAlgorithm::Difference => img_hash::HashAlg::Gradient,
            HashAlgorithm::Perceptual => img_hash::HashAlg::DoubleGradient,
            HashAlgorithm::Gradient => img_hash::HashAlg::Gradient,
        }
    }

    /// Calculate similarity between two hashes (0.0 to 1.0)
    pub fn similarity(&self, hash1: &PerceptualHash, hash2: &PerceptualHash) -> f64 {
        hash1.similarity(hash2)
    }

    /// Check if two images are duplicates
    ///
    /// Uses a threshold of 0.95 similarity (95%)
    pub fn are_duplicates(&self, hash1: &PerceptualHash, hash2: &PerceptualHash) -> bool {
        self.similarity(hash1, hash2) >= 0.95
    }

    /// Check if two images are similar
    ///
    /// Uses a threshold of 0.85 similarity (85%)
    pub fn are_similar(&self, hash1: &PerceptualHash, hash2: &PerceptualHash) -> bool {
        self.similarity(hash1, hash2) >= 0.85
    }

    /// Batch compare an image against multiple candidates
    ///
    /// Returns list of (index, similarity) for matches above threshold
    pub fn find_matches(
        &self,
        target: &PerceptualHash,
        candidates: &[PerceptualHash],
        threshold: f64,
    ) -> Vec<(usize, f64)> {
        candidates
            .iter()
            .enumerate()
            .filter_map(|(idx, candidate)| {
                let similarity = self.similarity(target, candidate);
                if similarity >= threshold {
                    Some((idx, similarity))
                } else {
                    None
                }
            })
            .collect()
    }
}

impl Default for PerceptualHasher {
    fn default() -> Self {
        Self::new()
    }
}

/// Compare images for merge intelligence
pub fn compare_images_for_merge(
    base: &PerceptualHash,
    ours: &PerceptualHash,
    theirs: &PerceptualHash,
) -> ImageMergeDecision {
    let ours_similarity = base.similarity(ours);
    let theirs_similarity = base.similarity(theirs);
    let mutual_similarity = ours.similarity(theirs);

    info!(
        "Image similarity: base→ours={:.2}%, base→theirs={:.2}%, ours↔theirs={:.2}%",
        ours_similarity * 100.0,
        theirs_similarity * 100.0,
        mutual_similarity * 100.0
    );

    // If both are very similar to base, minor edits
    if ours_similarity > 0.95 && theirs_similarity > 0.95 {
        if mutual_similarity > 0.95 {
            return ImageMergeDecision::IdenticalChanges;
        }
        return ImageMergeDecision::MinorEdits;
    }

    // If both significantly different from base
    if ours_similarity < 0.8 && theirs_similarity < 0.8 {
        return ImageMergeDecision::MajorConflict;
    }

    // One side changed significantly, other didn't
    if ours_similarity < 0.8 || theirs_similarity < 0.8 {
        return ImageMergeDecision::AsymmetricChange;
    }

    ImageMergeDecision::MinorEdits
}

/// Image merge decision based on perceptual hash comparison
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImageMergeDecision {
    /// Both sides made identical or nearly identical changes
    IdenticalChanges,
    /// Both sides made minor, non-conflicting edits
    MinorEdits,
    /// One side changed significantly, the other minimally
    AsymmetricChange,
    /// Both sides made major conflicting changes
    MajorConflict,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hamming_distance() {
        let hash1 = PerceptualHash {
            hash: vec![0b11110000, 0b10101010],
            algorithm: HashAlgorithm::Difference,
            hash_size: 16,
            image_width: 100,
            image_height: 100,
        };

        let hash2 = PerceptualHash {
            hash: vec![0b11110001, 0b10101010],  // 1 bit different
            algorithm: HashAlgorithm::Difference,
            hash_size: 16,
            image_width: 100,
            image_height: 100,
        };

        assert_eq!(hash1.hamming_distance(&hash2), 1);
    }

    #[test]
    fn test_similarity_calculation() {
        let hash1 = PerceptualHash {
            hash: vec![0b11111111],
            algorithm: HashAlgorithm::Difference,
            hash_size: 8,
            image_width: 100,
            image_height: 100,
        };

        let hash2 = PerceptualHash {
            hash: vec![0b11111111],  // Identical
            algorithm: HashAlgorithm::Difference,
            hash_size: 8,
            image_width: 100,
            image_height: 100,
        };

        assert_eq!(hash1.similarity(&hash2), 1.0);
    }

    #[test]
    fn test_algorithm_serialization() {
        let algo = HashAlgorithm::Perceptual;
        let json = serde_json::to_string(&algo).unwrap();
        assert!(json.contains("Perceptual"));
    }

    #[test]
    fn test_different_algorithms_incompatible() {
        let hash1 = PerceptualHash {
            hash: vec![0b11111111],
            algorithm: HashAlgorithm::Average,
            hash_size: 8,
            image_width: 100,
            image_height: 100,
        };

        let hash2 = PerceptualHash {
            hash: vec![0b11111111],
            algorithm: HashAlgorithm::Difference,  // Different algorithm
            hash_size: 8,
            image_width: 100,
            image_height: 100,
        };

        assert_eq!(hash1.hamming_distance(&hash2), u32::MAX);
        assert_eq!(hash1.similarity(&hash2), 0.0);
    }
}
