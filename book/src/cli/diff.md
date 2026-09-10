# mediagit diff

Show changes between commits, working tree, and staging area.

## Synopsis

```bash
mediagit diff [OPTIONS] [<revision1>] [<revision2>] [--] [<path>...]
```

## Description

Shows differences between various states in MediaGit:

- Working tree vs staging area (default, no arguments)
- Staging area vs last commit (`--cached`)
- Working tree vs a specific commit (`mediagit diff <revision>`)
- Between two commits (`mediagit diff <revision1> <revision2>`)

For media/binary files, the diff reports content-level change (changed
chunks and sizes) rather than attempting a textual diff.

## Options

| Option | Description |
|--------|-------------|
| `--cached` | Compare the staging area with the last commit instead of the working tree |
| `--stat` | Show a diffstat summary (files changed, insertions/deletions/sizes) |
| `--summary` | Show a condensed summary of changes |
| `-U <num>`, `--unified <num>` | Number of context lines for text diffs |
| `-q`, `--quiet` | Suppress output; exit status indicates whether differences exist |
| `-- <path>...` | Limit the diff to the given paths |

## Examples

### Working tree vs staging area

```bash
$ mediagit diff
```

### Staged changes

```bash
$ mediagit diff --cached
```

### Working tree vs a commit

```bash
$ mediagit diff HEAD
$ mediagit diff HEAD~2
```

### Compare two commits

```bash
$ mediagit diff HEAD~1 HEAD
$ mediagit diff main feature/optimize
```

### Stat summary

```bash
$ mediagit diff --stat
```

### Limit to specific paths

```bash
$ mediagit diff -- video.mp4
$ mediagit diff HEAD -- assets/
```

## Comparing Specific States

| Command | Compares |
|---------|----------|
| `mediagit diff` | Working tree vs staging area |
| `mediagit diff --cached` | Staging area vs HEAD |
| `mediagit diff HEAD` | Working tree + staging vs HEAD |
| `mediagit diff <rev1> <rev2>` | Two committed states |

## Exit Status

- `0` — no differences (or diff printed successfully)
- non-zero — error resolving revisions or reading objects


## Common options

Accepted by this command in addition to the options above.

| Flag | Description |
|---|---|
| `--color <WHEN>` | Colored output: `always`, `auto`, or `never` (default `auto`) |
| `-C`, `--repository <PATH>` | Run as if invoked in `PATH` |
| `-v`, `--verbose` | Enable verbose output |
| `-h`, `--help` | Print help |
| `-V`, `--version` | Print version |
## See Also

- [mediagit status](./status.md)
- [mediagit log](./log.md)
- [mediagit show](./show.md)
