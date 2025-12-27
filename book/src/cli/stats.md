# mediagit stats

Display repository statistics and metrics.

## Synopsis

```bash
mediagit stats [OPTIONS] [<path>...]
```

## Description

Shows comprehensive statistics about the repository including:
- Object counts and sizes
- Compression ratios and savings
- Deduplication effectiveness
- Storage backend information
- Performance metrics
- Commit history analysis

Particularly useful for:
- Understanding repository growth
- Analyzing storage efficiency
- Identifying optimization opportunities
- Capacity planning
- Cost analysis for cloud storage

## Options

### Report Scope

#### `--all`
Show all available statistics (default).

#### `--objects`
Show object database statistics only.

#### `--compression`
Show compression statistics only.

#### `--dedup`
Show deduplication statistics only.

#### `--commits`
Show commit history statistics only.

#### `--storage`
Show storage backend statistics only.

### Analysis Depth

#### `--detailed`
Show detailed breakdown by type and category.

#### `--summary`
Show high-level summary only.

#### `--trends`
Include historical trends and growth analysis.

### Path Filtering

#### `<path>...`
Show statistics for specific paths only.

#### `--branch=<branch>`
Analyze specific branch.

#### `--since=<date>`
Statistics since specified date.

#### `--until=<date>`
Statistics until specified date.

### Output Format

#### `--human-readable`, `-h`
Use human-readable sizes (default).

#### `--bytes`
Show sizes in bytes.

#### `--json`
Output in JSON format.

#### `--csv`
Output in CSV format for analysis.

### MediaGit-Specific Options

#### `--media-analysis`
Include media-specific statistics (formats, codecs, resolutions).

#### `--chunk-analysis`
Show chunk-level statistics and efficiency.

#### `--cost-analysis`
Show estimated storage costs by backend.

## Examples

### Basic statistics

```bash
$ mediagit stats

MediaGit Repository Statistics
================================

Repository Overview:
  Location: /path/to/repo
  Created: 2024-01-01
  Age: 15 days
  Branches: 5
  Commits: 142

Object Database:
  Total objects: 8,875
    Commits: 142
    Trees: 1,847
    Blobs: 6,886
  Object size: 3.2 GB (original)
  Stored size: 485.3 MB (compressed)
  Compression ratio: 84.8%

Deduplication:
  Total chunks: 2,847
  Unique chunks: 2,000
  Duplicate chunks: 847 (29.8%)
  Space saved: 267.4 MB (11.6%)
  Effective compression: 87.2%

Storage Backend:
  Type: AWS S3
  Region: us-west-2
  Bucket: mediagit-prod-assets
  Objects stored: 8,875
  Estimated cost: $0.0112/month
```

### Compression statistics

```bash
$ mediagit stats --compression

Compression Statistics
======================

Overview:
  Algorithm: Zstd (level 3)
  Total objects: 8,875
  Compressed objects: 6,886
  Uncompressed objects: 1,989 (commits, trees)

Size Analysis:
  Original size: 3.2 GB
  Compressed size: 485.3 MB
  Compression ratio: 84.8%
  Space saved: 2.7 GB

By File Type:
  Video files (mp4, mov, avi):
    Count: 47
    Original: 2.8 GB
    Compressed: 398.7 MB
    Ratio: 85.8%

  Image files (jpg, png, psd):
    Count: 123
    Original: 347.2 MB
    Compressed: 78.4 MB
    Ratio: 77.4%

  Audio files (wav, mp3):
    Count: 18
    Original: 89.5 MB
    Compressed: 16.8 MB
    Ratio: 81.2%

Compression Performance:
  Average compression time: 0.8s per 100 MB
  Average decompression time: 0.2s per 100 MB
  Throughput: 125 MB/s (compression)
  Throughput: 500 MB/s (decompression)
```

### Deduplication statistics

