# Phase 17 (kill-safety): transfers killed mid-flight must leave repos sound and retryable. ASCII-only, PS 5.1.
#
# Standalone by design - NOT in run_all's default phase list (80+ transfers; minutes).
# `-Phases 17` runs it together with 17_desktop.ps1. Run alone:
#
#   powershell -NoProfile -File .\17_desktop_killsafety.ps1
#
# WHY. The desktop app's Cancel button kills the `mediagit` child process (there is no
# in-band cancel). That is only safe if a transfer killed at ANY point leaves both repos
# consistent and the same operation, retried, completes with byte-exact results. The CLI
# already promises this for Ctrl-C (exit 130); this phase proves it for a hard kill.
#
# GATES (rows in gates.tsv; True / False / SKIP), one per operation:
#   desktop_kill_push    writer commits a fresh blob, push is killed at a random point,
#                        `fsck` on the writer is clean, the retried push exits 0
#   desktop_kill_fetch   reader fetch killed; reader fsck clean; retried fetch exits 0
#   desktop_kill_pull    reader pull killed; reader fsck clean; retried pull exits 0 and the
#                        reader tree is hash-identical to the writer tree
#   desktop_kill_clone   clone killed; retried clone into a fresh dir exits 0, fsck clean,
#                        tree hash-identical to the writer
# Every gate also reports how many kills landed while the process was still running
# (`landed`). landed=0 means every transfer finished before the kill: the gate then
# proved nothing and is recorded False, not True.
#
# KNOBS (all optional):
#   MG_QA_DESKTOP_KILLS          kills per operation, default 20 (plan gate G3)
#   MG_QA_DESKTOP_KILL_BACKEND   server backend: local (default) | minio | aws | azure | gcs
#   MG_QA_DESKTOP_KILL_MB        fresh payload per iteration in MB, default 24
#   MG_QA_DESKTOP_KILL_SEED      RNG seed for kill offsets, default 20261027
#   MG_QA_DESKTOP_KILL_OPS       comma list of ops to drill, default push,fetch,pull,clone.
#                                Unselected ops record SKIP "not selected (MG_QA_DESKTOP_KILL_OPS)".
# RED-PROOF: point $env:MG at a binary that corrupts on interrupt, or set
#   MG_QA_DESKTOP_KILL_INJECT=1 (test-only): after each kill, delete one random object file
#   under the reader's .mediagit/objects so fsck must go red.

. (Join-Path $PSScriptRoot "lib\common.ps1")
. (Join-Path $PSScriptRoot "lib\remote.ps1")

$Phase = "17_desktop_killsafety"
$kills = [int](_Env "MG_QA_DESKTOP_KILLS" "20")
$backend = (_Env "MG_QA_DESKTOP_KILL_BACKEND" "local").ToLower()
$payloadMB = [int](_Env "MG_QA_DESKTOP_KILL_MB" "24")
$rng = New-Object System.Random ([int](_Env "MG_QA_DESKTOP_KILL_SEED" "20261027"))
$inject = (_Env "MG_QA_DESKTOP_KILL_INJECT" "0") -eq "1"
$ops = (_Env "MG_QA_DESKTOP_KILL_OPS" "push,fetch,pull,clone") -split "," | ForEach-Object { $_.Trim().ToLower() } | Where-Object { $_ }
foreach ($o in @("push", "fetch", "pull", "clone")) {
  if ($ops -notcontains $o) { Write-QaGate $Phase "desktop_kill_$o" "SKIP" "not selected (MG_QA_DESKTOP_KILL_OPS)" }
}
if (-not $env:MEDIAGIT_CDC_SEED) { $env:MEDIAGIT_CDC_SEED = "20260716" }

# Deterministic incompressible-ish payload so every iteration has real bytes to move.
function New-KillPayload([string]$Path, [int]$SizeMB, [int]$Seed) {
  $dir = Split-Path $Path -Parent
  if (-not (Test-Path $dir)) { New-Item -ItemType Directory -Path $dir -Force | Out-Null }
  $r = New-Object System.Random $Seed
  $buf = New-Object byte[] (1MB)
  $fs = [System.IO.File]::Create($Path)
  try { for ($i = 0; $i -lt $SizeMB; $i++) { $r.NextBytes($buf); $fs.Write($buf, 0, $buf.Length) } } finally { $fs.Close() }
}

