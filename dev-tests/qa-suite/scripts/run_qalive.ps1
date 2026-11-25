# Live QA campaign runner: Playwright driving the real release desktop app + CLI
# (ui/tests/e2e/qa-live), with a pass/fail verdict and cleanup, like run_ga.ps1.
# See ui/tests/e2e/qa-live/README.md. ASCII-only, PS 5.1.
#
# -Cases is a comma-separated list of case ids ("000,S-01,T-06b"), matched exactly
# (an id never matches a longer id or a file name). Empty = every case.
#
# Verdict: ui/tests/e2e/qa-live/summarize.mjs reads the per-case JSONs and writes
# <run>\gates.tsv. Playwright's own exit code is NOT the verdict (checks are soft:
# it exits 0 with failed checks), but a non-zero Playwright exit also fails the run.
#
# Exit codes: 0 verdict PASS (>= 1 case passed, none failed) and cleanup ok;
# 1 verdict FAIL or no gates.tsv; 2 Playwright exited non-zero while the verdict passed;
# 3 cleanup failed (also -CleanCloud while a user mediagit process runs: the cloud script
# refuses then); 70 another qa-live Playwright run is already active; 71 unsafe work
# path; 72 missing prerequisite; 73 bad -Cases / -RunId.
#
# Cleanup touches only what THIS run created:
#  - <QA root>\work (path asserted to end in qa-live\work);
#  - processes: the PIDs in work\state.json (with their trees) and descendants of this
#    script, never another mediagit/node process (those are only reported);
#  - Credential Manager entries "http://127.0.0.1:<port>[/repo].mediagit" (or localhost),
#    port >= 49152, that were NOT present at run start. They are written to
#    <QA root>\keychain-ledger.txt before deletion; the next run deletes leftovers
#    from the ledger (a crash between create and ledger write is the one gap).
# The run folder (<QA root>\<RunId>) is always kept.

param(
  [string]$Cases = "",
  [string]$RunId = ("qalive-" + (Get-Date -Format "yyyyMMdd-HHmmss")),
  [switch]$CleanCloud,
  [switch]$SkipCleanup
)

$ErrorActionPreference = "Continue"
function Say([string]$m) { Write-Host ("[qalive] " + $m) }

if ($RunId -notmatch '^[A-Za-z0-9._-]+$' -or $RunId -eq "work" -or $RunId -match '[.]$') {
  Say "FATAL: bad -RunId '$RunId' (letters, digits, . _ - only; not 'work'; no trailing dot, Windows treats 'work.' as 'work')"; exit 73
}
if ($RunId -match '^(CON|PRN|AUX|NUL|COM[1-9]|LPT[1-9])([.].*)?$') { Say "FATAL: -RunId '$RunId' is a Windows device name"; exit 73 }
if (@("keychain-ledger.txt", "status.md") -contains $RunId.ToLower()) { Say "FATAL: -RunId '$RunId' is a reserved file name"; exit 73 }
$ids = @()
if ($Cases) {
  $ids = @($Cases -split ',' | ForEach-Object { $_.Trim() } | Where-Object { $_ })
  foreach ($i in $ids) { if ($i -notmatch '^[A-Za-z0-9-]+$') { Say "FATAL: bad case id '$i' in -Cases (ids like 000,S-01,T-06b)"; exit 73 } }
}

