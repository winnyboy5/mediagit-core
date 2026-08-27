# Persona: VFX artist (EXR sequences, codec variants, trims, sparse checkout, bisect).
# Scenarios V1-V5. ASCII-only, PS 5.1 compatible.
param([string]$Only)

. (Join-Path $PSScriptRoot "lib\common.ps1")

$Phase  = "persona_vfx"
$Tsv    = Join-Path $QA.Reports "scenario_vfx.tsv"
$Header = @("scenario", "step", "action", "expect", "exit", "pass", "sec", "detail")
$script:AllPass = $true

# --- shared helpers (duplicated per-script by design; see qa-suite contract) ---

function Add-Row([string]$Scenario, [string]$StepName, [string]$Action, [string]$Expect, $Exit, [string]$Status, $Sec, [string]$Detail = "") {
  Write-QaRow $Tsv $Header @($Scenario, $StepName, $Action, $Expect, $Exit, $Status, $Sec, $Detail)
  if ($Status -eq "FAIL") { $script:AllPass = $false }
  Write-QaLog $Phase ("{0}/{1} {2} -> {3} (exit={4} sec={5})" -f $Scenario, $StepName, $Action, $Status, $Exit, $Sec)
}

function Test-FsckGate([string]$Scenario, [string]$Repo) {
  $r = Invoke-MG $Repo @("fsck") $Phase
  $status = if ($r.Exit -eq 0) { "PASS" } else { "FAIL" }
  $detail = $r.Out.Substring(0, [Math]::Min(200, $r.Out.Length))
  Add-Row $Scenario "fsck" "fsck" "exit 0" $r.Exit $status $r.Sec $detail
  Write-QaGate $Phase "$Scenario-fsck" ($status -eq "PASS") $detail
}

function Assert-HashEq([string]$Scenario, [string]$StepName, [string]$Expected, [string]$Actual, [string]$Detail = "") {
  $status = if ($Actual -and $Expected -and ($Actual -eq $Expected)) { "PASS" } else { "FAIL" }
  $exp8 = if ($Expected) { $Expected.Substring(0, [Math]::Min(12, $Expected.Length)) } else { "null" }
  $act8 = if ($Actual) { $Actual.Substring(0, [Math]::Min(12, $Actual.Length)) } else { "null" }
  Add-Row $Scenario $StepName "hash-parity" "match" 0 $status 0 "$Detail expected=$exp8 actual=$act8"
}

function Edit-BytesInPlace([string]$Path, [int]$Seed, [int]$SliceLen = 4096) {
  $bytes = [IO.File]::ReadAllBytes($Path)
  if ($bytes.Length -eq 0) { return }
  $rnd = New-Object Random($Seed)
  $len = [Math]::Min($SliceLen, $bytes.Length)
  $start = if ($bytes.Length -gt $len) { [int](($bytes.Length - $len) / 2) } else { 0 }
  for ($i = 0; $i -lt $len; $i++) { $bytes[$start + $i] = [byte]$rnd.Next(0, 256) }
  [IO.File]::WriteAllBytes($Path, $bytes)
}

function Get-CommitHash([string]$Out) {
  if ($Out -match "Created commit ([0-9a-fA-F]{8,})") { return $Matches[1] }
  return $null
}

function Invoke-Scenario([string]$Id, [scriptblock]$Body) {
  try { & $Body }
  catch { Add-Row $Id "exception" "run" "no exception" -1 "FAIL" 0 ($_.Exception.Message) }
}

# Find a file under video-variants by wildcard; $null if absent/over tier cap.
function Get-VideoVariant([string]$Pattern) {
  $vv = Join-Path $QA.TestFiles "video-variants"
  if (-not (Test-Path $vv)) { return $null }
  $f = Get-ChildItem $vv -Filter $Pattern -File -EA SilentlyContinue | Sort-Object Length | Select-Object -First 1
  if (-not $f) { return $null }
  $sel = Select-TierFiles @($f.FullName)
  if ($sel) { return $f.FullName }
  return $null
}

