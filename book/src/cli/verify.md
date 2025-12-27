# mediagit verify

Verify integrity of specific objects or files.

## Synopsis

```bash
mediagit verify [OPTIONS] <object>...
mediagit verify [OPTIONS] --path <file>...
```

## Description

Verifies the integrity of specific objects, files, or commits. Unlike `fsck` which checks the entire repository, `verify` focuses on individual objects or files for quick validation.

Useful for:
- Verifying specific media files after download
- Checking object integrity before important operations
- Validating chunks after recovery
- Confirming successful transfers
- Quick integrity spot-checks

## Options

### Verification Mode

#### `--full`
Perform complete verification including chunk integrity.

#### `--quick`
Quick checksum validation only (default).

#### `--deep`
Deep verification with decompression and content validation.

### Target Selection

#### `<object>...`
Verify specific object OIDs.

#### `--path <file>...`
Verify files in working tree.

#### `--staged`
Verify files in staging area.

#### `--commit <commit>`
Verify all objects in specified commit.

#### `--tree <tree>`
Verify all objects in specified tree.

### Output Options

#### `-v`, `--verbose`
Show detailed verification information.

#### `-q`, `--quiet`
Suppress output except errors.

#### `--json`
Output results in JSON format.

### MediaGit-Specific Options

#### `--verify-chunks`
Verify chunk integrity for large files.

#### `--verify-compression`
Validate compression integrity.

#### `--verify-metadata`
Check media metadata consistency.

#### `--parallel=<n>`
Number of parallel verification threads.

## Examples

### Verify specific object

```bash
$ mediagit verify a3c8f9d
Verifying object a3c8f9d...

Object: a3c8f9d2e1b4f6a8c5d7e9f1a2b3c4d5e6f7a8b9c0d1e2f3a4b5c6d7e8f9a0b1
Type: commit
Size: 2.3 KB
Checksum: ✓ Valid

Object verification: OK
```

### Verify multiple objects

```bash
$ mediagit verify a3c8f9d b4d7e1a c5e9f2b
Verifying 3 objects...

a3c8f9d (commit): ✓ Valid
b4d7e1a (blob): ✓ Valid
c5e9f2b (tree): ✓ Valid

All objects verified successfully
```

### Verify file in working tree

```bash
$ mediagit verify --path video.mp4
Verifying file: video.mp4

File information:
  Path: video.mp4
  Size: 245.8 MB
  Type: video/mp4

Object reference:
  OID: b4d7e1a9f2c3d4e5f6a7b8c9d0e1f2a3b4c5d6e7f8a9b0c1d2e3f4a5b6c7d8e9
  Stored size: 42.1 MB (compressed)
  Compression: 82.9%

Verification:
  Checksum: ✓ Valid
  Content match: ✓ Identical

File verification: OK
```

### Verify with chunks

```bash
$ mediagit verify --verify-chunks b4d7e1a
Verifying blob b4d7e1a with chunk integrity check...

Object information:
  Type: blob
  File: video.mp4
  Size: 245.8 MB
  Compressed: 42.1 MB

Chunk verification:
  Total chunks: 63
  Verifying checksums: 100% (63/63) ✓
  Verifying content: 100% (63/63) ✓
  Decompression test: ✓ All chunks valid

Chunk integrity: EXCELLENT
```

### Verify entire commit

```bash
$ mediagit verify --commit HEAD
Verifying commit HEAD (a3c8f9d)...

Commit verification:
  Commit object: ✓ Valid
  Tree: ✓ Valid
  Parents: ✓ Valid (1 parent)
  Author/Committer: ✓ Valid

Tree contents (247 objects):
  Checking trees: 100% (47/47) ✓
  Checking blobs: 100% (200/200) ✓

All objects in commit verified successfully
```

### Deep verification

```bash
$ mediagit verify --deep b4d7e1a
Deep verification of blob b4d7e1a...

Object validation:
  Checksum: ✓ Valid
  Size: ✓ Matches expected (245.8 MB)

Compression validation:
  Decompression test: ✓ Success
  Recompression test: ✓ Matches original
  Compression integrity: ✓ Perfect

Content validation:
  Format: video/mp4
  Codec: H.264/AVC
  Container: Valid MP4
  Media integrity: ✓ Playable

Deep verification: PASSED
```

### Verify staged files

