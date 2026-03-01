# mediagit revert

Revert a commit by creating a new inverse commit.

## Synopsis

```bash
mediagit revert [OPTIONS] <COMMITS>...
mediagit revert --continue
mediagit revert --abort
mediagit revert --skip
```

## Description

Creates new commits that undo the changes introduced by the specified commits.
Unlike `mediagit reset`, `revert` **preserves history** — the original commits
remain, and new "revert" commits are added. Safe to use on pushed commits.

Multiple commits are reverted in reverse order (newest first).

## Arguments

#### `<COMMITS>`
One or more commits to revert. Accepts full or short OIDs and branch-relative
refs (e.g., `HEAD~2`).

## Options

#### `-n`, `--no-commit`
Apply the inverse changes to the working tree without creating commits.
Allows inspection and editing before committing.

#### `-m <MESSAGE>`, `--message <MESSAGE>`
Override the auto-generated revert commit message.

#### `--continue`
Continue reverting after manually resolving conflicts.

#### `--abort`
Abort a multi-commit revert in progress and restore pre-revert state.

#### `--skip`
Skip the current commit during a multi-commit revert sequence.

#### `-q`, `--quiet`
Suppress output.

## Examples

### Revert the most recent commit

```bash
$ mediagit revert HEAD
[main abc1234] Revert "Add experimental render pass"
1 file changed, 145 MB removed
```

### Revert a specific commit

```bash
$ mediagit revert def5678
[main ghi9012] Revert "Update hero texture"
```

### Revert multiple commits

```bash
$ mediagit revert HEAD~3..HEAD
Reverting: Update audio mix (3/3)
Reverting: Add V2 render assets (2/3)
Reverting: Adjust lighting rig (1/3)
3 revert commits created.
```

### Revert without committing (inspect first)

```bash
$ mediagit revert --no-commit HEAD~2..HEAD
Changes staged but not committed. Review and then:
  mediagit commit -m "Revert recent changes"
```

### Handle conflicts during revert

```bash
$ mediagit revert abc1234
CONFLICT (content): Merge conflict in scene.psd
error: could not revert abc1234 -- resolve conflicts manually

# Resolve conflicts, then:
$ mediagit add scene.psd
$ mediagit revert --continue

# Or skip this commit:
$ mediagit revert --skip

# Or abort entirely:
$ mediagit revert --abort
```

## When to Use Revert vs Reset

| Situation | Command |
|-----------|---------|
| Undo commits that haven't been pushed | `reset --hard` |
| Undo pushed commits (preserve history) | `revert` |
| Keep the change but remove from branch | `reset --soft` |
| Undo a specific old commit in history | `revert <hash>` |

## Exit Status

- **0**: Success
- **1**: Conflicts detected, manual resolution required
- **2**: Commit not found or invalid operation

## See Also

- [mediagit reset](./reset.md) - Reset branch pointer (rewrites history)
- [mediagit cherry-pick](./cherry-pick.md) - Apply commits from another branch
- [mediagit log](./log.md) - Find commits to revert
