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

use super::*;

/// Compression strategy selection
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompressionStrategy {
    /// No compression (store as-is)
    Store,

    /// Zlib compression (Git-compatible)
    Zlib(CompressionLevel),

    /// Zstd compression (fast, good ratio)
    Zstd(CompressionLevel),

    /// Brotli compression (best ratio, slower)
    Brotli(CompressionLevel),

    /// Delta compression (for similar files)
    Delta,
}

impl CompressionStrategy {
    /// Select optimal strategy for object type
    pub fn for_object_type(obj_type: ObjectType) -> Self {
        match obj_type {
            // Already compressed images: store without recompression
            ObjectType::Jpeg
            | ObjectType::Png
            | ObjectType::Gif
            | ObjectType::Webp
            | ObjectType::Avif
            | ObjectType::Heic
            | ObjectType::GpuTexture => CompressionStrategy::Store,

            // Uncompressed images: Zstd best compression
            // DPX: 10/12/16-bit uncompressed frames, highly compressible
            ObjectType::Tiff
            | ObjectType::Bmp
            | ObjectType::Raw
            | ObjectType::Exr
            | ObjectType::Hdr
            | ObjectType::Dpx => CompressionStrategy::Zstd(CompressionLevel::Best),

            // Already compressed video: store without recompression
            // MXF wraps compressed codecs (XDCAM, DNxHD, H.264) in professional environments
            ObjectType::Mp4
            | ObjectType::Mov
            | ObjectType::Avi
            | ObjectType::Mkv
            | ObjectType::Webm
            | ObjectType::Flv
            | ObjectType::Wmv
            | ObjectType::Mpg
            | ObjectType::Mxf => CompressionStrategy::Store,

            // Already compressed audio: store without recompression
            ObjectType::Mp3 | ObjectType::Aac | ObjectType::Ogg | ObjectType::Opus => {
                CompressionStrategy::Store
            }

            // Uncompressed PCM audio: Zstd best
            ObjectType::Wav | ObjectType::Aiff => CompressionStrategy::Zstd(CompressionLevel::Best),

            // Lossless-compressed audio (FLAC/ALAC is already entropy-coded;
            // measured gain from Best over Default is ~0.1%): cheap Zstd only.
            // Matches the chunk-level ChunkCodecHint::LosslessAudio routing.
            ObjectType::Flac | ObjectType::Alac => {
                CompressionStrategy::Zstd(CompressionLevel::Default)
            }

            // Documents: Zstd default
            ObjectType::Pdf | ObjectType::Svg | ObjectType::Eps => {
                CompressionStrategy::Zstd(CompressionLevel::Default)
            }

            // Text/Code: Brotli for best ratio on structured text data.
            // Large files (>500 MB) fall back to Zstd via for_object_type_with_size.
            ObjectType::Text
            | ObjectType::Json
            | ObjectType::Xml
            | ObjectType::Yaml
            | ObjectType::Toml
            | ObjectType::Csv => CompressionStrategy::Brotli(CompressionLevel::Default),

            // Already compressed archives: store
            ObjectType::Zip
            | ObjectType::Gz
            | ObjectType::SevenZ
            | ObjectType::Rar
            | ObjectType::Parquet
            | ObjectType::CompressedLog => CompressionStrategy::Store,

            // TAR is uncompressed container
            ObjectType::Tar => CompressionStrategy::Zstd(CompressionLevel::Default),

            // ML data formats: Zstd fast (good for large numeric arrays)
            ObjectType::MlData | ObjectType::MlModel => {
                CompressionStrategy::Zstd(CompressionLevel::Fast)
            }

            // ML training checkpoints: Zstd fast (huge files, created frequently)
            ObjectType::MlCheckpoint => CompressionStrategy::Zstd(CompressionLevel::Fast),

            // ML inference models: Zstd default (better compression for archival)
            ObjectType::MlInference | ObjectType::MlDeployment => {
                CompressionStrategy::Zstd(CompressionLevel::Default)
            }

            // 3D interchange formats: Zstd best (mesh/geometry data compresses well)
            // STL/OBJ/PLY: raw float triangles → 60-70% compression typical
            // GLB/FBX/DAE: binary mesh with metadata → 30-50% compression typical
            ObjectType::Model3D => CompressionStrategy::Zstd(CompressionLevel::Best),

            // PDF-based creative containers: store without recompression
            // AI/InDesign files are PDF containers with embedded compressed streams
            // Zstd compression expands the data on every chunk, wasting CPU
            ObjectType::AdobeIllustrator | ObjectType::AdobeIndesign => CompressionStrategy::Store,

            // Creative project files: Zstd default with heavy delta compression
            // These files have internal structure and benefit from both compression + delta
            ObjectType::AdobePhotoshop
            | ObjectType::AdobeAfterEffects
            | ObjectType::AdobePremiere
            | ObjectType::DavinciResolve
            | ObjectType::FinalCutPro
            | ObjectType::AvidMediaComposer
            | ObjectType::Blender
            | ObjectType::Maya
            | ObjectType::ThreeDsMax
            | ObjectType::Cinema4D
            | ObjectType::Houdini
            | ObjectType::ProTools
            | ObjectType::AbletonLive
            | ObjectType::FLStudio
            | ObjectType::LogicPro
            | ObjectType::AutoCad
            | ObjectType::SketchUp
            | ObjectType::Revit
            | ObjectType::UnityProject
            | ObjectType::UnrealProject
            | ObjectType::GodotProject => CompressionStrategy::Zstd(CompressionLevel::Default),

            // Office documents: store without recompression (ZIP containers with compressed XML)
            ObjectType::WordDocument
            | ObjectType::ExcelSpreadsheet
            | ObjectType::PowerpointPresentation
            | ObjectType::OpenDocument => CompressionStrategy::Store,

            // Database: Zstd default
            ObjectType::SqliteDatabase => CompressionStrategy::Zstd(CompressionLevel::Default),

            // Git objects: Zlib for compatibility
            ObjectType::GitBlob | ObjectType::GitTree | ObjectType::GitCommit => {
                CompressionStrategy::Zlib(CompressionLevel::Default)
            }

            // Unknown/binary: Zstd default (safe choice)
            ObjectType::Unknown => CompressionStrategy::Zstd(CompressionLevel::Default),
        }
    }

