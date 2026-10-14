// SPDX-License-Identifier: BUSL-1.1
// Copyright (C) 2025-2026 Aswin Krishnamoorthy

use super::super::repo::{create_storage_backend, find_repo_root};
use anyhow::{Context, Result};
use clap::Parser;
use console::style;
use mediagit_versioning::{Commit, ObjectDatabase, Oid, RefDatabase, Tag, Tree, resolve_revision};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Show object information
#[derive(Parser, Debug)]
pub struct ShowCmd {
    /// Object to show (commit, tag, tree, blob) - defaults to HEAD
    #[arg(value_name = "OBJECT")]
    pub object: Option<String>,

    /// Show patch (not yet implemented)
    #[arg(short = 'p', long, hide = true)]
    pub patch: bool,

    /// Show file change statistics (not yet implemented)
    #[arg(long, hide = true)]
    pub stat: bool,

    /// Show pretty format (not yet implemented)
    #[arg(long, value_name = "FORMAT", hide = true)]
    pub pretty: Option<String>,

    /// Number of context lines (not yet implemented)
    #[arg(short = 'U', long, value_name = "NUM", hide = true)]
    pub unified: Option<usize>,

    /// Quiet mode
    #[arg(short, long)]
    pub quiet: bool,

    /// Verbose mode
    #[arg(short, long)]
    pub verbose: bool,
}

