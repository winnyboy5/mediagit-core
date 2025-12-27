# mediagit push

Update remote repository with local commits.

## Synopsis

```bash
mediagit push [OPTIONS] [<repository> [<refspec>...]]
```

## Description

Updates remote repository with local commits and objects. Transfers commits, trees, and blobs from local repository to remote, ensuring remote has all objects needed to reconstruct the history.

MediaGit push is **optimized for large media files**, using:
- Incremental uploads (only new chunks)
- Parallel transfer for multiple objects
- Compression-aware transfers
- Deduplication across pushes

## Options

### Repository and Refspec

#### `<repository>`
Remote repository name or URL (default: `origin`).

#### `<refspec>`
Source and destination refs (default: current branch).

### Push Behavior

#### `-u`, `--set-upstream`
Set upstream tracking for current branch.

#### `--all`
Push all branches.

#### `--tags`
Push all tags.

#### `--follow-tags`
Push tags reachable from pushed commits.

#### `--mirror`
Mirror all refs (branches and tags).

### Force Options

#### `-f`, `--force`
Force update remote refs (dangerous).

#### `--force-with-lease[=<refname>[:<expect>]]`
Safer force push, refuses if remote changed unexpectedly.

#### `--force-if-includes`
Force push only if remote has commits we've seen.

### Execution Options

#### `-n`, `--dry-run`
Show what would be pushed without actually pushing.

#### `--receive-pack=<git-receive-pack>`
Path to receive-pack program on remote.

#### `--progress`
Force progress reporting.

#### `-q`, `--quiet`
Suppress all output.

#### `-v`, `--verbose`
Show detailed information.

### Ref Management

#### `--delete`
Delete remote ref.

#### `--prune`
Remove remote branches that don't exist locally.

### MediaGit-Specific Options

#### `--optimize-transfer`
Enable transfer optimizations for media files.

#### `--parallel=<n>`
Number of parallel upload threads (default: 4).

#### `--chunk-size=<size>`
Chunk size for large file transfers (default: 4MB).

#### `--compression-level=<n>`
Compression level for transfer (0-9).

## Examples

### Push current branch

```bash
$ mediagit push
Enumerating objects: 47, done.
Counting objects: 100% (47/47), done.
Compressing objects: 100% (23/23), done.
Writing objects: 100% (24/24), 63.6 MB | 8.2 MB/s, done.
Total 24 (delta 3), reused 18 (delta 1)

To https://github.com/user/mediagit-project.git
   a3c8f9d..b4d7e1a  main -> main
```

### Push with upstream tracking

```bash
$ mediagit push -u origin feature/video-optimization
Enumerating objects: 12, done.
Counting objects: 100% (12/12), done.
Compressing objects: 100% (8/8), done.
Writing objects: 100% (8/8), 75.9 MB | 9.8 MB/s, done.
Total 8 (delta 2), reused 0 (delta 0)

To https://github.com/user/mediagit-project.git
 * [new branch]      feature/video-optimization -> feature/video-optimization
Branch 'feature/video-optimization' set up to track 'origin/feature/video-optimization'.
```

### Push all branches

```bash
$ mediagit push --all origin
Enumerating objects: 84, done.
Counting objects: 100% (84/84), done.

To https://github.com/user/mediagit-project.git
   a3c8f9d..b4d7e1a  main -> main
   c5e9f2b..d6f0a3c  feature/optimize -> feature/optimize
 * [new branch]      feature/new-assets -> feature/new-assets
```

### Push with tags

```bash
$ mediagit push --follow-tags
Enumerating objects: 24, done.
Writing objects: 100% (24/24), 63.6 MB | 8.2 MB/s, done.

To https://github.com/user/mediagit-project.git
   a3c8f9d..b4d7e1a  main -> main
 * [new tag]         v1.0.0 -> v1.0.0
```

### Force push with lease

```bash
$ mediagit push --force-with-lease
Enumerating objects: 18, done.
Writing objects: 100% (12/12), 45.2 MB | 7.8 MB/s, done.

To https://github.com/user/mediagit-project.git
 + a3c8f9d...b4d7e1a main -> main (forced update)

warning: Force-pushed to main branch
```

### Dry run

```bash
$ mediagit push --dry-run
To https://github.com/user/mediagit-project.git
   a3c8f9d..b4d7e1a  main -> main

Dry run: Would push 3 commits (63.6 MB)
No changes were made to the remote repository.
```

### Delete remote branch

```bash
$ mediagit push origin --delete feature/old-branch
To https://github.com/user/mediagit-project.git
 - [deleted]         feature/old-branch
```

### Push specific branch

