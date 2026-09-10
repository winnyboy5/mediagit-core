# T1 Mathis-bound probe (docs/next-set/improvement-plan.md SS3) -- decide
# whether cloud throughput is limited by TCP loss or by the physical link.
#
#     BW ~= MSS / (RTT * sqrt(p))          (Mathis et al., the classic TCP
#                                            throughput/loss bound)
#
# If a measured transfer runs close to that bound, the bottleneck is TCP
# fighting loss and a QUIC/accelerator investigation (H-B) is worth the
# effort. If loss is ~0, or the transfer runs far below the bound anyway,
# TCP isn't the limiter -- the link itself is (H-A) -- and effort should go
# to dedup instead. Either answer is useful; this script only measures.
#
# WHY NOT ICMP FOR LOSS OR RTT. S3, GCS and Azure Blob all deprioritise or
# drop ICMP, so a ping-based number would be meaningless noise dressed up as
# a measurement (see lib\linkprobe.ps1's header for the longer version of
# this argument -- same reasoning applies here, technique re-derived rather
# than imported so this file stays standalone). RTT below is measured the
# same way linkprobe.ps1 does it: a raw TCP connect to :443, which is an
# operation these endpoints never filter differently from the product's own
# traffic.
#
# WHY netstat FOR LOSS. Windows has no per-connection retransmit counter
# worth trusting from a script; `netstat -s -p tcp` gives OS-wide TCP stats
# ("Segments Sent", "Segments Retransmitted"), and the ratio of the DELTA
# across a sustained transfer is the least-bad proxy for p available without
# a packet capture.
#
# THE CATCH, STATED PLAINLY: those counters are MACHINE-WIDE, not
# per-process. Any other TCP traffic on this machine during the transfer
# (a browser, a sync client, a campaign's own uploads, Windows Update)
# inflates the retransmit ratio and makes the link look worse than it is.
# Run this on an otherwise-quiet machine. A NOISY result is not evidence of
# loss -- it is evidence the machine was busy. This script cannot tell the
# difference and does not pretend to.
#
# SCOPE. Pure measurement. It never fails, never trips a gate, never writes
# into a campaign's results, and is not wired into any phase or run_all.ps1.
# Run it by hand, read the verdict block, disagree with it if the raw
# numbers say otherwise -- they are printed for exactly that reason.

param(
  [Parameter(Mandatory = $true)]
  [string]$Url,

  [int]$SizeMB = 200,

  [int]$RttSamples = 30,

  [string]$OutDir = (Join-Path $PSScriptRoot ("..\logs\t1-mathis-" + (Get-Date -Format "yyyyMMdd-HHmmss")))
)

$ErrorActionPreference = "Stop"
$MSS_BYTES = 1460

# --- campaign-perturbation check --------------------------------------
# Best-effort only: a campaign's own downloads compete for bandwidth with
# this probe's transfer AND feed the same machine-wide netstat counters
# this script reads, so either direction of interference is possible. This
# can only catch a campaign running in a way that leaves a local trace
# (the linkprobe background job, a live mediagit process) -- it cannot see
# one running on another machine or as a detached process. Absence of a
# warning here is not proof the machine is quiet.
$campaignSigns = @()
if (Get-Job -Name "qa-linkprobe" -ErrorAction SilentlyContinue) { $campaignSigns += "a qa-linkprobe background job is running" }
if (Get-Process -Name "mediagit" -ErrorAction SilentlyContinue) { $campaignSigns += "a mediagit.exe process is running" }
if ($campaignSigns.Count -gt 0) {
  Write-Host "WARNING: this looks like it might be sharing the machine with a live QA campaign:"
  foreach ($s in $campaignSigns) { Write-Host "  - $s" }
  Write-Host "Both directions of interference are possible: this probe's transfer competes for"
  Write-Host "bandwidth, and its netstat-based loss reading includes the campaign's own traffic."
  Write-Host "Consider re-running once the campaign is done."
}

New-Item -ItemType Directory -Force -Path $OutDir | Out-Null
$OutDir = (Resolve-Path $OutDir).Path
Write-Host "T1 Mathis probe -- output: $OutDir"

$u = [Uri]$Url
$rttHost = $u.Host
$rttPort = if ($u.Port -gt 0) { $u.Port } else { 443 }

