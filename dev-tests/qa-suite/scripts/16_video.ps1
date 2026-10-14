# Phase 16: video frame-chunker gates (dedup chain, round-trip, >16 GiB, RSS). ASCII-only, PS 5.1.
#
# Standalone by design - NOT in run_all's default phase list, for the same reason
# as 11_memprofile / 15_packmem: peak-RSS sampling reads `Get-Process mediagit`
# process-wide, so a concurrent campaign pollutes every number, and the >16 GiB
# stage writes ~20 GB of scratch. It is in run_ga.ps1's explicit list. Run alone:
#
#   powershell -NoProfile -File .\16_video.ps1
#
# WHAT IT GUARDS. The streaming frame-aligned MOV/MP4 chunker. Without it an
# edited NLE export (insert / trim) shares almost nothing with its predecessor:
# the old CDC path grew the store 53.9 MB / 4.4 MB on the U2 / U1 edits where
# the frame chunker grows 1.4 MB / 0 MB. And the whole add path must stay
# streaming: a >16 GiB file was the memory-cliff case.
#
# GATES (rows in gates.tsv; True / False / SKIP):
#   video_dedup_chain  U0 -> U2_insert -> U1_trim committed as one file; store growth per commit <= 3 MB
#   video_roundtrip    every committed version restored (delete + reset --hard) is SHA256-identical
#   video_gt16gib      >16 GiB file: add, commit, status clean, fsck PERFECT, show HEAD:f hash,
#                      merge from a branch hash, packed (gc --repack) restore hash
#   video_rss          peak private <= 512 MB AND peak working set <= 1024 MB on every big-file op
# A missing fixture / ffmpeg / disk makes the dependent gate SKIP with the reason, never PASS.
#
# KNOBS (all optional):
#   MG_QA_VIDEO_RSS_MB       private-bytes ceiling in MB, default 512 (video_rss)
#   MG_QA_VIDEO_WS_MB        working-set ceiling in MB, default 1024 (video_rss)
#   MG_QA_VIDEO_GROWTH_MB    per-commit store growth ceiling, default 3 (video_dedup_chain)
#   MG_QA_VIDEO_FFMPEG       path to ffmpeg.exe; default = the winget Gyan.FFmpeg install.
#                            A path that does not exist means SKIP (no silent fallback).
#   MG_QA_VIDEO_BIG_COPIES   U0 copies concatenated for the big file, default 40 (~17.9 GiB)
#   MG_QA_VIDEO_BIG_MIN_GIB  size the big file must exceed, default 16
#   MG_QA_VIDEO_FREE_GB      free disk required before the big stage, default 25
#   MG_QA_VIDEO_INJECT_CORRUPT=1  RED-PROOF HOOK, test-only: flip one byte in the largest stored object
#                            after each commit stage, so roundtrip / fsck gates must go red.
#   MEDIAGIT_VIDEO_CHUNKER=cdc    product knob, forces the old chunker: video_dedup_chain must go red.
# The environment pins MEDIAGIT_CDC_SEED so chunk boundaries are reproducible.

. (Join-Path $PSScriptRoot "lib\common.ps1")

$Phase = "16_video"
$rssCeilMB = [double](_Env "MG_QA_VIDEO_RSS_MB" "512")
$wsCeilMB = [double](_Env "MG_QA_VIDEO_WS_MB" "1024")
$growthCeilMB = [double](_Env "MG_QA_VIDEO_GROWTH_MB" "3")
$bigCopies = [int](_Env "MG_QA_VIDEO_BIG_COPIES" "40")
$bigMinGiB = [double](_Env "MG_QA_VIDEO_BIG_MIN_GIB" "16")
$freeNeedGB = [double](_Env "MG_QA_VIDEO_FREE_GB" "25")
$inject = (_Env "MG_QA_VIDEO_INJECT_CORRUPT" "0") -eq "1"

if (-not $env:MEDIAGIT_CDC_SEED) { $env:MEDIAGIT_CDC_SEED = "20260716" }

