# Linux x64 Installation

MediaGit-Core provides pre-built binaries and package manager support for Linux x64 systems.

## Quick Install (Recommended)

```bash
curl -fsSL https://raw.githubusercontent.com/winnyboy5/mediagit-core/main/install.sh | sh
```

This script automatically detects your architecture and downloads the correct binary from
[GitHub Releases](https://github.com/winnyboy5/mediagit-core/releases).

### Direct Download (x86_64)

```bash
curl -fsSL https://github.com/winnyboy5/mediagit-core/releases/download/v0.4.0-rc.1/mediagit-0.4.0-rc.1-x86_64-linux.tar.gz \
  | sudo tar xz -C /usr/local/bin
mediagit --version
```

The archive contains both `mediagit` and `mediagit-server` binaries.

## Distribution-Specific Installation

### Ubuntu / Debian

#### Using APT Repository

> **Not published.** There is no APT repository at `apt.mediagit.dev`, and no
> step in `.github/workflows` that would populate one. Use the install script
> or the tarball below.

#### Using a .deb Package

> **Not published.** There is no `.deb` in the releases. The release workflow
> builds five artifacts and none of them is a distro package — see
> [Manual Installation](#manual-installation) below for the tarball, which is
> the supported route on Debian and Ubuntu.

### Fedora / RHEL / CentOS

#### Using DNF/YUM

> **Not published.** There is no YUM/DNF repository at `rpm.mediagit.dev`. Use
> the install script or the tarball below.

#### Using an .rpm Package

> **Not published.** There is no `.rpm` in the releases, and no repository at
> `rpm.mediagit.dev`. Use the tarball in
> [Manual Installation](#manual-installation) instead.

### Arch Linux

> **Not published.** There is no AUR package — `aur.archlinux.org/mediagit-core`
> does not exist, so `yay -S`, `paru -S` and a manual `makepkg` all fail.

### openSUSE

> **Not published.** There is no openSUSE build-service repository. Use the
> install script or the tarball below.

## Manual Binary Installation

If package managers aren't available, install manually:

```bash
# Download archive
wget https://github.com/winnyboy5/mediagit-core/releases/download/v0.4.0-rc.1/mediagit-0.4.0-rc.1-x86_64-linux.tar.gz

# Verify checksum
wget https://github.com/winnyboy5/mediagit-core/releases/download/v0.4.0-rc.1/mediagit-0.4.0-rc.1-x86_64-linux.tar.gz.sha256
sha256sum -c mediagit-0.4.0-rc.1-x86_64-linux.tar.gz.sha256

# Extract (contains mediagit + mediagit-server)
tar -xzf mediagit-0.4.0-rc.1-x86_64-linux.tar.gz

# Move to bin directory
sudo mv mediagit mediagit-server /usr/local/bin/

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


# Optional: Enable debug logging
export MEDIAGIT_LOG=debug
```

Backend and storage location are **per repository**, not global: they are
set in that repo's `.mediagit/config.toml` (written by `mediagit init`).
There is no environment variable for either.

## Verify Installation

```bash
# Check version
mediagit --version

# Verify a repository's integrity (run inside a repo)
mediagit fsck --full

# Create test repository
mkdir test-repo
cd test-repo
mediagit init
```

Expected output:
```
mediagit-core 0.3.0-rc.5
✓ Initialized empty MediaGit repository in .mediagit/
```

## System Requirements

- **CPU**: x86_64 processor (Intel, AMD)
- **RAM**: 512MB minimum, 2GB recommended
- **Disk**: 100MB for binaries
- **OS**: Linux kernel 4.4+ (glibc 2.17+)
- **Dependencies**: None (statically linked)

### Build and CI Environment

The release binary is a statically-linked (no runtime distro dependencies)
x86_64 build produced on `ubuntu-22.04` in CI (`.github/workflows/release.yml`),
and CI itself only runs `ubuntu-latest`/`windows-latest` — there is no
per-distribution test matrix. It should run on any glibc 2.17+ Linux, but
"tested on Debian/Fedora/RHEL/CentOS/Arch/openSUSE" specifically has not been
verified; treat those as untested rather than confirmed.

## Troubleshooting

### Permission Denied

```bash
# If standard install fails, try user-local install
curl -fsSL https://raw.githubusercontent.com/winnyboy5/mediagit-core/main/install.sh | sh -s -- --no-sudo

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

### Manual Update

```bash
# Re-run install script (downloads latest)
curl -fsSL https://raw.githubusercontent.com/winnyboy5/mediagit-core/main/install.sh | sh

# Or download specific version manually
wget https://github.com/winnyboy5/mediagit-core/releases/download/v0.4.0-rc.1/mediagit-0.4.0-rc.1-x86_64-linux.tar.gz
tar -xzf mediagit-0.4.0-rc.1-x86_64-linux.tar.gz
sudo mv mediagit mediagit-server /usr/local/bin/
```

## Uninstalling

There is no package-manager install path (see [Distribution-Specific Installation](#distribution-specific-installation) above), so uninstalling is manual:

```bash
sudo rm /usr/local/bin/mediagit /usr/local/bin/mediagit-server
rm -rf ~/.mediagit
```

## Next Steps

- [Quickstart Guide](../quickstart.md) - Get started in 5 minutes
- [Configuration](../configuration.md) - Customize MediaGit
- [CLI Reference](../cli/README.md) - Learn all commands
