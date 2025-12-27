# mediagit fsck

Verify integrity and connectivity of objects in repository.

## Synopsis

```bash
mediagit fsck [OPTIONS] [<object>...]
```

## Description

Verifies the connectivity and validity of objects in the repository database. Checks for:
- Corrupt or missing objects
- Broken links between objects
- Orphaned or unreachable objects
- Invalid object references
- Checksum mismatches
- Chunk integrity for media files

MediaGit fsck provides **media-aware verification** including chunk-level integrity checks, compression validation, and deduplication consistency.

## Options

### Verification Mode

#### `--full`
Perform complete verification of all objects (default).

#### `--connectivity-only`
Check only connectivity, skip detailed validation.

#### `--dangling`
Print dangling (unreachable but valid) objects.

#### `--no-dangling`
Suppress dangling object reports (default).

#### `--unreachable`
Show all unreachable objects.

#### `--lost-found`
Write dangling objects to .mediagit/lost-found/.

### Object Selection

#### `<object>...`
Check specific objects instead of entire repository.

#### `--cache`
Check index file consistency.

#### `--commit-graph`
Verify commit graph integrity.

### Output Options

#### `-v`, `--verbose`
Show detailed verification information.

#### `--progress`
Show progress during verification.

#### `--no-progress`
Suppress progress reporting.

#### `-q`, `--quiet`
Suppress all output except errors.

### MediaGit-Specific Options

#### `--verify-chunks`
Verify chunk integrity and checksums.

#### `--verify-compression`
Validate compressed object integrity.

#### `--verify-dedup`
Check deduplication consistency.

#### `--repair`
Attempt to repair minor issues (use with caution).

## Examples

### Basic fsck

```bash
$ mediagit fsck
Checking object directory: .mediagit/objects
Checking objects: 100% (8,875/8,875), done.

Object verification:
  Commits: 142 ✓
  Trees: 1,847 ✓
  Blobs: 6,886 ✓
  Total: 8,875 objects verified

Connectivity check:
  All objects reachable ✓
  All refs valid ✓
  No broken links found ✓

Repository integrity: OK
```

### Full verification with chunks

```bash
$ mediagit fsck --verify-chunks
Checking object directory: .mediagit/objects
Checking objects: 100% (8,875/8,875), done.

Object verification:
  Commits: 142 ✓
  Trees: 1,847 ✓
  Blobs: 6,886 ✓

Chunk verification:
  Total chunks: 2,847
  Verifying checksums: 100% (2,847/2,847) ✓
  Verifying integrity: 100% (2,847/2,847) ✓
  Corrupted chunks: 0 ✓

Deduplication verification:
  Duplicate references: 847
  Reference consistency: 100% ✓
  No dedup errors ✓

Repository integrity: OK
```

### Show dangling objects

```bash
$ mediagit fsck --dangling
Checking objects: 100% (8,875/8,875), done.

Dangling objects:
  dangling commit a3c8f9d2e1b4f6a8c5d7e9f1a2b3c4d5e6f7a8b9c0d1e2f3a4b5c6d7e8f9a0b1
  dangling blob b4d7e1a9f2c3d4e5f6a7b8c9d0e1f2a3b4c5d6e7f8a9b0c1d2e3f4a5b6c7d8e9
  dangling tree c5e9f2b4764f2dbcee52635b91fedb1b3dcf7ab4d5e6f7a8b9c0d1e2f3a4b5c6d7e8

Total dangling objects: 3 (127.3 MB)

hint: Run 'mediagit fsck --lost-found' to save these objects
hint: Run 'mediagit gc' to remove dangling objects
```

### Show unreachable objects

```bash
$ mediagit fsck --unreachable
Checking objects: 100% (8,875/8,875), done.

Unreachable objects:
  unreachable commit a3c8f9d (from deleted branch 'feature/old')
    Author: Alice <alice@example.com>
    Date: 2024-01-10
    Message: "Old feature work"
    Size: 2.3 KB

  unreachable blob b4d7e1a (deleted file 'old-video.mp4')
    Size: 245.8 MB (compressed to 42.1 MB)

  unreachable tree c5e9f2b
    Size: 1.2 KB

Total unreachable: 3 objects (245.8 MB original, 42.1 MB stored)

hint: These objects will be pruned by 'mediagit gc --prune'
```

### Lost-found recovery

```bash
$ mediagit fsck --lost-found
Checking objects: 100% (8,875/8,875), done.

Dangling objects written to .mediagit/lost-found/:
  commit/a3c8f9d
  blob/b4d7e1a
  tree/c5e9f2b

3 objects saved for recovery
```

