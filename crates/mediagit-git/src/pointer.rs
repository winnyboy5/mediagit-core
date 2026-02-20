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
// SPDX-License-Identifier: AGPL-3.0
// Copyright (C) 2025 MediaGit Contributors

//! Pointer file implementation for MediaGit
//!
//! Pointer files are lightweight text files that replace large media files in
//! the Git repository. They contain metadata about the actual file stored in
//! MediaGit's object database.
//!
//! ## Format Specification
//!
//! ```text
//! version https://mediagit.dev/spec/v1
//! oid sha256:4d7a214614ab2935c943f9e0ff69d22eadbb8f32b1258daaa5e2ca24d17e2393
//! size 12345
//! ```
//!
//! The format is intentionally similar to Git LFS for familiarity but uses
//! MediaGit-specific version URLs.

use crate::error::{GitError, GitResult};
use serde::{Deserialize, Serialize};
use std::fmt;

/// MediaGit pointer file specification version
pub const POINTER_VERSION: &str = "https://mediagit.dev/spec/v1";

/// Maximum size of a pointer file (should be very small, ~200 bytes)
pub const MAX_POINTER_SIZE: usize = 512;

/// Represents a MediaGit pointer file
///
/// Pointer files are stored in Git instead of the actual media files.
/// They contain references to the actual content stored in MediaGit's
/// object database.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PointerFile {
    /// Version of the pointer file format
    pub version: String,

    /// Object ID (SHA-256 hash) of the actual file content
    pub oid: String,

    /// Size of the actual file in bytes
    pub size: u64,
}

impl PointerFile {
    /// Creates a new pointer file
    ///
    /// # Arguments
    ///
    /// * `oid` - SHA-256 hash of the file content
    /// * `size` - Size of the file in bytes
    ///
    /// # Example
    ///
    /// ```rust
    /// use mediagit_git::PointerFile;
    ///
    /// let pointer = PointerFile::new(
    ///     "4d7a214614ab2935c943f9e0ff69d22eadbb8f32b1258daaa5e2ca24d17e2393".to_string(),
    ///     12345
    /// );
    /// ```
    pub fn new(oid: String, size: u64) -> Self {
        Self {
            version: POINTER_VERSION.to_string(),
            oid,
            size,
        }
    }

    /// Parses a pointer file from its text representation
    ///
    /// # Arguments
    ///
    /// * `content` - Text content of the pointer file
    ///
    /// # Errors
    ///
    /// Returns `GitError::PointerParse` if the content is not valid pointer file format
    ///
    /// # Example
    ///
    /// ```rust
    /// use mediagit_git::PointerFile;
    ///
    /// let content = "version https://mediagit.dev/spec/v1\noid sha256:4d7a214614ab2935c943f9e0ff69d22eadbb8f32b1258daaa5e2ca24d17e2393\nsize 12345\n";
    /// let pointer = PointerFile::parse(content)?;
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn parse(content: &str) -> GitResult<Self> {
        if content.len() > MAX_POINTER_SIZE {
            return Err(GitError::InvalidPointerFormat(
                "Pointer file too large".to_string()
            ));
        }

        let mut version: Option<String> = None;
        let mut oid: Option<String> = None;
        let mut size: Option<u64> = None;

        for line in content.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }

            let parts: Vec<&str> = line.splitn(2, ' ').collect();
            if parts.len() != 2 {
                return Err(GitError::PointerParse(format!(
                    "Invalid line format: {}",
                    line
                )));
            }

