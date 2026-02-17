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

//! Smart compression with per-object-type strategy selection
//!
//! Automatically selects optimal compression based on file type and content.

use crate::error::CompressionResult;
use crate::{BrotliCompressor, CompressionLevel, Compressor, ZlibCompressor, ZstdCompressor};
use std::fmt;
use std::path::Path;

/// Object/File type classification for compression strategy selection
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ObjectType {
    // Already compressed image formats (lossy)
    Jpeg,
    Png,
    Gif,
    Webp,
    Avif,
    Heic,

    // GPU-compressed texture formats (game dev)
    GpuTexture,

    // Uncompressed/lossless image formats
    Tiff,
    Bmp,
    Raw,
    Exr,
    Hdr,

    // Video formats (typically already compressed)
    Mp4,
    Mov,
    Avi,
    Mkv,
    Webm,
    Flv,
    Wmv,
    Mpg,

    // Audio formats (compressed)
    Mp3,
    Aac,
    Ogg,
    Opus,

    // Audio formats (uncompressed/lossless)
    Flac,
    Wav,
    Aiff,
    Alac,

    // Document formats
    Pdf,
    Svg,
    Eps,

    // Text/Code
    Text,
    Json,
    Xml,
    Yaml,
    Toml,
    Csv,

    // Archives (already compressed)
    Zip,
    Tar,
    Gz,
    SevenZ,
    Rar,

    // ML/Data formats (already internally compressed)
    Parquet,

    // ML data formats (arrays, tensors)
    MlData,

    // ML model weights (PyTorch, TensorFlow, etc.)
    MlModel,

    // ML deployment formats (ONNX, TFLite, etc.)
    MlDeployment,

    // ML training checkpoints (large, frequent)
    MlCheckpoint,

    // ML inference models (optimized, archived)
    MlInference,

    // Creative project files - Adobe Creative Cloud
    AdobePhotoshop,    // .psd, .psb
    AdobeIllustrator,  // .ai
    AdobeIndesign,     // .indd, .idml
    AdobeAfterEffects, // .aep
    AdobePremiere,     // .prproj

    // Creative project files - Video editing
    DavinciResolve,    // .drp
    FinalCutPro,       // .fcpbundle, .fcpxml
    AvidMediaComposer, // .avb

    // Creative project files - 3D/DCC
    Blender,           // .blend
    Maya,              // .ma, .mb
    ThreeDsMax,        // .max
    Cinema4D,          // .c4d
    Houdini,           // .hip, .hipnc

    // Creative project files - Audio DAWs
    ProTools,          // .ptx
    AbletonLive,       // .als
    FLStudio,          // .flp
    LogicPro,          // .logic, .logicx

    // Creative project files - CAD
    AutoCad,           // .dwg, .dxf
    SketchUp,          // .skp
    Revit,             // .rvt

    // Creative project files - Game engines
    UnityProject,      // .unity, .prefab, .asset
    UnrealProject,     // .uasset, .umap
    GodotProject,      // .tscn, .tres

    // Office documents (modern XML-based)
    WordDocument,      // .docx, .doc
    ExcelSpreadsheet,  // .xlsx, .xls
    PowerpointPresentation, // .pptx, .ppt
    OpenDocument,      // .odt, .ods, .odp

    // Database formats
    SqliteDatabase,    // .sqlite, .db, .db3

    // Compressed text/logs
    CompressedLog,     // .log.gz, .log.bz2

    // Git object types (for interoperability)
    GitBlob,
    GitTree,
    GitCommit,

    // Unknown/binary
    Unknown,
}

impl ObjectType {
    /// Detect object type from file extension
    pub fn from_extension(ext: &str) -> Self {
        match ext.to_lowercase().as_str() {
            // Already compressed images
            "jpg" | "jpeg" => ObjectType::Jpeg,
            "png" => ObjectType::Png,
            "gif" => ObjectType::Gif,
            "webp" => ObjectType::Webp,
            "avif" => ObjectType::Avif,
            "heic" | "heif" => ObjectType::Heic,

            // GPU-compressed textures (game dev)
            "dds" | "ktx" | "ktx2" | "astc" | "pvr" | "basis" => ObjectType::GpuTexture,

            // Uncompressed images
            "tif" | "tiff" => ObjectType::Tiff,
            "bmp" | "dib" => ObjectType::Bmp,
            "psd" | "psb" => ObjectType::AdobePhotoshop, // Moved to creative projects
            "raw" | "cr2" | "cr3" | "nef" | "arw" | "dng" | "orf" | "rw2" => ObjectType::Raw,
            "exr" => ObjectType::Exr,
            "hdr" | "pic" => ObjectType::Hdr,

            // Video
            "mp4" | "m4v" => ObjectType::Mp4,
            "mov" | "qt" => ObjectType::Mov,
            "avi" => ObjectType::Avi,
            "mkv" => ObjectType::Mkv,
            "webm" => ObjectType::Webm,
            "flv" | "f4v" => ObjectType::Flv,
            "wmv" | "asf" => ObjectType::Wmv,
            "mpg" | "mpeg" | "m2v" => ObjectType::Mpg,

            // Audio (compressed)
            "mp3" => ObjectType::Mp3,
            "aac" => ObjectType::Aac,
            "m4a" => ObjectType::Aac, // Default m4a to AAC
            "ogg" | "oga" => ObjectType::Ogg,
            "opus" => ObjectType::Opus,

            // Audio (uncompressed/lossless)
            "flac" => ObjectType::Flac,
            "wav" => ObjectType::Wav,
            "aiff" | "aif" | "aifc" => ObjectType::Aiff,
            "alac" => ObjectType::Alac,

            // Documents
            "pdf" => ObjectType::Pdf,
            "svg" | "svgz" => ObjectType::Svg,
            "eps" => ObjectType::Eps, // "ai" moved to AdobeIllustrator

            // Text/Code
            "txt" | "md" | "markdown" | "rst" | "adoc" |
            "rs" | "js" | "ts" | "jsx" | "tsx" |
            "py" | "go" | "c" | "cpp" | "cc" | "cxx" |
            "h" | "hpp" | "hh" | "hxx" |
            "java" | "kt" | "swift" | "rb" | "php" |
            "sh" | "bash" | "zsh" | "fish" |
            "vim" | "lua" | "pl" | "r" | "m" => ObjectType::Text,
            "json" | "json5" | "jsonc" => ObjectType::Json,
            "xml" | "html" | "xhtml" | "htm" | "xsl" | "xslt" => ObjectType::Xml,
            "yml" | "yaml" => ObjectType::Yaml,
            "toml" => ObjectType::Toml,
            "csv" | "tsv" | "psv" => ObjectType::Csv,

            // Archives
            "zip" | "zipx" => ObjectType::Zip,
            "tar" => ObjectType::Tar,
            "gz" | "gzip" => ObjectType::Gz,
            "7z" => ObjectType::SevenZ,
            "rar" => ObjectType::Rar,

            // ML/Data formats (internally compressed)
            "parquet" | "arrow" | "feather" | "orc" | "avro" => ObjectType::Parquet,

            // ML data formats (arrays, tensors)
            "hdf5" | "h5" | "nc" | "netcdf" | "npy" | "npz" |
            "tfrecords" | "petastorm" => ObjectType::MlData,

            // ML model weights (general)
            "pb" | "safetensors" | "pkl" | "joblib" => ObjectType::MlModel,

            // ML training checkpoints (large, frequent saves during training)
            // Note: .pt/.pth/.bin can be either checkpoints or inference models
            // We default to checkpoint for aggressive compression since they're more common
            "ckpt" | "pt" | "pth" | "bin" => ObjectType::MlCheckpoint,

            // ML inference models (optimized for deployment)
            "onnx" | "gguf" | "ggml" | "tflite" | "mlmodel" | "coreml" |
            "keras" | "pte" | "mleap" | "pmml" | "llamafile" => ObjectType::MlInference,

            // Creative projects - Adobe Creative Cloud
            "ai" | "ait" => ObjectType::AdobeIllustrator,
            "indd" | "idml" | "indt" => ObjectType::AdobeIndesign,
            "aep" | "aet" => ObjectType::AdobeAfterEffects,
            "prproj" | "psq" => ObjectType::AdobePremiere,

            // Creative projects - Video editing
            "drp" | "drp_proxies" => ObjectType::DavinciResolve,
            "fcpbundle" | "fcpxml" | "fcpxmld" => ObjectType::FinalCutPro,
            "avb" | "avp" | "avs" => ObjectType::AvidMediaComposer,

            // Creative projects - 3D/DCC
            "blend" | "blend1" => ObjectType::Blender,
            "ma" | "mb" => ObjectType::Maya,
            "max" => ObjectType::ThreeDsMax,
            "c4d" => ObjectType::Cinema4D,
            "hip" | "hipnc" | "hiplc" => ObjectType::Houdini,

            // Creative projects - Audio DAWs
            "ptx" | "ptf" => ObjectType::ProTools,
            "als" => ObjectType::AbletonLive,
            "flp" => ObjectType::FLStudio,
            "logic" | "logicx" => ObjectType::LogicPro,

            // Creative projects - CAD
            "dwg" | "dxf" => ObjectType::AutoCad,
            "skp" => ObjectType::SketchUp,
            "rvt" | "rfa" | "rte" => ObjectType::Revit,

            // Creative projects - Game engines
            "unity" | "prefab" | "asset" | "unity3d" => ObjectType::UnityProject,
            "uasset" | "umap" | "upk" => ObjectType::UnrealProject,
            "tscn" | "tres" | "godot" => ObjectType::GodotProject,

            // Office documents
            "docx" | "doc" | "docm" | "dot" | "dotx" => ObjectType::WordDocument,
            "xlsx" | "xls" | "xlsm" | "xlsb" | "xlt" | "xltx" => ObjectType::ExcelSpreadsheet,
            "pptx" | "ppt" | "pptm" | "pot" | "potx" => ObjectType::PowerpointPresentation,
            "odt" | "ods" | "odp" | "odg" | "odf" => ObjectType::OpenDocument,

            // Database formats
            "sqlite" | "sqlite3" | "db" | "db3" | "s3db" => ObjectType::SqliteDatabase,

            // Special handling for compound extensions (must check before generic extensions)
            // This is handled in from_path() with better logic

            _ => ObjectType::Unknown,
        }
    }

