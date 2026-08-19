# Host watchdog: fail a campaign fast when the MACHINE wedges, instead of
# letting a drill hang against infrastructure that is no longer there.
#
# 2026-08-19 (20260819-gagate13). S5's bulk I/O drove Avast's `aswStm` driver
# ("Gen Stream Filter") to 8,121 kernel events in 47 minutes and wedged the
# filesystem path WSL2's virtual disk sits on. MinIO stopped answering, Docker's
# engine API -- a named PIPE, not TCP -- returned 500, and `mediagit push` sat
# for 41 minutes with no progress. Nothing failed loudly. The campaign simply
# stopped moving, 201 gates in, and burned the rest of the night.
#
# That driver storms on exactly two days in a month of System log: 07-25 and
# 08-19 -- the only two days S5-minio failed. On 08-18, when the same 11 GB
# drill passed, it logged zero. So the event count is a usable early signal,
# whether the storm is the cause or a symptom of the same I/O wedge.
#
# Scope, deliberately narrow: this watches the HOST, never the product. It must
# never convert a mediagit bug into "infrastructure, ignore it" -- so it trips
# only on evidence the machine itself is gone (MinIO unreachable, or a driver
# storm), and it says which one in the marker file.

$script:QaWatchdogJob = $null

# Consecutive seconds MinIO may be unreachable before we call the host wedged.
# 90s is well past any restart the A7 outage drill performs deliberately (it
# stops and starts the container on purpose and the product is expected to ride
# that out), so this cannot fire on A7's intentional outage.
$script:QA_WD_MINIO_DOWN_SEC = 90

# Cumulative aswStm events since the watchdog armed. Baseline on a healthy
# campaign day is 0; the two bad days ran 8,121 and 10,125. 1000 sits far above
# noise and far below either storm.
$script:QA_WD_STORM_EVENTS = 1000

# XPath, NOT -FilterHashtable. `aswStm` is a CLASSIC event-log source, not a
# registered ETW provider, and `-FilterHashtable @{ProviderName='aswStm'}`
# throws "The parameter is incorrect" against it. The first cut of this function
# did exactly that inside a try/catch, so it returned 0 forever and the storm
# branch could never fire -- the "gate that cannot fail" shape, caught only
# because the arming proof checked the query against real logged events (8,121).
$script:QA_WD_STORM_XPATH = "*[System[Provider[@Name='aswStm']]]"

function Get-QaStormCount {
  # Avast absent (CI, another machine) -> 0 forever -> this check never trips.
  # Degrading to "no signal" is correct; a watchdog that errors out is worse.
  param([datetime]$Since)
  try {
    $e = Get-WinEvent -LogName System -FilterXPath $script:QA_WD_STORM_XPATH -MaxEvents 20000 -ErrorAction SilentlyContinue |
      Where-Object { $_.TimeCreated -ge $Since }
    if ($null -eq $e) { return 0 }
    return @($e).Count
  } catch { return 0 }
}

function Start-QaWatchdog {
  param([string]$Phase, [string]$LogDir, [string]$MinioEndpoint)

  $marker = Join-Path $LogDir "WATCHDOG-TRIPPED.txt"
  if (Test-Path $marker) { Remove-Item $marker -Force -EA SilentlyContinue }

  $script:QaWatchdogJob = Start-Job -Name "qa-watchdog" -ScriptBlock {
    param($marker, $minio, $downSec, $stormMax, $armedAt)

    $downSince = $null
    # Only a backend that was ONCE healthy can be said to have died. Without
    # this, a docs-only or perf-only run -- which never starts MinIO -- would
    # abort itself 90 seconds in. "Never up" is not a wedge; it is preflight's
    # job to complain about, and it does.
    $everUp = $false

    while ($true) {
      Start-Sleep -Seconds 15
      $reason = $null

      # 1. Is MinIO answering? This is the symptom that actually costs hours.
      if ($minio) {
        $ok = $false
        try {
          $r = Invoke-WebRequest -Uri "$minio/minio/health/live" -TimeoutSec 5 -UseBasicParsing -EA Stop
          $ok = ($r.StatusCode -ge 200 -and $r.StatusCode -lt 500)
        } catch { $ok = $false }

        if ($ok) {
          $downSince = $null
          $everUp = $true
        } elseif ($everUp) {
          if (-not $downSince) { $downSince = Get-Date }
          $downFor = [int]((Get-Date) - $downSince).TotalSeconds
          if ($downFor -ge $downSec) {
            $reason = "MinIO at $minio was healthy and has now been unreachable for ${downFor}s (limit ${downSec}s)"
          }
        }
      }

      # 2. Driver storm -- names the cause while the evidence is still live.
      if (-not $reason) {
        try {
          # XPath, not -FilterHashtable: see the note on QA_WD_STORM_XPATH.
          $e = Get-WinEvent -LogName System -FilterXPath "*[System[Provider[@Name='aswStm']]]" -MaxEvents 20000 -EA SilentlyContinue |
            Where-Object { $_.TimeCreated -ge $armedAt }
          $n = if ($null -eq $e) { 0 } else { @($e).Count }
          if ($n -ge $stormMax) {
            $reason = "aswStm (Avast Gen Stream Filter) logged $n events since the run armed (limit $stormMax) - the host I/O path is wedging"
          }
        } catch { }
      }

      if ($reason) {
        $txt = @(
          "WATCHDOG TRIPPED $(Get-Date -Format 'yyyy-MM-dd HH:mm:ss')",
          $reason,
          "",
          "The HOST wedged; this is not a mediagit failure. Any drill result after",
          "this point is void. See lib/watchdog.ps1 for the 2026-08-19 incident.",
          "",
          "Recovery that worked: 'docker desktop stop' (compacts the vhdx too),",
          "then 'docker desktop start'. Do NOT use 'wsl --shutdown' - on this",
          "machine it cycles the Hyper-V vSwitch and kills the Wi-Fi Direct",
          "adapter with an NDIS fatal error, taking the network down with it."
        ) -join [Environment]::NewLine
        Set-Content -Path $marker -Value $txt -Encoding UTF8

        # Kill only the CLIENT. The drill then records a real non-zero exit and
        # the campaign moves on, instead of blocking for 41 minutes. Servers are
        # left alone so their logs stay intact for the post-mortem.
        Get-Process mediagit -EA SilentlyContinue | Stop-Process -Force -EA SilentlyContinue
        return
      }
    }
  } -ArgumentList $marker, $MinioEndpoint, $script:QA_WD_MINIO_DOWN_SEC, $script:QA_WD_STORM_EVENTS, (Get-Date)

  Write-QaLog $Phase ("watchdog armed (pid-job {0}): minio={1} down>{2}s, aswStm>{3} events -> abort" -f `
    $script:QaWatchdogJob.Id, $MinioEndpoint, $script:QA_WD_MINIO_DOWN_SEC, $script:QA_WD_STORM_EVENTS)
}

function Test-QaWatchdogTripped {
  param([string]$LogDir)
  $marker = Join-Path $LogDir "WATCHDOG-TRIPPED.txt"
  if (Test-Path $marker) { return (Get-Content $marker -Raw) }
  return $null
}

function Stop-QaWatchdog {
  param([string]$Phase)
  if ($script:QaWatchdogJob) {
    Stop-Job $script:QaWatchdogJob -EA SilentlyContinue
    Remove-Job $script:QaWatchdogJob -Force -EA SilentlyContinue
    $script:QaWatchdogJob = $null
    if ($Phase) { Write-QaLog $Phase "watchdog disarmed" }
  }
}
