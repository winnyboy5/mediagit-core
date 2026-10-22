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
$HEADER = @("family", "version", "fileMB", "odbGrowthMB", "savedPct", "addSec", "commitSec")

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
$anchors = @{ wav = 94.0; glb = 95.0; safetensors = 44.0; psd = 62.0; npz = 71.0; ai = 21.0 }  # safetensors rebased 48->44 on 2026-07-16: prior 48 was calibrated on lucky RANDOM-seed draws (48.9-50.9); under the pinned seed above the deterministic value is 45.5 (I11 RCA). wav pinned-seed value: 95.3.
# psd/npz/ai added 2026-08-19. Until then 10 of the 13 families had NO savings
# anchor: psd, npz and ai could each have collapsed to zero savings and only the
# accounting-accuracy and fsck gates would have fired - neither of which measures
# savings at all. Savings is the product's differentiator, so "compression died"
# was the one regression this phase could not see.
#
# Same convention as the three above: measured floor minus a 5pt tolerance, keyed
# on the LAST version's savedPct. Calibrated on two complete campaigns rather than
# one, so the tolerance covers observed spread and not just a single draw:
#   psd  v3 = 67.2 (20260818-gagate8) / 67.1 (20260819-gagate13) -> 62.0
#   npz  v3 = 76.7 / 76.7                                        -> 71.0
#   ai   v3 = 26.3 / 26.3                                        -> 21.0
# npz and ai reproduce exactly; psd is the only one that moves at all (0.1pt on
# v3, 0.8pt on v2) because it is real-file, not seeded synthetic.
#
# flac is deliberately still unanchored: its LAST version measures 0% savings in
# both runs (savings live in v3/v4), so an anchor keyed on the last version would
# have to be 0 - a gate that cannot fail. Anchoring it needs a per-version key,
# which is a bigger change than this one.

# Smallest ODB size, in MB, whose stats-vs-disk comparison may fail the gate.
#
# Keyed on the INPUT size (actual on-disk ODB), not on diffPct - same lesson as
# 08_perf's $MIN_GATED_SIZE_MB: a floor keyed on the measurement is circular. Both
# reportedMB and actual are independently rounded to 2 decimals ([math]::Round .. 2),
# a +/-0.005MB resolution each; below 1MB that rounding alone can swing diffPct past
# the 2% threshold (0.02 vs 0.03 MB -> 33%) even when nothing is actually wrong. At
# 1MB the same rounding noise caps out at ~1%, safely under the 2% gate, while a real
# accounting error still reads as a real percentage. Sub-floor records are still
# logged - just not able to fail.
#
# What this floor COSTS, stated so nobody assumes it is free: on the 20260729-202252
# corpus it ungates svg (0.03MB, the bug) and also jpg (0.63MB, which was fine). jpg's
# own noise band is ~1.6%, close enough to the 2% threshold that gating it would mostly
# be gating rounding. Every ungated family still prints its numbers each run, so the
# cost stays visible instead of becoming silent coverage loss.
$MIN_GATED_ODB_MB = 1.0
$statsRecords = @()

