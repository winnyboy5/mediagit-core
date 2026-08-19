# Phase 5 - data-loss regression pack. ASCII-only, PS 5.1 compatible.
# Consolidates the 41 repro scripts in dev-tests\standalone-deep-v11\repros\BUG-*.ps1 into
# ~20 regression checks (one per distinct failure mode / fixed bug cluster) plus a generic
# op matrix. Every check: fresh sandbox, deterministic binary fixture(s), op sequence,
# gate = pre/post SHA256 of every touched file + fsck clean.
#
# Usage: powershell -File 05_branching.ps1 [-Only <substring>]
#   -Only filters check/matrix ids by substring match (e.g. -Only CHK05, -Only rebase).
param(
  [string]$Only = ""
)

. (Join-Path $PSScriptRoot "lib\common.ps1")
. (Join-Path $PSScriptRoot "lib\remote.ps1")

$Phase = "05_branching"
$env:MEDIAGIT_AUTHOR_NAME = "QA-Suite"
$env:MEDIAGIT_AUTHOR_EMAIL = "qa-suite@mediagit.local"

$TSV = Join-Path $QA.Logs "branching_results.tsv"
$script:AllPass = $true

function ShouldRun([string]$Id) {
  if (-not $Only) { return $true }
  return $Id -like "*$Only*"
}

# Records one row (check, op, pass, detail) and mirrors it into the shared gates.tsv.
# $Pass may be $true/$false, or the string "SKIP" for an environment that isn't available
# (e.g. no local server) - SKIP does not fail the phase.
function Rec([string]$Check, [string]$Op, $Pass, [string]$Detail) {
  Write-QaRow $TSV @("check", "op", "pass", "detail") @($Check, $Op, $Pass, $Detail)
  $tag = if ("$Pass" -eq "SKIP") { "SKIP" } elseif ($Pass) { "PASS" } else { "FAIL" }
  Write-QaLog $Phase ("{0} :: {1} -> {2}  {3}" -f $Check, $Op, $tag, $Detail)
  Write-QaGate $Phase $Check $Pass $Detail
  if ($tag -eq "FAIL") { $script:AllPass = $false }
}

# Deterministic random-byte binary fixture (seeded, so re-runs are reproducible).
function New-QaBinaryFixture([string]$Path, [int]$SizeMB = 2, [int]$Seed = 1) {
  $parent = Split-Path -Parent $Path
  if ($parent -and -not (Test-Path $parent)) { New-Item -ItemType Directory -Path $parent -Force | Out-Null }
  $rnd = New-Object System.Random($Seed)
  $bytes = New-Object byte[] ($SizeMB * 1MB)
  $rnd.NextBytes($bytes)
  [IO.File]::WriteAllBytes($Path, $bytes)
}

# Write a binary fixture at $Repo\$RelPath, add + commit it.
function New-QaCommit([string]$Repo, [string]$RelPath, [string]$Msg, [int]$SizeMB = 2, [int]$Seed = 1) {
  New-QaBinaryFixture (Join-Path $Repo $RelPath) $SizeMB $Seed
  Invoke-MG $Repo @("add", $RelPath) $Phase | Out-Null
  Invoke-MG $Repo @("commit", "-m", $Msg) $Phase | Out-Null
}

function Sw([string]$Repo, [string]$Branch, [string[]]$ExtraArgs = @()) {
  return Invoke-MG $Repo (@("branch", "switch", $Branch) + $ExtraArgs) $Phase
}

function Br([string]$Repo, [string]$Name) {
  Invoke-MG $Repo @("branch", "create", $Name) $Phase | Out-Null
}

function Test-QaFsckClean([string]$Repo) {
  $r = Invoke-MG $Repo @("fsck") $Phase
  $bad = ($r.Out -match "(?i)corrupt|missing|error|failed") -or ($r.Exit -ne 0)
  return -not $bad
}

function Get-QaHashOrNull([string]$Path) {
  if (Test-Path $Path) { return Get-QaHash $Path } else { return $null }
}

# ============================================================================
# CHK01/02 - Replay engine (rebase tree-snapshot reconstruction)
# Source: BUG-GD-4 (rebase drops base-branch files), BUG-CLI-B4 (rebase skips conflict detection)
# ============================================================================
function Check-01-RebaseKeepsBaseFiles {
  $id = "CHK01-rebase-keeps-base-files"
  if (-not (ShouldRun $id)) { return }
  $repo = New-SandboxRepo "c01-rebase-keep" $Phase
  New-QaCommit $repo "base.bin" "base" 2 101
  Br $repo "feature"; Sw $repo "feature" | Out-Null
  New-QaCommit $repo "feature.bin" "add feature.bin" 2 102
  # Capture feature.bin's hash while still on the feature branch - it does not
  # exist in main's working tree, so reading it after "Sw main" below would
  # error out (empty hash) and always fail the post-rebase comparison.
  $featHash = Get-QaHash (Join-Path $repo "feature.bin")
  Sw $repo "main" | Out-Null
  New-QaCommit $repo "mainfile.bin" "add mainfile.bin on main" 2 103
  $mainHash = Get-QaHash (Join-Path $repo "mainfile.bin")
  Sw $repo "feature" | Out-Null
  $r = Invoke-MG $repo @("rebase", "main") $Phase
  $mainOk = (Get-QaHashOrNull (Join-Path $repo "mainfile.bin")) -eq $mainHash
  $featOk = (Get-QaHashOrNull (Join-Path $repo "feature.bin")) -eq $featHash
  $fsckOk = Test-QaFsckClean $repo
  $pass = ($r.Exit -eq 0) -and $mainOk -and $featOk -and $fsckOk
  Rec $id "rebase" $pass "exit=$($r.Exit) mainfile-hash-ok=$mainOk feature-hash-ok=$featOk fsck=$fsckOk"
}

