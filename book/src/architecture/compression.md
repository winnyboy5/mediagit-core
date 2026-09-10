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

### zlib
- **Use**: Git-compatible object encoding (internal git-format compat objects only)

### delta (Zstd Dictionary Delta Encoding)
- **Algorithm**: Zstd dictionary compression (chunk-level delta via `mediagit-versioning`), applied by the ODB on top of the base algorithm above — not a member of the `CompressionAlgorithm` enum itself
- **How**: Base chunk serves as a raw zstd dictionary (level 19) to compress target chunk
- **Ratio**: 33–83% reduction for updated files (type-dependent; validated March 2026)
- **Use**: Large files with incremental changes

## Algorithm Selection

Selection is driven by detected object type (`CompressionStrategy::for_object_type` in `mediagit-compression`), not by file extension directly:

```rust
fn select_strategy(obj_type: ObjectType) -> CompressionStrategy {
    match obj_type {
        // Already compressed (store as-is)
        ObjectType::Mp4 | ObjectType::Mov | ObjectType::Mkv | ObjectType::Avi => Store,
        ObjectType::Jpeg | ObjectType::Png | ObjectType::Webp => Store,
        ObjectType::Mp3 | ObjectType::Aac => Store,

        // Uncompressed PCM audio: zstd Best
        ObjectType::Wav | ObjectType::Aiff => Zstd(Best),
        // Lossless-compressed audio (already entropy-coded; Best over Default
        // measured ~0.1% gain): cheap zstd Default only
        ObjectType::Flac => Zstd(Default),

        // Text and code (brotli for better ratio)
        ObjectType::Text | ObjectType::Json | ObjectType::Xml => Brotli(Default),

        // Creative project files (zstd + chunk-level delta applied separately by the ODB)
        ObjectType::AdobePhotoshop | ObjectType::Blender => Zstd(Default),

        // Default
        _ => Zstd(Default),
    }
}
```

## Compression Levels

`SmartCompressor` picks the level automatically per object type — Fast
(zstd 1 / brotli 4), Default (zstd 3 / brotli 9), or Best (zstd **19** /
brotli 11) — as shown in [Algorithm Selection](#algorithm-selection)
above. Best is 19, not 22: zstd's "ultra" levels 20-22 need ~1 GB per
compression context and OOM'd a deep-test FLAC add under parallel staging on a
16 GB machine (measured 2026-07-07), for under 0.5% extra ratio on media data
(`crates/mediagit-compression/src/lib.rs:126-135`). There is no config knob that changes this: `[compression]` in
`.mediagit/config.toml` is written by `mediagit init` for reference only and
is **not read at runtime** (see [Configuration Reference](../reference/config.md#compression--compression-settings-informational)).

### Fast (Level 1)
- **zstd**: ~150 MB/s, 2x ratio
- **Used for**: ML checkpoints (large, created frequently)

### Default (Level 3)
- **zstd**: ~100 MB/s, 2.5x ratio
- **Used for**: creative project files, unknown/binary fallback

### Best (Level 19)
- **zstd**: ~10 MB/s, 3.5x ratio
- **Used for**: uncompressed images (TIFF/RAW/EXR), 3D-model interchange formats
- Levels 20–22 ("ultra") are deliberately never used — they need ~1 GB per compression context and have caused OOM under parallel adds, for <0.5% extra ratio on media data

## Performance Benchmarks

> Verified via standalone deep test suite (v0.2.7-beta.1, 2026-04-03).

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

### Per-Algorithm Override
```toml
[compression.algorithms.zstd]
level = 19
```
`CompressionConfig` supports one override map keyed by algorithm name (`algorithms: HashMap<String, AlgorithmConfig>`) for tuning a given algorithm's level; it does not support per-file-glob overrides. File-type-specific algorithm selection is automatic (see [Algorithm Selection](#algorithm-selection) above) and not user-configurable per extension. Note: zstd levels above 19 ("ultra") are not used — they need ~1 GB per compression context and have caused OOM under parallel adds, for <0.5% extra ratio on media data.

## Related Documentation

- [Delta Encoding](./delta-encoding.md)
- [Object Database (ODB)](./odb.md)
