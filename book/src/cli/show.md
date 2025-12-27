# mediagit show

Show commit details and contents.

## Synopsis

```bash
mediagit show [OPTIONS] [<object>...]
```

## Description

Shows one or more objects (commits, trees, blobs, tags). For commits, shows:
- Commit metadata (author, date, message)
- Full diff of changes introduced
- Compression and deduplication statistics
- Storage impact analysis

MediaGit show provides enhanced insights for media-heavy repositories including detailed compression metrics, deduplication analysis, and media-specific metadata.

## Options

### Output Format

#### `--pretty=<format>`
Pretty-print commit. Options:
- **oneline**: Compact single-line format
- **short**: Brief format with commit and author
- **medium**: Standard format (default)
- **full**: Full format with author and committer
- **fuller**: Like full but with dates
- **raw**: Raw commit object format

#### `--format=<format>`
Custom format string (see [mediagit log](./log.md) for placeholders).

#### `--abbrev-commit`
Show abbreviated commit OID.

#### `--no-abbrev-commit`
Show full 64-character SHA-256 OID.

### Diff Options

#### `-p`, `--patch`
Show patch (default for commits).

#### `-s`, `--no-patch`
Suppress diff output.

#### `--stat[=<width>[,<name-width>[,<count>]]]`
Show diffstat.

#### `--shortstat`
Show only summary line of --stat.

#### `--name-only`
Show only names of changed files.

#### `--name-status`
Show names and status of changed files.

#### `-U<n>`, `--unified=<n>`
Generate diffs with N lines of context.

### MediaGit-Specific Options

#### `--compression-details`
Show detailed compression statistics for the commit.

#### `--dedup-analysis`
Show deduplication analysis and chunk reuse information.

#### `--storage-impact`
Show storage impact and object database statistics.

#### `--media-metadata`
Show enhanced metadata for media files.

## Examples

### Show latest commit

```bash
$ mediagit show
commit a3c8f9d2e1b4f6a8c5d7e9f1a2b3c4d5e6f7a8b9c0d1e2f3a4b5c6d7e8f9a0b1
Author: Alice Developer <alice@example.com>
Date:   Mon Jan 15 14:30:22 2024 -0800

    Add promotional video assets

    Uploaded 5 new video files for Q1 marketing campaign.
    Includes various resolution versions and format variants.

    Compression: 410.2 MB → 75.9 MB (81.5% savings)
    Deduplication: 8 chunks (12.3 MB saved)
    Files: 5 added

diff --git a/videos/promo_1080p.mp4 b/videos/promo_1080p.mp4
new file mode 100644
index 0000000..a3c8f9d
Binary file videos/promo_1080p.mp4 added
Size: 245.8 MB (compressed to 42.1 MB, 82.9% savings)

diff --git a/videos/promo_4k.mp4 b/videos/promo_4k.mp4
new file mode 100644
index 0000000..b4d7e1a
Binary file videos/promo_4k.mp4 added
Size: 856.3 MB (compressed to 145.2 MB, 83.0% savings)

diff --git a/metadata.json b/metadata.json
index e4f3a2b..f5g4b3c 100644
--- a/metadata.json
+++ b/metadata.json
@@ -1,4 +1,9 @@
 {
+  "videos": [
+    "promo_1080p.mp4",
+    "promo_4k.mp4"
+  ],
   "version": "1.0"
 }
```

### Show specific commit

```bash
$ mediagit show b4d7e1a
commit b4d7e1a9f2c3d4e5f6a7b8c9d0e1f2a3b4c5d6e7f8a9b0c1d2e3f4a5b6c7d8e9
Author: Bob Designer <bob@example.com>
Date:   Sun Jan 14 09:15:47 2024 -0800

    Update brand identity assets

    - Updated logo files with new color scheme
    - Added high-res versions for print media
    - Removed deprecated logo variants

    Compression: 156.8 MB → 28.4 MB (81.9% savings)
    Files: 12 changed (8 added, 3 modified, 1 deleted)

diff --git a/assets/logo_old.png b/assets/logo_old.png
deleted file mode 100644
...
```

### Show without patch

```bash
$ mediagit show --no-patch
commit a3c8f9d2e1b4f6a8c5d7e9f1a2b3c4d5e6f7a8b9c0d1e2f3a4b5c6d7e8f9a0b1
Author: Alice Developer <alice@example.com>
Date:   Mon Jan 15 14:30:22 2024 -0800

    Add promotional video assets

    Compression: 410.2 MB → 75.9 MB (81.5% savings)
    Deduplication: 8 chunks (12.3 MB saved)
    Files: 5 added
```

### Show with stat

