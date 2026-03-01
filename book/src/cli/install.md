# mediagit install

Install MediaGit filter driver and hooks into a Git repository.

## Synopsis

```bash
mediagit install [OPTIONS]
```

## Description

Registers the MediaGit filter driver in the Git configuration so that files
marked with `filter=mediagit` in `.gitattributes` are transparently managed
through MediaGit's object store.

After installation, use [mediagit track](./track.md) to specify which file
patterns should be handled by MediaGit.

## Options

#### `-f`, `--force`
Force reinstallation even if the filter driver is already configured.

#### `-r <PATH>`, `--repo <PATH>`
Path to the Git repository to configure (default: current directory).

#### `-g`, `--global`
Install globally for all Git repositories (writes to `~/.gitconfig` instead
of the repository's `.git/config`).

## Examples

### Install in current Git repository

```bash
$ mediagit install
Installing MediaGit filter driver...
✓ Filter driver registered in .git/config
✓ Run 'mediagit track <PATTERN>' to configure tracked file types
```

### Install globally

```bash
$ mediagit install --global
✓ MediaGit filter driver registered globally in ~/.gitconfig
```

### Install in a specific repository

```bash
$ mediagit install --repo /path/to/my-git-repo
```

### Force reinstall

```bash
$ mediagit install --force
```

## What Gets Configured

Running `mediagit install` adds to `.git/config` (or `~/.gitconfig` with
`--global`):

```toml
[filter "mediagit"]
    clean  = mediagit filter clean %f
    smudge = mediagit filter smudge %f
    required = true
```

## Full Setup Workflow

```bash
# 1. In your Git repository, install the filter driver
mediagit install

# 2. Configure which file types go through MediaGit
mediagit track "*.psd"
mediagit track "*.mp4"
mediagit track "*.wav"

# 3. Now use Git normally — large files are transparently stored in MediaGit
git add textures/hero.psd    # MediaGit handles storage
git commit -m "Update hero texture"
git push
```

## Exit Status

- **0**: Success
- **1**: Not a Git repository or permission error

## See Also

- [mediagit track](./track.md) - Configure file tracking patterns
- [mediagit filter](./filter.md) - Filter driver internals
- [Advanced: Repository Migration](../advanced/migration.md)
