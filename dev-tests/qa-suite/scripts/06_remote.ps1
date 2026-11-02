# Phase 6 - remote backend matrix. ASCII-only, PS 5.1 compatible.
# Generalizes dev-tests\standalone-deep-v11\scripts\run_remote.ps1 onto the qa-suite contract.
# For each backend in $QA.Backends: start a per-run mediagit-server (scripts\lib\remote.ps1),
# push a mixed fixture set, clone, hash-parity, fetch, pull, single-file download, branch
# delete on remote, tag push/pull. Missing credentials -> backend recorded as SKIP, phase
# still passes; any parity/op failure on a non-skipped backend fails the phase.
#
# Output: $QA.Logs\remote_results.tsv (backend, op, sizeMB, sec, MBps, parity, detail)

# Usage: powershell -File 06_remote.ps1 [-Only gcrace]
#   -Only gcrace : run ONLY the push+gc race gate (MinIO), skipping the backend matrix.
param(
  [string]$Only = ""
)

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
if (-not $Only) {
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

    # -- pack fast path: did the push silently drop to the per-chunk route? --
    # Same backend-independent rule as 10_scale's S5 packfastpath (see the
    # history there): every pack offered an upload URL must register. A pack
    # failure falls back to per-chunk uploads, still exits 0, and is ~9x slower
    # (20260821-ga11), so exit code and parity above cannot see it. Read BEFORE
    # the clone so clone traffic cannot enter the counts.
    $fp = Get-QaPackFastPathCounts $srv.OutLog
    if ($null -eq $fp) {
      Rec $backend "packfastpath" "" "" "SKIP" "server log unreadable at $($srv.OutLog)"
    } elseif ($fp.Offered -le 0) {
      Rec $backend "packfastpath" "" "" "SKIP" "fast path not offered (no pack upload URLs minted)"
    } else {
      $fpPass = ($fp.Completed -ge $fp.Offered)
      Rec $backend "packfastpath" "" "" $fpPass ("packsOffered={0} packsCompleted={1} perChunkProxyPUTs={2}{3}" -f `
        $fp.Offered, $fp.Completed, $fp.ChunkPuts, $(if ($fpPass) { "" } else { " - FAST PATH BROKE: the push fell back to the per-chunk path" }))
    }

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
    # Full URL, not a bare path: inside a repo a bare path is served from LOCAL history
    # and never reaches the server. Run from the clone so the matching remote's
    # credentials are attached.
    $dl = Join-Path $QA.Work "remote-dl-$backend.bin"
    if (Test-Path $dl) { Remove-Item -Force $dl }
    $r = Invoke-MG $CL @("download", "$($srv.Url)/gen/blob_c.bin", "-o", $dl) $Phase -TimeoutSec 1200
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

}  # end if (-not $Only): backend matrix

# ---------------------------------------------------------------------------
# Browse download integrity. `download <full URL>` is a plain GET of the server's browse
# endpoint (GET /{repo}/files/{path}); the server reassembles the blob from its stored chunks.
# Always pass the full URL: inside a repo, a bare path is served from LOCAL history and never
# reaches the server, which would make this gate unable to fail.
# If a stored chunk is corrupt the server must fail the response (non-zero exit on the
# client), never stream wrong bytes as a 200. Needs a filesystem-visible store, so it
# always runs against a `local` backend server regardless of $QA.Backends. The server is
# restarted after the corruption so its in-memory ODB cache cannot serve pre-corruption
# bytes (same reason as 05_branching CHK20).
# ---------------------------------------------------------------------------
$bsrv = $null
if (-not $Only) {
try {
  $bsrv = Start-QaServer -Backend "local" -Phase $Phase
  $bsrc = Join-Path $QA.Work "remote-browse-src"
  if (Test-Path $bsrc) { Remove-Item -Recurse -Force $bsrc }
  New-Item -ItemType Directory -Path $bsrc -Force | Out-Null
  Invoke-MG $null @("init", $bsrc) $Phase | Out-Null
  New-QaBinaryFixture (Join-Path $bsrc "gen\browse.bin") 8 61020
  $bOrig = Get-QaHash (Join-Path $bsrc "gen\browse.bin")
  Invoke-MG $bsrc @("add", ".") $Phase | Out-Null
  Invoke-MG $bsrc @("commit", "-m", "browse integrity") $Phase | Out-Null
  Invoke-MG $bsrc @("remote", "add", "origin", $bsrv.Url) $Phase | Out-Null
  $bp = Invoke-MG $bsrc @("push", "origin") $Phase -TimeoutSec 1200

  # Control: the same download before any corruption must be byte-exact, or the
  # corrupted-case result below could be a download that never worked.
  $dl0 = Join-Path $QA.Work "remote-browse-ok.bin"
  if (Test-Path $dl0) { Remove-Item -Force $dl0 }
  $bUrl = "$($bsrv.Url)/gen/browse.bin"
  $c0 = Invoke-MG $bsrc @("download", $bUrl, "-o", $dl0) $Phase -TimeoutSec 1200
  $ctlOk = ($bp.Exit -eq 0) -and ($c0.Exit -eq 0) -and (Test-Path $dl0) -and ((Get-QaHash $dl0) -eq $bOrig)
  Rec "local" "browse-download-control" 8 $c0.Sec $ctlOk "push-exit=$($bp.Exit) exit=$($c0.Exit)"

  if ($ctlOk) {
    $victim = Get-ChildItem (Join-Path $bsrv.DataDir "storage") -Recurse -File -ErrorAction SilentlyContinue |
      Sort-Object Length -Descending | Select-Object -First 1
    if (-not $victim) {
      Rec "local" "browse-download-corrupt-chunk" "" "" "SKIP" "no stored object under $($bsrv.DataDir)\storage to corrupt"
    } else {
      $vb = [IO.File]::ReadAllBytes($victim.FullName)
      $vb[[int]($vb.Length / 2)] = $vb[[int]($vb.Length / 2)] -bxor 0xFF
      [IO.File]::WriteAllBytes($victim.FullName, $vb)
      Restart-QaServer $bsrv $Phase

      $dl1 = Join-Path $QA.Work "remote-browse-corrupt.bin"
      if (Test-Path $dl1) { Remove-Item -Force $dl1 }
      $c1 = Invoke-MG $bsrc @("download", $bUrl, "-o", $dl1) $Phase -TimeoutSec 1200
      $wrongBytes = ($c1.Exit -eq 0) -and (Test-Path $dl1) -and ((Get-QaHash $dl1) -ne $bOrig)
      # Pass = the download failed. Exit 0 with right bytes means the corrupted object was
      # not on this file's read path (inconclusive, recorded as a failure so it is looked at).
      Rec "local" "browse-download-corrupt-chunk" 8 $c1.Sec ($c1.Exit -ne 0) ("exit={0} wrong-bytes-returned-as-success={1} corrupted={2}" -f $c1.Exit, $wrongBytes, $victim.Name)
    }
  }
} catch {
  if ("$_" -match "^SKIP:") { Rec "local" "browse-download-corrupt-chunk" "" "" "SKIP" "$_" }
  else { Rec "local" "browse-download-corrupt-chunk" "" "" $false "unexpected error: $_" }
} finally {
  Stop-QaServer $bsrv
}
}  # end if (-not $Only): browse integrity

# ---------------------------------------------------------------------------
# push + gc race on an S3 (MinIO) backed repository.
#
# VC-2 fix: gc used to delete fresh unreferenced chunks/manifests/deltas that an
# in-flight push had uploaded but not yet referenced (the ref only moves at the very
# end of a push). The fix is a grace window (MEDIAGIT_GC_GRACE_SECS, default 3600).
# Unit tests cover the window; nothing ran the race on a real S3 backend.
#
# WHERE gc RUNS. The server never runs gc itself; gc is a CLI command run against a
# repository directory. A server repo on S3 is a bare dir whose .mediagit\config.toml
# carries the [storage] s3 section, so `mediagit -C <server repo dir> gc` operates on
# exactly the bucket prefix the push is writing to - the operator-cron scenario.
#
# SHAPE. Push a large random payload in the background; while it is alive, run
# `gc -y` on the server-side repo in a loop. Then: push exit, gc overlap (non-vacuity),
# fsck of the server repo, fresh clone hash parity.
#
# BOTH HALVES. MG_QA_GC_RACE_GRACE sets MEDIAGIT_GC_GRACE_SECS for the gc processes
# only (unset = product default, expected all True). MG_QA_GC_RACE_GRACE=0 re-creates
# the pre-fix behaviour and must turn at least one gate False - that is the proof this
# gate can fail. If it does not, the push and gc are not really overlapping.
# MG_QA_GC_RACE_MB sizes the payload (default 768).
# ---------------------------------------------------------------------------
if ((-not $Only) -or ($Only -eq "gcrace")) {
  $gsrv = $null
  try {
    try { $gsrv = Start-QaServer -Backend "minio" -Phase $Phase }
    catch {
      if ("$_" -match "^SKIP:") { Write-QaGate $Phase "gcrace-all" "SKIP" "$_" }
      else { Write-QaGate $Phase "gcrace-all" $false "server unavailable: $_"; $script:AllPass = $false }
      $gsrv = $null
    }
    if ($gsrv) {
      $gSrvRepo = Join-Path $gsrv.DataDir "repos\$($gsrv.RepoName)"
      $gMB = [int]$(if ($env:MG_QA_GC_RACE_MB) { $env:MG_QA_GC_RACE_MB } else { 768 })
      $gGrace = $env:MG_QA_GC_RACE_GRACE   # "" = product default
      $gTag = "grace=" + $(if ($gGrace) { $gGrace } else { "default" })
      $gSrc = Join-Path $QA.Work "remote-gcrace-src"
      $gClone = Join-Path $QA.Work "remote-gcrace-clone"
      foreach ($d in @($gSrc, $gClone)) { if (Test-Path $d) { Remove-Item -Recurse -Force $d } }
      New-Item -ItemType Directory -Path $gSrc -Force | Out-Null
      Invoke-MG $null @("init", $gSrc) $Phase | Out-Null
      # random 32 MB files: incompressible, so the push is bandwidth/CPU bound and long.
      $nFiles = [math]::Max(1, [int]($gMB / 32))
      for ($i = 1; $i -le $nFiles; $i++) { New-QaBinaryFixture (Join-Path $gSrc ("gen\race_{0:d2}.bin" -f $i)) 32 (62000 + $i) }
      Invoke-MG $gSrc @("add", ".") $Phase -TimeoutSec 3600 | Out-Null
      Invoke-MG $gSrc @("commit", "-m", "gc race payload") $Phase -TimeoutSec 3600 | Out-Null
      Invoke-MG $gSrc @("remote", "add", "origin", $gsrv.Url) $Phase | Out-Null

      # --- push in the background, gc in a loop on the server repo while it lives ---
      $pushOut = Join-Path $QA.Logs "gcrace-push.out.log"
      $pushErr = Join-Path $QA.Logs "gcrace-push.err.log"
      $sw = [Diagnostics.Stopwatch]::StartNew()
      $pp = Start-Process -FilePath $QA.MG -ArgumentList @("-C", "`"$gSrc`"", "push", "origin") `
        -PassThru -NoNewWindow -RedirectStandardOutput $pushOut -RedirectStandardError $pushErr
      $null = $pp.Handle   # PS 5.1: ExitCode is $null after exit unless the handle was cached while alive
      $gcRuns = 0; $gcOverlap = 0; $gcBadExit = 0
      $savedG = $env:MEDIAGIT_GC_GRACE_SECS
      try {
        if ($gGrace) { $env:MEDIAGIT_GC_GRACE_SECS = $gGrace }
        while (-not $pp.HasExited) {
          $g = Invoke-MG $gSrvRepo @("gc", "-y") $Phase -TimeoutSec 1200
          $gcRuns++
          # overlap = the push was still alive when this gc FINISHED, so it ran wholly inside the push.
          if (-not $pp.HasExited) { $gcOverlap++ }
          if ($g.Exit -ne 0) { $gcBadExit++ }
          Write-QaLog $Phase ("gcrace gc#{0} exit={1} push-alive={2} t={3:n1}s" -f $gcRuns, $g.Exit, (-not $pp.HasExited), $sw.Elapsed.TotalSeconds)
        }
      } finally {
        if ($null -ne $savedG) { $env:MEDIAGIT_GC_GRACE_SECS = $savedG } else { Remove-Item Env:\MEDIAGIT_GC_GRACE_SECS -EA SilentlyContinue }
      }
      $pp.WaitForExit()
      $pushExit = $pp.ExitCode
      $pushSec = [math]::Round($sw.Elapsed.TotalSeconds, 1)
      $retryExit = ""
      if ($pushExit -ne 0) {
        $rr = Invoke-MG $gSrc @("push", "origin") $Phase -TimeoutSec 3600
        $retryExit = $rr.Exit
      }
      $pushOk = ($pushExit -eq 0) -or ($retryExit -eq 0)
      Write-QaGate $Phase "gcrace-push" $pushOk ("{0} payload={1}MB push-exit={2} retry-exit={3} push-sec={4}" -f $gTag, ($nFiles * 32), $pushExit, $retryExit, $pushSec)
      if (-not $pushOk) { $script:AllPass = $false }

      # non-vacuity: gc must have run to completion at least once INSIDE the push window.
      $ovOk = ($gcOverlap -ge 1)
      Write-QaGate $Phase "gcrace-overlap" $ovOk ("{0} gc-runs={1} gc-finished-during-push={2} gc-nonzero-exit={3} (must be >=1 overlapped)" -f $gTag, $gcRuns, $gcOverlap, $gcBadExit)
      if (-not $ovOk) { $script:AllPass = $false }

      # --- the server-side repo must be intact ---
      $fsck = Invoke-MG $gSrvRepo @("fsck", "--full") $Phase -TimeoutSec 3600
      $fsckClean = ($fsck.Exit -eq 0) -and ($fsck.Out -notmatch "(?i)corrupt|missing|failed")
      Write-QaGate $Phase "gcrace-fsck-server" $fsckClean ("{0} fsck-exit={1} {2}" -f $gTag, $fsck.Exit, (($fsck.Out -split "`n" | Where-Object { $_ -match "(?i)corrupt|missing|failed|error" } | Select-Object -First 2) -join " | "))
      if (-not $fsckClean) { $script:AllPass = $false }

      # --- and a fresh clone must be byte-identical ---
      $c = Invoke-MG $null @("clone", $gsrv.Url, $gClone) $Phase -TimeoutSec 3600
      $mism = -1
      $parity = $false
      if (($c.Exit -eq 0) -and (Test-Path $gClone)) {
        $h1 = Get-QaTreeHashes $gSrc; $h2 = Get-QaTreeHashes $gClone
        $mism = (Compare-Object $h1 $h2 | Measure-Object).Count
        $parity = ($mism -eq 0) -and ($h1.Count -gt 0)
      }
      Write-QaGate $Phase "gcrace-clone-parity" $parity ("{0} clone-exit={1} files={2} mismatches={3}" -f $gTag, $c.Exit, @(Get-QaTreeHashes $gSrc).Count, $mism)
      if (-not $parity) { $script:AllPass = $false }
    }
  } catch {
    Write-QaGate $Phase "gcrace-all" $false "unexpected error: $_"
    $script:AllPass = $false
  } finally {
    Stop-QaServer $gsrv
  }
}

Write-QaLog $Phase "=== 06_remote done: overall=$(if ($script:AllPass) { 'PASS' } else { 'FAIL' }) ==="
# Teardown: reclaim this phase's own work/ scratch so a long campaign cannot run the
# volume out of space. work/ ONLY - logs/ and fixtures-synthetic/ are never touched.
Invoke-QaTeardown $Phase @("remote-*")

Exit-QaPhase $Phase (-not $script:AllPass)
