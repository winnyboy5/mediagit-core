# Persona: Game developer (asset import, rebase, cherry-pick, binary conflicts, remote, recovery).
# Scenarios G1-G6. ASCII-only, PS 5.1 compatible.
param([string]$Only)

. (Join-Path $PSScriptRoot "lib\common.ps1")

$Phase  = "persona_gamedev"
$Tsv    = Join-Path $QA.Reports "scenario_gamedev.tsv"
$Header = @("scenario", "step", "action", "expect", "exit", "pass", "sec", "detail")
$script:AllPass = $true

# --- shared helpers (duplicated per-script by design; see qa-suite contract) ---

function Add-Row([string]$Scenario, [string]$StepName, [string]$Action, [string]$Expect, $Exit, [string]$Status, $Sec, [string]$Detail = "") {
  Write-QaRow $Tsv $Header @($Scenario, $StepName, $Action, $Expect, $Exit, $Status, $Sec, $Detail)
  if ($Status -eq "FAIL") { $script:AllPass = $false }
  Write-QaLog $Phase ("{0}/{1} {2} -> {3} (exit={4} sec={5})" -f $Scenario, $StepName, $Action, $Status, $Exit, $Sec)
}

function Test-FsckGate([string]$Scenario, [string]$Repo) {
  $r = Invoke-MG $Repo @("fsck") $Phase
  $status = if ($r.Exit -eq 0) { "PASS" } else { "FAIL" }
  $detail = $r.Out.Substring(0, [Math]::Min(200, $r.Out.Length))
  Add-Row $Scenario "fsck" "fsck" "exit 0" $r.Exit $status $r.Sec $detail
  Write-QaGate $Phase "$Scenario-fsck" ($status -eq "PASS") $detail
}

function Assert-HashEq([string]$Scenario, [string]$StepName, [string]$Expected, [string]$Actual, [string]$Detail = "") {
  $status = if ($Actual -and $Expected -and ($Actual -eq $Expected)) { "PASS" } else { "FAIL" }
  $exp8 = if ($Expected) { $Expected.Substring(0, [Math]::Min(12, $Expected.Length)) } else { "null" }
  $act8 = if ($Actual) { $Actual.Substring(0, [Math]::Min(12, $Actual.Length)) } else { "null" }
  Add-Row $Scenario $StepName "hash-parity" "match" 0 $status 0 "$Detail expected=$exp8 actual=$act8"
}

function Edit-BytesInPlace([string]$Path, [int]$Seed, [int]$SliceLen = 4096) {
  $bytes = [IO.File]::ReadAllBytes($Path)
  if ($bytes.Length -eq 0) { return }
  $rnd = New-Object Random($Seed)
  $len = [Math]::Min($SliceLen, $bytes.Length)
  $start = if ($bytes.Length -gt $len) { [int](($bytes.Length - $len) / 2) } else { 0 }
  for ($i = 0; $i -lt $len; $i++) { $bytes[$start + $i] = [byte]$rnd.Next(0, 256) }
  [IO.File]::WriteAllBytes($Path, $bytes)
}

function Get-CommitHash([string]$Out) {
  if ($Out -match "Created commit ([0-9a-fA-F]{8,})") { return $Matches[1] }
  return $null
}

function Invoke-Scenario([string]$Id, [scriptblock]$Body) {
  try { & $Body }
  catch { Add-Row $Id "exception" "run" "no exception" -1 "FAIL" 0 ($_.Exception.Message) }
}

# Smallest matching file >= 1MB (STANDARD preference), fallback to largest; $null if none.
function Select-RealFixture([object[]]$Candidates, [int]$MinMB = 1) {
  if (-not $Candidates) { return $null }
  $items = $Candidates | Where-Object { $_ } | ForEach-Object { if ($_ -is [IO.FileInfo]) { $_ } else { Get-Item -LiteralPath $_ -EA SilentlyContinue } } | Where-Object { $_ }
  if (-not $items) { return $null }
  $over = $items | Where-Object { $_.Length -ge ($MinMB * 1MB) } | Sort-Object Length | Select-Object -First 1
  if ($over) { return $over.FullName }
  return ($items | Sort-Object Length -Descending | Select-Object -First 1).FullName
}

