# mediagit gc

Cleanup unnecessary files and optimize repository.

## Synopsis

```bash
mediagit gc [OPTIONS]
```

## Description

Runs garbage collection to optimize the repository by:
- Removing unreachable objects
- Compressing loose objects into packfiles
- Removing redundant packfiles
- Optimizing deduplication and compression
- Building or updating commit graph
- Pruning old reflog entries

For media repositories, garbage collection is particularly important to:
- Reclaim space from deleted large files
- Optimize chunk storage and deduplication
- Improve performance of object retrieval
- Reduce storage costs for remote backends

## Options

### Garbage Collection Mode

#### `--aggressive`
More aggressive optimization (slower but better compression).

#### `--auto`
Run only if repository needs optimization (default behavior).

#### `--prune[=<date>]`
Prune loose objects older than date (default: 2 weeks ago).

#### `--no-prune`
Do not prune any loose objects.

### Performance Options

#### `--quiet`, `-q`
Suppress all output.

#### `--force`
Force garbage collection even if another gc is running.

### MediaGit-Specific Options

#### `--optimize-chunks`
Reoptimize chunk storage and deduplication.

#### `--rebuild-index`
Rebuild object database index.

#### `--compress-level=<n>`
Recompress objects with specified level (0-9).

#### `--repack`
Repack loose objects into pack files for better compression and storage efficiency.

#### `--max-pack-size=<size>`
Maximum size per pack file (e.g., 100MB, 1GB). Default: unlimited.

#### `--verify`
Verify object integrity during gc.

## Examples

### Basic garbage collection

```bash
$ mediagit gc
Enumerating objects: 8,875, done.
Counting objects: 100% (8,875/8,875), done.
Compressing objects: 100% (4,238/4,238), done.
Writing objects: 100% (8,875/8,875), done.
Building commit graph: 100% (142/142), done.

Garbage collection complete:
  Objects processed: 8,875
  Objects removed: 247 (unreachable)
  Objects compressed: 4,238
  Space reclaimed: 127.3 MB
  Repository size: 612.6 MB → 485.3 MB
  Time: 12.4s
```

### Aggressive optimization

```bash
$ mediagit gc --aggressive
Running aggressive garbage collection...

Phase 1: Enumerating objects
  Objects found: 8,875
  Time: 2.1s

Phase 2: Removing unreachable objects
  Unreachable objects: 247
  Space reclaimed: 127.3 MB
  Time: 1.8s

Phase 3: Recompressing objects
  Algorithm: Zstd level 9 (max compression)
  Objects recompressed: 8,628
  Original size: 3.2 GB
  Compressed size: 485.3 MB → 412.8 MB
  Additional savings: 72.5 MB (14.9%)
  Time: 45.7s

Phase 4: Optimizing deduplication
  Duplicate chunks found: 89
  Deduplication savings: 45.2 MB
  Time: 5.3s

Phase 5: Building commit graph
  Commits: 142
  Time: 0.4s

Total time: 55.3s
Final repository size: 340.3 MB (89.4% compression from original)
```

### Prune old objects

```bash
$ mediagit gc --prune=1.week.ago
Pruning objects older than 1 week...

Pruned objects:
  Commits: 12
  Trees: 34
  Blobs: 156
  Total space reclaimed: 89.4 MB

Remaining objects: 8,673
Repository size: 485.3 MB → 395.9 MB
```

### Optimize chunk storage

```bash
$ mediagit gc --optimize-chunks
Analyzing chunk storage...

Chunk optimization:
  Total chunks: 2,847
  Duplicate chunks: 89
  Fragmented chunks: 23
  Suboptimal compression: 12

Reoptimizing chunks:
  Deduplication: 89 chunks → 45.2 MB saved
  Defragmentation: 23 chunks → 12.1 MB saved
  Recompression: 12 chunks → 3.8 MB saved
  Total savings: 61.1 MB

Final chunk count: 2,735 (-112)
Repository size: 485.3 MB → 424.2 MB
```

### Repack loose objects into pack files

Basic repack:
```bash
$ mediagit gc --repack
Repacking objects...
Packed 4,238 objects into pack file (2,847 deltas)
Pack file: objects/pack/pack_abc123.pack (127.3 MB)
Loose objects removed: 4,238
Space reclaimed: 89.4 MB
```

