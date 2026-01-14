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
    Psd,
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
            "psd" | "psb" => ObjectType::Psd,
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
            "eps" | "ai" => ObjectType::Eps,

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
            
            // ML model weights
            "pt" | "pth" | "ckpt" | "pb" | "safetensors" | "bin" |
            "pkl" | "joblib" => ObjectType::MlModel,
            
            // ML deployment formats
            "onnx" | "gguf" | "ggml" | "tflite" | "mlmodel" | "coreml" |
            "keras" | "pte" | "mleap" | "pmml" | "llamafile" => ObjectType::MlDeployment,

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
        )
    }

    /// Get the category of this object type
    pub fn category(self) -> ObjectCategory {
        match self {
            ObjectType::Jpeg | ObjectType::Png | ObjectType::Gif | ObjectType::Webp |
            ObjectType::Avif | ObjectType::Heic | ObjectType::GpuTexture | ObjectType::Tiff | ObjectType::Bmp |
            ObjectType::Psd | ObjectType::Raw | ObjectType::Exr | ObjectType::Hdr => ObjectCategory::Image,

            ObjectType::Mp4 | ObjectType::Mov | ObjectType::Avi | ObjectType::Mkv |
            ObjectType::Webm | ObjectType::Flv | ObjectType::Wmv | ObjectType::Mpg => ObjectCategory::Video,

            ObjectType::Mp3 | ObjectType::Aac | ObjectType::Ogg | ObjectType::Opus |
            ObjectType::Flac | ObjectType::Wav | ObjectType::Aiff | ObjectType::Alac => ObjectCategory::Audio,

            ObjectType::Pdf | ObjectType::Svg | ObjectType::Eps => ObjectCategory::Document,

            ObjectType::Text | ObjectType::Json | ObjectType::Xml | ObjectType::Yaml |
            ObjectType::Toml | ObjectType::Csv => ObjectCategory::Text,

            ObjectType::Zip | ObjectType::Tar | ObjectType::Gz | ObjectType::SevenZ |
            ObjectType::Rar => ObjectCategory::Archive,

            ObjectType::Parquet | ObjectType::MlData | ObjectType::MlModel |
            ObjectType::MlDeployment => ObjectCategory::Archive, // ML formats as data archives

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
    GitObject,
    Unknown,
}

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
            | ObjectType::Psd
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

            // Text/Code: Zstd default (fast with good compression)
            // Note: Brotli has better ratios but is 100x slower for large files
            ObjectType::Text
            | ObjectType::Json
            | ObjectType::Xml
            | ObjectType::Yaml
            | ObjectType::Toml
            | ObjectType::Csv => CompressionStrategy::Zstd(CompressionLevel::Default),

            // Already compressed archives: store
            ObjectType::Zip
            | ObjectType::Gz
            | ObjectType::SevenZ
            | ObjectType::Rar
            | ObjectType::Parquet => CompressionStrategy::Store,

            // TAR is uncompressed container
            ObjectType::Tar => CompressionStrategy::Zstd(CompressionLevel::Default),

            // ML data/models: Zstd fast (good for large numeric arrays)
            ObjectType::MlData
            | ObjectType::MlModel
            | ObjectType::MlDeployment => CompressionStrategy::Zstd(CompressionLevel::Fast),

            // Git objects: Zlib for compatibility
            ObjectType::GitBlob
            | ObjectType::GitTree
            | ObjectType::GitCommit => CompressionStrategy::Zlib(CompressionLevel::Default),

            // Unknown/binary: Zstd default (safe choice)
            ObjectType::Unknown => CompressionStrategy::Zstd(CompressionLevel::Default),
        }
    }
}

/// Type-aware compressor trait
pub trait TypeAwareCompressor: Send + Sync {
    /// Compress with automatic strategy selection
    fn compress_typed(&self, data: &[u8], obj_type: ObjectType) -> CompressionResult<Vec<u8>>;

    /// Decompress data (auto-detects algorithm)
    fn decompress_typed(&self, data: &[u8]) -> CompressionResult<Vec<u8>>;

    /// Get compression strategy for object type
    fn strategy_for_type(&self, obj_type: ObjectType) -> CompressionStrategy;
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
    fn compress_with_strategy(
        &self,
        data: &[u8],
        strategy: CompressionStrategy,
    ) -> CompressionResult<Vec<u8>> {
        match strategy {
            CompressionStrategy::Store => Ok(data.to_vec()),

            CompressionStrategy::Zlib(level) => {
                let compressor = ZlibCompressor::new(level);
                compressor.compress(data)
            }

            CompressionStrategy::Zstd(level) => {
                let compressor = match level {
                    CompressionLevel::Fast => &self.zstd_fast,
                    CompressionLevel::Default => &self.zstd_default,
                    CompressionLevel::Best => &self.zstd_best,
                };
                compressor.compress(data)
            }

            CompressionStrategy::Brotli(level) => {
                let compressor = BrotliCompressor::new(level);
                compressor.compress(data)
            }

            CompressionStrategy::Delta => {
                // Delta compression requires a base - not implemented in simple compress
                // Fall back to Zstd
                self.zstd_default.compress(data)
            }
        }
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

    fn decompress_typed(&self, data: &[u8]) -> CompressionResult<Vec<u8>> {
        // Auto-detect compression algorithm
        use crate::CompressionAlgorithm;

        let algo = CompressionAlgorithm::detect(data);

        match algo {
            CompressionAlgorithm::None => Ok(data.to_vec()),
            CompressionAlgorithm::Zlib => self.zlib.decompress(data),
            CompressionAlgorithm::Zstd => self.zstd_default.decompress(data),
            CompressionAlgorithm::Brotli => self.brotli_best.decompress(data),
        }
    }

    fn strategy_for_type(&self, obj_type: ObjectType) -> CompressionStrategy {
        CompressionStrategy::for_object_type(obj_type)
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

        assert!(!ObjectType::Tiff.is_already_compressed());
        assert!(!ObjectType::Bmp.is_already_compressed());
        assert!(!ObjectType::Text.is_already_compressed());
        assert!(!ObjectType::Raw.is_already_compressed());
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

        // Text → Brotli Best
        assert_eq!(
            CompressionStrategy::for_object_type(ObjectType::Text),
            CompressionStrategy::Brotli(CompressionLevel::Best)
        );
        assert_eq!(
            CompressionStrategy::for_object_type(ObjectType::Json),
            CompressionStrategy::Brotli(CompressionLevel::Best)
        );

        // Documents → Zstd Default
        assert_eq!(
            CompressionStrategy::for_object_type(ObjectType::Pdf),
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

        // Should store as-is (no compression)
        assert_eq!(compressed, jpeg_data);
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
            ObjectType::Tiff, ObjectType::Bmp, ObjectType::Psd, ObjectType::Raw,
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

        // JPEG should not compress (store)
        assert_eq!(jpeg_result.len(), content.len());

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
}