$fixDir = Join-Path $QA.TestFiles "video-implementation\NLE-pairs"
$fixU0 = Join-Path $fixDir "U0_v1.mov"
$fixU2 = Join-Path $fixDir "U2_insert.mov"
$fixU1 = Join-Path $fixDir "U1_trim.mov"
$haveFix = (Test-Path $fixU0) -and (Test-Path $fixU1) -and (Test-Path $fixU2)

$chainRepo = Join-Path $QA.Work "video-chain"
$bigRepo = Join-Path $QA.Work "video-big"
$showTmp = Join-Path $QA.Work "video-show.tmp"

function Get-OdbMB([string]$Repo) {
  $s = (Get-ChildItem (Join-Path $Repo ".mediagit") -Recurse -File -Force -EA SilentlyContinue | Measure-Object Length -Sum).Sum
  return [math]::Round(([double]$s) / 1MB, 2)
}

function Set-HardLink([string]$Link, [string]$Target) {
  if (Test-Path $Link) { Remove-Item $Link -Force }
  New-Item -ItemType HardLink -Path $Link -Target $Target | Out-Null
}

# Red-proof hook: flip one byte in the middle of the largest stored object.
function Invoke-InjectCorruption([string]$Repo) {
  if (-not $inject) { return }
  $f = Get-ChildItem (Join-Path $Repo ".mediagit\objects") -Recurse -File -Force |
    Sort-Object Length -Descending | Select-Object -First 1
  $fs = [System.IO.File]::Open($f.FullName, [System.IO.FileMode]::Open, [System.IO.FileAccess]::ReadWrite)
  try {
    $fs.Seek([long]($f.Length / 2), [System.IO.SeekOrigin]::Begin) | Out-Null
    $b = $fs.ReadByte()
    $fs.Seek(-1, [System.IO.SeekOrigin]::Current) | Out-Null
    $fs.WriteByte([byte](($b + 1) % 256))
  }
  finally { $fs.Close() }
  Write-QaLog $Phase ("INJECT corrupted 1 byte of {0} ({1} bytes)" -f $f.FullName, $f.Length)
}

function Test-FsckPerfect($R) { ($R.Exit -eq 0) -and ($R.Out -match "integrity:\s*PERFECT") }

