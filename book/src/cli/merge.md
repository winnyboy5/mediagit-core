# mediagit merge

Join two or more development histories together.

## Synopsis

```bash
mediagit merge [OPTIONS] <commit>...
mediagit merge --abort
mediagit merge --continue
```

## Description

Incorporates changes from named commits (typically branch heads) into the current branch. MediaGit performs a new commit representing the merged result (merge commit).

MediaGit provides **media-aware merging** strategies that intelligently handle conflicts in binary media files based on file type, modification time, and content analysis.

## Options

### Merge Strategy

#### `--ff`
Fast-forward when possible (default).

#### `--no-ff`
Create merge commit even when fast-forward is possible.

#### `--ff-only`
Refuse to merge unless fast-forward is possible.

#### `--squash`
Create working tree and index state but don't create merge commit.

### Merge Strategy Selection

#### `-s <strategy>`, `--strategy=<strategy>`
Use given merge strategy:
- **ours**: Always prefer our version for conflicts
- **theirs**: Always prefer their version for conflicts
- **latest-mtime**: Choose file with latest modification time (media default)
- **largest-size**: Choose largest file version
- **highest-quality**: Choose highest quality based on metadata analysis
- **manual**: Require manual conflict resolution

#### `-X <option>`, `--strategy-option=<option>`
Pass option to merge strategy.

### Commit Options

#### `--commit`
Perform merge and commit result (default).

#### `--no-commit`
Perform merge but don't commit, allowing inspection.

#### `-m <message>`
Set merge commit message.

#### `--edit`, `-e`
Open editor to modify merge commit message.

#### `--no-edit`
Accept auto-generated merge message.

### Conflict Resolution

#### `--abort`
Abort current conflict resolution and restore pre-merge state.

#### `--continue`
Continue merge after resolving conflicts.

#### `--quit`
Forget about current merge in progress.

### Fast-Forward Options

#### `--ff`
Allow fast-forward (default).

#### `--no-ff`
Always create merge commit.

#### `--ff-only`
Only allow fast-forward, abort otherwise.

### MediaGit-Specific Options

#### `--media-strategy=<strategy>`
Set media-specific merge strategy:
- **latest-mtime**: Latest modification time (default)
- **largest-size**: Largest file size
- **highest-bitrate**: Highest bitrate (video/audio)
- **highest-resolution**: Highest resolution (video/images)
- **manual**: Always require manual resolution

#### `--analyze-quality`
Perform quality analysis to determine best version.

#### `--preserve-both`
Keep both versions with conflict markers.

## Examples

### Fast-forward merge

```bash
$ mediagit merge feature/video-optimization
Updating a3c8f9d..b4d7e1a
Fast-forward
 videos/promo.mp4 | Binary: 245.8 MB → 198.3 MB (-47.5 MB)
 config.json      | 2 +-
 2 files changed, 1 insertion(+), 1 deletion(-)
```

### Three-way merge

```bash
$ mediagit merge feature/new-assets
Merge made by the 'recursive' strategy.
 assets/logo.png    | Binary: 156.3 KB added
 assets/banner.jpg  | Binary: 2.4 MB added
 metadata.json      | 5 insertions(+)
 3 files changed, 5 insertions(+)

Created merge commit c5e9f2b
```

### No fast-forward merge

```bash
$ mediagit merge --no-ff feature/hotfix
Merge branch 'feature/hotfix'

# Even though fast-forward was possible, creates merge commit
# to preserve branch history
```

### Merge with custom message

```bash
$ mediagit merge -m "Merge video optimization improvements" feature/optimize
Merge made by the 'recursive' strategy.
 videos/promo_1080p.mp4 | Binary: 245.8 MB → 198.3 MB (-19.3%)
 videos/promo_4k.mp4    | Binary: 856.3 MB → 712.5 MB (-16.8%)
 2 files changed
```

### Media-aware merge (automatic)

```bash
$ mediagit merge feature/audio-update
Auto-merging audio.wav using 'latest-mtime' strategy
CONFLICT (content): Merge conflict in video.mp4

MediaGit analyzed conflicting file:
  - Local version:  modified 2024-01-15 14:30
  - Remote version: modified 2024-01-14 09:00
  - Strategy: latest-mtime
  - Resolution: Keeping local version (newer)

Automatic merge successful.
 audio.wav   | Binary: auto-merged (latest-mtime)
 video.mp4   | Binary: auto-merged (latest-mtime, kept local)
 2 files changed
```

