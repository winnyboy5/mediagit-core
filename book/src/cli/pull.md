# mediagit pull

Fetch changes from a remote and integrate them into the current branch.

## Synopsis

```bash
mediagit pull [OPTIONS] [REMOTE] [BRANCH]
```

## Description

`pull` fetches every branch ref from `REMOTE` (default `origin`), updates the
local `refs/remotes/<remote>/*` tracking refs, then integrates the pulled
branch into the current branch:

- If the current branch is a plain ancestor of the remote tip, it fast-forwards.
- Otherwise it merges (default) or rebases (`--rebase`) the remote tip in.

If `BRANCH` is given, that branch's ref is fetched and updated, but it is only
merged/rebased into the working tree when it is also the branch HEAD is
currently on — pulling a different branch just updates its ref.

Objects transfer over a streaming protocol; already-present chunks are skipped
automatically, so re-running `pull` after an interruption only downloads what
is still missing.

Before any object moves, `pull` checks that the local repository's encryption
state matches the remote's and refuses if they disagree.

## Options

| Flag | Description |
|---|---|
| `-r`, `--rebase` | Rebase the current branch onto the remote tip instead of merging |
| `--dry-run` | Fetch and report what would change, without writing any ref or file |
| `-q`, `--quiet` | Suppress non-error output |
| `-v`, `--verbose` | Print remote URL, ref names, and per-object detail |
| `--color <WHEN>` | Colored output: `always`, `auto`, or `never` (default `auto`) |
| `-C`, `--repository <PATH>` | Run as if invoked in `PATH` |
| `-h`, `--help` | Print help |
| `-V`, `--version` | Print version |

`pull` refuses with an error before touching anything if the working tree has
uncommitted changes that the fast-forward, merge, or rebase would overwrite.

## Examples

### Pull the current branch

```bash
mediagit pull
```

Fetches `origin`, updates tracking refs, and fast-forwards or merges the
current branch.

### Pull and rebase instead of merge

```bash
mediagit pull --rebase
```

### Pull a specific remote and branch

```bash
mediagit pull origin release/v2
```

Updates `refs/remotes/origin/release/v2`. Only merges/rebases into the working
tree if `release/v2` is the currently checked-out branch.

### Preview without changing anything

```bash
mediagit pull --dry-run
```

## Conflicts

When the merge or rebase path hits a conflict, `pull` stops with the
conflicting paths staged provisionally and reports them. Text conflicts get
inline `<<<<<<<`/`=======`/`>>>>>>>` markers; binary files are never marked
inline (that would corrupt them) — one side is written to the working tree and
flagged unresolved instead. Edit or replace each conflicting file as needed,
then run `mediagit add <path>` to acknowledge it (this is the resolution step
for binary files too, whether or not you changed the content).

A pull that stops on conflict during the merge path is finished the same way
`mediagit merge` conflicts are: see [mediagit merge](./merge.md). A pull that
stops during the rebase path is resumed or discarded the same way
`mediagit rebase` conflicts are: see [mediagit rebase](./rebase.md).

## See Also

- [mediagit fetch](./fetch.md) - Download without integrating
- [mediagit merge](./merge.md) - Join branches together
- [mediagit rebase](./rebase.md) - Reapply commits on another base
- [mediagit push](./push.md) - Update a remote repository
