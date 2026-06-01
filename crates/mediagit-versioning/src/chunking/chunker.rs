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

impl ContentChunker {
    /// Create a new content chunker with the specified strategy
    pub fn new(strategy: ChunkStrategy) -> Self {
        Self { strategy }
    }

    /// Chunk data according to the configured strategy
    pub async fn chunk(&self, data: &[u8], filename: &str) -> Result<Vec<ContentChunk>> {
        match self.strategy {
            ChunkStrategy::Fixed { size } => self.chunk_fixed(data, size).await,
            ChunkStrategy::Rolling {
                avg_size,
                min_size,
                max_size,
            } => self.chunk_fastcdc(data, avg_size, min_size, max_size).await,
            ChunkStrategy::MediaAware => self.chunk_media_aware(data, filename).await,
        }
    }

    /// Chunk a file with streaming callback pattern (constant memory)
    ///
    /// This method processes large files without loading them entirely into memory.
    /// Each chunk is passed to the callback immediately and can be processed/stored,
    /// then dropped to free memory.
    ///
    /// # Arguments
    ///
    /// * `path` - Path to the file to chunk
    /// * `on_chunk` - Async callback called for each chunk. The chunk should be
    ///   processed (e.g., written to storage) within the callback.
    ///
    /// # Returns
    ///
    /// Vector of chunk OIDs (only the identifiers, not the chunk data)
    ///
    /// # Memory Usage
    ///
    /// Memory is bounded by the chunk size (~8MB max for TB+ files) regardless
    /// of total file size.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # use mediagit_versioning::chunking::{ContentChunker, ChunkStrategy};
    /// # async fn example() -> anyhow::Result<()> {
    /// let chunker = ContentChunker::new(ChunkStrategy::MediaAware);
    /// let chunk_oids = chunker.chunk_file_streaming(
    ///     "large_video.mp4",
    ///     |chunk| async move {
    ///         // Process chunk (e.g., write to storage)
    ///         println!("Got chunk: {} bytes", chunk.size);
    ///         Ok(())
    ///     }
    /// ).await?;
    /// println!("Created {} chunks", chunk_oids.len());
    /// # Ok(())
    /// # }
    /// ```
    pub async fn chunk_file_streaming<P, F, Fut>(
        &self,
        path: P,
        mut on_chunk: F,
    ) -> Result<Vec<Oid>>
    where
        P: AsRef<std::path::Path>,
        F: FnMut(ContentChunk) -> Fut,
        Fut: std::future::Future<Output = Result<()>>,
    {
        let path = path.as_ref();
        let file_size = std::fs::metadata(path)
            .map_err(|e| anyhow::anyhow!("Failed to get file metadata: {}", e))?
            .len();

        let mut chunk_oids = Vec::new();

        // Tier 1: Small files (< 10MB): Load into memory for fastest processing
        if file_size < 10 * 1024 * 1024 {
            let data = tokio::fs::read(path)
                .await
                .map_err(|e| anyhow::anyhow!("Failed to read file: {}", e))?;
            let filename = path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("unknown");

            let chunks = self.chunk(&data, filename).await?;
            for chunk in chunks {
                let oid = chunk.id;
                on_chunk(chunk).await?; // Process immediately
                chunk_oids.push(oid); // Only store the OID
            }
            return Ok(chunk_oids);
        }

        // Tier 2: Medium files (10-100MB): Load into memory but process with streaming callback
        // Still loads file, but processes chunks immediately to reduce peak memory
        if file_size < 100 * 1024 * 1024 {
            debug!(
                size = file_size,
                "Medium file: loading then streaming chunks"
            );
            let data = tokio::fs::read(path)
                .await
                .map_err(|e| anyhow::anyhow!("Failed to read file: {}", e))?;
            let filename = path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("unknown");

            // Use MediaAware chunking for better boundaries
            let chunks = self.chunk(&data, filename).await?;
            for chunk in chunks {
                let oid = chunk.id;
                on_chunk(chunk).await?; // Process immediately (allows GC of chunk data)
                chunk_oids.push(oid);
            }
            return Ok(chunk_oids);
        }

        // Large files: Stream with FastCDC content-defined chunking
        // FastCDC's StreamCDC uses gear table for O(1) boundary detection - fast enough for streaming
        let (avg_size, min_size, max_size) = get_chunk_params(file_size);
        info!(
            "FastCDC streaming: file_size={}, avg_chunk={}KB",
            file_size,
            avg_size / 1024
        );

        // Read file into memory for FastCDC (required by current API)
        // Note: FastCDC StreamCDC requires Read trait, but for truly streaming
        // we need to buffer chunks. Memory usage is bounded by max_chunk_size.
        let file =
            std::fs::File::open(path).map_err(|e| anyhow::anyhow!("Failed to open file: {}", e))?;

        // A10/XET: fastcdc::v2020::StreamCDC implements gear-hash cut-point skip
        // (advances min_size-window-1 bytes before testing the mask), matching
        // the XET/HuggingFace CDC optimization. No hand-rolled skip needed.
        let chunker =
            fastcdc::v2020::StreamCDC::new(file, min_size as u32, avg_size as u32, max_size as u32);

        for result in chunker {
            let entry = result.map_err(|e| anyhow::anyhow!("FastCDC stream error: {}", e))?;
            let id = Oid::hash(&entry.data);

            let chunk = ContentChunk {
                id,
                data: entry.data,
                offset: entry.offset,
                size: entry.length,
                chunk_type: ChunkType::Generic,
                perceptual_hash: None,
                codec_hint: CodecHint::Unknown,
            };

            on_chunk(chunk).await?;
            chunk_oids.push(id);
        }

        info!(
            "FastCDC streaming complete: {} chunks created",
            chunk_oids.len()
        );
        Ok(chunk_oids)
    }

