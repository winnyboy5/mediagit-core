# mediagit ls-files

List every file in the working folder with its status.

## Synopsis

```bash
mediagit ls-files [PATHS...] [--ignored]
```

## Description

Unlike `status`, which lists only what changed, `ls-files` lists every file, so a
file browser can show the whole folder. It uses the same working-folder walk and
`.mediagitignore` rules as `status`, and never descends into `.mediagit`.

Each file has one status: `unchanged`, `modified`, `new` (not in the last version),
`deleted` (in the last version, gone from disk) or `ignored`. `staged` is true when
the staging area holds a change for that path.

`PATHS` are repo-relative files or folders; only matching files are listed.

## Options

### `--ignored`
Also list files matched by `.mediagitignore` (status `ignored`).

## Machine output

`--machine` returns `{ "files": [ { "path", "status", "staged", "size" } ] }`, sorted by
path, with forward slashes. `size` is bytes on disk (null for deleted files).