$repo = (Resolve-Path (Join-Path $PSScriptRoot "..\..\..")).Path
$ui   = Join-Path $repo "ui"
$qaRoot = if ($env:MG_QA_ROOT) { $env:MG_QA_ROOT } else { Join-Path (Split-Path $repo -Parent) "qa-live" }
$qaRoot = [IO.Path]::GetFullPath($qaRoot).TrimEnd('\')
$work   = Join-Path $qaRoot "work"
$runDir = Join-Path $qaRoot $RunId
$ledger = Join-Path $qaRoot "keychain-ledger.txt"
$procNames = @("mediagit", "mediagit-server", "mediagit-desktop")

# The only folder this script ever deletes. Refuse anything not named <...qa-live>\work.
if ((Split-Path $work -Leaf) -ne "work" -or (Split-Path $qaRoot -Leaf) -notmatch 'qa-live$') {
  Say "FATAL: refusing to wipe '$work': the QA root folder name must end in 'qa-live' (set MG_QA_ROOT accordingly)"
  exit 71
}

if (Test-Path -LiteralPath $runDir -PathType Leaf) { Say "FATAL: '$runDir' exists and is a file"; exit 73 }
if ((Test-Path -LiteralPath $runDir -PathType Container) -and @(Get-ChildItem -LiteralPath $runDir -Force).Count -gt 0) {
  Say "FATAL: run folder '$runDir' already has files (a reused -RunId would let old results count). Pick a new -RunId."; exit 73
}

$runStart = Get-Date
$snap = @{}     # mediagit* processes that existed before this run (PID + start ticks): reported, never stopped
foreach ($p in (Get-Process -Name $procNames -ErrorAction SilentlyContinue)) {
  try { $snap[("{0}|{1}" -f $p.Id, $p.StartTime.Ticks)] = $true } catch { $snap[("{0}|?" -f $p.Id)] = $true }
}

# Only an already-running qa-live Playwright blocks a run (two would fight over the app and work\).
function Get-Busy {
  $busy = @()
  $pw = Get-CimInstance Win32_Process -Filter "Name='node.exe'" -ErrorAction SilentlyContinue |
        Where-Object { $_.ProcessId -ne $PID -and $_.CommandLine -match 'playwright.qa.config' }
  foreach ($p in $pw) { $busy += ("playwright pid={0}" -f $p.ProcessId) }
  return $busy
}

function Stop-Tree([int]$id) { & taskkill /PID $id /T /F 2>&1 | Out-Null }

# Stop what THIS run started: PIDs recorded in work\state.json (the detached desktop app is there),
# then every descendant of this script. Anything else that looks new is only reported.
function Stop-OurProcesses {
  $statePath = Join-Path $work "state.json"
  if (Test-Path $statePath) {
    try {
      $st0 = Get-Content $statePath -Raw | ConvertFrom-Json
      # procs = {pid, started (ms epoch at spawn), name}. PIDs are reused by Windows, so a PID is killed only if
      # a live process has that PID, started within a few seconds of the recorded spawn time, with a plausible name.
      foreach ($e in @($st0.procs)) {
        if (-not $e -or -not $e.pid) { continue }
        $p = Get-Process -Id $e.pid -ErrorAction SilentlyContinue
        if (-not $p) { continue }
        try { $st = $p.StartTime } catch { continue }
        $rec = [DateTimeOffset]::FromUnixTimeMilliseconds([int64]$e.started).LocalDateTime
        $delta = ($rec - $st).TotalSeconds
        if ($p.ProcessName -notmatch '^(mediagit|node|cmd|pnpm)' -or $delta -lt -2 -or $delta -gt 15) {
          Say ("  NOTE: recorded pid={0} is now {1} (started {2}): not the process this run started, left alone" -f $e.pid, $p.ProcessName, $st)
          continue
        }
        Say ("  stopping {0} pid={1} (recorded by this run)" -f $p.ProcessName, $e.pid)
        Stop-Tree $e.pid
      }
    } catch { Say "WARNING: could not read $statePath : $_" }
  }
  $all = @(Get-CimInstance Win32_Process -ErrorAction SilentlyContinue)
  $queue = New-Object System.Collections.Queue
  $queue.Enqueue([pscustomobject]@{ Id = $PID; Born = (Get-Process -Id $PID).StartTime })
  $seen = @{}
  while ($queue.Count -gt 0) {
    $par = $queue.Dequeue()
    foreach ($c in ($all | Where-Object { $_.ParentProcessId -eq $par.Id -and $_.ProcessId -ne $PID })) {
      if ($seen.ContainsKey($c.ProcessId) -or -not $c.CreationDate -or $c.CreationDate -lt $par.Born) { continue }
      $seen[$c.ProcessId] = $true
      $queue.Enqueue([pscustomobject]@{ Id = $c.ProcessId; Born = $c.CreationDate })
      if ($c.Name -match '^(mediagit|node|pnpm)') {
        Say ("  stopping {0} pid={1} (child of this run)" -f $c.Name, $c.ProcessId)
        Stop-Tree $c.ProcessId
      }
    }
  }
  Start-Sleep -Milliseconds 500
  foreach ($p in (Get-Process -Name $procNames -ErrorAction SilentlyContinue)) {
    try { $st = $p.StartTime } catch { continue }
    if ($snap.ContainsKey(("{0}|{1}" -f $p.Id, $st.Ticks)) -or $st -lt $runStart) { continue }
    Say ("  NOTE: {0} pid={1} started during the run but is not ours: left alone" -f $p.ProcessName, $p.Id)
  }
}

function Clear-Work {
  if (-not (Test-Path $work)) { return $true }
  for ($i = 0; $i -lt 5; $i++) {
    Remove-Item -LiteralPath $work -Recurse -Force -ErrorAction SilentlyContinue
    if (-not (Test-Path $work)) { return $true }
    Start-Sleep -Seconds 2
  }
  # L-08 makes names Win32 cannot delete (a trailing dot: "shot.v1."); the extended-length prefix can.
  cmd.exe /c rd /s /q ('\\?\' + $work) 2>&1 | Out-Null
  if (-not (Test-Path $work)) { return $true }
  Say "WARNING: could not fully remove $work"
  return $false
}

# CLI keyring: service "mediagit"; the Windows backend stores target "<account>.<service>"
# and the account is the server origin (repo.rs: keyring_account), e.g.
# "http://127.0.0.1:53859.mediagit". The desktop app also keeps ONE ephemeral-port server for the
# user, so presence alone proves nothing: only entries absent at run start are ours.
$kcPattern = 'Target:\s*(?:[A-Za-z]+:target=)?(http://(?:127\.0\.0\.1|localhost):(\d{1,5})(?:/[^\s]*)?\.mediagit)\s*$'
function Get-LoopbackTargets {
  $t = @()
  foreach ($l in (cmdkey /list 2>$null)) {
    if ($l -match $kcPattern -and [int]$Matches[2] -ge 49152) { $t += $Matches[1] }
  }
  return $t
}
# Delete, then re-list to prove it is gone (cmdkey's exit code is not trusted).
function Remove-Targets([string[]]$targets) {
  $ok = $true
  foreach ($t in $targets) {
    if ($t -notmatch '^http://(127\.0\.0\.1|localhost):\d+(/[^\s]*)?\.mediagit$') { continue }
    cmdkey /delete:$t 2>&1 | Out-Null
  }
  $left = Get-LoopbackTargets
  foreach ($t in $targets) { if ($left -contains $t) { Say "WARNING: keychain entry still present after delete: $t"; $ok = $false } }
  return $ok
}

function Clear-LedgerLeftovers {
  if (-not (Test-Path $ledger)) { return $true }
  $old = @(Get-Content $ledger | Where-Object { $_ } | Select-Object -Unique)
  $present = @(Get-LoopbackTargets | Where-Object { $old -contains $_ })
  if ($present.Count -gt 0) { Say ("removing {0} keychain entr(ies) a crashed earlier run left (from the ledger)" -f $present.Count) }
  $ok = Remove-Targets $present
  if ($ok) { Remove-Item -LiteralPath $ledger -Force -ErrorAction SilentlyContinue }
  return $ok
}

function Clear-NewKeychain {
  $new = @(Get-LoopbackTargets | Where-Object { $kcBefore -notcontains $_ })
  if ($new.Count -eq 0) { return $true }
  Add-Content -Path $ledger -Value $new                      # recorded BEFORE the delete
  Say ("removing {0} keychain entr(ies) created during this run" -f $new.Count)
  $ok = Remove-Targets $new
  if ($ok) { Remove-Item -LiteralPath $ledger -Force -ErrorAction SilentlyContinue }
  return $ok
}

# ---- pre-run ---------------------------------------------------------------
$busy = Get-Busy
if ($busy.Count -gt 0) {
  Say "FATAL: refusing to start, a qa-live Playwright run is already active:"
  $busy | ForEach-Object { Say "  $_" }
  exit 70
}
Say ("pre-existing mediagit* processes (left alone): " + $snap.Count)
foreach ($b in @("mediagit.exe", "mediagit-server.exe", "mediagit-desktop.exe")) {
  if (-not (Test-Path (Join-Path $repo "target\release\$b"))) { Say "FATAL: missing target\release\$b (build release first, see qa-live README)"; exit 72 }
}
if (-not (Test-Path (Join-Path $ui "dist\index.html"))) { Say "FATAL: ui\dist missing (pnpm --dir ui build)"; exit 72 }
Say ("HEAD = " + (git -C $repo rev-parse --short HEAD))

# Dot-sourced, never executed: it sets cloud credentials into this process only.
. (Join-Path $PSScriptRoot "campaign_env.ps1")

if (-not $SkipCleanup) {
  Say "cleanup (pre-run)"
  $a = Clear-Work
  $b = Clear-LedgerLeftovers
  if (-not ($a -and $b)) { Say "FATAL: pre-run cleanup incomplete"; exit 3 }
}
$kcBefore = @(Get-LoopbackTargets)
New-Item -ItemType Directory -Path $runDir -Force | Out-Null

# S-13 needs the Silo client; GA's native Silo keeps it in <profile>\silo. Unset and absent = S-13 records SKIP.
if (-not $env:MG_QA_MCLI) {
  $mcli = Join-Path $env:USERPROFILE "silo\mcli.exe"
  if (Test-Path $mcli) { $env:MG_QA_MCLI = $mcli; Say "MG_QA_MCLI = $mcli" } else { Say "MG_QA_MCLI unset and $mcli not found: S-13 will SKIP" }
}

$env:MG_QA_ROOT = $qaRoot
$env:MG_QA_DAY  = $RunId
$env:QALIVE_IDS = ($ids -join ',')
$log    = Join-Path $runDir "playwright.log"
$pwJson = Join-Path $runDir "playwright.json"
$env:PLAYWRIGHT_JSON_OUTPUT_NAME = $pwJson

# ---- run -------------------------------------------------------------------
$pwExit = -1
$verdict = 1
$cleanOk = $true
$push = Get-Location
try {
  # node directly (not pnpm.cmd): cmd.exe would parse | ^ [ in the -g pattern.
  $cli = Join-Path $ui "node_modules\@playwright\test\cli.js"
  $pwArgs = @($cli, "test", "-c", "playwright.qa.config.ts", "--reporter=list,json")
  if ($ids.Count -gt 0) {
    # an id matches only as a whole token (not inside a longer id, a number like 1,000 or a file name)
    $pwArgs += @("-g", ('(?<![\w.,/-])(' + ($ids -join '|') + ')(?![\w-])'))
  }
  Say ("playwright: node " + ($pwArgs -join " "))
  Set-Location $ui
  $sw = [Diagnostics.Stopwatch]::StartNew()
  & node @pwArgs 2>&1 | Tee-Object -FilePath $log | Out-Host
  $pwExit = $LASTEXITCODE
  $sw.Stop()
  Set-Location $push
  Say ("playwright exit $pwExit after {0:hh\:mm\:ss}" -f $sw.Elapsed)

  $global:LASTEXITCODE = 1                                    # a stale 0 must not pass a verdict that never ran
  & node (Join-Path $ui "tests\e2e\qa-live\summarize.mjs") $runDir $pwJson
  $verdict = $LASTEXITCODE
  $gates = Join-Path $runDir "gates.tsv"
  if (Test-Path $gates) { Say "gates.tsv:"; Get-Content $gates | ForEach-Object { Write-Host "  $_" } }
  else { Say "FATAL: no gates.tsv written"; $verdict = 1 }
}
finally {
  Set-Location $push
  if ($SkipCleanup) { Say "-SkipCleanup: leaving work, processes and keychain as they are" }
  else {
    Say "cleanup (post-run)"
    Stop-OurProcesses
    $a = Clear-Work
    $b = Clear-NewKeychain
    $cleanOk = ($a -and $b)
  }
}

if ($CleanCloud) {
  $running = @(Get-Process -Name $procNames -ErrorAction SilentlyContinue)
  if ($running.Count -gt 0) {
    Say ("-CleanCloud NOT run: mediagit process(es) still running that this run does not own ({0}); clean_after_run_cloud.ps1 refuses while any run. Stop them and run it by hand." -f (($running | ForEach-Object { "{0} pid={1}" -f $_.ProcessName, $_.Id }) -join ", "))
    $cleanOk = $false
  } else {
    Say "cloud cleanup (clean_after_run_cloud.ps1)"
    $global:LASTEXITCODE = 0
    & (Join-Path $PSScriptRoot "clean_after_run_cloud.ps1")
    if ($LASTEXITCODE -ne 0) { Say "cloud cleanup FAILED exit $LASTEXITCODE"; $cleanOk = $false }
  }
}

$skips = 0
$gatesFile = Join-Path $runDir "gates.tsv"
if (Test-Path $gatesFile) { $skips = @(Import-Csv $gatesFile -Delimiter "`t" | Where-Object { $_.result -eq "SKIP" }).Count }
$code = 0
if ($verdict -ne 0) { $code = 1 }
elseif ($pwExit -ne 0) { $code = 2 }
elseif (-not $cleanOk) { $code = 3 }
Say "QALIVE_DONE exit=$code verdict=$verdict playwright=$pwExit skipped=$skips cleanup=$cleanOk run=$runDir"
exit $code
