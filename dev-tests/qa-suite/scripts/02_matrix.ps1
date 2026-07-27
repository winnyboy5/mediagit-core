# 02_matrix.ps1 - CLI command x flag coverage runner.
# Generalized from dev-tests/standalone-deep-v11/scripts/run_matrix.ps1 (read-only reference; not modified).
# Classes: OK (exit 0) | USAGE (clap arg error) | ERROR (nonzero, clean message) | PANIC |
#          ERRTEXT-EXIT0 (error text but exit 0 = bug) | EXPECTED-ERR (remote command with no
#          server reachable: a nonzero exit with clean text is the correct outcome)
#
# Every row invokes the binary. There used to be a COVERED class that matched remote
# commands by regex and wrote a row claiming coverage WITHOUT running anything - roughly a
# quarter of the matrix was reported as exercised while never executing a single process.
# Remote commands are now really invoked (against the nonexistent origin of a scratch repo),
# where the contract is: fail POLITELY. A clean nonzero exit is a pass; a panic, or error
# text with exit 0, is a failure exactly as it is for any other command.
#
# Params:
#   -Rows   "1-20" (1-based inclusive range into coverage_matrix.tsv data rows) or a single number. Default: all rows.
#   -Filter regex applied to command_path. Default: all commands.
param(
  [string]$Rows = "",
  [string]$Filter = ""
)
. (Join-Path $PSScriptRoot "lib\common.ps1")
$Phase = "02_matrix"

$TSV_IN = Join-Path $PSScriptRoot "coverage_matrix.tsv"
$OUT    = Join-Path $QA.Logs "matrix_results.tsv"
$HEADER = @("row_id", "cmd", "class", "exit", "sec", "logRef")

# Unroutable target for every server-bound row: TCP port 1 is reserved and nothing
# listens on loopback there, so connects fail fast and deterministically.
$DEAD_URL = "http://127.0.0.1:1/mx-repo"

Write-QaLog $Phase "matrix run starting, tier=$($QA.Tier)"

# The matrix really invokes `auth logout` (including --all). Without this, a coverage
# sweep would reach into the developer's OS keychain and delete real stored credentials
# as a side effect. MEDIAGIT_NO_KEYRING=1 confines every auth row to process-local state.
$prevNoKeyring = $env:MEDIAGIT_NO_KEYRING
$env:MEDIAGIT_NO_KEYRING = "1"
Remove-Item Env:MEDIAGIT_TOKEN -ErrorAction SilentlyContinue
Remove-Item Env:MEDIAGIT_API_KEY -ErrorAction SilentlyContinue

# ---- template repo: 2 commits, extra branch, tag (mirrors v11 sandboxing approach) ----
$TPL = New-SandboxRepo "matrix-template" $Phase

function Resolve-Fixture([string]$RelChain, [string]$FallbackTestFile) {
  $p = Join-Path $QA.Fixtures $RelChain
  if (Test-Path $p) { return $p }
  $p2 = Join-Path $QA.TestFiles $FallbackTestFile
  if (Test-Path $p2) { return $p2 }
  return $null
}

$svg1 = Resolve-Fixture "chains\map_v1.svg" "3D_Model_of_the_Main_Gallery_in_Skednena_jama_Cave.svg"
$svg2 = Resolve-Fixture "chains\map_v2.svg" "3D_Model_of_the_Main_Gallery_in_Skednena_jama_Cave.svg"
$jpg1 = Resolve-Fixture "chains\photo_v1.jpg" "3-Modell_St._Mari_Kirche_auf_Hiro-Marker_(linke_Seite).jpg"
if (-not $svg1 -or -not $jpg1) { throw "no usable fixture/test-file found for matrix template repo (checked $($QA.Fixtures) and $($QA.TestFiles))" }
if (-not $svg2) { $svg2 = $svg1 }

