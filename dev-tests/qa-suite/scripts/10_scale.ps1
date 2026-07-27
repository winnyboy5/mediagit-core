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

# Set AFTER common.ps1 (which sets Continue) so it wins. Scale drills run for hours and
# write GBs; a silently-swallowed error here means a drill "passes" having done nothing.
# Every drill body is inside try/catch, so a terminating error becomes a recorded FAIL row
# rather than an unexplained exit.
# ponytail: applied to this phase + run_all + lib only. The older phases predate the
# convention and would need their own triage pass - tracked as follow-up, not silently
# assumed safe.
$ErrorActionPreference = "Stop"

$Phase = "10_scale"
$env:MEDIAGIT_AUTHOR_NAME = "QA-Suite"
$env:MEDIAGIT_AUTHOR_EMAIL = "qa-suite@mediagit.local"

$TSV = Join-Path $QA.Logs "scale_results.tsv"
$script:AllPass = $true
$script:Servers = @()   # Start-QaServer handles to stop at teardown

$MAX_DELTA_DEPTH = 10                      # crates/mediagit-versioning/src/odb/mod.rs
$PUSH_FLOOR_MBS  = [math]::Round(10240.0 / 720.0, 2)   # 10GB <=12min push SLO -> 14.22 MB/s
$PULL_FLOOR_MBS  = [math]::Round(10240.0 / 600.0, 2)   # 10GB <=10min pull SLO -> 17.07 MB/s
# Perturbed safetensors chains measure ~74% dedup in practice; 15% was so far below the
# observed floor that a near-total dedup collapse would still have passed.
$DEDUP_FLOOR_PCT = 35.0
# Cloud backends are WAN-bound, so their throughput floor is an operator-set knob rather
# than a fixed SLO. 0 (default) records the number without gating - set MG_QA_CLOUD_MBS_FLOOR
# once a link's real capability has been measured.
$CLOUD_FLOOR_MBS = $QA.CloudMbsFloor

# Backends S1 exercises. Fast local-ish backends always; billed ones only when the
# operator selected them (Start-QaServer SKIPs the rest with the not-selected marker).
function Get-ScaleBackends {
  $sel = @("local", "minio") + @($QA.Backends | Where-Object { $_ -in @("aws", "azure", "gcs") })
  return @($sel | Where-Object { $_ -eq "local" -or $QA.Backends -contains $_ } | Select-Object -Unique)
}

