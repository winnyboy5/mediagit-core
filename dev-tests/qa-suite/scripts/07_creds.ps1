# Phase 7 (creds) - client credential precedence (§F), the reported bug.
# ASCII-only, PS 5.1. Runs with the OS keychain LIVE (unlike every other phase,
# which sets MEDIAGIT_NO_KEYRING=1) because the keychain-vs-config interaction is
# exactly what's under test. Saves/restores any pre-existing entry for the test
# origin so a dev box's real credentials are never disturbed.
#
#   C1-config-beats-keychain  a stale/bad token cached in the keychain must NOT
#                             shadow an explicit remotes.<n>.token in config -
#                             the exact failure the user reported. After the §F
#                             reorder (env -> config -> keychain), the good config
#                             token wins and push succeeds.
#   C2-status-tier            `auth status` names the tier that answered.
#
# Deeper keychain mechanics (401-invalidation, legacy full-URL -> origin key
# migration) are covered by unit tests in
# crates/mediagit-cli/tests/credentials_resolution_test.rs (opt-in real-keyring
# cases) - they need direct keychain seeding under the legacy key that only the
# pre-upgrade binary wrote, which is not reproducible from a shell here.

. (Join-Path $PSScriptRoot "lib\common.ps1")
. (Join-Path $PSScriptRoot "lib\remote.ps1")

$Phase = "07_creds"
$env:MEDIAGIT_AUTHOR_NAME = "QA-Suite"
$env:MEDIAGIT_AUTHOR_EMAIL = "qa-suite@mediagit.local"

$TSV = Join-Path $QA.Logs "creds_results.tsv"
$script:AllPass = $true

function Rec([string]$Drill, $Pass, [string]$Detail) {
  Write-QaRow $TSV @("drill", "pass", "detail") @($Drill, $Pass, $Detail)
  $tag = if ("$Pass" -eq "SKIP") { "SKIP" } elseif ($Pass) { "PASS" } else { "FAIL" }
  Write-QaLog $Phase ("{0} -> {1}  {2}" -f $Drill, $tag, $Detail)
  Write-QaGate $Phase $Drill $Pass $Detail
  if ($tag -eq "FAIL") { $script:AllPass = $false }
}

Write-QaLog $Phase "=== 07_creds start ==="

$prevToken = $env:MEDIAGIT_TOKEN
$prevApiKey = $env:MEDIAGIT_API_KEY
$prevNoKeyring = $env:MEDIAGIT_NO_KEYRING
# Keychain LIVE for this phase.
Remove-Item Env:MEDIAGIT_NO_KEYRING -ErrorAction SilentlyContinue
Remove-Item Env:MEDIAGIT_TOKEN -ErrorAction SilentlyContinue
Remove-Item Env:MEDIAGIT_API_KEY -ErrorAction SilentlyContinue

$srv = $null
try {
  $srv = Start-QaServer -Backend "local" -Phase $Phase -EnableAuth -AdminUser "qa-admin" -AdminPass "admin-pw-123456"
  $base = $srv.BaseUrl

  # Get a VALID token for the admin (this is the good credential).
  $lr = Invoke-RestMethod -Method Post -Uri "$base/auth/login" -ContentType "application/json" `
    -Body (@{ identifier = "qa-admin"; password = "admin-pw-123456" } | ConvertTo-Json)
  $goodTok = $lr.tokens.access_token

  # Seed a fresh repo whose origin remote points at the server. Use init+remote
  # (not clone): the server repo is empty and cloning an empty repo is
  # unsupported, but init+remote still produces the [remotes.origin] config C1
  # needs, and C1's first push creates 'main' on the bare repo.
  $work = Join-Path $QA.Work "creds-$($QA.RunId)"
  if (Test-Path $work) { Remove-Item -Recurse -Force $work -ErrorAction SilentlyContinue }
  $init = Invoke-MG $null @("init", $work) $Phase
  if ($init.Exit -ne 0) { throw "SKIP: seed init failed (exit=$($init.Exit))" }
  $radd = Invoke-MG $work @("remote", "add", "origin", $srv.Url) $Phase
  if ($radd.Exit -ne 0) { throw "SKIP: seed remote add failed (exit=$($radd.Exit))" }

  # ---- C1-config-beats-keychain ----
  # 1) Cache a BAD token in the keychain via `auth login --token` (keychain tier).
  Invoke-MG $work @("auth", "login", "--server", $base, "--token", "bad-stale-token-xyz") $Phase | Out-Null
  # 2) Write a GOOD token explicitly into remotes.origin.token in config.
  $cfgPath = Join-Path $work ".mediagit\config.toml"
  $cfg = Get-Content $cfgPath -Raw
  # Append token under the [remotes.origin] table if not already present.
  if ($cfg -notmatch "(?m)^\s*token\s*=") {
    $cfg = $cfg -replace "(?ms)(\[remotes\.origin\][^\[]*)", ('$1' + "token = `"$goodTok`"`r`n")
  } else {
    $cfg = $cfg -replace "(?m)^\s*token\s*=.*$", "token = `"$goodTok`""
  }
  Set-Content $cfgPath $cfg -Encoding Ascii
  # 3) With NO env var, push must succeed - config token beats the stale keychain.
  Set-Content (Join-Path $work "c1.txt") "c1" -Encoding Ascii
  Invoke-MG $work @("add", "c1.txt") $Phase | Out-Null
  Invoke-MG $work @("commit", "-m", "c1") $Phase | Out-Null
  $push = Invoke-MG $work @("push", "origin") $Phase -TimeoutSec 600
  Rec "C1-config-beats-keychain" ($push.Exit -eq 0) `
    "push-with-config-token-over-stale-keychain=$($push.Exit -eq 0) (exit=$($push.Exit))"

  # ---- C2-status-tier ----
  $st = Invoke-MG $work @("auth", "status", "--server", $base) $Phase
  $namesTier = ($st.Out -match "config") -or ($st.Out -match "keychain") -or ($st.Out -match "env")
  Rec "C2-status-tier" (($st.Exit -eq 0) -and $namesTier) "status-exit=$($st.Exit) names-a-tier=$namesTier"

} catch {
  $skip = "$_" -match "^SKIP:"
  $tag = if ($skip) { "SKIP" } else { $false }
  $detail = if ($skip) { "$_" } else { "unexpected error: $_" }
  Rec "07_creds" $tag $detail
} finally {
  Stop-QaServer $srv
  # Clear any keychain entries this phase created (origin-keyed) so a dev box is
  # left clean; best-effort.
  Invoke-MG $null @("auth", "logout", "--all") $Phase 2>$null | Out-Null
  $env:MEDIAGIT_TOKEN = $prevToken
  $env:MEDIAGIT_API_KEY = $prevApiKey
  $env:MEDIAGIT_NO_KEYRING = $prevNoKeyring
}

Write-QaLog $Phase "=== 07_creds done: overall=$(if ($script:AllPass) { 'PASS' } else { 'FAIL' }) ==="
# Teardown: reclaim this phase's own work/ scratch so a long campaign cannot run the
# volume out of space. work/ ONLY - logs/ and fixtures-synthetic/ are never touched.
Invoke-QaTeardown $Phase @("creds-*")

Exit-QaPhase $Phase (-not $script:AllPass)