# Start `mediagit <args>` in $Cwd, kill it after $DelayMs. Returns $true if the kill landed
# on a live process (the transfer had not finished yet).
function Invoke-KilledMG([string]$Cwd, [string[]]$MgArgs, [int]$DelayMs) {
  $psi = New-Object System.Diagnostics.ProcessStartInfo
  $psi.FileName = $QA.MG
  $psi.Arguments = ($MgArgs | ForEach-Object { if ($_ -match "\s") { '"' + $_ + '"' } else { $_ } }) -join " "
  $psi.WorkingDirectory = $Cwd
  $psi.UseShellExecute = $false
  $psi.CreateNoWindow = $true
  $psi.RedirectStandardOutput = $true
  $psi.RedirectStandardError = $true
  $p = [System.Diagnostics.Process]::Start($psi)
  # Drain pipes asynchronously so a chatty child cannot block on a full pipe.
  $null = $p.StandardOutput.ReadToEndAsync()
  $null = $p.StandardError.ReadToEndAsync()
  $landed = -not $p.WaitForExit($DelayMs)
  if ($landed) {
    try { $p.Kill() } catch { $landed = $false }
    $null = $p.WaitForExit(30000)
  }
  return $landed
}

function Test-FsckClean([string]$Repo) {
  $f = Invoke-MG $Repo @("fsck") $Phase -TimeoutSec 1800
  return @{ Ok = (($f.Exit -eq 0) -and ($f.Out -notmatch "(?i)corrupt|missing|failed")); Exit = $f.Exit }
}

function Remove-RandomObject([string]$Repo) {
  $objs = @(Get-ChildItem (Join-Path $Repo ".mediagit\objects") -Recurse -File -EA SilentlyContinue)
  if ($objs.Count -gt 0) { Remove-Item -Force $objs[$rng.Next($objs.Count)].FullName }
}

# Measure one un-killed run so kill points spread over the real duration.
# A single cold run can be much slower than the steady state, so callers also
# shrink the window whenever a kill misses (Invoke-KilledMG returns $false):
# the op finished before that delay, so later draws use it as the new ceiling.
function Get-OpMs([scriptblock]$Op) {
  $sw = [System.Diagnostics.Stopwatch]::StartNew(); & $Op | Out-Null; $sw.Stop()
  return [math]::Max(200, [int]$sw.Elapsed.TotalMilliseconds)
}

Write-QaLog $Phase "=== $Phase start (backend=$backend kills=$kills payload=${payloadMB}MB inject=$inject) ==="