            match parts[0] {
                "version" => {
                    version = Some(parts[1].to_string());
                }
                "oid" => {
                    // Format: "sha256:hash"
                    let oid_parts: Vec<&str> = parts[1].splitn(2, ':').collect();
                    if oid_parts.len() != 2 {
                        return Err(GitError::InvalidOid(format!(
                            "OID must be in format 'sha256:hash', got: {}",
                            parts[1]
                        )));
                    }
                    if oid_parts[0] != "sha256" {
                        return Err(GitError::InvalidOid(format!(
                            "Only sha256 hashing is supported, got: {}",
                            oid_parts[0]
                        )));
                    }
                    // Validate hash format (64 hex characters)
                    if oid_parts[1].len() != 64 || !oid_parts[1].chars().all(|c| c.is_ascii_hexdigit()) {
                        return Err(GitError::InvalidOid(format!(
                            "Invalid SHA-256 hash: {}",
                            oid_parts[1]
                        )));
                    }
                    oid = Some(oid_parts[1].to_string());
                }
                "size" => {
                    size = Some(parts[1].parse::<u64>().map_err(|e| {
                        GitError::PointerParse(format!("Invalid size value: {}", e))
                    })?);
                }
                _ => {
                    return Err(GitError::PointerParse(format!(
                        "Unknown field: {}",
                        parts[0]
                    )));
                }
            }
        }

        // Validate required fields
        let version = version.ok_or_else(|| {
            GitError::MissingPointerField("version".to_string())
        })?;

        let oid = oid.ok_or_else(|| {
            GitError::MissingPointerField("oid".to_string())
        })?;

        let size = size.ok_or_else(|| {
            GitError::MissingPointerField("size".to_string())
        })?;

        Ok(Self { version, oid, size })
    }

    /// Checks if the given content looks like a pointer file
    ///
    /// This is a fast check that doesn't do full parsing, useful for
    /// determining if smudge filter should be applied.
    ///
    /// # Example
    ///
    /// ```rust
    /// use mediagit_git::PointerFile;
    ///
    /// let content = "version https://mediagit.dev/spec/v1\noid sha256:abc123\nsize 12345\n";
    /// assert!(PointerFile::is_pointer(content));
    ///
    /// let not_pointer = "This is just regular file content";
    /// assert!(!PointerFile::is_pointer(not_pointer));
    /// ```
    pub fn is_pointer(content: &str) -> bool {
        if content.len() > MAX_POINTER_SIZE {
            return false;
        }

        content.starts_with("version https://mediagit.dev/spec/")
            && content.contains("oid sha256:")
            && content.contains("size ")
    }

    /// Converts the pointer file to its text representation
    ///
    /// # Example
    ///
    /// ```rust
    /// use mediagit_git::PointerFile;
    ///
    /// let pointer = PointerFile::new("abc123".to_string(), 12345);
    /// let text = pointer.to_string();
    /// assert!(text.contains("version https://mediagit.dev/spec/v1"));
    /// ```
    pub fn to_bytes(&self) -> Vec<u8> {
        self.to_string().into_bytes()
    }

    /// Returns the OID with sha256 prefix
    ///
    /// # Example
    ///
    /// ```rust
    /// use mediagit_git::PointerFile;
    ///
    /// let pointer = PointerFile::new("abc123".to_string(), 12345);
    /// assert_eq!(pointer.oid_with_prefix(), "sha256:abc123");
    /// ```
    pub fn oid_with_prefix(&self) -> String {
        format!("sha256:{}", self.oid)
    }
}

impl fmt::Display for PointerFile {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "version {}\noid {}\nsize {}\n",
            self.version,
            self.oid_with_prefix(),
            self.size
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const VALID_OID: &str = "4d7a214614ab2935c943f9e0ff69d22eadbb8f32b1258daaa5e2ca24d17e2393";

    #[test]
    fn test_new_pointer() {
        let pointer = PointerFile::new(VALID_OID.to_string(), 12345);
        assert_eq!(pointer.version, POINTER_VERSION);
        assert_eq!(pointer.oid, VALID_OID);
        assert_eq!(pointer.size, 12345);
    }

    #[test]
    fn test_pointer_to_string() {
        let pointer = PointerFile::new(VALID_OID.to_string(), 12345);
        let text = pointer.to_string();

        assert!(text.contains("version https://mediagit.dev/spec/v1"));
        assert!(text.contains(&format!("oid sha256:{}", VALID_OID)));
        assert!(text.contains("size 12345"));
    }

