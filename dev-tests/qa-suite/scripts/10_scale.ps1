# Phase 10 - scale & aggression suite. ASCII-only, PS 5.1 compatible.
# Opt-in: runs under `-Phases 10`, and is appended to the default set only when
# MG_QA_TIER=SCALE (run_all.ps1). Sizes/counts come from the SCALE knobs in config.ps1
# ($QA.Scale, .FileCount, .Concurrency, .ChurnCommits, .CloudMaxMB, .DiskBudgetGB, .RssCeilMB).
#
# Drills (gate = no silent corruption, plus the per-drill gate named below):
#   S1 sustained concurrency   - $QA.Concurrency clients clone the same remote at once; all
#                                converge to the source tree hash, all fsck clean, server survives.
#   S2 rapid churn/deep chains - $QA.ChurnCommits rapid commits; chunk-delta MaxDepth <= 10
#                                (MAX_DELTA_DEPTH), no cycles, repo STAYS PUSHABLE (regression guard).
#   S3 interleaved conflicts   - N concurrent merge/rebase/cherry-pick rewrites; each internally
#                                consistent (fsck clean, expected file present), no untouched-file drift.
#   S4 resource pressure       - multi-GB blob + many-files corpus; peak client/server RSS <= ceiling,
#                                no OOM, fsck clean. Disk-budget bounded, self-cleaning.
#   S5 scale throughput+dedup  - full payload push/clone per backend (cloud capped by CloudMaxMB);
#                                minio/local must meet the throughput SLO floor; dedup savedPct >= floor.
#
# Output: $QA.Logs\scale_results.tsv (drill, backend, metric, value, pass, detail)
# Teardown: post-phase (finally) purges this phase's work/ scratch unless MG_QA_KEEP_SCRATCH=1.

. (Join-Path $PSScriptRoot "lib\common.ps1")
. (Join-Path $PSScriptRoot "lib\remote.ps1")

$Phase = "10_scale"
$env:MEDIAGIT_AUTHOR_NAME = "QA-Suite"
$env:MEDIAGIT_AUTHOR_EMAIL = "qa-suite@mediagit.local"

$TSV = Join-Path $QA.Logs "scale_results.tsv"
$script:AllPass = $true
$script:Servers = @()   # Start-QaServer handles to stop at teardown

$MAX_DELTA_DEPTH = 10                      # crates/mediagit-versioning/src/odb/mod.rs
$PUSH_FLOOR_MBS  = [math]::Round(10240.0 / 720.0, 2)   # 10GB <=12min push SLO -> 14.22 MB/s
$PULL_FLOOR_MBS  = [math]::Round(10240.0 / 600.0, 2)   # 10GB <=10min pull SLO -> 17.07 MB/s
$DEDUP_FLOOR_PCT = 15.0                     # perturbed safetensors chain dedups well above this

