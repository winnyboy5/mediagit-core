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
function Invoke-MG([string]$Repo, [string[]]$MgArgs, [string]$Phase = "misc", [int]$TimeoutSec = 600) {
  $sw = [Diagnostics.Stopwatch]::StartNew()
  $allArgs = if ($Repo) { @("-C", $Repo) + $MgArgs } else { $MgArgs }
  $out = & $QA.MG @allArgs 2>&1 | Out-String
  $code = $LASTEXITCODE
  $sw.Stop()
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
