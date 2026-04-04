# Installation

MediaGit-Core provides pre-built binaries for all major platforms and architectures. Choose your platform below for detailed installation instructions.

## Quick Install

### Linux / macOS (one-liner)

```bash
curl -fsSL https://raw.githubusercontent.com/winnyboy5/mediagit-core/main/install.sh | sh
```

The install script automatically detects your OS and architecture and downloads the correct binary.

### Linux (x86_64) — manual

```bash
curl -fsSL https://github.com/winnyboy5/mediagit-core/releases/download/v0.2.6-beta.1/mediagit-0.2.6-beta.1-x86_64-linux.tar.gz \
  | tar xz -C /usr/local/bin
```

### macOS (Apple Silicon) — manual

```bash
curl -fsSL https://github.com/winnyboy5/mediagit-core/releases/download/v0.2.6-beta.1/mediagit-0.2.6-beta.1-aarch64-macos.tar.gz \
  | tar xz -C /usr/local/bin
```

### Windows (x86_64 — PowerShell)

```powershell
Invoke-WebRequest -Uri "https://github.com/winnyboy5/mediagit-core/releases/download/v0.2.6-beta.1/mediagit-0.2.6-beta.1-x86_64-windows.zip" -OutFile mediagit.zip
Expand-Archive mediagit.zip -DestinationPath "$env:LOCALAPPDATA\MediaGit\bin"
```

### Docker

```bash
docker pull ghcr.io/winnyboy5/mediagit-core:0.2.6-beta.1
docker run --rm ghcr.io/winnyboy5/mediagit-core:0.2.6-beta.1 mediagit --version
```

### All Release Archives

Each release on [GitHub Releases](https://github.com/winnyboy5/mediagit-core/releases) includes:

| Platform | Archive |
|----------|---------|
| Linux x86_64 | `mediagit-{VERSION}-x86_64-linux.tar.gz` |
| Linux ARM64 | `mediagit-{VERSION}-aarch64-linux.tar.gz` |
| macOS Intel | `mediagit-{VERSION}-x86_64-macos.tar.gz` |
| macOS Apple Silicon | `mediagit-{VERSION}-aarch64-macos.tar.gz` |
| Windows x86_64 | `mediagit-{VERSION}-x86_64-windows.zip` |

Each archive contains both `mediagit` (CLI) and `mediagit-server` binaries, with a corresponding `.sha256` checksum file.

## Platform-Specific Guides

- [Linux x64](./linux-x64.md) - Ubuntu, Debian, Fedora, Arch, etc.
- [Linux ARM64](./linux-arm64.md) - Raspberry Pi, ARM servers
- [macOS Intel](./macos-intel.md) - Intel-based Macs
- [macOS ARM64 (M1/M2/M3)](./macos-arm64.md) - Apple Silicon Macs
- [Windows x64](./windows-x64.md) - Windows 10/11 64-bit
- [Windows ARM64](./windows-arm64.md) - Windows on ARM (Surface Pro X, etc.)
- [Building from Source](./from-source.md) - Build with Rust/Cargo

## System Requirements

### Minimum Requirements
- **CPU**: x64 or ARM64 processor (2+ cores recommended)
- **RAM**: 512MB minimum, 2GB recommended
- **Disk**: 100MB for binaries, additional space for repositories
- **OS**: Linux (kernel 4.4+), macOS 10.15+, Windows 10+

### Recommended Requirements
- **CPU**: 4+ cores for parallel operations
- **RAM**: 4GB+ for large repositories
- **Disk**: SSD for best performance
- **Network**: Stable internet for cloud backends

## Verifying Installation

After installation, verify MediaGit-Core is working:

```bash
# Check version
mediagit --version

# Should output: mediagit-core 0.2.6-beta.1

# Run self-test
mediagit fsck --self-test

# Should output: All checks passed ✓
```

## Cloud Backend Setup (Optional)

If you plan to use cloud storage backends (S3, Azure, GCS, etc.), you'll need:

### AWS S3
```bash
# Install AWS CLI
# Configure credentials
aws configure

# MediaGit will use AWS credentials automatically
```

### Azure Blob Storage
```bash
# Install Azure CLI
az login

# MediaGit will use Azure credentials automatically
```

### Google Cloud Storage
```bash
# Install gcloud CLI
gcloud auth login

# MediaGit will use gcloud credentials automatically
```

## Next Steps

After installation:
1. Follow the [Quickstart Guide](../quickstart.md) for a 5-minute tutorial
2. Read [Configuration](../configuration.md) to customize MediaGit
3. Explore [CLI Reference](../cli/README.md) for all available commands

## Troubleshooting

If you encounter issues:
- Check [Troubleshooting Guide](../guides/troubleshooting.md)
- Verify system requirements above
- Ensure PATH is configured correctly
- Try building from source as fallback

## Uninstalling

### Linux/macOS
```bash
sudo rm /usr/local/bin/mediagit /usr/local/bin/mediagit-server
```

### Windows
```powershell
# Remove binary directory and clean PATH
Remove-Item "$env:LOCALAPPDATA\MediaGit" -Recurse -Force
```

### Docker
```bash
docker rmi ghcr.io/winnyboy5/mediagit-core:0.2.6-beta.1
```
