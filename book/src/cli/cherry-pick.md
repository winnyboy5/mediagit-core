# mediagit cherry-pick

Apply commits from another branch onto the current branch.

## Synopsis

```bash
mediagit cherry-pick [OPTIONS] <COMMITS>...
mediagit cherry-pick --continue
mediagit cherry-pick --abort
mediagit cherry-pick --skip
```

## Description

Applies the changes introduced by the specified commits to the current branch,
creating new commits. Useful for porting specific fixes or features from one
branch to another without merging the full branch.

Short commit hashes (e.g., from `mediagit log --oneline`) are supported.

## Arguments

#### `<COMMITS>`
One or more commits to apply. Applied in order (first to last). Accepts full
64-char OIDs or abbreviated short hashes.

## Options

#### `-n`, `--no-commit`
Apply changes to the working tree without committing. Allows editing before
the commit is created.

#### `-e`, `--edit`
Open an editor to modify the commit message before committing.

#### `-x`, `--append-message`
Append the original commit hash to the commit message for traceability:
`(cherry picked from commit abc1234...)`.

#### `--continue`
Continue cherry-pick after resolving conflicts.

#### `--abort`
Abort the cherry-pick sequence and restore the pre-operation state.

#### `--skip`
Skip the current commit and continue with the rest of the sequence.

#### `-q`, `--quiet`
Suppress output.

## Examples

### Cherry-pick a single commit

```bash
# Find the commit to pick
$ mediagit log feature/hotfix --oneline
abc1234 Fix audio sync regression

# Apply it to current branch
$ mediagit cherry-pick abc1234
[main ghi9012] Fix audio sync regression
 1 file changed, 2.3 MB modified
```

### Cherry-pick multiple commits

```bash
$ mediagit cherry-pick abc1234 def5678 ghi9012
Applying: Fix audio sync regression
Applying: Update hero texture pack
Applying: Adjust lighting parameters
3 commits cherry-picked.
```

### Cherry-pick with traceability

```bash
$ mediagit cherry-pick --append-message abc1234
[main xyz5678] Fix audio sync regression

(cherry picked from commit abc1234def5678...)
```

### Handle conflicts

```bash
$ mediagit cherry-pick abc1234
CONFLICT: Apply commit abc1234 to current HEAD
Automatic merge failed in: audio/master.wav

# Resolve the conflict:
$ mediagit add audio/master.wav
$ mediagit cherry-pick --continue

# Or skip this commit:
$ mediagit cherry-pick --skip

# Or abort:
$ mediagit cherry-pick --abort
```

### Apply without auto-committing

```bash
$ mediagit cherry-pick --no-commit abc1234 def5678
Changes applied but not committed. Review and then commit:
  mediagit commit -m "Apply hotfixes from v1.1 branch"
```

## Cherry-Pick vs Merge vs Rebase

| Scenario | Recommended |
|----------|-------------|
| Bring specific fix to another branch | `cherry-pick` |
| Integrate all work from a branch | `merge` |
| Move a series of commits onto new base | `rebase` |
| Undo a specific commit | `revert` |

## Exit Status

- **0**: Success
- **1**: Conflicts detected, manual resolution required
- **2**: Commit not found or invalid operation

## See Also

- [mediagit revert](./revert.md) - Undo a commit with an inverse commit
- [mediagit rebase](./rebase.md) - Reapply a series of commits
- [mediagit merge](./merge.md) - Merge a full branch
- [mediagit log](./log.md) - Find commits to cherry-pick