    /// Detect object type from file path
    pub fn from_path<P: AsRef<Path>>(path: P) -> Self {
        path.as_ref()
            .extension()
            .and_then(|ext| ext.to_str())
            .map(Self::from_extension)
            .unwrap_or(ObjectType::Unknown)
    }

    /// Detect object type from magic bytes
    pub fn from_magic_bytes(data: &[u8]) -> Self {
        if data.len() < 4 {
            return ObjectType::Unknown;
        }

        // JPEG: FF D8 FF
        if data.len() >= 3 && data[0] == 0xFF && data[1] == 0xD8 && data[2] == 0xFF {
            return ObjectType::Jpeg;
        }

        // PNG: 89 50 4E 47
        if data.starts_with(&[0x89, 0x50, 0x4E, 0x47]) {
            return ObjectType::Png;
        }

        // GIF: 47 49 46 38
        if data.starts_with(b"GIF8") {
            return ObjectType::Gif;
        }

        // WEBP: RIFF....WEBP
        if data.len() >= 12 && data.starts_with(b"RIFF") && &data[8..12] == b"WEBP" {
            return ObjectType::Webp;
        }

        // TIFF: 49 49 2A 00 (little-endian) or 4D 4D 00 2A (big-endian)
        if data.starts_with(&[0x49, 0x49, 0x2A, 0x00]) || data.starts_with(&[0x4D, 0x4D, 0x00, 0x2A]) {
            return ObjectType::Tiff;
        }

        // BMP: 42 4D
        if data.starts_with(&[0x42, 0x4D]) {
            return ObjectType::Bmp;
        }

        // PDF: 25 50 44 46
        if data.starts_with(b"%PDF") {
            return ObjectType::Pdf;
        }

        // MP4: ftyp at offset 4
        if data.len() >= 12 && &data[4..8] == b"ftyp" {
            return ObjectType::Mp4;
        }

        // ZIP: 50 4B 03 04 or 50 4B 05 06
        if data.starts_with(&[0x50, 0x4B, 0x03, 0x04]) || data.starts_with(&[0x50, 0x4B, 0x05, 0x06]) {
            return ObjectType::Zip;
        }

        // GZIP: 1F 8B
        if data.starts_with(&[0x1F, 0x8B]) {
            return ObjectType::Gz;
        }

        ObjectType::Unknown
    }

    /// Check if this type is already compressed
    pub fn is_already_compressed(self) -> bool {
        matches!(
            self,
            ObjectType::Jpeg
                | ObjectType::Png
                | ObjectType::Gif
                | ObjectType::Webp
                | ObjectType::Avif
                | ObjectType::Heic
                | ObjectType::GpuTexture
                | ObjectType::Mp4
                | ObjectType::Mov
                | ObjectType::Avi
                | ObjectType::Mkv
                | ObjectType::Webm
                | ObjectType::Flv
                | ObjectType::Wmv
                | ObjectType::Mpg
                | ObjectType::Mp3
                | ObjectType::Aac
                | ObjectType::Ogg
                | ObjectType::Opus
                | ObjectType::Pdf
                | ObjectType::Zip
                | ObjectType::Gz
                | ObjectType::SevenZ
                | ObjectType::Rar
                | ObjectType::Parquet
                // PDF-based creative containers with embedded compressed streams
                | ObjectType::AdobeIllustrator
                | ObjectType::AdobeIndesign
                // Office documents are ZIP containers with compressed XML
                | ObjectType::WordDocument
                | ObjectType::ExcelSpreadsheet
                | ObjectType::PowerpointPresentation
                | ObjectType::OpenDocument
        )
    }

    /// Get the category of this object type
    pub fn category(self) -> ObjectCategory {
        match self {
            ObjectType::Jpeg | ObjectType::Png | ObjectType::Gif | ObjectType::Webp |
            ObjectType::Avif | ObjectType::Heic | ObjectType::GpuTexture | ObjectType::Tiff | ObjectType::Bmp |
            ObjectType::Raw | ObjectType::Exr | ObjectType::Hdr => ObjectCategory::Image,

            ObjectType::Mp4 | ObjectType::Mov | ObjectType::Avi | ObjectType::Mkv |
            ObjectType::Webm | ObjectType::Flv | ObjectType::Wmv | ObjectType::Mpg => ObjectCategory::Video,

            ObjectType::Mp3 | ObjectType::Aac | ObjectType::Ogg | ObjectType::Opus |
            ObjectType::Flac | ObjectType::Wav | ObjectType::Aiff | ObjectType::Alac => ObjectCategory::Audio,

            ObjectType::Pdf | ObjectType::Svg | ObjectType::Eps => ObjectCategory::Document,

            ObjectType::Text | ObjectType::Json | ObjectType::Xml | ObjectType::Yaml |
            ObjectType::Toml | ObjectType::Csv => ObjectCategory::Text,

            ObjectType::Zip | ObjectType::Tar | ObjectType::Gz | ObjectType::SevenZ |
            ObjectType::Rar | ObjectType::CompressedLog => ObjectCategory::Archive,

            ObjectType::Parquet | ObjectType::MlData | ObjectType::MlModel |
            ObjectType::MlDeployment => ObjectCategory::Archive, // ML formats as data archives

            // ML specialized (training vs inference)
            ObjectType::MlCheckpoint | ObjectType::MlInference => ObjectCategory::MlSpecialized,

            // Creative project files
            ObjectType::AdobePhotoshop | ObjectType::AdobeIllustrator | ObjectType::AdobeIndesign |
            ObjectType::AdobeAfterEffects | ObjectType::AdobePremiere |
            ObjectType::DavinciResolve | ObjectType::FinalCutPro | ObjectType::AvidMediaComposer |
            ObjectType::Blender | ObjectType::Maya | ObjectType::ThreeDsMax | ObjectType::Cinema4D |
            ObjectType::Houdini |
            ObjectType::ProTools | ObjectType::AbletonLive | ObjectType::FLStudio | ObjectType::LogicPro |
            ObjectType::AutoCad | ObjectType::SketchUp | ObjectType::Revit |
            ObjectType::UnityProject | ObjectType::UnrealProject | ObjectType::GodotProject => ObjectCategory::CreativeProject,

            // Office documents
            ObjectType::WordDocument | ObjectType::ExcelSpreadsheet |
            ObjectType::PowerpointPresentation | ObjectType::OpenDocument => ObjectCategory::Office,

            // Database
            ObjectType::SqliteDatabase => ObjectCategory::Database,

            ObjectType::GitBlob | ObjectType::GitTree | ObjectType::GitCommit => ObjectCategory::GitObject,

            ObjectType::Unknown => ObjectCategory::Unknown,
        }
    }
}

