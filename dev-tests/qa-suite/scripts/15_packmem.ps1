# Phase 15: pack-upload memory profile (X2 ratchet). ASCII-only, PS 5.1.
#
# Standalone by design - NOT in run_all's default phase list, for the same
# reason as 11_memprofile: peak-RSS sampling reads `Get-Process mediagit`
# process-wide, so any concurrent campaign pollutes every number here. Run it
# on a quiet machine, by itself:
#
#   powershell -NoProfile -File .\15_packmem.ps1
#
# WHAT IT GUARDS. X2 (v0.4.0) changed pack upload from "read the whole 64 MiB
# pack into RAM, then upload" to "range-read one MPU part at a time". The claim
# is that peak client residency is now one PART per in-flight pack rather than
# one PACK, which is what makes MEDIAGIT_PACK_UPLOAD_CONCURRENCY safe to raise
# above its current 8 against a limit of 64. Nothing in the campaign measures
# memory, so without this the claim is unfalsifiable -- and it was already
# wrong once: the first version of X2 still copied each part a second time via
# `.body(part_data.clone())` on a `Vec<u8>`, which showed up as 31.5 MB per
# extra in-flight pack against a 16 MiB part size. Almost exactly 2x the part.
# Nobody would have noticed without a number.
#
# HOW THE COUNTERFACTUAL IS MEASURED, NOT ASSUMED. Arm 3 raises
# MEDIAGIT_MPU_THRESHOLD_BYTES above the pack size, which sends the SHIPPING
# binary down the single-PUT fallback. That fallback still reads whole packs --
# deliberately, it has no part granularity - so it reproduces the pre-X2 memory
# shape with no revert, no rebuild and no arithmetic. It is also a live check
# that the two are still different code paths.
#
# MUST run against a backend that implements presigned MPU. Only `s3` and
# `minio` do; `gcs`, `azure`, `b2_spaces` and `local` inherit the trait default
# that bails, so against those the packs never take the MPU path and every
# number would describe the fallback. The per-arm mpuPacks count is the
# anti-vacuity check for exactly that: a 501 degrades silently to single PUT and
# the push still succeeds, so "the upload worked" proves nothing about which
# path ran.

. (Join-Path $PSScriptRoot "lib\common.ps1")
. (Join-Path $PSScriptRoot "lib\remote.ps1")

$Phase = "15_packmem"
$payloadMB = [int](_Env "MG_PACKMEM_MB" "768")

# Peak working set may grow by at most this much per extra in-flight pack.
#
# Sized from the part size, not from the observed number: packs are 64 MiB and
# `mpu_part_size_minio`/`_s3` floor the part at 16 MiB, so one resident part is
# ~16 MB and the ceiling is 1.5x that. It sits below BOTH known regressions -
# 64 MB/pack pre-X2 and 31.5 MB/pack with the Vec copy - so it fails if either
# comes back, and above one part with room for allocator slack so it does not
# fail on noise.
$slopeCeilMB = [double](_Env "MG_PACKMEM_SLOPE_CEIL_MB" "24")

# label, pack-upload concurrency, MPU threshold bytes ($null = leave default, MPU on)
$arms = @(
  @{ Label = "mpu-c1"; Conc = 1; Mpu = $null },
  @{ Label = "mpu-c8"; Conc = 8; Mpu = $null },
  @{ Label = "nompu-c8"; Conc = 8; Mpu = "999999999" }
)

$OUT = Join-Path $QA.Logs "packmem.tsv"
$HDR = @("arm", "concurrency", "mpu", "payloadMB", "clientPeakWsMB", "clientPrivMB", "serverPrivMB", "mpuPacks", "sec", "exit")

function New-PackmemBlob([string]$Path, [int]$SizeMB, [int]$Seed) {
  $dir = Split-Path $Path -Parent
  if (-not (Test-Path $dir)) { New-Item -ItemType Directory -Path $dir -Force | Out-Null }
  $rng = New-Object System.Random($Seed)
  $buf = New-Object byte[] (1MB)
  $fs = [System.IO.File]::Open($Path, [System.IO.FileMode]::Create)
  try { for ($i = 0; $i -lt $SizeMB; $i++) { $rng.NextBytes($buf); $fs.Write($buf, 0, $buf.Length) } }
  finally { $fs.Close() }
}

