# mediagit pull

Fetch from and integrate with remote repository.

## Synopsis

```bash
mediagit pull [OPTIONS] [<repository> [<refspec>...]]
```

## Description

Fetch changes from remote repository and integrate them into current branch. Equivalent to running `mediagit fetch` followed by `mediagit merge` (or `mediagit rebase` if configured).

MediaGit pull is **optimized for large media repositories**, using:
- Incremental downloads (only new objects)
- Parallel transfers for faster downloads
- Smart chunk reuse and deduplication
- Efficient merge strategies for media files

## Options

### Integration Method

#### `--rebase[=<mode>]`
Rebase current branch on top of upstream after fetch.
Modes: **false**, **true**, **merges**, **interactive**.

#### `--no-rebase`
Merge instead of rebase (default).

#### `--ff-only`
Only allow fast-forward merge.

#### `--no-ff`
Create merge commit even when fast-forward is possible.

### Fetch Options

#### `-a`, `--all`
Fetch from all remotes.

#### `--depth=<depth>`
Limit fetching to specified number of commits.

#### `--unshallow`
Convert shallow repository to complete one.

#### `--tags`
Fetch all tags.

#### `--no-tags`
Don't fetch tags.

### Merge/Rebase Options

#### `-s <strategy>`, `--strategy=<strategy>`
Use given merge strategy.

#### `-X <option>`, `--strategy-option=<option>`
Pass option to merge strategy.

#### `--commit`
Perform merge and commit (default).

#### `--no-commit`
Perform merge but don't commit.

### Execution Options

#### `-n`, `--dry-run`
Show what would be done without actually doing it.

#### `-q`, `--quiet`
Suppress output.

#### `-v`, `--verbose`
Show detailed information.

#### `--progress`
Force progress reporting.

### MediaGit-Specific Options

#### `--optimize-transfer`
Enable download optimizations for media files.

#### `--parallel=<n>`
Number of parallel download threads (default: 4).

#### `--verify-checksums`
Verify chunk integrity during download.

## Examples

### Basic pull

```bash
$ mediagit pull
Fetching origin
Enumerating objects: 24, done.
Counting objects: 100% (24/24), done.
Compressing objects: 100% (12/12), done.
Receiving objects: 100% (24/24), 63.6 MB | 9.2 MB/s, done.
Resolving deltas: 100% (3/3), done.

From https://github.com/user/mediagit-project
   a3c8f9d..b4d7e1a  main       -> origin/main

Updating a3c8f9d..b4d7e1a
Fast-forward
 videos/promo.mp4    | Binary: 245.8 MB → 198.3 MB (-19.3%)
 assets/logo.png     | Binary: 156.3 KB added
 2 files changed, 1 insertion(+), 1 deletion(-)
```

### Pull with rebase

```bash
$ mediagit pull --rebase
Fetching origin
From https://github.com/user/mediagit-project
   a3c8f9d..b4d7e1a  main       -> origin/main

Rebasing on top of origin/main...
Applying: Local commit 1
Applying: Local commit 2
Successfully rebased and updated refs/heads/main.
```

### Pull specific branch

```bash
$ mediagit pull origin feature/video-optimization
From https://github.com/user/mediagit-project
 * branch            feature/video-optimization -> FETCH_HEAD

Updating c5e9f2b..d6f0a3c
Fast-forward
 videos/optimized.mp4 | Binary: 856.3 MB → 712.5 MB (-16.8%)
 1 file changed
```

### Pull with fast-forward only

```bash
$ mediagit pull --ff-only
Fetching origin
From https://github.com/user/mediagit-project
   a3c8f9d..b4d7e1a  main       -> origin/main

fatal: Not possible to fast-forward, aborting.

# Need to merge or rebase
$ mediagit pull --rebase
# or
$ mediagit pull --no-ff
```

### Pull with merge commit

```bash
$ mediagit pull --no-ff
Fetching origin
From https://github.com/user/mediagit-project
   a3c8f9d..b4d7e1a  main       -> origin/main

Merge made by the 'recursive' strategy.
 videos/promo.mp4 | Binary: 245.8 MB → 198.3 MB
 1 file changed

Created merge commit e7g1b4d
```

### Pull with conflicts

```bash
$ mediagit pull
Fetching origin
From https://github.com/user/mediagit-project
   a3c8f9d..b4d7e1a  main       -> origin/main

Auto-merging config.json
CONFLICT (content): Merge conflict in config.json
Auto-merging video.mp4 using 'latest-mtime' strategy
Automatic merge failed; fix conflicts and then commit the result.

$ mediagit status
On branch main
You have unmerged paths.

Unmerged paths:
  both modified:   config.json

# Resolve conflicts
$ vim config.json
$ mediagit add config.json
$ mediagit commit -m "Merge with conflict resolution"
```

### Optimized pull for large media

