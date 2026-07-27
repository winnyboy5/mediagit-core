# Phase 11: per-operation memory profile + bench-counter validation (ST5 tail).
#
# Standalone by design - NOT in run_all's default phase list. Peak-RSS sampling reads
# `Get-Process mediagit,mediagit-server` process-wide, so ANY concurrent campaign pollutes
# every number here. Run it on a quiet machine, by itself:
#
#   powershell -NoProfile -File .\11_memprofile.ps1
#
# Produces:
#   memprofile.tsv  - op x {client,server} x {peak working set, private bytes}
#   gates.tsv rows  - counters-non-zero (the real gate) + an RSS ceiling per op
#
# Why both WS and private: on a 4 GB payload S4 measured 3268.7 MB working set against
# 673.6 MB private. Working set counts file-backed/page-cache pages the OS can drop under
# pressure; private bytes are what the process actually owns. Only private growth is a
# leak. Gate on private, report WS.

. (Join-Path $PSScriptRoot "lib\common.ps1")
. (Join-Path $PSScriptRoot "lib\remote.ps1")   # Start-QaServer / Stop-QaServer live here

$Phase = "11_memprofile"
$OUT = Join-Path $QA.Logs "memprofile.tsv"
$HDR = @("op", "payloadMB", "clientPeakWsMB", "clientPrivMB", "serverPeakWsMB", "serverPrivMB", "sec", "exit")

# Private-bytes ceiling per op. Deliberately generous: this is a leak detector, not a
# budget. Anything that streams should sit far below its payload; a whole-file read shows
# up as private tracking payload size.
$PRIV_CEIL_MB = [int](_Env "MG_QA_PRIV_CEIL_MB" "1536")

$payloadMB = [int](_Env "MG_QA_MEMPROF_MB" "512")

$root = Join-Path $QA.Work "memprof"
$src = Join-Path $root "src"
$clone = Join-Path $root "clone"

function New-Blob([string]$Path, [int]$SizeMB, [int]$Seed) {
  $dir = Split-Path $Path -Parent
  if (-not (Test-Path $dir)) { New-Item -ItemType Directory -Path $dir -Force | Out-Null }
  $rng = New-Object System.Random($Seed)
  $buf = New-Object byte[] (1MB)
  $fs = [System.IO.File]::Open($Path, [System.IO.FileMode]::Create)
  try { for ($i = 0; $i -lt $SizeMB; $i++) { $rng.NextBytes($buf); $fs.Write($buf, 0, $buf.Length) } }
  finally { $fs.Close() }
}

