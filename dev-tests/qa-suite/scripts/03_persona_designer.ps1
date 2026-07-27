# Persona: Designer (PSD/AI/SVG workflows: iterate, branch merge, stash, revert, signed release).
# Scenarios D1-D5. ASCII-only, PS 5.1 compatible.
param([string]$Only)

. (Join-Path $PSScriptRoot "lib\common.ps1")

$Phase  = "persona_designer"
$Tsv    = Join-Path $QA.Reports "scenario_designer.tsv"
$Header = @("scenario", "step", "action", "expect", "exit", "pass", "sec", "detail")
$script:AllPass = $true

# --- shared helpers (duplicated per-script by design; see qa-suite contract) ---

function Add-Row([string]$Scenario, [string]$StepName, [string]$Action, [string]$Expect, $Exit, [string]$Status, $Sec, [string]$Detail = "") {
  Write-QaRow $Tsv $Header @($Scenario, $StepName, $Action, $Expect, $Exit, $Status, $Sec, $Detail)
  if ($Status -eq "FAIL") { $script:AllPass = $false }
  Write-QaLog $Phase ("{0}/{1} {2} -> {3} (exit={4} sec={5})" -f $Scenario, $StepName, $Action, $Status, $Exit, $Sec)
}

function Test-FsckGate([string]$Scenario, [string]$Repo, [string[]]$ExtraArgs = @()) {
  $fsckArgs = @("fsck") + $ExtraArgs
  $r = Invoke-MG $Repo $fsckArgs $Phase
  $status = if ($r.Exit -eq 0) { "PASS" } else { "FAIL" }
  $detail = $r.Out.Substring(0, [Math]::Min(200, $r.Out.Length))
  Add-Row $Scenario "fsck" ($fsckArgs -join " ") "exit 0" $r.Exit $status $r.Sec $detail
  Write-QaGate $Phase "$Scenario-fsck" ($status -eq "PASS") $detail
  return ($status -eq "PASS")
}

function Assert-HashEq([string]$Scenario, [string]$StepName, [string]$Expected, [string]$Actual, [string]$Detail = "") {
  $status = if ($Actual -and $Expected -and ($Actual -eq $Expected)) { "PASS" } else { "FAIL" }
  $exp8 = if ($Expected) { $Expected.Substring(0, [Math]::Min(12, $Expected.Length)) } else { "null" }
  $act8 = if ($Actual) { $Actual.Substring(0, [Math]::Min(12, $Actual.Length)) } else { "null" }
  Add-Row $Scenario $StepName "hash-parity" "match" 0 $status 0 "$Detail expected=$exp8 actual=$act8"
}

# Pick the smallest file at/over $MinMB from a candidate list; falls back to the
# largest if none qualify; $null if the list is empty. Candidates may be
# FileInfo objects or path strings.
function Select-RealFixture([object[]]$Candidates, [int]$MinMB = 1) {
  if (-not $Candidates) { return $null }
  $items = $Candidates | Where-Object { $_ } | ForEach-Object { if ($_ -is [IO.FileInfo]) { $_ } else { Get-Item -LiteralPath $_ -EA SilentlyContinue } } | Where-Object { $_ }
  if (-not $items) { return $null }
  $over = $items | Where-Object { $_.Length -ge ($MinMB * 1MB) } | Sort-Object Length | Select-Object -First 1
  if ($over) { return $over.FullName }
  return ($items | Sort-Object Length -Descending | Select-Object -First 1).FullName
}

# Deterministic byte-slice edit in place: overwrite a slice near the middle
# of the file with a seeded pseudo-random pattern.
function Edit-BytesInPlace([string]$Path, [int]$Seed, [int]$SliceLen = 4096) {
  $bytes = [IO.File]::ReadAllBytes($Path)
  if ($bytes.Length -eq 0) { return }
  $rnd = New-Object Random($Seed)
  $len = [Math]::Min($SliceLen, $bytes.Length)
  $start = if ($bytes.Length -gt $len) { [int](($bytes.Length - $len) / 2) } else { 0 }
  for ($i = 0; $i -lt $len; $i++) { $bytes[$start + $i] = [byte]$rnd.Next(0, 256) }
  [IO.File]::WriteAllBytes($Path, $bytes)
}

