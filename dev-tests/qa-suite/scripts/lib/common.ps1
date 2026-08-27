# qa-suite shared helpers. ASCII-only, PS 5.1 compatible.
# Scripts dot-source ONLY this file; it pulls in config.ps1 (defines $QA).
. (Join-Path (Split-Path (Split-Path $PSScriptRoot -Parent) -Parent) "config.ps1")
Initialize-QaDirs

$ErrorActionPreference = "Continue"

# `commit` refuses an unconfigured author rather than recording
# "Unknown <unknown@localhost>", because commit authorship cannot be changed
# afterwards. A real user configures identity once; the harness that simulates
# one must do the same, or every phase that commits fails at the first commit
# and every later step cascades off a branch that was never created.
#
# Set here rather than per script: ten phases commit, and the four that already
# set it locally were the only reason this was not caught sooner. A phase that
# deliberately tests the refusal (12_safety SAFE11) clears these explicitly.
if (-not $env:MEDIAGIT_AUTHOR_NAME)  { $env:MEDIAGIT_AUTHOR_NAME  = "QA-Suite" }
if (-not $env:MEDIAGIT_AUTHOR_EMAIL) { $env:MEDIAGIT_AUTHOR_EMAIL = "qa-suite@mediagit.local" }

# A campaign appends to a handful of shared log/TSV files thousands of times over many
# hours; a scanner/indexer briefly opening one for a read is enough to win the race and
# throw "being used by another process" (seen 2026-08-06, S2-churn, full stack traced to
# this exact pattern - see harness-faults.log).
#
# THE RULE: writing a log line must NEVER void a measurement.
#
# That is the whole design, and it is deliberately independent of root cause.
# "Stream was not readable" (System.ArgumentException, thrown from
# FileSystemProvider.GetContentWriter -> StreamReader ctor) has now voided
# S2-churn/S3-conflicts across five campaigns. Two attempts to fix the CAUSE
# both failed to hold:
#   2026-08-17  widened the catch to include ArgumentException. gagate4 then
#               threw at the rethrow after the attempts ran out - so the
#               condition is persistent for that code path, not a brief lock.
#   2026-08-18  forced an explicit -Encoding on the theory that encoding
#               DETECTION was doing the offending read. Plausible (Write-QaRow,
#               the only caller that always passed "ASCII", has never appeared
#               in a fault stack) but NOT PROVEN: an attempt to reproduce it by
#               denying read-sharing produced IOException from both the
#               with-encoding and without-encoding paths, so that experiment
#               did not isolate the mechanism. Treat it as hardening, not as
#               the answer.
#
# So stop betting the drill on a diagnosis. After the retries are spent the line
# is DROPPED and recorded, never rethrown. A lost log line costs one line; an
# exception here costs a 40-minute scale drill and, worse, reports it as a
# harness error rather than a product result. The drop is loud, not silent: it
# lands in harness-faults.log, which `09_report` reads and gates on via
# `campaign-no-harness-faults` - so a persistent problem still fails the
# campaign, it just does not destroy the measurement on its way out.
function Add-QaContentRetry($Path, $Value, [string]$Encoding = "ASCII") {
  # $Encoding is kept for call-site compatibility (Write-QaRow passes "ASCII")
  # but the writer below is byte-level and always ASCII, matching what
  # Add-Content produced here before. The suite is ASCII-only by convention.
  if (-not $Encoding) { $Encoding = "ASCII" }
  for ($attempt = 1; $attempt -le 5; $attempt++) {
    try {
      # A raw FileStream opened with FileShare::ReadWrite, NOT Add-Content.
      #
      # This is the cause, finally isolated. `10_scale` runs 16 concurrent
      # clients and every one of them appends to the SAME cmds log; PowerShell's
      # FileSystemProvider opens the file in a way that cannot share with
      # another writer, and its GetContentWriter reads the file (for BOM and
      # encoding) as part of that -- which is where "Stream was not readable"
      # comes from. It is concurrent-append contention, not encoding detection:
      # 20260818-p10check2 still produced 36 of them AFTER an explicit
      # -Encoding was forced, which rules that hypothesis out.
      #
      # FileShare::ReadWrite lets concurrent writers coexist, and appending in
      # one Write call keeps a line intact. The retry and the drop below stay as
      # a backstop for a genuine external lock (a scanner or indexer), which is
      # the case this helper was originally written for.
      $bytes = [Text.Encoding]::ASCII.GetBytes(($Value | Out-String).TrimEnd("`r", "`n") + "`r`n")
      $fs = [IO.File]::Open($Path, [IO.FileMode]::Append, [IO.FileAccess]::Write, [IO.FileShare]::ReadWrite)
      try { $fs.Write($bytes, 0, $bytes.Length) } finally { $fs.Dispose() }
      return
    } catch {
      if ($attempt -lt 5) {
        Start-Sleep -Milliseconds (100 * $attempt)
        continue
      }
      # Spent. Record and carry on - never throw from a logging helper.
      try {
        $rec = @(
          "[QA-FAULT] stage=log-write type=$($_.Exception.GetType().FullName)",
          "message: $($_.Exception.Message)",
          "path: $Path",
          "note: log line DROPPED after 5 attempts; the drill continues on purpose.",
          "      A logging failure must not void a measurement - see Add-QaContentRetry.",
          ""
        ) -join "`r`n"
        [IO.File]::AppendAllText((Join-Path $QA.Logs "harness-faults.log"), $rec)
      } catch { }
      return
    }
  }
}

# Verdict for a stall snapshot, from the client's CPU delta, whether it had any
# connection at all, and - decisively - whether the server it is talking to is
# doing work on its behalf.
#
# Separated from the two call sites (mid-wait sample and post-exit flush) so
# the rule lives in ONE place. Both sites previously carried their own copy of
# the if/elseif chain, which is how the two would drift.
#
# The server term is what ga29 taught: an idle client whose server is burning
# CPU is being served, not hung. Without it this reported SUSPECT HANG on three
# healthy S5 clones, and a detector that cries wolf on healthy runs gets
# ignored - the same outcome as one that never fires.
# Second CPU reading for the server(s) identified at snapshot time. Returns
# $null when there was nothing local to sample (a purely remote backend, or the
# server exited), which the verdict treats as "cannot tell" rather than as
# evidence of a hang.
function _QaServerCpuDelta($Snap) {
  if ($null -eq $Snap.SrvCpu1 -or @($Snap.SrvPids).Count -eq 0) { return $null }
  try {
    $now = 0.0
    $seen = 0
    foreach ($sp in $Snap.SrvPids) {
      $o = Get-Process -Id $sp -EA SilentlyContinue
      if ($o) { $now += $o.TotalProcessorTime.TotalSeconds; $seen++ }
    }
    if ($seen -eq 0) { return $null }
    return $now - $Snap.SrvCpu1
  } catch { return $null }
}