function Rec([string]$Drill, [string]$Backend, [string]$Metric, $Value, $Pass, [string]$Detail) {
  Write-QaRow $TSV @("drill", "backend", "metric", "value", "pass", "detail") `
    @($Drill, $Backend, $Metric, $Value, $Pass, $Detail)
  $tag = if ("$Pass" -eq "SKIP") { "SKIP" } elseif ($Pass) { "PASS" } else { "FAIL" }
  Write-QaLog $Phase ("{0} [{1}] {2}={3} -> {4}  {5}" -f $Drill, $Backend, $Metric, $Value, $tag, $Detail)
  Write-QaGate $Phase "$Drill-$Metric" ($Pass -eq $true -or "$Pass" -eq "SKIP") $Detail
  if ($tag -eq "FAIL") { $script:AllPass = $false }
}

# Streaming multi-GB blob writer: fresh 1MB block per MB (not dedup-trivial), never holds the
# whole file in RAM - the single-array New-QaBinaryFixture would OOM the harness at GB scale.
function New-ScaleBlob([string]$Path, [int]$SizeMB, [int]$Seed) {
  $dir = Split-Path $Path -Parent
  if (-not (Test-Path $dir)) { New-Item -ItemType Directory -Path $dir -Force | Out-Null }
  $rng = New-Object System.Random($Seed)
  $buf = New-Object byte[] (1MB)
  $fs = [System.IO.File]::Open($Path, [System.IO.FileMode]::Create)
  try { for ($i = 0; $i -lt $SizeMB; $i++) { $rng.NextBytes($buf); $fs.Write($buf, 0, $buf.Length) } }
  finally { $fs.Close() }
}

function Test-QaFsckClean([string]$Repo) {
  $r = Invoke-MG $Repo @("fsck") $Phase
  return -not (($r.Out -match "(?i)corrupt|missing|error|failed") -or ($r.Exit -ne 0))
}

# Sorted "hash  relpath" lines for every non-.mediagit file - full-tree parity (as 06_remote).
function Get-QaTreeHashes([string]$Root) {
  $full = (Get-Item $Root).FullName
  Get-ChildItem $full -Recurse -File -ErrorAction SilentlyContinue |
    Where-Object { $_.FullName -notmatch '\\\.mediagit\\' } |
    ForEach-Object { "{0}  {1}" -f (Get-QaHash $_.FullName), $_.FullName.Substring($full.Length + 1) } |
    Sort-Object
}

# Scratch dir under a phase-owned prefix so teardown can glob-delete cleanly.
function New-ScaleDir([string]$Leaf) {
  $p = Join-Path $QA.Work "scale10-$Leaf"
  if (Test-Path $p) { Remove-Item -Recurse -Force $p -ErrorAction SilentlyContinue }
  New-Item -ItemType Directory -Path $p -Force | Out-Null
  return $p
}

$ManyFiles = Join-Path $QA.Fixtures "scale\manyfiles"

# Blob budget: keep payload + odb + a clone within the disk budget (~4x headroom).
$FullBlobMB = $QA.Scale * 1024
$BlobBudgetMB = [math]::Min($FullBlobMB, [int](($QA.DiskBudgetGB * 1024) / 4))
if ($BlobBudgetMB -lt $FullBlobMB) {
  Write-QaLog $Phase "blob payload capped $FullBlobMB MB -> $BlobBudgetMB MB by DiskBudgetGB=$($QA.DiskBudgetGB)"
}

Write-QaLog $Phase ("scale run: tier={0} scale={1} filecount={2} concurrency={3} churn={4} cloudMaxMB={5} blobBudgetMB={6}" -f `
    $QA.Tier, $QA.Scale, $QA.FileCount, $QA.Concurrency, $QA.ChurnCommits, $QA.CloudMaxMB, $BlobBudgetMB)

# Free-disk guard: refuse the size-heavy drills if the volume can't hold the budget.
$freeGB = Get-QaFreeDiskGB $QA.Work
$diskOk = ($freeGB -lt 0) -or ($freeGB -ge $QA.DiskBudgetGB)
if (-not $diskOk) {
  Write-QaLog $Phase "WARNING free disk ${freeGB}GB < budget $($QA.DiskBudgetGB)GB - size-heavy drills (S4/S5) will SKIP"
}

