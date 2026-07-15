# qa-suite server lifecycle helper. ASCII-only, PS 5.1 compatible.
# Dot-sourced by phase scripts alongside common.ps1:
#   . (Join-Path $PSScriptRoot "lib\common.ps1")
#   . (Join-Path $PSScriptRoot "lib\remote.ps1")
#
# Ground truth for config/launch mechanics: dev-tests\standalone-deep-v11\scripts\run_remote.ps1
# (per-backend bare repo + storage config.toml) and verify_auth.ps1 (server.toml + --config).
#
# API (fixed - other scripts depend on this shape):
#   Start-QaServer -Backend <minio|aws|azure|gcs|local> -Phase <name>  -> @{Url; Proc; DataDir; Backend}
#   Stop-QaServer <that object>
#
# Mechanics discovered in the codebase:
#   - mediagit-server.exe reads a GLOBAL config (--config server.toml: port/host/repos_dir/auth).
#   - Storage BACKEND selection is per-repo: it lives in "<repos_dir>/<repo>/.mediagit/config.toml"
#     under a [storage] table (schema: crates\mediagit-config\src\schema.rs StorageConfig enum:
#     "filesystem" | "s3" | "azure" | "gcs" | "multi"). Bare repos need this file written BEFORE
#     the server is started (or repos_dir rescanned), same as run_remote.ps1 does.
#   - Start-QaServer therefore creates one bare repo "proj-<RunId>" inside the per-run data dir,
#     writes its storage config from config\backends\<backend>.toml with tokens substituted, then
#     starts the server and returns Url already pointing AT that repo (ready for `mg remote add`).
#     The per-run repo name avoids collisions with leftover objects from a prior run under the
#     same bucket/container (repo_namespace config key is known-ignored; storage keys derive from
#     the bare-repo folder name instead - see project_pending_items memory / BUG-RM-2).
#   - /health is auth-exempt (crates\mediagit-server\src\lib.rs) - used as the readiness probe.

. (Join-Path $PSScriptRoot "common.ps1")

function Get-QaFreePort {
  $l = New-Object System.Net.Sockets.TcpListener ([System.Net.IPAddress]::Loopback, 0)
  $l.Start()
  $port = $l.LocalEndpoint.Port
  $l.Stop()
  return $port
}

# Resolve backend -> @{ Template=<filename in config\backends>; Tokens=<hashtable> }.
# Throws "SKIP: ..." when required credentials/env are missing.
function _QaBackendConfig([string]$Backend) {
  switch ($Backend) {
    "minio" {
      if (-not $QA.MinioAccessKey -or -not $QA.MinioSecretKey -or -not $QA.MinioEndpoint) {
        throw "SKIP: MinIO not configured (QA.MinioEndpoint/MinioAccessKey/MinioSecretKey)"
      }
      $bucket = _Env "MG_QA_MINIO_BUCKET" "mediagit-qa-suite"
      return @{ Template = "minio.toml"; Tokens = @{
        ENDPOINT = $QA.MinioEndpoint; BUCKET = $bucket
        ACCESS_KEY = $QA.MinioAccessKey; SECRET_KEY = $QA.MinioSecretKey; REGION = "us-east-1"
      } }
    }
    "aws" {
      $ak = [Environment]::GetEnvironmentVariable("AWS_ACCESS_KEY_ID")
      $sk = [Environment]::GetEnvironmentVariable("AWS_SECRET_ACCESS_KEY")
      $bucket = [Environment]::GetEnvironmentVariable("MG_QA_AWS_BUCKET")
      $region = [Environment]::GetEnvironmentVariable("AWS_DEFAULT_REGION")
      if (-not $region) { $region = _Env "AWS_REGION" "us-east-1" }
      if (-not $ak -or -not $sk -or -not $bucket) {
        throw "SKIP: AWS not configured (need AWS_ACCESS_KEY_ID, AWS_SECRET_ACCESS_KEY, MG_QA_AWS_BUCKET env vars)"
      }
      return @{ Template = "aws.toml"; Tokens = @{
        ACCESS_KEY = $ak; SECRET_KEY = $sk; BUCKET = $bucket; REGION = $region
      } }
    }
    "azure" {
      $acct = [Environment]::GetEnvironmentVariable("AZURE_STORAGE_ACCOUNT")
      $key  = [Environment]::GetEnvironmentVariable("AZURE_STORAGE_KEY")
      $cont = [Environment]::GetEnvironmentVariable("MG_QA_AZURE_CONTAINER")
      if (-not $acct -or -not $key -or -not $cont) {
        throw "SKIP: Azure not configured (need AZURE_STORAGE_ACCOUNT, AZURE_STORAGE_KEY, MG_QA_AZURE_CONTAINER env vars)"
      }
      return @{ Template = "azure.toml"; Tokens = @{ ACCOUNT = $acct; KEY = $key; CONTAINER = $cont } }
    }
    "gcs" {
      $proj = [Environment]::GetEnvironmentVariable("GCS_PROJECT_ID")
      if (-not $proj) { $proj = [Environment]::GetEnvironmentVariable("GOOGLE_CLOUD_PROJECT") }
      $bucket = [Environment]::GetEnvironmentVariable("GCS_BUCKET_NAME")
      $creds  = [Environment]::GetEnvironmentVariable("GOOGLE_APPLICATION_CREDENTIALS")
      if (-not $proj -or -not $bucket) {
        throw "SKIP: GCS not configured (need GCS_PROJECT_ID, GCS_BUCKET_NAME env vars; GOOGLE_APPLICATION_CREDENTIALS optional/ADC)"
      }
      $credsLine = ""
      if ($creds) { $credsLine = 'credentials_path = "' + ($creds -replace '\\', '/') + '"' }
      return @{ Template = "gcs.toml"; Tokens = @{ PROJECT = $proj; BUCKET = $bucket; CREDS_LINE = $credsLine } }
    }
    "local" {
      return @{ Template = "local.toml"; Tokens = @{} }
    }
    default { throw "SKIP: unknown backend '$Backend'" }
  }
}

