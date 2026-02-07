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

//! Object Identifier (OID) for content-addressable storage
//!
//! An OID is a SHA-256 hash of an object's content, providing:
//! - Unique identification of objects
//! - Automatic content deduplication
//! - Content verification capability

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fmt;

/// Object Identifier - SHA-256 hash of object content
///
/// The OID is a 32-byte (256-bit) SHA-256 hash that uniquely identifies
/// an object by its content. This provides automatic deduplication: identical
/// content produces identical OIDs.
///
/// # Examples
///
/// ```
/// use mediagit_versioning::Oid;
///
/// let data = b"Hello, World!";
/// let oid = Oid::hash(data);
/// println!("OID: {}", oid);
/// ```
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct Oid([u8; 32]);

impl Oid {
    /// Create an OID by hashing the given data
    ///
    /// # Examples
    ///
    /// ```
    /// use mediagit_versioning::Oid;
    ///
    /// let data = b"test content";
    /// let oid = Oid::hash(data);
    /// assert_eq!(oid.to_string().len(), 64); // 32 bytes = 64 hex chars
    /// ```
    pub fn hash(data: &[u8]) -> Self {
        let mut hasher = Sha256::new();
        hasher.update(data);
        let result = hasher.finalize();
        let mut bytes = [0u8; 32];
        bytes.copy_from_slice(&result);
        Oid(bytes)
    }

    /// Compute OID from file using streaming hash (constant memory)
    ///
    /// This method reads the file in 64KB chunks, maintaining constant memory
    /// usage regardless of file size. Suitable for files of any size including
    /// multi-terabyte files.
    ///
    /// # Arguments
    ///
    /// * `path` - Path to the file to hash
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use mediagit_versioning::Oid;
    /// use std::path::Path;
    ///
    /// let oid = Oid::from_file(Path::new("large_video.mp4")).unwrap();
    /// println!("File OID: {}", oid);
    /// ```
    pub fn from_file<P: AsRef<std::path::Path>>(path: P) -> anyhow::Result<Self> {
        use std::io::Read;
        
        let mut file = std::fs::File::open(path.as_ref())?;
        let mut hasher = Sha256::new();
        let mut buffer = [0u8; 64 * 1024]; // 64KB buffer - stack allocated
        
        loop {
            let bytes_read = file.read(&mut buffer)?;
            if bytes_read == 0 {
                break;
            }
            hasher.update(&buffer[..bytes_read]);
        }
        
        let result = hasher.finalize();
        let mut bytes = [0u8; 32];
        bytes.copy_from_slice(&result);
        Ok(Oid(bytes))
    }

    /// Compute OID from file using async streaming hash (constant memory)
    ///
    /// Async version of `from_file` that uses tokio for non-blocking I/O.
    /// Suitable for use in async contexts where blocking I/O would be problematic.
    ///
    /// # Arguments
    ///
    /// * `path` - Path to the file to hash
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use mediagit_versioning::Oid;
    /// use std::path::Path;
    ///
    /// # async fn example() -> anyhow::Result<()> {
    /// let oid = Oid::from_file_async(Path::new("large_video.mp4")).await?;
    /// println!("File OID: {}", oid);
    /// # Ok(())
    /// # }
    /// ```
    pub async fn from_file_async<P: AsRef<std::path::Path>>(path: P) -> anyhow::Result<Self> {
        use tokio::io::AsyncReadExt;
        
        let mut file = tokio::fs::File::open(path.as_ref()).await?;
        let mut hasher = Sha256::new();
        let mut buffer = vec![0u8; 64 * 1024]; // 64KB buffer - heap for async
        
        loop {
            let bytes_read = file.read(&mut buffer).await?;
            if bytes_read == 0 {
                break;
            }
            hasher.update(&buffer[..bytes_read]);
        }
        
        let result = hasher.finalize();
        let mut bytes = [0u8; 32];
        bytes.copy_from_slice(&result);
        Ok(Oid(bytes))
    }

    /// Create OID from raw bytes
    ///
    /// # Examples
    ///
    /// ```
    /// use mediagit_versioning::Oid;
    ///
    /// let bytes = [0u8; 32];
    /// let oid = Oid::from_bytes(bytes);
    /// assert_eq!(oid.as_bytes(), &bytes);
    /// ```
    pub fn from_bytes(bytes: [u8; 32]) -> Self {
        Oid(bytes)
    }

    /// Get the raw bytes of the OID
    ///
    /// # Examples
    ///
    /// ```
    /// use mediagit_versioning::Oid;
    ///
    /// let oid = Oid::hash(b"data");
    /// let bytes = oid.as_bytes();
    /// assert_eq!(bytes.len(), 32);
    /// ```
    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    /// Convert OID to hex string
    ///
    /// # Examples
    ///
    /// ```
    /// use mediagit_versioning::Oid;
    ///
    /// let oid = Oid::hash(b"test");
    /// let hex = oid.to_hex();
    /// assert_eq!(hex.len(), 64);
    /// assert!(hex.chars().all(|c| c.is_ascii_hexdigit()));
    /// ```
    pub fn to_hex(&self) -> String {
        hex::encode(self.0)
    }

    /// Create OID from hex string
    ///
    /// # Errors
    ///
    /// Returns error if the string is not 64 hex characters
    ///
    /// # Examples
    ///
    /// ```
    /// use mediagit_versioning::Oid;
    ///
    /// let oid1 = Oid::hash(b"test");
    /// let hex = oid1.to_hex();
    /// let oid2 = Oid::from_hex(&hex).unwrap();
    /// assert_eq!(oid1, oid2);
    /// ```
    pub fn from_hex(s: &str) -> anyhow::Result<Self> {
        if s.len() != 64 {
            anyhow::bail!("OID hex string must be 64 characters, got {}", s.len());
        }

        let bytes = hex::decode(s)?;
        if bytes.len() != 32 {
            anyhow::bail!("Decoded OID must be 32 bytes, got {}", bytes.len());
        }

        let mut oid_bytes = [0u8; 32];
        oid_bytes.copy_from_slice(&bytes);
        Ok(Oid(oid_bytes))
    }