```bash
$ mediagit show --stat
commit a3c8f9d...
Author: Alice Developer <alice@example.com>
Date:   Mon Jan 15 14:30:22 2024 -0800

    Add promotional video assets

 videos/promo_1080p.mp4  | Binary: 245.8 MB (compressed to 42.1 MB)
 videos/promo_4k.mp4     | Binary: 856.3 MB (compressed to 145.2 MB)
 videos/promo_mobile.mp4 | Binary: 89.5 MB (compressed to 18.7 MB)
 assets/thumbnail.jpg    | Binary: 2.4 MB (compressed to 0.8 MB)
 metadata.json           | 5 insertions(+)
 5 files changed (4 added, 1 modified)

 Storage impact:
   Original size: 410.2 MB
   Compressed size: 75.9 MB (81.5% savings)
   Net new objects: 3,847
   Reused objects: 142
```

### Show with compression details

```bash
$ mediagit show --compression-details a3c8f9d
commit a3c8f9d...
Author: Alice Developer <alice@example.com>
Date:   Mon Jan 15 14:30:22 2024 -0800

    Add promotional video assets

Compression Details:
  Algorithm: Zstd (level 3)

  Per-file breakdown:
    videos/promo_1080p.mp4:
      Original: 245.8 MB
      Compressed: 42.1 MB
      Ratio: 82.9%
      Chunks: 63 (4 MB each)
      Compression time: 2.3s

    videos/promo_4k.mp4:
      Original: 856.3 MB
      Compressed: 145.2 MB
      Ratio: 83.0%
      Chunks: 215 (4 MB each)
      Compression time: 8.1s

    videos/promo_mobile.mp4:
      Original: 89.5 MB
      Compressed: 18.7 MB
      Ratio: 79.1%
      Chunks: 23 (4 MB each)
      Compression time: 0.9s

  Aggregate statistics:
    Total original: 410.2 MB
    Total compressed: 75.9 MB
    Average ratio: 81.5%
    Total compression time: 11.3s
    Compression throughput: 36.3 MB/s
```

### Show with deduplication analysis

```bash
$ mediagit show --dedup-analysis a3c8f9d
commit a3c8f9d...
Author: Alice Developer <alice@example.com>
Date:   Mon Jan 15 14:30:22 2024 -0800

    Add promotional video assets

Deduplication Analysis:
  Total chunks processed: 301
  Unique chunks: 293
  Duplicate chunks found: 8
  Deduplication ratio: 2.7%

  Duplicate chunk details:
    Chunk a8b9c0d1... (4 MB):
      Found in: promo_1080p.mp4, promo_4k.mp4
      Reused 1 time
      Savings: 4.0 MB

    Chunk b9c0d1e2... (4 MB):
      Found in: promo_1080p.mp4, promo_mobile.mp4
      Reused 1 time
      Savings: 4.0 MB

    [6 more chunks...]

  Total space saved: 12.3 MB
  Effective compression: 84.2% (including deduplication)

  Object database impact:
    Chunks already in ODB: 142
    New unique chunks: 293
    Total ODB chunks: 8,582 → 8,875 (+293)
```

### Show with storage impact

```bash
$ mediagit show --storage-impact a3c8f9d
commit a3c8f9d...
Author: Alice Developer <alice@example.com>
Date:   Mon Jan 15 14:30:22 2024 -0800

    Add promotional video assets

Storage Impact Analysis:

  Size changes:
    New files: 5
    Total original size: 410.2 MB
    Total compressed: 75.9 MB
    Compression savings: 334.3 MB (81.5%)
    Deduplication savings: 12.3 MB (3.0%)
    Net storage increase: 63.6 MB

  Object database:
    Objects before: 5,028
    Objects after: 8,875
    New objects: 3,847
    Reused objects: 142
    Object growth: +76.5%

  Storage backend (AWS S3):
    Region: us-west-2
    Bucket: mediagit-prod-assets
    New objects written: 3,847
    Data transferred: 63.6 MB
    Estimated cost: $0.0015/month

  Repository totals:
    Total objects: 8,875
    Total size: 3.2 GB → 485.3 MB (84.8% compression)
    Repository growth: +63.6 MB (+15.1%)
```

### Show media metadata

```bash
$ mediagit show --media-metadata a3c8f9d
commit a3c8f9d...
Author: Alice Developer <alice@example.com>
Date:   Mon Jan 15 14:30:22 2024 -0800

    Add promotional video assets

Media File Analysis:

  videos/promo_1080p.mp4:
    Type: Video (H.264/AVC)
    Duration: 00:03:45
    Resolution: 1920x1080 (Full HD)
    Frame rate: 29.97 fps
    Bitrate: 8.5 Mbps
    Audio: AAC, 128 kbps, Stereo
    File size: 245.8 MB → 42.1 MB (compressed)

  videos/promo_4k.mp4:
    Type: Video (H.265/HEVC)
    Duration: 00:03:45
    Resolution: 3840x2160 (4K UHD)
    Frame rate: 29.97 fps
    Bitrate: 28.3 Mbps
    Audio: AAC, 256 kbps, Stereo
    File size: 856.3 MB → 145.2 MB (compressed)

  videos/promo_mobile.mp4:
    Type: Video (H.264/AVC)
    Duration: 00:03:45
    Resolution: 1280x720 (HD)
    Frame rate: 29.97 fps
    Bitrate: 3.2 Mbps
    Audio: AAC, 96 kbps, Stereo
    File size: 89.5 MB → 18.7 MB (compressed)

  assets/thumbnail.jpg:
    Type: Image (JPEG)
    Resolution: 1920x1080
    Color space: sRGB
    Quality: 90
    File size: 2.4 MB → 0.8 MB (compressed)
```