# Binary-conflict round trip used by G4 for .blend and .glb; the v11 P0 class.
function Test-BinaryConflict([string]$Id, [string]$Fixture, [string]$FileName, [int]$SeedBase) {
  $repo = New-SandboxRepo "gamedev-$Id-$([IO.Path]::GetFileNameWithoutExtension($FileName))" $Phase
  $dest = Join-Path $repo $FileName
  Copy-Item -LiteralPath $Fixture -Destination $dest -Force
  Invoke-MG $repo @("add", $dest) $Phase | Out-Null
  Invoke-MG $repo @("commit", "-m", "base") $Phase | Out-Null
  $baseHash = Get-QaHash $dest

  Invoke-MG $repo @("branch", "create", "side-a") $Phase | Out-Null
  Invoke-MG $repo @("branch", "create", "side-b") $Phase | Out-Null

  Invoke-MG $repo @("branch", "switch", "side-a") $Phase | Out-Null
  Edit-BytesInPlace $dest ($SeedBase + 1)
  Invoke-MG $repo @("add", $dest) $Phase | Out-Null
  Invoke-MG $repo @("commit", "-m", "side-a edit") $Phase | Out-Null
  $hashA = Get-QaHash $dest
  $copyA = Join-Path $QA.Work "gamedev-$Id-$FileName.side-a"
  Copy-Item -LiteralPath $dest -Destination $copyA -Force

  Invoke-MG $repo @("branch", "switch", "side-b") $Phase | Out-Null
  Edit-BytesInPlace $dest ($SeedBase + 2)
  Invoke-MG $repo @("add", $dest) $Phase | Out-Null
  Invoke-MG $repo @("commit", "-m", "side-b edit") $Phase | Out-Null
  $hashB = Get-QaHash $dest

  Invoke-MG $repo @("branch", "switch", "side-a") $Phase | Out-Null
  $mg = Invoke-MG $repo @("merge", "side-b") $Phase
  Add-Row $Id "$FileName-merge" "merge side-b into side-a" "exit 1 (binary conflict)" $mg.Exit $(if ($mg.Exit -ne 0) { "PASS" } else { "FAIL" }) $mg.Sec $mg.Out.Substring(0, [Math]::Min(150, $mg.Out.Length))

  # DURING conflict: working tree must hold ONE intact side, not marker soup.
  $duringHash = Get-QaHash $dest
  $intact = ($duringHash -eq $hashA) -or ($duringHash -eq $hashB)
  Add-Row $Id "$FileName-during" "hash during conflict" "matches side-a or side-b bytes" 0 $(if ($intact) { "PASS" } else { "FAIL" }) 0 "during=$($duringHash.Substring(0,12)) a=$($hashA.Substring(0,12)) b=$($hashB.Substring(0,12))"

  # resolve: choose side-a's version
  Copy-Item -LiteralPath $copyA -Destination $dest -Force
  Invoke-MG $repo @("add", $dest) $Phase | Out-Null
  $mc = Invoke-MG $repo @("merge", "--continue") $Phase
  Add-Row $Id "$FileName-resolve" "merge --continue-merge side-b" "exit 0" $mc.Exit $(if ($mc.Exit -eq 0) { "PASS" } else { "FAIL" }) $mc.Sec ""
  Assert-HashEq $Id "$FileName-after-resolve" $hashA (Get-QaHash $dest) "resolved file"

  # AFTER: both sides' bytes still recoverable via branch switch
  Invoke-MG $repo @("branch", "switch", "side-b") $Phase | Out-Null
  Assert-HashEq $Id "$FileName-b-recoverable" $hashB (Get-QaHash $dest) "switch side-b"
  Invoke-MG $repo @("branch", "switch", "main") $Phase | Out-Null
  Assert-HashEq $Id "$FileName-base-recoverable" $baseHash (Get-QaHash $dest) "switch main (base)"
  Invoke-MG $repo @("branch", "switch", "side-a") $Phase | Out-Null
  Assert-HashEq $Id "$FileName-a-recoverable" $hashA (Get-QaHash $dest) "switch side-a (post-merge tip carries side-a bytes)"

  Test-FsckGate $Id $repo
}

