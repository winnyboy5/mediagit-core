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

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use mediagit_versioning::{Oid, Ref, RefDatabase};
use std::path::PathBuf;

/// Manage tags
#[derive(Parser, Debug)]
pub struct TagCmd {
    #[command(subcommand)]
    pub subcommand: TagSubcommand,
}

#[derive(Subcommand, Debug)]
pub enum TagSubcommand {
    /// Create a new tag
    Create(CreateOpts),

    /// List tags
    #[command(alias = "ls")]
    List(ListOpts),

    /// Delete a tag
    #[command(alias = "rm")]
    Delete(DeleteOpts),

    /// Show tag information
    Show(ShowOpts),

    /// Verify a tag
    Verify(VerifyOpts),
}

/// Create a new tag
#[derive(Parser, Debug)]
pub struct CreateOpts {
    /// Tag name
    #[arg(value_name = "NAME")]
    pub name: String,

    /// Commit to tag (defaults to HEAD)
    #[arg(value_name = "COMMIT")]
    pub commit: Option<String>,

    /// Create annotated tag with message
    #[arg(short = 'm', long, value_name = "MESSAGE")]
    pub message: Option<String>,

    /// Tagger name (for annotated tags)
    #[arg(long, value_name = "NAME")]
    pub tagger: Option<String>,

    /// Tagger email (for annotated tags)
    #[arg(long, value_name = "EMAIL")]
    pub email: Option<String>,

    /// Force tag creation (overwrite existing)
    #[arg(short, long)]
    pub force: bool,

    /// Quiet mode
    #[arg(short, long)]
    pub quiet: bool,
}

/// List tags
#[derive(Parser, Debug)]
pub struct ListOpts {
    /// Pattern to filter tags (glob-style)
    #[arg(value_name = "PATTERN")]
    pub pattern: Option<String>,

    /// Show verbose output (include commit info)
    #[arg(short = 'n', long)]
    pub verbose: bool,

    /// Sort tags
    #[arg(long, value_name = "KEY", default_value = "refname")]
    pub sort: String,

    /// Reverse sort order
    #[arg(long)]
    pub reverse: bool,
}

/// Delete a tag
#[derive(Parser, Debug)]
pub struct DeleteOpts {
    /// Tag name(s) to delete
    #[arg(value_name = "NAME", required = true)]
    pub names: Vec<String>,

    /// Force deletion without confirmation
    #[arg(short, long)]
    pub force: bool,

    /// Quiet mode
    #[arg(short, long)]
    pub quiet: bool,
}

/// Show tag information
#[derive(Parser, Debug)]
pub struct ShowOpts {
    /// Tag name
    #[arg(value_name = "NAME")]
    pub name: String,

    /// Show full commit details
    #[arg(long)]
    pub full: bool,
}

/// Verify a tag
#[derive(Parser, Debug)]
pub struct VerifyOpts {
    /// Tag name to verify
    #[arg(value_name = "NAME")]
    pub name: String,

    /// Verbose output
    #[arg(short, long)]
    pub verbose: bool,
}

impl TagCmd {
    /// Execute tag command
    pub async fn execute(&self, repo_path: PathBuf) -> Result<()> {
        match &self.subcommand {
            TagSubcommand::Create(opts) => self.create(repo_path, opts).await,
            TagSubcommand::List(opts) => self.list(repo_path, opts).await,
            TagSubcommand::Delete(opts) => self.delete(repo_path, opts).await,
            TagSubcommand::Show(opts) => self.show(repo_path, opts).await,
            TagSubcommand::Verify(opts) => self.verify(repo_path, opts).await,
        }
    }

