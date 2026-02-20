// Copyright (C) 2026  winnyboy5
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU Affero General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU Affero General Public License for more details.
//
// You should have received a copy of the GNU Affero General Public License
// along with this program.  If not, see <https://www.gnu.org/licenses/>.
// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2025 MediaGit Contributors

//! Test fixture management.
//!
//! Provides utilities for managing test data, including sample files
//! for various media types.

use crate::platform::TestPaths;
use std::fs;
use std::path::PathBuf;

/// Test fixture management utilities.
pub struct TestFixtures;

impl TestFixtures {
    /// Get the path to a sample image file.
    ///
    /// Returns None if the test-files directory doesn't contain the requested image.
    pub fn sample_image(name: &str) -> Option<PathBuf> {
        let path = TestPaths::test_files_dir().join("images").join(name);
        if path.exists() {
            Some(path)
        } else {
            // Try without subdirectory
            let path = TestPaths::test_file(name);
            if path.exists() {
                Some(path)
            } else {
                None
            }
        }
    }

    /// Get the path to a sample video file.
    pub fn sample_video(name: &str) -> Option<PathBuf> {
        let path = TestPaths::test_files_dir().join("videos").join(name);
        if path.exists() {
            Some(path)
        } else {
            let path = TestPaths::test_file(name);
            if path.exists() {
                Some(path)
            } else {
                None
            }
        }
    }

    /// Get the path to a sample audio file.
    pub fn sample_audio(name: &str) -> Option<PathBuf> {
        let path = TestPaths::test_files_dir().join("audio").join(name);
        if path.exists() {
            Some(path)
        } else {
            let path = TestPaths::test_file(name);
            if path.exists() {
                Some(path)
            } else {
                None
            }
        }
    }

    /// Create a sample text file with the given content.
    pub fn text_file(content: &str) -> Vec<u8> {
        content.as_bytes().to_vec()
    }

    /// Create a sample binary file with random-ish content.
    ///
    /// Creates a file of the specified size with predictable but varied content.
    pub fn binary_file(size: usize) -> Vec<u8> {
        (0..size).map(|i| (i % 256) as u8).collect()
    }

    /// Create a sample large file for testing chunking/streaming.
    ///
    /// Creates a file with repeating pattern to achieve the target size.
    pub fn large_file(target_size: usize) -> Vec<u8> {
        let pattern = b"MediaGit Large File Test Pattern\n";
        let repeats = (target_size / pattern.len()) + 1;
        pattern.repeat(repeats).into_iter().take(target_size).collect()
    }

    /// Create a minimal PNG file (1x1 transparent pixel).
    ///
    /// This is useful for testing image detection without requiring external files.
    pub fn minimal_png() -> Vec<u8> {
        // Minimal valid 1x1 transparent PNG
        vec![
            0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, // PNG signature
            0x00, 0x00, 0x00, 0x0D, // IHDR length
            0x49, 0x48, 0x44, 0x52, // IHDR
            0x00, 0x00, 0x00, 0x01, // width: 1
            0x00, 0x00, 0x00, 0x01, // height: 1
            0x08, 0x06, 0x00, 0x00, 0x00, // 8-bit RGBA
            0x1F, 0x15, 0xC4, 0x89, // CRC
            0x00, 0x00, 0x00, 0x0A, // IDAT length
            0x49, 0x44, 0x41, 0x54, // IDAT
            0x78, 0x9C, 0x63, 0x00, 0x01, 0x00, 0x00, 0x05, 0x00, 0x01, // compressed data
            0x0D, 0x0A, 0x2D, 0xB4, // CRC
            0x00, 0x00, 0x00, 0x00, // IEND length
            0x49, 0x45, 0x4E, 0x44, // IEND
            0xAE, 0x42, 0x60, 0x82, // CRC
        ]
    }