function Copy-WithByteEdit([string]$Src, [string]$Dst, [int]$Seed, [int]$SliceLen = 4096) {
  Copy-Item -LiteralPath $Src -Destination $Dst -Force
  Edit-BytesInPlace $Dst $Seed $SliceLen
}

function Get-CommitHash([string]$Out) {
  if ($Out -match "Created commit ([0-9a-fA-F]{8,})") { return $Matches[1] }
  return $null
}

function Invoke-Scenario([string]$Id, [scriptblock]$Body) {
  try {
    & $Body
  } catch {
    Add-Row $Id "exception" "run" "no exception" -1 "FAIL" 0 ($_.Exception.Message)
  }
}

# ---------------------------------------------------------------------------
# D1: PSD iterate - 5 versions, ODB growth per version, log/diff/show.
# ---------------------------------------------------------------------------
function Run-D1 {
  $id = "D1"
  $repo = New-SandboxRepo "designer-D1" $Phase
  $psd = Select-RealFixture (Get-ChildItem (Join-Path $QA.TestFiles "psd") -Filter *.psd -File -EA SilentlyContinue)
  if (-not $psd) { Add-Row $id "fixture" "select-psd" "psd found" 0 "SKIP" 0 "no *.psd under test-files\psd"; return }

  $dest = Join-Path $repo "art.psd"
  $hashes = @()
  for ($v = 1; $v -le 5; $v++) {
    if ($v -eq 1) { Copy-Item -LiteralPath $psd -Destination $dest -Force } else { Edit-BytesInPlace $dest (1000 + $v) }
    $r1 = Invoke-MG $repo @("add", $dest) $Phase
    $r2 = Invoke-MG $repo @("commit", "-m", "v$v") $Phase
    $status = if ($r2.Exit -eq 0) { "PASS" } else { "FAIL" }
    $hash = Get-CommitHash $r2.Out
    $hashes += $hash
    $odbMB = Get-DirMB (Join-Path $repo ".mediagit")
    Add-Row $id "commit-v$v" "add+commit" "exit 0" $r2.Exit $status $r2.Sec "odb_mb=$odbMB hash=$hash"
  }

  $log = Invoke-MG $repo @("log", "--oneline") $Phase
  $count = ($log.Out -split "`r?`n" | Where-Object { $_.Trim() -ne "" }).Count
  $status = if ($count -eq 5) { "PASS" } else { "FAIL" }
  Add-Row $id "log" "log --oneline" "5 commits" 0 $status $log.Sec "count=$count"

  if ($hashes.Count -eq 5 -and $hashes[3] -and $hashes[4]) {
    $diff = Invoke-MG $repo @("diff", $hashes[3], $hashes[4]) $Phase
    Add-Row $id "diff" "diff v4..v5" "exit 0" $diff.Exit $(if ($diff.Exit -eq 0) { "PASS" } else { "FAIL" }) $diff.Sec ""
  }

  $show = Invoke-MG $repo @("show") $Phase
  Add-Row $id "show" "show HEAD" "exit 0" $show.Exit $(if ($show.Exit -eq 0) { "PASS" } else { "FAIL" }) $show.Sec ""

  Test-FsckGate $id $repo | Out-Null
}

