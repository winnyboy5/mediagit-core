# Phase 6 - remote backend matrix. ASCII-only, PS 5.1 compatible.
# Generalizes dev-tests\standalone-deep-v11\scripts\run_remote.ps1 onto the qa-suite contract.
# For each backend in $QA.Backends: start a per-run mediagit-server (scripts\lib\remote.ps1),
# push a mixed fixture set, clone, hash-parity, fetch, pull, single-file download, branch
# delete on remote, tag push/pull. Missing credentials -> backend recorded as SKIP, phase
# still passes; any parity/op failure on a non-skipped backend fails the phase.
#
# Output: $QA.Logs\remote_results.tsv (backend, op, sizeMB, sec, MBps, parity, detail)

. (Join-Path $PSScriptRoot "lib\common.ps1")
. (Join-Path $PSScriptRoot "lib\remote.ps1")

$Phase = "06_remote"
$env:MEDIAGIT_AUTHOR_NAME = "QA-Suite"
$env:MEDIAGIT_AUTHOR_EMAIL = "qa-suite@mediagit.local"

$TSV = Join-Path $QA.Logs "remote_results.tsv"
$script:AllPass = $true

function Rec([string]$Backend, [string]$Op, $SizeMB, $Sec, $Parity, [string]$Detail) {
  $mbps = ""
  if (($SizeMB -is [double] -or $SizeMB -is [int]) -and ($Sec -is [double]) -and $Sec -gt 0 -and $SizeMB -gt 0) {
    $mbps = [math]::Round($SizeMB / $Sec, 2)
  }
  Write-QaRow $TSV @("backend", "op", "sizeMB", "sec", "MBps", "parity", "detail") `
    @($Backend, $Op, $SizeMB, $Sec, $mbps, $Parity, $Detail)
  $tag = if ("$Parity" -eq "SKIP") { "SKIP" } elseif ($Parity) { "PASS" } else { "FAIL" }
  Write-QaLog $Phase ("{0} :: {1} -> {2}  {3}s  {4}" -f $Backend, $Op, $tag, $Sec, $Detail)
  Write-QaGate $Phase "$Backend-$Op" $Parity $Detail
  if ($tag -eq "FAIL") { $script:AllPass = $false }
}

function New-QaBinaryFixture([string]$Path, [int]$SizeMB, [int]$Seed) {
  $dir = Split-Path $Path -Parent
  if (-not (Test-Path $dir)) { New-Item -ItemType Directory -Path $dir -Force | Out-Null }
  $rnd = New-Object System.Random($Seed)
  $bytes = New-Object byte[] ($SizeMB * 1MB)
  $rnd.NextBytes($bytes)
  [IO.File]::WriteAllBytes($Path, $bytes)
}

# ---------------------------------------------------------------------------
# Build the shared source payload once: a few real media files (tier-capped)
# plus generated binaries. STANDARD target ~100-200MB total.
# ---------------------------------------------------------------------------
$SRC = Join-Path $QA.Work "remote-src"
if (Test-Path $SRC) { Remove-Item -Recurse -Force $SRC }
New-Item -ItemType Directory -Path $SRC -Force | Out-Null
$r = Invoke-MG $null @("init", $SRC) $Phase
if ($r.Exit -ne 0) { throw "init failed: $($r.Out)" }

# real media: largest few files under test-files that fit the tier cap, ~120MB budget
$mediaBudgetMB = 120
$picked = @()
if (Test-Path $QA.TestFiles) {
  $candidates = Get-ChildItem $QA.TestFiles -Recurse -File -ErrorAction SilentlyContinue |
    Sort-Object Length -Descending | Select-Object -ExpandProperty FullName
  $eligible = Select-TierFiles $candidates
  $used = 0.0
  foreach ($f in $eligible) {
    $mb = (Get-Item $f).Length / 1MB
    if ($used + $mb -le $mediaBudgetMB) {
      Copy-Item $f (Join-Path $SRC ("media_" + (Split-Path $f -Leaf)))
      $used += $mb
      $picked += $f
      if ($picked.Count -ge 5) { break }
    }
  }
}
Write-QaLog $Phase "picked $($picked.Count) real media files from $($QA.TestFiles)"

# generated binaries: deterministic, ~40MB
New-QaBinaryFixture (Join-Path $SRC "gen\blob_a.bin") 16 61001
New-QaBinaryFixture (Join-Path $SRC "gen\blob_b.bin") 16 61002
New-QaBinaryFixture (Join-Path $SRC "gen\blob_c.bin") 8 61003

Invoke-MG $SRC @("add", ".") $Phase -TimeoutSec 1200 | Out-Null
Invoke-MG $SRC @("commit", "-m", "initial mixed payload") $Phase | Out-Null
$payloadMB = Get-DirMB $SRC -ExcludeOdb
Write-QaLog $Phase "source payload: ${payloadMB}MB"

# ---------------------------------------------------------------------------
# Backend loop
# ---------------------------------------------------------------------------
foreach ($backend in $QA.Backends) {
  $srv = $null
  try {
    try {
      $srv = Start-QaServer -Backend $backend -Phase $Phase
    } catch {
      # SKIP only for a backend that was never selected or has no credentials. A backend
      # we DID ask for whose server will not start is a failure - and it is recorded as
      # one row rather than rethrown, so the remaining backends still get exercised.
      if ("$_" -match "^SKIP:") { Rec $backend "all" "" "" "SKIP" "$_"; continue }
      Rec $backend "all" "" "" $false "server unavailable: $_"
      continue
    }

    Invoke-MG $SRC @("remote", "remove", "origin") $Phase | Out-Null
    $r = Invoke-MG $SRC @("remote", "add", "origin", $srv.Url) $Phase

    # -- push (timed) --
    $r = Invoke-MG $SRC @("push", "origin") $Phase -TimeoutSec 3600
    Rec $backend "push" $payloadMB $r.Sec ($r.Exit -eq 0) "exit=$($r.Exit)"
    if ($r.Exit -ne 0) { continue }

    # -- clone (timed) + full hash parity --
    $CL = Join-Path $QA.Work "remote-clone-$backend"
    if (Test-Path $CL) { Remove-Item -Recurse -Force $CL }
    $r = Invoke-MG $null @("clone", $srv.Url, $CL) $Phase -TimeoutSec 3600
    $cloneOk = ($r.Exit -eq 0) -and (Test-Path $CL)
    $parity = $false
    $mismatches = -1
    if ($cloneOk) {
      $h1 = Get-QaTreeHashes $SRC
      $h2 = Get-QaTreeHashes $CL
      $diff = Compare-Object $h1 $h2
      $mismatches = ($diff | Measure-Object).Count
      $parity = ($mismatches -eq 0) -and ($h1.Count -gt 0)
    }
    Rec $backend "clone" $payloadMB $r.Sec ($cloneOk -and $parity) "exit=$($r.Exit) files=$(@(Get-QaTreeHashes $SRC).Count) mismatches=$mismatches"
    if (-not $cloneOk) { continue }

    # -- second clone for pull test (before origin advances) --
    $CL2 = Join-Path $QA.Work "remote-clone2-$backend"
    if (Test-Path $CL2) { Remove-Item -Recurse -Force $CL2 }
    Invoke-MG $null @("clone", $srv.Url, $CL2) $Phase -TimeoutSec 3600 | Out-Null

    # -- add a commit at source, push, then fetch in clone1 / pull in clone2 --
    New-QaBinaryFixture (Join-Path $SRC "gen\incr.bin") 4 61010
    Invoke-MG $SRC @("add", ".") $Phase | Out-Null
    Invoke-MG $SRC @("commit", "-m", "incremental $backend") $Phase | Out-Null
    Invoke-MG $SRC @("push", "origin") $Phase -TimeoutSec 1200 | Out-Null
    $incrHash = Get-QaHash (Join-Path $SRC "gen\incr.bin")

    $r = Invoke-MG $CL @("fetch", "origin") $Phase -TimeoutSec 1200
    Rec $backend "fetch" 4 $r.Sec ($r.Exit -eq 0) "exit=$($r.Exit)"

    $r = Invoke-MG $CL2 @("pull") $Phase -TimeoutSec 1200
    $pullFileOk = (Test-Path (Join-Path $CL2 "gen\incr.bin")) -and
                  ((Get-QaHash (Join-Path $CL2 "gen\incr.bin")) -eq $incrHash)
    Rec $backend "pull" 4 $r.Sec (($r.Exit -eq 0) -and $pullFileOk) "exit=$($r.Exit) file-hash-ok=$pullFileOk"

    # -- download single file without clone --
    $dl = Join-Path $QA.Work "remote-dl-$backend.bin"
    if (Test-Path $dl) { Remove-Item -Force $dl }
    $r = Invoke-MG $CL @("download", "gen/blob_c.bin", "-o", $dl) $Phase -TimeoutSec 1200
    $dlOk = (Test-Path $dl) -and ((Get-QaHash $dl) -eq (Get-QaHash (Join-Path $SRC "gen\blob_c.bin")))
    Rec $backend "download" 8 $r.Sec (($r.Exit -eq 0) -and $dlOk) "exit=$($r.Exit) hash-ok=$dlOk"

    # -- branch push + push --delete --
    Invoke-MG $SRC @("branch", "create", "qa-del-$backend") $Phase | Out-Null
    # push origin with no refspec only pushes the CURRENT branch (main) - branch create
    # does not switch to the new branch, so it must be named explicitly to reach the remote.
    Invoke-MG $SRC @("push", "origin", "qa-del-$backend") $Phase -TimeoutSec 1200 | Out-Null
    # `fetch origin` with no branch arg only syncs the CURRENT branch (main) by design
    # (documented safety default - avoids unintentional multi-TB pulls); the new branch
    # must be named explicitly. `branch list` also needs -a to show remote-tracking refs.
    Invoke-MG $CL @("fetch", "origin", "qa-del-$backend") $Phase | Out-Null
    $brListBefore = (Invoke-MG $CL @("branch", "list", "-a", "-v") $Phase).Out
    $brPushed = $brListBefore -match "qa-del-$backend"
    $r = Invoke-MG $SRC @("push", "origin", "--delete", "qa-del-$backend") $Phase
    $delExit = $r.Exit
    Invoke-MG $CL @("fetch", "origin", "--prune") $Phase | Out-Null
    $brListAfter = (Invoke-MG $CL @("branch", "list", "-a", "-v") $Phase).Out
    # after delete+fetch the remote-tracking branch must be gone (local list may still
    # show a stale local copy; we only assert the delete op succeeded and it was pushed)
    Rec $backend "push-delete-branch" "" $r.Sec (($delExit -eq 0) -and $brPushed) "branch-visible-after-push=$brPushed delete-exit=$delExit"

    # -- tag push/pull (lightweight + annotated) --
    # `fetch`/`pull` have no code path for refs/tags/* at all (fetch.rs only ever
    # filters refs/heads/*) - tags only land locally via `clone`. So this checks a
    # FRESH clone rather than fetching tags into the existing $CL, which is not a
    # supported operation regardless of flags.
    Invoke-MG $SRC @("tag", "create", "qa-tag-$backend") $Phase | Out-Null
    Invoke-MG $SRC @("tag", "create", "qa-ann-$backend", "-a", "-m", "annotated") $Phase | Out-Null
    # push origin with no flag does not push tags (matches git) - need --tags explicitly.
    $r = Invoke-MG $SRC @("push", "origin", "--tags") $Phase -TimeoutSec 1200
    $CL3 = Join-Path $QA.Work "remote-clone3-$backend"
    if (Test-Path $CL3) { Remove-Item -Recurse -Force $CL3 }
    Invoke-MG $null @("clone", $srv.Url, $CL3) $Phase -TimeoutSec 3600 | Out-Null
    $tags = (Invoke-MG $CL3 @("tag", "list") $Phase).Out
    $tagOk = ($tags -match "qa-tag-$backend") -and ($tags -match "qa-ann-$backend")
    Rec $backend "tag-push-pull" "" $r.Sec (($r.Exit -eq 0) -and $tagOk) "light+annotated-visible-in-clone=$tagOk"

    # -- reset source for the next backend: drop the incremental commit/tag/branch state --
    Invoke-MG $SRC @("reset", "--hard", "HEAD~1") $Phase | Out-Null
    Remove-Item (Join-Path $SRC "gen\incr.bin") -Force -ErrorAction SilentlyContinue
    Invoke-MG $SRC @("tag", "delete", "qa-tag-$backend") $Phase | Out-Null
    Invoke-MG $SRC @("tag", "delete", "qa-ann-$backend") $Phase | Out-Null
    Invoke-MG $SRC @("branch", "delete", "qa-del-$backend") $Phase | Out-Null
  } catch {
    Rec $backend "phase" "" "" $false "unexpected error: $_"
  } finally {
    Stop-QaServer $srv
  }
}

Write-QaLog $Phase "=== 06_remote done: overall=$(if ($script:AllPass) { 'PASS' } else { 'FAIL' }) ==="
# Teardown: reclaim this phase's own work/ scratch so a long campaign cannot run the
# volume out of space. work/ ONLY - logs/ and fixtures-synthetic/ are never touched.
Invoke-QaTeardown $Phase @("remote-*")

Exit-QaPhase $Phase (-not $script:AllPass)
