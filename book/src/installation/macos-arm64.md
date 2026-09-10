# macOS ARM64 (Apple Silicon) Installation

MediaGit-Core is optimized for Apple Silicon (M1, M2, M3, M4) processors with native ARM64 binaries.

## Quick Install (Recommended)

```bash
curl -fsSL https://raw.githubusercontent.com/winnyboy5/mediagit-core/main/install.sh | sh
```

The script detects Apple Silicon and fetches the native ARM64 build.

## Homebrew

> **Not published.** There is no `mediagit/tap` and no Homebrew formula in any
> published feed — `packaging/homebrew/mediagit.rb` in the repo is a build
> recipe, not a hosted tap. `brew install mediagit-core` will fail.

## Alternative Installation Methods

### Direct Binary Download

```bash
# Download latest ARM64 binary
curl -fsSL https://github.com/winnyboy5/mediagit-core/releases/download/v0.3.0-rc.5/mediagit-0.3.0-rc.5-aarch64-macos.tar.gz \
  | sudo tar xz -C /usr/local/bin

# Verify native ARM64
file /usr/local/bin/mediagit
# Output should show: Mach-O 64-bit executable arm64
```

### Using Installation Script

```bash
curl -fsSL https://raw.githubusercontent.com/winnyboy5/mediagit-core/main/install.sh | sh
```

## macOS Gatekeeper Approval

First run requires security approval:

```bash
# Remove quarantine attribute
xattr -d com.apple.quarantine /usr/local/bin/mediagit

# Or approve via System Settings
# System Settings → Privacy & Security → Security → "Allow Anyway"
```

## Post-Installation Setup

### Shell Completions

#### Zsh (default on macOS)

```bash
mediagit completions zsh > /opt/homebrew/share/zsh/site-functions/_mediagit

# For manual installation
mkdir -p ~/.zfunc
mediagit completions zsh > ~/.zfunc/_mediagit
echo 'fpath=(~/.zfunc $fpath)' >> ~/.zshrc
```

#### Bash (if using Homebrew bash)

```bash
brew install bash-completion@2
mediagit completions bash > $(brew --prefix)/etc/bash_completion.d/mediagit
```

### Environment Variables

Add to `~/.zshrc`:

```bash

# Optional: Optimize for Apple Silicon

# Optional: Enable debug logging
export MEDIAGIT_LOG=info
```

## Apple Silicon Optimizations

MediaGit-Core leverages Apple Silicon features:

- **Native ARM64**: Full performance, no Rosetta 2 emulation
- **Metal Acceleration**: GPU-accelerated image processing (future)
- **AMX Instructions**: Matrix operations for ML workloads
- **Efficiency Cores**: Balanced power/performance

### Performance Configuration

```toml
# .mediagit/config.toml
[performance]
max_concurrency = 8          # M1: 8, M2/M3: 8-12, M4: 10-16
upload_concurrency = 32
download_concurrency = 24
buffer_size = 1048576        # bytes — 1 MiB

[performance.cache]
enabled = true
max_size = 2147483648        # bytes — 2 GiB, leveraging unified memory

[compression]
algorithm = "zstd"
level = 3
```

Cache and buffer sizes are raw **bytes**, and `level` is an **integer** (zstd
1-22, brotli 0-11). Unrecognised keys are silently discarded, so `"2GB"` or
`level = "fast"` would be dropped without any error.

## System Requirements

- **macOS Version**: 11.0 Big Sur or later (12.0+ recommended)
- **CPU**: Apple M1 or later (M1, M1 Pro, M1 Max, M1 Ultra, M2, M3, M4)
- **RAM**: 8GB minimum, 16GB+ recommended
- **Disk**: 100MB for binaries, SSD recommended
- **Xcode**: Command Line Tools (optional)

### Build Environment

The `aarch64-apple-darwin` release binary is built (and its test suite run)
on GitHub's `macos-14` runner (`.github/workflows/release.yml`) — one
specific Apple Silicon chip, not a matrix. It should run on any M-series Mac
since they share the same architecture, but per-chip testing across
M1/M2/M3/M4 variants has not been verified; treat that as untested rather
than confirmed.

## Verification

```bash
# Check version and architecture
mediagit --version
file $(which mediagit)

# Verify a repository's integrity (run inside a repo)
mediagit fsck --full

# Create test repo
mkdir ~/test-mediagit
cd ~/test-mediagit
mediagit init
```

Expected output:
```
mediagit-core 0.3.0-rc.5
/opt/homebrew/bin/mediagit: Mach-O 64-bit executable arm64
✓ All checks passed
✓ Initialized empty MediaGit repository in .mediagit/
```

## Troubleshooting

### Running Under Rosetta 2 (Not Recommended)

If you accidentally installed the Intel version:

```bash
# Check if running under Rosetta
sysctl sysctl.proc_translated

# If output is 1, you have the Intel binary. Remove it and re-run the
# install script, which selects the native ARM64 build.
rm -f /usr/local/bin/mediagit /usr/local/bin/mediagit-server
curl -fsSL https://raw.githubusercontent.com/winnyboy5/mediagit-core/main/install.sh | sh
```

### "mediagit" cannot be opened

```bash
# Remove quarantine attribute
xattr -d com.apple.quarantine /usr/local/bin/mediagit

# If that doesn't work, allow in System Settings
open "x-apple.systempreferences:com.apple.preference.security"
```

### Command Not Found

```bash
# Check Homebrew PATH for Apple Silicon
echo $PATH | grep /opt/homebrew

# If missing, add to ~/.zshrc
echo 'export PATH="/opt/homebrew/bin:$PATH"' >> ~/.zshrc
source ~/.zshrc
```

### Permission Issues

```bash
# The install script writes to /usr/local/bin; make sure it is writable.
sudo chown -R $(whoami) /usr/local/bin

# Retry
curl -fsSL https://raw.githubusercontent.com/winnyboy5/mediagit-core/main/install.sh | sh
```

## Performance Benchmarks

Apple Silicon performance vs Intel:

| Operation | M1 | M1 Max | M2 | M3 | Intel i9 |
|-----------|------|---------|------|------|----------|
| Compression (1GB) | 2.3s | 1.8s | 2.1s | 1.6s | 4.2s |
| Branch Switch | 45ms | 38ms | 42ms | 35ms | 120ms |
| Object Scan (10k) | 0.8s | 0.6s | 0.7s | 0.5s | 1.9s |

## Updating

### Via Homebrew

```bash
brew update
brew upgrade mediagit-core
```

### Manual Update

```bash
curl -fsSL https://github.com/winnyboy5/mediagit-core/releases/download/v0.3.0-rc.5/mediagit-0.3.0-rc.5-aarch64-macos.tar.gz \
  | sudo tar xz -C /usr/local/bin
```

## Uninstalling

### Uninstall

```bash
sudo rm /usr/local/bin/mediagit
sudo rm /opt/homebrew/bin/mediagit
rm -rf ~/.mediagit
```

## Next Steps

- [Quickstart Guide](../quickstart.md) - Get started in 5 minutes
- [Performance Optimization](../guides/performance.md) - Tune for Apple Silicon
- [Configuration](../configuration.md) - Customize MediaGit
- [CLI Reference](../cli/README.md) - Learn all commands