    /// Collect chunks from a file synchronously, sending each through a `tokio::sync::mpsc` channel.
    ///
    /// Designed for use inside `tokio::task::spawn_blocking` to avoid blocking the tokio
    /// executor with synchronous FastCDC file I/O.  The caller should spawn a blocking task,
    /// then receive from the corresponding `tokio::sync::mpsc::Receiver` in async context.
    ///
    /// Uses `memmap2` to memory-map the file so the existing format-aware parsers
    /// (`chunk_mp4`, `chunk_matroska`, `chunk_avi`) can operate on files of any size,
    /// not just those below the 100 MB in-memory threshold.  Falls back to `StreamCDC`
    /// when mmap is unavailable (network filesystems, permission issues, 32-bit targets
    /// with very large files) or when format parsing returns an error.
    ///
    /// # Errors
    /// Returns an error if the file cannot be opened, read, or if the receiver has been dropped.
    pub fn collect_file_chunks_blocking<P: AsRef<std::path::Path>>(
        &self,
        path: P,
        sender: tokio::sync::mpsc::Sender<ContentChunk>,
    ) -> anyhow::Result<()> {
        let path = path.as_ref();
        let file_size = std::fs::metadata(path)
            .map_err(|e| {
                anyhow::anyhow!("Failed to read file metadata '{}': {}", path.display(), e)
            })?
            .len();

        if file_size == 0 {
            return Ok(());
        }

        let filename = path.file_name().and_then(|n| n.to_str()).unwrap_or("");

        // Only use mmap + format-aware chunking for extensions that have a dedicated
        // structure-aware parser (container formats where splitting at structural
        // boundaries genuinely improves deduplication).  All other files go straight
        // to StreamCDC so that chunk boundaries stay content-defined and
        // delta-compressible — matching the behaviour of v0.2.5-beta.1.
        let has_structure_parser = {
            let ext = std::path::Path::new(filename)
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("")
                .to_lowercase();
            matches!(
                ext.as_str(),
                // Video containers with dedicated parsers
                "avi" | "riff" | "mp4" | "mov" | "m4v" | "m4a" | "3gp"
                    | "mkv" | "webm" | "mka" | "mk3d"
                    // 3D model containers with dedicated parsers
                    | "glb" | "gltf" | "obj" | "stl" | "ply" | "fbx"
            )
        };

        if has_structure_parser {
            let file = std::fs::File::open(path)
                .map_err(|e| anyhow::anyhow!("Failed to open file '{}': {}", path.display(), e))?;

            // Attempt memory-mapped I/O so the format-aware chunkers run on large files.
            // Safety: the file is opened read-only and the mapping is read-only.
            // On 32-bit targets, skip mmap for files whose size would overflow usize.
            #[cfg(target_pointer_width = "32")]
            let mmap_attempt: Option<memmap2::Mmap> = if file_size <= usize::MAX as u64 {
                unsafe { memmap2::Mmap::map(&file).ok() }
            } else {
                None
            };
            #[cfg(not(target_pointer_width = "32"))]
            let mmap_attempt: Option<memmap2::Mmap> = unsafe { memmap2::Mmap::map(&file).ok() };

            if let Some(mmap) = mmap_attempt {
                // Spin up a minimal current-thread tokio runtime.  We cannot reuse the
                // parent runtime because we're inside spawn_blocking.
                match tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                {
                    Ok(rt) => match rt.block_on(self.chunk(&mmap[..], filename)) {
                        Ok(chunks) => {
                            debug!(
                                path = %path.display(),
                                chunks = chunks.len(),
                                "mmap format-aware chunking complete"
                            );
                            for chunk in chunks {
                                sender.blocking_send(chunk).map_err(|_| {
                                    anyhow::anyhow!("Chunk worker channel closed unexpectedly")
                                })?;
                            }
                            return Ok(());
                        }
                        Err(e) => {
                            warn!(
                                path = %path.display(),
                                error = %e,
                                "Format-aware chunking failed, falling back to StreamCDC"
                            );
                        }
                    },
                    Err(e) => {
                        warn!(
                            "Failed to build tokio runtime for mmap chunking ({}), \
                             falling back to StreamCDC",
                            e
                        );
                    }
                }
            } else {
                debug!(path = %path.display(), "mmap unavailable for container format, using StreamCDC");
            }
        }

        // StreamCDC: content-defined chunking (constant memory, all formats).
        // This is the primary path for non-container files and the fallback for
        // container files when mmap or format-aware parsing fails.
        // A10/XET: cut-point skip is built into fastcdc::v2020::StreamCDC — no mask
        // test runs until min_size bytes have been consumed per chunk.
        let (avg_size, min_size, max_size) = get_chunk_params(file_size);
        let file = std::fs::File::open(path)
            .map_err(|e| anyhow::anyhow!("Failed to open file '{}': {}", path.display(), e))?;
        let stream_cdc =
            fastcdc::v2020::StreamCDC::new(file, min_size as u32, avg_size as u32, max_size as u32);

        for result in stream_cdc {
            let entry = result.map_err(|e| anyhow::anyhow!("FastCDC streaming error: {}", e))?;
            let id = Oid::hash(&entry.data);
            let chunk = ContentChunk {
                id,
                data: entry.data,
                offset: entry.offset,
                size: entry.length,
                chunk_type: ChunkType::Generic,
                perceptual_hash: None,
                codec_hint: CodecHint::Unknown,
            };
            sender
                .blocking_send(chunk)
                .map_err(|_| anyhow::anyhow!("Chunk worker channel closed unexpectedly"))?;
        }

        Ok(())
    }

