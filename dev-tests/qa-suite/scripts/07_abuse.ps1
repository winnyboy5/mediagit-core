# Phase 7 - fault injection / abuse suite. ASCII-only, PS 5.1 compatible.
# Generalizes dev-tests\standalone-deep-v11\scripts\abuse_suite.ps1 onto the qa-suite contract.
# Gate: every injected fault is either recovered or cleanly reported - NO silent corruption.
# Silent = fsck says clean but content hashes differ from ground truth => hard fail.
#
# Drills:
#   A1 kill mid-add        A2 kill mid-push (MinIO)   A3 concurrent double-push
#   A4 corrupt chunk at rest -> fsck DETECT, recover via re-clone from intact remote
#   A5 read-only file in worktree
#   A6 path with spaces + unicode filename
# (disk-full is out of scope per campaign brief)
#
# Output: $QA.Logs\abuse_results.tsv (drill, pass, detail)

. (Join-Path $PSScriptRoot "lib\common.ps1")
. (Join-Path $PSScriptRoot "lib\remote.ps1")

$Phase = "07_abuse"
$env:MEDIAGIT_AUTHOR_NAME = "QA-Suite"
$env:MEDIAGIT_AUTHOR_EMAIL = "qa-suite@mediagit.local"

$TSV = Join-Path $QA.Logs "abuse_results.tsv"
$script:AllPass = $true

function Rec([string]$Drill, $Pass, [string]$Detail) {
  Write-QaRow $TSV @("drill", "pass", "detail") @($Drill, $Pass, $Detail)
  $tag = if ("$Pass" -eq "SKIP") { "SKIP" } elseif ($Pass) { "PASS" } else { "FAIL" }
  Write-QaLog $Phase ("{0} -> {1}  {2}" -f $Drill, $tag, $Detail)
  Write-QaGate $Phase $Drill ($Pass -eq $true -or "$Pass" -eq "SKIP") $Detail
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

function Test-QaFsckClean([string]$Repo) {
  $r = Invoke-MG $Repo @("fsck") $Phase
  return -not (($r.Out -match "(?i)corrupt|missing|error|failed") -or ($r.Exit -ne 0))
}

# ---------------------------------------------------------------------------
# A1: kill mediagit mid-add on a large file; repo must not be corrupted and a
# retried add + commit must succeed with the file hash-exact.
# ---------------------------------------------------------------------------
function Drill-A1-KillMidAdd {
  $drill = "A1-kill-mid-add"
  $repo = New-SandboxRepo "a1-killadd" $Phase
  New-QaBinaryFixture (Join-Path $repo "big.bin") 200 71001
  $origHash = Get-QaHash (Join-Path $repo "big.bin")
  $p = Start-Process $QA.MG -ArgumentList @("-C", $repo, "add", "big.bin") -PassThru -NoNewWindow `
    -RedirectStandardOutput (Join-Path $QA.Logs "a1-add.out") -RedirectStandardError (Join-Path $QA.Logs "a1-add.err")
  Start-Sleep -Milliseconds 1500
  $killed = $false
  if (-not $p.HasExited) { Stop-Process -Id $p.Id -Force; $killed = $true }
  $fsck1 = Test-QaFsckClean $repo
  $retry = Invoke-MG $repo @("add", "big.bin") $Phase -TimeoutSec 1200
  $cmt = Invoke-MG $repo @("commit", "-m", "after kill") $Phase
  $fsck2 = Test-QaFsckClean $repo
  $hashOk = (Get-QaHash (Join-Path $repo "big.bin")) -eq $origHash
  $pass = $fsck2 -and ($retry.Exit -eq 0) -and ($cmt.Exit -eq 0) -and $hashOk
  # silent-corruption guard: fsck clean but hash mismatch is the hard-fail combination
  if ($fsck2 -and -not $hashOk) { $pass = $false }
  Rec $drill $pass "killed=$killed post-kill-fsck=$fsck1 retry-add=$($retry.Exit) commit=$($cmt.Exit) final-fsck=$fsck2 hash-ok=$hashOk"
}

# ---------------------------------------------------------------------------
# A2: kill mediagit mid-push (MinIO); local repo must stay clean, a re-push must
# succeed, and a fresh clone must be hash-exact.
# ---------------------------------------------------------------------------
function Drill-A2-KillMidPush {
  $drill = "A2-kill-mid-push"
  $srv = $null
  try {
    # per-drill Phase suffix: A2/A3/A4 all target minio and would otherwise share the
    # same repo name ("proj-<runid>-07_abuse") within one run, tripping the namespace-
    # collision guard against each other's leftover bucket state.
    $srv = Start-QaServer -Backend "minio" -Phase "$Phase-A2"
    $repo = New-SandboxRepo "a2-killpush" $Phase
    # 600MB, not 100MB: measured locally, a 100MB push to loopback MinIO completes in
    # ~1s (400MB ~3s, roughly linear) - well under the 2.5s sleep below, so the kill
    # always fired after the push had already finished (killed=False every run,
    # never actually exercising the crash-recovery path). 600MB pushes in ~4-5s,
    # landing the kill solidly mid-transfer.
    New-QaBinaryFixture (Join-Path $repo "big.bin") 600 72001
    $origHash = Get-QaHash (Join-Path $repo "big.bin")
    Invoke-MG $repo @("add", ".") $Phase -TimeoutSec 1200 | Out-Null
    Invoke-MG $repo @("commit", "-m", "c1") $Phase | Out-Null
    Invoke-MG $repo @("remote", "add", "origin", $srv.Url) $Phase | Out-Null

    $p = Start-Process $QA.MG -ArgumentList @("-C", $repo, "push", "origin") -PassThru -NoNewWindow `
      -RedirectStandardOutput (Join-Path $QA.Logs "a2-push.out") -RedirectStandardError (Join-Path $QA.Logs "a2-push.err")
    Start-Sleep -Milliseconds 2500
    $killed = $false
    if (-not $p.HasExited) { Stop-Process -Id $p.Id -Force; $killed = $true }

    $fsck = Test-QaFsckClean $repo
    $re = Invoke-MG $repo @("push", "origin") $Phase -TimeoutSec 3600
    $clone = Join-Path $QA.Work "a2-clone"
    if (Test-Path $clone) { Remove-Item -Recurse -Force $clone }
    $cl = Invoke-MG $null @("clone", $srv.Url, $clone) $Phase -TimeoutSec 3600
    $cloneHashOk = (Test-Path (Join-Path $clone "big.bin")) -and
                   ((Get-QaHash (Join-Path $clone "big.bin")) -eq $origHash)
    $pass = $fsck -and ($re.Exit -eq 0) -and ($cl.Exit -eq 0) -and $cloneHashOk
    Rec $drill $pass "killed=$killed local-fsck=$fsck re-push=$($re.Exit) clone=$($cl.Exit) clone-hash-ok=$cloneHashOk"
  } catch {
    if ("$_" -match "^SKIP:") { Rec $drill "SKIP" "$_" } else { Rec $drill $false "unexpected error: $_" }
  } finally { Stop-QaServer $srv }
}

