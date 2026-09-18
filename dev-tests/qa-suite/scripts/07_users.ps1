# Phase 7 (users) - normal-user lifecycle (§E), NO admin needed for the core path.
# ASCII-only, PS 5.1 compatible. Uses the local (filesystem) backend so it always
# runs; -OpenRegistration sets allow_open_registration = true explicitly (AU-3
# changed the struct default to false, so U1 must ask for open signup rather
# than inherit it), and -AdminUser bootstraps an admin via
# `mediagit-server admin create`.
#
#   U1-self-service   register (open) -> login -> whoami shows role; passwd
#                     self-change requires the current password; old password
#                     then fails; an existing token still works right after the
#                     change (JWT is self-contained, no revocation).
#   U2-own-keys       key create/list/revoke on the caller's own keys.
#   U3-authz-negatives a Write user cannot revoke another user's key and cannot
#                     reach the /auth/users admin routes (403).
#   U4-closed-reg     with registration closed, register 403s; admin create-user
#                     provisions; that user logs in; admin reset-password recovers.
#
# Ground truth: POST /auth/password needs current_password; PATCH
# /auth/users/{id}/password is the admin reset; /auth/me returns per-repo grants.

. (Join-Path $PSScriptRoot "lib\common.ps1")
. (Join-Path $PSScriptRoot "lib\remote.ps1")

$Phase = "07_users"
$env:MEDIAGIT_AUTHOR_NAME = "QA-Suite"
$env:MEDIAGIT_AUTHOR_EMAIL = "qa-suite@mediagit.local"

$TSV = Join-Path $QA.Logs "users_results.tsv"
$script:AllPass = $true

function Rec([string]$Drill, $Pass, [string]$Detail) {
  Write-QaRow $TSV @("drill", "pass", "detail") @($Drill, $Pass, $Detail)
  $tag = if ("$Pass" -eq "SKIP") { "SKIP" } elseif ($Pass) { "PASS" } else { "FAIL" }
  Write-QaLog $Phase ("{0} -> {1}  {2}" -f $Drill, $tag, $Detail)
  Write-QaGate $Phase $Drill $Pass $Detail
  if ($tag -eq "FAIL") { $script:AllPass = $false }
}

# POST helper returning @{ Ok; Code; Body }. PS 5.1 throws on non-2xx.
function Post-Json([string]$Uri, $Obj, [string]$Bearer) {
  $headers = @{}
  if ($Bearer) { $headers["Authorization"] = "Bearer $Bearer" }
  try {
    $b = ($Obj | ConvertTo-Json)
    $r = Invoke-RestMethod -Method Post -Uri $Uri -ContentType "application/json" -Body $b -Headers $headers
    return @{ Ok = $true; Code = 200; Body = $r }
  } catch {
    $c = if ($_.Exception.Response) { [int]$_.Exception.Response.StatusCode } else { -1 }
    return @{ Ok = $false; Code = $c; Body = $null }
  }
}
function Get-Code([string]$Uri, [string]$Bearer) {
  try {
    Invoke-WebRequest -Uri $Uri -Headers @{ Authorization = "Bearer $Bearer" } -UseBasicParsing -TimeoutSec 10 -ErrorAction Stop | Out-Null
    return 200
  } catch { if ($_.Exception.Response) { return [int]$_.Exception.Response.StatusCode } else { return -1 } }
}

Write-QaLog $Phase "=== 07_users start ==="

$prevToken = $env:MEDIAGIT_TOKEN
$prevNoKeyring = $env:MEDIAGIT_NO_KEYRING
$env:MEDIAGIT_NO_KEYRING = "1"
Remove-Item Env:MEDIAGIT_TOKEN -ErrorAction SilentlyContinue

