# Phase 7 (setup) - interactive-setup elimination + auth-disabled (§G).
# ASCII-only, PS 5.1 compatible. Proves a MediaGit server can be stood up and
# used with NO curl, NO hand-edited TOML, NO hand-edited users.jsonl, and (the
# whole point) that the FIRST client login needs no env var and no config edit.
#
#   S1-wizard-auth-on   `mediagit-server init --non-interactive --enable-auth`
#                       writes a bootable toml (jwt_secret generated,
#                       allow_open_registration=false, rate limiting on), boots,
#                       admin created in one flow.
#   S2-login-no-env     auth login (no MEDIAGIT_TOKEN, no config token) -> clone
#                       -> push all succeed off the stored credential; status
#                       names the keychain tier; logout -> push then 401s.
#   S3-apikey           auth key create -> push with X-API-Key succeeds.
#   S4-jwt-env-override MEDIAGIT_JWT_SECRET overrides the toml jwt_secret
#                       (tokens minted under the toml secret then rejected).
#   S5-wizard-auth-off  init with auth OFF writes a bootable toml; a plain
#                       init/add/commit/push round-trip works; auth status/login
#                       against it exit 0 with the "auth disabled" message.
#   S6-nonloopback-off  init with a non-loopback host + auth off is REFUSED
#                       (mirrors the runtime bind refusal, main.rs).
#
# Uses the local (filesystem) backend so it runs on any dev box without cloud
# creds. Server launched from the WIZARD's own config (not Start-QaServer) since
# the wizard output is exactly what's under test.

. (Join-Path $PSScriptRoot "lib\common.ps1")
. (Join-Path $PSScriptRoot "lib\remote.ps1")

$Phase = "07_setup"
$env:MEDIAGIT_AUTHOR_NAME = "QA-Suite"
$env:MEDIAGIT_AUTHOR_EMAIL = "qa-suite@mediagit.local"

$TSV = Join-Path $QA.Logs "setup_results.tsv"
$script:AllPass = $true

function Rec([string]$Drill, $Pass, [string]$Detail) {
  Write-QaRow $TSV @("drill", "pass", "detail") @($Drill, $Pass, $Detail)
  $tag = if ("$Pass" -eq "SKIP") { "SKIP" } elseif ($Pass) { "PASS" } else { "FAIL" }
  Write-QaLog $Phase ("{0} -> {1}  {2}" -f $Drill, $tag, $Detail)
  Write-QaGate $Phase $Drill $Pass $Detail
  if ($tag -eq "FAIL") { $script:AllPass = $false }
}