# ---------------------------------------------------------------------------
# A3: concurrent double-push from two clones to the same remote; the remote must
# stay clonable and the surviving state hash-consistent with one of the pushers.
# ---------------------------------------------------------------------------
function Drill-A3-ConcurrentDoublePush {
  $drill = "A3-concurrent-double-push"
  $srv = $null
  try {
    # per-drill Phase suffix - see A2 comment above.
    $srv = Start-QaServer -Backend "minio" -Phase "$Phase-A3"
    $seed = New-SandboxRepo "a3-seed" $Phase
    New-QaBinaryFixture (Join-Path $seed "base.bin") 4 73000
    Invoke-MG $seed @("add", ".") $Phase | Out-Null
    Invoke-MG $seed @("commit", "-m", "base") $Phase | Out-Null
    Invoke-MG $seed @("remote", "add", "origin", $srv.Url) $Phase | Out-Null
    Invoke-MG $seed @("push", "origin") $Phase -TimeoutSec 1200 | Out-Null

    $c1 = Join-Path $QA.Work "a3-clone1"; $c2 = Join-Path $QA.Work "a3-clone2"
    foreach ($c in @($c1, $c2)) { if (Test-Path $c) { Remove-Item -Recurse -Force $c } }
    Invoke-MG $null @("clone", $srv.Url, $c1) $Phase | Out-Null
    Invoke-MG $null @("clone", $srv.Url, $c2) $Phase | Out-Null
    New-QaBinaryFixture (Join-Path $c1 "x.bin") 8 73001
    New-QaBinaryFixture (Join-Path $c2 "y.bin") 8 73002
    foreach ($c in @($c1, $c2)) {
      Invoke-MG $c @("add", ".") $Phase | Out-Null
      Invoke-MG $c @("commit", "-m", "from $(Split-Path $c -Leaf)") $Phase | Out-Null
    }

    $j1 = Start-Job { param($m, $r) & $m -C $r push origin --force 2>&1 | Out-String; $LASTEXITCODE } -ArgumentList $QA.MG, $c1
    $j2 = Start-Job { param($m, $r) & $m -C $r push origin --force 2>&1 | Out-String; $LASTEXITCODE } -ArgumentList $QA.MG, $c2
    Wait-Job $j1, $j2 -Timeout 600 | Out-Null
    $o1 = Receive-Job $j1 | Out-String; $o2 = Receive-Job $j2 | Out-String
    Remove-Job $j1, $j2 -Force -ErrorAction SilentlyContinue
    ($o1 + "`n" + $o2) | Add-Content (Join-Path $QA.Logs "$Phase-a3-pushes.log")

    $post = Join-Path $QA.Work "a3-postclone"
    if (Test-Path $post) { Remove-Item -Recurse -Force $post }
    $cl = Invoke-MG $null @("clone", $srv.Url, $post) $Phase -TimeoutSec 1200
    $fsckOk = $false
    if ($cl.Exit -eq 0) { $fsckOk = Test-QaFsckClean $post }
    # surviving head must exactly match one pusher's tree (x.bin xor y.bin, hash-exact)
    $winnerOk = $false
    if ($cl.Exit -eq 0) {
      $hasX = Test-Path (Join-Path $post "x.bin"); $hasY = Test-Path (Join-Path $post "y.bin")
      if ($hasX -and -not $hasY) { $winnerOk = (Get-QaHash (Join-Path $post "x.bin")) -eq (Get-QaHash (Join-Path $c1 "x.bin")) }
      elseif ($hasY -and -not $hasX) { $winnerOk = (Get-QaHash (Join-Path $post "y.bin")) -eq (Get-QaHash (Join-Path $c2 "y.bin")) }
    }
    $pass = ($cl.Exit -eq 0) -and $fsckOk -and $winnerOk
    Rec $drill $pass "post-clone=$($cl.Exit) post-fsck=$fsckOk winner-tree-hash-exact=$winnerOk"
  } catch {
    if ("$_" -match "^SKIP:") { Rec $drill "SKIP" "$_" } else { Rec $drill $false "unexpected error: $_" }
  } finally { Stop-QaServer $srv }
}

