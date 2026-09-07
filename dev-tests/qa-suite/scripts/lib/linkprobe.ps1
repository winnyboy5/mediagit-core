# Per-arm link measurement: record what the network was doing, continuously,
# for the whole campaign.
#
# WHY. run_ga already checks WLAN 8003 disconnects and power events, and ga41
# and ga42 both printed "the link held for the whole run" -- while the link was
# dropping roughly 1-in-10 packets with 8-320ms jitter and, in ga42, losing
# s3.ap-south-1 entirely for four minutes. Two independent HTTP stacks (server
# aws-sdk-s3 -> "dispatch failure", client reqwest -> "Connect, TimedOut" x82)
# failed against the same host in the same window and the run's own record had
# nothing to say about it. A bad-network run and a bad-code run were
# indistinguishable, which is why ga42 cost an hour of forensics.
#
# TCP CONNECT, NOT ICMP -- deliberately. S3, Azure Blob and GCS all deprioritise
# or drop ICMP, so a ping-based probe would report loss that means nothing and
# stay quiet on the failure that matters: this is the "gate that cannot fail"
# shape (feedback_gates_that_cannot_fail), and it has bitten this suite six
# times. A TCP handshake to :443 is the exact operation ga42 lost, is never
# filtered differently from the product's own traffic, and its failure is
# unambiguous.
#
# SCOPE. This only ever RECORDS. It never trips, never kills anything, and never
# fails a phase -- that is the watchdog's job. Its output exists so that a human
# reading a failed run can answer "product or link?" from the run's own record.
# It must never let a mediagit bug be waved away as "bad network": a clean link
# report is evidence AGAINST the network, which is just as useful.

$script:QaLinkProbeJob = $null

# 5s sampling. A 3h campaign is ~2,100 samples per host -- small enough to hold
# in a TSV and dense enough to resolve the four-minute S3 outage ga42 hit into
# ~48 consecutive failures rather than one ambiguous blip.
$script:QA_LP_INTERVAL_SEC = 5
$script:QA_LP_TIMEOUT_MS   = 5000

function Get-QaLinkHosts {
  # The arms the campaign actually pushes to. Azure is per-account, so it is
  # derived from the same env the drills use rather than hardcoded.
  $h = @("s3.$($env:AWS_DEFAULT_REGION).amazonaws.com", "storage.googleapis.com")
  if ($env:AZURE_STORAGE_ACCOUNT) { $h += "$($env:AZURE_STORAGE_ACCOUNT).blob.core.windows.net" }
  return $h
}

function Start-QaLinkProbe {
  param([string]$LogDir, [string[]]$Hosts)

  $tsv = Join-Path $LogDir "link-samples.tsv"
  "ts`thost`tconnect_ms`tok" | Set-Content -Path $tsv -Encoding UTF8

  $script:QaLinkProbeJob = Start-Job -Name "qa-linkprobe" -ScriptBlock {
    param($tsv, $hosts, $intervalSec, $timeoutMs)

    while ($true) {
      foreach ($h in $hosts) {
        $sw = [Diagnostics.Stopwatch]::StartNew()
        $ok = $false
        $c  = $null
        try {
          $c = New-Object Net.Sockets.TcpClient
          # ConnectAsync + Wait, not the blocking Connect: the blocking form
          # honours the OS connect timeout (~21s on Windows), which would let a
          # single dead host stall the sampler past its own interval and leave
          # a GAP in the record exactly when the record matters most.
          $t = $c.ConnectAsync($h, 443)
          $ok = $t.Wait($timeoutMs) -and $c.Connected
        } catch { $ok = $false }
        finally { if ($c) { try { $c.Close() } catch { } } }
        $sw.Stop()

        # Failures are recorded with the time they took to fail, not as a hole:
        # a fast refusal and a 5s timeout are different network conditions.
        ("{0}`t{1}`t{2}`t{3}" -f (Get-Date -Format "yyyy-MM-ddTHH:mm:ss"), $h,
          [int]$sw.Elapsed.TotalMilliseconds, $(if ($ok) { 1 } else { 0 })) |
          Add-Content -Path $tsv -Encoding UTF8
      }
      Start-Sleep -Seconds $intervalSec
    }
  } -ArgumentList $tsv, $Hosts, $script:QA_LP_INTERVAL_SEC, $script:QA_LP_TIMEOUT_MS

  Write-Host ("link probe armed: " + ($Hosts -join ", ") + "  -> $tsv")
}

function Stop-QaLinkProbe {
  if ($script:QaLinkProbeJob) {
    Stop-Job   $script:QaLinkProbeJob -EA SilentlyContinue
    Remove-Job $script:QaLinkProbeJob -Force -EA SilentlyContinue
    $script:QaLinkProbeJob = $null
  }
}

function Write-QaLinkSummary {
  param([string]$LogDir)

  $tsv = Join-Path $LogDir "link-samples.tsv"
  if (-not (Test-Path $tsv)) { Write-Host "no link samples recorded."; return }

  $rows = @(Import-Csv $tsv -Delimiter "`t")
  if ($rows.Count -eq 0) { Write-Host "link probe recorded no samples."; return }

  foreach ($g in ($rows | Group-Object host | Sort-Object Name)) {
    $n    = $g.Group.Count
    $bad  = @($g.Group | Where-Object { $_.ok -eq "0" })
    $good = @($g.Group | Where-Object { $_.ok -eq "1" } |
              ForEach-Object { [int]$_.connect_ms } | Sort-Object)
    $loss = if ($n) { [math]::Round(100.0 * $bad.Count / $n, 2) } else { 0 }

    # Floor, not [int]: PowerShell's [int] cast rounds half-to-even, so
    # [int](3 * 0.5) is 2 and a 3-sample p50 would report the MAX.
    $p50 = if ($good.Count) { $good[[math]::Floor($good.Count * 0.50)] } else { 0 }
    $p95 = if ($good.Count) { $good[[math]::Min($good.Count - 1, [math]::Floor($good.Count * 0.95))] } else { 0 }
    $max = if ($good.Count) { $good[-1] } else { 0 }

    $verdict = if ($loss -eq 0 -and $p95 -lt 1000) { "clean" }
               elseif ($loss -lt 2) { "MARGINAL" } else { "BAD" }
    Write-Host ("  {0,-46} n={1,-5} loss={2,6}%  p50={3,5}ms p95={4,6}ms max={5,6}ms  {6}" -f
      $g.Name, $n, $loss, $p50, $p95, $max, $verdict)

    # The correlation the forensics actually needed: WHEN was it bad, so a
    # failing phase's timestamp can be checked against it. Consecutive failures
    # are collapsed into outage windows -- ga42's four-minute S3 loss should
    # read as one line, not 48.
    $runs = @()
    $start = $null; $prev = $null
    foreach ($r in ($g.Group | Sort-Object ts)) {
      $t = [datetime]::Parse($r.ts)
      if ($r.ok -eq "0") {
        if (-not $start) { $start = $t }
        $prev = $t
      } elseif ($start) {
        $runs += [pscustomobject]@{ From = $start; To = $prev; Sec = [int]($prev - $start).TotalSeconds + 5 }
        $start = $null
      }
    }
    if ($start) { $runs += [pscustomobject]@{ From = $start; To = $prev; Sec = [int]($prev - $start).TotalSeconds + 5 } }

    foreach ($o in ($runs | Sort-Object Sec -Descending | Select-Object -First 5)) {
      Write-Host ("      UNREACHABLE {0:HH:mm:ss} -> {1:HH:mm:ss}  ({2}s)" -f $o.From, $o.To, $o.Sec)
    }
  }
}
