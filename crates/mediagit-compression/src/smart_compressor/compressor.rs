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
use crate::CompressionAlgorithm;
use crate::error::CompressionError;
use mediagit_security::encryption::EncryptionKey;
use std::io::{Cursor, Read, Write};

/// Type-aware compressor trait
pub trait TypeAwareCompressor: Send + Sync {
    /// Compress with automatic strategy selection
    fn compress_typed(&self, data: &[u8], obj_type: ObjectType) -> CompressionResult<Vec<u8>>;

    /// Compress with automatic strategy selection considering data size.
    /// Text types switch from Brotli to Zstd at ≥500 MB for speed.
    fn compress_typed_with_size(
        &self,
        data: &[u8],
        obj_type: ObjectType,
    ) -> CompressionResult<Vec<u8>>;

    /// Decompress data (auto-detects algorithm)
    fn decompress_typed(&self, data: &[u8]) -> CompressionResult<Vec<u8>>;

    /// Get compression strategy for object type
    fn strategy_for_type(&self, obj_type: ObjectType) -> CompressionStrategy;

    /// Get compression strategy for object type with size consideration
    fn strategy_for_type_with_size(
        &self,
        obj_type: ObjectType,
        data_size: usize,
    ) -> CompressionStrategy;
}

/// Smart compressor with automatic type-based strategy selection
#[derive(Clone)]
pub struct SmartCompressor {
    zlib: ZlibCompressor,
    zstd_fast: ZstdCompressor,
    zstd_default: ZstdCompressor,
    zstd_best: ZstdCompressor,
    brotli_best: BrotliCompressor,
    /// DC-7: at-rest encryption key, `None` for every repo that has not opted in.
    ///
    /// It lives here, and not at the ~30 `compress`/`decompress` call sites in
    /// `odb/`, because this type is the one thing all of them route through.
    /// Encrypting per call site would be 30 edits and 30 chances to miss one —
    /// and a missed READ site is not a missed feature, it is a repository that
    /// cannot be read back. The "ODB bypass" bug class has recurred six times
    /// in this codebase for exactly that reason.
    key: Option<EncryptionKey>,
}

impl SmartCompressor {
    /// Create new smart compressor with all algorithms ready.
    ///
    /// Adopts the process-global at-rest key if one was installed at startup
    /// (see [`crate::process_key`]). That indirection is the point: a new
    /// `SmartCompressor::new()` anywhere in the tree is encrypted-repo-correct
    /// without its author knowing encryption exists. With no key installed —
    /// the default, and every repo that has not opted in — this is one atomic
    /// load and the behaviour is byte-for-byte what it was before DC-7.
    pub fn new() -> Self {
        Self {
            zlib: ZlibCompressor::new(CompressionLevel::Default),
            zstd_fast: ZstdCompressor::new(CompressionLevel::Fast),
            zstd_default: ZstdCompressor::new(CompressionLevel::Default),
            zstd_best: ZstdCompressor::new(CompressionLevel::Best),
            brotli_best: BrotliCompressor::new(CompressionLevel::Best),
            key: crate::process_key::process_key().cloned(),
        }
    }

    /// Encrypt everything this compressor writes, and decrypt what it reads.
    ///
    /// Overrides the process-global key from [`crate::process_key`] for this
    /// one compressor. Kept as an explicit escape hatch because tests need to
    /// build keyed and unkeyed compressors side by side in one process, which
    /// a set-once global cannot express.
    ///
    /// Objects are sealed **after** compression, so compression still happens
    /// and still pays before the payload becomes incompressible.
    ///
    /// Reads stay tolerant in one direction only: with a key set, an object
    /// that is *not* sealed still reads normally, so a repo that switches
    /// encryption on keeps its existing objects readable and new writes are
    /// sealed. The reverse is deliberately fatal — see [`Self::unseal`].
    pub fn with_key(mut self, key: EncryptionKey) -> Self {
        self.key = Some(key);
        self
    }

    /// Is at-rest encryption configured?
    pub fn is_encrypted(&self) -> bool {
        self.key.is_some()
    }

    /// Seal `data` if a key is configured, otherwise hand it back untouched.
    ///
    /// The untouched path is what keeps the frozen format frozen: with no key,
    /// the bytes written are byte-for-byte what they were before DC-7 existed.
    fn seal(&self, data: Vec<u8>) -> CompressionResult<Vec<u8>> {
        match &self.key {
            None => Ok(data),
            Some(k) => mediagit_security::envelope::seal(k, &data)
                .map_err(|e| CompressionError::compression_failed(format!("seal object: {e}"))),
        }
    }

