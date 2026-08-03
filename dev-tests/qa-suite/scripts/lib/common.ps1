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

function Write-QaLog([string]$Phase, [string]$Msg) {
  $line = "{0} [{1}] {2}" -f (Get-Date -Format "HH:mm:ss"), $Phase, $Msg
  $line | Add-Content (Join-Path $QA.Logs "$Phase.log")
  Write-Host $line
}

# TSV row writer: creates file with header on first write, appends tab-joined sanitized values.
function Write-QaRow([string]$Path, [string[]]$Header, [object[]]$Values) {
  if (-not (Test-Path $Path)) { ($Header -join "`t") | Set-Content $Path -Encoding ASCII }
  $clean = $Values | ForEach-Object { ("" + $_) -replace "`t", " " -replace "`r?`n", " | " }
  ($clean -join "`t") | Add-Content $Path -Encoding ASCII
}

# Run mediagit against a repo. Returns @{Exit; Sec; Out} - Out is combined stdout+stderr text.
# Full output also appended to $QA.Logs\<Phase>-cmds.log for post-hoc digging.
# Enforces $TimeoutSec: on timeout, kills process tree and returns exit 124.
# -StdIn: lines fed to the child's stdin in order (one dialoguer Input/Password
# prompt per line), for driving interactive commands like `mediagit auth login`
# non-interactively. Omit (default) for every existing non-interactive call.
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

  $proc.Start() | Out-Null
  if ($StdIn) {
    foreach ($line in $StdIn) { $proc.StandardInput.WriteLine($line) }
    $proc.StandardInput.Close()
  }
  # Threadpool drain: ReadToEnd-after-WaitForExit deadlocks once the child fills
  # the pipe buffer; async tasks drain continuously without the PS event loop.
  $outTask = $proc.StandardOutput.ReadToEndAsync()
  $errTask = $proc.StandardError.ReadToEndAsync()

  if (-not $proc.WaitForExit($TimeoutSec * 1000)) {
    taskkill /T /F /PID $proc.Id 2>$null | Out-Null
    $proc.WaitForExit() | Out-Null   # pipes close on kill; tasks then complete
    $sw.Stop()
    $out = $outTask.Result + $errTask.Result + "`n[TIMEOUT after $TimeoutSec seconds]"
    $code = 124
  } else {
    $sw.Stop()
    $out = $outTask.Result + $errTask.Result
    $code = $proc.ExitCode
  }

  $log = Join-Path $QA.Logs "$Phase-cmds.log"
  ("### mediagit {0}  (repo={1} exit={2} sec={3:n1})" -f ($MgArgs -join " "), $Repo, $code, $sw.Elapsed.TotalSeconds) | Add-Content $log
  $out | Add-Content $log

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
# Verdicts. A gate is one of exactly four values in gates.tsv's `pass` column:
#   True  - checked, held.
#   False - checked, failed.
#   SKIP  - NOT checked (capability/credential absent). Never a pass. A campaign
#           whose gates are all SKIP has verified nothing and must not read green.
#   WARN  - checked, informational only (e.g. no perf baseline exists yet).
# Anything that is not recognisably one of these is False: an unset/garbled verdict
# is a harness bug, and the safe reading of "we don't know" is "not proven".
# ---------------------------------------------------------------------------
function Get-QaVerdict($Pass) {
  $s = ("" + $Pass).Trim().ToUpper()
  if ($s -eq "SKIP") { return "SKIP" }
  if ($s -eq "WARN") { return "WARN" }
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
  $unexpectedSkip = @($rows | Where-Object {
      $_.pass -eq "SKIP" -and ("" + $_.detail) -notlike ("*" + $QA_SKIP_NOT_SELECTED + "*")
    }).Count

  # A phase that recorded NO gates at all verified nothing, whatever else it printed.
  # This is the shape a phase takes when it dies early - a missing dot-source, a helper
  # that throws, a server that never starts - and reporting PASS for it is the same
  # greenwashing as counting a SKIP as a pass. Zero rows is never success.
  $verdict =
    if ($fail -gt 0 -or $ExtraFail) { "FAIL" }
    elseif ($rows.Count -eq 0) { "NOTHING-VERIFIED" }
    elseif ($pass -eq 0 -and $unexpectedSkip -gt 0) { "NOTHING-VERIFIED" }
    else { "PASS" }
  Write-QaLog $Phase ("=== {0} done: {1} (pass={2} fail={3} skip={4} warn={5} unexpected-skip={6} extra-fail={7}) ===" -f `
      $Phase, $verdict, $pass, $fail, $skip, $warn, $unexpectedSkip, $ExtraFail)

  switch ($verdict) {
    "FAIL" { exit 1 }
    "NOTHING-VERIFIED" { exit 3 }
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