impl ShowCmd {
    pub async fn execute(&self) -> Result<()> {
        // UX-5: declared but never read. `diff --stat` does work.
        if self.stat {
            anyhow::bail!(
                "show --stat is not yet implemented. Use `mediagit diff --stat` instead."
            );
        }
        if self.quiet {
            return Ok(());
        }

        let repo_root = find_repo_root()?;
        let storage_path = repo_root.join(".mediagit");
        let storage = create_storage_backend(&repo_root).await?;
        let refdb = RefDatabase::new(&storage_path);
        let odb = ObjectDatabase::with_smart_compression(storage, 1000);

        // Resolve object ID using revision parser (supports HEAD~N)
        let object_str = self.object.as_deref().unwrap_or("HEAD");

        // `<rev>:<path>` — historical file access. Split on the FIRST ':' so
        // OIDs/refs never contain one; only the path may.
        if let Some((rev, path)) = object_str.split_once(':') {
            return Self::show_path_at_revision(rev, path, &refdb, &odb).await;
        }

        let oid = resolve_revision(object_str, &refdb, &odb)
            .await
            .context(format!("Cannot resolve object: {}", object_str))?;

        // A chunked object is a large blob (never a commit or tag): describe it
        // from its manifest instead of reassembling it in RAM.
        if let Some(manifest) = odb.get_chunk_manifest(&oid).await? {
            let mut head = Vec::new();
            for chunk in &manifest.chunks {
                if head.len() >= 256 {
                    break;
                }
                head.extend_from_slice(&odb.get_chunk(&chunk.id).await?);
            }
            Self::print_raw_object_info(&oid, manifest.total_size as usize, &head, self.verbose);
            return Ok(());
        }

        // Read object
        let data = odb
            .read(&oid)
            .await
            .context(format!("Failed to read object {}", oid))?;

        // Try to deserialize as commit
        match Commit::deserialize(&data) {
            Ok(commit) => {
                println!(
                    "{} {}",
                    style("commit").yellow().bold(),
                    style(&oid).yellow()
                );
                println!("Author: {} <{}>", commit.author.name, commit.author.email);
                println!("Date:   {}", commit.author.timestamp);
                println!();
                for line in commit.message.lines() {
                    println!("    {}", line);
                }
                println!();

                if self.verbose {
                    println!("Tree: {}", commit.tree);
                    if !commit.parents.is_empty() {
                        println!("Parents:");
                        for parent in &commit.parents {
                            println!("  {}", parent);
                        }
                    }
                    println!();
                }

                // Show file changes (always for show, or when --stat explicitly)
                let current_tree_files = Self::get_tree_file_list(&odb, &commit.tree)
                    .await
                    .unwrap_or_default();

                let parent_tree_files = if let Some(parent_oid) = commit.parents.first() {
                    match odb.read(parent_oid).await {
                        Ok(parent_data) => match Commit::deserialize(&parent_data) {
                            Ok(parent_commit) => {
                                Self::get_tree_file_list(&odb, &parent_commit.tree)
                                    .await
                                    .unwrap_or_default()
                            }
                            _ => HashMap::new(),
                        },
                        _ => HashMap::new(),
                    }
                } else {
                    HashMap::new()
                };

                let mut added = Vec::new();
                let mut modified = Vec::new();
                let mut deleted = Vec::new();

                for (path, current_oid) in &current_tree_files {
                    match parent_tree_files.get(path) {
                        Some(parent_oid) if parent_oid != current_oid => {
                            modified.push(path.clone());
                        }
                        None => {
                            added.push(path.clone());
                        }
                        _ => {}
                    }
                }

                for path in parent_tree_files.keys() {
                    if !current_tree_files.contains_key(path) {
                        deleted.push(path.clone());
                    }
                }

                let total_changes = added.len() + modified.len() + deleted.len();
                if total_changes > 0 {
                    println!("---");
                    for path in &added {
                        println!(" {} | {}", path.display(), style("new file").green());
                        Self::print_media_line(&odb, &current_tree_files, path).await;
                    }
                    for path in &modified {
                        println!(" {} | {}", path.display(), style("modified").yellow());
                        Self::print_media_line(&odb, &current_tree_files, path).await;
                    }
                    for path in &deleted {
                        println!(" {} | {}", path.display(), style("deleted").red());
                    }
                    println!(
                        " {} file(s) changed, {} added, {} modified, {} deleted",
                        total_changes,
                        added.len(),
                        modified.len(),
                        deleted.len()
                    );
                    println!();
                } else if commit.parents.is_empty() {
                    // Root commit: show all files
                    println!("---");
                    for path in current_tree_files.keys() {
                        println!(" {} | {}", path.display(), style("new file").green());
                    }
                    println!(" {} file(s) in initial commit", current_tree_files.len());
                    println!();
                }
            }
            Err(_) if Tag::deserialize(&data).is_ok() => {
                let tag = Tag::deserialize(&data).expect("checked Ok above");
                println!(
                    "{} {} ({})",
                    style("tag").yellow().bold(),
                    style(&tag.name).yellow(),
                    oid
                );
                println!("Tagger: {} <{}>", tag.tagger.name, tag.tagger.email);
                println!("Date:   {}", tag.tagger.timestamp);
                println!();
                for line in tag.message.lines() {
                    println!("    {}", line);
                }
                println!();
                println!(
                    "{} {} {}",
                    style("target").cyan().bold(),
                    tag.target_type,
                    style(&tag.target).yellow()
                );
                match &tag.signature {
                    Some(_) => {
                        println!("Signature: present (use `mediagit tag verify` to check it)")
                    }
                    None => println!("Signature: none (unsigned)"),
                }
            }
            Err(_) => {
                // Not a commit or tag, show raw object info
                Self::print_raw_object_info(&oid, data.len(), &data, self.verbose);
            }
        }

        Ok(())
    }

    /// Raw-object summary. `head` must hold at least the first
    /// `min(size, 256)` bytes of the object.
    fn print_raw_object_info(oid: &Oid, size: usize, head: &[u8], verbose: bool) {
        println!("{} {}", style("object").cyan().bold(), style(oid).yellow());
        println!("Size: {} bytes", size);

        if verbose {
            // Show hex dump of first 256 bytes
            let display_len = size.min(256);
            println!("\nFirst {} bytes (hex):", display_len);
            for (i, chunk) in head[..display_len].chunks(16).enumerate() {
                print!("{:08x}  ", i * 16);
                for byte in chunk {
                    print!("{:02x} ", byte);
                }
                println!();
            }
            if size > 256 {
                println!("... ({} more bytes)", size - 256);
            }
        }
    }