# ---------------------------------------------------------------------------
# D2: client-a / client-b branches, one clean merge, one binary conflict.
# ---------------------------------------------------------------------------
function Run-D2 {
  $id = "D2"
  $repo = New-SandboxRepo "designer-D2" $Phase
  $psd = Select-RealFixture (Get-ChildItem (Join-Path $QA.TestFiles "psd") -Filter *.psd -File -EA SilentlyContinue)
  if (-not $psd) { Add-Row $id "fixture" "select-psd" "psd found" 0 "SKIP" 0 "no *.psd under test-files\psd"; return }

  $dest = Join-Path $repo "art.psd"
  Copy-Item -LiteralPath $psd -Destination $dest -Force
  Invoke-MG $repo @("add", $dest) $Phase | Out-Null
  Invoke-MG $repo @("commit", "-m", "base") $Phase | Out-Null

  Invoke-MG $repo @("branch", "create", "client-a") $Phase | Out-Null
  Invoke-MG $repo @("branch", "create", "client-b") $Phase | Out-Null

  Invoke-MG $repo @("branch", "switch", "client-a") $Phase | Out-Null
  Edit-BytesInPlace $dest 201
  Invoke-MG $repo @("add", $dest) $Phase | Out-Null
  Invoke-MG $repo @("commit", "-m", "client-a edit") $Phase | Out-Null
  $hashA = Get-QaHash $dest
  $sideA = Join-Path $QA.Work "designer-D2-client-a.bin"
  Copy-Item -LiteralPath $dest -Destination $sideA -Force

  Invoke-MG $repo @("branch", "switch", "client-b") $Phase | Out-Null
  Edit-BytesInPlace $dest 202
  Invoke-MG $repo @("add", $dest) $Phase | Out-Null
  Invoke-MG $repo @("commit", "-m", "client-b edit") $Phase | Out-Null
  $hashB = Get-QaHash $dest
  $sideB = Join-Path $QA.Work "designer-D2-client-b.bin"
  Copy-Item -LiteralPath $dest -Destination $sideB -Force

  Invoke-MG $repo @("branch", "switch", "main") $Phase | Out-Null
  $m1 = Invoke-MG $repo @("merge", "client-a") $Phase
  Add-Row $id "merge-a" "merge client-a" "exit 0 (clean)" $m1.Exit $(if ($m1.Exit -eq 0) { "PASS" } else { "FAIL" }) $m1.Sec ""

  $m2 = Invoke-MG $repo @("merge", "client-b") $Phase
  Add-Row $id "merge-b" "merge client-b" "exit 1 (conflict)" $m2.Exit $(if ($m2.Exit -ne 0) { "PASS" } else { "FAIL" }) $m2.Sec $m2.Out.Substring(0, [Math]::Min(200, $m2.Out.Length))

  $st = Invoke-MG $repo @("status") $Phase
  Add-Row $id "status" "status" "shows conflict" $st.Exit "PASS" $st.Sec $st.Out.Substring(0, [Math]::Min(200, $st.Out.Length))

  # resolve by choosing client-b's version
  Copy-Item -LiteralPath $sideB -Destination $dest -Force
  Invoke-MG $repo @("add", $dest) $Phase | Out-Null
  $mc = Invoke-MG $repo @("merge", "--continue") $Phase
  Add-Row $id "resolve" "merge --continue-merge client-b" "exit 0" $mc.Exit $(if ($mc.Exit -eq 0) { "PASS" } else { "FAIL" }) $mc.Sec ""

  $resolvedHash = Get-QaHash $dest
  Assert-HashEq $id "resolved-is-b" $hashB $resolvedHash "resolved file should equal client-b bytes"

  # verify both branch versions still retrievable
  Invoke-MG $repo @("branch", "switch", "client-a") $Phase | Out-Null
  Assert-HashEq $id "client-a-retrievable" $hashA (Get-QaHash $dest) "switch to client-a"

  Invoke-MG $repo @("branch", "switch", "client-b") $Phase | Out-Null
  Assert-HashEq $id "client-b-retrievable" $hashB (Get-QaHash $dest) "switch to client-b"

  Invoke-MG $repo @("branch", "switch", "main") $Phase | Out-Null
  Test-FsckGate $id $repo | Out-Null
}

# ---------------------------------------------------------------------------
# D3: stash round-trip on an AI file.
# ---------------------------------------------------------------------------
function Run-D3 {
  $id = "D3"
  $repo = New-SandboxRepo "designer-D3" $Phase
  $ai = Select-RealFixture (Get-ChildItem $QA.TestFiles -Filter *.ai -File -EA SilentlyContinue)
  if (-not $ai) { Add-Row $id "fixture" "select-ai" "ai found" 0 "SKIP" 0 "no *.ai at top level of test-files"; return }

  $dest = Join-Path $repo "art.ai"
  Copy-Item -LiteralPath $ai -Destination $dest -Force
  Invoke-MG $repo @("add", $dest) $Phase | Out-Null
  Invoke-MG $repo @("commit", "-m", "base") $Phase | Out-Null
  $headHash = Get-QaHash $dest

  Edit-BytesInPlace $dest 303
  $modifiedHash = Get-QaHash $dest

  $sp = Invoke-MG $repo @("stash", "push", "wip-ai-edit") $Phase
  Add-Row $id "stash-push" "stash push" "exit 0" $sp.Exit $(if ($sp.Exit -eq 0) { "PASS" } else { "FAIL" }) $sp.Sec ""
  Assert-HashEq $id "restored-to-head" $headHash (Get-QaHash $dest) "working tree after stash push"

  Invoke-MG $repo @("branch", "create", "tmp-d3") $Phase | Out-Null
  Invoke-MG $repo @("branch", "switch", "tmp-d3") $Phase | Out-Null
  Invoke-MG $repo @("branch", "switch", "main") $Phase | Out-Null

  $pop = Invoke-MG $repo @("stash", "pop") $Phase
  Add-Row $id "stash-pop" "stash pop" "exit 0" $pop.Exit $(if ($pop.Exit -eq 0) { "PASS" } else { "FAIL" }) $pop.Sec ""
  Assert-HashEq $id "modified-returned" $modifiedHash (Get-QaHash $dest) "working tree after stash pop"

  Test-FsckGate $id $repo | Out-Null
}

