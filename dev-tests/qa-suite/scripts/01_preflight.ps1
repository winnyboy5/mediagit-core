# Phase 01: preflight validation - binaries, backend reachability, fixture generation.
# Generalized from standalone-deep-v11/scripts/{preflight-validation.ps1, cloud-smoke.ps1, verify_auth.ps1}
# (originals untouched; patterns only). ASCII-only, PS 5.1 compatible.
#
# Fixture generators (gen_chain_fixtures.py, gen_ml_fixtures.py, gen_vfx_fixtures.py) import,
# on top of numpy (always required):
#   gen_chain_fixtures.py : soundfile, PIL (Pillow), pygltflib
#   gen_ml_fixtures.py    : safetensors, pyarrow, onnx
#   gen_vfx_fixtures.py   : OpenEXR
# gen_manifest.py is stdlib-only (hashlib/os/re).
# Missing packages => SKIP that generator's rows below, not a hard fail.
param(
  [switch]$Regen
)

. (Join-Path $PSScriptRoot "lib\common.ps1")

$Phase = "01_preflight"
$tsv = Join-Path $QA.Logs "preflight.tsv"
$header = @("check", "status", "detail")
$hardFail = $false

function Row([string]$Check, [string]$Status, [string]$Detail = "") {
  Write-QaRow $tsv $header @($Check, $Status, $Detail)
  Write-QaLog $Phase ("{0} = {1} {2}" -f $Check, $Status, $Detail)
}

# ---------------------------------------------------------------
# Binaries
# ---------------------------------------------------------------
if (Test-Path $QA.MG) {
  $verOut = & $QA.MG version 2>&1 | Out-String
  if ($LASTEXITCODE -eq 0) {
    Row "mediagit.exe" "OK" ($verOut.Trim() -replace "`r?`n", " | ")
  } else {
    Row "mediagit.exe" "FAIL" "version exited $LASTEXITCODE"
    $hardFail = $true
  }
} else {
  Row "mediagit.exe" "FAIL" "not found at $($QA.MG)"
  $hardFail = $true
}

if (Test-Path $QA.MGServer) {
  Row "mediagit-server.exe" "OK" $QA.MGServer
} else {
  # Hard fail: without the server binary every remote/auth/scale drill degrades to a
  # skip, and the campaign reports green having tested no networked path at all.
  Row "mediagit-server.exe" "FAIL" "not found at $($QA.MGServer)"
  $hardFail = $true
}

# ---------------------------------------------------------------
# S3 backend on the QA endpoint (native Silo on this host; MinIO elsewhere).
# The `minio` token is the backend IDENTIFIER used throughout the suite -
# gate names, log filenames and $backend comparisons - so it stays put.
# ---------------------------------------------------------------
if ($QA.Backends -contains "minio") {
  try {
    $r = Invoke-WebRequest -Uri "$($QA.MinioEndpoint)/minio/health/live" -TimeoutSec 5 -UseBasicParsing -EA Stop
    if ($r.StatusCode -eq 200) { Row "minio" "OK" $QA.MinioEndpoint }
    else { Row "minio" "FAIL" "status $($r.StatusCode)"; $hardFail = $true }
  } catch {
    Row "minio" "FAIL" $_.Exception.Message
    $hardFail = $true
  }
} else {
  Row "minio" "SKIP" "not in `$QA.Backends"
}

# ---------------------------------------------------------------
# Cloud backends: credential env vars + trivial reachability probe.
# Missing creds => SKIP, never a hard fail (per v11 cloud-smoke.ps1 pattern).
# ---------------------------------------------------------------
foreach ($b in @("aws", "azure", "gcs")) {
  if ($QA.Backends -notcontains $b) { Row $b "SKIP" "not in `$QA.Backends"; continue }

  switch ($b) {
    "aws" {
      $key = _Env "AWS_ACCESS_KEY_ID" ""
      $secret = _Env "AWS_SECRET_ACCESS_KEY" ""
      if (-not $key -or -not $secret) {
        Row "aws" "SKIP" "AWS_ACCESS_KEY_ID / AWS_SECRET_ACCESS_KEY not set"
      } else {
        Row "aws" "OK" "credentials present"
      }
    }
    "azure" {
      $acct = _Env "AZURE_STORAGE_ACCOUNT" ""
      $akey = _Env "AZURE_STORAGE_KEY" ""
      if (-not $acct -or -not $akey) {
        Row "azure" "SKIP" "AZURE_STORAGE_ACCOUNT / AZURE_STORAGE_KEY not set"
      } else {
        Row "azure" "OK" "credentials present"
      }
    }
    "gcs" {
      $creds = _Env "GOOGLE_APPLICATION_CREDENTIALS" ""
      if (-not $creds -or -not (Test-Path $creds)) {
        Row "gcs" "SKIP" "GOOGLE_APPLICATION_CREDENTIALS not set or file missing"
      } else {
        Row "gcs" "OK" "credentials present: $creds"
      }
    }
  }
}

