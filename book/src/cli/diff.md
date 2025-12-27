# mediagit diff

Show changes between commits, working tree, and staging area.

## Synopsis

```bash
mediagit diff [OPTIONS] [<commit>] [<commit>] [--] [<path>...]
```

## Description

Shows differences between various states in MediaGit:
- Working tree vs staging area (default)
- Staging area vs last commit (`--staged`)
- Between two commits
- Working tree vs specific commit

MediaGit diff provides media-aware comparison including:
- Visual diff for images (pixel-level comparison)
- Audio waveform comparison
- Video metadata and codec differences
- Binary file size changes with compression impact

## Options

### Diff Selection

#### `--staged`, `--cached`
Show changes staged for commit (between staging area and HEAD).

#### `<commit>`
Show changes between working tree and specified commit.

#### `<commit> <commit>`
Show changes between two commits.

#### `-- <path>...`
Limit diff to specified paths.

### Output Format

#### `-p`, `--patch`
Generate patch format (default).

#### `-s`, `--no-patch`
Suppress diff output, show only summary.

#### `--raw`
Generate raw diff format.

#### `--stat[=<width>[,<name-width>[,<count>]]]`
Generate diffstat showing file changes summary.

#### `--shortstat`
Output only last line of --stat format.

#### `--summary`
Output condensed summary of extended header information.

#### `--patch-with-stat`
Output patch along with stat.

### Unified Context

#### `-U<n>`, `--unified=<n>`
Generate diffs with N lines of context (default 3).

#### `--no-prefix`
Do not show "a/" and "b/" prefixes in diff output.

#### `--src-prefix=<prefix>`
Show given source prefix instead of "a/".

#### `--dst-prefix=<prefix>`
Show given destination prefix instead of "b/".

### Comparison Options

#### `-b`, `--ignore-space-change`
Ignore changes in amount of whitespace.

#### `-w`, `--ignore-all-space`
Ignore whitespace when comparing lines.

#### `--ignore-blank-lines`
Ignore changes whose lines are all blank.

#### `--binary`
Output binary diffs for binary files.

### MediaGit-Specific Options

#### `--media-diff`
Enable enhanced media file comparison:
- Image: Visual diff with pixel changes
- Audio: Waveform comparison
- Video: Metadata and codec changes

#### `--compression-impact`
Show compression impact of changes.

#### `--size-analysis`
Display detailed size analysis for binary files.

#### `--visual`
Generate visual representations for media files (requires external tools).

### Display Options

#### `--color[=<when>]`
Show colored diff. When: **always**, **never**, **auto** (default).

#### `--no-color`
Turn off colored diff.

#### `--word-diff[=<mode>]`
Show word-level diff. Mode: **color**, **plain**, **porcelain**, **none**.

#### `--name-only`
Show only names of changed files.

#### `--name-status`
Show names and status of changed files.

## Examples

### Working tree vs staging area

```bash
$ mediagit diff
diff --git a/video.mp4 b/video.mp4
index a3c8f9d..b4d7e1a 100644
Binary files a/video.mp4 and b/video.mp4 differ
Size: 245.8 MB → 256.3 MB (+10.5 MB)
Compression impact: +1.8 MB (compressed)

diff --git a/config.json b/config.json
index e4f3a2b..f5g4b3c 100644
--- a/config.json
+++ b/config.json
@@ -12,7 +12,7 @@
   "output": {
-    "format": "mp4",
+    "format": "webm",
     "quality": "high"
   }
```

### Staged changes

```bash
$ mediagit diff --staged
diff --git a/assets/logo.png b/assets/logo.png
new file mode 100644
index 0000000..a3c8f9d
Binary file assets/logo.png added
Size: 156.3 KB (compressed to 45.2 KB)

diff --git a/README.md b/README.md
index b4d7e1a..c5e9f2b 100644
--- a/README.md
+++ b/README.md
@@ -1,3 +1,5 @@
 # Media Project

+This project contains marketing assets.
+
 ## Usage
```

### Compare commits

```bash
$ mediagit diff HEAD~2 HEAD
diff --git a/video_old.mp4 b/video_old.mp4
deleted file mode 100644
index a3c8f9d..0000000
Binary file video_old.mp4 removed
Size: 245.8 MB (freed: 42.1 MB compressed)

diff --git a/video_new.mp4 b/video_new.mp4
new file mode 100644
index 0000000..f5g4b3c
Binary file video_new.mp4 added
Size: 312.5 MB (compressed to 56.3 MB)

Net storage impact: +14.2 MB
```