### Media conflict with analysis

```bash
$ mediagit merge --analyze-quality feature/video-remaster
Auto-merging videos/promo.mp4
CONFLICT (media): Both sides modified videos/promo.mp4

Quality Analysis:
  Local version:
    Size: 245.8 MB
    Resolution: 1920x1080
    Bitrate: 8.5 Mbps
    Codec: H.264
    Modified: 2024-01-15 14:30

  Remote version:
    Size: 198.3 MB
    Resolution: 1920x1080
    Bitrate: 7.2 Mbps
    Codec: H.265 (better compression)
    Modified: 2024-01-14 09:00

Recommendation: Remote version (H.265, better compression, minimal quality loss)
Auto-resolved using 'highest-quality' strategy: keeping remote

Automatic merge successful.
```

### Manual merge with conflicts

```bash
$ mediagit merge feature/concurrent-edits
Auto-merging metadata.json
CONFLICT (content): Merge conflict in metadata.json
Auto-merging video.mp4 using 'latest-mtime'
CONFLICT (media): Manual resolution required for image.jpg

Automatic merge failed; fix conflicts and then commit the result.

Conflicting files:
  - metadata.json: text conflict, manual edit required
  - image.jpg: both modified, resolution strategy unclear

Use "mediagit merge --abort" to cancel merge.
```

### Resolve conflicts and continue

```bash
$ mediagit status
On branch main
You have unmerged paths.
  (fix conflicts and run "mediagit commit")

Unmerged paths:
  (use "mediagit add <file>..." to mark resolution)
        both modified:   metadata.json
        both modified:   image.jpg

# Manually resolve conflicts
$ vim metadata.json
$ mediagit add metadata.json

# Choose version for media file
$ mediagit checkout --ours image.jpg
$ mediagit add image.jpg

$ mediagit commit -m "Merge feature/concurrent-edits with conflict resolution"
[main d6f0a3c] Merge feature/concurrent-edits
```

### Abort merge

```bash
$ mediagit merge feature/experimental
CONFLICT (content): Merge conflict in config.json
Automatic merge failed; fix conflicts and then commit the result.

$ mediagit merge --abort
Merge aborted, returning to pre-merge state.
```

### Squash merge

```bash
$ mediagit merge --squash feature/multiple-commits
Squash commit -- not updating HEAD
Automatic merge went well; stopped before committing as requested

$ mediagit commit -m "Add all video optimization changes"
[main e7g1b4d] Add all video optimization changes
 12 files changed
```

### Merge specific strategy

```bash
$ mediagit merge --strategy=ours feature/keep-our-config
Merge made by the 'ours' strategy.
# All conflicts automatically resolved using our version
```

### Media strategy: largest size

```bash
$ mediagit merge --media-strategy=largest-size feature/high-res
Auto-merging image.jpg using 'largest-size' strategy
CONFLICT (media): Comparing file sizes
  - Local: 2.4 MB (1920x1080)
  - Remote: 3.6 MB (2560x1440)
  - Resolution: Keeping remote (larger, higher resolution)

Automatic merge successful.
```

### Preserve both versions

```bash
$ mediagit merge --preserve-both feature/alternative-edit
CONFLICT (media): Both versions preserved

Created versions:
  - video.mp4.ours (local version)
  - video.mp4.theirs (remote version)
  - video.mp4 (requires manual selection)

Please review both versions and keep the desired one.
```

## Merge Commit Format

Default merge commit message:

```
Merge branch 'feature/branch-name'

# Conflicts resolved:
#   video.mp4: kept ours (latest-mtime)
#   audio.wav: kept theirs (highest-quality)
```

## Merge Strategies

### For Text Files

1. **three-way merge**: Standard git-style merge with common ancestor
2. **ours**: Keep our version
3. **theirs**: Keep their version

### For Media Files (MediaGit-Specific)

1. **latest-mtime** (default): Choose file with latest modification time
   - Best for: Active editing workflows
   - Assumption: Latest edit is the intended version

2. **largest-size**: Choose largest file
   - Best for: Resolution upgrades, lossless workflows
   - Assumption: Larger = higher quality or more content

3. **highest-quality**: Analyze metadata to determine quality
   - Best for: Re-encoding workflows
   - Considers: Bitrate, resolution, codec efficiency

4. **highest-resolution**: Choose highest resolution (images/video)
   - Best for: Archival, print media
   - Considers: Pixel dimensions only