# Invoke-MG buffers stdout as text; a blob must go to disk byte-exact.
function Invoke-MGToFile([string]$Repo, [string[]]$MgArgs, [string]$OutFile) {
  $argLine = (@("-C", $Repo) + $MgArgs | ForEach-Object { '"' + $_ + '"' }) -join " "
  $p = Start-Process -FilePath $QA.MG -ArgumentList $argLine -NoNewWindow -Wait -PassThru `
    -RedirectStandardOutput $OutFile -RedirectStandardError ($OutFile + ".err")
  return $p.ExitCode
}

function Find-Ffmpeg {
  $o = $env:MG_QA_VIDEO_FFMPEG
  if ($o) { if (Test-Path $o) { return $o } else { return $null } }
  $root = Join-Path $env:LOCALAPPDATA "Microsoft\WinGet\Packages"
  if (-not (Test-Path $root)) { return $null }
  $f = Get-ChildItem $root -Recurse -Filter ffmpeg.exe -File -EA SilentlyContinue | Select-Object -First 1
  if ($f) { return $f.FullName } else { return $null }
}

$rss = [ordered]@{}   # op -> @{ Priv; Ws }
function Measure-VideoOp([string]$Name, [scriptblock]$Action) {
  $m = Measure-PeakRSS -Phase $Phase -Label $Name -Action $Action
  # A command that failed (or was never sampled) measured nothing: the gate must not pass on it.
  $ex = if ($m.Result -is [int]) { $m.Result } else { $m.Result.Exit }
  $script:rss[$Name] = @{ Priv = [double]$m.ClientPrivatePeakMB; Ws = [double]$m.ClientPeakMB; Exit = $ex }
  Write-QaLog $Phase ("{0,-14} exit={3} peakWS={1}MB peakPriv={2}MB" -f $Name, $m.ClientPeakMB, $m.ClientPrivatePeakMB, $ex)
  return $m.Result
}

function Get-HeadOid([string]$Repo) {
  return ((Invoke-MG $Repo @("log", "--oneline", "-n", "1") $Phase).Out.Trim() -split " ")[0]
}

$g = @{}   # gate name -> @{ Pass; Detail }
try {
  foreach ($d in @($chainRepo, $bigRepo)) { if (Test-Path $d) { Remove-Item -Recurse -Force $d -EA SilentlyContinue } }
  Remove-Item $showTmp, ($showTmp + ".err") -Force -EA SilentlyContinue
  Write-QaLog $Phase ("=== 16_video start: rssCeil={0}MB wsCeil={1}MB growthCeil={2}MB bigCopies={3} minGiB={4} inject={5} chunker={6} ===" -f `
      $rssCeilMB, $wsCeilMB, $growthCeilMB, $bigCopies, $bigMinGiB, $inject, $(if ($env:MEDIAGIT_VIDEO_CHUNKER) { $env:MEDIAGIT_VIDEO_CHUNKER } else { "default" }))

  # ---------------- stage 1: dedup chain + round-trip ----------------
  if (-not $haveFix) {
    $why = "SKIP: NLE-pairs fixtures missing under $fixDir"
    $g["video_dedup_chain"] = @{ Pass = "SKIP"; Detail = $why }
    $g["video_roundtrip"] = @{ Pass = "SKIP"; Detail = $why }
  }
  else {
    $repo = New-SandboxRepo "video-chain" $Phase
    $edit = Join-Path $repo "edit.mov"
    $steps = @(
      @{ Name = "U0"; Src = $fixU0 },
      @{ Name = "U2"; Src = $fixU2 },
      @{ Name = "U1"; Src = $fixU1 }
    )
    $growth = @{}; $oids = @{}; $hashes = @{}; $allOk = $true
    foreach ($s in $steps) {
      Set-HardLink $edit $s.Src
      $hashes[$s.Name] = Get-QaHash $s.Src
      $before = Get-OdbMB $repo
      $a = Invoke-MG $repo @("add", "edit.mov") $Phase -TimeoutSec 3600
      $c = Invoke-MG $repo @("commit", "-m", ("video " + $s.Name)) $Phase -TimeoutSec 3600
      if ($a.Exit -ne 0 -or $c.Exit -ne 0) { $allOk = $false; Write-QaLog $Phase ("{0} add={1} commit={2}" -f $s.Name, $a.Exit, $c.Exit) }
      $growth[$s.Name] = [math]::Round((Get-OdbMB $repo) - $before, 2)
      $oids[$s.Name] = Get-HeadOid $repo
      Write-QaLog $Phase ("chain {0}: store growth {1} MB oid={2}" -f $s.Name, $growth[$s.Name], $oids[$s.Name])
    }
    $dedupOk = $allOk -and ($growth["U2"] -le $growthCeilMB) -and ($growth["U1"] -le $growthCeilMB)
    $g["video_dedup_chain"] = @{ Pass = $dedupOk; Detail = ("growth MB U0={0} U2={1} U1={2}; ceiling {3} on U2 and U1 (frame chunker 1.4/0; old CDC 53.9/4.4); commits ok={4}" -f `
          $growth["U0"], $growth["U2"], $growth["U1"], $growthCeilMB, $allOk) }

    Invoke-InjectCorruption $repo
    $bad = @()
    foreach ($n in @("U1", "U2", "U0")) {
      if (Test-Path $edit) { Remove-Item $edit -Force }
      $r = Invoke-MG $repo @("reset", "--hard", $oids[$n]) $Phase -TimeoutSec 3600
      $got = if (Test-Path $edit) { Get-QaHash $edit } else { "<missing>" }
      if ($got -ne $hashes[$n]) { $bad += ("{0}(exit={1})" -f $n, $r.Exit) }
    }
    $fsckR = Invoke-MG $repo @("fsck") $Phase -TimeoutSec 3600
    $g["video_roundtrip"] = @{ Pass = ($bad.Count -eq 0); Detail = $(if ($bad.Count) { "hash mismatch: " + ($bad -join " ") } else { "U0,U2,U1 restored byte-identical (SHA256); chain fsck perfect=" + (Test-FsckPerfect $fsckR) }) }
  }

  # ---------------- stage 2: >16 GiB ----------------
  $ffmpeg = if ($haveFix) { Find-Ffmpeg } else { $null }
  $freeGB = Get-QaFreeDiskGB $QA.Work
  $bigSkip = $null
  if (-not $haveFix) { $bigSkip = "SKIP: NLE-pairs fixtures missing" }
  elseif (-not $ffmpeg) { $bigSkip = "SKIP: ffmpeg.exe not found (MG_QA_VIDEO_FFMPEG='$($env:MG_QA_VIDEO_FFMPEG)')" }
  elseif ($freeGB -lt $freeNeedGB) { $bigSkip = "SKIP: free disk ${freeGB}GB < ${freeNeedGB}GB needed" }

  if ($bigSkip) {
    $g["video_gt16gib"] = @{ Pass = "SKIP"; Detail = $bigSkip }
    $g["video_rss"] = @{ Pass = "SKIP"; Detail = $bigSkip }
  }
  else {
    $repo = New-SandboxRepo "video-big" $Phase
    $big = Join-Path $repo "big.mov"
    $c = [ordered]@{}
    Set-Content (Join-Path $repo "base.txt") "base" -Encoding Ascii
    Invoke-MG $repo @("add", "base.txt") $Phase | Out-Null
    Invoke-MG $repo @("commit", "-m", "base") $Phase | Out-Null
    Invoke-MG $repo @("branch", "create", "big") $Phase | Out-Null
    Invoke-MG $repo @("branch", "switch", "big") $Phase | Out-Null

    # ffmpeg stream-copy concat of U0 x N. Forward slashes: the concat demuxer treats a backslash as an escape.
    $list = Join-Path $QA.Work "video-concat.txt"
    $line = "file '" + ($fixU0 -replace '\\', '/') + "'"
    (1..$bigCopies | ForEach-Object { $line }) | Set-Content $list -Encoding Ascii
    $ffArgs = '-v error -y -f concat -safe 0 -i "{0}" -map 0:v -map 0:a -c copy "{1}"' -f $list, $big
    $ff = Start-Process -FilePath $ffmpeg -ArgumentList $ffArgs -NoNewWindow -Wait -PassThru `
      -RedirectStandardError (Join-Path $QA.Logs "16_video-ffmpeg.err")
    Remove-Item $list -Force -EA SilentlyContinue
    $sizeGiB = if (Test-Path $big) { [math]::Round((Get-Item $big).Length / 1GB, 2) } else { 0 }
    $c["build"] = ($ff.ExitCode -eq 0) -and ($sizeGiB -gt $bigMinGiB)
    Write-QaLog $Phase ("ffmpeg exit={0} size={1}GiB (need > {2})" -f $ff.ExitCode, $sizeGiB, $bigMinGiB)

    if ($c["build"]) {
      $srcHash = Get-QaHash $big
      $a = Measure-VideoOp "big-add" { Invoke-MG $repo @("add", "big.mov") $Phase -TimeoutSec 7200 }
      $ci = Invoke-MG $repo @("commit", "-m", "big") $Phase -TimeoutSec 7200
      $c["add+commit"] = ($a.Exit -eq 0) -and ($ci.Exit -eq 0)
      Invoke-InjectCorruption $repo

      $st = Invoke-MG $repo @("status", "--porcelain") $Phase -TimeoutSec 3600
      $c["status"] = ($st.Exit -eq 0) -and ($st.Out -notmatch "big\.mov")
      $fs = Measure-VideoOp "big-fsck" { Invoke-MG $repo @("fsck") $Phase -TimeoutSec 7200 }
      $c["fsck"] = Test-FsckPerfect $fs

      # show HEAD:big.mov to a temp file. Delete the working copy first: source + temp must not both fit.
      Remove-Item $big -Force
      $showExit = Measure-VideoOp "big-show" { Invoke-MGToFile $repo @("show", "HEAD:big.mov") $showTmp }
      $c["show"] = ($showExit -eq 0) -and (Test-Path $showTmp) -and ((Get-QaHash $showTmp) -eq $srcHash)
      Remove-Item $showTmp, ($showTmp + ".err") -Force -EA SilentlyContinue

      # merge: main has no big.mov; fast-forward merge re-creates it.
      $sw = Invoke-MG $repo @("branch", "switch", "main") $Phase -TimeoutSec 3600
      if (Test-Path $big) { Remove-Item $big -Force }
      $mg = Measure-VideoOp "big-merge" { Invoke-MG $repo @("merge", "big") $Phase -TimeoutSec 7200 }
      $c["merge"] = ($sw.Exit -eq 0) -and ($mg.Exit -eq 0) -and (Test-Path $big) -and ((Get-QaHash $big) -eq $srcHash)

      # packed restore
      $gc = Measure-VideoOp "big-repack" { Invoke-MG $repo @("gc", "--repack", "-y") $Phase -TimeoutSec 7200 }
      if (Test-Path $big) { Remove-Item $big -Force }
      $rs = Measure-VideoOp "big-packed-restore" { Invoke-MG $repo @("reset", "--hard", "HEAD") $Phase -TimeoutSec 7200 }
      $c["packed-restore"] = ($gc.Exit -eq 0) -and ($rs.Exit -eq 0) -and (Test-Path $big) -and ((Get-QaHash $big) -eq $srcHash)
    }

    $bad = @($c.Keys | Where-Object { -not $c[$_] })
    $g["video_gt16gib"] = @{ Pass = ($bad.Count -eq 0 -and $c.Count -gt 1); Detail = ("size={0}GiB; {1}" -f $sizeGiB, (($c.Keys | ForEach-Object { "{0}={1}" -f $_, $c[$_] }) -join " ")) }

    # video_rss: gated ops are every big-file op; repack is reported but not gated.
    if (-not $c["build"]) {
      $g["video_rss"] = @{ Pass = "SKIP"; Detail = "SKIP: big file was not built, nothing measured" }
    }
    else {
      $over = @(); $unmeasured = @()
      foreach ($k in $rss.Keys) {
        if ($k -eq "big-repack") { continue }
        if ($rss[$k].Exit -ne 0 -or $rss[$k].Ws -le 0) { $unmeasured += ("{0}(exit={1} ws={2})" -f $k, $rss[$k].Exit, $rss[$k].Ws); continue }
        if ($rss[$k].Priv -gt $rssCeilMB -or $rss[$k].Ws -gt $wsCeilMB) { $over += ("{0}(priv={1} ws={2})" -f $k, $rss[$k].Priv, $rss[$k].Ws) }
      }
      $all = ($rss.Keys | ForEach-Object { "{0}: ws={1} priv={2}" -f $_, $rss[$_].Ws, $rss[$_].Priv }) -join "; "
      $g["video_rss"] = @{ Pass = ($over.Count -eq 0 -and $unmeasured.Count -eq 0 -and $rss.Count -ge 4); Detail = ("ceil priv<={0}MB ws<={1}MB; {2}{3}{4}" -f $rssCeilMB, $wsCeilMB, $all, $(if ($over.Count) { "; OVER: " + ($over -join " ") } else { "" }), $(if ($unmeasured.Count) { "; UNMEASURED (failed or unsampled): " + ($unmeasured -join " ") } else { "" })) }
    }
  }
}
catch {
  Write-QaLog $Phase "ERROR: $_"
  foreach ($n in @("video_dedup_chain", "video_roundtrip", "video_gt16gib", "video_rss")) {
    if (-not $g.Contains($n)) { $g[$n] = @{ Pass = "ERROR"; Detail = "phase threw: $_" } }
  }
}
finally {
  Remove-Item $showTmp, ($showTmp + ".err") -Force -EA SilentlyContinue
  Invoke-QaTeardown $Phase @("video-chain", "video-big")
  foreach ($d in @($chainRepo, $bigRepo)) { if (Test-Path $d) { Remove-Item -Recurse -Force $d -EA SilentlyContinue } }
}

foreach ($n in @("video_dedup_chain", "video_roundtrip", "video_gt16gib", "video_rss")) {
  if ($g.Contains($n)) { Write-QaGate $Phase $n $g[$n].Pass $g[$n].Detail }
  else { Write-QaGate $Phase $n "ERROR" "gate was never evaluated" }
}

Exit-QaPhase $Phase
