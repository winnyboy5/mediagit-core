# mediagit status

Display the working tree status.

## Synopsis

```bash
mediagit status [OPTIONS]
```

## Description

Shows the status of the working tree, displaying:
- Files staged for commit (in the index)
- Files with modifications not staged
- Untracked files not yet added
- Current branch and tracking information
- Storage statistics and compression metrics

MediaGit status provides enhanced insights compared to traditional version control:
- Real-time compression savings preview
- Deduplication opportunities for staged files
- Storage backend health indicators
- Media-specific file analysis

## Options

### `-s, --short`
Show output in short format, one file per line with status codes.

### `-b, --branch`
Show branch information even in short format.

### `-v, --verbose`
Show detailed information including:
- File sizes and compression ratios
- Deduplication statistics
- Storage backend status
- Object database metrics

### `--porcelain[=<version>]`
Machine-readable output format. Version can be `1` or `2`.

### `--long`
Show output in long format (default).

### `--show-stash`
Show number of entries currently stashed.

### `-u, --untracked-files[=<mode>]`
Show untracked files:
- **no**: Show no untracked files
- **normal**: Show untracked files and directories (default)
- **all**: Show individual files in untracked directories

### `--ignored[=<mode>]`
Show ignored files:
- **traditional**: Show ignored files and directories
- **matching**: Show ignored files matching ignore patterns
- **no**: Show no ignored files (default)

## Status Indicators

### Short Format Codes

```
 M  modified in working tree
M   staged for commit
MM  staged with additional modifications
A   new file staged
AM  staged new file with modifications
D   deleted in working tree
?? untracked file
!! ignored file
```

### Long Format Sections

```
Changes to be committed:
    (staged files ready for commit)

Changes not staged for commit:
    (modified files not yet staged)

Untracked files:
    (files not tracked by MediaGit)
```

## Examples

### Basic status

```bash
$ mediagit status
On branch main
Your branch is up to date with 'origin/main'.

Changes to be committed:
  (use "mediagit restore --staged <file>..." to unstage)
        new file:   project_video.mp4
        modified:   README.md

Changes not staged for commit:
  (use "mediagit add <file>..." to update what will be committed)
  (use "mediagit restore <file>..." to discard changes)
        modified:   config.json

Untracked files:
  (use "mediagit add <file>..." to include in commit)
        draft_design.psd
        temp_render.mov

Storage preview for staged files:
  Original size: 250.3 MB
  After compression: 42.1 MB (83.2% savings)
  Deduplication opportunities: 5 chunks (8.2 MB additional savings)
```

### Short format

```bash
$ mediagit status --short
M  README.md
A  project_video.mp4
 M config.json
?? draft_design.psd
?? temp_render.mov
```

### Short format with branch

```bash
$ mediagit status --short --branch
## main...origin/main [ahead 2]
M  README.md
A  project_video.mp4
 M config.json
?? draft_design.psd
```

### Verbose status

```bash
$ mediagit status --verbose
On branch main
Your branch is ahead of 'origin/main' by 2 commits.
  (use "mediagit push" to publish your local commits)

Changes to be committed:
  (use "mediagit restore --staged <file>..." to unstage)
        new file:   project_video.mp4
          Size: 245.8 MB → 38.5 MB (compressed, 84.3% savings)
          Chunks: 63 (4 MB each)
          Deduplication: 3 chunks already exist (11.8 MB saved)

        modified:   README.md
          Size: 4.5 KB → 1.2 KB (delta encoded, 73.3% savings)
          Previous version: e4f3a2b
          Changes: +28 lines, -15 lines

Changes not staged for commit:
        modified:   config.json
          Size: 2.1 KB (not yet staged)
          Changes: +5 lines, -2 lines

Untracked files:
        draft_design.psd (89.2 MB)
        temp_render.mov (156.7 MB)

Object Database Status:
  Total objects: 2,847
  Total size: 1.2 GB → 185.3 MB (stored)
  Compression ratio: 84.5%
  Deduplication savings: 127.8 MB
  Cache hit rate: 92.3%

Storage Backend: AWS S3 (us-west-2)
  Status: Healthy ✓
  Bucket: mediagit-prod-assets
  Last sync: 2 minutes ago
```

### Show ignored files

```bash
$ mediagit status --ignored
On branch main

Untracked files:
        new_asset.mp4

Ignored files:
        .DS_Store
        Thumbs.db
        *.tmp
        cache/
        node_modules/
```

### Machine-readable output