```bash
$ mediagit pull --optimize-transfer --parallel=8
Fetching origin
Analyzing remote objects...

Transfer optimization:
  Chunks already local: 847 (2.3 GB)
  New chunks to download: 293 (63.6 MB)
  Total savings: 97.3% (avoided 2.3 GB download)

Parallel download (8 threads):
  Thread 1: 42.1 MB/s
  Thread 2: 43.7 MB/s
  Thread 3: 41.5 MB/s
  Thread 4: 42.9 MB/s
  Thread 5: 43.2 MB/s
  Thread 6: 42.4 MB/s
  Thread 7: 41.8 MB/s
  Thread 8: 43.6 MB/s

Receiving objects: 100% (293/293), 63.6 MB | 341.2 MB/s (aggregate), done.

From https://github.com/user/mediagit-project
   a3c8f9d..b4d7e1a  main       -> origin/main

Updating a3c8f9d..b4d7e1a
Fast-forward
 videos/promo_4k.mp4 | Binary: 856.3 MB added (145.2 MB compressed)
 1 file changed
```

### Pull with verification

```bash
$ mediagit pull --verify-checksums
Fetching origin
Receiving objects: 100% (24/24), 63.6 MB | 9.2 MB/s, done.
Verifying checksums: 100% (24/24), done.
Verifying chunk integrity: 100% (293/293), done.

From https://github.com/user/mediagit-project
   a3c8f9d..b4d7e1a  main       -> origin/main

All objects verified successfully ✓
```

### Dry run

```bash
$ mediagit pull --dry-run
Fetching origin
From https://github.com/user/mediagit-project
   a3c8f9d..b4d7e1a  main       -> origin/main

Dry run: Would update main
Fast-forward a3c8f9d..b4d7e1a
Would download: 24 objects (63.6 MB)

No changes were made to the local repository.
```

### Interactive rebase during pull

```bash
$ mediagit pull --rebase=interactive
Fetching origin
From https://github.com/user/mediagit-project
   a3c8f9d..b4d7e1a  main       -> origin/main

# Editor opens with rebase todo list
# Modify, save, and close

Successfully rebased and updated refs/heads/main.
```

### Verbose output

```bash
$ mediagit pull -v
Fetching origin
POST git-upload-pack (175 bytes)
From https://github.com/user/mediagit-project
 = [up to date]      main       -> origin/main
 * [new branch]      feature/new -> origin/feature/new

Receiving objects: 100% (24/24), 63.6 MB | 9.2 MB/s, done.

Download statistics:
  Objects received: 24
  Bytes received: 63.6 MB
  Chunks received: 293
  Chunks reused: 142
  Download time: 6.9s
  Average speed: 9.2 MB/s

Updating a3c8f9d..b4d7e1a
Fast-forward
 videos/promo.mp4 | Binary: 245.8 MB added
```

## Pull Strategies

### Fast-Forward (Default)

```
Before pull:
  A---B---C  origin/main
       \
        D  main (local)

After pull (fast-forward):
  A---B---C  origin/main, main
```

### Merge (Non-Fast-Forward)

```
Before pull:
  A---B---C  origin/main
       \
        D---E  main (local)

After pull (merge):
  A---B---C-------F  main
       \         /
        D---E----
```

### Rebase

```
Before pull:
  A---B---C  origin/main
       \
        D---E  main (local)

After pull (rebase):
  A---B---C---D'---E'  main
              \
               origin/main
```

## Transfer Optimization

### Chunk Deduplication

MediaGit avoids re-downloading existing chunks:

```bash
$ mediagit pull
Analyzing local objects...
Local repository has 847 chunks (2.3 GB)
Remote objects reference 1,140 chunks (2.4 GB)
Downloading only new chunks: 293 chunks (63.6 MB)

Download: 63.6 MB instead of 2.4 GB (97.3% savings)
```

### Parallel Downloads

```bash
$ mediagit pull --parallel=8
Parallel download enabled (8 threads)
  Chunk batch 1: 37 chunks → Thread 1
  Chunk batch 2: 37 chunks → Thread 2
  ...
  Chunk batch 8: 36 chunks → Thread 8

Aggregate throughput: 341.2 MB/s
Time saved: 82% (1.2s vs 6.9s)
```

### Delta Resolution

```bash
$ mediagit pull
Receiving objects: 100% (24/24), done.
Resolving deltas: 100% (3/3), done.

Delta resolution:
  Base objects: 3
  Delta objects: 3
  Resolved size: 1.2 KB → 4.5 KB (reconstructed)
```

## Conflict Resolution

### Automatic Media Merge

```bash
$ mediagit pull
Auto-merging video.mp4 using 'latest-mtime' strategy
  Local: modified 2024-01-15 14:30
  Remote: modified 2024-01-14 09:00
  Resolution: Keeping local version (newer)

Merge successful.
```

### Manual Resolution Required