```bash
$ mediagit stats --dedup

Deduplication Statistics
========================

Chunk Analysis:
  Total chunks: 2,847
  Unique chunks: 2,000 (70.2%)
  Duplicate chunks: 847 (29.8%)
  Chunk size: 4 MB (standard)

Deduplication Savings:
  Total data: 2.9 GB (chunked)
  Unique data: 2.6 GB
  Duplicate data: 267.4 MB
  Space saved: 267.4 MB (9.2%)

Deduplication Patterns:
  Single reference: 1,423 chunks (71.2%)
  2-5 references: 412 chunks (20.6%)
  6-10 references: 127 chunks (6.4%)
  >10 references: 38 chunks (1.9%)

Most Deduplicated Chunk:
  Chunk: a8b9c0d1e2f3...
  Size: 4 MB
  References: 23
  Savings: 88 MB
  Files: video_1080p.mp4, video_720p.mp4, video_480p.mp4 (intro sequence)

Efficiency:
  Deduplication ratio: 9.2%
  Combined with compression: 87.2% total efficiency
```

### Commit history statistics

```bash
$ mediagit stats --commits

Commit History Statistics
=========================

Overview:
  Total commits: 142
  First commit: 2024-01-01 10:30:00
  Latest commit: 2024-01-15 14:30:22
  Time span: 15 days

Commit Activity:
  Commits per day: 9.5 (average)
  Busiest day: 2024-01-10 (23 commits)
  Quietest day: 2024-01-03 (2 commits)

Contributors:
  Total contributors: 3
  1. Alice Developer: 89 commits (62.7%)
  2. Bob Designer: 47 commits (33.1%)
  3. Charlie Editor: 6 commits (4.2%)

Commit Size Distribution:
  Small (<10 MB): 78 commits (54.9%)
  Medium (10-100 MB): 52 commits (36.6%)
  Large (>100 MB): 12 commits (8.5%)

  Largest commit: a3c8f9d (410.2 MB original, 75.9 MB stored)
  Smallest commit: e7g1b4d (1.2 KB)

Storage Growth:
  Starting size: 0 MB
  Current size: 485.3 MB
  Growth rate: 32.4 MB/day
  Projected (30 days): 970.7 MB
  Projected (90 days): 2.9 GB
```

### Storage backend statistics

```bash
$ mediagit stats --storage

Storage Backend Statistics
==========================

Backend Configuration:
  Type: AWS S3
  Region: us-west-2
  Bucket: mediagit-prod-assets
  Storage class: Standard
  Versioning: Disabled
  Encryption: AES-256

Storage Usage:
  Total objects: 8,875
  Total size: 485.3 MB
  Largest object: 145.2 MB
  Smallest object: 128 bytes
  Average object: 54.7 KB

Transfer Statistics (last 30 days):
  Uploads: 247 objects (342.8 MB)
  Downloads: 89 objects (127.3 MB)
  Data transfer: 470.1 MB total

Cost Analysis:
  Storage cost: $0.0112/month
  Request cost: $0.0003/month
  Transfer cost: $0.0021/month
  Total cost: $0.0136/month

  Projected (1 year): $0.16
  Cost per GB: $0.023

Performance Metrics:
  Average upload speed: 8.2 MB/s
  Average download speed: 9.4 MB/s
  Average latency: 45ms
```

### Media analysis

