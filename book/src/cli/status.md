# mediagit status

Display the working tree status.

## Synopsis

```bash
mediagit status [OPTIONS]
```

## Description

Shows the state of the working directory and staging area:

- Files staged for commit (in the index)
- Files with modifications not staged
- Deleted files not yet staged for removal
- Untracked files not yet added
- Ignored files (when `--ignored` is set)
- Current branch, and — when the branch has a configured upstream — ahead/behind counts (with `-b`)
- A one-line summary of counts (and, with `-v`, total size)

`status` never performs network I/O. Ahead/behind counts are computed entirely from local refs; if the
upstream's remote-tracking ref has never been fetched, the upstream name is shown without counts rather
than fetching to find out.

## Options

### `--tracked`
Show only tracked-file sections (staged / modified / deleted). Passing neither `--tracked` nor
`--untracked` shows both; passing one alone hides the other.

### `--untracked`
Show only the untracked-files section. See `--tracked` above for the combination rule.

### `--ignored`
Show files excluded by `.mediagitignore` in an "Ignored files:" section. Ignored files are always
hidden from the "Untracked files:" list; this flag makes them visible instead.

### `-s, --short`
Use the short one-line-per-file status codes (`A `, ` M`, ` D`) instead of the long
`new file:`/`modified:`/`deleted:` labels.