```bash
$ mediagit pull
CONFLICT (media): Both versions modified at same time
  video.mp4: both modified 2024-01-15 14:30

Please choose version:
  $ mediagit checkout --ours video.mp4    # Keep local
  $ mediagit checkout --theirs video.mp4  # Use remote

Then: $ mediagit add video.mp4
      $ mediagit commit
```

## Configuration

```toml
[pull]
# Rebase by default instead of merge
rebase = false  # false | true | merges | interactive

# Fast-forward only
ff = "only"  # false | true | only

# Octopus merge for multiple branches
octopus = false

[pull.transfer]
# Parallel download threads
parallel = 4

# Enable transfer optimization
optimize = true

# Verify checksums
verify = true

# Progress reporting
show_progress = true

[pull.media]
# Default media merge strategy
merge_strategy = "latest-mtime"
```

## Pull vs Fetch + Merge

### Pull (One Command)

```bash
$ mediagit pull
# Fetch + merge/rebase in one step
```

### Fetch + Merge (Two Commands)

```bash
$ mediagit fetch
# Review changes
$ mediagit log origin/main..HEAD
$ mediagit diff origin/main..HEAD

# Then merge or rebase
$ mediagit merge origin/main
# or
$ mediagit rebase origin/main
```

## Performance

### Small Pull
```
Objects: 5-10
Size: < 10 MB
Time: 1-3 seconds
```

### Medium Pull
```
Objects: 10-50
Size: 10-100 MB
Time: 5-15 seconds
```

### Large Pull
```
Objects: 50-200
Size: 100 MB - 1 GB
Time: 30-120 seconds
Optimization: Important
```

### Very Large Pull
```
Objects: 200+
Size: > 1 GB
Time: 2-10 minutes
Optimization: Critical
Recommendation: Use --optimize-transfer --parallel=8
```

## Exit Status

- **0**: Pull completed successfully
- **1**: Merge/rebase conflicts detected
- **2**: Invalid options or fetch failed

## Best Practices

### Before Pulling

1. **Commit or stash local changes**:
   ```bash
   $ mediagit status
   $ mediagit commit -m "Save work in progress"
   # or
   $ mediagit stash
   ```

2. **Review remote changes**:
   ```bash
   $ mediagit fetch
   $ mediagit log ..origin/main
   ```

### When Pulling

1. **Use rebase for clean history** (feature branches):
   ```bash
   $ mediagit pull --rebase
   ```

2. **Use merge for shared branches** (main, develop):
   ```bash
   $ mediagit pull --no-ff
   ```

3. **Enable optimization for large repositories**:
   ```bash
   $ mediagit pull --optimize-transfer --parallel=8
   ```

### After Pulling

1. **Verify changes**:
   ```bash
   $ mediagit log -3
   $ mediagit status
   ```

2. **Test changes**:
   ```bash
   $ cargo test
   # or your project's test command
   ```

## Troubleshooting

### Uncommitted Changes

```bash
$ mediagit pull
error: Your local changes would be overwritten by merge.
Please commit or stash them before pulling.

$ mediagit stash
$ mediagit pull
$ mediagit stash pop
```

### Cannot Fast-Forward

```bash
$ mediagit pull --ff-only
fatal: Not possible to fast-forward, aborting.

# Solution 1: Rebase
$ mediagit pull --rebase

# Solution 2: Merge
$ mediagit pull --no-ff
```

### Large Download Timeout

```bash
error: RPC failed; HTTP 500 timeout

# Solution: Use optimization
$ mediagit pull --optimize-transfer --parallel=8
```

### Merge Conflicts

```bash
$ mediagit pull
CONFLICT (content): Merge conflict in config.json

$ mediagit status
# Fix conflicts
$ vim config.json
$ mediagit add config.json
$ mediagit commit
```

## Notes

### Fetch vs Pull

- **fetch**: Only downloads, doesn't integrate
- **pull**: Downloads and integrates (fetch + merge/rebase)

Use `fetch` when you want to review changes before integrating.

### Media File Efficiency

MediaGit pull is highly efficient:
- Deduplication prevents re-downloading unchanged chunks (97%+ savings typical)
- Compression reduces transfer size (80%+ compression typical)
- Parallel downloads maximize bandwidth (8x faster with 8 threads)
- Incremental transfers save time and bandwidth costs

### Network Interruption

If pull is interrupted:
```bash
$ mediagit pull
# Network interruption

$ mediagit pull
# Resumes from where it left off
# Already-downloaded chunks are skipped
```

### Shallow Pulls

For very large repositories, consider shallow clone:
```bash
$ mediagit clone --depth 1 <url>
$ mediagit pull --depth 1

# Later, unshallow if needed
$ mediagit pull --unshallow
```

## See Also

- [mediagit fetch](./fetch.md) - Download from remote
- [mediagit merge](./merge.md) - Join branches together
- [mediagit rebase](./rebase.md) - Reapply commits
- [mediagit push](./push.md) - Update remote repository
- [mediagit remote](./remote.md) - Manage remotes
