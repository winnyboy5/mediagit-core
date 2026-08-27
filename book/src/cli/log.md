# mediagit log

Show commit history.

## Synopsis

```bash
mediagit log [OPTIONS] [<revision>] [[--] <path>...]
```

## Description

Shows the commit logs, displaying commit history with metadata, messages, and statistics. MediaGit log provides enhanced insights including compression metrics, deduplication statistics, and storage efficiency trends over time.

The log output shows commits in reverse chronological order by default, with the most recent commits appearing first.

## Options

### Output Format

#### `--oneline`
Condensed output showing one commit per line with short OID and message.

#### `--format=<template>`
Format each commit using a template string. Supported placeholders: `%H` (full BLAKE3 OID), `%h` (short OID), `%s` (subject), `%aN`/`%an` (author name), `%ae` (author email), `%ad` (author date), `%n` (newline).

#### `-n <number>`, `--max-count=<number>`
Limit number of commits to show.

#### `--skip=<number>`
Skip the first N commits before showing output.

#### `--since=<date>`
Show commits more recent than specified date.

#### `--until=<date>`
Show commits older than specified date.

#### `--author=<pattern>`
Filter commits by author name or email.

#### `--grep=<pattern>`
Filter commits by message content.

#### `--all`
Show commits from all branches.

#### `-- <path>...`
Show only commits affecting specified paths.

### Display Options

#### `--graph`
Draw ASCII graph showing branch and merge history.

#### `--stat`
Show file change statistics for each commit.

#### `--patch`, `-p`
Show patch (diff) for each commit.

## Format Placeholders

Custom format strings support these placeholders:

| Placeholder | Meaning |
|-------------|---------|
| `%H` | Full BLAKE3 commit OID (64 hex chars) |
| `%h` | Abbreviated OID (first 7 chars) |
| `%s` | Subject (first line of message) |
| `%aN`, `%an` | Author name |
| `%ae` | Author email |
| `%ad` | Author date |
| `%n` | Literal newline |
| `%%` | Literal `%` |

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
Date:   2024-01-15 14:30:22

    Add promotional video assets

 videos/promo_1080p.mp4 | new file
 videos/promo_4k.mp4    | new file
 videos/promo_mobile.mp4 | new file
 assets/thumbnail.jpg   | new file
 metadata.json          | modified
 5 file(s) changed, 4 added, 1 modified, 0 deleted
```

### Custom format

```bash
$ mediagit log --format="%h - %an : %s" -n 3
a3c8f9d - Alice Developer : Add promotional video assets
b4d7e1a - Bob Designer : Update brand identity assets
c5e9f2b - Alice Developer : Quick fix: correct video resolution
```

### Search commit messages

```bash
$ mediagit log --grep="video" --oneline
a3c8f9d Add promotional video assets
c5e9f2b Quick fix: correct video resolution
h0j4e7g Add training video series
```

### Branch history

```bash
$ mediagit log feature/video-opt --oneline
c5e9f2b Optimize video encoding parameters
i1k5f8h Add batch processing script
j2l6g9i Update compression profiles
```

## Starting Revisions

`log` walks history from a single starting revision (branch, tag, commit
hash, `HEAD~N`, or a full ref path). Git-style `A..B` / `A...B` range
syntax is **not supported**.

```bash
# Commits reachable from a branch
$ mediagit log feature/video-opt

# Commits reachable from a tag
$ mediagit log v1.0.0

# Starting N commits back
$ mediagit log HEAD~5

# All branches
$ mediagit log --all
```

## Exit Status

- **0**: Success, commits displayed
- **1**: Error accessing repository or objects

## Notes

### Performance Tips

For very large histories:
```bash
# Limit depth
$ mediagit log -n 100

# Filter early
$ mediagit log --since="1 month ago"
```

### Commit OIDs

All commit OIDs are BLAKE3 hashes (64 hex characters). `--oneline` displays the
first 7 characters; `--format=%H` prints the full 64-character OID.


## Common options

Accepted by this command in addition to the options above.

| Flag | Description |
|---|---|
| `--color <WHEN>` | Colored output: `always`, `auto`, or `never` (default `auto`) |
| `-C`, `--repository <PATH>` | Run as if invoked in `PATH` |
| `-q`, `--quiet` | Suppress output |
| `-v`, `--verbose` | Enable verbose output |
| `-h`, `--help` | Print help |
| `-V`, `--version` | Print version |
## See Also

- [mediagit show](./show.md) - Show commit details
- [mediagit diff](./diff.md) - Show changes between commits
- [mediagit branch](./branch.md) - List, create, or delete branches
- [mediagit reflog](./reflog.md) - Show reference log
