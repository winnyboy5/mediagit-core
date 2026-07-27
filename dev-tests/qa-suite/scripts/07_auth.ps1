# Phase 7 (auth) - auth-enabled server e2e. ASCII-only, PS 5.1 compatible.
# Covers the auth surface the rest of the suite deliberately runs WITHOUT
# (every other phase starts its servers auth-off):
#   A11-auth-rest    401 without token; register/login/me roundtrip
#   A11-auth-push    push with Bearer token succeeds; push with no creds fails
#   A11-auth-grants  admin promotion (offline `mediagit-server admin promote`),
#                    admin-route gating (bob 403), per-repo grants: read-grant
#                    clone ok / push rejected; write-grant push still ok
#
# Ground truth:
#   - POST /auth/register {username,email,password} -> 201 {user{id,role},tokens{access_token}};
#     every registered user gets Role::Write (crates/mediagit-security/src/auth/handlers.rs).
#   - The first Admin is minted out-of-band via `mediagit-server admin promote`
#     (--force to mutate a live server's store); the auth store loads at boot, so
#     a Restart is still needed to pick up the change.
#   - check_permission (crates/mediagit-server/src/handlers/mod.rs): admin always
#     allowed; zero grants -> flat role check; any grants -> per-repo levels
#     Read < Write < Admin.
#   - Client sends MEDIAGIT_TOKEN as Bearer; MEDIAGIT_NO_KEYRING=1 keeps the dev
#     box's OS keychain from satisfying auth behind the drill's back.
#
# Output: $QA.Logs\auth_results.tsv (drill, pass, detail)

. (Join-Path $PSScriptRoot "lib\common.ps1")
. (Join-Path $PSScriptRoot "lib\remote.ps1")

$Phase = "07_auth"
$env:MEDIAGIT_AUTHOR_NAME = "QA-Suite"
$env:MEDIAGIT_AUTHOR_EMAIL = "qa-suite@mediagit.local"

$TSV = Join-Path $QA.Logs "auth_results.tsv"
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

# Run a web call, return the HTTP status code (PS 5.1 throws on non-2xx).
function Get-HttpStatus([scriptblock]$Call) {
  try { & $Call | Out-Null; return 200 } catch {
    $resp = $_.Exception.Response
    if ($resp) { return [int]$resp.StatusCode } else { return -1 }
  }
}

Write-QaLog $Phase "=== 07_auth start ==="

$prevToken = $env:MEDIAGIT_TOKEN
$prevApiKey = $env:MEDIAGIT_API_KEY
$prevNoKeyring = $env:MEDIAGIT_NO_KEYRING
$env:MEDIAGIT_NO_KEYRING = "1"
Remove-Item Env:MEDIAGIT_TOKEN -ErrorAction SilentlyContinue
Remove-Item Env:MEDIAGIT_API_KEY -ErrorAction SilentlyContinue