function Rec([string]$Drill, [string]$Backend, [string]$Metric, $Value, $Pass, [string]$Detail) {
  Write-QaRow $TSV @("drill", "backend", "metric", "value", "pass", "detail") `
    @($Drill, $Backend, $Metric, $Value, $Pass, $Detail)
  $tag = if ("$Pass" -eq "SKIP") { "SKIP" } elseif ($Pass) { "PASS" } else { "FAIL" }
  Write-QaLog $Phase ("{0} [{1}] {2}={3} -> {4}  {5}" -f $Drill, $Backend, $Metric, $Value, $tag, $Detail)
  Write-QaGate $Phase "$Drill-$Metric" $Pass $Detail
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
  foreach ($backend in (Get-ScaleBackends)) {
    Drill-S1-ForBackend $backend
  }
}

function Drill-S1-ForBackend([string]$backend) {
  $drill = "S1-concurrency"
  $srv = $null
  $src = $null
  try {
    # per-drill Phase suffix so each Start-QaServer gets a unique repo name; without it,
    # S1/S2/S5 share proj-<runid>-10_scale and collide on each other's MinIO bucket state
    # (07_abuse uses the same -A2/-A3 convention).
    try { $srv = Start-QaServer -Backend $backend -Phase "$Phase-S1-$backend" } catch {
      # Only a genuinely unselected/unconfigured backend is a SKIP; a server that will
      # not start on a backend we DID ask for is a failure (see lib\remote.ps1).
      if ("$_" -match "^SKIP:") { Rec $drill $backend "clones-converge" "" "SKIP" "$_"; return }
      Rec $drill $backend "clones-converge" "" $false "server unavailable: $_"; return
    }
    $script:Servers += $srv

    # Source: many-files corpus (count pressure) + two modest blobs. Concurrency, not size.
    # Billed backends get the corpus only - this drill is about simultaneous clients,
    # and pushing the full corpus over a WAN link would dominate the runtime.
    $isFast = ($backend -eq "minio" -or $backend -eq "local")
    $src = New-ScaleDir "s1-src-$backend"
    Invoke-MG $null @("init", $src) $Phase | Out-Null
    if (Test-Path $ManyFiles) { Copy-Item $ManyFiles (Join-Path $src "manyfiles") -Recurse }
    $blobMB = if ($isFast) { 64 } else { [math]::Max(8, [math]::Min(64, [int]($QA.CloudMaxMB / 8))) }
    New-ScaleBlob (Join-Path $src "blob_a.bin") $blobMB 71001
    New-ScaleBlob (Join-Path $src "blob_b.bin") $blobMB 71002
    Invoke-MG $src @("add", ".") $Phase -TimeoutSec 1800 | Out-Null
    Invoke-MG $src @("commit", "-m", "s1 payload") $Phase | Out-Null
    Invoke-MG $src @("remote", "add", "origin", $srv.Url) $Phase | Out-Null
    $r = Invoke-MG $src @("push", "origin") $Phase -TimeoutSec 3600
    if ($r.Exit -ne 0) { Rec $drill $backend "clones-converge" 0 $false "push failed exit=$($r.Exit)"; return }
    $srcHashes = Get-QaTreeHashes $src

    # Cloud backends get fewer simultaneous clients: $QA.Concurrency (16) parallel WAN
    # clones is a bandwidth test, not a concurrency test, and would run for hours.
    $N = if ($isFast) { $QA.Concurrency } else { [math]::Min(4, $QA.Concurrency) }
    $jobs = @()
    for ($i = 1; $i -le $N; $i++) {
      $dest = Join-Path $QA.Work "scale10-s1-clone-$backend-$i"
      if (Test-Path $dest) { Remove-Item -Recurse -Force $dest -ErrorAction SilentlyContinue }
      $jobs += Start-Job -ScriptBlock {
        param($m, $u, $d)
        $out = & $m clone $u $d 2>&1 | Out-String
        # Emit the exit code AND the output: a clone that fails needs its error text
        # to be diagnosable, and a panic must not be invisible to the caller.
        "$LASTEXITCODE`n---OUT---`n$out"
      } -ArgumentList $QA.MG, $srv.Url, $dest
    }
    $done = Wait-Job $jobs -Timeout 3600
    $timedOut = @($jobs | Where-Object { $_.State -ne "Completed" }).Count
    $results = @($jobs | ForEach-Object { ("" + (Receive-Job $_)) })
    $jobs | Remove-Job -Force -ErrorAction SilentlyContinue

    # Exit codes are ASSERTED, not merely collected: a clone can exit nonzero and still
    # leave a directory behind, so Test-Path alone reports success for a failed clone.
    $exitOk = 0; $panics = 0
    foreach ($res in $results) {
      $code = (($res -split "`n")[0]).Trim()
      if ($code -eq "0") { $exitOk++ }
      if ($res -match "panicked|RUST_BACKTRACE") { $panics++ }
    }

    $converged = 0; $fsckClean = 0; $present = 0
    for ($i = 1; $i -le $N; $i++) {
      $dest = Join-Path $QA.Work "scale10-s1-clone-$backend-$i"
      if (-not (Test-Path $dest)) { continue }
      $present++
      # Full-tree SHA-256 parity against the source. The old cloneOk was Test-Path,
      # which proves a directory exists and nothing whatsoever about its contents.
      if ((Compare-Object $srcHashes (Get-QaTreeHashes $dest) | Measure-Object).Count -eq 0) { $converged++ }
      if (Test-QaFsckClean $dest) { $fsckClean++ }
      Remove-Item -Recurse -Force $dest -ErrorAction SilentlyContinue
    }
    $srvAlive = -not $srv.Proc.HasExited
    $pass = ($exitOk -eq $N) -and ($present -eq $N) -and ($converged -eq $N) -and
            ($fsckClean -eq $N) -and $srvAlive -and ($panics -eq 0) -and ($timedOut -eq 0)
    Rec $drill $backend "clones-converge" $converged $pass `
      "n=$N exit0=$exitOk present=$present converged=$converged fsckClean=$fsckClean panics=$panics timedOut=$timedOut serverAlive=$srvAlive"
  } catch {
    Rec $drill $backend "clones-converge" "" $false "unexpected error: $_"
  } finally {
    Stop-QaServer $srv
    if ($src) { Remove-Item -Recurse -Force $src -ErrorAction SilentlyContinue }
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
    # Per-100-commit wall time. The 2026-07-27 campaign measured commit cost growing
    # superlinearly across a 500-commit run (3.5 -> 5.3 -> 7.4 min per 100) while chain
    # depth stayed at 1 - so it is a cost curve, not a correctness problem, and gating on
    # absolute time would just encode this machine's speed. The slope is the durable
    # signal: a run whose last block costs far more than its first is degrading with
    # history size, which is the thing that would eventually make a repo unusable.
    $blockTimes = @()
    $blockSw = [Diagnostics.Stopwatch]::StartNew()
    for ($k = 1; $k -le $iters; $k++) {
      # rewrite a few interior MB to create a new near-duplicate version (delta-friendly)
      $rng.NextBytes($buf)
      $fs = [System.IO.File]::Open($asset, [System.IO.FileMode]::Open, [System.IO.FileAccess]::Write)
      try { $fs.Seek(($k % 30) * 1MB, [System.IO.SeekOrigin]::Begin) | Out-Null; $fs.Write($buf, 0, $buf.Length) }
      finally { $fs.Close() }
      Invoke-MG $repo @("add", "asset.bin") $Phase | Out-Null
      Invoke-MG $repo @("commit", "-m", "churn $k") $Phase | Out-Null
      if ($k % 100 -eq 0) {
        $blockTimes += [math]::Round($blockSw.Elapsed.TotalSeconds, 1)
        $blockSw.Restart()
        Write-QaLog $Phase ("S2 churn {0}/{1} (last 100 took {2}s)" -f $k, $iters, $blockTimes[-1])
      }
    }

    # Slope tripwire: fail only on runaway growth, not on the mild curve already observed.
    # 3x between the first and last block is well clear of the measured ~2.1x, so this
    # cannot fire on today's behaviour - it fires when degradation gets materially worse.
    if ($blockTimes.Count -ge 2) {
      $first = [double]$blockTimes[0]
      $last = [double]$blockTimes[-1]
      $ratio = if ($first -gt 0) { [math]::Round($last / $first, 2) } else { 0 }
      $slopeOk = ($first -le 0) -or ($ratio -le 3.0)
      Rec $drill "local" "churn-cost-slope" $ratio $slopeOk `
      ("blocks=$($blockTimes -join ',')s first=${first}s last=${last}s ratio=${ratio}x cap=3.0x")
    }

    # Ground truth for the round trip below: what the worktree holds after the churn.
    $postChurnHash = Get-QaHash $asset

    $stats = Get-QaChainStats $repo
    if ($stats.MaxDepth -lt 0) {
      # -1 means the repo has no chunk-delta storage at all. That is not "depth 0 = fine":
      # $iters near-duplicate commits are exactly the workload that is supposed to produce
      # deltas, so producing none means either the workload or delta selection has silently
      # stopped working, and the depth gate below would be measuring nothing.
      Rec $drill "local" "chain-depth" "none" $false `
        "commits=$iters produced NO chunk-deltas - delta path did not engage (expected near-duplicate versions to delta)"
    } else {
      $depthOk = ($stats.MaxDepth -le $MAX_DELTA_DEPTH) -and ($stats.CycleCount -eq 0)
      Rec $drill "local" "chain-depth" $stats.MaxDepth $depthOk `
        "commits=$iters maxDepth=$($stats.MaxDepth) cycles=$($stats.CycleCount) chains=$($stats.ChainCount) cap=$MAX_DELTA_DEPTH"
    }
    $fsckOk = Test-QaFsckClean $repo
    Rec $drill "local" "fsck-clean" $fsckOk $fsckOk "post-churn fsck"

    # regression guard: the churned repo must still push cleanly...
    try { $srv = Start-QaServer -Backend "minio" -Phase "$Phase-S2" } catch {
      if ("$_" -match "^SKIP:") { Rec $drill "minio" "still-pushable" "" "SKIP" "$_"; return }
      Rec $drill "minio" "still-pushable" "" $false "server unavailable: $_"; return
    }
    $script:Servers += $srv
    Invoke-MG $repo @("remote", "add", "origin", $srv.Url) $Phase | Out-Null
    $r = Invoke-MG $repo @("push", "origin") $Phase -TimeoutSec 3600
    Rec $drill "minio" "still-pushable" ($r.Exit -eq 0) ($r.Exit -eq 0) "push exit=$($r.Exit)"
    if ($r.Exit -ne 0) { return }

    # ...and what came back off the wire must be the bytes we pushed. A deep delta chain
    # that pushes "successfully" but reconstructs to different bytes is the exact failure
    # this drill exists for, and a push exit code alone cannot see it.
    $back = Join-Path $QA.Work "scale10-s2-cloneback"
    if (Test-Path $back) { Remove-Item -Recurse -Force $back -ErrorAction SilentlyContinue }
    $c = Invoke-MG $null @("clone", $srv.Url, $back) $Phase -TimeoutSec 3600
    $backAsset = Join-Path $back "asset.bin"
    $roundTripOk = ($c.Exit -eq 0) -and (Test-Path $backAsset) -and ((Get-QaHash $backAsset) -eq $postChurnHash)
    $backFsck = if (Test-Path $back) { Test-QaFsckClean $back } else { $false }
    Rec $drill "minio" "roundtrip-hash" $roundTripOk ($roundTripOk -and $backFsck) `
      "clone exit=$($c.Exit) hash-match=$roundTripOk fsck=$backFsck chainDepth=$($stats.MaxDepth)"
    Remove-Item -Recurse -Force $back -ErrorAction SilentlyContinue
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

    # Contender count: 3 by default; MG_QA_CONCURRENCY raises it when an operator wants
    # more simultaneous rewrites (names cycle a..z so the set stays deterministic).
    $opCount = [math]::Max(3, [math]::Min(26, $QA.Concurrency))
    $ops = @()
    for ($oi = 0; $oi -lt $opCount; $oi++) {
      $letter = [char](97 + $oi)
      $ops += @{ Name = "feat-$letter"; Line = "$([char](65 + $oi)) change" }
    }
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
        $out += & $m -C $d merge --abort 2>&1 | Out-String
        $out += & $m -C $d rebase --abort 2>&1 | Out-String
        "$ec`n---OUT---`n$out"
      } -ArgumentList $QA.MG, $dest, $verb, $op.Name
    }
    Wait-Job $jobs -Timeout 1800 | Out-Null
    $timedOut = @($jobs | Where-Object { $_.State -ne "Completed" }).Count
    # Outputs were previously piped to Out-Null - a panic in any contender was discarded
    # unread, and only fsck's opinion of the repo was ever consulted.
    $opResults = @($jobs | ForEach-Object { ("" + (Receive-Job $_)) })
    $jobs | Remove-Job -Force -ErrorAction SilentlyContinue

    $panics = @($opResults | Where-Object { $_ -match "panicked|RUST_BACKTRACE" }).Count
    $opExits = @($opResults | ForEach-Object { (($_ -split "`n")[0]).Trim() })

    # Every legal outcome of a conflicting rewrite leaves shared.txt holding lines that
    # were actually written by somebody. Anything else - a truncated line, a merge marker
    # left in place, a blend of two contenders' text - is silent corruption that fsck
    # cannot see, because the file is structurally fine and simply says the wrong thing.
    $knownLines = @("line0", "main change") + @($ops | ForEach-Object { $_.Line })
    $consistent = 0; $baselineIntact = 0; $sharedSane = 0; $sharedMissing = 0
    for ($i = 0; $i -lt $ops.Count; $i++) {
      $dest = Join-Path $QA.Work "scale10-s3-clone-$i"
      if (-not (Test-Path $dest)) { continue }
      if (Test-QaFsckClean $dest) { $consistent++ }
      $bl = Join-Path $dest "baseline.txt"
      if ((Test-Path $bl) -and ((Get-QaHash $bl) -eq $baselineHash)) { $baselineIntact++ }

      $sh = Join-Path $dest "shared.txt"
      if (-not (Test-Path $sh)) {
        $sharedMissing++
      } else {
        $lines = @(Get-Content $sh -ErrorAction SilentlyContinue | Where-Object { "$_".Trim() -ne "" })
        $unknown = @($lines | Where-Object { $knownLines -notcontains "$_".Trim() })
        $markers = @($lines | Where-Object { "$_" -match '^(<<<<<<<|>>>>>>>|=======)' })
        if ($unknown.Count -eq 0 -and $markers.Count -eq 0 -and $lines.Count -gt 0) { $sharedSane++ }
        else { Write-QaLog $Phase "S3 clone-$i shared.txt unexpected content: unknown=$($unknown -join '/') markers=$($markers.Count)" }
      }
      Remove-Item -Recurse -Force $dest -ErrorAction SilentlyContinue
    }
    $pass = ($consistent -eq $ops.Count) -and ($baselineIntact -eq $ops.Count) -and
            ($sharedSane -eq $ops.Count) -and ($panics -eq 0) -and ($timedOut -eq 0)
    Rec $drill "local" "no-data-loss" $consistent $pass `
      ("ops=$($ops.Count) fsckClean=$consistent baselineIntact=$baselineIntact sharedSane=$sharedSane " +
       "sharedMissing=$sharedMissing panics=$panics timedOut=$timedOut exits=$($opExits -join ',')")
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
  $srv = $null
  $repo = $null
  try {
    $repo = New-ScaleDir "s4-repo"
    Invoke-MG $null @("init", $repo) $Phase | Out-Null

    # one big blob (streaming) + count pressure from the corpus
    # 4096MB ceiling: above this the drill stops testing streaming behaviour and starts
    # testing the disk budget. Deliberate - do not raise without raising DiskBudgetGB.
    $bigMB = [math]::Min(4096, [math]::Max(256, [int]($BlobBudgetMB / 2)))
    $giant = Join-Path $repo "giant.bin"
    New-ScaleBlob $giant $bigMB 74000
    $giantHash = Get-QaHash $giant
    if (Test-Path $ManyFiles) { Copy-Item $ManyFiles (Join-Path $repo "manyfiles") -Recurse }

    $m = Measure-PeakRSS -Phase $Phase -Label "s4" -Action {
      Invoke-MG $repo @("add", ".") $Phase -TimeoutSec 3600 | Out-Null
      Invoke-MG $repo @("commit", "-m", "s4 giant+corpus") $Phase
    }
    $commit = $m.Result
    $noOom = ($commit.Exit -eq 0)                       # 124=timeout/kill, nonzero=crash
    $fsckOk = Test-QaFsckClean $repo
    # Client and server peaks are gated separately: a client that streams correctly must
    # not be excused by a server that does, or vice versa.
    $clientOk = ($m.ClientPeakMB -le $QA.RssCeilMB) -and ($m.ClientPeakMB -gt 0)
    # This drill runs no server, so Measure-PeakRSS reports "n/a" rather than 0 - a real
    # not-applicable, not a measurement of zero. Treat n/a as passing; gate any actual
    # number. (It used to record 0 and compare it to the ceiling, which meant a server-side
    # leak on a drill that DID run a server would also have passed silently.)
    $serverOk = ($m.ServerPeakMB -eq "n/a") -or ([double]$m.ServerPeakMB -le $QA.RssCeilMB)
    $pass = $clientOk -and $serverOk -and $noOom -and $fsckOk
    Rec $drill "local" "peak-rss-mb" $m.ClientPeakMB $pass `
      ("bigMB=$bigMB clientPeak=$($m.ClientPeakMB)MB serverPeak=$($m.ServerPeakMB)MB " +
       "clientPrivate=$($m.ClientPrivatePeakMB)MB serverPrivate=$($m.ServerPrivatePeakMB)MB " +
       "ceil=$($QA.RssCeilMB)MB commitExit=$($commit.Exit) fsck=$fsckOk samples=$(Split-Path $m.SamplesTsv -Leaf)")
    if (-not $noOom) { return }

    # The blob must survive a round trip. Peak RSS staying under a ceiling proves the
    # client streamed rather than buffered; it says nothing about whether the bytes it
    # streamed were the right ones, which is the failure that actually loses a user's work.
    try { $srv = Start-QaServer -Backend "minio" -Phase "$Phase-S4" } catch {
      if ("$_" -match "^SKIP:") { Rec $drill "minio" "giant-roundtrip" "" "SKIP" "$_"; return }
      Rec $drill "minio" "giant-roundtrip" "" $false "server unavailable: $_"; return
    }
    $script:Servers += $srv
    Invoke-MG $repo @("remote", "add", "origin", $srv.Url) $Phase | Out-Null
    $p = Invoke-MG $repo @("push", "origin") $Phase -TimeoutSec 7200
    if ($p.Exit -ne 0) {
      Rec $drill "minio" "giant-roundtrip" $false $false "push exit=$($p.Exit)"
      return
    }
    $back = Join-Path $QA.Work "scale10-s4-cloneback"
    if (Test-Path $back) { Remove-Item -Recurse -Force $back -ErrorAction SilentlyContinue }
    $c = Invoke-MG $null @("clone", $srv.Url, $back) $Phase -TimeoutSec 7200
    $backGiant = Join-Path $back "giant.bin"
    $hashOk = ($c.Exit -eq 0) -and (Test-Path $backGiant) -and ((Get-QaHash $backGiant) -eq $giantHash)
    Rec $drill "minio" "giant-roundtrip" $hashOk $hashOk `
      "bigMB=$bigMB pushExit=$($p.Exit) cloneExit=$($c.Exit) hash-match=$hashOk"
    Remove-Item -Recurse -Force $back -ErrorAction SilentlyContinue
  } catch {
    Rec $drill "local" "peak-rss-mb" "" $false "unexpected error: $_"
  } finally {
    Stop-QaServer $srv
    if ($repo) { Remove-Item -Recurse -Force $repo -ErrorAction SilentlyContinue }  # reclaim the GB now
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
        Rec $drill $backend "push-mbs" "" $false "server unavailable: $_"; continue
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
      # Fast backends are held to the SLO floor. Cloud backends are WAN-bound, so their
      # floor is the MG_QA_CLOUD_MBS_FLOOR knob: 0 (default) records the number without
      # gating, so a link's real capability can be measured before a threshold is set.
      $floor = if ($isFast) { $PUSH_FLOOR_MBS } else { $CLOUD_FLOOR_MBS }
      $pushPass = ($r.Exit -eq 0) -and (($floor -le 0) -or ($pushMbs -ge $floor))
      $floorTxt = if ($isFast) { "$PUSH_FLOOR_MBS" }
                  elseif ($CLOUD_FLOOR_MBS -gt 0) { "$CLOUD_FLOOR_MBS (MG_QA_CLOUD_MBS_FLOOR)" }
                  else { "none (WAN, informational - set MG_QA_CLOUD_MBS_FLOOR to gate)" }
      # Cloud MB/s is a single sample over whatever the operator's uplink was doing at the
      # time - it is a regression tripwire, never a performance claim. Labelled inline so a
      # number lifted out of this TSV into a report carries its own caveat. Real throughput
      # figures require a host co-located with the region.
      $sampleNote = if ($isFast) { "" } else { " [link-bound single sample, not a perf claim]" }
      Rec $drill $backend "push-mbs" $pushMbs $pushPass "payloadMB=$payloadMB sec=$($r.Sec) floor=$floorTxt exit=$($r.Exit)$sampleNote"
      if ($r.Exit -ne 0) { continue }

      $clone = Join-Path $QA.Work "scale10-s5-clone-$backend"
      if (Test-Path $clone) { Remove-Item -Recurse -Force $clone -ErrorAction SilentlyContinue }
      $r = Invoke-MG $null @("clone", $srv.Url, $clone) $Phase -TimeoutSec 7200
      $cloneMbs = if ($r.Sec -gt 0) { [math]::Round($payloadMB / $r.Sec, 2) } else { 0 }
      $parity = ($r.Exit -eq 0) -and (Test-Path $clone) -and `
        ((Compare-Object (Get-QaTreeHashes $pushSrc) (Get-QaTreeHashes $clone) | Measure-Object).Count -eq 0)
      $cloneFloor = if ($isFast) { $PULL_FLOOR_MBS } else { $CLOUD_FLOOR_MBS }
      $clonePass = $parity -and (($cloneFloor -le 0) -or ($cloneMbs -ge $cloneFloor))
      $cloneFloorTxt = if ($isFast) { "$PULL_FLOOR_MBS" }
                       elseif ($CLOUD_FLOOR_MBS -gt 0) { "$CLOUD_FLOOR_MBS (MG_QA_CLOUD_MBS_FLOOR)" }
                       else { "none (WAN, informational)" }
      # parity is the load-bearing assertion here, not cloneMbs: byte-identical clone-back
      # is backend correctness and holds regardless of link quality.
      Rec $drill $backend "clone-mbs" $cloneMbs $clonePass "parity=$parity sec=$($r.Sec) floor=$cloneFloorTxt exit=$($r.Exit)$sampleNote"
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
  # scale10-* covers every drill's scratch; drills suffix the phase name when starting
  # servers (10_scale-S1-minio), so match those data dirs too.
  Invoke-QaTeardown $Phase @("scale10-*", "server-*-10_scale*")
  if ($QA.PurgeFixtures -and -not $QA.KeepScratch) {
    $sf = Join-Path $QA.Fixtures "scale"
    if (Test-Path $sf) { Remove-Item -Recurse -Force $sf -ErrorAction SilentlyContinue }
    Write-QaLog $Phase "MG_QA_PURGE_FIXTURES=1 - deleted generated scale fixtures"
  }
}

# ---------------------------------------------------------------------------
# MG_QA_DRILLS lets a killed/resumed run finish just the drills it lost (each
# drill builds its own fixtures, so any subset is valid). Empty = all. e.g. "S4,S5".
$only = ($env:MG_QA_DRILLS -split "," | ForEach-Object { $_.Trim().ToUpper() } | Where-Object { $_ })
function _Want([string]$s) { -not $only -or ($only -contains $s) }
try {
  if (_Want "S1") { Drill-S1-Concurrency }
  if (_Want "S2") { Drill-S2-Churn }
  if (_Want "S3") { Drill-S3-Conflicts }
  if (_Want "S4") { Drill-S4-ResourcePressure }
  if (_Want "S5") { Drill-S5-ThroughputDedup }
} catch {
  Write-QaLog $Phase "UNHANDLED phase error: $_"
  $script:AllPass = $false
} finally {
  Invoke-ScaleTeardown
}

Exit-QaPhase $Phase (-not $script:AllPass)