    /// Unseal `data` if it is sealed, otherwise hand it back untouched.
    ///
    /// A sealed object with **no key configured** is a hard error, never a
    /// pass-through. Returning ciphertext here would send it on to the codec
    /// sniffer, which would find no magic it recognises, classify it as
    /// uncompressed, and hand a caller random bytes as if they were content.
    /// The caller's next act is to check an OID or write the result somewhere:
    /// silent corruption. Failing closed turns a misconfiguration into a
    /// message.
    fn unseal<'a>(&self, data: &'a [u8]) -> CompressionResult<std::borrow::Cow<'a, [u8]>> {
        if !mediagit_security::envelope::is_sealed(data) {
            return Ok(std::borrow::Cow::Borrowed(data));
        }
        let Some(k) = &self.key else {
            return Err(CompressionError::decompression_failed(
                "object is encrypted (MGEN envelope) but no encryption key is configured \
                 for this repository"
                    .to_string(),
            ));
        };
        mediagit_security::envelope::open(k, data)
            .map(std::borrow::Cow::Owned)
            .map_err(|e| CompressionError::decompression_failed(format!("open object: {e}")))
    }

    /// Compress a demuxed chunk using codec-aware strategy.
    ///
    /// Returns `None` if the codec hint is `Unknown` (caller should fall back to
    /// file-level `compress_typed_with_size`).
    pub fn compress_by_codec(
        &self,
        data: &[u8],
        codec_hint: ChunkCodecHint,
    ) -> Option<CompressionResult<Vec<u8>>> {
        let strategy = CompressionStrategy::for_codec_hint(codec_hint)?;
        Some(self.compress_with_strategy(data, strategy))
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
            return self.seal(result);
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
            return self.seal(result);
        }

        self.seal(compressed)
    }

    /// Decompress a byte stream, writing decompressed bytes to `sink` as they
    /// become available instead of returning a buffered `Vec<u8>`.
    ///
    /// Reuses exactly the codec-detection rule [`decompress_typed`](TypeAwareCompressor::decompress_typed)
    /// applies to a whole buffer — Store magic byte, then [`CompressionAlgorithm::detect`]
    /// — so the streaming and whole-buffer paths can never diverge on what a
    /// given chunk decodes to. Only ever buffers a fixed 4-byte peek (to pick
    /// the codec) and a fixed-size copy buffer; never the compressed input or
    /// the decompressed output in full. Callers needing an incremental digest
    /// (rather than the bytes themselves) pass a `Write` that hashes and
    /// discards, e.g. a `blake3::Hasher` wrapper.
    pub fn decompress_streaming(
        &self,
        mut reader: impl Read,
        mut sink: impl Write,
    ) -> CompressionResult<()> {
        // Same peek width as `CompressionAlgorithm::detect` inspects (Zstd/Brotli
        // magics are 4 bytes; Zlib and Store need fewer).
        let mut peek = [0u8; 4];
        let mut peek_len = 0usize;
        while peek_len < peek.len() {
            match reader.read(&mut peek[peek_len..]) {
                Ok(0) => break,
                Ok(n) => peek_len += n,
                Err(e) => {
                    return Err(CompressionError::decompression_failed(format!(
                        "stream read: {e}"
                    )));
                }
            }
        }
        let peeked = &peek[..peek_len];

        // DC-7: the peek is exactly four bytes, which is exactly the MGEN magic,
        // so a sealed object is recognised here for free.
        //
        // It cannot then be streamed. AES-GCM authenticates with a tag at the
        // END of the message, so there is no honest way to emit a plaintext
        // prefix before the whole envelope has been read and verified —
        // "streaming" it would mean handing out bytes that might yet fail
        // authentication, which is the one thing this must never do. So an
        // encrypted object is buffered, opened, and only then fed back through
        // the ordinary codec path.
        //
        // That is a real cost and it is stated rather than hidden: with
        // encryption on, this function's constant-memory guarantee becomes
        // "constant per object" instead of "constant, full stop". Unencrypted
        // repos are unaffected — they never take this branch.
        if mediagit_security::envelope::is_sealed(peeked) {
            let mut sealed = peeked.to_vec();
            reader.read_to_end(&mut sealed).map_err(|e| {
                CompressionError::decompression_failed(format!("stream read (sealed): {e}"))
            })?;
            let plain = self.decompress_typed(&sealed)?;
            return sink.write_all(&plain).map_err(|e| {
                CompressionError::decompression_failed(format!("stream write (sealed): {e}"))
            });
        }

        // Store prefix check first, mirroring decompress_typed exactly.
        if peek_len > 0 && peeked[0] == 0x00 {
            let mut combined = Cursor::new(peeked[1..].to_vec()).chain(reader);
            return copy_streaming(&mut combined, &mut sink);
        }

        match CompressionAlgorithm::detect(peeked) {
            CompressionAlgorithm::Zlib => {
                let combined = Cursor::new(peeked.to_vec()).chain(reader);
                let mut dec = flate2::read::ZlibDecoder::new(combined);
                copy_streaming(&mut dec, &mut sink)
            }
            CompressionAlgorithm::Zstd => {
                let combined = Cursor::new(peeked.to_vec()).chain(reader);
                let mut dec = zstd::stream::read::Decoder::new(combined)
                    .map_err(|e| CompressionError::zstd_error(format!("stream init: {e}")))?;
                copy_streaming(&mut dec, &mut sink)
            }
            CompressionAlgorithm::Brotli => {
                // The 4-byte "BRT\x01" marker is a marker we add on top of the
                // real brotli stream (see BrotliCompressor::compress), not part
                // of it — `detect` only returns Brotli once all 4 marker bytes
                // are in `peeked`, so `reader` now starts exactly at the real
                // payload; nothing to re-inject.
                let mut dec = brotli::Decompressor::new(reader, 4096);
                copy_streaming(&mut dec, &mut sink)
            }
            CompressionAlgorithm::None => {
                let mut combined = Cursor::new(peeked.to_vec()).chain(reader);
                copy_streaming(&mut combined, &mut sink)
            }
        }
    }
}