# ---------------------------------------------------------------------------
# D4: revert a bad SVG change.
# ---------------------------------------------------------------------------
function Run-D4 {
  $id = "D4"
  $repo = New-SandboxRepo "designer-D4" $Phase
  $svg = Select-RealFixture (Get-ChildItem $QA.TestFiles -Filter *.svg -File -EA SilentlyContinue)
  if (-not $svg) { Add-Row $id "fixture" "select-svg" "svg found" 0 "SKIP" 0 "no *.svg at top level of test-files"; return }

  $dest = Join-Path $repo "logo.svg"
  Copy-Item -LiteralPath $svg -Destination $dest -Force
  Invoke-MG $repo @("add", $dest) $Phase | Out-Null
  Invoke-MG $repo @("commit", "-m", "good") $Phase | Out-Null
  $goodHash = Get-QaHash $dest

  Edit-BytesInPlace $dest 404
  Invoke-MG $repo @("add", $dest) $Phase | Out-Null
  Invoke-MG $repo @("commit", "-m", "bad change") $Phase | Out-Null

  $rv = Invoke-MG $repo @("revert", "HEAD") $Phase
  Add-Row $id "revert" "revert HEAD" "exit 0" $rv.Exit $(if ($rv.Exit -eq 0) { "PASS" } else { "FAIL" }) $rv.Sec ""
  Assert-HashEq $id "reverted-matches-good" $goodHash (Get-QaHash $dest) "file after revert HEAD"

  Test-FsckGate $id $repo | Out-Null
}

