# mediagit log

Show commit history.

## Synopsis

```bash
mediagit log [OPTIONS] [<revision-range>] [[--] <path>...]
```

## Description

Shows the commit logs, displaying commit history with metadata, messages, and statistics. MediaGit log provides enhanced insights including compression metrics, deduplication statistics, and storage efficiency trends over time.

The log output shows commits in reverse chronological order by default, with the most recent commits appearing first.

## Options

### Output Format

#### `--oneline`
Condensed output showing one commit per line with short OID and message.

#### `--pretty=<format>`
Use custom format. Options:
- **oneline**: Compact single-line format
- **short**: Brief format with commit and author
- **medium**: Standard format with full details (default)
- **full**: Full format with author and committer
- **fuller**: Like full but with dates
- **raw**: Raw commit object format
- **format:<string>**: Custom format string

#### `--format=<format>`
Alias for `--pretty=format:<format>`.

#### `--abbrev-commit`
Show abbreviated commit OIDs (short form).

#### `--no-abbrev-commit`
Show full 64-character SHA-256 commit OIDs.

### Filtering

#### `-n <number>`, `--max-count=<number>`
Limit number of commits to show.

#### `--skip=<number>`
Skip the first N commits before showing output.

#### `--since=<date>`, `--after=<date>`
Show commits more recent than specified date.

#### `--until=<date>`, `--before=<date>`
Show commits older than specified date.

#### `--author=<pattern>`
Filter commits by author name or email.

#### `--committer=<pattern>`
Filter commits by committer name or email.

#### `--grep=<pattern>`
Filter commits by message content.

#### `-i`, `--regexp-ignore-case`
Case-insensitive pattern matching for --grep, --author, --committer.

### Content Filtering

#### `--all`
Show commits from all branches.

#### `--branches[=<pattern>]`
Show commits from matching branches.

#### `--tags[=<pattern>]`
Show commits from matching tags.

#### `-- <path>...`
Show only commits affecting specified paths.

### Display Options

#### `--graph`
Draw ASCII graph showing branch and merge history.

#### `--decorate[=<mode>]`
Show branch and tag names. Mode: **short**, **full**, **auto**, **no**.

#### `--stat[=<width>[,<name-width>[,<count>]]]`
Show diffstat for each commit.

#### `--shortstat`
Show only summary line from --stat.

#### `--name-only`
Show names of changed files.

#### `--name-status`
Show names and status of changed files.

#### `--patch`, `-p`
Show patch (diff) for each commit.

#### `--no-patch`
Suppress patch output.

### MediaGit-Specific Options

#### `--compression-stats`
Show compression and deduplication statistics for each commit.

#### `--storage-trend`
Display storage efficiency trends over time.

#### `--media-only`
Show only commits affecting media files.

#### `--backend-info`
Include storage backend information in output.

## Format Placeholders

Custom format strings support placeholders:

### Commit Information
- `%H`: Full commit hash (SHA-256)
- `%h`: Abbreviated commit hash
- `%T`: Tree hash
- `%t`: Abbreviated tree hash
- `%P`: Parent hashes
- `%p`: Abbreviated parent hashes

### Author/Committer
- `%an`: Author name
- `%ae`: Author email
- `%ad`: Author date
- `%ar`: Author date, relative
- `%cn`: Committer name
- `%ce`: Committer email
- `%cd`: Committer date
- `%cr`: Committer date, relative

### Message
- `%s`: Subject (first line)
- `%b`: Body
- `%B`: Raw body (subject and body)

### MediaGit Extensions
- `%Cs`: Compression savings
- `%Dd`: Deduplication savings
- `%Sz`: Total storage size
- `%Fc`: File count

## Examples

### Basic log

```bash
$ mediagit log
commit a3c8f9d2e1b4f6a8c5d7e9f1a2b3c4d5e6f7a8b9c0d1e2f3a4b5c6d7e8f9a0b1
Author: Alice Developer <alice@example.com>
Date:   Mon Jan 15 14:30:22 2024 -0800

    Add promotional video assets

    Uploaded 5 new video files for Q1 marketing campaign.
    Includes various resolution versions and format variants.

    Compression: 410.2 MB → 75.9 MB (81.5% savings)
    Deduplication: 8 chunks (12.3 MB saved)
    Files: 5 added

commit b4d7e1a9f2c3d4e5f6a7b8c9d0e1f2a3b4c5d6e7f8a9b0c1d2e3f4a5b6c7d8e9
Author: Bob Designer <bob@example.com>
Date:   Sun Jan 14 09:15:47 2024 -0800

    Update brand identity assets

    - Updated logo files with new color scheme
    - Added high-res versions for print media
    - Removed deprecated logo variants

    Compression: 156.8 MB → 28.4 MB (81.9% savings)
    Files: 12 changed (8 added, 3 modified, 1 deleted)
```