# --- step 1: RTT, via TCP connect, never ICMP --------------------------
function Get-QaConnectRtt {
  param([string]$RttHost, [int]$Port, [int]$Samples, [int]$TimeoutMs = 5000)

  $rows = @()
  for ($i = 1; $i -le $Samples; $i++) {
    $sw = [Diagnostics.Stopwatch]::StartNew()
    $ok = $false
    $c  = $null
    try {
      $c = New-Object Net.Sockets.TcpClient
      # ConnectAsync + Wait, not the blocking Connect -- same reason as
      # linkprobe.ps1: the blocking form honours the OS connect timeout
      # (~21s), which would let one bad sample stall the whole probe.
      $t  = $c.ConnectAsync($RttHost, $Port)
      $ok = $t.Wait($TimeoutMs) -and $c.Connected
    } catch { $ok = $false }
    finally { if ($c) { try { $c.Close() } catch { } } }
    $sw.Stop()
    $rows += [pscustomobject]@{
      ts         = Get-Date -Format "yyyy-MM-ddTHH:mm:ss"
      sample     = $i
      connect_ms = [int]$sw.Elapsed.TotalMilliseconds
      ok         = $(if ($ok) { 1 } else { 0 })
    }
  }
  return $rows
}

Write-Host "measuring RTT to ${rttHost}:${rttPort} ($RttSamples samples)..."
$rttRows = Get-QaConnectRtt -RttHost $rttHost -Port $rttPort -Samples $RttSamples
$rttTsv = Join-Path $OutDir "rtt-samples.tsv"
$rttRows | Export-Csv -Path $rttTsv -Delimiter "`t" -NoTypeInformation -Encoding UTF8

$rttGood = @($rttRows | Where-Object { $_.ok -eq 1 } | ForEach-Object { $_.connect_ms } | Sort-Object)
if ($rttGood.Count -eq 0) {
  Write-Host "ERROR: every RTT sample failed to connect to ${rttHost}:${rttPort} -- cannot proceed."
  exit 1
}
# Floor, not [int]: PowerShell's [int] cast rounds half-to-even, which would
# misreport the median/p90 on small sample counts (see linkprobe.ps1).
$rttMedianMs = $rttGood[[math]::Floor($rttGood.Count * 0.50)]
$rttP90Ms    = $rttGood[[math]::Min($rttGood.Count - 1, [math]::Floor($rttGood.Count * 0.90))]
if ($rttGood.Count -lt $RttSamples) {
  Write-Host ("  ({0} of {1} samples failed to connect -- median/p90 computed from the {2} that succeeded)" -f
    ($RttSamples - $rttGood.Count), $RttSamples, $rttGood.Count)
}

# --- step 2: netstat snapshot, before ----------------------------------
function Get-QaTcpStats {
  # "Segments Sent" / "Segments Retransmitted" from `netstat -s -p tcp`.
  # Machine-wide counters -- see the file header. Returns $null fields if
  # the output can't be parsed (unexpected locale/OS) rather than guessing.
  $raw = netstat -s -p tcp 2>&1 | Out-String
  $sent      = [regex]::Match($raw, "Segments Sent\s*=\s*(\d+)")
  $retrans   = [regex]::Match($raw, "Segments Retransmitted\s*=\s*(\d+)")
  [pscustomobject]@{
    raw               = $raw
    segments_sent     = $(if ($sent.Success) { [int64]$sent.Groups[1].Value } else { $null })
    segments_retrans  = $(if ($retrans.Success) { [int64]$retrans.Groups[1].Value } else { $null })
  }
}

$netBefore = Get-QaTcpStats
Set-Content -Path (Join-Path $OutDir "netstat-before.txt") -Value $netBefore.raw -Encoding UTF8

# --- step 3: sustained ranged GET, to generate real transfer traffic ---
# Int32 Range API limit: HttpWebRequest.AddRange(int,int) caps just under
# 2GB, comfortably above any sane manual SizeMB for this probe.
if ($SizeMB -gt 2000) {
  Write-Host "SizeMB $SizeMB exceeds the 2000 MB cap for this probe's Range request; clamping to 2000."
  $SizeMB = 2000
}
$targetBytes = [int64]$SizeMB * 1MB