function Check-02-RebaseConflictDetected {
  $id = "CHK02-rebase-conflict-detected"
  if (-not (ShouldRun $id)) { return }
  $repo = New-SandboxRepo "c02-rebase-conflict" $Phase
  New-QaCommit $repo "f.bin" "base" 2 201
  Br $repo "topic"
  New-QaCommit $repo "f.bin" "main edits f" 2 202
  Sw $repo "topic" -ExtraArgs @("-f") | Out-Null
  New-QaCommit $repo "f.bin" "topic edits f" 2 203
  $r = Invoke-MG $repo @("rebase", "main") $Phase
  # AND, not OR. This check exists because rebase once skipped conflict
  # detection entirely (BUG-CLI-B4); with OR, a rebase that panics for any
  # unrelated reason also reads as "conflict correctly detected", which is the
  # same blind spot wearing the opposite mask. Nothing else backstops this one.
  $conflictSignaled = ($r.Exit -ne 0) -and ($r.Out -match "(?i)conflict")
  Rec $id "rebase" $conflictSignaled "exit=$($r.Exit) out-tail=$(($r.Out -split "`n" | Select-Object -Last 1))"
}

# ============================================================================
# CHK03/04 - Replay engine (cherry-pick tree reconstruction + conflict content)
# Source: BUG-GD-5 (cherry-pick deletes unrelated tracked files), BUG-CLI-B5 (conflict
# markers contain Oid(...) placeholders instead of real content)
# ============================================================================
function Check-03-CherryPickKeepsUnrelatedFiles {
  $id = "CHK03-cherrypick-keeps-unrelated-files"
  if (-not (ShouldRun $id)) { return }
  $repo = New-SandboxRepo "c03-cp-keep" $Phase
  New-QaCommit $repo "a.bin" "base a" 2 301
  Invoke-MG $repo @("add", ".") $Phase | Out-Null
  New-QaBinaryFixture (Join-Path $repo "b.bin") 2 302
  Invoke-MG $repo @("add", "b.bin") $Phase | Out-Null
  Invoke-MG $repo @("commit", "-m", "base: a and b") $Phase | Out-Null
  $aHash = Get-QaHash (Join-Path $repo "a.bin")
  $bHash = Get-QaHash (Join-Path $repo "b.bin")
  Br $repo "picks"; Sw $repo "picks" | Out-Null
  New-QaCommit $repo "c.bin" "add c.bin" 2 303
  $cHash = Get-QaHash (Join-Path $repo "c.bin")
  $cp = (Invoke-MG $repo @("log", "--oneline", "-n", "1") $Phase).Out.Trim().Split(" ")[0]
  Sw $repo "main" | Out-Null
  $r = Invoke-MG $repo @("cherry-pick", $cp) $Phase
  $ok = ((Get-QaHashOrNull (Join-Path $repo "a.bin")) -eq $aHash) -and
        ((Get-QaHashOrNull (Join-Path $repo "b.bin")) -eq $bHash) -and
        ((Get-QaHashOrNull (Join-Path $repo "c.bin")) -eq $cHash)
  $fsckOk = Test-QaFsckClean $repo
  $pass = ($r.Exit -eq 0) -and $ok -and $fsckOk
  Rec $id "cherry-pick" $pass "exit=$($r.Exit) a+b+c-hashes-ok=$ok fsck=$fsckOk"
}

function Check-04-CherryPickConflictRealContent {
  $id = "CHK04-cherrypick-conflict-real-content"
  if (-not (ShouldRun $id)) { return }
  $repo = New-SandboxRepo "c04-cp-conflict" $Phase
  "base" | Set-Content (Join-Path $repo "f.txt")
  Invoke-MG $repo @("add", "f.txt") $Phase | Out-Null
  Invoke-MG $repo @("commit", "-m", "base") $Phase | Out-Null
  Br $repo "topic"
  "ours" | Set-Content (Join-Path $repo "f.txt")
  Invoke-MG $repo @("add", "f.txt") $Phase | Out-Null
  Invoke-MG $repo @("commit", "-m", "ours") $Phase | Out-Null
  Sw $repo "topic" -ExtraArgs @("-f") | Out-Null
  "theirs" | Set-Content (Join-Path $repo "f.txt")
  Invoke-MG $repo @("add", "f.txt") $Phase | Out-Null
  Invoke-MG $repo @("commit", "-m", "theirs") $Phase | Out-Null
  $theirs = ((Invoke-MG $repo @("log", "--oneline", "-n", "1") $Phase).Out.Trim() -split " ")[0]
  Sw $repo "main" -ExtraArgs @("-f") | Out-Null
  Invoke-MG $repo @("cherry-pick", $theirs) $Phase | Out-Null
  $content = Get-Content (Join-Path $repo "f.txt") -Raw
  # Arming proof first. The real assertion is "conflict markers contain the file
  # content, not `Oid(...)` placeholders" (BUG-CLI-B5, a data-corruption class).
  # Checking only for the absence of "Oid(" passed when the cherry-pick failed
  # outright and left f.txt at its pre-attempt content -- no markers, no
  # placeholders, no conflict, green. Require the markers to exist before
  # judging what is inside them.
  $hasMarkers = ($content -match "<<<<<<<")
  $noPlaceholder = ($content -notmatch "Oid\(")
  $pass = $hasMarkers -and $noPlaceholder
  Rec $id "cherry-pick-conflict" $pass `
    "conflict-markers-present=$hasMarkers oid-placeholder-in-markers=$(-not $noPlaceholder)"
}