# ---------------------------------------------------------------------------
# V1: EXR sequence - 48 frames, then re-grade 12 of them; dedup evidence.
# ---------------------------------------------------------------------------
function Run-V1 {
  $id = "V1"
  $seqDir = Join-Path $QA.Fixtures "vfx\shot010"
  if (-not (Test-Path $seqDir)) { Add-Row $id "fixture" "vfx\shot010" "exr frames present" 0 "SKIP" 0 "fixtures-synthetic\vfx\shot010 missing"; return }
  $frames = Get-ChildItem $seqDir -Filter *.exr -File | Sort-Object Name | Select-Object -First 48
  if ($frames.Count -lt 48) { Add-Row $id "fixture" "vfx\shot010" "48 frames" 0 "SKIP" 0 "only $($frames.Count) frames"; return }

  $repo = New-SandboxRepo "vfx-V1" $Phase
  $exrDir = Join-Path $repo "exr"
  New-Item -ItemType Directory -Path $exrDir -Force | Out-Null
  foreach ($f in $frames) { Copy-Item -LiteralPath $f.FullName -Destination (Join-Path $exrDir $f.Name) -Force }

  Invoke-MG $repo @("add", "-A") $Phase | Out-Null
  $c1 = Invoke-MG $repo @("commit", "-m", "shot010 48 frames") $Phase
  $odb1 = Get-DirMB (Join-Path $repo ".mediagit")
  Add-Row $id "commit-1" "add -A + commit (48 frames)" "exit 0" $c1.Exit $(if ($c1.Exit -eq 0) { "PASS" } else { "FAIL" }) $c1.Sec "odb_mb=$odb1"

  # Modify 12 frames: prefer real regrade variants, else byte-edit.
  $regradeDir = Join-Path $QA.Fixtures "vfx\shot010_regrade"
  $modified = @()
  for ($i = 1; $i -le 12; $i++) {
    $name = "frame_{0:d4}.exr" -f $i
    $dst = Join-Path $exrDir $name
    $src = Join-Path $regradeDir $name
    if (Test-Path $src) { Copy-Item -LiteralPath $src -Destination $dst -Force }
    else { Edit-BytesInPlace $dst (500 + $i) }
    $modified += $dst
  }
  $modMB = [math]::Round((($modified | ForEach-Object { (Get-Item $_).Length } | Measure-Object -Sum).Sum) / 1MB, 2)

  Invoke-MG $repo @("add", "-A") $Phase | Out-Null
  $c2 = Invoke-MG $repo @("commit", "-m", "regrade 12 frames") $Phase
  $odb2 = Get-DirMB (Join-Path $repo ".mediagit")
  $growth = [math]::Round($odb2 - $odb1, 2)
  Add-Row $id "commit-2" "commit (12 regraded frames)" "exit 0" $c2.Exit $(if ($c2.Exit -eq 0) { "PASS" } else { "FAIL" }) $c2.Sec "odb_growth_mb=$growth modified_frames_mb=$modMB dedup_evidence=$(if ($modMB -gt 0) { [math]::Round(100 * (1 - $growth / $modMB), 1) } else { 0 })pct"

  Test-FsckGate $id $repo
}