# ---------------------------------------------------------------------------
# G1: import a pioneer-master subtree, add -A, commit, status clean, stats.
# ---------------------------------------------------------------------------
function Run-G1 {
  $id = "G1"
  $srcRoot = Join-Path $QA.TestFiles "pioneer-master\pioneer-master\data\models"
  if (-not (Test-Path $srcRoot)) { $srcRoot = Join-Path $QA.TestFiles "pioneer-master" }
  if (-not (Test-Path $srcRoot)) { Add-Row $id "fixture" "pioneer-master" "present" 0 "SKIP" 0 "test-files\pioneer-master missing"; return }

  $capMB = if ($QA.Tier -eq "STRESS") { 999999 } else { 300 }
  $repo = New-SandboxRepo "gamedev-G1" $Phase
  $assets = Join-Path $repo "assets"
  New-Item -ItemType Directory -Path $assets -Force | Out-Null

  $copied = 0; $copiedMB = 0.0
  foreach ($f in (Get-ChildItem $srcRoot -Recurse -File)) {
    $sizeMB = $f.Length / 1MB
    if (($copiedMB + $sizeMB) -gt $capMB) { break }
    $rel = $f.FullName.Substring($srcRoot.Length + 1)
    $dst = Join-Path $assets $rel
    $dir = Split-Path $dst -Parent
    if (-not (Test-Path $dir)) { New-Item -ItemType Directory -Path $dir -Force | Out-Null }
    Copy-Item -LiteralPath $f.FullName -Destination $dst -Force
    $copied++; $copiedMB += $sizeMB
  }
  Add-Row $id "import" "copy subtree" "files copied" 0 $(if ($copied -gt 0) { "PASS" } else { "FAIL" }) 0 ("files=$copied mb=" + [math]::Round($copiedMB, 1) + " cap=$capMB")

  $add = Invoke-MG $repo @("add", "-A") $Phase
  Add-Row $id "add" "add -A" "exit 0" $add.Exit $(if ($add.Exit -eq 0) { "PASS" } else { "FAIL" }) $add.Sec ""

  $c = Invoke-MG $repo @("commit", "-m", "import game assets") $Phase
  Add-Row $id "commit" "commit" "exit 0" $c.Exit $(if ($c.Exit -eq 0) { "PASS" } else { "FAIL" }) $c.Sec "odb_mb=$(Get-DirMB (Join-Path $repo '.mediagit'))"

  $st = Invoke-MG $repo @("status", "--porcelain") $Phase
  $dirty = ($st.Out -split "`r?`n" | Where-Object { $_ -match "^\s*(\?\?|M |A |D )" }).Count
  Add-Row $id "status" "status --porcelain" "clean" $st.Exit $(if ($st.Exit -eq 0 -and $dirty -eq 0) { "PASS" } else { "FAIL" }) $st.Sec "dirty_lines=$dirty"

  $stats = Invoke-MG $repo @("stats") $Phase
  Add-Row $id "stats" "stats" "exit 0" $stats.Exit $(if ($stats.Exit -eq 0) { "PASS" } else { "FAIL" }) $stats.Sec ""

  Test-FsckGate $id $repo
}