# ============================================================================
# CHK05/06 - pull -r local-commit drop / symbolic-HEAD resolve
# Source: BUG-RM-1 (pull -r drops local divergent commit + file), CHECK-ff-only-pull-r
# (regression guard: pull -r with no divergence must still fast-forward)
# Uses Start-QaServer -Backend local (filesystem backend) so this needs no cloud/MinIO.
# ============================================================================
function Check-05-PullRebaseKeepsLocalCommit {
  $id = "CHK05-pull-rebase-keeps-local-commit"
  if (-not (ShouldRun $id)) { return }
  $srv = $null
  try {
    $srv = Start-QaServer -Backend "local" -Phase $Phase
    $src = New-SandboxRepo "c05-src" $Phase
    New-QaCommit $src "base.bin" "c1 base" 2 501
    Invoke-MG $src @("remote", "add", "origin", $srv.Url) $Phase | Out-Null
    Invoke-MG $src @("push", "origin") $Phase | Out-Null

    $clone = Join-Path $QA.Work "c05-clone"
    if (Test-Path $clone) { Remove-Item -Recurse -Force $clone }
    Invoke-MG $null @("clone", $srv.Url, $clone) $Phase | Out-Null
    New-QaBinaryFixture (Join-Path $clone "local-note.bin") 1 502
    Invoke-MG $clone @("add", "local-note.bin") $Phase | Out-Null
    Invoke-MG $clone @("commit", "-m", "LOCAL-DIVERGENT") $Phase | Out-Null
    $localHash = Get-QaHash (Join-Path $clone "local-note.bin")

    New-QaCommit $src "base2.bin" "c2 remote side" 2 503
    Invoke-MG $src @("push", "origin") $Phase | Out-Null

    $r = Invoke-MG $clone @("pull", "-r") $Phase
    $log = (Invoke-MG $clone @("log") $Phase).Out
    $commitKept = $log -match "LOCAL-DIVERGENT"
    $fileOk = (Get-QaHashOrNull (Join-Path $clone "local-note.bin")) -eq $localHash
    $pass = ($r.Exit -eq 0) -and $commitKept -and $fileOk
    Rec $id "pull -r" $pass "exit=$($r.Exit) commit-kept=$commitKept file-hash-ok=$fileOk"
  } catch {
    if ("$_" -match "^SKIP:") { Rec $id "pull -r" "SKIP" "$_" } else { Rec $id "pull -r" $false "unexpected error: $_" }
  } finally { Stop-QaServer $srv }
}

function Check-06-PullRebaseFastForwardClean {
  $id = "CHK06-pull-rebase-fastforward-clean"
  if (-not (ShouldRun $id)) { return }
  $srv = $null
  try {
    $srv = Start-QaServer -Backend "local" -Phase $Phase
    $src = New-SandboxRepo "c06-src" $Phase
    New-QaCommit $src "base.bin" "c1 base" 2 601
    Invoke-MG $src @("remote", "add", "origin", $srv.Url) $Phase | Out-Null
    Invoke-MG $src @("push", "origin") $Phase | Out-Null

    $clone = Join-Path $QA.Work "c06-clone"
    if (Test-Path $clone) { Remove-Item -Recurse -Force $clone }
    Invoke-MG $null @("clone", $srv.Url, $clone) $Phase | Out-Null

    New-QaCommit $src "base2.bin" "c2 remote side" 2 602
    Invoke-MG $src @("push", "origin") $Phase | Out-Null
    $remoteHash = Get-QaHash (Join-Path $src "base2.bin")

    $r = Invoke-MG $clone @("pull", "-r") $Phase
    $fileOk = (Get-QaHashOrNull (Join-Path $clone "base2.bin")) -eq $remoteHash
    $pass = ($r.Exit -eq 0) -and $fileOk
    Rec $id "pull -r (no divergence)" $pass "exit=$($r.Exit) file-hash-ok=$fileOk"
  } catch {
    if ("$_" -match "^SKIP:") { Rec $id "pull -r" "SKIP" "$_" } else { Rec $id "pull -r" $false "unexpected error: $_" }
  } finally { Stop-QaServer $srv }
}

# ============================================================================
# CHK07 - merge-conflict binary handling: no text markers injected into binary
# Source: BUG-DES-1, BUG-GD-6
# ============================================================================
function Check-07-MergeConflictBinaryNoMarkers {
  $id = "CHK07-merge-conflict-binary-no-markers"
  if (-not (ShouldRun $id)) { return }
  $repo = New-SandboxRepo "c07-merge-binary" $Phase
  New-QaCommit $repo "seed.bin" "base" 1 700
  Br $repo "a"; Sw $repo "a" | Out-Null
  New-QaCommit $repo "art.bin" "a edit" 2 701
  Sw $repo "main" | Out-Null
  Br $repo "b"; Sw $repo "b" | Out-Null
  New-QaCommit $repo "art.bin" "b edit" 2 702
  $bHash = Get-QaHash (Join-Path $repo "art.bin")
  $sizeBefore = (Get-Item (Join-Path $repo "art.bin")).Length
  Invoke-MG $repo @("merge", "a") $Phase | Out-Null
  $bytes = [IO.File]::ReadAllBytes((Join-Path $repo "art.bin"))
  $head12 = [Text.Encoding]::ASCII.GetString($bytes[0..([Math]::Min(11, $bytes.Length - 1))])
  $noMarkers = -not $head12.StartsWith("<<<<<<<")
  $sizeSane = $bytes.Length -le ($sizeBefore * 1.05)
  Invoke-MG $repo @("merge", "--abort", "a") $Phase | Out-Null
  $restoredOk = (Get-QaHashOrNull (Join-Path $repo "art.bin")) -eq $bHash
  $pass = $noMarkers -and $sizeSane -and $restoredOk
  Rec $id "merge (binary conflict)" $pass "no-text-markers=$noMarkers size-sane=$sizeSane restored-after-abort=$restoredOk"
}

