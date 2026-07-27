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
# After common.ps1 (which sets Continue) so it wins. The orchestrator decides the
# campaign verdict; an error swallowed HERE would mis-report every phase under it.
$ErrorActionPreference = "Stop"

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

# ---- per-script extra arguments -------------------------------------------------
# 08_perf gates against baselines\perf.tsv when it exists. The file is NOT created
# automatically: an operator promotes a known-good campaign's perf-bench.tsv into it
# (copy logs\<runid>\perf-bench.tsv -> baselines\perf.tsv), so the harness can never
# quietly re-baseline itself onto a regression it just measured.
$perfBaseline = Join-Path $QA.Root "baselines\perf.tsv"

# 01_preflight regenerates fixtures when the tier changed since the last run: fixture
# SIZES are tier-dependent (MG_QA_SCALE), so a STANDARD-sized corpus left over from a
# previous run would silently make a SCALE campaign test the wrong thing.
$tierMarker = Join-Path $QA.Work ".last-tier"
$lastTier = if (Test-Path $tierMarker) { ((Get-Content $tierMarker -Raw) + "").Trim() } else { "" }
$tierChanged = ($lastTier -ne $QA.Tier)
if ($tierChanged -and $lastTier) {
  Write-QaLog $Phase "tier changed '$lastTier' -> '$($QA.Tier)': 01_preflight will run with -Regen"
}

function Get-PhaseArgs([string]$ScriptName) {
  switch -Regex ($ScriptName) {
    '^01_preflight' { if ($tierChanged) { return @("-Regen") } else { return @() } }
    '^08_perf' {
      if (Test-Path $perfBaseline) { return @("-Baseline", $perfBaseline) }
      Write-QaLog $Phase "no perf baseline at $perfBaseline - 08_perf runs informational (WARN)"
      return @()
    }
    default { return @() }
  }
}

$summaryTsv = Join-Path $QA.Logs "run_all-summary.tsv"
$summaryHeader = @("phase", "script", "status", "exit", "sec")
$results = @()
$anyFail = $false

$scriptsDir = Join-Path $QA.Root "scripts"
foreach ($tok in $Phases) {
  $found = Get-ChildItem -Path $scriptsDir -Filter "$tok*.ps1" -File -EA SilentlyContinue |
    Where-Object { $_.Name -ne "run_all.ps1" } | Sort-Object Name

  if (-not $found -or $found.Count -eq 0) {
    # A token that matches no script is a typo in the invocation, not an absent
    # capability. Silently skipping it meant `-Phases 1,3` (instead of 01,03) ran
    # nothing and exited 0 - a green campaign that tested nothing at all.
    Write-QaLog $Phase "phase '$tok' : NO SCRIPT MATCHES scripts\$tok*.ps1 - bad phase token"
    Write-QaRow $summaryTsv $summaryHeader @($tok, "", "BAD-TOKEN", "", "")
    Write-Host ""
    Write-Host "ERROR: unknown phase token '$tok' - no scripts\$tok*.ps1 exists."
    Write-Host ("Available tokens: " + ((Get-ChildItem -Path $scriptsDir -Filter "*.ps1" -File |
      Where-Object { $_.Name -match '^\d' } | ForEach-Object { ($_.Name -split '_')[0] } |
      Sort-Object -Unique) -join ", "))
    exit 1
  }

  foreach ($script in $found) {
    if ($anyFail -and -not $ContinueOnFail) {
      Write-QaLog $Phase ("phase {0} : {1} NOT-RUN (earlier failure, -ContinueOnFail not set)" -f $tok, $script.Name)
      Write-QaRow $summaryTsv $summaryHeader @($tok, $script.Name, "NOT-RUN", "", "")
      $results += [pscustomobject]@{ Phase = $tok; Script = $script.Name; Status = "NOT-RUN"; Exit = ""; Sec = "" }
      continue
    }

    # @() guard: a function returning @("-Regen") unrolls the single-element array to a
    # bare string, and the empty case returns $null - splatting either one at a native
    # command throws. Re-wrapping makes both shapes a real array again.
    $extra = @(Get-PhaseArgs $script.Name)
    Write-QaLog $Phase ("phase {0} : running {1} {2}" -f $tok, $script.Name, ($extra -join " "))
    $sw = [Diagnostics.Stopwatch]::StartNew()
    & powershell -NoProfile -File $script.FullName @extra
    $code = $LASTEXITCODE
    $sw.Stop()
    # Exit 3 is Exit-QaPhase's "nothing verified": the phase failed nothing because it
    # checked nothing. It is a failure, and it is labelled so it is not read as a pass.
    $status = if ($code -eq 0) { "PASS" } elseif ($code -eq 3) { "NOTHING-VERIFIED" } else { "FAIL" }
    if ($status -ne "PASS") { $anyFail = $true }

    Write-QaLog $Phase ("phase {0} : {1} {2} (exit={3} sec={4:n1})" -f $tok, $script.Name, $status, $code, $sw.Elapsed.TotalSeconds)
    Write-QaRow $summaryTsv $summaryHeader @($tok, $script.Name, $status, $code, [math]::Round($sw.Elapsed.TotalSeconds, 1))
    $results += [pscustomobject]@{ Phase = $tok; Script = $script.Name; Status = $status; Exit = $code; Sec = [math]::Round($sw.Elapsed.TotalSeconds, 1) }
  }
}

Set-Content $tierMarker $QA.Tier -Encoding ASCII

Write-Host ""
Write-Host "===== qa-suite run $($QA.RunId) : phase summary ====="
$results | Format-Table -AutoSize | Out-String | Write-Host

if ($anyFail) { exit 1 }
exit 0
