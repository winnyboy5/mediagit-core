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

//! Similarity detection for delta compression
//!
//! This module provides efficient similarity detection between objects
//! using sampling-based comparison to find good delta base candidates.

use crate::Oid;
use std::collections::HashMap;
use tracing::{debug, info};

/// Minimum similarity threshold for delta compression (0.0 to 1.0)
pub const MIN_SIMILARITY_THRESHOLD: f64 = 0.30;

/// Get type-aware similarity threshold based on file extension
///
/// Different file types have different similarity characteristics:
/// - Text/code: High threshold (small changes matter)
/// - Configuration: Very high threshold (exact matches preferred)
/// - Images: Lower threshold (perceptual similarity)
/// - Video: Very low threshold (metadata/timeline changes)
///
/// # Arguments
///
/// * `filename` - Optional filename for type inference
///
/// # Returns
///
/// Threshold value (0.0 to 1.0)
pub fn get_similarity_threshold(filename: Option<&str>) -> f64 {
    if let Some(name) = filename {
        let ext = std::path::Path::new(name)
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("");

        match ext.to_lowercase().as_str() {
            // Creative/PDF containers: Very low threshold (embedded compressed streams
            // shift chunk boundaries, but structural similarity remains)
            "ai" | "ait" | "indd" | "idml" | "indt" | "eps" | "pdf" => 0.15,

            // Office documents (ZIP containers with shared structure)
            "docx" | "xlsx" | "pptx" | "odt" | "ods" | "odp" => 0.20,

            // Text/code: High similarity threshold (small changes matter)
            "txt" | "md" | "py" | "rs" | "js" | "ts" | "go" | "java" | "c" | "cpp" | "h" | "hpp" => 0.85,

            // Configuration: Very high threshold (exact matches preferred)
            "json" | "yaml" | "toml" | "xml" => 0.95,

            // Images: Lower threshold (perceptual similarity)
            "jpg" | "jpeg" | "png" | "psd" => 0.70,

            // Video: Very low threshold (metadata/timeline changes significant)
            "mp4" | "mov" | "avi" | "mkv" => 0.50,

            // Audio: Medium threshold
            "wav" | "aiff" | "mp3" | "flac" => 0.65,

            // 3D models: Medium threshold
            "obj" | "fbx" | "blend" | "gltf" | "glb" => 0.70,

            // Unknown: Use default
            _ => MIN_SIMILARITY_THRESHOLD,
        }
    } else {
        // No filename: use default threshold
        MIN_SIMILARITY_THRESHOLD
    }
}

/// Get type-aware size ratio threshold based on file extension
///
/// Creative files (AI, PDF) can have large size differences between
/// versions while still sharing significant internal structure.
/// A lower threshold allows delta matching across larger size changes.
///
/// # Arguments
///
/// * `filename` - Optional filename for type inference
///
/// # Returns
///
/// Size ratio threshold (0.0 to 1.0). Candidate chunks with
/// (smaller/larger) below this ratio are skipped during similarity search.
pub fn get_size_ratio_threshold(filename: Option<&str>) -> f64 {
    if let Some(name) = filename {
        let ext = std::path::Path::new(name)
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("");

        match ext.to_lowercase().as_str() {
            // Creative/PDF containers: allow 50% size difference
            "ai" | "ait" | "indd" | "idml" | "indt" | "eps" | "pdf" => 0.50,
            // Office documents: allow 40% size difference
            "docx" | "xlsx" | "pptx" | "odt" | "ods" | "odp" => 0.60,
            // Video: allow 30% size difference
            "mp4" | "mov" | "avi" | "mkv" => 0.70,
            // Default: 80% (20% max difference)
            _ => 0.80,
        }
    } else {
        0.80
    }
}

/// Maximum objects to consider for similarity matching
pub const MAX_SIMILARITY_CANDIDATES: usize = 50;

/// Sample size for quick similarity estimation
pub const SAMPLE_SIZE: usize = 1024; // 1 KB samples

/// Number of samples to take from each object
pub const SAMPLE_COUNT: usize = 10;

/// Similarity score between two objects (0.0 to 1.0)
#[derive(Debug, Clone, Copy)]
pub struct SimilarityScore {
    /// Similarity score (0.0 = completely different, 1.0 = identical)
    pub score: f64,

    /// Size ratio (smaller / larger)
    pub size_ratio: f64,

    /// Sample match count
    pub sample_matches: usize,
}

impl SimilarityScore {
    /// Create a new similarity score
    pub fn new(score: f64, size_ratio: f64, sample_matches: usize) -> Self {
        Self {
            score,
            size_ratio,
            sample_matches,
        }
    }

    /// Check if this score meets the threshold for delta compression
    pub fn is_good_match(&self) -> bool {
        self.score >= MIN_SIMILARITY_THRESHOLD
    }

    /// Check if this score meets a type-aware threshold
    ///
    /// Uses filename-based threshold if available, otherwise uses default
    ///
    /// # Arguments
    ///
    /// * `filename` - Optional filename for type-aware threshold
    ///
    /// # Returns
    ///
    /// True if score meets the threshold
    pub fn is_good_match_for_type(&self, filename: Option<&str>) -> bool {
        let threshold = get_similarity_threshold(filename);
        self.score >= threshold
    }
}

