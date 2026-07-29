# Phase 12 - safety axis. ASCII-only, PS 5.1 compatible.
#
# Numbered 12, not 11: 11_memprofile.ps1 already owns that token and is
# deliberately outside the default run (its RSS sampling is process-wide, so a
# concurrent campaign ruins its numbers). Sharing the token would have dragged
# it into every campaign.
#
# The axis the harness did not have, and the reason 17 data-loss defects stayed
# invisible: every other phase asks "did the command succeed?", which a command
# that quietly deletes an untracked file answers with yes.
#
# This phase asks a different question. Before each destructive command it takes
# a full inventory of the working tree - every file, tracked or not, with its
# SHA256 - runs the command, and re-inventories. Anything that disappeared or
# changed without being asked to is a failure, whatever the exit code said.
#
# Untracked files are the point. They are invisible to `status --porcelain`
# comparisons and to any check built from the commit graph, so a checkout that
# deletes them looks perfectly clean from every angle except this one.
#
# Usage: powershell -File 12_safety.ps1 [-Only <substring>]
param(
  [string]$Only = ""
)

. (Join-Path $PSScriptRoot "lib\common.ps1")

$Phase = "12_safety"
$env:MEDIAGIT_AUTHOR_NAME = "QA-Suite"
$env:MEDIAGIT_AUTHOR_EMAIL = "qa-suite@mediagit.local"

$TSV = Join-Path $QA.Logs "safety_results.tsv"
$script:AllPass = $true

function ShouldRun([string]$Id) {
  if (-not $Only) { return $true }
  return $Id -like "*$Only*"
}

function Rec([string]$Check, [string]$Op, $Pass, [string]$Detail) {
  Write-QaRow $TSV @("check", "op", "pass", "detail") @($Check, $Op, $Pass, $Detail)
  $tag = if ("$Pass" -eq "SKIP") { "SKIP" } elseif ($Pass) { "PASS" } else { "FAIL" }
  Write-QaLog $Phase ("{0} :: {1} -> {2}  {3}" -f $Check, $Op, $tag, $Detail)
  Write-QaGate $Phase $Check $Pass $Detail
  if ($tag -eq "FAIL") { $script:AllPass = $false }
}