    /// Print the raw bytes of `path` as it existed in `rev` (historical file
    /// access, e.g. `show <oid>:frame.txt`). `rev` may be an OID, branch, or
    /// tag (annotated tags are peeled to their target commit).
    async fn show_path_at_revision(
        rev: &str,
        path: &str,
        refdb: &RefDatabase,
        odb: &ObjectDatabase,
    ) -> Result<()> {
        Self::write_path_at_revision(rev, path, refdb, odb, &mut tokio::io::stdout()).await
    }

    async fn write_path_at_revision<W: tokio::io::AsyncWrite + Unpin + Send>(
        rev: &str,
        path: &str,
        refdb: &RefDatabase,
        odb: &ObjectDatabase,
        out: &mut W,
    ) -> Result<()> {
        let oid = resolve_revision(rev, refdb, odb)
            .await
            .context(format!("Cannot resolve revision: {}", rev))?;
        let commit_oid = Self::peel_to_commit(oid, odb).await?;
        let data = odb.read(&commit_oid).await?;
        let commit =
            Commit::deserialize(&data).context(format!("Object {} is not a commit", commit_oid))?;

        let file_oid = Self::find_path_in_tree(odb, &commit.tree, path)
            .await
            .context(format!("Path '{}' not found in revision '{}'", path, rev))?;
        // Streamed: the blob may be far larger than RAM.
        use tokio::io::AsyncWriteExt;
        odb.stream_object_to(&file_oid, out)
            .await
            .context(format!("Failed to read blob for '{}'", path))?;
        out.flush().await?;
        Ok(())
    }

    /// Follow a Tag object to its target, repeating until a Commit is
    /// reached (lightweight tags/branches/OIDs already point at a commit and
    /// return immediately).
    pub(crate) async fn peel_to_commit(mut oid: Oid, odb: &ObjectDatabase) -> Result<Oid> {
        loop {
            let data = odb
                .read(&oid)
                .await
                .context(format!("Failed to read object {}", oid))?;
            if Commit::deserialize(&data).is_ok() {
                return Ok(oid);
            }
            match Tag::deserialize(&data) {
                Ok(tag) => oid = tag.target,
                Err(_) => anyhow::bail!("Object {} is not a commit or tag", oid),
            }
        }
    }

    /// Look up `path` (`a/b/c`) in the commit's tree and return the OID of
    /// the blob.
    ///
    /// Commits build a single-level (flat) tree keyed by full relative path
    /// (see `commit.rs`); no nested `Directory` entries are ever produced.
    // ponytail: flat-tree direct key lookup, no subtree walk needed.
    // Upgrade path: nested trees, if that's ever adopted.
    pub(crate) async fn find_path_in_tree(
        odb: &ObjectDatabase,
        tree_oid: &Oid,
        path: &str,
    ) -> Result<Oid> {
        let normalized = path.replace('\\', "/");
        let key = normalized
            .split('/')
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join("/");
        if key.is_empty() {
            anyhow::bail!("Empty path");
        }

        let tree_data = odb.read(tree_oid).await?;
        let tree = Tree::deserialize(&tree_data)?;
        let entry = tree
            .entries
            .get(key.as_str())
            .ok_or_else(|| anyhow::anyhow!("'{}' not found", path))?;
        Ok(entry.oid)
    }

