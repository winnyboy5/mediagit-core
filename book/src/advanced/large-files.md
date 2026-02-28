# Large File Optimization

Strategies for handling very large files — video masters, high-resolution image sequences, 3D scene files, and game assets.

## How MediaGit Handles Large Files

MediaGit automatically adapts its behavior based on file size:

| File size | Chunker | Typical chunks | Strategy |
|-----------|---------|----------------|----------|
| < 10 MB | FastCDC (small params) | 2–10 | Single-threaded |
| 10–100 MB | FastCDC (medium params) | 10–100 | Single-threaded |
| > 100 MB | StreamCDC | 100–2000 | Parallel workers |
| MP4 / MKV / WebM | Video container-aware | 1 per GOP | Per-format |
| PSD | Layer-aware | 1 per layer group | Per-format |
| WAV | Audio-aware | Fixed segments | Per-format |

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

**Expected throughput (validated benchmarks):**

| File type | Sequential | Parallel (16 cores) |
|-----------|-----------|---------------------|
| PSD (71 MB) | ~2 MB/s | ~35 MB/s |
| MP4 (500 MB) | ~3 MB/s | ~20 MB/s |
| Pre-compressed (JPEG) | ~80 MB/s | ~200 MB/s |

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
- StreamCDC (>100 MB files): 16–64 MB per chunk

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
mediagit gc --aggressive
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