# ---------------------------------------------------------------------------
# S1: sustained concurrency - $QA.Concurrency clients clone the same remote at once.
# Scale-up of 07_abuse A3 (which is 2). All clones must converge to the source tree.
# ---------------------------------------------------------------------------
function Drill-S1-Concurrency {
  $drill = "S1-concurrency"
  $srv = $null
  try {
    # per-drill Phase suffix so each Start-QaServer gets a unique repo name; without it,
    # S1/S2/S5 share proj-<runid>-10_scale and collide on each other's MinIO bucket state
    # (07_abuse uses the same -A2/-A3 convention).
    try { $srv = Start-QaServer -Backend "minio" -Phase "$Phase-S1" } catch {
      if ("$_" -match "^SKIP:") { Rec $drill "minio" "clones-converge" "" "SKIP" "$_"; return }
      throw
    }
    $script:Servers += $srv

    # Source: many-files corpus (count pressure) + two modest blobs. Concurrency, not size.
    $src = New-ScaleDir "s1-src"
    Invoke-MG $null @("init", $src) $Phase | Out-Null
    if (Test-Path $ManyFiles) { Copy-Item $ManyFiles (Join-Path $src "manyfiles") -Recurse }
    New-ScaleBlob (Join-Path $src "blob_a.bin") 64 71001
    New-ScaleBlob (Join-Path $src "blob_b.bin") 64 71002
    Invoke-MG $src @("add", ".") $Phase -TimeoutSec 1800 | Out-Null
    Invoke-MG $src @("commit", "-m", "s1 payload") $Phase | Out-Null
    Invoke-MG $src @("remote", "add", "origin", $srv.Url) $Phase | Out-Null
    $r = Invoke-MG $src @("push", "origin") $Phase -TimeoutSec 3600
    if ($r.Exit -ne 0) { Rec $drill "minio" "clones-converge" 0 $false "push failed exit=$($r.Exit)"; return }
    $srcHashes = Get-QaTreeHashes $src

    $N = $QA.Concurrency
    $jobs = @()
    for ($i = 1; $i -le $N; $i++) {
      $dest = Join-Path $QA.Work "scale10-s1-clone-$i"
      if (Test-Path $dest) { Remove-Item -Recurse -Force $dest -ErrorAction SilentlyContinue }
      $jobs += Start-Job -ScriptBlock {
        param($m, $u, $d)
        $out = & $m clone $u $d 2>&1 | Out-String
        "$LASTEXITCODE"
      } -ArgumentList $QA.MG, $srv.Url, $dest
    }
    Wait-Job $jobs -Timeout 3600 | Out-Null
    $exitCodes = $jobs | ForEach-Object { ("" + (Receive-Job $_)).Trim() }
    $jobs | Remove-Job -Force -ErrorAction SilentlyContinue

    $converged = 0; $fsckClean = 0; $cloneOk = 0
    for ($i = 1; $i -le $N; $i++) {
      $dest = Join-Path $QA.Work "scale10-s1-clone-$i"
      if (-not (Test-Path $dest)) { continue }
      $cloneOk++
      if ((Compare-Object $srcHashes (Get-QaTreeHashes $dest) | Measure-Object).Count -eq 0) { $converged++ }
      if (Test-QaFsckClean $dest) { $fsckClean++ }
      Remove-Item -Recurse -Force $dest -ErrorAction SilentlyContinue
    }
    $srvAlive = -not $srv.Proc.HasExited
    $pass = ($cloneOk -eq $N) -and ($converged -eq $N) -and ($fsckClean -eq $N) -and $srvAlive
    Rec $drill "minio" "clones-converge" $converged $pass `
      "n=$N cloneOk=$cloneOk converged=$converged fsckClean=$fsckClean serverAlive=$srvAlive"
  } catch {
    Rec $drill "minio" "clones-converge" "" $false "unexpected error: $_"
  } finally {
    Stop-QaServer $srv
  }
}

# ---------------------------------------------------------------------------
# S2: rapid churn / deep chains. $QA.ChurnCommits rapid commits mutating one binary to
# force chunk-delta growth; the writer re-bases so MaxDepth must stay <= MAX_DELTA_DEPTH.
# Regression guard: an over-deep chain once made repos unpushable.
# ---------------------------------------------------------------------------
function Drill-S2-Churn {
  $drill = "S2-churn"
  $srv = $null
  try {
    $repo = New-ScaleDir "s2-repo"
    Invoke-MG $null @("init", $repo) $Phase | Out-Null
    $asset = Join-Path $repo "asset.bin"
    New-ScaleBlob $asset 32 72000
    Invoke-MG $repo @("add", "asset.bin") $Phase | Out-Null
    Invoke-MG $repo @("commit", "-m", "s2 base") $Phase | Out-Null

    $iters = $QA.ChurnCommits
    $rng = New-Object System.Random(72001)
    $buf = New-Object byte[] (1MB)
    for ($k = 1; $k -le $iters; $k++) {
      # rewrite a few interior MB to create a new near-duplicate version (delta-friendly)
      $rng.NextBytes($buf)
      $fs = [System.IO.File]::Open($asset, [System.IO.FileMode]::Open, [System.IO.FileAccess]::Write)
      try { $fs.Seek(($k % 30) * 1MB, [System.IO.SeekOrigin]::Begin) | Out-Null; $fs.Write($buf, 0, $buf.Length) }
      finally { $fs.Close() }
      Invoke-MG $repo @("add", "asset.bin") $Phase | Out-Null
      Invoke-MG $repo @("commit", "-m", "churn $k") $Phase | Out-Null
      if ($k % 100 -eq 0) { Write-QaLog $Phase "S2 churn $k/$iters" }
    }

    $stats = Get-QaChainStats $repo
    $depthOk = ($stats.MaxDepth -le $MAX_DELTA_DEPTH) -and ($stats.CycleCount -eq 0)
    $fsckOk = Test-QaFsckClean $repo
    Rec $drill "local" "chain-depth" $stats.MaxDepth $depthOk `
      "commits=$iters maxDepth=$($stats.MaxDepth) cycles=$($stats.CycleCount) chains=$($stats.ChainCount) cap=$MAX_DELTA_DEPTH"
    Rec $drill "local" "fsck-clean" $fsckOk $fsckOk "post-churn fsck"

    # regression guard: the churned repo must still push cleanly.
    try { $srv = Start-QaServer -Backend "minio" -Phase "$Phase-S2" } catch {
      if ("$_" -match "^SKIP:") { Rec $drill "minio" "still-pushable" "" "SKIP" "$_"; return }
      throw
    }
    $script:Servers += $srv
    Invoke-MG $repo @("remote", "add", "origin", $srv.Url) $Phase | Out-Null
    $r = Invoke-MG $repo @("push", "origin") $Phase -TimeoutSec 3600
    Rec $drill "minio" "still-pushable" ($r.Exit -eq 0) ($r.Exit -eq 0) "push exit=$($r.Exit)"
  } catch {
    Rec $drill "local" "churn" "" $false "unexpected error: $_"
  } finally {
    Stop-QaServer $srv
  }
}