Write-QaLog $Phase ("=== 15_packmem start: payload={0}MB arms={1} backend=minio slopeCeil={2}MB ===" -f `
    $payloadMB, (($arms | ForEach-Object { $_.Label }) -join ","), $slopeCeilMB)

$rows = @{}
$seed = 55000
foreach ($arm in $arms) {
  $seed++
  $srv = $null
  try {
    # Distinct seed per arm so each push uploads real bytes rather than
    # deduping against the previous arm and measuring nothing.
    $repo = New-SandboxRepo ("packmem-" + $arm.Label) $Phase
    New-PackmemBlob (Join-Path $repo "big.bin") $payloadMB $seed
    $a = Invoke-MG $repo @("add", "big.bin") $Phase -TimeoutSec 3600
    $ci = Invoke-MG $repo @("commit", "-m", ("packmem " + $arm.Label)) $Phase -TimeoutSec 3600
    if ($a.Exit -ne 0 -or $ci.Exit -ne 0) { throw "setup failed (add=$($a.Exit) commit=$($ci.Exit))" }

    $srv = Start-QaServer -Backend "minio" -Phase $Phase
    Invoke-MG $repo @("remote", "add", "origin", $srv.Url) $Phase | Out-Null

    $env:MEDIAGIT_PACK_UPLOAD_CONCURRENCY = "$($arm.Conc)"
    if ($arm.Mpu) { $env:MEDIAGIT_MPU_THRESHOLD_BYTES = $arm.Mpu }
    # Scoped to one module on purpose: full debug on a 768MB push would dwarf
    # the signal and perturb the thing being measured.
    $env:MEDIAGIT_LOG = "mediagit_protocol::pack_builder=debug"

    $sw = [Diagnostics.Stopwatch]::StartNew()
    $m = Measure-PeakRSS -Phase $Phase -Label ("push-" + $arm.Label) -Action {
      Invoke-MG $repo @("push", "origin") $Phase -TimeoutSec 3600
    }
    $sw.Stop()

    Remove-Item Env:\MEDIAGIT_PACK_UPLOAD_CONCURRENCY -ErrorAction SilentlyContinue
    Remove-Item Env:\MEDIAGIT_MPU_THRESHOLD_BYTES -ErrorAction SilentlyContinue
    Remove-Item Env:\MEDIAGIT_LOG -ErrorAction SilentlyContinue

    $r = $m.Result
    $mpu = ([regex]::Matches("" + $r.Out, "Pack uploaded via MPU")).Count

    Write-QaRow $OUT $HDR @($arm.Label, $arm.Conc, $(if ($arm.Mpu) { "off" } else { "on" }), $payloadMB,
      $m.ClientPeakMB, $m.ClientPrivatePeakMB, $m.ServerPrivatePeakMB, $mpu,
      [math]::Round($sw.Elapsed.TotalSeconds, 1), $r.Exit)
    Write-QaLog $Phase ("{0,-9} conc={1} exit={2} clientWS={3}MB clientPriv={4}MB serverPriv={5}MB mpuPacks={6} {7}s" -f `
        $arm.Label, $arm.Conc, $r.Exit, $m.ClientPeakMB, $m.ClientPrivatePeakMB, $m.ServerPrivatePeakMB, $mpu,
      [math]::Round($sw.Elapsed.TotalSeconds, 1))

    $rows[$arm.Label] = @{ Priv = [double]$m.ClientPrivatePeakMB; Ws = [double]$m.ClientPeakMB; Mpu = $mpu; Exit = $r.Exit }
  }
  catch {
    if ("$_" -match "^SKIP:") { Write-QaLog $Phase "$($arm.Label) SKIP: $_" }
    else { Write-QaLog $Phase "$($arm.Label) ERROR: $_" }
    $rows[$arm.Label] = @{ Priv = -1; Ws = -1; Mpu = 0; Exit = -1 }
  }
  finally {
    if ($srv) { Stop-QaServer $srv }
  }
}

