# Performance Optimization

Practical tips for maximizing MediaGit throughput and minimizing storage costs.

## Parallel Add

The single biggest performance lever. By default, MediaGit uses all available CPU cores.

```bash
# Let MediaGit choose (default: all CPUs)
mediagit add assets/

# Explicit job count
mediagit add --jobs 16 assets/

# Disable parallelism (for debugging or resource-constrained systems)
mediagit add --no-parallel assets/
```

**Expected throughput** (validated benchmarks):
| File type | Sequential | Parallel (16 cores) |
|-----------|-----------|---------------------|
| PSD (71 MB) | ~2 MB/s | ~35 MB/s |
| MP4 (500 MB) | ~3 MB/s | ~20 MB/s |
| CSV/text | ~5 MB/s | ~50 MB/s |
| Pre-compressed (JPEG, zip) | ~80 MB/s | ~200 MB/s |

## Compression Strategy

MediaGit automatically selects the best compression strategy per file type. You can tune the global defaults:

```toml
# .mediagit/config.toml
[compression]
algorithm = "zstd"
level = 3      # 1 (fast) → 22 (best). Default 3 is optimal for most cases.
min_size = 1024  # Don't compress files smaller than 1 KB
```

### Format-Specific Behavior

MediaGit never wastes CPU re-compressing already-compressed formats:

| Format | Strategy | Reason |
|--------|----------|--------|
| JPEG, PNG, WebP | Store (level 0) | Already compressed |
| MP4, MOV, AVI | Store | Already compressed |
| ZIP, DOCX, XLSX | Store | ZIP container |
| PDF, AI, InDesign | Store | Contains compressed streams |
| PSD | Zstd Best | Raw layer data compresses well |
| OBJ, FBX, GLB, STL | Zstd Best | Binary 3D data |
| WAV, FLAC | Zstd Default | Uncompressed audio |
| Text, JSON, TOML | Zstd Default | Highly compressible |

### Delta Encoding

For versioned files that change incrementally (e.g., evolving PSD files), MediaGit uses delta encoding to store only the differences between versions:

```toml
# Similarity thresholds (in smart_compressor.rs — not yet configurable via TOML)
# AI/PDF files: 15% similarity → try delta encoding
# Office docs: 20% similarity → try delta encoding
# General: 80% similarity threshold
```

Delta chains are capped at depth 10 to prevent slow reads on deeply-chained objects.

## Chunking

Large files are split into chunks for efficient deduplication and parallel transfer. MediaGit uses different chunkers per file type:

| File size / type | Chunker | Typical chunk count |
|------------------|---------|---------------------|
| < 10 MB | FastCDC (small) | 2–10 |
| 10–100 MB | FastCDC (medium) | 10–100 |
| > 100 MB | StreamCDC | 100–2000 |
| MP4 / MKV / WebM | Video container-aware | 1 per GOP |
| WAV | Audio-aware | Fixed-size segments |
| PSD | Layer-aware | 1 per layer group |

**Deduplication**: Identical chunks across files or versions are stored only once. For a 6 GB CSV dataset, this yielded 83% storage savings in testing.

## Storage Backend Performance

Cloud backend upload speeds depend on network, not MediaGit:

| Backend | Upload | Download | Notes |
|---------|--------|----------|-------|
| Local filesystem | 200–500 MB/s | 200–500 MB/s | Limited by disk I/O |
| MinIO (local) | 100–300 MB/s | 200–500 MB/s | Validated: 108 MB/s upload |
| Amazon S3 | 50–200 MB/s | 100–400 MB/s | Depends on region + instance |
| Azure Blob | 50–150 MB/s | 100–300 MB/s | |
| Google Cloud Storage | 50–200 MB/s | 100–400 MB/s | |

### S3 Transfer Optimization

```toml
[performance]
max_concurrency = 32  # More parallel uploads

[performance.connection_pool]
max_connections = 32

[performance.timeouts]
request = 300  # 5 min for very large files
write = 120
```

## Memory Usage

Cache settings control how much object data MediaGit keeps in memory:

```toml
[performance.cache]
enabled = true
max_size = 1073741824  # 1 GB (for large repos)
ttl = 7200             # 2 hours
```

For workstations with < 8 GB RAM, reduce to 256 MB:
```toml
max_size = 268435456  # 256 MB
```

## Repository Maintenance

### Garbage Collection

Run after many branch deletions or partial operations:

```bash
mediagit gc
```

GC removes unreferenced objects and repacks data. Safe to run any time.

### Verify Integrity

```bash
# Quick check (metadata only)
mediagit fsck

# Full cryptographic verification
mediagit verify
```

### Statistics

```bash
mediagit stats
```

Shows compression ratio, deduplication rate, object count, and chunk distribution by file category.

## Profiling

For investigating performance bottlenecks in development:

```bash
# Enable trace-level logging
RUST_LOG=mediagit_versioning=trace mediagit add large-file.psd

# Benchmark specific operations
cargo bench --workspace -p mediagit-compression
```

## CI/CD Performance Tips

- **Cache the binary**: Download once, cache with `actions/cache`, skip re-download on subsequent runs
- **Parallel jobs**: Match `--jobs` to the CI runner's CPU count (`nproc` on Linux)
- **Avoid re-verifying in CI**: `mediagit fsck` is fast; `mediagit verify` does full SHA-256 re-reads and is slower
- **Use regional buckets**: Place S3 buckets in the same region as your CI runners

## See Also

- [Delta Compression Guide](./delta-compression.md)
- [Storage Backend Configuration](./storage-config.md)
- [Large File Optimization](../advanced/large-files.md)