### One-line format

```bash
$ mediagit log --oneline
a3c8f9d Add promotional video assets
b4d7e1a Update brand identity assets
c5e9f2b Quick fix: correct video resolution
d6f0a3c Initial commit with base assets
```

### Limited count

```bash
$ mediagit log -n 3
commit a3c8f9d...
...

commit b4d7e1a...
...

commit c5e9f2b...
...
```

### Graph view

```bash
$ mediagit log --graph --oneline --all
* a3c8f9d (HEAD -> main) Add promotional video assets
* b4d7e1a Update brand identity assets
| * c5e9f2b (feature/video-opt) Optimize video encoding
|/
* d6f0a3c Initial commit with base assets
```

### Date range filtering

```bash
$ mediagit log --since="2 weeks ago" --until="3 days ago"
commit b4d7e1a...
Author: Bob Designer <bob@example.com>
Date:   Sun Jan 14 09:15:47 2024 -0800

    Update brand identity assets
...
```

### Author filtering

```bash
$ mediagit log --author="Alice" --oneline
a3c8f9d Add promotional video assets
e7g1b4d Add product photography
f8h2c5e Major redesign of homepage assets
```

### Path-specific history

```bash
$ mediagit log -- videos/
commit a3c8f9d...
Author: Alice Developer <alice@example.com>
Date:   Mon Jan 15 14:30:22 2024 -0800

    Add promotional video assets

    Files in videos/ directory:
    - videos/promo_1080p.mp4 (added)
    - videos/promo_4k.mp4 (added)
    - videos/promo_mobile.mp4 (added)
```

### Stat output

```bash
$ mediagit log --stat -n 1
commit a3c8f9d2e1b4f6a8c5d7e9f1a2b3c4d5e6f7a8b9c0d1e2f3a4b5c6d7e8f9a0b1
Author: Alice Developer <alice@example.com>
Date:   Mon Jan 15 14:30:22 2024 -0800

    Add promotional video assets

 videos/promo_1080p.mp4  | 245.8 MB → 42.1 MB (82.9% savings)
 videos/promo_4k.mp4     | 856.3 MB → 145.2 MB (83.0% savings)
 videos/promo_mobile.mp4 | 89.5 MB → 18.7 MB (79.1% savings)
 assets/thumbnail.jpg    | 2.4 MB → 0.8 MB (66.7% savings)
 metadata.json           | 1.2 KB → 0.4 KB (delta)
 5 files changed, 3 insertions(+), 0 deletions(-)
```

### Custom format

```bash
$ mediagit log --format="%h - %an, %ar : %s" -n 3
a3c8f9d - Alice Developer, 2 hours ago : Add promotional video assets
b4d7e1a - Bob Designer, 1 day ago : Update brand identity assets
c5e9f2b - Alice Developer, 3 days ago : Quick fix: correct video resolution
```

### Compression statistics

```bash
$ mediagit log --compression-stats -n 2
commit a3c8f9d...
Author: Alice Developer <alice@example.com>
Date:   Mon Jan 15 14:30:22 2024 -0800

    Add promotional video assets

    Compression Statistics:
      Algorithm: Zstd (level 3)
      Original size: 410.2 MB
      Compressed size: 75.9 MB
      Compression ratio: 81.5%

    Deduplication:
      Duplicate chunks found: 8
      Space saved: 12.3 MB
      Efficiency gain: 3.0%

    Delta Encoding:
      Delta-encoded files: 1 (metadata.json)
      Delta savings: 0.8 KB

    Total Storage Impact:
      New objects: 3,847
      Reused objects: 142
      Net storage increase: 63.6 MB

commit b4d7e1a...
...
```

### Storage trend

