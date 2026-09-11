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
curl -fsSL https://github.com/winnyboy5/mediagit-core/releases/download/v0.4.0-rc.1/mediagit-0.4.0-rc.1-x86_64-linux.tar.gz \
  | tar xz -C /usr/local/bin
```

### macOS (Apple Silicon) — manual

```bash
curl -fsSL https://github.com/winnyboy5/mediagit-core/releases/download/v0.4.0-rc.1/mediagit-0.4.0-rc.1-aarch64-macos.tar.gz \
  | tar xz -C /usr/local/bin
```

### Windows (x86_64 — PowerShell)

```powershell
Invoke-WebRequest -Uri "https://github.com/winnyboy5/mediagit-core/releases/download/v0.4.0-rc.1/mediagit-0.4.0-rc.1-x86_64-windows.zip" -OutFile mediagit.zip
Expand-Archive mediagit.zip -DestinationPath "$env:LOCALAPPDATA\MediaGit\bin"
```

### Docker

```bash
docker pull ghcr.io/winnyboy5/mediagit-core:0.4.0-rc.1
docker run --rm ghcr.io/winnyboy5/mediagit-core:0.4.0-rc.1 mediagit --version
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

# Should output: mediagit-core 0.3.0-rc.5

# Verify a repository's integrity (run inside a repo)
mediagit fsck --full

# Should output: ✅ Repository integrity: PERFECT
```

## Cloud Backend Setup (Optional)

If you plan to use cloud storage backends (S3, Azure, GCS), credentials come
from `.mediagit/config.toml`, not from the environment or a CLI-managed
credential store — with one exception (GCS). See
[Configuration Reference](../reference/config.md) for the full schema.

### AWS S3 (and S3-compatible / MinIO)

`aws configure` has no effect on MediaGit. Put the key and secret directly in
`config.toml`:

```toml
[storage]
backend = "s3"
bucket = "my-bucket"
region = "us-east-1"
access_key_id = "..."
secret_access_key = "..."
```

### Azure Blob Storage

`az login` has no effect on MediaGit. Put the credential in `config.toml` as
a tagged `auth` table under `[storage]`, e.g. `account_key`:

```toml
[storage]
backend = "azure"
container = "my-container"
auth = { type = "account_key", account_name = "...", account_key = "..." }
```

### Google Cloud Storage

GCS is the one backend that genuinely picks up ambient credentials: if
`credentials_path` is unset, MediaGit falls back to Application Default
Credentials, which honours `GOOGLE_APPLICATION_CREDENTIALS` and
`gcloud auth application-default login`.

```bash
# Install gcloud CLI
gcloud auth application-default login

# MediaGit will use ADC automatically
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
docker rmi ghcr.io/winnyboy5/mediagit-core:0.4.0-rc.1
```
