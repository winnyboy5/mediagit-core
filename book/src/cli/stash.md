# mediagit stash

Save and restore uncommitted changes.

## Synopsis

```bash
mediagit stash save [-m <MESSAGE>] [-u] [PATHS...]
mediagit stash list
mediagit stash show [<STASH>]
mediagit stash apply [<STASH>]
mediagit stash pop [<STASH>]
mediagit stash drop [<STASH>]
mediagit stash clear [--force]
```

## Description

Stash allows you to save uncommitted changes to a stack and restore them later.
Useful when you need to switch branches without committing work-in-progress.

Stash entries are stored in `.mediagit/stash/` as numbered entries.

## Subcommands

### `save`

Save current changes to the stash.

```bash
mediagit stash save [-m <MESSAGE>] [-u] [PATHS...]
```

Options:
- `-m`, `--message <MESSAGE>` — Descriptive message for the stash entry
- `-u`, `--include-untracked` — Also stash untracked files
- `PATHS` — Stash only specific files/directories (optional)
- `-q`, `--quiet` — Suppress output

### `list`

Show all stash entries.

```bash
mediagit stash list
```

### `show`

Show the diff of a stash entry.

```bash
mediagit stash show [<STASH>]
```

`STASH` defaults to `stash@{0}` (most recent).

### `apply`

Apply a stash entry without removing it from the stack.

```bash
mediagit stash apply [<STASH>]
```

### `pop`

Apply and remove the most recent (or specified) stash entry.

```bash
mediagit stash pop [<STASH>]
```

### `drop`

Remove a stash entry without applying it.

```bash
mediagit stash drop [<STASH>]
```

### `clear`

Remove all stash entries. Prompts for confirmation unless `--force` is specified.

```bash
mediagit stash clear [--force]
```

Options:
- `-f`, `--force` — Skip confirmation prompt (useful in scripts/CI)

## Examples

### Save current changes

```bash
$ mediagit stash save -m "WIP: texture adjustments"
Stashed changes in stash@{0}: WIP: texture adjustments
```

### List stashes

```bash
$ mediagit stash list
stash@{0}: WIP: texture adjustments (3 files, 145 MB)
stash@{1}: Experimental lighting rig
stash@{2}: Audio sync fixes
```

### Apply and remove latest stash

```bash
$ mediagit stash pop
Applying stash@{0}: WIP: texture adjustments
3 files restored
```

### Apply an older stash

```bash
$ mediagit stash apply stash@{2}
```

### Stash including untracked files

```bash
$ mediagit stash save --include-untracked -m "Work in progress"
```

### Clear all stashes in a script

```bash
$ mediagit stash clear --force
All stash entries cleared.
```

## Exit Status

- **0**: Success
- **1**: No stash entries found or stash index out of range

## See Also

- [mediagit status](./status.md) - Show working tree status
- [mediagit branch](./branch.md) - Switch branches
- [mediagit reset](./reset.md) - Reset working tree changes
