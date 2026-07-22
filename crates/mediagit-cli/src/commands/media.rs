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

//! Inspect media file metadata (#7): `mediagit media info <path>`.
//!
//! A subcommand group (only `info` this cycle, room for more later) that
//! surfaces the full detail from the `mediagit-media` format parsers —
//! image, video, audio, PSD, and 3D — for a file in the working tree.
//! Unlike `media_meta.rs`'s one-line `media: ...` summary (used by
//! `status`/`show`/`stats`), this prints every top-level field as
//! `label: value` lines, or the full parsed struct as `--json`.

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use std::path::PathBuf;

/// Blobs larger than this are never parsed — mirrors `media_meta.rs`'s cap
/// so `media info` and the `status`/`show` summary lines agree on what
/// "too large to inspect" means.
const MAX_MEDIA_INFO_BYTES: u64 = 256 * 1024 * 1024;

/// Inspect media file metadata
#[derive(Parser, Debug)]
pub struct MediaCmd {
    #[command(subcommand)]
    pub subcommand: MediaSubcommand,
}

#[derive(Subcommand, Debug)]
pub enum MediaSubcommand {
    /// Show metadata for an image/video/audio/PSD/3D-model file
    Info(InfoOpts),
}

/// Show metadata for a media file
#[derive(Parser, Debug)]
#[command(after_help = "EXAMPLES:
    # Human-readable label: value output
    mediagit media info assets/hero.png

    # Machine-readable JSON (the full parsed metadata struct)
    mediagit media info assets/clip.mp4 --json

SEE ALSO:
    mediagit-status(1), mediagit-show(1)")]
pub struct InfoOpts {
    /// Path to the media file (in the working tree)
    #[arg(value_name = "PATH")]
    pub path: PathBuf,

    /// Output the full parsed metadata as JSON
    #[arg(long)]
    pub json: bool,
}

impl MediaCmd {
    pub async fn execute(&self) -> Result<()> {
        match &self.subcommand {
            MediaSubcommand::Info(opts) => opts.execute().await,
        }
    }
}

impl InfoOpts {
    pub async fn execute(&self) -> Result<()> {
        let filename = self.path.to_string_lossy().to_string();
        let Some(ext) = self
            .path
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_lowercase())
        else {
            println!("Not a recognized media file: {filename}");
            return Ok(());
        };

        if !is_supported_extension(&ext) {
            println!(
                "Unsupported media type: '.{ext}' — no metadata parser available for {filename}"
            );
            return Ok(());
        }

        let size = std::fs::metadata(&self.path)
            .with_context(|| format!("Failed to stat '{filename}'"))?
            .len();
        if size > MAX_MEDIA_INFO_BYTES {
            println!(
                "{filename}: {} exceeds the {} MB media-metadata cap, skipping",
                indicatif::HumanBytes(size),
                MAX_MEDIA_INFO_BYTES / (1024 * 1024)
            );
            return Ok(());
        }

        let data =
            std::fs::read(&self.path).with_context(|| format!("Failed to read '{filename}'"))?;