```bash
$ mediagit stats --media-analysis

Media File Analysis
===================

Overview:
  Total media files: 188
  Total size: 2.8 GB (original)
  Stored size: 423.9 MB (compressed)

Video Files (47):
  Formats: MP4 (35), MOV (8), AVI (4)
  Total size: 2.4 GB → 398.7 MB
  Resolutions:
    4K (3840x2160): 8 files (1.2 GB)
    1080p (1920x1080): 23 files (945.3 MB)
    720p (1280x720): 12 files (234.7 MB)
    480p (854x480): 4 files (67.2 MB)

  Codecs:
    H.265/HEVC: 18 files (better compression)
    H.264/AVC: 27 files (wider compatibility)
    Other: 2 files

  Duration Distribution:
    <1 minute: 12 files
    1-5 minutes: 28 files
    >5 minutes: 7 files
    Total duration: 3 hours 47 minutes

Image Files (123):
  Formats: JPEG (89), PNG (28), PSD (6)
  Total size: 347.2 MB → 78.4 MB
  Resolutions:
    >4K: 12 files
    1080p-4K: 47 files
    <1080p: 64 files

Audio Files (18):
  Formats: WAV (12), MP3 (6)
  Total size: 89.5 MB → 16.8 MB
  Sample rates: 44.1 kHz (12), 48 kHz (6)
  Bit depths: 16-bit (12), 24-bit (6)
```

### Detailed statistics

```bash
$ mediagit stats --detailed

Detailed Repository Statistics
===============================

[... includes all above sections plus ...]

Object Type Breakdown:
  Commits:
    Count: 142
    Average size: 2.3 KB
    Total size: 326.6 KB

  Trees:
    Count: 1,847
    Average size: 847 bytes
    Total size: 1.5 MB

  Blobs:
    Small (<1 MB): 6,234 blobs (89.2 MB)
    Medium (1-10 MB): 478 blobs (2.1 GB)
    Large (>10 MB): 174 blobs (1.0 GB)
    Total: 6,886 blobs (3.2 GB)

Chunk Distribution:
  0-10 references: 1,835 chunks (91.8%)
  11-20 references: 127 chunks (6.4%)
  21-30 references: 32 chunks (1.6%)
  >30 references: 6 chunks (0.3%)

Performance Characteristics:
  Index size: 2.4 MB
  Commit graph size: 24.7 KB
  Reflog size: 8.2 KB
  Average lookup time: 0.3ms
  Cache hit rate: 92.3%
```

### Trend analysis

```bash
$ mediagit stats --trends --since="1 week ago"

Repository Trends (Last 7 Days)
================================

Growth Analysis:
  Starting size: 398.2 MB
  Current size: 485.3 MB
  Growth: +87.1 MB (+21.9%)
  Daily average: +12.4 MB

Commit Activity Trend:
  Day 1: 8 commits (45.2 MB)
  Day 2: 12 commits (67.8 MB)
  Day 3: 6 commits (23.4 MB)
  Day 4: 15 commits (89.7 MB)
  Day 5: 9 commits (34.5 MB)
  Day 6: 11 commits (56.3 MB)
  Day 7: 7 commits (28.9 MB)

Compression Efficiency Trend:
  Day 1: 82.1% compression
  Day 2: 83.5% compression
  Day 3: 84.2% compression
  Day 4: 83.8% compression
  Day 5: 84.7% compression
  Day 6: 85.1% compression
  Day 7: 84.9% compression
  Trend: Improving (+2.8%)

Deduplication Trend:
  Week start: 8.2% dedup savings
  Week end: 9.2% dedup savings
  Trend: Improving (+1.0%)
```

### JSON output

```bash
$ mediagit stats --json
{
  "repository": {
    "location": "/path/to/repo",
    "created": "2024-01-01T10:30:00Z",
    "age_days": 15,
    "branches": 5,
    "commits": 142
  },
  "objects": {
    "total": 8875,
    "commits": 142,
    "trees": 1847,
    "blobs": 6886,
    "original_size": 3435973836,
    "stored_size": 509012992,
    "compression_ratio": 0.848
  },
  "deduplication": {
    "total_chunks": 2847,
    "unique_chunks": 2000,
    "duplicate_chunks": 847,
    "savings_bytes": 280475648,
    "dedup_ratio": 0.092
  },
  "storage_backend": {
    "type": "s3",
    "region": "us-west-2",
    "bucket": "mediagit-prod-assets",
    "cost_per_month_usd": 0.0136
  }
}
```