# ---------------------------------------------------------------------------
# G2: rebase feature/art (3 FBX versions) onto advanced main.
# ---------------------------------------------------------------------------
function Run-G2 {
  $id = "G2"
  $fbx = Select-RealFixture (Get-ChildItem (Join-Path $QA.TestFiles "56-fbx") -Recurse -Filter *.fbx -File -EA SilentlyContinue)
  $dds = Select-RealFixture (Get-ChildItem $QA.TestFiles -Recurse -Filter *.dds -File -EA SilentlyContinue | Select-Object -First 40)
  if (-not $fbx -or -not $dds) { Add-Row $id "fixture" "fbx+dds" "present" 0 "SKIP" 0 "fbx=$fbx dds=$dds"; return }

  $repo = New-SandboxRepo "gamedev-G2" $Phase
  $fbxDest = Join-Path $repo "model.fbx"
  $ddsDest = Join-Path $repo "texture.dds"
  Copy-Item -LiteralPath $fbx -Destination $fbxDest -Force
  Copy-Item -LiteralPath $dds -Destination $ddsDest -Force
  Invoke-MG $repo @("add", "-A") $Phase | Out-Null
  Invoke-MG $repo @("commit", "-m", "fbx v1 + dds") $Phase | Out-Null
  $fbxHashes = @(Get-QaHash $fbxDest)

  Invoke-MG $repo @("branch", "create", "feature/art") $Phase | Out-Null
  Invoke-MG $repo @("branch", "switch", "feature/art") $Phase | Out-Null
  for ($v = 2; $v -le 4; $v++) {
    Edit-BytesInPlace $fbxDest (600 + $v)
    Invoke-MG $repo @("add", $fbxDest) $Phase | Out-Null
    Invoke-MG $repo @("commit", "-m", "fbx v$v") $Phase | Out-Null
    $fbxHashes += Get-QaHash $fbxDest
  }
  $tipHash = $fbxHashes[3]

  # main advances with edits to a DIFFERENT file
  Invoke-MG $repo @("branch", "switch", "main") $Phase | Out-Null
  Edit-BytesInPlace $ddsDest 611
  Invoke-MG $repo @("add", $ddsDest) $Phase | Out-Null
  Invoke-MG $repo @("commit", "-m", "dds tweak 1") $Phase | Out-Null
  Edit-BytesInPlace $ddsDest 612
  Invoke-MG $repo @("add", $ddsDest) $Phase | Out-Null
  Invoke-MG $repo @("commit", "-m", "dds tweak 2") $Phase | Out-Null
  $ddsMainHash = Get-QaHash $ddsDest

  Invoke-MG $repo @("branch", "switch", "feature/art") $Phase | Out-Null
  $rb = Invoke-MG $repo @("rebase", "main") $Phase
  Add-Row $id "rebase" "rebase main (on feature/art)" "exit 0" $rb.Exit $(if ($rb.Exit -eq 0) { "PASS" } else { "FAIL" }) $rb.Sec $rb.Out.Substring(0, [Math]::Min(150, $rb.Out.Length))

  # final working-tree content: fbx = feature tip, dds = main's latest
  Assert-HashEq $id "fbx-final" $tipHash (Get-QaHash $fbxDest) "fbx after rebase"
  Assert-HashEq $id "dds-from-main" $ddsMainHash (Get-QaHash $ddsDest) "dds after rebase"

  # all 4 fbx versions reachable: log shows >= 6 commits (1 base + 2 main + 3 rebased)
  $log = Invoke-MG $repo @("log", "--oneline") $Phase
  $count = ($log.Out -split "`r?`n" | Where-Object { $_.Trim() -ne "" }).Count
  Add-Row $id "log" "log --oneline" "6 commits" $log.Exit $(if ($count -eq 6) { "PASS" } else { "FAIL" }) $log.Sec "count=$count"

  # walk back through fbx history via reset-less checkout: use log + show? Cheaper:
  # verify each fbx version retrievable by switching to each rebased commit via bisect-free
  # method: reset --hard HEAD~N would move the branch; instead verify via `diff` presence.
  # Minimal reachability proof: fsck connectivity + log count above; deep per-version
  # hash walk is done for tip + base below.
  Invoke-MG $repo @("branch", "switch", "main") $Phase | Out-Null
  Assert-HashEq $id "fbx-main-untouched" $fbxHashes[0] (Get-QaHash $fbxDest) "fbx v1 on main"
  Invoke-MG $repo @("branch", "switch", "feature/art") $Phase | Out-Null

  Test-FsckGate $id $repo
}

