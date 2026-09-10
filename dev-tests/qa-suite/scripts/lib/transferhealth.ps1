# Per-arm TRANSFER health: did sustained reads actually complete?
#
# WHY THIS EXISTS, AND WHY linkprobe.ps1 IS NOT ENOUGH.
#
# linkprobe.ps1 samples a TCP handshake to :443. In ga47 it scored the aws clone
# window 0 failures out of 321 samples, median connect 78ms -- "clean" -- while
# 44 sustained reads died mid-body in the same window. A handshake says the path
# can be OPENED. It says nothing about whether a 60MB body can be PULLED through
# it, which is the only thing a clone actually needs. Counting mid-body aborts is
# a genuine signal the connect probe cannot see, and that is why this file stays.
#
# CORRECTION, 2026-09-09 -- THIS HEADER PREVIOUSLY CLAIMED THOSE 44 ABORTS KILLED
# THE ga47 CLONE. THEY DID NOT. Read back from the run's own logs:
#
#   server-aws-10_scale-S5-10.out.log : all 44 are mediagit_storage::minio WARN
#     "Operation failed (attempt N/5), retrying". Distribution 38x attempt 1,
#     6x attempt 2. Zero reached 5. Zero "Failed after N retries". Zero ERROR
#     lines in the whole file. Every one of them RECOVERED -- with_retry did
#     its job.
#
#   10_scale-cmds.log:4351 : what actually killed the clone --
#     "Failed to download chunk 070ae806...: error sending request for url
#      (http://127.0.0.1:58579/...)". 65 of those, all against LOOPBACK, all at
#     send() before any body existed; 36 failed once, 17 twice, 12 exhausted
#     CHUNK_GET_MAX_RETRIES=3. The server was idle at the time (idle_s=41 in the
#     heartbeat at the moment of the final failure), so this was never the link
#     and never the backend.
#
# So this file did to itself precisely what it was written to prevent: it took a
# correlation -- aborts and a failure in the same window -- and wrote it down as
# a cause, inside the instrument built to stop "product or link?" from being
# guessed. That is the gate-that-cannot-fail shape one level up
# (feedback_gates_that_cannot_fail): the instrument was sound, the CONCLUSION
# drawn from it was never checked against the terminal error. Count the aborts
# here; do not let the count name a cause on its own.
#
# WHY IT READS LOGS INSTEAD OF GENERATING TRAFFIC.
#
# The obvious fix -- have the probe pull a large object every N seconds -- is
# wrong here. It would add concurrent bucket reads to a campaign whose open
# question was whether concurrent bucket reads are harmful. The instrument would
# perturb the thing it measures.
#
# The signal is already recorded, on the campaign's REAL traffic, for free: the
# server logs every aborted read and every verification's observed MB/s. This
# reads that back. Zero added load, and it measures the actual product path
# rather than a synthetic stand-in for it.
#
# SCOPE. Records only. Never trips, never kills anything, never fails a phase.
# A clean report here is evidence AGAINST the network and FOR a product bug,
# which is just as useful as the reverse.

# A read that was opened successfully and then died partway through the body.
#
# Two distinct classes, deliberately counted apart -- they implicate different
# code and conflating them hides which one moved:
#   verify : the background pack verifier's ranged reads  (repo.rs)
#   serve  : storage-layer reads made while serving a client request (minio.rs)
# "stream error mid-entry" does not contain "streaming error" (stream vs
# streaming), so the two patterns cannot double-count the same line.
$script:QA_TH_VERIFY_ABORT = 'stream error mid-entry'
$script:QA_TH_SERVE_ABORT  = 'streaming error|dispatch failure'
$script:QA_TH_THROUGHPUT   = 'mb_per_sec="([0-9.]+)"'

