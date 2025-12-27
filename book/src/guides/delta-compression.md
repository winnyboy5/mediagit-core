# Delta Compression Guide

Complete guide to understanding and optimizing delta compression in MediaGit.

## What is Delta Compression?

Delta compression stores only the differences between file versions instead of complete copies:

```
Traditional Storage:
  v1.psd: 500 MB
  v2.psd: 500 MB (full copy)
  v3.psd: 500 MB (full copy)
  Total: 1,500 MB

Delta Compression:
  v1.psd: 500 MB (base)
  v2.psd: 15 MB (delta from v1)
  v3.psd: 8 MB (delta from v2)
  Total: 523 MB (65% savings!)
```

## How MediaGit Applies Delta Compression

### Automatic Detection

MediaGit automatically applies delta compression based on:
1. **File size** - Must be >10MB
2. **File similarity** - Content similarity above threshold
3. **File type** - Media-aware thresholds
4. **Savings check** - Delta must be <90% of full size

### Similarity Thresholds by File Type

| File Type | Threshold | Behavior |
|-----------|-----------|----------|
| **PSD/PSB** (Photoshop) | 0.85 | Aggressive delta (layer changes) |
| **BLEND** (Blender) | 0.85 | Aggressive delta (material/vertex changes) |
| **FBX/OBJ** (3D Models) | 0.75 | Moderate delta (geometry changes) |
| **WAV/AIF** (Audio) | 0.90 | Conservative (audio usually rewritten) |
| **MP4/MOV** (Video) | 0.95+ | Very conservative (rarely delta) |
| **TXT/Code** | 0.70 | Very aggressive (excellent compression) |
| **Default** | 0.75 | Moderate (unknown types) |

**Lower threshold** = more files use delta compression
**Higher threshold** = only very similar files use delta

## Checking Delta Status

### Show Delta Information

```bash
# Show file storage info
$ mediagit show --stat large-file.psd

Object: 5891b5b522d5df086d...
Type: blob (delta)
Size: 15.3 MB (delta)
Base: a3c5d7e2f1b8c9a4d... (500 MB)
Compression ratio: 96.9%
Delta chain depth: 3
```

### List Objects with Delta Info

```bash
$ mediagit stats --verbose

Object database statistics:
  Total objects: 8,875
  Loose objects: 247
  Packed objects: 8,628

Delta statistics:
  Objects with deltas: 2,847 (32%)
  Average chain depth: 4.2
  Max chain depth: 12
  Total delta savings: 3.2 GB (78%)
```

## Configuring Delta Compression

### Global Configuration

Edit `.mediagit/config`:

```toml
[compression.delta]
# Enable automatic delta compression
enabled = true

# Minimum file size for delta consideration
min_size = "10MB"

# Minimum savings required (10% = 0.1)
min_savings = 0.1

# Maximum delta chain depth before creating new base
max_depth = 50

# Per-file-type similarity thresholds
[compression.delta.thresholds]
psd = 0.85        # Photoshop documents
psb = 0.85        # Large Photoshop documents
blend = 0.85      # Blender projects
fbx = 0.75        # FBX 3D models
obj = 0.75        # OBJ 3D models
wav = 0.90        # WAV audio
aif = 0.90        # AIF audio
mp4 = 0.95        # MP4 video (rarely delta)
mov = 0.95        # QuickTime video
default = 0.75    # Unknown file types
```

### Adjust Aggressiveness

```bash
# More aggressive (delta more files)
$ mediagit config set compression.delta.thresholds.default 0.65

# More conservative (fewer deltas, safer)
$ mediagit config set compression.delta.thresholds.default 0.85

# Disable delta for specific types
$ mediagit config set compression.delta.thresholds.mp4 1.0
```

### Override for Single File

```bash
# Force delta compression
$ mediagit add --force-delta large-file.blend

# Disable delta for this file
$ mediagit add --no-delta huge-video.mp4
```

## Optimizing Delta Chains

### Understanding Delta Chains

Delta chains form when multiple versions are stored:

```
Base (v1) → Δ2 → Δ3 → Δ4 → Δ5
```

**Reconstruction** requires applying all deltas in sequence:
- Chain depth 1-10: Fast reconstruction
- Chain depth 11-50: Acceptable performance
- Chain depth >50: Slow, should optimize

### Check Chain Depth

```bash
$ mediagit verify --check-deltas

Analyzing delta chains...

Long chains detected:
  assets/scene.blend: depth 52 (slow reconstruction)
  images/poster.psd: depth 48
  models/character.fbx: depth 45

Recommendation: Run 'mediagit gc --aggressive' to optimize chains
```

### Optimize Chains

```bash
# Standard GC (optimizes chains >50 depth)
$ mediagit gc

# Aggressive GC (optimizes chains >20 depth)
$ mediagit gc --aggressive

# Result:
Optimizing delta chains...
  Chains optimized: 23
  New bases created: 23
  Average depth reduced: 52 → 8
  Repository size: 485 MB → 467 MB
```

