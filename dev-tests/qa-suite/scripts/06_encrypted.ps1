# 06_encrypted.ps1 - DC-7/D4: an encrypted repository, pushed and cloned, per backend.
#
# WHY THIS EXISTS, and why the rest of the suite does not cover it:
#
# Every other phase runs UNENCRYPTED. The only encryption drills are A16/A17
# in 07_abuse, and both are purely local - `key init`, seal, read back, recover.
# Nothing anywhere pushed an encrypted repository to a real backend and cloned
# it back, which meant the whole D4 escrow path had zero cloud coverage.
#
# That gap hid a P0. Encrypted clone was completely broken -- the client
# re-sealed chunk bytes that arrived already sealed, so every chunk failed its
# integrity check on checkout. The unit suite was green, and the Rust e2e drill
# passed, because it reads back through a server-side ODB and never through the
# clone ingest path. A real clone is what caught it.
#
# The cloud path is also genuinely different from the local one: chunks travel
# over presigned PUT/GET straight to the object store, so the server verifies
# bytes it never transported. That is exactly where a verification site holding
# an unkeyed compressor stops being a wrong answer and starts quarantining
# good packs.
#
# Gate: for every selected backend, an encrypted push succeeds, a clone comes
# back byte-identical, and the objects on the backend are actually sealed.
# The last one is not decoration -- without it a silently-unencrypted repo
# round-trips perfectly and passes everything else here.

param([switch]$KeepWork)

. (Join-Path $PSScriptRoot "lib\common.ps1")
. (Join-Path $PSScriptRoot "lib\remote.ps1")

$Phase = "06_encrypted"
$env:MEDIAGIT_AUTHOR_NAME = "QA-Suite"
$env:MEDIAGIT_AUTHOR_EMAIL = "qa-suite@mediagit.local"

$TSV = Join-Path $QA.Logs "encrypted_results.tsv"
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

Write-QaLog $Phase "encrypted round-trip: backends = $($QA.Backends -join ',')"

# A master keyfile for the client and one for the server. 32 raw bytes, the
# shape both accept (the other is 64 hex chars).
function New-QaKeyfile([string]$Path) {
  $dir = Split-Path $Path -Parent
  if (-not (Test-Path $dir)) { New-Item -ItemType Directory -Path $dir -Force | Out-Null }
  $b = New-Object byte[] 32
  (New-Object Security.Cryptography.RNGCryptoServiceProvider).GetBytes($b)
  [IO.File]::WriteAllBytes($Path, $b)
}

# Is anything under this repo's object storage carrying the MGEN envelope?
#
# Counts what it examined and reports both numbers. "0 sealed of 0 examined"
# and "0 sealed of 400 examined" are completely different findings, and a
# detector that cannot tell them apart passes for the wrong reason -- which is
# the exact mistake that shipped once already in an equivalent Rust test.
function Get-QaSealCount([string]$RepoRoot) {
  $stats = @{ Examined = 0; Sealed = 0 }
  $md = Join-Path $RepoRoot ".mediagit"
  if (-not (Test-Path $md)) { return $stats }
  foreach ($f in Get-ChildItem $md -Recurse -File -EA SilentlyContinue) {
    $p = $f.FullName
    # Match on the object DIRECTORIES rather than a fixed prefix. An inited
    # repo stores under .mediagit\objects\<ns>\chunks\.., a clone under
    # .mediagit\<ns>\chunks\.. -- a hardcoded path finds one and silently
    # reports "0 of 0" for the other, which reads as a pass.
    $isObjectDir = ($p -like '*\chunks\*') -or ($p -like '*\objects\*') -or ($p -like '*\manifests\*')
    if (-not $isObjectDir) { continue }
    if ($f.Name -eq "LAYOUT") { continue }
    # Pack containers are plaintext framing around sealed entries; delta
    # sidecars are a known unsealed gap, recorded in the rc.3 changelog.
    if (($p -like '*\packs\*') -or ($f.Extension -eq ".meta")) { continue }
    $stats.Examined++
    $fs = [IO.File]::OpenRead($p)
    try {
      $head = New-Object byte[] 4
      if ($fs.Read($head, 0, 4) -eq 4 -and
          $head[0] -eq 0x4D -and $head[1] -eq 0x47 -and $head[2] -eq 0x45 -and $head[3] -eq 0x4E) {
        $stats.Sealed++
      }
    } finally { $fs.Dispose() }
  }
  return $stats
}

$prevKeyfile = $env:MEDIAGIT_ENCRYPTION_KEYFILE
$results = @()

