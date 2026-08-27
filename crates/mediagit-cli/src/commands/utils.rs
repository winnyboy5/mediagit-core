// SPDX-License-Identifier: BUSL-1.1
// Copyright (C) 2025-2026 Aswin Krishnamoorthy

//! Shared utility functions for CLI commands.

use anyhow::Result;
use chrono::Duration;

/// Size threshold above which files use streaming (constant-memory) hashing
/// instead of reading fully into memory. `add` and `status` must agree on
/// this value — otherwise they can hash the same file via different paths
/// and report false "modified" changes.
pub const STREAMING_THRESHOLD: u64 = 5 * 1024 * 1024; // 5MB

/// Format a duration as a human-readable "time ago" string.
pub fn format_duration_ago(duration: Duration) -> String {
    let secs = duration.num_seconds();
    if secs < 60 {
        format!("{} seconds ago", secs)
    } else if secs < 3600 {
        format!("{} minutes ago", secs / 60)
    } else if secs < 86400 {
        format!("{} hours ago", secs / 3600)
    } else {
        format!("{} days ago", secs / 86400)
    }
}

/// Categorize a file extension into a broad media type group.
pub fn categorize_extension(ext: &str) -> &'static str {
    match ext.to_lowercase().as_str() {
        "mp4" | "mov" | "avi" | "mkv" | "webm" | "flv" | "wmv" | "m4v" | "mxf" | "r3d" => "video",
        "wav" | "aiff" | "aif" | "mp3" | "flac" | "ogg" | "m4a" | "aac" | "opus" => "audio",
        "jpg" | "jpeg" | "png" | "tif" | "tiff" | "bmp" | "webp" | "heic" | "raw" | "dng"
        | "cr2" | "nef" | "arw" => "image",
        "psd" | "psb" | "ai" | "ait" | "indd" | "idml" | "eps" | "pdf" | "xd" => "creative",
        "glb" | "gltf" | "fbx" | "obj" | "blend" | "ma" | "mb" | "abc" | "usd" | "usda"
        | "usdc" | "usdz" | "stl" | "ply" => "3d",
        "safetensors" | "parquet" | "npz" | "onnx" | "gguf" => "model/data",
        "docx" | "xlsx" | "pptx" | "doc" | "xls" | "ppt" | "odt" | "ods" | "odp" => "office",
        _ => "other",
    }
}

/// Validate a ref name for safety.
///
/// Ref names must not contain special characters that could cause filesystem issues.
/// Based on git's ref naming rules.
pub fn validate_ref_name(name: &str) -> Result<()> {
    if name.is_empty() {
        anyhow::bail!("Ref name cannot be empty");
    }

    let prohibited_chars = ['\\', ':', '?', '*', '"', '<', '>', '|', '\0'];
    for c in prohibited_chars {
        if name.contains(c) {
            anyhow::bail!("Ref name '{}' contains prohibited character '{}'", name, c);
        }
    }

    if name.starts_with('.') || name.ends_with('.') {
        anyhow::bail!("Ref name '{}' cannot start or end with '.'", name);
    }
    if name.starts_with('/') || name.ends_with('/') {
        anyhow::bail!("Ref name '{}' cannot start or end with '/'", name);
    }
    if name.contains("..") {
        anyhow::bail!("Ref name '{}' cannot contain '..'", name);
    }
    if name.contains("//") {
        anyhow::bail!("Ref name '{}' cannot contain consecutive '/'", name);
    }
    if name.ends_with(".lock") {
        anyhow::bail!("Ref name '{}' cannot end with '.lock'", name);
    }
    if name.contains("@{") {
        anyhow::bail!("Ref name '{}' cannot contain '@{{'", name);
    }

    Ok(())
}

/// Shared test-only lock for `MEDIAGIT_REPO`, the process-global env var
/// `find_repo_root()` honors (set by the `-C` flag in production). Several
/// command test modules (merge, rebase, cherry-pick, stash) need to point
/// `find_repo_root()` at a temp repo without changing the process cwd; since
/// `cargo test` runs them concurrently in one binary, all such tests must
/// serialize through this one lock — a lock local to each module would not
/// prevent two modules from stomping the same env var at once.
#[cfg(test)]
pub(crate) mod test_support {
    use std::sync::Mutex;
    pub(crate) static REPO_ENV_LOCK: Mutex<()> = Mutex::new(());

    /// Initialize a minimal repo at `repo_path`: `.mediagit` + `refs/heads/main`
    /// pointing at a fresh empty-tree commit, with `HEAD` tracking it. Shared by
    /// merge/rebase/cherry-pick tests that need `find_repo_root()` to succeed
    /// and `HEAD` to resolve to a real commit. Returns the initial commit's OID.
    pub(crate) async fn init_repo_with_commit(
        repo_path: &std::path::Path,
    ) -> mediagit_versioning::Oid {
        use mediagit_versioning::{
            Commit, ObjectDatabase, ObjectType, Ref, RefDatabase, Signature, Tree,
        };

        let mediagit_dir = repo_path.join(".mediagit");
        tokio::fs::create_dir_all(mediagit_dir.join("refs/heads"))
            .await
            .unwrap();
        tokio::fs::create_dir_all(mediagit_dir.join("refs/tags"))
            .await
            .unwrap();

        let storage = crate::repo::create_storage_backend(repo_path)
            .await
            .unwrap();
        let odb = ObjectDatabase::with_smart_compression(storage, 1000);

        let tree_oid = Tree::new().write(&odb).await.unwrap();
        let sig = Signature::now("Test".to_string(), "test@example.com".to_string());
        let commit = Commit::new(tree_oid, sig.clone(), sig, "Initial commit".to_string());
        let commit_oid = odb
            .write(ObjectType::Commit, &commit.serialize().unwrap())
            .await
            .unwrap();

        let refdb = RefDatabase::new(&mediagit_dir);
        refdb
            .write(&Ref::new_direct("refs/heads/main".to_string(), commit_oid))
            .await
            .unwrap();
        refdb
            .write(&Ref::new_symbolic(
                "HEAD".to_string(),
                "refs/heads/main".to_string(),
            ))
            .await
            .unwrap();

        commit_oid
    }
}