Copy-Item $svg1 (Join-Path $TPL "f1.svg") -Force
Copy-Item $jpg1 (Join-Path $TPL "f2.jpg") -Force
Invoke-MG $TPL @("add", ".") $Phase | Out-Null
Invoke-MG $TPL @("commit", "-m", "c1") $Phase | Out-Null
Copy-Item $svg2 (Join-Path $TPL "f1.svg") -Force
Invoke-MG $TPL @("add", "f1.svg") $Phase | Out-Null
Invoke-MG $TPL @("commit", "-m", "c2") $Phase | Out-Null
Invoke-MG $TPL @("branch", "create", "feat") $Phase | Out-Null
Invoke-MG $TPL @("tag", "create", "t1") $Phase | Out-Null
# Port 1 is reserved and never listening, so a connect attempt fails immediately instead
# of hanging on a routable-but-silent address. Remote rows resolve this origin and then
# exercise the real connect-and-report path rather than bailing at "no such remote".
Invoke-MG $TPL @("remote", "add", "origin", $DEAD_URL) $Phase | Out-Null

# ---- positional-arg / value maps (same intent as v11's run_matrix.ps1) ----
# Commands that need a server. They are still invoked; they just have a different
# success contract (see EXPECTED-ERR above).
$remoteCmds = "^(clone|push|pull|fetch|download|auth)"
$posMap = @{
  "add" = @("f1.svg"); "commit" = @(); "diff" = @(); "log" = @(); "show" = @("HEAD"); "status" = @()
  "branch create" = @("nb1"); "branch delete" = @("feat"); "branch switch" = @("feat"); "branch rename" = @("feat", "feat2")
  "branch show" = @("feat"); "branch protect" = @("feat"); "branch list" = @(); "branch" = @()
  "tag create" = @("nt1"); "tag delete" = @("t1"); "tag show" = @("t1"); "tag verify" = @("t1"); "tag list" = @(); "tag" = @()
  "merge" = @("feat"); "rebase" = @("feat"); "cherry-pick" = @("HEAD"); "revert" = @("HEAD"); "reset" = @()
  "bisect start" = @(); "bisect good" = @(); "bisect bad" = @(); "bisect reset" = @(); "bisect skip" = @(); "bisect log" = @(); "bisect replay" = @("bisect.log"); "bisect" = @()
  "stash push" = @(); "stash save" = @("wip"); "stash pop" = @(); "stash apply" = @(); "stash drop" = @(); "stash list" = @(); "stash show" = @(); "stash clear" = @(); "stash" = @()
  "reflog" = @(); "reflog show" = @(); "reflog delete" = @("HEAD@{0}"); "reflog expire" = @()
  "sparse-checkout set" = @("f1.svg"); "sparse-checkout list" = @(); "sparse-checkout disable" = @(); "sparse-checkout" = @()
  "media info" = @("f2.jpg"); "media" = @(); "completions" = @("bash")
  "completions zsh" = @(); "completions fish" = @(); "completions powershell" = @()
  "gc" = @("-y"); "fsck" = @(); "verify" = @(); "stats" = @(); "version" = @(); "GLOBAL" = @()
  # Remote-side commands. The template repo carries an `origin` pointing at a closed
  # port, so these get past "no such remote" and exercise the real connect/error path.
  # absolute dest: Invoke-MG runs the binary with -C <repo>, so a relative clone target
  # would land in the harness's own cwd rather than the scratch tree.
  "clone" = @($DEAD_URL, (Join-Path $QA.Work "matrix-clone-dest"))
  "push" = @("origin"); "pull" = @(); "fetch" = @("origin")
  "download" = @("f1.svg")
  # `remote` subcommands are purely local bookkeeping - no server needed, so these are
  # held to the normal exit-0 contract like any other local command.
  "remote add" = @("mx-extra", $DEAD_URL); "remote remove" = @("mx-extra"); "remote list" = @()
  "remote rename" = @("origin", "origin2"); "remote set-url" = @("origin", $DEAD_URL)
  "remote show" = @("origin"); "remote" = @()
  # auth: every subcommand is server-bound; --server keeps it from needing a repo remote.
  "auth login" = @("--server", $DEAD_URL); "auth register" = @("--server", $DEAD_URL)
  "auth status" = @("--server", $DEAD_URL); "auth logout" = @("--server", $DEAD_URL)
  "auth whoami" = @("--server", $DEAD_URL); "auth passwd" = @("--server", $DEAD_URL)
  "auth key list" = @("--server", $DEAD_URL); "auth key create" = @("--server", $DEAD_URL, "--name", "mx")
  "auth key revoke" = @("--server", $DEAD_URL, "mx-key-id")
  "auth admin list-users" = @("--server", $DEAD_URL)
  "auth admin set-role" = @("--server", $DEAD_URL, "mxuser", "read")
  "auth admin create-user" = @("--server", $DEAD_URL, "mxuser", "--role", "read")
  "auth admin reset-password" = @("--server", $DEAD_URL, "mxuser")
  "auth admin grant" = @("--server", $DEAD_URL, "mxuser", "mxrepo", "read")
  "auth admin revoke-grant" = @("--server", $DEAD_URL, "mxuser", "mxrepo")
}
$valMap = @(
  @{rx = 'message|^-m$'; v = "msg" },
  @{rx = 'color'; v = "never" },
  @{rx = 'format'; v = "short" },
  @{rx = 'shell|completions'; v = "bash" },
  @{rx = 'depth|count|^-n$|max|jobs|limit'; v = "1" },
  @{rx = 'output|^-o$|file|^-F$'; v = (Join-Path $QA.Work "mx-out.bin") },
  @{rx = 'date|expire'; v = "2026-01-01" },
  @{rx = 'branch|track'; v = "feat" },
  @{rx = 'ref|revision|commit|start|path'; v = "HEAD" },
  @{rx = 'author'; v = "QA <qa@test>" },
  @{rx = '.'; v = "x" }
)
function ValFor([string]$flag) {
  foreach ($m in $valMap) { if ($flag -match $m.rx) { return $m.v } }
  return "x"
}

