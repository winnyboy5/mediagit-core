# mediagit reflog

Show the history of HEAD and branch movements.

## Synopsis

```bash
mediagit reflog [OPTIONS] [REF]
mediagit reflog expire
mediagit reflog delete
```

## Description

The reflog records every time HEAD or a branch pointer moves — including commits,
merges, rebases, resets, checkouts, and cherry-picks. It is the safety net for
recovering from accidental resets and history rewrites.

Reflog entries are local to your repository and not shared when pushing.

## Arguments

#### `[REF]`
Reference to show the reflog for (default: `HEAD`). Examples: `main`,
`feature/vfx`, `HEAD`.

## Options

#### `-n <COUNT>`, `--count <COUNT>`
Limit output to the most recent N entries.

#### `-q`, `--quiet`
Show only OIDs without messages.

## Subcommands

### `expire`

Remove reflog entries older than the configured retention period (default: 90 days).

### `delete`

Remove all reflog entries for a specific ref.

## Examples

### View HEAD reflog

```bash
$ mediagit reflog
HEAD@{0}  abc1234  reset:      moving to HEAD~1
HEAD@{1}  def5678  commit:     Add hero texture v3
HEAD@{2}  ghi9012  cherry-pick: Pick lighting fix
HEAD@{3}  jkl3456  merge:      Merge branch 'feature/audio'
HEAD@{4}  mno7890  checkout:   switching from main to feature/audio
```

### Limit to recent entries

```bash
$ mediagit reflog --count 5
```

### View reflog for a branch

```bash
$ mediagit reflog main
main@{0}  abc1234  reset:  moving to HEAD~1
main@{1}  def5678  commit: Add hero texture v3
```

### Recover from accidental reset

```bash
# Accidentally reset too far back
$ mediagit reset --hard HEAD~5

# Find the lost commit in the reflog
$ mediagit reflog --count 10
HEAD@{0}  oldoid   reset: moving to HEAD~5
HEAD@{1}  abc1234  commit: Add final render pass (THIS IS WHAT WE WANT)

# Restore
$ mediagit reset --hard HEAD@{1}
HEAD is now at abc1234 Add final render pass
```

### Recover after accidental branch deletion

```bash
# View reflog to find the last commit on the deleted branch
$ mediagit reflog
...
HEAD@{3}  abc1234  checkout: moving from feature/vfx to main

# Recreate the branch at that commit
$ mediagit branch create feature/vfx abc1234
```

## When Reflog Entries Are Created

- `commit` — New commit created
- `reset` — Branch pointer moved with `reset`
- `checkout` — Switched branch with `branch switch`
- `merge` — Branches merged
- `rebase` — Commits rebased
- `cherry-pick` — Commits applied via cherry-pick
- `revert` — Revert commit created

## Exit Status

- **0**: Success
- **1**: Ref not found or reflog is empty

## See Also

- [mediagit reset](./reset.md) - Reset branch (use reflog to recover)
- [mediagit log](./log.md) - Show commit history
- [mediagit branch](./branch.md) - Manage branches