    /// Select optimal strategy for object type with size consideration.
    ///
    /// Text types use Brotli by default; at ≥500 MB the encode cost tips in Zstd's favour
    /// (~10× faster at only ~20% worse ratio) so we switch automatically.
    pub fn for_object_type_with_size(obj_type: ObjectType, data_size: usize) -> Self {
        let base = Self::for_object_type(obj_type);
        if data_size >= LARGE_TEXT_THRESHOLD {
            if let CompressionStrategy::Brotli(_) = base {
                return CompressionStrategy::Zstd(CompressionLevel::Default);
            }
        }
        base
    }
}

/// Codec-level compression strategy for individual chunks inside video containers.
///
/// Unlike file-level strategy (which treats the whole container as pre-compressed),
/// this picks optimal compression per demuxed stream chunk based on the actual codec.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ChunkCodecHint {
    /// High-entropy lossy video (H.264, H.265, VP9, AV1) — already compressed
    HighEntropyVideo,
    /// Low-entropy / intra-only video (ProRes, DNxHR, JPEG2000) — moderate compressibility
    IntraOnlyVideo,
    /// Raw / uncompressed video — highly compressible
    RawVideo,
    /// Lossy compressed audio (AAC, Opus, MP3, Vorbis) — already compressed
    CompressedAudio,
    /// Lossless / uncompressed audio (PCM, FLAC, ALAC) — compressible
    LosslessAudio,
    /// Text subtitles — highly compressible
    TextSubtitle,
    /// Bitmap subtitles (PGS, VobSub) — moderately compressible
    BitmapSubtitle,
    /// Container metadata — compressible
    Metadata,
    /// Unknown codec — use file-level strategy
    Unknown,
}

impl CompressionStrategy {
    /// Select compression strategy for a demuxed chunk based on its codec.
    ///
    /// Returns `None` if the codec hint is `Unknown`, meaning the caller should
    /// fall back to file-level strategy.
    pub fn for_codec_hint(hint: ChunkCodecHint) -> Option<Self> {
        match hint {
            // Already compressed — store as-is
            ChunkCodecHint::HighEntropyVideo | ChunkCodecHint::CompressedAudio => {
                Some(CompressionStrategy::Store)
            }
            // Intra-only video (ProRes/DNxHR) — light Zstd for ~10-20% savings
            ChunkCodecHint::IntraOnlyVideo => {
                Some(CompressionStrategy::Zstd(CompressionLevel::Fast))
            }
            // Raw video — excellent Zstd compression
            ChunkCodecHint::RawVideo => Some(CompressionStrategy::Zstd(CompressionLevel::Default)),
            // PCM/FLAC/ALAC — Zstd default for ~40-65% savings
            ChunkCodecHint::LosslessAudio => {
                Some(CompressionStrategy::Zstd(CompressionLevel::Default))
            }
            // Text subtitles — Brotli best (tiny chunks, pure text; Brotli wins on ratio)
            ChunkCodecHint::TextSubtitle => {
                Some(CompressionStrategy::Brotli(CompressionLevel::Best))
            }
            // Bitmap subtitles — Zstd default
            ChunkCodecHint::BitmapSubtitle => {
                Some(CompressionStrategy::Zstd(CompressionLevel::Default))
            }
            // Container metadata — Zstd default
            ChunkCodecHint::Metadata => Some(CompressionStrategy::Zstd(CompressionLevel::Default)),
            // Unknown — let caller use file-level strategy
            ChunkCodecHint::Unknown => None,
        }
    }
}