/// Object category for high-level classification
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ObjectCategory {
    Image,
    Video,
    Audio,
    Document,
    Text,
    Archive,
    CreativeProject,  // Adobe, video NLEs, DAWs, 3D/DCC, CAD, game engines
    Office,           // Word, Excel, PowerPoint, OpenDocument
    MlSpecialized,    // ML training checkpoints vs inference models
    Database,         // SQLite, database files
    GitObject,
    Unknown,
}

/// Size threshold for switching from Brotli to Zstd for text files
/// At 500MB+, Brotli level 9 becomes too slow; Zstd provides 10x faster compression
/// with only ~20% compression ratio loss
const LARGE_TEXT_THRESHOLD: usize = 500 * 1024 * 1024; // 500 MB

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
            ObjectType::Tiff
            | ObjectType::Bmp
            | ObjectType::Raw
            | ObjectType::Exr
            | ObjectType::Hdr => CompressionStrategy::Zstd(CompressionLevel::Best),

            // Already compressed video: store without recompression
            ObjectType::Mp4
            | ObjectType::Mov
            | ObjectType::Avi
            | ObjectType::Mkv
            | ObjectType::Webm
            | ObjectType::Flv
            | ObjectType::Wmv
            | ObjectType::Mpg => CompressionStrategy::Store,

            // Already compressed audio: store without recompression
            ObjectType::Mp3
            | ObjectType::Aac
            | ObjectType::Ogg
            | ObjectType::Opus => CompressionStrategy::Store,

            // Uncompressed/lossless audio: Zstd best
            ObjectType::Flac
            | ObjectType::Wav
            | ObjectType::Aiff
            | ObjectType::Alac => CompressionStrategy::Zstd(CompressionLevel::Best),

            // Documents: Zstd default
            ObjectType::Pdf | ObjectType::Svg | ObjectType::Eps => {
                CompressionStrategy::Zstd(CompressionLevel::Default)
            }

            // Text/Code: Brotli for maximum compression on structured data
            // CHANGED: Switched from Zstd to Brotli for 15-30% better compression ratios
            // Brotli excels at text/structured data with dictionary-based compression
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
            ObjectType::MlData | ObjectType::MlModel => CompressionStrategy::Zstd(CompressionLevel::Fast),

            // ML training checkpoints: Zstd fast (huge files, created frequently)
            ObjectType::MlCheckpoint => CompressionStrategy::Zstd(CompressionLevel::Fast),

            // ML inference models: Zstd default (better compression for archival)
            ObjectType::MlInference | ObjectType::MlDeployment => {
                CompressionStrategy::Zstd(CompressionLevel::Default)
            }

            // PDF-based creative containers: store without recompression
            // AI/InDesign files are PDF containers with embedded compressed streams
            // Zstd compression expands the data on every chunk, wasting CPU
            ObjectType::AdobeIllustrator
            | ObjectType::AdobeIndesign => CompressionStrategy::Store,

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
            ObjectType::GitBlob
            | ObjectType::GitTree
            | ObjectType::GitCommit => CompressionStrategy::Zlib(CompressionLevel::Default),

            // Unknown/binary: Zstd default (safe choice)
            ObjectType::Unknown => CompressionStrategy::Zstd(CompressionLevel::Default),
        }
    }

    /// Select optimal strategy for object type with size consideration
    /// 
    /// For large text files (>500MB), switches from Brotli to Zstd for 10x faster compression
    /// with only ~20% compression ratio loss.
    pub fn for_object_type_with_size(obj_type: ObjectType, data_size: usize) -> Self {
        // Check if this is a text type that would normally use Brotli
        let base_strategy = Self::for_object_type(obj_type);
        
        // For large text files, switch from Brotli to Zstd for faster compression
        if data_size >= LARGE_TEXT_THRESHOLD {
            if let CompressionStrategy::Brotli(_) = base_strategy {
                // Use Zstd Default for large text files (10x faster, ~20% worse ratio)
                return CompressionStrategy::Zstd(CompressionLevel::Default);
            }
        }
        
        base_strategy
    }
}

/// Type-aware compressor trait
pub trait TypeAwareCompressor: Send + Sync {
    /// Compress with automatic strategy selection
    fn compress_typed(&self, data: &[u8], obj_type: ObjectType) -> CompressionResult<Vec<u8>>;

    /// Compress with automatic strategy selection considering data size
    /// For large text files (>500MB), uses Zstd instead of Brotli for faster compression
    fn compress_typed_with_size(&self, data: &[u8], obj_type: ObjectType) -> CompressionResult<Vec<u8>>;

    /// Decompress data (auto-detects algorithm)
    fn decompress_typed(&self, data: &[u8]) -> CompressionResult<Vec<u8>>;

    /// Get compression strategy for object type
    fn strategy_for_type(&self, obj_type: ObjectType) -> CompressionStrategy;

    /// Get compression strategy for object type with size consideration
    fn strategy_for_type_with_size(&self, obj_type: ObjectType, data_size: usize) -> CompressionStrategy;
}

/// Smart compressor with automatic type-based strategy selection
#[derive(Clone)]
pub struct SmartCompressor {
    zlib: ZlibCompressor,
    zstd_fast: ZstdCompressor,
    zstd_default: ZstdCompressor,
    zstd_best: ZstdCompressor,
    brotli_best: BrotliCompressor,
}

impl SmartCompressor {
    /// Create new smart compressor with all algorithms ready
    pub fn new() -> Self {
        Self {
            zlib: ZlibCompressor::new(CompressionLevel::Default),
            zstd_fast: ZstdCompressor::new(CompressionLevel::Fast),
            zstd_default: ZstdCompressor::new(CompressionLevel::Default),
            zstd_best: ZstdCompressor::new(CompressionLevel::Best),
            brotli_best: BrotliCompressor::new(CompressionLevel::Best),
        }
    }