    /// Fixed-size chunking
    pub(super) async fn chunk_fixed(
        &self,
        data: &[u8],
        chunk_size: usize,
    ) -> Result<Vec<ContentChunk>> {
        let mut chunks = Vec::new();
        let mut offset = 0u64;

        for chunk_data in data.chunks(chunk_size) {
            let id = Oid::hash(chunk_data);

            chunks.push(ContentChunk {
                id,
                data: chunk_data.to_vec(),
                offset,
                size: chunk_data.len(),
                chunk_type: ChunkType::Generic,
                perceptual_hash: None,
                codec_hint: CodecHint::Unknown,
            });

            offset += chunk_data.len() as u64;
        }

        debug!(
            chunks = chunks.len(),
            chunk_size = chunk_size,
            total_size = data.len(),
            "Fixed-size chunking complete"
        );

        Ok(chunks)
    }

    /// Content-defined chunking using FastCDC algorithm
    ///
    /// Uses gear table-based hashing for O(1) boundary detection per byte,
    /// approximately 10x faster than traditional rolling hash implementations.
    pub(super) async fn chunk_fastcdc(
        &self,
        data: &[u8],
        avg_size: usize,
        min_size: usize,
        max_size: usize,
    ) -> Result<Vec<ContentChunk>> {
        use fastcdc::v2020::FastCDC;

        let chunker = FastCDC::new(data, min_size as u32, avg_size as u32, max_size as u32);
        let mut chunks = Vec::new();

        for entry in chunker {
            let chunk_data = &data[entry.offset..entry.offset + entry.length];
            let id = Oid::hash(chunk_data);

            chunks.push(ContentChunk {
                id,
                data: chunk_data.to_vec(),
                offset: entry.offset as u64,
                size: entry.length,
                chunk_type: ChunkType::Generic,
                perceptual_hash: None,
                codec_hint: CodecHint::Unknown,
            });
        }

        debug!(
            chunks = chunks.len(),
            avg_size = avg_size,
            total_size = data.len(),
            "FastCDC chunking complete"
        );

        Ok(chunks)
    }

