# MediaGit Chocolatey Installation Script

$ErrorActionPreference = 'Stop'

$packageName = 'mediagit'
$toolsDir = "$(Split-Path -parent $MyInvocation.MyCommand.Definition)"
$version = '0.1.0'

# Detect architecture
$arch = if ([Environment]::GetEnvironmentVariable("PROCESSOR_ARCHITECTURE") -eq "ARM64") {
    "aarch64"
} else {
    "x86_64"
}

$url = "https://github.com/yourusername/mediagit-core/releases/download/v${version}/mediagit-${version}-${arch}-windows.zip"
$checksum = 'PLACEHOLDER_CHECKSUM'
$checksumType = 'sha256'

$packageArgs = @{
  packageName   = $packageName
  unzipLocation = $toolsDir
  url64bit      = $url
  checksum64    = $checksum
  checksumType64= $checksumType
}

Install-ChocolateyZipPackage @packageArgs

# Create shim for mediagit.exe
$exePath = Join-Path $toolsDir "mediagit.exe"
if (Test-Path $exePath) {
    Install-BinFile -Name "mediagit" -Path $exePath
}
