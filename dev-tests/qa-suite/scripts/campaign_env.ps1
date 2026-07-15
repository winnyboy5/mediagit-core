# Dot-source to export cloud credentials from dev-tests\dev-server\config.*.toml
# into the env vars lib\remote.ps1 consumes. Values are never printed.
# ponytail: naive key="value"/key=value TOML line parse - fine for these flat files.

$devServer = Join-Path (Split-Path (Split-Path $PSScriptRoot -Parent) -Parent) "dev-server"

function _TomlVal([string]$File, [string]$Key) {
  if (-not (Test-Path $File)) { return $null }
  $m = Select-String -Path $File -Pattern ("^\s*" + $Key + "\s*=\s*(.+)$") | Select-Object -First 1
  if (-not $m) { return $null }
  return $m.Matches[0].Groups[1].Value.Trim().Trim('"')
}

$aws = Join-Path $devServer "config.aws.toml"
if (-not $env:AWS_ACCESS_KEY_ID)     { $env:AWS_ACCESS_KEY_ID     = _TomlVal $aws "access_key_id" }
if (-not $env:AWS_SECRET_ACCESS_KEY) { $env:AWS_SECRET_ACCESS_KEY = _TomlVal $aws "secret_access_key" }
if (-not $env:AWS_DEFAULT_REGION)    { $env:AWS_DEFAULT_REGION    = _TomlVal $aws "region" }
if (-not $env:MG_QA_AWS_BUCKET)      { $env:MG_QA_AWS_BUCKET      = _TomlVal $aws "bucket" }

$az = Join-Path $devServer "config.azure.toml"
if (-not $env:AZURE_STORAGE_ACCOUNT)  { $env:AZURE_STORAGE_ACCOUNT  = _TomlVal $az "account_name" }
if (-not $env:AZURE_STORAGE_KEY)      { $env:AZURE_STORAGE_KEY      = _TomlVal $az "account_key" }
if (-not $env:MG_QA_AZURE_CONTAINER)  { $env:MG_QA_AZURE_CONTAINER  = _TomlVal $az "container" }

$gcs = Join-Path $devServer "config.gcs.toml"
if (-not $env:GCS_PROJECT_ID)   { $env:GCS_PROJECT_ID   = _TomlVal $gcs "project_id" }
if (-not $env:GCS_BUCKET_NAME)  { $env:GCS_BUCKET_NAME  = _TomlVal $gcs "bucket" }
if (-not $env:GOOGLE_APPLICATION_CREDENTIALS) {
  $credPath = _TomlVal $gcs "credentials_path"
  if ($credPath -and (Test-Path $credPath)) { $env:GOOGLE_APPLICATION_CREDENTIALS = $credPath }
}

$set = @("AWS_ACCESS_KEY_ID","AWS_SECRET_ACCESS_KEY","AZURE_STORAGE_ACCOUNT","AZURE_STORAGE_KEY","GCS_PROJECT_ID","GOOGLE_APPLICATION_CREDENTIALS") |
  ForEach-Object { "{0}={1}" -f $_, $(if ([Environment]::GetEnvironmentVariable($_)) { "set" } else { "MISSING" }) }
Write-Host ("campaign_env: " + ($set -join " "))
