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

//! Read-only media metadata summary lines for `show`/`stats`.
//!
//! Wraps the format-specific parsers in `mediagit-media` to produce a short
//! `media: ...` line for display purposes only. Never writes anything, never
//! touches storage/chunking/delta paths — bytes are read via the existing
//! `ObjectDatabase::read` by the caller and handed to us.
//!
//! Set `MEDIAGIT_MEDIA_META=0` to disable all parsing/printing and restore
//! prior output exactly.

use std::path::Path;

/// Blobs larger than this are skipped (never parsed for metadata).
pub(crate) const MAX_MEDIA_META_BYTES: u64 = 256 * 1024 * 1024;

/// Kill switch: `MEDIAGIT_MEDIA_META=0` disables all metadata parsing/printing.
pub fn media_meta_enabled() -> bool {
    std::env::var("MEDIAGIT_MEDIA_META")
        .map(|v| v != "0")
        .unwrap_or(true)
}

/// Build a short `media: ...` summary line for a blob, if `filename`'s
/// extension is a supported image/video/audio/PSD format.
///
/// Returns `None` silently (logging at debug level) when: metadata display is
/// disabled via the kill switch, the blob exceeds the size cap, the extension
/// isn't recognized, or parsing fails (e.g. malformed/truncated file).
pub async fn media_summary_line(data: &[u8], filename: &str) -> Option<String> {
    if !media_meta_enabled() {
        return None;
    }
    if data.len() as u64 > MAX_MEDIA_META_BYTES {
        return None;
    }

    let ext = Path::new(filename).extension()?.to_str()?.to_lowercase();

    let result = match ext.as_str() {
        "jpg" | "jpeg" | "png" | "tif" | "tiff" | "webp" => image_summary(data, filename).await,
        "mp4" | "mov" | "m4v" => video_summary(data).await,
        "wav" | "mp3" | "flac" | "aac" | "ogg" | "m4a" => audio_summary(data, filename, &ext).await,
        "psd" => psd_summary(data).await,
        "obj" | "fbx" | "blend" | "gltf" | "glb" | "stl" | "usd" | "usda" | "usdc" | "usdz"
        | "ply" => model3d_summary(data, filename).await,
        _ => return None,
    };

    match result {
        Ok(line) => Some(line),
        Err(e) => {
            tracing::debug!("media metadata parse failed for {}: {}", filename, e);
            None
        }
    }
}

async fn image_summary(data: &[u8], filename: &str) -> mediagit_media::Result<String> {
    let meta = mediagit_media::ImageMetadataParser::parse(data, filename).await?;
    Ok(format!(
        "media: {}x{} {}",
        meta.width,
        meta.height,
        format!("{:?}", meta.format).to_lowercase()
    ))
}

async fn video_summary(data: &[u8]) -> mediagit_media::Result<String> {
    let info = mediagit_media::VideoParser::new().parse(data).await?;
    let dims = info
        .tracks
        .iter()
        .find(|t| t.track_type == "video")
        .and_then(|t| Some((t.width?, t.height?)));
    let codec = info
        .video_codec
        .as_deref()
        .unwrap_or("unknown")
        .to_lowercase();

    Ok(match dims {
        Some((w, h)) => format!("media: {}x{} {} {:.1}s", w, h, codec, info.duration_seconds),
        None => format!("media: {} {:.1}s", codec, info.duration_seconds),
    })
}

async fn audio_summary(data: &[u8], filename: &str, ext: &str) -> mediagit_media::Result<String> {
    let info = mediagit_media::AudioParser::new()
        .parse(data, filename)
        .await?;
    // NOTE: AudioInfo.codec is symphonia's raw `CodecType` Debug output, which
    // is an unlabeled numeric tuple (e.g. "CodecType(4353)"), not a readable
    // name — the file extension is a more useful codec label here.
    Ok(format!(
        "media: {}Hz {}ch {} {:.1}s",
        info.sample_rate, info.channels, ext, info.duration_seconds
    ))
}

async fn model3d_summary(data: &[u8], filename: &str) -> mediagit_media::Result<String> {
    let info = mediagit_media::Model3DParser::new()
        .parse(data, filename)
        .await?;
    Ok(format!(
        "media: {:?} {} verts, {} faces, {} object(s)",
        info.format, info.vertex_count, info.face_count, info.object_count
    ))
}

async fn psd_summary(data: &[u8]) -> mediagit_media::Result<String> {
    let info = mediagit_media::PsdParser::new().parse(data).await?;
    // NOTE: the `psd` crate doesn't expose document DPI, so we show
    // dimensions instead of resolution alongside the layer count.
    Ok(format!(
        "media: PSD {}x{} {} layers",
        info.width,
        info.height,
        info.layers.len()
    ))
}