# ---- load + filter rows ----
$allRows = Get-Content $TSV_IN | Select-Object -Skip 1 | ForEach-Object {
  $c = $_ -split "`t"
  [pscustomobject]@{ cmd = $c[0]; flag = $c[1]; takes = $c[2] }
}
$selected = @()
for ($idx = 0; $idx -lt $allRows.Count; $idx++) {
  $selected += [pscustomobject]@{ id = $idx + 1; row = $allRows[$idx] }
}
if ($Rows) {
  if ($Rows -match '^(\d+)-(\d+)$') {
    $lo = [int]$Matches[1]; $hi = [int]$Matches[2]
    $selected = $selected | Where-Object { $_.id -ge $lo -and $_.id -le $hi }
  } elseif ($Rows -match '^\d+$') {
    $n = [int]$Rows
    $selected = $selected | Where-Object { $_.id -eq $n }
  }
}
if ($Filter) {
  $selected = $selected | Where-Object { $_.row.cmd -match $Filter }
}

Write-QaLog $Phase "running $($selected.Count) of $($allRows.Count) rows (Rows='$Rows' Filter='$Filter')"

$panicCount = 0
$errtextExit0Count = 0
$timeoutCount = 0
$classCounts = @{}
$sb = Join-Path $QA.Work "matrix-run"