# Launch mediagit-server from an existing config; poll /health; return a handle
# shaped like Start-QaServer's so Stop-QaServer works. Throws on unhealthy.
function Start-FromConfig([string]$ConfigPath, [string]$BaseUrl, [string]$Tag) {
  $outLog = Join-Path $QA.Logs "server-$Tag.out.log"
  $errLog = Join-Path $QA.Logs "server-$Tag.err.log"
  $proc = Start-Process -FilePath $QA.MGServer -ArgumentList @("--config", $ConfigPath) `
    -PassThru -NoNewWindow -RedirectStandardOutput $outLog -RedirectStandardError $errLog
  for ($i = 0; $i -lt 40; $i++) {
    if ($proc.HasExited) { break }
    try {
      $r = Invoke-WebRequest -Uri "$BaseUrl/health" -UseBasicParsing -TimeoutSec 2 -ErrorAction Stop
      if ($r.StatusCode -eq 200) { return @{ Proc = $proc; BaseUrl = $BaseUrl; OutLog = $outLog; ErrLog = $errLog } }
    } catch {}
    Start-Sleep -Milliseconds 500
  }
  $err = if (Test-Path $errLog) { Get-Content $errLog -Raw -ErrorAction SilentlyContinue } else { "" }
  if (-not $proc.HasExited) { Stop-Process -Id $proc.Id -Force -ErrorAction SilentlyContinue }
  throw "server ($Tag) never became healthy: $err"
}

Write-QaLog $Phase "=== 07_setup start ==="

# Isolate from any ambient credentials/keychain for the whole phase.
$prevToken = $env:MEDIAGIT_TOKEN
$prevApiKey = $env:MEDIAGIT_API_KEY
$prevNoKeyring = $env:MEDIAGIT_NO_KEYRING
$prevJwt = $env:MEDIAGIT_JWT_SECRET
Remove-Item Env:MEDIAGIT_TOKEN -ErrorAction SilentlyContinue
Remove-Item Env:MEDIAGIT_API_KEY -ErrorAction SilentlyContinue
Remove-Item Env:MEDIAGIT_JWT_SECRET -ErrorAction SilentlyContinue
# S2 exercises the keychain tier on purpose; keep it enabled but sandbox it by
# using a per-run origin so we never touch a real entry, and clear on the way out.
$env:MEDIAGIT_NO_KEYRING = "1"  # default off; S2 re-enables locally where needed

$srv = $null
try {
  $port = Get-QaFreePort
  $baseUrl = "http://127.0.0.1:$port"
  $setupDir = Join-Path $QA.Work "setup-$($QA.RunId)"
  if (Test-Path $setupDir) { Remove-Item -Recurse -Force $setupDir -ErrorAction SilentlyContinue }
  New-Item -ItemType Directory -Path $setupDir -Force | Out-Null
  $cfgOn = Join-Path $setupDir "server-on.toml"
  $reposOn = Join-Path $setupDir "repos-on"

  # ---- S1-wizard-auth-on ----
  $adminPass = "copper-valley-signal-37"
  # AU-15: password passed via environment, not a flag — a command-line
  # argument is visible in `ps` output and persists in shell history.
  $env:MEDIAGIT_ADMIN_PASSWORD = $adminPass
  $initOut = & $QA.MGServer @(
    "init", "--non-interactive", "--enable-auth",
    "--config", $cfgOn, "--data-dir", $reposOn, "--host", "127.0.0.1", "--port", "$port",
    "--admin-username", "qa-owner", "--admin-email", "qa-owner@qa.local"
  ) 2>&1
  $initExit = $LASTEXITCODE
  Remove-Item Env:MEDIAGIT_ADMIN_PASSWORD -ErrorAction SilentlyContinue
  $cfgText = if (Test-Path $cfgOn) { Get-Content $cfgOn -Raw } else { "" }
  $hasSecret = $cfgText -match "jwt_secret"
  $regClosed = $cfgText -match "allow_open_registration = false"
  $rateOn = $cfgText -match "enable_rate_limiting = true"
  # Bootable proof: init a bare repo in repos_dir, then boot the wizard's config.
  $repoName = "proj-$($QA.RunId)"
  $bareRepo = Join-Path $reposOn $repoName
  $bareInit = Invoke-MG $null @("init", "--bare", $bareRepo) $Phase
  $srv = Start-FromConfig $cfgOn $baseUrl "setup-on"
  $repoUrl = "$baseUrl/$repoName"

  # EFFECT, not text. `$regClosed` above only proves the wizard WROTE the line.
  # That is precisely the gate AU-3 walked through: `allow_open_registration`
  # was wired to nothing for months, the endpoint stayed open, and this
  # assertion was green the entire time -- because a written key and an
  # honoured key are indistinguishable from the file. It would have passed
  # identically against a build that deleted the field.
  #
  # So ask the booted server. Anonymous registration against the wizard's own
  # config must be refused.
  $regClosedEffect = $false
  $regClosedCode = 0
  if ($srv) {
    try {
      $rb = @{ username = "qa-intruder"; email = "qa-intruder@qa.local"; password = "copper-valley-signal-77" } | ConvertTo-Json
      Invoke-RestMethod -Method Post -Uri "$baseUrl/auth/register" -ContentType "application/json" -Body $rb | Out-Null
      # A 2xx here means the endpoint is OPEN on a config that says closed.
      $regClosedCode = 200
    } catch {
      try { $regClosedCode = [int]$_.Exception.Response.StatusCode } catch { $regClosedCode = -1 }
    }
    $regClosedEffect = ($regClosedCode -eq 403)
  }

  Rec "S1-wizard-auth-on" (($initExit -eq 0) -and $hasSecret -and $regClosed -and $regClosedEffect -and $rateOn -and ($bareInit.Exit -eq 0) -and [bool]$srv) `
    "init-exit=$initExit jwt_secret=$hasSecret reg-closed-written=$regClosed reg-closed-enforced=$regClosedEffect($regClosedCode want 403) rate-on=$rateOn booted=$([bool]$srv)"

  # ---- S2-login-no-env (the point of the whole cycle) ----
  # No MEDIAGIT_TOKEN, no config token: auth login must store the credential and
  # every following command must resolve it from the keychain by server origin.
  Remove-Item Env:MEDIAGIT_NO_KEYRING -ErrorAction SilentlyContinue
  $loginOk = $false
  # Password prompting can't be scripted non-interactively, so obtain a token
  # over REST, then have the CLIENT store it via `auth login --token` (still the
  # client persisting the credential by origin - no env var, no hand-edited file).
  $loginBody = @{ identifier = "qa-owner"; password = $adminPass } | ConvertTo-Json
  $tok = $null
  try {
    $lr = Invoke-RestMethod -Method Post -Uri "$baseUrl/auth/login" -ContentType "application/json" -Body $loginBody
    $tok = $lr.tokens.access_token
  } catch {}
  if ($tok) {
    $store = Invoke-MG $null @("auth", "login", "--server", $baseUrl, "--token", $tok) $Phase
    $loginOk = ($store.Exit -eq 0)
  }
  # Seed content onto the server repo so there is a 'main' branch to clone
  # (cloning an empty repo is unsupported). The seed push uses the env token;
  # the CLONE under test then runs with NO env, resolving the keychain by origin.
  $seed = New-SandboxRepo "setup-seed-$($QA.RunId)" $Phase
  Set-Content (Join-Path $seed "seed.txt") "seed" -Encoding Ascii
  Invoke-MG $seed @("add", "seed.txt") $Phase | Out-Null
  Invoke-MG $seed @("commit", "-m", "seed") $Phase | Out-Null
  Invoke-MG $seed @("remote", "add", "origin", $repoUrl) $Phase | Out-Null
  $env:MEDIAGIT_TOKEN = $tok
  Invoke-MG $seed @("push", "origin") $Phase -TimeoutSec 600 | Out-Null
  Remove-Item Env:MEDIAGIT_TOKEN -ErrorAction SilentlyContinue
  # Clone with NO env var - credential must come from the keychain by origin.
  $cloneDir = Join-Path $setupDir "clone-owner"
  $clone = Invoke-MG $null @("clone", $repoUrl, $cloneDir) $Phase -TimeoutSec 600
  $cloneOk = ($clone.Exit -eq 0)
  $statusOut = (Invoke-MG $cloneDir @("auth", "status", "--server", $baseUrl) $Phase).Out
  $tierNamed = ($statusOut -match "keychain")
  Rec "S2-login-no-env" ($loginOk -and $cloneOk -and $tierNamed) `
    "login-store=$loginOk clone-no-env=$cloneOk status-names-keychain=$tierNamed"

  # ---- S2c-login-sets-author ----
  # auth login inside a repo records the authenticated identity as the repo's
  # commit author (config [author]) so commits are attributed to the user.
  $cfgFile = Join-Path $cloneDir ".mediagit\config.toml"
  $authorBefore = if (Test-Path $cfgFile) { (Get-Content $cfgFile -Raw) -match '(?ms)\[author\][^\[]*name\s*=\s*"' } else { $false }
  Invoke-MG $cloneDir @("auth", "login", "--server", $baseUrl, "--token", $tok) $Phase | Out-Null
  $cfgAfter = if (Test-Path $cfgFile) { Get-Content $cfgFile -Raw } else { "" }
  $authorSet = ($cfgAfter -match 'name\s*=\s*"qa-owner"') -and ($cfgAfter -match 'qa-owner@qa\.local')
  Rec "S2c-login-sets-author" $authorSet "author-written=$authorSet (before-had-name=$authorBefore)"

  # ---- S3-apikey ----
  $keyCreate = Invoke-MG $cloneDir @("auth", "key", "create", "--name", "ci-key", "--server", $baseUrl) $Phase
  $apiKey = $null
  if ($keyCreate.Exit -eq 0) {
    # The plaintext key is 64 hex chars (printed on its own line); the id is
    # ak_<32 hex>. Match the 64-hex key, not the shorter id.
    $m = [regex]::Match($keyCreate.Out, "\b[0-9a-f]{64}\b")
    if ($m.Success) { $apiKey = $m.Value }
  }
  $apiPushOk = $false
  if ($apiKey) {
    New-Item -ItemType File -Path (Join-Path $cloneDir "ci.txt") -Force | Out-Null
    Set-Content (Join-Path $cloneDir "ci.txt") "ci" -Encoding Ascii
    Invoke-MG $cloneDir @("add", "ci.txt") $Phase | Out-Null
    Invoke-MG $cloneDir @("commit", "-m", "ci commit") $Phase | Out-Null
    $prevTok2 = $env:MEDIAGIT_TOKEN; $prevNk = $env:MEDIAGIT_NO_KEYRING
    Remove-Item Env:MEDIAGIT_TOKEN -ErrorAction SilentlyContinue
    $env:MEDIAGIT_NO_KEYRING = "1"  # force it to use the API key env, not the keychain token
    $env:MEDIAGIT_API_KEY = $apiKey
    $apiPush = Invoke-MG $cloneDir @("push", "origin") $Phase -TimeoutSec 600
    $apiPushOk = ($apiPush.Exit -eq 0)
    Remove-Item Env:MEDIAGIT_API_KEY -ErrorAction SilentlyContinue
    $env:MEDIAGIT_TOKEN = $prevTok2; $env:MEDIAGIT_NO_KEYRING = $prevNk
  }
  Rec "S3-apikey" (($keyCreate.Exit -eq 0) -and [bool]$apiKey -and $apiPushOk) `
    "key-create=$($keyCreate.Exit) key-parsed=$([bool]$apiKey) apikey-push=$apiPushOk"

  # ---- logout invalidation ----
  Invoke-MG $cloneDir @("auth", "logout", "--server", $baseUrl) $Phase | Out-Null
  Remove-Item Env:MEDIAGIT_TOKEN -ErrorAction SilentlyContinue
  Remove-Item Env:MEDIAGIT_API_KEY -ErrorAction SilentlyContinue
  New-Item -ItemType File -Path (Join-Path $cloneDir "after-logout.txt") -Force | Out-Null
  Invoke-MG $cloneDir @("add", "after-logout.txt") $Phase | Out-Null
  Invoke-MG $cloneDir @("commit", "-m", "post logout") $Phase | Out-Null
  # A logged-out push must be REJECTED, and rejected PROMPTLY. "exit -ne 0" alone
  # cannot tell those apart, and 20260825-ga18 is what that costs:
  #
  #   02:09:45 SLOW: mediagit push origin took 600.3s (exit=124) - possible stall
  #   02:09:45 S2b-logout-blocks -> PASS post-logout-push-rejected=True (exit=124)
  #
  # exit=124 is the harness TIMEOUT killing a push that hung for ten minutes
  # against a server that had stopped serving. The drill scored that as a clean
  # rejection. The same wedge in ga15 (07_auth, push 600.0s) went unnoticed for
  # the same reason - a gate that treats "did not succeed" as "was refused"
  # cannot distinguish a 401 from a hang, so the wedge stayed invisible in runs
  # that reported green.
  #
  # A real rejection is fast: the server answers 401/403 before any data moves.
  # 60s is far above a loopback auth round-trip and far below the 600s timeout,
  # so it separates the two without being sensitive to a slow box.
  $swLogout = [Diagnostics.Stopwatch]::StartNew()
  $postLogoutPush = Invoke-MG $cloneDir @("push", "origin") $Phase
  $swLogout.Stop()
  $logoutSecs = [math]::Round($swLogout.Elapsed.TotalSeconds, 1)
  $rejected = ($postLogoutPush.Exit -ne 0)
  $prompt = ($logoutSecs -lt 60)
  Rec "S2b-logout-blocks" ($rejected -and $prompt) `
    ("post-logout-push-rejected=$rejected prompt=$prompt sec=$logoutSecs (exit=$($postLogoutPush.Exit); " +
     "a hang killed by the harness timeout is NOT a rejection - see 20260825-ga18)")

  # ---- S4-jwt-env-override ----
  # Restart the server with a DIFFERENT jwt secret via env; a token minted under
  # the toml secret must now be rejected (env wins, main.rs).
  Stop-QaServer @{ Proc = $srv.Proc }
  $env:MEDIAGIT_JWT_SECRET = "override-secret-ffffffffffffffffffffffffffffffff"
  $srv2 = Start-FromConfig $cfgOn $baseUrl "setup-jwt-override"
  $meCode = $null
  try {
    Invoke-WebRequest -Uri "$baseUrl/auth/me" -Headers @{ Authorization = "Bearer $tok" } -UseBasicParsing -TimeoutSec 10 -ErrorAction Stop | Out-Null
    $meCode = 200
  } catch { $meCode = [int]$_.Exception.Response.StatusCode }
  Rec "S4-jwt-env-override" ($meCode -eq 401) "old-token-rejected-under-env-secret=$($meCode -eq 401) (code=$meCode)"
  Stop-QaServer @{ Proc = $srv2.Proc }
  Remove-Item Env:MEDIAGIT_JWT_SECRET -ErrorAction SilentlyContinue
  $srv = $null

  # ---- S5-wizard-auth-off (§G) ----
  $port2 = Get-QaFreePort
  $baseUrl2 = "http://127.0.0.1:$port2"
  $cfgOff = Join-Path $setupDir "server-off.toml"
  $reposOff = Join-Path $setupDir "repos-off"
  $initOffOut = & $QA.MGServer @(
    "init", "--non-interactive", "--config", $cfgOff, "--data-dir", $reposOff, "--host", "127.0.0.1", "--port", "$port2"
  ) 2>&1
  $initOffExit = $LASTEXITCODE
  $offRepo = "proj-off-$($QA.RunId)"
  $offBare = Join-Path $reposOff $offRepo
  Invoke-MG $null @("init", "--bare", $offBare) $Phase | Out-Null
  $srvOff = Start-FromConfig $cfgOff $baseUrl2 "setup-off"
  # Plain round-trip against an auth-off server.
  $wc = New-SandboxRepo "setup-off-work" $Phase
  Set-Content (Join-Path $wc "a.txt") "hello" -Encoding Ascii
  Invoke-MG $wc @("add", "a.txt") $Phase | Out-Null
  Invoke-MG $wc @("commit", "-m", "off base") $Phase | Out-Null
  Invoke-MG $wc @("remote", "add", "origin", "$baseUrl2/$offRepo") $Phase | Out-Null
  $offPush = Invoke-MG $wc @("push", "origin") $Phase -TimeoutSec 600
  # auth status/login must exit 0 with the disabled message, NOT an error.
  $offStatus = Invoke-MG $wc @("auth", "status", "--server", $baseUrl2) $Phase
  $offLogin = Invoke-MG $null @("auth", "login", "--server", $baseUrl2, "--username", "nobody", "--token", "x") $Phase
  $disabledMsg = ($offStatus.Out -match "authentication disabled") -or ($offStatus.Out -match "disabled")
  Rec "S5-wizard-auth-off" (($initOffExit -eq 0) -and ($offPush.Exit -eq 0) -and ($offStatus.Exit -eq 0) -and $disabledMsg) `
    "init-off-exit=$initOffExit off-push=$($offPush.Exit) status-exit=$($offStatus.Exit) disabled-msg=$disabledMsg"
  Stop-QaServer @{ Proc = $srvOff.Proc }

  # ---- S6-nonloopback-off (wizard refusal) ----
  $cfgBad = Join-Path $setupDir "server-bad.toml"
  $prevInsecure = $env:MEDIAGIT_ALLOW_INSECURE_BIND
  Remove-Item Env:MEDIAGIT_ALLOW_INSECURE_BIND -ErrorAction SilentlyContinue
  $badOut = & $QA.MGServer @(
    "init", "--non-interactive", "--config", $cfgBad, "--data-dir", (Join-Path $setupDir "repos-bad"),
    "--host", "0.0.0.0", "--port", "3999"
  ) 2>&1
  $badExit = $LASTEXITCODE
  $refused = ($badExit -ne 0) -and (-not (Test-Path $cfgBad))
  Rec "S6-nonloopback-off" $refused "init-refused=$refused (exit=$badExit, config-absent=$(-not (Test-Path $cfgBad)))"
  if ($prevInsecure) { $env:MEDIAGIT_ALLOW_INSECURE_BIND = $prevInsecure }

} catch {
  $skip = "$_" -match "^SKIP:"
  $tag = if ($skip) { "SKIP" } else { $false }
  $detail = if ($skip) { "$_" } else { "unexpected error: $_" }
  Rec "07_setup" $tag $detail
} finally {
  if ($srv) { Stop-QaServer @{ Proc = $srv.Proc } }
  # Best-effort: clear any keychain entries this phase created for its origins.
  Invoke-MG $null @("auth", "logout", "--all") $Phase 2>$null | Out-Null
  $env:MEDIAGIT_TOKEN = $prevToken
  $env:MEDIAGIT_API_KEY = $prevApiKey
  $env:MEDIAGIT_NO_KEYRING = $prevNoKeyring
  if ($prevJwt) { $env:MEDIAGIT_JWT_SECRET = $prevJwt } else { Remove-Item Env:MEDIAGIT_JWT_SECRET -ErrorAction SilentlyContinue }
  Remove-Item Env:MEDIAGIT_QA_UNUSED -ErrorAction SilentlyContinue
}

Write-QaLog $Phase "=== 07_setup done: overall=$(if ($script:AllPass) { 'PASS' } else { 'FAIL' }) ==="
# Teardown: reclaim this phase's own work/ scratch so a long campaign cannot run the
# volume out of space. work/ ONLY - logs/ and fixtures-synthetic/ are never touched.
Invoke-QaTeardown $Phase @("setup-*")

Exit-QaPhase $Phase (-not $script:AllPass)