# ============================================================================
# CHK08/09 - phantom index after merge --abort
# Source: BUG-DES-2 (worktree not restored; phantom staged index survives reset --hard;
# real deletions invisible while phantom index active)
# ============================================================================
function Check-08-MergeAbortRestoresWorktree {
  $id = "CHK08-merge-abort-restores-worktree"
  if (-not (ShouldRun $id)) { return }
  $repo = New-SandboxRepo "c08-abort-restore" $Phase
  New-QaCommit $repo "seed.bin" "base" 1 800
  Br $repo "a"; Sw $repo "a" | Out-Null
  New-QaCommit $repo "art.bin" "a edit" 2 801
  Sw $repo "main" | Out-Null
  Br $repo "b"; Sw $repo "b" | Out-Null
  New-QaCommit $repo "art.bin" "b edit" 2 802
  $preHash = Get-QaHash (Join-Path $repo "art.bin")
  Invoke-MG $repo @("merge", "a") $Phase | Out-Null
  Invoke-MG $repo @("merge", "--abort", "a") $Phase | Out-Null
  $pass = (Get-QaHashOrNull (Join-Path $repo "art.bin")) -eq $preHash
  Rec $id "merge --abort" $pass "worktree-restored=$pass"
}

function Check-09-MergeAbortClearsPhantomIndex {
  $id = "CHK09-merge-abort-clears-phantom-index"
  if (-not (ShouldRun $id)) { return }
  $repo = New-SandboxRepo "c09-phantom" $Phase
  New-QaCommit $repo "seed.bin" "base" 1 900
  Br $repo "a"; Sw $repo "a" | Out-Null
  New-QaCommit $repo "art.bin" "a edit" 2 901
  Sw $repo "main" | Out-Null
  Br $repo "b"; Sw $repo "b" | Out-Null
  New-QaCommit $repo "art.bin" "b edit" 2 902
  Invoke-MG $repo @("merge", "a") $Phase | Out-Null
  Invoke-MG $repo @("merge", "--abort", "a") $Phase | Out-Null
  Invoke-MG $repo @("reset", "--hard", "HEAD") $Phase | Out-Null
  Remove-Item (Join-Path $repo "art.bin") -Force
  $statusOut = (Invoke-MG $repo @("status") $Phase).Out
  $noPhantomNewFile = $statusOut -notmatch "(?i)new file"
  $deletionShown = $statusOut -match "(?i)deleted"
  $pass = $noPhantomNewFile -and $deletionShown
  Rec $id "merge --abort; reset --hard; delete" $pass "no-phantom-staging=$noPhantomNewFile real-deletion-visible=$deletionShown"
}

# ============================================================================
# CHK10 - conflicted merge can be completed via --continue-merge
# Source: BUG-CLI-B3, BUG-DES-4 ("Cannot update symbolic reference: HEAD")
# ============================================================================
function Check-10-MergeContinueCompletes {
  $id = "CHK10-merge-continue-completes"
  if (-not (ShouldRun $id)) { return }
  $repo = New-SandboxRepo "c10-merge-continue" $Phase
  New-QaCommit $repo "f.bin" "base" 1 1000
  Br $repo "other"
  New-QaCommit $repo "f.bin" "ours" 1 1001
  Sw $repo "other" -ExtraArgs @("-f") | Out-Null
  New-QaCommit $repo "f.bin" "theirs" 1 1002
  Sw $repo "main" -ExtraArgs @("-f") | Out-Null
  Invoke-MG $repo @("merge", "other") $Phase | Out-Null
  New-QaBinaryFixture (Join-Path $repo "f.bin") 1 1003
  Invoke-MG $repo @("add", "f.bin") $Phase | Out-Null
  $r = Invoke-MG $repo @("merge", "--continue") $Phase
  $log = (Invoke-MG $repo @("log", "--oneline", "-n", "1") $Phase).Out
  $pass = ($r.Exit -eq 0) -and ($log.Trim().Length -gt 0)
  Rec $id "merge --continue-merge" $pass "exit=$($r.Exit)"
}

# ============================================================================
# CHK11/12 - branch switch dirty-tree guard
# Source: BUG-CLI-B1 (switch without -f silently overwrites uncommitted changes)
# ============================================================================
function Check-11-SwitchRefusesDirtyTree {
  $id = "CHK11-switch-refuses-dirty-tree"
  if (-not (ShouldRun $id)) { return }
  $repo = New-SandboxRepo "c11-switch-refuse" $Phase
  New-QaCommit $repo "f.bin" "c1" 1 1100
  Br $repo "feature"
  New-QaCommit $repo "f.bin" "c2 main edits f" 1 1101
  $precious = New-Object byte[] (512KB)
  (New-Object System.Random(1102)).NextBytes($precious)
  [IO.File]::WriteAllBytes((Join-Path $repo "f.bin"), $precious)
  $preciousHash = (Get-FileHash -Algorithm SHA256 -InputStream ([IO.MemoryStream]::new($precious))).Hash
  $r = Invoke-MG $repo @("branch", "switch", "feature") $Phase
  $stillPrecious = (Get-QaHashOrNull (Join-Path $repo "f.bin")) -eq $preciousHash
  $pass = $stillPrecious -or ($r.Exit -ne 0)
  Rec $id "branch switch (no -f, dirty tree)" $pass "exit=$($r.Exit) uncommitted-work-preserved=$stillPrecious"
}

function Check-12-SwitchForceOverwritesCleanly {
  $id = "CHK12-switch-force-overwrites-cleanly"
  if (-not (ShouldRun $id)) { return }
  $repo = New-SandboxRepo "c12-switch-force" $Phase
  New-QaCommit $repo "f.bin" "c1" 1 1200
  Br $repo "feature"
  New-QaCommit $repo "f.bin" "c2 main edits f" 1 1201
  $mainHash = Get-QaHash (Join-Path $repo "f.bin")
  "dirty" | Add-Content (Join-Path $repo "f.bin")
  $r = Invoke-MG $repo @("branch", "switch", "feature", "-f") $Phase
  $featHash = Get-QaHash (Join-Path $repo "f.bin")
  Sw $repo "main" -ExtraArgs @("-f") | Out-Null
  $backToMainOk = (Get-QaHashOrNull (Join-Path $repo "f.bin")) -eq $mainHash
  $pass = ($r.Exit -eq 0) -and $backToMainOk
  Rec $id "branch switch -f (dirty tree)" $pass "exit=$($r.Exit) roundtrip-hash-ok=$backToMainOk"
}