### `--porcelain`
Machine-readable output: no headers, no colors, one `XY path` line per changed file, `?? path` for
untracked, `!! path` for ignored (with `--ignored`). This format is a stable contract — new status
codes may be added in future releases, but existing lines never change meaning. See
[Porcelain Format](#porcelain-format) below for the exact codes.

### `-b, --branch`
Show a branch header line (`On branch: <name>`), including upstream tracking and ahead/behind counts
when configured. See [Branch Tracking](#branch-tracking) below.

### `-q, --quiet`
Suppress all output except porcelain lines (porcelain output is unaffected by `--quiet`).

### `-v, --verbose`
Add a total size (staged + modified + untracked, `HumanBytes`-formatted) to the summary line.

### `--json`
Print a single JSON document to stdout instead of human-readable text, and suppress all colors,
headers, and progress output. See [JSON Output](#json-output) below for the schema.

### `--color <WHEN>`
Colored output: `always`, `auto`, or `never`. Default: `auto`. Global option, shared by every
`mediagit` subcommand.

### `-C, --repository <PATH>`
Run as if `status` was started in `<PATH>` instead of the current directory. Global option, shared
by every `mediagit` subcommand.

### `-h, --help`
Print help for `status` and exit.

### `-V, --version`
Print the `mediagit` version and exit.

## Long Format Output

```
Changes to be committed:
  (use "mediagit reset <file>..." to unstage)
  new file:   project_video.mp4

Changes not staged for commit:
  (use "mediagit add <file>..." to update what will be committed)
  modified:   README.md
  deleted:    old_notes.txt

Untracked files:
  (use "mediagit add <file>..." to include in what will be committed)
  draft_design.psd

1 staged, 1 modified, 1 deleted, 1 untracked

Nothing to commit, working tree clean
```

The summary line only appears when there is at least one staged, modified, deleted, or untracked file.
With `-v`, it gains a total-size suffix: `1 staged, 1 modified, 1 deleted, 1 untracked (total size: 4.2 MiB)`.

## Porcelain Format

```
M  path    staged, and the path already existed in HEAD (modified)
A  path    staged, and the path is new (not in HEAD)
 M path    modified in the working tree, not staged
 D path    deleted in the working tree, not staged
?? path    untracked
!! path    ignored (only emitted with --ignored)
```

Porcelain output has no header, no colors, and respects `--tracked`/`--untracked` the same way the
human format does.

## Branch Tracking

With `-b`, the branch line reflects upstream state when the current branch has one configured
(written by `clone` for the default branch, or by setting up a tracking branch via `branch switch`):

```bash
# No upstream configured — header unchanged
$ mediagit status -b
On branch: main

# Upstream configured, tracking ref never fetched locally — name only, no counts
$ mediagit status -b
On branch: main — origin/main

# Ahead only
$ mediagit status -b
On branch: main — origin/main: ahead 2

# Behind only
$ mediagit status -b
On branch: main — origin/main: behind 1

# Diverged
$ mediagit status -b
On branch: main — origin/main: ahead 2, behind 1

# Detached HEAD
$ mediagit status -b
HEAD detached at a3c8f9d1...
```

Ahead/behind counts are the sizes of the symmetric difference between the local branch's commit
ancestry and the remote-tracking ref's commit ancestry (`refs/remotes/<remote>/<branch>`). They
require the relevant commits to already be present in the local object database (i.e. `fetch` has
run); `status` itself never fetches.

## JSON Output

`--json` serializes the exact same data the human and porcelain formats are built from, as one JSON
document:

```json
{
  "format_version": 1,
  "branch": {
    "name": "main",
    "detached": false,
    "detached_oid": null,
    "has_commits": true,
    "upstream": {
      "name": "origin/main",
      "ahead": 2,
      "behind": 0
    }
  },
  "staged": [
    { "path": "project_video.mp4", "kind": "added" }
  ],
  "modified": ["README.md"],
  "deleted": ["old_notes.txt"],
  "untracked": ["draft_design.psd"],
  "ignored": [],
  "summary": {
    "staged": 1,
    "modified": 1,
    "deleted": 1,
    "untracked": 1,
    "total_size": null
  }
}
```

Notes:

- `format_version` only changes on an incompatible schema change.
- `branch.upstream` is `null` when the branch has no configured upstream.
- `upstream.ahead`/`upstream.behind` are `null` (not `0`) when the remote-tracking ref has never been
  fetched locally — distinguish this from a real 0/0 (in sync).
- `staged[].kind` is `"added"` or `"modified"`.
- `summary.total_size` is `null` unless `-v/--verbose` was also passed.
- `ignored` is always populated (regardless of `--ignored`); `--ignored` only affects whether the
  human/porcelain formats print it.

## Examples

### Basic status

```bash
$ mediagit status
Changes to be committed:
  (use "mediagit reset <file>..." to unstage)
  new file:   project_video.mp4

Changes not staged for commit:
  (use "mediagit add <file>..." to update what will be committed)
  modified:   README.md

Untracked files:
  (use "mediagit add <file>..." to include in what will be committed)
  draft_design.psd

1 staged, 1 modified, 0 deleted, 1 untracked
```

### Short format

```bash
$ mediagit status --short
A  project_video.mp4
 M README.md
?? draft_design.psd
```

### Branch header with ahead/behind

```bash
$ mediagit status -b
On branch: main — origin/main: ahead 2
```

### JSON

```bash
$ mediagit status --json
{"format_version":1,"branch":{"name":"main", ...}, ...}
```

### Show ignored files

```bash
$ mediagit status --ignored
Untracked files:
  (use "mediagit add <file>..." to include in what will be committed)
  new_asset.mp4

Ignored files:
  (add .mediagitignore negation '!<pattern>' to un-ignore)
  cache.tmp
```

### Machine-readable output

```bash
$ mediagit status --porcelain
A  project_video.mp4
 M README.md
?? draft_design.psd
```

### Clean working tree

```bash
$ mediagit status
Nothing to commit, working tree clean
```

### Detached HEAD

```bash
$ mediagit status -b
HEAD detached at a3c8f9d1e2f4b6c8d0a1b2c3d4e5f6a7b8c9d0e1
Nothing to commit, working tree clean
```

## See Also

- [mediagit add](./add.md) - Add files to the staging area
- [mediagit commit](./commit.md) - Record changes to the repository
- [mediagit diff](./diff.md) - Show changes between commits
- [mediagit branch](./branch.md) - List, create, or delete branches
- [mediagit fetch](./remote-operations.md) - Update remote-tracking refs (required before ahead/behind reflects the remote's latest state)
