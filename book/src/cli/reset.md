# mediagit reset

Reset the current branch to a specified commit.

## Synopsis

```bash
mediagit reset [--soft | --hard] [<COMMIT>]
mediagit reset <COMMIT> [--] <PATHS>...
```

## Description

Moves the current branch pointer to the specified commit. How the working tree
and staging area are affected depends on the reset mode.

When paths are specified, only those files are affected (path-specific reset
does not move the branch pointer).

## Arguments

#### `[COMMIT]`
Target commit (default: `HEAD`). Accepts full 64-char OIDs, short hashes (e.g.,
`abc1234`), branch names, or `HEAD~N` notation.

#### `[PATHS]`
Specific files to reset. Cannot be combined with `--soft` or `--hard`.

## Options

#### `--soft`
Move the branch pointer to `<COMMIT>`. Working tree and staging area are
unchanged — changes from undone commits appear as staged.

Passing neither `--soft` nor `--hard` moves the branch pointer and resets the staging area to
match `<COMMIT>` (this is the default mode); the working tree is unchanged and those changes
appear as unstaged.

#### `--hard`
Move the branch pointer, reset staging area, and discard all working tree
changes. **Destructive — cannot be undone without the reflog.**

#### `-q`, `--quiet`
Suppress output.

#### `-v`, `--verbose`
Enable verbose output.

#### `--color <WHEN>`
Colored output: `always`, `auto`, or `never`. Default: `auto`. Global option, shared by every
`mediagit` subcommand.

#### `-C`, `--repository <PATH>`
Run as if `reset` was started in `<PATH>` instead of the current directory. Global option, shared
by every `mediagit` subcommand.

#### `-h`, `--help`
Print help for `reset` and exit.

#### `-V`, `--version`
Print the `mediagit` version and exit.

## Examples

### Undo last commit, keep changes staged

```bash
$ mediagit reset --soft HEAD~1
```

### Undo last commit, unstage changes (default)

```bash
$ mediagit reset HEAD~1
```

### Discard last commit and all changes

```bash
$ mediagit reset --hard HEAD~1
```

### Reset to a specific commit

```bash
$ mediagit reset --hard abc1234def
HEAD is now at abc1234d Previous stable state
```

### Unstage specific files

```bash
$ mediagit reset -- textures/hero.psd
Unstaged: textures/hero.psd
```

### Recover from an accidental hard reset

```bash
# Find the lost commit in the reflog
$ mediagit reflog
HEAD@{0}: reset: moving to HEAD~1
HEAD@{1}: commit: Add hero texture

# Restore the lost commit
$ mediagit reset --hard HEAD@{1}
```

## Reset Modes Comparison

| Mode | Branch pointer | Staging area | Working tree |
|------|---------------|--------------|--------------|
| `--soft` | Moved | Unchanged | Unchanged |
| default (neither flag) | Moved | Reset | Unchanged |
| `--hard` | Moved | Reset | Reset (destructive) |

## Exit Status

- **0**: Success
- **1**: Commit not found or invalid options

## See Also

- [mediagit revert](./revert.md) - Undo a commit by creating an inverse commit
- [mediagit reflog](./reflog.md) - Recover from accidental resets
- [mediagit stash](./stash.md) - Stash changes without committing
