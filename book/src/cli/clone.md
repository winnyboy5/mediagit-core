# mediagit clone

Clone a remote MediaGit repository.

## Synopsis

```bash
mediagit clone [OPTIONS] <URL> [DIRECTORY]
```

## Description

Creates a local copy of a remote MediaGit repository, downloading all objects and
setting up a remote named `origin` pointing to the source URL.

## Arguments

#### `<URL>`
Remote repository URL. Supports `http://`, `https://`, and `file://` schemes.

#### `[DIRECTORY]`
Local directory to clone into. Defaults to the repository name derived from the URL.

## Options

#### `-b <BRANCH>`, `--branch <BRANCH>`
Check out the specified branch after cloning instead of the default (`main`).

#### `-q`, `--quiet`
Suppress progress output.

#### `-v`, `--verbose`
Show detailed transfer information.

## Examples

### Basic clone

```bash
$ mediagit clone http://media-server.example.com/my-project
Cloning into 'my-project'...
Receiving objects: 100% (1,234 objects, 2.4 GB)
✓ Cloned 'my-project' in 12.3s
```

### Clone into specific directory

```bash
$ mediagit clone http://media-server.example.com/vfx-shots game-assets
Cloning into 'game-assets'...
```

### Clone specific branch

```bash
$ mediagit clone --branch production http://media-server.example.com/my-project
```

## After Cloning

```bash
cd my-project
mediagit log --oneline    # view history
mediagit status           # check working tree
```

## Exit Status

- **0**: Success
- **1**: Network error or repository not found
- **2**: Destination directory already exists

## See Also

- [mediagit remote](./remote.md) - Manage remote repositories
- [mediagit fetch](./fetch.md) - Fetch from remote
- [mediagit pull](./pull.md) - Fetch and merge from remote