    /// Create a new tag
    async fn create(&self, repo_path: PathBuf, opts: &CreateOpts) -> Result<()> {
        let mediagit_dir = repo_path.join(".mediagit");
        let refdb = RefDatabase::new(&mediagit_dir);

        // Validate tag name
        self.validate_tag_name(&opts.name)?;

        // Check if tag already exists
        let tag_ref = format!("refs/tags/{}", opts.name);
        if refdb.exists(&tag_ref).await? && !opts.force {
            anyhow::bail!("Tag '{}' already exists. Use --force to overwrite.", opts.name);
        }

        // Resolve target commit
        let target_oid = if let Some(ref commit_ref) = opts.commit {
            self.resolve_commit(&refdb, commit_ref).await?
        } else {
            // Default to HEAD
            refdb.resolve("HEAD").await.context("Failed to resolve HEAD")?
        };

        // Create tag based on type
        if let Some(ref message) = opts.message {
            // Annotated tag
            self.create_annotated_tag(&refdb, &opts.name, target_oid, message, opts).await?;
        } else {
            // Lightweight tag
            self.create_lightweight_tag(&refdb, &opts.name, target_oid).await?;
        }

        if !opts.quiet {
            let tag_type = if opts.message.is_some() { "annotated" } else { "lightweight" };
            println!("Created {} tag '{}' at {}", tag_type, opts.name, target_oid.to_hex());
        }

        Ok(())
    }

    /// Create lightweight tag (ref pointing directly to commit)
    async fn create_lightweight_tag(&self, refdb: &RefDatabase, name: &str, commit_oid: Oid) -> Result<()> {
        let tag_ref = format!("refs/tags/{}", name);
        let r = Ref::new_direct(tag_ref, commit_oid);
        refdb.write(&r).await?;
        Ok(())
    }

    /// Create annotated tag (tag object with metadata)
    async fn create_annotated_tag(
        &self,
        refdb: &RefDatabase,
        name: &str,
        commit_oid: Oid,
        message: &str,
        opts: &CreateOpts,
    ) -> Result<()> {
        // For now, create a lightweight tag with metadata stored separately
        // TODO: Implement proper tag objects when object storage supports Tag type

        let tag_ref = format!("refs/tags/{}", name);
        let r = Ref::new_direct(tag_ref.clone(), commit_oid);
        refdb.write(&r).await?;

        // Store tag metadata in companion file
        let metadata = TagMetadata {
            tag_name: name.to_string(),
            commit_oid: commit_oid.to_hex(),
            message: message.to_string(),
            tagger: opts.tagger.clone(),
            email: opts.email.clone(),
            timestamp: chrono::Utc::now().to_rfc3339(),
        };

        // Get metadata path from mediagit dir
        let mediagit_dir = std::env::current_dir()?.join(".mediagit");
        let metadata_path = get_ref_path(&mediagit_dir, &format!("{}.meta", tag_ref));
        if let Some(parent) = metadata_path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }

        let metadata_json = serde_json::to_string_pretty(&metadata)?;
        tokio::fs::write(metadata_path, metadata_json).await?;