# ---------------------------------------------------------------------------
# G3: cherry-pick a DDS texture fix from release-1 onto release-2.
# ---------------------------------------------------------------------------
function Run-G3 {
  $id = "G3"
  $dds = Select-RealFixture (Get-ChildItem $QA.TestFiles -Recurse -Filter *.dds -File -EA SilentlyContinue | Select-Object -First 40)
  if (-not $dds) { Add-Row $id "fixture" "dds" "present" 0 "SKIP" 0 "no .dds found"; return }

  $repo = New-SandboxRepo "gamedev-G3" $Phase
  $dest = Join-Path $repo "texture.dds"
  Copy-Item -LiteralPath $dds -Destination $dest -Force
  Invoke-MG $repo @("add", $dest) $Phase | Out-Null
  Invoke-MG $repo @("commit", "-m", "common base") $Phase | Out-Null

  Invoke-MG $repo @("branch", "create", "release-1") $Phase | Out-Null
  Invoke-MG $repo @("branch", "create", "release-2") $Phase | Out-Null

  Invoke-MG $repo @("branch", "switch", "release-1") $Phase | Out-Null
  Edit-BytesInPlace $dest 701
  Invoke-MG $repo @("add", $dest) $Phase | Out-Null
  $fix = Invoke-MG $repo @("commit", "-m", "texture fix") $Phase
  $fixOid = Get-CommitHash $fix.Out
  $fixHash = Get-QaHash $dest
  Add-Row $id "fix-commit" "commit texture fix on release-1" "exit 0" $fix.Exit $(if ($fix.Exit -eq 0 -and $fixOid) { "PASS" } else { "FAIL" }) $fix.Sec "oid=$fixOid"

  Invoke-MG $repo @("branch", "switch", "release-2") $Phase | Out-Null
  $cp = Invoke-MG $repo @("cherry-pick", $fixOid) $Phase
  Add-Row $id "cherry-pick" "cherry-pick <fix>" "exit 0" $cp.Exit $(if ($cp.Exit -eq 0) { "PASS" } else { "FAIL" }) $cp.Sec ""

  Assert-HashEq $id "release-2-dds" $fixHash (Get-QaHash $dest) "dds on release-2 after cherry-pick"
  Invoke-MG $repo @("branch", "switch", "release-1") $Phase | Out-Null
  Assert-HashEq $id "release-1-dds" $fixHash (Get-QaHash $dest) "dds on release-1"

  Test-FsckGate $id $repo
}

# ---------------------------------------------------------------------------
# G4: binary conflicts - .blend then .glb (the v11 10x P0 data-loss class).
# ---------------------------------------------------------------------------
function Run-G4 {
  $id = "G4"
  $blend = Select-RealFixture (Get-ChildItem (Join-Path $QA.TestFiles "27-blender") -Recurse -Filter *.blend -File -EA SilentlyContinue)
  if ($blend) { Test-BinaryConflict $id $blend "scene.blend" 800 }
  else { Add-Row $id "scene.blend" "select-blend" "present" 0 "SKIP" 0 "no .blend under 27-blender" }

  $glb = Select-RealFixture (Get-ChildItem $QA.TestFiles -Recurse -Filter *.glb -File -EA SilentlyContinue | Select-Object -First 10)
  if ($glb) { Test-BinaryConflict $id $glb "model.glb" 820 }
  else { Add-Row $id "model.glb" "select-glb" "present" 0 "SKIP" 0 "no .glb found" }
}

