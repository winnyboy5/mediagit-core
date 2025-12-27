# mediagit branch

List, create, or delete branches.

## Synopsis

```bash
mediagit branch [OPTIONS] [<branch>]
mediagit branch [OPTIONS] (-d | -D) <branch>...
mediagit branch [OPTIONS] (-m | -M) [<old-branch>] <new-branch>
mediagit branch [OPTIONS] (-c | -C) [<old-branch>] <new-branch>
```

## Description

Manage branches in MediaGit repository. With no arguments, lists existing branches. The current branch is highlighted with an asterisk.

Branch operations in MediaGit are lightweight and instant, as they simply create references to commit objects. Branches are ideal for organizing parallel work on media projects, feature development, or maintaining multiple versions.

## Options

### Branch Listing

#### `-l`, `--list [<pattern>]`
List branches matching optional pattern.

#### `-a`, `--all`
List both local and remote-tracking branches.

#### `-r`, `--remotes`
List remote-tracking branches only.

#### `-v`, `--verbose`
Show commit OID and message for each branch.

#### `-vv`
Show upstream branch and tracking status (ahead/behind).

#### `--merged [<commit>]`
List branches merged into specified commit (default: HEAD).

#### `--no-merged [<commit>]`
List branches not merged into specified commit.

#### `--contains <commit>`
List branches containing specified commit.

#### `--no-contains <commit>`
List branches not containing specified commit.

### Branch Creation

#### `<branch>`
Create new branch at current HEAD without switching to it.

#### `<branch> <start-point>`
Create new branch at specified commit/branch.

#### `-c`, `--copy [<old-branch>] <new-branch>`
Copy a branch and its reflog.

#### `-C`
Force copy even if target exists.

### Branch Deletion

#### `-d`, `--delete <branch>...`
Delete fully merged branch. Refuses if branch not merged.

#### `-D`
Force delete branch regardless of merge status.

### Branch Renaming

#### `-m`, `--move [<old-branch>] <new-branch>`
Rename branch. If old-branch omitted, rename current branch.

#### `-M`
Force rename even if new-branch exists.

### Branch Configuration

#### `-u <upstream>`, `--set-upstream-to=<upstream>`
Set upstream tracking for current or specified branch.

#### `--unset-upstream`
Remove upstream tracking information.

#### `-t`, `--track [direct|inherit]`
Set branch tracking mode when creating.

#### `--no-track`
Do not set up tracking even if configured.

### Display Options

#### `--color[=<when>]`
Color branches. When: **always**, **never**, **auto** (default).

#### `--no-color`
Turn off branch coloring.

#### `--column[=<options>]`
Display branches in columns.

#### `--sort=<key>`
Sort by key: **refname**, **objectsize**, **authordate**, **committerdate**.

## Examples

### List branches

```bash
$ mediagit branch
  feature/video-optimization
  feature/new-assets
* main
  release/v1.0
```

### List with details

```bash
$ mediagit branch -v
  feature/video-optimization a3c8f9d Optimize video encoding
  feature/new-assets         b4d7e1a Add new promotional assets
* main                       c5e9f2b Update README
  release/v1.0               d6f0a3c Release version 1.0
```

### List with tracking info

```bash
$ mediagit branch -vv
  feature/video-optimization a3c8f9d [origin/feature: ahead 2] Optimize encoding
  feature/new-assets         b4d7e1a Add new promotional assets
* main                       c5e9f2b [origin/main] Update README
  release/v1.0               d6f0a3c [origin/release/v1.0: behind 1] Release
```

### List all branches (including remotes)

```bash
$ mediagit branch -a
  feature/video-optimization
  feature/new-assets
* main
  release/v1.0
  remotes/origin/HEAD -> origin/main
  remotes/origin/main
  remotes/origin/feature/video-optimization
  remotes/origin/release/v1.0
```

### List remote branches only

```bash
$ mediagit branch -r
  origin/HEAD -> origin/main
  origin/main
  origin/feature/video-optimization
  origin/release/v1.0
```

### Create new branch

```bash
$ mediagit branch feature/audio-processing
$ mediagit branch -v
  feature/audio-processing   c5e9f2b Update README
  feature/video-optimization a3c8f9d Optimize video encoding
* main                       c5e9f2b Update README
```

### Create branch at specific commit

```bash
$ mediagit branch hotfix/urgent-fix a3c8f9d
$ mediagit branch -v
  hotfix/urgent-fix          a3c8f9d Optimize video encoding
* main                       c5e9f2b Update README
```

### Create and track remote branch

```bash
$ mediagit branch feature/new-feature origin/main
$ mediagit branch -u origin/feature/new-feature feature/new-feature
Branch 'feature/new-feature' set up to track 'origin/feature/new-feature'.
```

### Delete merged branch

```bash
$ mediagit branch -d feature/completed
Deleted branch feature/completed (was a3c8f9d).
```

### Force delete unmerged branch

```bash
$ mediagit branch -d feature/experimental
error: branch 'feature/experimental' not fully merged

$ mediagit branch -D feature/experimental
Deleted branch feature/experimental (was b4d7e1a).
warning: Deleted branch was not fully merged.
```

### Rename branch

```bash
$ mediagit branch -m feature/old-name feature/new-name
Renamed branch 'feature/old-name' to 'feature/new-name'.
```

### Rename current branch

```bash
$ mediagit branch -m new-branch-name
Renamed branch 'old-branch-name' to 'new-branch-name'.
```

### Copy branch

```bash
$ mediagit branch -c main backup-main
Created branch 'backup-main' from 'main'.
```

### List merged branches

```bash
$ mediagit branch --merged
  feature/completed-task-1
  feature/completed-task-2
* main
```