```bash
$ mediagit push origin feature/video-optimization
Enumerating objects: 8, done.
Writing objects: 100% (8/8), 75.9 MB | 9.8 MB/s, done.

To https://github.com/user/mediagit-project.git
   c5e9f2b..d6f0a3c  feature/video-optimization -> feature/video-optimization
```

### Push to different remote name

```bash
$ mediagit push origin main:production
Total 12 (delta 3), reused 8 (delta 2)

To https://github.com/user/mediagit-project.git
   a3c8f9d..b4d7e1a  main -> production
```

### Optimized transfer for large media

```bash
$ mediagit push --optimize-transfer --parallel=8
Enumerating objects: 47, done.
Counting objects: 100% (47/47), done.

Transfer optimization enabled:
  Deduplication: Skipping 142 chunks already on remote
  New chunks: 293 chunks to upload
  Total data: 410.2 MB → 63.6 MB (84.5% reduction)

Parallel upload (8 threads):
  Thread 1: 36.7 MB/s
  Thread 2: 38.2 MB/s
  Thread 3: 37.1 MB/s
  Thread 4: 35.9 MB/s
  Thread 5: 36.4 MB/s
  Thread 6: 37.8 MB/s
  Thread 7: 36.2 MB/s
  Thread 8: 38.5 MB/s

Uploading objects: 100% (293/293), 63.6 MB | 296.8 MB/s (aggregate), done.

To https://s3.amazonaws.com/mediagit-prod-assets
   a3c8f9d..b4d7e1a  main -> main
```

### Verbose output

```bash
$ mediagit push -v
Pushing to https://github.com/user/mediagit-project.git
Enumerating objects: 24, done.
Counting objects: 100% (24/24), done.

Objects to push:
  Commits: 3
  Trees: 5
  Blobs: 16
  Total size: 410.2 MB (original)
  Compressed: 63.6 MB

Upload progress:
  [============================] 100% (24/24 objects)
  [============================] 100% (63.6/63.6 MB)
  Average speed: 8.2 MB/s
  Time elapsed: 7.8s

To https://github.com/user/mediagit-project.git
   a3c8f9d..b4d7e1a  main -> main

Post-push statistics:
  Remote objects: 5,028 → 5,052 (+24)
  Remote size: 421.7 MB → 485.3 MB (+63.6 MB)
```

## Transfer Optimization

### Chunk Deduplication

MediaGit avoids re-uploading existing chunks:

```bash
$ mediagit push
Analyzing remote objects...
Remote already has 142 chunks (58.2 MB)
Uploading only new chunks: 293 chunks (63.6 MB)

Upload: 63.6 MB instead of 121.8 MB (47.8% savings)
```

### Parallel Transfers

```bash
$ mediagit push --parallel=8
Parallel upload enabled (8 threads)
  Chunk batch 1: 8 chunks → Thread 1
  Chunk batch 2: 8 chunks → Thread 2
  ...
  Chunk batch 37: 5 chunks → Thread 5

Aggregate throughput: 296.8 MB/s
```

### Compression During Transfer

```bash
# Already compressed objects
$ mediagit push
Objects already compressed (Zstd level 3)
Transfer compression: None (efficient)
Upload speed: 8.2 MB/s

# Additional transfer compression
$ mediagit push --compression-level=6
Transfer compression: Gzip level 6
Network transfer: 63.6 MB → 58.1 MB (8.6% additional savings)
Upload speed: 7.5 MB/s (slightly slower due to compression overhead)
```

## Push Rejection

### Non-Fast-Forward Update

```bash
$ mediagit push
To https://github.com/user/mediagit-project.git
 ! [rejected]        main -> main (non-fast-forward)
error: failed to push some refs to 'https://github.com/user/mediagit-project.git'
hint: Updates were rejected because the tip of your current branch is behind
hint: its remote counterpart. Integrate the remote changes (e.g.
hint: 'mediagit pull ...') before pushing again.
hint: See the 'Note about fast-forwards' for details.

$ mediagit pull --rebase
$ mediagit push
# Success
```

### Force Push Required

```bash
$ mediagit push --force-with-lease
# Safer than --force, checks remote hasn't changed unexpectedly
```

## Push Refspecs

### Format

```
[+]<src>:<dst>
```

- **+**: Force update (optional)
- **src**: Local ref
- **dst**: Remote ref

### Examples

```bash
# Push local main to remote main
$ mediagit push origin main:main

# Push local feature to remote with different name
$ mediagit push origin feature/local:feature/remote

# Force push
$ mediagit push origin +main:main

# Delete remote branch
$ mediagit push origin :feature/old-branch
# or
$ mediagit push origin --delete feature/old-branch

# Push all branches
$ mediagit push origin 'refs/heads/*:refs/heads/*'

# Push all tags
$ mediagit push origin 'refs/tags/*:refs/tags/*'
```

