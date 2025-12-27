# mediagit commit

Record changes to the repository.

## Synopsis

```bash
mediagit commit [OPTIONS]
```

## Description

Creates a new commit containing the current contents of the staging area along with a log message describing the changes. The commit becomes the new tip of the current branch.

MediaGit commits include:
- SHA-256 hash of the commit tree
- Parent commit references
- Author and committer information
- Timestamp
- Commit message
- Compression and deduplication statistics

## Options

### `-m, --message <MESSAGE>`
**Required** (unless `--amend` or `--file` is used). Commit message describing the changes.

### `-F, --file <FILE>`
Read commit message from file instead of command line.

### `--amend`
Replace the tip of the current branch by creating a new commit with the previous commit's contents plus staged changes.

### `-a, --all`
Automatically stage all modified tracked files before committing.

### `--allow-empty`
Allow creating a commit with no changes.

### `-v, --verbose`
Show diff of changes being committed.

### `--author <AUTHOR>`
Override the commit author.

- **Format**: `"Name <email@example.com>"`

### `--date <DATE>`
Override the author date.

- **Format**: ISO 8601 or Unix timestamp

## Examples

### Basic commit
```bash
$ mediagit commit -m "Add promotional video assets"
[main a3c8f9d] Add promotional video assets
 5 files changed
 Compression ratio: 18.5% (saved 410.2 MB)
 Deduplication: 0 identical chunks
 Time: 2.3s
```

### Commit with detailed message
```bash
$ mediagit commit -m "Update brand identity assets

- Updated logo files with new color scheme
- Added high-res versions for print media
- Removed deprecated logo variants"
[main b4d7e1a] Update brand identity assets
 12 files changed (8 added, 3 modified, 1 deleted)
 Compression: 156.8 MB → 28.4 MB (81.9% savings)
```

### Commit all changes
```bash
$ mediagit commit -am "Quick fix: correct video resolution"
✓ Auto-staged 3 modified files
[main c5e9f2b] Quick fix: correct video resolution
 3 files changed
 Compression: 450.3 MB → 68.2 MB (84.8% savings)
```

### Amend last commit
```bash
$ mediagit commit --amend -m "Add promotional video assets (final version)"
[main d6f0a3c] Add promotional video assets (final version)
 5 files changed
 ℹ Amended previous commit
```

### Commit with verbose output
```bash
$ mediagit commit -v -m "Add product photography"
Changes to be committed:
  new file:   product_shot_001.jpg
  new file:   product_shot_002.jpg
  new file:   product_shot_003.jpg
  modified:   catalog.json

[main e7g1b4d] Add product photography
 4 files changed (3 added, 1 modified)
 Compression: 45.6 MB → 12.3 MB (73.0% savings)
 Deduplication: 0 chunks
 Delta encoding: catalog.json (95% size reduction)
```

### Commit from file
```bash
$ cat commit-message.txt
Major redesign of homepage assets

This commit includes:
- New hero images optimized for retina displays
- Updated video backgrounds with improved compression
- Refreshed icon set with SVG versions

$ mediagit commit -F commit-message.txt
[main f8h2c5e] Major redesign of homepage assets
 28 files changed
 Compression: 820.4 MB → 142.7 MB (82.6% savings)
```

### Override author
```bash
$ mediagit commit -m "Apply design changes" --author "Designer <designer@company.com>"
[main g9i3d6f] Apply design changes
 Author: Designer <designer@company.com>
 7 files changed
```

## Commit Statistics

Each commit displays optimization metrics:

```bash
$ mediagit commit -m "Add training video series"
[main h0j4e7g] Add training video series
 12 files changed
 Original size: 3.2 GB
 Stored size: 485.3 MB (84.8% savings)

 Breakdown:
   Compression: 3.2 GB → 890.2 MB (72.2% savings)
   Deduplication: 15 chunks (105.8 MB saved)
   Delta encoding: 2 files (299.1 MB saved)

 Storage:
   New unique objects: 3,847
   Reused objects: 142
   Total objects in ODB: 8,293
```

## Empty Commits

Empty commits require explicit flag:

```bash
$ mediagit commit -m "Trigger CI rebuild"
error: no changes added to commit

$ mediagit commit -m "Trigger CI rebuild" --allow-empty
[main i1k5f8h] Trigger CI rebuild
 0 files changed
```

## Commit Tree

View commit history with `mediagit log`:

```bash
$ mediagit log --oneline -5
i1k5f8h (HEAD -> main) Trigger CI rebuild
h0j4e7g Add training video series
g9i3d6f Apply design changes
f8h2c5e Major redesign of homepage assets
e7g1b4d Add product photography
```

## Commit Message Guidelines

### Format
```
Short summary (50 chars or less)

More detailed explanation if needed. Wrap at 72 characters.
Explain the problem this commit solves and why you made
the changes.

- Bullet points are fine
- Use present tense: "Add feature" not "Added feature"
- Reference issues: Fixes #123
```

### Example
```bash
$ mediagit commit -m "Optimize video encoding for mobile devices

Reduced bitrate for mobile-targeted videos to improve
load times on slower connections. Videos maintain quality
on small screens while reducing file size by 40%.

Affected files:
- All videos in mobile/campaigns/ directory
- Thumbnail generation updated

Closes #456"
```

## Exit Status

- **0**: Commit created successfully
- **1**: Nothing to commit (no staged changes)
- **2**: Invalid options or commit message

## Notes

### Best Practices

1. **Atomic commits**: Each commit should represent one logical change
2. **Descriptive messages**: Explain why changes were made, not just what changed
3. **Test before committing**: Ensure files work as expected
4. **Review staged changes**: Use `mediagit status` and `mediagit diff --staged`

### Performance

MediaGit optimizes commit operations:
- **Parallel processing**: Multiple files processed concurrently
- **Incremental hashing**: Only new content is hashed
- **Smart caching**: Reuse computations where possible
- **Background cleanup**: Object database optimization happens asynchronously

## See Also

- [mediagit add](./add.md) - Add files to the staging area
- [mediagit status](./status.md) - Show the working tree status
- [mediagit log](./log.md) - Show commit history
- [mediagit diff](./diff.md) - Show changes between commits
- [mediagit show](./show.md) - Show commit details
