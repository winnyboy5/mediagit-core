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

//! Reference log (reflog) implementation for tracking ref history.
//!
//! The reflog records updates to branches and HEAD, enabling recovery of
//! previous states and tracking how refs have changed over time.
//!
//! # Storage Format
//!
//! Entries are stored in `.mediagit/logs/` as plain text files:
//! - `.mediagit/logs/HEAD` - tracks HEAD changes
//! - `.mediagit/logs/refs/heads/{branch}` - tracks branch changes
//!
//! Each line contains:
//! ```text
//! <old_oid> <new_oid> <name> <email> <timestamp> <tz>\t<reason>
//! ```
//!
//! # Examples
//!
//! ```no_run
//! use mediagit_versioning::{Reflog, ReflogEntry, Oid, Signature};
//!
//! #[tokio::main]
//! async fn main() -> anyhow::Result<()> {
//!     let reflog = Reflog::new("/path/to/.mediagit");
//!
//!     // Append a new entry
//!     let entry = ReflogEntry {
//!         old_oid: Oid::from_bytes([0u8; 32]),
//!         new_oid: Oid::hash(b"commit data"),
//!         committer: Signature::now("Alice".to_string(), "alice@example.com".to_string()),
//!         message: "commit: Initial commit".to_string(),
//!     };
//!     reflog.append("HEAD", &entry).await?;
//!
//!     // Read reflog entries
//!     let entries = reflog.read("HEAD", Some(10)).await?;
//!     for (i, entry) in entries.iter().enumerate() {
//!         println!("HEAD@{{{}}}: {} -> {} - {}",
//!             i, entry.old_oid, entry.new_oid, entry.message);
//!     }
//!
//!     Ok(())
//! }
//! ```

use anyhow::{Context, Result};
use chrono::{TimeZone, Utc};
use std::path::PathBuf;
use tokio::fs::{self, OpenOptions};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

use crate::{Oid, Signature};

/// A single entry in the reflog
#[derive(Debug, Clone)]
pub struct ReflogEntry {
    /// OID before the change (zero for new refs)
    pub old_oid: Oid,
    /// OID after the change
    pub new_oid: Oid,
    /// Who made the change
    pub committer: Signature,
    /// Description of why the change was made (e.g., "commit: Initial commit")
    pub message: String,
}

impl ReflogEntry {
    /// Create a new reflog entry for the current time
    pub fn now(old_oid: Oid, new_oid: Oid, name: &str, email: &str, message: &str) -> Self {
        Self {
            old_oid,
            new_oid,
            committer: Signature::now(name.to_string(), email.to_string()),
            message: message.to_string(),
        }
    }

    /// Format entry as a single reflog line
    pub fn to_line(&self) -> String {
        let timestamp = self.committer.timestamp.timestamp();
        let offset_secs = 0i32; // UTC for simplicity, could enhance later
        let tz = format!("{:+05}", offset_secs / 36);
        
        format!(
            "{} {} {} <{}> {} {}\t{}\n",
            self.old_oid.to_hex(),
            self.new_oid.to_hex(),
            self.committer.name,
            self.committer.email,
            timestamp,
            tz,
            self.message
        )
    }

    /// Parse a reflog entry from a line
    pub fn from_line(line: &str) -> Result<Self> {
        // Format: <old_oid> <new_oid> <name> <<email>> <timestamp> <tz>\t<message>
        let parts: Vec<&str> = line.splitn(2, '\t').collect();
        if parts.len() < 2 {
            anyhow::bail!("Invalid reflog line: missing tab separator");
        }

        let header = parts[0];
        let message = parts[1].trim().to_string();

        // Parse header: old_oid new_oid name <email> timestamp tz
        let header_parts: Vec<&str> = header.split_whitespace().collect();
        if header_parts.len() < 6 {
            anyhow::bail!("Invalid reflog header: insufficient parts");
        }

        let old_oid = Oid::from_hex(header_parts[0])
            .context("Invalid old OID in reflog")?;
        let new_oid = Oid::from_hex(header_parts[1])
            .context("Invalid new OID in reflog")?;

        // Name may contain spaces, so we need to find the email which is enclosed in <>
        let header_rest = header_parts[2..].join(" ");
        let email_start = header_rest.find('<').context("Missing email start bracket")?;
        let email_end = header_rest.find('>').context("Missing email end bracket")?;
        
        let name = header_rest[..email_start].trim().to_string();
        let email = header_rest[email_start + 1..email_end].to_string();
        
        // Parse timestamp and timezone after email
        let after_email = header_rest[email_end + 1..].trim();
        let time_parts: Vec<&str> = after_email.split_whitespace().collect();
        if time_parts.is_empty() {
            anyhow::bail!("Invalid timestamp in reflog");
        }
        
        let timestamp: i64 = time_parts[0].parse()
            .context("Invalid timestamp number")?;
        let datetime = Utc.timestamp_opt(timestamp, 0)
            .single()
            .context("Invalid timestamp value")?;

        Ok(Self {
            old_oid,
            new_oid,
            committer: Signature {
                name,
                email,
                timestamp: datetime,
            },
            message,
        })
    }
}