try {
  foreach ($backend in $QA.Backends) {
    $srv = $null
    $tag = "enc-$backend"
    try {
      $clientKey = Join-Path $QA.Work "$tag-client-master.key"
      $serverKey = Join-Path $QA.Work "$tag-server-master.key"
      New-QaKeyfile $clientKey
      New-QaKeyfile $serverKey

      try {
        $srv = Start-QaServer -Backend $backend -Phase $Phase -EncryptionKeyfile $serverKey
      } catch {
        if ("$_" -match "^SKIP:") { Rec "$tag-roundtrip" "SKIP" "$_"; continue }
        Rec "$tag-roundtrip" $false "server unavailable: $_"
        continue
      }

      # --- a fresh, encrypted repository. Encryption is init-time-only, so the
      # key has to exist before the first object does.
      $env:MEDIAGIT_ENCRYPTION_KEYFILE = $clientKey
      $repo = New-SandboxRepo "$tag-src" $Phase
      $init = Invoke-MG $repo @("key", "init") $Phase
      if ($init.Exit -ne 0) { Rec "$tag-roundtrip" $false "key init failed: $($init.Out)"; continue }

      # Enough content to chunk: files over 1 MiB travel as individual chunks
      # over the presigned path, which is the path this phase exists to cover.
      $asset = Join-Path $repo "asset.bin"
      New-QaBinaryFixture $asset 6 70413
      $asset2 = Join-Path $repo "asset2.bin"
      New-QaBinaryFixture $asset2 3 70414
      Invoke-MG $repo @("add", ".") $Phase | Out-Null
      $commit = Invoke-MG $repo @("commit", "-m", "encrypted $backend") $Phase
      if ($commit.Exit -ne 0) { Rec "$tag-roundtrip" $false "commit failed: $($commit.Out)"; continue }

      $srcHashes = Get-QaTreeHashes $repo
      if ($srcHashes.Count -eq 0) { Rec "$tag-roundtrip" $false "source tree hashed to nothing"; continue }

      # --- push. This is what refused outright before D4.
      Invoke-MG $repo @("remote", "add", "origin", $srv.Url) $Phase | Out-Null
      $push = Invoke-MG $repo @("push", "origin") $Phase -TimeoutSec 3600
      $pushOk = ($push.Exit -eq 0)
      Rec "$tag-push" $pushOk "exit=$($push.Exit) sec=$($push.Sec)"
      if (-not $pushOk) { Rec "$tag-roundtrip" $false "push failed: $($push.Out)"; continue }

      # --- clone into a machine that has the same master keyfile available.
      # The repo key itself comes back from escrow; the master is what unwraps
      # the local copy the clone writes.
      $clone = Join-Path $QA.Work "$tag-clone"
      if (Test-Path $clone) { Remove-Item -Recurse -Force $clone }
      $cl = Invoke-MG $null @("clone", $srv.Url, $clone) $Phase -TimeoutSec 3600
      $cloneOk = ($cl.Exit -eq 0) -and (Test-Path $clone)
      Rec "$tag-clone" $cloneOk "exit=$($cl.Exit) sec=$($cl.Sec)"
      if (-not $cloneOk) { Rec "$tag-roundtrip" $false "clone failed: $($cl.Out)"; continue }

      # --- byte parity. The P0 this phase was written for produced a clone that
      # errored; a subtler one would produce a clone that differs.
      $cloneHashes = Get-QaTreeHashes $clone
      $mismatch = @(Compare-Object $srcHashes $cloneHashes).Count
      $parity = ($mismatch -eq 0) -and ($cloneHashes.Count -eq $srcHashes.Count)

      # --- and it must actually be encrypted, on both ends. A repo that
      # silently stored plaintext round-trips perfectly.
      $srcSeal = Get-QaSealCount $repo
      $cloneSeal = Get-QaSealCount $clone
      $sealedOk = ($srcSeal.Examined -gt 0) -and ($srcSeal.Sealed -eq $srcSeal.Examined) -and
                  ($cloneSeal.Examined -gt 0) -and ($cloneSeal.Sealed -eq $cloneSeal.Examined)

      # --- fsck the clone: parity says the working tree matches, fsck says the
      # object store behind it is coherent.
      $fsck = Invoke-MG $clone @("fsck") $Phase -TimeoutSec 1200
      $fsckOk = ($fsck.Exit -eq 0)

      Rec "$tag-roundtrip" ($parity -and $sealedOk -and $fsckOk) (
        "files=$($srcHashes.Count) mismatches=$mismatch parity=$parity " +
        "src-sealed=$($srcSeal.Sealed)/$($srcSeal.Examined) " +
        "clone-sealed=$($cloneSeal.Sealed)/$($cloneSeal.Examined) " +
        "sealed-ok=$sealedOk fsck-exit=$($fsck.Exit)")

      $results += [pscustomobject]@{ Backend = $backend; Parity = $parity; Sealed = $sealedOk }
    } catch {
      if ("$_" -match "^SKIP:") { Rec "$tag-roundtrip" "SKIP" "$_" }
      else { Rec "$tag-roundtrip" $false "unexpected error: $_" }
    } finally {
      if ($srv) { Stop-QaServer $srv }
      if (-not $KeepWork) {
        foreach ($d in @("$tag-clone", "$tag-src")) {
          $p = Join-Path $QA.Work $d
          if (Test-Path $p) { Remove-Item -Recurse -Force $p -EA SilentlyContinue }
        }
      }
    }
  }
} finally {
  if ($null -eq $prevKeyfile) { Remove-Item Env:MEDIAGIT_ENCRYPTION_KEYFILE -EA SilentlyContinue }
  else { $env:MEDIAGIT_ENCRYPTION_KEYFILE = $prevKeyfile }
}

if (-not $KeepWork) { Invoke-QaTeardown $Phase @("enc-*") }

Exit-QaPhase $Phase (-not $script:AllPass)
