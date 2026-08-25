# mediagit show

Show commit details and contents.

## Synopsis

```bash
mediagit show [OPTIONS] [<object>]
```

## Description

Shows a commit object (defaults to `HEAD`). Displays the commit's author, date,
message, and a summary of file changes (added, modified, deleted). Use
`-v`/`--verbose` to include the tree OID and parent OIDs.

## Options

#### `<object>`
Object to show (commit OID, abbreviated hash, branch name, tag, or `HEAD`). Defaults to `HEAD`.

#### `-v`, `--verbose`
Show additional details (tree OID, parent OIDs).

#### `-q`, `--quiet`
Suppress all output (useful in scripts).

## Examples

### Show HEAD commit

```bash
$ mediagit show
commit a3c8f9d2e1b4f6a8c5d7e9f1a2b3c4d5e6f7a8b9c0d1e2f3a4b5c6d7e8f9a0b1
Author: Alice Developer <alice@example.com>
Date:   2024-01-15 14:30:22

    Add promotional video assets

---
 videos/promo_1080p.mp4 | new file
 videos/promo_4k.mp4    | new file
 metadata.json          | modified
 3 file(s) changed, 2 added, 1 modified, 0 deleted
```

### Show specific commit

```bash
$ mediagit show b4d7e1a
commit b4d7e1a9f2c3d4e5f6a7b8c9d0e1f2a3b4c5d6e7f8a9b0c1d2e3f4a5b6c7d8e9
Author: Bob Designer <bob@example.com>
Date:   2024-01-14 09:15:47

    Update brand identity assets

---
 assets/logo_old.png | deleted
 assets/logo_new.png | new file
 2 file(s) changed, 1 added, 0 modified, 1 deleted
```

### Show with verbose detail (tree OID, parents)

```bash
$ mediagit show -v HEAD
commit a3c8f9d2e1b4f6a8c5d7e9f1a2b3c4d5e6f7a8b9c0d1e2f3a4b5c6d7e8f9a0b1
Author: Alice Developer <alice@example.com>
Date:   2024-01-15 14:30:22

    Add promotional video assets

Tree: fb3a8bdd0ceddd019615af4d57a53f43d8cee2bfa1b2c3d4e5f6a7b8c9d0e1f2a3
Parents:
  b4d7e1a9f2c3d4e5f6a7b8c9d0e1f2a3b4c5d6e7f8a9b0c1d2e3f4a5b6c7d8e9

---
 videos/promo_1080p.mp4 | new file
 ...
```

### Show parent commit

```bash
$ mediagit show HEAD~1
$ mediagit show HEAD^
```

### Show a branch tip

```bash
$ mediagit show feature/new-assets
```

## Revision Syntax

```bash
mediagit show HEAD          # current commit
mediagit show HEAD~1        # one commit back
mediagit show HEAD^         # first parent
mediagit show a3c8f9d       # abbreviated BLAKE3 OID
mediagit show main          # branch tip
```

## Exit Status

- **0**: Object shown successfully
- **1**: Object not found or repository error

## Notes

### Object Addressing

MediaGit uses BLAKE3 for all objects. Full OIDs are 64 hexadecimal characters;
abbreviated forms (first 7+ chars) are accepted wherever an OID is expected.


## Common options

Accepted by this command in addition to the options above.

| Flag | Description |
|---|---|
| `--color <WHEN>` | Colored output: `always`, `auto`, or `never` (default `auto`) |
| `-C`, `--repository <PATH>` | Run as if invoked in `PATH` |
| `-h`, `--help` | Print help |
| `-V`, `--version` | Print version |
## See Also

- [mediagit log](./log.md) - Show commit history
- [mediagit diff](./diff.md) - Show changes between commits
- [mediagit verify](./verify.md) - Verify object integrity
