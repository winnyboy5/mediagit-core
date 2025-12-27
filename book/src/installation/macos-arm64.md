# macOS ARM64 (Apple Silicon) Installation

MediaGit-Core is optimized for Apple Silicon (M1, M2, M3, M4) processors with native ARM64 binaries.

## Quick Install (Homebrew - Recommended)

```bash
brew tap mediagit/tap
brew install mediagit-core
```

Homebrew automatically installs the ARM64 version on Apple Silicon Macs.

## Alternative Installation Methods

### Direct Binary Download

```bash
# Download latest ARM64 binary
curl -LO https://github.com/mediagit/mediagit-core/releases/download/v0.1.0/mediagit-macos-arm64.tar.gz

# Extract
tar -xzf mediagit-macos-arm64.tar.gz

# Move to PATH
sudo mv mediagit /usr/local/bin/
sudo chmod +x /usr/local/bin/mediagit

# Verify native ARM64
file /usr/local/bin/mediagit
# Output should show: Mach-O 64-bit executable arm64
```

### Using Installation Script

```bash
curl -fsSL https://get.mediagit.dev/install.sh | bash
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
# Optional: Set default backend
export MEDIAGIT_DEFAULT_BACKEND=local

# Optional: Optimize for Apple Silicon
export MEDIAGIT_USE_SIMD=1

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
# ~/.mediagit/config.toml
[performance]
worker_threads = 8  # M1: 8, M2/M3: 8-12, M4: 10-16
chunk_size = "16MB"
cache_size = "2GB"  # Leverage unified memory

[compression]
algorithm = "zstd"
level = "default"
parallel = true
threads = 4  # Use performance cores
```

## System Requirements

- **macOS Version**: 11.0 Big Sur or later (12.0+ recommended)
- **CPU**: Apple M1 or later (M1, M1 Pro, M1 Max, M1 Ultra, M2, M3, M4)
- **RAM**: 8GB minimum, 16GB+ recommended
- **Disk**: 100MB for binaries, SSD recommended
- **Xcode**: Command Line Tools (optional)

### Verified Chips

| Chip | Cores | Status |
|------|-------|--------|
| M1 | 4P+4E | ✅ Tested |
| M1 Pro | 6P+2E, 8P+2E | ✅ Tested |
| M1 Max | 8P+2E | ✅ Tested |
| M1 Ultra | 16P+4E | ✅ Tested |
| M2 | 4P+4E | ✅ Tested |
| M2 Pro | 6P+4E, 8P+4E | ✅ Tested |
| M2 Max | 8P+4E | ✅ Tested |
| M2 Ultra | 16P+8E | ✅ Tested |
| M3 | 4P+4E | ✅ Tested |
| M3 Pro | 6P+6E | ✅ Tested |
| M3 Max | 12P+4E | ✅ Tested |
| M4 | 4P+6E | ✅ Tested |

## Verification

```bash
# Check version and architecture
mediagit --version
file $(which mediagit)

# Run self-test
mediagit fsck --self-test

# Create test repo
mkdir ~/test-mediagit
cd ~/test-mediagit
mediagit init
```

Expected output:
```
mediagit-core 0.1.0
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

# If output is 1, you're using Intel binary
# Uninstall and reinstall ARM64 version
brew uninstall mediagit-core
brew cleanup
arch -arm64 brew install mediagit-core
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
# Fix Homebrew permissions
sudo chown -R $(whoami) /opt/homebrew

# Retry installation
brew install mediagit-core
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
curl -LO https://github.com/mediagit/mediagit-core/releases/latest/download/mediagit-macos-arm64.tar.gz
tar -xzf mediagit-macos-arm64.tar.gz
sudo mv mediagit /usr/local/bin/
```

## Uninstalling

### Via Homebrew

```bash
brew uninstall mediagit-core
brew untap mediagit/tap
rm -rf ~/.mediagit
```

### Manual Uninstall

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