        Ok(())
    }

    /// List tags
    async fn list(&self, repo_path: PathBuf, opts: &ListOpts) -> Result<()> {
        let mediagit_dir = repo_path.join(".mediagit");
        let refdb = RefDatabase::new(&mediagit_dir);

        let mut tags = refdb.list_tags().await?;

        // Filter by pattern if provided
        if let Some(ref pattern) = opts.pattern {
            tags = self.filter_tags_by_pattern(tags, pattern);
        }

        // Sort tags
        tags = self.sort_tags(tags, &opts.sort, opts.reverse);

        // Display tags
        if opts.verbose {
            self.list_verbose(&refdb, tags).await?;
        } else {
            self.list_simple(tags);
        }

        Ok(())
    }

    /// Filter tags by glob pattern
    fn filter_tags_by_pattern(&self, tags: Vec<String>, pattern: &str) -> Vec<String> {
        let glob_pattern = match glob::Pattern::new(pattern) {
            Ok(p) => p,
            Err(_) => return tags, // Invalid pattern, return all
        };

        tags.into_iter()
            .filter(|tag| {
                let tag_name = tag.strip_prefix("refs/tags/").unwrap_or(tag);
                glob_pattern.matches(tag_name)
            })
            .collect()
    }

    /// Sort tags
    fn sort_tags(&self, mut tags: Vec<String>, sort_key: &str, reverse: bool) -> Vec<String> {
        match sort_key {
            "refname" | "name" => {
                tags.sort();
            }
            "version" => {
                // Sort by semantic version
                tags.sort_by(|a, b| {
                    let a_name = a.strip_prefix("refs/tags/").unwrap_or(a);
                    let b_name = b.strip_prefix("refs/tags/").unwrap_or(b);
                    self.compare_versions(a_name, b_name)
                });
            }
            _ => {
                tags.sort(); // Default to name sorting
            }
        }

        if reverse {
            tags.reverse();
        }

        tags
    }

    /// Compare version strings (semantic versioning aware)
    fn compare_versions(&self, a: &str, b: &str) -> std::cmp::Ordering {
        // Try to parse as semver
        let a_parts: Vec<&str> = a.trim_start_matches('v').split('.').collect();
        let b_parts: Vec<&str> = b.trim_start_matches('v').split('.').collect();

        for (a_part, b_part) in a_parts.iter().zip(b_parts.iter()) {
            if let (Ok(a_num), Ok(b_num)) = (a_part.parse::<u32>(), b_part.parse::<u32>()) {
                match a_num.cmp(&b_num) {
                    std::cmp::Ordering::Equal => continue,
                    other => return other,
                }
            } else {
                return a_part.cmp(b_part);
            }
        }

        a_parts.len().cmp(&b_parts.len())
    }

    /// List tags with simple output
    fn list_simple(&self, tags: Vec<String>) {
        for tag in tags {
            let tag_name = tag.strip_prefix("refs/tags/").unwrap_or(&tag);
            println!("{}", tag_name);
        }
    }

    /// List tags with verbose output
    async fn list_verbose(&self, refdb: &RefDatabase, tags: Vec<String>) -> Result<()> {
        let mediagit_dir = std::env::current_dir()?.join(".mediagit");

        for tag_ref in tags {
            let tag_name = tag_ref.strip_prefix("refs/tags/").unwrap_or(&tag_ref);

            // Read tag reference
            let r = refdb.read(&tag_ref).await?;
            let commit_oid = r.oid.context("Tag has no OID")?;

            // Check for metadata (annotated tag)
            let metadata_path = get_ref_path(&mediagit_dir, &format!("{}.meta", tag_ref));
            let is_annotated = tokio::fs::metadata(&metadata_path).await.is_ok();

            if is_annotated {
                if let Ok(metadata_json) = tokio::fs::read_to_string(&metadata_path).await {
                    if let Ok(metadata) = serde_json::from_str::<TagMetadata>(&metadata_json) {
                        println!("{:<20} {} (annotated)", tag_name, commit_oid.to_hex());
                        println!("  Message: {}", metadata.message.lines().next().unwrap_or(""));
                        if let Some(tagger) = metadata.tagger {
                            println!("  Tagger:  {}", tagger);
                        }
                        continue;
                    }
                }
            }

            // Lightweight tag
            println!("{:<20} {}", tag_name, commit_oid.to_hex());
        }

        Ok(())
    }

    /// Delete tag(s)
    async fn delete(&self, repo_path: PathBuf, opts: &DeleteOpts) -> Result<()> {
        let mediagit_dir = repo_path.join(".mediagit");
        let refdb = RefDatabase::new(&mediagit_dir);

        for name in &opts.names {
            let tag_ref = format!("refs/tags/{}", name);

            // Check if tag exists
            if !refdb.exists(&tag_ref).await? {
                if !opts.quiet {
                    eprintln!("Warning: Tag '{}' does not exist", name);
                }
                continue;
            }

            // Delete tag reference
            refdb.delete(&tag_ref).await.context(format!("Failed to delete tag '{}'", name))?;

            // Delete metadata if exists
            let metadata_path = get_ref_path(&mediagit_dir, &format!("{}.meta", tag_ref));
            if tokio::fs::metadata(&metadata_path).await.is_ok() {
                let _ = tokio::fs::remove_file(&metadata_path).await;
            }

            if !opts.quiet {
                println!("Deleted tag '{}'", name);
            }
        }

        Ok(())
    }

    /// Show tag information
    async fn show(&self, repo_path: PathBuf, opts: &ShowOpts) -> Result<()> {
        let mediagit_dir = repo_path.join(".mediagit");
        let refdb = RefDatabase::new(&mediagit_dir);

        let tag_ref = format!("refs/tags/{}", opts.name);

        // Check if tag exists
        if !refdb.exists(&tag_ref).await? {
            anyhow::bail!("Tag '{}' does not exist", opts.name);
        }

        // Read tag reference
        let r = refdb.read(&tag_ref).await?;
        let commit_oid = r.oid.context("Tag has no OID")?;

        println!("Tag:     {}", opts.name);
        println!("Commit:  {}", commit_oid.to_hex());

        // Check for metadata (annotated tag)
        let metadata_path = get_ref_path(&mediagit_dir, &format!("{}.meta", tag_ref));
        if let Ok(metadata_json) = tokio::fs::read_to_string(&metadata_path).await {
            if let Ok(metadata) = serde_json::from_str::<TagMetadata>(&metadata_json) {
                println!("Type:    annotated");
                println!("\nMessage:\n{}", metadata.message);

                if let Some(tagger) = metadata.tagger {
                    println!("\nTagger:  {}", tagger);
                }
                if let Some(email) = metadata.email {
                    println!("Email:   {}", email);
                }
                println!("Date:    {}", metadata.timestamp);
            }
        } else {
            println!("Type:    lightweight");
        }

        Ok(())
    }

    /// Verify tag
    async fn verify(&self, repo_path: PathBuf, opts: &VerifyOpts) -> Result<()> {
        let mediagit_dir = repo_path.join(".mediagit");
        let refdb = RefDatabase::new(&mediagit_dir);

        let tag_ref = format!("refs/tags/{}", opts.name);

        // Check if tag exists
        if !refdb.exists(&tag_ref).await? {
            anyhow::bail!("Tag '{}' does not exist", opts.name);
        }

        // Read and validate tag reference
        let r = refdb.read(&tag_ref).await.context("Failed to read tag")?;
        r.validate().context("Tag reference is invalid")?;

        let commit_oid = r.oid.context("Tag has no OID")?;

        if opts.verbose {
            println!("Tag '{}' is valid", opts.name);
            println!("  Points to: {}", commit_oid.to_hex());
            println!("  Type: {}", if r.is_tag() { "tag" } else { "unknown" });
        } else {
            println!("Tag '{}' is valid", opts.name);
        }

        Ok(())
    }

    /// Resolve commit reference to OID
    async fn resolve_commit(&self, refdb: &RefDatabase, commit_ref: &str) -> Result<Oid> {
        // Try direct OID first
        if let Ok(oid) = Oid::from_hex(commit_ref) {
            return Ok(oid);
        }

        // Try as branch reference
        let branch_ref = format!("refs/heads/{}", commit_ref);
        if refdb.exists(&branch_ref).await? {
            return refdb.resolve(&branch_ref).await;
        }

        // Try as tag reference
        let tag_ref = format!("refs/tags/{}", commit_ref);
        if refdb.exists(&tag_ref).await? {
            return refdb.resolve(&tag_ref).await;
        }

        // Try resolving directly
        refdb.resolve(commit_ref).await
            .context(format!("Cannot resolve commit reference: {}", commit_ref))
    }

    /// Validate tag name
    fn validate_tag_name(&self, name: &str) -> Result<()> {
        if name.is_empty() {
            anyhow::bail!("Tag name cannot be empty");
        }

        if name.contains("..") || name.starts_with('/') || name.ends_with('/') {
            anyhow::bail!("Invalid tag name: {}", name);
        }

        if name.contains(char::is_whitespace) {
            anyhow::bail!("Tag name cannot contain whitespace");
        }

        Ok(())
    }
}