With size limit:
```bash
$ mediagit gc --repack --max-pack-size=500MB
Creating pack files with 500MB limit...
Pack file 1: objects/pack/pack_abc123.pack (500 MB)
Pack file 2: objects/pack/pack_def456.pack (287 MB)
Total packed: 4,238 objects
```

Verify packing:
```bash
$ mediagit stats
Object database:
  Loose objects: 0
  Pack files: 2
  Total objects: 4,238
```

### Rebuild object index

```bash
$ mediagit gc --rebuild-index
Rebuilding object database index...

Index statistics:
  Objects indexed: 8,875
  Chunks indexed: 2,847
  Index size: 2.4 MB
  Build time: 3.7s

Index verification: ✓ All objects accessible
```

### Recompress with different level

```bash
$ mediagit gc --compress-level=9
Recompressing objects with Zstd level 9...

Compression analysis:
  Current level: 3
  New level: 9
  Objects to recompress: 8,628

Recompression progress:
  [============================] 100% (8,628/8,628)

Results:
  Original compressed size: 485.3 MB (level 3)
  New compressed size: 412.8 MB (level 9)
  Additional savings: 72.5 MB (14.9%)
  Compression time: 38.2s
  Decompression impact: ~15% slower

Recommendation: Level 9 trades 15% slower access for 14.9% storage savings.
```

### Verify during gc

```bash
$ mediagit gc --verify
Running garbage collection with verification...

Phase 1: Object enumeration and verification
  Verifying checksums: 100% (8,875/8,875) ✓
  Verifying chunk integrity: 100% (2,847/2,847) ✓
  All objects verified successfully

Phase 2: Garbage collection
  Objects removed: 247
  Space reclaimed: 127.3 MB

Phase 3: Post-gc verification
  Verifying object database: ✓
  Verifying commit graph: ✓
  Verifying refs: ✓

Garbage collection complete with full verification ✓
```

### Auto mode

```bash
$ mediagit gc --auto
Repository does not need garbage collection yet.
(Last gc: 2 days ago, objects: 8,875, fragmentation: 12%)

Next gc recommended when:
  - Objects > 10,000 OR
  - Fragmentation > 25% OR
  - Time since last gc > 1 week

Use 'mediagit gc' to run now anyway.
```

### Quiet mode

```bash
$ mediagit gc --quiet
# No output, runs in background
# Check status with:
$ echo $?
0  # Success
```

## Garbage Collection Phases

### Phase 1: Object Enumeration

```
Enumerating objects: 8,875, done.
```

Scans repository to find all reachable objects from refs (branches, tags).

### Phase 2: Unreachable Object Removal

```
Removing unreachable objects: 247
```

Deletes objects not reachable from any ref (orphaned by deleted branches, amended commits, etc.).

### Phase 3: Object Compression

```
Compressing objects: 100% (4,238/4,238), done.
```

Compresses loose objects into efficient storage format.

### Phase 4: Deduplication Optimization

```
Optimizing deduplication: 2,847 chunks
```

Identifies and removes duplicate chunks across files.

### Phase 5: Commit Graph

```
Building commit graph: 100% (142/142), done.
```

Builds commit graph for fast traversal operations (log, merge-base, etc.).

## When to Run GC

### Automatic Triggers

MediaGit automatically suggests gc when:
- Loose objects > 1,000
- Repository fragmentation > 25%
- Time since last gc > 1 week
- Space waste > 500 MB

### Manual Triggers

Run gc manually:
- After deleting large branches
- After rebasing or amending many commits
- Before pushing to save bandwidth
- Before archiving repository
- When storage costs are concern

### Repack for Efficiency

Use `--repack` when:
- Space is limited (consolidates objects)
- Preparing for push (single pack file transfers faster)
- Before backup/archival (easier restoration)
- In CI/CD pipelines (regular maintenance)

**Note**: Repack is automatic during aggressive GC, can be run standalone

## Performance Impact

### During GC

Repository operations may be slower:
- Read operations: ~10-20% slower
- Write operations: Not recommended during gc
- Duration: 10s - 5 minutes depending on size

### After GC

Repository operations are faster:
- Read operations: 20-40% faster
- Object retrieval: 30-50% faster
- Clone/fetch: Faster due to smaller size
- Push: Faster due to better compression

## Storage Savings

### Typical Savings