        match ext.as_str() {
            "jpg" | "jpeg" | "png" | "tif" | "tiff" | "webp" => {
                self.show_image(&data, &filename).await
            }
            "mp4" | "mov" | "m4v" => self.show_video(&data).await,
            "wav" | "mp3" | "flac" | "aac" | "ogg" | "m4a" => {
                self.show_audio(&data, &filename).await
            }
            "psd" => self.show_psd(&data).await,
            "obj" | "fbx" | "blend" | "gltf" | "glb" | "stl" | "usd" | "usda" | "usdc" | "usdz"
            | "ply" => self.show_model3d(&data, &filename).await,
            _ => unreachable!("is_supported_extension() gated this match"),
        }
    }

    async fn show_image(&self, data: &[u8], filename: &str) -> Result<()> {
        let meta = mediagit_media::ImageMetadataParser::parse(data, filename).await?;
        if self.json {
            print_json(&meta)
        } else {
            println!("format: {:?}", meta.format);
            println!("width: {}", meta.width);
            println!("height: {}", meta.height);
            println!("file_size: {}", meta.file_size);
            if let Some(cs) = &meta.color_space {
                println!("color_space: {cs}");
            }
            if let Some(bd) = meta.bit_depth {
                println!("bit_depth: {bd}");
            }
            println!("perceptual_hash: {}", meta.perceptual_hash.hash_value);
            Ok(())
        }
    }

    async fn show_video(&self, data: &[u8]) -> Result<()> {
        let info = mediagit_media::VideoParser::new().parse(data).await?;
        if self.json {
            print_json(&info)
        } else {
            println!("duration_seconds: {:.2}", info.duration_seconds);
            println!("brand: {}", info.brand);
            if let Some(codec) = &info.video_codec {
                println!("video_codec: {codec}");
            }
            if let Some(codec) = &info.audio_codec {
                println!("audio_codec: {codec}");
            }
            if let Some(track) = info.tracks.iter().find(|t| t.track_type == "video")
                && let (Some(w), Some(h)) = (track.width, track.height)
            {
                println!("width: {w}");
                println!("height: {h}");
            }
            println!("tracks: {}", info.tracks.len());
            Ok(())
        }
    }

    async fn show_audio(&self, data: &[u8], filename: &str) -> Result<()> {
        let info = mediagit_media::AudioParser::new()
            .parse(data, filename)
            .await?;
        if self.json {
            print_json(&info)
        } else {
            println!("duration_seconds: {:.2}", info.duration_seconds);
            println!("sample_rate: {}", info.sample_rate);
            println!("channels: {}", info.channels);
            println!("codec: {}", info.codec);
            if let Some(bd) = info.bit_depth {
                println!("bit_depth: {bd}");
            }
            if let Some(br) = info.bitrate {
                println!("bitrate: {br}");
            }
            println!("tracks: {}", info.tracks.len());
            Ok(())
        }
    }

    async fn show_psd(&self, data: &[u8]) -> Result<()> {
        let info = mediagit_media::PsdParser::new().parse(data).await?;
        if self.json {
            print_json(&info)
        } else {
            println!("width: {}", info.width);
            println!("height: {}", info.height);
            println!("depth: {}", info.depth);
            println!("color_mode: {}", info.color_mode);
            println!("channels: {}", info.channels);
            println!("layers: {}", info.layers.len());
            println!("groups: {}", info.groups.len());
            Ok(())
        }
    }

    async fn show_model3d(&self, data: &[u8], filename: &str) -> Result<()> {
        let info = mediagit_media::Model3DParser::new()
            .parse(data, filename)
            .await?;
        if self.json {
            print_json(&info)
        } else {
            println!("format: {:?}", info.format);
            match info.vertex_count {
                Some(v) => println!("vertex_count: {v}"),
                None => println!("vertex_count: n/a (not parsed)"),
            }
            match info.face_count {
                Some(f) => println!("face_count: {f}"),
                None => println!("face_count: n/a (not parsed)"),
            }
            println!("object_count: {}", info.object_count);
            println!("materials: {}", info.materials.len());
            println!("textures: {}", info.textures.len());
            println!("has_animations: {}", info.has_animations);
            println!("has_rigging: {}", info.has_rigging);
            if let Some(bbox) = &info.bounding_box {
                println!(
                    "bounding_box: [{:.3}, {:.3}, {:.3}] .. [{:.3}, {:.3}, {:.3}]",
                    bbox.min.0, bbox.min.1, bbox.min.2, bbox.max.0, bbox.max.1, bbox.max.2
                );
            }
            Ok(())
        }
    }
}

fn print_json<T: serde::Serialize>(value: &T) -> Result<()> {
    println!("{}", serde_json::to_string_pretty(value)?);
    Ok(())
}

fn is_supported_extension(ext: &str) -> bool {
    matches!(
        ext,
        "jpg"
            | "jpeg"
            | "png"
            | "tif"
            | "tiff"
            | "webp"
            | "mp4"
            | "mov"
            | "m4v"
            | "wav"
            | "mp3"
            | "flac"
            | "aac"
            | "ogg"
            | "m4a"
            | "psd"
            | "obj"
            | "fbx"
            | "blend"
            | "gltf"
            | "glb"
            | "stl"
            | "usd"
            | "usda"
            | "usdc"
            | "usdz"
            | "ply"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn supported_extensions_cover_every_format() {
        assert!(is_supported_extension("png"));
        assert!(is_supported_extension("mp4"));
        assert!(is_supported_extension("wav"));
        assert!(is_supported_extension("psd"));
        assert!(is_supported_extension("glb"));
        assert!(is_supported_extension("stl"));
        assert!(is_supported_extension("ply"));
        assert!(!is_supported_extension("txt"));
        assert!(!is_supported_extension(""));
    }
}
