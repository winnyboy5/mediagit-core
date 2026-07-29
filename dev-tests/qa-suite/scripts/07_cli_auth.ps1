# Phase 7 (cli-auth) - the `mediagit auth *` CLI surface, end to end. ASCII-only, PS 5.1.
#
# 07_auth.ps1 and 07_users.ps1 exercise the server's /auth/* HTTP routes directly. That
# proves the SERVER is correct and proves nothing at all about the CLI a user actually
# types: every auth subcommand builds its own request, parses its own response and
# resolves its own credentials, and none of that code is touched by an HTTP-level drill.
# This phase drives the binary and uses the HTTP surface only as an ORACLE - if the CLI
# claims a password changed, the oracle has to agree that it changed.
#
# Drills:
#   C1-cli-register   auth register (interactive prompts driven over stdin) -> oracle can log in
#   C2-cli-whoami     auth whoami reports the right username/role; bad credential is rejected
#   C3-cli-passwd     auth passwd -> old password stops working, new one starts
#   C4-cli-key        auth key create/list/revoke -> the REVOKED key is refused afterwards
#   C5-cli-admin      auth admin create-user / list-users / grant / revoke-grant /
#                     set-role / reset-password
#
# Mechanics that matter here:
#   - Every subcommand takes --server, so no repo remote is needed.
#   - register/passwd/create-user/reset-password prompt via dialoguer; Invoke-MG -StdIn
#     feeds one line per prompt (password prompts ask twice: value + confirmation).
#   - MEDIAGIT_TOKEN is read ahead of the OS keychain (crates/mediagit-cli/src/repo.rs),
#     and MEDIAGIT_NO_KEYRING=1 keeps the dev box's real keychain out of the drill.
#
# Output: $QA.Logs\cli_auth_results.tsv (drill, pass, detail)

. (Join-Path $PSScriptRoot "lib\common.ps1")
. (Join-Path $PSScriptRoot "lib\remote.ps1")

$Phase = "07_cli_auth"
$env:MEDIAGIT_AUTHOR_NAME = "QA-Suite"
$env:MEDIAGIT_AUTHOR_EMAIL = "qa-suite@mediagit.local"

$TSV = Join-Path $QA.Logs "cli_auth_results.tsv"
$script:AllPass = $true

function Rec([string]$Drill, $Pass, [string]$Detail) {
  Write-QaRow $TSV @("drill", "pass", "detail") @($Drill, $Pass, $Detail)
  $tag = if ("$Pass" -eq "SKIP") { "SKIP" } elseif ($Pass) { "PASS" } else { "FAIL" }
  Write-QaLog $Phase ("{0} -> {1}  {2}" -f $Drill, $tag, $Detail)
  Write-QaGate $Phase $Drill $Pass $Detail
  if ($tag -eq "FAIL") { $script:AllPass = $false }
}

# Run `mediagit auth ...` with no repo context. $StdIn drives dialoguer prompts.
function Invoke-Auth([string[]]$AuthArgs, [string[]]$StdIn = $null, [string]$Token = $null) {
  $prev = $env:MEDIAGIT_TOKEN
  $prevKey = $env:MEDIAGIT_API_KEY
  if ($Token) { $env:MEDIAGIT_TOKEN = $Token } else { Remove-Item Env:MEDIAGIT_TOKEN -ErrorAction SilentlyContinue }
  try {
    # 60s: every one of these is a single request to a loopback server, so anything
    # slower is a hang worth failing on rather than waiting out.
    return Invoke-MG $null (@("auth") + $AuthArgs) $Phase -TimeoutSec 60 -StdIn $StdIn
  } finally {
    $env:MEDIAGIT_TOKEN = $prev
    $env:MEDIAGIT_API_KEY = $prevKey
  }
}

# HTTP oracle: can this identity log in with this password?
function Test-OracleLogin([string]$Base, [string]$Identifier, [string]$Password) {
  try {
    $body = @{ identifier = $Identifier; password = $Password } | ConvertTo-Json
    $r = Invoke-RestMethod -Method Post -Uri "$Base/auth/login" -ContentType "application/json" -Body $body
    return @{ Ok = $true; Token = $r.tokens.access_token; Role = ("" + $r.user.role) }
  } catch { return @{ Ok = $false; Token = $null; Role = "" } }
}

Write-QaLog $Phase "=== 07_cli_auth start ==="

$prevToken = $env:MEDIAGIT_TOKEN
$prevApiKey = $env:MEDIAGIT_API_KEY
$prevNoKeyring = $env:MEDIAGIT_NO_KEYRING
$env:MEDIAGIT_NO_KEYRING = "1"
Remove-Item Env:MEDIAGIT_TOKEN -ErrorAction SilentlyContinue
Remove-Item Env:MEDIAGIT_API_KEY -ErrorAction SilentlyContinue

