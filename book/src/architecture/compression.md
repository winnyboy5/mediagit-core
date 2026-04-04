# Compression Strategy

MediaGit employs intelligent compression based on file type and size to minimize storage while maintaining performance.

## Algorithms

### zstd (Default)
- **Speed**: 100-500 MB/s compression, 500-2000 MB/s decompression
- **Ratio**: 2-3x for binaries, 5-10x for text
- **Use**: Default for all file types

### brotli
- **Speed**: 10-50 MB/s compression, 200-400 MB/s decompression
- **Ratio**: 3-5x for binaries, 10-20x for text
- **Use**: Text and code files when size matters more than speed

### delta (Zstd Dictionary Delta Encoding)
- **Algorithm**: Zstd dictionary compression (chunk-level delta via `mediagit-versioning`)
- **How**: Base chunk serves as a raw zstd dictionary (level 19) to compress target chunk
- **Ratio**: 33–83% reduction for updated files (type-dependent; validated March 2026)
- **Use**: Large files with incremental changes

## Algorithm Selection

```rust
fn select_algorithm(path: &Path, size: u64) -> CompressionAlgorithm {
    match path.extension().and_then(|s| s.to_str()) {
        // Already compressed (store as-is)
        Some("mp4" | "mov" | "mkv" | "avi") => None,
        Some("jpg" | "jpeg" | "png" | "webp") => None,
        Some("mp3" | "aac" | "m4a") => None,

        // Lossless audio (zstd Best — uncompressed, good ratio)
        Some("flac" | "wav" | "aiff") => Zstd,

        // Text and code (brotli for better ratio)
        Some("txt" | "md" | "rs" | "py" | "js" | "ts") => Brotli,

        // Large binaries (zstd + delta)
        Some("psd" | "psb") if size > 10_MB => ZstdWithDelta,
        Some("blend") if size > 10_MB => ZstdWithDelta,
        Some("fbx" | "obj") if size > 10_MB => ZstdWithDelta,

        // Default
        _ => Zstd,
    }
}
```

## Compression Levels

### Fast (Level 1)
- **zstd**: ~150 MB/s, 2x ratio
- **Use**: Quick commits, local repos
- **Command**: `mediagit config compression.level fast`

### Default (Level 3)
- **zstd**: ~100 MB/s, 2.5x ratio
- **Use**: Balanced performance
- **Command**: `mediagit config compression.level default`

### Best (Level 19)
- **zstd**: ~10 MB/s, 3.5x ratio
- **Use**: Archival, cloud storage (bandwidth limited)
- **Command**: `mediagit config compression.level best`

## Performance Benchmarks

> Verified via standalone deep test suite (v0.2.6-beta.1, 2026-04-03).

| File Type | Size | Algorithm | Savings | Throughput |
|-----------|------|-----------|---------|------------|
| PSD-xl (Photoshop) | 213 MB | zstd-19 | 70.9% (3.44x) | 4.0 MB/s |
| FBX-ascii (3D) | 16 MB | zstd/brotli | 81.0% (5.27x) | 0.27 MB/s |
| DAE (Collada 3D) | 8.6 MB | zstd/brotli | 81.4% (5.37x) | 0.39 MB/s |
| SVG (Vector) | 496 KB | brotli | 80.8% (5.20x) | 1.90 MB/s |
| WAV (Uncompressed Audio) | 54 MB | zstd-19 | 54.1% (2.18x) | 1.04 MB/s |
| GLB (3D Binary) | 13 MB | zstd | 50.6% (2.03x) | 0.77 MB/s |
| MP4 (Video) | 4.9 MB | Store | 0% (1.00x) | 27 MB/s |
| FLAC (Audio) | 37 MB | Store | 0% (1.00x) | 1.28 MB/s |
| ZIP (Archive) | 656 MB | Store | 0% (1.00x) | 6.62 MB/s |

## Configuration

### Repository-Level
```toml
# .mediagit/config.toml
[compression]
algorithm = "zstd"
level = 3        # zstd: 1 (fastest) – 22 (best compression)
min_size = 1024  # bytes; files smaller than this skip compression
```

### Per-File Override
```toml
[compression.overrides]
"*.psd" = { algorithm = "zstd", level = 22 }
"*.txt" = { algorithm = "brotli", level = 6 }
"*.mp4" = { algorithm = "none" }
```

## Related Documentation

- [Delta Encoding](./delta-encoding.md)
- [Object Database (ODB)](./odb.md)
