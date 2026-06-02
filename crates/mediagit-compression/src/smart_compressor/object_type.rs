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

/// Object/File type classification for compression strategy selection
#[allow(missing_docs)]
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
    Dpx, // DPX digital intermediate (uncompressed frames, VFX)

    // Video formats (typically already compressed)
    Mp4,
    Mov,
    Avi,
    Mkv,
    Webm,
    Flv,
    Wmv,
    Mpg,
    Mxf, // MXF container (broadcast/VFX professional)

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

    // 3D interchange/exchange formats (mesh/scene data)
    Model3D, // .stl, .obj, .fbx, .glb, .gltf, .ply, .dae, .abc, .3ds, .usd, .usda, .usdc

    // Creative project files - 3D/DCC
    Blender,    // .blend
    Maya,       // .ma, .mb
    ThreeDsMax, // .max
    Cinema4D,   // .c4d
    Houdini,    // .hip, .hipnc

    // Creative project files - Audio DAWs
    ProTools,    // .ptx
    AbletonLive, // .als
    FLStudio,    // .flp
    LogicPro,    // .logic, .logicx

    // Creative project files - CAD
    AutoCad,  // .dwg, .dxf
    SketchUp, // .skp
    Revit,    // .rvt

    // Creative project files - Game engines
    UnityProject,  // .unity, .prefab, .asset
    UnrealProject, // .uasset, .umap
    GodotProject,  // .tscn, .tres

    // Office documents (modern XML-based)
    WordDocument,           // .docx, .doc
    ExcelSpreadsheet,       // .xlsx, .xls
    PowerpointPresentation, // .pptx, .ppt
    OpenDocument,           // .odt, .ods, .odp

    // Database formats
    SqliteDatabase, // .sqlite, .db, .db3

    // Compressed text/logs
    CompressedLog, // .log.gz, .log.bz2

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
            // Cinema camera RAW (VFX/broadcast)
            "braw" | "r3d" | "ari" | "arriraw" | "cine" | "crm" => ObjectType::Raw,
            "exr" => ObjectType::Exr,
            "hdr" | "pic" => ObjectType::Hdr,
            "dpx" => ObjectType::Dpx,

            // Video
            "mp4" | "m4v" => ObjectType::Mp4,
            "mov" | "qt" => ObjectType::Mov,
            "avi" => ObjectType::Avi,
            "mkv" | "mk3d" => ObjectType::Mkv,
            "webm" => ObjectType::Webm,
            "mka" => ObjectType::Mkv,
            "flv" | "f4v" => ObjectType::Flv,
            "wmv" | "asf" => ObjectType::Wmv,
            "mpg" | "mpeg" | "m2v" => ObjectType::Mpg,
            "mxf" => ObjectType::Mxf,
            // MPEG transport streams and legacy compressed video → Store
            // Note: .ts/.mts are also TypeScript extensions; .m2ts/.vob are unambiguously video
            "m2ts" | "vob" | "m2p" => ObjectType::Mpg,
            // Mobile video containers → Store
            "3gp" | "3g2" | "3gpp" | "3gpp2" => ObjectType::Mp4,
            // Legacy compressed video → Store
            "rm" | "rmvb" | "rv" => ObjectType::Flv,

            // Audio (compressed)
            "mp3" => ObjectType::Mp3,
            "aac" => ObjectType::Aac,
            "m4a" | "m4b" | "m4r" => ObjectType::Aac, // AAC in MPEG-4 container
            "ogg" | "oga" => ObjectType::Ogg,
            "opus" => ObjectType::Opus,
            // Additional compressed audio formats → Store
            "wma" | "amr" | "awb" => ObjectType::Aac,

            // Audio (uncompressed/lossless)
            "flac" => ObjectType::Flac,
            "wav" => ObjectType::Wav,
            "aiff" | "aif" | "aifc" => ObjectType::Aiff,
            "alac" => ObjectType::Alac,
            // Additional lossless audio → Zstd Best
            "ape" | "wv" | "wvp" => ObjectType::Flac,

            // Documents
            "pdf" => ObjectType::Pdf,
            "svg" | "svgz" => ObjectType::Svg,
            "eps" => ObjectType::Eps, // "ai" moved to AdobeIllustrator

            // Text/Code
            "txt" | "md" | "markdown" | "rst" | "adoc" | "rs" | "js" | "ts" | "jsx" | "tsx"
            | "py" | "go" | "c" | "cpp" | "cc" | "cxx" | "h" | "hpp" | "hh" | "hxx" | "java"
            | "kt" | "swift" | "rb" | "php" | "sh" | "bash" | "zsh" | "fish" | "vim" | "lua"
            | "pl" | "r" | "m" => ObjectType::Text,
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
            // Additional compressed archive formats → Store
            "bz2" | "bzip2" | "xz" | "lzma" | "lz4" | "zst" | "zstd" | "lz" | "z" | "br" => {
                ObjectType::Gz
            }
            // ZIP-based app packages and installers → Store
            "whl" | "egg" | "apk" | "ipa" | "aab" | "jar" | "war" | "ear" | "crx" | "xpi" => {
                ObjectType::Zip
            }

            // ML/Data formats (internally compressed)
            "parquet" | "arrow" | "feather" | "orc" | "avro" => ObjectType::Parquet,

            // ML data formats (arrays, tensors)
            "hdf5" | "h5" | "nc" | "netcdf" | "npy" | "npz" | "tfrecords" | "petastorm" => {
                ObjectType::MlData
            }

            // ML model weights (general)
            "pb" | "safetensors" | "pkl" | "joblib" => ObjectType::MlModel,

            // ML training checkpoints (large, frequent saves during training)
            // Note: .pt/.pth/.bin can be either checkpoints or inference models
            // We default to checkpoint for aggressive compression since they're more common
            "ckpt" | "pt" | "pth" | "bin" => ObjectType::MlCheckpoint,

            // ML inference models (optimized for deployment)
            "onnx" | "gguf" | "ggml" | "tflite" | "mlmodel" | "coreml" | "keras" | "pte"
            | "mleap" | "pmml" | "llamafile" => ObjectType::MlInference,

            // Creative projects - Adobe Creative Cloud
            "ai" | "ait" => ObjectType::AdobeIllustrator,
            "indd" | "idml" | "indt" => ObjectType::AdobeIndesign,
            "aep" | "aet" => ObjectType::AdobeAfterEffects,
            "prproj" | "psq" => ObjectType::AdobePremiere,

            // Creative projects - Video editing
            "drp" | "drp_proxies" => ObjectType::DavinciResolve,
            "fcpbundle" | "fcpxml" | "fcpxmld" => ObjectType::FinalCutPro,
            "avb" | "avp" | "avs" => ObjectType::AvidMediaComposer,

            // 3D interchange/exchange formats (mesh/scene data)
            // Note: usdz is a ZIP container → maps to Zip (Store strategy)
            "stl" | "obj" | "fbx" | "glb" | "gltf" | "ply" | "dae" | "abc" | "3ds" | "usd"
            | "usda" | "usdc" => ObjectType::Model3D,
            "usdz" => ObjectType::Zip,

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

        // RIFF container: dispatch by subtype at bytes 8-11
        if data.len() >= 12 && data.starts_with(b"RIFF") {
            return match &data[8..12] {
                b"WEBP" => ObjectType::Webp,
                b"WAVE" => ObjectType::Wav,
                b"AVI " => ObjectType::Avi,
                _ => ObjectType::Unknown,
            };
        }

        // TIFF: 49 49 2A 00 (little-endian) or 4D 4D 00 2A (big-endian)
        if data.starts_with(&[0x49, 0x49, 0x2A, 0x00])
            || data.starts_with(&[0x4D, 0x4D, 0x00, 0x2A])
        {
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
        if data.starts_with(&[0x50, 0x4B, 0x03, 0x04])
            || data.starts_with(&[0x50, 0x4B, 0x05, 0x06])
        {
            return ObjectType::Zip;
        }

        // GZIP: 1F 8B
        if data.starts_with(&[0x1F, 0x8B]) {
            return ObjectType::Gz;
        }

        // MKV/WebM: EBML header (Matroska container)
        if data.starts_with(&[0x1A, 0x45, 0xDF, 0xA3]) {
            return ObjectType::Mkv;
        }

        // FLAC: "fLaC"
        if data.starts_with(b"fLaC") {
            return ObjectType::Flac;
        }

        // EXR: OpenEXR magic
        if data.starts_with(&[0x76, 0x2F, 0x31, 0x01]) {
            return ObjectType::Exr;
        }

        // PSD/PSB: "8BPS"
        if data.starts_with(&[0x38, 0x42, 0x50, 0x53]) {
            return ObjectType::AdobePhotoshop;
        }

        // 7-Zip: 37 7A BC AF 27 1C
        if data.len() >= 6 && data.starts_with(&[0x37, 0x7A, 0xBC, 0xAF, 0x27, 0x1C]) {
            return ObjectType::SevenZ;
        }

        // RAR5: "Rar!\x1A\x07"
        if data.len() >= 6 && data.starts_with(&[0x52, 0x61, 0x72, 0x21, 0x1A, 0x07]) {
            return ObjectType::Rar;
        }

        // XZ: FD 37 7A 58 5A 00
        if data.len() >= 6 && data.starts_with(&[0xFD, 0x37, 0x7A, 0x58, 0x5A, 0x00]) {
            return ObjectType::Gz;
        }

        // Bzip2: "BZh"
        if data.starts_with(&[0x42, 0x5A, 0x68]) {
            return ObjectType::Gz;
        }

        // Zstd frame: 28 B5 2F FD
        if data.starts_with(&[0x28, 0xB5, 0x2F, 0xFD]) {
            return ObjectType::Gz;
        }

        // LZ4 frame: 04 22 4D 18
        if data.starts_with(&[0x04, 0x22, 0x4D, 0x18]) {
            return ObjectType::Gz;
        }

        // MP3: ID3 tag at start
        if data.starts_with(b"ID3") {
            return ObjectType::Mp3;
        }

        // MP3: sync word (conservative: require >32 bytes + valid sync bits)
        if data.len() > 32 && data[0] == 0xFF && (data[1] & 0xE0) == 0xE0 {
            return ObjectType::Mp3;
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
                | ObjectType::Mxf
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
            ObjectType::Jpeg
            | ObjectType::Png
            | ObjectType::Gif
            | ObjectType::Webp
            | ObjectType::Avif
            | ObjectType::Heic
            | ObjectType::GpuTexture
            | ObjectType::Tiff
            | ObjectType::Bmp
            | ObjectType::Raw
            | ObjectType::Exr
            | ObjectType::Hdr
            | ObjectType::Dpx => ObjectCategory::Image,

            ObjectType::Mp4
            | ObjectType::Mov
            | ObjectType::Avi
            | ObjectType::Mkv
            | ObjectType::Webm
            | ObjectType::Flv
            | ObjectType::Wmv
            | ObjectType::Mpg
            | ObjectType::Mxf => ObjectCategory::Video,

            ObjectType::Mp3
            | ObjectType::Aac
            | ObjectType::Ogg
            | ObjectType::Opus
            | ObjectType::Flac
            | ObjectType::Wav
            | ObjectType::Aiff
            | ObjectType::Alac => ObjectCategory::Audio,

            ObjectType::Pdf | ObjectType::Svg | ObjectType::Eps => ObjectCategory::Document,

            ObjectType::Text
            | ObjectType::Json
            | ObjectType::Xml
            | ObjectType::Yaml
            | ObjectType::Toml
            | ObjectType::Csv => ObjectCategory::Text,

            ObjectType::Zip
            | ObjectType::Tar
            | ObjectType::Gz
            | ObjectType::SevenZ
            | ObjectType::Rar
            | ObjectType::CompressedLog => ObjectCategory::Archive,

            ObjectType::Parquet
            | ObjectType::MlData
            | ObjectType::MlModel
            | ObjectType::MlDeployment => ObjectCategory::Archive, // ML formats as data archives

            // ML specialized (training vs inference)
            ObjectType::MlCheckpoint | ObjectType::MlInference => ObjectCategory::MlSpecialized,

            // 3D interchange/exchange formats
            ObjectType::Model3D => ObjectCategory::CreativeProject,

            // Creative project files
            ObjectType::AdobePhotoshop
            | ObjectType::AdobeIllustrator
            | ObjectType::AdobeIndesign
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
            | ObjectType::GodotProject => ObjectCategory::CreativeProject,

            // Office documents
            ObjectType::WordDocument
            | ObjectType::ExcelSpreadsheet
            | ObjectType::PowerpointPresentation
            | ObjectType::OpenDocument => ObjectCategory::Office,

            // Database
            ObjectType::SqliteDatabase => ObjectCategory::Database,

            ObjectType::GitBlob | ObjectType::GitTree | ObjectType::GitCommit => {
                ObjectCategory::GitObject
            }

            ObjectType::Unknown => ObjectCategory::Unknown,
        }
    }
}

/// Object category for high-level classification
#[allow(missing_docs)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ObjectCategory {
    Image,
    Video,
    Audio,
    Document,
    Text,
    Archive,
    CreativeProject, // Adobe, video NLEs, DAWs, 3D/DCC, CAD, game engines
    Office,          // Word, Excel, PowerPoint, OpenDocument
    MlSpecialized,   // ML training checkpoints vs inference models
    Database,        // SQLite, database files
    GitObject,
    Unknown,
}

/// Size threshold for switching from Brotli to Zstd for text files.
/// At 500 MB+, Brotli level-9 encodes ~10× slower than Zstd with only ~20% worse ratio.
pub(super) const LARGE_TEXT_THRESHOLD: usize = 500 * 1024 * 1024; // 500 MB
