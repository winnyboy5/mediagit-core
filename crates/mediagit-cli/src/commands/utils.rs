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

//! Shared utility functions for CLI commands.

use anyhow::Result;
use chrono::Duration;

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
