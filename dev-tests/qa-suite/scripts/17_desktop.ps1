# Phase 17: desktop app gates (CLI --machine contract, engine, UI checks, status latency). ASCII-only, PS 5.1.
#
# Standalone by design - NOT in run_all's default phase list. It drives cargo and
# pnpm builds (minutes, CPU-heavy) and its latency gate must not share the machine
# with a campaign. Run alone:
#
#   powershell -NoProfile -File .\17_desktop.ps1
#
# WHAT IT GUARDS. The desktop app (crates/mediagit-desktop + ui/) never links the
# core crates: it runs `mediagit --machine <cmd>` per operation and parses one JSON
# envelope from stdout plus NDJSON progress from stderr. So the contract, the engine
# that parses it, the UI that renders it, and the per-call cost of a child process
# are what can break the app.
#
# GATES (rows in gates.tsv; True / False / SKIP):
#   desktop_exe_machine      $QA.MG answers `--machine version` with contract 1 (proves the
#                            binary under test contains the contract before anything is timed)
#   desktop_cli_contract     cargo test -p mediagit-cli --test cli_machine_contract_test
#   desktop_engine           cargo build -p mediagit-cli; cargo test -p mediagit-desktop
#                            (engine unit tests + integration tests against the real CLI)
#   desktop_ui_typecheck     pnpm --dir ui typecheck
#   desktop_ui_unit          pnpm --dir ui test
#   desktop_ui_e2e_mocked    pnpm --dir ui e2e   (Playwright vs Vite, mocked IPC, light+dark)
#   desktop_spawn_overhead   p95 wall time of `mediagit --machine version` <= MG_QA_DESKTOP_SPAWN_P95_MS:
#                            the per-operation cost the child-process engine adds (plan gate G1)
#   desktop_status_latency   p95 wall time of `mediagit -C repo --machine status` on a committed
#                            repo of MG_QA_FILECOUNT files vs MG_QA_DESKTOP_STATUS_P95_MS. Over the
#                            ceiling is WARN, not False: status cost is the CLI's own (issue #94,
#                            ~50 us/file), not the desktop engine's, and the UI runs it async.
#                            A bad envelope or failed setup is still False.
# A missing pnpm or node_modules makes the UI gates SKIP with the reason, never PASS.
# A binary without --machine makes desktop_status_latency SKIP (nothing to time).
#
# KNOBS (all optional):
#   MG_QA_DESKTOP_SPAWN_P95_MS    process-start ceiling, default 50
#   MG_QA_DESKTOP_STATUS_P95_MS   status latency WARN threshold, default 300
#   MG_QA_DESKTOP_RUNS            timed status runs, default 20
#   MG_QA_FILECOUNT               files in the latency repo, default 10000 (shared knob)
#   MG_QA_DESKTOP_LATENCY_ONLY=1  run only desktop_exe_machine + the two timing gates (no cargo,
#                                 no pnpm): for timing a release binary on a memory-tight machine.
#                                 The skipped gates are recorded SKIP with that reason.
# RED-PROOF: MG_QA_DESKTOP_SPAWN_P95_MS=1 must turn desktop_spawn_overhead False;
#   MG_QA_DESKTOP_STATUS_P95_MS=1 must turn desktop_status_latency WARN.
# The environment pins MEDIAGIT_CDC_SEED so chunk boundaries are reproducible.

. (Join-Path $PSScriptRoot "lib\common.ps1")

$Phase = "17_desktop"
$p95CeilMs = [double](_Env "MG_QA_DESKTOP_STATUS_P95_MS" "300")
$spawnCeilMs = [double](_Env "MG_QA_DESKTOP_SPAWN_P95_MS" "50")
$runs = [int](_Env "MG_QA_DESKTOP_RUNS" "20")
$latencyOnly = (_Env "MG_QA_DESKTOP_LATENCY_ONLY" "0") -eq "1"
if (-not $env:MEDIAGIT_CDC_SEED) { $env:MEDIAGIT_CDC_SEED = "20260716" }