# ---------------------------------------------------------------
# Python + fixture-generator dependencies
# ---------------------------------------------------------------
$pyOk = $false
# Prefer the suite-local venv (created once: python -m venv .venv; pip install numpy pillow pyarrow soundfile pygltflib onnx safetensors OpenEXR)
$py = Join-Path $QA.Root ".venv\Scripts\python.exe"
if (-not (Test-Path $py)) { $py = "python" }
try {
  $pyVer = & $py --version 2>&1 | Out-String
  if ($LASTEXITCODE -eq 0) { $pyOk = $true; Row "python" "OK" $pyVer.Trim() }
  else { Row "python" "SKIP" "python not runnable" }
} catch {
  Row "python" "SKIP" $_.Exception.Message
}

# generator -> required non-stdlib packages
$genDeps = [ordered]@{
  "gen_chain_fixtures.py" = @("numpy", "soundfile", "PIL", "pygltflib")
  "gen_ml_fixtures.py"    = @("numpy", "safetensors", "pyarrow", "onnx")
  "gen_vfx_fixtures.py"   = @("numpy", "OpenEXR")
  "gen_manifest.py"       = @()
}
$genReady = [ordered]@{}

if ($pyOk) {
  foreach ($gen in $genDeps.Keys) {
    $missing = @()
    foreach ($mod in $genDeps[$gen]) {
      & $py -c "import $mod" 2>$null
      if ($LASTEXITCODE -ne 0) { $missing += $mod }
    }
    $genReady[$gen] = ($missing.Count -eq 0)
    if ($missing.Count -eq 0) {
      Row "pydeps:$gen" "OK" ($genDeps[$gen] -join ",")
    } else {
      Row "pydeps:$gen" "SKIP" ("missing: " + ($missing -join ","))
    }
  }
} else {
  foreach ($gen in $genDeps.Keys) { $genReady[$gen] = $false; Row "pydeps:$gen" "SKIP" "python unavailable" }
}

# ---------------------------------------------------------------
# Fixture generation (skip if manifest already present and -Regen not passed)
# ---------------------------------------------------------------
$manifestFile = Join-Path $QA.Fixtures "manifest-synthetic.tsv"
# Export the RESOLVED paths for the python generators. $QA.Fixtures/$QA.TestFiles already
# honour MG_QA_FIXTURES/MG_QA_TESTFILES (config.ps1), so an operator pointing the campaign
# at an alternate corpus is respected here instead of being overwritten with the default.
$env:MG_QA_FIXTURES = $QA.Fixtures
$env:MG_QA_TESTFILES = $QA.TestFiles
Write-QaLog $Phase "fixtures dir = $($env:MG_QA_FIXTURES); test-files dir = $($env:MG_QA_TESTFILES)"

if ((Test-Path $manifestFile) -and (-not $Regen)) {
  Row "fixture-gen" "SKIP" "manifest already present ($manifestFile); pass -Regen to force"
} elseif (-not $pyOk) {
  Row "fixture-gen" "SKIP" "python unavailable"
} else {
  foreach ($gen in @("gen_chain_fixtures.py", "gen_ml_fixtures.py", "gen_vfx_fixtures.py")) {
    if ($genReady[$gen]) {
      $out = & $py (Join-Path $PSScriptRoot $gen) 2>&1 | Out-String
      $out | Add-Content (Join-Path $QA.Logs "$Phase-$gen.log")
      # A generator whose deps are present but which then FAILS is a broken corpus, not
      # an absent capability: every downstream phase would silently test a partial fixture
      # set. Missing deps stay a SKIP; a crash is a hard fail.
      if ($LASTEXITCODE -eq 0) { Row "run:$gen" "OK" "" }
      else { Row "run:$gen" "FAIL" "exit $LASTEXITCODE (see $Phase-$gen.log)"; $hardFail = $true }
    } else {
      Row "run:$gen" "SKIP" "missing deps"
    }
  }

  $out = & $py (Join-Path $PSScriptRoot "gen_manifest.py") 2>&1 | Out-String
  $out | Add-Content (Join-Path $QA.Logs "$Phase-gen_manifest.py.log")
  if ($LASTEXITCODE -eq 0) { Row "run:gen_manifest.py" "OK" "" }
  else { Row "run:gen_manifest.py" "FAIL" "exit $LASTEXITCODE"; $hardFail = $true }
}

