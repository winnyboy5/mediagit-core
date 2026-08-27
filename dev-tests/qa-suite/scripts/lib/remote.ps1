# qa-suite server lifecycle helper. ASCII-only, PS 5.1 compatible.
# Dot-sourced by phase scripts alongside common.ps1:
#   . (Join-Path $PSScriptRoot "lib\common.ps1")
#   . (Join-Path $PSScriptRoot "lib\remote.ps1")
#
# Ground truth for config/launch mechanics: dev-tests\standalone-deep-v11\scripts\run_remote.ps1
# (per-backend bare repo + storage config.toml) and verify_auth.ps1 (server.toml + --config).
#
# API (fixed - other scripts depend on this shape):
#   Start-QaServer -Backend <minio|aws|azure|gcs|local> -Phase <name>  -> @{Url; Proc; DataDir; Backend}
#   Stop-QaServer <that object>
#
# Mechanics discovered in the codebase:
#   - mediagit-server.exe reads a GLOBAL config (--config server.toml: port/host/repos_dir/auth).
#   - Storage BACKEND selection is per-repo: it lives in "<repos_dir>/<repo>/.mediagit/config.toml"
#     under a [storage] table (schema: crates\mediagit-config\src\schema.rs StorageConfig enum:
#     "filesystem" | "s3" | "azure" | "gcs" | "multi"). Bare repos need this file written BEFORE
#     the server is started (or repos_dir rescanned), same as run_remote.ps1 does.
#   - Start-QaServer therefore creates one bare repo "proj-<RunId>" inside the per-run data dir,
#     writes its storage config from config\backends\<backend>.toml with tokens substituted, then
#     starts the server and returns Url already pointing AT that repo (ready for `mg remote add`).
#     The per-run repo name avoids collisions with leftover objects from a prior run under the
#     same bucket/container (repo_namespace config key is known-ignored; storage keys derive from
#     the bare-repo folder name instead - see project_pending_items memory / BUG-RM-2).
#   - /health is auth-exempt (crates\mediagit-server\src\lib.rs) - used as the readiness probe.

. (Join-Path $PSScriptRoot "common.ps1")

# Readiness budget for a freshly started mediagit-server.
#
# MUST stay above the server's OWN startup probe, which is a 30s tokio timeout
# around backend validation (crates\mediagit-server\src\main.rs: "startup probe
# timed out after 30s validating N repo(s)"). Waiting less than 30s here turns a
# merely SLOW backend into "server never became healthy" -- a false product
# failure the harness cannot tell apart from a real one.
#
# Both call sites previously used `for ($i = 0; $i -lt 40; $i++)` with a 500ms
# sleep and called it 20s. Two defects in that:
#   1. 20s < the 30s the server is allowed. 20260820-ga5's A11-auth-grants died
#      exactly here, and 20260820-ga2's server-up-{aws,azure,gcs} rows carry the
#      same "startup probe timed out after 30s" text.
#   2. An iteration count is not a timeout. Real elapsed time depended on how
#      long each failed probe took -- a refused connection returns in ms (~20s
#      total), a connection that HANGS burns the full -TimeoutSec 2 (~100s
#      total). The budget silently varied 5x with the failure mode.
# A wall-clock deadline fixes both and makes the timeout mean what it says.
$script:QA_SERVER_HEALTH_TIMEOUT_SEC =
  [int]$(if ($env:MG_QA_SERVER_HEALTH_TIMEOUT_SEC) { $env:MG_QA_SERVER_HEALTH_TIMEOUT_SEC } else { 60 })

# Poll <BaseUrl>/health until 200, the process exits, or the deadline passes.
# Returns $true only on a real 200. Single implementation on purpose: this loop
# lived in both Start-QaServer and Restart-QaServer and the two drifted, which
# is how the restart path ended up with no stderr in its error message.
function Wait-QaServerHealthy {
  param($Proc, [string]$BaseUrl, [int]$TimeoutSec = 0)
  if ($TimeoutSec -le 0) { $TimeoutSec = $script:QA_SERVER_HEALTH_TIMEOUT_SEC }
  $deadline = (Get-Date).AddSeconds($TimeoutSec)
  while ((Get-Date) -lt $deadline) {
    # A process that exited will never answer; stop waiting out the deadline.
    if ($Proc.HasExited) { return $false }
    try {
      $resp = Invoke-WebRequest -Uri "$BaseUrl/health" -UseBasicParsing -TimeoutSec 2 -ErrorAction Stop
      if ($resp.StatusCode -eq 200) { return $true }
    } catch { }
    Start-Sleep -Milliseconds 500
  }
  return $false
}

