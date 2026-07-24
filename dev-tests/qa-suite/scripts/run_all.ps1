# Orchestrator: runs qa-suite phase scripts in order under one shared run id.
# Each phase script is a standalone .ps1 (00_archive.ps1, 01_preflight.ps1, 02_*.ps1, ...)
# invoked as its own process so its `exit N` only ends that phase, not this orchestrator.
#
# Phase tokens resolve to scripts\<token>*.ps1 (sorted by name), so a single token can match
# several scripts - e.g. "03" matching four persona scripts (03a_*.ps1 .. 03d_*.ps1) needs no
# special-casing, it falls out of the same glob-and-run-each-match loop as every other phase.
#
# Usage:
#   run_all.ps1                                   # run every phase 00..09
#   run_all.ps1 -Phases 01,03                     # run only these phase tokens
#   run_all.ps1 -ContinueOnFail                   # keep going past a failed phase
param(
  # Left unset so an explicit -Phases can be told apart from a default full run: the
  # SCALE tier auto-adds phase 10 (before 09/report) ONLY for a default full run.
  [string[]]$Phases,
  [switch]$ContinueOnFail
)

if (-not $env:MG_QA_RUN_ID) {
  $env:MG_QA_RUN_ID = Get-Date -Format "yyyyMMdd-HHmmss"
}

. (Join-Path $PSScriptRoot "lib\common.ps1")

if (-not $Phases -or $Phases.Count -eq 0) {
  # Default full run. Under SCALE, phase 10 runs after 08 but before 09 so the
  # report aggregates the scale gates.
  $Phases = if ($QA.Tier -eq "SCALE") {
    @("00", "01", "02", "03", "04", "05", "06", "07", "08", "10", "09")
  } else {
    @("00", "01", "02", "03", "04", "05", "06", "07", "08", "09")
  }
}

$Phase = "run_all"
Write-QaLog $Phase ("run id = {0}; phases = {1}" -f $QA.RunId, ($Phases -join ","))

$summaryTsv = Join-Path $QA.Logs "run_all-summary.tsv"
$summaryHeader = @("phase", "script", "status", "exit", "sec")
$results = @()
$anyFail = $false

$scriptsDir = Join-Path $QA.Root "scripts"
foreach ($tok in $Phases) {
  $found = Get-ChildItem -Path $scriptsDir -Filter "$tok*.ps1" -File -EA SilentlyContinue |
    Where-Object { $_.Name -ne "run_all.ps1" } | Sort-Object Name

  if (-not $found -or $found.Count -eq 0) {
    Write-QaLog $Phase "phase $tok : no script found, SKIP"
    Write-QaRow $summaryTsv $summaryHeader @($tok, "", "SKIP", "", "")
    $results += [pscustomobject]@{ Phase = $tok; Script = ""; Status = "SKIP"; Exit = ""; Sec = "" }
    continue
  }

  foreach ($script in $found) {
    if ($anyFail -and -not $ContinueOnFail) {
      Write-QaLog $Phase ("phase {0} : {1} NOT-RUN (earlier failure, -ContinueOnFail not set)" -f $tok, $script.Name)
      Write-QaRow $summaryTsv $summaryHeader @($tok, $script.Name, "NOT-RUN", "", "")
      $results += [pscustomobject]@{ Phase = $tok; Script = $script.Name; Status = "NOT-RUN"; Exit = ""; Sec = "" }
      continue
    }

    Write-QaLog $Phase ("phase {0} : running {1}" -f $tok, $script.Name)
    $sw = [Diagnostics.Stopwatch]::StartNew()
    & powershell -NoProfile -File $script.FullName
    $code = $LASTEXITCODE
    $sw.Stop()
    $status = if ($code -eq 0) { "PASS" } else { "FAIL" }
    if ($status -eq "FAIL") { $anyFail = $true }

    Write-QaLog $Phase ("phase {0} : {1} {2} (exit={3} sec={4:n1})" -f $tok, $script.Name, $status, $code, $sw.Elapsed.TotalSeconds)
    Write-QaRow $summaryTsv $summaryHeader @($tok, $script.Name, $status, $code, [math]::Round($sw.Elapsed.TotalSeconds, 1))
    $results += [pscustomobject]@{ Phase = $tok; Script = $script.Name; Status = $status; Exit = $code; Sec = [math]::Round($sw.Elapsed.TotalSeconds, 1) }
  }
}

Write-Host ""
Write-Host "===== qa-suite run $($QA.RunId) : phase summary ====="
$results | Format-Table -AutoSize | Out-String | Write-Host

if ($anyFail) { exit 1 }
exit 0