$c1 = $rows["mpu-c1"]
$c8 = $rows["mpu-c8"]
$nm = $rows["nompu-c8"]

# ---- Gate 1: every arm succeeded. A failed arm measures nothing. ------------
$ranOk = ($c1.Exit -eq 0) -and ($c8.Exit -eq 0) -and ($nm.Exit -eq 0)
Write-QaGate $Phase "packmem-arms-succeed" $ranOk `
("exits mpu-c1={0} mpu-c8={1} nompu-c8={2}" -f $c1.Exit, $c8.Exit, $nm.Exit)

# ---- Gate 2: anti-vacuity. Each arm took the path it claims to measure. -----
$pathOk = ($c1.Mpu -gt 0) -and ($c8.Mpu -gt 0) -and ($nm.Mpu -eq 0)
Write-QaGate $Phase "packmem-arms-took-their-paths" $pathOk `
("mpuPacks mpu-c1={0} mpu-c8={1} nompu-c8={2} - want >0 >0 =0; a 0 in an MPU arm means 501/404 degraded to single PUT, a >0 in the nompu arm means the threshold did not take" -f `
    $c1.Mpu, $c8.Mpu, $nm.Mpu)

# ---- Gate 3: the payload is big enough for concurrency 8 to mean anything. --
#
# The slope divides by "extra in-flight packs", and with fewer packs than the
# concurrency there are no extra packs to be in flight. Caught by lowering the
# ceiling to prove the gate could fail and shrinking the payload in the same
# run: 4 packs against concurrency 8 divided by 7 anyway and reported 6.3MB,
# passing a ceiling the real number is well above. A gate that silently
# weakens itself on smaller input is the failure mode this suite has recorded
# nine times, so the precondition is now checked rather than assumed.
$concMax = ($arms | ForEach-Object { $_.Conc } | Measure-Object -Maximum).Maximum
$saturated = $ranOk -and ($c8.Mpu -ge $concMax)
Write-QaGate $Phase "packmem-payload-saturates-concurrency" $saturated `
("{0} packs at concurrency {1} (need >= {1}; raise MG_PACKMEM_MB - packs are 64 MiB, so ~{2}MB minimum)" -f `
    $c8.Mpu, $concMax, ($concMax * 64))

# ---- Gate 4: the ratchet. Memory per extra in-flight pack stays near one part.
$extra = [math]::Max(1, [math]::Min($concMax, $c8.Mpu) - 1)
$slope = if ($ranOk) { [math]::Round(($c8.Ws - $c1.Ws) / $extra, 1) } else { -1 }
$slopeOk = $ranOk -and $saturated -and ($slope -ge 0) -and ($slope -le $slopeCeilMB)
Write-QaGate $Phase "packmem-slope-under-ceiling" $slopeOk `
("{0}MB per extra in-flight pack over {1} extra packs, ceiling {2}MB (one 16 MiB part; pre-X2 was ~64MB/pack, the Vec-clone regression was 31.5MB/pack)" -f `
    $slope, $extra, $slopeCeilMB)

Write-QaLog $Phase ("RESULT peakWS  mpu-c1={0}MB  mpu-c8={1}MB  nompu-c8={2}MB" -f $c1.Ws, $c8.Ws, $nm.Ws)
Write-QaLog $Phase ("RESULT privMB  mpu-c1={0}MB  mpu-c8={1}MB  nompu-c8={2}MB" -f $c1.Priv, $c8.Priv, $nm.Priv)
if ($ranOk -and $nm.Ws -gt 0 -and $c8.Ws -gt 0) {
  Write-QaLog $Phase ("RESULT X2 saving at conc=8: {0}MB peak WS ({1}x vs the whole-pack path)" -f `
    [math]::Round($nm.Ws - $c8.Ws, 1), [math]::Round($nm.Ws / $c8.Ws, 2))
}

Write-QaLog $Phase "=== 15_packmem done ==="