$srv = $null
$restDone = $false
$pushDone = $false
try {
  $srv = Start-QaServer -Backend "minio" -Phase $Phase -EnableAuth
  $base = $srv.BaseUrl

  # ---- A11-auth-rest ----
  $unauthCode = Get-HttpStatus {
    Invoke-WebRequest -Uri "$($srv.Url)/info/refs" -UseBasicParsing -TimeoutSec 10 -ErrorAction Stop
  }
  $unauthRejected = ($unauthCode -eq 401)

  $users = @{}
  foreach ($u in @("qa-admin", "alice", "bob")) {
    $body = @{ username = $u; email = "$u@qa.local"; password = "pw-$u-123456" } | ConvertTo-Json
    $resp = Invoke-RestMethod -Method Post -Uri "$base/auth/register" -ContentType "application/json" -Body $body
    $users[$u] = @{ Id = $resp.user.id; Token = $resp.tokens.access_token }
  }
  $registered = ($users.Count -eq 3) -and [bool]$users["alice"].Token -and [bool]$users["alice"].Id

  $loginBody = @{ identifier = "alice@qa.local"; password = "pw-alice-123456" } | ConvertTo-Json
  $loginResp = Invoke-RestMethod -Method Post -Uri "$base/auth/login" -ContentType "application/json" -Body $loginBody
  $loginOk = [bool]$loginResp.tokens.access_token
  if ($loginOk) { $users["alice"].Token = $loginResp.tokens.access_token }

  $me = Invoke-RestMethod -Uri "$base/auth/me" -Headers @{ Authorization = "Bearer " + $users["alice"].Token }
  $meOk = ($me.username -eq "alice")

  Rec "A11-auth-rest" ($unauthRejected -and $registered -and $loginOk -and $meOk) `
    "unauth-401=$unauthRejected (code=$unauthCode) register-x3=$registered login=$loginOk me=$meOk"
  $restDone = $true

  # ---- A11-auth-push ----
  $seed = New-SandboxRepo "a11-seed" $Phase
  New-QaBinaryFixture (Join-Path $seed "asset.bin") 4 82001
  Invoke-MG $seed @("add", ".") $Phase | Out-Null
  Invoke-MG $seed @("commit", "-m", "base") $Phase | Out-Null
  Invoke-MG $seed @("remote", "add", "origin", $srv.Url) $Phase | Out-Null

  $noCredPush = Invoke-MG $seed @("push", "origin") $Phase
  $noCredRejected = ($noCredPush.Exit -ne 0)

  $env:MEDIAGIT_TOKEN = $users["alice"].Token
  $authPush = Invoke-MG $seed @("push", "origin") $Phase -TimeoutSec 1200
  $authPushOk = ($authPush.Exit -eq 0)

  Rec "A11-auth-push" ($noCredRejected -and $authPushOk) `
    "no-cred-push-rejected=$noCredRejected (exit=$($noCredPush.Exit)) bearer-push=$authPushOk (exit=$($authPush.Exit))"
  $pushDone = $true

  # ---- A11-auth-grants ----
  # Promote qa-admin via the offline server CLI (`mediagit-server admin promote`).
  # register can only mint Role::Write; the supported bootstrap is the offline
  # subcommand, not a hand-edit of users.jsonl. --force is required because the
  # server is live (the CLI refuses to mutate a running server's store otherwise);
  # Restart reloads the auth store (it loads at boot only).
  $promoteRun = & $QA.MGServer @("admin", "--config", $srv.ConfigPath, "--force", "promote", "qa-admin") 2>&1
  $promoteOk = ($LASTEXITCODE -eq 0)
  Write-QaLog $Phase "admin promote qa-admin -> exit=$LASTEXITCODE $promoteRun"
  Restart-QaServer $srv $Phase

  $adminBody = @{ identifier = "qa-admin@qa.local"; password = "pw-qa-admin-123456" } | ConvertTo-Json
  $adminLogin = Invoke-RestMethod -Method Post -Uri "$base/auth/login" -ContentType "application/json" -Body $adminBody
  $adminTok = $adminLogin.tokens.access_token
  $adminIsAdmin = ("" + $adminLogin.user.role -eq "Admin")

  $userList = Invoke-RestMethod -Uri "$base/auth/users" -Headers @{ Authorization = "Bearer $adminTok" }
  $adminListOk = (@($userList).Count -ge 3)

  $bobAdminCode = Get-HttpStatus {
    Invoke-WebRequest -Uri "$base/auth/users" -Headers @{ Authorization = "Bearer " + $users["bob"].Token } `
      -UseBasicParsing -TimeoutSec 10 -ErrorAction Stop
  }
  $bobAdminRejected = ($bobAdminCode -eq 403)

  foreach ($g in @(@{ U = "alice"; L = "write" }, @{ U = "bob"; L = "read" })) {
    $gUri = "$base/auth/users/" + $users[$g.U].Id + "/grants"
    $gBody = @{ repo = $srv.RepoName; level = $g.L } | ConvertTo-Json
    Invoke-RestMethod -Method Post -Uri $gUri -ContentType "application/json" `
      -Headers @{ Authorization = "Bearer $adminTok" } -Body $gBody | Out-Null
  }

  # bob (read grant): clone allowed, push rejected
  $bobDir = Join-Path $QA.Work "a11-bob"
  if (Test-Path $bobDir) { Remove-Item -Recurse -Force $bobDir }
  $env:MEDIAGIT_TOKEN = $users["bob"].Token
  $bobClone = Invoke-MG $null @("clone", $srv.Url, $bobDir) $Phase -TimeoutSec 1200
  $bobCloneOk = ($bobClone.Exit -eq 0)

  New-QaBinaryFixture (Join-Path $bobDir "asset.bin") 4 82002
  Invoke-MG $bobDir @("add", "asset.bin") $Phase | Out-Null
  Invoke-MG $bobDir @("commit", "-m", "bob edit") $Phase | Out-Null
  $bobPush = Invoke-MG $bobDir @("push", "origin") $Phase
  $bobPushRejected = ($bobPush.Exit -ne 0)

  # alice (write grant): push still succeeds with grants enforced
  $env:MEDIAGIT_TOKEN = $users["alice"].Token
  New-QaBinaryFixture (Join-Path $seed "asset2.bin") 2 82003
  Invoke-MG $seed @("add", "asset2.bin") $Phase | Out-Null
  Invoke-MG $seed @("commit", "-m", "alice again") $Phase | Out-Null
  $alicePush2 = Invoke-MG $seed @("push", "origin") $Phase -TimeoutSec 1200
  $alicePush2Ok = ($alicePush2.Exit -eq 0)

  Rec "A11-auth-grants" ($promoteOk -and $adminIsAdmin -and $adminListOk -and $bobAdminRejected -and $bobCloneOk -and $bobPushRejected -and $alicePush2Ok) `
    ("promote-cli=$promoteOk admin-role=$adminIsAdmin admin-list=$adminListOk bob-admin-403=$bobAdminRejected (code=$bobAdminCode) " +
     "read-clone=$bobCloneOk read-push-rejected=$bobPushRejected (exit=$($bobPush.Exit)) write-push=$alicePush2Ok")
} catch {
  $skip = "$_" -match "^SKIP:"
  $tag = if ($skip) { "SKIP" } else { $false }
  $detail = if ($skip) { "$_" } else { "unexpected error: $_" }
  if (-not $restDone) { Rec "A11-auth-rest" $tag $detail }
  if (-not $pushDone) { Rec "A11-auth-push" $tag $detail }
  Rec "A11-auth-grants" $tag $detail
} finally {
  Stop-QaServer $srv
  $env:MEDIAGIT_TOKEN = $prevToken
  $env:MEDIAGIT_API_KEY = $prevApiKey
  $env:MEDIAGIT_NO_KEYRING = $prevNoKeyring
}

Write-QaLog $Phase "=== 07_auth done: overall=$(if ($script:AllPass) { 'PASS' } else { 'FAIL' }) ==="
# Teardown: reclaim this phase's own work/ scratch so a long campaign cannot run the
# volume out of space. work/ ONLY - logs/ and fixtures-synthetic/ are never touched.
Invoke-QaTeardown $Phase @("a11-*")

Exit-QaPhase $Phase (-not $script:AllPass)
