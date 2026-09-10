# Stop/start the NATIVE Silo process serving the QA S3 endpoint.
#
# WHY THIS EXISTS. 07_abuse's A7 drill proves the product survives a backend
# outage mid-push. It used to `docker stop mediagit-minio`; when the S3 backend
# moved to a native Silo process there was no container, so the drill SKIPped --
# and a SKIP reads as green. 2406ce0 turned that into a FAIL with a message
# naming this script, but the script was never written, so ga32 failed
# A7-backend-outage with "configuration, not capability".
#
# NOTHING HERE IS HARDCODED TO ONE MACHINE. `stop` reads the live process's own
# command line and records it, so `start` relaunches exactly what was running.
# That keeps the pair correct on any host without this tracked file carrying
# somebody's local install path.
#
# Contract expected by 07_abuse.ps1 (~line 424): each action is invoked as
#   powershell -NoProfile -Command "<cmd>"
# and must be SYNCHRONOUS -- the drill stops the backend mid-push and then
# asserts on the client's behaviour, so returning before the port is actually
# down (or actually back up) would make the drill race the backend.

param(
  [Parameter(Mandatory = $true)][ValidateSet("stop", "start", "status")]
  [string]$Action,
  [int]$Port = 9000,
  [int]$TimeoutSec = 60
)

$ErrorActionPreference = "Continue"

# Survives between the two separate powershell invocations.
$stateFile = Join-Path $env:TEMP "mg-qa-silo-cmdline.txt"
$health    = "http://127.0.0.1:$Port/minio/health/live"

function Get-Listener {
  $c = Get-NetTCPConnection -LocalPort $Port -State Listen -ErrorAction SilentlyContinue |
       Select-Object -First 1
  if (-not $c) { return $null }
  return Get-Process -Id $c.OwningProcess -ErrorAction SilentlyContinue
}

function Test-Healthy {
  try {
    return (Invoke-WebRequest -Uri $health -UseBasicParsing -TimeoutSec 3).StatusCode -eq 200
  } catch { return $false }
}

switch ($Action) {

  "status" {
    $p = Get-Listener
    if ($p) { Write-Host "silo_native: :$Port held by $($p.ProcessName) pid=$($p.Id) healthy=$(Test-Healthy)" }
    else    { Write-Host "silo_native: nothing listening on :$Port" }
    exit 0
  }

  "stop" {
    $p = Get-Listener
    if (-not $p) { Write-Host "silo_native: already down"; exit 0 }

    # Record the command line BEFORE killing -- afterwards it is unrecoverable,
    # and a start that cannot reproduce the original arguments would bring the
    # backend back pointing at the wrong data directory.
    $wmi = Get-CimInstance Win32_Process -Filter "ProcessId=$($p.Id)" -ErrorAction SilentlyContinue
    if ($wmi -and $wmi.CommandLine) {
      Set-Content -Path $stateFile -Value $wmi.CommandLine -Encoding ascii
      Write-Host "silo_native: recorded cmdline for restart"
    } elseif (-not (Test-Path $stateFile)) {
      Write-Host "silo_native: FATAL cannot read cmdline of pid=$($p.Id) and no prior record exists"
      exit 2
    }

    Stop-Process -Id $p.Id -Force -ErrorAction SilentlyContinue

    # Synchronous: the drill's next step assumes the backend is genuinely gone.
    $deadline = (Get-Date).AddSeconds($TimeoutSec)
    while ((Get-Date) -lt $deadline) {
      if (-not (Get-Listener)) { Write-Host "silo_native: stopped, :$Port free"; exit 0 }
      Start-Sleep -Milliseconds 200
    }
    Write-Host "silo_native: FATAL :$Port still held after ${TimeoutSec}s"
    exit 3
  }

  "start" {
    if (Test-Healthy) { Write-Host "silo_native: already up"; exit 0 }
    if (-not (Test-Path $stateFile)) {
      Write-Host "silo_native: FATAL no recorded cmdline at $stateFile - run -Action stop first"
      exit 4
    }

    $cmdline = (Get-Content $stateFile -Raw).Trim()
    # Split "exe" from its arguments, honouring the quoted exe path.
    if ($cmdline -match '^\s*"([^"]+)"\s*(.*)$') { $exe = $Matches[1]; $rest = $Matches[2] }
    elseif ($cmdline -match '^\s*(\S+)\s*(.*)$')  { $exe = $Matches[1]; $rest = $Matches[2] }
    else { Write-Host "silo_native: FATAL unparseable cmdline: $cmdline"; exit 5 }

    if (-not (Test-Path $exe)) { Write-Host "silo_native: FATAL exe not found: $exe"; exit 6 }

    Start-Process -FilePath $exe -ArgumentList $rest -WindowStyle Hidden | Out-Null

    # Health, not "the command returned" -- a campaign that resumes against a
    # backend still coming up fails in a way that reads like a product bug.
    $deadline = (Get-Date).AddSeconds($TimeoutSec)
    while ((Get-Date) -lt $deadline) {
      if (Test-Healthy) { Write-Host "silo_native: started, healthy on :$Port"; exit 0 }
      Start-Sleep -Milliseconds 300
    }
    Write-Host "silo_native: FATAL did not become healthy within ${TimeoutSec}s"
    exit 7
  }
}
