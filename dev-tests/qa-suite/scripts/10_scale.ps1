# Phase 10 - scale & aggression suite. ASCII-only, PS 5.1 compatible.
# Opt-in: runs under `-Phases 10`, and is appended to the default set only when
# MG_QA_TIER=SCALE (run_all.ps1). Sizes/counts come from the SCALE knobs in config.ps1
# ($QA.Scale, .FileCount, .Concurrency, .ChurnCommits, .CloudMaxMB, .DiskBudgetGB,
#  .RssPrivateCeilMB - S4 gates private bytes; .RssCeilMB bounds the working-set report).
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
# Seconds to idle between S5 backends. S5 is the only drill that runs multi-GB
# payloads back-to-back across backends, and 186cb4d made that local-then-MinIO
# without a pause: ~22 GB through the host I/O path in minutes. 0 disables.
$S5_COOLDOWN_SEC = [int]($env:MG_QA_S5_COOLDOWN_SEC | ForEach-Object { if ($_) { $_ } else { 60 } })
# Cloud backends are WAN-bound, so their throughput floor is an operator-set knob rather
# than a fixed SLO. 0 (default) records the number without gating - set MG_QA_CLOUD_MBS_FLOOR
# once a link's real capability has been measured.
$CLOUD_FLOOR_MBS = $QA.CloudMbsFloor

# ---- observed-throughput regression floor (fast backends only) --------------
#
# The SLO floors above are PRODUCT requirements, not measurements, and on this
# machine they sit ~20x below what the code actually does: local pushed
# 311.86 MB/s against a 14.22 MB/s floor. A regression that made push twenty
# times slower would still have passed every S5 run. Nothing anywhere else
# detects a push/clone throughput regression - `baselines\perf.tsv` covers only
# local `add` and `commit`.
#
# So: when baselines\scale.tsv exists, a fast backend must clear BOTH the SLO
# and half of its own worst observed throughput. Promoted the same way as the
# perf baseline - an operator copies numbers from a known-good campaign, so the
# harness can never quietly re-baseline itself onto a regression it just
# measured. When the file is absent this is a no-op and only the SLO applies,
# which is the pre-2026-08-20 behaviour.
#
# 50% is deliberately loose. These numbers move with page cache, the host's
# filesystem filter and disk state; the job here is to catch a 2x-and-worse
# collapse, not to police normal variance. Cloud backends are excluded on
# purpose - they are WAN-bound single samples (see $CLOUD_FLOOR_MBS), and a
# link having a bad day is not a product regression.
#
# A BASELINE IS ONLY VALID FOR THE TOPOLOGY IT WAS MEASURED ON. The minio rows
# were re-derived on 2026-08-21 when the S3 backend moved from a Docker/WSL2
# container to a NATIVE Windows Silo process writing to NTFS. The old numbers
# were 57.2 / 57.47 (floors 28.6 / 28.74); native measured 283.91 / 188.57 in
# 20260821-ga8 — roughly 5x. Left alone, that gate would have accepted an 80%
# throughput collapse as healthy, which is worse than having no gate at all
# because it reads as coverage.
#
# The local rows are deliberately UNCHANGED: `local` is filesystem storage and
# never went through the container, and ga8 measured 336.81 / 188.60 — above the
# recorded worst, so 311.86 / 174.83 remain the worst observed.
#
# The minio rows are a SINGLE native sample. That is acceptable only because the
# 50% fraction is this loose: it catches a 2x collapse, not variance. Promote a
# lower number here if a later clean campaign observes one — worst-observed is
# the contract, so the file should only ever move DOWN except on a topology
# change like this one.
#
# Format (tab-separated, same shape as baselines\perf.tsv):
#   backend  metric     value
#   minio    push-mbs   283.91
$SCALE_BASELINE = Join-Path $QA.Root "baselines\scale.tsv"
$SCALE_FLOOR_FRACTION = 0.5
$script:ScaleBaseRows = $null
if (Test-Path $SCALE_BASELINE) {
  $script:ScaleBaseRows = @(Import-Csv $SCALE_BASELINE -Delimiter "`t")
  Write-QaLog $Phase "scale baseline loaded: $SCALE_BASELINE ($($script:ScaleBaseRows.Count) rows, floor=$([int]($SCALE_FLOOR_FRACTION*100))% of observed)"
} else {
  Write-QaLog $Phase "no scale baseline at $SCALE_BASELINE - throughput gated on the SLO floor only (promote a green campaign's numbers to enable regression detection)"
}

