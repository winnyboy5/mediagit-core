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

### bsdiff (Delta Encoding)
- **Speed**: 50-100 MB/s encoding/decoding
- **Ratio**: 90%+ reduction for updated files
- **Use**: Large files with small changes

## Algorithm Selection

```rust
fn select_algorithm(path: &Path, size: u64) -> CompressionAlgorithm {
    match path.extension().and_then(|s| s.to_str()) {
        // Already compressed (store as-is)
        Some("mp4" | "mov" | "mkv" | "avi") => None,
        Some("jpg" | "jpeg" | "png" | "webp") => None,
        Some("mp3" | "aac" | "m4a" | "flac") => None,

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

| File Type | Size | Algorithm | Ratio | Time |
|-----------|------|-----------|-------|------|
| PSD (Photoshop) | 100 MB | zstd-3 | 2.8x | 1.2s |
| Blend (Blender) | 50 MB | zstd-3 | 3.1x | 0.6s |
| MP4 (Video) | 200 MB | none | 1.0x | 0.1s |
| TXT (Code) | 10 MB | brotli-6 | 12x | 2.5s |
| WAV (Audio) | 80 MB | zstd-3 | 1.9x | 0.9s |

## Configuration

### Repository-Level
```toml
# .mediagit/config
[compression]
algorithm = "zstd"
level = "default"
use-delta = true
delta-max-chain = 50
```

### Per-File Override
```toml
[compression.overrides]
"*.psd" = { algorithm = "zstd", level = "best" }
"*.txt" = { algorithm = "brotli", level = 6 }
"*.mp4" = { algorithm = "none" }
```

## Related Documentation

- [Delta Encoding](./delta-encoding.md)
- [Object Database (ODB)](./odb.md)
