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

# Same, but for a key that may sit INSIDE an inline table rather than at the
# start of its own line.
#
# config_version 3 moved the Azure credentials into a tagged block --
# `auth = { type = "account_key", account_name = "...", account_key = "..." }`
# -- so exactly one credential kind is representable. `_TomlVal` anchors with
# `^\s*`, so after that migration it could no longer see either value and
# quietly returned $null for both. The result was not an error: `remote.ps1`
# throws "SKIP: Azure not configured", and a SKIP reads as green. Azure was
# therefore able to drop out of a run while the run still looked complete --
# the failure shape this suite keeps finding in itself.
#
# `type = "account_key"` does NOT collide with the `account_key` lookup: there
# the text is inside quotes and is not followed by `=`.
function _TomlInlineVal([string]$File, [string]$Key) {
  if (-not (Test-Path $File)) { return $null }
  $m = Select-String -Path $File -Pattern ($Key + '\s*=\s*"([^"]*)"') | Select-Object -First 1
  if (-not $m) { return $null }
  return $m.Matches[0].Groups[1].Value
}

$aws = Join-Path $devServer "config.aws.toml"
if (-not $env:AWS_ACCESS_KEY_ID)     { $env:AWS_ACCESS_KEY_ID     = _TomlVal $aws "access_key_id" }
if (-not $env:AWS_SECRET_ACCESS_KEY) { $env:AWS_SECRET_ACCESS_KEY = _TomlVal $aws "secret_access_key" }
if (-not $env:AWS_DEFAULT_REGION)    { $env:AWS_DEFAULT_REGION    = _TomlVal $aws "region" }
if (-not $env:MG_QA_AWS_BUCKET)      { $env:MG_QA_AWS_BUCKET      = _TomlVal $aws "bucket" }

$az = Join-Path $devServer "config.azure.toml"
if (-not $env:AZURE_STORAGE_ACCOUNT)  {
  $v = _TomlVal $az "account_name"; if (-not $v) { $v = _TomlInlineVal $az "account_name" }
  $env:AZURE_STORAGE_ACCOUNT = $v
}
if (-not $env:AZURE_STORAGE_KEY)      {
  $v = _TomlVal $az "account_key"; if (-not $v) { $v = _TomlInlineVal $az "account_key" }
  $env:AZURE_STORAGE_KEY = $v
}
if (-not $env:MG_QA_AZURE_CONTAINER)  { $env:MG_QA_AZURE_CONTAINER  = _TomlVal $az "container" }

$gcs = Join-Path $devServer "config.gcs.toml"
if (-not $env:GCS_PROJECT_ID)   { $env:GCS_PROJECT_ID   = _TomlVal $gcs "project_id" }
if (-not $env:GCS_BUCKET_NAME)  { $env:GCS_BUCKET_NAME  = _TomlVal $gcs "bucket" }
if (-not $env:GOOGLE_APPLICATION_CREDENTIALS) {
  $credPath = _TomlVal $gcs "credentials_path"
  # credentials_path in the TOML may be relative; consumers (server per-run dirs) have a
  # different cwd, so export an absolute path or GCS pushes 500 on "file not found".
  if ($credPath -and -not [IO.Path]::IsPathRooted($credPath)) {
    # Repo root included because these paths are conventionally written
    # relative to it, while the campaign is launched from scripts\ — without
    # it GCS silently preflight-SKIPs and a "full 5-backend" campaign quietly
    # covers four.
    $repoRoot = Split-Path (Split-Path (Split-Path $PSScriptRoot -Parent) -Parent) -Parent
    foreach ($base in @($devServer, $repoRoot, (Get-Location).Path)) {
      $cand = Join-Path $base $credPath
      if (Test-Path $cand) { $credPath = (Resolve-Path $cand).Path; break }
    }
  }
  if ($credPath -and (Test-Path $credPath)) { $env:GOOGLE_APPLICATION_CREDENTIALS = (Resolve-Path $credPath).Path }
}

# How 07_abuse's A7 drill cycles the S3 backend. Docker stays the default when
# a container is actually there; this only fills the gap on a host where a
# NATIVE process holds the endpoint, which is where A7 failed in ga32 with
# "configuration, not capability".
#
# Detected rather than hardcoded: silo_native.ps1 reads the running process's
# own command line, so this tracked file carries no local install path and the
# pair is correct on whatever host the campaign runs on.
if (-not $env:MG_QA_BACKEND_STOP_CMD -and -not $env:MG_QA_BACKEND_START_CMD) {
  $siloScript = Join-Path $PSScriptRoot "silo_native.ps1"
  $holder = $null
  try {
    # Derive the port from MG_QA_MINIO rather than assuming 9000. A7's
    # stop/start pair must target the port the campaign actually uses, or the
    # drill cycles a backend nobody is talking to and still reports a result.
    $siloPort = 9000
    if ($env:MG_QA_MINIO -and ($env:MG_QA_MINIO -match ':(\d+)')) { $siloPort = [int]$matches[1] }
    $conn = Get-NetTCPConnection -LocalPort $siloPort -State Listen -ErrorAction SilentlyContinue |
            Select-Object -First 1
    if ($conn) { $holder = (Get-Process -Id $conn.OwningProcess -ErrorAction SilentlyContinue).ProcessName }
  } catch { }
  if ($holder -eq "silo" -and (Test-Path $siloScript)) {
    $env:MG_QA_BACKEND_STOP_CMD  = "& '$siloScript' -Action stop -Port $siloPort"
    $env:MG_QA_BACKEND_START_CMD = "& '$siloScript' -Action start -Port $siloPort"
    Write-Host "campaign_env: native silo on :$siloPort - A7 backend-cycle commands wired to silo_native.ps1"
  }
}

$set = @("AWS_ACCESS_KEY_ID","AWS_SECRET_ACCESS_KEY","AZURE_STORAGE_ACCOUNT","AZURE_STORAGE_KEY","MG_QA_AZURE_CONTAINER","GCS_PROJECT_ID","GOOGLE_APPLICATION_CREDENTIALS") |
  ForEach-Object { "{0}={1}" -f $_, $(if ([Environment]::GetEnvironmentVariable($_)) { "set" } else { "MISSING" }) }
Write-Host ("campaign_env: " + ($set -join " "))