/// Reference log manager for tracking ref history
///
/// The reflog records all changes to refs, enabling recovery of previous
/// states and debugging of ref movements.
pub struct Reflog {
    /// Root path to the .mediagit directory
    root: PathBuf,
}

impl Reflog {
    /// Create a new Reflog manager
    ///
    /// # Arguments
    /// * `root` - Path to the .mediagit directory
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self {
            root: root.into(),
        }
    }

    /// Get the path to a reflog file
    fn reflog_path(&self, ref_name: &str) -> PathBuf {
        let logs_dir = self.root.join("logs");
        
        // Normalize ref name
        if ref_name == "HEAD" {
            logs_dir.join("HEAD")
        } else if ref_name.starts_with("refs/") {
            logs_dir.join(ref_name)
        } else {
            // Assume it's a branch name
            logs_dir.join("refs").join("heads").join(ref_name)
        }
    }

    /// Append an entry to the reflog
    ///
    /// Creates parent directories if they don't exist.
    ///
    /// # Arguments
    /// * `ref_name` - Name of the ref (e.g., "HEAD", "refs/heads/main", or "main")
    /// * `entry` - The reflog entry to append
    pub async fn append(&self, ref_name: &str, entry: &ReflogEntry) -> Result<()> {
        let path = self.reflog_path(ref_name);
        
        // Ensure parent directory exists
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).await
                .context("Failed to create reflog directory")?;
        }

        // Append to file (create if doesn't exist)
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .await
            .context("Failed to open reflog file")?;

        let line = entry.to_line();
        file.write_all(line.as_bytes()).await
            .context("Failed to write reflog entry")?;

        Ok(())
    }

    /// Read reflog entries for a ref
    ///
    /// Returns entries in reverse chronological order (newest first).
    ///
    /// # Arguments
    /// * `ref_name` - Name of the ref
    /// * `limit` - Maximum number of entries to return (None for all)
    pub async fn read(&self, ref_name: &str, limit: Option<usize>) -> Result<Vec<ReflogEntry>> {
        let path = self.reflog_path(ref_name);
        
        if !path.exists() {
            return Ok(Vec::new());
        }

        let file = fs::File::open(&path).await
            .context("Failed to open reflog file")?;
        let reader = BufReader::new(file);
        let mut lines = reader.lines();

        let mut entries = Vec::new();
        while let Some(line) = lines.next_line().await? {
            if line.trim().is_empty() {
                continue;
            }
            match ReflogEntry::from_line(&line) {
                Ok(entry) => entries.push(entry),
                Err(e) => {
                    tracing::warn!("Skipping invalid reflog line: {}", e);
                }
            }
        }

        // Reverse to get newest first
        entries.reverse();

        // Apply limit
        if let Some(n) = limit {
            entries.truncate(n);
        }

        Ok(entries)
    }

    /// Expire old reflog entries, keeping only the most recent ones
    ///
    /// # Arguments
    /// * `ref_name` - Name of the ref
    /// * `keep` - Number of entries to keep
    ///
    /// # Returns
    /// Number of entries that were expired (removed)
    pub async fn expire(&self, ref_name: &str, keep: usize) -> Result<usize> {
        let path = self.reflog_path(ref_name);
        
        if !path.exists() {
            return Ok(0);
        }

        // Read all entries
        let entries = self.read(ref_name, None).await?;
        
        if entries.len() <= keep {
            return Ok(0);
        }

        let expired_count = entries.len() - keep;
        
        // Keep only the most recent entries (entries are newest-first)
        let to_keep = &entries[..keep];
        
        // Rewrite file with kept entries (reverse back to oldest-first for storage)
        let mut kept_entries: Vec<_> = to_keep.to_vec();
        kept_entries.reverse();

        let mut content = String::new();
        for entry in kept_entries {
            content.push_str(&entry.to_line());
        }

        fs::write(&path, content).await
            .context("Failed to write expired reflog")?;

        Ok(expired_count)
    }

    /// Delete the reflog for a ref
    ///
    /// # Arguments
    /// * `ref_name` - Name of the ref
    pub async fn delete(&self, ref_name: &str) -> Result<bool> {
        let path = self.reflog_path(ref_name);
        
        if path.exists() {
            fs::remove_file(&path).await
                .context("Failed to delete reflog file")?;
            Ok(true)
        } else {
            Ok(false)
        }
    }

    /// Check if a reflog exists for a ref
    pub async fn exists(&self, ref_name: &str) -> bool {
        self.reflog_path(ref_name).exists()
    }

    /// List all refs that have reflogs
    pub async fn list_refs(&self) -> Result<Vec<String>> {
        let logs_dir = self.root.join("logs");
        if !logs_dir.exists() {
            return Ok(Vec::new());
        }

        let mut refs = Vec::new();

        // Check HEAD
        if logs_dir.join("HEAD").exists() {
            refs.push("HEAD".to_string());
        }

        // Walk refs directory
        let refs_logs_dir = logs_dir.join("refs");
        if refs_logs_dir.exists() {
            refs.extend(self.walk_refs_dir(&refs_logs_dir, "refs").await?);
        }

        Ok(refs)
    }

    /// Recursively walk refs directory to find all reflogs
    #[allow(clippy::only_used_in_recursion)]
    fn walk_refs_dir<'a>(
        &'a self,
        dir: &'a PathBuf,
        prefix: &'a str,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Vec<String>>> + Send + 'a>> {
        Box::pin(async move {
            let mut refs = Vec::new();
            let mut entries = fs::read_dir(dir).await?;

            while let Some(entry) = entries.next_entry().await? {
                let path = entry.path();
                let name = entry.file_name().to_string_lossy().to_string();
                let ref_name = format!("{}/{}", prefix, name);

                if path.is_dir() {
                    refs.extend(self.walk_refs_dir(&path, &ref_name).await?);
                } else if path.is_file() {
                    refs.push(ref_name);
                }
            }

            Ok(refs)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[tokio::test]
    async fn test_reflog_entry_roundtrip() {
        let entry = ReflogEntry::now(
            Oid::from_bytes([0u8; 32]),
            Oid::hash(b"test"),
            "Test User",
            "test@example.com",
            "commit: Test commit",
        );

        let line = entry.to_line();
        let parsed = ReflogEntry::from_line(line.trim()).unwrap();

        assert_eq!(entry.old_oid, parsed.old_oid);
        assert_eq!(entry.new_oid, parsed.new_oid);
        assert_eq!(entry.committer.name, parsed.committer.name);
        assert_eq!(entry.committer.email, parsed.committer.email);
        assert_eq!(entry.message, parsed.message);
    }

    #[tokio::test]
    async fn test_reflog_append_and_read() {
        let tmp = TempDir::new().unwrap();
        let reflog = Reflog::new(tmp.path());

        // Append entries
        let entry1 = ReflogEntry::now(
            Oid::from_bytes([0u8; 32]),
            Oid::hash(b"commit1"),
            "User",
            "user@test.com",
            "commit: First",
        );
        reflog.append("HEAD", &entry1).await.unwrap();

        let entry2 = ReflogEntry::now(
            Oid::hash(b"commit1"),
            Oid::hash(b"commit2"),
            "User",
            "user@test.com",
            "commit: Second",
        );
        reflog.append("HEAD", &entry2).await.unwrap();

        // Read back
        let entries = reflog.read("HEAD", None).await.unwrap();
        assert_eq!(entries.len(), 2);
        // Newest first
        assert_eq!(entries[0].message, "commit: Second");
        assert_eq!(entries[1].message, "commit: First");
    }

    #[tokio::test]
    async fn test_reflog_expire() {
        let tmp = TempDir::new().unwrap();
        let reflog = Reflog::new(tmp.path());

        // Append 5 entries
        for i in 0..5 {
            let entry = ReflogEntry::now(
                Oid::hash(format!("old{}", i).as_bytes()),
                Oid::hash(format!("new{}", i).as_bytes()),
                "User",
                "user@test.com",
                &format!("commit: Entry {}", i),
            );
            reflog.append("refs/heads/main", &entry).await.unwrap();
        }

        // Expire, keep only 2
        let expired = reflog.expire("refs/heads/main", 2).await.unwrap();
        assert_eq!(expired, 3);

        // Verify only 2 remain
        let entries = reflog.read("refs/heads/main", None).await.unwrap();
        assert_eq!(entries.len(), 2);
    }

    #[tokio::test]
    async fn test_reflog_with_branch_shorthand() {
        let tmp = TempDir::new().unwrap();
        let reflog = Reflog::new(tmp.path());

        let entry = ReflogEntry::now(
            Oid::from_bytes([0u8; 32]),
            Oid::hash(b"test"),
            "User",
            "user@test.com",
            "commit: Test",
        );

        // Use shorthand branch name
        reflog.append("feature", &entry).await.unwrap();

        // Should be stored in refs/heads/feature
        let path = tmp.path().join("logs/refs/heads/feature");
        assert!(path.exists());

        // Read with shorthand
        let entries = reflog.read("feature", None).await.unwrap();
        assert_eq!(entries.len(), 1);
    }
}
