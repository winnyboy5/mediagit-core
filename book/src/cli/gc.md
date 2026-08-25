# mediagit gc

Clean up repository and optimize storage.

## Synopsis

```bash
mediagit gc [OPTIONS]
```

## Description

Runs garbage collection to reclaim space in the repository:

1. Builds a reachability set from refs + the index
2. Scans the object database and identifies unreachable objects
3. Deletes unreachable objects (prompts for confirmation above 100 objects,
   unless `--yes` or `--auto`)
4. Scans for and deletes orphaned chunks and chunk manifests
5. Scans for and deletes orphaned blob-level and chunk-level deltas
6. Optionally repacks loose objects into pack files (`--repack`)
7. Regenerates/prunes reachability bitmaps, when bitmaps are enabled

By default gc prunes (deletes) everything unreachable it finds; pass
`--no-prune` to only report what's unreachable without deleting it.

## Options

| Flag | Description |
|---|---|
| `--no-prune` | Skip pruning unreachable objects (by default, gc prunes) |
| `--auto` | Auto gc threshold (run only if thresholds exceeded) |
| `--dry-run` | Show what would be done without deleting |
| `-y`, `--yes` | Skip confirmation prompts (auto-confirm deletions) |
| `-q`, `--quiet` | Quiet mode (minimal output) |
| `-v`, `--verbose` | Verbose mode (detailed output) |
| `--repack` | Repack loose objects into pack files |
| `--max-pack-size <N>` | Maximum objects per pack file (0 = unlimited) [default: 0] |
| `--color <WHEN>` | Colored output: `always`, `auto`, or `never` (default: `auto`) |
| `-C`, `--repository <PATH>` | Repository path |
| `-V`, `--version` | Print version |

## Examples

### Basic garbage collection

```bash
$ mediagit gc
→ Building reachability graph from refs + index...
→ Scanning object database...
→ Identifying unreachable objects...
→ Deleting unreachable objects...
✓ Deleted 247 objects, reclaimed 12.30 MB

→ Scanning for orphaned chunks and manifests...
ℹ Found 3 orphan manifests and 5 orphan chunks (1.80 MB)
✓ Deleted 3 manifests + 5 chunks, reclaimed 1.80 MB

→ Scanning for orphaned blob-level deltas...
→ Scanning for orphaned chunk-level deltas...

=== GC Statistics ===
Objects scanned:         156
Reachable objects:       109
Unreachable objects:     247
Objects deleted:         247
Space reclaimed:         12.30 MB
Manifests deleted:       3
Chunks deleted:          5
Chunk space reclaimed:   1.80 MB
Time taken:              1.24s
```

### Dry run

```bash
$ mediagit gc --dry-run
ℹ Running in dry-run mode (no changes will be made)
→ Building reachability graph from refs + index...
→ Scanning object database...
→ Identifying unreachable objects...

ℹ Would delete 247 objects (12.30 MB total)

→ Scanning for orphaned chunks and manifests...
ℹ Found 3 orphan manifests and 5 orphan chunks (1.80 MB)

=== GC Statistics ===
Objects scanned:         156
Reachable objects:       109
Unreachable objects:     247
Objects deleted:         247
Space reclaimed:         12.30 MB
Time taken:              0.31s
```

### No prune (report only)

`--no-prune` stops after reporting unreachable objects — it does not delete
anything, and it does not run the chunk/manifest/delta scans:

```bash
$ mediagit gc --no-prune
→ Building reachability graph from refs + index...
→ Scanning object database...
→ Identifying unreachable objects...
ℹ Found 247 unreachable objects (12.30 MB) - skipping prune (--no-prune)

=== GC Statistics ===
Objects scanned:         156
Reachable objects:       109
Unreachable objects:     247
Objects deleted:         0
Space reclaimed:         0 bytes
Time taken:              0.18s
```

### Skip the confirmation prompt

Deleting more than 100 unreachable objects normally prompts for confirmation;
`-y`/`--yes` (or `--auto`) skips it:

```bash
$ mediagit gc -y
→ Building reachability graph from refs + index...
→ Scanning object database...
→ Identifying unreachable objects...
→ Deleting unreachable objects...
✓ Deleted 247 objects, reclaimed 12.30 MB
...
```

### Auto mode

`--auto` skips the whole run silently unless the reclaimable garbage exceeds
50 MiB or 100 orphaned objects, and never prompts for confirmation:

```bash
$ mediagit gc --auto -v
ℹ auto-gc: 12 orphans / 3.20 MB below thresholds (>= 50.00 MB or >= 100 objects); skipping
```

```bash
$ mediagit gc --auto
→ auto-gc: reclaiming 247 orphans (12.30 MB)
→ Building reachability graph from refs + index...
...
```

### Repack loose objects into pack files

```bash
$ mediagit gc --repack
→ Building reachability graph from refs + index...
...
→ Repacking loose objects...
✓ Packed 4,238 objects into pack file (2,847 deltas)
   Pack size: 127.30 MB, Saved: 89.40 MB
```

With a pack-size limit:

```bash
$ mediagit gc --repack --max-pack-size=500
```

### Quiet mode

```bash
$ mediagit gc --quiet
$ echo $?
0
```

## Exit Status

- **0**: GC completed successfully
- non-zero: GC failed (errors during deletion, repack failure, or not a repository)

## See Also

- [mediagit fsck](./fsck.md) - Verify repository integrity
- [mediagit stats](./stats.md) - Show repository statistics
