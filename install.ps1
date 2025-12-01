# MediaGit Installation Script for Windows PowerShell
# Usage: iwr https://raw.githubusercontent.com/yourusername/mediagit-core/main/install.ps1 | iex

param(
    [string]$Version = "latest",
    [string]$InstallDir = "$env:LOCALAPPDATA\MediaGit\bin"
)

$ErrorActionPreference = "Stop"
$ProgressPreference = "SilentlyContinue"

$Repo = "yourusername/mediagit-core"

Write-Host "MediaGit Installation Script" -ForegroundColor Green
Write-Host "==============================" -ForegroundColor Green
Write-Host ""

# Detect architecture
$Arch = if ([Environment]::GetEnvironmentVariable("PROCESSOR_ARCHITECTURE") -eq "ARM64") {
    "aarch64"
} elseif ([Environment]::Is64BitOperatingSystem) {
    "x86_64"
} else {
    throw "32-bit Windows is not supported"
}

Write-Host "Platform: Windows ${Arch}" -ForegroundColor Cyan

# Fetch latest version if not specified
if ($Version -eq "latest") {
    Write-Host "Fetching latest version..." -ForegroundColor Cyan
    try {
        $LatestRelease = Invoke-RestMethod -Uri "https://api.github.com/repos/${Repo}/releases/latest"
        $Version = $LatestRelease.tag_name -replace '^v', ''
    }
    catch {
        Write-Host "Error: Could not fetch latest version" -ForegroundColor Red
        exit 1
    }
}

$Archive = "mediagit-${Version}-${Arch}-windows.zip"
$Url = "https://github.com/${Repo}/releases/download/v${Version}/${Archive}"

Write-Host "Version: ${Version}" -ForegroundColor Cyan
Write-Host "Download URL: ${Url}" -ForegroundColor Cyan
Write-Host ""

# Create install directory
Write-Host "Creating installation directory..." -ForegroundColor Cyan
New-Item -ItemType Directory -Force -Path $InstallDir | Out-Null

# Download archive
$TempZip = Join-Path $env:TEMP $Archive
Write-Host "Downloading MediaGit ${Version}..." -ForegroundColor Cyan
try {
    Invoke-WebRequest -Uri $Url -OutFile $TempZip -UseBasicParsing
}
catch {
    Write-Host "Error: Failed to download from ${Url}" -ForegroundColor Red
    Write-Host $_.Exception.Message -ForegroundColor Red
    exit 1
}

# Extract
Write-Host "Extracting archive..." -ForegroundColor Cyan
try {
    Expand-Archive -Path $TempZip -DestinationPath $InstallDir -Force
}
catch {
    Write-Host "Error: Failed to extract archive" -ForegroundColor Red
    Write-Host $_.Exception.Message -ForegroundColor Red
    exit 1
}

# Clean up
Remove-Item $TempZip -Force

# Add to PATH if not already there
$UserPath = [Environment]::GetEnvironmentVariable("Path", "User")
if ($UserPath -notlike "*$InstallDir*") {
    Write-Host "Adding to PATH..." -ForegroundColor Cyan
    $NewPath = if ($UserPath) { "$UserPath;$InstallDir" } else { $InstallDir }
    [Environment]::SetEnvironmentVariable("Path", $NewPath, "User")
    $env:Path = "$env:Path;$InstallDir"  # Update current session
    Write-Host "✓ Added ${InstallDir} to PATH" -ForegroundColor Yellow
    Write-Host "  You may need to restart your terminal for PATH changes to take effect" -ForegroundColor Yellow
}

# Verify installation
Write-Host ""
Write-Host "✓ MediaGit installed successfully!" -ForegroundColor Green
Write-Host ""

$MediaGitPath = Join-Path $InstallDir "mediagit.exe"
if (Test-Path $MediaGitPath) {
    Write-Host "Installed version:" -ForegroundColor Cyan
    & $MediaGitPath --version
    Write-Host ""
    Write-Host "Get started with:" -ForegroundColor Cyan
    Write-Host "  mediagit init          # Initialize a new repository"
    Write-Host "  mediagit --help        # Show all commands"
    Write-Host ""
    Write-Host "Documentation: https://docs.mediagit.dev" -ForegroundColor Cyan
}
else {
    Write-Host "Warning: mediagit.exe not found at ${MediaGitPath}" -ForegroundColor Yellow
}