    /// Get object path for Git-like object storage
    ///
    /// Returns path in format: `{first2hex}/{remaining62hex}`
    ///
    /// # Examples
    ///
    /// ```
    /// use mediagit_versioning::Oid;
    ///
    /// let oid = Oid::hash(b"test");
    /// let path = oid.to_path();
    /// // Format: "ab/cdef..." (first 2 hex chars / remaining 62)
    /// assert!(path.contains('/'));
    /// ```
    pub fn to_path(&self) -> String {
        let hex = self.to_hex();
        format!("{}/{}", &hex[..2], &hex[2..])
    }
}

impl fmt::Display for Oid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.to_hex())
    }
}

impl fmt::Debug for Oid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Oid({})", self.to_hex())
    }
}

impl From<[u8; 32]> for Oid {
    fn from(bytes: [u8; 32]) -> Self {
        Oid(bytes)
    }
}

impl From<Oid> for [u8; 32] {
    fn from(oid: Oid) -> Self {
        oid.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hash_deterministic() {
        let data = b"test content";
        let oid1 = Oid::hash(data);
        let oid2 = Oid::hash(data);
        assert_eq!(oid1, oid2, "Same content should produce same OID");
    }

    #[test]
    fn test_hash_different_content() {
        let oid1 = Oid::hash(b"content1");
        let oid2 = Oid::hash(b"content2");
        assert_ne!(oid1, oid2, "Different content should produce different OIDs");
    }

    #[test]
    fn test_hex_roundtrip() {
        let oid1 = Oid::hash(b"test");
        let hex = oid1.to_hex();
        let oid2 = Oid::from_hex(&hex).unwrap();
        assert_eq!(oid1, oid2, "Hex roundtrip should preserve OID");
    }

    #[test]
    fn test_hex_length() {
        let oid = Oid::hash(b"test");
        let hex = oid.to_hex();
        assert_eq!(hex.len(), 64, "SHA-256 hex should be 64 characters");
    }

    #[test]
    fn test_invalid_hex() {
        assert!(Oid::from_hex("too_short").is_err());
        assert!(Oid::from_hex(&"z".repeat(64)).is_err());
    }

    #[test]
    fn test_path_format() {
        let oid = Oid::hash(b"test");
        let path = oid.to_path();
        let parts: Vec<&str> = path.split('/').collect();
        assert_eq!(parts.len(), 2);
        assert_eq!(parts[0].len(), 2);
        assert_eq!(parts[1].len(), 62);
    }

    #[test]
    fn test_display() {
        let oid = Oid::hash(b"test");
        let display = format!("{}", oid);
        assert_eq!(display.len(), 64);
        assert!(display.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn test_from_file_matches_hash() {
        // Create a temp file with known content
        let temp_dir = std::env::temp_dir();
        let test_path = temp_dir.join("mediagit_test_oid_streaming.bin");
        let test_data = b"Hello, World! This is test content for streaming hash.";
        std::fs::write(&test_path, test_data).expect("Failed to write test file");
        
        // Compute both hashes
        let memory_oid = Oid::hash(test_data);
        let file_oid = Oid::from_file(&test_path).expect("Failed to hash file");
        
        // Cleanup
        let _ = std::fs::remove_file(&test_path);
        
        // Verify they match
        assert_eq!(memory_oid, file_oid, "Streaming hash should match in-memory hash");
    }

    #[test]
    fn test_from_file_empty() {
        // Test with empty file
        let temp_dir = std::env::temp_dir();
        let test_path = temp_dir.join("mediagit_test_oid_empty.bin");
        std::fs::write(&test_path, b"").expect("Failed to write empty test file");
        
        let memory_oid = Oid::hash(b"");
        let file_oid = Oid::from_file(&test_path).expect("Failed to hash empty file");
        
        let _ = std::fs::remove_file(&test_path);
        
        assert_eq!(memory_oid, file_oid, "Empty file hash should match empty slice hash");
    }

    #[tokio::test]
    async fn test_from_file_async_matches_hash() {
        let temp_dir = std::env::temp_dir();
        let test_path = temp_dir.join("mediagit_test_oid_async.bin");
        let test_data = b"Async streaming hash test content with more data to ensure buffer works.";
        tokio::fs::write(&test_path, test_data).await.expect("Failed to write test file");
        
        let memory_oid = Oid::hash(test_data);
        let file_oid = Oid::from_file_async(&test_path).await.expect("Failed to hash file async");
        
        let _ = tokio::fs::remove_file(&test_path).await;
        
        assert_eq!(memory_oid, file_oid, "Async streaming hash should match in-memory hash");
    }

    #[test]
    fn test_from_file_large_data() {
        // Test with data larger than buffer (64KB)
        let temp_dir = std::env::temp_dir();
        let test_path = temp_dir.join("mediagit_test_oid_large.bin");
        let test_data: Vec<u8> = (0..100_000).map(|i| (i % 256) as u8).collect();
        std::fs::write(&test_path, &test_data).expect("Failed to write large test file");
        
        let memory_oid = Oid::hash(&test_data);
        let file_oid = Oid::from_file(&test_path).expect("Failed to hash large file");
        
        let _ = std::fs::remove_file(&test_path);
        
        assert_eq!(memory_oid, file_oid, "Large file streaming hash should match in-memory hash");
    }
}
