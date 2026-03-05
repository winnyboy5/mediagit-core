# Delta Encoding

Delta encoding stores only differences between file versions, dramatically reducing storage requirements for large binary files.

## Concept

Instead of storing full copies:
```
Version 1: 100 MB (full)
Version 2: 100 MB (full)
Version 3: 100 MB (full)
Total: 300 MB
```

Store base + deltas:
```
Version 1: 100 MB (base)
Version 2: 5 MB (delta from v1)
Version 3: 3 MB (delta from v2)
Total: 108 MB (64% reduction!)
```

## Delta Algorithm

MediaGit uses a **dual-layer** delta system:

### Layer 1: bsdiff (Whole-File Delta)
- **Crate**: `mediagit-compression` — uses `bsdiff::diff/patch` with zstd compression wrapper
- **When**: Applied at `add` time for whole-file delta against previous version
- **Efficiency**: 90%+ reduction for typical media workflows

### Layer 2: Sliding-Window (Chunk-Level Delta)
- **Crate**: `mediagit-versioning` — custom `DeltaEncoder` with Copy/Insert instructions
- **When**: Applied by the ODB at chunk level for similar chunks
- **Algorithm**: Hash-table based pattern matching with 32 KB sliding window

### How It Works
1. Compare new version with base version
2. Build hash table of base sequences (4-byte minimum match)
3. Scan target for matching sequences using the hash table
4. Store only differences as delta instructions:
   - **COPY**: reference offset + length from base
   - **INSERT**: literal new bytes

## Delta Chain Management

### Chain Depth
```
Base → Delta 1 → Delta 2 → Delta 3 → ... → Delta N
```

- **Default Max Depth**: 10 (`MAX_DELTA_DEPTH` in `odb.rs`)
- **Reason**: Deeper chains = slower reconstruction
- **Solution**: After depth 10, the next version is stored as a new full base

### Optimal Chain Depth
- **Depth 1-5**: Fast reconstruction, excellent savings
- **Depth 6-10**: Good reconstruction speed, good savings
- **Depth >10**: Automatically triggers new base creation

## When to Use Deltas

### Good Candidates
- Large files (>10 MB)
- Small changes between versions
- Frequently updated files

### Poor Candidates
- Already compressed media (MP4, JPG, PNG)
- Small files (<1 MB)
- Completely rewritten files

### Automatic Detection
```rust
// Simplified from add.rs — type-based eligibility
fn should_use_delta(filename: &str, data: &[u8]) -> bool {
    match ext.to_lowercase().as_str() {
        // Text-based: Always eligible
        "txt" | "md" | "json" | "xml" | "rs" | "py" | "js" => true,
        // Uncompressed media: Always eligible
        "psd" | "tiff" | "bmp" | "wav" | "aiff" => true,
        // Uncompressed video: Always eligible
        "avi" | "mov" => true,
        // Compressed video: Only for very large files
        "mp4" | "mkv" | "flv" => data.len() > 100_MB,
        // PDF/Creative: Only for large files
        "ai" | "indd" | "pdf" => data.len() > 50_MB,
        // 3D text formats: Always eligible
        "obj" | "gltf" | "ply" | "stl" => true,
        // 3D binary: Only for files >1MB
        "glb" | "fbx" | "blend" | "usd" => data.len() > 1_MB,
        // Compressed images/archives: Never
        "jpg" | "png" | "webp" | "zip" | "gz" => false,
        // Unknown: Only for large files
        _ => data.len() > 50_MB,
    }
}
```

## Similarity Detection

MediaGit automatically determines whether to apply delta compression based on file similarity:

### How It Works

```rust
// Pseudo-code for similarity decision
if file_size > minimum_threshold {
    similarity = calculate_content_similarity(previous_version, new_version);
    if similarity > type_specific_threshold {
        delta_size = encode_delta(previous_version, new_version);
        if delta_size < full_size * 0.9 {  // At least 10% savings
            use_delta_compression();
        } else {
            store_full_copy();  // Not worth it
        }
    }
}
```

### Configuration by File Type

MediaGit uses intelligent thresholds based on file characteristics:

| File Type | Example | Threshold | Rationale |
|-----------|---------|-----------|-----------|
| **Creative/PDF** | AI, InDesign, PDF | 0.15 | Compressed streams shift bytes; structural similarity remains |
| **Office** | DOCX, XLSX, PPTX | 0.20 | ZIP containers with shared structure |
| **Video** | MP4, MOV, AVI, MKV | 0.50 | Metadata/timeline changes significant |
| **Audio** | WAV, AIFF, MP3, FLAC | 0.65 | Medium threshold |
| **Images** | JPG, PNG, PSD | 0.70 | Perceptual similarity |
| **3D Models** | OBJ, FBX, BLEND, glTF, GLB | 0.70 | Vertex/animation changes |
| **Text/Code** | TXT, PY, RS, JS | 0.85 | Small changes matter |
| **Config** | JSON, YAML, TOML, XML | 0.95 | Exact matches preferred |
| **Default** | Unknown types | 0.30 | Global minimum (`MIN_SIMILARITY_THRESHOLD`) |