### List unmerged branches

```bash
$ mediagit branch --no-merged
  feature/in-progress
  feature/experimental
  hotfix/pending-review
```

### List branches containing commit

```bash
$ mediagit branch --contains a3c8f9d
  feature/video-optimization
* main
```

### Set upstream tracking

```bash
$ mediagit branch -u origin/main
Branch 'main' set up to track 'origin/main'.

$ mediagit branch -vv
* main c5e9f2b [origin/main] Update README
```

### Unset upstream tracking

```bash
$ mediagit branch --unset-upstream
Branch 'main' upstream tracking removed.
```

### Sort branches by date

```bash
$ mediagit branch --sort=-committerdate
* main                       (2024-01-15)
  feature/video-optimization (2024-01-14)
  feature/new-assets         (2024-01-12)
  release/v1.0               (2024-01-10)
```

### Filter branches with pattern

```bash
$ mediagit branch --list 'feature/*'
  feature/video-optimization
  feature/new-assets
  feature/audio-processing
```

## Branch Management Strategies

### Feature Branches

```bash
# Create feature branch
$ mediagit branch feature/add-transitions main
$ mediagit checkout feature/add-transitions

# Work on feature...

# When complete, merge back
$ mediagit checkout main
$ mediagit merge feature/add-transitions
$ mediagit branch -d feature/add-transitions
```

### Release Branches

```bash
# Create release branch
$ mediagit branch release/v2.0 main

# Finalize release
$ mediagit checkout release/v2.0
# Make release-specific changes...
$ mediagit commit -m "Prepare release v2.0"

# Tag release
$ mediagit tag v2.0

# Merge back to main
$ mediagit checkout main
$ mediagit merge release/v2.0
```

### Hotfix Workflow

```bash
# Create hotfix from production tag
$ mediagit branch hotfix/critical-fix v1.0
$ mediagit checkout hotfix/critical-fix

# Fix issue
$ mediagit commit -m "Fix critical audio glitch"

# Merge to main and release
$ mediagit checkout main
$ mediagit merge hotfix/critical-fix

$ mediagit checkout release/v1.0
$ mediagit merge hotfix/critical-fix
$ mediagit tag v1.0.1
```

## Branch Naming Conventions

Recommended patterns:

```
feature/<description>   - New features
  feature/video-editor
  feature/audio-mixer

bugfix/<description>    - Bug fixes
  bugfix/audio-sync
  bugfix/rendering-issue

hotfix/<description>    - Urgent production fixes
  hotfix/critical-crash
  hotfix/security-patch

release/<version>       - Release preparation
  release/v1.0
  release/v2.0-beta

experiment/<description> - Experimental work
  experiment/new-codec
  experiment/ml-upscaling
```

## Branch Storage

MediaGit branches are lightweight:
- Stored as references (refs/heads/<branch>)
- Simply point to commit OIDs
- No data duplication on creation
- Instant creation and deletion

```bash
$ mediagit branch feature/test
Created branch 'feature/test' (takes 0.001s)

$ ls -lh .mediagit/refs/heads/feature/test
-rw-r--r-- 1 user group 65 Jan 15 10:30 .mediagit/refs/heads/feature/test

$ cat .mediagit/refs/heads/feature/test
c5e9f2b4764f2dbcee52635b91fedb1b3dcf7ab4d5e6f7a8b9c0d1e2f3a4b5c6d7e8
```

## Performance

Branch operations are extremely fast:
- **List branches**: < 10ms (even with 1000+ branches)
- **Create branch**: < 1ms
- **Delete branch**: < 5ms
- **Rename branch**: < 5ms
- **List with details** (-v): < 50ms

## Exit Status

- **0**: Operation completed successfully
- **1**: Branch not found or operation failed
- **2**: Invalid options or branch name

## Configuration

```toml
[branch]
# Automatically set up tracking
autosetupmerge = true  # always | true | false

# Rebase on pull by default
autosetuprebase = never  # always | local | remote | never

# Sort order for branch listing
sort = "-committerdate"

[color.branch]
# Branch name colors
current = "green bold"
local = "normal"
remote = "red"
upstream = "blue"
```

## Branch Protection

MediaGit supports branch protection rules:

```toml
[branch "main"]
# Require review before merge
protected = true

# Prevent force push
allow_force_push = false

# Require status checks
require_checks = true
```

## Remote Tracking

Set up tracking for collaboration:

```bash
# Automatic tracking on push
$ mediagit push -u origin feature/new-feature

# Manual tracking setup
$ mediagit branch -u origin/feature/new-feature

# View tracking relationships
$ mediagit branch -vv
* feature/new-feature a3c8f9d [origin/feature/new-feature: ahead 2] Latest work
  main               c5e9f2b [origin/main] Update README
```

## Notes

### Detached HEAD

When not on a branch:
```bash
$ mediagit checkout a3c8f9d
You are in 'detached HEAD' state...

$ mediagit branch
* (HEAD detached at a3c8f9d)
  main
  feature/test
```

### Branch Deletion Safety

MediaGit prevents accidental deletion:
- Refuses to delete unmerged branches with `-d`
- Requires `-D` for force deletion
- Shows warning when deleting unmerged branches

### Large Repository Performance

For repositories with many branches:
- Use pattern matching: `--list 'feature/*'`
- Use `--no-merged` to focus on active work
- Sort by date: `--sort=-committerdate`

## See Also

- [mediagit checkout](./checkout.md) - Switch branches
- [mediagit merge](./merge.md) - Merge branches
- [mediagit rebase](./rebase.md) - Rebase branches
- [mediagit log](./log.md) - View branch history
- [mediagit remote](./remote.md) - Manage remote repositories