function Get-QaStallVerdict($Exited, $CpuDelta, $Established, $SrvCpuDelta) {
  if ($Exited) { return "COMPLETED during the snapshot window - slow, not hung" }
  if ($CpuDelta -gt 0.05) { return "PROGRESSING - client burning CPU; slow, not hung" }
  if ($null -ne $SrvCpuDelta -and $SrvCpuDelta -gt 0.05) {
    return ("SERVED - client idle but its server is working ({0:n3}s CPU); waiting on the server, not hung" -f $SrvCpuDelta)
  }
  if ($Established -eq 0) {
    return "SUSPECT HANG - no CPU and NO established connection: the request never left this box"
  }
  if ($null -eq $SrvCpuDelta) {
    return "SUSPECT HANG - client idle with a connection to a remote we cannot sample; sent, awaiting a reply"
  }
  return "SUSPECT HANG - client idle AND its server idle: both ends stopped, nothing is driving this forward"
}

function Write-QaLog([string]$Phase, [string]$Msg) {
  $line = "{0} [{1}] {2}" -f (Get-Date -Format "HH:mm:ss"), $Phase, $Msg
  Add-QaContentRetry (Join-Path $QA.Logs "$Phase.log") $line
  Write-Host $line
}

# TSV row writer: creates file with header on first write, appends tab-joined sanitized values.
function Write-QaRow([string]$Path, [string[]]$Header, [object[]]$Values) {
  if (-not (Test-Path $Path)) { ($Header -join "`t") | Set-Content $Path -Encoding ASCII }
  $clean = $Values | ForEach-Object { ("" + $_) -replace "`t", " " -replace "`r?`n", " | " }
  Add-QaContentRetry $Path ($clean -join "`t") "ASCII"
}

# Exception detail shared by every catch that logs a harness fault: type, message, inner
# exception, flattened AggregateException, exception ToString(), and PS stack trace.
# `"$_"` alone is MESSAGE ONLY - no type, no inner exception, no stack - which is exactly
# why "Stream was not readable" was recorded three times (Invoke-MG's own catch, below)
# and diagnosed zero times, and then recorded a fourth time by 10_scale.ps1's drill-level
# catches with only "$_" too. This is the one place that logic lives now; Invoke-MG and
# Write-QaFault (below) both call it instead of re-deriving it.
# Returns @{ ExType; Lines } - Lines is the ordered detail block, empty entries dropped.
function Get-QaExceptionDetail($ErrorRecord) {
  $ex = $ErrorRecord.Exception
  $exType = if ($ex) { $ex.GetType().FullName } else { "(none)" }
  $inner = if ($ex -and $ex.InnerException) { "$($ex.InnerException.GetType().FullName): $($ex.InnerException.Message)" } else { "" }
  # AggregateException from a faulted async op (e.g. ReadToEndAsync) hides the real cause
  # one level down; a bare .Message reads "One or more errors occurred".
  $flat = ""
  if ($ex -is [System.AggregateException]) {
    $flat = (($ex.Flatten().InnerExceptions | ForEach-Object { "$($_.GetType().FullName): $($_.Message)" }) -join " | ")
  }
  return @{
    ExType = $exType
    Lines  = @(
      "message: $ErrorRecord"
      $(if ($inner) { "inner: $inner" })
      $(if ($flat) { "aggregated: $flat" })
      "exception:"
      $(if ($ex) { $ex.ToString() } else { "(no exception object)" })
      "script-stack:"
      "$($ErrorRecord.ScriptStackTrace)"
    ) | Where-Object { $_ }
  }
}

# For catches OUTSIDE Invoke-MG (a drill body's own try/catch, not the subprocess wrapper):
# logs the same class of detail Invoke-MG's catch captures to the same collated
# logs\harness-faults.log, so 09_report's campaign-no-harness-faults gate sees it too.
# $Context identifies the call site (e.g. "S2-churn-minio") since every source appends to
# one file. Returns a short one-line summary for a TSV cell - the long form goes to the log,
# not the cell (see 10_scale.ps1's drill catches).
function Write-QaFault([string]$Context, $ErrorRecord) {
  $xd = Get-QaExceptionDetail $ErrorRecord
  $detail = (@("[QA-FAULT] stage=$Context type=$($xd.ExType)") + $xd.Lines) | Out-String
  try { $detail | Add-Content (Join-Path $QA.Logs "harness-faults.log") -Encoding UTF8 } catch {}
  Write-Warning "harness fault [$Context] ($($xd.ExType)) - see logs\harness-faults.log"
  return "$($xd.ExType): $ErrorRecord (see harness-faults.log)"
}

# Run mediagit against a repo. Returns @{Exit; Sec; Out} - Out is combined stdout+stderr text.
# Full output also appended to $QA.Logs\<Phase>-cmds.log for post-hoc digging.
# Enforces $TimeoutSec: on timeout, kills process tree and returns exit 124.
# -StdIn: lines fed to the child's stdin in order (one dialoguer Input/Password
# prompt per line), for driving interactive commands like `mediagit auth login`
# non-interactively. Omit (default) for every existing non-interactive call.
# How long to wait for a dead child's pipes to reach EOF before declaring the
# handles leaked. Generous - this is a backstop against an inherited handle, not
# a throughput knob - but finite, which is the whole point.
$script:QaDrainTimeoutMs = 30000