$root = $QA.RepoRoot
$ui = Join-Path $root "ui"
$buildLog = Join-Path $QA.Logs "17_desktop_build.log"

# Run a command in the repo root, append all output to the build log, return the exit code.
function Invoke-DesktopStep([string]$Label, [string]$Exe, [string[]]$StepArgs) {
  Write-QaLog $Phase ("step {0}: {1} {2}" -f $Label, $Exe, ($StepArgs -join " "))
  Add-QaContentRetry $buildLog ("===== {0}: {1} {2}" -f $Label, $Exe, ($StepArgs -join " "))
  Push-Location $root
  try {
    $out = & $Exe @StepArgs 2>&1 | ForEach-Object { "$_" }
    $code = $LASTEXITCODE
  } finally {
    Pop-Location
  }
  Add-QaContentRetry $buildLog (($out | Select-Object -Last 200) -join "`r`n")
  return $code
}

Write-QaLog $Phase "=== 17_desktop start (p95 ceiling $p95CeilMs ms, $runs runs, $($QA.FileCount) files) ==="

# --- desktop_exe_machine --------------------------------------------------------
$machineOk = $false
$detail = ""
if (-not (Test-Path $QA.MG)) {
  $detail = "binary missing: $($QA.MG)"
} else {
  $raw = & $QA.MG --machine version 2>$null
  try {
    $env1 = ($raw | Out-String).Trim() | ConvertFrom-Json
    $machineOk = ($env1.v -eq 1) -and $env1.ok -and ($env1.data.contract -eq 1)
    $detail = "version=$($env1.data.version) contract=$($env1.data.contract)"
  } catch {
    $detail = "not a machine envelope: " + (($raw | Out-String).Trim() -replace "\s+", " ")
  }
}
Write-QaGate $Phase "desktop_exe_machine" $machineOk $detail

# --- cargo gates ---------------------------------------------------------------
$buildGates = @("desktop_cli_contract", "desktop_engine", "desktop_ui_typecheck", "desktop_ui_unit", "desktop_ui_e2e_mocked")
if ($latencyOnly) {
  foreach ($g in $buildGates) { Write-QaGate $Phase $g "SKIP" "not run (MG_QA_DESKTOP_LATENCY_ONLY=1)" }
} else {
$c = Invoke-DesktopStep "cli_contract" "cargo" @("test", "-p", "mediagit-cli", "--test", "cli_machine_contract_test")
Write-QaGate $Phase "desktop_cli_contract" ($c -eq 0) "exit=$c (log: 17_desktop_build.log)"

$b = Invoke-DesktopStep "cli_build" "cargo" @("build", "-p", "mediagit-cli")
$t = if ($b -eq 0) { Invoke-DesktopStep "engine" "cargo" @("test", "-p", "mediagit-desktop") } else { $b }
Write-QaGate $Phase "desktop_engine" (($b -eq 0) -and ($t -eq 0)) "build=$b test=$t"

# --- UI gates ------------------------------------------------------------------
$pnpm = Get-Command pnpm -EA SilentlyContinue
$uiReason = if (-not $pnpm) { "pnpm not on PATH" }
            elseif (-not (Test-Path (Join-Path $ui "node_modules"))) { "ui/node_modules missing (pnpm --dir ui install)" }
            else { "" }
foreach ($g in @(@("desktop_ui_typecheck", "typecheck"), @("desktop_ui_unit", "test"), @("desktop_ui_e2e_mocked", "e2e"))) {
  if ($uiReason) {
    Write-QaGate $Phase $g[0] "SKIP" $uiReason
    continue
  }
  $u = Invoke-DesktopStep $g[1] "pnpm" @("--dir", "ui", $g[1])
  Write-QaGate $Phase $g[0] ($u -eq 0) "exit=$u"
}
}

