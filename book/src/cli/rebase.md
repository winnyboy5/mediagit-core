# mediagit rebase

Reapply commits on top of another base.

## Synopsis

```bash
mediagit rebase [OPTIONS] [<upstream> [<branch>]]
mediagit rebase --continue
mediagit rebase --skip
mediagit rebase --abort
```

## Description

Reapply commits from current branch on top of another base branch. This creates a linear history by moving the entire branch to begin from a different commit.

MediaGit rebase is **media-aware**, handling large binary files efficiently during the reapplication process and preserving compression and deduplication benefits.

## Options

### Basic Rebase

#### `<upstream>`
Upstream branch to rebase onto (default: configured upstream).

#### `<branch>`
Branch to rebase (default: current branch).

### Rebase Control

#### `--continue`
Continue rebase after resolving conflicts.

#### `--skip`
Skip current commit and continue rebase.

#### `--abort`
Abort rebase and return to original branch state.

#### `--quit`
Abort rebase but HEAD stays at current position.

### Strategy Options

#### `-s <strategy>`, `--strategy=<strategy>`
Use given merge strategy for rebase.

#### `-X <option>`, `--strategy-option=<option>`
Pass option to merge strategy.

#### `--merge`
Use merging strategies to rebase (instead of apply).

### Commit Handling

#### `--keep-empty`
Keep commits that become empty during rebase.

#### `--empty=<mode>`
How to handle commits that become empty: **drop**, **keep**, **ask**.

#### `--rebase-merges[=<mode>]`
Preserve merge commits during rebase.

### Other Options

#### `-f`, `--force-rebase`
Force rebase even if branch is up to date.

#### `--fork-point`
Use reflog to find better common ancestor.

#### `--ignore-date`
Use current timestamp instead of original author date.

#### `--committer-date-is-author-date`
Use author date as committer date.

## Examples

### Basic rebase

```bash
$ mediagit rebase main
First, rewinding head to replay your work on top of it...
Applying: Add video optimization
Applying: Update compression settings
Applying: Add quality metrics

Successfully rebased and updated refs/heads/feature/optimize.
```

### Rebase with conflicts

```bash
$ mediagit rebase main
Applying: Update video.mp4
CONFLICT (content): Merge conflict in video.mp4
error: could not apply a3c8f9d... Update video.mp4

Resolve all conflicts manually, mark them as resolved with
"mediagit add/rm <conflicted_files>", then run "mediagit rebase --continue".
You can instead skip this commit: run "mediagit rebase --skip".
To abort and get back to the state before "mediagit rebase", run "mediagit rebase --abort".

Could not apply a3c8f9d... Update video.mp4

$ mediagit status
rebase in progress; onto b4d7e1a
You are currently rebasing branch 'feature/optimize' on 'b4d7e1a'.
  (fix conflicts and run "mediagit rebase --continue")
  (use "mediagit rebase --skip" to skip this patch)
  (use "mediagit rebase --abort" to check out the original branch)

Unmerged paths:
  both modified:   video.mp4

# Resolve conflict
$ mediagit checkout --ours video.mp4
$ mediagit add video.mp4
$ mediagit rebase --continue
Applying: Update video.mp4
Applying: Add quality metrics
Successfully rebased and updated refs/heads/feature/optimize.
```

### Rebase onto different branch

```bash
$ mediagit checkout feature/audio-fix
$ mediagit rebase --onto main feature/base feature/audio-fix

# Rebase commits from feature/audio-fix that aren't in feature/base
# onto main branch
```

### Skip commit during rebase

```bash
$ mediagit rebase main
Applying: Temporary debug changes
CONFLICT (content): Multiple conflicts

# Don't want this commit
$ mediagit rebase --skip
Applying: Add important feature
Successfully rebased and updated refs/heads/feature/cleanup.
```

### Abort rebase

```bash
$ mediagit rebase main
CONFLICT (content): Merge conflict in config.json

$ mediagit rebase --abort
Rebase aborted, returning to original branch state.
```

### Rebase with strategy

```bash
$ mediagit rebase -s recursive -X theirs main
# Automatically resolve conflicts using 'theirs' version
```

### Rebase preserving merges

```bash
$ mediagit rebase --rebase-merges main
Successfully rebased and updated refs/heads/feature/complex.

# Preserves merge commits in branch history
```

## Before and After Rebase

### Before Rebase