# ---------------------------------------------------------------
# SCALE tier: many-files corpus (gen_scale_fixtures.py) + free-disk budget check.
# Only under MG_QA_TIER=SCALE; STANDARD/STRESS runs are unaffected. (For SCALED ml/vfx
# fixture SIZES, re-run this phase with -Regen under SCALE - MG_QA_SCALE is read there.)
# ---------------------------------------------------------------
if ($QA.Tier -eq "SCALE") {
  $manyFiles = Join-Path $QA.Fixtures "scale\manyfiles"
  if (-not $pyOk) {
    Row "scale-fixtures" "SKIP" "python unavailable"
  } elseif ((Test-Path $manyFiles) -and (-not $Regen)) {
    Row "scale-fixtures" "SKIP" "present ($manyFiles); pass -Regen to force"
  } else {
    $out = & $py (Join-Path $PSScriptRoot "gen_scale_fixtures.py") 2>&1 | Out-String
    $out | Add-Content (Join-Path $QA.Logs "$Phase-gen_scale_fixtures.py.log")
    if ($LASTEXITCODE -eq 0) { Row "run:gen_scale_fixtures.py" "OK" "filecount=$($QA.FileCount)" }
    else { Row "run:gen_scale_fixtures.py" "FAIL" "exit $LASTEXITCODE" }
  }

  $freeGB = Get-QaFreeDiskGB $QA.Work
  if ($freeGB -lt 0) {
    Row "scale-disk-budget" "SKIP" "free space undeterminable"
  } elseif ($freeGB -ge $QA.DiskBudgetGB) {
    Row "scale-disk-budget" "OK" "free ${freeGB}GB >= budget $($QA.DiskBudgetGB)GB"
  } else {
    # not a hard fail: phase 10's size-heavy drills (S4/S5) SKIP themselves when disk is short.
    Row "scale-disk-budget" "WARN" "free ${freeGB}GB < budget $($QA.DiskBudgetGB)GB - S4/S5 will SKIP"
  }
}

# ---------------------------------------------------------------
# Determinism gate: regenerate ONE small fixture (map_v1.svg, chain generator) into a temp
# dir and compare its hash to the one already in $QA.Fixtures.
# ---------------------------------------------------------------
$fixturesPresent = Test-Path $manifestFile
if ($fixturesPresent -and $pyOk -and $genReady["gen_chain_fixtures.py"]) {
  $tmpOut = Join-Path $QA.Work "determinism-check"
  if (Test-Path $tmpOut) { Remove-Item -Recurse -Force $tmpOut }
  New-Item -ItemType Directory -Path $tmpOut -Force | Out-Null

  $prevFixtures = $env:MG_QA_FIXTURES
  $env:MG_QA_FIXTURES = $tmpOut
  & $py (Join-Path $PSScriptRoot "gen_chain_fixtures.py") *> (Join-Path $QA.Logs "$Phase-determinism.log")
  $env:MG_QA_FIXTURES = $prevFixtures

  $orig = Join-Path $QA.Fixtures "chains\map_v1.svg"
  $redo = Join-Path $tmpOut "chains\map_v1.svg"
  if ((Test-Path $orig) -and (Test-Path $redo)) {
    $h1 = Get-QaHash $orig
    $h2 = Get-QaHash $redo
    if ($h1 -eq $h2) { Row "determinism" "OK" "map_v1.svg hash matches" }
    else {
      # Hard gate. Byte-identical regeneration is the premise the whole campaign rests on:
      # dedup percentages, delta chains and clone-parity comparisons are only meaningful
      # against a corpus that is the same corpus every run. A drifting generator produces
      # storage-economics numbers that cannot be compared to any previous or later run.
      Row "determinism" "FAIL" "map_v1.svg hash mismatch: $h1 vs $h2"
      $hardFail = $true
    }
  } else {
    Row "determinism" "SKIP" "map_v1.svg missing in original or redo output"
  }
} else {
  Row "determinism" "SKIP" "fixtures or chain generator unavailable"
}

# ---------------------------------------------------------------
# Gates
# ---------------------------------------------------------------
$fixturesGateOk = Test-Path $manifestFile
Write-QaGate $Phase "preflight-hard-checks" (-not $hardFail) "binaries, selected backends, fixture generation, determinism"
Write-QaGate $Phase "fixtures-present" $fixturesGateOk $manifestFile

# The determinism check regenerates a full chain fixture set (~366MB) into work/ purely
# to hash one svg; nothing downstream reads it.
Invoke-QaTeardown $Phase @("determinism-check")

Exit-QaPhase $Phase
