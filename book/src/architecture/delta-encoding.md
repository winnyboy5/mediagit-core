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

MediaGit uses **zstd dictionary compression** for chunk-level delta encoding:

### Zstd Dictionary Mode (Chunk-Level Delta)
- **Crate**: `mediagit-versioning` — `DeltaEncoder` using zstd dictionary compression
- **When**: Applied by the ODB at chunk level for similar chunks
- **Algorithm**: Base chunk serves as a raw zstd dictionary (level 19) to compress target chunk

### How It Works
1. Compare new chunk with similar base chunk (via `SimilarityDetector`)
2. Use base chunk as a zstd dictionary: `zstd::bulk::Compressor::with_dictionary(19, base)`
3. Compress target chunk against the dictionary
4. Store as wire format v2: `[0x5A, 0x44]` magic + varint(base_size) + varint(result_size) + zstd bytes

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
| **Creative/PDF** | AI, InDesign, PDF, **PSD/PSB** | 0.15 | Compressed streams shift bytes; structural similarity remains (PSD's embedded compressed image/mask streams put it in this bucket, not Images) |
| **Office** | DOCX, XLSX, PPTX | 0.20 | ZIP containers with shared structure |
| **Blender** | BLEND, BLEND1 | 0.40 | Binary scene files, heavy per-edit diffs |
| **Video** | MP4, MOV, AVI, MKV | 0.50 | Metadata/timeline changes significant |
| **Audio** | WAV, AIFF, MP3, FLAC | 0.65 | Medium threshold |
| **Images** | JPG, PNG | 0.70 | Perceptual similarity |
| **3D Models (interchange)** | OBJ, FBX, glTF, GLB | 0.70 | Vertex/animation changes |
| **Text/Code** | TXT, PY, RS, JS | 0.85 | Small changes matter |
| **Config** | JSON, YAML, TOML, XML | 0.95 | Exact matches preferred |
| **Default** | Unknown types | 0.30 | Global minimum (`MIN_SIMILARITY_THRESHOLD`) |

`similarity.rs` defines thresholds for many more type-specific buckets not shown above (Maya 0.50, Houdini 0.35, Cinema4D 0.40, NLE projects 0.25, DAW projects 0.55, CAD 0.45, Revit 0.30, game-engine assets 0.35–0.40, ML weights 0.15).

**Lower threshold** = more aggressive compression (more files use delta)
**Higher threshold** = more conservative (only very similar files use delta)

### Similarity Configuration

Per-file-type similarity thresholds are compiled-in constants in `get_similarity_threshold()` (`mediagit-versioning/src/similarity.rs`), keyed off file extension. There is no `mediagit config` CLI subcommand and no `[compression.delta.thresholds]` config section — thresholds are not currently user-configurable; changing aggressiveness requires editing `similarity.rs` and rebuilding.

### Similarity Detection Performance

The similarity checking process:

| File Size | Detection Time | Typical Savings |
|-----------|----------------|-----------------|
| 10 MB     | 0.1s          | 15–45%          |
| 100 MB    | 0.5s          | 20–50%          |
| 1 GB      | 2-3s          | 25–65%          |

**Trade-off**: Small detection cost for significant storage savings

## Delta Generation

### Process
1. Read base version from ODB
2. Read new version from working directory
3. Generate delta (zstd dictionary at chunk level)
4. If delta < 80% of full object, store delta
5. If delta larger, store full object (no benefit)

### Example (Chunk-Level Delta in ODB)
```rust
// From odb.rs — chunk-level delta using zstd dictionary
let delta = DeltaEncoder::encode(&base_data, &chunk.data)?;
let delta_bytes = delta.to_bytes();
let delta_ratio = delta_bytes.len() as f64 / chunk.data.len() as f64;

if delta_ratio < 0.80 {
    // Store as delta (already zstd-compressed via dictionary)
    backend.put(&delta_key, &delta_bytes).await?;
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
// From odb.rs — reconstruct from zstd dictionary delta
let delta = Delta::from_bytes(&delta_bytes)?;
let reconstructed = DeltaDecoder::apply(&base_data, &delta)?;

// Verify integrity
let actual_hash = blake3_hash(&reconstructed);
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
- Chains need no reoptimization command: `MAX_DELTA_DEPTH` (10) is enforced on write, so a chain that would exceed it gets a fresh base instead

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
- Layer additions: 35–65% savings
- Small edits: 37–64% savings (validated: 37% on 71 MB, 64% on 181 MB)
- Complete redesign: 0–10% savings

**3D Models** (GLB, FBX, STL, PLY, DAE):
- Mesh tweaks: 33–83% savings (validated: GLB 33–52%, FBX 47%, STL 70%, PLY 73%, DAE 83%)
- Material changes: 20–45% savings
- New scene: 0–10% savings

**Audio Files** (WAV, FLAC):
- Clip edits: 20–55% savings (validated: WAV 54%)
- Effects applied: 15–30% savings
- Re-recording: 0–5% savings

### Delta Efficiency Benchmarks (v0.2.7-beta.1)

> Measured via standalone deep test suite, 2026-04-03. All `fsck` verified.

| Format | Delta Efficiency | Overhead |
|--------|-----------------|----------|
| GLB (13–24MB) | 100% | 3–4 KB |
| AI-lg (123MB) | 100% | 4.5 KB |
| PSD-xl (213MB) | 99.8% | 424 KB |
| WAV (54MB) | 99.8% | 139 KB |
| ZIP (656MB) | 99.9% | 569 KB |
| FLAC (37MB) | 98.5% | 593 KB |
| FBX-ascii (16MB) | 99.4% | 100 KB |
| DAE (8.6MB) | 94.9% | 455 KB |

## Related Documentation

- [Compression Strategy](./compression.md)
- [Object Database (ODB)](./odb.md)
- [Garbage Collection](../cli/gc.md)
