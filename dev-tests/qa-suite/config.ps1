# qa-suite config - single source of truth, every knob env-overridable. ASCII-only, PS 5.1 compatible.
# Dot-sourced by scripts/lib/common.ps1; do not run directly.

function _Env([string]$name, [string]$default) {
  $v = [Environment]::GetEnvironmentVariable($name)
  if ($v) { $v } else { $default }
}

$QA = [ordered]@{}
$QA.Root      = $PSScriptRoot                                                    # dev-tests/qa-suite
$QA.RepoRoot  = (Resolve-Path (Join-Path $PSScriptRoot "..\..")).Path            # mediagit-core
$QA.MG        = _Env "MG"               (Join-Path $QA.RepoRoot "target\release\mediagit.exe")
$QA.MGServer  = _Env "MG_SERVER"        (Join-Path $QA.RepoRoot "target\release\mediagit-server.exe")
$QA.TestFiles = _Env "MG_QA_TESTFILES"  (Join-Path $QA.RepoRoot "test-files")
$QA.Tier      = (_Env "MG_QA_TIER" "STANDARD").ToUpper()                         # STANDARD | STRESS
$QA.MaxFixtureMB = if ($QA.Tier -eq "STRESS") { 999999 } else { [int](_Env "MG_QA_MAX_MB" "500") }
$QA.Backends  = (_Env "MG_QA_BACKENDS" "minio,aws,azure,gcs") -split "," | ForEach-Object { $_.Trim().ToLower() } | Where-Object { $_ }
$QA.MinioEndpoint  = _Env "MG_QA_MINIO"        "http://localhost:9000"
$QA.MinioAccessKey = _Env "MG_QA_MINIO_ACCESS" "minioadmin"
$QA.MinioSecretKey = _Env "MG_QA_MINIO_SECRET" "minioadmin"
$QA.Work      = _Env "MG_QA_WORKDIR" (Join-Path $QA.Root "work")
$QA.Fixtures  = Join-Path $QA.Root "fixtures-synthetic"
# One run id shared across phases: run_all.ps1 exports MG_QA_RUN_ID; standalone script runs get their own.
$QA.RunId     = _Env "MG_QA_RUN_ID" (Get-Date -Format "yyyyMMdd-HHmmss")
$QA.Logs      = Join-Path $QA.Root ("logs\" + $QA.RunId)
$QA.Reports   = Join-Path $QA.Root ("reports\" + $QA.RunId)

foreach ($d in @($QA.Work, $QA.Fixtures, $QA.Logs, $QA.Reports)) {
  if (-not (Test-Path $d)) { New-Item -ItemType Directory -Path $d -Force | Out-Null }
}

# Prevent editor hangs in matrix rows
$env:EDITOR = 'cmd /c rem'
$env:GIT_EDITOR = 'cmd /c rem'