# ---------------------------------------------------------------------------
# A4: corrupt one chunk file at rest in local .mediagit. fsck must DETECT it;
# recovery = re-clone from the intact remote must be hash-exact and fsck-clean.
# Silent corruption (fsck clean + hash mismatch) = hard fail.
# ---------------------------------------------------------------------------
function Drill-A4-CorruptChunkAtRest {
  $drill = "A4-corrupt-chunk-at-rest"
  $srv = $null
  try {
    $srv = Start-QaServer -Backend "minio" -Phase $Phase
    $repo = New-SandboxRepo "a4-corrupt" $Phase
    New-QaBinaryFixture (Join-Path $repo "asset.bin") 8 74001
    $origHash = Get-QaHash (Join-Path $repo "asset.bin")
    Invoke-MG $repo @("add", ".") $Phase | Out-Null
    Invoke-MG $repo @("commit", "-m", "c1") $Phase | Out-Null
    Invoke-MG $repo @("remote", "add", "origin", $srv.Url) $Phase | Out-Null
    Invoke-MG $repo @("push", "origin") $Phase -TimeoutSec 1200 | Out-Null

    # flip a byte in the largest object under .mediagit (chunk or pack)
    $obj = Get-ChildItem (Join-Path $repo ".mediagit") -Recurse -File -ErrorAction SilentlyContinue |
      Where-Object { $_.Name -notmatch "\.(toml|json|log|lock)$" } |
      Sort-Object Length -Descending | Select-Object -First 1
    if (-not $obj) { Rec $drill $false "no object file found to corrupt"; return }
    $bytes = [IO.File]::ReadAllBytes($obj.FullName)
    $mid = [int]($bytes.Length / 2)
    $bytes[$mid] = $bytes[$mid] -bxor 0xFF
    [IO.File]::WriteAllBytes($obj.FullName, $bytes)

    $f = Invoke-MG $repo @("fsck") $Phase
    $detected = ($f.Exit -ne 0) -or ($f.Out -match "(?i)corrupt|fail|mismatch|error")

    # recovery: re-clone from the intact remote
    $reclone = Join-Path $QA.Work "a4-reclone"
    if (Test-Path $reclone) { Remove-Item -Recurse -Force $reclone }
    $cl = Invoke-MG $null @("clone", $srv.Url, $reclone) $Phase -TimeoutSec 1200
    $recoveredHashOk = (Test-Path (Join-Path $reclone "asset.bin")) -and
                       ((Get-QaHash (Join-Path $reclone "asset.bin")) -eq $origHash)
    $recloneFsck = $false
    if ($cl.Exit -eq 0) { $recloneFsck = Test-QaFsckClean $reclone }

    $silent = (-not $detected)   # fsck said clean on a repo we know we corrupted
    $pass = $detected -and ($cl.Exit -eq 0) -and $recoveredHashOk -and $recloneFsck
    Rec $drill $pass "detected=$detected (silent-corruption=$silent) corrupted=$($obj.Name) re-clone=$($cl.Exit) recovered-hash-ok=$recoveredHashOk reclone-fsck=$recloneFsck"
  } catch {
    if ("$_" -match "^SKIP:") { Rec $drill "SKIP" "$_" } else { Rec $drill $false "unexpected error: $_" }
  } finally { Stop-QaServer $srv }
}