```
Small repository (<100 MB):
  Unreachable objects: 5-15 MB
  Better compression: 10-20 MB
  Deduplication: 5-10 MB
  Total savings: 20-45 MB (20-45%)

Medium repository (100 MB - 1 GB):
  Unreachable objects: 50-150 MB
  Better compression: 100-200 MB
  Deduplication: 50-100 MB
  Total savings: 200-450 MB (30-50%)

Large repository (>1 GB):
  Unreachable objects: 100-500 MB
  Better compression: 200-800 MB
  Deduplication: 100-400 MB
  Total savings: 400-1,700 MB (40-60%)
```

## Configuration

```toml
[gc]
# Run gc automatically after certain operations
auto = true

# Threshold for auto gc (loose objects)
auto_limit = 1000

# Prune objects older than this
prune_expire = "2.weeks.ago"

# Aggressive compression
aggressive = false

# Verify during gc
verify = false

[gc.compression]
# Compression level (0-9)
level = 3

# Recompress during gc
recompress = false

# Target compression level
target_level = 3

[gc.dedup]
# Enable deduplication optimization
optimize = true

# Deduplication threshold
threshold = "4MB"

[gc.repack]
# Enable object repacking
enabled = true

# Maximum size per pack file (0 = unlimited)
max_size = 0

# Build delta chains during repack
build_deltas = true

[gc.performance]
# Parallel object processing
parallel = true

# Number of threads
threads = 4
```

## Remote Storage Backends

### S3 Storage

```bash
$ mediagit gc
Garbage collection for S3 backend...

Local cleanup:
  Objects removed: 247
  Space reclaimed: 127.3 MB

Remote cleanup:
  Analyzing remote objects...
  Orphaned objects: 89
  Remote space reclaimed: 245.8 MB
  Storage cost reduction: $0.0056/month

Note: Remote objects pruned after 30-day grace period
```

### Azure/GCS Storage

Similar optimizations apply to all storage backends with additional benefits:
- Reduced API calls
- Lower storage costs
- Faster transfers
- Improved cache hit rates

## Exit Status

- **0**: GC completed successfully
- **1**: GC failed or was interrupted
- **2**: Invalid options

## Best Practices

### Regular Maintenance

```bash
# Weekly gc for active repositories
$ mediagit gc

# Monthly aggressive gc
$ mediagit gc --aggressive

# Before major operations
$ mediagit gc --verify
```

### Before Sharing

```bash
# Optimize before push
$ mediagit gc
$ mediagit push

# Optimize before archive
$ mediagit gc --aggressive
$ tar czf repo-backup.tar.gz .mediagit/
```

### After Bulk Operations

```bash
# After branch cleanup
$ mediagit branch -d old-feature-1 old-feature-2 old-feature-3
$ mediagit gc --prune=now

# After rebase/amend
$ mediagit rebase -i HEAD~10
$ mediagit gc

# After large file removal
$ mediagit rm large-file.mp4
$ mediagit commit -m "Remove large file"
$ mediagit gc --aggressive
```

## Troubleshooting

### GC Taking Too Long

```bash
# Use auto mode to check if needed
$ mediagit gc --auto

# Run with less aggressive settings
$ mediagit gc --no-prune
```

### Disk Space Issues

```bash
# Aggressive cleanup
$ mediagit gc --aggressive --prune=now

# Verify space reclaimed
$ du -sh .mediagit/
```

### Corrupted Objects

```bash
# Run gc with verification
$ mediagit gc --verify

# If corruption found
$ mediagit fsck
$ mediagit gc --rebuild-index
```

## Notes

### Safety

Garbage collection is safe:
- Never deletes reachable objects
- Maintains repository integrity
- Can be interrupted and resumed
- Verifies data when requested

### Concurrent GC

MediaGit prevents concurrent gc:
```bash
$ mediagit gc
fatal: gc is already running (pid 1234)
hint: Use --force to run anyway (not recommended)
```

### Media Repository Benefits

For media-heavy repositories, gc provides:
- Massive space savings (40-60% typical)
- Faster operations (20-40% improvement)
- Lower storage costs
- Better deduplication efficiency
- Improved cache performance

## See Also

- [mediagit fsck](./fsck.md) - Verify repository integrity
- [mediagit prune](./prune.md) - Prune unreachable objects
- [mediagit verify](./verify.md) - Verify object integrity
- [mediagit stats](./stats.md) - Show repository statistics
