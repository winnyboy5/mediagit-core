# mediagit fetch

Download objects and refs from a remote repository without merging.

## Synopsis

```bash
mediagit fetch [OPTIONS] [REMOTE] [BRANCH]
```

## Description

Fetches branches and objects from a remote repository, updating remote-tracking
refs. Does **not** modify the local working tree — use `mediagit pull` to fetch
and merge in one step. Objects are downloaded via presigned-URL direct-from-backend
transfer, with server-proxy fallback.

## Arguments

#### `[REMOTE]`
Remote name (default: `origin`).

#### `[BRANCH]`
Specific branch to fetch. If omitted, fetches all branches.

## Options

#### `--all`
Fetch from all configured remotes.

#### `-p`, `--prune`
Remove remote-tracking refs that no longer exist on the remote.

#### `-q`, `--quiet`
Suppress progress output.

#### `-v`, `--verbose`
Show detailed transfer information.

## Examples

### Fetch from origin

```bash
$ mediagit fetch
From http://media-server.example.com/my-project
   abc1234..def5678  main       -> origin/main
   new branch        feature/vfx -> origin/feature/vfx
```

### Fetch specific branch

```bash
$ mediagit fetch origin feature/lighting
```

### Fetch from all remotes

```bash
$ mediagit fetch --all
```

### Prune deleted remote branches

```bash
$ mediagit fetch --prune
From http://media-server.example.com/my-project
 - [deleted]         (none)     -> origin/feature/old
```

## After Fetching

```bash
# Review changes from remote
mediagit log origin/main --oneline

# Merge fetched changes
mediagit merge origin/main
```

## Exit Status

- **0**: Success
- **1**: Network error or remote not found


## Common options

Accepted by this command in addition to the options above.

| Flag | Description |
|---|---|
| `--color <WHEN>` | Colored output: `always`, `auto`, or `never` (default `auto`) |
| `-C`, `--repository <PATH>` | Run as if invoked in `PATH` |
| `-h`, `--help` | Print help |
| `-V`, `--version` | Print version |
## See Also

- [mediagit pull](./pull.md) - Fetch and merge in one step
- [mediagit remote](./remote.md) - Manage remotes
- [mediagit merge](./merge.md) - Merge branches