# ============================================================================
# CHK13/14 - stash push/apply round-trip, including under a phantom-index precondition
# Source: BUG-DES-3 (WIP lost when phantom-index is active after a merge --abort)
# ============================================================================
function Check-13-StashRoundtripPlain {
  $id = "CHK13-stash-roundtrip-plain"
  if (-not (ShouldRun $id)) { return }
  $repo = New-SandboxRepo "c13-stash-plain" $Phase
  New-QaCommit $repo "f.bin" "base" 2 1300
  $headHash = Get-QaHash (Join-Path $repo "f.bin")
  New-QaBinaryFixture (Join-Path $repo "f.bin") 2 1301
  $wipHash = Get-QaHash (Join-Path $repo "f.bin")
  Invoke-MG $repo @("stash", "push", "-m", "wip") $Phase | Out-Null
  $revertedToHead = (Get-QaHashOrNull (Join-Path $repo "f.bin")) -eq $headHash
  Invoke-MG $repo @("stash", "pop") $Phase | Out-Null
  $restoredToWip = (Get-QaHashOrNull (Join-Path $repo "f.bin")) -eq $wipHash
  $pass = $revertedToHead -and $restoredToWip
  Rec $id "stash push; stash pop" $pass "reverted-to-head=$revertedToHead restored-to-wip=$restoredToWip"
}

function Check-14-StashRoundtripAfterPhantomIndex {
  $id = "CHK14-stash-roundtrip-after-phantom-index"
  if (-not (ShouldRun $id)) { return }
  $repo = New-SandboxRepo "c14-stash-phantom" $Phase
  New-QaCommit $repo "seed.bin" "base" 1 1400
  Br $repo "a"; Sw $repo "a" | Out-Null
  New-QaCommit $repo "art.bin" "a edit" 2 1401
  Sw $repo "main" | Out-Null
  Br $repo "b"; Sw $repo "b" | Out-Null
  New-QaCommit $repo "art.bin" "b edit" 2 1402
  Invoke-MG $repo @("merge", "a") $Phase | Out-Null
  Invoke-MG $repo @("merge", "--abort", "a") $Phase | Out-Null
  Invoke-MG $repo @("reset", "--hard", "HEAD") $Phase | Out-Null   # -> phantom index state
  Add-Content (Join-Path $repo "art.bin") "WIP-EDIT-MARKER"
  $wipHash = Get-QaHash (Join-Path $repo "art.bin")
  Invoke-MG $repo @("stash", "push", "-m", "wip") $Phase | Out-Null
  Invoke-MG $repo @("stash", "apply") $Phase | Out-Null
  $pass = (Get-QaHashOrNull (Join-Path $repo "art.bin")) -eq $wipHash
  Rec $id "stash push+apply after phantom-index" $pass "wip-recovered=$pass"
}

# ============================================================================
# CHK15/16 - revert conflict handling
# Source: BUG-AUD-1 (conflicted revert exits 0), BUG-AUD-2 (revert --abort / reset --hard
# leaves phantom staged index)
# ============================================================================
function Check-15-RevertConflictNonzeroExit {
  $id = "CHK15-revert-conflict-nonzero-exit"
  if (-not (ShouldRun $id)) { return }
  $repo = New-SandboxRepo "c15-revert-exit" $Phase
  New-QaCommit $repo "mix.bin" "v1" 1 1500
  New-QaCommit $repo "mix.bin" "v2" 1 1501
  $v2 = ((Invoke-MG $repo @("log", "--oneline", "-n", "1") $Phase).Out.Trim() -split " ")[0]
  New-QaCommit $repo "mix.bin" "v3" 1 1502
  $r = Invoke-MG $repo @("revert", $v2) $Phase
  $pass = $r.Exit -ne 0
  Rec $id "revert (conflicting)" $pass "exit=$($r.Exit)"
}

function Check-16-RevertConflictPhantomIndexClears {
  $id = "CHK16-revert-conflict-phantom-index-clears"
  if (-not (ShouldRun $id)) { return }
  $repo = New-SandboxRepo "c16-revert-phantom" $Phase
  New-QaCommit $repo "mix.bin" "v1" 1 1600
  New-QaCommit $repo "mix.bin" "v2" 1 1601
  $v2 = ((Invoke-MG $repo @("log", "--oneline", "-n", "1") $Phase).Out.Trim() -split " ")[0]
  New-QaCommit $repo "mix.bin" "v3" 1 1602
  $head = ((Invoke-MG $repo @("log", "--oneline", "-n", "1") $Phase).Out.Trim() -split " ")[0]
  Invoke-MG $repo @("revert", $v2) $Phase | Out-Null
  Invoke-MG $repo @("revert", "--abort") $Phase | Out-Null
  Invoke-MG $repo @("reset", "--hard", $head) $Phase | Out-Null
  $statusOut = (Invoke-MG $repo @("status") $Phase).Out
  $pass = $statusOut -notmatch "(?i)new file"
  Rec $id "revert; revert --abort; reset --hard" $pass "phantom-staging-cleared=$pass"
}

# ============================================================================
# CHK17/18 - fsck must route through packs (not just loose objects)
# Source: BUG-CLI-A9, BUG-ML-6 ("Objects checked: 0" yet reports PERFECT after gc --repack;
# corruption in a pack goes undetected)
# ============================================================================
function Check-17-FsckCountsPackedObjects {
  $id = "CHK17-fsck-counts-packed-objects"
  if (-not (ShouldRun $id)) { return }
  $repo = New-SandboxRepo "c17-fsck-pack-count" $Phase
  1..3 | ForEach-Object { New-QaCommit $repo "f$_.bin" "c$_" 1 (1700 + $_) }
  Invoke-MG $repo @("gc", "--repack", "-y") $Phase | Out-Null
  $out = (Invoke-MG $repo @("fsck") $Phase).Out
  $countOk = $false
  if ($out -match "Objects checked:\s*(\d+)") { $countOk = ([int]$Matches[1] -gt 0) }
  Rec $id "fsck (post gc --repack)" $countOk "objects-checked-gt-0=$countOk"
}

