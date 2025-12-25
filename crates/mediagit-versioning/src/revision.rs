// MediaGit - Git for Media Files
// Copyright (C) 2025 MediaGit Contributors
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU Affero General Public License as published
// by the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.

//! Revision parsing and resolution
//!
//! This module provides functionality to parse and resolve revision specifiers:
//! - HEAD~N: N-th ancestor via first parent
//! - Direct OID references
//! - Branch names and refs

use anyhow::{Context, Result};
use crate::{Commit, ObjectDatabase, Oid, RefDatabase};

/// Parse and resolve a revision specifier to an OID
///
/// Supports:
/// - Direct OID (full or abbreviated)
/// - HEAD
/// - HEAD~N (N-th ancestor via first parent)
/// - Branch names
/// - Full ref paths (refs/heads/...)
pub async fn resolve_revision(
    revision: &str,
    refdb: &RefDatabase,
    odb: &ObjectDatabase,
) -> Result<Oid> {
    // Check for HEAD~N notation
    if let Some(parent_count) = parse_parent_notation(revision)? {
        let (base, count) = parent_count;

        // Resolve base reference
        let base_oid = if base == "HEAD" {
            refdb.resolve("HEAD").await
                .context("Cannot resolve HEAD")?
        } else {
            // Try direct OID first
            if let Ok(oid) = Oid::from_hex(&base) {
                oid
            } else {
                // Try as reference
                match refdb.resolve(&base).await {
                    Ok(oid) => oid,
                    Err(_) => {
                        // Try with refs/heads prefix
                        let with_prefix = format!("refs/heads/{}", base);
                        refdb.resolve(&with_prefix).await
                            .context(format!("Cannot resolve base revision: {}", base))?
                    }
                }
            }
        };

        // Walk parent chain
        return walk_parents(base_oid, count, odb).await;
    }

    // Try as direct OID
    if let Ok(oid) = Oid::from_hex(revision) {
        return Ok(oid);
    }

    // Try to resolve as reference (handles symbolic refs like HEAD)
    if let Ok(oid) = refdb.resolve(revision).await {
        return Ok(oid);
    }

    // Try with refs/heads prefix
    let with_prefix = format!("refs/heads/{}", revision);
    if let Ok(oid) = refdb.resolve(&with_prefix).await {
        return Ok(oid);
    }

    anyhow::bail!("Cannot resolve revision: {}", revision)
}

/// Parse parent notation like HEAD~N or branch~N
///
/// Returns Some((base, count)) if notation is found, None otherwise
fn parse_parent_notation(revision: &str) -> Result<Option<(String, usize)>> {
    if let Some(tilde_pos) = revision.rfind('~') {
        let base = &revision[..tilde_pos];
        let count_str = &revision[tilde_pos + 1..];

        // Parse the count
        let count: usize = count_str.parse()
            .context(format!("Invalid parent count in '{}': must be a number", revision))?;

        if count == 0 {
            anyhow::bail!("Parent count must be positive in '{}'", revision);
        }

        Ok(Some((base.to_string(), count)))
    } else {
        Ok(None)
    }
}

/// Walk N parents back from the given OID via first parent
async fn walk_parents(start_oid: Oid, count: usize, odb: &ObjectDatabase) -> Result<Oid> {
    let mut current_oid = start_oid;

    for i in 0..count {
        // Read commit
        let data = odb.read(&current_oid).await
            .context(format!("Failed to read commit {} (parent {})", current_oid, i))?;

        // Deserialize as commit
        let commit = Commit::deserialize(&data)
            .context(format!("Object {} is not a commit", current_oid))?;

        // Get first parent
        if commit.parents.is_empty() {
            anyhow::bail!(
                "Cannot go back {} generation(s): commit {} has no parents (reached root at generation {})",
                count, current_oid, i
            );
        }

        current_oid = commit.parents[0];
    }

    Ok(current_oid)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_parent_notation() {
        // Valid cases
        assert_eq!(
            parse_parent_notation("HEAD~1").unwrap(),
            Some(("HEAD".to_string(), 1))
        );
        assert_eq!(
            parse_parent_notation("HEAD~5").unwrap(),
            Some(("HEAD".to_string(), 5))
        );
        assert_eq!(
            parse_parent_notation("main~2").unwrap(),
            Some(("main".to_string(), 2))
        );
        assert_eq!(
            parse_parent_notation("refs/heads/feature~3").unwrap(),
            Some(("refs/heads/feature".to_string(), 3))
        );

        // No notation
        assert_eq!(parse_parent_notation("HEAD").unwrap(), None);
        assert_eq!(parse_parent_notation("main").unwrap(), None);

        // Invalid count
        assert!(parse_parent_notation("HEAD~abc").is_err());
        assert!(parse_parent_notation("HEAD~0").is_err());
    }
}