/// Object metadata for similarity detection
#[derive(Debug, Clone)]
pub struct ObjectMetadata {
    /// Object identifier
    pub oid: Oid,

    /// Object size in bytes
    pub size: usize,

    /// Object type
    pub obj_type: crate::ObjectType,

    /// Filename if available
    pub filename: Option<String>,

    /// Sample hashes for quick comparison
    pub sample_hashes: Vec<u64>,

    /// Whether this chunk was stored as a delta (avoid using as base)
    pub is_delta: bool,
}

impl ObjectMetadata {
    /// Create new object metadata
    pub fn new(
        oid: Oid,
        size: usize,
        obj_type: crate::ObjectType,
        filename: Option<String>,
    ) -> Self {
        Self {
            oid,
            size,
            obj_type,
            filename,
            sample_hashes: Vec::new(),
            is_delta: false,
        }
    }

    /// Generate sample hashes from object data
    pub fn generate_samples(&mut self, data: &[u8]) {
        if data.is_empty() {
            return;
        }

        let mut samples = Vec::with_capacity(SAMPLE_COUNT);

        // Take evenly distributed samples
        let step = data.len() / (SAMPLE_COUNT + 1);

        for i in 1..=SAMPLE_COUNT {
            let offset = i * step;
            if offset + SAMPLE_SIZE <= data.len() {
                let sample = &data[offset..offset + SAMPLE_SIZE];
                let hash = Self::hash_sample(sample);
                samples.push(hash);
            } else if offset < data.len() {
                // Last sample might be smaller
                let sample = &data[offset..];
                let hash = Self::hash_sample(sample);
                samples.push(hash);
            }
        }

        self.sample_hashes = samples;

        debug!(
            oid = %self.oid,
            samples = self.sample_hashes.len(),
            "Generated sample hashes"
        );
    }

    /// Simple hash function for samples (FNV-1a)
    fn hash_sample(data: &[u8]) -> u64 {
        let mut hash: u64 = 0xcbf29ce484222325;
        const FNV_PRIME: u64 = 0x100000001b3;

        for &byte in data {
            hash ^= byte as u64;
            hash = hash.wrapping_mul(FNV_PRIME);
        }

        hash
    }
}

/// Similarity detector for finding delta base candidates
pub struct SimilarityDetector {
    /// Recent objects for similarity matching (VecDeque for O(1) front insertion)
    recent_objects: std::collections::VecDeque<ObjectMetadata>,

    /// Maximum number of recent objects to track
    max_recent: usize,
}

impl SimilarityDetector {
    /// Create a new similarity detector
    pub fn new(max_recent: usize) -> Self {
        Self {
            recent_objects: std::collections::VecDeque::new(),
            max_recent,
        }
    }

    /// Add an object to the recent objects list
    pub fn add_object(&mut self, metadata: ObjectMetadata) {
        // Add to front of deque — O(1) vs the previous O(N) Vec::insert(0, …)
        self.recent_objects.push_front(metadata);

        // Trim oldest entry — O(1) vs the previous O(N) Vec::truncate()
        if self.recent_objects.len() > self.max_recent {
            self.recent_objects.pop_back();
        }
    }

    /// Find similar objects for delta base selection
    ///
    /// Returns the best matching object metadata and similarity score.
    ///
    /// # Arguments
    ///
    /// * `target` - The object to find matches for
    /// * `min_similarity` - Minimum similarity score threshold
    /// * `size_ratio_threshold` - Optional size ratio threshold (defaults to 0.80)
    pub fn find_similar(
        &self,
        target: &ObjectMetadata,
        min_similarity: f64,
    ) -> Option<(Oid, SimilarityScore)> {
        self.find_similar_with_size_ratio(target, min_similarity, 0.80)
    }

    /// Find similar objects with configurable size ratio threshold
    pub fn find_similar_with_size_ratio(
        &self,
        target: &ObjectMetadata,
        min_similarity: f64,
        size_ratio_threshold: f64,
    ) -> Option<(Oid, SimilarityScore)> {
        if self.recent_objects.is_empty() || target.sample_hashes.is_empty() {
            return None;
        }

        let mut best_match: Option<(Oid, SimilarityScore)> = None;
        let mut best_score = min_similarity;

        for candidate in &self.recent_objects {
            // Skip if same object
            if candidate.oid == target.oid {
                continue;
            }

            // Skip if different types
            if candidate.obj_type != target.obj_type {
                continue;
            }

            // Skip delta chunks to prevent delta chains (no I/O needed)
            if candidate.is_delta {
                continue;
            }

            // Size-based filtering using configurable threshold
            let size_ratio = if candidate.size < target.size {
                candidate.size as f64 / target.size as f64
            } else {
                target.size as f64 / candidate.size as f64
            };

            if size_ratio < size_ratio_threshold {
                debug!(
                    target_oid = %target.oid,
                    candidate_oid = %candidate.oid,
                    size_ratio,
                    threshold = size_ratio_threshold,
                    "Size difference too large, skipping"
                );
                continue;
            }

            // Sample-based similarity
            let similarity = self.compute_similarity(target, candidate, size_ratio);

            if similarity.score > best_score {
                best_score = similarity.score;
                best_match = Some((candidate.oid, similarity));

                debug!(
                    target_oid = %target.oid,
                    candidate_oid = %candidate.oid,
                    score = similarity.score,
                    "New best similarity match"
                );
            }
        }

        if let Some((oid, score)) = &best_match {
            info!(
                target_oid = %target.oid,
                base_oid = %oid,
                similarity = score.score,
                size_ratio = score.size_ratio,
                "Found similar object for delta base"
            );
        }

        best_match
    }