$gateFailCount = 0
foreach ($fam in $families.Keys) {
  $files = @($families[$fam] | Where-Object { Test-Path $_ })
  $files = @(Select-TierFiles $files)
  if ($files.Count -lt 2) {
    Write-QaRow $OUT $HEADER @($fam, "SKIP", 0, 0, 0, 0, 0)
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
    # `add` is where chunking/hashing/compression actually happen, so its wall time is
    # the number the addSec column and the report both claim to show. This used to
    # discard the add result and record the COMMIT time under the addSec heading -
    # a near-zero figure that made the expensive half of the pipeline look free.
    $a = Invoke-MG $rp @("add", "asset$ext") $Phase
    $c = Invoke-MG $rp @("commit", "-m", "v$i") $Phase
    $odb = Get-DirMB (Join-Path $rp ".mediagit")
    $fmb = [math]::Round((Get-Item $f).Length / 1MB, 2)
    $growth = [math]::Round($odb - $prev, 2)
    $saved = if ($fmb -gt 0) { [math]::Round((1 - $growth / $fmb) * 100, 1) } else { 0 }
    Write-QaRow $OUT $HEADER @($fam, "v$i", $fmb, $growth, $saved, $a.Sec, $c.Sec)
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
    $gatable = $odbActualMB -ge $MIN_GATED_ODB_MB
    $detail = "reported=$reportedMB actual=$odbActualMB diffPct=$([math]::Round($diffPct,2))"
    if ($gatable) {
      # Per-family gate retained deliberately: the aggregate below can only say
      # "failures=N", and a failure you cannot attribute to a family and a
      # magnitude is not actionable. These numbers ARE the decision inputs.
      $famPass = ($diffPct -le 2.0)
      Write-QaGate $Phase "stats-vs-diskMB-$fam" $famPass $detail
      $statsRecords += [pscustomobject]@{ fam = $fam; pass = $famPass }
    } else {
      # Too small to support a percentage claim -> report, but do not gate. See $MIN_GATED_ODB_MB.
      Write-QaLog $Phase "stats-vs-diskMB-$fam ungated, below floor: $detail (min-odb-size=${MIN_GATED_ODB_MB}MB)"
      $statsRecords += [pscustomobject]@{ fam = $fam; pass = $null }
    }
  } else {
    Write-QaLog $Phase "stats-vs-diskMB-$fam skipped: could not locate a storage-bytes field in stats --json"
  }
}

# Single aggregate gate across all families, mirroring 08_perf's baseline-regression
# gate shape: gated=N/M is always printed so a floor set above every family reads as
# a visible ratio, not silence. A gate that measured nothing must FAIL, not pass
# vacuously - see $MIN_GATED_ODB_MB comment above and 08_perf's identical rule.
$statsGated = @($statsRecords | Where-Object { $null -ne $_.pass })
if ($statsGated.Count -eq 0) {
  Write-QaGate $Phase "stats-vs-diskMB" $false `
    "gated=0/$($statsRecords.Count) - the gate measured nothing (min-odb-size=${MIN_GATED_ODB_MB}MB)"
  $gateFailCount++
} else {
  $statsFailed = @($statsGated | Where-Object { -not $_.pass })
  $statsPassOverall = $statsFailed.Count -eq 0
  Write-QaGate $Phase "stats-vs-diskMB" $statsPassOverall `
    "gated=$($statsGated.Count)/$($statsRecords.Count) failures=$($statsFailed.Count) threshold=2% min-odb-size=${MIN_GATED_ODB_MB}MB"
  if (-not $statsPassOverall) { $gateFailCount++ }
}

# ---- container-savings gates (stream-boundary cuts, .ai zstd) ----
#
# MEDIAGIT_STREAM_CUTS (default ON for pdf/ai/fig/sketch/xd/idml; =0 turns it off):
# the product claim is that cutting at stream boundaries never COSTS storage and
# never changes content. Checked on a real .fig chain (default vs knob off) and on
# PSD, which stream cuts do not touch, so its store must not move. Fixtures:
# test-files\figma\UI-Kit-Italia (1..3).fig, test-files\psd (smallest psd + an edited copy),
# the smallest top-level test-files\*.ai >= 1MB (test-label-org.ai). The CDC seed is
# pinned above, so default vs off differs only by the knob.
function Get-QaDirBytes([string]$Path) {
  [long](Get-ChildItem $Path -Recurse -File -EA SilentlyContinue | Measure-Object Length -Sum).Sum
}

function Edit-QaBytes([string]$Path, [int]$Seed, [int]$SliceLen = 4096) {
  $bytes = [IO.File]::ReadAllBytes($Path)
  if ($bytes.Length -eq 0) { return }
  $rnd = New-Object Random($Seed)
  $len = [Math]::Min($SliceLen, $bytes.Length)
  $start = if ($bytes.Length -gt $len) { [int](($bytes.Length - $len) / 2) } else { 0 }
  for ($i = 0; $i -lt $len; $i++) { $bytes[$start + $i] = [byte]$rnd.Next(0, 256) }
  [IO.File]::WriteAllBytes($Path, $bytes)
}