### Stat summary

```bash
$ mediagit diff --stat
 video.mp4       | Binary file: 245.8 MB → 256.3 MB (+10.5 MB)
 config.json     | 2 +-
 README.md       | 3 +++
 assets/logo.png | Binary file: 156.3 KB added
 4 files changed, 4 insertions(+), 1 deletion(-)
```

### Name and status only

```bash
$ mediagit diff --name-status
M       video.mp4
M       config.json
M       README.md
A       assets/logo.png
```

### Specific path

```bash
$ mediagit diff -- video.mp4
diff --git a/video.mp4 b/video.mp4
index a3c8f9d..b4d7e1a 100644
Binary files a/video.mp4 and b/video.mp4 differ
Size: 245.8 MB → 256.3 MB (+10.5 MB)
Compression impact: +1.8 MB (compressed)

Metadata changes:
  Duration: 00:03:45 → 00:03:52 (+7 seconds)
  Resolution: 1920x1080 (unchanged)
  Codec: H.264 → H.265 (HEVC)
  Bitrate: 8.5 Mbps → 7.8 Mbps (-0.7 Mbps)
```

### Media-aware diff

```bash
$ mediagit diff --media-diff image.jpg
diff --git a/image.jpg b/image.jpg
index e4f3a2b..f5g4b3c 100644
--- a/image.jpg
+++ b/image.jpg
@@ Image comparison:
  Size: 2.4 MB → 2.6 MB (+0.2 MB)
  Dimensions: 3840x2160 → 4096x2304 (+6.5% pixels)
  Format: JPEG (quality 90) → JPEG (quality 95)

  Pixel differences:
    Changed pixels: 127,483 (1.4% of image)
    Average color difference: 12.3 (ΔE)
    Significant changes in regions:
      - Top-left quadrant: +15.2% brightness
      - Bottom-right corner: Color shift (+8 hue)

  Compression impact: +0.3 MB (compressed)
```

### Audio comparison

```bash
$ mediagit diff --media-diff audio.wav
diff --git a/audio.wav b/audio.wav
index a3c8f9d..b4d7e1a 100644
--- a/audio.wav
+++ b/audio.wav
@@ Audio comparison:
  Size: 45.2 MB → 47.8 MB (+2.6 MB)
  Duration: 00:04:32 → 00:04:32 (unchanged)
  Sample rate: 44.1 kHz → 48 kHz
  Bit depth: 16-bit → 24-bit
  Channels: Stereo (unchanged)

  Waveform analysis:
    RMS level change: -2.3 dB (quieter)
    Peak amplitude: -0.1 dB → -0.3 dB
    Dynamic range: 18.2 dB → 22.5 dB (improved)

  Compression impact: +0.8 MB (compressed with Zstd)
```

### Compression impact analysis

```bash
$ mediagit diff --compression-impact --stat
 video_1.mp4   | Binary: 245.8 MB → 256.3 MB | Compressed: +1.8 MB
 video_2.mp4   | Binary: 312.5 MB → 298.7 MB | Compressed: -2.3 MB
 image.jpg     | Binary: 2.4 MB → 2.6 MB     | Compressed: +0.3 MB
 audio.wav     | Binary: 45.2 MB → 47.8 MB   | Compressed: +0.8 MB
 config.json   | 2 lines changed             | Compressed: +12 bytes (delta)

 Total impact:
   Original size change: +18.9 MB
   Compressed size change: +0.6 MB (96.8% compression efficiency)
   Deduplication opportunities: 3 chunks (4.2 MB potential savings)
```

### Size analysis

```bash
$ mediagit diff --size-analysis video.mp4
diff --git a/video.mp4 b/video.mp4
index a3c8f9d..b4d7e1a 100644

Size Analysis:
  Original files:
    Before: 245.8 MB
    After:  256.3 MB
    Change: +10.5 MB (+4.3%)

  Compressed (Zstd level 3):
    Before: 42.1 MB (82.9% compression)
    After:  43.9 MB (82.9% compression)
    Change: +1.8 MB (+4.3%)

  Storage breakdown:
    Video stream: +10.2 MB
    Audio stream: +0.2 MB
    Metadata: +0.1 MB

  Chunk analysis:
    Total chunks: 63 → 65 (+2 chunks)
    Dedup chunks: 15 → 18 (+3 reused)
    New unique: 48 → 47 (-1 unique)
    Net new storage: +1.8 MB
```

