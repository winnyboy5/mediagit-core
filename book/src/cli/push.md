# mediagit push

Update remote references and send the objects needed to satisfy them.

## Synopsis

```bash
mediagit push [OPTIONS] [REMOTE] [REFSPEC]...
```

## Description

Pushes local commits to `REMOTE` (default `origin`), updating the remote's
refs to point at the new commits and uploading whatever objects the remote is
missing. With no `REFSPEC`, pushes the branch HEAD currently points to.

`REFSPEC` accepts bare names, which are resolved as a branch (`refs/heads/*`)
first and a tag (`refs/tags/*`) if no matching branch exists locally.

Chunk data is uploaded to the storage backend, deduplicated against what the
remote already has. If the repository is encrypted, its key is escrowed with
the remote before the first object is uploaded.

Pushing a new, non-default branch (i.e. not `main`/`master`) without
`-u`/`--set-upstream` or `--no-track` is refused with a suggestion to rerun
with `-u`.

## Options

| Flag | Description |
|---|---|
| `-a`, `--all` | Push every local branch |
| `--tags` | Also push every local tag |
| `--follow-tags` | Also push tags that are ancestors of a pushed branch tip |
| `--dry-run` | Report what would be pushed without sending anything |
| `-f`, `--force` | Force-update remote refs, ignoring divergence |
| `--force-with-lease` | Force-update, but refuse if the remote ref has moved since it was last seen locally |
| `-d`, `--delete` | Delete the given ref(s) on the remote; requires at least one `REFSPEC` |
| `-u`, `--set-upstream` | Record the pushed branch's upstream as `REMOTE`/branch |
| `--no-track` | Push a new branch without setting an upstream |
| `-q`, `--quiet` | Suppress non-error output |
| `-v`, `--verbose` | Print remote URL, refspecs, and per-ref detail |
| `--repair` | Strong-verify (BLAKE3) every chunk reachable from the pushed refs and force re-upload any the remote reports as corrupted, using the local repo as source of truth. Runs even when refs are already up to date |
| `--color <WHEN>` | Colored output: `always`, `auto`, or `never` (default `auto`) |
| `-C`, `--repository <PATH>` | Run as if invoked in `PATH` |
| `-h`, `--help` | Print help |
| `-V`, `--version` | Print version |

## Examples

### Push the current branch

```bash
mediagit push
```

### Push a new branch and track it

```bash
mediagit push -u origin feature/video-optimization
```

### Push every branch

```bash
mediagit push --all
```

### Push tags reachable from what's being pushed

```bash
mediagit push --follow-tags
```

### Force-push safely

```bash
mediagit push --force-with-lease
```

### Delete a remote branch

```bash
mediagit push origin --delete feature/old-branch
```

### Preview without sending

```bash
mediagit push --dry-run
```

### Re-verify and repair remote chunks

```bash
mediagit push --repair
```

Use this after a remote is suspected to hold a corrupted chunk (e.g. from an
interrupted or poisoned upload) — it runs even if every ref is already
up to date, which ordinary push dedup would otherwise skip.

## See Also

- [mediagit pull](./pull.md) - Fetch and merge from remote
- [mediagit fetch](./fetch.md) - Download from remote
- [mediagit remote](./remote.md) - Manage remote repositories
- [mediagit branch](./branch.md) - Manage branches
