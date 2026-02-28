# Windows ARM64 Installation

> **Status**: Windows ARM64 pre-built binaries are not currently included in official releases.
> The `cross-rs` tool used for cross-compilation does not support Windows targets.
> Windows ARM64 users must build from source.

## Build from Source

Building from source on a Windows ARM64 machine (Surface Pro X, Snapdragon-based Copilot+ PCs):

### Prerequisites

1. **Install Rust** — download from [rustup.rs](https://rustup.rs)
   ```powershell
   winget install Rustlang.Rustup
   ```

2. **Install Visual Studio Build Tools** (required for MSVC linker):
   ```powershell
   winget install Microsoft.VisualStudio.2022.BuildTools
   # Select "Desktop development with C++"
   ```

3. **Add the ARM64 Rust target**:
   ```powershell
   rustup target add aarch64-pc-windows-msvc
   ```

### Build

```powershell
git clone https://github.com/mediagit/mediagit-core.git
cd mediagit-core

# Build for native ARM64
cargo build --release --target aarch64-pc-windows-msvc --bin mediagit --bin mediagit-server
```

### Install

```powershell
# Copy to a directory on your PATH
$dest = "$env:USERPROFILE\bin"
New-Item -ItemType Directory -Force -Path $dest | Out-Null
Copy-Item "target\aarch64-pc-windows-msvc\release\mediagit.exe" -Destination $dest
Copy-Item "target\aarch64-pc-windows-msvc\release\mediagit-server.exe" -Destination $dest

# Add to PATH (if not already done)
$userPath = [Environment]::GetEnvironmentVariable("Path", "User")
if ($userPath -notlike "*$dest*") {
    [Environment]::SetEnvironmentVariable("Path", "$userPath;$dest", "User")
    Write-Host "Added $dest to PATH. Restart your terminal."
}
```

### Verify

```powershell
mediagit --version
```

## Alternative: x64 Binary on ARM64 Windows

Windows ARM64 supports running x64 binaries via emulation. You can download the standard
`mediagit-x86_64-windows.zip` from the [Releases page](https://github.com/mediagit/mediagit-core/releases)
and run it directly. Performance will be lower than native, but it is fully functional.

## Tracking Issue

Follow [GitHub Issues](https://github.com/mediagit/mediagit-core/issues) for native Windows
ARM64 release support. Once a native ARM64 Windows GitHub Actions runner becomes available
(or an alternative cross-compilation strategy is adopted), pre-built binaries will be added
to official releases.
