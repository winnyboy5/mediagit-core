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

## xdelta3 Algorithm

MediaGit uses xdelta3 for binary delta compression:
- **Efficiency**: 90%+ reduction for typical media workflows
- **Speed**: Fast reconstruction (50-100 MB/s)
- **Streaming**: No need to load full file into memory

### How It Works
1. Compare new version with base version
2. Identify identical blocks (using rolling hash)
3. Store only differences as delta instructions:
   - COPY blocks from base
   - INSERT new bytes

## Delta Chain Management

### Chain Depth
```
Base → Delta 1 → Delta 2 → Delta 3 → ... → Delta N
```

- **Default Max Depth**: 50
- **Reason**: Deeper chains = slower reconstruction
- **Solution**: `mediagit gc` creates new bases

### Optimal Chain Depth
Empirical analysis shows:
- **Depth 1-10**: Fast reconstruction, good savings
- **Depth 11-50**: Slower reconstruction, diminishing returns
- **Depth >50**: Unacceptably slow, poor user experience

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
fn should_use_delta(file: &Path, size: u64, previous_version: Option<Oid>) -> bool {
    // Must be large enough
    if size < 10_MB {
        return false;
    }

    // Must have previous version
    if previous_version.is_none() {
        return false;
    }

    // Check file type
    match file.extension() {
        // Already compressed
        Some("mp4" | "mov" | "jpg" | "png") => false,

        // Good delta candidates
        Some("psd" | "psb" | "blend" | "fbx" | "wav" | "aif") => true,

        // Default: try delta
        _ => true,
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
| **Uncompressed Images** | PSD, XCF, BLEND | 0.85 | Layer/material changes are localized |
| **Compressed Media** | MP4, MOV, JPEG | 0.95+ | Rarely edited, usually complete re-records |
| **Audio Files** | WAV, AIF, MP3 | 0.90 | Edits create new audio, not modifications |
| **Text/Code** | TXT, PY, JS, RS | 0.70 | Very effective delta compression |
| **3D Models** | FBX, OBJ | 0.75 | Vertex/animation changes |
| **Default** | Unknown types | 0.75 | Conservative middle ground |

**Lower threshold** = more aggressive compression (more files use delta)
**Higher threshold** = more conservative (only very similar files use delta)

### Similarity Configuration

Customize thresholds in `.mediagit/config`:

```toml
[compression.delta]
# Enable similarity detection (default: true)
auto_detect = true

# Minimum file size for delta consideration (default: 10MB)
min_size = "10MB"

# Minimum savings threshold (default: 10%, 0.1)
min_savings = 0.1

# Per-file-type similarity thresholds
[compression.delta.thresholds]
psd = 0.85        # Photoshop documents
blend = 0.85      # Blender projects
fbx = 0.75        # 3D models
wav = 0.90        # Audio files
mp4 = 0.95        # Compressed video
mov = 0.95        # QuickTime video
default = 0.75    # Unknown types
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
3. Generate delta using xdelta3
4. If delta smaller than full object, store delta
5. If delta larger, store full object (no benefit)

### Example
```rust
use xdelta3::encode;

let base = odb.read(base_oid)?;
let new_content = std::fs::read("large-file.psd")?;

let delta = encode(&base, &new_content)?;

if delta.len() < new_content.len() {
    odb.write_delta(base_oid, &delta)?;
} else {
    odb.write_blob(&new_content)?;
}
```

## Delta Reconstruction

### Process
1. Identify delta chain: target ← delta3 ← delta2 ← delta1 ← base
2. Read base object
3. Apply deltas in sequence
4. Verify final content hash

### Example
```rust
fn reconstruct(odb: &Odb, target_oid: Oid) -> Result<Vec<u8>> {
    let chain = odb.get_delta_chain(target_oid)?;

    let mut content = odb.read(chain.base)?;

    for delta_oid in chain.deltas {
        let delta = odb.read(delta_oid)?;
        content = xdelta3::decode(&content, &delta)?;
    }

    // Verify
    assert_eq!(sha256(&content), target_oid);
    Ok(content)
}
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
1. Identify long chains (depth >50)
2. Create new base from most recent version
3. Regenerate deltas from new base
4. Delete old chain

### Example
```
Before GC:
Base (v1) → Δ2 → Δ3 → ... → Δ52 (depth 51)

After GC:
Base (v52) → Δ1 → Δ2 → ... → Δ10 (depth 10, reversed)
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