    /// Compress with explicit strategy
    /// 
    /// If compression would EXPAND the data (common for already-compressed content
    /// like embedded JPEGs in AI/PSD files), automatically falls back to Store mode.
    fn compress_with_strategy(
        &self,
        data: &[u8],
        strategy: CompressionStrategy,
    ) -> CompressionResult<Vec<u8>> {
        // Store mode: prefix with 0x00 magic byte
        if matches!(strategy, CompressionStrategy::Store) {
            let mut result = Vec::with_capacity(data.len() + 1);
            result.push(0x00); // Store magic byte
            result.extend_from_slice(data);
            return Ok(result);
        }
        
        let compressed = match strategy {
            CompressionStrategy::Store => unreachable!(), // Handled above

            CompressionStrategy::Zlib(level) => {
                let compressor = ZlibCompressor::new(level);
                compressor.compress(data)?
            }

            CompressionStrategy::Zstd(level) => {
                let compressor = match level {
                    CompressionLevel::Fast => &self.zstd_fast,
                    CompressionLevel::Default => &self.zstd_default,
                    CompressionLevel::Best => &self.zstd_best,
                };
                compressor.compress(data)?
            }

            CompressionStrategy::Brotli(level) => {
                let compressor = BrotliCompressor::new(level);
                compressor.compress(data)?
            }

            CompressionStrategy::Delta => {
                // Delta compression requires a base - not implemented in simple compress
                // Fall back to Zstd
                self.zstd_default.compress(data)?
            }
        };
        
        // CRITICAL FIX: If compression expanded the data (happens with already-compressed
        // content like embedded JPEGs in AI/PSD files), fall back to Store mode.
        // This prevents significant size overhead on creative files.
        if compressed.len() >= data.len() {
            tracing::debug!(
                original_size = data.len(),
                compressed_size = compressed.len(),
                "Compression expanded data, falling back to Store mode"
            );
            let mut result = Vec::with_capacity(data.len() + 1);
            result.push(0x00); // Store magic byte
            result.extend_from_slice(data);
            return Ok(result);
        }
        
        Ok(compressed)
    }
}

impl Default for SmartCompressor {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Debug for SmartCompressor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SmartCompressor")
            .field("strategies", &"Zlib|Zstd|Brotli|Delta")
            .finish()
    }
}

impl TypeAwareCompressor for SmartCompressor {
    fn compress_typed(&self, data: &[u8], obj_type: ObjectType) -> CompressionResult<Vec<u8>> {
        let strategy = self.strategy_for_type(obj_type);
        self.compress_with_strategy(data, strategy)
    }

    fn compress_typed_with_size(&self, data: &[u8], obj_type: ObjectType) -> CompressionResult<Vec<u8>> {
        let strategy = self.strategy_for_type_with_size(obj_type, data.len());
        self.compress_with_strategy(data, strategy)
    }

    fn decompress_typed(&self, data: &[u8]) -> CompressionResult<Vec<u8>> {
        // Auto-detect compression algorithm
        use crate::CompressionAlgorithm;

        // Check for Store mode magic byte (0x00 prefix added by compress_with_strategy fallback)
        // This handles data that couldn't be compressed efficiently (already-compressed content).
        if !data.is_empty() && data[0] == 0x00 {
            // Check if this looks like Store mode (no compression magic after the prefix)
            let remaining = &data[1..];
            let algo = CompressionAlgorithm::detect(remaining);
            if algo == CompressionAlgorithm::None {
                // Strip the Store prefix and return raw data
                return Ok(remaining.to_vec());
            }
        }

        let algo = CompressionAlgorithm::detect(data);

        match algo {
            CompressionAlgorithm::None => Ok(data.to_vec()),
            CompressionAlgorithm::Zlib => {
                // False positive possible: raw data starting with 0x78 + valid checksum byte
                // can be misdetected as zlib. Fall back to raw data if decompression fails.
                Ok(self.zlib.decompress(data).unwrap_or_else(|_| data.to_vec()))
            }
            CompressionAlgorithm::Zstd => {
                // False positive rare but possible: raw data starting with zstd magic bytes
                // (0x28 0xB5 0x2F 0xFD). Fall back to raw data if decompression fails.
                Ok(self.zstd_default.decompress(data).unwrap_or_else(|_| data.to_vec()))
            }
            CompressionAlgorithm::Brotli => {
                // False positive rare but possible: raw data starting with "BRT\x01".
                // Fall back to raw data if decompression fails.
                Ok(self.brotli_best.decompress(data).unwrap_or_else(|_| data.to_vec()))
            }
        }

    }


    fn strategy_for_type(&self, obj_type: ObjectType) -> CompressionStrategy {
        CompressionStrategy::for_object_type(obj_type)
    }

    fn strategy_for_type_with_size(&self, obj_type: ObjectType, data_size: usize) -> CompressionStrategy {
        CompressionStrategy::for_object_type_with_size(obj_type, data_size)
    }
}

impl Compressor for SmartCompressor {
    fn compress(&self, data: &[u8]) -> CompressionResult<Vec<u8>> {
        // Default to Zstd when no type information available
        self.zstd_default.compress(data)
    }

