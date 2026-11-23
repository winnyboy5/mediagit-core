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

# --- gc must never turn "cannot read a reachable object" into "delete it" ------
#
# Both checks run gc with the prune grace window and the reflog roots switched OFF
# (MEDIAGIT_GC_GRACE_SECS=0, MEDIAGIT_GC_REFLOG_HORIZON_DAYS=0). With the defaults
# every object is under the one-hour grace and gc deletes nothing, so a gc that
# loses data would pass: the gate would be unable to fail.


# SAFE12: a reachable manifest that no longer parses. gc cannot enumerate that
# blob's chunks; it must refuse (non-zero exit) and delete NO chunk file, rather
# than treat the blob as having no chunks and sweep them all.
if (ShouldRun "SAFE12") {
  try {
    $repo = New-SandboxRepo "safety-SAFE12" $Phase
    $rnd = New-Object System.Random(120012)
    $buf = New-Object byte[] (8MB)
    $rnd.NextBytes($buf)
    [IO.File]::WriteAllBytes((Join-Path $repo "asset.bin"), $buf)
    Invoke-MG $repo @("add", "asset.bin") $Phase | Out-Null
    Invoke-MG $repo @("commit", "-m", "chunked blob") $Phase | Out-Null

    $objFiles = @(Get-ChildItem (Join-Path $repo ".mediagit\objects") -Recurse -File -EA SilentlyContinue)
    $manifest = $objFiles | Where-Object { $_.FullName -match '\\manifests\\' } | Sort-Object Length -Descending | Select-Object -First 1
    $chunkCount = @($objFiles | Where-Object { $_.FullName -match '\\chunks\\' }).Count
    if (-not $manifest -or $chunkCount -eq 0) {
      Rec "SAFE12" "gc with corrupt reachable manifest" "SKIP" ("no loose manifest/chunk files found (manifest={0} chunks={1}); layout changed or objects packed" -f [bool]$manifest, $chunkCount)
    } else {
      [IO.File]::WriteAllBytes($manifest.FullName, [byte[]](0xFF, 0xFE, 0xFD, 0xFC, 0x00, 0x01, 0x02, 0x03, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF))
      $gc = Invoke-QaGcHarsh $repo $Phase
      $chunksAfter = @(Get-ChildItem (Join-Path $repo ".mediagit\objects") -Recurse -File -EA SilentlyContinue |
        Where-Object { $_.FullName -match '\\chunks\\' }).Count
      # non-zero for the RIGHT reason: an unrelated early failure must not pass this gate.
      $pass = ($gc.Exit -ne 0) -and ($gc.Out -match "cannot read reachable chunk manifest") -and ($chunksAfter -eq $chunkCount)
      Rec "SAFE12" "gc with corrupt reachable manifest" $pass ("gc-exit={0} (must be non-zero) chunks before={1} after={2} (must be equal)" -f $gc.Exit, $chunkCount, $chunksAfter)
    }
  } catch {
    Rec "SAFE12" "gc with corrupt reachable manifest" $false "unexpected error: $_"
  }
}

