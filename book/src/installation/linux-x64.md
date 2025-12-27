# Linux x64 Installation

MediaGit-Core provides pre-built binaries and package manager support for Linux x64 systems.

## Quick Install (Recommended)

```bash
curl -fsSL https://get.mediagit.dev/install.sh | bash
```

This script automatically:
- Detects your Linux distribution
- Downloads the latest x64 binary
- Installs to `/usr/local/bin/mediagit`
- Sets up shell completions
- Configures PATH if needed

## Distribution-Specific Installation

### Ubuntu / Debian

#### Using APT Repository

```bash
# Add MediaGit repository
curl -fsSL https://apt.mediagit.dev/gpg.key | sudo gpg --dearmor -o /usr/share/keyrings/mediagit-archive-keyring.gpg
echo "deb [signed-by=/usr/share/keyrings/mediagit-archive-keyring.gpg] https://apt.mediagit.dev stable main" | sudo tee /etc/apt/sources.list.d/mediagit.list

# Update and install
sudo apt update
sudo apt install mediagit-core
```

#### Using .deb Package

```bash
# Download latest release
wget https://github.com/mediagit/mediagit-core/releases/download/v0.1.0/mediagit_0.1.0_amd64.deb

# Install
sudo dpkg -i mediagit_0.1.0_amd64.deb

# Fix dependencies if needed
sudo apt-get install -f
```

### Fedora / RHEL / CentOS

#### Using DNF/YUM

```bash
# Add MediaGit repository
sudo dnf config-manager --add-repo https://rpm.mediagit.dev/mediagit.repo

# Install
sudo dnf install mediagit-core

# For older systems using yum
sudo yum install mediagit-core
```

#### Using .rpm Package

```bash
# Download latest release
wget https://github.com/mediagit/mediagit-core/releases/download/v0.1.0/mediagit-0.1.0-1.x86_64.rpm

# Install
sudo rpm -i mediagit-0.1.0-1.x86_64.rpm
```

### Arch Linux

```bash
# Install from AUR
yay -S mediagit-core

# Or using paru
paru -S mediagit-core

# Manual AUR installation
git clone https://aur.archlinux.org/mediagit-core.git
cd mediagit-core
makepkg -si
```

### openSUSE

```bash
# Add repository
sudo zypper addrepo https://download.opensuse.org/repositories/home:mediagit/openSUSE_Tumbleweed/home:mediagit.repo

# Install
sudo zypper refresh
sudo zypper install mediagit-core
```

## Manual Binary Installation

If package managers aren't available, install manually:

```bash
# Download binary
wget https://github.com/mediagit/mediagit-core/releases/download/v0.1.0/mediagit-linux-x64.tar.gz

# Extract
tar -xzf mediagit-linux-x64.tar.gz

# Move to bin directory
sudo mv mediagit /usr/local/bin/

# Make executable
sudo chmod +x /usr/local/bin/mediagit

# Verify installation
mediagit --version
```

## Post-Installation Setup

### Shell Completions

#### Bash

```bash
mediagit completions bash > ~/.local/share/bash-completion/completions/mediagit
```

#### Zsh

```bash
mediagit completions zsh > ~/.zfunc/_mediagit
echo 'fpath=(~/.zfunc $fpath)' >> ~/.zshrc
```

#### Fish

```bash
mediagit completions fish > ~/.config/fish/completions/mediagit.fish
```

### Environment Variables

Add to `~/.bashrc` or `~/.zshrc`:

```bash
# Optional: Set default backend
export MEDIAGIT_DEFAULT_BACKEND=local

# Optional: Set storage path
export MEDIAGIT_STORAGE_PATH=~/.mediagit/storage

# Optional: Enable debug logging
export MEDIAGIT_LOG=debug
```

## Verify Installation

```bash
# Check version
mediagit --version

# Run self-test
mediagit fsck --self-test

# Create test repository
mkdir test-repo
cd test-repo
mediagit init
```

Expected output:
```
mediagit-core 0.1.0
✓ Initialized empty MediaGit repository in .mediagit/
```

## System Requirements

- **CPU**: x86_64 processor (Intel, AMD)
- **RAM**: 512MB minimum, 2GB recommended
- **Disk**: 100MB for binaries
- **OS**: Linux kernel 4.4+ (glibc 2.17+)
- **Dependencies**: None (statically linked)

### Verified Distributions

| Distribution | Version | Status |
|-------------|---------|--------|
| Ubuntu | 20.04, 22.04, 24.04 | ✅ Tested |
| Debian | 10, 11, 12 | ✅ Tested |
| Fedora | 38, 39, 40 | ✅ Tested |
| RHEL | 8, 9 | ✅ Tested |
| CentOS | 7, 8, Stream 9 | ✅ Tested |
| Arch Linux | Rolling | ✅ Tested |
| openSUSE | Leap 15.5, Tumbleweed | ✅ Tested |

## Troubleshooting

### Permission Denied

```bash
# If standard install fails, try user-local install
curl -fsSL https://get.mediagit.dev/install.sh | bash -s -- --no-sudo

# Add to PATH
export PATH="$HOME/.local/bin:$PATH"
```

### GLIBC Version Too Old

If you see: `version 'GLIBC_2.17' not found`:

```bash
# Check your glibc version
ldd --version

# Solution 1: Upgrade your system
sudo apt update && sudo apt upgrade

# Solution 2: Build from source
# See: Building from Source guide
```

### Command Not Found

```bash
# Check if binary exists
which mediagit

# If not found, add to PATH
export PATH="/usr/local/bin:$PATH"
echo 'export PATH="/usr/local/bin:$PATH"' >> ~/.bashrc
```

### SSL Certificate Errors

```bash
# Update CA certificates
sudo apt update
sudo apt install ca-certificates

# Or for Fedora/RHEL
sudo dnf install ca-certificates
```

## Updating

### APT/DNF Repository

```bash
# Ubuntu/Debian
sudo apt update && sudo apt upgrade mediagit-core

# Fedora/RHEL
sudo dnf update mediagit-core
```

### Manual Update

```bash
# Download latest version
curl -fsSL https://get.mediagit.dev/install.sh | bash

# Or manually
wget https://github.com/mediagit/mediagit-core/releases/latest/download/mediagit-linux-x64.tar.gz
tar -xzf mediagit-linux-x64.tar.gz
sudo mv mediagit /usr/local/bin/
```

## Uninstalling

### APT

```bash
sudo apt remove mediagit-core
```

### DNF/YUM

```bash
sudo dnf remove mediagit-core
```

### Manual

```bash
sudo rm /usr/local/bin/mediagit
rm -rf ~/.mediagit
```

## Next Steps

- [Quickstart Guide](../quickstart.md) - Get started in 5 minutes
- [Configuration](../configuration.md) - Customize MediaGit
- [CLI Reference](../cli/README.md) - Learn all commands
