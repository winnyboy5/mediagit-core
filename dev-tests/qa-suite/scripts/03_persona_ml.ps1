# Persona: ML engineer (checkpoint chains, datasets, staging flags, experiments, maintenance).
# Scenarios M1-M5. ASCII-only, PS 5.1 compatible.
param([string]$Only)

. (Join-Path $PSScriptRoot "lib\common.ps1")

$Phase  = "persona_ml"
$Tsv    = Join-Path $QA.Reports "scenario_ml.tsv"
$Header = @("scenario", "step", "action", "expect", "exit", "pass", "sec", "detail")
$script:AllPass = $true

# --- shared helpers (duplicated per-script by design; see qa-suite contract) ---

function Add-Row([string]$Scenario, [string]$StepName, [string]$Action, [string]$Expect, $Exit, [string]$Status, $Sec, [string]$Detail = "") {
  Write-QaRow $Tsv $Header @($Scenario, $StepName, $Action, $Expect, $Exit, $Status, $Sec, $Detail)
  if ($Status -eq "FAIL") { $script:AllPass = $false }
  Write-QaLog $Phase ("{0}/{1} {2} -> {3} (exit={4} sec={5})" -f $Scenario, $StepName, $Action, $Status, $Exit, $Sec)
}

function Test-FsckGate([string]$Scenario, [string]$Repo, [string[]]$ExtraArgs = @()) {
  $fsckArgs = @("fsck") + $ExtraArgs
  $r = Invoke-MG $Repo $fsckArgs $Phase
  $status = if ($r.Exit -eq 0) { "PASS" } else { "FAIL" }
  $detail = $r.Out.Substring(0, [Math]::Min(200, $r.Out.Length))
  Add-Row $Scenario "fsck" ($fsckArgs -join " ") "exit 0" $r.Exit $status $r.Sec $detail
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

function Invoke-Scenario([string]$Id, [scriptblock]$Body) {
  try { & $Body }
  catch { Add-Row $Id "exception" "run" "no exception" -1 "FAIL" 0 ($_.Exception.Message) }
}

# Build the M1 checkpoint chain into $Repo. Returns @{Ok; Odb0; OdbEnd; VersionsMB}
# or $null when fixtures are missing (a SKIP row is written).
function Build-CheckpointChain([string]$Id, [string]$Repo) {
  $mlDir = Join-Path $QA.Fixtures "ml"
  $versions = Get-ChildItem $mlDir -Filter "model_v*.safetensors" -File -EA SilentlyContinue | Sort-Object Name
  if ($versions.Count -lt 5) {
    Add-Row $Id "fixture" "ml\model_v1..5.safetensors" "5 versions" 0 "SKIP" 0 "found $($versions.Count)"
    return $null
  }
  $dest = Join-Path $Repo "model.safetensors"
  $odbPrev = 0.0; $odb0 = 0.0; $versionsMB = 0.0
  for ($v = 1; $v -le 5; $v++) {
    Copy-Item -LiteralPath $versions[$v - 1].FullName -Destination $dest -Force
    Invoke-MG $Repo @("add", $dest) $Phase | Out-Null
    $c = Invoke-MG $Repo @("commit", "-m", "checkpoint v$v") $Phase
    $odb = Get-DirMB (Join-Path $Repo ".mediagit")
    $growth = [math]::Round($odb - $odbPrev, 2)
    $vMB = [math]::Round($versions[$v - 1].Length / 1MB, 2)
    if ($v -eq 1) { $odb0 = $odb } else { $versionsMB += $vMB }
    $savedPct = if ($v -gt 1 -and $vMB -gt 0) { [math]::Round(100 * (1 - $growth / $vMB), 1) } else { "" }
    Add-Row $Id "commit-v$v" "commit model_v$v" "exit 0" $c.Exit $(if ($c.Exit -eq 0) { "PASS" } else { "FAIL" }) $c.Sec "odb_mb=$odb growth_mb=$growth version_mb=$vMB saved_pct=$savedPct"
    $odbPrev = $odb
  }
  return @{ Ok = $true; Odb0 = $odb0; OdbEnd = $odbPrev; VersionsMB = $versionsMB }
}

# ---------------------------------------------------------------------------
# M1: checkpoint chain dedup - informational rows, gate only if saved < 50 pct.
# ---------------------------------------------------------------------------
function Run-M1 {
  $id = "M1"
  $repo = New-SandboxRepo "ml-M1" $Phase
  $chain = Build-CheckpointChain $id $repo
  if (-not $chain) { return }

  $chainGrowth = [math]::Round($chain.OdbEnd - $chain.Odb0, 2)
  $totalSaved = if ($chain.VersionsMB -gt 0) { [math]::Round(100 * (1 - $chainGrowth / $chain.VersionsMB), 1) } else { 0 }
  $status = if ($totalSaved -ge 50) { "PASS" } else { "FAIL" }
  Add-Row $id "dedup-gate" "chain dedup summary" ">= 50 pct saved (expect ~74)" 0 $status 0 "saved_pct=$totalSaved growth_mb=$chainGrowth v2..v5_mb=$($chain.VersionsMB)"

  Test-FsckGate $id $repo
}

# ---------------------------------------------------------------------------
# M2: dataset - real parquet, then append-simulated growth.
# ---------------------------------------------------------------------------
function Run-M2 {
  $id = "M2"
  $parquets = Get-ChildItem $QA.TestFiles -Recurse -Filter *.parquet -File -EA SilentlyContinue | Sort-Object Length
  $parquet = $parquets | Where-Object { ($_.Length / 1MB) -le $QA.MaxFixtureMB } | Select-Object -First 1
  if (-not $parquet) { Add-Row $id "fixture" "*.parquet under test-files" "present within tier" 0 "SKIP" 0 "none within $($QA.MaxFixtureMB)MB"; return }

  $repo = New-SandboxRepo "ml-M2" $Phase
  $dest = Join-Path $repo "data.parquet"
  Copy-Item -LiteralPath $parquet.FullName -Destination $dest -Force
  Invoke-MG $repo @("add", $dest) $Phase | Out-Null
  $c1 = Invoke-MG $repo @("commit", "-m", "dataset v1") $Phase
  $odb1 = Get-DirMB (Join-Path $repo ".mediagit")
  Add-Row $id "commit-v1" "commit parquet ($([math]::Round($parquet.Length/1MB,1))MB)" "exit 0" $c1.Exit $(if ($c1.Exit -eq 0) { "PASS" } else { "FAIL" }) $c1.Sec "odb_mb=$odb1"

  # append-simulate: same bytes + 5MB deterministic-random tail
  $tail = New-Object byte[] (5MB)
  (New-Object Random(42)).NextBytes($tail)
  $fs = [IO.File]::Open($dest, [IO.FileMode]::Append)
  try { $fs.Write($tail, 0, $tail.Length) } finally { $fs.Close() }

  Invoke-MG $repo @("add", $dest) $Phase | Out-Null
  $c2 = Invoke-MG $repo @("commit", "-m", "dataset v2 (appended)") $Phase
  $odb2 = Get-DirMB (Join-Path $repo ".mediagit")
  $growth = [math]::Round($odb2 - $odb1, 2)
  Add-Row $id "commit-v2" "commit appended parquet" "exit 0; growth ~ tail size" $c2.Exit $(if ($c2.Exit -eq 0) { "PASS" } else { "FAIL" }) $c2.Sec "odb_growth_mb=$growth tail_mb=5"

  if ($QA.Tier -eq "STRESS") {
    $big = $parquets | Sort-Object Length -Descending | Select-Object -First 1
    if ($big -and $big.FullName -ne $parquet.FullName) {
      $repoB = New-SandboxRepo "ml-M2-stress" $Phase
      $destB = Join-Path $repoB "big.parquet"
      Copy-Item -LiteralPath $big.FullName -Destination $destB -Force
      Invoke-MG $repoB @("add", $destB) $Phase | Out-Null
      $cb = Invoke-MG $repoB @("commit", "-m", "big dataset") $Phase 3600
      Add-Row $id "stress-big" "commit $([math]::Round($big.Length/1GB,1))GB parquet" "exit 0" $cb.Exit $(if ($cb.Exit -eq 0) { "PASS" } else { "FAIL" }) $cb.Sec "odb_mb=$(Get-DirMB (Join-Path $repoB '.mediagit'))"
      Test-FsckGate $id $repoB
    }
  }

  Test-FsckGate $id $repo
}

# ---------------------------------------------------------------------------
# M3: staging flags - dry-run, -A, -u, --no-chunking vs default on an npz.
# ---------------------------------------------------------------------------
function Run-M3 {
  $id = "M3"
  $repo = New-SandboxRepo "ml-M3" $Phase

  # small CSV + notebook churn
  "epoch,loss`n1,0.9`n2,0.7" | Set-Content (Join-Path $repo "metrics.csv") -Encoding ASCII
  '{"cells":[],"nbformat":4}' | Set-Content (Join-Path $repo "train.ipynb") -Encoding ASCII

  $dr = Invoke-MG $repo @("add", "--dry-run", "-A") $Phase
  $st = Invoke-MG $repo @("status", "--porcelain") $Phase
  $stillUntracked = ($st.Out -match "\?\? metrics.csv") -and ($st.Out -match "\?\? train.ipynb")
  Add-Row $id "dry-run" "add --dry-run -A" "nothing actually staged" $dr.Exit $(if ($stillUntracked) { "PASS" } else { "FAIL" }) $dr.Sec "note: dry-run output claims 'Staged N file(s)' - misleading wording"

  Invoke-MG $repo @("add", "-A") $Phase | Out-Null
  $c1 = Invoke-MG $repo @("commit", "-m", "initial") $Phase
  Add-Row $id "commit" "add -A + commit" "exit 0" $c1.Exit $(if ($c1.Exit -eq 0) { "PASS" } else { "FAIL" }) $c1.Sec ""

  # modify both tracked files, add -u must stage them
  "epoch,loss`n1,0.9`n2,0.7`n3,0.5" | Set-Content (Join-Path $repo "metrics.csv") -Encoding ASCII
  '{"cells":[{"cell_type":"code"}],"nbformat":4}' | Set-Content (Join-Path $repo "train.ipynb") -Encoding ASCII
  $au = Invoke-MG $repo @("add", "-u") $Phase
  $c2 = Invoke-MG $repo @("commit", "-m", "churn") $Phase
  Add-Row $id "add-u" "add -u + commit" "exit 0" $c2.Exit $(if ($au.Exit -eq 0 -and $c2.Exit -eq 0) { "PASS" } else { "FAIL" }) $c2.Sec ""

  # npz: --no-chunking vs default in two repos
  $npz = Get-ChildItem (Join-Path $QA.Fixtures "ml") -Filter "checkpoint_v*.npz" -File -EA SilentlyContinue | Sort-Object Length -Descending | Select-Object -First 1
  if ($npz) {
    $sizes = @{}
    foreach ($mode in @("default", "no-chunking")) {
      $r2 = New-SandboxRepo "ml-M3-$mode" $Phase
      $d2 = Join-Path $r2 "checkpoint.npz"
      Copy-Item -LiteralPath $npz.FullName -Destination $d2 -Force
      $addArgs = if ($mode -eq "no-chunking") { @("add", "--no-chunking", $d2) } else { @("add", $d2) }
      Invoke-MG $r2 $addArgs $Phase | Out-Null
      Invoke-MG $r2 @("commit", "-m", "npz $mode") $Phase | Out-Null
      $sizes[$mode] = Get-DirMB (Join-Path $r2 ".mediagit")
      Test-FsckGate $id $r2
    }
    Add-Row $id "chunking-compare" "npz default vs --no-chunking" "informational" 0 "PASS" 0 "default_odb_mb=$($sizes['default']) nochunk_odb_mb=$($sizes['no-chunking']) npz_mb=$([math]::Round($npz.Length/1MB,2))"
  } else {
    Add-Row $id "chunking-compare" "npz default vs --no-chunking" "informational" 0 "SKIP" 0 "no checkpoint_v*.npz in fixtures ml"
  }

  Test-FsckGate $id $repo
}

# ---------------------------------------------------------------------------
# M4: experiments - 3 branches, merge one, tag, push to MinIO, clone parity.
# ---------------------------------------------------------------------------
function Run-M4 {
  $id = "M4"
  $mlDir = Join-Path $QA.Fixtures "ml"
  $base = Get-ChildItem $mlDir -Filter "model_v1.safetensors" -File -EA SilentlyContinue | Select-Object -First 1
  if (-not $base) { Add-Row $id "fixture" "ml\model_v1.safetensors" "present" 0 "SKIP" 0 "missing"; return }

  $repo = New-SandboxRepo "ml-M4" $Phase
  $dest = Join-Path $repo "model.safetensors"
  Copy-Item -LiteralPath $base.FullName -Destination $dest -Force
  Invoke-MG $repo @("add", $dest) $Phase | Out-Null
  Invoke-MG $repo @("commit", "-m", "base checkpoint") $Phase | Out-Null

  $expHashes = @{}
  foreach ($n in 1, 2, 3) {
    Invoke-MG $repo @("branch", "switch", "-c", "exp-$n") $Phase | Out-Null
    Edit-BytesInPlace $dest (1100 + $n)
    Invoke-MG $repo @("add", $dest) $Phase | Out-Null
    $c = Invoke-MG $repo @("commit", "-m", "exp-$n tweak") $Phase
    $expHashes["exp-$n"] = Get-QaHash $dest
    Add-Row $id "exp-$n" "branch + edit + commit" "exit 0" $c.Exit $(if ($c.Exit -eq 0) { "PASS" } else { "FAIL" }) $c.Sec ""
    Invoke-MG $repo @("branch", "switch", "main") $Phase | Out-Null
  }

  $mg = Invoke-MG $repo @("merge", "exp-2") $Phase
  if ($mg.Exit -ne 0) {
    # conflict (main did not advance, so ff/clean is expected - record either way)
    Add-Row $id "merge" "merge exp-2" "record actual" $mg.Exit "PASS" $mg.Sec "outcome=conflict; resolving with exp-2 bytes"
    $side = Join-Path $QA.Work "ml-M4-exp2.bin"
    Invoke-MG $repo @("branch", "switch", "exp-2") $Phase | Out-Null
    Copy-Item -LiteralPath $dest -Destination $side -Force
    Invoke-MG $repo @("branch", "switch", "main") $Phase | Out-Null
    Copy-Item -LiteralPath $side -Destination $dest -Force
    Invoke-MG $repo @("add", $dest) $Phase | Out-Null
    $mc = Invoke-MG $repo @("merge", "--continue") $Phase
    Add-Row $id "merge-resolve" "merge --continue-merge exp-2" "exit 0" $mc.Exit $(if ($mc.Exit -eq 0) { "PASS" } else { "FAIL" }) $mc.Sec ""
  } else {
    Add-Row $id "merge" "merge exp-2" "record actual" $mg.Exit "PASS" $mg.Sec "outcome=clean"
  }
  Assert-HashEq $id "main-is-exp2" $expHashes["exp-2"] (Get-QaHash $dest) "main after merge"

  $tc = Invoke-MG $repo @("tag", "create", "model-v1") $Phase
  Add-Row $id "tag" "tag create model-v1" "exit 0" $tc.Exit $(if ($tc.Exit -eq 0) { "PASS" } else { "FAIL" }) $tc.Sec ""

  $remoteLib = Join-Path $PSScriptRoot "lib\remote.ps1"
  $srv = $null
  if (Test-Path $remoteLib) {
    . $remoteLib
    try { $srv = Start-QaServer -Backend minio -Phase $Phase }
    catch { Add-Row $id "server" "Start-QaServer minio" "server up" 1 "SKIP" 0 ($_.Exception.Message) }
  } else {
    Add-Row $id "server" "Start-QaServer minio" "server up" 0 "SKIP" 0 "lib\remote.ps1 not present yet"
  }
  if ($srv) {
    try {
      Invoke-MG $repo @("remote", "add", "origin", $srv.Url) $Phase | Out-Null
      $push = Invoke-MG $repo @("push", "-u", "origin", "main", "--tags") $Phase
      Add-Row $id "push" "push -u origin main --tags" "exit 0" $push.Exit $(if ($push.Exit -eq 0) { "PASS" } else { "FAIL" }) $push.Sec ""

      $cloneDir = Join-Path $QA.Work "ml-M4-clone"
      if (Test-Path $cloneDir) { Remove-Item -Recurse -Force $cloneDir }
      $cl = Invoke-MG $null @("clone", $srv.Url, $cloneDir) $Phase
      Add-Row $id "clone" "clone (second machine)" "exit 0" $cl.Exit $(if ($cl.Exit -eq 0) { "PASS" } else { "FAIL" }) $cl.Sec ""

      if ($cl.Exit -eq 0) {
        $tl = Invoke-MG $cloneDir @("tag", "list") $Phase
        $tagThere = $tl.Out -match "model-v1"
        Add-Row $id "clone-tag" "tag list (clone)" "model-v1 present" $tl.Exit $(if ($tagThere) { "PASS" } else { "FAIL" }) $tl.Sec ""
        # every working-tree file hash parity
        $mismatch = 0; $checked = 0
        Get-ChildItem $repo -Recurse -File | Where-Object { $_.FullName -notmatch '\\\.mediagit\\' -and $_.FullName -notmatch '\\\.qa-' } | ForEach-Object {
          $rel = $_.FullName.Substring($repo.Length + 1)
          $p2 = Join-Path $cloneDir $rel
          $checked++
          if (-not (Test-Path $p2) -or ((Get-QaHash $p2) -ne (Get-QaHash $_.FullName))) { $mismatch++ }
        }
        Add-Row $id "clone-parity" "hash every file vs clone" "0 mismatches" 0 $(if ($mismatch -eq 0 -and $checked -gt 0) { "PASS" } else { "FAIL" }) 0 "files=$checked mismatches=$mismatch"
        Test-FsckGate $id $cloneDir
      }
    } finally {
      Stop-QaServer $srv
    }
  }

  Test-FsckGate $id $repo
}

# ---------------------------------------------------------------------------
# M5: maintenance - gc --repack, gc --aggressive, fsck --full, stats parity.
# ---------------------------------------------------------------------------
function Run-M5 {
  $id = "M5"
  $repo = New-SandboxRepo "ml-M5" $Phase
  $chain = Build-CheckpointChain $id $repo
  if (-not $chain) { return }

  $dest = Join-Path $repo "model.safetensors"
  $preHash = Get-QaHash $dest
  $statsBefore = Invoke-MG $repo @("stats", "--json") $Phase
  Add-Row $id "stats-before" "stats --json" "exit 0" $statsBefore.Exit $(if ($statsBefore.Exit -eq 0) { "PASS" } else { "FAIL" }) $statsBefore.Sec ""

  $gc1 = Invoke-MG $repo @("gc", "--repack", "-y") $Phase
  Add-Row $id "gc-repack" "gc --repack -y" "exit 0" $gc1.Exit $(if ($gc1.Exit -eq 0) { "PASS" } else { "FAIL" }) $gc1.Sec "odb_mb=$(Get-DirMB (Join-Path $repo '.mediagit'))"

  # `--aggressive` was never implemented - its own struct comment called it
  # "a no-op CLI flag" - and it now refuses instead of silently doing nothing
  # (UX-5). This step's purpose is "gc still works after the ML workload",
  # which plain gc covers; passing a flag that does nothing never tested
  # anything the bare command did not.
  $gc2 = Invoke-MG $repo @("gc", "-y") $Phase
  Add-Row $id "gc-after-workload" "gc -y" "exit 0" $gc2.Exit $(if ($gc2.Exit -eq 0) { "PASS" } else { "FAIL" }) $gc2.Sec "odb_mb=$(Get-DirMB (Join-Path $repo '.mediagit'))"

  Test-FsckGate $id $repo @("--full")

  $statsAfter = Invoke-MG $repo @("stats", "--json") $Phase
  Add-Row $id "stats-after" "stats --json" "exit 0" $statsAfter.Exit $(if ($statsAfter.Exit -eq 0) { "PASS" } else { "FAIL" }) $statsAfter.Sec ""

  # logical parity: file untouched by gc, and checkout still materializes it
  Assert-HashEq $id "worktree-after-gc" $preHash (Get-QaHash $dest) "model.safetensors after gc"
  Invoke-MG $repo @("branch", "switch", "-c", "post-gc-check") $Phase | Out-Null
  Invoke-MG $repo @("branch", "switch", "main") $Phase | Out-Null
  Remove-Item -Force $dest
  $rs = Invoke-MG $repo @("reset", "--hard", "HEAD") $Phase
  $restored = (Test-Path $dest) -and ((Get-QaHash $dest) -eq $preHash)
  Add-Row $id "checkout-after-gc" "delete file + reset --hard HEAD" "file rematerialized from ODB" $rs.Exit $(if ($restored) { "PASS" } else { "FAIL" }) $rs.Sec ""
}

# ---------------------------------------------------------------------------
# Dispatch
# ---------------------------------------------------------------------------
$allScenarios = @("M1", "M2", "M3", "M4", "M5")
$toRun = if ($Only) { @($Only) } else { $allScenarios }

foreach ($sid in $toRun) {
  switch ($sid) {
    "M1" { Invoke-Scenario "M1" { Run-M1 } }
    "M2" { Invoke-Scenario "M2" { Run-M2 } }
    "M3" { Invoke-Scenario "M3" { Run-M3 } }
    "M4" { Invoke-Scenario "M4" { Run-M4 } }
    "M5" { Invoke-Scenario "M5" { Run-M5 } }
    default { Write-QaLog $Phase "Unknown scenario id: $sid" }
  }
}

# Teardown: reclaim this phase's own work/ scratch so a long campaign cannot run the
# volume out of space. work/ ONLY - logs/ and fixtures-synthetic/ are never touched.
Invoke-QaTeardown $Phase @("ml-*")

Exit-QaPhase $Phase (-not $script:AllPass)