$srv = $null
try {
  try {
    $srv = Start-QaServer -Backend $backend -Phase $Phase
  } catch {
    $why = "$_"
    foreach ($g in @("desktop_kill_push", "desktop_kill_fetch", "desktop_kill_pull", "desktop_kill_clone")) {
      if ($why -match "^SKIP:") { Write-QaGate $Phase $g "SKIP" $why } else { Write-QaGate $Phase $g $false "server unavailable: $why" }
    }
    Exit-QaPhase $Phase
  }

  $writer = New-SandboxRepo "kill-writer" $Phase
  New-KillPayload (Join-Path $writer "seed.bin") $payloadMB 1
  Invoke-MG $writer @("add", ".") $Phase | Out-Null
  Invoke-MG $writer @("commit", "-m", "seed") $Phase | Out-Null
  Invoke-MG $writer @("remote", "add", "origin", $srv.Url) $Phase | Out-Null
  $seedPush = Invoke-MG $writer @("push", "origin") $Phase -TimeoutSec 1800

  $reader = Join-Path $QA.Work "kill-reader"
  if (Test-Path $reader) { Remove-Item -Recurse -Force $reader }
  $seedClone = Invoke-MG $null @("clone", $srv.Url, $reader) $Phase -TimeoutSec 1800
  if ($seedPush.Exit -ne 0 -or $seedClone.Exit -ne 0) {
    foreach ($g in @("desktop_kill_push", "desktop_kill_fetch", "desktop_kill_pull", "desktop_kill_clone")) {
      Write-QaGate $Phase $g $false "setup failed: seed push exit=$($seedPush.Exit) clone exit=$($seedClone.Exit)"
    }
    Exit-QaPhase $Phase
  }

  $iter = 0
  # Each writer commit gets a fresh blob, so every push/fetch/pull has new bytes to move.
  function Add-WriterCommit {
    $script:iter++
    New-KillPayload (Join-Path $writer ("data\it{0:D3}.bin" -f $script:iter)) $payloadMB (1000 + $script:iter)
    Invoke-MG $writer @("add", ".") $Phase | Out-Null
    Invoke-MG $writer @("commit", "-m", "iteration $script:iter") $Phase | Out-Null
  }

  # --- push ---
  if ($ops -contains "push") {
  Add-WriterCommit
  $pushMs = Get-OpMs { Invoke-MG $writer @("push", "origin") $Phase -TimeoutSec 1800 }
  $landed = 0; $failures = @()
  for ($k = 1; $k -le $kills; $k++) {
    Add-WriterCommit
    $delay = $rng.Next(20, $pushMs)
    if (Invoke-KilledMG $writer @("push", "origin") $delay) { $landed++ } else { $pushMs = [math]::Max(40, $delay) }
    $fs = Test-FsckClean $writer
    $retry = Invoke-MG $writer @("push", "origin") $Phase -TimeoutSec 1800
    if (-not $fs.Ok -or $retry.Exit -ne 0) { $failures += "k${k}:fsck=$($fs.Exit),retry=$($retry.Exit)" }
  }
  Write-QaGate $Phase "desktop_kill_push" (($failures.Count -eq 0) -and ($landed -gt 0)) `
    ("kills={0} landed={1} op_ms={2} failures={3}" -f $kills, $landed, $pushMs, ($failures -join ";"))
  }

  # --- fetch / pull ---
  foreach ($op in @("fetch", "pull")) {
    if ($ops -notcontains $op) { continue }
    Add-WriterCommit
    Invoke-MG $writer @("push", "origin") $Phase -TimeoutSec 1800 | Out-Null
    $opMs = Get-OpMs { Invoke-MG $reader @($op, "origin") $Phase -TimeoutSec 1800 }
    $landed = 0; $failures = @()
    for ($k = 1; $k -le $kills; $k++) {
      Add-WriterCommit
      Invoke-MG $writer @("push", "origin") $Phase -TimeoutSec 1800 | Out-Null
      $delay = $rng.Next(20, $opMs)
      if (Invoke-KilledMG $reader @($op, "origin") $delay) { $landed++ } else { $opMs = [math]::Max(40, $delay) }
      if ($inject) { Remove-RandomObject $reader }
      $fs = Test-FsckClean $reader
      $retry = Invoke-MG $reader @($op, "origin") $Phase -TimeoutSec 1800
      if (-not $fs.Ok -or $retry.Exit -ne 0) { $failures += "k${k}:fsck=$($fs.Exit),retry=$($retry.Exit)" }
    }
    $detail = "kills={0} landed={1} op_ms={2} failures={3}" -f $kills, $landed, $opMs, ($failures -join ";")
    $pass = ($failures.Count -eq 0) -and ($landed -gt 0)
    if ($op -eq "pull") {
      $diff = @(Compare-Object (Get-QaTreeHashes $writer) (Get-QaTreeHashes $reader)).Count
      $pass = $pass -and ($diff -eq 0)
      $detail += " tree_mismatches=$diff"
    }
    Write-QaGate $Phase "desktop_kill_$op" $pass $detail
  }

  # --- clone ---
  if ($ops -contains "clone") {
  $cloneMs = Get-OpMs {
    $d = Join-Path $QA.Work "kill-clone-probe"
    if (Test-Path $d) { Remove-Item -Recurse -Force $d }
    Invoke-MG $null @("clone", $srv.Url, $d) $Phase -TimeoutSec 1800
  }
  $landed = 0; $failures = @()
  $writerHashes = Get-QaTreeHashes $writer
  for ($k = 1; $k -le $kills; $k++) {
    $dead = Join-Path $QA.Work ("kill-clone-dead-{0:D2}" -f $k)
    $good = Join-Path $QA.Work ("kill-clone-good-{0:D2}" -f $k)
    foreach ($d in @($dead, $good)) { if (Test-Path $d) { Remove-Item -Recurse -Force $d } }
    $delay = $rng.Next(20, $cloneMs)
    if (Invoke-KilledMG $QA.Work @("clone", $srv.Url, $dead) $delay) { $landed++ } else { $cloneMs = [math]::Max(40, $delay) }
    $retry = Invoke-MG $null @("clone", $srv.Url, $good) $Phase -TimeoutSec 1800
    $fs = if ($retry.Exit -eq 0) { Test-FsckClean $good } else { @{ Ok = $false; Exit = -1 } }
    $same = ($retry.Exit -eq 0) -and (@(Compare-Object $writerHashes (Get-QaTreeHashes $good)).Count -eq 0)
    if (-not $fs.Ok -or -not $same) { $failures += "k${k}:retry=$($retry.Exit),fsck=$($fs.Exit),parity=$same" }
    foreach ($d in @($dead, $good)) { if (Test-Path $d) { Remove-Item -Recurse -Force $d -EA SilentlyContinue } }
  }
  Write-QaGate $Phase "desktop_kill_clone" (($failures.Count -eq 0) -and ($landed -gt 0)) `
    ("kills={0} landed={1} op_ms={2} failures={3}" -f $kills, $landed, $cloneMs, ($failures -join ";"))
  }
} finally {
  Stop-QaServer $srv
}

Exit-QaPhase $Phase
