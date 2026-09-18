// SPDX-License-Identifier: BUSL-1.1
// Copyright (C) 2025-2026 Aswin Krishnamoorthy

use super::super::repo::create_storage_backend;
use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use mediagit_versioning::{
    Commit, ObjectDatabase, ObjectType, Oid, Ref, RefDatabase, Signature, Tag, Tree,
};
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
#[command(
    long_about = "Create a new tag.\n\nSigning is controlled by MEDIAGIT_SIGN environment variable.\nWhen MEDIAGIT_SIGN=true, tags are signed using the key specified in MEDIAGIT_SIGN_KEY.\nThe signature uses SSHSIG format with namespace 'mediagit-tag'."
)]
pub struct CreateOpts {
    /// Tag name
    #[arg(value_name = "NAME")]
    pub name: String,

    /// Commit to tag (defaults to HEAD)
    #[arg(value_name = "COMMIT")]
    pub commit: Option<String>,

    /// Create annotated tag
    #[arg(short = 'a', long)]
    pub annotated: bool,

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

    /// Show commit info for each tag (git-style `-n`)
    #[arg(short = 'n', long = "info")]
    pub show_info: bool,

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

/// Verify a tag. Signed tags are checked against the signer key embedded
/// in the signature (needs no local key); "valid" proves contents intact,
/// not key ownership — compare the reported fingerprint out of band.
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

        // Determine if tag should be annotated
        let is_annotated = opts.annotated
            || opts.message.is_some()
            || opts.tagger.is_some()
            || opts.email.is_some();

        // Validate: annotated tags require a message
        if is_annotated && opts.message.is_none() {
            anyhow::bail!("annotated tag requires --message");
        }

        // Check if tag already exists
        let tag_ref = format!("refs/tags/{}", opts.name);
        if refdb.exists(&tag_ref).await? && !opts.force {
            anyhow::bail!(
                "Tag '{}' already exists. Use --force to overwrite.",
                opts.name
            );
        }

        // Resolve target commit
        let target_oid = if let Some(ref commit_ref) = opts.commit {
            self.resolve_commit(&refdb, commit_ref).await?
        } else {
            // Default to HEAD
            refdb
                .resolve("HEAD")
                .await
                .context("Failed to resolve HEAD")?
        };

        // Create tag based on type
        if is_annotated {
            // Annotated tag (message is guaranteed to be Some due to validation above)
            self.create_annotated_tag(
                &repo_path,
                &refdb,
                &opts.name,
                target_oid,
                opts.message.as_ref().unwrap(),
                opts,
            )
            .await?;
        } else {
            // Lightweight tag
            self.create_lightweight_tag(&refdb, &opts.name, target_oid)
                .await?;
        }

        if !opts.quiet {
            let tag_type = if is_annotated {
                "annotated"
            } else {
                "lightweight"
            };
            println!(
                "Created {} tag '{}' at {}",
                tag_type,
                opts.name,
                target_oid.to_hex()
            );
        }