# SAFE13: delta-base retention. v2's chunks are stored as deltas against chunks that
# exist ONLY through v1. v1's commit is then made unreachable, so those base chunks
# are reachable from nothing except the delta chain of a chunk v2 needs. gc must keep
# them: v2 must still check out byte-exact and fsck must stay clean.
#   c0 (base.txt) <- c1 (v1.psd, dropped by reset) ; c0 <- c2 (v2 = edited copy of v1)
#
# NON-VACUITY. gc roots (crates\mediagit-cli\src\commands\gc.rs build_reachability_set):
#   HEAD :341, local branches :353, tags :358, reflog :405 (off when
#   MEDIAGIT_GC_REFLOG_HORIZON_DAYS=0), remotes/* :454, STASH_LIST :476 (stash_roots :528),
#   in-progress state files :490 (in_progress_roots :559: rebase-apply\state.json,
#   CHERRY_PICK_STATE, REVERT_STATE, BISECT_STATE, MERGE_HEAD, ORIG_HEAD), staged index :505;
#   plus the prune grace window (past_grace, MEDIAGIT_GC_GRACE_SECS=0 disables).
# reset.rs writes only the reflog (:204-215) and never ORIG_HEAD; this test has no tags,
# remotes, stash or v1 index entries, and deletes the state files anyway. Proof that c1
# really became unreachable is behavioural: at least one object file that c1 added to the
# store must be GONE after gc. If none was deleted, a root still held c1 (or gc deleted
# nothing) and the gate FAILS as vacuous instead of passing.
if (ShouldRun "SAFE13") {
  try {
    $psd = Get-ChildItem (Join-Path $QA.TestFiles "psd") -Filter *.psd -File -EA SilentlyContinue |
      Sort-Object Length | Select-Object -First 1
    if (-not $psd) {
      Rec "SAFE13" "gc keeps chunk-delta bases of reachable chunks" "SKIP" "no psd fixture under test-files\psd"
    } else {
      $repo = New-SandboxRepo "safety-SAFE13" $Phase
      $asset = Join-Path $repo "asset.psd"
      Set-Content -LiteralPath (Join-Path $repo "base.txt") -Value "base" -NoNewline
      Invoke-MG $repo @("add", "base.txt") $Phase | Out-Null
      $c0 = Invoke-MG $repo @("commit", "-m", "c0") $Phase
      $oid0 = if ($c0.Out -match "Created commit ([0-9a-fA-F]{8,})") { $Matches[1] } else { $null }
      $objRoot = Join-Path $repo ".mediagit\objects"
      $snap = { @(Get-ChildItem $objRoot -Recurse -File -EA SilentlyContinue | ForEach-Object { $_.FullName.Substring($objRoot.Length) }) }
      $snap0 = & $snap

      Copy-Item -LiteralPath $psd.FullName -Destination $asset -Force
      Invoke-MG $repo @("add", "asset.psd") $Phase -TimeoutSec 3600 | Out-Null
      $c1 = Invoke-MG $repo @("commit", "-m", "c1 v1") $Phase -TimeoutSec 3600
      $oid1 = if ($c1.Out -match "Created commit ([0-9a-fA-F]{8,})") { $Matches[1] } else { $null }
      $c1Only = @(Compare-Object $snap0 (& $snap) | Where-Object { $_.SideIndicator -eq "=>" } | ForEach-Object { $_.InputObject })

      # drop c1 from every ref: only c0 is reachable again.
      $rs = Invoke-MG $repo @("reset", "--hard", $oid0) $Phase -TimeoutSec 3600
      $bytes = [IO.File]::ReadAllBytes($psd.FullName)
      $r2 = New-Object System.Random(130013)
      $start = [int](($bytes.Length - 4096) / 2)
      for ($i = 0; $i -lt 4096; $i++) { $bytes[$start + $i] = [byte]$r2.Next(0, 256) }
      [IO.File]::WriteAllBytes($asset, $bytes)
      $v2Hash = Get-QaHash $asset
      Invoke-MG $repo @("add", "asset.psd") $Phase -TimeoutSec 3600 | Out-Null
      Invoke-MG $repo @("commit", "-m", "c2 v2") $Phase -TimeoutSec 3600 | Out-Null

      # belt and braces: clear every state-file root so none can hold c1.
      foreach ($f in @("ORIG_HEAD", "MERGE_HEAD", "CHERRY_PICK_STATE", "REVERT_STATE", "BISECT_STATE", "STASH_LIST")) {
        Remove-Item -LiteralPath (Join-Path $repo ".mediagit\$f") -Force -EA SilentlyContinue
      }
      Remove-Item -LiteralPath (Join-Path $repo ".mediagit\rebase-apply") -Recurse -Force -EA SilentlyContinue

      $chain = Get-QaChainStats $repo
      if (-not $oid0 -or -not $oid1 -or $rs.Exit -ne 0 -or $c1Only.Count -eq 0) {
        Rec "SAFE13" "gc keeps chunk-delta bases of reachable chunks" $false ("setup failed: oid0={0} oid1={1} reset-exit={2} c1-added-files={3}" -f $oid0, $oid1, $rs.Exit, $c1Only.Count)
      } elseif ($chain.ChainCount -eq 0) {
        Rec "SAFE13" "gc keeps chunk-delta bases of reachable chunks" "SKIP" "v2 produced no chunk-delta sidecars - base retention not exercised"
      } else {
        $gc = Invoke-QaGcHarsh $repo $Phase
        $after = & $snap
        $deleted = @($c1Only | Where-Object { $after -notcontains $_ }).Count
        $nonVacuous = ($gc.Exit -eq 0) -and ($deleted -gt 0)
        Rec "SAFE13" "gc actually collected c1 (non-vacuity)" $nonVacuous ("gc-exit={0} c1-added-files={1} of-which-deleted={2} (must be >0, else a root still holds c1)" -f $gc.Exit, $c1Only.Count, $deleted)

        Remove-Item -LiteralPath $asset -Force
        $co = Invoke-MG $repo @("reset", "--hard", "HEAD") $Phase -TimeoutSec 3600
        $exact = (Test-Path $asset) -and ((Get-QaHash $asset) -eq $v2Hash)
        $fsck = Invoke-MG $repo @("fsck", "--full") $Phase -TimeoutSec 3600
        $fsckClean = ($fsck.Exit -eq 0) -and ($fsck.Out -notmatch "(?i)corrupt|missing|failed")
        Rec "SAFE13" "gc keeps chunk-delta bases of reachable chunks" ($nonVacuous -and $exact -and $fsckClean) `
          ("delta-chunks={0} non-vacuous={1} checkout-exit={2} v2-byte-exact={3} fsck-exit={4}" -f $chain.ChainCount, $nonVacuous, $co.Exit, $exact, $fsck.Exit)
      }
    }
  } catch {
    Rec "SAFE13" "gc keeps chunk-delta bases of reachable chunks" $false "unexpected error: $_"
  }
}

# --- QA-live regression gates (2026-11-23) --------------------------------------
# The 7 P0s and the data-loss class found by the 2026-11 live campaign (BUG-002, 046,
# 041, 008, 011; 032 and 006 live in 05_branching because they need a server). Each gate
# is built so the OLD behaviour fails it: every assertion is on the outcome a user would
# be hurt by (a "clean" verdict on a broken store, a lost file), never on message text
# alone, and every gate first proves its own baseline (the repo was healthy / the file
# really existed) so a setup that silently did nothing cannot turn it green.

function Sf-Write([string]$Dir, [string]$Rel, [string]$Text) {
  $p = Join-Path $Dir $Rel
  $d = Split-Path -Parent $p
  if (-not (Test-Path -LiteralPath $d)) { New-Item -ItemType Directory -Force -Path $d | Out-Null }
  [IO.File]::WriteAllText($p, $Text)
}

function Sf-Rand([string]$Path, [int]$Bytes, [int]$Seed) {
  $b = New-Object byte[] $Bytes
  (New-Object System.Random($Seed)).NextBytes($b)
  [IO.File]::WriteAllBytes($Path, $b)
}

# Repo with a 300 KB random blob (one big object) and a tiny text file, committed.
function New-ObjRepo([string]$Name) {
  $r = New-SandboxRepo $Name $Phase
  Sf-Rand (Join-Path $r "big.bin") 300000 4242
  Sf-Write $r "s.txt" "small"
  Invoke-MG $r @("add", "-A") $Phase | Out-Null
  Invoke-MG $r @("commit", "-m", "c1") $Phase | Out-Null
  return $r
}

# The largest object file in the store = the blob of big.bin.
function Get-BigObject([string]$Repo) {
  return (Get-ChildItem -Recurse -File -LiteralPath (Join-Path $Repo ".mediagit\objects") |
    Where-Object { $_.FullName -match '\\objects\\[0-9a-f]{2}\\' } |
    Sort-Object Length -Descending | Select-Object -First 1)
}

function Flip-Byte([string]$Path) {
  $fs = [IO.File]::Open($Path, [IO.FileMode]::Open, [IO.FileAccess]::ReadWrite, [IO.FileShare]::None)
  try { $fs.Seek(500, [IO.SeekOrigin]::Begin) | Out-Null; $b = $fs.ReadByte(); $fs.Seek(500, [IO.SeekOrigin]::Begin) | Out-Null; $fs.WriteByte([byte](($b + 1) -band 255)) }
  finally { $fs.Dispose() }
}

# SAFE14 (BUG-002): a saved file whose data is gone from the store. fsck and verify
# used to say PERFECT / "All verifications passed"; only checkout noticed.
if (ShouldRun "SAFE14") {
  $r = New-ObjRepo "SAFE14"
  $base = Invoke-MG $r @("fsck", "--full") $Phase
  $obj = Get-BigObject $r
  Remove-Item -LiteralPath $obj.FullName -Force
  $f = Invoke-MG $r @("fsck", "--full") $Phase
  $v = Invoke-MG $r @("verify", "--checksums") $Phase
  $pass = ($base.Exit -eq 0) -and ($f.Exit -ne 0) -and ($f.Out -notmatch "integrity:\s*PERFECT") -and ($v.Exit -ne 0)
  Rec "SAFE14" "fsck/verify with a referenced object deleted" $pass ("baseline-fsck-exit={0} fsck-exit={1} verify-exit={2}" -f $base.Exit, $f.Exit, $v.Exit)
}

# SAFE15 (BUG-046): repair of a damaged object. It used to DELETE the object, after which
# fsck said PERFECT and the saved version was unrecoverable. With an intact working file
# the object must come back, and checkout from the store must work.
if (ShouldRun "SAFE15") {
  $r = New-ObjRepo "SAFE15"
  $want = Get-QaHash (Join-Path $r "big.bin")
  $obj = Get-BigObject $r
  Flip-Byte $obj.FullName
  $detect = Invoke-MG $r @("fsck", "--full") $Phase
  $rep = Invoke-MG $r @("fsck", "--repair") $Phase
  $after = Invoke-MG $r @("fsck", "--full") $Phase
  Remove-Item -LiteralPath (Join-Path $r "big.bin") -Force
  $co = Invoke-MG $r @("reset", "--hard", "HEAD") $Phase
  $got = if (Test-Path -LiteralPath (Join-Path $r "big.bin")) { Get-QaHash (Join-Path $r "big.bin") } else { "" }
  $pass = ($detect.Exit -ne 0) -and ($after.Exit -eq 0) -and ($co.Exit -eq 0) -and ($got -eq $want)
  Rec "SAFE15" "fsck --repair restores a damaged object from the working file" $pass ("detect-exit={0} repair-exit={1} after-exit={2} checkout-exit={3} bytes-back={4}" -f $detect.Exit, $rep.Exit, $after.Exit, $co.Exit, ($got -eq $want))
}

# SAFE16 (BUG-046, no good source): nothing can restore the object. Repair must not turn
# a visible fault into an invisible one: fsck has to keep failing, and the damaged bytes
# are quarantined, not destroyed.
if (ShouldRun "SAFE16") {
  $r = New-ObjRepo "SAFE16"
  $obj = Get-BigObject $r
  Flip-Byte $obj.FullName
  Remove-Item -LiteralPath (Join-Path $r "big.bin") -Force
  $rep = Invoke-MG $r @("fsck", "--repair") $Phase
  $after = Invoke-MG $r @("fsck", "--full") $Phase
  $ver = Invoke-MG $r @("verify") $Phase
  $q = Join-Path $r ".mediagit\quarantine"
  $kept = (Test-Path -LiteralPath $q) -and (@(Get-ChildItem -Recurse -File -LiteralPath $q).Count -ge 1)
  $pass = ($after.Exit -ne 0) -and ($after.Out -notmatch "integrity:\s*PERFECT") -and ($ver.Exit -ne 0) -and $kept
  Rec "SAFE16" "fsck --repair with no good source keeps failing and quarantines" $pass ("after-exit={0} verify-exit={1} quarantined={2}" -f $after.Exit, $ver.Exit, $kept)
}

# SAFE17 (BUG-041): under sparse checkout, `add --all` / `add -u` staged every excluded
# file as deleted, so the next commit dropped them from history. Two modes, same check.
foreach ($mode in @("--all", "-u")) {
  $id = "SAFE17" + $(if ($mode -eq "-u") { "b" } else { "a" })
  if (-not (ShouldRun $id)) { continue }
  $r = New-SandboxRepo $id $Phase
  Sf-Write $r "keep\k.txt" "k1"
  Sf-Write $r "big\b1.txt" "one"
  Sf-Write $r "big\b2.txt" "two"
  Invoke-MG $r @("add", "-A") $Phase | Out-Null
  Invoke-MG $r @("commit", "-m", "v1") $Phase | Out-Null
  $sp = Invoke-MG $r @("sparse-checkout", "set", "keep") $Phase
  $gone = -not (Test-Path -LiteralPath (Join-Path $r "big\b1.txt"))
  Sf-Write $r "keep\k.txt" "k2"
  $a = Invoke-MG $r @("add", $mode) $Phase
  $c = Invoke-MG $r @("commit", "-m", "v2") $Phase
  $d = Invoke-MG $r @("diff", "HEAD~1", "HEAD") $Phase
  $noDel = ($d.Out -notmatch "(?i)deleted") -and ($d.Out -match "k\.txt")
  Invoke-MG $r @("sparse-checkout", "disable") $Phase | Out-Null
  $back = (Test-Path -LiteralPath (Join-Path $r "big\b1.txt")) -and (Test-Path -LiteralPath (Join-Path $r "big\b2.txt")) -and
          ([IO.File]::ReadAllText((Join-Path $r "big\b2.txt")) -eq "two")
  $pass = ($sp.Exit -eq 0) -and $gone -and ($a.Exit -eq 0) -and ($c.Exit -eq 0) -and $noDel -and $back
  Rec $id "sparse-checkout then add $mode and commit keeps excluded files" $pass ("baseline-excluded-off-disk={0} add-exit={1} no-deletion-in-v2={2} restored-after-disable={3}" -f $gone, $a.Exit, $noDel, $back)
}

# SAFE18 (BUG-008): a path over 260 characters was staged as an absolute \\?\ path, the
# commit could not be checked out, and the whole checkout (including normal files) failed.
if (ShouldRun "SAFE18") {
  $r = New-SandboxRepo "SAFE18" $Phase
  Sf-Write $r "ok\plain.txt" "plain"
  $rel = "ok2"
  1..10 | ForEach-Object { $rel += "\seq_0123456789012345678901234567" }
  $lp = "\\?\" + (Join-Path $r $rel)
  $fileLp = $lp + "\final.txt"
  try {
    [IO.Directory]::CreateDirectory($lp) | Out-Null
    [IO.File]::WriteAllText($fileLp, "deep")
    $longOk = ((Join-Path $r $rel).Length + 10) -gt 260
    $a = Invoke-MG $r @("add", "-A") $Phase
    $c = Invoke-MG $r @("commit", "-m", "long") $Phase
    $st = Invoke-MG $r @("status", "--porcelain") $Phase
    $noAbs = ($st.Out -notmatch '//\?/') -and ($st.Out -notmatch '\\\\\?\\')
    Remove-Item -LiteralPath (Join-Path $r "ok\plain.txt") -Force
    [IO.File]::Delete($fileLp)
    $co = Invoke-MG $r @("reset", "--hard", "HEAD") $Phase
    $plainBack = (Test-Path -LiteralPath (Join-Path $r "ok\plain.txt"))
    $deepBack = [IO.File]::Exists($fileLp) -and ([IO.File]::ReadAllText($fileLp) -eq "deep")
    $pass = $longOk -and ($a.Exit -eq 0) -and ($c.Exit -eq 0) -and $noAbs -and ($co.Exit -eq 0) -and $plainBack -and $deepBack
    Rec "SAFE18" "path over 260 chars is stored relative and checks out" $pass ("path-over-260={0} add-exit={1} status-has-no-absolute-path={2} checkout-exit={3} plain-restored={4} deep-restored={5}" -f $longOk, $a.Exit, $noAbs, $co.Exit, $plainBack, $deepBack)
  }
  finally {
    # PowerShell 5.1 cannot Remove-Item a path this long; cmd with the \\?\ prefix can.
    & cmd.exe /c "rd /s /q `"\\?\$r`"" 2>&1 | Out-Null
  }
}

# SAFE19 (BUG-011): case-only rename (Hero.txt to hero.txt). add --all staged only the
# new name, the version held both, and it could not be checked out on a case-insensitive
# filesystem. Skipped where the filesystem is case-sensitive: the failure cannot occur there.
if (ShouldRun "SAFE19") {
  $r = New-SandboxRepo "SAFE19" $Phase
  Sf-Write $r "probe.TMP" "x"
  $insensitive = Test-Path -LiteralPath (Join-Path $r "probe.tmp")
  Remove-Item -LiteralPath (Join-Path $r "probe.TMP") -Force
  if (-not $insensitive) {
    Rec "SAFE19" "case-only rename" "SKIP" "filesystem is case-sensitive"
  }
  else {
    Sf-Write $r "art\Hero.txt" "hero"
    Sf-Write $r "keep.txt" "keep"
    Invoke-MG $r @("add", "-A") $Phase | Out-Null
    Invoke-MG $r @("commit", "-m", "one") $Phase | Out-Null
    Rename-Item -LiteralPath (Join-Path $r "art\Hero.txt") -NewName "tmp.txt"
    Rename-Item -LiteralPath (Join-Path $r "art\tmp.txt") -NewName "hero.txt"
    $a = Invoke-MG $r @("add", "-A") $Phase
    $c = Invoke-MG $r @("commit", "-m", "rename") $Phase
    $show = Invoke-MG $r @("show", "HEAD") $Phase
    $one = ($show.Out -match "hero\.txt\s*\|\s*new file") -and ($show.Out -match "Hero\.txt\s*\|\s*deleted")
    Remove-Item -LiteralPath (Join-Path $r "keep.txt") -Force
    $co = Invoke-MG $r @("reset", "--hard", "HEAD") $Phase
    $names = @(Get-ChildItem -LiteralPath (Join-Path $r "art") -Name)
    $exact = ($names.Count -eq 1) -and ($names[0] -ceq "hero.txt")
    $keepBack = Test-Path -LiteralPath (Join-Path $r "keep.txt")
    $fs = Invoke-MG $r @("fsck", "--full") $Phase
    $pass = ($a.Exit -eq 0) -and ($c.Exit -eq 0) -and $one -and ($co.Exit -eq 0) -and $exact -and $keepBack -and ($fs.Exit -eq 0)
    Rec "SAFE19" "case-only rename stores one file and checks out" $pass ("rename-is-new+deleted={0} checkout-exit={1} exact-case-hero.txt={2} unrelated-file-restored={3} fsck-exit={4}" -f $one, $co.Exit, $exact, $keepBack, $fs.Exit)
  }
}

# --- final fix pass: guards on operations that could lose uncommitted work ----------
# Two branches off one base. feat always adds g.txt. With -Conflict both sides edit f.txt
# (merge stops); without it main adds m.txt (merge is clean).
function New-ForkRepo([string]$Name, [switch]$Conflict) {
  $r = New-SandboxRepo $Name $Phase
  Sf-Write $r "f.txt" "base"
  Invoke-MG $r @("add", "-A") $Phase | Out-Null
  Invoke-MG $r @("commit", "-m", "base") $Phase | Out-Null
  Invoke-MG $r @("branch", "create", "feat") $Phase | Out-Null
  Invoke-MG $r @("branch", "switch", "feat") $Phase | Out-Null
  Sf-Write $r "g.txt" "from-feat"
  if ($Conflict) { Sf-Write $r "f.txt" "feat-side" }
  Invoke-MG $r @("add", "-A") $Phase | Out-Null
  Invoke-MG $r @("commit", "-m", "feat") $Phase | Out-Null
  Invoke-MG $r @("branch", "switch", "main") $Phase | Out-Null
  if ($Conflict) { Sf-Write $r "f.txt" "main-side" } else { Sf-Write $r "m.txt" "from-main" }
  Invoke-MG $r @("add", "-A") $Phase | Out-Null
  Invoke-MG $r @("commit", "-m", "main") $Phase | Out-Null
  return $r
}

function Get-HeadId([string]$Repo) {
  return ((Invoke-MG $Repo @("log", "--oneline", "-n", "1") $Phase).Out.Trim() -split "\s+")[0]
}

# SAFE20: a staged NEW file is carried across a branch switch like git does; it must
# survive on disk and stay staged.
if (ShouldRun "SAFE20") {
  $r = New-ForkRepo "SAFE20"
  Sf-Write $r "n.txt" "mine"
  Invoke-MG $r @("add", "n.txt") $Phase | Out-Null
  $sw = Invoke-MG $r @("branch", "switch", "feat") $Phase
  $st = Invoke-MG $r @("status", "--porcelain") $Phase
  $kept = (Test-Path -LiteralPath (Join-Path $r "n.txt")) -and ([IO.File]::ReadAllText((Join-Path $r "n.txt")) -eq "mine")
  $staged = $st.Out -match '(?m)^A\s+n\.txt'
  $onFeat = Test-Path -LiteralPath (Join-Path $r "g.txt")
  $pass = ($sw.Exit -eq 0) -and $kept -and $staged -and $onFeat
  Rec "SAFE20" "branch switch carries a staged new file" $pass ("exit={0} file-kept={1} still-staged={2} switched={3}" -f $sw.Exit, $kept, $staged, $onFeat)
}

# SAFE21: a DIFFERENT staged file at a path the target branch also has must refuse the
# switch (would_overwrite) and leave the staged bytes, the branch and HEAD alone.
if (ShouldRun "SAFE21") {
  $r = New-ForkRepo "SAFE21"
  $head = Get-HeadId $r
  Sf-Write $r "g.txt" "mine-not-feats"
  Invoke-MG $r @("add", "g.txt") $Phase | Out-Null
  $sw = Invoke-MG $r @("branch", "switch", "feat") $Phase
  $intact = [IO.File]::ReadAllText((Join-Path $r "g.txt")) -eq "mine-not-feats"
  $still = (Get-HeadId $r) -eq $head
  $br = (Invoke-MG $r @("branch") $Phase).Out
  $onMain = $br -match '\*\s*main'
  $pass = ($sw.Exit -ne 0) -and $intact -and $still -and $onMain
  Rec "SAFE21" "branch switch refuses to overwrite a different staged file" $pass ("exit={0} staged-bytes-intact={1} head-unchanged={2} still-on-main={3}" -f $sw.Exit, $intact, $still, $onMain)
}

# SAFE22: merge refuses to START with a staged change (it would be folded into the merge).
if (ShouldRun "SAFE22") {
  $r = New-ForkRepo "SAFE22"
  $head = Get-HeadId $r
  Sf-Write $r "s.txt" "staged"
  Invoke-MG $r @("add", "s.txt") $Phase | Out-Null
  $m = Invoke-MG $r @("merge", "feat") $Phase
  $intact = (Test-Path -LiteralPath (Join-Path $r "s.txt")) -and ([IO.File]::ReadAllText((Join-Path $r "s.txt")) -eq "staged")
  $noMerge = (Get-HeadId $r) -eq $head -and -not (Test-Path -LiteralPath (Join-Path $r ".mediagit\MERGE_HEAD")) -and -not (Test-Path -LiteralPath (Join-Path $r "g.txt"))
  $pass = ($m.Exit -ne 0) -and $intact -and $noMerge
  Rec "SAFE22" "merge refuses with a staged change" $pass ("exit={0} staged-file-intact={1} nothing-merged={2}" -f $m.Exit, $intact, $noMerge)
}

# SAFE23: `commit` during a merge stopped on conflict must refuse; it used to record the
# half-merged index as an ordinary commit and lose the merge state.
if (ShouldRun "SAFE23") {
  $r = New-ForkRepo "SAFE23" -Conflict
  $head = Get-HeadId $r
  $m = Invoke-MG $r @("merge", "feat") $Phase
  $stopped = Test-Path -LiteralPath (Join-Path $r ".mediagit\MERGE_HEAD")
  $c = Invoke-MG $r @("commit", "-m", "should-be-refused") $Phase
  $still = (Get-HeadId $r) -eq $head
  $keepsState = Test-Path -LiteralPath (Join-Path $r ".mediagit\MERGE_HEAD")
  $ab = Invoke-MG $r @("merge", "--abort") $Phase
  $pass = $stopped -and ($c.Exit -ne 0) -and $still -and $keepsState -and ($ab.Exit -eq 0)
  Rec "SAFE23" "commit during a stopped merge is refused" $pass ("merge-stopped={0} commit-exit={1} head-unchanged={2} merge-state-kept={3} abort-exit={4}" -f $stopped, $c.Exit, $still, $keepsState, $ab.Exit)
}

# SAFE24: `reset` takes only plain revisions. HEAD^^, HEAD^2 and a bare HEAD~ used to be
# read as "first parent" and moved HEAD to a commit the user did not name. --hard here, so
# the old behaviour also threw away the working tree.
if (ShouldRun "SAFE24") {
  $r = New-ForkRepo "SAFE24"
  Sf-Write $r "w.txt" "work"
  Invoke-MG $r @("add", "w.txt") $Phase | Out-Null
  Invoke-MG $r @("commit", "-m", "third") $Phase | Out-Null
  $head = Get-HeadId $r
  $bad = @()
  foreach ($rev in @('HEAD^^', 'HEAD^2', 'HEAD~')) {
    $x = Invoke-MG $r @("reset", "--hard", $rev) $Phase
    if (($x.Exit -eq 0) -or ((Get-HeadId $r) -ne $head) -or -not (Test-Path -LiteralPath (Join-Path $r "w.txt"))) { $bad += $rev }
  }
  Rec "SAFE24" "reset refuses HEAD^^ / HEAD^2 / HEAD~ and changes nothing" ($bad.Count -eq 0) ("accepted-or-moved=[{0}]" -f ($bad -join ","))
}

# SAFE25: merge --no-commit leaves a pending merge that `merge --continue` finishes
# (it used to dead-end: commit was the only way and refused/lost the second parent).
if (ShouldRun "SAFE25") {
  $r = New-ForkRepo "SAFE25"
  $head = Get-HeadId $r
  $m = Invoke-MG $r @("merge", "--no-commit", "feat") $Phase
  $pending = Test-Path -LiteralPath (Join-Path $r ".mediagit\MERGE_HEAD")
  $mid = (Get-HeadId $r) -eq $head
  $cont = Invoke-MG $r @("merge", "--continue") $Phase
  $done = ((Get-HeadId $r) -ne $head) -and -not (Test-Path -LiteralPath (Join-Path $r ".mediagit\MERGE_HEAD"))
  $files = (Test-Path -LiteralPath (Join-Path $r "g.txt")) -and (Test-Path -LiteralPath (Join-Path $r "m.txt"))
  $pass = ($m.Exit -eq 0) -and $pending -and $mid -and ($cont.Exit -eq 0) -and $done -and $files
  Rec "SAFE25" "merge --no-commit is finished by merge --continue" $pass ("no-commit-exit={0} pending={1} head-held={2} continue-exit={3} merge-commit-made={4} both-sides-present={5}" -f $m.Exit, $pending, $mid, $cont.Exit, $done, $files)
}

# --- summary -----------------------------------------------------------------

# Exit-QaPhase, not a hand-rolled exit. This was the ONLY phase script in the
# suite that rolled its own, and it silently dropped the protection every
# sibling gets: Exit-QaPhase treats "no gates recorded" as NOTHING-VERIFIED
# (common.ps1:425), whereas `$script:AllPass` only flips on an actual FAIL and
# so stays at its initial $true when nothing ran at all.
#
# Failing input that used to report success: `-Only zzz-matches-nothing`. Every
# check sits behind ShouldRun, `Rec` never fires, and the phase printed
# "PHASE PASS - no untracked work destroyed" and exited 0 having verified
# nothing. In the one phase whose entire purpose is catching silent data loss.
#
# Not reachable from a campaign today - run_all.ps1 never threads -Only through
# to a phase - so this is a latent trap being closed before it becomes one,
# not an active hole in any past result.
#
# $AllPass is still passed as ExtraFail so a recorded FAIL still fails the
# phase even if the gate rows alone would not say so.
Write-QaLog $Phase $(if ($script:AllPass) { "no untracked work destroyed" } else { "FAILURES - see $TSV" })
Exit-QaPhase $Phase (-not $script:AllPass)