5. **highest-bitrate**: Choose highest bitrate (audio/video)
   - Best for: Audio mastering, video production
   - Considers: Data rate regardless of codec

6. **manual**: Always require explicit choice
   - Best for: Critical assets, legal requirements
   - User must explicitly choose version

## Conflict Resolution

### Text File Conflicts

```
<<<<<<< HEAD (ours)
{
  "format": "mp4",
  "quality": "high"
}
=======
{
  "format": "webm",
  "quality": "ultra"
}
>>>>>>> feature/updates (theirs)
```

Edit file to resolve, then:
```bash
$ mediagit add config.json
$ mediagit commit
```

### Media File Conflicts

```bash
# List conflict status
$ mediagit status
Unmerged paths:
  both modified:   video.mp4

# Choose our version
$ mediagit checkout --ours video.mp4
$ mediagit add video.mp4

# Or choose their version
$ mediagit checkout --theirs video.mp4
$ mediagit add video.mp4

# Or keep both
$ mediagit checkout --ours video.mp4 --to video_ours.mp4
$ mediagit checkout --theirs video.mp4 --to video_theirs.mp4
# Manually decide, then add chosen version
```

## Fast-Forward Merge

When possible, MediaGit performs fast-forward:

```
Before:
  A---B---C  main
           \
            D---E  feature

After (fast-forward):
  A---B---C---D---E  main, feature
```

Use `--no-ff` to always create merge commit:

```
After (no fast-forward):
  A---B---C-------F  main
           \     /
            D---E  feature
```

## Three-Way Merge

When branches have diverged:

```
Before:
  A---B---C---D  main
       \
        E---F  feature

After:
  A---B---C---D---G  main
       \         /
        E---F----  feature

G is the merge commit with parents D and F
```

## Merge Conflicts Statistics

```bash
$ mediagit merge feature/large-update
Auto-merging 47 files...

Merge Summary:
  Fast-forward: 12 files
  Auto-merged (text): 23 files
  Auto-merged (media): 10 files
    - latest-mtime: 7 files
    - highest-quality: 3 files
  Manual resolution required: 2 files
    - metadata.json (text conflict)
    - video.mp4 (unclear strategy)

Please resolve conflicts and commit.
```

## Performance

MediaGit merge is optimized for media repositories:
- **Fast-forward**: < 50ms
- **Small merge** (<10 files): < 500ms
- **Large merge** (100+ files): < 5s
- **Media analysis**: 50-100 MB/s

## Exit Status

- **0**: Merge completed successfully
- **1**: Conflicts detected, manual resolution required
- **2**: Merge aborted or invalid operation

## Configuration

```toml
[merge]
# Default merge strategy
default_strategy = "recursive"

# Fast-forward mode
ff = true  # true | false | only

# Conflict style
conflict_style = "merge"  # merge | diff3 | zdiff3

# Show stat after merge
stat = true

[merge.media]
# Default media merge strategy
strategy = "latest-mtime"  # latest-mtime | largest-size | highest-quality | manual

# Perform quality analysis
analyze_quality = false

# Preserve both versions on conflict
preserve_both = false

# Media file extensions to auto-merge
auto_merge_types = ["mp4", "mov", "avi", "jpg", "png", "wav", "mp3"]
```

## Notes

### Best Practices

1. **Before Merging**:
   - Ensure working tree is clean
   - Review changes with `mediagit log`
   - Consider impact with `mediagit diff`

2. **During Merge**:
   - Read conflict messages carefully
   - Test media files after resolution
   - Verify quality hasn't degraded

3. **After Merge**:
   - Test merged assets thoroughly
   - Push changes to remote
   - Delete merged feature branch

### Media Merge Guidelines

- **Video files**: Prefer latest-mtime or highest-quality
- **Images**: Consider resolution and file size
- **Audio**: Consider bitrate and sample rate
- **Documents**: Use manual resolution for critical files

### Large Binary Merges

For very large media files:
- Merge uses chunk-level comparison (fast)
- No content extraction needed
- Metadata analysis is quick
- Storage impact calculated efficiently

## See Also

- [mediagit branch](./branch.md) - Manage branches
- [mediagit rebase](./rebase.md) - Reapply commits on top of another branch
- [mediagit diff](./diff.md) - Show changes between commits
- [mediagit log](./log.md) - Show commit history
- [mediagit status](./status.md) - Show working tree status