# ---------------------------------------------------------------------------
# G5: remote - push assets\ + src\, sparse clone assets only, single-file download.
# ---------------------------------------------------------------------------
function Run-G5 {
  $id = "G5"
  $glb = Select-RealFixture (Get-ChildItem $QA.TestFiles -Recurse -Filter *.glb -File -EA SilentlyContinue | Select-Object -First 10)
  if (-not $glb) { Add-Row $id "fixture" "glb" "present" 0 "SKIP" 0 "no .glb found"; return }

  $remoteLib = Join-Path $PSScriptRoot "lib\remote.ps1"
  if (-not (Test-Path $remoteLib)) { Add-Row $id "server" "Start-QaServer minio" "server up" 0 "SKIP" 0 "lib\remote.ps1 not present yet"; return }
  . $remoteLib
  $srv = $null
  try { $srv = Start-QaServer -Backend minio -Phase $Phase }
  catch { Add-Row $id "server" "Start-QaServer minio" "server up" 1 "SKIP" 0 ($_.Exception.Message); return }

  try {
    $repo = New-SandboxRepo "gamedev-G5" $Phase
    $assets = Join-Path $repo "assets"
    $src = Join-Path $repo "src"
    New-Item -ItemType Directory -Path $assets, $src -Force | Out-Null
    Copy-Item -LiteralPath $glb -Destination (Join-Path $assets "hero.glb") -Force
    "// game code" | Set-Content (Join-Path $src "main.cs") -Encoding ASCII
    $glbHash = Get-QaHash (Join-Path $assets "hero.glb")

    Invoke-MG $repo @("add", "-A") $Phase | Out-Null
    Invoke-MG $repo @("commit", "-m", "assets + src") $Phase | Out-Null
    Invoke-MG $repo @("remote", "add", "origin", $srv.Url) $Phase | Out-Null
    $push = Invoke-MG $repo @("push", "-u", "origin", "main") $Phase
    Add-Row $id "push" "push -u origin main" "exit 0" $push.Exit $(if ($push.Exit -eq 0) { "PASS" } else { "FAIL" }) $push.Sec ""

    # clone help has no sparse flag (checked at build time) - clone then sparse-checkout set
    $cloneDir = Join-Path $QA.Work "gamedev-G5-clone"
    if (Test-Path $cloneDir) { Remove-Item -Recurse -Force $cloneDir }
    $cl = Invoke-MG $null @("clone", $srv.Url, $cloneDir) $Phase
    Add-Row $id "clone" "clone" "exit 0" $cl.Exit $(if ($cl.Exit -eq 0) { "PASS" } else { "FAIL" }) $cl.Sec "note=clone has no sparse flag; using post-clone sparse-checkout"

    if ($cl.Exit -eq 0) {
      $sc = Invoke-MG $cloneDir @("sparse-checkout", "set", "assets") $Phase
      $srcGone = -not (Test-Path (Join-Path $cloneDir "src\main.cs"))
      $assetThere = Test-Path (Join-Path $cloneDir "assets\hero.glb")
      Add-Row $id "sparse" "sparse-checkout set assets" "src absent + assets present" $sc.Exit $(if ($sc.Exit -eq 0 -and $srcGone -and $assetThere) { "PASS" } else { "FAIL" }) $sc.Sec "srcGone=$srcGone assetThere=$assetThere"
      if ($assetThere) { Assert-HashEq $id "sparse-glb-parity" $glbHash (Get-QaHash (Join-Path $cloneDir "assets\hero.glb")) "glb in sparse clone" }
    }

    # single-file download by full URL, no clone
    $dlOut = Join-Path $QA.Work "gamedev-G5-download.glb"
    if (Test-Path $dlOut) { Remove-Item -Force $dlOut }
    $dl = Invoke-MG $null @("download", "$($srv.Url)/assets/hero.glb", "-o", $dlOut) $Phase
    $dlOk = ($dl.Exit -eq 0) -and (Test-Path $dlOut)
    Add-Row $id "download" "download <url>/assets/hero.glb" "exit 0 + file" $dl.Exit $(if ($dlOk) { "PASS" } else { "FAIL" }) $dl.Sec ""
    if ($dlOk) { Assert-HashEq $id "download-parity" $glbHash (Get-QaHash $dlOut) "downloaded glb" }

    Test-FsckGate $id $repo
    if ($cl.Exit -eq 0) { Test-FsckGate $id $cloneDir }
  } finally {
    Stop-QaServer $srv
  }
}

