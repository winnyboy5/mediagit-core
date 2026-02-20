# Building from Source

Build MediaGit from source when:

- A pre-built binary is not available for your platform (e.g., Windows ARM64)
- You want to build with custom feature flags
- You are contributing to MediaGit development
- You need to cross-compile for a different target

## Prerequisites

### All platforms

- **Rust 1.91.0 or later** — install via [rustup.rs](https://rustup.rs)
  ```bash
  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
  rustup update
  ```
- **Git** — to clone the repository

### Linux

A C linker is needed (usually already installed):

```bash
# Debian/Ubuntu
sudo apt install build-essential

# Fedora/RHEL
sudo dnf install gcc
```

### macOS

Install Xcode Command Line Tools:

```bash
xcode-select --install
```

### Windows

Install Visual Studio Build Tools with the "Desktop development with C++" workload:

```powershell
winget install Microsoft.VisualStudio.2022.BuildTools
```

Or install the full Visual Studio 2022.

---

## Build

```bash
# Clone the repository
git clone https://github.com/mediagit/mediagit-core.git
cd mediagit-core

# Build release binaries
cargo build --release --bin mediagit --bin mediagit-server
```

Binaries are written to `target/release/`:

- `target/release/mediagit` (Linux/macOS)
- `target/release/mediagit.exe` (Windows)
- `target/release/mediagit-server` / `mediagit-server.exe`

Build time: 3–8 minutes on first build; incremental rebuilds are much faster.

---

## Install

### Linux / macOS

```bash
# Install to ~/.cargo/bin (already on PATH if you used rustup)
cargo install --path crates/mediagit-cli --locked
cargo install --path crates/mediagit-server --locked

# Or copy manually
sudo cp target/release/mediagit /usr/local/bin/
sudo cp target/release/mediagit-server /usr/local/bin/
```

### Windows

```powershell
# Copy to a directory on your PATH
$dest = "$env:USERPROFILE\bin"
New-Item -ItemType Directory -Force -Path $dest | Out-Null
Copy-Item "target\release\mediagit.exe" -Destination $dest
Copy-Item "target\release\mediagit-server.exe" -Destination $dest

# Add to PATH if not already present
$userPath = [Environment]::GetEnvironmentVariable("Path", "User")
if ($userPath -notlike "*$dest*") {
    [Environment]::SetEnvironmentVariable("Path", "$userPath;$dest", "User")
    Write-Host "Added $dest to PATH. Restart your terminal."
}
```

---

## Verify

```bash
mediagit --version
mediagit-server --version
```

---

## Cross-Compilation

### Linux ARM64 (from Linux x64)

Install `cross`:

```bash
cargo install cross --git https://github.com/cross-rs/cross
```

Build:

```bash
cross build --release --target aarch64-unknown-linux-gnu \
  --bin mediagit --bin mediagit-server
```

### macOS Apple Silicon (from macOS Intel)

```bash
rustup target add aarch64-apple-darwin
cargo build --release --target aarch64-apple-darwin \
  --bin mediagit --bin mediagit-server
```

### macOS Intel (from macOS Apple Silicon)

```bash
rustup target add x86_64-apple-darwin
cargo build --release --target x86_64-apple-darwin \
  --bin mediagit --bin mediagit-server
```

### Windows x64 (from Linux)

> **Note**: `cross-rs` does not support Windows targets. To build for Windows, use a Windows machine or a Windows GitHub Actions runner.

On Windows:

```bash
rustup target add x86_64-pc-windows-msvc
cargo build --release --target x86_64-pc-windows-msvc \
  --bin mediagit --bin mediagit-server
```

### Windows ARM64

See [Windows ARM64 Installation](./windows-arm64.md).

---

## Checking MSRV

MediaGit's Minimum Supported Rust Version (MSRV) is **1.91.0**. Verify compatibility:

```bash
cargo +1.91.0 check --workspace --all-features
```

---

## Running Tests

```bash
# Unit and integration tests (no external services)
cargo test --workspace

# Skip slow tests
cargo test --workspace -- --skip integration

# With all features
cargo test --workspace --all-features
```

Integration tests that require Docker (MinIO, Azurite, fake-gcs-server):

```bash
docker compose -f docker-compose.test.yml up -d

AWS_ACCESS_KEY_ID=minioadmin \
AWS_SECRET_ACCESS_KEY=minioadmin \
AWS_ENDPOINT_URL=http://localhost:9000 \
AWS_REGION=us-east-1 \
AZURE_STORAGE_CONNECTION_STRING="DefaultEndpointsProtocol=http;AccountName=devstoreaccount1;AccountKey=Eby8vdM02xNOcqFlqUwJPLlmEtlCDXJ1OUzFT50uSRZ6IFsuFq2UVErCz4I6tq/K1SZFPTOtr/KBHBeksoGMGw==;BlobEndpoint=http://localhost:10000/devstoreaccount1;" \
GCS_EMULATOR_HOST=http://localhost:4443 \
cargo test --ignored -p mediagit-storage -p mediagit-server

docker compose -f docker-compose.test.yml down -v
```

---

## WSL2 Notes

Building on a Windows NTFS filesystem (e.g., `/mnt/d/...`) from WSL2 can cause Cargo's incremental build fingerprinting to miss source changes. For reliable builds during development, use a WSL2-native path:

```bash
# Clone to WSL2 filesystem
git clone ... ~/projects/mediagit-core
cd ~/projects/mediagit-core
cargo build --release
```

The resulting Linux ELF binary can then be tested in WSL2. To build the Windows PE binary, run `cargo build --release` from a Windows terminal (cmd, PowerShell, or Windows Terminal).

---

## See Also

- [Windows ARM64 Installation](./windows-arm64.md)
- [Contributing — Development Setup](../contributing/development.md)
- [Contributing — Release Process](../contributing/releases.md)