```bash
$ mediagit log --storage-trend --oneline -n 5
a3c8f9d Add promotional video assets      [Storage: +63.6 MB, Total: 1.2 GB]
b4d7e1a Update brand identity assets     [Storage: +12.4 MB, Total: 1.1 GB]
c5e9f2b Quick fix: correct video         [Storage: -2.1 MB, Total: 1.1 GB]
d6f0a3c Optimize existing media          [Storage: -45.8 MB, Total: 1.1 GB]
e7g1b4d Add product photography          [Storage: +8.9 MB, Total: 1.2 GB]

Storage Efficiency Trend:
  Average compression ratio: 82.3%
  Total deduplication savings: 267.4 MB
  Trend: Improving (+2.1% over last 10 commits)
```

### Media-only commits

```bash
$ mediagit log --media-only --oneline -n 5
a3c8f9d Add promotional video assets (5 video files)
e7g1b4d Add product photography (12 image files)
f8h2c5e Major redesign of homepage assets (28 media files)
g9i3d6f Apply design changes (7 image files)
h0j4e7g Add training video series (12 video files)
```

### Search commit messages

```bash
$ mediagit log --grep="video" --oneline
a3c8f9d Add promotional video assets
c5e9f2b Quick fix: correct video resolution
h0j4e7g Add training video series
```

### Branch comparison

```bash
$ mediagit log main..feature/video-opt --oneline
c5e9f2b Optimize video encoding parameters
i1k5f8h Add batch processing script
j2l6g9i Update compression profiles
```

## Output Formatting

### Decorations

```bash
$ mediagit log --oneline --decorate --graph -n 5
* a3c8f9d (HEAD -> main, origin/main) Add promotional video assets
* b4d7e1a Update brand identity assets
| * c5e9f2b (feature/video-opt) Optimize video encoding
|/
* d6f0a3c (tag: v1.0.0) Initial commit
```

### Name and status

```bash
$ mediagit log --name-status -n 1
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

## Revision Ranges

Show commits in a range:

```bash
# Commits in branch2 not in branch1
$ mediagit log branch1..branch2

# Commits in either branch, but not both
$ mediagit log branch1...branch2

# Commits reachable from tag
$ mediagit log v1.0.0

# Commits since tag
$ mediagit log v1.0.0..HEAD

# All commits not in origin/main
$ mediagit log origin/main..HEAD
```

## Performance

MediaGit log is optimized for large repositories:
- **Commit streaming**: Display results as they're found
- **Index utilization**: Fast commit traversal using commit graph
- **Parallel object loading**: Multi-threaded object database access
- **Smart caching**: Cache recent commits for faster repeated queries

Typical performance:
- **Unbounded log**: Streams immediately, ~5000 commits/second
- **Filtered log**: Depends on filter complexity
- **With --stat**: ~1000 commits/second
- **With --patch**: ~100 commits/second

## Exit Status

- **0**: Success, commits displayed
- **1**: Error accessing repository or objects
- **2**: Invalid options or revision range

## Configuration

```toml
[log]
# Default output format
format = "medium"  # oneline | short | medium | full

# Show decorations by default
decorate = "auto"  # auto | short | full | no

# Date format
date = "default"  # default | iso | rfc | relative | short

# Abbreviate commit OIDs
abbrev_commit = true

# Follow renames
follow = true

[log.compression]
# Show compression stats by default
show_stats = false

# Include deduplication info
show_dedup = false

# Show storage trends
show_trends = false
```

## Notes

### Media-Aware Features

MediaGit log provides additional insights for media repositories:
- **Format detection**: Identify video, image, audio files
- **Codec changes**: Track encoding parameter changes
- **Quality trends**: Monitor compression quality over time
- **Size analysis**: Identify commits with large storage impact

### Performance Optimization

For very large histories:
```bash
# Limit depth
$ mediagit log --max-count=100

# Skip expensive operations
$ mediagit log --no-patch --no-stat

# Filter early
$ mediagit log --since="1 month ago"
```

### Commit Graph

MediaGit builds a commit graph for fast traversal:
```bash
# Rebuild if corrupted
$ mediagit gc --build-commit-graph

# Verify integrity
$ mediagit fsck --commit-graph
```

## See Also

- [mediagit show](./show.md) - Show commit details
- [mediagit diff](./diff.md) - Show changes between commits
- [mediagit branch](./branch.md) - List, create, or delete branches
- [mediagit reflog](./reflog.md) - Show reference log
- [mediagit blame](./blame.md) - Show last modification for each line