# Build a v1..vN chain of $Files in a fresh repo. $StreamCuts "0" sets the off-switch,
# "" leaves the knob unset (product default). Returns Repo, Ext, Oids, Bytes (store size).
function New-QaChainRepo([string]$Name, [string[]]$Files, [string]$StreamCuts) {
  $rp = Join-Path $EC $Name
  $init = Invoke-MG $null @("init", $rp) $Phase
  if ($init.Exit -ne 0) { throw "init failed for $Name : $($init.Out)" }
  $ext = [IO.Path]::GetExtension($Files[0])
  $oids = @()
  $saved = $env:MEDIAGIT_STREAM_CUTS
  try {
    if ($StreamCuts) { $env:MEDIAGIT_STREAM_CUTS = $StreamCuts } else { Remove-Item Env:\MEDIAGIT_STREAM_CUTS -EA SilentlyContinue }
    $i = 0
    foreach ($f in $Files) {
      $i++
      Copy-Item $f (Join-Path $rp "asset$ext") -Force
      Invoke-MG $rp @("add", "asset$ext") $Phase -TimeoutSec 3600 | Out-Null
      $c = Invoke-MG $rp @("commit", "-m", "v$i") $Phase -TimeoutSec 3600
      $oids += $(if ($c.Out -match "Created commit ([0-9a-fA-F]{8,})") { $Matches[1] } else { $null })
    }
  } finally {
    if ($null -ne $saved) { $env:MEDIAGIT_STREAM_CUTS = $saved } else { Remove-Item Env:\MEDIAGIT_STREAM_CUTS -EA SilentlyContinue }
  }
  return @{ Repo = $rp; Ext = $ext; Oids = $oids; Bytes = (Get-QaDirBytes (Join-Path $rp ".mediagit")) }
}

# reset --hard to each oid and compare the materialized asset to the source file.
function Test-QaChainExact($Chain, [string[]]$Sources) {
  $bad = @()
  $asset = Join-Path $Chain.Repo ("asset" + $Chain.Ext)
  for ($v = 0; $v -lt $Sources.Count; $v++) {
    if (-not $Chain.Oids[$v]) { $bad += "v$($v + 1):no-oid"; continue }
    # Delete first: checkout skips a file that already matches, so without this the
    # last (or only) version would be hashed against itself and never read from the store.
    Remove-Item -LiteralPath $asset -Force -EA SilentlyContinue
    $rs = Invoke-MG $Chain.Repo @("reset", "--hard", $Chain.Oids[$v]) $Phase -TimeoutSec 3600
    $ok = ($rs.Exit -eq 0) -and (Test-Path $asset) -and ((Get-QaHash $asset) -eq (Get-QaHash $Sources[$v]))
    if (-not $ok) { $bad += "v$($v + 1):exit=$($rs.Exit)" }
  }
  return $bad
}

$figFiles = @(Select-TierFiles @(Get-ChildItem (Join-Path $QA.TestFiles "figma") -Filter "UI-Kit-Italia (*).fig" -File -EA SilentlyContinue |
  Sort-Object Name | Select-Object -First 3 | ForEach-Object { $_.FullName }))
if ($figFiles.Count -lt 3) {
  foreach ($g in @("stream-cuts-fig-checkout-exact", "stream-cuts-fig-store-not-larger", "stream-cuts-fig-fsck")) {
    Write-QaGate $Phase $g "SKIP" "fewer than 3 UI-Kit-Italia (N).fig fixtures within tier under test-files\figma"
  }
} else {
  $figOn  = New-QaChainRepo "streamcuts-fig-on"  $figFiles ""
  $figOff = New-QaChainRepo "streamcuts-fig-off" $figFiles "0"
  $bad = @(Test-QaChainExact $figOn $figFiles)
  Write-QaGate $Phase "stream-cuts-fig-checkout-exact" ($bad.Count -eq 0) ("versions=3 bad=" + ($bad -join ","))
  if ($bad.Count -gt 0) { $gateFailCount++ }
  # Strictly smaller (measured -19% on 6 versions): equal would also mean the knob never
  # reached the child process or the cuts did nothing.
  $le = $figOn.Bytes -lt $figOff.Bytes
  Write-QaGate $Phase "stream-cuts-fig-store-not-larger" $le "default=$($figOn.Bytes) off=$($figOff.Bytes) (default must be < off)"
  if (-not $le) { $gateFailCount++ }
  $ff = Invoke-MG $figOn.Repo @("fsck", "--full") $Phase -TimeoutSec 3600
  $ffOk = ($ff.Exit -eq 0) -and ($ff.Out -notmatch "corrupt|missing|failed")
  Write-QaGate $Phase "stream-cuts-fig-fsck" $ffOk "exit=$($ff.Exit)"
  if (-not $ffOk) { $gateFailCount++ }
}

