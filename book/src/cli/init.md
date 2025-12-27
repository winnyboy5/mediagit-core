# mediagit init

Initialize a new MediaGit repository in the current directory.

## Synopsis

```bash
mediagit init [OPTIONS] [PATH]
```

## Description

Creates a new MediaGit repository by setting up the `.mediagit` directory structure with configuration files, object database, and references. This command prepares a directory for version control of media files.

If `PATH` is not specified, initializes the repository in the current directory.

## Options

### `--storage-backend <BACKEND>`
Storage backend to use for the object database.

- **Values**: `local`, `s3`, `azure`, `gcs`, `minio`, `b2`, `spaces`
- **Default**: `local`

### `--compression <ALGORITHM>`
Compression algorithm for storing objects.

- **Values**: `zstd`, `brotli`, `none`
- **Default**: `zstd`

### `--compression-level <LEVEL>`
Compression level to use.

- **Values**: `fast`, `default`, `best`
- **Default**: `default`

### `-b, --initial-branch <BRANCH>`
Name of the initial branch.

- **Default**: `main`

### `--bare`
Create a bare repository without a working directory.

## Examples

### Initialize in current directory
```bash
$ mediagit init
✓ Initialized empty MediaGit repository in .mediagit/
```

### Initialize with specific path
```bash
$ mediagit init my-media-project
✓ Initialized empty MediaGit repository in my-media-project/.mediagit/
```

### Initialize with S3 backend
```bash
$ mediagit init --storage-backend s3
✓ Initialized empty MediaGit repository in .mediagit/
✓ Configured AWS S3 storage backend
```

### Initialize with custom initial branch
```bash
$ mediagit init --initial-branch develop
✓ Initialized empty MediaGit repository in .mediagit/
✓ Created initial branch: develop
```

### Initialize bare repository
```bash
$ mediagit init --bare repo.git
✓ Initialized bare MediaGit repository in repo.git/
```

## Repository Structure

After initialization, the `.mediagit` directory contains:

```
.mediagit/
├── config.toml          # Repository configuration
├── HEAD                 # Current branch reference
├── objects/             # Object database
├── refs/
│   ├── heads/          # Branch references
│   └── tags/           # Tag references
└── index               # Staging area
```

## Configuration File

The generated `config.toml` contains default settings:

```toml
[core]
repository_format_version = 1
bare = false

[compression]
algorithm = "zstd"
level = "default"

[storage]
backend = "local"

[delta]
enabled = true
similarity_threshold = 0.80
max_chain_depth = 10

[cache]
max_size_mb = 1000
```

## Exit Status

- **0**: Successful initialization
- **1**: Directory already contains a repository
- **2**: Invalid options or configuration

## See Also

- [mediagit add](./add.md) - Add files to the staging area
- [mediagit commit](./commit.md) - Record changes to the repository
- [mediagit config](./config.md) - Get and set repository options
