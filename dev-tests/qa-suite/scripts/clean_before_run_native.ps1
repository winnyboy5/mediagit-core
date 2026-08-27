# Pre-run cleanup for the ga32 clearance campaign -- NATIVE SILO variant.
#
# WHY A VARIANT. clean_before_run.ps1 resets storage with
# `docker compose -f docker-compose.minio.yml down -v`. That file now pulls
# Silo images (914412e), but the Docker daemon is not running on this box at
# all: the S3 backend is a NATIVE silo.exe --
#   C:\Users\Admin\silo\silo.exe server D:\silo-data --address 127.0.0.1:9000
#
# Running the Docker script here is worse than useless. `down -v` fails on a
# dead daemon, and then its health probe HTTP-GETs :9000 and PASSES, because
# native Silo answers it -- so it prints "MinIO healthy on :9000" after a reset
# that did nothing. It aborts at the bucket check (exit 92), but the healthy
# line is the same gate-that-lies shape this suite keeps finding in itself.
#
# Everything else (work/, .last-tier, log retention) is carried over verbatim.

param(
  [switch]$Fixtures,
  [string[]]$KeepLogs = @("20260818-gagate8", "20260819-gagate13",
                          "perfguard-a", "perfguard-b", "perfguard-c",
                          "20260820-gate14", "provefloor"),
  [int]$KeepHours = 48
)

$ErrorActionPreference = "Continue"
$repo = "D:\own\saas\mediagit-core"
$qa   = Join-Path $repo "dev-tests\qa-suite"
$mc   = "C:\Users\Admin\silo\mcli.exe"
Set-Location $repo

function Say([string]$m) { Write-Host ("[clean] " + $m) }

# ---- 1. work ---------------------------------------------------------------
$work = Join-Path $qa "work"
if (Test-Path $work) {
    $mb = [math]::Round((Get-ChildItem $work -Recurse -Force -EA SilentlyContinue |
          Measure-Object -Property Length -Sum).Sum / 1GB, 2)
    Say "work/ = $mb GB - removing"
    Get-Process mediagit-server, mediagit -EA SilentlyContinue | ForEach-Object {
        Say "  killing leftover $($_.ProcessName) pid=$($_.Id)"
        Stop-Process -Id $_.Id -Force -EA SilentlyContinue
    }
    Remove-Item $work -Recurse -Force -EA SilentlyContinue
}
New-Item -ItemType Directory -Path $work -Force | Out-Null
if (-not $Fixtures) {
    Set-Content -Path (Join-Path $work ".last-tier") -Value "SCALE" -Encoding ascii
    Say "wrote work\.last-tier = SCALE (fixtures kept)"
}

# ---- 2. fixtures (opt-in) --------------------------------------------------
if ($Fixtures) {
    Say "fixtures/ - removing (forces 01_preflight -Regen)"
    Remove-Item (Join-Path $qa "fixtures-synthetic") -Recurse -Force -EA SilentlyContinue
}

# ---- 3. logs ---------------------------------------------------------------
$logs = Join-Path $qa "logs"
if (Test-Path $logs) {
    $cutoff = (Get-Date).AddHours(-$KeepHours)
    Get-ChildItem $logs -Force | ForEach-Object {
        if ($KeepLogs -contains $_.Name) {
            Say "keeping logs\$($_.Name) (named)"
        } elseif ($_.LastWriteTime -gt $cutoff) {
            Say "keeping logs\$($_.Name) (modified $([int]((Get-Date) - $_.LastWriteTime).TotalHours)h ago, under ${KeepHours}h)"
        } else {
            Say "removing logs\$($_.Name)"
            Remove-Item $_.FullName -Recurse -Force -EA SilentlyContinue
        }
    }
}

# ---- 4. Silo buckets -------------------------------------------------------
if (-not (Test-Path $mc)) { Say "FATAL: mcli.exe not found at $mc"; exit 90 }

# The server must be the one we think it is before we delete anything through it.
$listener = Get-NetTCPConnection -LocalPort 9000 -State Listen -EA SilentlyContinue |
            Select-Object -First 1
if (-not $listener) { Say "FATAL: nothing is listening on :9000"; exit 91 }
$owner = (Get-Process -Id $listener.OwningProcess -EA SilentlyContinue).ProcessName
Say ":9000 owned by '$owner' pid=$($listener.OwningProcess)"
if ($owner -ne "silo") { Say "FATAL: expected native 'silo' on :9000, found '$owner'"; exit 92 }

& $mc alias set qa http://127.0.0.1:9000 minioadmin minioadmin | Out-Null
if ($LASTEXITCODE -ne 0) { Say "FATAL: mcli could not reach Silo"; exit 93 }

$buckets = @("mediagit-qa-suite", "mediagit-repos", "mediagit-test")
foreach ($b in $buckets) {
    $before = @(& $mc ls --recursive "qa/$b" 2>$null).Count
    Say "$b - $before object(s) before"
    if ($before -gt 0) {
        # --force is required for a non-empty recursive remove; it is scoped to
        # this one bucket path, not the alias.
        & $mc rm --recursive --force "qa/$b" 2>&1 | Select-Object -Last 2 | Out-String | Write-Host
    }
}

# Verify EMPTY by re-listing, not by trusting the remove's exit code. The
# original script's comment is the reason: a check that reports "0 objects"
# because the command failed to run is worse than no check.
$total = 0
foreach ($b in $buckets) {
    $n = @(& $mc ls --recursive "qa/$b" 2>$null).Count
    Say "$b - $n object(s) after"
    $total += $n
}
if ($total -ne 0) { Say "FATAL: buckets still hold $total object(s)"; exit 94 }

# Buckets must still EXIST. Emptying is not the same as dropping, and a
# campaign against a missing bucket fails in a way that reads like a product bug.
$have = (& $mc ls qa 2>&1 | Out-String)
foreach ($b in $buckets) {
    if ($have -notmatch [regex]::Escape($b)) { Say "FATAL: bucket $b missing after cleanup"; exit 95 }
}
Say "buckets present and empty"

Say "CLEAN_DONE"
