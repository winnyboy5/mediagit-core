# qa-suite shared helpers. ASCII-only, PS 5.1 compatible.
# Scripts dot-source ONLY this file; it pulls in config.ps1 (defines $QA).
. (Join-Path (Split-Path (Split-Path $PSScriptRoot -Parent) -Parent) "config.ps1")

$ErrorActionPreference = "Continue"

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

# Gate helper: record pass/fail in the phase gate TSV; nonzero exit is caller's job.
function Write-QaGate([string]$Phase, [string]$Gate, [bool]$Pass, [string]$Detail = "") {
  Write-QaRow (Join-Path $QA.Logs "gates.tsv") @("phase", "gate", "pass", "detail") @($Phase, $Gate, $Pass, $Detail)
  Write-QaLog $Phase ("GATE {0} = {1} {2}" -f $Gate, $(if ($Pass) { "PASS" } else { "FAIL" }), $Detail)
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
#   MaxDepth   deepest chain, in delta hops above a full chunk
#   CycleCount chains that revisit a node (self-loop or longer cycle)
#   ChainCount number of chunk-delta sidecars found
#
# Guards the class of defect where a repo becomes unreadable because a chain
# grew past what the reader will reconstruct (MAX_DELTA_DEPTH = 10).
function Get-QaChainStats([string]$Repo) {
  $stats = @{ MaxDepth = 0; CycleCount = 0; ChainCount = 0 }
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

# Run $Action while sampling peak WorkingSet64 of the mediagit process(es) in a
# background job (Invoke-MG blocks, so in-process sampling can't observe it).
# Returns @{ PeakMB; Result } where Result is whatever $Action returned.
# ponytail: 200ms polling approximates the true peak - fine for a ceiling gate,
# not a profiler. Process names are inlined in the job: passing an array through
# Start-Job -ArgumentList nests it and Get-Process -Name then matches nothing.
function Measure-PeakRSS {
  param([Parameter(Mandatory = $true)][scriptblock]$Action)
  $peakFile = Join-Path $QA.Work ("rss-" + [guid]::NewGuid().ToString("N") + ".txt")
  $stopFile = "$peakFile.stop"
  "0" | Set-Content $peakFile -Encoding Ascii
  $sampler = Start-Job -ScriptBlock {
    param($pf, $sf)
    $peak = 0L
    while (-not (Test-Path $sf)) {
      foreach ($p in (Get-Process -Name "mediagit", "mediagit-server" -ErrorAction SilentlyContinue)) {
        if ($p.WorkingSet64 -gt $peak) { $peak = $p.WorkingSet64 }
      }
      $peak | Set-Content $pf -Encoding Ascii
      Start-Sleep -Milliseconds 200
    }
  } -ArgumentList $peakFile, $stopFile

  $result = $null
  try { $result = & $Action }
  finally {
    New-Item $stopFile -ItemType File -Force | Out-Null
    Wait-Job $sampler -Timeout 5 | Out-Null
    Stop-Job $sampler -ErrorAction SilentlyContinue
    Remove-Job $sampler -Force -ErrorAction SilentlyContinue
  }
  $peakBytes = 0L
  if (Test-Path $peakFile) { [long]::TryParse(((Get-Content $peakFile -Raw) + "").Trim(), [ref]$peakBytes) | Out-Null }
  Remove-Item $peakFile, $stopFile -Force -ErrorAction SilentlyContinue
  return @{ PeakMB = [math]::Round($peakBytes / 1MB, 1); Result = $result }
}
