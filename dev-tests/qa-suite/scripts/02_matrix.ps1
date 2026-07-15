# 02_matrix.ps1 - CLI command x flag coverage runner.
# Generalized from dev-tests/standalone-deep-v11/scripts/run_matrix.ps1 (read-only reference; not modified).
# Classes: OK (exit 0) | USAGE (clap arg error) | ERROR (nonzero, clean message) | PANIC | ERRTEXT-EXIT0 (error text but exit 0 = bug) | COVERED (remote ops, exercised by other phases)
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

Write-QaLog $Phase "matrix run starting, tier=$($QA.Tier)"

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

# ---- positional-arg / value maps (same intent as v11's run_matrix.ps1) ----
$remoteCmds = "^(clone|push|pull|fetch|download|remote)"
$posMap = @{
  "add" = @("f1.svg"); "commit" = @(); "diff" = @(); "log" = @(); "show" = @("HEAD"); "status" = @()
  "branch create" = @("nb1"); "branch delete" = @("feat"); "branch switch" = @("feat"); "branch rename" = @("feat", "feat2")
  "branch show" = @("feat"); "branch merge" = @("feat"); "branch protect" = @("feat"); "branch list" = @(); "branch" = @()
  "tag create" = @("nt1"); "tag delete" = @("t1"); "tag show" = @("t1"); "tag verify" = @("t1"); "tag list" = @(); "tag" = @()
  "merge" = @("feat"); "rebase" = @("feat"); "cherry-pick" = @("HEAD"); "revert" = @("HEAD"); "reset" = @()
  "bisect start" = @(); "bisect good" = @(); "bisect bad" = @(); "bisect reset" = @(); "bisect skip" = @(); "bisect log" = @(); "bisect replay" = @("bisect.log"); "bisect" = @()
  "stash push" = @(); "stash save" = @("wip"); "stash pop" = @(); "stash apply" = @(); "stash drop" = @(); "stash list" = @(); "stash show" = @(); "stash clear" = @(); "stash" = @()
  "reflog" = @(); "reflog show" = @(); "reflog delete" = @("HEAD@{0}"); "reflog expire" = @()
  "sparse-checkout set" = @("f1.svg"); "sparse-checkout list" = @(); "sparse-checkout disable" = @(); "sparse-checkout" = @()
  "media info" = @("f2.jpg"); "media" = @(); "completions" = @("bash")
  "gc" = @("-y"); "fsck" = @(); "verify" = @(); "stats" = @(); "version" = @(); "GLOBAL" = @()
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
$sb = Join-Path $QA.Work "matrix-run"

foreach ($item in $selected) {
  $i = $item.id
  $r = $item.row

  if ($r.cmd -match $remoteCmds) {
    Write-QaRow $OUT $HEADER @($i, "$($r.cmd) $($r.flag)", "COVERED", "-", "0", "remote_results.tsv")
    continue
  }

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
  if ($null -ne $pos) { $argv += $pos }
  if ($r.cmd -eq "init") { $argv += "init-sub-$i" }

  Write-QaLog $Phase "ROW $i : mediagit $($argv -join ' ')"
  $res = Invoke-MG $sb $argv $Phase
  $outText = $res.Out
  $panic = $outText -match "panicked|RUST_BACKTRACE"
  $errtext = $outText -match "Error|error:"
  $usage = $outText -match "(?i)usage:|unexpected argument|invalid value|required"
  $class =
    if ($panic) { "PANIC" }
    elseif ($res.Exit -eq 0 -and $errtext) { "ERRTEXT-EXIT0" }
    elseif ($res.Exit -eq 0) { "OK" }
    elseif ($usage) { "USAGE" }
    else { "ERROR" }

  if ($class -eq "PANIC") { $panicCount++ }
  if ($class -eq "ERRTEXT-EXIT0") { $errtextExit0Count++ }

  Write-QaRow $OUT $HEADER @($i, "$($r.cmd) $($r.flag)", $class, $res.Exit, $res.Sec, "$Phase-cmds.log")
}
Remove-Item -Recurse -Force $sb -EA SilentlyContinue

Write-QaGate $Phase "no-panics" ($panicCount -eq 0) "panicCount=$panicCount"
Write-QaGate $Phase "no-errtext-exit0" ($errtextExit0Count -eq 0) "errtextExit0Count=$errtextExit0Count"
Write-QaLog $Phase "done: $($selected.Count) rows run, panics=$panicCount errtextExit0=$errtextExit0Count"

if ($panicCount -eq 0 -and $errtextExit0Count -eq 0) { exit 0 } else { exit 1 }
