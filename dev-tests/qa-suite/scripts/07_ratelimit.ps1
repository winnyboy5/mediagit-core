# Phase 7 (ratelimit) - the rate limiter, actually armed.
# ASCII-only, PS 5.1 compatible.
#
# This phase exists because the suite had never once run a server with rate
# limiting on. `enable_rate_limiting` defaults to false and nothing in the
# harness set it, so every campaign measured a limiter that was not there. The
# visible cost: 07_abuse's A13 (a per-chunk push must NOT trip 429) could not
# fail, and a 10 rps default shipped and produced 429 storms on real pushes.
#
# Every drill here is built so it CAN fail. RL1 is the arming proof for the
# whole phase: if the limiter is not running RL1 goes red, and the "no 429"
# drills below mean nothing on their own.
#
#   RL1-enforces        a tiny budget (1 rps / burst 2) DOES return 429 past
#                       the burst. Proves the limiter is on and reachable.
#   RL2-429-headers     that 429 carries Retry-After and x-ratelimit-* so a
#                       client can back off instead of guessing.
#   RL3-push-defaults   a real multi-chunk push under the PRODUCT's own
#                       defaults completes with no 429. Regression guard for
#                       the shipped bug.
#   RL4-clone-defaults  same for clone, which fans out reads.
#   RL5-hardened        push+clone still succeed under the tighter "public"
#                       profile an operator might deploy.

. (Join-Path $PSScriptRoot "lib\common.ps1")
. (Join-Path $PSScriptRoot "lib\remote.ps1")

$Phase = "07_ratelimit"
$env:MEDIAGIT_AUTHOR_NAME = "QA-Suite"
$env:MEDIAGIT_AUTHOR_EMAIL = "qa-suite@mediagit.local"

$TSV = Join-Path $QA.Logs "ratelimit_results.tsv"
$script:AllPass = $true

function Rec([string]$Drill, $Pass, [string]$Detail) {
  Write-QaRow $TSV @("drill", "pass", "detail") @($Drill, $Pass, $Detail)
  $tag = if ("$Pass" -eq "SKIP") { "SKIP" } elseif ($Pass) { "PASS" } else { "FAIL" }
  Write-QaLog $Phase ("{0} -> {1}  {2}" -f $Drill, $tag, $Detail)
  Write-QaGate $Phase $Drill $Pass $Detail
  if ($tag -eq "FAIL") { $script:AllPass = $false }
}

function New-QaBinaryFixture([string]$Path, [int]$SizeMB, [int]$Seed) {
  $dir = Split-Path $Path -Parent
  if (-not (Test-Path $dir)) { New-Item -ItemType Directory -Path $dir -Force | Out-Null }
  $rnd = New-Object System.Random($Seed)
  $bytes = New-Object byte[] ($SizeMB * 1MB)
  $rnd.NextBytes($bytes)
  [IO.File]::WriteAllBytes($Path, $bytes)
}

# PS 5.1 throws on non-2xx. Returns status plus headers, because RL2 gates on
# the headers of the SAME 429 that RL1 produced.
function Invoke-QaProbe([string]$Uri) {
  try {
    $r = Invoke-WebRequest -Uri $Uri -UseBasicParsing -TimeoutSec 10 -ErrorAction Stop
    return @{ Code = [int]$r.StatusCode; Headers = $r.Headers }
  } catch {
    $resp = $_.Exception.Response
    if (-not $resp) { return @{ Code = -1; Headers = @{} } }
    $h = @{}
    try { foreach ($k in $resp.Headers.AllKeys) { $h[$k] = $resp.Headers[$k] } } catch {}
    return @{ Code = [int]$resp.StatusCode; Headers = $h }
  }
}

Write-QaLog $Phase "=== 07_ratelimit start ==="