# ---------------------------------------------------------------------------
# G6: branch protect + reset --hard recovery via reflog.
# ---------------------------------------------------------------------------
function Run-G6 {
  $id = "G6"
  $dds = Select-RealFixture (Get-ChildItem $QA.TestFiles -Recurse -Filter *.dds -File -EA SilentlyContinue | Select-Object -First 40)
  if (-not $dds) { Add-Row $id "fixture" "dds" "present" 0 "SKIP" 0 "no .dds found"; return }

  $repo = New-SandboxRepo "gamedev-G6" $Phase
  $dest = Join-Path $repo "texture.dds"
  Copy-Item -LiteralPath $dds -Destination $dest -Force
  Invoke-MG $repo @("add", $dest) $Phase | Out-Null
  Invoke-MG $repo @("commit", "-m", "base") $Phase | Out-Null

  $pr = Invoke-MG $repo @("branch", "protect", "main") $Phase
  Add-Row $id "protect" "branch protect main" "exit 0" $pr.Exit $(if ($pr.Exit -eq 0) { "PASS" } else { "FAIL" }) $pr.Sec $pr.Out.Substring(0, [Math]::Min(150, $pr.Out.Length))

  # verify a protected op is refused: delete of the protected branch (from another branch)
  Invoke-MG $repo @("branch", "create", "work") $Phase | Out-Null
  Invoke-MG $repo @("branch", "switch", "work") $Phase | Out-Null
  $del = Invoke-MG $repo @("branch", "delete", "main") $Phase
  $refused = ($del.Out -match "protected") -or ($del.Exit -ne 0)
  Add-Row $id "protect-enforced" "branch delete main (protected)" "refused" $del.Exit $(if ($refused) { "PASS" } else { "FAIL" }) $del.Sec ("blocks=delete; " + $del.Out.Substring(0, [Math]::Min(120, $del.Out.Length)))
  $stillThere = (Invoke-MG $repo @("branch", "list") $Phase).Out -match "main"
  Add-Row $id "protect-branch-alive" "branch list" "main still listed" 0 $(if ($stillThere) { "PASS" } else { "FAIL" }) 0 ""

  # on the work branch: commit v2, reset --hard HEAD~1, recover via reflog
  Edit-BytesInPlace $dest 901
  $v2Hash = Get-QaHash $dest
  Invoke-MG $repo @("add", $dest) $Phase | Out-Null
  $c2 = Invoke-MG $repo @("commit", "-m", "v2 texture") $Phase
  $v2Oid = Get-CommitHash $c2.Out

  $rs = Invoke-MG $repo @("reset", "--hard", "HEAD~1") $Phase
  Add-Row $id "reset-hard" "reset --hard HEAD~1" "exit 0" $rs.Exit $(if ($rs.Exit -eq 0) { "PASS" } else { "FAIL" }) $rs.Sec ""
  $lostConfirmed = (Get-QaHash $dest) -ne $v2Hash
  Add-Row $id "v2-lost" "hash after reset" "v2 gone from worktree" 0 $(if ($lostConfirmed) { "PASS" } else { "FAIL" }) 0 ""

  $rl = Invoke-MG $repo @("reflog") $Phase
  $foundInReflog = $v2Oid -and ($rl.Out -match $v2Oid.Substring(0, 7))
  Add-Row $id "reflog" "reflog" "lost commit listed" $rl.Exit $(if ($foundInReflog) { "PASS" } else { "FAIL" }) $rl.Sec "v2_oid=$v2Oid"

  $rb = Invoke-MG $repo @("reset", "--hard", $v2Oid) $Phase
  Add-Row $id "reset-back" "reset --hard <lost oid>" "exit 0" $rb.Exit $(if ($rb.Exit -eq 0) { "PASS" } else { "FAIL" }) $rb.Sec ""
  Assert-HashEq $id "v2-recovered" $v2Hash (Get-QaHash $dest) "texture after recovery"

  Test-FsckGate $id $repo
}

# ---------------------------------------------------------------------------
# Dispatch
# ---------------------------------------------------------------------------
$allScenarios = @("G1", "G2", "G3", "G4", "G5", "G6")
$toRun = if ($Only) { @($Only) } else { $allScenarios }

foreach ($sid in $toRun) {
  switch ($sid) {
    "G1" { Invoke-Scenario "G1" { Run-G1 } }
    "G2" { Invoke-Scenario "G2" { Run-G2 } }
    "G3" { Invoke-Scenario "G3" { Run-G3 } }
    "G4" { Invoke-Scenario "G4" { Run-G4 } }
    "G5" { Invoke-Scenario "G5" { Run-G5 } }
    "G6" { Invoke-Scenario "G6" { Run-G6 } }
    default { Write-QaLog $Phase "Unknown scenario id: $sid" }
  }
}

# Teardown: reclaim this phase's own work/ scratch so a long campaign cannot run the
# volume out of space. work/ ONLY - logs/ and fixtures-synthetic/ are never touched.
Invoke-QaTeardown $Phase @("gamedev-*")

Exit-QaPhase $Phase (-not $script:AllPass)