### Verify specific objects

```bash
$ mediagit fsck HEAD main feature/video-optimization
Checking specified objects...

HEAD (a3c8f9d):
  Type: commit ✓
  Tree: valid ✓
  Parents: valid ✓
  Author/Committer: valid ✓

main (a3c8f9d):
  Points to valid commit ✓

feature/video-optimization (b4d7e1a):
  Points to valid commit ✓

3 objects verified, all OK
```

### Verify index

```bash
$ mediagit fsck --cache
Checking index file...

Index statistics:
  Version: 2
  Entries: 247
  Size: 12.4 KB

Index verification:
  Entry checksums: 100% (247/247) ✓
  Object references: 100% (247/247) ✓
  Path ordering: valid ✓
  Extension data: valid ✓

Index integrity: OK
```

### Verify commit graph

```bash
$ mediagit fsck --commit-graph
Checking commit graph...

Commit graph statistics:
  Version: 1
  Commits: 142
  Size: 24.7 KB

Commit graph verification:
  Commit OIDs: 100% (142/142) ✓
  Parent links: 100% (128/128) ✓
  Generation numbers: valid ✓
  Bloom filters: valid ✓

Commit graph integrity: OK
```

### Verbose output

```bash
$ mediagit fsck -v
Checking object directory: .mediagit/objects
Checking objects: 100% (8,875/8,875), done.

Detailed verification:

Commits (142):
  Checking commit a3c8f9d... OK
  Checking commit b4d7e1a... OK
  [140 more commits]
  All commits valid ✓

Trees (1,847):
  Checking tree c5e9f2b... OK
  Checking tree d6f0a3c... OK
  [1,845 more trees]
  All trees valid ✓

Blobs (6,886):
  Checking blob e7g1b4d... OK (video.mp4, 42.1 MB)
  Checking blob f8h2c5e... OK (image.jpg, 0.8 MB)
  [6,884 more blobs]
  All blobs valid ✓

Connectivity:
  Checking refs... 5 refs checked ✓
  Checking reflog... 42 entries checked ✓
  Checking HEAD... valid ✓

No errors found
Repository integrity: EXCELLENT
```

### Compression verification

```bash
$ mediagit fsck --verify-compression
Checking objects: 100% (8,875/8,875), done.

Compression verification:
  Total compressed objects: 6,886
  Decompression test: 100% (6,886/6,886) ✓
  Checksum validation: 100% (6,886/6,886) ✓
  Compression ratio verification: ✓

Compression statistics:
  Average ratio: 82.3%
  Corrupted objects: 0 ✓
  Invalid compression: 0 ✓

All compression valid ✓
```

### Deduplication verification

```bash
$ mediagit fsck --verify-dedup
Checking objects: 100% (8,875/8,875), done.

Deduplication verification:
  Total chunks: 2,847
  Chunk references: 8,472
  Duplicate chunks: 847

Dedup consistency checks:
  Reference integrity: 100% (8,472/8,472) ✓
  Chunk checksums: 100% (2,847/2,847) ✓
  Reference counts: valid ✓
  No orphaned chunks: ✓

Deduplication savings: 2.3 GB (45.2%)
All dedup structures valid ✓
```

### Repair mode (cautious)

```bash
$ mediagit fsck --repair
WARNING: Repair mode may modify repository
Create backup before proceeding? [y/N] y

Creating backup...
Backup created: .mediagit-backup-20240115-143022

Checking objects: 100% (8,875/8,875), done.

Issues found:
  Missing chunk reference in blob e7g1b4d
  Attempting repair... SUCCESS ✓

  Invalid checksum in blob f8h2c5e
  Attempting repair from backup... SUCCESS ✓

Repairs completed: 2
Failed repairs: 0

Re-verifying repository...
Repository integrity: OK

Backup location: .mediagit-backup-20240115-143022
```

## Error Detection

### Corrupt Object

```bash
$ mediagit fsck
Checking objects: 100% (8,875/8,875), done.

ERROR: Corrupt blob b4d7e1a
  Expected checksum: b4d7e1a9f2c3d4e5f6a7b8c9d0e1f2a3b4c5d6e7f8a9b0c1d2e3f4a5b6c7d8e9
  Actual checksum:   b4d7e1a9f2c3d4e5f6a7b8c9d0e1f2a3XXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXX
  File: video.mp4
  Size: 245.8 MB

Repository integrity: DAMAGED
Run 'mediagit fsck --repair' to attempt recovery
```