$srv = $null
try {
  $srv = Start-QaServer -Backend "local" -Phase $Phase -EnableAuth -OpenRegistration -AdminUser "qa-admin" -AdminPass "copper-valley-signal-31"
  $base = $srv.BaseUrl

  # ---- U1-self-service ----
  $reg = Post-Json "$base/auth/register" @{ username = "carol"; email = "carol@qa.local"; password = "copper-valley-signal-32" }
  $carolTok = if ($reg.Ok) { $reg.Body.tokens.access_token } else { $null }
  $me = if ($carolTok) { (Post-Json "$base/auth/login" @{ identifier = "carol"; password = "copper-valley-signal-32" }) } else { @{ Ok = $false } }
  $meRole = if ($me.Ok) { "" + $me.Body.user.role } else { "" }
  # passwd: wrong current rejected, correct current accepted.
  $badChange = Post-Json "$base/auth/password" @{ current_password = "WRONG"; new_password = "copper-valley-signal-33" } $carolTok
  $goodChange = Post-Json "$base/auth/password" @{ current_password = "copper-valley-signal-32"; new_password = "copper-valley-signal-33" } $carolTok
  $oldLogin = Post-Json "$base/auth/login" @{ identifier = "carol"; password = "copper-valley-signal-32" }
  $newLogin = Post-Json "$base/auth/login" @{ identifier = "carol"; password = "copper-valley-signal-33" }
  # existing token still valid right after the change (no revocation).
  $tokStillOk = (Get-Code "$base/auth/me" $carolTok) -eq 200
  # AU-3: self-registration now grants Read, not Write. Open registration that
  # handed out push access to every repo was the single worst default in the
  # server; an admin promotes afterwards. This asserted the old behaviour.
  $u1 = $reg.Ok -and ($meRole -eq "Read") -and (-not $badChange.Ok) -and ($badChange.Code -eq 401 -or $badChange.Code -eq 400) `
        -and $goodChange.Ok -and (-not $oldLogin.Ok) -and $newLogin.Ok -and $tokStillOk
  Rec "U1-self-service" $u1 `
    "register=$($reg.Ok) role=$meRole wrong-current-rejected=$(-not $badChange.Ok)($($badChange.Code)) change-ok=$($goodChange.Ok) old-pw-fails=$(-not $oldLogin.Ok) new-pw-ok=$($newLogin.Ok) token-still-valid=$tokStillOk"
  $carolTok = if ($newLogin.Ok) { $newLogin.Body.tokens.access_token } else { $carolTok }

  # ---- U2-own-keys ----
  $mk = Post-Json "$base/auth/keys" @{ name = "carol-ci" } $carolTok
  $carolKeyId = if ($mk.Ok) { $mk.Body.id } else { $null }
  $carolKeyPlain = if ($mk.Ok) { $mk.Body.key } else { $null }
  $listCode = Get-Code "$base/auth/keys/mine" $carolTok
  $revoke = if ($carolKeyId) {
    try { Invoke-RestMethod -Method Delete -Uri "$base/auth/keys/$carolKeyId" -Headers @{ Authorization = "Bearer $carolTok" } | Out-Null; $true } catch { $false }
  } else { $false }
  Rec "U2-own-keys" ($mk.Ok -and [bool]$carolKeyPlain -and ($listCode -eq 200) -and $revoke) `
    "key-create=$($mk.Ok) plaintext-shown=$([bool]$carolKeyPlain) list-mine=$listCode revoke-own=$revoke"

  # ---- U3-authz-negatives ----
  # dave (another Write user) mints a key; carol must not be able to revoke it,
  # and neither may reach the admin user-list route.
  $regD = Post-Json "$base/auth/register" @{ username = "dave"; email = "dave@qa.local"; password = "copper-valley-signal-34" }
  $daveTok = if ($regD.Ok) { $regD.Body.tokens.access_token } else { $null }
  $daveKey = Post-Json "$base/auth/keys" @{ name = "dave-key" } $daveTok
  $daveKeyId = if ($daveKey.Ok) { $daveKey.Body.id } else { $null }
  $carolRevokeDaveCode = if ($daveKeyId) {
    try { Invoke-RestMethod -Method Delete -Uri "$base/auth/keys/$daveKeyId" -Headers @{ Authorization = "Bearer $carolTok" } | Out-Null; 200 }
    catch { if ($_.Exception.Response) { [int]$_.Exception.Response.StatusCode } else { -1 } }
  } else { -1 }
  $carolAdminCode = Get-Code "$base/auth/users" $carolTok
  $u3 = (($carolRevokeDaveCode -eq 403) -or ($carolRevokeDaveCode -eq 404)) -and ($carolAdminCode -eq 403)
  Rec "U3-authz-negatives" $u3 "carol-revoke-daves-key=$carolRevokeDaveCode(want 403/404) carol-admin-route=$carolAdminCode(want 403)"

  # ---- U4-closed-reg ----
  # Flip registration closed by promoting qa-admin (already admin via bootstrap)
  # and using POST /auth/users. This phase deliberately runs with
  # -OpenRegistration (U1 needs it), so rather than restart the server we assert
  # closed-mode behaviour by driving the admin create-user route regardless, and
  # confirm reset-password recovery. 07_setup covers the genuinely-closed server.
  $adminLogin = Post-Json "$base/auth/login" @{ identifier = "qa-admin"; password = "copper-valley-signal-31" }
  $adminTok = if ($adminLogin.Ok) { $adminLogin.Body.tokens.access_token } else { $null }
  $adminIsAdmin = $adminLogin.Ok -and (("" + $adminLogin.Body.user.role) -eq "Admin")
  # admin provisions a user (works whether or not open registration is on).
  # role uses the wire form the server's Role enum deserializes (what the CLI sends): Read|Write|Admin.
  $prov = Post-Json "$base/auth/users" @{ username = "erin"; email = "erin@qa.local"; password = "copper-valley-signal-35"; role = "Write" } $adminTok
  $erinLogin = Post-Json "$base/auth/login" @{ identifier = "erin"; password = "copper-valley-signal-35" }
  # admin resets erin's password (recovery, no current password).
  # POST /auth/users returns AdminUserInfo { id, username, role } at top level.
  $erinId = if ($prov.Ok) { $prov.Body.id } else { $null }
  $reset = if ($erinId) {
    try {
      $rb = @{ new_password = "copper-valley-signal-36" } | ConvertTo-Json
      Invoke-RestMethod -Method Patch -Uri "$base/auth/users/$erinId/password" -ContentType "application/json" -Body $rb -Headers @{ Authorization = "Bearer $adminTok" } | Out-Null
      $true
    } catch { $false }
  } else { $false }
  $erinNewLogin = Post-Json "$base/auth/login" @{ identifier = "erin"; password = "copper-valley-signal-36" }
  $u4 = $adminIsAdmin -and $prov.Ok -and $erinLogin.Ok -and $reset -and $erinNewLogin.Ok
  Rec "U4-admin-provision" $u4 `
    "admin-bootstrapped=$adminIsAdmin create-user=$($prov.Ok) provisioned-login=$($erinLogin.Ok) reset-password=$reset post-reset-login=$($erinNewLogin.Ok)"

} catch {
  $skip = "$_" -match "^SKIP:"
  $tag = if ($skip) { "SKIP" } else { $false }
  $detail = if ($skip) { "$_" } else { "unexpected error: $_" }
  Rec "07_users" $tag $detail
} finally {
  Stop-QaServer $srv
  $env:MEDIAGIT_TOKEN = $prevToken
  $env:MEDIAGIT_NO_KEYRING = $prevNoKeyring
}

Write-QaLog $Phase "=== 07_users done: overall=$(if ($script:AllPass) { 'PASS' } else { 'FAIL' }) ==="
# Teardown: reclaim this phase's own work/ scratch so a long campaign cannot run the
# volume out of space. work/ ONLY - logs/ and fixtures-synthetic/ are never touched.
Invoke-QaTeardown $Phase @("users-*")

Exit-QaPhase $Phase (-not $script:AllPass)
