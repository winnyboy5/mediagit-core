# Quickstart Guide

Get up and running with MediaGit in 5 minutes!

## Prerequisites

- Rust 1.92.0 or later (if building from source)
- Git (for installing from source or contributing)

## Installation

### Quick Install (Recommended)

```bash
# Linux/macOS — one-liner install
curl -fsSL https://raw.githubusercontent.com/winnyboy5/mediagit-core/main/install.sh | sh
```

```powershell
# Windows (PowerShell)
Invoke-WebRequest -Uri "https://github.com/winnyboy5/mediagit-core/releases/download/v0.2.0/mediagit-0.2.0-x86_64-windows.zip" -OutFile mediagit.zip
Expand-Archive mediagit.zip -DestinationPath "$env:LOCALAPPDATA\MediaGit\bin"
```

### Docker

```bash
docker pull ghcr.io/winnyboy5/mediagit-core:0.2.0
docker run --rm ghcr.io/winnyboy5/mediagit-core:0.2.0 mediagit --version
```

### From Pre-built Binaries

Download the latest release for your platform from [GitHub Releases](https://github.com/winnyboy5/mediagit-core/releases):

| Platform | Archive |
|----------|---------|
| Linux x86_64 | `mediagit-0.2.0-x86_64-linux.tar.gz` |
| Linux ARM64 | `mediagit-0.2.0-aarch64-linux.tar.gz` |
| macOS Intel | `mediagit-0.2.0-x86_64-macos.tar.gz` |
| macOS Apple Silicon | `mediagit-0.2.0-aarch64-macos.tar.gz` |
| Windows x86_64 | `mediagit-0.2.0-x86_64-windows.zip` |

### From Source

```bash
git clone https://github.com/winnyboy5/mediagit-core.git
cd mediagit-core
cargo build --release
```

## Your First Repository

### 1. Initialize a Repository

```bash
mkdir my-media-project
cd my-media-project
mediagit init
```

Output:
```
✓ Initialized empty MediaGit repository in .mediagit/
```

### 2. Add Files

```bash
# Add a single file
mediagit add my-video.mp4

# Add multiple files
mediagit add images/*.jpg videos/*.mp4

# Add entire directory
mediagit add assets/
```

### 3. Check Status

```bash
mediagit status
```

Output:
```
On branch main

Changes to be committed:
  new file:   my-video.mp4
  new file:   images/photo1.jpg
  new file:   images/photo2.jpg
```

### 4. Commit Changes

```bash
mediagit commit -m "Initial commit: Add project media files"
```

Output:
```
[main abc1234] Initial commit: Add project media files
 3 files changed
 Compression ratio: 15.2% (saved 42.3 MB)
 Deduplication: 2 identical chunks found
```

### 5. View History

```bash
mediagit log
```

Output:
```
commit abc1234def5678
Author: Your Name <you@example.com>
Date:   Mon Nov 24 2025 12:00:00

    Initial commit: Add project media files

    Files: 3
    Size: 42.3 MB → 6.4 MB (84.8% savings)
```

## Working with Branches

### Create a Feature Branch

```bash
mediagit branch create feature/new-assets
mediagit branch switch feature/new-assets
```

### Make Changes

```bash
# Add new files
mediagit add new-video.mp4
mediagit commit -m "Add new promotional video"
```

### Merge Back to Main

```bash
mediagit branch switch main
mediagit merge feature/new-assets
```

## Storage Backend Configuration

MediaGit supports multiple storage backends. By default, it uses local filesystem storage.

### Configure AWS S3 Backend

```bash
# Edit .mediagit/config.toml
mediagit config set storage.backend s3
mediagit config set storage.s3.bucket my-media-bucket
mediagit config set storage.s3.region us-west-2
```

### Configure Azure Blob Storage

```bash
mediagit config set storage.backend azure
mediagit config set storage.azure.account my-storage-account
mediagit config set storage.azure.container media-container
```

See [Storage Backend Configuration](./guides/storage-config.md) for detailed setup instructions.

## Media-Aware Features

### Automatic Conflict Resolution for Images

When merging branches with image edits:

```bash
mediagit merge feature/photo-edits
```

MediaGit automatically detects:
- ✅ Non-overlapping edits (auto-merge)
- ✅ Metadata-only changes (auto-merge)
- ⚠️  Overlapping pixel edits (manual resolution required)

### PSD Layer Merging

MediaGit understands PSD layer structure:

```bash
mediagit merge feature/design-updates
```

- ✅ Different layer edits → Auto-merge
- ✅ New layers added → Auto-merge
- ⚠️  Same layer modified → Conflict marker

### Video Timeline Merging

MediaGit can merge non-overlapping video edits:

```bash
mediagit merge feature/video-cuts
```

- ✅ Different timeline ranges → Auto-merge
- ✅ Different audio tracks → Auto-merge
- ⚠️  Overlapping timeline edits → Manual resolution

## Performance Tips

### Enable Compression

Compression is enabled by default. Adjust levels in `.mediagit/config.toml`:

```toml
[compression]
algorithm = "zstd"  # or "brotli"
level = "default"   # "fast", "default", or "best"
```

### Delta Encoding

For incremental changes to large files:

```toml
[delta]
enabled = true
similarity_threshold = 0.80  # 80% similar = use delta
max_chain_depth = 10
```

### Deduplication

MediaGit automatically deduplicates identical content:

```bash
# Check deduplication statistics
mediagit stats

# Output:
# Total objects: 1,234
# Unique objects: 856 (69.4%)
# Deduplicated: 378 (30.6%)
# Space saved: 1.2 GB
```

## Next Steps

- 📚 [Basic Workflow Guide](./guides/basic-workflow.md) - Learn common workflows
- 🌿 [Branching Strategies](./guides/branching-strategies.md) - Effective branch management
- 🎨 [Merging Media Files](./guides/merging-media.md) - Advanced media-aware merging
- 🚀 [Performance Optimization](./guides/performance.md) - Optimize for large files
- 📖 [CLI Reference](./cli/README.md) - Complete command documentation

## Getting Help

- 📖 [Documentation](https://docs.mediagit.dev)
- 💬 [Discord Community](https://discord.gg/mediagit)
- 🐛 [Issue Tracker](https://github.com/winnyboy5/mediagit-core/issues)
- 📧 Email: support@mediagit.dev

## Common Issues

### Permission Denied on Install

```bash
# Linux/macOS: Use sudo
sudo sh -c 'curl -fsSL https://raw.githubusercontent.com/winnyboy5/mediagit-core/main/install.sh | sh'

# Or install to user directory
curl -fsSL https://raw.githubusercontent.com/winnyboy5/mediagit-core/main/install.sh | sh -s -- --no-sudo
```

### Command Not Found After Install

Add MediaGit to your PATH:

```bash
# Linux/macOS (bash)
echo 'export PATH="$HOME/.mediagit/bin:$PATH"' >> ~/.bashrc
source ~/.bashrc

# macOS (zsh)
echo 'export PATH="$HOME/.mediagit/bin:$PATH"' >> ~/.zshrc
source ~/.zshrc

# Windows
# Add %USERPROFILE%\.mediagit\bin to System PATH
```

### Large File Upload Timeout

Increase timeout in configuration:

```toml
[storage]
timeout_seconds = 300  # 5 minutes
```

For more troubleshooting, see the [Troubleshooting Guide](./guides/troubleshooting.md).
