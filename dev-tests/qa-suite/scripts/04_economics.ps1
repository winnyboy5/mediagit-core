# 04_economics.ps1 - storage economics: per-format v1..vN chain repos.
# Generalized from dev-tests/standalone-deep-v11/scripts/measure_economics.ps1 (read-only reference; not modified).
param()
. (Join-Path $PSScriptRoot "lib\common.ps1")
$Phase = "04_economics"

# Pin the CDC seed for economics measurement: every `mediagit init` otherwise
# draws a random seed, and v3+ dedup on shifted-content chains (wav fade/append/
# trim) swings 2-4pp on boundary alignment alone (I11 RCA 2026-07-16: same
# release binary spanned 94.6-99.5 across three seed draws). A regression gate
# must be deterministic; real-world unpinned behavior is covered by every other
# phase. Value is arbitrary but MUST stay fixed - changing it re-anchors every
# threshold below.
$env:MEDIAGIT_CDC_SEED = "20260716"

$OUT = Join-Path $QA.Logs "economics.tsv"
$HEADER = @("family", "version", "fileMB", "odbGrowthMB", "savedPct", "addSec")

Write-QaLog $Phase "economics run starting, tier=$($QA.Tier)"

$EC = Join-Path $QA.Work "economics"
if (Test-Path $EC) { Remove-Item -Recurse -Force $EC }
New-Item -ItemType Directory $EC -Force | Out-Null

# ---- families: fixture chains (v1..vN of the SAME asset) ----
$families = [ordered]@{
  jpg         = @(1..5 | ForEach-Object { Join-Path $QA.Fixtures "chains\photo_v$_.jpg" })
  png         = @(1..5 | ForEach-Object { Join-Path $QA.Fixtures "chains\render_v$_.png" })
  svg         = @(1..5 | ForEach-Object { Join-Path $QA.Fixtures "chains\map_v$_.svg" })
  wav         = @(1..5 | ForEach-Object { Join-Path $QA.Fixtures "chains\aria_v$_.wav" })
  flac        = @(1..5 | ForEach-Object { Join-Path $QA.Fixtures "chains\aria_v$_.flac" })
  glb         = @(1..3 | ForEach-Object { Join-Path $QA.Fixtures "chains\car_v$_.glb" })
  safetensors = @(1..5 | ForEach-Object { Join-Path $QA.Fixtures "ml\model_v$_.safetensors" })
  npz         = @(1..3 | ForEach-Object { Join-Path $QA.Fixtures "ml\checkpoint_v$_.npz" })
  parquet     = @(1..3 | ForEach-Object { Join-Path $QA.Fixtures "ml\data_v$_.parquet" })
  onnx        = @(1..2 | ForEach-Object { Join-Path $QA.Fixtures "ml\model_v$_.onnx" })
}

# vfx\ subdir: auto-discover <base>_vN.<ext> groups instead of hardcoding names,
# since the fixture-generation agent owns exact filenames there.
$vfxDir = Join-Path $QA.Fixtures "vfx"
if (Test-Path $vfxDir) {
  $vfxFiles = Get-ChildItem $vfxDir -File -EA SilentlyContinue
  $groups = $vfxFiles | Where-Object { $_.BaseName -match '_v(\d+)$' } | Group-Object { $_.BaseName -replace '_v\d+$', '' }
  foreach ($g in $groups) {
    $ordered = $g.Group | Sort-Object { [int]([regex]::Match($_.BaseName, '_v(\d+)$').Groups[1].Value) }
    $families["vfx-$($g.Name)"] = @($ordered | ForEach-Object { $_.FullName })
  }
}

# ---- real-file pseudo-chains: distinct real assets of the same format treated as v1..vN ----
$psdFiles = Get-ChildItem (Join-Path $QA.TestFiles "psd") -Filter "*.psd" -File -EA SilentlyContinue | Sort-Object Name
if ($psdFiles) { $families["psd"] = @($psdFiles | ForEach-Object { $_.FullName }) }
$aiFiles = Get-ChildItem $QA.TestFiles -Filter "*.ai" -File -EA SilentlyContinue | Sort-Object Name
if ($aiFiles) { $families["ai"] = @($aiFiles | ForEach-Object { $_.FullName }) }
$videoFiles = Get-ChildItem (Join-Path $QA.TestFiles "video-variants") -File -EA SilentlyContinue | Sort-Object Name
if ($videoFiles) { $families["video-variants"] = @($videoFiles | ForEach-Object { $_.FullName }) }

# ---- anchor gates: v11 measured floors minus 5pt tolerance, keyed on latest version's savedPct ----
$anchors = @{ wav = 94.0; glb = 95.0; safetensors = 44.0 }  # safetensors rebased 48->44 on 2026-07-16: prior 48 was calibrated on lucky RANDOM-seed draws (48.9-50.9); under the pinned seed above the deterministic value is 45.5 (I11 RCA). wav pinned-seed value: 95.3.