# Start-QaServer -Backend <minio|aws|azure|gcs|local> -Phase <name>
# Returns @{ Url; Proc; DataDir; Backend } on success.
# Throws an exception whose message starts with "SKIP:" when the backend cannot be
# used (missing credentials, server failed to become healthy, etc) - callers should
# wrap the call in try/catch and record a SKIP row rather than fail the whole phase.
function Start-QaServer {
  param(
    [Parameter(Mandatory = $true)][ValidateSet("minio", "aws", "azure", "gcs", "local")][string]$Backend,
    [Parameter(Mandatory = $true)][string]$Phase
  )

  $bc = _QaBackendConfig $Backend
  $tplPath = Join-Path $QA.Root "config\backends\$($bc.Template)"
  if (-not (Test-Path $tplPath)) { throw "SKIP: missing template $tplPath" }

  $srvDir = Join-Path $QA.Work "server-$Backend"
  if (Test-Path $srvDir) { Remove-Item -Recurse -Force $srvDir -ErrorAction SilentlyContinue }
  New-Item -ItemType Directory -Path (Join-Path $srvDir "repos") -Force | Out-Null

  $repoName = "proj-$($QA.RunId)-$Phase"
  $repoDir = Join-Path $srvDir "repos\$repoName"
  $r = Invoke-MG $null @("init", "--bare", $repoDir) $Phase
  if ($r.Exit -ne 0) { throw "init --bare failed for $repoDir : $($r.Out)" }

  $tokens = $bc.Tokens
  $tokens["BASE_PATH"] = (Join-Path $srvDir "storage") -replace '\\', '/'
  $cfg = Get-Content $tplPath -Raw
  foreach ($k in $tokens.Keys) { $cfg = $cfg.Replace("{{$k}}", "" + $tokens[$k]) }
  $cfg | Set-Content (Join-Path $repoDir ".mediagit\config.toml") -Encoding Ascii

  $port = Get-QaFreePort
  $reposDirFwd = (Join-Path $srvDir "repos") -replace '\\', '/'
  @"
port = $port
host = "127.0.0.1"
repos_dir = "$reposDirFwd"
"@ | Set-Content (Join-Path $srvDir "server.toml") -Encoding Ascii

  $outLog = Join-Path $QA.Logs "server-$Backend-$Phase.out.log"
  $errLog = Join-Path $QA.Logs "server-$Backend-$Phase.err.log"
  $proc = Start-Process -FilePath $QA.MGServer `
    -ArgumentList @("--config", (Join-Path $srvDir "server.toml")) `
    -PassThru -NoNewWindow -RedirectStandardOutput $outLog -RedirectStandardError $errLog

  $url = "http://127.0.0.1:$port"
  $healthy = $false
  for ($i = 0; $i -lt 40; $i++) {
    if ($proc.HasExited) { break }
    try {
      $resp = Invoke-WebRequest -Uri "$url/health" -UseBasicParsing -TimeoutSec 2 -ErrorAction Stop
      if ($resp.StatusCode -eq 200) { $healthy = $true; break }
    } catch {}
    Start-Sleep -Milliseconds 500
  }

  if (-not $healthy) {
    $errText = if (Test-Path $errLog) { (Get-Content $errLog -Raw -ErrorAction SilentlyContinue) } else { "" }
    if (-not $proc.HasExited) { Stop-Process -Id $proc.Id -Force -ErrorAction SilentlyContinue }
    throw "SKIP: mediagit-server ($Backend) never became healthy: $errText"
  }

  Write-QaLog $Phase "server $Backend up: $url/$repoName (pid $($proc.Id))"
  return @{ Url = "$url/$repoName"; Proc = $proc; DataDir = $srvDir; Backend = $Backend }
}

# Stop-QaServer <handle returned by Start-QaServer>. Kills the process tree; safe to call
# on $null or an already-stopped handle.
function Stop-QaServer($Handle) {
  if (-not $Handle -or -not $Handle.Proc) { return }
  try {
    $procId = $Handle.Proc.Id
    Get-CimInstance Win32_Process -Filter "ParentProcessId = $procId" -ErrorAction SilentlyContinue |
      ForEach-Object { Stop-Process -Id $_.ProcessId -Force -ErrorAction SilentlyContinue }
    if (-not $Handle.Proc.HasExited) { Stop-Process -Id $procId -Force -ErrorAction SilentlyContinue }
  } catch {}
}