### Show specific file from commit

```bash
$ mediagit show a3c8f9d:metadata.json
{
  "videos": [
    "promo_1080p.mp4",
    "promo_4k.mp4",
    "promo_mobile.mp4"
  ],
  "campaign": "Q1 2024",
  "version": "1.0"
}
```

### Show multiple commits

```bash
$ mediagit show b4d7e1a a3c8f9d
commit b4d7e1a...
Author: Bob Designer <bob@example.com>
Date:   Sun Jan 14 09:15:47 2024 -0800

    Update brand identity assets
...

commit a3c8f9d...
Author: Alice Developer <alice@example.com>
Date:   Mon Jan 15 14:30:22 2024 -0800

    Add promotional video assets
...
```

### Show tree object

```bash
$ mediagit show HEAD^{tree}
tree fb3a8bdd0ceddd019615af4d57a53f43d8cee2bf

videos/
assets/
metadata.json
README.md
```

### Show blob object

```bash
$ mediagit show a3c8f9d:README.md
# Media Project

This project contains marketing assets for Q1 2024 campaign.

## Contents

- Promotional videos in various resolutions
- Brand identity assets
- Product photography
```

### One-line format

```bash
$ mediagit show --oneline --no-patch
a3c8f9d Add promotional video assets
```

### Name and status

```bash
$ mediagit show --name-status
commit a3c8f9d...
Author: Alice Developer <alice@example.com>
Date:   Mon Jan 15 14:30:22 2024 -0800

    Add promotional video assets

A       videos/promo_1080p.mp4
A       videos/promo_4k.mp4
A       videos/promo_mobile.mp4
A       assets/thumbnail.jpg
M       metadata.json
```

## Object Types

### Commits

Default behavior shows commit details with diff:
```bash
$ mediagit show <commit>
```

### Trees

Shows directory listing:
```bash
$ mediagit show <commit>^{tree}
$ mediagit show <commit>:path/to/directory/
```

### Blobs

Shows file contents:
```bash
$ mediagit show <commit>:path/to/file
```

### Tags

Shows tag information and tagged object:
```bash
$ mediagit show v1.0.0
```

## Revision Syntax

```bash
# Show HEAD
$ mediagit show HEAD

# Show parent commit
$ mediagit show HEAD~1
$ mediagit show HEAD^

# Show specific commit
$ mediagit show a3c8f9d

# Show branch tip
$ mediagit show main

# Show tag
$ mediagit show v1.0.0

# Show file at commit
$ mediagit show HEAD:path/to/file
```

## Performance

MediaGit show is optimized for large repositories:
- **Lazy loading**: Load only requested objects
- **Chunk caching**: Reuse decompressed chunks
- **Parallel processing**: Multi-threaded diff generation
- **Smart formatting**: Stream output progressively

Typical performance:
- **Commit metadata**: < 10ms
- **Small diffs**: < 100ms
- **Large diffs** (100+ files): < 2s
- **Media analysis**: 50-100 MB/s

## Exit Status

- **0**: Object shown successfully
- **1**: Object not found or invalid
- **2**: Error accessing repository

## Configuration

```toml
[show]
# Default format
format = "medium"

# Show patch by default
patch = true

# Abbreviate OIDs
abbrev_commit = false

[show.compression]
# Show compression details by default
details = false

# Show deduplication analysis
dedup = false

# Show storage impact
storage_impact = false

[show.media]
# Show media metadata
metadata = false

# Media analysis depth
analysis = "basic"  # basic | detailed | full
```

## Notes

### Large File Handling

For very large files in commits:
- Use `--no-patch` to skip diff generation
- Use `--stat` for summary only
- Metadata shown even for large binaries

### Object Addressing

MediaGit uses SHA-256 for all objects:
- Full OID: 64 hexadecimal characters
- Short OID: First 7-12 characters (configurable)
- Automatic disambiguation for short OIDs

### Media File Support

MediaGit provides enhanced display for:
- **Videos**: Codec, resolution, bitrate, duration
- **Images**: Resolution, format, color space
- **Audio**: Sample rate, bit depth, channels
- **Documents**: Page count, format version (when available)

## See Also

- [mediagit log](./log.md) - Show commit history
- [mediagit diff](./diff.md) - Show changes between commits
- [mediagit cat-file](./cat-file.md) - Show raw object contents
- [mediagit ls-tree](./ls-tree.md) - List tree contents
- [mediagit blame](./blame.md) - Show last modification for each line