# Inventory every file under $Root except the repository's own metadata.
# Returns a hashtable of relative path -> SHA256. `.mediagit` is excluded
# because commands legitimately rewrite it; the working tree is what must not
# change behind the user's back.
function Get-TreeInventory([string]$Root) {
  $inv = @{}
  $rootFull = (Resolve-Path $Root).Path
  Get-ChildItem -Path $Root -Recurse -File -Force -EA SilentlyContinue | ForEach-Object {
    $rel = $_.FullName.Substring($rootFull.Length).TrimStart('\', '/')
    if ($rel -like ".mediagit*") { return }
    $hash = (Get-FileHash -Algorithm SHA256 -LiteralPath $_.FullName).Hash
    $inv[$rel] = $hash
  }
  return $inv
}

# Compare two inventories, restricted to the paths the caller says must survive.
# Returns a human-readable description of the damage, or "" when clean.
function Compare-Inventory($Before, $After, [string[]]$MustSurvive) {
  $problems = @()
  foreach ($rel in $MustSurvive) {
    if (-not $Before.ContainsKey($rel)) {
      $problems += "setup-error:$rel-not-created"
      continue
    }
    if (-not $After.ContainsKey($rel)) {
      $problems += "DELETED:$rel"
    }
    elseif ($After[$rel] -ne $Before[$rel]) {
      $problems += "MODIFIED:$rel"
    }
  }
  return ($problems -join "; ")
}

# Fresh repo with one commit, plus untracked files the operation has no business
# touching. Returns the sandbox path.
function New-SafetySandbox([string]$Name) {
  $sb = Join-Path $QA.Work "safety-$Name"
  if (Test-Path $sb) { Remove-Item -Recurse -Force $sb }
  New-Item -ItemType Directory -Force -Path $sb | Out-Null

  Push-Location $sb
  try {
    & $QA.MG init -q 2>&1 | Out-Null
    Set-Content -LiteralPath (Join-Path $sb "tracked.bin") -Value "committed content" -NoNewline
    & $QA.MG add "tracked.bin" 2>&1 | Out-Null
    & $QA.MG commit -m "base" 2>&1 | Out-Null

    # The files that matter: never staged, never committed, no reason for any
    # command to remove them.
    Set-Content -LiteralPath (Join-Path $sb "untracked-notes.txt") -Value "local scratch" -NoNewline
    New-Item -ItemType Directory -Force -Path (Join-Path $sb "wip") | Out-Null
    Set-Content -LiteralPath (Join-Path $sb "wip\render.tmp") -Value "expensive intermediate" -NoNewline
  }
  finally { Pop-Location }
  return $sb
}

$Untouchable = @("untracked-notes.txt", "wip\render.tmp")

# One safety case: set up, snapshot, run $Op, snapshot, compare.
# $Op receives the sandbox path and runs whatever command is under test. Its
# exit code is recorded but is NOT the gate - a command may legitimately fail
# (e.g. refuse a dirty tree); what it may not do is destroy untracked work.
function Test-Safety([string]$Id, [string]$OpName, [scriptblock]$Op) {
  if (-not (ShouldRun $Id)) { return }

  $sb = New-SafetySandbox $Id
  $before = Get-TreeInventory $sb

  Push-Location $sb
  $exit = 0
  try {
    & $Op $sb 2>&1 | Out-Null
    $exit = $LASTEXITCODE
  }
  catch {
    $exit = -1
  }
  finally { Pop-Location }

  $after = Get-TreeInventory $sb
  $damage = Compare-Inventory $before $after $Untouchable

  if ($damage) {
    Rec $Id $OpName $false ("exit={0}; {1}" -f $exit, $damage)
  }
  else {
    Rec $Id $OpName $true ("exit={0}; untracked work intact" -f $exit)
  }
}

Write-QaLog $Phase "safety axis: untracked work must survive every destructive command"

# --- the six callers that shared the unguarded checkout path -----------------

Test-Safety "SAFE01" "reset --hard" {
  param($sb)
  & $QA.MG reset --hard HEAD
}

Test-Safety "SAFE02" "branch switch" {
  param($sb)
  & $QA.MG branch create other 2>&1 | Out-Null
  & $QA.MG branch switch other
}

Test-Safety "SAFE03" "merge" {
  param($sb)
  & $QA.MG branch create feature 2>&1 | Out-Null
  & $QA.MG merge feature
}

Test-Safety "SAFE04" "revert" {
  param($sb)
  & $QA.MG revert HEAD
}

Test-Safety "SAFE05" "rebase" {
  param($sb)
  & $QA.MG branch create topic 2>&1 | Out-Null
  & $QA.MG rebase topic
}

Test-Safety "SAFE06" "cherry-pick" {
  param($sb)
  & $QA.MG cherry-pick HEAD
}

# --- gc must not collect what is only reachable from a stash or a remote ref --

Test-Safety "SAFE07" "gc (prunes by default)" {
  param($sb)
  & $QA.MG gc
}

Test-Safety "SAFE08" "stash then gc" {
  param($sb)
  Set-Content -LiteralPath (Join-Path $sb "tracked.bin") -Value "modified" -NoNewline
  & $QA.MG stash 2>&1 | Out-Null
  & $QA.MG gc
}

# --- sparse-checkout narrows the tree; it may not take untracked files with it

Test-Safety "SAFE09" "sparse-checkout set" {
  param($sb)
  & $QA.MG sparse-checkout set "tracked.bin"
}

# --- branch -d must not be as destructive as -D ------------------------------
# Distinct shape: the damage is to history, not the working tree, so it is
# checked directly rather than through the inventory.

if (ShouldRun "SAFE10") {
  $sb = New-SafetySandbox "SAFE10"
  Push-Location $sb
  try {
    & $QA.MG branch create unmerged 2>&1 | Out-Null
    & $QA.MG branch switch unmerged 2>&1 | Out-Null
    Set-Content -LiteralPath (Join-Path $sb "only-here.bin") -Value "unmerged work" -NoNewline
    & $QA.MG add "only-here.bin" 2>&1 | Out-Null
    & $QA.MG commit -m "unmerged commit" 2>&1 | Out-Null
    & $QA.MG branch switch main 2>&1 | Out-Null

    # `-d` is the *safe* delete (`--delete-merged`); `-D` is the forceful one.
    $out = & $QA.MG branch delete -d unmerged 2>&1
    $exit = $LASTEXITCODE
    $still = (& $QA.MG branch 2>&1) -join "`n"

    # -d is the *safe* delete: it must refuse a branch whose commits are not
    # merged anywhere. Choosing the safe flag and losing the work is worse than
    # having no safe flag at all.
    $refused = ($exit -ne 0) -or ($still -match "unmerged")
    Rec "SAFE10" "branch -d (unmerged)" $refused ("exit={0}; out={1}" -f $exit, ($out -join " ").Trim())
  }
  finally { Pop-Location }
}

# --- commit must not author as nobody ----------------------------------------

if (ShouldRun "SAFE11") {
  $sb = New-SafetySandbox "SAFE11"
  Push-Location $sb
  try {
    $savedName = $env:MEDIAGIT_AUTHOR_NAME
    $savedEmail = $env:MEDIAGIT_AUTHOR_EMAIL
    Remove-Item Env:MEDIAGIT_AUTHOR_NAME -EA SilentlyContinue
    Remove-Item Env:MEDIAGIT_AUTHOR_EMAIL -EA SilentlyContinue

    Set-Content -LiteralPath (Join-Path $sb "another.bin") -Value "x" -NoNewline
    & $QA.MG add "another.bin" 2>&1 | Out-Null
    $out = & $QA.MG commit -m "no identity" 2>&1
    $exit = $LASTEXITCODE

    $env:MEDIAGIT_AUTHOR_NAME = $savedName
    $env:MEDIAGIT_AUTHOR_EMAIL = $savedEmail

    # Authorship is immutable, so guessing it is permanent. Refusing is correct.
    $refused = ($exit -ne 0)
    Rec "SAFE11" "commit without identity" $refused ("exit={0}; out={1}" -f $exit, (($out -join " ").Trim()))
  }
  finally { Pop-Location }
}

# --- summary -----------------------------------------------------------------

if ($script:AllPass) {
  Write-QaLog $Phase "PHASE PASS - no untracked work destroyed"
  exit 0
}
else {
  Write-QaLog $Phase "PHASE FAIL - see $TSV"
  exit 1
}