function Get-QaRangedThroughput {
  param([string]$Url, [long]$TargetBytes)

  $ranged = $true
  $req = [Net.HttpWebRequest]::Create($Url)
  $req.Method = "GET"
  $req.Timeout = 600000
  $req.ReadWriteTimeout = 600000
  try {
    $req.AddRange(0, [int]($TargetBytes - 1))
  } catch {
    $ranged = $false
  }

  $sw = [Diagnostics.Stopwatch]::StartNew()
  try {
    $resp = $req.GetResponse()
  } catch [Net.WebException] {
    # 416 Range Not Satisfiable (object smaller than SizeMB, or the endpoint
    # rejected the Range header) -- fall back to a plain GET and self-limit
    # by byte count instead, so a small object doesn't error the probe out.
    $ranged = $false
    $req2 = [Net.HttpWebRequest]::Create($Url)
    $req2.Method = "GET"
    $req2.Timeout = 600000
    $req2.ReadWriteTimeout = 600000
    $sw.Restart()
    $resp = $req2.GetResponse()
  }

  $stream = $resp.GetResponseStream()
  $buf = New-Object byte[] 65536
  $total = 0L
  while ($total -lt $TargetBytes) {
    $toRead = [Math]::Min($buf.Length, $TargetBytes - $total)
    $n = $stream.Read($buf, 0, $toRead)
    if ($n -le 0) { break }
    $total += $n
  }
  $sw.Stop()
  $stream.Close()
  $resp.Close()

  [pscustomobject]@{
    ranged       = $ranged
    bytes        = $total
    elapsed_sec  = $sw.Elapsed.TotalSeconds
    mbps         = $(if ($sw.Elapsed.TotalSeconds -gt 0) { ($total / 1MB) / $sw.Elapsed.TotalSeconds } else { 0 })
  }
}

Write-Host ("downloading ~{0} MB from {1} to generate transfer traffic..." -f $SizeMB, $Url)
$dl = Get-QaRangedThroughput -Url $Url -TargetBytes $targetBytes
if (-not $dl.ranged) {
  Write-Host "  (server did not honour the Range request; measured a plain GET capped at the same byte count)"
}
Write-Host ("  {0:N1} MB in {1:N1}s -> {2:N2} MB/s observed" -f ($dl.bytes / 1MB), $dl.elapsed_sec, $dl.mbps)

# --- step 4: netstat snapshot, after; diff for the loss proxy ----------
$netAfter = Get-QaTcpStats
Set-Content -Path (Join-Path $OutDir "netstat-after.txt") -Value $netAfter.raw -Encoding UTF8

$havePmath = ($null -ne $netBefore.segments_sent) -and ($null -ne $netAfter.segments_sent) -and
             ($null -ne $netBefore.segments_retrans) -and ($null -ne $netAfter.segments_retrans)

$summary = New-Object Text.StringBuilder
[void]$summary.AppendLine("T1 Mathis probe -- $(Get-Date -Format 'yyyy-MM-ddTHH:mm:ss')")
[void]$summary.AppendLine("url:               $Url")
[void]$summary.AppendLine("rtt host:port:     ${rttHost}:${rttPort}")
[void]$summary.AppendLine("rtt median / p90:  $rttMedianMs ms / $rttP90Ms ms  (n=$($rttGood.Count) of $RttSamples)")
[void]$summary.AppendLine("observed transfer: $($dl.bytes) bytes in $([math]::Round($dl.elapsed_sec,1))s -> $([math]::Round($dl.mbps,2)) MB/s (ranged=$($dl.ranged))")

if (-not $havePmath) {
  [void]$summary.AppendLine("")
  [void]$summary.AppendLine("Could not parse 'Segments Sent'/'Segments Retransmitted' from netstat -s -p tcp.")
  [void]$summary.AppendLine("No loss proxy, no Mathis bound, no verdict -- see netstat-before.txt / netstat-after.txt.")
  Write-Host ""
  Write-Host $summary.ToString()
  Set-Content -Path (Join-Path $OutDir "summary.txt") -Value $summary.ToString() -Encoding UTF8
  exit 0
}

$sentDelta    = $netAfter.segments_sent    - $netBefore.segments_sent
$retransDelta = $netAfter.segments_retrans - $netBefore.segments_retrans
$p = if ($sentDelta -gt 0) { [math]::Max(0.0, $retransDelta / [double]$sentDelta) } else { 0.0 }

[void]$summary.AppendLine("segments sent (delta):        $sentDelta")
[void]$summary.AppendLine("segments retransmitted (delta): $retransDelta")
[void]$summary.AppendLine("p (retransmit ratio):          $([math]::Round($p * 100, 4))%")

# Resolution guard. p is a ratio of machine-wide counters, so it can only
# resolve loss down to ~1/sentDelta. A 200MB transfer is ~137k segments and
# resolves ~0.0007%, which is plenty -- but a small -SizeMB (or a transfer that
# died early) can report "p = 0" purely because it never sent enough segments
# for one retransmit to be likely. Saying "no loss" from too few samples is the
# gate-that-cannot-fail shape: an answer that cannot come out any other way.
$MIN_SEGMENTS_FOR_P = 20000
if ($sentDelta -lt $MIN_SEGMENTS_FOR_P) {
  [void]$summary.AppendLine("")
  [void]$summary.AppendLine("VERDICT: inconclusive -- only $sentDelta segments sent (need >= $MIN_SEGMENTS_FOR_P).")
  [void]$summary.AppendLine("Too few to resolve a loss rate: p would be an artefact of sample size, not a")
  [void]$summary.AppendLine("measurement. Re-run with a larger -SizeMB, or check the transfer completed.")
  Write-Host ""
  Write-Host $summary.ToString()
  Set-Content -Path (Join-Path $OutDir "summary.txt") -Value $summary.ToString() -Encoding UTF8
  exit 0
}

