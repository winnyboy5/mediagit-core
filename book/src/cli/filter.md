# mediagit filter

Git filter driver for clean/smudge operations.

## Synopsis

```bash
mediagit filter clean [<FILE>]
mediagit filter smudge [<FILE>]
```

## Description

Implements the Git clean/smudge filter protocol, enabling MediaGit to act as a
transparent filter driver within a Git repository. This command is invoked
**automatically by Git** — you do not call it directly during normal use.

- **clean**: Called by `git add`. Reads file content from stdin, stores the
  object in the MediaGit object store, and writes a pointer file to stdout.
- **smudge**: Called by `git checkout`. Reads a pointer file from stdin,
  retrieves the actual content from the MediaGit object store, and writes the
  file content to stdout.

For most users, the relevant setup commands are [mediagit install](./install.md)
and [mediagit track](./track.md).

## Arguments

#### `[FILE]`
The path of the file being processed. Used for logging and error messages.

## Filter Configuration

After running `mediagit install`, the following entry is added to `.git/config`
(or the global Git config with `--global`):

```toml
[filter "mediagit"]
    clean  = mediagit filter clean %f
    smudge = mediagit filter smudge %f
    required = true
```

Files registered with `filter=mediagit` in `.gitattributes` are transparently
processed through these filters.

## Minimum File Size

Files smaller than 1 MB are passed through without processing (stored directly
in Git). Only files ≥ 1 MB are managed by the MediaGit object store.

## Examples

These examples show the internal protocol — you do not run them manually.

### Clean filter (called by `git add`):

```
echo "file contents..." | mediagit filter clean textures/hero.psd
# Outputs: mediagit-pointer v1\noid sha256:abc1234...\nsize 145234567\n
```

### Smudge filter (called by `git checkout`):

```
echo "mediagit-pointer v1\noid sha256:abc1234...\n" | mediagit filter smudge textures/hero.psd
# Outputs: actual file contents (145 MB)
```

## See Also

- [mediagit install](./install.md) - Install the filter driver into a Git repo
- [mediagit track](./track.md) - Configure which files use the filter
- [Architecture: Git migration support](../architecture/storage-backends.md)