# PSD is not a stream-cut format: v2 is an edited copy of v1, so delta still runs, and
# the store must be the same with the knob on or off (0.5% allowance for commit-object
# timestamps; a real regression is whole percents).
$psdSrc = Get-ChildItem (Join-Path $QA.TestFiles "psd") -Filter *.psd -File -EA SilentlyContinue |
  Sort-Object Length | Select-Object -First 1
if (-not $psdSrc -or $psdSrc.Length -gt ($QA.MaxFixtureMB * 1MB)) {
  Write-QaGate $Phase "stream-cuts-psd-unchanged" "SKIP" "no psd fixture within tier under test-files\psd"
} else {
  $psdV2 = Join-Path $EC "streamcuts-psd-v2.psd"
  Copy-Item $psdSrc.FullName $psdV2 -Force
  Edit-QaBytes $psdV2 9101
  $psdChain = @($psdSrc.FullName, $psdV2)
  $psdOn  = New-QaChainRepo "streamcuts-psd-on"  $psdChain ""
  $psdOff = New-QaChainRepo "streamcuts-psd-off" $psdChain "0"
  $diffPct = if ($psdOff.Bytes -gt 0) { [math]::Abs($psdOn.Bytes - $psdOff.Bytes) / $psdOff.Bytes * 100 } else { 100 }
  $same = $diffPct -le 0.5
  Write-QaGate $Phase "stream-cuts-psd-unchanged" $same ("default=$($psdOn.Bytes) off=$($psdOff.Bytes) diffPct=$([math]::Round($diffPct, 3)) allowance=0.5%")
  if (-not $same) { $gateFailCount++ }
}

# .ai is compressed with zstd instead of Store: a lone .ai must occupy less than its
# raw bytes and still round-trip byte-exact. Single file, so no dedup can explain it.
$aiSrc = Get-ChildItem $QA.TestFiles -Filter *.ai -File -EA SilentlyContinue |
  Where-Object { $_.Length -ge 1MB -and $_.Length -le ($QA.MaxFixtureMB * 1MB) } |
  Sort-Object Length | Select-Object -First 1
if (-not $aiSrc) {
  foreach ($g in @("ai-zstd-store-smaller-than-raw", "ai-zstd-roundtrip")) {
    Write-QaGate $Phase $g "SKIP" "no top-level test-files\*.ai between 1MB and the tier cap"
  }
} else {
  $aiChain = New-QaChainRepo "ai-zstd" @($aiSrc.FullName) ""
  $smaller = $aiChain.Bytes -lt $aiSrc.Length
  Write-QaGate $Phase "ai-zstd-store-smaller-than-raw" $smaller "file=$($aiSrc.Name) raw=$($aiSrc.Length) stored=$($aiChain.Bytes)"
  if (-not $smaller) { $gateFailCount++ }
  $badAi = @(Test-QaChainExact $aiChain @($aiSrc.FullName))
  Write-QaGate $Phase "ai-zstd-roundtrip" ($badAi.Count -eq 0) ("file=$($aiSrc.Name) bad=" + ($badAi -join ","))
  if ($badAi.Count -gt 0) { $gateFailCount++ }
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
# Teardown: reclaim this phase's own work/ scratch so a long campaign cannot run the
# volume out of space. work/ ONLY - logs/ and fixtures-synthetic/ are never touched.
Invoke-QaTeardown $Phase @("economics*")

Exit-QaPhase $Phase
