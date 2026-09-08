# Linux ARM64 Installation

MediaGit-Core supports ARM64 Linux systems including Raspberry Pi, ARM servers, and cloud ARM instances.

## Quick Install

```bash
curl -fsSL https://raw.githubusercontent.com/winnyboy5/mediagit-core/main/install.sh | sh
```

The script auto-detects ARM64 architecture and downloads the correct binary from
[GitHub Releases](https://github.com/winnyboy5/mediagit-core/releases).

## Raspberry Pi Setup

### Raspberry Pi OS (64-bit)

```bash
# Update system
sudo apt update && sudo apt upgrade

# Install MediaGit
wget https://github.com/winnyboy5/mediagit-core/releases/download/v0.3.0-rc.5/mediagit-0.3.0-rc.5-aarch64-linux.tar.gz
tar -xzf mediagit-0.3.0-rc.5-aarch64-linux.tar.gz
sudo mv mediagit /usr/local/bin/
sudo chmod +x /usr/local/bin/mediagit

# Verify
mediagit --version
```

### Raspberry Pi 4/5 Optimization

```toml
# .mediagit/config.toml
[performance]
max_concurrency = 4          # Raspberry Pi 4/5 has 4 cores
upload_concurrency = 4
download_concurrency = 4

[compression]
algorithm = "zstd"
level = 1                    # lowest CPU cost; level is an integer, not a name
```

## ARM Server Installation

### Ubuntu Server ARM64

```bash
# Add MediaGit repository
curl -fsSL https://apt.mediagit.dev/gpg.key | sudo gpg --dearmor -o /usr/share/keyrings/mediagit-archive-keyring.gpg
echo "deb [arch=arm64 signed-by=/usr/share/keyrings/mediagit-archive-keyring.gpg] https://apt.mediagit.dev stable main" | sudo tee /etc/apt/sources.list.d/mediagit.list

# Install
sudo apt update
sudo apt install mediagit-core
```

### Amazon Linux 2 (Graviton)

```bash
# Download ARM64 build
wget https://github.com/winnyboy5/mediagit-core/releases/download/v0.3.0-rc.5/mediagit-0.3.0-rc.5-aarch64-linux.tar.gz

# Install
tar -xzf mediagit-0.3.0-rc.5-aarch64-linux.tar.gz
sudo mv mediagit /usr/local/bin/
sudo chmod +x /usr/local/bin/mediagit
```

## Cloud ARM Instances

### AWS Graviton (EC2 t4g, c7g)

Optimized for AWS Graviton processors:

```bash
# Install
curl -fsSL https://raw.githubusercontent.com/winnyboy5/mediagit-core/main/install.sh | sh

# Configure for Graviton. `config set` accepts only author.name, author.email,
# performance.upload_concurrency and performance.download_concurrency —
# everything else is edited in .mediagit/config.toml directly.
mediagit config set performance.upload_concurrency $(nproc)
```

### Oracle Cloud Ampere

```bash
# Install on Oracle Cloud ARM instances
sudo dnf config-manager --add-repo https://rpm.mediagit.dev/mediagit.repo
sudo dnf install mediagit-core
```

### Azure ARM VMs

```bash
# Ubuntu 22.04 ARM64
curl -fsSL https://raw.githubusercontent.com/winnyboy5/mediagit-core/main/install.sh | sh
```

## Manual Binary Installation

```bash
# Download ARM64 binary
wget https://github.com/winnyboy5/mediagit-core/releases/download/v0.3.0-rc.5/mediagit-0.3.0-rc.5-aarch64-linux.tar.gz

# Extract and install
tar -xzf mediagit-0.3.0-rc.5-aarch64-linux.tar.gz
sudo mv mediagit /usr/local/bin/
sudo chmod +x /usr/local/bin/mediagit

# Verify
mediagit --version
```

## Performance Tuning for ARM

### Memory-Constrained Devices (1-2GB RAM)

```toml
# .mediagit/config.toml
[performance]
max_concurrency = 2          # keep peak memory down on a 1-2GB board
upload_concurrency = 4
download_concurrency = 4
buffer_size = 65536          # bytes

[performance.cache]
enabled = true
max_size = 268435456         # bytes — 256 MiB

[compression]
algorithm = "zstd"
level = 1                    # 1 is the cheapest zstd level; 22 is the slowest
```

### High-Performance ARM Servers (Graviton 3, Ampere Altra)

```toml
# .mediagit/config.toml
[performance]
max_concurrency = 64
upload_concurrency = 32
download_concurrency = 24
pack_workers = 8
buffer_size = 1048576        # bytes — 1 MiB

[performance.cache]
enabled = true
max_size = 4294967296        # bytes — 4 GiB

[compression]
algorithm = "zstd"
level = 3
```

Sizes are raw **bytes**, and `level` is an **integer** (zstd 1-22, brotli 0-11).
Unrecognised keys in `config.toml` are silently discarded, so a value written in
the wrong shape — `"256MB"`, or `level = "fast"` — is indistinguishable from
never having written it at all.

## System Requirements

- **CPU**: ARMv8-A or later (AArch64)
- **RAM**: 512MB minimum, 2GB recommended
- **Disk**: 100MB for binaries
- **OS**: Linux kernel 4.4+

### Verified ARM Platforms

| Platform | Version | Status |
|----------|---------|--------|
| Raspberry Pi 4 | 8GB | ✅ Tested |
| Raspberry Pi 5 | 4GB, 8GB | ✅ Tested |
| AWS Graviton 2/3 | All instance types | ✅ Tested |
| Oracle Ampere A1 | All shapes | ✅ Tested |
| Azure ARM64 VMs | Dpsv5, Epsv5 series | ✅ Tested |
| Ampere Altra | All SKUs | ✅ Tested |

## Troubleshooting

### Illegal Instruction Error

If you see "Illegal instruction":

```bash
# Check CPU features
cat /proc/cpuinfo | grep Features

# Ensure ARMv8-A or later
uname -m  # Should output: aarch64
```

### Out of Memory on Raspberry Pi

```toml
# Reduce memory usage — .mediagit/config.toml
[performance]
max_concurrency = 1
upload_concurrency = 2
download_concurrency = 2
buffer_size = 32768          # bytes — 32 KiB

[performance.cache]
enabled = true
max_size = 134217728         # bytes — 128 MiB

[compression]
algorithm = "zstd"
level = 1
```

### Slow Performance

```bash
# Check CPU frequency (may be throttled)
cat /sys/devices/system/cpu/cpu0/cpufreq/scaling_cur_freq

# Enable performance governor
sudo apt install cpufrequtils
sudo cpufreq-set -g performance
```

## Building from Source (ARM64)

If pre-built binaries don't work:

```bash
# Install Rust
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

# Clone and build
git clone https://github.com/winnyboy5/mediagit-core.git
cd mediagit-core
cargo build --release --target aarch64-unknown-linux-gnu

# Install
sudo mv target/aarch64-unknown-linux-gnu/release/mediagit /usr/local/bin/
```

## Cross-Compilation (Advanced)

Compile ARM64 binaries on x64 machines:

```bash
# Install cross-compilation tools
rustup target add aarch64-unknown-linux-gnu
sudo apt install gcc-aarch64-linux-gnu

# Build
cargo build --release --target aarch64-unknown-linux-gnu
```

## Next Steps

- [Quickstart Guide](../quickstart.md) - Get started in 5 minutes
- [Performance Optimization](../guides/performance.md) - Tune for your hardware
- [Configuration](../configuration.md) - Customize settings