function Get-QaFreePort {
  $l = New-Object System.Net.Sockets.TcpListener ([System.Net.IPAddress]::Loopback, 0)
  $l.Start()
  $port = $l.LocalEndpoint.Port
  $l.Stop()
  return $port
}

# Resolve backend -> @{ Template=<filename in config\backends>; Tokens=<hashtable> }.
#
# Throws "SKIP: ..." ONLY for a backend the operator did not ask for or did not supply
# credentials for - i.e. things that are absent by choice. Everything else (a missing
# template, a server that will not start, a backend that is selected but broken) throws
# a plain error so the caller records a FAILURE. Infrastructure dying mid-campaign is
# not a skip: reporting it as one is how a run with nothing verified reads green.
function _QaBackendConfig([string]$Backend) {
  # "local" is always available (filesystem storage); every other backend has to be
  # selected. This is the one skip that may leave an all-skip phase green, so it
  # carries the marker Exit-QaPhase looks for.
  if ($Backend -ne "local" -and $QA.Backends -notcontains $Backend) {
    throw "SKIP: backend '$Backend' $QA_SKIP_NOT_SELECTED"
  }
  switch ($Backend) {
    "minio" {
      if (-not $QA.MinioAccessKey -or -not $QA.MinioSecretKey -or -not $QA.MinioEndpoint) {
        throw "SKIP: MinIO not configured (QA.MinioEndpoint/MinioAccessKey/MinioSecretKey)"
      }
      $bucket = _Env "MG_QA_MINIO_BUCKET" "mediagit-qa-suite"
      return @{ Template = "minio.toml"; Tokens = @{
        ENDPOINT = $QA.MinioEndpoint; BUCKET = $bucket
        ACCESS_KEY = $QA.MinioAccessKey; SECRET_KEY = $QA.MinioSecretKey; REGION = "us-east-1"
      } }
    }
    "aws" {
      $ak = [Environment]::GetEnvironmentVariable("AWS_ACCESS_KEY_ID")
      $sk = [Environment]::GetEnvironmentVariable("AWS_SECRET_ACCESS_KEY")
      $bucket = [Environment]::GetEnvironmentVariable("MG_QA_AWS_BUCKET")
      $region = [Environment]::GetEnvironmentVariable("AWS_DEFAULT_REGION")
      if (-not $region) { $region = _Env "AWS_REGION" "us-east-1" }
      if (-not $ak -or -not $sk -or -not $bucket) {
        throw "SKIP: AWS not configured (need AWS_ACCESS_KEY_ID, AWS_SECRET_ACCESS_KEY, MG_QA_AWS_BUCKET env vars)"
      }
      return @{ Template = "aws.toml"; Tokens = @{
        ACCESS_KEY = $ak; SECRET_KEY = $sk; BUCKET = $bucket; REGION = $region
      } }
    }
    "azure" {
      $acct = [Environment]::GetEnvironmentVariable("AZURE_STORAGE_ACCOUNT")
      $key  = [Environment]::GetEnvironmentVariable("AZURE_STORAGE_KEY")
      $cont = [Environment]::GetEnvironmentVariable("MG_QA_AZURE_CONTAINER")
      if (-not $acct -or -not $key -or -not $cont) {
        throw "SKIP: Azure not configured (need AZURE_STORAGE_ACCOUNT, AZURE_STORAGE_KEY, MG_QA_AZURE_CONTAINER env vars)"
      }
      return @{ Template = "azure.toml"; Tokens = @{ ACCOUNT = $acct; KEY = $key; CONTAINER = $cont } }
    }
    "gcs" {
      $proj = [Environment]::GetEnvironmentVariable("GCS_PROJECT_ID")
      if (-not $proj) { $proj = [Environment]::GetEnvironmentVariable("GOOGLE_CLOUD_PROJECT") }
      $bucket = [Environment]::GetEnvironmentVariable("GCS_BUCKET_NAME")
      $creds  = [Environment]::GetEnvironmentVariable("GOOGLE_APPLICATION_CREDENTIALS")
      if (-not $proj -or -not $bucket) {
        throw "SKIP: GCS not configured (need GCS_PROJECT_ID, GCS_BUCKET_NAME env vars; GOOGLE_APPLICATION_CREDENTIALS optional/ADC)"
      }
      $credsLine = ""
      if ($creds) { $credsLine = 'credentials_path = "' + ($creds -replace '\\', '/') + '"' }
      # Per-run prefix, deliberately non-empty: the server used to drop it, so a rooted
      # layout is the regression signal. Also keeps concurrent runs from colliding.
      $prefix = [Environment]::GetEnvironmentVariable("MG_QA_GCS_PREFIX")
      if (-not $prefix) { $prefix = "qa/$($QA.RunId)" }
      return @{ Template = "gcs.toml"; Tokens = @{ PROJECT = $proj; BUCKET = $bucket; PREFIX = $prefix; CREDS_LINE = $credsLine } }
    }
    "local" {
      return @{ Template = "local.toml"; Tokens = @{} }
    }
    default { throw "SKIP: unknown backend '$Backend'" }
  }
}