# Returns 0 when there is no baseline row, so callers can always Max() it in.
function Get-ObservedFloor([string]$Backend, [string]$Metric) {
  if (-not $script:ScaleBaseRows) { return 0.0 }
  $m = $script:ScaleBaseRows | Where-Object { $_.backend -eq $Backend -and $_.metric -eq $Metric } | Select-Object -First 1
  if (-not $m) { return 0.0 }
  $v = 0.0
  if (-not [double]::TryParse($m.value, [ref]$v)) { return 0.0 }
  return [math]::Round($v * $SCALE_FLOOR_FRACTION, 2)
}

# Backends S1 exercises. Fast local-ish backends always; billed ones only when the
# operator selected them (Start-QaServer SKIPs the rest with the not-selected marker).
function Get-ScaleBackends {
  $sel = @("local", "minio") + @($QA.Backends | Where-Object { $_ -in @("aws", "azure", "gcs") })
  return @($sel | Where-Object { $_ -eq "local" -or $QA.Backends -contains $_ } | Select-Object -Unique)
}

function Rec([string]$Drill, [string]$Backend, [string]$Metric, $Value, $Pass, [string]$Detail) {
  Write-QaRow $TSV @("drill", "backend", "metric", "value", "pass", "detail") `
    @($Drill, $Backend, $Metric, $Value, $Pass, $Detail)
  # ERROR (the drill blew up before measuring anything - see the "unexpected error"
  # catches below) is deliberately NOT folded into FAIL: it must not read as a
  # product defect, and must not flip $script:AllPass (that would force the phase's
  # own FAIL verdict via Exit-QaPhase's $ExtraFail and defeat the whole distinction).
  $tag = if ("$Pass" -eq "SKIP") { "SKIP" } elseif ("$Pass" -eq "ERROR") { "ERROR" } elseif ($Pass) { "PASS" } else { "FAIL" }
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
    $fault = Write-QaFault "$drill-$backend" $_
    Rec $drill $backend "clones-converge" "" "ERROR" $fault
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
    #
    # The FIRST block is excluded as warm-up, and that is the whole point of this shape.
    # The metric used to be last/first, which reads only 2 of N points and puts the
    # noisiest one in the denominator. Campaign 20260730-camp1 measured
    # 108.9, 240.5, 332.7, 428.6, 529.9s: blocks 2..5 grow almost perfectly linearly
    # (+92, +96, +101s) while block 1 sits 2.2x below block 2 - a cold page cache on a
    # freshly written 32 MB blob, not a history effect. last/first read 4.87x and failed;
    # the same run over blocks 2..5 reads 2.20x. The 2026-07-27 reference behaves
    # identically (210, 318, 444s -> 2.11x endpoint, 1.40x excluding warm-up).
    #
    # The cap stays at 3.0. Raising it to absorb 4.87 would have baselined away whatever
    # signal is really in there; excluding a measurement that never described history
    # growth is a different act, and it is the one that makes the number mean what the
    # gate claims it means.
    #
    # Every block is still printed, warm-up included and labelled, so the exclusion is
    # visible on every run rather than being a silent narrowing (the dead-gate lesson).
    if ($blockTimes.Count -ge 3) {
      $warmup = [double]$blockTimes[0]
      $gatedBlocks = @($blockTimes | Select-Object -Skip 1)
      $first = [double]$gatedBlocks[0]
      $last = [double]$gatedBlocks[-1]
      $ratio = if ($first -gt 0) { [math]::Round($last / $first, 2) } else { 0 }
      $slopeOk = ($first -le 0) -or ($ratio -le 3.0)
      Rec $drill "local" "churn-cost-slope" $ratio $slopeOk `
      ("blocks=$($blockTimes -join ',')s warmup-excluded=${warmup}s gated=$($gatedBlocks.Count)/$($blockTimes.Count) first=${first}s last=${last}s ratio=${ratio}x cap=3.0x")
    }
    else {
      # <3 blocks means at most one gated block after dropping warm-up, and a ratio
      # needs two. Report it as unmeasured rather than emitting a ratio computed from
      # one point, or worse passing silently: a gate whose PASS is compatible with
      # "measured nothing" is the defect this suite has now found nine times.
      Rec $drill "local" "churn-cost-slope" 0 $false `
      ("insufficient blocks to measure slope: got $($blockTimes.Count), need >=3 (100 commits per block, ChurnCommits=$iters)")
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
    $fault = Write-QaFault $drill $_
    Rec $drill "local" "churn" "" "ERROR" $fault
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
    $fault = Write-QaFault $drill $_
    Rec $drill "local" "no-data-loss" "" "ERROR" $fault
  }
}

# ---------------------------------------------------------------------------
# S4: resource pressure. A multi-GB blob + the many-files corpus; peak client/server RSS
# must stay under the ceiling (streaming, no load-whole-file) and no OOM. Disk-bounded.
# ---------------------------------------------------------------------------
function Drill-S4-ResourcePressure {
  $drill = "S4-resource"
  if (-not $diskOk) { Rec $drill "local" "peak-private-mb" "" "SKIP" "insufficient free disk (${freeGB}GB < $($QA.DiskBudgetGB)GB)"; return }
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
    # Gated on PRIVATE bytes, not working set. The question this drill asks is "did the
    # client buffer the blob instead of streaming it", and private bytes is the metric
    # that answers it: working set on Windows also counts mapped-file and page-cache
    # pages, so it moved 1223 -> 2931 -> 3697 MB across three identical runs while
    # private stayed within 689-707. Gating the noisy one meant this drill could fail on
    # a loaded machine for reasons that have nothing to do with MediaGit. Working set is
    # still measured and reported - it is useful triage - it just does not decide.
    # See $QA.RssPrivateCeilMB in config.ps1 for how the ceiling was chosen.
    $clientPrivOk = ($m.ClientPrivatePeakMB -le $QA.RssPrivateCeilMB) -and ($m.ClientPrivatePeakMB -gt 0)
    # This drill runs no server, so Measure-PeakRSS reports "n/a" rather than 0 - a real
    # not-applicable, not a measurement of zero. Treat n/a as passing; gate any actual
    # number. (It used to record 0 and compare it to the ceiling, which meant a server-side
    # leak on a drill that DID run a server would also have passed silently.)
    $serverPrivOk = ($m.ServerPrivatePeakMB -eq "n/a") -or ([double]$m.ServerPrivatePeakMB -le $QA.RssPrivateCeilMB)
    $pass = $clientPrivOk -and $serverPrivOk -and $noOom -and $fsckOk
    Rec $drill "local" "peak-private-mb" $m.ClientPrivatePeakMB $pass `
      ("bigMB=$bigMB clientPrivate=$($m.ClientPrivatePeakMB)MB serverPrivate=$($m.ServerPrivatePeakMB)MB " +
       "privCeil=$($QA.RssPrivateCeilMB)MB clientPeakWS=$($m.ClientPeakMB)MB serverPeakWS=$($m.ServerPeakMB)MB " +
       "(working set reported, NOT gated - see config.ps1) " +
       "commitExit=$($commit.Exit) fsck=$fsckOk samples=$(Split-Path $m.SamplesTsv -Leaf)")
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
    $fault = Write-QaFault $drill $_
    Rec $drill "local" "peak-private-mb" "" "ERROR" $fault
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
  # Get-ScaleBackends, not $QA.Backends: the raw list defaults to
  # "minio,aws,azure,gcs" with no "local", so S5 silently measured no local
  # throughput at all while S1 (which uses the helper) did. That made the two
  # SLO floors below -- PUSH_FLOOR_MBS/PULL_FLOOR_MBS, the "fast backends are
  # held to the SLO" branch -- reachable only through minio, so a local-path
  # throughput regression had nothing to fail against. Same helper as S1 now,
  # so "which backends does scale cover" has one answer.
  $s5First = $true
  foreach ($backend in (Get-ScaleBackends)) {
    # Let the page cache and the host's filesystem filter drain before the next
    # multi-GB payload. Between backends only, so a single-backend run is unaffected.
    if (-not $s5First -and $S5_COOLDOWN_SEC -gt 0) {
      Write-QaLog $Phase "S5 cooldown ${S5_COOLDOWN_SEC}s before [$backend] (MG_QA_S5_COOLDOWN_SEC=0 disables)"
      Start-Sleep -Seconds $S5_COOLDOWN_SEC
    }
    $s5First = $false

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
      $obsFloor = Get-ObservedFloor $backend "push-mbs"
      $floor = if ($isFast) { [math]::Max($PUSH_FLOOR_MBS, $obsFloor) } else { $CLOUD_FLOOR_MBS }
      $pushPass = ($r.Exit -eq 0) -and (($floor -le 0) -or ($pushMbs -ge $floor))
      $floorTxt = if ($isFast -and $obsFloor -gt $PUSH_FLOOR_MBS) { "$floor (observed-regression floor; SLO is $PUSH_FLOOR_MBS)" }
                  elseif ($isFast) { "$PUSH_FLOOR_MBS (SLO)" }
                  elseif ($CLOUD_FLOOR_MBS -gt 0) { "$CLOUD_FLOOR_MBS (MG_QA_CLOUD_MBS_FLOOR)" }
                  else { "none (WAN, informational - set MG_QA_CLOUD_MBS_FLOOR to gate)" }
      # Cloud MB/s is a single sample over whatever the operator's uplink was doing at the
      # time - it is a regression tripwire, never a performance claim. Labelled inline so a
      # number lifted out of this TSV into a report carries its own caveat. Real throughput
      # figures require a host co-located with the region.
      $sampleNote = if ($isFast) { "" } else { " [link-bound single sample, not a perf claim]" }
      Rec $drill $backend "push-mbs" $pushMbs $pushPass "payloadMB=$payloadMB sec=$($r.Sec) floor=$floorTxt exit=$($r.Exit)$sampleNote"

      # Did that push actually use the cloud-pack fast path? Asked directly,
      # because push-mbs above cannot answer it. In ga11 a single transient status
      # on one pack dropped the whole push to the per-chunk proxy path - 8.9x
      # slower, still correct, still exit 0 - and the only reason anyone noticed
      # is that Azure landed at 0.98 against a 1.0 floor. A faster link that day
      # and the same regression ships unseen.
      #
      # Read BEFORE the clone below so clone traffic cannot enter the counts.
      # ga8 measured all five backends (local included) at offered == completed
      # with zero proxy PUTs, so this needs no per-backend capability table.
      $fp = Get-QaPackFastPathCounts $srv.OutLog
      if ($null -eq $fp) {
        Rec $drill $backend "packfastpath" "" "SKIP" "server log unreadable at $($srv.OutLog)"
      } elseif ($fp.Offered -le 0) {
        # No URLs minted at all: the fast path was never on offer, so there is
        # nothing to have degraded FROM. SKIP is the honest verdict - passing
        # here would be a gate that cannot fail.
        Rec $drill $backend "packfastpath" $fp.ChunkPuts "SKIP" "fast path not offered (no pack upload URLs minted)"
      } else {
        # Completed must reach Offered. The first version of this gate asked for
        # Completed > 0 and ChunkPuts == 0, and 20260821-s5check proved that
        # unfailable on GCS: 4 pack pushes FAILED, only 18 of 32 packs completed,
        # and the gate said PASS - because GCS's fallback uses presigned
        # per-chunk URLs straight to the bucket, which our server never sees as
        # "PUT chunk". ChunkPuts is blind to exactly one backend, and it is the
        # backend whose blindness was already on record from ga11.
        #
        # Offered vs Completed is the backend-INDEPENDENT signal: a pack that was
        # offered a URL and never registered did not land, whatever route the
        # fallback then took. Both halves are kept, because they fail on
        # different things - Completed<Offered catches a fast path that broke,
        # ChunkPuts>0 catches one that was abandoned for the proxy route.
        #
        # >= not ==: a retried pack can register more than once, and that is not
        # a failure. Verified against every run on record - ga8 (all 5 backends)
        # and s5check local/minio all sit exactly at Offered == Completed.
        #
        # ChunkPuts is REPORTED but NOT gated on, after this gate went wrong in
        # both directions inside one day:
        #   v1  Completed>0 and ChunkPuts==0  - UNFAILABLE on gcs (s5check: 4 pack
        #       pushes failed, 18/32 completed, gate said PASS)
        #   v2  Completed>=Offered and ChunkPuts==0 - FALSE FAILURE on local
        #       (cloudcheck: 173/173 packs, 391.24 MB/s - the fastest local push
        #       on record - failed on a SINGLE stray proxy PUT)
        #
        # A lone proxy PUT is ordinary traffic, not a fallback. A real fallback
        # moves every remaining chunk that way - s5check measured 65, 724 and
        # 2,284 - and it cannot happen without packs failing to register, which
        # Completed<Offered already catches. Checked against all 12 datasets on
        # record (ga8 x5, s5check x5, the reconstructed degraded log, and
        # cloudcheck local): Completed>=Offered alone is correct on every one,
        # so the second condition earned nothing and cost a false failure.
        #
        # A gate that fires on healthy runs gets ignored, which is the same
        # outcome as one that never fires.
        $fpPass = ($fp.Completed -ge $fp.Offered)
        Rec $drill $backend "packfastpath" $fp.ChunkPuts $fpPass ("packsOffered={0} packsCompleted={1} perChunkProxyPUTs={2}{3}" -f `
          $fp.Offered, $fp.Completed, $fp.ChunkPuts, $(if ($fpPass) { "" } else { " - FAST PATH BROKE: $($fp.Offered - $fp.Completed) of $($fp.Offered) packs never registered; the push fell back to the per-chunk path and will be multi-x slower regardless of the MB/s above" }))
      }

      if ($r.Exit -ne 0) { continue }

      $clone = Join-Path $QA.Work "scale10-s5-clone-$backend"
      if (Test-Path $clone) { Remove-Item -Recurse -Force $clone -ErrorAction SilentlyContinue }
      $r = Invoke-MG $null @("clone", $srv.Url, $clone) $Phase -TimeoutSec 7200
      $cloneMbs = if ($r.Sec -gt 0) { [math]::Round($payloadMB / $r.Sec, 2) } else { 0 }
      $parity = ($r.Exit -eq 0) -and (Test-Path $clone) -and `
        ((Compare-Object (Get-QaTreeHashes $pushSrc) (Get-QaTreeHashes $clone) | Measure-Object).Count -eq 0)
      $obsCloneFloor = Get-ObservedFloor $backend "clone-mbs"
      $cloneFloor = if ($isFast) { [math]::Max($PULL_FLOOR_MBS, $obsCloneFloor) } else { $CLOUD_FLOOR_MBS }
      $clonePass = $parity -and (($cloneFloor -le 0) -or ($cloneMbs -ge $cloneFloor))
      $cloneFloorTxt = if ($isFast -and $obsCloneFloor -gt $PULL_FLOOR_MBS) { "$cloneFloor (observed-regression floor; SLO is $PULL_FLOOR_MBS)" }
                       elseif ($isFast) { "$PULL_FLOOR_MBS (SLO)" }
                       elseif ($CLOUD_FLOOR_MBS -gt 0) { "$CLOUD_FLOOR_MBS (MG_QA_CLOUD_MBS_FLOOR)" }
                       else { "none (WAN, informational)" }
      # parity is the load-bearing assertion here, not cloneMbs: byte-identical clone-back
      # is backend correctness and holds regardless of link quality.
      Rec $drill $backend "clone-mbs" $cloneMbs $clonePass "parity=$parity sec=$($r.Sec) floor=$cloneFloorTxt exit=$($r.Exit)$sampleNote"
      Remove-Item -Recurse -Force $clone -ErrorAction SilentlyContinue
    } catch {
      $fault = Write-QaFault "$drill-$backend" $_
      Rec $drill $backend "phase" "" "ERROR" $fault
    } finally {
      Stop-QaServer $srv
    }
  }
  } catch {
    $fault = Write-QaFault "$drill-setup" $_
    Rec $drill "local" "setup" "" "ERROR" $fault
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
# Announce a filtered run. A stale MG_QA_DRILLS left in the shell would otherwise
# reduce this phase to one drill and still report it green - the variable is a
# resume/reproduction knob, and nothing about a shortened phase is visible in the
# gate list unless it says so. Same guard as 07_abuse.
if ($only) {
    Write-QaLog $Phase ("MG_QA_DRILLS is set - running ONLY: " + ($only -join ",") +
        ". This is a PARTIAL phase; a campaign must run with it unset.")
}
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