    fn decompress(&self, data: &[u8]) -> CompressionResult<Vec<u8>> {
        self.decompress_typed(data)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_object_type_from_extension() {
        assert_eq!(ObjectType::from_extension("jpg"), ObjectType::Jpeg);
        assert_eq!(ObjectType::from_extension("JPEG"), ObjectType::Jpeg);
        assert_eq!(ObjectType::from_extension("png"), ObjectType::Png);
        assert_eq!(ObjectType::from_extension("tiff"), ObjectType::Tiff);
        assert_eq!(ObjectType::from_extension("mp4"), ObjectType::Mp4);
        assert_eq!(ObjectType::from_extension("pdf"), ObjectType::Pdf);
        assert_eq!(ObjectType::from_extension("txt"), ObjectType::Text);
        assert_eq!(ObjectType::from_extension("rs"), ObjectType::Text);
        assert_eq!(ObjectType::from_extension("json"), ObjectType::Json);
        assert_eq!(ObjectType::from_extension("unknown"), ObjectType::Unknown);
    }

    #[test]
    fn test_object_type_from_path() {
        assert_eq!(ObjectType::from_path("image.jpg"), ObjectType::Jpeg);
        assert_eq!(ObjectType::from_path("/path/to/file.png"), ObjectType::Png);
        assert_eq!(ObjectType::from_path("document.PDF"), ObjectType::Pdf);
        assert_eq!(ObjectType::from_path("code.rs"), ObjectType::Text);
        assert_eq!(ObjectType::from_path("noextension"), ObjectType::Unknown);
    }

    #[test]
    fn test_object_type_from_magic_bytes() {
        // JPEG
        let jpeg_data = [0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x10];
        assert_eq!(ObjectType::from_magic_bytes(&jpeg_data), ObjectType::Jpeg);

        // PNG
        let png_data = [0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];
        assert_eq!(ObjectType::from_magic_bytes(&png_data), ObjectType::Png);

        // GIF
        let gif_data = b"GIF89a";
        assert_eq!(ObjectType::from_magic_bytes(gif_data), ObjectType::Gif);

        // PDF
        let pdf_data = b"%PDF-1.4";
        assert_eq!(ObjectType::from_magic_bytes(pdf_data), ObjectType::Pdf);

        // Unknown
        let unknown_data = b"random";
        assert_eq!(ObjectType::from_magic_bytes(unknown_data), ObjectType::Unknown);
    }

    #[test]
    fn test_is_already_compressed() {
        assert!(ObjectType::Jpeg.is_already_compressed());
        assert!(ObjectType::Png.is_already_compressed());
        assert!(ObjectType::Mp4.is_already_compressed());
        assert!(ObjectType::Zip.is_already_compressed());
        // PDF-based creative containers
        assert!(ObjectType::AdobeIllustrator.is_already_compressed());
        assert!(ObjectType::AdobeIndesign.is_already_compressed());
        // Office ZIP containers
        assert!(ObjectType::WordDocument.is_already_compressed());
        assert!(ObjectType::ExcelSpreadsheet.is_already_compressed());

        assert!(!ObjectType::Tiff.is_already_compressed());
        assert!(!ObjectType::Bmp.is_already_compressed());
        assert!(!ObjectType::Text.is_already_compressed());
        assert!(!ObjectType::Raw.is_already_compressed());
        // PSD is NOT already compressed (uncompressed layer data)
        assert!(!ObjectType::AdobePhotoshop.is_already_compressed());
    }

    #[test]
    fn test_compression_strategy_selection() {
        // Already compressed → Store
        assert_eq!(
            CompressionStrategy::for_object_type(ObjectType::Jpeg),
            CompressionStrategy::Store
        );
        assert_eq!(
            CompressionStrategy::for_object_type(ObjectType::Mp4),
            CompressionStrategy::Store
        );

        // Uncompressed images → Zstd Best
        assert_eq!(
            CompressionStrategy::for_object_type(ObjectType::Tiff),
            CompressionStrategy::Zstd(CompressionLevel::Best)
        );
        assert_eq!(
            CompressionStrategy::for_object_type(ObjectType::Raw),
            CompressionStrategy::Zstd(CompressionLevel::Best)
        );

        // Text → Brotli Default (15-30% better compression for structured data)
        assert_eq!(
            CompressionStrategy::for_object_type(ObjectType::Text),
            CompressionStrategy::Brotli(CompressionLevel::Default)
        );
        assert_eq!(
            CompressionStrategy::for_object_type(ObjectType::Json),
            CompressionStrategy::Brotli(CompressionLevel::Default)
        );

        // Documents → Zstd Default
        assert_eq!(
            CompressionStrategy::for_object_type(ObjectType::Pdf),
            CompressionStrategy::Zstd(CompressionLevel::Default)
        );

        // PDF-based creative containers → Store (already compressed internally)
        assert_eq!(
            CompressionStrategy::for_object_type(ObjectType::AdobeIllustrator),
            CompressionStrategy::Store
        );
        assert_eq!(
            CompressionStrategy::for_object_type(ObjectType::AdobeIndesign),
            CompressionStrategy::Store
        );

        // Office ZIP containers → Store (already compressed internally)
        assert_eq!(
            CompressionStrategy::for_object_type(ObjectType::WordDocument),
            CompressionStrategy::Store
        );

        // PSD still gets Zstd (uncompressed layer data benefits from compression)
        assert_eq!(
            CompressionStrategy::for_object_type(ObjectType::AdobePhotoshop),
            CompressionStrategy::Zstd(CompressionLevel::Default)
        );
    }

    #[test]
    fn test_smart_compressor_jpeg_no_compression() {
        let compressor = SmartCompressor::new();
        let jpeg_data = vec![0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x10, 0x4A, 0x46];

        let compressed = compressor
            .compress_typed(&jpeg_data, ObjectType::Jpeg)
            .unwrap();

        // Should store with 0x00 prefix (Store mode magic byte)
        assert_eq!(compressed.len(), jpeg_data.len() + 1);
        assert_eq!(compressed[0], 0x00);
        assert_eq!(&compressed[1..], &jpeg_data[..]);
    }

    #[test]
    fn test_smart_compressor_text_compression() {
        let compressor = SmartCompressor::new();
        let text_data = b"Hello, World! ".repeat(100);

        let compressed = compressor
            .compress_typed(&text_data, ObjectType::Text)
            .unwrap();

        // Text should compress well
        assert!(compressed.len() < text_data.len());

        let decompressed = compressor.decompress_typed(&compressed).unwrap();
        assert_eq!(decompressed, text_data);
    }

    #[test]
    fn test_smart_compressor_unknown_type() {
        let compressor = SmartCompressor::new();
        let data = b"Some binary data...".repeat(50);

        let compressed = compressor
            .compress_typed(&data, ObjectType::Unknown)
            .unwrap();

        // Unknown should use Zstd default
        assert!(compressed.len() < data.len());

        let decompressed = compressor.decompress_typed(&compressed).unwrap();
        assert_eq!(decompressed, data);
    }

    #[test]
    fn test_smart_compressor_fallback() {
        let compressor = SmartCompressor::new();
        let data = b"Test data";

        // compress() without type should use default (Zstd)
        let compressed = compressor.compress(data).unwrap();
        let decompressed = compressor.decompress(&compressed).unwrap();

        assert_eq!(decompressed, data);
    }

    #[test]
    fn test_compression_strategy_for_all_types() {
        // Verify strategy exists for all types
        let all_types = [
            // Images
            ObjectType::Jpeg, ObjectType::Png, ObjectType::Gif, ObjectType::Webp,
            ObjectType::Avif, ObjectType::Heic,
            ObjectType::Tiff, ObjectType::Bmp, ObjectType::Raw,
            ObjectType::Exr, ObjectType::Hdr,
            // Video
            ObjectType::Mp4, ObjectType::Mov, ObjectType::Avi, ObjectType::Mkv,
            ObjectType::Webm, ObjectType::Flv, ObjectType::Wmv, ObjectType::Mpg,
            // Audio
            ObjectType::Mp3, ObjectType::Aac, ObjectType::Ogg, ObjectType::Opus,
            ObjectType::Flac, ObjectType::Wav, ObjectType::Aiff, ObjectType::Alac,
            // Documents
            ObjectType::Pdf, ObjectType::Svg, ObjectType::Eps,
            // Text
            ObjectType::Text, ObjectType::Json, ObjectType::Xml, ObjectType::Yaml,
            ObjectType::Toml, ObjectType::Csv,
            // Archives
            ObjectType::Zip, ObjectType::Tar, ObjectType::Gz, ObjectType::SevenZ, ObjectType::Rar,
            // Creative projects (sample)
            ObjectType::AdobePhotoshop, ObjectType::Blender,
            // Office (sample)
            ObjectType::WordDocument,
            // ML specialized (sample)
            ObjectType::MlCheckpoint, ObjectType::MlInference,
            // Git
            ObjectType::GitBlob, ObjectType::GitTree, ObjectType::GitCommit,
            // Unknown
            ObjectType::Unknown,
        ];

        for obj_type in &all_types {
            let strategy = CompressionStrategy::for_object_type(*obj_type);
            // Just verify it doesn't panic
            assert!(matches!(
                strategy,
                CompressionStrategy::Store
                    | CompressionStrategy::Zlib(_)
                    | CompressionStrategy::Zstd(_)
                    | CompressionStrategy::Brotli(_)
                    | CompressionStrategy::Delta
            ));
        }
    }

    #[test]
    fn test_object_category() {
        assert_eq!(ObjectType::Jpeg.category(), ObjectCategory::Image);
        assert_eq!(ObjectType::Tiff.category(), ObjectCategory::Image);
        assert_eq!(ObjectType::Mp4.category(), ObjectCategory::Video);
        assert_eq!(ObjectType::Mp3.category(), ObjectCategory::Audio);
        assert_eq!(ObjectType::Wav.category(), ObjectCategory::Audio);
        assert_eq!(ObjectType::Pdf.category(), ObjectCategory::Document);
        assert_eq!(ObjectType::Text.category(), ObjectCategory::Text);
        assert_eq!(ObjectType::Json.category(), ObjectCategory::Text);
        assert_eq!(ObjectType::Zip.category(), ObjectCategory::Archive);
        assert_eq!(ObjectType::GitBlob.category(), ObjectCategory::GitObject);
        assert_eq!(ObjectType::Unknown.category(), ObjectCategory::Unknown);
    }

    #[test]
    fn test_new_file_extensions() {
        assert_eq!(ObjectType::from_extension("avif"), ObjectType::Avif);
        assert_eq!(ObjectType::from_extension("heic"), ObjectType::Heic);
        assert_eq!(ObjectType::from_extension("exr"), ObjectType::Exr);
        assert_eq!(ObjectType::from_extension("hdr"), ObjectType::Hdr);
        assert_eq!(ObjectType::from_extension("flv"), ObjectType::Flv);
        assert_eq!(ObjectType::from_extension("wmv"), ObjectType::Wmv);
        assert_eq!(ObjectType::from_extension("opus"), ObjectType::Opus);
        assert_eq!(ObjectType::from_extension("aiff"), ObjectType::Aiff);
        assert_eq!(ObjectType::from_extension("toml"), ObjectType::Toml);
        assert_eq!(ObjectType::from_extension("csv"), ObjectType::Csv);
        assert_eq!(ObjectType::from_extension("7z"), ObjectType::SevenZ);
    }

    #[test]
    fn test_smart_compressor_multiple_types() {
        let compressor = SmartCompressor::new();

        // Test different types with same content
        let content = b"Test content ".repeat(100);

        let jpeg_result = compressor.compress_typed(&content, ObjectType::Jpeg).unwrap();
        let text_result = compressor.compress_typed(&content, ObjectType::Text).unwrap();
        let tiff_result = compressor.compress_typed(&content, ObjectType::Tiff).unwrap();

        // JPEG should not compress (store with 0x00 prefix)
        assert_eq!(jpeg_result.len(), content.len() + 1);

        // Text and TIFF should compress (different algorithms)
        assert!(text_result.len() < content.len());
        assert!(tiff_result.len() < content.len());

        // All should decompress correctly
        assert_eq!(compressor.decompress_typed(&jpeg_result).unwrap(), content);
        assert_eq!(compressor.decompress_typed(&text_result).unwrap(), content);
        assert_eq!(compressor.decompress_typed(&tiff_result).unwrap(), content);
    }

    #[test]
    fn test_smart_compressor_empty_data() {
        let compressor = SmartCompressor::new();
        let empty: &[u8] = b"";

        let compressed = compressor.compress_typed(empty, ObjectType::Text).unwrap();
        let decompressed = compressor.decompress_typed(&compressed).unwrap();

        assert_eq!(decompressed, empty);
    }

    #[test]
    fn test_debug_format() {
        let compressor = SmartCompressor::new();
        let debug_str = format!("{:?}", compressor);
        assert!(debug_str.contains("SmartCompressor"));
    }

    // ============================================================================
    // NEW TESTS FOR ENHANCED COMPRESSION STRATEGY
    // ============================================================================

    #[test]
    fn test_creative_project_file_extensions() {
        // Adobe Creative Cloud
        assert_eq!(ObjectType::from_extension("psd"), ObjectType::AdobePhotoshop);
        assert_eq!(ObjectType::from_extension("psb"), ObjectType::AdobePhotoshop);
        assert_eq!(ObjectType::from_extension("ai"), ObjectType::AdobeIllustrator);
        assert_eq!(ObjectType::from_extension("indd"), ObjectType::AdobeIndesign);
        assert_eq!(ObjectType::from_extension("aep"), ObjectType::AdobeAfterEffects);
        assert_eq!(ObjectType::from_extension("prproj"), ObjectType::AdobePremiere);

        // Video NLEs
        assert_eq!(ObjectType::from_extension("drp"), ObjectType::DavinciResolve);
        assert_eq!(ObjectType::from_extension("fcpbundle"), ObjectType::FinalCutPro);
        assert_eq!(ObjectType::from_extension("avb"), ObjectType::AvidMediaComposer);

        // 3D/DCC
        assert_eq!(ObjectType::from_extension("blend"), ObjectType::Blender);
        assert_eq!(ObjectType::from_extension("ma"), ObjectType::Maya);
        assert_eq!(ObjectType::from_extension("max"), ObjectType::ThreeDsMax);
        assert_eq!(ObjectType::from_extension("c4d"), ObjectType::Cinema4D);
        assert_eq!(ObjectType::from_extension("hip"), ObjectType::Houdini);

        // Audio DAWs
        assert_eq!(ObjectType::from_extension("ptx"), ObjectType::ProTools);
        assert_eq!(ObjectType::from_extension("als"), ObjectType::AbletonLive);
        assert_eq!(ObjectType::from_extension("flp"), ObjectType::FLStudio);
        assert_eq!(ObjectType::from_extension("logic"), ObjectType::LogicPro);

        // CAD
        assert_eq!(ObjectType::from_extension("dwg"), ObjectType::AutoCad);
        assert_eq!(ObjectType::from_extension("skp"), ObjectType::SketchUp);
        assert_eq!(ObjectType::from_extension("rvt"), ObjectType::Revit);

        // Game engines
        assert_eq!(ObjectType::from_extension("unity"), ObjectType::UnityProject);
        assert_eq!(ObjectType::from_extension("uasset"), ObjectType::UnrealProject);
        assert_eq!(ObjectType::from_extension("tscn"), ObjectType::GodotProject);
    }

    #[test]
    fn test_office_document_extensions() {
        assert_eq!(ObjectType::from_extension("docx"), ObjectType::WordDocument);
        assert_eq!(ObjectType::from_extension("doc"), ObjectType::WordDocument);
        assert_eq!(ObjectType::from_extension("xlsx"), ObjectType::ExcelSpreadsheet);
        assert_eq!(ObjectType::from_extension("xls"), ObjectType::ExcelSpreadsheet);
        assert_eq!(ObjectType::from_extension("pptx"), ObjectType::PowerpointPresentation);
        assert_eq!(ObjectType::from_extension("ppt"), ObjectType::PowerpointPresentation);
        assert_eq!(ObjectType::from_extension("odt"), ObjectType::OpenDocument);
        assert_eq!(ObjectType::from_extension("ods"), ObjectType::OpenDocument);
    }

    #[test]
    fn test_ml_specialized_extensions() {
        // Training checkpoints
        assert_eq!(ObjectType::from_extension("ckpt"), ObjectType::MlCheckpoint);
        assert_eq!(ObjectType::from_extension("pt"), ObjectType::MlCheckpoint);
        assert_eq!(ObjectType::from_extension("pth"), ObjectType::MlCheckpoint);

        // Inference models
        assert_eq!(ObjectType::from_extension("onnx"), ObjectType::MlInference);
        assert_eq!(ObjectType::from_extension("gguf"), ObjectType::MlInference);
        assert_eq!(ObjectType::from_extension("tflite"), ObjectType::MlInference);
        assert_eq!(ObjectType::from_extension("llamafile"), ObjectType::MlInference);
    }

    #[test]
    fn test_database_extensions() {
        assert_eq!(ObjectType::from_extension("sqlite"), ObjectType::SqliteDatabase);
        assert_eq!(ObjectType::from_extension("db"), ObjectType::SqliteDatabase);
        assert_eq!(ObjectType::from_extension("db3"), ObjectType::SqliteDatabase);
    }

    #[test]
    fn test_creative_project_categories() {
        assert_eq!(ObjectType::AdobePhotoshop.category(), ObjectCategory::CreativeProject);
        assert_eq!(ObjectType::Blender.category(), ObjectCategory::CreativeProject);
        assert_eq!(ObjectType::DavinciResolve.category(), ObjectCategory::CreativeProject);
        assert_eq!(ObjectType::ProTools.category(), ObjectCategory::CreativeProject);
        assert_eq!(ObjectType::AutoCad.category(), ObjectCategory::CreativeProject);
        assert_eq!(ObjectType::UnityProject.category(), ObjectCategory::CreativeProject);
    }

    #[test]
    fn test_office_category() {
        assert_eq!(ObjectType::WordDocument.category(), ObjectCategory::Office);
        assert_eq!(ObjectType::ExcelSpreadsheet.category(), ObjectCategory::Office);
        assert_eq!(ObjectType::PowerpointPresentation.category(), ObjectCategory::Office);
        assert_eq!(ObjectType::OpenDocument.category(), ObjectCategory::Office);
    }

    #[test]
    fn test_ml_specialized_category() {
        assert_eq!(ObjectType::MlCheckpoint.category(), ObjectCategory::MlSpecialized);
        assert_eq!(ObjectType::MlInference.category(), ObjectCategory::MlSpecialized);
    }

    #[test]
    fn test_database_category() {
        assert_eq!(ObjectType::SqliteDatabase.category(), ObjectCategory::Database);
    }

    #[test]
    fn test_creative_project_compression_strategy() {
        // All creative projects should use Zstd Default
        assert_eq!(
            CompressionStrategy::for_object_type(ObjectType::AdobePhotoshop),
            CompressionStrategy::Zstd(CompressionLevel::Default)
        );
        assert_eq!(
            CompressionStrategy::for_object_type(ObjectType::Blender),
            CompressionStrategy::Zstd(CompressionLevel::Default)
        );
        assert_eq!(
            CompressionStrategy::for_object_type(ObjectType::DavinciResolve),
            CompressionStrategy::Zstd(CompressionLevel::Default)
        );
    }

    #[test]
    fn test_ml_specialized_compression_strategy() {
        // Training checkpoints use Fast (for speed with large files)
        assert_eq!(
            CompressionStrategy::for_object_type(ObjectType::MlCheckpoint),
            CompressionStrategy::Zstd(CompressionLevel::Fast)
        );
        // Inference models use Default (better compression for archival)
        assert_eq!(
            CompressionStrategy::for_object_type(ObjectType::MlInference),
            CompressionStrategy::Zstd(CompressionLevel::Default)
        );
    }

    #[test]
    fn test_office_compression_strategy() {
        assert_eq!(
            CompressionStrategy::for_object_type(ObjectType::WordDocument),
            CompressionStrategy::Store
        );
        assert_eq!(
            CompressionStrategy::for_object_type(ObjectType::ExcelSpreadsheet),
            CompressionStrategy::Store
        );
    }

    #[test]
    fn test_text_uses_brotli() {
        // Verify text/structured data now uses Brotli instead of Zstd
        assert_eq!(
            CompressionStrategy::for_object_type(ObjectType::Text),
            CompressionStrategy::Brotli(CompressionLevel::Default)
        );
        assert_eq!(
            CompressionStrategy::for_object_type(ObjectType::Json),
            CompressionStrategy::Brotli(CompressionLevel::Default)
        );
        assert_eq!(
            CompressionStrategy::for_object_type(ObjectType::Csv),
            CompressionStrategy::Brotli(CompressionLevel::Default)
        );
        assert_eq!(
            CompressionStrategy::for_object_type(ObjectType::Xml),
            CompressionStrategy::Brotli(CompressionLevel::Default)
        );
    }

    #[test]
    fn test_case_insensitive_extensions() {
        // Test uppercase extensions work correctly
        assert_eq!(ObjectType::from_extension("PSD"), ObjectType::AdobePhotoshop);
        assert_eq!(ObjectType::from_extension("BLEND"), ObjectType::Blender);
        assert_eq!(ObjectType::from_extension("ONNX"), ObjectType::MlInference);
        assert_eq!(ObjectType::from_extension("DOCX"), ObjectType::WordDocument);
    }

    #[test]
    fn test_psd_no_longer_in_image_uncompressed() {
        // PSD is now AdobePhotoshop (creative project), not in uncompressed images
        // It should use Zstd Default, not Zstd Best
        assert_eq!(
            CompressionStrategy::for_object_type(ObjectType::AdobePhotoshop),
            CompressionStrategy::Zstd(CompressionLevel::Default)
        );

        // Compare with actual uncompressed image (should use Best)
        assert_eq!(
            CompressionStrategy::for_object_type(ObjectType::Tiff),
            CompressionStrategy::Zstd(CompressionLevel::Best)
        );
    }

    // ============================================================================
    // INTEGRATION TESTS - VERIFY ALL COMPRESSION/DECOMPRESSION FLOWS
    // ============================================================================

    #[test]
    fn test_integration_brotli_text_roundtrip() {
        // Test that Brotli compression for text types works end-to-end
        let compressor = SmartCompressor::new();

        // Test JSON
        let json_data = r#"{"name": "MediaGit", "version": "1.0", "features": ["compression", "delta"]}"#.repeat(50);
        let compressed = compressor.compress_typed(json_data.as_bytes(), ObjectType::Json).unwrap();
        assert!(compressed.len() < json_data.len(), "JSON should compress");
        let decompressed = compressor.decompress_typed(&compressed).unwrap();
        assert_eq!(json_data.as_bytes(), &decompressed[..], "JSON roundtrip failed");

        // Test CSV
        let csv_data = "id,name,value\n1,Alice,100\n2,Bob,200\n".repeat(100);
        let compressed = compressor.compress_typed(csv_data.as_bytes(), ObjectType::Csv).unwrap();
        assert!(compressed.len() < csv_data.len(), "CSV should compress");
        let decompressed = compressor.decompress_typed(&compressed).unwrap();
        assert_eq!(csv_data.as_bytes(), &decompressed[..], "CSV roundtrip failed");

        // Test XML
        let xml_data = "<root><item id=\"1\">Value</item></root>".repeat(50);
        let compressed = compressor.compress_typed(xml_data.as_bytes(), ObjectType::Xml).unwrap();
        assert!(compressed.len() < xml_data.len(), "XML should compress");
        let decompressed = compressor.decompress_typed(&compressed).unwrap();
        assert_eq!(xml_data.as_bytes(), &decompressed[..], "XML roundtrip failed");

        // Test plain text
        let text_data = "The quick brown fox jumps over the lazy dog. ".repeat(100);
        let compressed = compressor.compress_typed(text_data.as_bytes(), ObjectType::Text).unwrap();
        assert!(compressed.len() < text_data.len(), "Text should compress");
        let decompressed = compressor.decompress_typed(&compressed).unwrap();
        assert_eq!(text_data.as_bytes(), &decompressed[..], "Text roundtrip failed");
    }

    #[test]
    fn test_integration_creative_project_roundtrip() {
        // Test that creative project files use correct compression
        let compressor = SmartCompressor::new();

        // Simulate PSD file data (binary with some structure)
        let psd_data = vec![0x38, 0x42, 0x50, 0x53]; // "8BPS" header
        let mut data = psd_data.clone();
        data.extend_from_slice(&vec![0xAB; 10000]); // Add some data

        let compressed = compressor.compress_typed(&data, ObjectType::AdobePhotoshop).unwrap();
        assert!(compressed.len() < data.len(), "PSD should compress");

        let decompressed = compressor.decompress_typed(&compressed).unwrap();
        assert_eq!(data, decompressed, "PSD roundtrip failed");
    }

    #[test]
    fn test_integration_ml_specialized_roundtrip() {
        // Test ML checkpoint (Zstd Fast) vs inference model (Zstd Default)
        let compressor = SmartCompressor::new();

        // Simulate model weights (numeric data)
        let model_data = (0..5000).map(|x| (x % 256) as u8).collect::<Vec<_>>();

        // Test checkpoint compression
        let checkpoint_compressed = compressor.compress_typed(&model_data, ObjectType::MlCheckpoint).unwrap();
        let checkpoint_decompressed = compressor.decompress_typed(&checkpoint_compressed).unwrap();
        assert_eq!(model_data, checkpoint_decompressed, "Checkpoint roundtrip failed");

        // Test inference model compression
        let inference_compressed = compressor.compress_typed(&model_data, ObjectType::MlInference).unwrap();
        let inference_decompressed = compressor.decompress_typed(&inference_compressed).unwrap();
        assert_eq!(model_data, inference_decompressed, "Inference model roundtrip failed");

        // Both should work, but inference might compress better (Default vs Fast)
        // We just verify both decompress correctly
    }

    #[test]
    fn test_integration_office_document_roundtrip() {
        // Test office documents (ZIP containers with XML)
        let compressor = SmartCompressor::new();

        // Simulate docx structure (ZIP-like)
        let docx_data = b"PK\x03\x04...document content...".repeat(100);

        let compressed = compressor.compress_typed(&docx_data, ObjectType::WordDocument).unwrap();
        let decompressed = compressor.decompress_typed(&compressed).unwrap();
        assert_eq!(docx_data, &decompressed[..], "DOCX roundtrip failed");
    }

    #[test]
    fn test_integration_database_roundtrip() {
        // Test SQLite database compression
        let compressor = SmartCompressor::new();

        // Simulate SQLite data
        let db_data = b"SQLite format 3\x00...table data...".repeat(100);

        let compressed = compressor.compress_typed(&db_data, ObjectType::SqliteDatabase).unwrap();
        let decompressed = compressor.decompress_typed(&compressed).unwrap();
        assert_eq!(db_data, &decompressed[..], "SQLite roundtrip failed");
    }

    #[test]
    fn test_integration_auto_detection_mixed_types() {
        // Test that auto-detection works across all compression types
        let compressor = SmartCompressor::new();

        let test_data = b"Test data for compression ".repeat(50);

        // Compress with different types and verify all decompress correctly
        let types = vec![
            ObjectType::Text,           // Brotli
            ObjectType::Json,           // Brotli
            ObjectType::AdobePhotoshop, // Zstd Default
            ObjectType::MlCheckpoint,   // Zstd Fast
            ObjectType::WordDocument,   // Zstd Default
            ObjectType::Tiff,           // Zstd Best
        ];

        for obj_type in types {
            let compressed = compressor.compress_typed(&test_data, obj_type).unwrap();
            let decompressed = compressor.decompress_typed(&compressed).unwrap();
            assert_eq!(
                test_data,
                &decompressed[..],
                "Auto-detection failed for {:?}",
                obj_type
            );
        }
    }

    #[test]
    fn test_integration_already_compressed_types() {
        // Verify that already-compressed types are stored without recompression
        let compressor = SmartCompressor::new();

        // Simulate compressed formats
        let jpeg_data = vec![0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x10, 0x4A, 0x46];
        let mp4_data = b"....ftypisom....";
        let zip_data = vec![0x50, 0x4B, 0x03, 0x04];

        // These should be stored with 0x00 Store prefix (not recompressed)
        let jpeg_compressed = compressor.compress_typed(&jpeg_data, ObjectType::Jpeg).unwrap();
        assert_eq!(jpeg_compressed[0], 0x00, "JPEG should have Store prefix");
        assert_eq!(&jpeg_compressed[1..], &jpeg_data[..], "JPEG should not be recompressed");

        let mp4_compressed = compressor.compress_typed(mp4_data, ObjectType::Mp4).unwrap();
        assert_eq!(mp4_compressed[0], 0x00, "MP4 should have Store prefix");
        assert_eq!(&mp4_compressed[1..], mp4_data, "MP4 should not be recompressed");

        let zip_compressed = compressor.compress_typed(&zip_data, ObjectType::Zip).unwrap();
        assert_eq!(zip_compressed[0], 0x00, "ZIP should have Store prefix");
        assert_eq!(&zip_compressed[1..], &zip_data[..], "ZIP should not be recompressed");
    }

    #[test]
    fn test_integration_compression_ratio_expectations() {
        // Test that compression ratios meet expectations for different types
        let compressor = SmartCompressor::new();

        // Highly repetitive text should compress very well with Brotli
        let repetitive_text = "AAAAAAAAAA".repeat(1000);
        let compressed = compressor.compress_typed(repetitive_text.as_bytes(), ObjectType::Text).unwrap();
        let ratio = compressed.len() as f64 / repetitive_text.len() as f64;
        assert!(ratio < 0.1, "Repetitive text should compress to <10% with Brotli, got {:.2}%", ratio * 100.0);

        // Verify decompression
        let decompressed = compressor.decompress_typed(&compressed).unwrap();
        assert_eq!(repetitive_text.as_bytes(), &decompressed[..]);
    }

    #[test]
    fn test_integration_empty_data_all_types() {
        // Verify empty data handling for all compression strategies
        let compressor = SmartCompressor::new();
        let empty: &[u8] = b"";

        let types = vec![
            ObjectType::Text,           // Brotli
            ObjectType::Json,           // Brotli
            ObjectType::AdobePhotoshop, // Zstd Default
            ObjectType::MlCheckpoint,   // Zstd Fast
            ObjectType::Tiff,           // Zstd Best
            ObjectType::Jpeg,           // Store
        ];

        for obj_type in types {
            let compressed = compressor.compress_typed(empty, obj_type).unwrap();
            let decompressed = compressor.decompress_typed(&compressed).unwrap();
            assert_eq!(empty, &decompressed[..], "Empty data failed for {:?}", obj_type);
        }
    }

    #[test]
    fn test_integration_large_data_performance() {
        // Test that large files compress/decompress correctly
        let compressor = SmartCompressor::new();

        // 1MB of structured data
        let large_json = format!(r#"{{"data": [{}]}}"#, (0..10000).map(|i| format!("{}", i)).collect::<Vec<_>>().join(","));

        let compressed = compressor.compress_typed(large_json.as_bytes(), ObjectType::Json).unwrap();
        assert!(compressed.len() < large_json.len(), "Large JSON should compress");

        let decompressed = compressor.decompress_typed(&compressed).unwrap();
        assert_eq!(large_json.as_bytes(), &decompressed[..], "Large JSON roundtrip failed");

        // Verify significant compression for structured data
        let ratio = compressed.len() as f64 / large_json.len() as f64;
        assert!(ratio < 0.5, "Large JSON should compress to <50%, got {:.2}%", ratio * 100.0);
    }

    #[test]
    fn test_integration_all_new_extensions_mapped() {
        // Verify all new extensions have valid mappings
        let new_extensions = vec![
            // Creative projects
            ("psd", ObjectType::AdobePhotoshop),
            ("ai", ObjectType::AdobeIllustrator),
            ("indd", ObjectType::AdobeIndesign),
            ("aep", ObjectType::AdobeAfterEffects),
            ("prproj", ObjectType::AdobePremiere),
            ("drp", ObjectType::DavinciResolve),
            ("blend", ObjectType::Blender),
            ("ma", ObjectType::Maya),
            ("als", ObjectType::AbletonLive),
            ("dwg", ObjectType::AutoCad),
            ("unity", ObjectType::UnityProject),
            // Office
            ("docx", ObjectType::WordDocument),
            ("xlsx", ObjectType::ExcelSpreadsheet),
            ("pptx", ObjectType::PowerpointPresentation),
            ("odt", ObjectType::OpenDocument),
            // ML
            ("ckpt", ObjectType::MlCheckpoint),
            ("onnx", ObjectType::MlInference),
            // Database
            ("sqlite", ObjectType::SqliteDatabase),
        ];

        for (ext, expected_type) in new_extensions {
            let detected_type = ObjectType::from_extension(ext);
            assert_eq!(
                detected_type,
                expected_type,
                "Extension '{}' should map to {:?}, got {:?}",
                ext,
                expected_type,
                detected_type
            );

            // Verify each type has a compression strategy
            let strategy = CompressionStrategy::for_object_type(detected_type);
            assert!(
                matches!(
                    strategy,
                    CompressionStrategy::Store
                        | CompressionStrategy::Zlib(_)
                        | CompressionStrategy::Zstd(_)
                        | CompressionStrategy::Brotli(_)
                        | CompressionStrategy::Delta
                ),
                "Type {:?} has invalid strategy: {:?}",
                detected_type,
                strategy
            );
        }
    }

    #[test]
    fn test_integration_category_coverage() {
        // Verify all new categories are properly configured
        let category_samples = vec![
            (ObjectType::AdobePhotoshop, ObjectCategory::CreativeProject),
            (ObjectType::WordDocument, ObjectCategory::Office),
            (ObjectType::MlCheckpoint, ObjectCategory::MlSpecialized),
            (ObjectType::SqliteDatabase, ObjectCategory::Database),
        ];

        for (obj_type, expected_category) in category_samples {
            let category = obj_type.category();
            assert_eq!(
                category,
                expected_category,
                "{:?} should be in {:?} category",
                obj_type,
                expected_category
            );
        }
    }
}
