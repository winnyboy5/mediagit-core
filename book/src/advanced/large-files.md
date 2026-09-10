# Large File Optimization

Strategies for handling very large files — video masters, high-resolution image sequences, 3D scene files, and game assets.

## How MediaGit Handles Large Files

MediaGit automatically adapts its behavior based on file size and type. For
formats without a dedicated parser, content-defined chunking (FastCDC) scales
its average chunk size with the file size:

| File size | Average chunk (range) |
|-----------|------------------------|
| < 100 MB | 1 MB (512 KB – 4 MB) |
| 100 MB – 10 GB | 2 MB (1 – 8 MB) |
| 10 GB – 100 GB | 4 MB (1 – 16 MB) |
| > 100 GB | 8 MB (1 – 32 MB) |

Formats with a dedicated structure-aware parser split at container
boundaries instead of the generic size tiers above:

| Format | Chunker |
|--------|---------|
| MP4 / MOV / M4V | Atom/box-aware (splits around `moov`/`mdat`) |
| MKV / WebM | Matroska EBML-aware |
| AVI | RIFF-aware |
| GLB / glTF / OBJ / STL / PLY / FBX | 3D-model structure-aware |
| Blender (`.blend`) | BHEAD block walker |
| PSD / AI / PDF / EPS / INDD | Generic FastCDC, but with small creative-container params (1 MB avg / 512 KB–4 MB) so re-sync after an embedded-stream shift stays quick |

Structure-aware parsers fall back to plain FastCDC above
`MEDIAGIT_CONTAINER_CHUNK_CAP_MB` (default 100 MB) or if parsing fails.

No configuration is required. MediaGit detects file size and type automatically.

---

## Parallel Ingestion

The single most effective optimization for large files is parallelism. MediaGit uses all CPU cores by default:

```bash
# Default: uses all cores
mediagit add assets/

# Explicit job count (match to your I/O bandwidth, not just CPU count)
mediagit add --jobs 8 assets/

# Disable for debugging or resource-constrained systems
mediagit add --no-parallel assets/
```

**Expected throughput (release build, measured staging throughput):**

| File type | Throughput |
|-----------|-----------|
| Pre-compressed (MP4, MOV, JPEG, USDZ) | 25–240 MB/s — store-mode, zero CPU overhead |
| Compressible (PSD, TIFF, WAV) | 2–120 MB/s — Zstd compression + optional chunking |
| Chunked large files (GLB, FLAC, AI) | 1.9–5.2 MB/s — CDC chunking + delta encoding |

For very large files (10–100 GB), I/O tends to be the bottleneck rather than CPU. Use SSDs and tune `--jobs` to match your disk's sequential read throughput divided by average chunk size.

---

## Adding a 1 TB Media Collection

A 1 TB media collection with 16 CPU cores and an SSD:

```bash
# Time estimate: 33–105 minutes depending on content
mediagit add --jobs 16 /media/collection/
```

Progress is shown per-file and per-chunk. The parallel pipeline:

1. File-level: multiple files processed concurrently (bounded semaphore)
2. Chunk-level: each file's chunks compressed and stored in parallel (async-channel producer-consumer)

---

## Memory Usage for Large Files

Each worker holds one uncompressed chunk in memory. Chunk sizes are approximately:

- FastCDC medium: 4–32 MB per chunk
- StreamCDC (>100 MB files): 1–8 MB per chunk (adaptive by file size)

With `--jobs 16` and 32 MB average chunk size, expect ~512 MB peak memory during add.

Tune the object cache separately from worker memory:

```toml
[performance.cache]
max_size = 1073741824  # 1 GB — for repositories with many reads
```

Reduce if your system has less than 8 GB RAM:

```toml
[performance.cache]
max_size = 268435456  # 256 MB
```

---

## Cloud Backend Tips for Large Files

### S3 / MinIO

Increase connection pool and concurrency for large parallel uploads:

```toml
[performance]
max_concurrency = 32

[performance.connection_pool]
max_connections = 32

[performance.timeouts]
request = 300   # 5 minutes for very large chunks
write = 120
```

Use a bucket in the same region as your workstation or CI runner.

### Azure Blob

The Azure backend uses block upload for large objects. Increase timeout if uploads fail:

```toml
[performance.timeouts]
write = 120
```

### Local Filesystem

For local storage of very large repos, `sync = true` ensures data safety on crash at the cost of ~30% write throughput:

```toml
[storage]
backend = "filesystem"
base_path = "./data"
sync = false   # set true for critical data
```

---

## Delta Encoding for Large Files

MediaGit applies delta encoding when a new version of a file has chunks similar to the stored version. For large files, delta encoding can reduce storage from GB to MB per revision:

```
v1.psd: 500 MB (base)
v2.psd:  15 MB (delta — only changed layers stored)
v3.psd:   8 MB (delta — minor touch-up)
Total:  523 MB (vs 1,500 MB without delta)
```

Delta chains are capped at depth 10 to prevent slow reads. After 10 revisions, the next version is stored as a new base.

Run `mediagit gc` periodically to optimize chain depth:

```bash
mediagit gc
```

---

## Garbage Collection

After deleting branches or files containing large objects, run GC to reclaim storage:

```bash
mediagit gc
```

For maximum reclamation (slower):

```bash
mediagit gc --repack
```

---

## Integrity Verification

After adding very large files, verify chunk integrity:

```bash
# Quick checksum check
mediagit fsck

# Full chunk-level verification (slower)
mediagit verify --path /path/to/large-file.mov
```

---

## File Format Recommendations

| File type | Notes |
|-----------|-------|
| **MP4 / MOV / MKV** | Already compressed; stored as-is. Deduplication works at GOP level. |
| **JPEG / PNG / WebP** | Already compressed; stored as-is. No re-compression overhead. |
| **PSD / PSB** | Layer-aware chunking + Zstd compression. Excellent delta savings per revision. |
| **TIFF (uncompressed)** | Zstd compresses well. Large but effective delta encoding. |
| **EXR** | Typically compressed. Stored as-is. |
| **WAV / AIFF** | Audio-aware chunking. Zstd compresses ~40–60% on uncompressed audio. |
| **PDF / AI / InDesign** | PDF containers with internal compression; stored as-is. |
| **ZIP / DOCX / XLSX** | ZIP containers; stored as-is. |
| **3D (OBJ, FBX, GLB, STL)** | Binary 3D data; Zstd Best compression applied. |

---

## See Also

- [Performance Optimization](../guides/performance.md)
- [Delta Compression Guide](../guides/delta-compression.md)
- [Storage Backend Configuration](../guides/storage-config.md)
- [mediagit add](../cli/add.md)
- [mediagit gc](../cli/gc.md)