# ---------------------------------------------------------------------------
# V2: codec variants - h264 baseline vs h265 branch; remux dedup in 2nd repo.
# ---------------------------------------------------------------------------
function Run-V2 {
  $id = "V2"
  $h264 = Get-VideoVariant "*h264*"
  $h265 = Get-VideoVariant "h265*"
  if (-not $h264 -or -not $h265) { Add-Row $id "fixture" "video-variants h264/h265" "both present" 0 "SKIP" 0 "h264=$h264 h265=$h265"; return }

  $repo = New-SandboxRepo "vfx-V2" $Phase
  $dest = Join-Path $repo "clip.video"
  Copy-Item -LiteralPath $h264 -Destination $dest -Force
  $h264Hash = Get-QaHash $dest
  Invoke-MG $repo @("add", $dest) $Phase | Out-Null
  Invoke-MG $repo @("commit", "-m", "h264 baseline") $Phase | Out-Null

  Invoke-MG $repo @("branch", "create", "codec-test") $Phase | Out-Null
  Invoke-MG $repo @("branch", "switch", "codec-test") $Phase | Out-Null
  Copy-Item -LiteralPath $h265 -Destination $dest -Force
  $h265Hash = Get-QaHash $dest
  Invoke-MG $repo @("add", $dest) $Phase | Out-Null
  $cc = Invoke-MG $repo @("commit", "-m", "h265 variant") $Phase
  Add-Row $id "h265-commit" "commit h265 on codec-test" "exit 0" $cc.Exit $(if ($cc.Exit -eq 0) { "PASS" } else { "FAIL" }) $cc.Sec ""

  $diff = Invoke-MG $repo @("diff", "main", "codec-test") $Phase
  Add-Row $id "diff" "diff main codec-test" "exit 0" $diff.Exit $(if ($diff.Exit -eq 0) { "PASS" } else { "FAIL" }) $diff.Sec ""

  Invoke-MG $repo @("branch", "switch", "main") $Phase | Out-Null
  # merge codec-test into main: ff or clean expected (main has no divergent commit) - record actual
  $mg = Invoke-MG $repo @("merge", "codec-test") $Phase
  $mergeKind = if ($mg.Exit -eq 0) { "clean" } else { "conflict" }
  # See the note in 03_persona_ml.ps1 M4: "record actual" is about which of two
  # legitimate outcomes occurred, not about accepting any exit code. A hardcoded
  # PASS here could not distinguish a conflict from a panic.
  Add-Row $id "merge" "merge codec-test into main" "exit 0 (clean) or 1 (conflict)" $mg.Exit `
    $(if ($mg.Exit -in 0, 1) { "PASS" } else { "FAIL" }) $mg.Sec "outcome=$mergeKind"
  if ($mg.Exit -ne 0) {
    # resolve by taking codec-test's version so fsck runs on a settled repo
    Copy-Item -LiteralPath $h265 -Destination $dest -Force
    Invoke-MG $repo @("add", $dest) $Phase | Out-Null
    $mc = Invoke-MG $repo @("merge", "--continue") $Phase
    Add-Row $id "merge-resolve" "merge --continue-merge codec-test" "exit 0" $mc.Exit $(if ($mc.Exit -eq 0) { "PASS" } else { "FAIL" }) $mc.Sec ""
  }
  Assert-HashEq $id "merged-is-h265" $h265Hash (Get-QaHash $dest) "post-merge clip"

  # both versions still retrievable via branch switch
  Invoke-MG $repo @("branch", "switch", "codec-test") $Phase | Out-Null
  Assert-HashEq $id "h265-retrievable" $h265Hash (Get-QaHash $dest) "codec-test branch"
  Invoke-MG $repo @("branch", "switch", "main") $Phase | Out-Null

  Test-FsckGate $id $repo

  # --- separate repo: original then remux variant, dedup evidence ---
  $orig  = Get-VideoVariant "remux-faststart*"
  $remux = Get-VideoVariant "remux-mkv*"
  if (-not $orig) { $orig = Get-VideoVariant "metadata-changed*" }
  if ($orig -and $remux) {
    $repo2 = New-SandboxRepo "vfx-V2-remux" $Phase
    $d2 = Join-Path $repo2 "master.video"
    Copy-Item -LiteralPath $orig -Destination $d2 -Force
    Invoke-MG $repo2 @("add", $d2) $Phase | Out-Null
    Invoke-MG $repo2 @("commit", "-m", "original") $Phase | Out-Null
    $odbA = Get-DirMB (Join-Path $repo2 ".mediagit")

    Copy-Item -LiteralPath $remux -Destination $d2 -Force
    Invoke-MG $repo2 @("add", $d2) $Phase | Out-Null
    $rc = Invoke-MG $repo2 @("commit", "-m", "remux variant") $Phase
    $odbB = Get-DirMB (Join-Path $repo2 ".mediagit")
    $remuxMB = [math]::Round((Get-Item $remux).Length / 1MB, 2)
    $growth = [math]::Round($odbB - $odbA, 2)
    $savedPct = if ($remuxMB -gt 0) { [math]::Round(100 * (1 - $growth / $remuxMB), 1) } else { 0 }
    Add-Row $id "remux-dedup" "commit remux over original" "informational" $rc.Exit $(if ($rc.Exit -eq 0) { "PASS" } else { "FAIL" }) $rc.Sec "odb_growth_mb=$growth remux_mb=$remuxMB saved_pct=$savedPct"
    Test-FsckGate $id $repo2
  } else {
    Add-Row $id "remux-dedup" "commit remux over original" "informational" 0 "SKIP" 0 "remux variants not found"
  }
}

# ---------------------------------------------------------------------------
# V3: trim iteration - original clip then trimmed variants as same path.
# ---------------------------------------------------------------------------
function Run-V3 {
  $id = "V3"
  $orig = Get-VideoVariant "bbb-5s-h264.mkv"
  if (-not $orig) { $orig = Get-VideoVariant "*h264*" }
  if (-not $orig) { Add-Row $id "fixture" "video-variants clip" "clip present" 0 "SKIP" 0 "no h264 clip"; return }

  $repo = New-SandboxRepo "vfx-V3" $Phase
  $dest = Join-Path $repo "edit.video"

  # v1: original
  Copy-Item -LiteralPath $orig -Destination $dest -Force
  Invoke-MG $repo @("add", $dest) $Phase | Out-Null
  $c1 = Invoke-MG $repo @("commit", "-m", "v1 original") $Phase
  $odbPrev = Get-DirMB (Join-Path $repo ".mediagit")
  Add-Row $id "commit-v1" "commit original" "exit 0" $c1.Exit $(if ($c1.Exit -eq 0) { "PASS" } else { "FAIL" }) $c1.Sec "odb_mb=$odbPrev"

  # v2: real trimmed variant if present, else 60 pct byte truncation of original
  $trimmed = Get-VideoVariant "trimmed*"
  if ($trimmed) { Copy-Item -LiteralPath $trimmed -Destination $dest -Force }
  else {
    $bytes = [IO.File]::ReadAllBytes($orig)
    [IO.File]::WriteAllBytes($dest, $bytes[0..([int]($bytes.Length * 0.6) - 1)])
  }
  Invoke-MG $repo @("add", $dest) $Phase | Out-Null
  $c2 = Invoke-MG $repo @("commit", "-m", "v2 trim") $Phase
  $odb2 = Get-DirMB (Join-Path $repo ".mediagit")
  Add-Row $id "commit-v2" "commit trim" "exit 0" $c2.Exit $(if ($c2.Exit -eq 0) { "PASS" } else { "FAIL" }) $c2.Sec "odb_growth_mb=$([math]::Round($odb2 - $odbPrev, 2)) src=$(if ($trimmed) { 'real-trimmed' } else { 'synthetic-truncate' })"

  # v3: synthetic tighter trim (first 80 pct of v2 bytes) - delta evidence
  $bytes = [IO.File]::ReadAllBytes($dest)
  if ($bytes.Length -gt 100) { [IO.File]::WriteAllBytes($dest, $bytes[0..([int]($bytes.Length * 0.8) - 1)]) }
  Invoke-MG $repo @("add", $dest) $Phase | Out-Null
  $c3 = Invoke-MG $repo @("commit", "-m", "v3 tighter trim") $Phase
  $odb3 = Get-DirMB (Join-Path $repo ".mediagit")
  Add-Row $id "commit-v3" "commit tighter trim" "exit 0" $c3.Exit $(if ($c3.Exit -eq 0) { "PASS" } else { "FAIL" }) $c3.Sec "odb_growth_mb=$([math]::Round($odb3 - $odb2, 2))"

  Test-FsckGate $id $repo
}

# ---------------------------------------------------------------------------
# V4: sparse checkout - exr\ kept, video\ dropped, then full restore parity.
# ---------------------------------------------------------------------------
function Run-V4 {
  $id = "V4"
  $seqDir = Join-Path $QA.Fixtures "vfx\shot010"
  $clip1 = Get-VideoVariant "*h264*"
  $clip2 = Get-VideoVariant "h265*"
  if (-not (Test-Path $seqDir) -or -not $clip1 -or -not $clip2) {
    Add-Row $id "fixture" "exr+clips" "present" 0 "SKIP" 0 "seqDir=$(Test-Path $seqDir) clip1=$clip1 clip2=$clip2"; return
  }
  $frames = Get-ChildItem $seqDir -Filter *.exr -File | Sort-Object Name | Select-Object -First 12
  if ($frames.Count -lt 12) { Add-Row $id "fixture" "12 exr frames" "present" 0 "SKIP" 0 "only $($frames.Count)"; return }

  $repo = New-SandboxRepo "vfx-V4" $Phase
  $exrDir = Join-Path $repo "exr"
  $vidDir = Join-Path $repo "video"
  New-Item -ItemType Directory -Path $exrDir, $vidDir -Force | Out-Null
  foreach ($f in $frames) { Copy-Item -LiteralPath $f.FullName -Destination (Join-Path $exrDir $f.Name) -Force }
  Copy-Item -LiteralPath $clip1 -Destination (Join-Path $vidDir "clip1.video") -Force
  Copy-Item -LiteralPath $clip2 -Destination (Join-Path $vidDir "clip2.video") -Force

  # record pre-hash of every file for full-restore parity
  $preHashes = @{}
  Get-ChildItem $repo -Recurse -File | Where-Object { $_.FullName -notmatch '\\\.mediagit\\' } | ForEach-Object {
    $rel = $_.FullName.Substring($repo.Length + 1)
    $preHashes[$rel] = Get-QaHash $_.FullName
  }

  Invoke-MG $repo @("add", "-A") $Phase | Out-Null
  $c1 = Invoke-MG $repo @("commit", "-m", "exr + video") $Phase
  Add-Row $id "commit" "commit exr+video" "exit 0" $c1.Exit $(if ($c1.Exit -eq 0) { "PASS" } else { "FAIL" }) $c1.Sec ""

  $sc = Invoke-MG $repo @("sparse-checkout", "set", "exr") $Phase
  Add-Row $id "sparse-set" "sparse-checkout set exr" "exit 0" $sc.Exit $(if ($sc.Exit -eq 0) { "PASS" } else { "FAIL" }) $sc.Sec ""

  $videoGone = -not (Test-Path (Join-Path $vidDir "clip1.video"))
  $exrThere  = Test-Path (Join-Path $exrDir $frames[0].Name)
  Add-Row $id "sparse-state" "verify tree" "video absent + exr present" 0 $(if ($videoGone -and $exrThere) { "PASS" } else { "FAIL" }) 0 "videoGone=$videoGone exrThere=$exrThere"

  $sd = Invoke-MG $repo @("sparse-checkout", "disable") $Phase
  Add-Row $id "sparse-disable" "sparse-checkout disable" "exit 0" $sd.Exit $(if ($sd.Exit -eq 0) { "PASS" } else { "FAIL" }) $sd.Sec ""

  $mismatch = 0
  foreach ($rel in $preHashes.Keys) {
    $p = Join-Path $repo $rel
    if (-not (Test-Path $p) -or ((Get-QaHash $p) -ne $preHashes[$rel])) { $mismatch++ }
  }
  Add-Row $id "restore-parity" "hash all files after disable" "0 mismatches" 0 $(if ($mismatch -eq 0) { "PASS" } else { "FAIL" }) 0 "files=$($preHashes.Count) mismatches=$mismatch"

  Test-FsckGate $id $repo
}

# ---------------------------------------------------------------------------
# V5: bisect - locate the commit that corrupted frame_001 (commit 7 of 10).
# ---------------------------------------------------------------------------
function Run-V5 {
  $id = "V5"
  $seqDir = Join-Path $QA.Fixtures "vfx\shot010"
  if (-not (Test-Path $seqDir)) { Add-Row $id "fixture" "vfx\shot010" "present" 0 "SKIP" 0 "missing"; return }
  $frames = Get-ChildItem $seqDir -Filter *.exr -File | Sort-Object Name | Select-Object -First 10
  if ($frames.Count -lt 10) { Add-Row $id "fixture" "10 exr frames" "present" 0 "SKIP" 0 "only $($frames.Count)"; return }

  $repo = New-SandboxRepo "vfx-V5" $Phase
  $goodFrame1Hash = $null
  $commitOids = @()
  for ($n = 1; $n -le 10; $n++) {
    $name = "frame_{0:d3}.exr" -f $n
    Copy-Item -LiteralPath $frames[$n - 1].FullName -Destination (Join-Path $repo $name) -Force
    if ($n -eq 1) { $goodFrame1Hash = Get-QaHash (Join-Path $repo "frame_001.exr") }
    if ($n -eq 7) {
      # corrupt frame_001 in this commit (byte-flipped copy)
      Edit-BytesInPlace (Join-Path $repo "frame_001.exr") 777
    }
    Invoke-MG $repo @("add", "-A") $Phase | Out-Null
    $c = Invoke-MG $repo @("commit", "-m", "add frame $n") $Phase
    $oid = Get-CommitHash $c.Out
    $commitOids += $oid
    if ($c.Exit -ne 0) { Add-Row $id "commit-$n" "commit frame $n" "exit 0" $c.Exit "FAIL" $c.Sec ""; return }
  }
  Add-Row $id "history" "10 commits, corruption at commit 7" "built" 0 "PASS" 0 "bad_oid=$($commitOids[6])"

  $bs = Invoke-MG $repo @("bisect", "start", $commitOids[9], $commitOids[0]) $Phase
  Add-Row $id "bisect-start" "bisect start <bad> <good>" "exit 0" $bs.Exit $(if ($bs.Exit -eq 0) { "PASS" } else { "FAIL" }) $bs.Sec ""

  # Drive good/bad marks by hashing frame_001 in the working tree.
  $firstBad = $null
  $lastOut = $bs.Out
  for ($step = 0; $step -lt 8; $step++) {
    if ($lastOut -match "([0-9a-fA-F]{7,})\s+is the first bad commit") { $firstBad = $Matches[1]; break }
    $cur = Get-QaHash (Join-Path $repo "frame_001.exr")
    $mark = if ($cur -eq $goodFrame1Hash) { "good" } else { "bad" }
    $r = Invoke-MG $repo @("bisect", $mark) $Phase
    Add-Row $id "bisect-step$step" "bisect $mark" "exit 0" $r.Exit $(if ($r.Exit -eq 0) { "PASS" } else { "FAIL" }) $r.Sec ""
    $lastOut = $r.Out
  }
  if (-not $firstBad -and $lastOut -match "([0-9a-fA-F]{7,})\s+is the first bad commit") { $firstBad = $Matches[1] }

  $expected = $commitOids[6]
  $found = $firstBad -and $expected -and ($firstBad.StartsWith($expected) -or $expected.StartsWith($firstBad))
  Add-Row $id "bisect-result" "first bad commit" "commit 7" 0 $(if ($found) { "PASS" } else { "FAIL" }) 0 "expected=$expected found=$firstBad"

  Invoke-MG $repo @("bisect", "reset") $Phase | Out-Null
  Test-FsckGate $id $repo
}

# ---------------------------------------------------------------------------
# Dispatch
# ---------------------------------------------------------------------------
$allScenarios = @("V1", "V2", "V3", "V4", "V5")
$toRun = if ($Only) { @($Only) } else { $allScenarios }

foreach ($sid in $toRun) {
  switch ($sid) {
    "V1" { Invoke-Scenario "V1" { Run-V1 } }
    "V2" { Invoke-Scenario "V2" { Run-V2 } }
    "V3" { Invoke-Scenario "V3" { Run-V3 } }
    "V4" { Invoke-Scenario "V4" { Run-V4 } }
    "V5" { Invoke-Scenario "V5" { Run-V5 } }
    default { Write-QaLog $Phase "Unknown scenario id: $sid" }
  }
}

# Teardown: reclaim this phase's own work/ scratch so a long campaign cannot run the
# volume out of space. work/ ONLY - logs/ and fixtures-synthetic/ are never touched.
Invoke-QaTeardown $Phase @("vfx-*")

Exit-QaPhase $Phase (-not $script:AllPass)