## Performance

### Small Push

```
Objects: 5-10
Size: < 10 MB
Time: 1-3 seconds
```

### Medium Push

```
Objects: 10-50
Size: 10-100 MB
Time: 5-15 seconds
```

### Large Push

```
Objects: 50-200
Size: 100 MB - 1 GB
Time: 30-120 seconds
Optimization: Critical
```

### Very Large Push

```
Objects: 200+
Size: > 1 GB
Time: 2-10 minutes
Optimization: Essential
Recommendation: Use --optimize-transfer --parallel=8
```

## Exit Status

- **0**: Push completed successfully
- **1**: Push rejected or failed
- **2**: Invalid options or configuration

## Configuration

```toml
[push]
# Default push behavior
default = "simple"  # nothing | current | upstream | simple | matching

# Push tags automatically
follow_tags = false

# Require force-with-lease instead of force
use_force_if_includes = true

# Recurse into submodules
recurse_submodules = "check"  # check | on-demand | only | no

[push.transfer]
# Parallel upload threads
parallel = 4

# Chunk size for large files
chunk_size = "4MB"

# Enable transfer optimization
optimize = true

# Transfer compression level (0-9)
compression_level = 0  # 0 = none

# Progress reporting
show_progress = true
```

## Remote Storage Backends

### S3-Compatible Storage

```bash
$ mediagit push
To s3://mediagit-prod-assets/repo.git
   a3c8f9d..b4d7e1a  main -> main

Transfer details:
  Endpoint: s3.us-west-2.amazonaws.com
  Bucket: mediagit-prod-assets
  Objects uploaded: 24
  Data transferred: 63.6 MB
  Estimated cost: $0.0012
```

### Azure Blob Storage

```bash
$ mediagit push
To azure://mediagitprod.blob.core.windows.net/repo.git
   a3c8f9d..b4d7e1a  main -> main

Transfer details:
  Account: mediagitprod
  Container: repo
  Objects uploaded: 24
  Data transferred: 63.6 MB
```

### Google Cloud Storage

```bash
$ mediagit push
To gs://mediagit-prod-assets/repo.git
   a3c8f9d..b4d7e1a  main -> main

Transfer details:
  Bucket: mediagit-prod-assets
  Region: us-central1
  Objects uploaded: 24
  Data transferred: 63.6 MB
```

## Best Practices

### Before Pushing

1. **Review changes**:
   ```bash
   $ mediagit log origin/main..HEAD
   $ mediagit diff origin/main..HEAD
   ```

2. **Ensure tests pass**:
   ```bash
   $ mediagit push --dry-run
   ```

3. **Check remote state**:
   ```bash
   $ mediagit fetch
   $ mediagit status
   ```

### When Pushing

1. **Use descriptive branch names**
2. **Set upstream tracking** (`-u` on first push)
3. **Avoid force push to shared branches**
4. **Use `--force-with-lease` instead of `--force`**

### After Pushing

1. **Verify success**:
   ```bash
   $ mediagit log origin/main
   ```

2. **Clean up local branches**:
   ```bash
   $ mediagit branch -d feature/merged-branch
   ```

## Troubleshooting

### Authentication Failed

```bash
error: Authentication failed
hint: Check your credentials or access token
```

Solution: Update credentials or regenerate access token.

### Large File Timeout

```bash
error: RPC failed; HTTP 500 curl 22 timeout
```

Solution: Use `--optimize-transfer` and `--parallel`:
```bash
$ mediagit push --optimize-transfer --parallel=8
```

### Storage Quota Exceeded

```bash
error: Remote storage quota exceeded
hint: Current usage: 9.8 GB / 10 GB
```

Solution: Run garbage collection or upgrade storage plan.

## Notes

### Force Push Warning

⚠️ **Force pushing rewrites remote history**

Only force push when:
- Working on personal feature branch
- Coordinated with team
- Absolutely necessary

Always prefer `--force-with-lease` over `--force`.

### Media File Efficiency

MediaGit push is highly efficient for media files:
- Deduplication prevents re-uploading unchanged chunks
- Compression reduces transfer size
- Parallel uploads maximize bandwidth
- Incremental transfers save time and costs

### Network Interruption

If push is interrupted:
```bash
$ mediagit push
# Network interruption

$ mediagit push
# Resumes from where it left off
# Already-uploaded chunks are skipped
```

## See Also

- [mediagit pull](./pull.md) - Fetch and merge from remote
- [mediagit fetch](./fetch.md) - Download from remote
- [mediagit remote](./remote.md) - Manage remote repositories
- [mediagit branch](./branch.md) - Manage branches