## Performance Tuning

### Parallel Delta Processing

```toml
[compression.delta.performance]
# Enable parallel delta encoding
parallel = true

# Number of threads (0 = auto-detect)
threads = 0

# Chunk size for large file delta
chunk_size = "64MB"
```

### Memory Limits

```toml
[compression.delta.memory]
# Maximum memory for delta buffers
max_buffer_size = "512MB"

# Stream large deltas (reduces memory)
streaming_threshold = "100MB"
```

## Troubleshooting

### Delta Compression Not Applied

**Check file size**:
```bash
$ ls -lh large-file.psd
-rw-r--r-- 1 user user 8.5M  # Too small (<10MB)
```
**Solution**: Delta only applies to files >10MB by default

**Check similarity**:
```bash
$ mediagit show --similarity large-file.psd
Previous version similarity: 0.42 (threshold: 0.85)
Reason: File significantly changed, delta not beneficial
```
**Solution**: File rewritten, delta won't help

### Slow Reconstruction

**Check delta chain depth**:
```bash
$ mediagit show large-file.psd
Delta chain depth: 87 (very deep!)
```

**Solution**: Optimize chains
```bash
$ mediagit gc --aggressive
```

### High Memory Usage

**Check delta streaming**:
```toml
[compression.delta.memory]
# Force streaming for large deltas
streaming_threshold = "50MB"  # Lower threshold
```

## Best Practices

### 1. Regular Garbage Collection

```bash
# Weekly maintenance
$ mediagit gc

# Monthly aggressive optimization
$ mediagit gc --aggressive
```

### 2. Tune for Your Workflow

**Photo/Design Work** (many small edits):
```toml
[compression.delta.thresholds]
psd = 0.80  # More aggressive
blend = 0.80
```

**Video/Audio** (large rewrites):
```toml
[compression.delta.thresholds]
mp4 = 1.0  # Disable delta
mov = 1.0
wav = 0.95  # Very conservative
```

### 3. Monitor Delta Effectiveness

```bash
# Check delta savings
$ mediagit stats --delta-report

Delta compression effectiveness:
  File type    | Files | Avg savings | Total saved
  -------------+-------+-------------+-------------
  PSD          | 1,247 | 92.3%       | 2.4 GB
  BLEND        |   389 | 88.7%       | 876 MB
  FBX          |   156 | 74.2%       | 234 MB
  WAV          |    89 | 45.1%       |  89 MB
  Other        |   203 | 67.8%       | 156 MB
  -------------+-------+-------------+-------------
  Total        | 2,084 | 85.4%       | 3.75 GB
```

### 4. Verify After Major Changes

```bash
# After configuration changes
$ mediagit verify --check-deltas

# Ensure chains are healthy
$ mediagit gc --verify
```

## Advanced Topics

### Custom Similarity Functions

For specific workflows, you can customize similarity detection (requires building from source):

```rust
// Custom similarity for your file type
fn custom_similarity(old: &[u8], new: &[u8]) -> f64 {
    // Your custom similarity logic
    // Return 0.0-1.0 (0 = completely different, 1 = identical)
}
```

### Delta Debugging

Enable detailed delta logging:
```bash
$ RUST_LOG=mediagit_compression::delta=debug mediagit add large-file.psd

DEBUG mediagit_compression::delta: Calculating similarity...
DEBUG mediagit_compression::delta: Similarity: 0.89 (threshold: 0.85) ✓
DEBUG mediagit_compression::delta: Generating delta...
DEBUG mediagit_compression::delta: Delta size: 15.3 MB (full: 500 MB)
DEBUG mediagit_compression::delta: Savings: 96.9% ✓ (min: 10%)
DEBUG mediagit_compression::delta: Delta compression applied
```

## Performance Benchmarks

### Delta Encoding Speed

| File Size | Encoding Time | Throughput |
|-----------|---------------|------------|
| 10 MB     | 0.1s         | 100 MB/s   |
| 100 MB    | 0.8s         | 125 MB/s   |
| 500 MB    | 4.2s         | 119 MB/s   |
| 1 GB      | 8.7s         | 115 MB/s   |

### Reconstruction Speed

| Chain Depth | File Size | Reconstruction Time |
|-------------|-----------|---------------------|
| 1-5         | 500 MB   | 0.5s (1000 MB/s)   |
| 6-10        | 500 MB   | 1.2s (416 MB/s)    |
| 11-20       | 500 MB   | 2.8s (178 MB/s)    |
| 21-50       | 500 MB   | 6.5s (77 MB/s)     |
| >50         | 500 MB   | 15s+ (33 MB/s)     | ← Optimize!

## Related Documentation

- [Delta Encoding Architecture](../architecture/delta-encoding.md)
- [Compression Strategy](../architecture/compression.md)
- [Garbage Collection](../cli/gc.md)
- [Performance Optimization](./performance.md)