# ---------------------------------------------------------------------------
# S3: interleaved conflicts. N concurrent history-rewrite ops (merge/rebase/cherry-pick),
# each on its own clone. Scale-up of the v11 data-loss class. Each result must be
# internally consistent and no untouched baseline file may silently change.
# ---------------------------------------------------------------------------
function Drill-S3-Conflicts {
  $drill = "S3-conflicts"
  try {
    # Base repo: main with a stable baseline file + three feature branches touching shared.txt.
    $base = New-ScaleDir "s3-base"
    Invoke-MG $null @("init", $base) $Phase | Out-Null
    Set-Content (Join-Path $base "baseline.txt") "immutable baseline" -Encoding Ascii
    Set-Content (Join-Path $base "shared.txt") "line0`n" -Encoding Ascii
    Invoke-MG $base @("add", ".") $Phase | Out-Null
    Invoke-MG $base @("commit", "-m", "s3 base") $Phase | Out-Null
    $baselineHash = Get-QaHash (Join-Path $base "baseline.txt")

    $ops = @(
      @{ Name = "feat-a"; Line = "A change" },
      @{ Name = "feat-b"; Line = "B change" },
      @{ Name = "feat-c"; Line = "C change" }
    )
    foreach ($op in $ops) {
      Invoke-MG $base @("branch", "create", $op.Name) $Phase | Out-Null
      Invoke-MG $base @("branch", "switch", $op.Name) $Phase | Out-Null
      Add-Content (Join-Path $base "shared.txt") ($op.Line + "`n")
      Invoke-MG $base @("add", ".") $Phase | Out-Null
      Invoke-MG $base @("commit", "-m", ("s3 " + $op.Name)) $Phase | Out-Null
      Invoke-MG $base @("branch", "switch", "main") $Phase | Out-Null
    }
    # advance main too, so each branch merge/rebase is a real 3-way conflict on shared.txt
    Add-Content (Join-Path $base "shared.txt") "main change`n"
    Invoke-MG $base @("add", ".") $Phase | Out-Null
    Invoke-MG $base @("commit", "-m", "s3 main advance") $Phase | Out-Null

    # N concurrent clones, each attempting a different rewrite op against its branch.
    $verbs = @("merge", "rebase", "cherry-pick")
    $jobs = @()
    for ($i = 0; $i -lt $ops.Count; $i++) {
      $op = $ops[$i]; $verb = $verbs[$i % $verbs.Count]
      $dest = Join-Path $QA.Work "scale10-s3-clone-$i"
      if (Test-Path $dest) { Remove-Item -Recurse -Force $dest -ErrorAction SilentlyContinue }
      Copy-Item $base $dest -Recurse
      $jobs += Start-Job -ScriptBlock {
        param($m, $d, $verb, $branch)
        # abort-on-conflict is the safe, non-interactive outcome; either a clean apply or a
        # clean abort is acceptable - a panic / partial write is not.
        $out = ""
        switch ($verb) {
          "merge"       { $out = & $m -C $d merge $branch 2>&1 | Out-String }
          "rebase"      { $out = & $m -C $d branch switch $branch 2>&1 | Out-String; $out += & $m -C $d rebase main 2>&1 | Out-String }
          "cherry-pick" { $out = & $m -C $d cherry-pick $branch 2>&1 | Out-String }
        }
        $ec = $LASTEXITCODE
        # abort any half-open op so the repo lands in a consistent state
        & $m -C $d merge --abort 2>&1 | Out-Null
        & $m -C $d rebase --abort 2>&1 | Out-Null
        "$ec"
      } -ArgumentList $QA.MG, $dest, $verb, $op.Name
    }
    Wait-Job $jobs -Timeout 1800 | Out-Null
    $jobs | ForEach-Object { Receive-Job $_ | Out-Null }
    $jobs | Remove-Job -Force -ErrorAction SilentlyContinue

    $consistent = 0; $baselineIntact = 0; $panic = $false
    for ($i = 0; $i -lt $ops.Count; $i++) {
      $dest = Join-Path $QA.Work "scale10-s3-clone-$i"
      if (-not (Test-Path $dest)) { continue }
      if (Test-QaFsckClean $dest) { $consistent++ }
      $bl = Join-Path $dest "baseline.txt"
      if ((Test-Path $bl) -and ((Get-QaHash $bl) -eq $baselineHash)) { $baselineIntact++ }
      Remove-Item -Recurse -Force $dest -ErrorAction SilentlyContinue
    }
    $pass = ($consistent -eq $ops.Count) -and ($baselineIntact -eq $ops.Count)
    Rec $drill "local" "no-data-loss" $consistent $pass `
      "ops=$($ops.Count) fsckClean=$consistent baselineIntact=$baselineIntact"
  } catch {
    Rec $drill "local" "no-data-loss" "" $false "unexpected error: $_"
  }
}

# ---------------------------------------------------------------------------
# S4: resource pressure. A multi-GB blob + the many-files corpus; peak client/server RSS
# must stay under the ceiling (streaming, no load-whole-file) and no OOM. Disk-bounded.
# ---------------------------------------------------------------------------
function Drill-S4-ResourcePressure {
  $drill = "S4-resource"
  if (-not $diskOk) { Rec $drill "local" "peak-rss-mb" "" "SKIP" "insufficient free disk (${freeGB}GB < $($QA.DiskBudgetGB)GB)"; return }
  try {
    $repo = New-ScaleDir "s4-repo"
    Invoke-MG $null @("init", $repo) $Phase | Out-Null

    # one big blob (streaming) + count pressure from the corpus
    $bigMB = [math]::Max(256, [int]($BlobBudgetMB / 2))
    New-ScaleBlob (Join-Path $repo "giant.bin") $bigMB 74000
    if (Test-Path $ManyFiles) { Copy-Item $ManyFiles (Join-Path $repo "manyfiles") -Recurse }

    $m = Measure-PeakRSS -Action {
      Invoke-MG $repo @("add", ".") $Phase -TimeoutSec 3600 | Out-Null
      Invoke-MG $repo @("commit", "-m", "s4 giant+corpus") $Phase
    }
    $commit = $m.Result
    $peakMB = $m.PeakMB
    $noOom = ($commit.Exit -eq 0)                       # 124=timeout/kill, nonzero=crash
    $fsckOk = Test-QaFsckClean $repo
    $rssOk = ($peakMB -le $QA.RssCeilMB) -and ($peakMB -gt 0)
    $pass = $rssOk -and $noOom -and $fsckOk
    Rec $drill "local" "peak-rss-mb" $peakMB $pass `
      "bigMB=$bigMB peakRSS=${peakMB}MB ceil=$($QA.RssCeilMB)MB commitExit=$($commit.Exit) fsck=$fsckOk"
    Remove-Item -Recurse -Force $repo -ErrorAction SilentlyContinue   # reclaim the GB now
  } catch {
    Rec $drill "local" "peak-rss-mb" "" $false "unexpected error: $_"
  }
}