```bash
$ mediagit verify --staged
Verifying staged files...

Staged files (5):
  video.mp4: ✓ Valid (245.8 MB → 42.1 MB)
  image.jpg: ✓ Valid (2.4 MB → 0.8 MB)
  audio.wav: ✓ Valid (45.2 MB → 8.7 MB)
  config.json: ✓ Valid (1.2 KB)
  README.md: ✓ Valid (4.5 KB)

All staged files verified
```

### Verify with metadata

```bash
$ mediagit verify --verify-metadata --path video.mp4
Verifying file: video.mp4 with metadata check

File verification:
  Checksum: ✓ Valid
  Size: ✓ Matches (245.8 MB)

Metadata verification:
  Format: MP4
  Video codec: H.264/AVC ✓
  Audio codec: AAC ✓
  Duration: 00:03:45 ✓
  Resolution: 1920x1080 ✓
  Frame rate: 29.97 fps ✓
  Bitrate: 8.5 Mbps ✓

Metadata stored vs actual:
  All metadata matches ✓
  No corruption detected ✓

Complete verification: PASSED
```

### Parallel verification

```bash
$ mediagit verify --parallel=8 --commit HEAD~5..HEAD
Verifying commits HEAD~5..HEAD with 8 threads...

Parallel verification (8 threads):
  Thread 1: 38 objects ✓
  Thread 2: 42 objects ✓
  Thread 3: 35 objects ✓
  Thread 4: 41 objects ✓
  Thread 5: 37 objects ✓
  Thread 6: 39 objects ✓
  Thread 7: 36 objects ✓
  Thread 8: 40 objects ✓

Total: 308 objects verified in 2.3s
All objects valid ✓
```

### Verbose output

```bash
$ mediagit verify -v b4d7e1a
Detailed verification of blob b4d7e1a...

Object Header:
  Type: blob
  Size: 245,787,392 bytes
  Compressed size: 42,143,128 bytes
  Compression ratio: 82.9%
  Compression algorithm: Zstd level 3

Checksum Verification:
  Algorithm: SHA-256
  Expected: b4d7e1a9f2c3d4e5f6a7b8c9d0e1f2a3b4c5d6e7f8a9b0c1d2e3f4a5b6c7d8e9
  Computed: b4d7e1a9f2c3d4e5f6a7b8c9d0e1f2a3b4c5d6e7f8a9b0c1d2e3f4a5b6c7d8e9
  Result: ✓ MATCH

Chunk Structure:
  Chunk size: 4 MB
  Total chunks: 63
  Last chunk: 2,787,392 bytes

Decompression Test:
  Decompressing: 100% (42.1 MB → 245.8 MB)
  Result: ✓ Success
  Time: 1.2s

Content Hash:
  Rehashing decompressed content...
  Hash: b4d7e1a9... ✓ Matches original

Verification: COMPLETE
Status: VALID
```

### JSON output

```bash
$ mediagit verify --json b4d7e1a
{
  "object": "b4d7e1a9f2c3d4e5f6a7b8c9d0e1f2a3b4c5d6e7f8a9b0c1d2e3f4a5b6c7d8e9",
  "type": "blob",
  "size": 245787392,
  "compressed_size": 42143128,
  "compression_ratio": 0.829,
  "checksum_valid": true,
  "decompression_valid": true,
  "chunks": {
    "total": 63,
    "verified": 63,
    "corrupted": 0
  },
  "status": "valid",
  "verification_time_ms": 1247
}
```

### Quiet mode

```bash
$ mediagit verify --quiet b4d7e1a c5e9f2b
$ echo $?
0  # Success (all valid)

$ mediagit verify --quiet corrupted-object
$ echo $?
1  # Failure (invalid object)
```

## Verification Levels

### Quick (Default)

```
Verification: Checksum only
Time: < 10ms per object
Use case: Fast spot-checks
```

### Full

```
Verification: Checksum + chunk integrity
Time: 50-100ms per MB
Use case: After downloads, before operations
```

### Deep

```
Verification: Full + decompression + content validation
Time: 100-200ms per MB
Use case: After recovery, corruption investigation
```

## Error Detection

### Corrupt Object

```bash
$ mediagit verify b4d7e1a
Verifying object b4d7e1a...

ERROR: Checksum mismatch
  Expected: b4d7e1a9f2c3d4e5f6a7b8c9d0e1f2a3b4c5d6e7f8a9b0c1d2e3f4a5b6c7d8e9
  Actual:   b4d7e1a9f2c3d4e5f6a7b8c9d0e1f2a3XXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXX

Object verification: FAILED
```