    /// Media-aware chunking (parse file structure)
    pub(super) async fn chunk_media_aware(
        &self,
        data: &[u8],
        filename: &str,
    ) -> Result<Vec<ContentChunk>> {
        // Detect file type from extension
        let extension = std::path::Path::new(filename)
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("");

        match extension.to_lowercase().as_str() {
            // Media - Structure-aware chunking
            "avi" | "riff" => self.chunk_avi(data).await,
            // WAV is RIFF-based audio but not interleaved A/V — use rolling CDC
            "wav" => {
                let (avg, min, max) = get_chunk_params(data.len() as u64);
                self.chunk_fastcdc(data, avg, min, max).await
            }
            "mp4" | "mov" | "m4v" | "m4a" | "3gp" => self.chunk_mp4(data).await,
            "mkv" | "webm" | "mka" | "mk3d" => self.chunk_matroska(data).await,
            "mpg" | "mpeg" | "vob" | "mts" | "m2ts" => {
                // MPEG Program/Transport Streams - use rolling CDC
                let (avg, min, max) = get_chunk_params(data.len() as u64);
                self.chunk_fastcdc(data, avg, min, max).await
            }

            // 3D Models - Structure-aware chunking
            "glb" | "gltf" => self.chunk_glb(data).await,
            "obj" | "stl" | "ply" => self.chunk_3d_text(data).await,
            "fbx" => self.chunk_fbx(data).await,
            "usd" | "usda" | "usdc" | "usdz" => {
                // USD ecosystem - rolling CDC for scene graph dedup
                let (avg, min, max) = get_chunk_params(data.len() as u64);
                self.chunk_fastcdc(data, avg, min, max).await
            }
            "abc" => {
                // Alembic cache - rolling CDC for animation dedup
                let (avg, min, max) = get_chunk_params(data.len() as u64);
                self.chunk_fastcdc(data, avg, min, max).await
            }
            "blend" | "max" | "ma" | "mb" | "c4d" | "hip" | "zpr" | "ztl" => {
                // Application-specific 3D formats - rolling CDC
                let (avg, min, max) = get_chunk_params(data.len() as u64);
                self.chunk_fastcdc(data, avg, min, max).await
            }

            // Text/Code - Rolling CDC for incremental dedup (Brotli compression)
            "csv" | "tsv" | "json" | "xml" | "html" | "txt" | "md" | "rst" | "rs" | "py" | "js"
            | "ts" | "go" | "java" | "c" | "cpp" | "h" | "yaml" | "yml" | "toml" | "ini"
            | "cfg" | "sql" | "graphql" | "proto" => {
                let (avg, min, max) = get_chunk_params(data.len() as u64);
                self.chunk_fastcdc(data, avg, min, max).await
            }

            // ML Data formats - Rolling CDC for dataset versioning
            "parquet" | "arrow" | "feather" | "orc" | "avro" | "hdf5" | "h5" | "nc" | "netcdf"
            | "npy" | "npz" | "tfrecords" | "petastorm" => {
                let (avg, min, max) = get_chunk_params(data.len() as u64);
                self.chunk_fastcdc(data, avg, min, max).await
            }

            // ML Models - Rolling CDC for fine-tuning dedup
            "pt" | "pth" | "ckpt" | "pb" | "safetensors" | "bin" | "pkl" | "joblib" => {
                let (avg, min, max) = get_chunk_params(data.len() as u64);
                self.chunk_fastcdc(data, avg, min, max).await
            }

            // ML Deployment - Rolling CDC for model versioning
            "onnx" | "gguf" | "ggml" | "tflite" | "mlmodel" | "coreml" | "keras" | "pte"
            | "mleap" | "pmml" | "llamafile" => {
                let (avg, min, max) = get_chunk_params(data.len() as u64);
                self.chunk_fastcdc(data, avg, min, max).await
            }

            // Documents - Rolling CDC with creative-container params for
            // formats with embedded compressed streams + trailing xref
            // (AI/PDF/EPS). Smaller chunks improve boundary re-sync after
            // insertions, unlocking dedup across versions. SVG is plain
            // text XML, so it uses generic tier params.
            "svg" => {
                let (avg, min, max) = get_chunk_params(data.len() as u64);
                self.chunk_fastcdc(data, avg, min, max).await
            }
            "pdf" | "eps" | "ai" | "psd" | "psb" => {
                let (avg, min, max) = get_creative_chunk_params(data.len() as u64);
                self.chunk_fastcdc(data, avg, min, max).await
            }

            // Design tools - creative containers (Fig/Sketch/XD/INDD) use
            // the same small-chunk params for cross-version dedup.
            "fig" | "sketch" | "xd" | "indd" | "indt" => {
                let (avg, min, max) = get_creative_chunk_params(data.len() as u64);
                self.chunk_fastcdc(data, avg, min, max).await
            }

            // Lossless audio - Rolling CDC
            "flac" | "aiff" | "alac" => {
                let (avg, min, max) = get_chunk_params(data.len() as u64);
                self.chunk_fastcdc(data, avg, min, max).await
            }

            // Compressed formats - Fixed chunking (already compressed, replaced entirely)
            "jpg" | "jpeg" | "png" | "gif" | "webp" | "avif" | "heic" | "mp3" | "aac" | "ogg"
            | "opus" | "zip" | "7z" | "rar" | "gz" | "xz" | "bz2" => {
                self.chunk_fixed(data, 4 * 1024 * 1024).await
            }

            // Unknown - Rolling CDC as safe default for dedup
            _ => {
                debug!(
                    extension = extension,
                    "Unknown type, using rolling (CDC) chunking for dedup"
                );
                let (avg, min, max) = get_chunk_params(data.len() as u64);
                self.chunk_fastcdc(data, avg, min, max).await
            }
        }
    }
}
