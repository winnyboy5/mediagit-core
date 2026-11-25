# Demo video runner: Playwright drives the real release desktop app (ui/tests/e2e/demo),
# one clip per chapter, then cloud cleanup, Silo wipe and stitch. ASCII-only, PS 5.1.
#   -Chapters "01,07"   only these chapters (default: all)
# Output: D:\own\saas\qa-live\demo\NN-slug.mp4 and mediagit-demo.mp4.
# Exit: 0 ok; 1 playwright failed; 3 cloud or Silo cleanup failed; 4 stitch failed; 73 bad -Chapters.
# Teardown (demo-setup.ts) stops only the processes this run recorded. The cloud cleaner
# refuses while ANY mediagit process runs, so a user session left open fails the run (exit 3).
# Silo wipe: objects under mediagit-qa-suite/demo-video-silo/ on :9000, nothing else. Creds from
# MG_DEMO_SILO_KEY / MG_DEMO_SILO_SECRET, else the local-only dev pair S-13 uses (read from its
# spec at run time, never printed). Skipped with a warning when :9000 is down.
# Cloud variables come from campaign_env.ps1 unless MG_DEMO_NO_ENV is set.

param([string]$Chapters = "")

$ErrorActionPreference = "Continue"
function Say([string]$m) { Write-Host ("[demo] " + $m) }

if (-not $env:MG_DEMO_NO_ENV) { . (Join-Path $PSScriptRoot "campaign_env.ps1") }

$repo = (Resolve-Path (Join-Path $PSScriptRoot "..\..\..")).Path
$ui   = Join-Path $repo "ui"
$out  = "D:\own\saas\qa-live\demo"
New-Item -ItemType Directory -Force $out | Out-Null
$env:MG_DEMO_OUT = $out

$grep = @()
if ($Chapters) {
  $ids = @($Chapters -split ',' | ForEach-Object { $_.Trim() } | Where-Object { $_ })
  foreach ($i in $ids) { if ($i -notmatch '^\d\d$') { Say "FATAL: bad chapter '$i' (two digits, e.g. 01,07)"; exit 73 } }
  $grep = @("-g", ("(^| )(" + ($ids -join "|") + ") [A-Z]"))
}

# Returns $true when the Silo prefix is empty afterwards (or Silo is down and skipped).
function Wipe-Silo {
  $up = $false
  try { $up = ((Invoke-WebRequest -UseBasicParsing -TimeoutSec 5 "http://127.0.0.1:9000/minio/health/live").StatusCode -eq 200) } catch { }
  if (-not $up) { Say "WARNING: Silo not answering on :9000, skipping the Silo wipe"; return $true }
  $mcli = $env:MG_QA_MCLI
  if (-not $mcli) { $mcli = Join-Path $env:USERPROFILE "silo\mcli.exe" }
  if (-not (Test-Path $mcli)) { Say "Silo wipe FAILED: no mcli client (set MG_QA_MCLI)"; return $false }
  $key = $env:MG_DEMO_SILO_KEY; $secret = $env:MG_DEMO_SILO_SECRET
  if (-not $key -or -not $secret) {
    $spec = Get-Content (Join-Path $ui "tests\e2e\qa-live\S-13-silo-storage.spec.ts") -Raw
    if ($spec -match "key: '([^']+)', secret: '([^']+)'") { $key = $matches[1]; $secret = $matches[2] }
  }
  if (-not $key -or -not $secret) { Say "Silo wipe FAILED: no credentials (set MG_DEMO_SILO_KEY / MG_DEMO_SILO_SECRET)"; return $false }
  $env:MC_HOST_s = "http://${key}:${secret}@127.0.0.1:9000"
  $target = "s/mediagit-qa-suite/demo-video-silo/"
  try {
    & $mcli rm --recursive --force $target *> $null
    $left = @(& $mcli ls --recursive $target 2> $null | Where-Object { $_ }).Count
  } finally { Remove-Item Env:MC_HOST_s -ErrorAction SilentlyContinue }
  if ($left -ne 0) { Say "Silo wipe FAILED: $left object(s) left under demo-video-silo/"; return $false }
  Say "Silo wipe done (demo-video-silo/ empty)"
  return $true
}

$code = 0
Push-Location $ui
try {
  # Narration: synthesize every caption once (cached) before recording. MG_DEMO_TTS=none skips it; default sapi.
  # A stale manifest from another engine would narrate with the wrong voice, so none also removes it.
  if (-not $env:MG_DEMO_VOICE) { $env:MG_DEMO_VOICE = "expr-voice-3-m" }
  if ($env:MG_DEMO_TTS -eq "none") {
    Remove-Item (Join-Path $out "tts-cache\manifest.json") -ErrorAction SilentlyContinue
  } else {
    & node (Join-Path $ui "tests\e2e\demo\narrate.mjs") $out
    if ($LASTEXITCODE -ne 0) { Say "narrate exited $LASTEXITCODE"; exit 5 }
  }
  & pnpm exec playwright test -c playwright.demo.config.ts @grep
  if ($LASTEXITCODE -ne 0) { Say "playwright exited $LASTEXITCODE"; $code = 1 }
}
finally {
  Pop-Location
  & powershell -NoProfile -File (Join-Path $PSScriptRoot "clean_after_run_cloud.ps1")
  if ($LASTEXITCODE -ne 0) { Say "cloud cleanup exited $LASTEXITCODE"; if ($code -eq 0) { $code = 3 } }
  if (-not (Wipe-Silo)) { if ($code -eq 0) { $code = 3 } }
  & node (Join-Path $ui "tests\e2e\demo\stitch.mjs") $out
  if ($LASTEXITCODE -ne 0) { Say "stitch exited $LASTEXITCODE"; if ($code -eq 0) { $code = 4 } }
}
exit $code