# Run one op under the RSS sampler and record a row. Returns the measurement.
function Profile-Op([string]$Name, [string]$Repo, [string[]]$MgArgs, [int]$PayloadMB) {
  $sw = [Diagnostics.Stopwatch]::StartNew()
  $m = Measure-PeakRSS -Phase $Phase -Label $Name -Action {
    Invoke-MG $Repo $MgArgs $Phase -TimeoutSec 3600
  }
  $sw.Stop()
  $r = $m.Result
  Write-QaRow $OUT $HDR @($Name, $PayloadMB, $m.ClientPeakMB, $m.ClientPrivatePeakMB,
    $m.ServerPeakMB, $m.ServerPrivatePeakMB, [math]::Round($sw.Elapsed.TotalSeconds, 1), $r.Exit)
  Write-QaLog $Phase ("{0,-8} exit={1} clientWS={2}MB clientPriv={3}MB serverWS={4}MB serverPriv={5}MB {6}s" -f `
      $Name, $r.Exit, $m.ClientPeakMB, $m.ClientPrivatePeakMB, $m.ServerPeakMB, $m.ServerPrivatePeakMB,
    [math]::Round($sw.Elapsed.TotalSeconds, 1))
  return @{ M = $m; R = $r }
}

function Parse-BenchLines([string]$Text) {
  # [bench] goes to stderr; Invoke-MG's 2>&1 wraps it in PS NativeCommandError rendering.
  # Same slicing as 08_perf.ps1.
  $records = @()
  $chunks = $Text -split '\[bench\]'
  for ($ci = 1; $ci -lt $chunks.Count; $ci++) {
    $seg = ($chunks[$ci] -split "`r?`nAt |`r?`n\s*\+ ")[0] -replace "`r?`n", " "
    $h = @{}
    foreach ($m in [regex]::Matches($seg, '(\w+)=([^\s]+)')) { $h[$m.Groups[1].Value] = $m.Groups[2].Value }
    if ($h['op']) { $records += $h }
  }
  return $records
}

$srv = $null
try {
  if (Test-Path $root) { Remove-Item -Recurse -Force $root -ErrorAction SilentlyContinue }
  New-Item -ItemType Directory -Path $src -Force | Out-Null

  Write-QaLog $Phase "payload=${payloadMB}MB privCeil=${PRIV_CEIL_MB}MB"

  # Mixed corpus: one big incompressible blob (streaming path) + many small files (count
  # pressure). Both matter - a whole-file read shows on the blob, per-object overhead on
  # the corpus.
  New-Blob (Join-Path $src "big.bin") $payloadMB 90210
  for ($i = 0; $i -lt 500; $i++) {
    Set-Content (Join-Path $src ("f{0:d4}.txt" -f $i)) ("line " * 200) -Encoding Ascii
  }

  Invoke-MG $null @("init", $src) $Phase | Out-Null

  $env:MEDIAGIT_BENCH = "1"
  try {
    $add = Profile-Op "add"    $src @("add", ".") $payloadMB
    $ci = Profile-Op "commit" $src @("commit", "-m", "memprofile") $payloadMB

    $srv = Start-QaServer -Backend "local" -Phase $Phase
    Invoke-MG $src @("remote", "add", "origin", $srv.Url) $Phase | Out-Null

    $push = Profile-Op "push"  $src @("push", "origin") $payloadMB
    $cl = Profile-Op "clone" $null @("clone", $srv.Url, $clone) $payloadMB

    # Give pull something to do, so it is not a no-op measurement.
    Set-Content (Join-Path $src "delta.txt") "second revision" -Encoding Ascii
    Invoke-MG $src @("add", "delta.txt") $Phase | Out-Null
    Invoke-MG $src @("commit", "-m", "r2") $Phase | Out-Null
    Invoke-MG $src @("push", "origin") $Phase | Out-Null

    $pull = Profile-Op "pull"  $clone @("pull") $payloadMB
    $gc = Profile-Op "gc"    $src @("gc") $payloadMB

    # ---- Gate 1: bench counters are live on the DEFAULT path ----
    # throughput_mbs silently read 0 on the default push/pull path until the pack-mode
    # [bench] wiring landed. A counter that is present but always zero is worse than a
    # missing one: it looks measured.
    $benchText = ($push.R.Out + "`n" + $cl.R.Out + "`n" + $pull.R.Out)
    $recs = @(Parse-BenchLines $benchText)
    $ops = @($recs | ForEach-Object { $_['op'] } | Sort-Object -Unique)
    $tps = @($recs | Where-Object { $_['throughput_mbs'] } |
      ForEach-Object { [double](($_['throughput_mbs'] -replace '[^\d.]', '')) })
    $nonZero = @($tps | Where-Object { $_ -gt 0 }).Count
    $counterOk = ($recs.Count -gt 0) -and ($nonZero -gt 0)
    Write-QaGate $Phase "bench-counters-live" $counterOk `
    ("records={0} ops={1} throughput_mbs values={2} nonZero={3}" -f `
        $recs.Count, ($ops -join ","), ($tps -join ","), $nonZero)

    # ---- Gate 2: no op leaks - private bytes stay under ceiling ----
    $profiles = @{ add = $add; commit = $ci; push = $push; clone = $cl; pull = $pull; gc = $gc }
    $over = @()
    $failed = @()
    foreach ($k in @("add", "commit", "push", "clone", "pull", "gc")) {
      $p = $profiles[$k]
      $priv = [double]$p.M.ClientPrivatePeakMB
      if ($priv -gt $PRIV_CEIL_MB) { $over += ("{0}={1}MB" -f $k, $priv) }
      if ($p.R.Exit -ne 0) { $failed += ("{0}=exit{1}" -f $k, $p.R.Exit) }
    }
    Write-QaGate $Phase "ops-succeed" ($failed.Count -eq 0) `
    $(if ($failed.Count) { "failed: " + ($failed -join " ") } else { "add,commit,push,clone,pull,gc all exit 0" })
    Write-QaGate $Phase "private-bytes-under-ceiling" ($over.Count -eq 0) `
    ("ceil={0}MB payload={1}MB {2}" -f $PRIV_CEIL_MB, $payloadMB,
      $(if ($over.Count) { "OVER: " + ($over -join " ") } else { "all ops within ceiling" }))
  }
  finally {
    Remove-Item Env:\MEDIAGIT_BENCH -ErrorAction SilentlyContinue
  }
}
finally {
  if ($srv) { Stop-QaServer $srv }
  Invoke-QaTeardown $Phase @("memprof")
}

Exit-QaPhase $Phase