### Missing Object

```bash
$ mediagit fsck
Checking objects: 100% (8,875/8,875), done.

ERROR: Missing blob c5e9f2b
  Referenced by: tree d6f0a3c
  Path: assets/logo.png
  Expected size: 156.3 KB

ERROR: Broken link in tree d6f0a3c
  Missing child: c5e9f2b

Repository integrity: DAMAGED
2 errors found
```

### Broken Chain

```bash
$ mediagit fsck
Checking connectivity...

ERROR: Broken chain detected
  Commit a3c8f9d references parent b4d7e1a
  Parent b4d7e1a: NOT FOUND

ERROR: Unreachable commit d6f0a3c
  Orphaned by broken chain

Repository integrity: DAMAGED
```

## Verification Levels

### Quick (Connectivity Only)

```bash
$ mediagit fsck --connectivity-only
Quick verification: Checking reachability only
Checking refs: 100% (5/5), done.

All refs reachable ✓
Repository connectivity: OK

Note: Run full fsck to verify object integrity
```

### Standard (Default)

```bash
$ mediagit fsck
Full verification with checksum validation
Time: 10-30 seconds for typical repository
```

### Complete (All Checks)

```bash
$ mediagit fsck --verify-chunks --verify-compression --verify-dedup -v
Comprehensive verification including media-specific checks
Time: 30-120 seconds for large media repository
```

## Performance

### Small Repository (<100 MB)
```
Objects: < 1,000
Time: 1-3 seconds
```

### Medium Repository (100 MB - 1 GB)
```
Objects: 1,000-10,000
Time: 5-15 seconds
```

### Large Repository (>1 GB)
```
Objects: 10,000+
Time: 30-120 seconds
With --verify-chunks: 2-5 minutes
```

## Exit Status

- **0**: No errors found, repository OK
- **1**: Errors detected, repository damaged
- **2**: Invalid options or fsck failed to run

## Configuration

```toml
[fsck]
# Verify chunks by default
verify_chunks = false

# Verify compression
verify_compression = false

# Verify deduplication
verify_dedup = false

# Show dangling objects
show_dangling = false

# Progress reporting
show_progress = true

[fsck.repair]
# Allow automatic repair
auto_repair = false

# Create backup before repair
backup_before_repair = true

# Repair attempts
max_attempts = 3
```

## Best Practices

### Regular Checks

```bash
# Weekly fsck for active repositories
$ mediagit fsck

# Monthly comprehensive check
$ mediagit fsck --verify-chunks --verify-compression --verify-dedup
```

### Before Important Operations

```bash
# Before backup
$ mediagit fsck --verify-chunks
$ tar czf backup.tar.gz .mediagit/

# Before migration
$ mediagit fsck --full
$ mediagit push --mirror new-remote
```

### After Errors

```bash
# After system crash
$ mediagit fsck --verify-chunks
$ mediagit gc --verify

# After transfer interruption
$ mediagit fsck
$ mediagit pull --verify-checksums
```

## Troubleshooting

### Corrupt Objects Found

```bash
# Try repair
$ mediagit fsck --repair

# If repair fails, restore from backup
$ cp -r .mediagit-backup/* .mediagit/
$ mediagit fsck
```

### Missing Objects

```bash
# Check if available in remote
$ mediagit fetch
$ mediagit fsck

# Restore from lost-found
$ mediagit fsck --lost-found
$ ls .mediagit/lost-found/
```

### Performance Issues

```bash
# Quick check first
$ mediagit fsck --connectivity-only

# Full check if quick check passes
$ mediagit fsck
```

## Notes

### Safety

fsck is safe and read-only by default:
- Never modifies repository without --repair
- Can be run at any time
- Does not interfere with other operations
- Creates backups before repairs

### Media Repository Checks

For media-heavy repositories:
- Chunk verification essential for integrity
- Compression validation ensures quality
- Deduplication checks prevent storage waste
- Regular fsck prevents gradual corruption

### Automation

```bash
# Daily fsck in CI/CD
$ mediagit fsck --quiet && echo "Repository OK" || echo "INTEGRITY ERROR"

# Weekly comprehensive check
$ mediagit fsck --verify-chunks --verify-compression
```

## See Also

- [mediagit gc](./gc.md) - Garbage collection and optimization
- [mediagit verify](./verify.md) - Verify specific objects
- [mediagit stats](./stats.md) - Repository statistics
- [mediagit prune](./prune.md) - Prune unreachable objects