/// Copy every byte from `r` to `w`, in fixed-size chunks, until EOF.
fn copy_streaming(r: &mut impl Read, w: &mut impl Write) -> CompressionResult<()> {
    let mut buf = [0u8; 65536];
    loop {
        let n = r
            .read(&mut buf)
            .map_err(|e| CompressionError::decompression_failed(format!("stream decode: {e}")))?;
        if n == 0 {
            return Ok(());
        }
        w.write_all(&buf[..n])
            .map_err(|e| CompressionError::decompression_failed(format!("stream sink: {e}")))?;
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

    fn compress_typed_with_size(
        &self,
        data: &[u8],
        obj_type: ObjectType,
    ) -> CompressionResult<Vec<u8>> {
        let strategy = if obj_type == ObjectType::Unknown {
            // For Unknown types, use entropy analysis to pick a smarter strategy.
            // Sample at most 64KB to bound CPU cost on large files.
            let sample = &data[..data.len().min(65_536)];
            let entropy = crate::calculate_entropy(sample);
            let entropy_class = crate::EntropyClass::classify(entropy);

            match entropy_class {
                crate::EntropyClass::High => CompressionStrategy::Store,
                crate::EntropyClass::VeryLow | crate::EntropyClass::Low => {
                    CompressionStrategy::Zstd(CompressionLevel::Default)
                }
                crate::EntropyClass::Medium => CompressionStrategy::Zstd(CompressionLevel::Default),
            }
        } else {
            self.strategy_for_type_with_size(obj_type, data.len())
        };
        self.compress_with_strategy(data, strategy)
    }

    fn decompress_typed(&self, data: &[u8]) -> CompressionResult<Vec<u8>> {
        // DC-7: unwrap the MGEN envelope BEFORE any sniffing. AES-GCM output is
        // indistinguishable from random, so there is no leading byte the
        // detector below could correctly interpret — it would pick whichever
        // codec the first random byte resembled. Borrowed when there is no
        // envelope, so an unencrypted repo pays one 4-byte comparison.
        let unsealed = self.unseal(data)?;
        let data: &[u8] = &unsealed;

        // Auto-detect compression algorithm
        use crate::CompressionAlgorithm;

        // Store mode magic byte (0x00), written by BOTH store paths in
        // compress_with_strategy. Stripped unconditionally: no codec we emit can start
        // with 0x00 (zlib = 0x78, zstd = 0x28, brotli = "BRT"), so a leading 0x00 is
        // always the Store prefix and never payload.
        //
        // This used to strip only when the remaining bytes looked uncompressed, which
        // silently corrupted every stored object whose raw content happened to begin
        // with a codec magic - e.g. 0x78 0xF9, a valid zlib header. Such an object read
        // back one byte too long, failed its oid check, and became permanently
        // unreadable (~1 in 8000 incompressible objects).
        if !data.is_empty() && data[0] == 0x00 {
            return Ok(data[1..].to_vec());
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
                Ok(self
                    .zstd_default
                    .decompress(data)
                    .unwrap_or_else(|_| data.to_vec()))
            }
            CompressionAlgorithm::Brotli => {
                // False positive rare but possible: raw data starting with "BRT\x01".
                // Fall back to raw data if decompression fails.
                Ok(self
                    .brotli_best
                    .decompress(data)
                    .unwrap_or_else(|_| data.to_vec()))
            }
        }
    }

    fn strategy_for_type(&self, obj_type: ObjectType) -> CompressionStrategy {
        CompressionStrategy::for_object_type(obj_type)
    }

    fn strategy_for_type_with_size(
        &self,
        obj_type: ObjectType,
        data_size: usize,
    ) -> CompressionStrategy {
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
#[allow(clippy::unwrap_used)]
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
        assert_eq!(
            ObjectType::from_magic_bytes(unknown_data),
            ObjectType::Unknown
        );
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

        // Text → Brotli Default (best ratio for structured text; large files fall back to Zstd)
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

        // Unknown uses entropy-adaptive strategy (low-entropy data → Zstd Default)
        assert!(compressed.len() < data.len());

        let decompressed = compressor.decompress_typed(&compressed).unwrap();
        assert_eq!(decompressed, data);
    }

    #[test]
    fn test_unknown_type_entropy_fallback() {
        let compressor = SmartCompressor::new();

        // High entropy (pseudo-random) → Store (0x00 prefix)
        let high_entropy: Vec<u8> = (0..10000).map(|i| ((i * 7 + 13) % 256) as u8).collect();
        let compressed = compressor
            .compress_typed_with_size(&high_entropy, ObjectType::Unknown)
            .unwrap();
        assert_eq!(compressed[0], 0x00);
        assert_eq!(&compressed[1..], &high_entropy[..]);

        // Low entropy (repetitive) → Zstd Default (unknown type, Brotli's text advantage absent)
        let low_entropy = b"aaaa".repeat(5000);
        let compressed = compressor
            .compress_typed_with_size(&low_entropy, ObjectType::Unknown)
            .unwrap();
        assert!(compressed.len() < low_entropy.len());
        let decompressed = compressor.decompress_typed(&compressed).unwrap();
        assert_eq!(decompressed, low_entropy);

        // Medium entropy → Zstd Default (compresses and roundtrips)
        let medium_entropy: Vec<u8> = (0..5000)
            .map(|i| {
                let base = (i % 64) as u8;
                base.wrapping_add((i / 64) as u8)
            })
            .collect();
        let compressed = compressor
            .compress_typed_with_size(&medium_entropy, ObjectType::Unknown)
            .unwrap();
        let decompressed = compressor.decompress_typed(&compressed).unwrap();
        assert_eq!(decompressed, medium_entropy);
    }

    #[test]
    fn test_from_magic_bytes_riff_dispatcher() {
        // WebP (existing behavior preserved)
        let webp = b"RIFF\x00\x00\x00\x00WEBP";
        assert_eq!(ObjectType::from_magic_bytes(webp), ObjectType::Webp);

        // WAV
        let wav = b"RIFF\x00\x00\x00\x00WAVE";
        assert_eq!(ObjectType::from_magic_bytes(wav), ObjectType::Wav);

        // AVI
        let avi = b"RIFF\x00\x00\x00\x00AVI ";
        assert_eq!(ObjectType::from_magic_bytes(avi), ObjectType::Avi);

        // Unknown RIFF subtype
        let unknown_riff = b"RIFF\x00\x00\x00\x00XXXX";
        assert_eq!(
            ObjectType::from_magic_bytes(unknown_riff),
            ObjectType::Unknown
        );
    }

    #[test]
    fn test_from_magic_bytes_new_formats() {
        // MKV/WebM (EBML)
        assert_eq!(
            ObjectType::from_magic_bytes(&[0x1A, 0x45, 0xDF, 0xA3, 0x00]),
            ObjectType::Mkv
        );

        // FLAC
        assert_eq!(ObjectType::from_magic_bytes(b"fLaC\x00"), ObjectType::Flac);

        // EXR
        assert_eq!(
            ObjectType::from_magic_bytes(&[0x76, 0x2F, 0x31, 0x01, 0x00]),
            ObjectType::Exr
        );

        // PSD
        assert_eq!(
            ObjectType::from_magic_bytes(b"8BPS\x00\x01"),
            ObjectType::AdobePhotoshop
        );

        // 7-Zip
        assert_eq!(
            ObjectType::from_magic_bytes(&[0x37, 0x7A, 0xBC, 0xAF, 0x27, 0x1C, 0x00]),
            ObjectType::SevenZ
        );

        // RAR
        assert_eq!(
            ObjectType::from_magic_bytes(&[0x52, 0x61, 0x72, 0x21, 0x1A, 0x07, 0x00]),
            ObjectType::Rar
        );

        // XZ
        assert_eq!(
            ObjectType::from_magic_bytes(&[0xFD, 0x37, 0x7A, 0x58, 0x5A, 0x00, 0x00]),
            ObjectType::Gz
        );

        // Bzip2
        assert_eq!(
            ObjectType::from_magic_bytes(&[0x42, 0x5A, 0x68, 0x39, 0x00]),
            ObjectType::Gz
        );

        // Zstd
        assert_eq!(
            ObjectType::from_magic_bytes(&[0x28, 0xB5, 0x2F, 0xFD, 0x00]),
            ObjectType::Gz
        );

        // LZ4
        assert_eq!(
            ObjectType::from_magic_bytes(&[0x04, 0x22, 0x4D, 0x18, 0x00]),
            ObjectType::Gz
        );

        // MP3 with ID3
        assert_eq!(
            ObjectType::from_magic_bytes(b"ID3\x04\x00"),
            ObjectType::Mp3
        );

        // MP3 sync word (must be >32 bytes)
        let mut mp3_sync = vec![0xFF, 0xFB];
        mp3_sync.extend(vec![0x00u8; 40]);
        assert_eq!(ObjectType::from_magic_bytes(&mp3_sync), ObjectType::Mp3);

        // MP3 sync word too short (<= 32 bytes) should NOT match
        let short_mp3 = [0xFF, 0xFB, 0x00, 0x00];
        assert_eq!(
            ObjectType::from_magic_bytes(&short_mp3),
            ObjectType::Unknown
        );
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
            ObjectType::Jpeg,
            ObjectType::Png,
            ObjectType::Gif,
            ObjectType::Webp,
            ObjectType::Avif,
            ObjectType::Heic,
            ObjectType::Tiff,
            ObjectType::Bmp,
            ObjectType::Raw,
            ObjectType::Exr,
            ObjectType::Hdr,
            // Video
            ObjectType::Mp4,
            ObjectType::Mov,
            ObjectType::Avi,
            ObjectType::Mkv,
            ObjectType::Webm,
            ObjectType::Flv,
            ObjectType::Wmv,
            ObjectType::Mpg,
            // Audio
            ObjectType::Mp3,
            ObjectType::Aac,
            ObjectType::Ogg,
            ObjectType::Opus,
            ObjectType::Flac,
            ObjectType::Wav,
            ObjectType::Aiff,
            ObjectType::Alac,
            // Documents
            ObjectType::Pdf,
            ObjectType::Svg,
            ObjectType::Eps,
            // Text
            ObjectType::Text,
            ObjectType::Json,
            ObjectType::Xml,
            ObjectType::Yaml,
            ObjectType::Toml,
            ObjectType::Csv,
            // Archives
            ObjectType::Zip,
            ObjectType::Tar,
            ObjectType::Gz,
            ObjectType::SevenZ,
            ObjectType::Rar,
            // Creative projects (sample)
            ObjectType::AdobePhotoshop,
            ObjectType::Blender,
            // Office (sample)
            ObjectType::WordDocument,
            // ML specialized (sample)
            ObjectType::MlCheckpoint,
            ObjectType::MlInference,
            // Git
            ObjectType::GitBlob,
            ObjectType::GitTree,
            ObjectType::GitCommit,
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

        let jpeg_result = compressor
            .compress_typed(&content, ObjectType::Jpeg)
            .unwrap();
        let text_result = compressor
            .compress_typed(&content, ObjectType::Text)
            .unwrap();
        let tiff_result = compressor
            .compress_typed(&content, ObjectType::Tiff)
            .unwrap();

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
        assert_eq!(
            ObjectType::from_extension("psd"),
            ObjectType::AdobePhotoshop
        );
        assert_eq!(
            ObjectType::from_extension("psb"),
            ObjectType::AdobePhotoshop
        );
        assert_eq!(
            ObjectType::from_extension("ai"),
            ObjectType::AdobeIllustrator
        );
        assert_eq!(
            ObjectType::from_extension("indd"),
            ObjectType::AdobeIndesign
        );
        assert_eq!(
            ObjectType::from_extension("aep"),
            ObjectType::AdobeAfterEffects
        );
        assert_eq!(
            ObjectType::from_extension("prproj"),
            ObjectType::AdobePremiere
        );

        // Video NLEs
        assert_eq!(
            ObjectType::from_extension("drp"),
            ObjectType::DavinciResolve
        );
        assert_eq!(
            ObjectType::from_extension("fcpbundle"),
            ObjectType::FinalCutPro
        );
        assert_eq!(
            ObjectType::from_extension("avb"),
            ObjectType::AvidMediaComposer
        );

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
        assert_eq!(
            ObjectType::from_extension("unity"),
            ObjectType::UnityProject
        );
        assert_eq!(
            ObjectType::from_extension("uasset"),
            ObjectType::UnrealProject
        );
        assert_eq!(ObjectType::from_extension("tscn"), ObjectType::GodotProject);
    }

    #[test]
    fn test_office_document_extensions() {
        assert_eq!(ObjectType::from_extension("docx"), ObjectType::WordDocument);
        assert_eq!(ObjectType::from_extension("doc"), ObjectType::WordDocument);
        assert_eq!(
            ObjectType::from_extension("xlsx"),
            ObjectType::ExcelSpreadsheet
        );
        assert_eq!(
            ObjectType::from_extension("xls"),
            ObjectType::ExcelSpreadsheet
        );
        assert_eq!(
            ObjectType::from_extension("pptx"),
            ObjectType::PowerpointPresentation
        );
        assert_eq!(
            ObjectType::from_extension("ppt"),
            ObjectType::PowerpointPresentation
        );
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
        assert_eq!(
            ObjectType::from_extension("tflite"),
            ObjectType::MlInference
        );
        assert_eq!(
            ObjectType::from_extension("llamafile"),
            ObjectType::MlInference
        );
    }

    #[test]
    fn test_database_extensions() {
        assert_eq!(
            ObjectType::from_extension("sqlite"),
            ObjectType::SqliteDatabase
        );
        assert_eq!(ObjectType::from_extension("db"), ObjectType::SqliteDatabase);
        assert_eq!(
            ObjectType::from_extension("db3"),
            ObjectType::SqliteDatabase
        );
    }

    #[test]
    fn test_creative_project_categories() {
        assert_eq!(
            ObjectType::AdobePhotoshop.category(),
            ObjectCategory::CreativeProject
        );
        assert_eq!(
            ObjectType::Blender.category(),
            ObjectCategory::CreativeProject
        );
        assert_eq!(
            ObjectType::DavinciResolve.category(),
            ObjectCategory::CreativeProject
        );
        assert_eq!(
            ObjectType::ProTools.category(),
            ObjectCategory::CreativeProject
        );
        assert_eq!(
            ObjectType::AutoCad.category(),
            ObjectCategory::CreativeProject
        );
        assert_eq!(
            ObjectType::UnityProject.category(),
            ObjectCategory::CreativeProject
        );
    }

    #[test]
    fn test_office_category() {
        assert_eq!(ObjectType::WordDocument.category(), ObjectCategory::Office);
        assert_eq!(
            ObjectType::ExcelSpreadsheet.category(),
            ObjectCategory::Office
        );
        assert_eq!(
            ObjectType::PowerpointPresentation.category(),
            ObjectCategory::Office
        );
        assert_eq!(ObjectType::OpenDocument.category(), ObjectCategory::Office);
    }

    #[test]
    fn test_ml_specialized_category() {
        assert_eq!(
            ObjectType::MlCheckpoint.category(),
            ObjectCategory::MlSpecialized
        );
        assert_eq!(
            ObjectType::MlInference.category(),
            ObjectCategory::MlSpecialized
        );
    }

    #[test]
    fn test_database_category() {
        assert_eq!(
            ObjectType::SqliteDatabase.category(),
            ObjectCategory::Database
        );
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
        // Verify text/structured data uses Brotli (best ratio for known-text types)
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
        assert_eq!(
            ObjectType::from_extension("PSD"),
            ObjectType::AdobePhotoshop
        );
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
        let json_data =
            r#"{"name": "MediaGit", "version": "1.0", "features": ["compression", "delta"]}"#
                .repeat(50);
        let compressed = compressor
            .compress_typed(json_data.as_bytes(), ObjectType::Json)
            .unwrap();
        assert!(compressed.len() < json_data.len(), "JSON should compress");
        let decompressed = compressor.decompress_typed(&compressed).unwrap();
        assert_eq!(
            json_data.as_bytes(),
            &decompressed[..],
            "JSON roundtrip failed"
        );

        // Test CSV
        let csv_data = "id,name,value\n1,Alice,100\n2,Bob,200\n".repeat(100);
        let compressed = compressor
            .compress_typed(csv_data.as_bytes(), ObjectType::Csv)
            .unwrap();
        assert!(compressed.len() < csv_data.len(), "CSV should compress");
        let decompressed = compressor.decompress_typed(&compressed).unwrap();
        assert_eq!(
            csv_data.as_bytes(),
            &decompressed[..],
            "CSV roundtrip failed"
        );

        // Test XML
        let xml_data = "<root><item id=\"1\">Value</item></root>".repeat(50);
        let compressed = compressor
            .compress_typed(xml_data.as_bytes(), ObjectType::Xml)
            .unwrap();
        assert!(compressed.len() < xml_data.len(), "XML should compress");
        let decompressed = compressor.decompress_typed(&compressed).unwrap();
        assert_eq!(
            xml_data.as_bytes(),
            &decompressed[..],
            "XML roundtrip failed"
        );

        // Test plain text
        let text_data = "The quick brown fox jumps over the lazy dog. ".repeat(100);
        let compressed = compressor
            .compress_typed(text_data.as_bytes(), ObjectType::Text)
            .unwrap();
        assert!(compressed.len() < text_data.len(), "Text should compress");
        let decompressed = compressor.decompress_typed(&compressed).unwrap();
        assert_eq!(
            text_data.as_bytes(),
            &decompressed[..],
            "Text roundtrip failed"
        );
    }

    #[test]
    fn test_integration_creative_project_roundtrip() {
        // Test that creative project files use correct compression
        let compressor = SmartCompressor::new();

        // Simulate PSD file data (binary with some structure)
        let psd_data = vec![0x38, 0x42, 0x50, 0x53]; // "8BPS" header
        let mut data = psd_data.clone();
        data.extend_from_slice(&vec![0xAB; 10000]); // Add some data

        let compressed = compressor
            .compress_typed(&data, ObjectType::AdobePhotoshop)
            .unwrap();
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
        let checkpoint_compressed = compressor
            .compress_typed(&model_data, ObjectType::MlCheckpoint)
            .unwrap();
        let checkpoint_decompressed = compressor.decompress_typed(&checkpoint_compressed).unwrap();
        assert_eq!(
            model_data, checkpoint_decompressed,
            "Checkpoint roundtrip failed"
        );

        // Test inference model compression
        let inference_compressed = compressor
            .compress_typed(&model_data, ObjectType::MlInference)
            .unwrap();
        let inference_decompressed = compressor.decompress_typed(&inference_compressed).unwrap();
        assert_eq!(
            model_data, inference_decompressed,
            "Inference model roundtrip failed"
        );

        // Both should work, but inference might compress better (Default vs Fast)
        // We just verify both decompress correctly
    }

    #[test]
    fn test_integration_office_document_roundtrip() {
        // Test office documents (ZIP containers with XML)
        let compressor = SmartCompressor::new();

        // Simulate docx structure (ZIP-like)
        let docx_data = b"PK\x03\x04...document content...".repeat(100);

        let compressed = compressor
            .compress_typed(&docx_data, ObjectType::WordDocument)
            .unwrap();
        let decompressed = compressor.decompress_typed(&compressed).unwrap();
        assert_eq!(docx_data, &decompressed[..], "DOCX roundtrip failed");
    }

    #[test]
    fn test_integration_database_roundtrip() {
        // Test SQLite database compression
        let compressor = SmartCompressor::new();

        // Simulate SQLite data
        let db_data = b"SQLite format 3\x00...table data...".repeat(100);

        let compressed = compressor
            .compress_typed(&db_data, ObjectType::SqliteDatabase)
            .unwrap();
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
            ObjectType::Text,           // Brotli Default
            ObjectType::Json,           // Brotli Default
            ObjectType::AdobePhotoshop, // Zstd Default
            ObjectType::MlCheckpoint,   // Zstd Fast
            ObjectType::WordDocument,   // Store
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
        let jpeg_compressed = compressor
            .compress_typed(&jpeg_data, ObjectType::Jpeg)
            .unwrap();
        assert_eq!(jpeg_compressed[0], 0x00, "JPEG should have Store prefix");
        assert_eq!(
            &jpeg_compressed[1..],
            &jpeg_data[..],
            "JPEG should not be recompressed"
        );

        let mp4_compressed = compressor
            .compress_typed(mp4_data, ObjectType::Mp4)
            .unwrap();
        assert_eq!(mp4_compressed[0], 0x00, "MP4 should have Store prefix");
        assert_eq!(
            &mp4_compressed[1..],
            mp4_data,
            "MP4 should not be recompressed"
        );

        let zip_compressed = compressor
            .compress_typed(&zip_data, ObjectType::Zip)
            .unwrap();
        assert_eq!(zip_compressed[0], 0x00, "ZIP should have Store prefix");
        assert_eq!(
            &zip_compressed[1..],
            &zip_data[..],
            "ZIP should not be recompressed"
        );
    }

    #[test]
    fn test_integration_compression_ratio_expectations() {
        // Test that compression ratios meet expectations for different types
        let compressor = SmartCompressor::new();

        // Highly repetitive text should compress very well with Brotli
        let repetitive_text = "AAAAAAAAAA".repeat(1000);
        let compressed = compressor
            .compress_typed(repetitive_text.as_bytes(), ObjectType::Text)
            .unwrap();
        let ratio = compressed.len() as f64 / repetitive_text.len() as f64;
        assert!(
            ratio < 0.1,
            "Repetitive text should compress to <10% with Brotli, got {:.2}%",
            ratio * 100.0
        );

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
            ObjectType::Text,           // Brotli Default
            ObjectType::Json,           // Brotli Default
            ObjectType::AdobePhotoshop, // Zstd Default
            ObjectType::MlCheckpoint,   // Zstd Fast
            ObjectType::Tiff,           // Zstd Best
            ObjectType::Jpeg,           // Store
        ];

        for obj_type in types {
            let compressed = compressor.compress_typed(empty, obj_type).unwrap();
            let decompressed = compressor.decompress_typed(&compressed).unwrap();
            assert_eq!(
                empty,
                &decompressed[..],
                "Empty data failed for {:?}",
                obj_type
            );
        }
    }

    #[test]
    fn test_integration_large_data_performance() {
        // Test that large files compress/decompress correctly
        let compressor = SmartCompressor::new();

        // 1MB of structured data
        let large_json = format!(
            r#"{{"data": [{}]}}"#,
            (0..10000)
                .map(|i| format!("{}", i))
                .collect::<Vec<_>>()
                .join(",")
        );

        let compressed = compressor
            .compress_typed(large_json.as_bytes(), ObjectType::Json)
            .unwrap();
        assert!(
            compressed.len() < large_json.len(),
            "Large JSON should compress"
        );

        let decompressed = compressor.decompress_typed(&compressed).unwrap();
        assert_eq!(
            large_json.as_bytes(),
            &decompressed[..],
            "Large JSON roundtrip failed"
        );

        // Verify significant compression for structured data
        let ratio = compressed.len() as f64 / large_json.len() as f64;
        assert!(
            ratio < 0.5,
            "Large JSON should compress to <50%, got {:.2}%",
            ratio * 100.0
        );
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
                detected_type, expected_type,
                "Extension '{}' should map to {:?}, got {:?}",
                ext, expected_type, detected_type
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
                category, expected_category,
                "{:?} should be in {:?} category",
                obj_type, expected_category
            );
        }
    }

    /// Stored (incompressible) data whose first bytes mimic a codec magic must still
    /// round-trip. These payloads previously came back with the 0x00 Store prefix still
    /// attached, so their oid check failed and the object was unreadable for good.
    #[test]
    fn store_roundtrip_survives_payloads_that_look_like_codec_magic() {
        let sc = SmartCompressor::new();
        // 0x78F9 and 0x78DA are valid zlib headers; the other two are the zstd frame
        // magic and our brotli marker. All four are real prefixes seen in stored data.
        let leaders: [&[u8]; 4] = [
            &[0x78, 0xF9],
            &[0x78, 0xDA],
            &[0x28, 0xB5, 0x2F, 0xFD],
            b"BRT\x01",
        ];

        for leader in leaders {
            // High-entropy tail so compression expands and the Store path is taken.
            let mut original = leader.to_vec();
            let mut x: u32 = 0x9E37_79B9;
            for _ in 0..4096 {
                x = x.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                original.push((x >> 24) as u8);
            }

            let stored = sc
                .compress_with_strategy(&original, CompressionStrategy::Store)
                .expect("store must not fail");
            assert_eq!(stored[0], 0x00, "store mode must write its magic byte");

            let back = sc
                .decompress_typed(&stored)
                .expect("decompress must not fail");
            assert_eq!(
                back, original,
                "payload starting {:02X?} did not round-trip",
                leader
            );
        }
    }

    /// `decompress_streaming` must agree with `decompress_typed` for every
    /// codec branch — Store, Zlib, Zstd, Brotli, and the "not recognized,
    /// pass through" case — since a divergence between them is exactly the
    /// bug class `verify_chunk_content` exists to prevent a second copy of.
    #[test]
    fn decompress_streaming_matches_decompress_typed_for_every_codec() {
        let sc = SmartCompressor::new();
        let content = b"the quick brown fox jumps over the lazy dog ".repeat(200);

        let cases: [(&str, Vec<u8>); 5] = [
            (
                "store",
                sc.compress_with_strategy(&content, CompressionStrategy::Store)
                    .unwrap(),
            ),
            (
                "zlib",
                ZlibCompressor::new(CompressionLevel::Default)
                    .compress(&content)
                    .unwrap(),
            ),
            ("zstd", sc.compress(&content).unwrap()),
            (
                "brotli",
                BrotliCompressor::new(CompressionLevel::Default)
                    .compress(&content)
                    .unwrap(),
            ),
            ("raw/unrecognized", content.clone()),
        ];

        for (label, compressed) in cases {
            let whole = sc
                .decompress_typed(&compressed)
                .unwrap_or_else(|e| panic!("{label}: decompress_typed failed: {e}"));

            let mut streamed = Vec::new();
            sc.decompress_streaming(Cursor::new(compressed.clone()), &mut streamed)
                .unwrap_or_else(|e| panic!("{label}: decompress_streaming failed: {e}"));

            assert_eq!(
                whole, streamed,
                "{label}: streaming output diverged from whole-buffer decompress_typed"
            );
            assert_eq!(
                streamed, content,
                "{label}: did not recover original content"
            );
        }
    }

    /// RED-verify: a corrupted zstd frame must surface as an `Err`, not
    /// silently produce wrong bytes — the caller (pack verification) relies
    /// on this to fail closed.
    #[test]
    fn decompress_streaming_corrupt_frame_is_err() {
        let sc = SmartCompressor::new();
        let mut corrupt = vec![0x28u8, 0xb5, 0x2f, 0xfd]; // zstd magic
        corrupt.extend_from_slice(&[0xFFu8; 64]); // garbage body

        let mut sink = Vec::new();
        let err = sc
            .decompress_streaming(Cursor::new(corrupt), &mut sink)
            .expect_err("corrupt zstd frame must return Err, not a wrong-bytes Ok");
        let msg = format!("{err}");
        assert!(
            msg.contains("decompression") || msg.contains("stream"),
            "error must be a decompression error, got: {msg}"
        );
    }

    // ---- DC-7: at-rest encryption ----

    fn enc_key() -> EncryptionKey {
        EncryptionKey::from_bytes(vec![7u8; 32]).unwrap()
    }

    /// THE format-freeze guarantee. `FORMATS.md` has been frozen since rc.1, and
    /// DC-7 is only additive if a repo with no key writes exactly what it wrote
    /// before. Asserted over every strategy and over content that trips the
    /// expand-to-Store fallback, because that path has its own return.
    #[test]
    fn without_a_key_output_is_byte_identical() {
        let plain = SmartCompressor::new();
        let cases: Vec<Vec<u8>> = vec![
            b"highly compressible text ".repeat(200),
            (0u8..=255).cycle().take(5000).collect(),
            vec![0x78, 0xF9, 0x00, 0x01, 0x02],
            vec![],
        ];
        for strategy in [
            CompressionStrategy::Store,
            CompressionStrategy::Zlib(CompressionLevel::Default),
            CompressionStrategy::Zstd(CompressionLevel::Default),
            CompressionStrategy::Brotli(CompressionLevel::Best),
        ] {
            for data in &cases {
                let out = plain.compress_with_strategy(data, strategy).unwrap();
                assert!(
                    !mediagit_security::envelope::is_sealed(&out),
                    "a compressor with no key must never emit an envelope"
                );
                // And the bytes still round-trip through the untouched path.
                assert_eq!(&plain.decompress_typed(&out).unwrap(), data);
            }
        }
    }

    #[test]
    fn with_a_key_every_write_is_sealed_and_round_trips() {
        let enc = SmartCompressor::new().with_key(enc_key());
        assert!(enc.is_encrypted());
        for strategy in [
            CompressionStrategy::Store,
            CompressionStrategy::Zstd(CompressionLevel::Default),
            CompressionStrategy::Brotli(CompressionLevel::Best),
        ] {
            // Incompressible content too, so the expand-to-Store fallback exit
            // is covered as well as the ordinary one.
            for data in [
                b"compressible ".repeat(300),
                (0u8..=255).cycle().take(777).collect(),
            ] {
                let out = enc.compress_with_strategy(&data, strategy).unwrap();
                assert!(
                    mediagit_security::envelope::is_sealed(&out),
                    "every exit of the compress sink must seal, including the                      expand-to-Store fallback"
                );
                assert_eq!(enc.decompress_typed(&out).unwrap(), data);
            }
        }
    }

    /// A repo that turns encryption on must keep reading what it wrote before.
    /// Without this, enabling the feature would orphan every existing object.
    #[test]
    fn a_keyed_compressor_still_reads_unencrypted_objects() {
        let plain = SmartCompressor::new();
        let data = b"written before encryption was switched on".repeat(20);
        let legacy = plain
            .compress_with_strategy(&data, CompressionStrategy::Zstd(CompressionLevel::Default))
            .unwrap();

        let enc = SmartCompressor::new().with_key(enc_key());
        assert_eq!(enc.decompress_typed(&legacy).unwrap(), data);
    }

    /// The opposite direction must NOT be tolerant. Handing ciphertext back
    /// would send it to the codec sniffer, which finds no magic it knows,
    /// classifies it as uncompressed and returns random bytes as content.
    #[test]
    fn a_sealed_object_without_a_key_is_an_error_not_garbage() {
        let enc = SmartCompressor::new().with_key(enc_key());
        let sealed = enc
            .compress_with_strategy(b"secret", CompressionStrategy::Store)
            .unwrap();

        let err = SmartCompressor::new()
            .decompress_typed(&sealed)
            .expect_err("a sealed object with no key must fail, never pass through");
        assert!(
            format!("{err}").contains("encryption key"),
            "the error must say what is missing, got: {err}"
        );
    }

    #[test]
    fn the_wrong_key_is_an_error_not_garbage() {
        let sealed = SmartCompressor::new()
            .with_key(EncryptionKey::from_bytes(vec![1u8; 32]).unwrap())
            .compress_with_strategy(b"secret", CompressionStrategy::Store)
            .unwrap();
        assert!(
            SmartCompressor::new()
                .with_key(EncryptionKey::from_bytes(vec![2u8; 32]).unwrap())
                .decompress_typed(&sealed)
                .is_err()
        );
    }

    /// The invariant the existing `decompress_streaming` tests defend, extended
    /// to sealed objects: the two read paths must agree, or an object written by
    /// one and read by the other is corrupt. This is the case where they are
    /// most likely to drift, because the streaming path has to abandon streaming
    /// to handle an envelope at all.
    #[test]
    fn both_read_paths_agree_on_a_sealed_object() {
        let enc = SmartCompressor::new().with_key(enc_key());
        let data = b"payload read two different ways".repeat(50);
        let sealed = enc
            .compress_with_strategy(&data, CompressionStrategy::Zstd(CompressionLevel::Default))
            .unwrap();

        let buffered = enc.decompress_typed(&sealed).unwrap();
        let mut streamed = Vec::new();
        enc.decompress_streaming(Cursor::new(sealed), &mut streamed)
            .unwrap();
        assert_eq!(buffered, streamed);
        assert_eq!(streamed, data);
    }

    #[test]
    fn the_streaming_path_also_fails_closed_without_a_key() {
        let sealed = SmartCompressor::new()
            .with_key(enc_key())
            .compress_with_strategy(b"secret", CompressionStrategy::Store)
            .unwrap();
        let mut sink = Vec::new();
        assert!(
            SmartCompressor::new()
                .decompress_streaming(Cursor::new(sealed), &mut sink)
                .is_err(),
            "the streaming path must fail closed exactly like the buffered one"
        );
        assert!(sink.is_empty(), "nothing must reach the sink on failure");
    }
}
