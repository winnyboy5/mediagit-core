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

//! Object types for the MediaGit version control system

use serde::{Deserialize, Serialize};
use std::fmt;

/// Object types in the MediaGit object database
///
/// Compatible with Git object types, allowing interoperability
/// and familiar semantics for version control operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ObjectType {
    /// Blob - arbitrary binary data (files, media assets)
    Blob,
    /// Tree - directory structure with references to other objects
    Tree,
    /// Commit - snapshot metadata with parent references
    Commit,
}

impl ObjectType {
    /// Get the type as a string identifier
    ///
    /// # Examples
    ///
    /// ```
    /// use mediagit_versioning::ObjectType;
    ///
    /// assert_eq!(ObjectType::Blob.as_str(), "blob");
    /// assert_eq!(ObjectType::Tree.as_str(), "tree");
    /// assert_eq!(ObjectType::Commit.as_str(), "commit");
    /// ```
    pub fn as_str(&self) -> &'static str {
        match self {
            ObjectType::Blob => "blob",
            ObjectType::Tree => "tree",
            ObjectType::Commit => "commit",
        }
    }

    /// Parse object type from string
    ///
    /// # Examples
    ///
    /// ```
    /// use mediagit_versioning::ObjectType;
    ///
    /// assert_eq!(ObjectType::parse("blob").unwrap(), ObjectType::Blob);
    /// assert_eq!(ObjectType::parse("tree").unwrap(), ObjectType::Tree);
    /// assert!(ObjectType::parse("invalid").is_err());
    /// ```
    pub fn parse(s: &str) -> anyhow::Result<Self> {
        match s {
            "blob" => Ok(ObjectType::Blob),
            "tree" => Ok(ObjectType::Tree),
            "commit" => Ok(ObjectType::Commit),
            _ => anyhow::bail!("Unknown object type: {}", s),
        }
    }

    /// Convert object type to byte value
    pub fn to_u8(self) -> u8 {
        match self {
            ObjectType::Blob => 1,
            ObjectType::Tree => 2,
            ObjectType::Commit => 3,
        }
    }

    /// Parse object type from byte value
    pub fn from_u8(value: u8) -> Option<Self> {
        match value {
            1 => Some(ObjectType::Blob),
            2 => Some(ObjectType::Tree),
            3 => Some(ObjectType::Commit),
            _ => None,
        }
    }
}

impl fmt::Display for ObjectType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_object_type_str() {
        assert_eq!(ObjectType::Blob.as_str(), "blob");
        assert_eq!(ObjectType::Tree.as_str(), "tree");
        assert_eq!(ObjectType::Commit.as_str(), "commit");
    }

    #[test]
    fn test_object_type_from_str() {
        assert_eq!(ObjectType::parse("blob").unwrap(), ObjectType::Blob);
        assert_eq!(ObjectType::parse("tree").unwrap(), ObjectType::Tree);
        assert_eq!(ObjectType::parse("commit").unwrap(), ObjectType::Commit);
        assert!(ObjectType::parse("invalid").is_err());
    }

    #[test]
    fn test_object_type_display() {
        assert_eq!(format!("{}", ObjectType::Blob), "blob");
        assert_eq!(format!("{}", ObjectType::Tree), "tree");
        assert_eq!(format!("{}", ObjectType::Commit), "commit");
    }

    #[test]
    fn test_object_type_roundtrip() {
        for obj_type in [ObjectType::Blob, ObjectType::Tree, ObjectType::Commit] {
            let s = obj_type.as_str();
            let parsed = ObjectType::parse(s).unwrap();
            assert_eq!(obj_type, parsed);
        }
    }
}
