# Windows x64 Installation

MediaGit-Core provides native Windows binaries for x64 systems (Windows 10/11).

## Quick Install (Chocolatey - Recommended)

```powershell
# Open PowerShell as Administrator
choco install mediagit-core
```

## Alternative Installation Methods

### Windows Package Manager (winget)

```powershell
winget install MediaGit.MediaGitCore
```

### Direct Installer Download

1. Download the latest installer: [mediagit-setup-x64.msi](https://github.com/mediagit/mediagit-core/releases/download/v0.1.0/mediagit-setup-x64.msi)
2. Double-click the `.msi` file
3. Follow the installation wizard
4. MediaGit will be added to your PATH automatically

### Manual Binary Installation

```powershell
# Download ZIP archive
Invoke-WebRequest -Uri "https://github.com/mediagit/mediagit-core/releases/download/v0.1.0/mediagit-windows-x64.zip" -OutFile "mediagit.zip"

# Extract
Expand-Archive -Path mediagit.zip -DestinationPath "C:\Program Files\MediaGit"

# Add to PATH
[Environment]::SetEnvironmentVariable("Path", "$env:Path;C:\Program Files\MediaGit", [EnvironmentVariableTarget]::Machine)

# Verify
mediagit --version
```

## Post-Installation Setup

### PowerShell Completions

```powershell
# Generate completions
mediagit completions powershell > $PROFILE\..\Completions\mediagit.ps1

# Enable completions in profile
Add-Content $PROFILE "`nImport-Module `"$PROFILE\..\Completions\mediagit.ps1`""
```

### Git Bash Completions (if using Git Bash on Windows)

```bash
mediagit completions bash > ~/.bash_completion.d/mediagit
```

### Environment Variables

Set via System Properties or PowerShell:

```powershell
# Optional: Set default backend
[Environment]::SetEnvironmentVariable("MEDIAGIT_DEFAULT_BACKEND", "local", "User")

# Optional: Set storage path
[Environment]::SetEnvironmentVariable("MEDIAGIT_STORAGE_PATH", "$env:USERPROFILE\.mediagit\storage", "User")

# Optional: Enable debug logging
[Environment]::SetEnvironmentVariable("MEDIAGIT_LOG", "info", "User")
```

## System Requirements

- **Windows Version**: Windows 10 (1809+) or Windows 11
- **CPU**: x64 processor (Intel, AMD)
- **RAM**: 1GB minimum, 4GB recommended
- **Disk**: 100MB for binaries
- **Dependencies**: Visual C++ Redistributable 2015-2022 (included in installer)

### Visual C++ Redistributable

The MSI installer includes required dependencies. For manual installation:

```powershell
# Download and install VC++ Redistributable
Invoke-WebRequest -Uri "https://aka.ms/vs/17/release/vc_redist.x64.exe" -OutFile "vc_redist.x64.exe"
.\vc_redist.x64.exe /install /quiet /norestart
```

## Verification

```powershell
# Check version
mediagit --version

# Run self-test
mediagit fsck --self-test

# Create test repository
mkdir C:\test-mediagit
cd C:\test-mediagit
mediagit init
```

Expected output:
```
mediagit-core 0.1.0
✓ All checks passed
✓ Initialized empty MediaGit repository in .mediagit/
```

## Windows-Specific Configuration

### Long Path Support (Required for Deep Repositories)

Enable long path support in Windows:

```powershell
# Run as Administrator
New-ItemProperty -Path "HKLM:\SYSTEM\CurrentControlSet\Control\FileSystem" `
  -Name "LongPathsEnabled" -Value 1 -PropertyType DWORD -Force
```

### Windows Defender Exclusions

For better performance, exclude MediaGit directories:

```powershell
# Run as Administrator
Add-MpPreference -ExclusionPath "C:\Program Files\MediaGit"
Add-MpPreference -ExclusionPath "$env:USERPROFILE\.mediagit"
```

### File System Configuration

```toml
# %USERPROFILE%\.mediagit\config.toml
[filesystem]
case_sensitive = false  # Windows is case-insensitive
symlinks_enabled = false  # Limited symlink support
line_endings = "crlf"  # Windows-style line endings
```

## Troubleshooting

### "mediagit is not recognized"

```powershell
# Check if in PATH
$env:Path -split ';' | Select-String mediagit

# If not found, add to PATH
$oldPath = [Environment]::GetEnvironmentVariable("Path", "User")
$newPath = "$oldPath;C:\Program Files\MediaGit"
[Environment]::SetEnvironmentVariable("Path", $newPath, "User")

# Restart PowerShell
```

### Permission Denied

```powershell
# Run PowerShell as Administrator
Start-Process powershell -Verb RunAs

# Or adjust execution policy
Set-ExecutionPolicy -ExecutionPolicy RemoteSigned -Scope CurrentUser
```

### Visual C++ Runtime Error

```powershell
# Install missing runtime
choco install vcredist140

# Or download directly
Invoke-WebRequest -Uri "https://aka.ms/vs/17/release/vc_redist.x64.exe" -OutFile "vc_redist.x64.exe"
.\vc_redist.x64.exe /install /quiet /norestart
```

### Slow Performance on Network Drives

```toml
# Disable real-time scanning for MediaGit operations
[performance]
disable_indexing = true
bypass_cache_manager = true
```

### Chocolatey Not Found

```powershell
# Install Chocolatey first
Set-ExecutionPolicy Bypass -Scope Process -Force
[System.Net.ServicePointManager]::SecurityProtocol = [System.Net.ServicePointManager]::SecurityProtocol -bor 3072
iex ((New-Object System.Net.WebClient).DownloadString('https://community.chocolatey.org/install.ps1'))
```

## Integration with Windows Tools

### Windows Terminal

Add MediaGit-specific profile:

```json
// settings.json
{
  "profiles": {
    "list": [
      {
        "name": "MediaGit",
        "commandline": "powershell.exe -NoExit -Command \"cd $env:USERPROFILE\\repos\"",
        "icon": "C:\\Program Files\\MediaGit\\icon.ico"
      }
    ]
  }
}
```

### VS Code Integration

Install the MediaGit extension:

```powershell
code --install-extension mediagit.mediagit-vscode
```

### File Explorer Context Menu

Right-click integration (requires registry edit):

```powershell
# Run as Administrator
$regPath = "HKCU:\Software\Classes\Directory\shell\MediaGit"
New-Item -Path $regPath -Force
New-ItemProperty -Path $regPath -Name "(Default)" -Value "Open MediaGit Here" -Force
New-ItemProperty -Path $regPath -Name "Icon" -Value "C:\Program Files\MediaGit\mediagit.exe" -Force

$commandPath = "$regPath\command"
New-Item -Path $commandPath -Force
New-ItemProperty -Path $commandPath -Name "(Default)" -Value "wt.exe -d `"%V`" powershell.exe -NoExit -Command mediagit status" -Force
```

## Updating

### Via Chocolatey

```powershell
choco upgrade mediagit-core
```

### Via winget

```powershell
winget upgrade MediaGit.MediaGitCore
```

### Manual Update

1. Download latest installer from [Releases](https://github.com/mediagit/mediagit-core/releases)
2. Run the new installer (it will replace the old version)

## Uninstalling

### Via Chocolatey

```powershell
choco uninstall mediagit-core
```

### Via Windows Settings

1. Open Settings → Apps → Installed apps
2. Find "MediaGit-Core"
3. Click "Uninstall"

### Manual Uninstall

```powershell
# Remove program files
Remove-Item "C:\Program Files\MediaGit" -Recurse -Force

# Remove user data
Remove-Item "$env:USERPROFILE\.mediagit" -Recurse -Force

# Remove from PATH
$oldPath = [Environment]::GetEnvironmentVariable("Path", "User")
$newPath = ($oldPath -split ';' | Where-Object { $_ -notlike '*MediaGit*' }) -join ';'
[Environment]::SetEnvironmentVariable("Path", $newPath, "User")
```

## WSL2 Integration

Use Windows MediaGit from WSL2:

```bash
# Add Windows PATH to WSL
echo 'export PATH="$PATH:/mnt/c/Program Files/MediaGit"' >> ~/.bashrc
source ~/.bashrc

# Create alias
echo 'alias mediagit="/mnt/c/Program\ Files/MediaGit/mediagit.exe"' >> ~/.bashrc
```

## Next Steps

- [Quickstart Guide](../quickstart.md) - Get started in 5 minutes
- [Configuration](../configuration.md) - Customize MediaGit
- [CLI Reference](../cli/README.md) - Learn all commands
- [Windows-Specific Tips](../guides/troubleshooting.md#windows) - Platform-specific guidance