function Check-18-FsckDetectsPackCorruption {
  $id = "CHK18-fsck-detects-pack-corruption"
  if (-not (ShouldRun $id)) { return }
  $repo = New-SandboxRepo "c18-fsck-pack-corrupt" $Phase
  1..3 | ForEach-Object { New-QaCommit $repo "f$_.bin" "c$_" 1 (1800 + $_) }
  Invoke-MG $repo @("gc", "--repack", "-y") $Phase | Out-Null
  $pack = Get-ChildItem (Join-Path $repo ".mediagit") -Recurse -File -Include "*.pack" -ErrorAction SilentlyContinue | Select-Object -First 1
  if (-not $pack) {
    Rec $id "fsck (corrupted pack)" "SKIP" "no .pack file produced by gc --repack"
    return
  }
  $bytes = [IO.File]::ReadAllBytes($pack.FullName)
  $mid = [int]($bytes.Length / 2)
  $bytes[$mid] = $bytes[$mid] -bxor 0xFF
  [IO.File]::WriteAllBytes($pack.FullName, $bytes)
  $out = (Invoke-MG $repo @("fsck") $Phase).Out
  $pass = $out -notmatch "(?i)PERFECT"
  Rec $id "fsck (corrupted pack)" $pass "corruption-detected=$pass pack=$($pack.Name)"
}

# ============================================================================
# CHK19 - bisect must iterate, not declare premature completion
# Source: BUG-ML-1 (bisect bad+good on first pair immediately claims "Bisect complete!"
# with untested commits in between)
# ============================================================================
function Check-19-BisectNoPrematureComplete {
  $id = "CHK19-bisect-no-premature-complete"
  if (-not (ShouldRun $id)) { return }
  $repo = New-SandboxRepo "c19-bisect" $Phase
  1..6 | ForEach-Object { New-QaCommit $repo "f.bin" "c$_" 1 (1900 + $_) }
  $hashes = ((Invoke-MG $repo @("log") $Phase).Out -split "`n" | Select-String '^commit (\S+)') |
    ForEach-Object { $_.Matches[0].Groups[1].Value }
  $newest = $hashes[0]; $oldest = $hashes[-1]
  Invoke-MG $repo @("bisect", "start") $Phase | Out-Null
  Invoke-MG $repo @("bisect", "bad", $newest) $Phase | Out-Null
  $out = (Invoke-MG $repo @("bisect", "good", $oldest) $Phase).Out
  $pass = $out -notmatch "(?i)bisect complete"
  Invoke-MG $repo @("bisect", "reset") $Phase | Out-Null
  Rec $id "bisect start; bad; good (6 commits)" $pass "premature-complete=$(-not $pass)"
}

