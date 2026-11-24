# mediagit restore

Put chosen files back to a saved version, throwing away their unsaved changes.

## Synopsis

```bash
mediagit restore [--source <REV>] [--delete-new] [--yes] [--] <PATHS>...
```

## Description

Each named file is written back exactly as it is in the source version (the
latest version by default), and any staged change to it is dropped. A file
deleted from the working folder comes back.

**Unsaved changes are lost for good.** MediaGit has no copy of work that was
never saved in a version, so `restore` never acts without your confirmation:

- In a terminal it lists every file that loses its changes, with sizes, and
  asks `y/N` (default No).
- In scripts and with `--machine`, pass `--yes`. Without it the command is
  refused before any file is touched.

A file that is not in the source version (a new file) can only be "restored"
by deleting it. That needs `--delete-new` as well as the confirmation, and
folders it leaves empty are removed too.

If one file cannot be restored (for example, another program has it open),
the others still are; the failures are listed and the command exits non-zero.

## Arguments

#### `<PATHS>...`
Files to restore. Folders are refused: name the files. Paths outside the
repository and MediaGit's own `.mediagit` folder are refused.

## Options

#### `--source <REV>`
Version to take the files from: a commit id, branch, tag or `HEAD~N`
(default `HEAD`).

#### `--delete-new`
Also delete named files that are not in the source version. They are gone for
good.

#### `-y`, `--yes`
Do not ask. Required with `--machine` and when stdin is not a terminal.

#### `-q`, `--quiet`
Print nothing on success.

## Examples

```bash
# Throw away the edits to one file (asks first)
$ mediagit restore -- art/hero.psd

# Take a file from two versions ago
$ mediagit restore --source HEAD~2 -- art/hero.psd

# Delete a scratch render that was never saved (asks first)
$ mediagit restore --delete-new -- render/tmp.exr
```

## Exit status

`0` when every file was restored. Non-zero when the person said no, when
confirmation was needed but not given (`needs_input`), when a new file was
named without `--delete-new` (`would_delete`), for a refused path
(`invalid_args`), or when any file failed.

## See also

- [mediagit reset](./reset.md): move the branch, or unstage files without
  touching them
- [mediagit stash](./stash.md): set changes aside instead of throwing them away