# Start-QaServer -Backend <minio|aws|azure|gcs|local> -Phase <name>
# Returns @{ Url; Proc; DataDir; Backend } on success.
# Throws "SKIP: ..." ONLY when the backend is absent BY CHOICE - not selected in
# $QA.Backends, or credentials not supplied. Those are the exceptions a caller may
# legitimately record as a SKIP row.
#
# Any other failure - a server that never became healthy, a missing template, a failed
# `init --bare` - throws a PLAIN error and has already written a FAILED gate to
# gates.tsv before throwing. A caller that blanket-catches and records SKIP cannot
# soften it. Do NOT extend the SKIP: prefix to infrastructure failures: "server failed
# to become healthy" was previously documented here as a skip, and that sentence is
# what kept callers converting a dead MinIO into a green campaign.
function Start-QaServer {
  param(
    [Parameter(Mandatory = $true)][ValidateSet("minio", "aws", "azure", "gcs", "local")][string]$Backend,
    [Parameter(Mandatory = $true)][string]$Phase,
    # 07_auth: enable_auth=true + jwt_secret in server.toml. Auth store
    # (users/api_keys/grants.jsonl) lands in <DataDir>\auth - the server's
    # default auth_store_dir is a sibling "auth" dir next to repos_dir.
    [switch]$EnableAuth,
    # Optional: bootstrap a first Admin via `mediagit-server admin create` once
    # the server dir/config exist. Implies -EnableAuth. The admin is created
    # BEFORE the server process starts (no --force needed, store not yet held in
    # memory). Returned handle gains .AdminUser/.AdminPass for the caller.
    [string]$AdminUser,
    [string]$AdminPass,
    # DC-7/D4: serve encrypted repositories. The server needs a master key of
    # its own to wrap each repo key it is handed via `PUT /{repo}/encryption-key`;
    # without one it answers 404 there and an encrypted push refuses.
    [string]$EncryptionKeyfile,
    # Rate limiting is ON for every QA server, deliberately.
    #
    # It ships OFF by default (ServerConfig::enable_rate_limiting) and nothing in
    # this harness ever turned it on, so every gate this suite has run was
    # measured against a limiter that was not there. That is how A13 in 07_abuse
    # -- whose entire assertion is "a per-chunk push must NOT trip 429" -- has
    # passed without the ability to fail, and how a 10 rps default survived to
    # reach a user as a 429 storm on ordinary pushes.
    #
    # Left at 0/0 the server uses its OWN defaults, which is the point: the full
    # campaign then doubles as the end-to-end check that those defaults carry
    # real pushes and clones across five backends. 07_ratelimit passes explicit
    # values to test enforcement.
    [int]$RateLimitRps = 0,
    [int]$RateLimitBurst = 0,
    # Only for a drill that must prove behaviour with the limiter absent.
    [switch]$NoRateLimit
  )
  if ($AdminUser) { $EnableAuth = $true }

  $bc = _QaBackendConfig $Backend
  $tplPath = Join-Path $QA.Root "config\backends\$($bc.Template)"
  # A missing template is a broken harness checkout, not an absent capability - fail loudly.
  if (-not (Test-Path $tplPath)) { throw "missing backend template $tplPath" }

  # Unique dir per invocation: multiple drills in one phase reuse the same backend, and a
  # shared "server-$Backend" dir let a new drill's wipe race a lingering prior server
  # (J3/A10 finding 2026-07-17). Old dirs are cheap and useful for post-mortem.
  $script:QaSrvSeq = [int]$script:QaSrvSeq + 1
  $srvDir = Join-Path $QA.Work "server-$Backend-$Phase-$($script:QaSrvSeq)"
  if (Test-Path $srvDir) { Remove-Item -Recurse -Force $srvDir -ErrorAction SilentlyContinue }
  New-Item -ItemType Directory -Path (Join-Path $srvDir "repos") -Force | Out-Null

  # The sequence number belongs in the repo NAME too, not just the directory above.
  # For cloud backends the repo name becomes the storage namespace, and `init --bare`
  # mints a fresh repo_id every call — so two invocations that resolve to the same name
  # on one bucket trip the layout-marker collision guard ("already owned by repo_id X").
  # That guard is correct and must not be relaxed: it is what stops one repo's `gc` from
  # deleting another's objects. The bug was here.
  #
  # Hit in campaign 20260730-camp2: phase 07 was killed mid-run, and re-running it under
  # the SAME MG_QA_RUN_ID produced the same repo name with a new repo_id, so the server
  # refused to start. Resuming a phase is exactly what the chunked-campaign workflow does
  # after a kill, so this made campaigns non-resumable against minio/aws/azure/gcs while
  # looking fine on local. `Restart-QaServer` reuses an existing handle and never calls
  # this path, so restart-in-place semantics are unaffected.
  #
  # $script:QaSrvSeq ALONE IS NOT ENOUGH and the first attempt at this fix proved it:
  # the counter is per-PowerShell-process, so every `run_all.ps1` invocation resets it to
  # 0 and the first server again claims "...-A2-1" — the exact name the previous attempt
  # registered. That fixes collisions WITHIN a run while leaving the re-run case, which is
  # the one that matters, still broken. $PID varies per invocation and keeps the name
  # traceable back to the process that created it (a random suffix would too, but then a
  # stranded bucket namespace cannot be matched to anything in the logs).
  $repoName = "proj-$($QA.RunId)-$Phase-$PID-$($script:QaSrvSeq)"
  $repoDir = Join-Path $srvDir "repos\$repoName"
  $r = Invoke-MG $null @("init", "--bare", $repoDir) $Phase
  if ($r.Exit -ne 0) { throw "init --bare failed for $repoDir : $($r.Out)" }

  $tokens = $bc.Tokens
  $tokens["BASE_PATH"] = (Join-Path $srvDir "storage") -replace '\\', '/'
  $cfg = Get-Content $tplPath -Raw
  foreach ($k in $tokens.Keys) { $cfg = $cfg.Replace("{{$k}}", "" + $tokens[$k]) }
  $cfg | Set-Content (Join-Path $repoDir ".mediagit\config.toml") -Encoding Ascii

  $port = Get-QaFreePort
  $reposDirFwd = (Join-Path $srvDir "repos") -replace '\\', '/'
  $authLines = ""
  if ($EnableAuth) {
    $authLines = "`nenable_auth = true`njwt_secret = `"qa-suite-jwt-secret-0123456789abcdef0123456789abcdef`""
  }
  $rlLines = ""
  if (-not $NoRateLimit) {
    $rlLines = "`nenable_rate_limiting = true"
    if ($RateLimitRps   -gt 0) { $rlLines += "`nrate_limit_rps = $RateLimitRps" }
    if ($RateLimitBurst -gt 0) { $rlLines += "`nrate_limit_burst = $RateLimitBurst" }
  }
  # `[encryption]` goes LAST: ServerConfig is flat apart from that section, so
  # any table header swallows every top-level key written after it.
  $encLines = ""
  if ($EncryptionKeyfile) {
    $kfFwd = $EncryptionKeyfile.Replace('\', '/')
    $encLines = "`n`n[encryption]`nenabled = true`nmaster_key_path = `"$kfFwd`""
  }
  @"
port = $port
host = "127.0.0.1"
repos_dir = "$reposDirFwd"$authLines$rlLines$encLines
"@ | Set-Content (Join-Path $srvDir "server.toml") -Encoding Ascii

  # Bootstrap the first Admin offline before the process starts: the store is not
  # yet held in memory, so no --force is needed and no restart is required.
  if ($AdminUser) {
    $srvToml = Join-Path $srvDir "server.toml"
    $adminOut = & $QA.MGServer @(
      "admin", "--config", $srvToml, "create", $AdminUser, "$AdminUser@qa.local", "--password", $AdminPass
    ) 2>&1
    if ($LASTEXITCODE -ne 0) { throw "admin create failed for $AdminUser : $adminOut" }
    Write-QaLog $Phase "bootstrapped admin '$AdminUser' via mediagit-server admin create"
  }

  # Include the per-phase server sequence, matching the data-dir naming above.
  # Without it, every server started under one phase name TRUNCATES the previous
  # one's log (Start-Process -RedirectStandardOutput truncates), and 07_abuse
  # starts >=4 servers under a single phase. That is exactly how the 2026-08-03
  # A4 stall lost its only evidence: an 8 MiB push took 1,188s and by the time it
  # was investigated its server log had been overwritten by a later drill's
  # server, which also produced a wrong inference ("the server received zero
  # requests" - it had simply been truncated).
  $outLog = Join-Path $QA.Logs "server-$Backend-$Phase-$($script:QaSrvSeq).out.log"
  $errLog = Join-Path $QA.Logs "server-$Backend-$Phase-$($script:QaSrvSeq).err.log"
  $proc = Start-Process -FilePath $QA.MGServer `
    -ArgumentList @("--config", (Join-Path $srvDir "server.toml")) `
    -PassThru -NoNewWindow -RedirectStandardOutput $outLog -RedirectStandardError $errLog

  $url = "http://127.0.0.1:$port"
  $healthy = Wait-QaServerHealthy -Proc $proc -BaseUrl $url

  if (-not $healthy) {
    $errText = if (Test-Path $errLog) { (Get-Content $errLog -Raw -ErrorAction SilentlyContinue) } else { "" }
    if (-not $proc.HasExited) { Stop-Process -Id $proc.Id -Force -ErrorAction SilentlyContinue }
    # NOT a SKIP: the backend was selected and its credentials resolved, so a server
    # that will not come up is a live defect (or dead infrastructure) and must fail
    # the phase. This used to be a SKIP, which turned every MinIO outage into a green run.
    #
    # Throwing is not enough on its own. Every caller wraps this in
    # `try/catch { Add-Row ... "SKIP" ... }`, and those rows go to the phase's own step
    # table, never to gates.tsv -- so Exit-QaPhase counted skip=0 and the phase reported
    # PASS on whatever local gates it had while push and clone silently never ran
    # (campaign 20260730-120513: persona_designer PASS with 5 fsck gates, zero remote
    # coverage). Recording the gate HERE makes the failure unbypassable by a lenient
    # caller, which is the only version of this that stays fixed.
    Write-QaGate $Phase "server-up-$Backend" $false "server never became healthy"
    throw "mediagit-server ($Backend) never became healthy: $errText"
  }

  Write-QaLog $Phase "server $Backend up: $url/$repoName (pid $($proc.Id))"
  return @{
    Url = "$url/$repoName"; Proc = $proc; DataDir = $srvDir; Backend = $Backend
    BaseUrl = $url; RepoName = $repoName
    ConfigPath = (Join-Path $srvDir "server.toml")
    OutLog = $outLog; ErrLog = $errLog
    AdminUser = $AdminUser; AdminPass = $AdminPass
  }
}

# Restart-QaServer <handle> -Phase <name>: kill the current process and start a
# fresh one against the SAME server.toml/data dir. Used by 07_auth to pick up
# an edited users.jsonl (auth store loads at boot only). Updates $Handle.Proc.
function Restart-QaServer($Handle, [string]$Phase = "misc") {
  Stop-QaServer $Handle
  $proc = Start-Process -FilePath $QA.MGServer `
    -ArgumentList @("--config", $Handle.ConfigPath) `
    -PassThru -NoNewWindow -RedirectStandardOutput $Handle.OutLog -RedirectStandardError $Handle.ErrLog
  $healthy = Wait-QaServerHealthy -Proc $proc -BaseUrl $Handle.BaseUrl
  if (-not $healthy) {
    # Include the server's OWN stderr, as Start-QaServer already does. Without
    # it this threw a bare "did not become healthy", which is true but says
    # nothing -- diagnosing 20260820-ga5 meant going and finding the .err.log by
    # hand, where the server had plainly written why it quit ("startup probe
    # timed out after 30s validating 1 repo(s)"). An error that omits the cause
    # it already has in a file next to it is a diagnostic dead end.
    $errText = if ($Handle.ErrLog -and (Test-Path $Handle.ErrLog)) {
      (Get-Content $Handle.ErrLog -Raw -ErrorAction SilentlyContinue)
    } else { "" }
    if (-not $proc.HasExited) { Stop-Process -Id $proc.Id -Force -ErrorAction SilentlyContinue }
    throw "Restart-QaServer: server did not become healthy again at $($Handle.BaseUrl): $errText"
  }
  $Handle.Proc = $proc
  Write-QaLog $Phase "server restarted: $($Handle.Url) (pid $($proc.Id))"
}

# Stop-QaServer <handle returned by Start-QaServer>. Kills the process tree; safe to call
# on $null or an already-stopped handle.
function Stop-QaServer($Handle) {
  if (-not $Handle -or -not $Handle.Proc) { return }
  try {
    $procId = $Handle.Proc.Id
    # taskkill /T kills the whole tree in one shot; the prior child-enumeration approach
    # left orphaned mediagit-server.exe processes behind (J3 finding 2026-07-17).
    & taskkill /PID $procId /T /F 2>$null | Out-Null
    if (-not $Handle.Proc.WaitForExit(5000)) {
      Write-Warning "Stop-QaServer: pid $procId still alive 5s after taskkill /T /F"
    }
  } catch {}
}