```
      A---B---C  feature/optimize
     /
D---E---F---G  main
```

### After Rebase

```
              A'--B'--C'  feature/optimize
             /
D---E---F---G  main
```

Note: A', B', C' are new commits with same changes but different OIDs.

## Media-Aware Rebase

MediaGit handles media files efficiently during rebase:

```bash
$ mediagit rebase main
Applying: Update promo video
  Processing: video.mp4 (245.8 MB)
  Reusing chunks: 142 chunks (85% of file)
  New chunks: 23 chunks (15% of file)
  Compression: Preserved from original commit
  Deduplication: Maintained

Applying: Add thumbnail
  Processing: thumbnail.jpg (2.4 MB)
  Delta encoding: Applied against base
  Compression: Zstd level 3

Successfully rebased 2 commits in 3.2s
Media processing: 248.2 MB in 2.8s (88.6 MB/s)
```

## Rebase vs Merge

### Use Rebase When

- Cleaning up local branch before pushing
- Keeping linear history
- Working on feature branch alone
- Want to avoid merge commits

### Use Merge When

- Collaborating on shared branch
- Want to preserve context of branch
- Branch represents significant feature
- Need to show when features were integrated

## Performance

MediaGit rebase performance:
- **Small rebase** (<10 commits): < 1s
- **Medium rebase** (10-50 commits): < 5s
- **Large rebase** (50+ commits): ~0.1s per commit
- **Media files**: Chunk reuse makes it efficient (50-100 MB/s)

## Exit Status

- **0**: Rebase completed successfully
- **1**: Conflicts detected or rebase in progress
- **2**: Invalid operation or error

## Configuration

```toml
[rebase]
# Automatically stash before rebase
autostash = true

# Automatically squash fixup! commits
autosquash = true

# Use abbreviate commands in todo list
abbreviate_commands = false

# Update refs during rebase
update_refs = true

[rebase.media]
# Preserve compression during rebase
preserve_compression = true

# Maintain deduplication
maintain_dedup = true

# Chunk reuse optimization
chunk_reuse = true
```

## Common Workflows

### Update feature branch with latest main

```bash
$ mediagit checkout feature/my-feature
$ mediagit rebase main
# Apply feature commits on top of latest main
```

## Resolving Conflicts

### During Rebase

```bash
$ mediagit rebase main
CONFLICT (content): Merge conflict in config.json

# Check status
$ mediagit status
rebase in progress

# Resolve conflicts
$ vim config.json
$ mediagit add config.json

# Continue rebase
$ mediagit rebase --continue
```

### Multiple Conflicts

```bash
# Resolve first conflict
$ mediagit add file1.txt
$ mediagit rebase --continue

# Another conflict appears
CONFLICT (content): Merge conflict in file2.json
# Resolve and continue
$ vim file2.json
$ mediagit add file2.json
$ mediagit rebase --continue

# Repeat until complete
```

## Notes

### Golden Rule of Rebasing

**Never rebase commits that have been pushed to a public repository.**

Rebasing rewrites history, creating new commit OIDs. This causes problems for others who have based work on the old commits.

### Safe Rebase Scenarios

✅ **Safe**:
- Local commits not yet pushed
- Feature branch only you work on
- After force-push to your fork

❌ **Dangerous**:
- Shared branches (main, develop)
- Commits already pulled by others
- Public release branches

### Force Push After Rebase

After rebasing pushed commits:

```bash
$ mediagit rebase main
$ mediagit push --force-with-lease origin feature/my-branch
# Use --force-with-lease to prevent overwriting others' work
```

### Large Media File Considerations

MediaGit optimizes rebase for media files:
- Chunks are reused across commits
- Compression preserved from original
- Deduplication maintained
- Fast processing (50-100 MB/s)

## Troubleshooting

### Rebase got messy

```bash
$ mediagit rebase --abort
# Start over with clean state
```

### Lost commits during rebase

```bash
$ mediagit reflog
# Find lost commits
$ mediagit reset --hard <commit-before-rebase>
```

### Too many conflicts

```bash
# Consider merge instead
$ mediagit rebase --abort
$ mediagit merge main
```

## See Also

- [mediagit merge](./merge.md) - Join branches together
- [mediagit branch](./branch.md) - Manage branches
- [mediagit commit](./commit.md) - Record changes
- [mediagit reset](./reset.md) - Reset current HEAD
- [mediagit reflog](./reflog.md) - Show reference log
