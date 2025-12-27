# macOS Intel Installation

MediaGit-Core provides native binaries optimized for Intel-based Macs.

## Quick Install (Homebrew - Recommended)

```bash
brew tap mediagit/tap
brew install mediagit-core
```

## Alternative Installation Methods

### Direct Binary Download

```bash
# Download latest Intel binary
curl -LO https://github.com/mediagit/mediagit-core/releases/download/v0.1.0/mediagit-macos-intel.tar.gz

# Extract
tar -xzf mediagit-macos-intel.tar.gz

# Move to PATH
sudo mv mediagit /usr/local/bin/
sudo chmod +x /usr/local/bin/mediagit

# Verify
mediagit --version
```

### Using Installation Script

```bash
curl -fsSL https://get.mediagit.dev/install.sh | bash
```

## macOS Gatekeeper Approval

First run may require security approval:

```bash
# If you see "cannot be opened because the developer cannot be verified"
xattr -d com.apple.quarantine /usr/local/bin/mediagit

# Or approve via System Preferences
# System Preferences → Security & Privacy → General → "Allow Anyway"
```

## Post-Installation Setup

### Shell Completions

#### Zsh (default on macOS 10.15+)

```bash
mediagit completions zsh > /usr/local/share/zsh/site-functions/_mediagit
```

#### Bash

```bash
brew install bash-completion
mediagit completions bash > /usr/local/etc/bash_completion.d/mediagit
```

### Environment Variables

Add to `~/.zshrc`:

```bash
# Optional: Set default backend
export MEDIAGIT_DEFAULT_BACKEND=local

# Optional: Enable debug logging
export MEDIAGIT_LOG=info
```

## System Requirements

- **macOS Version**: 10.15 Catalina or later
- **CPU**: Intel Core 2 Duo or later
- **RAM**: 512MB minimum, 2GB recommended
- **Disk**: 100MB for binaries
- **Xcode**: Command Line Tools (optional, for some features)

### Install Xcode Command Line Tools

```bash
xcode-select --install
```

## Verification

```bash
# Check version
mediagit --version

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
✓ All checks passed
✓ Initialized empty MediaGit repository in .mediagit/
```

## Troubleshooting

### "mediagit" cannot be opened

```bash
# Remove quarantine attribute
xattr -d com.apple.quarantine /usr/local/bin/mediagit

# If that doesn't work, explicitly allow in System Preferences
open "x-apple.systempreferences:com.apple.preference.security"
```

### Command Not Found

```bash
# Check if binary exists
which mediagit

# If not found, check PATH
echo $PATH

# Add /usr/local/bin to PATH
export PATH="/usr/local/bin:$PATH"
echo 'export PATH="/usr/local/bin:$PATH"' >> ~/.zshrc
```

### Homebrew Not Found

```bash
# Install Homebrew first
/bin/bash -c "$(curl -fsSL https://raw.githubusercontent.com/Homebrew/install/HEAD/install.sh)"
```

### Permission Issues

```bash
# If brew install fails with permissions
sudo chown -R $(whoami) /usr/local/bin /usr/local/lib /usr/local/share

# Retry installation
brew install mediagit-core
```

## Updating

### Via Homebrew

```bash
brew update
brew upgrade mediagit-core
```

### Manual Update

```bash
curl -LO https://github.com/mediagit/mediagit-core/releases/latest/download/mediagit-macos-intel.tar.gz
tar -xzf mediagit-macos-intel.tar.gz
sudo mv mediagit /usr/local/bin/
```

## Uninstalling

### Via Homebrew

```bash
brew uninstall mediagit-core
brew untap mediagit/tap
```

### Manual Uninstall

```bash
sudo rm /usr/local/bin/mediagit
rm -rf ~/.mediagit
```

## Next Steps

- [Quickstart Guide](../quickstart.md) - Get started in 5 minutes
- [Configuration](../configuration.md) - Customize MediaGit
- [CLI Reference](../cli/README.md) - Learn all commands
