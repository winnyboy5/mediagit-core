# Installation

MediaGit-Core provides pre-built binaries for all major platforms and architectures. Choose your platform below for detailed installation instructions.

## Quick Install

### Linux (x64)
```bash
curl -fsSL https://get.mediagit.dev/install.sh | bash
```

### macOS (Intel/ARM64)
```bash
brew install mediagit/tap/mediagit-core
```

### Windows (x64)
```powershell
choco install mediagit-core
```

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

# Should output: mediagit-core 0.1.0

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
# If installed via script
sudo rm /usr/local/bin/mediagit

# If installed via package manager
brew uninstall mediagit-core  # macOS
sudo apt remove mediagit-core  # Debian/Ubuntu
```

### Windows
```powershell
# If installed via Chocolatey
choco uninstall mediagit-core

# If installed manually
# Remove from Program Files and update PATH
```