$gateFailCount = 0
foreach ($fam in $families.Keys) {
  $files = @($families[$fam] | Where-Object { Test-Path $_ })
  $files = @(Select-TierFiles $files)
  if ($files.Count -lt 2) {
    Write-QaRow $OUT $HEADER @($fam, "SKIP", 0, 0, 0, 0)
    Write-QaLog $Phase "SKIP family $fam (fewer than 2 usable fixtures)"
    continue
  }

  $rp = Join-Path $EC $fam
  $init = Invoke-MG $null @("init", $rp) $Phase
  if ($init.Exit -ne 0) { Write-QaLog $Phase "init failed for $fam : $($init.Out)"; continue }

  $ext = [IO.Path]::GetExtension($files[0])
  $prev = 0.0
  $lastSaved = $null
  $i = 0
  foreach ($f in $files) {
    $i++
    Copy-Item $f (Join-Path $rp "asset$ext") -Force
    Invoke-MG $rp @("add", "asset$ext") $Phase | Out-Null
    $c = Invoke-MG $rp @("commit", "-m", "v$i") $Phase
    $odb = Get-DirMB (Join-Path $rp ".mediagit")
    $fmb = [math]::Round((Get-Item $f).Length / 1MB, 2)
    $growth = [math]::Round($odb - $prev, 2)
    $saved = if ($fmb -gt 0) { [math]::Round((1 - $growth / $fmb) * 100, 1) } else { 0 }
    Write-QaRow $OUT $HEADER @($fam, "v$i", $fmb, $growth, $saved, $c.Sec)
    $prev = $odb
    $lastSaved = $saved
  }

  if ($anchors.ContainsKey($fam)) {
    $pass = $lastSaved -ge $anchors[$fam]
    Write-QaGate $Phase "anchor-$fam" $pass "savedPct=$lastSaved threshold=$($anchors[$fam])"
    if (-not $pass) { $gateFailCount++ }
  }

  # repack + fsck invariant
  Invoke-MG $rp @("gc", "--repack", "-y") $Phase | Out-Null
  $fsck = Invoke-MG $rp @("fsck", "--full") $Phase
  $fsckClean = ($fsck.Exit -eq 0) -and ($fsck.Out -notmatch "corrupt|missing|failed")
  Write-QaGate $Phase "fsck-clean-$fam" $fsckClean "exit=$($fsck.Exit)"
  if (-not $fsckClean) { $gateFailCount++ }

  # stats --json totals vs measured ODB dir size, within 2%
  $statsRes = Invoke-MG $rp @("stats", "--json") $Phase
  $odbActualMB = Get-DirMB (Join-Path $rp ".mediagit")
  try {
    $statsJson = $statsRes.Out | ConvertFrom-Json
    $reportedMB = $null
    if ($statsJson.storage -and $statsJson.storage.PSObject.Properties.Name -contains "total_bytes") {
      # `stats --json` shape: { storage: { total_bytes, loose_bytes, pack_bytes, ... }, ... } - confirmed via live probe.
      $reportedMB = [math]::Round($statsJson.storage.total_bytes / 1MB, 2)
    } else {
      foreach ($k in @("storage_bytes", "total_bytes", "odb_bytes", "storageBytes")) {
        if ($statsJson.PSObject.Properties.Name -contains $k) { $reportedMB = [math]::Round($statsJson.$k / 1MB, 2); break }
      }
    }
  } catch {
    $reportedMB = $null
  }
  if ($null -ne $reportedMB -and $odbActualMB -gt 0) {
    $diffPct = [math]::Abs($reportedMB - $odbActualMB) / $odbActualMB * 100
    $statsPass = ($diffPct -le 2.0) -or ([math]::Abs($reportedMB - $odbActualMB) -le 0.05)  # absolute floor: pct is meaningless on ~0.01MB deltas (svg)
    Write-QaGate $Phase "stats-vs-diskMB-$fam" $statsPass "reported=$reportedMB actual=$odbActualMB diffPct=$([math]::Round($diffPct,2))"
    if (-not $statsPass) { $gateFailCount++ }
  } else {
    Write-QaLog $Phase "stats-vs-diskMB-$fam skipped: could not locate a storage-bytes field in stats --json"
  }
}

# ---- existing dedup regression gate (unchanged script, invoked as-is) ----
$compareScript = Join-Path $QA.RepoRoot "dev-tests\compare_dedup.ps1"
$baseline = Join-Path $QA.RepoRoot "dev-tests\dedup-baseline.json"
$dedupExe = Join-Path $QA.RepoRoot "target\release\examples\dedup_report.exe"
if ((Test-Path $compareScript) -and (Test-Path $baseline) -and (Test-Path $dedupExe)) {
  $currentJson = Join-Path $QA.Logs "dedup-current.json"
  # dedup_report.exe prints JSON to stdout and a PEAK_RSS_MB diagnostic line to stderr - drop stderr so it can't corrupt the JSON.
  # dedup_report has no repo config (seed 0 = deterministic) and its baseline
  # was locked under that regime - shield it from this phase's pinned seed.
  $savedSeed = $env:MEDIAGIT_CDC_SEED
  Remove-Item Env:MEDIAGIT_CDC_SEED -ErrorAction SilentlyContinue
  $reportOut = & $dedupExe 2>$null
  $env:MEDIAGIT_CDC_SEED = $savedSeed
  $reportOut | Set-Content $currentJson
  $global:LASTEXITCODE = 0
  & powershell -NoProfile -File $compareScript -Baseline $baseline -Current $currentJson 2>&1 | Add-Content (Join-Path $QA.Logs "$Phase-cmds.log")
  $dedupPass = ($LASTEXITCODE -eq 0)
  Write-QaGate $Phase "compare-dedup" $dedupPass "exit=$LASTEXITCODE"
  if (-not $dedupPass) { $gateFailCount++ }
} else {
  Write-QaLog $Phase "compare-dedup skipped: prebuilt dedup_report.exe not found at $dedupExe (build with: cargo build --release -p mediagit-versioning --example dedup_report)"
}

Write-QaLog $Phase "done: families=$($families.Count) gateFailures=$gateFailCount"
if ($gateFailCount -eq 0) { exit 0 } else { exit 1 }
