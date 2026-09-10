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
$QA.Tier      = (_Env "MG_QA_TIER" "STANDARD").ToUpper()                         # STANDARD | STRESS | SCALE
# STRESS and SCALE both lift the fixture size cap; SCALE additionally turns on the
# scale/aggression axes below and phase 10 (see run_all.ps1).
$QA.MaxFixtureMB = if ($QA.Tier -eq "STRESS" -or $QA.Tier -eq "SCALE") { 999999 } else { [int](_Env "MG_QA_MAX_MB" "500") }

# ---- Scale/aggression knobs (phase 10; only meaningful under MG_QA_TIER=SCALE) ----
# MG_QA_SCALE multiplies synthetic fixture sizes (read by gen_ml/gen_vfx/gen_scale);
# defaults to 10 under SCALE so a run lands near the ~10GB target, 1 otherwise.
$QA.Scale        = [int](_Env "MG_QA_SCALE" $(if ($QA.Tier -eq "SCALE") { "10" } else { "1" }))
$QA.FileCount    = [int](_Env "MG_QA_FILECOUNT" "10000")     # many-files corpus size (gen_scale)
$QA.Concurrency  = [int](_Env "MG_QA_CONCURRENCY" "16")      # parallel clients for S1
$QA.ChurnCommits = [int](_Env "MG_QA_CHURN_COMMITS" "500")   # rapid-commit count for S2
$QA.CloudMaxMB   = [int](_Env "MG_QA_CLOUD_MAX_MB" "2048")   # cap cloud-backend payload
# Cloud throughput floor, MB/s, applied to BOTH push and clone on billed backends
# (10_scale.ps1:549,563). 0 = informational only.
# 1.0 is a catastrophic-regression detector, NOT a performance target: it is deliberately
# far below measured throughput so ordinary WAN variance can never fail a run, while an
# order-of-magnitude regression still does. Reference measurements 2026-07-27 on a 2048 MB
# payload: aws 2.76 push / 6.93 clone, azure 3.17 push / 8.06 clone. At 1.0 MB/s a 2 GB
# transfer must exceed ~34 min to fail - which is what a reverted Azure ranged-read (full
# blob per range) or a dead upload-concurrency knob would actually look like.
# Raise this only with a faster link; never lower it to make a failing run pass.
$QA.CloudMbsFloor = [double](_Env "MG_QA_CLOUD_MBS_FLOOR" "1.0")
$QA.DiskBudgetGB = [double](_Env "MG_QA_DISK_BUDGET_GB" "40")# scratch footprint ceiling
$QA.RssCeilMB    = [int](_Env "MG_QA_RSS_CEIL_MB" "4096")    # peak working-set REPORT bound (S4)
# S4's actual gate. Private bytes, not working set: on Windows the working set
# absorbs mapped-file and page-cache pages that are not the process's own memory,
# so reading a multi-GB blob inflates it by however much RAM happened to be free.
# Measured across three identical SCALE runs: working set 1223 / 2931 / 3697 MB
# (3x spread, pure noise) while private held 689 / 704 / 707 MB (2.6% spread).
# 1536 is ~2.2x the observed private peak -- loose enough to absorb variance,
# and still far below the 4096MB blob, so a client that buffers instead of
# streaming blows through it. Do not raise it to 4096: that is the blob size,
# and a ceiling at the blob size cannot catch buffering at all.
$QA.RssPrivateCeilMB = [int](_Env "MG_QA_RSS_PRIVATE_CEIL_MB" "1536")
$QA.KeepScratch  = (_Env "MG_QA_KEEP_SCRATCH" "0") -eq "1"   # skip post-phase teardown for triage
$QA.PurgeFixtures = (_Env "MG_QA_PURGE_FIXTURES" "0") -eq "1"# also delete generated scale fixtures
$QA.Backends  = (_Env "MG_QA_BACKENDS" "minio,aws,azure,gcs") -split "," | ForEach-Object { $_.Trim().ToLower() } | Where-Object { $_ }
# 127.0.0.1, not localhost: on Windows, localhost resolves ::1 first and a
# wedged Docker Desktop wslrelay on ::1:9000 accepts-but-never-forwards,
# hanging every S3 call (seen 2026-07-19 after the A7 outage drill).
$QA.MinioEndpoint  = _Env "MG_QA_MINIO"        "http://127.0.0.1:9000"
$QA.MinioAccessKey = _Env "MG_QA_MINIO_ACCESS" "minioadmin"
$QA.MinioSecretKey = _Env "MG_QA_MINIO_SECRET" "minioadmin"
$QA.Work      = _Env "MG_QA_WORKDIR" (Join-Path $QA.Root "work")
$QA.Fixtures  = _Env "MG_QA_FIXTURES" (Join-Path $QA.Root "fixtures-synthetic")
# One run id shared across phases: run_all.ps1 exports MG_QA_RUN_ID; standalone script runs get their own.
$QA.RunId     = _Env "MG_QA_RUN_ID" (Get-Date -Format "yyyyMMdd-HHmmss")
$QA.Logs      = Join-Path $QA.Root ("logs\" + $QA.RunId)
$QA.Reports   = Join-Path $QA.Root ("reports\" + $QA.RunId)

# Directory creation is a side effect, so it is a function rather than load-time work:
# dot-sourcing config.ps1 to READ a knob (run_all's phase planning, an operator's
# `. config.ps1; $QA.Tier`) must not litter logs\<runid>\ dirs for runs that never happen.
# lib\common.ps1 calls this immediately after loading, which covers run_all and every
# standalone phase invocation.
function Initialize-QaDirs {
  foreach ($d in @($QA.Work, $QA.Fixtures, $QA.Logs, $QA.Reports)) {
    if (-not (Test-Path $d)) { New-Item -ItemType Directory -Path $d -Force | Out-Null }
  }
}

# Prevent editor hangs in matrix rows
$env:EDITOR = 'cmd /c rem'
$env:GIT_EDITOR = 'cmd /c rem'