function Invoke-MG([string]$Repo, [string[]]$MgArgs, [string]$Phase = "misc", [int]$TimeoutSec = 600, [string[]]$StdIn = $null) {
  $sw = [Diagnostics.Stopwatch]::StartNew()
  $allArgs = if ($Repo) { @("-C", $Repo) + $MgArgs } else { $MgArgs }
  # Quote each arg (A6 tests spaces/unicode paths); escape embedded quotes.
  $argLine = ($allArgs | ForEach-Object { '"' + ($_ -replace '"', '\"') + '"' }) -join " "

  $proc = New-Object System.Diagnostics.Process
  $proc.StartInfo.FileName = $QA.MG
  $proc.StartInfo.Arguments = $argLine
  $proc.StartInfo.UseShellExecute = $false
  $proc.StartInfo.RedirectStandardOutput = $true
  $proc.StartInfo.RedirectStandardError = $true
  if ($StdIn) { $proc.StartInfo.RedirectStandardInput = $true }
  $proc.StartInfo.CreateNoWindow = $true

  # try/finally Dispose(): Process (and the pipe handles behind StandardOutput/
  # StandardError) is IDisposable, and letting $proc fall out of scope only makes
  # it GC-eligible - the handles are not freed until the finalizer runs. Disposing
  # deterministically is correct practice regardless, and measurably reduces
  # handle churn: 400 spawns of mediagit.exe grew this host's handle count by
  # +367 without Dispose vs +47 with it (measured 2026-08-04).
  #
  # HONEST SCOPE - this is hygiene, NOT a proven cure. It was added while
  # investigating the 20260804-scale-verify2 S2/S3/S4 fault ("Stream was not
  # readable", which voided three drills). The leak hypothesis did NOT survive
  # testing: a 1,200-spawn burst - more than S2's whole churn loop - produced
  # ZERO failures both with and without Dispose, and handle count plateaued at
  # ~1,000, i.e. the finalizer does keep up. So the cause of that fault remains
  # UNKNOWN; do not record it as solved. What actually protects the campaign is
  # the catch block below plus the "ERROR" verdict, which stop a harness fault
  # from being silently counted as a product failure.
  # Which call was in flight when a fault hit. `$_` alone cannot answer this,
  # and it is the first thing you need: "Stream was not readable" thrown by
  # Start() (pipe/handle creation failed) is a completely different defect from
  # the same message thrown while draining (the stream died mid-read). Three
  # campaigns recorded that fault and none recorded which one it was.
  $stage = "start"
  $started = $false
  try {
    $proc.Start() | Out-Null
    $started = $true
    if ($StdIn) {
      $stage = "stdin"
      foreach ($line in $StdIn) { $proc.StandardInput.WriteLine($line) }
      $proc.StandardInput.Close()
    }
    # Threadpool drain: ReadToEnd-after-WaitForExit deadlocks once the child fills
    # the pipe buffer; async tasks drain continuously without the PS event loop.
    $stage = "open-streams"
    $stdout = $proc.StandardOutput
    $stderr = $proc.StandardError
    $stage = "begin-drain"
    $outTask = $stdout.ReadToEndAsync()
    $errTask = $stderr.ReadToEndAsync()

    # LIVE STALL SNAPSHOT.
    #
    # `SLOW: ... possible stall` already reports a slow command, but only AFTER
    # the process has exited - by which point its threads, sockets and CPU
    # counters are gone. Every capture of the client-hang family has therefore
    # been reconstructed from logs after the fact, and the one question that
    # decides the investigation - did the client's request ever leave the box -
    # was never answerable.
    #
    # 20260826-ga27 is the case in point: `push` spent 600s and died with
    # "Failed to send GET /info/refs ... operation timed out" against
    # 127.0.0.1, while the server it was talking to bound correctly, ticked 90
    # runtime heartbeats, and logged ZERO requests. Server-side is now proven
    # healthy; the missing half is the client's own socket state at that moment.
    #
    # So: wait in slices, and when a command crosses the stall threshold, dump
    # the child's TCP connections, thread wait-reasons and two CPU samples
    # BEFORE it dies. Two samples, not one, because a single reading cannot
    # separate "blocked" from "slow but progressing" - that ambiguity cost a day
    # on 20260820-ga4.
    #
    # Fires at most once per command, and only past the threshold, so a healthy
    # campaign pays nothing.
    $stage = "wait"
    $script:QaSnapPending = $null
    $stallSnapSec = [int](_Env "MG_QA_STALL_SNAPSHOT_SEC" "240")
    $snapped = $false
    $waitedMs = 0
    $sliceMs = 5000
    while ($waitedMs -lt ($TimeoutSec * 1000) -and -not $proc.HasExited) {
      $proc.WaitForExit([Math]::Min($sliceMs, ($TimeoutSec * 1000) - $waitedMs)) | Out-Null
      $waitedMs += $sliceMs
      if (-not $snapped -and -not $proc.HasExited -and $waitedMs -ge ($stallSnapSec * 1000)) {
        $snapped = $true
        try {
          Write-QaLog $Phase "STALL-SNAPSHOT after $([int]($waitedMs/1000))s: $($QA.MG) $argLine"
          $c1 = $proc.TotalProcessorTime.TotalSeconds
          $conns = @(Get-NetTCPConnection -OwningProcess $proc.Id -EA SilentlyContinue)
          $byState = ($conns | Group-Object State | ForEach-Object { "$($_.Name)=$($_.Count)" }) -join " "
          $est = @($conns | Where-Object { $_.State -eq 'Established' })
          $remotes = ($est | ForEach-Object { "$($_.RemoteAddress):$($_.RemotePort)" } |
                      Sort-Object -Unique | Select-Object -First 4) -join ","
          # A running thread has no WaitReason, and Group-Object throws on the
          # null rather than bucketing it - which printed an ErrorRecord over
          # the top of the very diagnostic being collected. Project to a string
          # first so "(running)" is just another bucket.
          $waits = ((Get-Process -Id $proc.Id -EA SilentlyContinue).Threads |
                    ForEach-Object { if ($_.WaitReason) { "$($_.WaitReason)" } else { "(running)" } } |
                    Group-Object | ForEach-Object { "$($_.Name)=$($_.Count)" }) -join " "
          # THE SERVER IS THE OTHER HALF OF THE ANSWER.
          #
          # 20260826-ga29 is why this is here. Three S5 clones were flagged
          # SUSPECT HANG - no client CPU, one established connection - and all
          # three finished normally. The verdict was wrong because it was
          # reached from the client alone: a clone blocked on the local server
          # while that server pulls 11 GB from a cloud backend looks EXACTLY
          # like a deadlock from the client's side. Idle client + working
          # server is healthy; idle client + idle server is not, and only the
          # second reading separates them.
          #
          # Identified by the ports the client is actually connected to, not by
          # process name: a campaign runs several mediagit-server instances at
          # once and blaming the wrong one is worse than saying nothing.
          $srvCpu1 = $null
          $srvPids = @()
          try {
            $localPorts = @($est | Where-Object {
              $_.RemoteAddress -eq '127.0.0.1' -or $_.RemoteAddress -eq '::1'
            } | ForEach-Object { $_.RemotePort } | Sort-Object -Unique)
            foreach ($lp in $localPorts) {
              $owner = Get-NetTCPConnection -LocalPort $lp -State Listen -EA SilentlyContinue |
                       Select-Object -First 1 -ExpandProperty OwningProcess
              if ($owner) { $srvPids += $owner }
            }
            $srvPids = @($srvPids | Sort-Object -Unique)
            if ($srvPids.Count -gt 0) {
              $srvCpu1 = 0.0
              foreach ($sp in $srvPids) {
                $o = Get-Process -Id $sp -EA SilentlyContinue
                if ($o) { $srvCpu1 += $o.TotalProcessorTime.TotalSeconds }
              }
            }
          } catch { }
          # The CPU second sample comes from the NEXT wait slice, not from a
          # Start-Sleep here. A sleep inside this loop stops it calling
          # WaitForExit, so a command that finished during the sleep is not
          # noticed until it ends - up to 5s of pure measurement error added to
          # a drill that is being timed. Deferring costs nothing: the loop was
          # going to wait anyway.
          $script:QaSnapC1 = $c1
          $script:QaSnapWhen = Get-Date
          $script:QaSnapPending = @{
            Phase = $Phase; ByState = $byState; Remotes = $remotes
            Waits = $waits; Established = $est.Count
            SrvPids = $srvPids; SrvCpu1 = $srvCpu1
          }
        } catch {
          Write-QaLog $Phase "STALL-SNAPSHOT failed: $($_.Exception.GetType().Name)"
        }
      }

      # Second CPU sample, one slice later. STATE THE VERDICT rather than
      # printing a rubric for a human to apply: on 20260826-ga28 all three
      # firings were healthy long clones, and a reader skimming the log still
      # had to do the classification by hand to learn that.
      if ($script:QaSnapPending -and ((Get-Date) - $script:QaSnapWhen).TotalSeconds -ge 4) {
        try {
          $sp = $script:QaSnapPending
          $script:QaSnapPending = $null
          $elapsed = ((Get-Date) - $script:QaSnapWhen).TotalSeconds
          $cpu = 0.0
          if (-not $proc.HasExited) { $cpu = $proc.TotalProcessorTime.TotalSeconds - $script:QaSnapC1 }
          $srvCpu = _QaServerCpuDelta $sp
          $verdict = Get-QaStallVerdict $false $cpu $sp.Established $srvCpu
          Write-QaLog $sp.Phase ("STALL-SNAPSHOT tcp[{0}] remotes[{1}] threadwaits[{2}] cpu_delta={3:n3}s/{4:n1}s srv_cpu_delta={5} -> {6}" -f `
            $sp.ByState, $sp.Remotes, $sp.Waits, $cpu, $elapsed,
            $(if ($null -eq $srvCpu) { "n/a" } else { "{0:n3}s" -f $srvCpu }), $verdict)
        } catch {
          Write-QaLog $Phase "STALL-SNAPSHOT second sample failed: $($_.Exception.GetType().Name)"
        }
      }
    }
    # FLUSH. Deferring the second CPU sample to the next wait slice removed the
    # added latency, but introduced a worse bug: a command that exited between
    # the two samples emitted the "after Ns" header and then NOTHING - the
    # diagnostic vanished precisely in the case where a stall resolved itself,
    # which is a case worth seeing. Caught by the PROGRESSING drill, which went
    # from FAIL(no verdict) to a real verdict once this was added.
    if ($script:QaSnapPending) {
      try {
        $sp = $script:QaSnapPending
        $script:QaSnapPending = $null
        $elapsed = ((Get-Date) - $script:QaSnapWhen).TotalSeconds
        $cpu = $proc.TotalProcessorTime.TotalSeconds - $script:QaSnapC1
        $srvCpu = _QaServerCpuDelta $sp
        $verdict = Get-QaStallVerdict $proc.HasExited $cpu $sp.Established $srvCpu
        Write-QaLog $sp.Phase ("STALL-SNAPSHOT tcp[{0}] remotes[{1}] threadwaits[{2}] cpu_delta={3:n3}s/{4:n1}s srv_cpu_delta={5} -> {6}" -f `
          $sp.ByState, $sp.Remotes, $sp.Waits, $cpu, $elapsed,
          $(if ($null -eq $srvCpu) { "n/a" } else { "{0:n3}s" -f $srvCpu }), $verdict)
      } catch {
        Write-QaLog $Phase "STALL-SNAPSHOT flush failed: $($_.Exception.GetType().Name)"
      }
    }

    if (-not $proc.HasExited) {
      # A timeout is a RESULT, not a harness fault, and nothing in this block may
      # turn it into one. `taskkill` writes to stderr when it cannot terminate
      # part of the tree ("The process with PID N (child process of PID M) could
      # not be terminated"), and PowerShell surfaces a native command's stderr as
      # an ErrorRecord that becomes TERMINATING under the `$ErrorActionPreference
      # = 'Stop'` that phase scripts set. On 20260817-gagate3 that turned a clean
      # 3,600s stall in S2-churn into `exit=-1` with the whole `[INVOKE-MG-ERROR]`
      # block in place of the product's own output -- so a real product bug was
      # both mislabelled as ours AND stripped of the evidence needed to diagnose
      # it. Exactly the run where the output matters most.
      try { taskkill /T /F /PID $proc.Id 2>$null | Out-Null } catch { }
      # BOUNDED. The parameterless WaitForExit() does not just wait for the
      # process - it waits for EOF on the redirected pipes as well. "pipes close
      # on kill" is only true if this child is the sole holder; a daemon that
      # inherited the handles keeps them open and this call never returns. See
      # the drain note in the success path below for the measured case.
      try { $proc.WaitForExit($script:QaDrainTimeoutMs) | Out-Null } catch { }
      $sw.Stop()
      $stage = "collect-after-timeout"
      # Same reasoning: a faulted read task on a killed child must not cost us
      # the 124. Whatever was captured before the kill is still worth keeping.
      $partial = ""
      try { $partial = $outTask.Result + $errTask.Result } catch {
        $partial = "[output unavailable: read task faulted after kill - $($_.Exception.GetType().Name)]"
      }
      $out = $partial + "`n[TIMEOUT after $TimeoutSec seconds]"
      $code = 124
    } else {
      $sw.Stop()
      $stage = "collect"
      # BOUNDED DRAIN. `$task.Result` waits for the read to COMPLETE, and a read
      # on a redirected pipe completes at EOF - which happens when the last
      # handle to the write end closes, NOT when the child exits. So a daemon
      # that inherited this child's stdout/stderr keeps the pipe open and
      # `.Result` blocks forever: WaitForExit already returned true, the child is
      # gone, there is no timeout and no error, and the phase log simply stops
      # mid-drill.
      #
      # That is not hypothetical. A7-backend-outage hung exactly this way for
      # 28 minutes (reproduced standalone 2026-08-25): silo, restarted by the
      # drill, inherited the harness's stdout via `Start-Process
      # -RedirectStandardOutput` and pinned it. Proven by killing silo - the
      # harness's own log file, still locked with every powershell and mediagit
      # process already dead, became deletable the instant silo died.
      #
      # The daemon side is fixed where it belongs (start detached, so nothing is
      # inherited), but this function takes a TimeoutSec and must honour it no
      # matter what a backend-cycling command supplied from outside the suite
      # decides to spawn. An unbounded wait inside a timed API is a bug on its
      # own terms.
      $drained = $false
      try {
        $drained = $outTask.Wait($script:QaDrainTimeoutMs) -and $errTask.Wait($script:QaDrainTimeoutMs)
      } catch { $drained = $false }
      if ($drained) {
        $out = $outTask.Result + $errTask.Result
      } else {
        $out = "[INVOKE-MG-DRAIN-TIMEOUT] child exited but its stdout/stderr pipe never reached EOF " +
               "within $([int]($script:QaDrainTimeoutMs / 1000))s. Some still-running process inherited " +
               "this child's handles - look for a daemon started with " +
               "Start-Process -RedirectStandardOutput. Command: $($QA.MG) $argLine"
      }
      $code = $proc.ExitCode
    }
  } catch {
    # A stream/pipe fault surfacing here (as above) must not escape this call and
    # unwind past the drill that made it - that is what turned one harness fault
    # into three voided drills. Report it as a harness-side ERROR result instead of
    # throwing; the caller decides how to record it (see Get-QaVerdict "ERROR").
    $sw.Stop()
    $ex = $_.Exception

    # `"$_"` is the MESSAGE ONLY. No type, no inner exception, no stack - which
    # is exactly why "Stream was not readable" has been recorded three times and
    # diagnosed zero times. Get-QaExceptionDetail (below in this file) is what
    # was missing; it is shared with every other catch that logs a harness fault
    # so the fix does not need re-deriving at each call site.
    $xd = Get-QaExceptionDetail $_
    $exType = $xd.ExType
    # Statement form, not `$x = try {...} catch {...}`: `try` is not an
    # expression in PS 5.1 (which this harness targets), so the assignment
    # form silently yields $null and every one of these fields logs blank --
    # which is exactly the "recorded nothing while looking like it recorded"
    # failure this whole block exists to end. Caught by probing it.
    # Gate on $started rather than catching around .HasExited: PowerShell makes
    # a throwing property access NON-terminating, so `try { $proc.HasExited }
    # catch {}` never enters the catch and just yields $null -- logging an empty
    # field that reads as "no data" when the truth is "there was no child".
    # Probed: the catch version printed `hasExited=` and the fix prints
    # `hasExited=n/a (never started)`.
    $hasExited = "n/a (never started)"; $childExit = "n/a"; $handles = "unknown"
    if ($started) {
      $hasExited = "$($proc.HasExited)"
      $childExit = $(if ($proc.HasExited) { "$($proc.ExitCode)" } else { "still running" })
    }
    try { $handles = "$([Diagnostics.Process]::GetCurrentProcess().HandleCount)" } catch {}

    $detail = (@(
      "[INVOKE-MG-ERROR] stage=$stage type=$exType"
    ) + $xd.Lines + @(
      "child: hasExited=$hasExited exitCode=$childExit elapsed=$([math]::Round($sw.Elapsed.TotalSeconds,2))s"
      "harness-process-handles: $handles"
      "command: $($QA.MG) $argLine"
    )) | Where-Object { $_ } | Out-String

    $out = $detail
    $code = -1

    # One collated file for the whole campaign. Per-phase logs are where these
    # faults went to die: by the time anyone looked, the interesting run was
    # buried under a dozen phases of ordinary output.
    try { $detail | Add-Content (Join-Path $QA.Logs "harness-faults.log") -Encoding UTF8 } catch {}
    Write-Warning "harness fault at stage '$stage' ($exType) - see logs\harness-faults.log"
  } finally {
    $proc.Dispose()
  }

  $log = Join-Path $QA.Logs "$Phase-cmds.log"
  Add-QaContentRetry $log ("### mediagit {0}  (repo={1} exit={2} sec={3:n1})" -f ($MgArgs -join " "), $Repo, $code, $sw.Elapsed.TotalSeconds)
  Add-QaContentRetry $log $out

  # Stall visibility. Most callers discard this result with `| Out-Null` (11
  # pushes + 10 clones across the suite), so a command that takes absurdly long
  # while still exiting 0 is invisible: on 2026-08-03 an 8 MiB push took 1,188s
  # against a loopback backend and the phase reported PASS.
  #
  # A WARNING, not a gate, on purpose. S4 legitimately pushes 4 GB for minutes,
  # so a blanket absolute-time gate would false-fire - see the S2 note in
  # 10_scale.ps1 on absolute-time gates encoding this machine's speed rather
  # than a defect. Enforcement stays per-drill where the payload size is known
  # (e.g. A4 in 07_abuse.ps1).
  $stallWarnSec = [int](_Env "MG_QA_STALL_WARN_SEC" "600")
  if ($sw.Elapsed.TotalSeconds -ge $stallWarnSec) {
    Write-QaLog $Phase ("SLOW: mediagit {0} took {1:n1}s (>= MG_QA_STALL_WARN_SEC={2}s, exit={3}) - possible stall" `
      -f ($MgArgs -join " "), $sw.Elapsed.TotalSeconds, $stallWarnSec, $code)
  }

  return @{ Exit = $code; Sec = [math]::Round($sw.Elapsed.TotalSeconds, 2); Out = $out }
}

# Fresh sandbox repo under work/. Returns path.
function New-SandboxRepo([string]$Name, [string]$Phase = "misc") {
  $p = Join-Path $QA.Work $Name
  if (Test-Path $p) { Remove-Item -Recurse -Force $p }
  New-Item -ItemType Directory -Path $p -Force | Out-Null
  $r = Invoke-MG $null @("init", $p) $Phase
  if ($r.Exit -ne 0) { throw "init failed for $p : $($r.Out)" }
  return $p
}

function Get-QaHash([string]$Path) { (Get-FileHash -Algorithm SHA256 -Path $Path).Hash }

function Get-DirMB([string]$Path, [switch]$ExcludeOdb) {
  $f = Get-ChildItem $Path -Recurse -File -EA SilentlyContinue
  if ($ExcludeOdb) { $f = $f | Where-Object { $_.FullName -notmatch '\\\.mediagit\\' } }
  return [math]::Round((($f | Measure-Object Length -Sum).Sum) / 1MB, 2)
}

# Tier filter: drop fixtures above the size cap (STANDARD=500MB, STRESS=unlimited).
function Select-TierFiles([string[]]$Paths) {
  $Paths | Where-Object { (Test-Path $_) -and ((Get-Item $_).Length / 1MB) -le $QA.MaxFixtureMB }
}

# ---------------------------------------------------------------------------
# Verdicts. A gate is one of exactly five values in gates.tsv's `pass` column:
#   True  - checked, held.
#   False - checked, failed. A measured product defect.
#   SKIP  - NOT checked (capability/credential absent). Never a pass. A campaign
#           whose gates are all SKIP has verified nothing and must not read green.
#   WARN  - checked, informational only (e.g. no perf baseline exists yet).
#   ERROR - NOT checked: the drill itself blew up (harness/infra fault - a stream
#           fault, a server that vanished mid-drill, etc.) before it could measure
#           anything. Distinct from False on purpose: a drill that failed to run is
#           not evidence the product is broken, and folding it into False turns a
#           harness bug into a false product-failure report (2026-08-04 postmortem,
#           10_scale S2/S3/S4 - see Invoke-MG's catch in this file).
# Anything that is not recognisably one of these is False: an unset/garbled verdict
# is a harness bug, and the safe reading of "we don't know" is "not proven".
# ---------------------------------------------------------------------------
function Get-QaVerdict($Pass) {
  $s = ("" + $Pass).Trim().ToUpper()
  if ($s -eq "SKIP") { return "SKIP" }
  if ($s -eq "WARN") { return "WARN" }
  if ($s -eq "ERROR") { return "ERROR" }
  if ($Pass -eq $true -or $s -eq "TRUE") { return "True" }
  return "False"
}

# Marker embedded in the detail of skips the operator asked for by NOT selecting a
# backend. Those are the only skips that may leave an all-skip phase green - a skip
# from missing credentials on a SELECTED backend, or from dead infrastructure, is a
# phase that proved nothing.
$QA_SKIP_NOT_SELECTED = "not selected (MG_QA_BACKENDS)"

# Gate helper: record the verdict in the phase gate TSV; the exit code is Exit-QaPhase's job.
function Write-QaGate([string]$Phase, [string]$Gate, $Pass, [string]$Detail = "") {
  $v = Get-QaVerdict $Pass
  Write-QaRow (Join-Path $QA.Logs "gates.tsv") @("phase", "gate", "pass", "detail") @($Phase, $Gate, $v, $Detail)
  Write-QaLog $Phase ("GATE {0} = {1} {2}" -f $Gate, $(if ($v -eq "True") { "PASS" } else { $v.ToUpper() }), $Detail)
}

# Phase exit contract, enforced in ONE place so no phase can invent a friendlier one.
# Reads back this phase's rows from gates.tsv (the same record the report aggregates,
# so the exit code and the report can never disagree) and exits:
#   any False                                     -> 1  failure
#   no True at all, and a skip that wasn't asked  -> 3  nothing verified
#   otherwise                                     -> 0
# $ExtraFail is for phases that also record failures outside gates.tsv (the persona
# scripts gate only fsck, but their step rows can fail): pass (-not $script:AllPass)
# so a step failure cannot be hidden by a phase whose gates all happen to be green.
# Call as the LAST statement of a phase script: it does not return.
function Exit-QaPhase([string]$Phase, [bool]$ExtraFail = $false) {
  $rows = @()
  $gatesTsv = Join-Path $QA.Logs "gates.tsv"
  if (Test-Path $gatesTsv) {
    $lines = Get-Content $gatesTsv
    if ($lines -and $lines.Count -ge 2) {
      $rows = @($lines | ConvertFrom-Csv -Delimiter "`t" | Where-Object { $_.phase -eq $Phase })
    }
  }
  $pass = @($rows | Where-Object { $_.pass -eq "True" }).Count
  $fail = @($rows | Where-Object { $_.pass -eq "False" }).Count
  $skip = @($rows | Where-Object { $_.pass -eq "SKIP" }).Count
  $warn = @($rows | Where-Object { $_.pass -eq "WARN" }).Count
  $err  = @($rows | Where-Object { $_.pass -eq "ERROR" }).Count
  $unexpectedSkip = @($rows | Where-Object {
      $_.pass -eq "SKIP" -and ("" + $_.detail) -notlike ("*" + $QA_SKIP_NOT_SELECTED + "*")
    }).Count

  # A phase that recorded NO gates at all verified nothing, whatever else it printed.
  # This is the shape a phase takes when it dies early - a missing dot-source, a helper
  # that throws, a server that never starts - and reporting PASS for it is the same
  # greenwashing as counting a SKIP as a pass. Zero rows is never success.
  #
  # ERROR sits below FAIL and above PASS on purpose: it must not read as a product
  # defect (that's what FAIL is for), but a phase that errored out cannot read as a
  # clean PASS either - something there was never measured. exit 2 keeps it out of
  # both of run_all.ps1's other buckets.
  $verdict =
    if ($fail -gt 0 -or $ExtraFail) { "FAIL" }
    elseif ($rows.Count -eq 0) { "NOTHING-VERIFIED" }
    elseif ($pass -eq 0 -and $unexpectedSkip -gt 0) { "NOTHING-VERIFIED" }
    elseif ($err -gt 0) { "ERROR" }
    else { "PASS" }
  Write-QaLog $Phase ("=== {0} done: {1} (pass={2} fail={3} skip={4} warn={5} error={6} unexpected-skip={7} extra-fail={8}) ===" -f `
      $Phase, $verdict, $pass, $fail, $skip, $warn, $err, $unexpectedSkip, $ExtraFail)

  switch ($verdict) {
    "FAIL" { exit 1 }
    "NOTHING-VERIFIED" { exit 3 }
    "ERROR" { exit 2 }
    default { exit 0 }
  }
}

# Sorted "hash  relpath" lines for every non-.mediagit file: full-tree content parity.
# Used wherever a clone/pull must be proven byte-identical to its source - Test-Path
# on the destination proves only that a directory exists, never that it holds the
# right bytes, and that is exactly the class of defect these drills exist to catch.
function Get-QaTreeHashes([string]$Root) {
  $full = (Get-Item $Root).FullName
  Get-ChildItem $full -Recurse -File -ErrorAction SilentlyContinue |
    Where-Object { $_.FullName -notmatch '\\\.mediagit\\' } |
    ForEach-Object { "{0}  {1}" -f (Get-QaHash $_.FullName), $_.FullName.Substring($full.Length + 1) } |
    Sort-Object
}

# Resolve a repo's chunk-deltas directory, whatever its object namespace is.
# Returns $null when the repo has no chunk-delta storage yet.
function Get-QaChunkDeltaDir([string]$Repo) {
  $objects = Join-Path $Repo ".mediagit\objects"
  if (-not (Test-Path $objects)) { return $null }
  $hit = Get-ChildItem $objects -Directory -EA SilentlyContinue | ForEach-Object {
    $c = Join-Path $_.FullName "chunk-deltas"
    if (Test-Path $c) { $c }
  } | Select-Object -First 1
  return $hit
}

# Chunk-delta chain topology of a repo, read straight off the .meta sidecars.
#
# fsck's own chain walk reads only these sidecars (never chunk payloads), so
# this needs no binary and is cheap even on large repos. Returns:
#   MaxDepth   deepest chain, in delta hops above a full chunk; -1 when the repo
#              has NO chunk-delta storage at all
#   CycleCount chains that revisit a node (self-loop or longer cycle)
#   ChainCount number of chunk-delta sidecars found
#
# Guards the class of defect where a repo becomes unreadable because a chain
# grew past what the reader will reconstruct (MAX_DELTA_DEPTH = 10).
#
# MaxDepth is -1, not 0, when there is no chunk-deltas directory: "no chains exist"
# and "chains exist and are all depth 0" are different facts, and collapsing them
# makes a depth gate pass a workload that silently stopped producing deltas at all.
# Callers must branch on -1 explicitly.
function Get-QaChainStats([string]$Repo) {
  $stats = @{ MaxDepth = -1; CycleCount = 0; ChainCount = 0 }
  # Objects live under .mediagit\objects\<repo_namespace>\, and the namespace
  # is the repo directory name - not a literal "repo". Discover it instead of
  # assuming, or this silently reports zero chains on every real repository.
  $deltaDir = Get-QaChunkDeltaDir $Repo
  if (-not $deltaDir) { return $stats }

  # base map: <chunk hex> -> <base hex>
  $bases = @{}
  Get-ChildItem $deltaDir -Recurse -File -Filter "*.meta" -EA SilentlyContinue | ForEach-Object {
    $txt = (Get-Content $_.FullName -Raw -EA SilentlyContinue)
    if ($txt -and $txt.Trim() -match '^base:([0-9a-f]+)') {
      $bases[$_.BaseName] = $Matches[1]
    }
  }
  $stats.ChainCount = $bases.Count
  if ($bases.Count -eq 0) { return $stats }
  $stats.MaxDepth = 0

  foreach ($start in $bases.Keys) {
    $seen = New-Object 'System.Collections.Generic.HashSet[string]'
    $cur = $start
    $depth = 0
    while ($bases.ContainsKey($cur)) {
      if (-not $seen.Add($cur)) { $stats.CycleCount++; break }
      $cur = $bases[$cur]
      $depth++
      # Hard stop well above any legal chain so a malformed repo cannot hang
      # the harness; a chain this long is already a failure by definition.
      if ($depth -gt 200) { $stats.CycleCount++; break }
    }
    if ($depth -gt $stats.MaxDepth) { $stats.MaxDepth = $depth }
  }
  return $stats
}

# ---------------------------------------------------------------------------
# Scale-tier helpers (phase 10). All ASCII / PS 5.1 compatible.
# ---------------------------------------------------------------------------

# Cloud analogue of Select-TierFiles: caps payload sent to (slow, billed) cloud
# backends at $QA.CloudMaxMB while minio/local get the full scale corpus.
function Select-CloudTierFiles([string[]]$Paths) {
  $Paths | Where-Object { (Test-Path $_) -and ((Get-Item $_).Length / 1MB) -le $QA.CloudMaxMB }
}

# Free space (GB) on the volume backing $Path - used by 01_preflight to refuse a
# SCALE run that cannot fit the disk budget.
function Get-QaFreeDiskGB([string]$Path) {
  $root = [System.IO.Path]::GetPathRoot((Resolve-Path $Path).Path)
  try { return [math]::Round((New-Object System.IO.DriveInfo($root)).AvailableFreeSpace / 1GB, 1) }
  catch { return -1 }
}

# Run $Action while sampling memory of the mediagit process(es) in a background job
# (Invoke-MG blocks, so in-process sampling can't observe it).
#
# Samples PeakWorkingSet64, not just WorkingSet64: PeakWorkingSet64 is the kernel's
# own high-water mark over the process lifetime, so a spike between two polls is still
# recorded, whereas polled WorkingSet64 only ever sees the instants it happens to land
# on and under-reports every transient allocation - the exact shape an RSS ceiling gate
# is supposed to catch. WorkingSet64 is still sampled for the trail, and
# PrivateMemorySize64 alongside it (working set excludes paged-out pages and includes
# shared ones; private commit is the number that tracks a real leak).
#
# Client and server peaks are reported separately - "peak RSS was 3 GB" is not
# actionable until you know which side of the wire spent it.
#
# Returns @{ ClientPeakMB; ServerPeakMB; ClientPrivatePeakMB; ServerPrivatePeakMB;
#            PeakMB (max of all, legacy); SamplesTsv; Result }.
# ponytail: 100ms polling + kernel peak. Process names are inlined in the job: passing
# an array through Start-Job -ArgumentList nests it and Get-Process -Name matches nothing.
function Measure-PeakRSS {
  param(
    [Parameter(Mandatory = $true)][scriptblock]$Action,
    [string]$Phase = "misc",
    [string]$Label = "rss"
  )
  $stamp = (Get-Date -Format "HHmmss") + "-" + [guid]::NewGuid().ToString("N").Substring(0, 6)
  $samplesTsv = Join-Path $QA.Logs ("$Phase-$Label-$stamp.tsv")
  $peakFile = Join-Path $QA.Work ("rss-" + $stamp + ".txt")
  $stopFile = "$peakFile.stop"
  # client_ws, server_ws, client_priv, server_priv  (bytes)
  "0`t0`t0`t0" | Set-Content $peakFile -Encoding Ascii

  $sampler = Start-Job -ScriptBlock {
    param($pf, $sf, $tsv)
    "time`tpid`tname`tws_mb`tpeak_ws_mb`tprivate_mb" | Set-Content $tsv -Encoding Ascii
    $cWs = 0L; $sWs = 0L; $cPriv = 0L; $sPriv = 0L
    while (-not (Test-Path $sf)) {
      foreach ($p in (Get-Process -Name "mediagit", "mediagit-server" -ErrorAction SilentlyContinue)) {
        $isServer = ($p.ProcessName -eq "mediagit-server")
        # PeakWorkingSet64 is monotonic per process; max across processes of a kind.
        if ($isServer) {
          if ($p.PeakWorkingSet64 -gt $sWs) { $sWs = $p.PeakWorkingSet64 }
          if ($p.PrivateMemorySize64 -gt $sPriv) { $sPriv = $p.PrivateMemorySize64 }
        } else {
          if ($p.PeakWorkingSet64 -gt $cWs) { $cWs = $p.PeakWorkingSet64 }
          if ($p.PrivateMemorySize64 -gt $cPriv) { $cPriv = $p.PrivateMemorySize64 }
        }
        ("{0}`t{1}`t{2}`t{3}`t{4}`t{5}" -f (Get-Date -Format "HH:mm:ss.fff"), $p.Id, $p.ProcessName,
          [math]::Round($p.WorkingSet64 / 1MB, 1), [math]::Round($p.PeakWorkingSet64 / 1MB, 1),
          [math]::Round($p.PrivateMemorySize64 / 1MB, 1)) | Add-Content $tsv -Encoding Ascii
      }
      "$cWs`t$sWs`t$cPriv`t$sPriv" | Set-Content $pf -Encoding Ascii
      Start-Sleep -Milliseconds 100
    }
  } -ArgumentList $peakFile, $stopFile, $samplesTsv

  $result = $null
  try { $result = & $Action }
  finally {
    New-Item $stopFile -ItemType File -Force | Out-Null
    Wait-Job $sampler -Timeout 5 | Out-Null
    Stop-Job $sampler -ErrorAction SilentlyContinue
    Remove-Job $sampler -Force -ErrorAction SilentlyContinue
  }

  $vals = @(0L, 0L, 0L, 0L)
  if (Test-Path $peakFile) {
    $parts = ((Get-Content $peakFile -Raw) + "").Trim() -split "`t"
    for ($i = 0; $i -lt 4 -and $i -lt $parts.Count; $i++) {
      $n = 0L; [long]::TryParse($parts[$i], [ref]$n) | Out-Null; $vals[$i] = $n
    }
  }
  Remove-Item $peakFile, $stopFile -Force -ErrorAction SilentlyContinue
  $mb = { param($b) [math]::Round($b / 1MB, 1) }
  # A process the sampler never saw is UNMEASURED, not 0 MB. Reporting 0 reads as
  # "looked, found nothing" - the same lie as counting a SKIP as a pass - and it is why
  # S4-local silently claimed serverPeak=0MB while running no server at all. Callers that
  # want a number should test for "n/a" first.
  $mbOrNa = { param($b) if ($b -le 0) { "n/a" } else { [math]::Round($b / 1MB, 1) } }
  return @{
    ClientPeakMB        = (& $mb $vals[0])
    ServerPeakMB        = (& $mbOrNa $vals[1])
    ClientPrivatePeakMB = (& $mb $vals[2])
    ServerPrivatePeakMB = (& $mbOrNa $vals[3])
    PeakMB              = (& $mb ([Math]::Max([Math]::Max($vals[0], $vals[1]), [Math]::Max($vals[2], $vals[3]))))
    SamplesTsv          = $samplesTsv
    Result              = $result
  }
}

# Purge a phase's own work/ scratch. Phases call this from a finally block so a killed
# or failed drill cannot leave tens of GB behind (a SCALE campaign fills a disk in one
# run, and the next phase then fails for reasons that have nothing to do with the code).
#
# $Patterns are -like wildcards matched against work/ subdirectory NAMES. Server data
# dirs are purged automatically: Start-QaServer names them server-<backend>-<phase>-<n>,
# so every phase's own servers are covered without each one restating the pattern.
#
# Deletes inside work/ ONLY - logs/, reports/ and fixtures-synthetic/ are never touched
# from here, and MG_QA_KEEP_SCRATCH=1 preserves everything for triage.
function Invoke-QaTeardown([string]$Phase, [string[]]$Patterns) {
  if ($QA.KeepScratch) { Write-QaLog $Phase "MG_QA_KEEP_SCRATCH=1 - scratch preserved under $($QA.Work)"; return }
  if (-not (Test-Path $QA.Work)) { return }
  $all = @($Patterns) + @("server-*-$Phase-*")
  $before = Get-DirMB $QA.Work
  Get-ChildItem $QA.Work -Directory -ErrorAction SilentlyContinue |
    Where-Object { $n = $_.Name; @($all | Where-Object { $n -like $_ }).Count -gt 0 } |
    ForEach-Object { Remove-Item -Recurse -Force $_.FullName -ErrorAction SilentlyContinue }
  $after = Get-DirMB $QA.Work
  Write-QaLog $Phase ("teardown reclaimed {0} MB (work/ {1} -> {2} MB)" -f [math]::Round($before - $after, 1), $before, $after)
}


# Did a push actually USE the cloud-pack fast path, or did it silently degrade?
#
# 20260821-ga11 pushed to Azure at 0.98 MB/s against 8.69 in ga8 - a 8.9x
# slowdown - because ONE transient status on ONE pack made the client abandon
# packs for the ENTIRE push and upload 2,284 chunks one at a time through the
# server proxy. The push SUCCEEDED. Integrity was perfect. Only the clock moved.
#
# The single reason it was ever noticed is that Azure happened to land at 0.98
# against a 1.0 floor. On a slightly faster link the identical fallback passes
# silently: the throughput floor measures speed, and speed is only a proxy for
# the thing actually at stake. This asks the direct question instead, so the
# answer does not depend on how good the operator's uplink was that afternoon.
#
# Three INFO-level server messages, all confirmed present in real campaign logs:
#   "Presigned pack upload URLs generated" (count=N)  - fast path was OFFERED
#   "Pack manifest registered"                        - a pack COMPLETED
#   "PUT chunk"                                       - a per-chunk PROXY upload
#
# INFO rather than the tower_http request lines on purpose: those are DEBUG and
# vanish if a server ever runs at info, which would quietly turn this into a
# gate that cannot fail.
#
# count=N is SUMMED rather than the lines counted: one request may mint URLs for
# many packs. They were 1:1 across all five backends in ga8, which is exactly
# the kind of coincidence that becomes a false gate the day the client batches.
#
# ANSI is stripped first. tracing writes `count<ESC>[2m=<ESC>[0m32`, so a plain
# count=(\d+) matches nothing whatsoever against a real log.
function Get-QaPackFastPathCounts([string]$LogPath) {
  if (-not $LogPath -or -not (Test-Path $LogPath)) { return $null }
  $txt = Get-Content $LogPath -Raw -ErrorAction SilentlyContinue
  if (-not $txt) { return $null }
  $txt = $txt -replace "\x1b\[[0-9;]*m", ""
  $offered = 0
  foreach ($m in [regex]::Matches($txt, "Presigned pack upload URLs generated[^\r\n]*?count=(\d+)")) {
    $offered += [int]$m.Groups[1].Value
  }
  return [pscustomobject]@{
    Offered   = $offered
    Completed = ([regex]::Matches($txt, "Pack manifest registered")).Count
    ChunkPuts = ([regex]::Matches($txt, "PUT chunk")).Count
  }
}