    /// Print a `media: ...` metadata line for a changed file, if applicable.
    ///
    /// Reads the blob bytes via the object database (never the working tree)
    /// and parses them with `mediagit-media`. Silent on any failure or when
    /// the file isn't a recognized media type.
    async fn print_media_line(
        odb: &ObjectDatabase,
        current_tree_files: &HashMap<PathBuf, Oid>,
        path: &Path,
    ) {
        let Some(oid) = current_tree_files.get(path) else {
            return;
        };
        // Size first (manifest only for chunked blobs): never load a blob the
        // summary would reject anyway.
        match odb.get_object_size(oid).await {
            Ok(size) if size as u64 <= super::super::media_meta::MAX_MEDIA_META_BYTES => {}
            _ => return,
        }
        let Ok(data) = odb.read(oid).await else {
            return;
        };
        if let Some(line) =
            super::super::media_meta::media_summary_line(&data, &path.to_string_lossy()).await
        {
            println!("   {}", style(line).dim());
        }
    }

    /// Helper to get a flat map of file paths to OIDs from a tree
    async fn get_tree_file_list(
        odb: &ObjectDatabase,
        tree_oid: &Oid,
    ) -> Result<HashMap<PathBuf, Oid>> {
        let mut files = HashMap::new();
        Self::walk_tree(odb, tree_oid, &PathBuf::new(), &mut files).await?;
        Ok(files)
    }

    /// Recursively walk a tree, collecting file entries
    fn walk_tree<'a>(
        odb: &'a ObjectDatabase,
        tree_oid: &'a Oid,
        prefix: &'a Path,
        files: &'a mut HashMap<PathBuf, Oid>,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<()>> + 'a>> {
        Box::pin(async move {
            let tree_data = odb.read(tree_oid).await?;
            let tree: Tree = mediagit_versioning::format::deserialize(&tree_data)?;

            for entry in tree.iter() {
                let entry_path = prefix.join(&entry.name);
                match entry.mode {
                    mediagit_versioning::FileMode::Directory => {
                        Self::walk_tree(odb, &entry.oid, &entry_path, files).await?;
                    }
                    _ => {
                        files.insert(entry_path, entry.oid);
                    }
                }
            }
            Ok(())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mediagit_storage::mock::MockBackend;
    use mediagit_versioning::{ChunkStrategy, FileMode, ObjectType, Signature, TreeEntry};
    use std::sync::Arc;

    /// `show <rev>:<path>` of a chunked blob must stream it, never `read()`
    /// it whole. `read()` bumps a cache hit/miss counter on every call; the
    /// lookup itself reads exactly three objects (peel, commit, tree).
    #[tokio::test]
    async fn show_path_streams_chunked_blob_without_whole_read() {
        let odb = ObjectDatabase::with_optimizations(
            Arc::new(MockBackend::new()),
            100,
            Some(ChunkStrategy::Fixed { size: 256 * 1024 }),
            false,
            0,
        );
        let data: Vec<u8> = (0..2 * 1024 * 1024).map(|i| (i % 251) as u8).collect();
        let blob = odb
            .write_chunked(ObjectType::Blob, &data, "big.bin")
            .await
            .unwrap();
        assert!(odb.is_chunked(&blob).await.unwrap());

        let mut tree = Tree::new();
        tree.add_entry(TreeEntry::new("big.bin".into(), FileMode::Regular, blob));
        let tree_oid = tree.write(&odb).await.unwrap();
        let sig = Signature::now("T".into(), "t@example.com".into());
        let commit = Commit::new(tree_oid, sig.clone(), sig, "m".into());
        let commit_oid = commit.write(&odb).await.unwrap();

        let dir = tempfile::TempDir::new().unwrap();
        let refdb = RefDatabase::new(dir.path());
        let before = odb.metrics().await;
        let mut out = Vec::new();
        ShowCmd::write_path_at_revision(&commit_oid.to_hex(), "big.bin", &refdb, &odb, &mut out)
            .await
            .unwrap();
        let after = odb.metrics().await;

        assert_eq!(out, data);
        let reads =
            (after.cache_hits - before.cache_hits) + (after.cache_misses - before.cache_misses);
        assert_eq!(reads, 3, "blob must not go through odb.read()");
    }
}