$srv = $null
try {
  # ---- RL1 + RL2: enforcement, on a budget small enough to trip deliberately ----
  $srv = Start-QaServer -Backend "minio" -Phase "$Phase-enforce" -RateLimitRps 1 -RateLimitBurst 2
  $codes = @()
  $limitedHeaders = $null
  for ($i = 0; $i -lt 12; $i++) {
    # $srv.Url already ends in the repo name, so this is /{repo}/info/refs.
    # An earlier version appended another segment and every probe came back 404
    # from the route fallback without ever reaching the limiter.
    $probe = Invoke-QaProbe "$($srv.Url)/info/refs"
    $codes += $probe.Code
    if ($probe.Code -eq 429 -and -not $limitedHeaders) { $limitedHeaders = $probe.Headers }
  }
  $got429 = ($codes -contains 429)
  # Not merely "something answered": some request must have been SERVED, or a
  # server that 429s everything, or one that is simply broken, reads as
  # enforcement working.
  $servedSome = @($codes | Where-Object { $_ -ne 429 -and $_ -ne -1 }).Count -gt 0
  Rec "RL1-enforces" ($got429 -and $servedSome) `
    ("budget=1rps/burst2 codes=[{0}] got429={1} servedSome={2}" -f ($codes -join ","), $got429, $servedSome)

  if ($got429) {
    $hdrNames = @($limitedHeaders.Keys)
    $hasRetryAfter = @($hdrNames | Where-Object { $_ -match "(?i)^retry-after$" }).Count -gt 0
    $hasRlHeaders = @($hdrNames | Where-Object { $_ -match "(?i)^x-ratelimit-" }).Count -gt 0
    Rec "RL2-429-headers" ($hasRetryAfter -and $hasRlHeaders) `
      ("retry-after={0} x-ratelimit-star={1} headers=[{2}]" -f $hasRetryAfter, $hasRlHeaders, ($hdrNames -join ","))
  } else {
    # Honest: no 429 means RL2 inspected nothing. That is a FAIL, not a pass -
    # recording it green here is exactly the vacuity this phase exists to end.
    Rec "RL2-429-headers" $false "no 429 was produced, so its headers were never observed (see RL1)"
  }
  Stop-QaServer @{ Proc = $srv.Proc }; $srv = $null

  # ---- RL3 + RL4: the product's own defaults must carry real work ----
  # No -RateLimitRps/-RateLimitBurst, so the server uses its compiled-in
  # defaults - exactly what an operator gets from `mediagit-server init`.
  $srv = Start-QaServer -Backend "minio" -Phase "$Phase-defaults"
  # Anti-vacuous: prove the limiter is configured on for THIS server too, or
  # "no 429" below repeats the A13 mistake verbatim.
  $cfgText = Get-Content $srv.ConfigPath -Raw
  $limiterOn = $cfgText -match "enable_rate_limiting\s*=\s*true"

  $repo = New-SandboxRepo "rl-defaults" $Phase
  for ($i = 0; $i -lt 6; $i++) {
    New-QaBinaryFixture (Join-Path $repo "part$i.bin") 8 (77100 + $i)
  }
  Invoke-MG $repo @("add", ".") $Phase | Out-Null
  Invoke-MG $repo @("commit", "-m", "rate limit defaults") $Phase | Out-Null
  Invoke-MG $repo @("remote", "add", "origin", $srv.Url) $Phase | Out-Null

  $push = Invoke-MG $repo @("push", "-u", "origin", "main") $Phase -TimeoutSec 1800
  $pushLimited = ($push.Out -match "(?i)429|rate.?limit|too many requests")
  Rec "RL3-push-defaults" (($push.Exit -eq 0) -and (-not $pushLimited) -and $limiterOn) `
    ("exit=$($push.Exit) sec=$($push.Sec) saw-429=$pushLimited limiter-configured-on=$limiterOn")

  $cloneDir = Join-Path (Split-Path $repo -Parent) "rl-defaults-clone"
  $clone = Invoke-MG $null @("clone", $srv.Url, $cloneDir) $Phase -TimeoutSec 1800
  $cloneLimited = ($clone.Out -match "(?i)429|rate.?limit|too many requests")
  Rec "RL4-clone-defaults" (($clone.Exit -eq 0) -and (-not $cloneLimited) -and $limiterOn) `
    ("exit=$($clone.Exit) sec=$($clone.Sec) saw-429=$cloneLimited limiter-configured-on=$limiterOn")
  Stop-QaServer @{ Proc = $srv.Proc }; $srv = $null

  # ---- RL5: the tighter profile a public deployment would use ----
  # If it cannot carry one push and one clone then the profile is not
  # deployable and the documentation recommending it is wrong.
  $srv = Start-QaServer -Backend "minio" -Phase "$Phase-hardened" -RateLimitRps 20 -RateLimitBurst 40
  $repo2 = New-SandboxRepo "rl-hardened" $Phase
  for ($i = 0; $i -lt 4; $i++) {
    New-QaBinaryFixture (Join-Path $repo2 "h$i.bin") 8 (77200 + $i)
  }
  Invoke-MG $repo2 @("add", ".") $Phase | Out-Null
  Invoke-MG $repo2 @("commit", "-m", "hardened profile") $Phase | Out-Null
  Invoke-MG $repo2 @("remote", "add", "origin", $srv.Url) $Phase | Out-Null
  $push2 = Invoke-MG $repo2 @("push", "-u", "origin", "main") $Phase -TimeoutSec 1800
  $clone2Dir = Join-Path (Split-Path $repo2 -Parent) "rl-hardened-clone"
  $clone2 = Invoke-MG $null @("clone", $srv.Url, $clone2Dir) $Phase -TimeoutSec 1800
  # A 429 here is not a failure: at 20 rps a real push is SUPPOSED to hit the
  # limit. The gate is that the operation still finishes, because the client
  # honours Retry-After and retries. Gating on "never saw a 429" would have
  # made this drill fail for the limiter doing its job, and would have hidden
  # the thing actually worth testing - recovery.
  $sawLimit = ($push2.Out -match "(?i)429|too many requests") -or ($clone2.Out -match "(?i)429|too many requests")
  $completed = ($push2.Exit -eq 0) -and ($clone2.Exit -eq 0)
  Rec "RL5-hardened-profile-survivable" $completed `
    ("profile=20rps/40burst push=$($push2.Exit) clone=$($clone2.Exit) hit-limit-and-recovered=$sawLimit")

  Stop-QaServer @{ Proc = $srv.Proc }; $srv = $null

  # ---- RL6: recovery, on a budget tight enough to GUARANTEE throttling ----
  #
  # This used to piggyback on RL5's 20rps/40burst server and only assert
  # recovery "if a 429 happened". Once `requests_per_second` was fixed to mean
  # requests per second (it was being passed where tower_governor wanted a
  # replenish INTERVAL, so 20 meant one request every 20 seconds), 20 rps became
  # genuinely comfortable and a small push stopped tripping the limiter at all -
  # so the drill could no longer arm itself and correctly reported that it had
  # verified nothing.
  #
  # Recovery and survivability are two different claims and need two different
  # budgets. RL5 asks "is the documented public profile usable?" and wants
  # headroom. RL6 asks "when the limiter DOES fire, does the client come back?"
  # and therefore needs a budget the workload cannot help but exceed.
  #
  # 2 rps / burst 4: low enough that a multi-chunk push is throttled repeatedly,
  # high enough that the server's own startup probe still completes (at 1 rps
  # under the OLD semantics the probe timed out and the server never came up).
  $srv = Start-QaServer -Backend "minio" -Phase "$Phase-recovery" -RateLimitRps 2 -RateLimitBurst 4
  $repo3 = New-SandboxRepo "rl-recovery" $Phase
  for ($i = 0; $i -lt 6; $i++) {
    New-QaBinaryFixture (Join-Path $repo3 "r$i.bin") 8 (77300 + $i)
  }
  Invoke-MG $repo3 @("add", ".") $Phase | Out-Null
  Invoke-MG $repo3 @("commit", "-m", "recovery") $Phase | Out-Null
  Invoke-MG $repo3 @("remote", "add", "origin", $srv.Url) $Phase | Out-Null
  $push3 = Invoke-MG $repo3 @("push", "-u", "origin", "main") $Phase -TimeoutSec 1800
  $clone3Dir = Join-Path (Split-Path $repo3 -Parent) "rl-recovery-clone"
  $clone3 = Invoke-MG $null @("clone", $srv.Url, $clone3Dir) $Phase -TimeoutSec 1800

  $throttled = ($push3.Out -match "(?i)429|too many requests") -or `
               ($clone3.Out -match "(?i)429|too many requests")
  $recovered = ($push3.Exit -eq 0) -and ($clone3.Exit -eq 0)
  # BOTH halves required. Without $throttled this passes on a limiter that never
  # fired; without $recovered it passes on a client that gave up.
  Rec "RL6-client-recovers-from-429" ($throttled -and $recovered) `
    ("budget=2rps/burst4 throttled=$throttled push=$($push3.Exit) clone=$($clone3.Exit) recovered=$recovered")

} catch {
  $skip = ("$_" -match "^SKIP:")
  $tag = if ($skip) { "SKIP" } else { $false }
  $detail = if ($skip) { "$_" } else { "unexpected error: $_" }
  Rec "07_ratelimit" $tag $detail
} finally {
  if ($srv) { Stop-QaServer @{ Proc = $srv.Proc } }
}

Write-QaLog $Phase "=== 07_ratelimit done: overall=$(if ($script:AllPass) { 'PASS' } else { 'FAIL' }) ==="
# Teardown: reclaim this phase's own work/ scratch. work/ ONLY - logs/ and
# fixtures-synthetic/ are never touched.
Invoke-QaTeardown $Phase @("rl-*")

Exit-QaPhase $Phase (-not $script:AllPass)