# --- desktop_spawn_overhead (G1) -----------------------------------------------
if (-not $machineOk) {
  Write-QaGate $Phase "desktop_spawn_overhead" "SKIP" "binary has no --machine contract (see desktop_exe_machine)"
} else {
  & $QA.MG --machine version *> $null   # warm; not timed
  $sp = @()
  for ($i = 0; $i -lt $runs; $i++) {
    $sw = [System.Diagnostics.Stopwatch]::StartNew()
    & $QA.MG --machine version *> $null
    $sw.Stop()
    $sp += $sw.Elapsed.TotalMilliseconds
  }
  $sps = $sp | Sort-Object
  $spP95 = $sps[[math]::Min($sps.Count - 1, [math]::Ceiling(0.95 * $sps.Count) - 1)]
  Write-QaGate $Phase "desktop_spawn_overhead" ($spP95 -le $spawnCeilMs) ("runs={0} p50={1:N0}ms p95={2:N0}ms ceiling={3}ms" -f `
      $runs, $sps[[math]::Floor($sps.Count / 2)], $spP95, $spawnCeilMs)
}

# --- desktop_status_latency (informational, issue #94) -------------------------
if (-not $machineOk) {
  Write-QaGate $Phase "desktop_status_latency" "SKIP" "binary has no --machine contract (see desktop_exe_machine)"
} else {
  $repo = New-SandboxRepo "desktop-latency" $Phase
  $dirs = 100
  $perDir = [math]::Ceiling($QA.FileCount / $dirs)
  $n = 0
  for ($d = 0; $d -lt $dirs -and $n -lt $QA.FileCount; $d++) {
    $dir = Join-Path $repo ("assets\d{0:D3}" -f $d)
    New-Item -ItemType Directory -Path $dir -Force | Out-Null
    for ($f = 0; $f -lt $perDir -and $n -lt $QA.FileCount; $f++) {
      [System.IO.File]::WriteAllText((Join-Path $dir ("f{0:D5}.txt" -f $n)), "asset $n")
      $n++
    }
  }
  $add = Invoke-MG $repo @("add", ".") $Phase
  $com = Invoke-MG $repo @("commit", "-m", "latency corpus") $Phase
  # One untracked + one modified file so status walks the dirty path too.
  [System.IO.File]::WriteAllText((Join-Path $repo "assets\d000\f00000.txt"), "changed")
  [System.IO.File]::WriteAllText((Join-Path $repo "untracked.txt"), "new")

  $ms = @()
  $bad = 0
  & $QA.MG -C $repo --machine status *> $null   # warm the file cache; not timed
  for ($i = 0; $i -lt $runs; $i++) {
    $sw = [System.Diagnostics.Stopwatch]::StartNew()
    $out = & $QA.MG -C $repo --machine status 2>$null
    $sw.Stop()
    $ms += $sw.Elapsed.TotalMilliseconds
    try { if (-not ((($out | Out-String).Trim() | ConvertFrom-Json).ok)) { $bad++ } } catch { $bad++ }
  }
  $sorted = $ms | Sort-Object
  $p95 = $sorted[[math]::Min($sorted.Count - 1, [math]::Ceiling(0.95 * $sorted.Count) - 1)]
  $p50 = $sorted[[math]::Floor($sorted.Count / 2)]
  $sound = ($bad -eq 0) -and ($n -eq $QA.FileCount) -and ($add.Exit -eq 0) -and ($com.Exit -eq 0)
  $pass = if (-not $sound) { $false } elseif ($p95 -le $p95CeilMs) { $true } else { "WARN" }
  Write-QaGate $Phase "desktop_status_latency" $pass ("files={0} runs={1} p50={2:N0}ms p95={3:N0}ms ceiling={4}ms bad_envelopes={5} add_exit={6} commit_exit={7}" -f `
      $n, $runs, $p50, $p95, $p95CeilMs, $bad, $add.Exit, $com.Exit)
}

Exit-QaPhase $Phase