# Did packs actually reach the verified state, or did verification give up?
#
# A pack that never verifies is not a correctness problem -- an unverified pack
# can never have a URL minted for it, so clients take the proxy path and get
# bytes that are hashed inline before being served. It is a SILENT PERFORMANCE
# problem: every clone of that pack relays through the server forever, and the
# campaign still reports pass=241. Nothing else in the suite can see it.
$script:QA_TH_VERIFIED = 'pack verified clean'
$script:QA_TH_GAVEUP   = 'exceeded its wall-clock budget|incomplete after retries'

function Get-QaBackendFromLogName {
  param([string]$Name)
  # server-<backend>-<phase>-<n>.out.log
  if ($Name -match '^server-([^-]+)-') { return $Matches[1] }
  return "unknown"
}

function Write-QaTransferHealth {
  param([string]$LogDir)

  $logs = @(Get-ChildItem -Path $LogDir -Filter "server-*.out.log" -EA SilentlyContinue)

  # Loud on empty, for the same reason the link probe is. "Nothing found" must
  # never read as "nothing wrong": it means this run carries NO transfer
  # evidence, so a cloud failure in it cannot be attributed either way.
  if ($logs.Count -eq 0) {
    Write-Host "WARNING: no server logs found under $LogDir - transfer health is UNKNOWN for this run."
    Write-Host "This run has NO evidence about whether sustained reads completed."
    Write-Host "Do not read a cloud failure here as a product bug on this basis."
    return
  }

  $byBackend = @{}
  foreach ($f in $logs) {
    $b = Get-QaBackendFromLogName $f.Name
    if (-not $byBackend.ContainsKey($b)) {
      $byBackend[$b] = [pscustomobject]@{
        Logs = 0; VerifyAborts = 0; ServeAborts = 0; Verified = 0; GaveUp = 0
        Mbps = New-Object System.Collections.ArrayList
        AbortMinutes = @{}
      }
    }
    $acc = $byBackend[$b]
    $acc.Logs++

    # Strip the tracing crate's ANSI colouring before matching: field names
    # arrive wrapped in escape sequences, so a naive `mb_per_sec=` never hits.
    $lines = Get-Content -Path $f.FullName -EA SilentlyContinue
    foreach ($raw in $lines) {
      $line = $raw -replace "\x1b\[[0-9;]*m", ""

      $isVerify = $line -match $script:QA_TH_VERIFY_ABORT
      $isServe  = $line -match $script:QA_TH_SERVE_ABORT
      if ($isVerify) { $acc.VerifyAborts++ }
      if ($isServe)  { $acc.ServeAborts++ }

      if ($isVerify -or $isServe) {
        # Bucket to the minute so the timeline can be correlated against a
        # failing drill's timestamp without dumping hundreds of lines.
        if ($line -match '^(\d{4}-\d{2}-\d{2}T\d{2}:\d{2})') {
          $m = $Matches[1]
          if ($acc.AbortMinutes.ContainsKey($m)) { $acc.AbortMinutes[$m]++ }
          else { $acc.AbortMinutes[$m] = 1 }
        }
      }

      if ($line -match $script:QA_TH_VERIFIED) { $acc.Verified++ }
      if ($line -match $script:QA_TH_GAVEUP)   { $acc.GaveUp++ }

      if ($line -match $script:QA_TH_THROUGHPUT) {
        [void]$acc.Mbps.Add([double]$Matches[1])
      }
    }
  }

  Write-Host ("  scanned {0} server log(s) across {1} backend(s)" -f $logs.Count, $byBackend.Count)

  foreach ($b in ($byBackend.Keys | Sort-Object)) {
    $a = $byBackend[$b]
    $total = $a.VerifyAborts + $a.ServeAborts

    # Floor, not [int]: PowerShell's [int] rounds half-to-even, so a 3-sample
    # median would report the max. Same trap already fixed in linkprobe.ps1.
    $sorted = @($a.Mbps | Sort-Object)
    $medMbps = if ($sorted.Count) { $sorted[[math]::Floor($sorted.Count * 0.50)] } else { $null }
    $minMbps = if ($sorted.Count) { $sorted[0] } else { $null }

    # The verdict keys on SERVE aborts, not the total. Calibrated against real
    # runs rather than guessed:
    #
    #   ga45 (clean, 241/0) aws: serve=0  verify=6  -> clone PASSED
    #   ga47 (failed)       aws: serve=45 verify=28 -> clone FAILED exit=1
    #
    # A verify abort is absorbed by design -- verify_pack_with_budget retries
    # unreadable entries three times, and in ga45 every pack still verified
    # clean. Scoring that arm "BAD" would have flagged a passing run, and a
    # gate that fires on a non-problem is one people learn to wave through.
    # A serve abort has no such ladder above it: it is a read dying mid-body
    # while a client waits on those exact bytes.
    #
    # Spread still matters for serve aborts: 38 inside ONE minute is a discrete
    # event (A7 deliberately kills the backend and its minio arm scores exactly
    # that), where the same count spread over twenty minutes is a sick link.
    # DESCRIBES the pattern; it does not pronounce pass/fail. That is deliberate.
    #
    # A first cut scored BAD/MARGINAL and, checked against real runs, labelled
    # ga44 "BAD" on aws and ga45 "BAD" on minio -- both of which passed 241/0.
    # Tuning a verdict threshold against four campaigns is how you get a gate
    # people wave through. So: report the shape, let the reader correlate it
    # with a failing drill's timestamp, and never imply a judgement the data
    # does not support.
    #
    # The shape that empirically tracks a broken clone is SUSTAINED-and-sparse
    # (ga47 aws: 45 serve-aborts spread over 27 min, clone failed), not
    # dense-and-brief (ga45 minio: 53 in 2 min, which is A7 deliberately
    # killing the backend, and the run passed).
    $span = $a.AbortMinutes.Count
    $density = if ($span) { [math]::Round($a.ServeAborts / $span, 1) } else { 0 }
    $verdict = if ($total -eq 0) { "clean" }
               elseif ($a.ServeAborts -eq 0) { "verify-only - retry ladder absorbed $($a.VerifyAborts)" }
               elseif ($span -le 2 -and $density -ge 10) { "burst ($density/min over $span min - looks like a deliberate outage drill)" }
               elseif ($span -ge 5) { "SUSTAINED - $($a.ServeAborts) serve-aborts over $span min; correlate with any failing drill in that window" }
               else { "scattered - $($a.ServeAborts) serve-aborts over $span min" }

    $tp = if ($null -ne $medMbps) {
      ("verify-MB/s med={0:N2} min={1:N2} n={2}" -f $medMbps, $minMbps, $sorted.Count)
    } else { "verify-MB/s n/a" }

    Write-Host ("  {0,-8} logs={1,-3} aborts={2,-4} (serve={3} verify={4})  {5}  {6}" -f
      $b, $a.Logs, $total, $a.ServeAborts, $a.VerifyAborts, $tp, $verdict)
    Write-Host ("           packs verified={0}  gave-up={1}{2}" -f
      $a.Verified, $a.GaveUp,
      $(if ($a.GaveUp -gt 0) { "  <- those packs stay on the PROXY path for every future clone" } else { "" }))

    # The correlation the ga47 forensics actually needed by hand: WHEN were the
    # aborts, so a failing drill's window can be checked against them. Densest
    # minutes first.
    if ($total -gt 0) {
      $top = $a.AbortMinutes.GetEnumerator() | Sort-Object Value -Descending | Select-Object -First 5
      foreach ($m in $top) {
        Write-Host ("      ABORTS {0}  x{1}" -f $m.Key, $m.Value)
      }
    }
  }

  Write-Host ""
  Write-Host "  A backend with aborts>0 had reads that OPENED and then died mid-body."
  Write-Host "  The connect probe above cannot see that condition and will still say 'clean'."
}