```bash
$ mediagit status --porcelain
M  README.md
A  project_video.mp4
 M config.json
?? draft_design.psd
?? temp_render.mov
```

### After staging all changes

```bash
$ mediagit add --all
$ mediagit status
On branch main
Your branch is ahead of 'origin/main' by 1 commit.

Changes to be committed:
  (use "mediagit restore --staged <file>..." to unstage)
        new file:   draft_design.psd
        new file:   project_video.mp4
        new file:   temp_render.mov
        modified:   README.md
        modified:   config.json

Commit preview:
  5 files changed (3 added, 2 modified)
  Original size: 495.3 MB
  Compressed size: 78.6 MB (84.1% savings)
  Ready to commit ✓
```

## Working Tree Status

### Clean working tree

```bash
$ mediagit status
On branch main
Your branch is up to date with 'origin/main'.

nothing to commit, working tree clean
```

### Detached HEAD

```bash
$ mediagit status
HEAD detached at a3c8f9d
nothing to commit, working tree clean
```

### Merge conflict

```bash
$ mediagit status
On branch feature-video
You have unmerged paths.
  (fix conflicts and run "mediagit commit")
  (use "mediagit merge --abort" to abort the merge)

Unmerged paths:
  (use "mediagit add <file>..." to mark resolution)
        both modified:   project_settings.json

Automatic merge strategy: Latest Modified Time
Conflicting files: 1
  - project_settings.json: local modified 2024-01-15, remote modified 2024-01-14
  - Resolution: Keep local version (newer)
```

## Branch Tracking

### Ahead of remote

```bash
$ mediagit status
On branch main
Your branch is ahead of 'origin/main' by 3 commits.
  (use "mediagit push" to publish your local commits)
```

### Behind remote

```bash
$ mediagit status
On branch main
Your branch is behind 'origin/main' by 5 commits.
  (use "mediagit pull" to update your local branch)
```

### Diverged branches

```bash
$ mediagit status
On branch main
Your branch and 'origin/main' have diverged,
and have 2 and 3 different commits each, respectively.
  (use "mediagit pull" to merge the remote branch)
```

## Storage Insights

MediaGit status provides storage optimization insights:

```bash
$ mediagit status --verbose
...

Storage Optimization Opportunities:
  ✓ 3 files can benefit from delta encoding: 45.2 MB → 12.1 MB
  ✓ 2 files have duplicate chunks: 23.8 MB deduplication potential
  ✓ 1 file can use better compression: 18.3 MB → 5.2 MB

  Run "mediagit add --optimize" to apply optimizations
```

## Performance

MediaGit status is optimized for large repositories:
- **Incremental checks**: Only scan files with changed mtimes
- **Parallel scanning**: Multi-threaded file status checking
- **Cache utilization**: Reuse hash computations from previous operations
- **Smart sampling**: For very large files, sample-based change detection

Typical performance:
- **Small repos** (&lt;1000 files): &lt;100ms
- **Medium repos** (&lt;10000 files): &lt;500ms
- **Large repos** (&gt;10000 files): &lt;2s

## Exit Status

- **0**: Working tree is clean, or changes displayed successfully
- **1**: Error accessing working tree or repository
- **2**: Invalid options or configuration error

## Configuration

Status display can be customized:

```toml
[status]
# Show short format by default
short = false

# Always show branch information
show_branch = true

# Show untracked files
show_untracked = "normal"  # normal | no | all

# Color output
color = "auto"  # auto | always | never

[status.preview]
# Show compression preview for staged files
compression = true

# Show deduplication opportunities
deduplication = true

# Show storage backend status
backend = true
```

## Notes

### Working Tree States

1. **Clean**: No changes, nothing to commit
2. **Modified**: Changes exist but not staged
3. **Staged**: Changes ready for commit
4. **Conflicted**: Merge conflicts need resolution
5. **Detached**: HEAD not on a branch tip

### Performance Tips

- Use `--short` for faster output in scripts
- Use `--porcelain` for machine parsing
- Add frequently-checked paths to `.mediagitignore`
- Enable status caching for very large repositories

### Media File Tracking

MediaGit provides enhanced status for media files:
- Automatic format detection (video, image, audio)
- Codec and quality information
- Resolution and duration metadata
- Compatibility warnings for unsupported formats

## See Also

- [mediagit add](./add.md) - Add files to the staging area
- [mediagit commit](./commit.md) - Record changes to the repository
- [mediagit diff](./diff.md) - Show changes between commits
- [mediagit restore](./restore.md) - Restore working tree files
- [mediagit branch](./branch.md) - List, create, or delete branches