foreach ($item in $selected) {
  $i = $item.id
  $r = $item.row

  $isRemote = $r.cmd -match $remoteCmds

  if (Test-Path $sb) { Remove-Item -Recurse -Force $sb }
  Copy-Item $TPL $sb -Recurse

  $argv = @()
  if ($r.cmd -ne "GLOBAL") { $argv += ($r.cmd -split " ") } else { $argv += "status" }
  $flagTok = ($r.flag -split ",")[-1].Trim()
  if ($flagTok -and $flagTok -ne "-(bare invocation)") {
    $argv += $flagTok
    if ($r.takes -eq "yes") { $argv += (ValFor $flagTok) }
  }
  $pos = $posMap[$r.cmd]
  if ($null -ne $pos) {
    # Drop any canned argument that duplicates the flag under test: rows exercising
    # --server/--name would otherwise get it twice (once as the flag, once from the map)
    # and clap would reject the whole invocation, turning real coverage into a USAGE row.
    $filtered = @()
    for ($pi = 0; $pi -lt $pos.Count; $pi++) {
      if ($pos[$pi] -eq $flagTok) { $pi++; continue }   # skip the flag AND its value
      $filtered += $pos[$pi]
    }
    $argv += $filtered
  }
  if ($r.cmd -eq "init") { $argv += "init-sub-$i" }

  Write-QaLog $Phase "ROW $i : mediagit $($argv -join ' ')"
  # `auth` prompts interactively (dialoguer Input/Password). Closing stdin immediately
  # gives it EOF, which it must report as an error rather than blocking forever; the
  # short timeout turns a hang into a recorded TIMEOUT row instead of stalling the phase.
  $res =
    if ($r.cmd -match '^auth') { Invoke-MG $sb $argv $Phase -TimeoutSec 30 -StdIn @("") }
    elseif ($isRemote) { Invoke-MG $sb $argv $Phase -TimeoutSec 60 }
    else { Invoke-MG $sb $argv $Phase }

  $outText = $res.Out
  $panic = $outText -match "panicked|RUST_BACKTRACE"
  $errtext = ($r.cmd -notmatch '^completions') -and ($outText -match "Error|error:")
  $usage = $outText -match "(?i)usage:|unexpected argument|invalid value|required"
  $class =
    if ($panic) { "PANIC" }
    elseif ($res.Exit -eq 124) { "TIMEOUT" }
    elseif ($res.Exit -eq 0 -and $errtext) { "ERRTEXT-EXIT0" }
    elseif ($res.Exit -eq 0) { "OK" }
    elseif ($usage) { "USAGE" }
    # A server-bound command with nothing listening SHOULD fail. Clean nonzero exit is
    # the correct behaviour, tracked separately so the honest OK count stays honest.
    elseif ($isRemote) { "EXPECTED-ERR" }
    else { "ERROR" }

  switch ($class) {
    "PANIC" { $panicCount++ }
    "ERRTEXT-EXIT0" { $errtextExit0Count++ }
    "TIMEOUT" { $timeoutCount++ }
  }
  $classCounts[$class] = [int]$classCounts[$class] + 1

  Write-QaRow $OUT $HEADER @($i, "$($r.cmd) $($r.flag)", $class, $res.Exit, $res.Sec, "$Phase-cmds.log")
}
Remove-Item -Recurse -Force $sb -EA SilentlyContinue
Remove-Item -Recurse -Force (Join-Path $QA.Work "matrix-clone-dest") -EA SilentlyContinue
Remove-Item -Recurse -Force $TPL -EA SilentlyContinue
$env:MEDIAGIT_NO_KEYRING = $prevNoKeyring

$counts = (($classCounts.Keys | Sort-Object | ForEach-Object { "$_=$($classCounts[$_])" }) -join " ")
Write-QaGate $Phase "no-panics" ($panicCount -eq 0) "panicCount=$panicCount"
Write-QaGate $Phase "no-errtext-exit0" ($errtextExit0Count -eq 0) "errtextExit0Count=$errtextExit0Count"
# A command that never returns is as broken as one that panics, and it is invisible in
# an exit-code-only view: gate it explicitly.
Write-QaGate $Phase "no-hangs" ($timeoutCount -eq 0) "timeoutCount=$timeoutCount"
Write-QaLog $Phase "done: $($selected.Count) rows invoked; $counts"

Exit-QaPhase $Phase