**Lower threshold** = more aggressive compression (more files use delta)
**Higher threshold** = more conservative (only very similar files use delta)

### Similarity Configuration

Customize thresholds in `.mediagit/config`:

```toml
[compression.delta]
# Enable similarity detection (default: true)
auto_detect = true

# Minimum savings threshold (default: 10%, 0.1)
min_savings = 0.1

# Per-file-type similarity thresholds (from similarity.rs)
[compression.delta.thresholds]
psd = 0.70        # Images (perceptual similarity)
blend = 0.70      # 3D models
fbx = 0.70        # 3D models
wav = 0.65        # Audio files
mp4 = 0.50        # Video (metadata changes)
mov = 0.50        # Video
ai = 0.15         # Creative/PDF containers
pdf = 0.15        # PDF containers
rs = 0.85         # Text/code
default = 0.30    # Global minimum
```

**Change similarity aggressiveness**:
```bash
# Be more aggressive (compress more files)
$ mediagit config set compression.delta.thresholds.default 0.65

# Be more conservative (fewer deltas, safer)
$ mediagit config set compression.delta.thresholds.default 0.85
```

**Disable delta for specific types**:
```bash
# Treat MP4s as already compressed (skip delta)
$ mediagit config set compression.delta.thresholds.mp4 1.0
```

### Similarity Detection Performance

The similarity checking process:

| File Size | Detection Time | Typical Savings |
|-----------|----------------|-----------------|
| 10 MB     | 0.1s          | 70-90%          |
| 100 MB    | 0.5s          | 80-95%          |
| 1 GB      | 2-3s          | 85-98%          |

**Trade-off**: Small detection cost for massive storage savings

## Delta Generation

### Process
1. Read base version from ODB
2. Read new version from working directory
3. Generate delta (bsdiff at file level, sliding-window at chunk level)
4. If delta < 80% of full object, store delta
5. If delta larger, store full object (no benefit)

### Example (Chunk-Level Delta in ODB)
```rust
// From odb.rs — chunk-level delta using sliding-window
let delta = DeltaEncoder::encode(&base_data, &chunk.data);
let delta_bytes = delta.to_bytes();
let delta_ratio = delta_bytes.len() as f64 / chunk.data.len() as f64;

if delta_ratio < 0.80 {
    // Store as delta (compressed with zstd)
    backend.put(&delta_key, &compressed_delta).await?;
} else {
    // Store full chunk
    backend.put(&chunk_key, &compressed_chunk).await?;
}
```

## Delta Reconstruction

### Process
1. Identify delta chain: target ← delta3 ← delta2 ← delta1 ← base
2. Read base object
3. Apply deltas in sequence
4. Verify final content hash

### Example (Chunk-Level Reconstruction)
```rust
// From odb.rs — reconstruct from sliding-window delta
let delta = Delta::from_bytes(&decompressed_delta)?;
let reconstructed = DeltaDecoder::apply(&base_data, &delta)?;

// Verify integrity
let actual_hash = sha256(&reconstructed);
assert_eq!(actual_hash, expected_hash);
```

## Performance Optimization

### Parallel Reconstruction
For multiple files:
```rust
let futures: Vec<_> = oids.iter()
    .map(|oid| reconstruct(odb, *oid))
    .collect();

let contents = futures::future::join_all(futures).await;
```

### Delta Chain Caching
- Cache intermediate reconstructions
- Speeds up repeated access to same chain
- LRU eviction policy

### Base Selection
Choose base to minimize average reconstruction time:
- Prefer recent versions as bases
- Avoid deep chains for frequently accessed versions
- `mediagit gc --optimize-deltas` reoptimizes chains

## Garbage Collection Integration

### Recompression
`mediagit gc` optimizes delta chains:
1. Identify long chains (depth >10)
2. Create new base from most recent version
3. Regenerate deltas from new base
4. Delete old chain

### Example
```
Before GC:
Base (v1) → Δ2 → Δ3 → ... → Δ12 (depth 11)

After GC:
Base (v12) → Δ1 → ... → Δ5 (depth 5, rebalanced)
```

## Storage Savings

### Typical Scenarios

**PSD Files** (Photoshop documents):
- Layer additions: 80-95% savings
- Small edits: 95-99% savings
- Complete redesign: 0-20% savings

**Blender Files** (3D scenes):
- Mesh tweaks: 85-95% savings
- Material changes: 90-98% savings
- New scene: 0-10% savings

**Audio Files** (WAV, AIF):
- Clip edits: 70-90% savings
- Effects applied: 50-80% savings
- Re-recording: 0-5% savings

## Related Documentation

- [Compression Strategy](./compression.md)
- [Object Database (ODB)](./odb.md)
- [Garbage Collection](../cli/gc.md)