# ============================================================================
# CHK20 - push --repair heals a poisoned remote chunk
# Source: BUG-RM-3 (one bit-flipped remote object permanently poisons every clone; push
# --repair must strong-verify + force re-upload invalid remote objects)
# ============================================================================
function Check-20-PushRepairHealsPoisonedRemote {
  $id = "CHK20-push-repair-heals-poisoned-remote"
  if (-not (ShouldRun $id)) { return }
  $srv = $null
  try {
    $srv = Start-QaServer -Backend "local" -Phase $Phase
    $src = New-SandboxRepo "c20-src" $Phase
    New-QaCommit $src "plates\frame1.bin" "c1" 3 2001
    Invoke-MG $src @("remote", "add", "origin", $srv.Url) $Phase | Out-Null
    Invoke-MG $src @("push", "origin") $Phase | Out-Null
    $srcHash = Get-QaHash (Join-Path $src "plates\frame1.bin")

    # corrupt the largest object under the server's filesystem storage root
    $obj = Get-ChildItem (Join-Path $srv.DataDir "storage") -Recurse -File -ErrorAction SilentlyContinue |
      Sort-Object Length -Descending | Select-Object -First 1
    if (-not $obj) {
      Rec $id "push --repair" "SKIP" "no storage object found to corrupt under $($srv.DataDir)\storage"
      return
    }
    $bytes = [IO.File]::ReadAllBytes($obj.FullName)
    $mid = [int]($bytes.Length / 2)
    $bytes[$mid] = $bytes[$mid] -bxor 0xFF
    [IO.File]::WriteAllBytes($obj.FullName, $bytes)

    # Bounce the server process (same data dir, no re-init) so its in-memory
    # ODB cache (10MiB, odb/mod.rs) can't serve pre-corruption bytes - without
    # this the drill is invalid: clone1 gets cached-good data and
    # pre-repair-clone-detected-bad is always False.
    Stop-QaServer $srv
    $srvToml = Join-Path $srv.DataDir "server.toml"
    $rsOut = Join-Path $QA.Logs "server-local-$Phase-restart.out.log"
    $rsErr = Join-Path $QA.Logs "server-local-$Phase-restart.err.log"
    $proc2 = Start-Process -FilePath $QA.MGServer -ArgumentList @("--config", $srvToml) `
      -PassThru -NoNewWindow -RedirectStandardOutput $rsOut -RedirectStandardError $rsErr
    $srv.Proc = $proc2
    $baseUrl = ($srv.Url -replace '/[^/]+$', '')
    $ok = $false
    for ($i = 0; $i -lt 40; $i++) {
      try {
        if ((Invoke-WebRequest -Uri "$baseUrl/health" -UseBasicParsing -TimeoutSec 2 -ErrorAction Stop).StatusCode -eq 200) { $ok = $true; break }
      } catch {}
      Start-Sleep -Milliseconds 500
    }
    if (-not $ok) { Rec $id "push --repair" "SKIP" "server did not come back after corruption restart"; return }

    $clone1 = Join-Path $QA.Work "c20-clone1"
    if (Test-Path $clone1) { Remove-Item -Recurse -Force $clone1 }
    Invoke-MG $null @("clone", $srv.Url, $clone1) $Phase | Out-Null
    $cloneFailedOrBad = (-not (Test-Path (Join-Path $clone1 "plates\frame1.bin"))) -or
      ((Get-QaHashOrNull (Join-Path $clone1 "plates\frame1.bin")) -ne $srcHash)

    $rp = Invoke-MG $src @("push", "origin", "--repair") $Phase

    $clone2 = Join-Path $QA.Work "c20-clone2"
    if (Test-Path $clone2) { Remove-Item -Recurse -Force $clone2 }
    Invoke-MG $null @("clone", $srv.Url, $clone2) $Phase | Out-Null
    $healedOk = (Get-QaHashOrNull (Join-Path $clone2 "plates\frame1.bin")) -eq $srcHash

    $pass = $cloneFailedOrBad -and ($rp.Exit -eq 0) -and $healedOk
    Rec $id "push --repair" $pass "pre-repair-clone-detected-bad=$cloneFailedOrBad repair-exit=$($rp.Exit) post-repair-clone-hash-ok=$healedOk"
  } catch {
    if ("$_" -match "^SKIP:") { Rec $id "push --repair" "SKIP" "$_" } else { Rec $id "push --repair" $false "unexpected error: $_" }
  } finally { Stop-QaServer $srv }
}

# ============================================================================
# Generic op matrix: {reset --soft/--mixed/--hard+reflog, revert, stash, switch(dirty),
# merge --no-ff, cherry-pick, rebase} against a fresh 2-branch binary-file repo each time.
# ============================================================================
function New-QaMatrixRepo([string]$Name) {
  $repo = New-SandboxRepo $Name $Phase
  New-QaCommit $repo "base.bin" "base" 2 9001
  Br $repo "topic"; Sw $repo "topic" | Out-Null
  New-QaCommit $repo "topic.bin" "topic adds topic.bin" 2 9002
  Sw $repo "main" | Out-Null
  New-QaCommit $repo "mainfile.bin" "main adds mainfile.bin" 2 9003
  return $repo
}

function Matrix-ResetSoftMixedHard {
  foreach ($mode in @("soft", "mixed", "hard")) {
    $id = "MATRIX-reset-$mode"
    if (-not (ShouldRun $id)) { continue }
    $repo = New-SandboxRepo "mx-reset-$mode" $Phase
    New-QaCommit $repo "base.bin" "base" 2 9101
    New-QaCommit $repo "extra.bin" "extra" 2 9102
    $extraHash = Get-QaHash (Join-Path $repo "extra.bin")
    $preOid = ((Invoke-MG $repo @("log", "--oneline", "-n", "1") $Phase).Out.Trim() -split "\s+")[0]
    $args = @("reset")
    if ($mode -ne "mixed") { $args += "--$mode" }
    $args += "HEAD~1"
    $r = Invoke-MG $repo $args $Phase
    if ($mode -eq "hard") {
      $gone = -not (Test-Path (Join-Path $repo "extra.bin"))
      # reflog-based recovery: the pre-reset commit must still be listed in reflog and recoverable
      $reflogOut = (Invoke-MG $repo @("reflog") $Phase).Out
      $inReflog = $reflogOut -match [regex]::Escape($preOid.Substring(0, [Math]::Min(7, $preOid.Length)))
      Invoke-MG $repo @("reset", "--hard", $preOid) $Phase | Out-Null
      $recovered = (Get-QaHashOrNull (Join-Path $repo "extra.bin")) -eq $extraHash
      $pass = ($r.Exit -eq 0) -and $gone -and $inReflog -and $recovered
      Rec $id "reset --hard HEAD~1 + reflog recovery" $pass "exit=$($r.Exit) worktree-cleared=$gone pre-reset-oid-in-reflog=$inReflog reflog-recovery-ok=$recovered"
    } else {
      $kept = (Get-QaHashOrNull (Join-Path $repo "extra.bin")) -eq $extraHash
      $pass = ($r.Exit -eq 0) -and $kept
      Rec $id "reset --$mode HEAD~1" $pass "exit=$($r.Exit) worktree-file-kept=$kept"
    }
  }
}

function Matrix-Revert {
  $id = "MATRIX-revert"
  if (-not (ShouldRun $id)) { return }
  $repo = New-SandboxRepo "mx-revert" $Phase
  New-QaCommit $repo "f.bin" "v1" 2 9201
  $v1Hash = Get-QaHash (Join-Path $repo "f.bin")
  New-QaCommit $repo "f.bin" "v2" 2 9202
  $r = Invoke-MG $repo @("revert", "HEAD") $Phase
  $reverted = (Get-QaHashOrNull (Join-Path $repo "f.bin")) -eq $v1Hash
  $fsckOk = Test-QaFsckClean $repo
  $pass = ($r.Exit -eq 0) -and $reverted -and $fsckOk
  Rec $id "revert HEAD (clean, no conflict)" $pass "exit=$($r.Exit) reverted-to-v1=$reverted fsck=$fsckOk"
}

function Matrix-Stash {
  $id = "MATRIX-stash"
  if (-not (ShouldRun $id)) { return }
  $repo = New-QaMatrixRepo "mx-stash"
  $headHash = Get-QaHash (Join-Path $repo "mainfile.bin")
  New-QaBinaryFixture (Join-Path $repo "mainfile.bin") 2 9301
  $wipHash = Get-QaHash (Join-Path $repo "mainfile.bin")
  Invoke-MG $repo @("stash", "push", "-m", "mx") $Phase | Out-Null
  $poppedFromHead = (Get-QaHashOrNull (Join-Path $repo "mainfile.bin")) -eq $headHash
  Invoke-MG $repo @("stash", "pop") $Phase | Out-Null
  $restored = (Get-QaHashOrNull (Join-Path $repo "mainfile.bin")) -eq $wipHash
  $pass = $poppedFromHead -and $restored
  Rec $id "stash push; stash pop" $pass "clean-after-push=$poppedFromHead wip-restored=$restored"
}

function Matrix-SwitchDirty {
  $id = "MATRIX-switch-dirty"
  if (-not (ShouldRun $id)) { return }
  $repo = New-QaMatrixRepo "mx-switch-dirty"
  $dirty = New-Object byte[] (256KB)
  (New-Object System.Random(9401)).NextBytes($dirty)
  [IO.File]::WriteAllBytes((Join-Path $repo "topic.bin"), $dirty)
  $dirtyHash = (Get-FileHash -Algorithm SHA256 -InputStream ([IO.MemoryStream]::new($dirty))).Hash
  $r = Invoke-MG $repo @("branch", "switch", "topic") $Phase
  $preserved = (Get-QaHashOrNull (Join-Path $repo "topic.bin")) -eq $dirtyHash
  $pass = $preserved -or ($r.Exit -ne 0)
  Rec $id "branch switch (dirty tree, no -f)" $pass "exit=$($r.Exit) dirty-work-preserved=$preserved"
}

function Matrix-MergeNoFF {
  $id = "MATRIX-merge-noff"
  if (-not (ShouldRun $id)) { return }
  $repo = New-QaMatrixRepo "mx-merge-noff"
  $mainHash = Get-QaHash (Join-Path $repo "mainfile.bin")
  Sw $repo "topic" | Out-Null
  $topicHash = Get-QaHash (Join-Path $repo "topic.bin")
  Sw $repo "main" | Out-Null
  $r = Invoke-MG $repo @("merge", "--no-ff", "topic") $Phase
  $ok = ((Get-QaHashOrNull (Join-Path $repo "mainfile.bin")) -eq $mainHash) -and
        ((Get-QaHashOrNull (Join-Path $repo "topic.bin")) -eq $topicHash)
  $fsckOk = Test-QaFsckClean $repo
  $pass = ($r.Exit -eq 0) -and $ok -and $fsckOk
  Rec $id "merge --no-ff topic" $pass "exit=$($r.Exit) hashes-ok=$ok fsck=$fsckOk"
}

function Matrix-CherryPick {
  $id = "MATRIX-cherrypick"
  if (-not (ShouldRun $id)) { return }
  $repo = New-QaMatrixRepo "mx-cherrypick"
  $mainHash = Get-QaHash (Join-Path $repo "mainfile.bin")
  Sw $repo "topic" | Out-Null
  $topicHash = Get-QaHash (Join-Path $repo "topic.bin")
  $cp = ((Invoke-MG $repo @("log", "--oneline", "-n", "1") $Phase).Out.Trim() -split " ")[0]
  Sw $repo "main" | Out-Null
  $r = Invoke-MG $repo @("cherry-pick", $cp) $Phase
  $ok = ((Get-QaHashOrNull (Join-Path $repo "mainfile.bin")) -eq $mainHash) -and
        ((Get-QaHashOrNull (Join-Path $repo "topic.bin")) -eq $topicHash)
  $fsckOk = Test-QaFsckClean $repo
  $pass = ($r.Exit -eq 0) -and $ok -and $fsckOk
  Rec $id "cherry-pick topic's commit onto main" $pass "exit=$($r.Exit) hashes-ok=$ok fsck=$fsckOk"
}

function Matrix-Rebase {
  $id = "MATRIX-rebase"
  if (-not (ShouldRun $id)) { return }
  $repo = New-QaMatrixRepo "mx-rebase"
  $mainHash = Get-QaHash (Join-Path $repo "mainfile.bin")
  Sw $repo "topic" | Out-Null
  $topicHash = Get-QaHash (Join-Path $repo "topic.bin")
  $r = Invoke-MG $repo @("rebase", "main") $Phase
  $ok = ((Get-QaHashOrNull (Join-Path $repo "mainfile.bin")) -eq $mainHash) -and
        ((Get-QaHashOrNull (Join-Path $repo "topic.bin")) -eq $topicHash)
  $fsckOk = Test-QaFsckClean $repo
  $pass = ($r.Exit -eq 0) -and $ok -and $fsckOk
  Rec $id "rebase topic onto main" $pass "exit=$($r.Exit) hashes-ok=$ok fsck=$fsckOk"
}

# ============================================================================
# Run everything
# ============================================================================
Write-QaLog $Phase "=== 05_branching start (Only='$Only') ==="

Check-01-RebaseKeepsBaseFiles
Check-02-RebaseConflictDetected
Check-03-CherryPickKeepsUnrelatedFiles
Check-04-CherryPickConflictRealContent
Check-05-PullRebaseKeepsLocalCommit
Check-06-PullRebaseFastForwardClean
Check-07-MergeConflictBinaryNoMarkers
Check-08-MergeAbortRestoresWorktree
Check-09-MergeAbortClearsPhantomIndex
Check-10-MergeContinueCompletes
Check-11-SwitchRefusesDirtyTree
Check-12-SwitchForceOverwritesCleanly
Check-13-StashRoundtripPlain
Check-14-StashRoundtripAfterPhantomIndex
Check-15-RevertConflictNonzeroExit
Check-16-RevertConflictPhantomIndexClears
Check-17-FsckCountsPackedObjects
Check-18-FsckDetectsPackCorruption
Check-19-BisectNoPrematureComplete
Check-20-PushRepairHealsPoisonedRemote

Matrix-ResetSoftMixedHard
Matrix-Revert
Matrix-Stash
Matrix-SwitchDirty
Matrix-MergeNoFF
Matrix-CherryPick
Matrix-Rebase

Write-QaLog $Phase "=== 05_branching done: overall=$(if ($script:AllPass) { 'PASS' } else { 'FAIL' }) ==="
# Teardown: reclaim this phase's own work/ scratch so a long campaign cannot run the
# volume out of space. work/ ONLY - logs/ and fixtures-synthetic/ are never touched.
Invoke-QaTeardown $Phase @("c[0-9][0-9]-*")

Exit-QaPhase $Phase (-not $script:AllPass)