        Ok(())
    }

    /// Create lightweight tag (ref pointing directly to commit)
    async fn create_lightweight_tag(
        &self,
        refdb: &RefDatabase,
        name: &str,
        commit_oid: Oid,
    ) -> Result<()> {
        let tag_ref = format!("refs/tags/{}", name);
        let r = Ref::new_direct(tag_ref, commit_oid);
        refdb.write(&r).await?;
        Ok(())
    }

    /// Create annotated tag: writes a real `Tag` object to the ODB and
    /// points `refs/tags/<name>` at its OID (replacing the old
    /// `{tag_ref}.meta` companion-file hack).
    async fn create_annotated_tag(
        &self,
        repo_path: &std::path::Path,
        refdb: &RefDatabase,
        name: &str,
        commit_oid: Oid,
        message: &str,
        opts: &CreateOpts,
    ) -> Result<()> {
        let storage = create_storage_backend(repo_path).await?;
        let odb = ObjectDatabase::with_smart_compression(storage, 1000);

        let (tagger_name, tagger_email) = self.resolve_tagger(repo_path, opts).await;
        let tagger = Signature::now(tagger_name, tagger_email);

        // `commit_oid` is usually a real commit, but `tag create newtag
        // existingtag` where `existingtag` is itself annotated resolves to
        // the *Tag* object's OID (refs/tags/* store the tag object, not its
        // target) — detect the actual type instead of assuming Commit.
        let target_type = detect_object_type(&odb, &commit_oid).await;

        let mut tag = Tag::new(
            commit_oid,
            target_type,
            name.to_string(),
            tagger,
            message.to_string(),
        );

        if mediagit_security::sign::sign_enabled() {
            let key_path = mediagit_security::sign::default_key_path()
                .context("Failed to resolve MEDIAGIT_SIGN key path")?;
            let payload = tag.signing_payload()?;
            let signature = mediagit_security::sign::sign(&payload, &key_path)
                .context("MEDIAGIT_SIGN is enabled but signing the tag failed")?;
            tag.signature = Some(signature);
        }

        let tag_oid = tag
            .write(&odb)
            .await
            .context("Failed to write tag object")?;

        let tag_ref = format!("refs/tags/{}", name);
        let r = Ref::new_direct(tag_ref, tag_oid);
        refdb.write(&r).await?;

        Ok(())
    }

    /// Resolve the tagger identity for an annotated tag.
    /// Priority: `--tagger`/`--email` flags > `MEDIAGIT_AUTHOR_NAME` /
    /// `MEDIAGIT_AUTHOR_EMAIL` >
    /// `config.toml [author]` > `$USER` > defaults. Mirrors `commit`'s
    /// author-resolution precedence.
    async fn resolve_tagger(
        &self,
        repo_path: &std::path::Path,
        opts: &CreateOpts,
    ) -> (String, String) {
        let config = mediagit_config::Config::load(repo_path)
            .await
            .unwrap_or_default();

        let name = opts.tagger.clone().unwrap_or_else(|| {
            std::env::var("MEDIAGIT_AUTHOR_NAME").unwrap_or_else(|_| {
                config.author.name.clone().unwrap_or_else(|| {
                    std::env::var("USER").unwrap_or_else(|_| "Unknown".to_string())
                })
            })
        });
        let email = opts.email.clone().unwrap_or_else(|| {
            std::env::var("MEDIAGIT_AUTHOR_EMAIL").unwrap_or_else(|_| {
                config.author.email.clone().unwrap_or_else(|| {
                    std::env::var("USER")
                        .map(|u| format!("{}@localhost", u))
                        .unwrap_or_else(|_| "unknown@localhost".to_string())
                })
            })
        });

        (name, email)
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
        if opts.show_info {
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
        let repo_path = std::env::current_dir()?;
        let storage = create_storage_backend(&repo_path).await?;
        let odb = ObjectDatabase::with_smart_compression(storage, 1000);

        for tag_ref in tags {
            let tag_name = tag_ref.strip_prefix("refs/tags/").unwrap_or(&tag_ref);

            // Read tag reference
            let r = refdb.read(&tag_ref).await?;
            let oid = r.oid.context("Tag has no OID")?;

            // An annotated tag's ref points at a Tag object; a lightweight
            // tag's ref points directly at a commit.
            match self.read_tag_object(&odb, &oid).await {
                Some(tag) => {
                    println!("{:<20} {} (annotated)", tag_name, tag.target.to_hex());
                    println!("  Message: {}", tag.message.lines().next().unwrap_or(""));
                    println!("  Tagger:  {} <{}>", tag.tagger.name, tag.tagger.email);
                }
                None => {
                    println!("{:<20} {}", tag_name, oid.to_hex());
                }
            }
        }

        Ok(())
    }

    /// Read `oid` from the ODB and return it as a [`Tag`] if it deserializes
    /// as one (annotated tag). `None` for a lightweight tag (oid is a
    /// commit) or a missing object.
    async fn read_tag_object(&self, odb: &ObjectDatabase, oid: &Oid) -> Option<Tag> {
        let data = odb.read(oid).await.ok()?;
        Tag::deserialize(&data).ok()
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

            // Delete tag reference. Note: this does not garbage-collect the
            // Tag object itself for an annotated tag — that's `gc`'s job,
            // same as any other now-unreferenced object.
            refdb
                .delete(&tag_ref)
                .await
                .context(format!("Failed to delete tag '{}'", name))?;

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
        let oid = r.oid.context("Tag has no OID")?;

        let storage = create_storage_backend(&repo_path).await?;
        let odb = ObjectDatabase::with_smart_compression(storage, 1000);

        println!("Tag:     {}", opts.name);

        match self.read_tag_object(&odb, &oid).await {
            Some(tag) => {
                println!("Commit:  {}", tag.target.to_hex());
                println!("Type:    annotated");
                println!("\nMessage:\n{}", tag.message);
                println!("\nTagger:  {} <{}>", tag.tagger.name, tag.tagger.email);
                println!("Date:    {}", tag.tagger.timestamp);
                match &tag.signature {
                    Some(_) => println!("Signature: present (use `tag verify` to check it)"),
                    None => println!("Signature: none (unsigned)"),
                }
            }
            None => {
                println!("Commit:  {}", oid.to_hex());
                println!("Type:    lightweight");
            }
        }

        Ok(())
    }

    /// Verify a tag: reference validity for all tags, plus signature
    /// verification (valid / invalid / unsigned) for annotated tags.
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
        let oid = r.oid.context("Tag has no OID")?;

        let storage = create_storage_backend(&repo_path).await?;
        let odb = ObjectDatabase::with_smart_compression(storage, 1000);

        // BUG-VFX-1: unlike read_tag_object() (used by `show`/`list -v`,
        // which treat any failed read as "lightweight tag, nothing to show"),
        // `verify` must surface a CONTENT-HASH mismatch (tampered/corrupt tag
        // object) as a non-zero exit. But a *missing* object is not corruption:
        // a lightweight tag points straight at a commit (which may legitimately
        // be absent locally), so only an integrity failure is fatal here.
        let data = match odb.read(&oid).await {
            Ok(d) => Some(d),
            Err(e) if e.to_string().contains("integrity check failed") => {
                return Err(e).context(format!(
                    "Tag '{}': object is corrupted (content does not match its OID)",
                    opts.name
                ));
            }
            Err(_) => {
                // If a sidecar exists, this proves it was an annotated tag and the
                // object loss is a corruption, not a lightweight tag.
                let sidecar_ref = format!("refs/tag-meta/{}", opts.name);
                if refdb.exists(&sidecar_ref).await.unwrap_or(false) {
                    return Err(anyhow::anyhow!(
                        "Tag '{}': annotated tag object is missing (sidecar proves it was annotated)",
                        opts.name
                    ));
                }
                None // no sidecar -> treat as lightweight tag
            }
        };

        match data.as_deref().and_then(|d| Tag::deserialize(d).ok()) {
            Some(tag) => {
                if opts.verbose {
                    println!("Tag '{}' ref is valid (annotated)", opts.name);
                    println!("  Points to: {}", tag.target.to_hex());
                }
                match &tag.signature {
                    None => println!("Tag '{}': unsigned", opts.name),
                    Some(signature) => {
                        // Verify against the signer key embedded in the SSH
                        // signature (TOFU): proves contents intact, needs no
                        // local key. Whether the reported fingerprint is
                        // trusted is the user's call — MediaGit keeps no
                        // trust store.
                        let payload = tag.signing_payload()?;
                        match mediagit_security::sign::verify_embedded(&payload, signature)? {
                            Some(fingerprint) => println!(
                                "Tag '{}': valid signature, signed by {} \
                                 (contents intact; key ownership not verified)",
                                opts.name, fingerprint
                            ),
                            None => anyhow::bail!(
                                "Tag '{}': INVALID signature — contents do not match signature",
                                opts.name
                            ),
                        }
                    }
                }
            }
            None => {
                if opts.verbose {
                    println!("Tag '{}' is valid", opts.name);
                    println!("  Points to: {}", oid.to_hex());
                    println!(
                        "  Type: {}",
                        if r.is_tag() {
                            "lightweight tag"
                        } else {
                            "unknown"
                        }
                    );
                } else {
                    println!("Tag '{}' is valid", opts.name);
                }
            }
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
        refdb
            .resolve(commit_ref)
            .await
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

/// Detect an OID's actual object type by reading it and trying each
/// deserializer in turn (Commit, Tree, Tag; else Blob) — same ordering as
/// `mediagit_versioning::reachability`'s sniff chain. Falls back to
/// `ObjectType::Commit` if the object can't be read, matching this crate's
/// pre-existing default for the common case (tagging a commit).
async fn detect_object_type(odb: &ObjectDatabase, oid: &Oid) -> ObjectType {
    let Ok(data) = odb.read(oid).await else {
        return ObjectType::Commit;
    };
    if Commit::deserialize(&data).is_ok() {
        ObjectType::Commit
    } else if Tree::deserialize(&data).is_ok() {
        ObjectType::Tree
    } else if Tag::deserialize(&data).is_ok() {
        ObjectType::Tag
    } else {
        ObjectType::Blob
    }
}

#[cfg(test)]
#[allow(unsafe_code)] // edition-2024: test-only env::set_var/remove_var requires unsafe
mod tests {
    use super::*;
    use tempfile::TempDir;

    async fn setup_test_repo() -> (TempDir, PathBuf) {
        let temp_dir = TempDir::new().unwrap();
        let repo_path = temp_dir.path().to_path_buf();
        let mediagit_dir = repo_path.join(".mediagit");

        tokio::fs::create_dir_all(&mediagit_dir).await.unwrap();
        tokio::fs::create_dir_all(mediagit_dir.join("refs/tags"))
            .await
            .unwrap();
        tokio::fs::create_dir_all(mediagit_dir.join("refs/heads"))
            .await
            .unwrap();

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
                annotated: false,
                message: None,
                tagger: None,
                email: None,
                force: false,
                quiet: true,
            }),
        };

        let result = cmd.execute(repo_path.clone()).await;
        assert!(
            result.is_ok(),
            "Failed to create lightweight tag: {:?}",
            result.err()
        );

        // Verify tag exists
        let mediagit_dir = repo_path.join(".mediagit");
        let refdb = RefDatabase::new(&mediagit_dir);
        assert!(refdb.exists("refs/tags/v1.0.0").await.unwrap());
    }

    #[tokio::test]
    // Creates an annotated tag, so must not race the signing tests' env-var
    // mutation (see comment on test_sign_tag_when_enabled_and_verify_reports_valid).
    #[allow(clippy::await_holding_lock)]
    async fn test_create_annotated_tag() {
        let _guard = SIGN_ENV_LOCK.lock().unwrap();
        let (_temp, repo_path) = setup_test_repo().await;

        let cmd = TagCmd {
            subcommand: TagSubcommand::Create(CreateOpts {
                name: "v2.0.0".to_string(),
                commit: None,
                annotated: false,
                message: Some("Release version 2.0.0".to_string()),
                tagger: Some("Test User".to_string()),
                email: Some("test@example.com".to_string()),
                force: false,
                quiet: true,
            }),
        };

        let result = cmd.execute(repo_path.clone()).await;
        assert!(
            result.is_ok(),
            "Failed to create annotated tag: {:?}",
            result.err()
        );

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
                show_info: false,
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
                show_info: false,
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
                annotated: false,
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
                show_info: false,
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
                annotated: false,
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
                annotated: false,
                message: None,
                tagger: None,
                email: None,
                force: true,
                quiet: true,
            }),
        };

        assert!(cmd_force.execute(repo_path).await.is_ok());
    }

    #[tokio::test]
    // Asserts `tag.signature.is_none()`, so must not race the signing
    // tests' env-var mutation (see comment on
    // test_sign_tag_when_enabled_and_verify_reports_valid).
    #[allow(clippy::await_holding_lock)]
    async fn test_annotated_flag_with_message() {
        let _guard = SIGN_ENV_LOCK.lock().unwrap();
        let (_temp, repo_path) = setup_test_repo().await;

        // Create annotated tag using -a flag with -m
        let cmd = TagCmd {
            subcommand: TagSubcommand::Create(CreateOpts {
                name: "rel-1".to_string(),
                commit: None,
                annotated: true,
                message: Some("msg".to_string()),
                tagger: None,
                email: None,
                force: false,
                quiet: true,
            }),
        };

        let result = cmd.execute(repo_path.clone()).await;
        assert!(
            result.is_ok(),
            "Failed to create annotated tag with -a flag: {:?}",
            result.err()
        );

        // Verify the tag ref exists and points at a real Tag ODB object
        // (not a `.meta` companion file, which must no longer be written).
        let mediagit_dir = repo_path.join(".mediagit");
        let refdb = RefDatabase::new(&mediagit_dir);
        assert!(refdb.exists("refs/tags/rel-1").await.unwrap());

        let metadata_path = mediagit_dir.join("refs/tags/rel-1.meta");
        assert!(
            tokio::fs::metadata(&metadata_path).await.is_err(),
            ".meta companion file must not be written"
        );

        let r = refdb.read("refs/tags/rel-1").await.unwrap();
        let oid = r.oid.unwrap();
        let storage = create_storage_backend(&repo_path).await.unwrap();
        let odb = ObjectDatabase::with_smart_compression(storage, 1000);
        let data = odb.read(&oid).await.unwrap();
        let tag = Tag::deserialize(&data).expect("ref must point at a real Tag ODB object");
        assert_eq!(tag.name, "rel-1");
        assert_eq!(tag.message, "msg");
        assert_eq!(tag.target_type, ObjectType::Commit);
        assert!(tag.signature.is_none(), "MEDIAGIT_SIGN was not set");
    }

    #[tokio::test]
    // Same isolation requirement as test_annotated_flag_with_message: asserts
    // target_type on a freshly created tag, must not race the signing tests.
    #[allow(clippy::await_holding_lock)]
    async fn test_nested_annotated_tag_records_tag_target_type() {
        let _guard = SIGN_ENV_LOCK.lock().unwrap();
        let (_temp, repo_path) = setup_test_repo().await;

        // Create the first annotated tag (points at a commit).
        TagCmd {
            subcommand: TagSubcommand::Create(CreateOpts {
                name: "base".to_string(),
                commit: None,
                annotated: true,
                message: Some("base release".to_string()),
                tagger: None,
                email: None,
                force: false,
                quiet: true,
            }),
        }
        .execute(repo_path.clone())
        .await
        .expect("Failed to create base annotated tag");

        // Tag the *tag* itself: `tag create newtag base` resolves "base" via
        // refs/tags/base, which stores the Tag object's OID, not a commit's.
        TagCmd {
            subcommand: TagSubcommand::Create(CreateOpts {
                name: "nested".to_string(),
                commit: Some("base".to_string()),
                annotated: true,
                message: Some("nested release".to_string()),
                tagger: None,
                email: None,
                force: false,
                quiet: true,
            }),
        }
        .execute(repo_path.clone())
        .await
        .expect("Failed to create nested annotated tag");

        let mediagit_dir = repo_path.join(".mediagit");
        let refdb = RefDatabase::new(&mediagit_dir);
        let r = refdb.read("refs/tags/nested").await.unwrap();
        let oid = r.oid.unwrap();
        let storage = create_storage_backend(&repo_path).await.unwrap();
        let odb = ObjectDatabase::with_smart_compression(storage, 1000);
        let data = odb.read(&oid).await.unwrap();
        let tag = Tag::deserialize(&data).expect("ref must point at a real Tag ODB object");

        // The bug: target_type was previously recorded unconditionally as
        // Commit, even though `tag.target` is actually the "base" Tag
        // object's OID.
        assert_eq!(tag.target_type, ObjectType::Tag);

        let base_ref = refdb.read("refs/tags/base").await.unwrap();
        assert_eq!(tag.target, base_ref.oid.unwrap());
    }

    #[tokio::test]
    async fn test_annotated_flag_without_message() {
        let (_temp, repo_path) = setup_test_repo().await;

        // Try to create annotated tag without message (should fail)
        let cmd = TagCmd {
            subcommand: TagSubcommand::Create(CreateOpts {
                name: "rel-2".to_string(),
                commit: None,
                annotated: true,
                message: None,
                tagger: None,
                email: None,
                force: false,
                quiet: true,
            }),
        };

        let result = cmd.execute(repo_path).await;
        assert!(result.is_err(), "Should fail when -a without -m");

        let err_msg = result.err().unwrap().to_string();
        assert!(err_msg.contains("annotated tag requires --message"));
    }

    /// Serializes access to `MEDIAGIT_SIGN`/`MEDIAGIT_SIGN_KEY` across the
    /// signing tests below — env vars are process-global, and these tests
    /// run concurrently under `#[tokio::test]` in the same binary.
    static SIGN_ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn write_test_signing_key(dir: &std::path::Path) -> PathBuf {
        use ssh_key::{Algorithm, PrivateKey, rand_core::OsRng};
        let key = PrivateKey::random(&mut OsRng, Algorithm::Ed25519).unwrap();
        let pem = key.to_openssh(ssh_key::LineEnding::LF).unwrap();
        let path = dir.join("id_ed25519_signing_test");
        std::fs::write(&path, pem.as_bytes()).unwrap();
        path
    }

    #[tokio::test]
    // ponytail: std Mutex held across .await is intentional here — it's the
    // whole-test critical section guarding process-global MEDIAGIT_SIGN*
    // env vars, not an async-correctness primitive. Upgrade to an
    // async-aware mutex if these tests ever need real concurrency.
    #[allow(clippy::await_holding_lock)]
    async fn test_sign_tag_when_enabled_and_verify_reports_valid() {
        let _guard = SIGN_ENV_LOCK.lock().unwrap();
        let (_temp, repo_path) = setup_test_repo().await;
        let key_path = write_test_signing_key(_temp.path());

        // FIXME: Audit that the environment access only happens in single-threaded code.
        unsafe { std::env::set_var("MEDIAGIT_SIGN", "1") };
        // FIXME: Audit that the environment access only happens in single-threaded code.
        unsafe { std::env::set_var("MEDIAGIT_SIGN_KEY", key_path.to_str().unwrap()) };

        let create_result = TagCmd {
            subcommand: TagSubcommand::Create(CreateOpts {
                name: "signed-1".to_string(),
                commit: None,
                annotated: true,
                message: Some("signed release".to_string()),
                tagger: Some("Signer".to_string()),
                email: Some("signer@example.com".to_string()),
                force: false,
                quiet: true,
            }),
        }
        .execute(repo_path.clone())
        .await;

        // Verify the Tag object actually carries a signature.
        let mediagit_dir = repo_path.join(".mediagit");
        let refdb = RefDatabase::new(&mediagit_dir);
        let r = refdb.read("refs/tags/signed-1").await.unwrap();
        let oid = r.oid.unwrap();
        let storage = create_storage_backend(&repo_path).await.unwrap();
        let odb = ObjectDatabase::with_smart_compression(storage, 1000);
        let data = odb.read(&oid).await.unwrap();
        let tag = Tag::deserialize(&data).unwrap();

        let verify_result = TagCmd {
            subcommand: TagSubcommand::Verify(VerifyOpts {
                name: "signed-1".to_string(),
                verbose: false,
            }),
        }
        .execute(repo_path)
        .await;

        // FIXME: Audit that the environment access only happens in single-threaded code.
        unsafe { std::env::remove_var("MEDIAGIT_SIGN") };
        // FIXME: Audit that the environment access only happens in single-threaded code.
        unsafe { std::env::remove_var("MEDIAGIT_SIGN_KEY") };

        assert!(create_result.is_ok(), "{:?}", create_result.err());
        assert!(tag.signature.is_some(), "MEDIAGIT_SIGN=1 must sign the tag");
        assert!(verify_result.is_ok(), "{:?}", verify_result.err());
    }

    #[tokio::test]
    #[allow(clippy::await_holding_lock)] // see comment on the previous test
    async fn test_tag_unsigned_when_sign_disabled() {
        let _guard = SIGN_ENV_LOCK.lock().unwrap();
        let (_temp, repo_path) = setup_test_repo().await;

        // FIXME: Audit that the environment access only happens in single-threaded code.
        unsafe { std::env::remove_var("MEDIAGIT_SIGN") };
        // FIXME: Audit that the environment access only happens in single-threaded code.
        unsafe { std::env::remove_var("MEDIAGIT_SIGN_KEY") };

        TagCmd {
            subcommand: TagSubcommand::Create(CreateOpts {
                name: "unsigned-1".to_string(),
                commit: None,
                annotated: true,
                message: Some("plain release".to_string()),
                tagger: None,
                email: None,
                force: false,
                quiet: true,
            }),
        }
        .execute(repo_path.clone())
        .await
        .unwrap();

        let mediagit_dir = repo_path.join(".mediagit");
        let refdb = RefDatabase::new(&mediagit_dir);
        let r = refdb.read("refs/tags/unsigned-1").await.unwrap();
        let oid = r.oid.unwrap();
        let storage = create_storage_backend(&repo_path).await.unwrap();
        let odb = ObjectDatabase::with_smart_compression(storage, 1000);
        let data = odb.read(&oid).await.unwrap();
        let tag = Tag::deserialize(&data).unwrap();

        assert!(
            tag.signature.is_none(),
            "MEDIAGIT_SIGN unset must produce an unsigned tag"
        );
    }

    #[tokio::test]
    #[allow(clippy::await_holding_lock)] // see comment on test_sign_tag_when_enabled...
    async fn test_verify_reports_invalid_on_tampered_signature() {
        let _guard = SIGN_ENV_LOCK.lock().unwrap();
        let (_temp, repo_path) = setup_test_repo().await;
        let key_path = write_test_signing_key(_temp.path());

        // FIXME: Audit that the environment access only happens in single-threaded code.
        unsafe { std::env::set_var("MEDIAGIT_SIGN", "1") };
        // FIXME: Audit that the environment access only happens in single-threaded code.
        unsafe { std::env::set_var("MEDIAGIT_SIGN_KEY", key_path.to_str().unwrap()) };

        TagCmd {
            subcommand: TagSubcommand::Create(CreateOpts {
                name: "tampered-1".to_string(),
                commit: None,
                annotated: true,
                message: Some("will be tampered".to_string()),
                tagger: None,
                email: None,
                force: false,
                quiet: true,
            }),
        }
        .execute(repo_path.clone())
        .await
        .unwrap();

        // Tamper with the stored Tag object's message after the fact (the
        // signature was computed over the original message), write the
        // tampered object back and repoint the ref so the actual `tag
        // verify` subcommand sees it — this exercises the CLI INVALID
        // branch end-to-end, not just the sign primitive.
        let mediagit_dir = repo_path.join(".mediagit");
        let refdb = RefDatabase::new(&mediagit_dir);
        let r = refdb.read("refs/tags/tampered-1").await.unwrap();
        let oid = r.oid.unwrap();
        let storage = create_storage_backend(&repo_path).await.unwrap();
        let odb = ObjectDatabase::with_smart_compression(storage.clone(), 1000);
        let data = odb.read(&oid).await.unwrap();
        let mut tag = Tag::deserialize(&data).unwrap();
        tag.message = "tampered message".to_string();
        let tampered_oid = tag.write(&odb).await.unwrap();
        refdb
            .write(&Ref::new_direct(
                "refs/tags/tampered-1".to_string(),
                tampered_oid,
            ))
            .await
            .unwrap();

        // Verification must need no local key (the signer key is embedded
        // in the signature), so drop the env vars before verifying.
        // FIXME: Audit that the environment access only happens in single-threaded code.
        unsafe { std::env::remove_var("MEDIAGIT_SIGN") };
        // FIXME: Audit that the environment access only happens in single-threaded code.
        unsafe { std::env::remove_var("MEDIAGIT_SIGN_KEY") };
        drop(key_path);

        let verify_result = TagCmd {
            subcommand: TagSubcommand::Verify(VerifyOpts {
                name: "tampered-1".to_string(),
                verbose: false,
            }),
        }
        .execute(repo_path)
        .await;

        let err = verify_result.expect_err("tampered tag must fail verification");
        assert!(
            err.to_string().contains("INVALID signature"),
            "error must report INVALID, got: {err}"
        );
    }

    #[tokio::test]
    #[allow(clippy::await_holding_lock)] // see comment on test_sign_tag_when_enabled...
    async fn test_verify_needs_no_local_key() {
        let _guard = SIGN_ENV_LOCK.lock().unwrap();
        let (_temp, repo_path) = setup_test_repo().await;
        let key_path = write_test_signing_key(_temp.path());

        // FIXME: Audit that the environment access only happens in single-threaded code.
        unsafe { std::env::set_var("MEDIAGIT_SIGN", "1") };
        // FIXME: Audit that the environment access only happens in single-threaded code.
        unsafe { std::env::set_var("MEDIAGIT_SIGN_KEY", key_path.to_str().unwrap()) };

        TagCmd {
            subcommand: TagSubcommand::Create(CreateOpts {
                name: "portable-1".to_string(),
                commit: None,
                annotated: true,
                message: Some("verifiable anywhere".to_string()),
                tagger: None,
                email: None,
                force: false,
                quiet: true,
            }),
        }
        .execute(repo_path.clone())
        .await
        .unwrap();

        // Simulate a different machine: signing key gone, env cleared. The
        // embedded-key (TOFU) model must still verify successfully.
        // FIXME: Audit that the environment access only happens in single-threaded code.
        unsafe { std::env::remove_var("MEDIAGIT_SIGN") };
        // FIXME: Audit that the environment access only happens in single-threaded code.
        unsafe { std::env::remove_var("MEDIAGIT_SIGN_KEY") };
        std::fs::remove_file(&key_path).unwrap();

        let verify_result = TagCmd {
            subcommand: TagSubcommand::Verify(VerifyOpts {
                name: "portable-1".to_string(),
                verbose: false,
            }),
        }
        .execute(repo_path)
        .await;

        assert!(
            verify_result.is_ok(),
            "verify must not require any local key: {:?}",
            verify_result.err()
        );
    }
}