    #[test]
    fn test_parse_valid_pointer() {
        let content = format!(
            "version https://mediagit.dev/spec/v1\noid sha256:{}\nsize 12345\n",
            VALID_OID
        );

        let pointer = PointerFile::parse(&content).unwrap();
        assert_eq!(pointer.version, POINTER_VERSION);
        assert_eq!(pointer.oid, VALID_OID);
        assert_eq!(pointer.size, 12345);
    }

    #[test]
    fn test_parse_with_extra_whitespace() {
        let content = format!(
            "  version https://mediagit.dev/spec/v1  \n  oid sha256:{}  \n  size 12345  \n",
            VALID_OID
        );

        let pointer = PointerFile::parse(&content).unwrap();
        assert_eq!(pointer.oid, VALID_OID);
    }

    #[test]
    fn test_parse_missing_version() {
        let content = format!("oid sha256:{}\nsize 12345\n", VALID_OID);
        let result = PointerFile::parse(&content);
        assert!(matches!(result, Err(GitError::MissingPointerField(_))));
    }

    #[test]
    fn test_parse_missing_oid() {
        let content = "version https://mediagit.dev/spec/v1\nsize 12345\n";
        let result = PointerFile::parse(content);
        assert!(matches!(result, Err(GitError::MissingPointerField(_))));
    }

    #[test]
    fn test_parse_missing_size() {
        let content = format!("version https://mediagit.dev/spec/v1\noid sha256:{}\n", VALID_OID);
        let result = PointerFile::parse(&content);
        assert!(matches!(result, Err(GitError::MissingPointerField(_))));
    }

    #[test]
    fn test_parse_invalid_oid_format() {
        let content = "version https://mediagit.dev/spec/v1\noid invalid\nsize 12345\n";
        let result = PointerFile::parse(content);
        assert!(matches!(result, Err(GitError::InvalidOid(_))));
    }

    #[test]
    fn test_parse_invalid_hash() {
        let content = "version https://mediagit.dev/spec/v1\noid sha256:notahash\nsize 12345\n";
        let result = PointerFile::parse(content);
        assert!(matches!(result, Err(GitError::InvalidOid(_))));
    }

    #[test]
    fn test_parse_invalid_size() {
        let content = format!(
            "version https://mediagit.dev/spec/v1\noid sha256:{}\nsize notanumber\n",
            VALID_OID
        );
        let result = PointerFile::parse(&content);
        assert!(matches!(result, Err(GitError::PointerParse(_))));
    }

    #[test]
    fn test_is_pointer_valid() {
        let content = format!(
            "version https://mediagit.dev/spec/v1\noid sha256:{}\nsize 12345\n",
            VALID_OID
        );
        assert!(PointerFile::is_pointer(&content));
    }

    #[test]
    fn test_is_pointer_invalid() {
        assert!(!PointerFile::is_pointer("This is just regular file content"));
        assert!(!PointerFile::is_pointer("version something\noid something\n"));
    }

    #[test]
    fn test_is_pointer_too_large() {
        let large_content = "version https://mediagit.dev/spec/v1\n".to_string()
            + &"x".repeat(MAX_POINTER_SIZE);
        assert!(!PointerFile::is_pointer(&large_content));
    }

    #[test]
    fn test_roundtrip() {
        let original = PointerFile::new(VALID_OID.to_string(), 12345);
        let text = original.to_string();
        let parsed = PointerFile::parse(&text).unwrap();
        assert_eq!(original, parsed);
    }

    #[test]
    fn test_oid_with_prefix() {
        let pointer = PointerFile::new(VALID_OID.to_string(), 12345);
        assert_eq!(pointer.oid_with_prefix(), format!("sha256:{}", VALID_OID));
    }
}
