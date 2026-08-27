# mediagit merge

Join another branch's history into the current branch.

## Synopsis

```bash
mediagit merge [OPTIONS] <BRANCH>
mediagit merge --abort
mediagit merge --continue
```

## Description

Merges `BRANCH` (a branch name, tag, or commit OID — resolved the same way as
other revision arguments, including `refs/remotes/*` tracking refs) into the
current branch.

If the current branch is a plain ancestor of `BRANCH`, the merge fast-forwards
by default. Otherwise it computes a three-way merge and, if there are no
conflicts, creates a merge commit with both branches as parents.

## Options

| Flag | Description |
|---|---|
| `-m`, `--message <MESSAGE>` | Merge commit message (default: `Merge branch '<BRANCH>' into HEAD`, or a squash-merge equivalent with `--squash`) |
| `--no-ff` | Always create a merge commit, even when a fast-forward is possible |
| `--ff-only` | Fail instead of merging if a fast-forward isn't possible |
| `--squash` | Collapse `BRANCH`'s changes into a single-parent commit instead of a merge commit |
| `-s`, `--strategy <STRATEGY>` | `ours`, `theirs`, or `recursive` (default) |
| `-X`, `--strategy-option <OPTION>` | Accepted and parsed, but not currently applied by any strategy |
| `--no-commit` | Perform the merge but leave the result staged, uncommitted |
| `--abort` | Discard an in-progress merge and restore the pre-merge working tree |
| `-q`, `--quiet` | Suppress non-error output |
| `-v`, `--verbose` | Print conflict type detail |
| `--color <WHEN>` | Colored output: `always`, `auto`, or `never` (default `auto`) |
| `-C`, `--repository <PATH>` | Run as if invoked in `PATH` |
| `-h`, `--help` | Print help |
| `-V`, `--version` | Print version |

`--continue` finishes a merge that stopped on conflict: it builds a commit
from the staged (resolved) index and the pending `MERGE_HEAD`, with both the
pre-merge HEAD and `MERGE_HEAD` as parents.

`merge` refuses with an error before writing anything if the working tree has
uncommitted changes.

## Examples

### Merge a branch

```bash
mediagit merge feature/video-optimization
```

### Force a merge commit even when fast-forward is possible

```bash
mediagit merge --no-ff feature/hotfix
```

### Only merge if it's a fast-forward

```bash
mediagit merge --ff-only feature/hotfix
```

### Merge with a custom message

```bash
mediagit merge -m "Merge video optimization improvements" feature/optimize
```

### Resolve conflicts and continue

```bash
# merge reports conflicting paths and exits 1
vim config.json        # resolve a text conflict (inline <<<<<<< markers)
mediagit add config.json

# a binary conflict gets one side written to the working tree, unmarked;
# edit or replace it as needed, then stage it the same way
mediagit add video.mp4

mediagit merge --continue
```

### Abandon a conflicted merge

```bash
mediagit merge --abort
```

### Squash-merge

```bash
mediagit merge --squash feature/multiple-commits
mediagit commit -m "Add video optimization changes"
```

`--squash` leaves the result staged (like `--no-commit`) unless the merge was
fast-forward-eligible, in which case it commits immediately with a single
parent.

## See Also

- [mediagit branch](./branch.md) - Manage branches
- [mediagit rebase](./rebase.md) - Reapply commits on another base
- [mediagit diff](./diff.md) - Show changes between commits
- [mediagit status](./status.md) - Show working tree status