### Path-specific statistics

```bash
$ mediagit stats videos/
Statistics for path: videos/

Objects: 47 files
Original size: 2.4 GB
Stored size: 398.7 MB
Compression: 83.4%

File breakdown:
  promo_4k.mp4: 856.3 MB → 145.2 MB (83.0%)
  promo_1080p.mp4: 245.8 MB → 42.1 MB (82.9%)
  training_*.mp4: 1.2 GB → 189.3 MB (84.2%)
  [44 more files]

Deduplication savings: 45.2 MB (18.9%)
```

## Cost Analysis

```bash
$ mediagit stats --cost-analysis

Storage Cost Analysis
=====================

Current Costs (Monthly):
  AWS S3 Standard:
    Storage: $0.0112 (485.3 MB @ $0.023/GB)
    Requests: $0.0003 (247 PUT, 89 GET)
    Transfer: $0.0021 (470.1 MB @ $0.09/GB)
    Total: $0.0136/month

Projected Costs:
  1 month: $0.014
  3 months: $0.042
  6 months: $0.084
  1 year: $0.168

Alternative Storage Classes:
  S3 Infrequent Access: $0.0081/month (40% savings)
  S3 Glacier: $0.0023/month (83% savings)
  S3 Deep Archive: $0.0005/month (96% savings)

Cost Optimization Recommendations:
  1. Run gc: Potential savings $0.0032/month
  2. Use S3 IA for old objects: Savings $0.0054/month
  3. Enable lifecycle policy: Savings $0.0089/month
  Total potential savings: $0.0175/month (128%)
```

## Performance

Statistics generation is fast:
- **Summary**: < 100ms
- **Detailed**: < 500ms
- **Full analysis** (all options): < 2s
- **Trends** (historical): < 5s

## Exit Status

- **0**: Statistics generated successfully
- **1**: Error generating statistics
- **2**: Invalid options

## Configuration

```toml
[stats]
# Default output format
format = "human"  # human | json | csv

# Show detailed stats by default
detailed = false

# Include trends
include_trends = false

# Cache statistics
cache_enabled = true
cache_ttl = "5 minutes"

[stats.display]
# Use colors in output
color = true

# Show ASCII charts
charts = true

# Number precision
precision = 1
```

## Best Practices

### Regular Monitoring

```bash
# Weekly stats check
$ mediagit stats --summary

# Monthly detailed analysis
$ mediagit stats --detailed --trends
```

### Before/After Comparison

```bash
# Before optimization
$ mediagit stats > stats-before.txt

# Run optimization
$ mediagit gc --aggressive

# After optimization
$ mediagit stats > stats-after.txt
$ diff stats-before.txt stats-after.txt
```

### Capacity Planning

```bash
# Analyze growth trends
$ mediagit stats --trends --since="30 days ago"

# Project future size
$ mediagit stats --commits --detailed
# Calculate: current_size + (daily_growth * days_ahead)
```

### Cost Optimization

```bash
# Analyze costs
$ mediagit stats --cost-analysis

# Identify optimization opportunities
$ mediagit gc
$ mediagit stats --cost-analysis  # Compare savings
```

## Notes

### Accuracy

Statistics are accurate as of last gc:
- Run `mediagit gc` for most accurate stats
- Statistics cached for performance (5 min TTL)
- Use `--force` to bypass cache

### Media Repository Benefits

For media-heavy repositories, stats reveal:
- Compression effectiveness by media type
- Deduplication opportunities
- Storage cost trends
- Growth patterns

Helps optimize:
- Compression settings
- Storage backend selection
- Garbage collection schedule
- Cost management

## See Also

- [mediagit gc](./gc.md) - Optimize repository
- [mediagit fsck](./fsck.md) - Verify integrity
- [mediagit verify](./verify.md) - Verify specific objects
- [mediagit log](./log.md) - Show commit history with storage stats