    /// Create a minimal JPEG file.
    pub fn minimal_jpeg() -> Vec<u8> {
        // Minimal valid JPEG (smallest valid JPEG possible)
        vec![
            0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x10, 0x4A, 0x46, 0x49, 0x46, 0x00, 0x01,
            0x01, 0x00, 0x00, 0x01, 0x00, 0x01, 0x00, 0x00, 0xFF, 0xDB, 0x00, 0x43,
            0x00, 0x08, 0x06, 0x06, 0x07, 0x06, 0x05, 0x08, 0x07, 0x07, 0x07, 0x09,
            0x09, 0x08, 0x0A, 0x0C, 0x14, 0x0D, 0x0C, 0x0B, 0x0B, 0x0C, 0x19, 0x12,
            0x13, 0x0F, 0x14, 0x1D, 0x1A, 0x1F, 0x1E, 0x1D, 0x1A, 0x1C, 0x1C, 0x20,
            0x24, 0x2E, 0x27, 0x20, 0x22, 0x2C, 0x23, 0x1C, 0x1C, 0x28, 0x37, 0x29,
            0x2C, 0x30, 0x31, 0x34, 0x34, 0x34, 0x1F, 0x27, 0x39, 0x3D, 0x38, 0x32,
            0x3C, 0x2E, 0x33, 0x34, 0x32, 0xFF, 0xC0, 0x00, 0x0B, 0x08, 0x00, 0x01,
            0x00, 0x01, 0x01, 0x01, 0x11, 0x00, 0xFF, 0xC4, 0x00, 0x1F, 0x00, 0x00,
            0x01, 0x05, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08,
            0x09, 0x0A, 0x0B, 0xFF, 0xC4, 0x00, 0xB5, 0x10, 0x00, 0x02, 0x01, 0x03,
            0x03, 0x02, 0x04, 0x03, 0x05, 0x05, 0x04, 0x04, 0x00, 0x00, 0x01, 0x7D,
            0x01, 0x02, 0x03, 0x00, 0x04, 0x11, 0x05, 0x12, 0x21, 0x31, 0x41, 0x06,
            0x13, 0x51, 0x61, 0x07, 0x22, 0x71, 0x14, 0x32, 0x81, 0x91, 0xA1, 0x08,
            0x23, 0x42, 0xB1, 0xC1, 0x15, 0x52, 0xD1, 0xF0, 0x24, 0x33, 0x62, 0x72,
            0x82, 0x09, 0x0A, 0x16, 0x17, 0x18, 0x19, 0x1A, 0x25, 0x26, 0x27, 0x28,
            0x29, 0x2A, 0x34, 0x35, 0x36, 0x37, 0x38, 0x39, 0x3A, 0x43, 0x44, 0x45,
            0x46, 0x47, 0x48, 0x49, 0x4A, 0x53, 0x54, 0x55, 0x56, 0x57, 0x58, 0x59,
            0x5A, 0x63, 0x64, 0x65, 0x66, 0x67, 0x68, 0x69, 0x6A, 0x73, 0x74, 0x75,
            0x76, 0x77, 0x78, 0x79, 0x7A, 0x83, 0x84, 0x85, 0x86, 0x87, 0x88, 0x89,
            0x8A, 0x92, 0x93, 0x94, 0x95, 0x96, 0x97, 0x98, 0x99, 0x9A, 0xA2, 0xA3,
            0xA4, 0xA5, 0xA6, 0xA7, 0xA8, 0xA9, 0xAA, 0xB2, 0xB3, 0xB4, 0xB5, 0xB6,
            0xB7, 0xB8, 0xB9, 0xBA, 0xC2, 0xC3, 0xC4, 0xC5, 0xC6, 0xC7, 0xC8, 0xC9,
            0xCA, 0xD2, 0xD3, 0xD4, 0xD5, 0xD6, 0xD7, 0xD8, 0xD9, 0xDA, 0xE1, 0xE2,
            0xE3, 0xE4, 0xE5, 0xE6, 0xE7, 0xE8, 0xE9, 0xEA, 0xF1, 0xF2, 0xF3, 0xF4,
            0xF5, 0xF6, 0xF7, 0xF8, 0xF9, 0xFA, 0xFF, 0xDA, 0x00, 0x08, 0x01, 0x01,
            0x00, 0x00, 0x3F, 0x00, 0xFB, 0xD5, 0xDB, 0x20, 0xA8, 0xF1, 0x58, 0xBF,
            0xFF, 0xD9,
        ]
    }

    /// Create a sample Git-compatible config file.
    pub fn sample_config() -> String {
        r#"[core]
    repositoryformatversion = 0
    filemode = true
    bare = false
[user]
    name = Test User
    email = test@example.com
"#.to_string()
    }

    /// List all available test fixtures in the test-files directory.
    pub fn list_available() -> Vec<PathBuf> {
        let test_dir = TestPaths::test_files_dir();
        if !test_dir.exists() {
            return Vec::new();
        }

        Self::walk_dir(&test_dir)
    }

    fn walk_dir(dir: &std::path::Path) -> Vec<PathBuf> {
        let mut files = Vec::new();
        if let Ok(entries) = fs::read_dir(dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_file() {
                    files.push(path);
                } else if path.is_dir() {
                    files.extend(Self::walk_dir(&path));
                }
            }
        }
        files
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_binary_file_creation() {
        let data = TestFixtures::binary_file(1024);
        assert_eq!(data.len(), 1024);
    }

    #[test]
    fn test_large_file_creation() {
        let size = 1024 * 1024; // 1 MB
        let data = TestFixtures::large_file(size);
        assert_eq!(data.len(), size);
    }

    #[test]
    fn test_minimal_png_signature() {
        let png = TestFixtures::minimal_png();
        assert_eq!(&png[0..8], &[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A]);
    }

    #[test]
    fn test_minimal_jpeg_signature() {
        let jpeg = TestFixtures::minimal_jpeg();
        assert_eq!(&jpeg[0..2], &[0xFF, 0xD8]);
    }
}
