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

**Expected throughput** (validated benchmarks, release build):
| File type | Throughput | Notes |
|-----------|-----------|-------|
| PSD (72–181 MB) | 72–119 MB/s | Zstd Default; creative-container params |
| MP4/MOV (5–398 MB) | 146–174 MB/s | Pre-compressed; store-mode, zero CPU overhead |
| GLB (14–25 MB) | 3.0–5.2 MB/s | GLB parser + CDC chunking + Zstd |
| WAV (55–57 MB) | 2.1–3.6 MB/s | RIFF parser + chunking (CPU-bound) |
| Pre-compressed (JPEG, USDZ) | 25–182 MB/s | Direct write, no chunking |

## Compression Strategy

MediaGit automatically selects the best compression strategy per file type.
This is not tunable — there is no `[compression]` config section (one was
accepted until v0.4.0 but never read, and was removed in that release).

### Format-Specific Behavior

MediaGit never wastes CPU re-compressing already-compressed formats:

| Format | Strategy | Reason |
|--------|----------|--------|
| JPEG, PNG, WebP | Store | Already compressed |
| MP4, MOV, AVI | Store | Already compressed |
| ZIP, DOCX, XLSX | Store | ZIP container |
| AI, InDesign | Store | Contains compressed streams; re-zstd would expand it |
| PDF, SVG, PSD | Zstd Default | PDF/SVG are documents; PSD is a creative-container format handled at Default, not Best |
| TIFF, RAW, EXR (uncompressed images) | Zstd Best | Raw pixel data compresses well |
| OBJ, FBX, GLB, STL (3D interchange) | Zstd Best | Binary/text mesh data |
| WAV, AIFF | Zstd Best | Uncompressed PCM audio |
| FLAC, ALAC | Zstd Default | Already entropy-coded; Best over Default measured ~0.1% gain |
| Text, JSON, TOML | Brotli Default | Best ratio on structured text; falls back to Zstd above 500 MB |

### Delta Encoding

For versioned files that change incrementally (e.g., evolving PSD files), MediaGit uses delta encoding to store only the differences between versions:

```toml
# Similarity thresholds — NOT yet configurable via TOML.
# Set in crates/mediagit-versioning/src/similarity.rs (by file extension):
#   AI/PDF/PSD: 0.15   Office docs: 0.20   Images: 0.70   Text: 0.85   Config: 0.95
# and crates/mediagit-versioning/src/odb/mod.rs (by codec):
#   ProRes/DNxHR/J2K: 0.60   Subtitles/metadata: 0.90   Default: 0.80
```

Delta chains are capped at depth 10 to prevent slow reads on deeply-chained objects.

## Chunking

Large files are split into chunks for efficient deduplication and parallel transfer. MediaGit uses different chunkers per file type:

For formats without a dedicated parser, FastCDC's average chunk size scales
with file size:

| File size | Average chunk (range) |
|-----------|------------------------|
| < 100 MB | 1 MB (512 KB – 4 MB) |
| 100 MB – 10 GB | 2 MB (1 – 8 MB) |
| 10 GB – 100 GB | 4 MB (1 – 16 MB) |
| > 100 GB | 8 MB (1 – 32 MB) |

Formats with a dedicated structure-aware parser split at container
boundaries instead: MP4/MOV (atom/box-aware), MKV/WebM (Matroska
EBML-aware), AVI (RIFF-aware), GLB/glTF/OBJ/STL/PLY/FBX (3D-model
structure-aware), and Blender (`.blend`, BHEAD block walker). PSD does not
have a dedicated chunker — it uses generic FastCDC with small
creative-container params (1 MB avg / 512 KB–4 MB) for faster re-sync after
an embedded-stream shift.

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

```bash
MEDIAGIT_UPLOAD_CONCURRENCY=32    # parallel chunk uploads (default 32)
MEDIAGIT_DOWNLOAD_CONCURRENCY=32  # parallel chunk downloads (default 24)
MEDIAGIT_HTTP_POOL_MAX=64         # idle connections kept per host (default 64)
```

Equivalent `config.toml` keys exist for the first two
(`[performance] upload_concurrency` / `download_concurrency`). There is no
`max_concurrency` key and no `[performance.connection_pool]` section — both were
removed in v0.4.0 because nothing read them.

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

## Clone Behaviour

Clone downloads repository data and then materializes your working tree. Two
things about that are worth knowing.

### The working tree is written while media is still downloading

Small files are complete as soon as the initial transfer finishes, so MediaGit
writes them immediately instead of waiting for large media to finish arriving.
Only files backed by chunked media wait for their own chunks.

This is on by default. To turn it off — to compare timings, or to rule it out
while diagnosing something:

```bash
MEDIAGIT_CLONE_OVERLAP=0 mediagit clone http://server:3000/my-project
```

Both settings produce an identical working tree; only the order of writes
differs.

### An interrupted clone resumes

If a clone fails partway through, or you interrupt it with Ctrl-C, the partial
directory is **kept**. Re-run the same `clone` command against the same URL and
branch, and it skips everything already downloaded:

```bash
mediagit clone http://server:3000/my-project    # interrupted at 90%
mediagit clone http://server:3000/my-project    # resumes; re-downloads only what is missing
```

Details worth knowing:

- Resume works at chunk granularity. A chunk interrupted mid-download is
  re-fetched whole, not from a byte offset.
- A clone that fails during *setup* — bad URL, bad credentials, no such
  repository — still cleans up after itself. There is nothing to resume, and
  leaving a stub directory behind would just make the next attempt fail.
- MediaGit will only resume into a directory it can prove it created, and only
  for the same URL and branch. Any other existing directory is refused, as
  before. To start over, delete the directory.

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
- **Avoid re-verifying in CI**: `mediagit fsck` is fast; `mediagit verify` does full BLAKE3 re-reads and is slower
- **Use regional buckets**: Place S3 buckets in the same region as your CI runners

## See Also

- [Delta Compression Guide](./delta-compression.md)
- [Storage Backend Configuration](./storage-config.md)
- [Large File Optimization](../advanced/large-files.md)
