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
#   A7 backend outage mid-push (docker stop/start mediagit-minio) -> clean fail, retry ok
#   A8 disk-full (capability-gated: needs admin for a small VHD volume) -> clean fail, recover
#   A9 server-enforced lock e2e (push rejected/force-unlock/retry) + no-auth force-required variant
#   A10 batch-get-disabled fallback -> clone still succeeds via per-chunk path
#   A11 chunk-delta chain depth over a run of similar versions -> stays <= MAX_DELTA_DEPTH
#   A12 fabricated chunk-delta cycle / self-loop -> fsck detects and terminates
#   A13 per-chunk fallback (packs disabled) -> completes without tripping rate limits
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
  Write-QaGate $Phase $Drill $Pass $Detail
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

# Capability probe for A7: is the named docker container reachable at all?
# False on any docker error (daemon not running, container missing, docker
# not installed) - callers SKIP rather than fail when this is false.
function Test-QaDockerAvailable([string]$Container) {
  try {
    & docker inspect -f "{{.State.Status}}" $Container *> $null
    return ($LASTEXITCODE -eq 0)
  } catch { return $false }
}

# Poll a MinIO endpoint's health-live probe until it responds or times out.
function Wait-QaMinioUp([string]$Endpoint, [int]$TimeoutSec = 30) {
  $sw = [Diagnostics.Stopwatch]::StartNew()
  while ($sw.Elapsed.TotalSeconds -lt $TimeoutSec) {
    try {
      $r = Invoke-WebRequest -Uri "$Endpoint/minio/health/live" -UseBasicParsing -TimeoutSec 2 -ErrorAction Stop
      if ($r.StatusCode -eq 200) { return $true }
    } catch {}
    Start-Sleep -Milliseconds 500
  }
  return $false
}

