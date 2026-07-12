# mediagit sparse-checkout

Materialize only part of the working tree.

## Synopsis

```bash
mediagit sparse-checkout <SUBCOMMAND>
```

## Description

Sparse checkout lets a large media repository keep only a subset of tracked
files on disk. Patterns are stored in `.mediagit/info/sparse-checkout`. The
file being absent, or having no pattern lines, means sparse checkout is
**disabled** — full checkout, every path included.

Two pattern styles:

- **Cone mode** (default): each pattern is a directory prefix, included
  recursively.
- **Pattern mode** (`--patterns`): gitignore-style globs, where a match means
  *include* — the inverse of `.mediagitignore`, where a match means exclude.

## Semantics

- Excluded files are never written by ordinary checkout operations (branch
  switch, clone, reset, merge, etc.).
- A file that already exists on disk but is sparse-excluded is **never
  deleted** by those operations either — it is simply outside the cone, not
  a deletion target.
- Materializing newly-included files and removing newly-excluded ones is the
  explicit job of `sparse-checkout set` and `sparse-checkout disable` — not
  of ordinary checkout.
- `mediagit status` treats a sparse-excluded path as absent, not as a
  deletion: it never appears in `deleted`/porcelain `D` output.

This is MediaGit's own model, not a port of git's sparse-checkout — there is
no git interop goal.

## Subcommands

#### `set <PATTERN>... [--patterns]`
Write the given patterns to `.mediagit/info/sparse-checkout` and apply them
immediately against the current `HEAD` tree: newly-excluded tracked files are
removed from disk, newly-included ones are materialized.

#### `list` (alias `ls`)
Print the active mode (`cone` or `pattern`) and pattern list. Prints "Sparse
checkout is disabled" if there's no active pattern file.

#### `disable`
Remove the pattern file and materialize every tracked file that was excluded
— restores the full working tree, byte-identical to a fresh full checkout.

## Examples

### Cone mode: include two directories

```bash
$ mediagit sparse-checkout set assets/textures assets/audio
✔ Sparse checkout set: 2 pattern(s) (cone mode) — 143 file(s) removed, 0 file(s) added
```

### Pattern mode: gitignore-style globs

```bash
$ mediagit sparse-checkout set --patterns '*.png' '*.wav'
```

### List active patterns

```bash
$ mediagit sparse-checkout list
Mode: cone
assets/textures
assets/audio
```

### Restore the full tree

```bash
$ mediagit sparse-checkout disable
✔ Sparse checkout disabled; 143 file(s) restored.
```

## Exit Status

- **0**: Success
- **1**: Not inside a repository, or an I/O error materializing/removing a file

## See Also

- [mediagit status](./status.md) - Working tree status (sparse-excluded
  paths never show as deleted)
- [mediagit branch](./branch.md) - Branch switch keeps the active sparse
  filter