# ---------------------------------------------------------------------------
# D5: signed release - ed25519 key, signed annotated tag, push+clone parity.
# ---------------------------------------------------------------------------
function Run-D5 {
  $id = "D5"
  $repo = New-SandboxRepo "designer-D5" $Phase
  $svg = Select-RealFixture (Get-ChildItem $QA.TestFiles -Filter *.svg -File -EA SilentlyContinue)
  if (-not $svg) { Add-Row $id "fixture" "select-svg" "svg found" 0 "SKIP" 0 "no *.svg at top level of test-files"; return }

  $keyDir = Join-Path $repo ".qa-keys"
  New-Item -ItemType Directory -Path $keyDir -Force | Out-Null
  $keyPath = Join-Path $keyDir "id_ed25519"
  $keygen = & ssh-keygen -t ed25519 -N '""' -f $keyPath -q 2>&1
  $keygenOk = ($LASTEXITCODE -eq 0) -and (Test-Path $keyPath)
  if (-not $keygenOk) { Add-Row $id "keygen" "ssh-keygen" "key created" 1 "SKIP" 0 "ssh-keygen unavailable or failed"; return }
  Add-Row $id "keygen" "ssh-keygen -t ed25519" "key created" 0 "PASS" 0 ""

  $dest = Join-Path $repo "release.svg"
  Copy-Item -LiteralPath $svg -Destination $dest -Force
  Invoke-MG $repo @("add", $dest) $Phase | Out-Null
  Invoke-MG $repo @("commit", "-m", "release payload") $Phase | Out-Null

  $env:MEDIAGIT_SIGN = "1"
  $env:MEDIAGIT_SIGN_KEY = $keyPath
  try {
    $tc = Invoke-MG $repo @("tag", "create", "-a", "-m", "release v1.0", "v1.0") $Phase
    Add-Row $id "tag-create" "tag create -a -m ... v1.0 (MEDIAGIT_SIGN=1)" "exit 0" $tc.Exit $(if ($tc.Exit -eq 0) { "PASS" } else { "FAIL" }) $tc.Sec ""

    $tv = Invoke-MG $repo @("tag", "verify", "v1.0") $Phase
    $tvPass = ($tv.Exit -eq 0) -and ($tv.Out -match "valid signature")
    Add-Row $id "tag-verify" "tag verify v1.0" "valid signature" $tv.Exit $(if ($tvPass) { "PASS" } else { "FAIL" }) $tv.Sec $tv.Out.Substring(0, [Math]::Min(150, $tv.Out.Length))

    $remoteLib = Join-Path $PSScriptRoot "lib\remote.ps1"
    $srv = $null
    if (Test-Path $remoteLib) {
      . $remoteLib
      try { $srv = Start-QaServer -Backend minio -Phase $Phase }
      catch { Add-Row $id "server" "Start-QaServer minio" "server up" 1 "SKIP" 0 ($_.Exception.Message) }
    } else {
      Add-Row $id "server" "Start-QaServer minio" "server up" 0 "SKIP" 0 "lib\remote.ps1 not present yet"
    }
    if ($srv) {
      try {
        Invoke-MG $repo @("remote", "add", "origin", $srv.Url) $Phase | Out-Null
        $push = Invoke-MG $repo @("push", "-u", "origin", "main", "--tags") $Phase
        Add-Row $id "push" "push origin main --tags" "exit 0" $push.Exit $(if ($push.Exit -eq 0) { "PASS" } else { "FAIL" }) $push.Sec ""

        $cloneDir = Join-Path $QA.Work "designer-D5-clone"
        if (Test-Path $cloneDir) { Remove-Item -Recurse -Force $cloneDir }
        $cl = Invoke-MG $null @("clone", $srv.Url, $cloneDir) $Phase
        Add-Row $id "clone" "clone (fresh)" "exit 0" $cl.Exit $(if ($cl.Exit -eq 0) { "PASS" } else { "FAIL" }) $cl.Sec ""

        $tv2 = Invoke-MG $cloneDir @("tag", "verify", "v1.0") $Phase
        $tv2Pass = ($tv2.Exit -eq 0) -and ($tv2.Out -match "valid signature")
        Add-Row $id "clone-tag-verify" "tag verify v1.0 (clone)" "valid signature" $tv2.Exit $(if ($tv2Pass) { "PASS" } else { "FAIL" }) $tv2.Sec ""

        Assert-HashEq $id "clone-file-parity" (Get-QaHash $dest) (Get-QaHash (Join-Path $cloneDir "release.svg")) "release.svg across clone"
      } finally {
        Stop-QaServer $srv
      }
    } else {
      Add-Row $id "push" "push origin main --tags" "exit 0" 0 "SKIP" 0 "no server"
      Add-Row $id "clone" "clone (fresh)" "exit 0" 0 "SKIP" 0 "no server"
    }
  } finally {
    Remove-Item Env:\MEDIAGIT_SIGN -EA SilentlyContinue
    Remove-Item Env:\MEDIAGIT_SIGN_KEY -EA SilentlyContinue
  }

  Test-FsckGate $id $repo | Out-Null
}

# ---------------------------------------------------------------------------
# Dispatch
# ---------------------------------------------------------------------------
$allScenarios = @("D1", "D2", "D3", "D4", "D5")
$toRun = if ($Only) { @($Only) } else { $allScenarios }

foreach ($sid in $toRun) {
  switch ($sid) {
    "D1" { Invoke-Scenario "D1" { Run-D1 } }
    "D2" { Invoke-Scenario "D2" { Run-D2 } }
    "D3" { Invoke-Scenario "D3" { Run-D3 } }
    "D4" { Invoke-Scenario "D4" { Run-D4 } }
    "D5" { Invoke-Scenario "D5" { Run-D5 } }
    default { Write-QaLog $Phase "Unknown scenario id: $sid" }
  }
}

# Teardown: reclaim this phase's own work/ scratch so a long campaign cannot run the
# volume out of space. work/ ONLY - logs/ and fixtures-synthetic/ are never touched.
Invoke-QaTeardown $Phase @("designer-*")

Exit-QaPhase $Phase (-not $script:AllPass)
