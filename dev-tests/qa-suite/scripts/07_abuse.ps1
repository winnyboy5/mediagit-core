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

# A32-byte MEDIAGIT_ENCRYPTION_KEYFILE master key. Raw bytes, not hex text - one of
# the two shapes encryption.rs's keyfile_master() accepts (the other is 64 hex chars).
function New-QaEncryptionKeyfile([string]$Path) {
  $dir = Split-Path $Path -Parent
  if (-not (Test-Path $dir)) { New-Item -ItemType Directory -Path $dir -Force | Out-Null }
  $bytes = New-Object byte[] 32
  (New-Object Security.Cryptography.RNGCryptoServiceProvider).GetBytes($bytes)
  [IO.File]::WriteAllBytes($Path, $bytes)
}

# Every regular file under .mediagit/objects, examined for the MGEN envelope magic
# (mediagit-security/src/envelope.rs). Mirrors cli_encryption_test.rs's own
# object_files()/any_object_is_sealed() helpers - same question, asked from the
# outside: does the ODB actually contain sealed bytes, not just an unlocked repo.
function Get-QaEncryptionSealStats([string]$Repo) {
  $stats = @{ Examined = 0; Sealed = 0 }
  $objRoot = Join-Path $Repo ".mediagit\objects"
  if (-not (Test-Path $objRoot)) { return $stats }
  Get-ChildItem $objRoot -Recurse -File -ErrorAction SilentlyContinue | ForEach-Object {
    $head = New-Object byte[] 4
    $fs = $null
    try {
      $fs = [IO.File]::OpenRead($_.FullName)
      $n = $fs.Read($head, 0, 4)
    } catch { $n = 0 } finally { if ($fs) { $fs.Dispose() } }
    if ($n -eq 4) {
      $stats.Examined++
      if ([Text.Encoding]::ASCII.GetString($head) -eq "MGEN") { $stats.Sealed++ }
    }
  }
  return $stats
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
  # 600MB, for exactly the reason A2 below is 600MB. Observed in 20260818-gagate4:
  # a 200MB add finished inside the 1.5s sleep, so `$p.HasExited` was already true,
  # Stop-Process never fired, and the drill recorded PASS with killed=False -
  # having tested "add, add again, commit, fsck" and nothing about surviving a
  # kill. The arming assertion below is what stops that reading as success.
  New-QaBinaryFixture (Join-Path $repo "big.bin") 600 71001
  $origHash = Get-QaHash (Join-Path $repo "big.bin")
  $p = Start-Process $QA.MG -ArgumentList @("-C", $repo, "add", "big.bin") -PassThru -NoNewWindow `
    -RedirectStandardOutput (Join-Path $QA.Logs "a1-add.out") -RedirectStandardError (Join-Path $QA.Logs "a1-add.err")
  # Kill EARLY, and stop racing a fixed sleep.
  #
  # 1500ms was a coin flip: standalone the 600MB add ran past it (killed=True),
  # but inside a campaign the same add finished first (killed=False, gagate6)
  # because the fixture was still warm in the page cache. A drill whose arming
  # depends on which way that race lands is not a drill.
  #
  # 250ms cannot be beaten by a 600MB add - that would need ~2.4 GB/s end to
  # end, including chunking and BLAKE3 - while still landing well inside the
  # operation. Polled rather than slept in one go so the kill goes in at the
  # first opportunity.
  $killed = $false
  for ($waited = 0; $waited -lt 250; $waited += 25) {
    Start-Sleep -Milliseconds 25
    if ($p.HasExited) { break }
  }
  if (-not $p.HasExited) { Stop-Process -Id $p.Id -Force; $killed = $true }
  $fsck1 = Test-QaFsckClean $repo
  $retry = Invoke-MG $repo @("add", "big.bin") $Phase -TimeoutSec 1200
  $cmt = Invoke-MG $repo @("commit", "-m", "after kill") $Phase
  $fsck2 = Test-QaFsckClean $repo
  $hashOk = (Get-QaHash (Join-Path $repo "big.bin")) -eq $origHash
  # $killed is an ARMING condition, not decoration: without it this drill cannot
  # fail for the reason it exists, because everything below is equally true of an
  # add that was never interrupted.
  $pass = $killed -and $fsck2 -and ($retry.Exit -eq 0) -and ($cmt.Exit -eq 0) -and $hashOk
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

  # How this drill takes the backend down and brings it back.
  #
  # It used to `docker stop mediagit-minio` unconditionally. When the S3 backend
  # moved to a NATIVE Silo process on 2026-08-21 there was no container, so the
  # capability check below SKIPped - and a SKIP reads as green. The one drill
  # that proves the product survives a mid-push backend outage silently stopped
  # testing anything.
  #
  # MG_QA_BACKEND_STOP_CMD / _START_CMD make the mechanism explicit, so the
  # drill works against any topology (native process, container, remote host)
  # instead of assuming Docker. Docker remains the default when a container is
  # actually there, so existing setups are unchanged.
  $stopCmd  = $env:MG_QA_BACKEND_STOP_CMD
  $startCmd = $env:MG_QA_BACKEND_START_CMD
  $useCmds  = $stopCmd -and $startCmd

  if (-not $useCmds -and -not (Test-QaDockerAvailable $container)) {
    Rec $drill "SKIP" ("no way to cycle the backend: docker container '$container' not reachable " +
                       "AND MG_QA_BACKEND_STOP_CMD/_START_CMD unset. " +
                       "Set both to a shell command that stops/starts your S3 backend " +
                       "(native Silo: silo_native.ps1 -Action stop / -Action start).")
    return
  }

  # One place each, so the retry path below cannot drift from the primary path.
  $StopBackend = {
    if ($useCmds) { & powershell -NoProfile -Command $stopCmd *> $null }
    else          { & docker stop $container *> $null }
  }
  $StartBackend = {
    if ($useCmds) { & powershell -NoProfile -Command $startCmd *> $null }
    else          { & docker start $container *> $null }
  }
  $RestartBackend = {
    if ($useCmds) {
      & powershell -NoProfile -Command $stopCmd *> $null
      & powershell -NoProfile -Command $startCmd *> $null
    } else { & docker restart $container *> $null }
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
    # Cache the handle, exactly as A15 does and for the same reason: without it
    # Start-Process -PassThru hands back an object whose .ExitCode reads $null
    # after exit. `$null -ne 0` is TRUE in PowerShell, so the clean-fail
    # assertion below would pass on an exit code that was never read. Observed
    # here: the drill reported `push-exit=` blank and PASS in the same line.
    $null = $p.Handle
    Start-Sleep -Milliseconds 2000
    & $StopBackend
    $stoppedContainer = $true

    # Progress markers through the recovery sequence.
    #
    # A7 has hung twice inside a campaign (ga10) and once standalone, and every
    # time the evidence was the same useless shape: a phase log that stops after
    # "server up" and never says another word. Every step below can block - two
    # of them shell out to a backend-cycling command supplied from outside the
    # suite - so "which one" has to be recorded as it happens, not inferred
    # afterwards from a log that ends mid-drill.
    Write-QaLog $Phase "A7: push started, backend stopped; waiting for client exit"
    $exited = $p.WaitForExit(120000)
    # Read the code only once the process has really gone, and record whether it
    # could be read at all. "we could not read the exit code" must FAIL, not be
    # silently promoted into evidence of a clean refusal.
    $exitCode = -1
    $exitRead = $false
    if ($exited) {
      try { $p.WaitForExit(5000) | Out-Null } catch { }
      try { $exitCode = $p.ExitCode; $exitRead = ($exitCode -is [int]) } catch { $exitRead = $false }
    }
    Write-QaLog $Phase "A7: client exited=$exited exit=$exitCode read=$exitRead"
    Remove-Item Env:\MEDIAGIT_PUSH_DEADLINE_SECS -ErrorAction SilentlyContinue
    if (-not $exited) { Stop-Process -Id $p.Id -Force -ErrorAction SilentlyContinue }
    $outText = "" + (Get-Content (Join-Path $QA.Logs "a7-push.out") -Raw -ErrorAction SilentlyContinue) `
                   + (Get-Content (Join-Path $QA.Logs "a7-push.err") -Raw -ErrorAction SilentlyContinue)
    $panic = $outText -match "panicked"
    # $exitRead is load-bearing and separate from the code itself: an unreadable
    # exit code must fail the drill rather than be read as a clean refusal.
    $cleanFail = $exited -and $exitRead -and ($exitCode -ne 0) -and (-not $panic)

    Write-QaLog $Phase "A7: restarting backend"
    & $StartBackend
    $stoppedContainer = $false
    Write-QaLog $Phase "A7: backend start command returned; polling health"
    $up = Wait-QaMinioUp $QA.MinioEndpoint 30
    if (-not $up) {
      # Docker Desktop's host port-proxy can stay wedged after `docker start`
      # (seen 2026-07-19: container healthy, localhost:9000 dead for 14+ min).
      # A full `docker restart` rebinds it; one retry keeps A7 from cascading
      # into A9/A10 SKIPs on what is a host-networking hiccup, not a product bug.
      Write-QaLog $Phase "A7: host port not back after docker start; retrying with docker restart $container"
      & $RestartBackend
      $up = Wait-QaMinioUp $QA.MinioEndpoint 60
    }

    Write-QaLog $Phase "A7: backend up=$up; running local fsck"
    $fsckLocal = Test-QaFsckClean $repo
    Write-QaLog $Phase "A7: local fsck=$fsckLocal; retrying push"
    $retry = Invoke-MG $repo @("push", "origin") $Phase -TimeoutSec 3600
    Write-QaLog $Phase "A7: retry push exit=$($retry.Exit); cloning"
    $clone = Join-Path $QA.Work "a7-clone"
    if (Test-Path $clone) { Remove-Item -Recurse -Force $clone }
    $cl = Invoke-MG $null @("clone", $srv.Url, $clone) $Phase -TimeoutSec 3600
    Write-QaLog $Phase "A7: clone exit=$($cl.Exit); verifying"
    $cloneHashOk = (Test-Path (Join-Path $clone "big.bin")) -and
                   ((Get-QaHash (Join-Path $clone "big.bin")) -eq $origHash)
    $fsckClone = if ($cl.Exit -eq 0) { Test-QaFsckClean $clone } else { $false }

    $pass = $up -and $cleanFail -and $fsckLocal -and ($retry.Exit -eq 0) -and ($cl.Exit -eq 0) -and $cloneHashOk -and $fsckClone
    Rec $drill $pass "minio-restarted=$up push-exited=$exited push-exit=$exitCode exit-read=$exitRead panic=$panic clean-fail=$cleanFail local-fsck=$fsckLocal retry-push=$($retry.Exit) clone=$($cl.Exit) clone-hash-ok=$cloneHashOk clone-fsck=$fsckClone"
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
    # Actionable, because this skip is permanent otherwise: it has been the one
    # standing unexpected-skip in every campaign, and a skip nobody knows how to
    # clear eventually reads as "this drill does not exist". There is no
    # non-elevated equivalent on Windows -- diskpart, New-VHD and fsutil quota
    # all require it -- so the only alternatives are an elevated shell or a
    # fault-injection hook in the product, and the latter is not worth a test.
    Rec $drill "SKIP" ("requires admin: a size-capped volume needs diskpart 'attach vdisk'. " +
      "To run it, start the campaign from an elevated PowerShell " +
      "(Start-Process powershell -Verb RunAs) and re-run run_all.ps1; everything " +
      "else in the suite runs unelevated.")
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
    # Rate limiting is read from the SERVER's response statuses, not by grepping
    # the client's stdout.
    #
    # The old check was `$push.Out -match "(?i)429|rate.?limit|too many requests"`.
    # 20260824-ga13 failed it on this line:
    #
    #   Created commit 429fb41bc77390ae2f4206a8aff13c7704e6ab57c0eb2f5e7ec2d9d1...
    #
    # The bare `429` matched the first three hex digits of a BLAKE3 commit OID, so
    # roughly one push in 4096 failed this gate no matter how the code behaved.
    # Worse, the textual half could never fire either: the client emits no
    # user-visible "429" or "rate limit" text at all (the only hits in the source
    # are test assertions), so the whole check was a false-positive-only detector.
    #
    # `status=429` on the server's own tower_http response line is the ground
    # truth for "the server rate limited us", comes from the log this drill
    # already reads, and cannot collide with a hash.
    $rateLimited = $false

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
        $rl = ([regex]::Matches($log, 'status=429')).Count
        $rateLimited = ($rl -gt 0)
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

# MG_QA_DRILLS runs just the named drills; empty = all. Same shape as 10_scale.
# Each drill builds its own fixtures and its own server, so any subset is valid.
#
# This exists so a single drill can be reproduced standalone instead of only
# inside a 2-hour campaign. A7 is the case in point: it had to be investigated
# in isolation before its native-backend cycling could be trusted, and there was
# no way to run it alone. e.g. MG_QA_DRILLS="A7".
$only = ($env:MG_QA_DRILLS -split "," | ForEach-Object { $_.Trim().ToUpper() } | Where-Object { $_ })
function _Want([string]$s) { -not $only -or ($only -contains $s) }

# ANNOUNCE a filtered run, loudly.
#
# Honouring MG_QA_DRILLS is what makes a single drill reproducible standalone,
# but it also creates a way for a campaign to run one drill instead of
# seventeen and still report the phase green - a stale variable left in the
# shell (run_a7_native.ps1 sets it, and run_ga.ps1 does not clear it) is all it
# takes. A skipped drill that says nothing is the defect class this suite keeps
# hitting, so a filtered run has to be visible in the log rather than inferred
# afterwards from a suspiciously short gate list.
if ($only) {
    Write-QaLog $Phase ("MG_QA_DRILLS is set - running ONLY: " + ($only -join ",") +
        ". This is a PARTIAL phase; a campaign must run with it unset.")
}

if (_Want "A1")  { Drill-A1-KillMidAdd }
if (_Want "A2")  { Drill-A2-KillMidPush }
if (_Want "A3")  { Drill-A3-ConcurrentDoublePush }
if (_Want "A4")  { Drill-A4-CorruptChunkAtRest }
if (_Want "A5")  { Drill-A5-ReadOnlyFile }
if (_Want "A6")  { Drill-A6-SpacesAndUnicodePaths }
if (_Want "A7")  { Drill-A7-BackendOutage }
if (_Want "A8")  { Drill-A8-DiskFull }
if (_Want "A9")  { Drill-A9-LockE2E }
if (_Want "A10") { Drill-A10-BatchGetFallback }
if (_Want "A11") { Drill-A11-DeltaChainDepth }
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
        # Strip ANSI first. tracing's pretty formatter writes a field as
        # `<esc>[3munbound<esc>[0m<esc>[2m=<esc>[0m5`, so a regex expecting
        # `unbound=` matches NOTHING and the count silently reads 0 — which
        # looked exactly like "the branch was never entered" on the first run
        # of this drill, when the server had in fact reported unbound=5.
        # Any regex over a server log needs this.
        $log = $log -replace '\x1b\[[0-9;]*m', ''
        $presignCalls = ([regex]::Matches($log, 'Presigned chunk upload URLs generated')).Count
        foreach ($m in [regex]::Matches($log, 'unbound\s*=\s*(\d+)')) {
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

function Drill-A15-SecondInstanceRefused {
  # AU-10. Two servers sharing one repos_dir silently discard each other's auth
  # writes, lock records and delta-chain guards. Nothing about that fails
  # loudly, so the only enforceable place is startup.
  $drill = "A15-second-instance-refused"
  $srv = $null
  # Clear the escape hatch for the refusal half. If it is set in the ambient
  # environment - by a developer, or by the override half below leaking - the
  # second instance STARTS, and this drill would report PASS while proving the
  # exact opposite of what it exists to prove. Found by red-verifying: with the
  # var set, the drill passed.
  $prevAllowOuter = $env:MEDIAGIT_ALLOW_MULTI_INSTANCE
  Remove-Item Env:MEDIAGIT_ALLOW_MULTI_INSTANCE -EA SilentlyContinue
  try {
    $srv = Start-QaServer -Backend "local" -Phase "$Phase-A15"

    # A DIFFERENT port on purpose. Reusing the first server's port would make
    # the second process die on bind, and the drill would pass while proving
    # nothing about the lock -- which is precisely the deployment that eats
    # data: two instances, different ports, one directory.
    $otherPort = Get-QaFreePort
    $out2 = Join-Path $QA.Logs "a15-second-instance.out.log"
    $err2 = Join-Path $QA.Logs "a15-second-instance.err.log"
    $p2 = Start-Process -FilePath $QA.MGServer `
      -ArgumentList @("--config", $srv.ConfigPath, "--port", "$otherPort") `
      -PassThru -NoNewWindow -RedirectStandardOutput $out2 -RedirectStandardError $err2
    # Touching .Handle caches the process handle while the process is still
    # alive. Without it, Start-Process -PassThru hands back an object whose
    # .ExitCode reads $null after exit - and `$null -ne 0` is TRUE, so the
    # assertion below passed on an exit code that was never read. Probed: the
    # drill reported `exit=` blank and PASS at the same time.
    $null = $p2.Handle
    $exited = $p2.WaitForExit(30000)
    if (-not $exited) { Stop-Process -Id $p2.Id -Force -EA SilentlyContinue }
    # The parameterless WaitForExit() after the timed one is NOT redundant: the
    # timed overload returns as soon as the process signals, but .ExitCode can
    # still be unpopulated on the Start-Process -PassThru object, and reading it
    # then yields empty. Probed - the drill first reported `exit=` with no value,
    # which would have made the "exit -ne 0" assertion pass on nothing at all.
    $exitCode = -1
    if ($exited) { $p2.WaitForExit(); $exitCode = $p2.ExitCode }

    # Read from the redirect files, never from $p2.StandardOutput: reading a
    # redirected stream after the process has exited is where the harness's
    # "Stream was not readable" faults come from.
    $text = ""
    foreach ($f in @($out2, $err2)) {
      if (Test-Path $f) { $text += ((Get-Content $f -Raw -EA SilentlyContinue) + "`n") }
    }
    $text = $text -replace '\x1b\[[0-9;]*m', ''

    # 'refusing to start', not 'already owns'. The override path WARNS and
    # continues, and that warning quotes the underlying error - which contains
    # 'already owns'. Matching on it therefore reported a refusal for a server
    # that had started perfectly happily. Only the refusal says "refusing to
    # start"; the warning cannot.
    $refusedForLock = ($text -match 'refusing to start') -and ($text -notmatch 'MEDIAGIT_ALLOW_MULTI_INSTANCE=1:')
    # Normalise separators on BOTH sides before comparing. The server writes
    # repos_dir into server.toml with forward slashes and echoes it back that
    # way, while Split-Path hands back backslashes - so an exact match is
    # guaranteed to fail and `names-dir` read False against a refusal that did
    # name the directory perfectly well. Probed.
    $reposDir = (Join-Path (Split-Path $srv.ConfigPath -Parent) "repos") -replace '\\', '/'
    $namesDir = (($text -replace '\\', '/') -match [regex]::Escape($reposDir))
    # A bind collision would also produce a non-zero exit. Reject that reading
    # explicitly rather than accept any failure as evidence of the lock.
    $notABindError = -not ($text -match 'address .*in use|10048')

    # ANTI-VACUOUS: a drill that only checks "the second one died" also passes
    # if BOTH died. The survivor is the whole point of the feature.
    $firstAlive = $false
    try {
      $resp = Invoke-WebRequest -Uri "$($srv.BaseUrl)/health" -UseBasicParsing -TimeoutSec 5 -EA Stop
      $firstAlive = ($resp.StatusCode -eq 200)
    } catch {}

    # `$exitCode -is [int]` first, and not as a formality: an unreadable exit
    # code must FAIL, not pass. `$null -ne 0` is True in PowerShell, so the
    # obvious spelling turns "we could not read the exit code" into evidence of
    # a clean refusal - a gate passing on a measurement it never made.
    $exitRead = ($exitCode -is [int])
    # `$exited` is load-bearing and separate from the exit code. A second
    # instance that STARTS and keeps running gets killed at the 30s timeout and
    # reports -1, which is "-ne 0" and read as a refusal. That is the precise
    # shape of the failure this drill must catch, so "it exited on its own" has
    # to be asserted, not inferred from a nonzero code.
    $pass = $exited -and $exitRead -and ($exitCode -ne 0) -and $refusedForLock -and $namesDir `
      -and $notABindError -and $firstAlive
    Rec $drill $pass ("exited-on-its-own=$exited exit=$exitCode exit-code-read=$exitRead " +
      "refused-for-lock=$refusedForLock names-dir=$namesDir " +
      "not-a-bind-error=$notABindError " +
      "first-server-still-serving=$firstAlive port2=$otherPort")

    # The escape hatch must actually let the operator through, or it is not an
    # escape hatch and the only way past a false positive is a code change.
    $prevAllow = $env:MEDIAGIT_ALLOW_MULTI_INSTANCE
    try {
      $env:MEDIAGIT_ALLOW_MULTI_INSTANCE = "1"
      $port3 = Get-QaFreePort
      $out3 = Join-Path $QA.Logs "a15-override.out.log"
      $err3 = Join-Path $QA.Logs "a15-override.err.log"
      $p3 = Start-Process -FilePath $QA.MGServer `
        -ArgumentList @("--config", $srv.ConfigPath, "--port", "$port3") `
        -PassThru -NoNewWindow -RedirectStandardOutput $out3 -RedirectStandardError $err3
      $up3 = $false
      for ($i = 0; $i -lt 20; $i++) {
        if ($p3.HasExited) { break }
        try {
          $r3 = Invoke-WebRequest -Uri "http://127.0.0.1:$port3/health" -UseBasicParsing -TimeoutSec 2 -EA Stop
          if ($r3.StatusCode -eq 200) { $up3 = $true; break }
        } catch {}
        Start-Sleep -Milliseconds 500
      }
      if (-not $p3.HasExited) { & taskkill /PID $p3.Id /T /F 2>$null | Out-Null }
      $warned = $false
      foreach ($f in @($out3, $err3)) {
        if (Test-Path $f) {
          $t3 = ((Get-Content $f -Raw -EA SilentlyContinue) -replace '\x1b\[[0-9;]*m', '')
          if ($t3 -match 'MEDIAGIT_ALLOW_MULTI_INSTANCE=1') { $warned = $true }
        }
      }
      # Starting is not enough: an override that starts SILENTLY is worse than
      # no override, because the operator gets no record of what they disabled.
      Rec "$drill-override" ($up3 -and $warned) "started=$up3 warned-loudly=$warned port3=$port3"
    } finally {
      if ($null -eq $prevAllow) {
        Remove-Item Env:MEDIAGIT_ALLOW_MULTI_INSTANCE -EA SilentlyContinue
      } else {
        $env:MEDIAGIT_ALLOW_MULTI_INSTANCE = $prevAllow
      }
    }
  } catch {
    if ("$_" -match "^SKIP:") { Rec $drill "SKIP" "$_" } else { Rec $drill $false "unexpected error: $_" }
  } finally {
    Stop-QaServer $srv
    # Put back whatever the campaign had before this drill cleared it, so a
    # later phase sees the environment it expects.
    if ($null -eq $prevAllowOuter) {
      Remove-Item Env:MEDIAGIT_ALLOW_MULTI_INSTANCE -EA SilentlyContinue
    } else {
      $env:MEDIAGIT_ALLOW_MULTI_INSTANCE = $prevAllowOuter
    }
  }
}

# ---------------------------------------------------------------------------
# A16: at-rest encryption (DC-7/D2+D3) lifecycle - status off, init, sealed
# objects, round-trip with the key, fail-closed without it, push refused
# (D4 escrow does not exist yet), re-init refused. Shipped today with hand
# verification only (crates/mediagit-cli/tests/cli_encryption_test.rs) and
# zero campaign coverage.
#
# MEDIAGIT_ENCRYPTION_KEYFILE is the only non-interactive master-key source
# (keychain and passphrase both prompt); cli_encryption_test.rs pins the same
# variable for the same reason - a keyfile is the one source this harness can
# drive without a TTY, and without it `key init` would write a real secret
# into this machine's OS keychain.
# ---------------------------------------------------------------------------
function Drill-A16-EncryptionLifecycle {
  $drill = "A16-encryption-lifecycle"
  $prevKeyfileEnv = $env:MEDIAGIT_ENCRYPTION_KEYFILE
  Remove-Item Env:MEDIAGIT_ENCRYPTION_KEYFILE -ErrorAction SilentlyContinue
  try {
    $repo = New-SandboxRepo "a16-encryption" $Phase

    # 1. an ordinary repo reports encryption off.
    $st1 = Invoke-MG $repo @("key", "status") $Phase
    $statusOffOk = ($st1.Exit -eq 0) -and ($st1.Out -match "At-rest encryption: off")

    $keyfile = Join-Path $QA.Work "a16-master.key"
    New-QaEncryptionKeyfile $keyfile
    $env:MEDIAGIT_ENCRYPTION_KEYFILE = $keyfile

    # 2. init succeeds and prints a one-time recovery code. Captured into a
    # variable only - never Write-QaLog'd, never put in a Rec detail string.
    # (Invoke-MG's own $Phase-cmds.log transcript still contains it, same as
    # it would for any command's stdout; that file is gitignored scratch, not
    # a report artifact - see the caveat in the drill-author's own notes.)
    $init1 = Invoke-MG $repo @("key", "init") $Phase
    $recoveryCode = $null
    if ($init1.Exit -eq 0) {
      $recoveryCode = ($init1.Out -split "`r?`n" | ForEach-Object { $_.Trim() } |
        Where-Object { $_.Length -eq 71 -and (($_.ToCharArray() | Where-Object { $_ -eq '-' }).Count -eq 7) } |
        Select-Object -First 1)
    }
    $initOk = ($init1.Exit -eq 0) -and ($null -ne $recoveryCode)

    # 3. objects written by `add` are sealed. Anti-vacuous: a run that examines
    # zero objects passes for the wrong reason (this exact mistake shipped once
    # already, per the task brief - an equivalent Rust test passed against an
    # empty listing).
    $asset = Join-Path $repo "asset.bin"
    New-QaBinaryFixture $asset 2 91601
    $origHash = Get-QaHash $asset
    Invoke-MG $repo @("add", "asset.bin") $Phase | Out-Null
    Invoke-MG $repo @("commit", "-m", "sealed commit") $Phase | Out-Null
    $seal = Get-QaEncryptionSealStats $repo
    $sealExercised = ($seal.Examined -gt 0)
    $sealOk = $sealExercised -and ($seal.Sealed -gt 0)

    # 4. round-trip WITH the key: delete the working file, reset --hard, hash matches.
    Remove-Item $asset -Force
    $reset1 = Invoke-MG $repo @("reset", "--hard", "HEAD") $Phase
    $roundTripOk = ($reset1.Exit -eq 0) -and (Test-Path $asset) -and ((Get-QaHash $asset) -eq $origHash)

    # 5. read WITHOUT the key must fail - both halves gated separately. A
    # nonzero exit alone would also be "true" for a run that failed AFTER
    # quietly writing the plaintext back out; the file must simply not be there.
    Remove-Item $asset -Force -ErrorAction SilentlyContinue
    Remove-Item Env:MEDIAGIT_ENCRYPTION_KEYFILE -ErrorAction SilentlyContinue
    $reset2 = Invoke-MG $repo @("reset", "--hard", "HEAD") $Phase
    $env:MEDIAGIT_ENCRYPTION_KEYFILE = $keyfile
    $nokeyFailed = ($reset2.Exit -ne 0)
    $nokeyNotRestored = -not (Test-Path $asset)

    # 6. push escrows the key BEFORE it uploads anything, and stops there if it
    # cannot (DC-7/D4). The remote is a closed port, which is the bluntest
    # version of "escrow did not happen": push must fail while still talking
    # about the encryption key, not after moving objects. If the escrow step
    # were ever reordered behind the upload, this reads as an ordinary
    # connection failure with no mention of a key, and fails.
    Invoke-MG $repo @("remote", "add", "origin", "http://127.0.0.1:59999/a16") $Phase | Out-Null
    $push = Invoke-MG $repo @("push") $Phase
    $pushRefused = ($push.Exit -ne 0) -and ($push.Out -match "(?i)encryption-key")

    # 7. a second `key init` must be refused - overwriting the key would orphan
    # every object already sealed under it.
    $init2 = Invoke-MG $repo @("key", "init") $Phase
    $reinitRefused = ($init2.Exit -ne 0) -and ($init2.Out -match "(?i)already has an encryption key")

    $pass = $statusOffOk -and $initOk -and $sealOk -and $roundTripOk -and
      $nokeyFailed -and $nokeyNotRestored -and $pushRefused -and $reinitRefused
    Rec $drill $pass ("status-off=$statusOffOk init-ok=$initOk recovery-code-captured=$($null -ne $recoveryCode) " +
      "objects-examined=$($seal.Examined) sealed=$($seal.Sealed) seal-exercised=$sealExercised " +
      "roundtrip-with-key=$roundTripOk nokey-reset-exit=$($reset2.Exit) nokey-failed=$nokeyFailed " +
      "nokey-file-not-restored=$nokeyNotRestored push-stopped-at-escrow=$pushRefused " +
      "reinit-refused=$reinitRefused")
  } catch {
    if ("$_" -match "^SKIP:") { Rec $drill "SKIP" "$_" } else { Rec $drill $false "unexpected error: $_" }
  } finally {
    if ($null -eq $prevKeyfileEnv) { Remove-Item Env:MEDIAGIT_ENCRYPTION_KEYFILE -ErrorAction SilentlyContinue }
    else { $env:MEDIAGIT_ENCRYPTION_KEYFILE = $prevKeyfileEnv }
  }
}

# ---------------------------------------------------------------------------
# A17: encryption recovery (DC-7 recovery slot). The master keyfile is the only
# copy of the master key this harness holds - destroying it and confirming
# reads fail is the "lost my USB stick" scenario the recovery code exists for.
# `key recover` unlocks under a brand new keyfile and re-wraps there; a wrong
# code must be refused and must not touch the key file at all.
# ---------------------------------------------------------------------------
function Drill-A17-EncryptionRecovery {
  $drill = "A17-encryption-recovery"
  $prevKeyfileEnv = $env:MEDIAGIT_ENCRYPTION_KEYFILE
  Remove-Item Env:MEDIAGIT_ENCRYPTION_KEYFILE -ErrorAction SilentlyContinue
  try {
    $repo = New-SandboxRepo "a17-recovery" $Phase
    $keyfile1 = Join-Path $QA.Work "a17-master1.key"
    New-QaEncryptionKeyfile $keyfile1
    $env:MEDIAGIT_ENCRYPTION_KEYFILE = $keyfile1

    $init = Invoke-MG $repo @("key", "init") $Phase
    $recoveryCode = $null
    if ($init.Exit -eq 0) {
      $recoveryCode = ($init.Out -split "`r?`n" | ForEach-Object { $_.Trim() } |
        Where-Object { $_.Length -eq 71 -and (($_.ToCharArray() | Where-Object { $_ -eq '-' }).Count -eq 7) } |
        Select-Object -First 1)
    }
    if (-not $recoveryCode) { Rec $drill $false "key init did not produce a usable recovery code (exit=$($init.Exit))"; return }

    $asset = Join-Path $repo "asset.bin"
    New-QaBinaryFixture $asset 2 91701
    $origHash = Get-QaHash $asset
    Invoke-MG $repo @("add", "asset.bin") $Phase | Out-Null
    Invoke-MG $repo @("commit", "-m", "before recovery") $Phase | Out-Null

    # 8a. destroy the master keyfile; reads must now fail.
    Remove-Item $keyfile1 -Force
    $logNoKey = Invoke-MG $repo @("log") $Phase
    $readsFailAfterDestroy = ($logNoKey.Exit -ne 0)

    # 8b. recover against a brand new keyfile.
    $keyfile2 = Join-Path $QA.Work "a17-master2.key"
    New-QaEncryptionKeyfile $keyfile2
    $env:MEDIAGIT_ENCRYPTION_KEYFILE = $keyfile2
    $recover = Invoke-MG $repo @("key", "recover", $recoveryCode) $Phase
    $recoverOk = ($recover.Exit -eq 0) -and ($recover.Out -match "(?i)Repository unlocked")

    # confirm the original data reads back byte-identical under the new master key.
    $logAfter = Invoke-MG $repo @("log") $Phase
    $logShowsCommit = ($logAfter.Exit -eq 0) -and ($logAfter.Out -match "before recovery")
    Remove-Item $asset -Force
    $resetAfter = Invoke-MG $repo @("reset", "--hard", "HEAD") $Phase
    $recoveredHashOk = ($resetAfter.Exit -eq 0) -and (Test-Path $asset) -and ((Get-QaHash $asset) -eq $origHash)

    # 9. a WRONG recovery code must be refused and must change nothing on disk.
    $keyFilePath = Join-Path $repo ".mediagit\encryption-key"
    $beforeWrongBytes = Get-QaHash $keyFilePath
    $wrongCode = $recoveryCode.Substring(0, $recoveryCode.Length - 1) +
      $(if ($recoveryCode.Substring($recoveryCode.Length - 1) -eq "0") { "1" } else { "0" })
    $wrongAttempt = Invoke-MG $repo @("key", "recover", $wrongCode) $Phase
    $wrongRefused = ($wrongAttempt.Exit -ne 0)
    $wrongChangedNothing = ((Get-QaHash $keyFilePath) -eq $beforeWrongBytes)

    $pass = $readsFailAfterDestroy -and $recoverOk -and $logShowsCommit -and $recoveredHashOk -and
      $wrongRefused -and $wrongChangedNothing
    Rec $drill $pass ("reads-fail-after-keyfile-destroyed=$readsFailAfterDestroy recover-exit=$($recover.Exit) " +
      "recover-ok=$recoverOk log-shows-commit=$logShowsCommit recovered-hash-ok=$recoveredHashOk " +
      "wrong-code-refused=$wrongRefused (exit=$($wrongAttempt.Exit)) wrong-code-changed-nothing=$wrongChangedNothing")
  } catch {
    if ("$_" -match "^SKIP:") { Rec $drill "SKIP" "$_" } else { Rec $drill $false "unexpected error: $_" }
  } finally {
    if ($null -eq $prevKeyfileEnv) { Remove-Item Env:MEDIAGIT_ENCRYPTION_KEYFILE -ErrorAction SilentlyContinue }
    else { $env:MEDIAGIT_ENCRYPTION_KEYFILE = $prevKeyfileEnv }
  }
}

if (_Want "A12") { Drill-A12-DeltaChainCycle }
if (_Want "A13") { Drill-A13-PerChunkFallbackNoRateLimit }
if (_Want "A14") { Drill-A14-UnboundPresignedPut }
if (_Want "A15") { Drill-A15-SecondInstanceRefused }
if (_Want "A16") { Drill-A16-EncryptionLifecycle }
if (_Want "A17") { Drill-A17-EncryptionRecovery }

Write-QaLog $Phase "=== 07_abuse done: overall=$(if ($script:AllPass) { 'PASS' } else { 'FAIL' }) ==="
# Teardown: reclaim this phase's own work/ scratch so a long campaign cannot run the
# volume out of space. work/ ONLY - logs/ and fixtures-synthetic/ are never touched.
Invoke-QaTeardown $Phase @("a[0-9]*")

Exit-QaPhase $Phase (-not $script:AllPass)
