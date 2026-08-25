# mediagit stats

Display repository statistics and metrics.

## Synopsis

```bash
mediagit stats [OPTIONS]
```

## Description

Shows statistics about the repository, computed live from the object database,
ref database, and storage backend:

- Recent operation history (last pull/push/branch switch)
- Storage: object counts and bytes, broken down as loose/chunks/deltas, plus
  original-vs-stored size and compression ratio
- Branch, tag, and remote-branch counts
- Commit history (total commits, first/last commit date)
- Tracked file counts (media/text/other), and a by-media-type breakdown when
  media metadata tracking is enabled
- Author commit counts
- Compression: algorithm, storage used, and a per-file-type breakdown for
  chunked (media) files

With no scope flag, all sections are shown. Passing one or more scope flags
(`--storage`, `--files`, etc.) shows only those sections.

## Options

| Flag | Description |
|---|---|
| `--storage` | Show storage statistics |
| `--files` | Show file statistics |
| `--commits` | Show commit statistics |
| `--branches` | Show branch statistics |
| `--authors` | Show author statistics |
| `--compression` | Show compression metrics |
| `--all` | All statistics (default when no scope flag is given) |
| `--json` | Format as JSON |
| `--prometheus` | Format as Prometheus exposition text |
| `-q`, `--quiet` | Quiet mode — suppresses all output, including `--json`/`--prometheus` |
| `-v`, `--verbose` | Verbose mode — adds a storage breakdown (loose/packs/chunks/deltas bytes), pack/manifest counts, largest shard bucket, and session cache hit/miss counters |
| `--color <WHEN>` | Colored output: `always`, `auto`, or `never` (default: `auto`) |
| `-C`, `--repository <PATH>` | Repository path |
| `-h`, `--help` | Print help |
| `-V`, `--version` | Print version |

`--quiet` takes priority over `--json` and `--prometheus`: with `-q`, `stats` prints nothing and exits `0`.

## Examples

### Basic statistics

```bash
$ mediagit stats
📊 Repository Statistics

Recent Operations:
  Last pull: ↓ 128.4 MiB, 42 objects received in 3s (12 minutes ago)
  Last push: No history
  Last branch switch: No history

Storage:
  Total objects: 156 (89 loose, 62 chunks, 5 deltas)
  Original size: 3.20 GiB
  Storage used:  485.30 MiB
  Compression:   6.6x ratio (84.8% saved)

Branches:
  Local branches: 3
  Remote branches: 1
  Tags: 2

Commits:
  Total commits: 42
  First commit: 2026-06-01
  Last commit: 2026-08-20

Files:
  Tracked files: 188
  Media files: 165
  Text files: 20
  Other files: 3

Authors:
  Alice Developer <alice@example.com>: 30 commits
  Bob Designer <bob@example.com>: 12 commits

Compression:
  Algorithm: zstd (type-aware, store for pre-compressed)
  Storage used: 485.30 MiB
  Chunked files: 12 manifests, 3.20 GiB original → 425.10 MiB stored (7.7x, 87.0% saved)
  By file type:
    video   : 8 files, 2.80 GiB original
    image   : 4 files, 400.00 MiB original

Repository is operational
```

### Single scope

```bash
$ mediagit stats --storage
Storage:
  Total objects: 156 (89 loose, 62 chunks, 5 deltas)
  Original size: 3.20 GiB
  Storage used:  485.30 MiB
  Compression:   6.6x ratio (84.8% saved)
```

### Verbose

`-v` adds a breakdown line, pack/manifest counts, the largest hash-shard
bucket (a fanout diagnostic), and this session's cache counters:

```bash
$ mediagit stats --storage -v
Storage:
  Total objects: 156 (89 loose, 62 chunks, 5 deltas)
  Original size: 3.20 GiB
  Storage used:  485.30 MiB
  Compression:   6.6x ratio (84.8% saved)
  Breakdown: loose: 42.10 MiB, packs: 0 B, chunks: 425.10 MiB, deltas: 18.10 MiB
  Chunk manifests: 12
  Largest shard directory: 4 entries in chunks/a3
  Session writes: 0
  Session bytes: 0
  Cache hits: 0
  Cache misses: 0
```

### JSON output

```bash
$ mediagit stats --json
{
  "storage": {
    "total_bytes": 508936192,
    "original_bytes": 3435973836,
    "loose_bytes": 44149350,
    "pack_bytes": 0,
    "chunk_bytes": 445751296,
    "delta_bytes": 18980864,
    "loose_objects": 89,
    "pack_files": 0,
    "chunks": 62,
    "deltas": 5,
    "manifests": 12,
    "largest_shard_bucket": {
      "bucket": "chunks/a3",
      "count": 4
    }
  },
  "commits": {
    "total": 42,
    "first_date": "2026-06-01T10:30:00+00:00",
    "last_date": "2026-08-20T14:30:22+00:00"
  },
  "branches": {
    "local": 3,
    "remote": 1,
    "tags": 2
  },
  "files": {
    "total": 188,
    "media": 165,
    "text": 20,
    "other": 3
  },
  "authors": [
    { "name": "Alice Developer", "email": "alice@example.com", "commits": 30 },
    { "name": "Bob Designer", "email": "bob@example.com", "commits": 12 }
  ]
}
```

### Prometheus output

```bash
$ mediagit stats --prometheus
# HELP mediagit_storage_bytes_total Total storage bytes
# TYPE mediagit_storage_bytes_total gauge
mediagit_storage_bytes_total 508936192
# HELP mediagit_objects_total Total objects stored
# TYPE mediagit_objects_total gauge
mediagit_objects_total 151
# HELP mediagit_commits_total Total commits
# TYPE mediagit_commits_total gauge
mediagit_commits_total 42
# HELP mediagit_packs_total Pack files
# TYPE mediagit_packs_total gauge
mediagit_packs_total 0
# HELP mediagit_chunks_total Chunks stored
# TYPE mediagit_chunks_total gauge
mediagit_chunks_total 62
```

## Exit Status

- **0**: Statistics generated successfully (including the no-op `--quiet` case)
- non-zero: Failed to open the repository or read storage/ref/object data

## See Also

- [mediagit gc](./gc.md) - Optimize repository
- [mediagit fsck](./fsck.md) - Verify integrity
