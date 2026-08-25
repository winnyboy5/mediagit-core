# mediagit branch

Create, list, rename, protect, and delete branches.

## Synopsis

```bash
mediagit branch <subcommand> [options]

mediagit branch list [-r|--remote] [-a|--all] [-v|--verbose] [--sort <key>]
mediagit branch create <name> [<start-point>] [-u <upstream>] [--track|--no-track]
mediagit branch switch <branch> [-c|--create] [--track] [-f|--force] [--no-guess]
mediagit branch delete <branches>... [-D|--force] [-d|--delete-merged] [-r|--remote]
mediagit branch rename [<old-name>] <new-name> [-f|--force]
mediagit branch show [<branch>] [-v|--verbose]
mediagit branch protect <branch> [--require-reviews] [--unprotect]
```

## Description

Branches are lightweight references to commits stored under `refs/heads/`,
enabling parallel development workflows.

`branch` requires a subcommand. As git-style sugar, bare invocations are
translated automatically: `mediagit branch` runs `branch list`, and
`mediagit branch <name>` runs `branch create <name>`. The `mediagit
checkout <name>` / `mediagit co <name>` shims translate to `branch switch
<name>`.

## Subcommands

### `list` (alias: `ls`)

List branches. This is the default when `branch` is run with no arguments.

| Option | Description |
|--------|-------------|
| `-r`, `--remote` | List remote branches |
| `-a`, `--all` | List all branches (local and remote) |
| `-v`, `--verbose` | Show verbose output |
| `-q`, `--quiet` | Quiet mode |
| `--sort <key>` | Sort branches |

### `create <name> [<start-point>]`

Create a new branch at `<start-point>` (defaults to HEAD).

| Option | Description |
|--------|-------------|
| `-u`, `--set-upstream <upstream>` | Set upstream branch |
| `--track` / `--no-track` | Control remote-branch tracking |
| `-q`, `--quiet` | Quiet mode |

### `switch <branch>` (shims: `checkout`, `co`)

Switch to a branch, updating the working tree.

| Option | Description |
|--------|-------------|
| `-c`, `--create` | Create and switch to a new branch |
| `--track` | With `--create` and a `<remote>/<name>` argument, create local `<name>` from the tracking ref and set it to track the remote branch |
| `-f`, `--force` | Switch even with local changes |
| `--no-guess` | Don't check out files |
| `-q`, `--quiet` | Quiet mode |

### `delete <branches>...`

Delete one or more branches.

| Option | Description |
|--------|-------------|
| `-d`, `--delete-merged` | Delete only if merged |
| `-D`, `--force` | Force delete (ignore merge status) |
| `-r`, `--remote` | Delete remote-tracking branches (e.g. `origin/feature`) |
| `-q`, `--quiet` | Quiet mode |

### `rename [<old-name>] <new-name>`

With one argument, renames the current branch; with two, renames `<old-name>`
to `<new-name>`. `-f`/`--force` overwrites an existing name.

### `show [<branch>]`

Show information about a branch (current branch when omitted). `-v` for
verbose output.

### `protect <branch>`

Protect a branch against force-push and deletion (see
`[protected_branches]` in the [Configuration Reference](../reference/config.md)).
`--require-reviews` additionally requires pull-request reviews before merge;
`--unprotect` removes protection.

## Examples

```bash
# List all local branches (bare `branch` does the same)
$ mediagit branch list

# List everything, verbose
$ mediagit branch list -a -v

# Create a branch from HEAD, then from a specific commit
$ mediagit branch create feature-textures
$ mediagit branch create hotfix abc123

# Create and switch in one step
$ mediagit branch switch -c feature-audio

# Track a remote branch locally
$ mediagit branch switch --track -c origin/feature-lighting

# Rename the current branch
$ mediagit branch rename new-name

# Delete a merged branch; force-delete an unmerged one
$ mediagit branch delete -d feature-done
$ mediagit branch delete -D feature-abandoned

# Protect main
$ mediagit branch protect main --require-reviews
```


## Common options

Accepted by this command in addition to the options above.

| Flag | Description |
|---|---|
| `--color <WHEN>` | Colored output: `always`, `auto`, or `never` (default `auto`) |
| `-C`, `--repository <PATH>` | Run as if invoked in `PATH` |
| `-h`, `--help` | Print help |
| `-V`, `--version` | Print version |
## See Also

- [mediagit merge](./merge.md)
- [mediagit tag](./tag.md)
- [Branch Model](../architecture/branching.md)