# ---------------------------------------------------------------------------
# S5: scale throughput + dedup, per backend. Full payload on minio/local (SLO-gated),
# CloudMaxMB-capped on billed backends (throughput informational). Dedup measured off the
# perturbed safetensors chain when present.
# ---------------------------------------------------------------------------
function Drill-S5-ThroughputDedup {
  $drill = "S5-throughput"
  if (-not $diskOk) { Rec $drill "all" "push-mbs" "" "SKIP" "insufficient free disk (${freeGB}GB < $($QA.DiskBudgetGB)GB)"; return }

  $src = $null
  try {
  # shared full-scale source built once; cloud backends push a capped subset of it.
  $src = New-ScaleDir "s5-src"
  Invoke-MG $null @("init", $src) $Phase | Out-Null
  $blobMB = [math]::Max(256, $BlobBudgetMB)
  $nBlobs = 4
  $per = [int]($blobMB / $nBlobs)
  for ($i = 1; $i -le $nBlobs; $i++) { New-ScaleBlob (Join-Path $src "blobs\big_$i.bin") $per (75000 + $i) }
  # dedup material: the ml safetensors chain (near-duplicate versions) if generated.
  $mlDir = Join-Path $QA.Fixtures "ml"
  $mlFiles = @()
  if (Test-Path $mlDir) { $mlFiles = @(Get-ChildItem $mlDir -Filter "model_v*.safetensors" -File | Sort-Object Name) }
  if ($mlFiles.Count -gt 0) { New-Item -ItemType Directory -Path (Join-Path $src "ml") -Force | Out-Null }
  foreach ($f in $mlFiles) { Copy-Item $f.FullName (Join-Path $src ("ml\" + $f.Name)) -Force }
  Invoke-MG $src @("add", ".") $Phase -TimeoutSec 3600 | Out-Null
  Invoke-MG $src @("commit", "-m", "s5 payload") $Phase | Out-Null

  # dedup: measure the safetensors chain in a DEDICATED repo - the throughput $src also
  # holds 512MB of random blobs, so its whole-odb footprint says nothing about chain dedup.
  if ($mlFiles.Count -ge 2) {
    $dr = New-ScaleDir "s5-dedup"
    Invoke-MG $null @("init", $dr) $Phase | Out-Null
    foreach ($f in $mlFiles) { Copy-Item $f.FullName (Join-Path $dr $f.Name) -Force }
    Invoke-MG $dr @("add", ".") $Phase -TimeoutSec 3600 | Out-Null
    Invoke-MG $dr @("commit", "-m", "dedup chain") $Phase | Out-Null
    $rawMB = [math]::Round((($mlFiles | Measure-Object Length -Sum).Sum) / 1MB, 2)
    $odbMB = Get-DirMB (Join-Path $dr ".mediagit\objects")
    $savedPct = if ($rawMB -gt 0) { [math]::Round((1.0 - $odbMB / $rawMB) * 100.0, 1) } else { 0 }
    $dedupOk = $savedPct -ge $DEDUP_FLOOR_PCT
    Rec $drill "local" "dedup-pct" $savedPct $dedupOk "chain=$($mlFiles.Count) rawMB=$rawMB odbMB=$odbMB floor=$DEDUP_FLOOR_PCT%"
    Remove-Item -Recurse -Force $dr -ErrorAction SilentlyContinue
  } else {
    Rec $drill "local" "dedup-pct" "" "SKIP" "safetensors chain not present (ml deps missing)"
  }

  $cloudSrc = $null
  foreach ($backend in $QA.Backends) {
    $srv = $null
    try {
      try { $srv = Start-QaServer -Backend $backend -Phase "$Phase-S5" } catch {
        if ("$_" -match "^SKIP:") { Rec $drill $backend "push-mbs" "" "SKIP" "$_"; continue }
        throw
      }
      $script:Servers += $srv
      $isFast = ($backend -eq "minio" -or $backend -eq "local")

      # Fast backends push the full $src; billed WAN backends push a CloudMaxMB-capped,
      # blobs-only source (built once) so a matrix run is minutes, not hours - the
      # safetensors dedup is already measured locally above.
      if ($isFast) {
        $pushSrc = $src
      } else {
        if (-not $cloudSrc) {
          $cloudSrc = New-ScaleDir "s5-cloudsrc"
          Invoke-MG $null @("init", $cloudSrc) $Phase | Out-Null
          $cloudMB = [math]::Min($blobMB, $QA.CloudMaxMB)
          $cn = 4; $cper = [math]::Max(16, [int]($cloudMB / $cn))
          for ($ci = 1; $ci -le $cn; $ci++) { New-ScaleBlob (Join-Path $cloudSrc "blobs\c_$ci.bin") $cper (76000 + $ci) }
          Invoke-MG $cloudSrc @("add", ".") $Phase -TimeoutSec 3600 | Out-Null
          Invoke-MG $cloudSrc @("commit", "-m", "s5 cloud payload") $Phase | Out-Null
        }
        $pushSrc = $cloudSrc
      }

      Invoke-MG $pushSrc @("remote", "remove", "origin") $Phase | Out-Null
      Invoke-MG $pushSrc @("remote", "add", "origin", $srv.Url) $Phase | Out-Null

      $payloadMB = Get-DirMB $pushSrc -ExcludeOdb
      $r = Invoke-MG $pushSrc @("push", "origin") $Phase -TimeoutSec 7200
      $pushMbs = if ($r.Sec -gt 0) { [math]::Round($payloadMB / $r.Sec, 2) } else { 0 }
      # SLO floor gates only fast backends; clouds are WAN-bound -> informational pass.
      $pushPass = if ($isFast) { ($r.Exit -eq 0) -and ($pushMbs -ge $PUSH_FLOOR_MBS) } else { ($r.Exit -eq 0) }
      Rec $drill $backend "push-mbs" $pushMbs $pushPass "payloadMB=$payloadMB sec=$($r.Sec) floor=$(if($isFast){$PUSH_FLOOR_MBS}else{'n/a(WAN)'}) exit=$($r.Exit)"
      if ($r.Exit -ne 0) { continue }

      $clone = Join-Path $QA.Work "scale10-s5-clone-$backend"
      if (Test-Path $clone) { Remove-Item -Recurse -Force $clone -ErrorAction SilentlyContinue }
      $r = Invoke-MG $null @("clone", $srv.Url, $clone) $Phase -TimeoutSec 7200
      $cloneMbs = if ($r.Sec -gt 0) { [math]::Round($payloadMB / $r.Sec, 2) } else { 0 }
      $parity = ($r.Exit -eq 0) -and (Test-Path $clone) -and `
        ((Compare-Object (Get-QaTreeHashes $pushSrc) (Get-QaTreeHashes $clone) | Measure-Object).Count -eq 0)
      $clonePass = $parity -and $(if ($isFast) { $cloneMbs -ge $PULL_FLOOR_MBS } else { $true })
      Rec $drill $backend "clone-mbs" $cloneMbs $clonePass "parity=$parity sec=$($r.Sec) floor=$(if($isFast){$PULL_FLOOR_MBS}else{'n/a(WAN)'}) exit=$($r.Exit)"
      Remove-Item -Recurse -Force $clone -ErrorAction SilentlyContinue
    } catch {
      Rec $drill $backend "phase" "" $false "unexpected error: $_"
    } finally {
      Stop-QaServer $srv
    }
  }
  } catch {
    Rec $drill "local" "setup" "" $false "S5 setup error: $_"
  } finally {
    if ($src) { Remove-Item -Recurse -Force $src -ErrorAction SilentlyContinue }
    if ($cloudSrc) { Remove-Item -Recurse -Force $cloudSrc -ErrorAction SilentlyContinue }
  }
}

# ---------------------------------------------------------------------------
# Teardown: reclaim this phase's work/ scratch. Only phase-owned dirs are removed;
# test-files/logs/reports are never touched. MG_QA_KEEP_SCRATCH=1 preserves for triage.
# ---------------------------------------------------------------------------
function Invoke-ScaleTeardown {
  foreach ($s in $script:Servers) { Stop-QaServer $s }
  if ($QA.KeepScratch) { Write-QaLog $Phase "MG_QA_KEEP_SCRATCH=1 - scratch preserved under $($QA.Work)"; return }
  $before = Get-DirMB $QA.Work
  Get-ChildItem $QA.Work -Directory -ErrorAction SilentlyContinue |
    Where-Object { $_.Name -like "scale10-*" -or $_.Name -like "server-*-10_scale-*" } |
    ForEach-Object { Remove-Item -Recurse -Force $_.FullName -ErrorAction SilentlyContinue }
  if ($QA.PurgeFixtures) {
    $sf = Join-Path $QA.Fixtures "scale"
    if (Test-Path $sf) { Remove-Item -Recurse -Force $sf -ErrorAction SilentlyContinue }
    Write-QaLog $Phase "MG_QA_PURGE_FIXTURES=1 - deleted generated scale fixtures"
  }
  $after = Get-DirMB $QA.Work
  Write-QaLog $Phase ("teardown reclaimed {0} MB (work/ {1} -> {2} MB)" -f [math]::Round($before - $after, 1), $before, $after)
}

# ---------------------------------------------------------------------------
try {
  Drill-S1-Concurrency
  Drill-S2-Churn
  Drill-S3-Conflicts
  Drill-S4-ResourcePressure
  Drill-S5-ThroughputDedup
} catch {
  Write-QaLog $Phase "UNHANDLED phase error: $_"
  $script:AllPass = $false
} finally {
  Invoke-ScaleTeardown
}

Write-QaLog $Phase "=== 10_scale done: overall=$(if ($script:AllPass) { 'PASS' } else { 'FAIL' }) ==="
if ($script:AllPass) { exit 0 } else { exit 1 }