# Capability probe for A8: disk-full needs a small fixed-size volume, which on
# Windows needs diskpart's "attach vdisk" - that needs admin. No admin = SKIP.
function Test-QaAdminRights {
  try {
    $id = [Security.Principal.WindowsIdentity]::GetCurrent()
    $pr = New-Object Security.Principal.WindowsPrincipal($id)
    return $pr.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)
  } catch { return $false }
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

    # This push used to be `| Out-Null` - its result was discarded entirely, so the
    # drill could only see corruption-detection and recovery. On 2026-08-03 that
    # blindness hid a real defect: this exact push took 1,188s for 8 MiB (the
    # adjacent identical push took 0.2s) and still exited 0, because the client
    # burned four consecutive 300s upload timeouts and succeeded on the fifth.
    # The phase reported PASS with a 20-minute stall inside it.
    #
    # So gate BOTH: the push must succeed, AND it must not stall. The seconds
    # bound is deliberately absurd rather than tuned - 8 MiB in 120s is 0.07 MB/s,
    # which no healthy path produces on a loopback backend - so this detects a
    # stall without encoding this machine's speed (cf. the S2 note in 10_scale.ps1
    # on why absolute-time gates are usually the wrong tool).
    $a4push = Invoke-MG $repo @("push", "origin") $Phase -TimeoutSec 1200
    Rec "A4-push-completes-without-stalling" (($a4push.Exit -eq 0) -and ($a4push.Sec -lt 120)) `
      ("exit={0} sec={1} sizeMB=8 stallBoundSec=120" -f $a4push.Exit, $a4push.Sec)

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
# A7: kill MinIO mid-push (docker stop mediagit-minio); the push must fail
# cleanly (nonzero exit, no panic), a restarted MinIO must let a retried push
# succeed, and a fresh clone must be hash-exact and fsck-clean.
# Capability-gated: SKIPs cleanly when the "mediagit-minio" container isn't
# reachable (docker not installed/running, or a differently-named container).
# ---------------------------------------------------------------------------
function Drill-A7-BackendOutage {
  $drill = "A7-backend-outage"
  $container = "mediagit-minio"
  if (-not (Test-QaDockerAvailable $container)) {
    Rec $drill "SKIP" "docker container '$container' not reachable (docker not installed/running, or container missing)"
    return
  }

  $srv = $null
  $stoppedContainer = $false
  try {
    $srv = Start-QaServer -Backend "minio" -Phase "$Phase-A7"
    $repo = New-SandboxRepo "a7-outage" $Phase
    # 600MB, same sizing rationale as A2: large enough that the push is still
    # mid-transfer ~2s in, so the docker stop lands mid-push rather than after.
    New-QaBinaryFixture (Join-Path $repo "big.bin") 600 77001
    $origHash = Get-QaHash (Join-Path $repo "big.bin")
    Invoke-MG $repo @("add", ".") $Phase -TimeoutSec 1200 | Out-Null
    Invoke-MG $repo @("commit", "-m", "c1") $Phase | Out-Null
    Invoke-MG $repo @("remote", "add", "origin", $srv.Url) $Phase | Out-Null

    # Bound the client push so a mid-transfer backend outage fails fast with a
    # clear error instead of hanging on retries. 60s >> a normal localhost push
    # (~seconds) but well inside the 120s WaitForExit window below.
    $env:MEDIAGIT_PUSH_DEADLINE_SECS = "60"
    $p = Start-Process $QA.MG -ArgumentList @("-C", $repo, "push", "origin") -PassThru -NoNewWindow `
      -RedirectStandardOutput (Join-Path $QA.Logs "a7-push.out") -RedirectStandardError (Join-Path $QA.Logs "a7-push.err")
    Start-Sleep -Milliseconds 2000
    & docker stop $container *> $null
    $stoppedContainer = $true

    $exited = $p.WaitForExit(120000)
    Remove-Item Env:\MEDIAGIT_PUSH_DEADLINE_SECS -ErrorAction SilentlyContinue
    if (-not $exited) { Stop-Process -Id $p.Id -Force -ErrorAction SilentlyContinue }
    $outText = "" + (Get-Content (Join-Path $QA.Logs "a7-push.out") -Raw -ErrorAction SilentlyContinue) `
                   + (Get-Content (Join-Path $QA.Logs "a7-push.err") -Raw -ErrorAction SilentlyContinue)
    $panic = $outText -match "panicked"
    $cleanFail = $exited -and ($p.ExitCode -ne 0) -and (-not $panic)

    & docker start $container *> $null
    $stoppedContainer = $false
    $up = Wait-QaMinioUp $QA.MinioEndpoint 30
    if (-not $up) {
      # Docker Desktop's host port-proxy can stay wedged after `docker start`
      # (seen 2026-07-19: container healthy, localhost:9000 dead for 14+ min).
      # A full `docker restart` rebinds it; one retry keeps A7 from cascading
      # into A9/A10 SKIPs on what is a host-networking hiccup, not a product bug.
      Write-QaLog $Phase "A7: host port not back after docker start; retrying with docker restart $container"
      & docker restart $container *> $null
      $up = Wait-QaMinioUp $QA.MinioEndpoint 60
    }

    $fsckLocal = Test-QaFsckClean $repo
    $retry = Invoke-MG $repo @("push", "origin") $Phase -TimeoutSec 3600
    $clone = Join-Path $QA.Work "a7-clone"
    if (Test-Path $clone) { Remove-Item -Recurse -Force $clone }
    $cl = Invoke-MG $null @("clone", $srv.Url, $clone) $Phase -TimeoutSec 3600
    $cloneHashOk = (Test-Path (Join-Path $clone "big.bin")) -and
                   ((Get-QaHash (Join-Path $clone "big.bin")) -eq $origHash)
    $fsckClone = if ($cl.Exit -eq 0) { Test-QaFsckClean $clone } else { $false }

    $pass = $up -and $cleanFail -and $fsckLocal -and ($retry.Exit -eq 0) -and ($cl.Exit -eq 0) -and $cloneHashOk -and $fsckClone
    Rec $drill $pass "minio-restarted=$up push-exited=$exited push-exit=$($p.ExitCode) panic=$panic clean-fail=$cleanFail local-fsck=$fsckLocal retry-push=$($retry.Exit) clone=$($cl.Exit) clone-hash-ok=$cloneHashOk clone-fsck=$fsckClone"
  } catch {
    if ($stoppedContainer) { & docker start $container *> $null }
    if ("$_" -match "^SKIP:") { Rec $drill "SKIP" "$_" } else { Rec $drill $false "unexpected error: $_" }
  } finally {
    if ($stoppedContainer) { & docker start $container *> $null }
    Stop-QaServer $srv
  }
}

# ---------------------------------------------------------------------------
# A8: disk-full. Capability-gated - needs admin rights to attach a small
# fixed-size VHD via diskpart (subst doesn't shrink the underlying volume, so
# it can't simulate ENOSPC). SKIPs cleanly without admin; on a capable
# machine, writes an oversized fixture into a repo living on a tiny volume,
# expects a clean add failure, then frees space and verifies full recovery.
# ---------------------------------------------------------------------------
function Drill-A8-DiskFull {
  $drill = "A8-disk-full"
  if (-not (Test-QaAdminRights)) {
    Rec $drill "SKIP" "requires admin rights to attach a small fixed-size VHD via diskpart; not available on this host"
    return
  }

  $vhd = Join-Path $QA.Work "a8-tiny.vhd"
  $driveLetter = $null
  $attached = $false
  try {
    if (Test-Path $vhd) { Remove-Item -Force $vhd }
    $sizeMB = 100
    $driveLetter = 90..70 | ForEach-Object { [char]$_ } | Where-Object { -not (Test-Path "$($_):\") } | Select-Object -First 1
    if (-not $driveLetter) { throw "SKIP: no free drive letter available for the tiny volume" }

    $dpScript = @"
create vdisk file="$vhd" maximum=$sizeMB type=fixed
select vdisk file="$vhd"
attach vdisk
create partition primary
format fs=ntfs quick label=QAA8
assign letter=$driveLetter
"@
    $dpFile = Join-Path $QA.Work "a8-diskpart.txt"
    $dpScript | Set-Content $dpFile -Encoding ASCII
    $dpOut = & diskpart /s $dpFile 2>&1
    if (-not (Test-Path "$($driveLetter):\")) { throw "SKIP: diskpart failed to create/attach/format the tiny volume: $dpOut" }
    $attached = $true

    $repo = New-SandboxRepo "$($driveLetter):\a8-repo" $Phase
    # Fixture bigger than the whole 100MB volume, so add() runs out of space
    # partway through writing chunks into .mediagit on that volume - the
    # source lives on the normal (large) work drive, only the repo is tiny.
    $srcFixture = Join-Path $QA.Work "a8-src.bin"
    New-QaBinaryFixture $srcFixture 150 78001
    Copy-Item $srcFixture (Join-Path $repo "big.bin")

    $a = Invoke-MG $repo @("add", "big.bin") $Phase -TimeoutSec 300
    $panic = $a.Out -match "panicked"
    $cleanFail = ($a.Exit -ne 0) -and (-not $panic)

    # Free the volume back up and verify full recovery: no partial state left
    # behind, and a normal add+commit on the same repo succeeds afterward.
    Remove-Item -Force (Join-Path $repo "big.bin") -ErrorAction SilentlyContinue
    $fsckAfterClear = Test-QaFsckClean $repo
    New-QaBinaryFixture (Join-Path $repo "small.bin") 2 78002
    $retryAdd = Invoke-MG $repo @("add", "small.bin") $Phase
    $retryCommit = Invoke-MG $repo @("commit", "-m", "after disk-full recovery") $Phase
    $fsckOk = Test-QaFsckClean $repo

    $pass = $cleanFail -and $fsckAfterClear -and ($retryAdd.Exit -eq 0) -and ($retryCommit.Exit -eq 0) -and $fsckOk
    Rec $drill $pass "add-exit=$($a.Exit) panic=$panic clean-fail=$cleanFail fsck-after-clear=$fsckAfterClear retry-add=$($retryAdd.Exit) retry-commit=$($retryCommit.Exit) final-fsck=$fsckOk"
  } catch {
    if ("$_" -match "^SKIP:") { Rec $drill "SKIP" "$_" } else { Rec $drill $false "unexpected error: $_" }
  } finally {
    if ($attached -and $driveLetter) {
      $dpCleanup = Join-Path $QA.Work "a8-diskpart-cleanup.txt"
      @"
select vdisk file="$vhd"
detach vdisk
"@ | Set-Content $dpCleanup -Encoding ASCII
      & diskpart /s $dpCleanup 2>&1 | Out-Null
    }
    if (Test-Path $vhd) { Remove-Item -Force $vhd -ErrorAction SilentlyContinue }
  }
}

# ---------------------------------------------------------------------------
# A9: server-enforced file lock e2e. user1 (alice) locks a shared path;
# user2 (bob) touching that path is rejected on push with the lock error
# string; force-unlock releases it; bob's retry succeeds.
# No-auth variant (same server - this qa-suite never enables auth): a
# no-auth server has no proven pusher/requester identity, so (a) a push from
# *anyone* touching a locked path is rejected regardless of who's asking, and
# (b) a plain (non-force) unlock always 403s, even for the "owner" name that
# created the lock - --force is the only way to release a lock at all.
# ---------------------------------------------------------------------------
function Drill-A9-LockE2E {
  $drill = "A9-lock-e2e"
  $drillNoAuth = "A9-lock-noauth-variant"
  $srv = $null
  $noAuthDone = $false
  try {
    $srv = Start-QaServer -Backend "minio" -Phase "$Phase-A9"
    $seed = New-SandboxRepo "a9-seed" $Phase
    New-QaBinaryFixture (Join-Path $seed "shared.bin") 4 79001
    Invoke-MG $seed @("add", ".") $Phase | Out-Null
    Invoke-MG $seed @("commit", "-m", "base") $Phase | Out-Null
    Invoke-MG $seed @("remote", "add", "origin", $srv.Url) $Phase | Out-Null
    Invoke-MG $seed @("push", "origin") $Phase -TimeoutSec 1200 | Out-Null

    $alice = Join-Path $QA.Work "a9-alice"; $bob = Join-Path $QA.Work "a9-bob"
    foreach ($c in @($alice, $bob)) { if (Test-Path $c) { Remove-Item -Recurse -Force $c } }
    Invoke-MG $null @("clone", $srv.Url, $alice) $Phase | Out-Null
    Invoke-MG $null @("clone", $srv.Url, $bob) $Phase | Out-Null

    # user1 (alice) locks the shared path
    $lc = Invoke-MG $alice @("lock", "create", "shared.bin", "--owner", "alice") $Phase
    $lockCreated = ($lc.Exit -eq 0) -and ($lc.Out -match "Locked")

    # duplicate lock attempt must surface the "already locked" error string
    $dup = Invoke-MG $bob @("lock", "create", "shared.bin", "--owner", "bob") $Phase
    $dupRejected = ($dup.Exit -ne 0) -and ($dup.Out -match "is already locked by")

    # user2 (bob) touches the locked path and pushes -> must be rejected
    New-QaBinaryFixture (Join-Path $bob "shared.bin") 4 79002
    Invoke-MG $bob @("add", "shared.bin") $Phase | Out-Null
    Invoke-MG $bob @("commit", "-m", "bob edits shared") $Phase | Out-Null
    $pushBlocked = Invoke-MG $bob @("push", "origin") $Phase
    $pushRejected = ($pushBlocked.Exit -ne 0) -and ($pushBlocked.Out -match "is locked by")

    # no-auth variant: this server proves no requester identity, so a plain
    # (non-force) unlock always 403s - even bob "naming himself" doesn't
    # matter, nobody can prove they're the owner without auth.
    $plainUnlock = Invoke-MG $bob @("lock", "unlock", "shared.bin") $Phase
    $plainUnlockRejected = ($plainUnlock.Exit -ne 0)

    Rec $drillNoAuth ($dupRejected -and $plainUnlockRejected) ("dup-lock-rejected=$dupRejected " +
      "(out has 'is already locked by') plain-unlock-rejected=$plainUnlockRejected (exit=$($plainUnlock.Exit), no-auth so no owner can be proven)")
    $noAuthDone = $true

    # force-unlock releases it - the only way anyone can unlock on a no-auth
    # deployment, per the doc comments on the server's delete_lock handler.
    $forceUnlock = Invoke-MG $bob @("lock", "unlock", "shared.bin", "--force") $Phase
    $forceOk = ($forceUnlock.Exit -eq 0)

    # user2 retries the push -> must now succeed
    $retryPush = Invoke-MG $bob @("push", "origin") $Phase -TimeoutSec 1200
    $retryOk = ($retryPush.Exit -eq 0)

    $reclone = Join-Path $QA.Work "a9-reclone"
    if (Test-Path $reclone) { Remove-Item -Recurse -Force $reclone }
    $cl = Invoke-MG $null @("clone", $srv.Url, $reclone) $Phase -TimeoutSec 1200
    $fsckOk = if ($cl.Exit -eq 0) { Test-QaFsckClean $reclone } else { $false }
    $bobHashOk = (Test-Path (Join-Path $reclone "shared.bin")) -and
                 ((Get-QaHash (Join-Path $reclone "shared.bin")) -eq (Get-QaHash (Join-Path $bob "shared.bin")))

    $pass = $lockCreated -and $pushRejected -and $forceOk -and $retryOk -and ($cl.Exit -eq 0) -and $fsckOk -and $bobHashOk
    Rec $drill $pass "lock-created=$lockCreated push-rejected=$pushRejected (out has 'is locked by') force-unlock=$forceOk retry-push=$retryOk clone=$($cl.Exit) fsck=$fsckOk hash-ok=$bobHashOk"
  } catch {
    $skip = "$_" -match "^SKIP:"
    $tag = if ($skip) { "SKIP" } else { $false }
    $detail = if ($skip) { "$_" } else { "unexpected error: $_" }
    if (-not $noAuthDone) { Rec $drillNoAuth $tag $detail }
    Rec $drill $tag $detail
  } finally { Stop-QaServer $srv }
}

# ---------------------------------------------------------------------------
# A10: batch-get-disabled fallback. Server started with
# MEDIAGIT_DISABLE_BATCH_GET=1 (POST /packs/batch-get always 404s - see the
# "Drill/compat knob ... QA A10" comment in chunks.rs); a clone of a
# pack-bearing repo must still succeed by falling back to the per-chunk path,
# hash-exact. The "server saw fewer batch requests" angle is logged as an
# informational, best-effort note only - MinIO always presigns direct-to-
# bucket transfers, so the server-side request log is not a reliable signal
# here and must never gate the drill.
# ---------------------------------------------------------------------------
function Drill-A10-BatchGetFallback {
  $drill = "A10-batch-get-fallback"
  $srv = $null
  $prevDisable = $env:MEDIAGIT_DISABLE_BATCH_GET
  try {
    $env:MEDIAGIT_DISABLE_BATCH_GET = "1"
    $srv = Start-QaServer -Backend "minio" -Phase "$Phase-A10"
    $repo = New-SandboxRepo "a10-src" $Phase
    # Several files across several commits so a push-side pack actually forms.
    for ($i = 1; $i -le 5; $i++) {
      New-QaBinaryFixture (Join-Path $repo "asset$i.bin") 6 (80000 + $i)
      Invoke-MG $repo @("add", "asset$i.bin") $Phase | Out-Null
      Invoke-MG $repo @("commit", "-m", "asset $i") $Phase | Out-Null
    }
    Invoke-MG $repo @("remote", "add", "origin", $srv.Url) $Phase | Out-Null
    $push = Invoke-MG $repo @("push", "origin") $Phase -TimeoutSec 1200

    $clone = Join-Path $QA.Work "a10-clone"
    if (Test-Path $clone) { Remove-Item -Recurse -Force $clone }
    $cl = Invoke-MG $null @("clone", $srv.Url, $clone) $Phase -TimeoutSec 1800

    $hashesOk = $true
    for ($i = 1; $i -le 5; $i++) {
      $srcH = Get-QaHash (Join-Path $repo "asset$i.bin")
      $dstPath = Join-Path $clone "asset$i.bin"
      if (-not (Test-Path $dstPath) -or ((Get-QaHash $dstPath) -ne $srcH)) { $hashesOk = $false }
    }
    $fsckOk = if ($cl.Exit -eq 0) { Test-QaFsckClean $clone } else { $false }

    # Best-effort, informational only - never a gate (see header comment).
    # Glob, don't hardcode: server logs carry a per-phase sequence suffix
    # (lib\remote.ps1) so that multiple servers in one phase stop truncating each
    # other's log. A hardcoded name would silently match nothing and report 0
    # hits, which reads identically to "the feature never fired".
    $srvLogs = @(Get-ChildItem -Path $QA.Logs -Filter "server-minio-$Phase-A10*.out.log" -ErrorAction SilentlyContinue)
    $batchHits = 0; $chunkHits = 0
    foreach ($sl in $srvLogs) {
      $batchHits += (Select-String -Path $sl.FullName -Pattern "packs/batch-get" -ErrorAction SilentlyContinue | Measure-Object).Count
      $chunkHits += (Select-String -Path $sl.FullName -Pattern "/chunks/" -ErrorAction SilentlyContinue | Measure-Object).Count
    }
    Write-QaLog $Phase ("A10 informational (best-effort, not gated): server-log batch-get hits=$batchHits " +
      "per-chunk hits=$chunkHits - MinIO presigns direct-to-bucket transfers, so this count is not a reliable signal")

    $pass = ($push.Exit -eq 0) -and ($cl.Exit -eq 0) -and $hashesOk -and $fsckOk
    Rec $drill $pass "push=$($push.Exit) clone=$($cl.Exit) hashes-ok=$hashesOk fsck=$fsckOk batch-get-disabled=true batch-hits=$batchHits chunk-hits=$chunkHits"
  } catch {
    if ("$_" -match "^SKIP:") { Rec $drill "SKIP" "$_" } else { Rec $drill $false "unexpected error: $_" }
  } finally {
    Stop-QaServer $srv
    $env:MEDIAGIT_DISABLE_BATCH_GET = $prevDisable
  }
}


# ---------------------------------------------------------------------------
# A11: chunk-delta chain depth under a run of similar versions. Editing and
# re-committing the same large asset repeatedly makes each new chunk a delta of
# the previous one; with no write-side depth guard the chain grows past what
# get_chunk will reconstruct (MAX_DELTA_DEPTH = 10) and the repo becomes
# permanently unpushable and unclonable - the 624MB psds failure, where push
# died with "Chunk delta chain too deep (> 10)" and fsck --repair could not fix
# it. Gate: chains stay within the limit, content round-trips, push succeeds.
# ---------------------------------------------------------------------------
function Drill-A11-DeltaChainDepth {
  $drill = "A11-delta-chain-depth"
  $srv = $null
  try {
    # The depth assertion is LOCAL and deliberately runs before any server is
    # started. This drill guards a data-loss defect, so it must never report
    # SKIP-as-PASS because some remote dependency was unavailable - which is
    # exactly what happened on 2026-07-23 (a namespace collision skipped the
    # drill and it was still recorded True in gates.tsv).
    $repo = New-SandboxRepo "a11-chain-depth" $Phase
    $asset = Join-Path $repo "asset.psd"

    # 25 mutually-similar files staged in ONE `add`, each a small CUMULATIVE
    # edit of the previous. Both properties are load-bearing, measured
    # 2026-07-23 against the pre-fix binary:
    #
    #   * ONE add - within a single invocation the similarity detector
    #     accumulates every chunk just written, so file N+1 nominates file N's
    #     chunk, which is itself already a delta. Across separate commits the
    #     detector is only seeded from the previous manifest and that seeding
    #     skips bases deeper than 2, so one-file-per-commit tops out at
    #     depth 1 and would NOT reproduce the defect (this drill did exactly
    #     that on its first run and passed while discriminating nothing).
    #   * CUMULATIVE edits - independent edits off one base are each
    #     most-similar to that base, giving depth 1 and no chain.
    #
    # Pre-fix this shape measured depth 12; post-fix it stays <= 10.
    New-QaBinaryFixture $asset 6 91101
    $cur = [IO.File]::ReadAllBytes($asset)
    Remove-Item $asset -Force
    $versions = 25
    for ($v = 0; $v -lt $versions; $v++) {
      for ($k = 0; $k -lt 4096; $k++) {
        $idx = ($v * 65536 + $k) % $cur.Length
        $cur[$idx] = [byte]((($v * 7 + $k) -band 0x7F) -bor 0x80)
      }
      [IO.File]::WriteAllBytes((Join-Path $repo "asset$v.psd"), $cur)
    }
    Invoke-MG $repo @("add", ".") $Phase -TimeoutSec 1800 | Out-Null
    Invoke-MG $repo @("commit", "-m", "all versions") $Phase | Out-Null
    $asset = Join-Path $repo ("asset" + ($versions - 1) + ".psd")
    $finalHash = Get-QaHash $asset

    $stats = Get-QaChainStats $repo
    $depthOk = ($stats.MaxDepth -le 10)
    $noCycles = ($stats.CycleCount -eq 0)
    # A run this long with no deltas at all would make the depth check vacuous.
    $exercised = ($stats.ChainCount -gt 0)
    $fsckOk = Test-QaFsckClean $repo

    $localPass = $depthOk -and $noCycles -and $exercised -and $fsckOk
    $detail = ("maxDepth=$($stats.MaxDepth) (limit=10) cycles=$($stats.CycleCount) " +
      "chains=$($stats.ChainCount) exercised=$exercised fsck=$fsckOk")

    # Remote half: the reported symptom was push failing on an unreadable
    # chunk, and a clone must reproduce the content byte-exactly. If no server
    # is available this degrades to "local-only" - it never turns the local
    # verdict into a pass.
    try {
      $srv = Start-QaServer -Backend "minio" -Phase "$Phase-A11"
      Invoke-MG $repo @("remote", "add", "origin", $srv.Url) $Phase | Out-Null
      $push = Invoke-MG $repo @("push", "-u", "origin", "main") $Phase -TimeoutSec 1800
      $clone = Join-Path $QA.Work "a11-clone"
      if (Test-Path $clone) { Remove-Item -Recurse -Force $clone }
      $cl = Invoke-MG $null @("clone", $srv.Url, $clone) $Phase -TimeoutSec 1800
      $leaf = "asset" + ($versions - 1) + ".psd"
      $cloneHashOk = (Test-Path (Join-Path $clone $leaf)) -and
                     ((Get-QaHash (Join-Path $clone $leaf)) -eq $finalHash)
      $remotePass = ($push.Exit -eq 0) -and ($cl.Exit -eq 0) -and $cloneHashOk
      Rec $drill ($localPass -and $remotePass) ($detail +
        " push=$($push.Exit) clone=$($cl.Exit) clone-hash-ok=$cloneHashOk")
    } catch {
      if ("$_" -match "^SKIP:") {
        # Local half still gates - report its real verdict, not SKIP.
        Rec $drill $localPass ($detail + " remote=SKIPPED ($_)")
      } else { throw }
    }
  } catch {
    Rec $drill $false "unexpected error: $_"
  } finally { Stop-QaServer $srv }
}

# ---------------------------------------------------------------------------
# A12: fabricated cyclic chunk-delta chains (A->B->A and an A->A self-loop).
# fsck must DETECT them, must TERMINATE (a naive walk spins forever), and must
# never report the repo clean. Guards the 2026-07-07 cycle fix, which had no
# campaign coverage. Sidecars alone suffice - the chain walk reads only .meta,
# never chunk payloads.
# ---------------------------------------------------------------------------
function Drill-A12-DeltaChainCycle {
  $drill = "A12-delta-chain-cycle"
  try {
    $repo = New-SandboxRepo "a12-chain-cycle" $Phase
    New-QaBinaryFixture (Join-Path $repo "seed.bin") 2 91201
    Invoke-MG $repo @("add", ".") $Phase | Out-Null
    Invoke-MG $repo @("commit", "-m", "seed") $Phase | Out-Null

    # Namespace is the repo dir name, not a literal "repo" - resolve it, or the
    # fabricated sidecars land somewhere fsck never looks and the drill passes
    # vacuously.
    $deltaDir = Get-QaChunkDeltaDir $repo
    if (-not $deltaDir) {
      $ns = Get-ChildItem (Join-Path $repo ".mediagit\objects") -Directory | Select-Object -First 1
      $deltaDir = Join-Path $ns.FullName "chunk-deltas"
    }
    New-Item -ItemType Directory -Path $deltaDir -Force | Out-Null

    # Sidecars must land in the ODB's real TWO-LEVEL shard layout
    # (chunk-deltas/<c0c1>/<c2c3>/<id>.meta). Writing them flat under
    # chunk-deltas/ puts them somewhere fsck never enumerates: measured
    # 2026-07-23, flat => "Repository integrity: PERFECT" while a cycle sat on
    # disk; correctly sharded => "Integrity check failed with 2 error(s)".
    function Write-QaFakeDeltaMeta([string]$Root, [string]$Id, [string]$Base) {
      $shard = Join-Path (Join-Path $Root $Id.Substring(0, 2)) $Id.Substring(2, 2)
      New-Item -ItemType Directory -Path $shard -Force | Out-Null
      Set-Content -NoNewline -Path (Join-Path $shard "$Id.meta") -Value "base:$Base"
    }

    # Two-node cycle A->B->A, plus a degenerate self-loop C->C.
    $a = "a" * 64
    $b = "b" * 64
    $c = "c" * 64
    Write-QaFakeDeltaMeta $deltaDir $a $b
    Write-QaFakeDeltaMeta $deltaDir $b $a
    Write-QaFakeDeltaMeta $deltaDir $c $c

    # Harness-side view must see them too (and must not hang).
    $stats = Get-QaChainStats $repo
    $statsSawCycle = ($stats.CycleCount -gt 0)

    # fsck must terminate; a spin would blow the timeout instead of returning.
    $sw = [Diagnostics.Stopwatch]::StartNew()
    $f = Invoke-MG $repo @("fsck") $Phase -TimeoutSec 300
    $sw.Stop()
    $terminated = ($sw.Elapsed.TotalSeconds -lt 300)
    $detected = ($f.Exit -ne 0) -or ($f.Out -match "(?i)cycle|circular")
    $notSilent = -not (Test-QaFsckClean $repo)

    $pass = $statsSawCycle -and $terminated -and $detected -and $notSilent
    Rec $drill $pass ("harness-saw-cycle=$statsSawCycle fsck-detected=$detected " +
      "terminated=$terminated ($([math]::Round($sw.Elapsed.TotalSeconds,1))s) not-silent=$notSilent")
  } catch {
    if ("$_" -match "^SKIP:") { Rec $drill "SKIP" "$_" } else { Rec $drill $false "unexpected error: $_" }
  }
}

# ---------------------------------------------------------------------------
# A13: per-chunk fallback must not become a request storm. When the pack path
# fails, push falls back to one request per chunk; against the default per-IP
# rate limiter that produced the 429s which masked the real corruption.
# Gate: with packs disabled, a large push still completes under DEFAULT limits.
# ---------------------------------------------------------------------------
function Drill-A13-PerChunkFallbackNoRateLimit {
  $drill = "A13-per-chunk-fallback-no-429"
  $srv = $null
  # MEDIAGIT_CLOUD_PACKS is the knob the PUSH path actually reads
  # (push.rs: unwrap_or("1")). The earlier version of this drill set
  # MEDIAGIT_PACK_ENABLED, which the push path never reads — so packs stayed
  # on and the per-chunk fallback was never exercised (a vacuous pass).
  $prevPack = $env:MEDIAGIT_CLOUD_PACKS
  try {
    $env:MEDIAGIT_CLOUD_PACKS = "0"
    # Distinct phase tag: sharing the bare $Phase reuses a repo namespace an
    # earlier drill already claimed in the bucket, and the server's collision
    # guard then refuses to start.
    $srv = Start-QaServer -Backend "minio" -Phase "$Phase-A13"
    $repo = New-SandboxRepo "a13-fallback" $Phase
    # Several files so the per-chunk path issues many individual requests.
    for ($i = 0; $i -lt 6; $i++) {
      New-QaBinaryFixture (Join-Path $repo "part$i.bin") 8 (91300 + $i)
    }
    Invoke-MG $repo @("add", ".") $Phase | Out-Null
    Invoke-MG $repo @("commit", "-m", "fallback") $Phase | Out-Null
    Invoke-MG $repo @("remote", "add", "origin", $srv.Url) $Phase | Out-Null

    $push = Invoke-MG $repo @("push", "-u", "origin", "main") $Phase -TimeoutSec 1800
    $rateLimited = ($push.Out -match "(?i)429|rate.?limit|too many requests")

    # Anti-vacuous: prove the fallback was ACTUALLY taken. With packs off there
    # must be ZERO `packs/` endpoint hits. The per-chunk path itself may still
    # move bytes either via presigned per-chunk URLs (`chunks/upload-urls`,
    # direct-to-bucket on MinIO) OR proxy `PUT /chunks/<hex>` — both are valid
    # fallback shapes, so "took fallback" = no packs AND some chunk activity.
    # NOTE the two `upload-urls` endpoints are distinct: `packs/upload-urls` is
    # the pack path, `chunks/upload-urls` is the fallback — matching them
    # together (an earlier bug) conflates the very thing this drill separates.
    $packHits = 0
    $chunkUrlMints = 0
    $chunkProxyPuts = 0
    if ($srv.OutLog -and (Test-Path $srv.OutLog)) {
      $log = Get-Content $srv.OutLog -Raw -EA SilentlyContinue
      if ($log) {
        $packHits = ([regex]::Matches($log, 'packs/upload-urls|packs/presign|Presigned pack')).Count
        $chunkUrlMints = ([regex]::Matches($log, 'chunks/upload-urls')).Count
        $chunkProxyPuts = ([regex]::Matches($log, 'PUT /[^ ]*/chunks/[0-9a-f]')).Count
      }
    }
    $chunkActivity = ($chunkUrlMints + $chunkProxyPuts)
    $tookFallback = ($packHits -eq 0) -and ($chunkActivity -gt 0)

    $pass = ($push.Exit -eq 0) -and (-not $rateLimited) -and $tookFallback
    Rec $drill $pass ("push=$($push.Exit) rate-limited=$rateLimited " +
      "pack-hits=$packHits chunk-url-mints=$chunkUrlMints chunk-proxy-puts=$chunkProxyPuts " +
      "took-fallback=$tookFallback (packs off via MEDIAGIT_CLOUD_PACKS, default limits)")
  } catch {
    if ("$_" -match "^SKIP:") { Rec $drill "SKIP" "$_" } else { Rec $drill $false "unexpected error: $_" }
  } finally {
    Stop-QaServer $srv
    $env:MEDIAGIT_CLOUD_PACKS = $prevPack
  }
}

# ---------------------------------------------------------------------------
Write-QaLog $Phase "=== 07_abuse start ==="

Drill-A1-KillMidAdd
Drill-A2-KillMidPush
Drill-A3-ConcurrentDoublePush
Drill-A4-CorruptChunkAtRest
Drill-A5-ReadOnlyFile
Drill-A6-SpacesAndUnicodePaths
Drill-A7-BackendOutage
Drill-A8-DiskFull
Drill-A9-LockE2E
Drill-A10-BatchGetFallback
Drill-A11-DeltaChainDepth
# ---------------------------------------------------------------------------
# A14: presigned PUT on the UNBOUND path. The server signs a content-length
# into the URL whenever the client can tell it one; that signed header is then
# part of SignedHeaders, and the client must NOT add its own (doing so appended
# a second header and produced SignatureDoesNotMatch - 976+748 of them across
# two campaigns, fixed in 82977f5).
#
# The complementary branch - the client supplying content-length itself because
# the signature does NOT commit to one - had only unit coverage. It is reached
# only when `compressed_chunk_len` returns None, i.e. a chunk with no loose copy
# at `chunks/<hex>`: delta-encoded or gc-repacked (odb/chunks.rs:2543,
# push.rs:851). A13 cannot reach it - fresh fixtures are all loose.
#
# So: build the repo, `gc --repack` to pack the loose chunks away, THEN push
# with packs off. Anti-vacuous: the server now reports `unbound=N` on its
# presign log line, so this drill can prove the branch was entered rather than
# assume it. unbound=0 fails deliberately - it means the condition was never
# created and any "pass" would be empty.
# ---------------------------------------------------------------------------
function Drill-A14-UnboundPresignedPut {
  $drill = "A14-unbound-presigned-put"
  $srv = $null
  $prevPack = $env:MEDIAGIT_CLOUD_PACKS
  try {
    $env:MEDIAGIT_CLOUD_PACKS = "0"
    $srv = Start-QaServer -Backend "minio" -Phase "$Phase-A14"
    $repo = New-SandboxRepo "a14-unbound" $Phase

    # Compressible content on purpose: the original bug only bit when the
    # compressed length differed from the uncompressed one.
    for ($i = 0; $i -lt 3; $i++) {
      $txt = Join-Path $repo "doc$i.txt"
      (1..4000 | ForEach-Object { "line $_ of document $i - repetitive compressible payload" }) |
        Set-Content $txt -Encoding ASCII
    }
    New-QaBinaryFixture (Join-Path $repo "asset.bin") 8 77120
    Invoke-MG $repo @("add", ".") $Phase | Out-Null
    Invoke-MG $repo @("commit", "-m", "a14 base") $Phase | Out-Null

    # Pack the loose chunks away so their length is no longer cheaply knowable.
    $gc = Invoke-MG $repo @("gc", "--repack", "-y") $Phase -TimeoutSec 900

    $srcHashes = @{}
    Get-ChildItem $repo -File | ForEach-Object { $srcHashes[$_.Name] = (Get-QaHash $_.FullName) }

    Invoke-MG $repo @("remote", "add", "origin", $srv.Url) $Phase | Out-Null
    $push = Invoke-MG $repo @("push", "-u", "origin", "main") $Phase -TimeoutSec 1800

    # Did any URL actually get signed WITHOUT a content-length?
    $unbound = 0
    $presignCalls = 0
    if ($srv.OutLog -and (Test-Path $srv.OutLog)) {
      $log = Get-Content $srv.OutLog -Raw -EA SilentlyContinue
      if ($log) {
        $presignCalls = ([regex]::Matches($log, 'Presigned chunk upload URLs generated')).Count
        foreach ($m in [regex]::Matches($log, 'unbound[=:]\s*(\d+)')) {
          $unbound += [int]$m.Groups[1].Value
        }
      }
    }

    # Bytes must survive the round trip - a signature the peer accepts is not
    # the same claim as a body it stored intact.
    $back = Join-Path $QA.Work "a14-clone"
    if (Test-Path $back) { Remove-Item -Recurse -Force $back -EA SilentlyContinue }
    $clone = Invoke-MG $null @("clone", $srv.Url, $back) $Phase -TimeoutSec 1800
    $hashOk = $true
    foreach ($name in $srcHashes.Keys) {
      $f = Join-Path $back $name
      if (-not (Test-Path $f) -or ((Get-QaHash $f) -ne $srcHashes[$name])) { $hashOk = $false }
    }
    $fsckOk = if (Test-Path $back) { Test-QaFsckClean $back } else { $false }
    Remove-Item -Recurse -Force $back -EA SilentlyContinue

    $exercised = ($unbound -gt 0)
    $pass = ($push.Exit -eq 0) -and ($clone.Exit -eq 0) -and $hashOk -and $fsckOk -and $exercised
    $note = if ($exercised) { "" } else {
      " -- UNBOUND PATH NOT EXERCISED: gc --repack may not pack chunks, or the branch is unreachable in practice. Investigate before treating this as covered." }
    Rec $drill $pass ("gc=$($gc.Exit) push=$($push.Exit) clone=$($clone.Exit) hash-ok=$hashOk fsck=$fsckOk " +
      "presign-calls=$presignCalls unbound=$unbound exercised=$exercised$note")
  } catch {
    if ("$_" -match "^SKIP:") { Rec $drill "SKIP" "$_" } else { Rec $drill $false "unexpected error: $_" }
  } finally {
    Stop-QaServer $srv
    $env:MEDIAGIT_CLOUD_PACKS = $prevPack
  }
}

Drill-A12-DeltaChainCycle
Drill-A13-PerChunkFallbackNoRateLimit
Drill-A14-UnboundPresignedPut

Write-QaLog $Phase "=== 07_abuse done: overall=$(if ($script:AllPass) { 'PASS' } else { 'FAIL' }) ==="
# Teardown: reclaim this phase's own work/ scratch so a long campaign cannot run the
# volume out of space. work/ ONLY - logs/ and fixtures-synthetic/ are never touched.
Invoke-QaTeardown $Phase @("a[0-9]*")

Exit-QaPhase $Phase (-not $script:AllPass)