/// Tag metadata for annotated tags
#[derive(Debug, serde::Serialize, serde::Deserialize)]
struct TagMetadata {
    tag_name: String,
    commit_oid: String,
    message: String,
    tagger: Option<String>,
    email: Option<String>,
    timestamp: String,
}

// Helper to get ref path
fn get_ref_path(mediagit_dir: &PathBuf, ref_name: &str) -> PathBuf {
    mediagit_dir.join(ref_name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    async fn setup_test_repo() -> (TempDir, PathBuf) {
        let temp_dir = TempDir::new().unwrap();
        let repo_path = temp_dir.path().to_path_buf();
        let mediagit_dir = repo_path.join(".mediagit");

        tokio::fs::create_dir_all(&mediagit_dir).await.unwrap();
        tokio::fs::create_dir_all(mediagit_dir.join("refs/tags")).await.unwrap();
        tokio::fs::create_dir_all(mediagit_dir.join("refs/heads")).await.unwrap();

        // Create HEAD pointing to main
        let refdb = RefDatabase::new(&mediagit_dir);
        let commit_oid = Oid::hash(b"test commit");
        let main_ref = Ref::new_direct("refs/heads/main".to_string(), commit_oid);
        refdb.write(&main_ref).await.unwrap();

        let head_ref = Ref::new_symbolic("HEAD".to_string(), "refs/heads/main".to_string());
        refdb.write(&head_ref).await.unwrap();

        (temp_dir, repo_path)
    }

    #[tokio::test]
    async fn test_create_lightweight_tag() {
        let (_temp, repo_path) = setup_test_repo().await;

        let cmd = TagCmd {
            subcommand: TagSubcommand::Create(CreateOpts {
                name: "v1.0.0".to_string(),
                commit: None,
                message: None,
                tagger: None,
                email: None,
                force: false,
                quiet: true,
            }),
        };

        let result = cmd.execute(repo_path.clone()).await;
        assert!(result.is_ok(), "Failed to create lightweight tag: {:?}", result.err());

        // Verify tag exists
        let mediagit_dir = repo_path.join(".mediagit");
        let refdb = RefDatabase::new(&mediagit_dir);
        assert!(refdb.exists("refs/tags/v1.0.0").await.unwrap());
    }

    #[tokio::test]
    async fn test_create_annotated_tag() {
        let (_temp, repo_path) = setup_test_repo().await;

        let cmd = TagCmd {
            subcommand: TagSubcommand::Create(CreateOpts {
                name: "v2.0.0".to_string(),
                commit: None,
                message: Some("Release version 2.0.0".to_string()),
                tagger: Some("Test User".to_string()),
                email: Some("test@example.com".to_string()),
                force: false,
                quiet: true,
            }),
        };

        let result = cmd.execute(repo_path.clone()).await;
        assert!(result.is_ok(), "Failed to create annotated tag: {:?}", result.err());

        // Verify tag and metadata exist
        let mediagit_dir = repo_path.join(".mediagit");
        let refdb = RefDatabase::new(&mediagit_dir);
        assert!(refdb.exists("refs/tags/v2.0.0").await.unwrap());
    }

    #[tokio::test]
    async fn test_list_tags() {
        let (_temp, repo_path) = setup_test_repo().await;

        // Create multiple tags
        let mediagit_dir = repo_path.join(".mediagit");
        let refdb = RefDatabase::new(&mediagit_dir);
        let oid = Oid::hash(b"commit");

        for tag in ["v1.0.0", "v1.1.0", "v2.0.0"] {
            let tag_ref = Ref::new_direct(format!("refs/tags/{}", tag), oid);
            refdb.write(&tag_ref).await.unwrap();
        }

        let cmd = TagCmd {
            subcommand: TagSubcommand::List(ListOpts {
                pattern: None,
                verbose: false,
                sort: "refname".to_string(),
                reverse: false,
            }),
        };

        let result = cmd.execute(repo_path).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_list_tags_with_pattern() {
        let (_temp, repo_path) = setup_test_repo().await;

        // Create tags
        let mediagit_dir = repo_path.join(".mediagit");
        let refdb = RefDatabase::new(&mediagit_dir);
        let oid = Oid::hash(b"commit");

        for tag in ["v1.0.0", "v1.1.0", "v2.0.0", "beta-1"] {
            let tag_ref = Ref::new_direct(format!("refs/tags/{}", tag), oid);
            refdb.write(&tag_ref).await.unwrap();
        }

        let cmd = TagCmd {
            subcommand: TagSubcommand::List(ListOpts {
                pattern: Some("v1.*".to_string()),
                verbose: false,
                sort: "refname".to_string(),
                reverse: false,
            }),
        };

        let result = cmd.execute(repo_path).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_delete_tag() {
        let (_temp, repo_path) = setup_test_repo().await;

        // Create a tag
        let mediagit_dir = repo_path.join(".mediagit");
        let refdb = RefDatabase::new(&mediagit_dir);
        let oid = Oid::hash(b"commit");
        let tag_ref = Ref::new_direct("refs/tags/delete-me".to_string(), oid);
        refdb.write(&tag_ref).await.unwrap();

        let cmd = TagCmd {
            subcommand: TagSubcommand::Delete(DeleteOpts {
                names: vec!["delete-me".to_string()],
                force: true,
                quiet: true,
            }),
        };

        let result = cmd.execute(repo_path.clone()).await;
        assert!(result.is_ok());

        // Verify tag deleted
        assert!(!refdb.exists("refs/tags/delete-me").await.unwrap());
    }

    #[tokio::test]
    async fn test_show_tag() {
        let (_temp, repo_path) = setup_test_repo().await;

        // Create annotated tag
        let mediagit_dir = repo_path.join(".mediagit");
        let refdb = RefDatabase::new(&mediagit_dir);
        let oid = Oid::hash(b"commit");
        let tag_ref = Ref::new_direct("refs/tags/v1.0.0".to_string(), oid);
        refdb.write(&tag_ref).await.unwrap();

        let cmd = TagCmd {
            subcommand: TagSubcommand::Show(ShowOpts {
                name: "v1.0.0".to_string(),
                full: false,
            }),
        };

        let result = cmd.execute(repo_path).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_verify_tag() {
        let (_temp, repo_path) = setup_test_repo().await;

        // Create tag
        let mediagit_dir = repo_path.join(".mediagit");
        let refdb = RefDatabase::new(&mediagit_dir);
        let oid = Oid::hash(b"commit");
        let tag_ref = Ref::new_direct("refs/tags/verify-me".to_string(), oid);
        refdb.write(&tag_ref).await.unwrap();

        let cmd = TagCmd {
            subcommand: TagSubcommand::Verify(VerifyOpts {
                name: "verify-me".to_string(),
                verbose: true,
            }),
        };

        let result = cmd.execute(repo_path).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_tag_name_validation() {
        let cmd = TagCmd {
            subcommand: TagSubcommand::Create(CreateOpts {
                name: "".to_string(),
                commit: None,
                message: None,
                tagger: None,
                email: None,
                force: false,
                quiet: true,
            }),
        };

        assert!(cmd.validate_tag_name("").is_err());
        assert!(cmd.validate_tag_name("v1.0.0").is_ok());
        assert!(cmd.validate_tag_name("tag with spaces").is_err());
        assert!(cmd.validate_tag_name("../../../etc/passwd").is_err());
    }

    #[tokio::test]
    async fn test_version_sorting() {
        let cmd = TagCmd {
            subcommand: TagSubcommand::List(ListOpts {
                pattern: None,
                verbose: false,
                sort: "version".to_string(),
                reverse: false,
            }),
        };

        let tags = vec![
            "refs/tags/v2.0.0".to_string(),
            "refs/tags/v1.0.0".to_string(),
            "refs/tags/v1.10.0".to_string(),
            "refs/tags/v1.2.0".to_string(),
        ];

        let sorted = cmd.sort_tags(tags, "version", false);

        assert_eq!(sorted[0], "refs/tags/v1.0.0");
        assert_eq!(sorted[1], "refs/tags/v1.2.0");
        assert_eq!(sorted[2], "refs/tags/v1.10.0");
        assert_eq!(sorted[3], "refs/tags/v2.0.0");
    }

    #[tokio::test]
    async fn test_force_tag_creation() {
        let (_temp, repo_path) = setup_test_repo().await;

        // Create initial tag
        let mediagit_dir = repo_path.join(".mediagit");
        let refdb = RefDatabase::new(&mediagit_dir);
        let oid1 = Oid::hash(b"commit1");
        let tag_ref = Ref::new_direct("refs/tags/v1.0.0".to_string(), oid1);
        refdb.write(&tag_ref).await.unwrap();

        // Try to overwrite without force (should fail)
        let cmd_no_force = TagCmd {
            subcommand: TagSubcommand::Create(CreateOpts {
                name: "v1.0.0".to_string(),
                commit: None,
                message: None,
                tagger: None,
                email: None,
                force: false,
                quiet: true,
            }),
        };

        assert!(cmd_no_force.execute(repo_path.clone()).await.is_err());

        // Overwrite with force (should succeed)
        let cmd_force = TagCmd {
            subcommand: TagSubcommand::Create(CreateOpts {
                name: "v1.0.0".to_string(),
                commit: None,
                message: None,
                tagger: None,
                email: None,
                force: true,
                quiet: true,
            }),
        };

        assert!(cmd_force.execute(repo_path).await.is_ok());
    }
}