### Missing Chunks

```bash
$ mediagit verify --verify-chunks b4d7e1a
Verifying blob b4d7e1a with chunks...

ERROR: Missing chunks detected
  Total chunks: 63
  Present: 60
  Missing: 3 (chunks 23, 47, 58)

Chunk verification: FAILED
Missing data: 12 MB
```

### Compression Corruption

```bash
$ mediagit verify --verify-compression b4d7e1a
Verifying compression integrity...

ERROR: Decompression failed
  Chunk: 23
  Compressed size: 4,194,304 bytes
  Error: Invalid compressed data

Compression verification: FAILED
```

## Use Cases

### After Download

```bash
# Verify after pull
$ mediagit pull
$ mediagit verify --commit HEAD

# Verify specific large file
$ mediagit verify --path video.mp4 --verify-chunks
```

### Before Important Operations

```bash
# Before commit
$ mediagit verify --staged

# Before push
$ mediagit verify --commit HEAD
```

### Corruption Investigation

```bash
# Deep check of suspected file
$ mediagit verify --deep --verify-chunks video.mp4

# Verify entire branch
$ mediagit verify --commit main
```

### Batch Verification

```bash
# Verify all media files
$ find . -name "*.mp4" -exec mediagit verify --path {} \;

# Verify recent commits
$ mediagit verify --commit HEAD~10..HEAD
```

## Performance

### Quick Verification

```
Single object: < 10ms
100 objects: < 1s
1000 objects: < 10s
```

### Full Verification (with chunks)

```
Small file (<10 MB): 100-200ms
Medium file (10-100 MB): 1-5s
Large file (>100 MB): 5-20s
```

### Deep Verification

```
Small file: 200-500ms
Medium file: 2-10s
Large file: 10-60s
```

## Exit Status

- **0**: All objects valid
- **1**: One or more objects invalid or corrupted
- **2**: Invalid options or verification failed to run

## Configuration

```toml
[verify]
# Default verification level
level = "quick"  # quick | full | deep

# Verify chunks by default
verify_chunks = false

# Verify compression
verify_compression = false

# Verify metadata
verify_metadata = false

# Parallel threads
parallel = 4

# Show progress
show_progress = true
```

## Best Practices

### Regular Spot-Checks

```bash
# Daily verification of recent work
$ mediagit verify --commit HEAD~5..HEAD

# Weekly verification of large files
$ mediagit verify --verify-chunks --path *.mp4
```

### After Transfers

```bash
# After pull
$ mediagit pull
$ mediagit verify --commit FETCH_HEAD

# After clone
$ mediagit clone <url>
$ cd <repo>
$ mediagit verify --commit HEAD --verify-chunks
```

### Before Critical Operations

```bash
# Before backup
$ mediagit verify --commit HEAD --full
$ tar czf backup.tar.gz .mediagit/

# Before migration
$ mediagit verify --commit --all
$ mediagit push --mirror new-remote
```

## Troubleshooting

### Verification Fails

```bash
# Try deep verification for more info
$ mediagit verify --deep <object>

# Check fsck for repository-wide issues
$ mediagit fsck

# Attempt recovery
$ mediagit fsck --repair
```

### Slow Verification

```bash
# Use quick mode
$ mediagit verify --quick <object>

# Reduce parallelism
$ mediagit verify --parallel=2 <objects>

# Verify without chunks
$ mediagit verify --path <file>  # skip --verify-chunks
```

## Notes

### Safety

Verify is read-only and safe:
- Never modifies repository
- Can be run at any time
- Does not interfere with other operations
- Useful for automated checks

### Verify vs Fsck

- **verify**: Specific objects, quick, targeted
- **fsck**: Entire repository, comprehensive, slower

Use verify for spot-checks, fsck for complete validation.

### Media Files

For media-heavy repositories:
- Quick verification (checksum) is usually sufficient
- Use `--verify-chunks` after downloads
- Use `--deep` only when investigating corruption
- Batch verification with `--parallel` for efficiency

## See Also

- [mediagit fsck](./fsck.md) - Full repository verification
- [mediagit gc](./gc.md) - Garbage collection with verification
- [mediagit stats](./stats.md) - Repository statistics
- [mediagit show](./show.md) - Show object contents
