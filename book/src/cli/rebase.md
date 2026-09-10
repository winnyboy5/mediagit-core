# mediagit rebase

Replay the commits unique to a branch onto a new base.

## Synopsis

```bash
mediagit rebase [OPTIONS] <UPSTREAM> [BRANCH]
mediagit rebase --continue
mediagit rebase --skip
mediagit rebase --abort
```

## Description

Finds the common ancestor of `BRANCH` (default: current branch) and
`UPSTREAM`, then replays each commit unique to `BRANCH` on top of `UPSTREAM`
in order, producing new commits with new OIDs.

`UPSTREAM` accepts a branch name, a tag, a full or abbreviated commit OID, or
a remote-tracking ref such as `origin/main`.

Each commit is replayed as a real three-way merge (base = the commit's own
parent, ours = the new parent so far, theirs = the commit itself), not a
verbatim copy of its tree — so changes already present in `UPSTREAM` are kept
rather than clobbered.

`UPSTREAM` is required except with `--continue`, `--skip`, or `--abort`,
which resume or discard a rebase already in progress and take no upstream
argument.

## Options

| Flag | Description |
|---|---|
| `--keep-empty` | Keep commits that become empty after replay, instead of dropping them |
| `--abort` | Discard the in-progress rebase and restore HEAD to where it was before rebase started |
| `--skip` | Drop the commit that stopped the rebase on conflict and continue with the rest |
| `-q`, `--quiet` | Suppress non-error output |
| `-v`, `--verbose` | Print merge base, per-commit progress |
| `--color <WHEN>` | Colored output: `always`, `auto`, or `never` (default `auto`) |
| `-C`, `--repository <PATH>` | Run as if invoked in `PATH` |
| `-h`, `--help` | Print help |
| `-V`, `--version` | Print version |

`--continue` resumes a rebase that stopped on conflict: it commits the
resolution under the original commit's message and author, then keeps
replaying the remaining commits.

`rebase` refuses with an error before writing anything if the working tree
has uncommitted changes, and refuses to start a second rebase while one is
already in progress.

## Examples

### Rebase the current branch onto main

```bash
mediagit rebase main
```

### Rebase a specific branch onto a tracking ref

```bash
mediagit rebase origin/main feature/audio-fix
```

### Resolve a conflict and continue

```bash
# rebase stops, reporting the conflicting path(s)
vim video.mp4          # or edit/replace as needed
mediagit add video.mp4
mediagit rebase --continue
```

Binary conflicts never get inline markers (that would corrupt the file); one
side is written to the working tree and flagged unresolved instead. Staging
the path with `add` is the acknowledgement `--continue` needs, whether or not
you changed the content.

### Drop a commit instead of resolving it

```bash
mediagit rebase --skip
```

### Abandon the rebase

```bash
mediagit rebase --abort
```

## See Also

- [mediagit merge](./merge.md) - Join branches together
- [mediagit branch](./branch.md) - Manage branches
- [mediagit commit](./commit.md) - Record changes
- [mediagit reflog](./reflog.md) - Show reference logs