### Word-level diff

```bash
$ mediagit diff --word-diff config.json
diff --git a/config.json b/config.json
index e4f3a2b..f5g4b3c 100644
--- a/config.json
+++ b/config.json
@@ -12,7 +12,7 @@
   "output": {
    "format": [-"mp4"-]{+"webm"+},
    "quality": "high"
  }
```

### Branch comparison

```bash
$ mediagit diff main..feature/optimize
diff --git a/video.mp4 b/video.mp4
index a3c8f9d..b4d7e1a 100644
Binary files differ
Size: 245.8 MB → 198.3 MB (-47.5 MB, 19.3% reduction)

Optimization applied:
  Codec: H.264 → H.265 (better compression)
  Bitrate: 8.5 Mbps → 7.2 Mbps
  Quality loss: Minimal (PSNR > 45 dB)

Compression impact: -8.1 MB compressed (-19.2%)
```

## Comparing Specific States

### Working tree vs HEAD

```bash
$ mediagit diff HEAD
# Shows unstaged and staged changes
```

### Staging area vs HEAD

```bash
$ mediagit diff --staged
# Shows only staged changes
```

### Working tree vs staging area

```bash
$ mediagit diff
# Shows only unstaged changes (default)
```

### Two branches

```bash
$ mediagit diff branch1..branch2
$ mediagit diff branch1...branch2  # symmetric difference
```

## Status Codes

When using `--name-status`:

```
A   Added
M   Modified
D   Deleted
R   Renamed
C   Copied
T   Type changed
U   Unmerged
```

## Performance

MediaGit diff is optimized for media files:
- **Smart binary detection**: Skip detailed diff for large binaries
- **Chunk-level comparison**: Fast comparison using content-addressable chunks
- **Parallel processing**: Multi-threaded diff generation
- **Incremental loading**: Stream large diffs progressively

Typical performance:
- **Text files**: ~100MB/s
- **Binary files** (basic): ~500MB/s
- **Media analysis**: ~50MB/s
- **Visual diff**: ~10MB/s (CPU-intensive)

## Exit Status

- **0**: No differences found
- **1**: Differences found
- **2**: Error during diff operation

## Configuration

```toml
[diff]
# Enable/disable color
color = "auto"  # auto | always | never

# Context lines
context = 3

# Rename detection
renames = true

# Binary file handling
binary = true

[diff.media]
# Enable media-aware diff
enabled = true

# Include metadata analysis
metadata = true

# Visual comparison for images
visual = false  # CPU-intensive

# Audio waveform comparison
waveform = false  # requires ffmpeg

[diff.compression]
# Show compression impact
show_impact = true

# Size analysis detail level
size_detail = "summary"  # summary | detailed | full
```

## Media File Handling

### Images

MediaGit can compare images at various levels:
```bash
# Basic size comparison (fast)
$ mediagit diff image.jpg

# Pixel-level comparison
$ mediagit diff --media-diff image.jpg

# Visual diff (generates side-by-side comparison)
$ mediagit diff --visual image.jpg
```

### Videos

Video comparison includes:
- Metadata changes (codec, resolution, bitrate)
- Duration differences
- Quality metrics (when available)
- File size and compression impact

### Audio

Audio comparison features:
- Format and encoding changes
- Sample rate and bit depth
- Waveform analysis (requires ffmpeg)
- RMS and peak levels

## Notes

### Binary File Diffs

Binary files show size changes by default:
```
Binary files a/video.mp4 and b/video.mp4 differ
Size: 245.8 MB → 256.3 MB (+10.5 MB)
```

Use `--binary` to see binary patch format (useful for small binaries).

### Large File Performance

For very large files, diff uses:
- Chunk-based comparison (fast)
- Sampling for preview (configurable)
- Metadata-only mode for quick checks

### Rename Detection

MediaGit automatically detects file renames:
```bash
$ mediagit diff --name-status
R100    old_video.mp4 -> new_video.mp4
```

The number indicates similarity percentage (100 = identical content).

## See Also

- [mediagit status](./status.md) - Show working tree status
- [mediagit log](./log.md) - Show commit history
- [mediagit show](./show.md) - Show commit details
- [mediagit add](./add.md) - Add files to staging area
- [mediagit restore](./restore.md) - Restore working tree files