$ADMIN = "cliadmin"
$ADMIN_PW = "amber-tunnel-drift-21"
$USER = "cliuser"
$USER_PW = "amber-tunnel-drift-22"
$USER_PW2 = "amber-tunnel-drift-23"

$srv = $null
$done = @{}
try {
  # local backend: this phase tests the auth CLI, not object storage.
  $srv = Start-QaServer -Backend "local" -Phase $Phase -EnableAuth -AdminUser $ADMIN -AdminPass $ADMIN_PW
  $base = $srv.BaseUrl
  $adminLogin = Test-OracleLogin $base $ADMIN $ADMIN_PW
  $adminTok = $adminLogin.Token
  if (-not $adminTok) { throw "bootstrap admin '$ADMIN' cannot log in - server auth store not usable" }

  # ---- C1-cli-register ----
  # prompts: Username, Email, Password, Confirm password
  $reg = Invoke-Auth @("register", "--server", $base) @($USER, "$USER@qa.local", $USER_PW, $USER_PW)
  $regExit0 = ($reg.Exit -eq 0)
  $oracle = Test-OracleLogin $base $USER $USER_PW
  $userTok = $oracle.Token
  Rec "C1-cli-register" ($regExit0 -and $oracle.Ok) `
    "cli-exit=$($reg.Exit) oracle-login=$($oracle.Ok) role=$($oracle.Role)"
  $done["C1"] = $true

  # ---- C2-cli-whoami ----
  $who = Invoke-Auth @("whoami", "--server", $base) -Token $userTok
  $whoOk = ($who.Exit -eq 0) -and ($who.Out -match [regex]::Escape($USER))
  # A garbage bearer token must be REJECTED, not silently treated as anonymous: an
  # auth CLI that exits 0 on a bad credential is the worst possible failure mode.
  $whoBad = Invoke-Auth @("whoami", "--server", $base) -Token "not-a-real-token"
  $badRejected = ($whoBad.Exit -ne 0)
  Rec "C2-cli-whoami" ($whoOk -and $badRejected) `
    "whoami-exit=$($who.Exit) username-shown=$whoOk bad-token-rejected=$badRejected (exit=$($whoBad.Exit))"
  $done["C2"] = $true

  # ---- C3-cli-passwd ----
  # prompts: Current password, New password, Confirm new password
  $pw = Invoke-Auth @("passwd", "--server", $base) @($USER_PW, $USER_PW2, $USER_PW2) -Token $userTok
  $oldGone = -not (Test-OracleLogin $base $USER $USER_PW).Ok
  $newWorks = Test-OracleLogin $base $USER $USER_PW2
  Rec "C3-cli-passwd" (($pw.Exit -eq 0) -and $oldGone -and $newWorks.Ok) `
    "cli-exit=$($pw.Exit) old-password-rejected=$oldGone new-password-accepted=$($newWorks.Ok)"
  if ($newWorks.Ok) { $userTok = $newWorks.Token }
  $done["C3"] = $true

  # ---- C4-cli-key ----
  # create prints "Created key '<name>' (id <uuid>)" then the plaintext key on its own line.
  $kc = Invoke-Auth @("key", "create", "--server", $base, "--name", "cli-ci") -Token $userTok
  $keyId = ""
  if ($kc.Out -match "\(id\s+([A-Za-z0-9_-]+)\)") { $keyId = $Matches[1] }
  $keyPlain = ""
  # the plaintext key is the first bare token that is neither the id nor decorated output
  foreach ($line in ($kc.Out -split "`r?`n")) {
    $t = $line.Trim()
    if ($t -and $t -notmatch '\s' -and $t -ne $keyId -and $t.Length -ge 16) { $keyPlain = $t; break }
  }
  $kl = Invoke-Auth @("key", "list", "--server", $base) -Token $userTok
  $listed = [bool]$keyId -and ($kl.Out -match [regex]::Escape($keyId))

  # The key must WORK before revocation, or "rejected after revoke" proves nothing.
  $env:MEDIAGIT_API_KEY = $keyPlain
  $beforeRevoke = Invoke-MG $null @("auth", "whoami", "--server", $base) $Phase -TimeoutSec 60
  $keyWorked = ($beforeRevoke.Exit -eq 0)
  Remove-Item Env:MEDIAGIT_API_KEY -ErrorAction SilentlyContinue

  $kr = if ($keyId) { Invoke-Auth @("key", "revoke", "--server", $base, $keyId) -Token $userTok } else { @{ Exit = 1; Out = "no key id parsed" } }

  # ...and must be refused on the next authenticated op once revoked.
  $env:MEDIAGIT_API_KEY = $keyPlain
  $afterRevoke = Invoke-MG $null @("auth", "whoami", "--server", $base) $Phase -TimeoutSec 60
  $revokedRejected = ($afterRevoke.Exit -ne 0)
  Remove-Item Env:MEDIAGIT_API_KEY -ErrorAction SilentlyContinue

  Rec "C4-cli-key" (($kc.Exit -eq 0) -and [bool]$keyPlain -and $listed -and $keyWorked -and ($kr.Exit -eq 0) -and $revokedRejected) `
    ("create-exit=$($kc.Exit) id-parsed=$([bool]$keyId) plaintext-shown=$([bool]$keyPlain) listed=$listed " +
     "worked-before-revoke=$keyWorked revoke-exit=$($kr.Exit) rejected-after-revoke=$revokedRejected (exit=$($afterRevoke.Exit))")
  $done["C4"] = $true

  # ---- C5-cli-admin ----
  # create-user prompts: Email, Password, Confirm password
  $NEWU = "clinewbie"
  $NEWU_PW = "amber-tunnel-drift-24"
  $NEWU_PW2 = "amber-tunnel-drift-25"
  $cu = Invoke-Auth @("admin", "create-user", "--server", $base, $NEWU, "--role", "read") `
    @("$NEWU@qa.local", $NEWU_PW, $NEWU_PW) -Token $adminTok
  $newLogin = Test-OracleLogin $base $NEWU $NEWU_PW
  $createdOk = ($cu.Exit -eq 0) -and $newLogin.Ok

  $lu = Invoke-Auth @("admin", "list-users", "--server", $base) -Token $adminTok
  $listedUsers = ($lu.Exit -eq 0) -and ($lu.Out -match [regex]::Escape($NEWU)) -and ($lu.Out -match [regex]::Escape($USER))

  # A non-admin must NOT be able to reach the admin subcommands.
  $luDenied = Invoke-Auth @("admin", "list-users", "--server", $base) -Token $userTok
  $nonAdminRejected = ($luDenied.Exit -ne 0)

  $repo = $srv.RepoName
  $gr = Invoke-Auth @("admin", "grant", "--server", $base, $NEWU, $repo, "read") -Token $adminTok
  # whoami as the granted user must now show the grant - this is the read-back that
  # proves the grant landed, rather than trusting the command's own exit code.
  $whoGrant = Invoke-Auth @("whoami", "--server", $base) -Token $newLogin.Token
  $grantVisible = ($whoGrant.Out -match [regex]::Escape($repo))

  $rg = Invoke-Auth @("admin", "revoke-grant", "--server", $base, $NEWU, $repo) -Token $adminTok
  $whoNoGrant = Invoke-Auth @("whoami", "--server", $base) -Token $newLogin.Token
  $grantGone = -not ($whoNoGrant.Out -match [regex]::Escape($repo))

  $sr = Invoke-Auth @("admin", "set-role", "--server", $base, $NEWU, "write") -Token $adminTok
  $roleAfter = (Test-OracleLogin $base $NEWU $NEWU_PW).Role
  $roleChanged = ($roleAfter -eq "Write")

  # reset-password prompts: New password, Confirm new password
  $rp = Invoke-Auth @("admin", "reset-password", "--server", $base, $NEWU) @($NEWU_PW2, $NEWU_PW2) -Token $adminTok
  $resetOk = ($rp.Exit -eq 0) -and (Test-OracleLogin $base $NEWU $NEWU_PW2).Ok -and
             (-not (Test-OracleLogin $base $NEWU $NEWU_PW).Ok)

  Rec "C5-cli-admin" ($createdOk -and $listedUsers -and $nonAdminRejected -and ($gr.Exit -eq 0) -and $grantVisible -and
                      ($rg.Exit -eq 0) -and $grantGone -and ($sr.Exit -eq 0) -and $roleChanged -and $resetOk) `
    ("create-user=$createdOk list-users=$listedUsers non-admin-rejected=$nonAdminRejected " +
     "grant=$($gr.Exit)/visible=$grantVisible revoke-grant=$($rg.Exit)/gone=$grantGone " +
     "set-role=$($sr.Exit)/role=$roleAfter reset-password=$resetOk")
  $done["C5"] = $true
} catch {
  $skip = "$_" -match "^SKIP:"
  $tag = if ($skip) { "SKIP" } else { $false }
  $detail = if ($skip) { "$_" } else { "unexpected error: $_" }
  foreach ($d in @("C1-cli-register", "C2-cli-whoami", "C3-cli-passwd", "C4-cli-key", "C5-cli-admin")) {
    if (-not $done[($d -split "-")[0]]) { Rec $d $tag $detail }
  }
} finally {
  Stop-QaServer $srv
  $env:MEDIAGIT_TOKEN = $prevToken
  $env:MEDIAGIT_API_KEY = $prevApiKey
  $env:MEDIAGIT_NO_KEYRING = $prevNoKeyring
}

Invoke-QaTeardown $Phase @("cliauth-*")

Exit-QaPhase $Phase (-not $script:AllPass)
