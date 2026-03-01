# mediagit track

Configure file tracking patterns for the MediaGit filter driver.

## Synopsis

```bash
mediagit track [<PATTERN>]
mediagit track --list
```

## Description

Manages `.gitattributes` entries that register glob patterns with the MediaGit
filter driver. When a pattern is tracked, files matching it are processed
through MediaGit's clean/smudge filters during `git add` and `git checkout`.

This command is for use when running MediaGit **alongside** a Git repository
(e.g., code in Git, media assets in MediaGit). For repositories using MediaGit
as the primary VCS, this command is not needed.

## Arguments

#### `[PATTERN]`
Glob pattern to track (e.g., `*.psd`, `*.mp4`, `assets/**`). Adds the pattern
to `.gitattributes`.

## Options

#### `-l`, `--list`
Show all currently tracked patterns without adding new ones.

## Examples

### Track PSD files

```bash
$ mediagit track "*.psd"
Tracking: *.psd (added to .gitattributes)
```

### Track multiple formats

```bash
$ mediagit track "*.mp4"
$ mediagit track "*.mov"
$ mediagit track "*.wav"
$ mediagit track "*.psd"
```

### Track an entire directory

```bash
$ mediagit track "assets/**"
Tracking: assets/** (added to .gitattributes)
```

### List tracked patterns

```bash
$ mediagit track --list
Tracked patterns:
  *.psd     filter=mediagit diff=mediagit merge=mediagit -text
  *.mp4     filter=mediagit diff=mediagit merge=mediagit -text
  *.wav     filter=mediagit diff=mediagit merge=mediagit -text
  assets/** filter=mediagit diff=mediagit merge=mediagit -text
```

## Generated .gitattributes

`mediagit track "*.psd"` appends to `.gitattributes`:

```
*.psd filter=mediagit diff=mediagit merge=mediagit -text
```

## Workflow with Git + MediaGit Side-by-Side

```bash
# In a Git repository, set up MediaGit filters:
mediagit install                    # Register filter driver in .git/config
mediagit track "*.psd" "*.mp4"     # Configure which files go through MediaGit

# Now git add/commit works transparently:
git add hero.psd                    # MediaGit filter stores object, Git tracks pointer
git commit -m "Update hero texture"
```

## See Also

- [mediagit install](./install.md) - Install MediaGit filter driver into a Git repo
- [mediagit filter](./filter.md) - Filter driver internals (clean/smudge)
- [mediagit add](./add.md) - Add files to a MediaGit repository directly