    /// Compute similarity between two objects using sample hashes
    fn compute_similarity(
        &self,
        target: &ObjectMetadata,
        candidate: &ObjectMetadata,
        size_ratio: f64,
    ) -> SimilarityScore {
        if candidate.sample_hashes.is_empty() || target.sample_hashes.is_empty() {
            return SimilarityScore::new(0.0, size_ratio, 0);
        }

        // Count matching samples
        let mut matches = 0;
        let total_samples = target.sample_hashes.len().min(candidate.sample_hashes.len());

        // Create hash map for fast lookup
        let candidate_hashes: HashMap<u64, usize> = candidate
            .sample_hashes
            .iter()
            .enumerate()
            .map(|(i, &hash)| (hash, i))
            .collect();

        for &hash in &target.sample_hashes {
            if candidate_hashes.contains_key(&hash) {
                matches += 1;
            }
        }

        // Calculate similarity score
        // Factor in both sample matches and size similarity
        let sample_score = matches as f64 / total_samples as f64;
        let similarity = (sample_score * 0.7) + (size_ratio * 0.3);

        SimilarityScore::new(similarity, size_ratio, matches)
    }

    /// Get the number of tracked objects
    pub fn object_count(&self) -> usize {
        self.recent_objects.len()
    }

    /// Clear all tracked objects
    pub fn clear(&mut self) {
        self.recent_objects.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ObjectType;

    #[test]
    fn test_sample_generation() {
        let data = vec![0u8; 10240]; // 10 KB of zeros
        let mut metadata = ObjectMetadata::new(
            Oid::hash(&data),
            data.len(),
            ObjectType::Blob,
            None,
        );

        metadata.generate_samples(&data);
        assert_eq!(metadata.sample_hashes.len(), SAMPLE_COUNT);
    }

    #[test]
    fn test_identical_objects() {
        let data = vec![42u8; 10240];

        let mut target = ObjectMetadata::new(
            Oid::hash(&data),
            data.len(),
            ObjectType::Blob,
            None,
        );
        target.generate_samples(&data);

        let mut candidate = ObjectMetadata::new(
            Oid::hash(&data),
            data.len(),
            ObjectType::Blob,
            None,
        );
        candidate.generate_samples(&data);

        let detector = SimilarityDetector::new(50);
        let size_ratio = 1.0;
        let score = detector.compute_similarity(&target, &candidate, size_ratio);

        // Identical objects should have high similarity
        assert!(score.score > 0.95);
        assert_eq!(score.sample_matches, SAMPLE_COUNT);
    }

    #[test]
    fn test_different_objects() {
        let data1 = vec![1u8; 10240];
        let data2 = vec![2u8; 10240];

        let mut target = ObjectMetadata::new(
            Oid::hash(&data1),
            data1.len(),
            ObjectType::Blob,
            None,
        );
        target.generate_samples(&data1);

        let mut candidate = ObjectMetadata::new(
            Oid::hash(&data2),
            data2.len(),
            ObjectType::Blob,
            None,
        );
        candidate.generate_samples(&data2);

        let detector = SimilarityDetector::new(50);
        let size_ratio = 1.0;
        let score = detector.compute_similarity(&target, &candidate, size_ratio);

        // Different objects should have low similarity
        // Note: With size_ratio=1.0 and 0 sample matches, score = (0 * 0.7) + (1.0 * 0.3) = 0.3
        assert!(score.score <= MIN_SIMILARITY_THRESHOLD);
        assert_eq!(score.sample_matches, 0);
    }

    #[test]
    fn test_size_filtering() {
        let mut detector = SimilarityDetector::new(50);

        // Add a small object
        let small_data = vec![1u8; 1000];
        let mut small = ObjectMetadata::new(
            Oid::hash(&small_data),
            small_data.len(),
            ObjectType::Blob,
            None,
        );
        small.generate_samples(&small_data);
        detector.add_object(small);

        // Try to find similar for a large object (10x larger)
        let large_data = vec![1u8; 10000];
        let mut large = ObjectMetadata::new(
            Oid::hash(&large_data),
            large_data.len(),
            ObjectType::Blob,
            None,
        );
        large.generate_samples(&large_data);

        // Should not find a match due to size difference
        let result = detector.find_similar(&large, MIN_SIMILARITY_THRESHOLD);
        assert!(result.is_none());
    }
}