# ---------------------------------------------------------------------------
# A5: read-only file in worktree; add/commit must either succeed or fail with a
# clean error - never panic, never corrupt the repo.
# ---------------------------------------------------------------------------
function Drill-A5-ReadOnlyFile {
  $drill = "A5-readonly-file"
  $repo = New-SandboxRepo "a5-readonly" $Phase
  New-QaBinaryFixture (Join-Path $repo "ro.bin") 2 75001
  $roHash = Get-QaHash (Join-Path $repo "ro.bin")
  Set-ItemProperty (Join-Path $repo "ro.bin") -Name IsReadOnly -Value $true
  try {
    $a = Invoke-MG $repo @("add", "ro.bin") $Phase
    $c = Invoke-MG $repo @("commit", "-m", "ro") $Phase
    $panic = ($a.Out -match "panicked") -or ($c.Out -match "panicked")
    $fsckOk = Test-QaFsckClean $repo
    $hashOk = (Get-QaHash (Join-Path $repo "ro.bin")) -eq $roHash
    # acceptable: clean success, or clean refusal - as long as no panic, fsck clean, bytes intact
    $pass = (-not $panic) -and $fsckOk -and $hashOk
    Rec $drill $pass "add=$($a.Exit) commit=$($c.Exit) panic=$panic fsck=$fsckOk file-bytes-intact=$hashOk"
  } finally {
    Set-ItemProperty (Join-Path $repo "ro.bin") -Name IsReadOnly -Value $false -ErrorAction SilentlyContinue
  }
}

# ---------------------------------------------------------------------------
# A6: path with spaces + unicode filename; full add/commit/fsck round-trip with
# hash verification on every touched file.
# ---------------------------------------------------------------------------
function Drill-A6-SpacesAndUnicodePaths {
  $drill = "A6-spaces-unicode-paths"
  $repo = New-SandboxRepo "a6-paths" $Phase
  $spaceDir = Join-Path $repo "dir with spaces"
  New-Item -ItemType Directory -Path $spaceDir -Force | Out-Null
  $spaceFile = Join-Path $spaceDir "file with spaces.bin"
  New-QaBinaryFixture $spaceFile 2 76001
  # unicode name built from codepoints so the script itself stays ASCII-only
  $uname = "u" + [char]0x00FC + [char]0x00F1 + " ph oto " + [char]0x5199 + [char]0x771F + ".bin"
  $uniFile = Join-Path $repo $uname
  New-QaBinaryFixture $uniFile 2 76002
  $h1 = Get-QaHash $spaceFile
  $h2 = Get-QaHash $uniFile
  $a = Invoke-MG $repo @("add", ".") $Phase
  $c = Invoke-MG $repo @("commit", "-m", "paths") $Phase
  $fsckOk = Test-QaFsckClean $repo
  # round-trip through reset --hard to force a checkout of both paths
  Invoke-MG $repo @("reset", "--hard", "HEAD") $Phase | Out-Null
  $h1b = if (Test-Path $spaceFile) { Get-QaHash $spaceFile } else { $null }
  $h2b = if (Test-Path $uniFile) { Get-QaHash $uniFile } else { $null }
  $hashOk = ($h1b -eq $h1) -and ($h2b -eq $h2)
  $pass = ($a.Exit -eq 0) -and ($c.Exit -eq 0) -and $fsckOk -and $hashOk
  Rec $drill $pass "add=$($a.Exit) commit=$($c.Exit) fsck=$fsckOk roundtrip-hashes-ok=$hashOk"
}

# ---------------------------------------------------------------------------
Write-QaLog $Phase "=== 07_abuse start ==="

Drill-A1-KillMidAdd
Drill-A2-KillMidPush
Drill-A3-ConcurrentDoublePush
Drill-A4-CorruptChunkAtRest
Drill-A5-ReadOnlyFile
Drill-A6-SpacesAndUnicodePaths

Write-QaLog $Phase "=== 07_abuse done: overall=$(if ($script:AllPass) { 'PASS' } else { 'FAIL' }) ==="
if ($script:AllPass) { exit 0 } else { exit 1 }
