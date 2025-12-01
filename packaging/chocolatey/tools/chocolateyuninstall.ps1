# MediaGit Chocolatey Uninstallation Script

$ErrorActionPreference = 'Stop'

$packageName = 'mediagit'

# Remove shim
Uninstall-BinFile -Name "mediagit"