# THE BIAS IN THIS MEASUREMENT, AND WHICH WAY IT POINTS.
#
# RTT is sampled BEFORE the transfer, on an idle path. Mathis wants the RTT of
# the LOADED path, which on any bufferbloated link is higher -- sometimes much
# higher. A too-small RTT makes the computed bound too LARGE, which makes the
# observed/bound ratio too SMALL, which biases this script toward printing
# "H-A: link-limited".
#
# That is the dangerous direction: H-A closes the whole transport category
# (QUIC, relay, the accelerator shape) permanently. So an H-A verdict here is
# the one to distrust, and the summary says so where it is printed rather than
# only in this comment. H-B, by contrast, is reported against the bias and is
# the sturdier of the two answers.
if ($p -le 0) {
  [void]$summary.AppendLine("")
  [void]$summary.AppendLine("p ~= 0 -> Mathis bound is undefined (no measurable loss to plug in).")
  [void]$summary.AppendLine("VERDICT: H-A: link-limited -- no loss observed over $sentDelta segments.")
  [void]$summary.AppendLine("Spend effort on dedup rather than transport.")
  [void]$summary.AppendLine("")
  [void]$summary.AppendLine("BEFORE ACTING ON THIS: zero retransmits across a real transfer is a strong")
  [void]$summary.AppendLine("result, but it is also what a transfer that never left the local network")
  [void]$summary.AppendLine("looks like. Confirm the RTT above is a WAN number (tens of ms, not ~0) --")
  [void]$summary.AppendLine("if it is ~0 you measured loopback or a LAN cache, and this verdict is void.")
} else {
  $rttSec = $rttMedianMs / 1000.0
  $boundBps  = $MSS_BYTES / ($rttSec * [math]::Sqrt($p))
  $boundMBps = $boundBps / 1MB
  $ratio = if ($boundMBps -gt 0) { $dl.mbps / $boundMBps } else { 0 }

  [void]$summary.AppendLine("mathis bound:                  $([math]::Round($boundMBps, 2)) MB/s  (MSS=$MSS_BYTES, RTT=$([math]::Round($rttSec*1000,1))ms)")
  [void]$summary.AppendLine("observed / bound ratio:        $([math]::Round($ratio, 2))x")
  [void]$summary.AppendLine("")

  if ($ratio -ge 0.5 -and $ratio -le 2.0) {
    [void]$summary.AppendLine("VERDICT: H-B: TCP/loss-limited -- observed throughput tracks the Mathis bound (within ~2x).")
    [void]$summary.AppendLine("The accelerator/QUIC category is worth pursuing.")
  } elseif ($ratio -lt 0.5) {
    [void]$summary.AppendLine("VERDICT: H-A: link-limited -- observed throughput is far BELOW the Mathis bound.")
    [void]$summary.AppendLine("TCP has headroom it isn't using; loss/RTT are not the bottleneck. Spend effort on dedup.")
    [void]$summary.AppendLine("")
    [void]$summary.AppendLine("TREAT THIS AS THE WEAKER VERDICT. RTT above was sampled on an IDLE path; the")
    [void]$summary.AppendLine("loaded RTT is higher on any bufferbloated link, which inflates the bound and")
    [void]$summary.AppendLine("drags this ratio down -- i.e. the measurement is biased TOWARD H-A, and H-A is")
    [void]$summary.AppendLine("the verdict that closes the transport category for good. Before acting on it,")
    [void]$summary.AppendLine("re-measure RTT DURING a transfer; if loaded RTT is well above the $rttMedianMs ms")
    [void]$summary.AppendLine("used here, recompute before believing this.")
  } else {
    [void]$summary.AppendLine("VERDICT: inconclusive -- observed throughput exceeds the Mathis bound, which the bound")
    [void]$summary.AppendLine("shouldn't allow. Likely a bad RTT or p sample (see the machine-wide netstat caveat above).")
    [void]$summary.AppendLine("Re-run on a quiet machine before trusting either H-A or H-B from this run.")
  }
}

Write-Host ""
Write-Host $summary.ToString()
Set-Content -Path (Join-Path $OutDir "summary.txt") -Value $summary.ToString() -Encoding UTF8
