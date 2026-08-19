# 02_compat.ps1 - wire/persisted-format freeze gate (J4).
# Clones+fscks the FROZEN compat fixture at dev-tests/compat-fixture/repo, whose
# bytes were produced once, at the format-freeze commit, by that build's release
# binary. A future build that can no longer read those bytes (a pack/manifest/
# chunk-delta/LAYOUT format regression) fails fsck here instead of silently
# breaking clones of pre-freeze repositories. See docs/FORMATS.md sec 11-12.
#
# fsck is the enforceable half (it parses every persisted format); the wire/clone
# transfer path is exercised continuously by 06_remote against live repos.
. (Join-Path $PSScriptRoot "lib\common.ps1")
$Phase = "02_compat"

$fixture = Join-Path $QA.RepoRoot "dev-tests\compat-fixture\repo"
Write-QaLog $Phase ("compat fixture = {0}" -f $fixture)

if (-not (Test-Path (Join-Path $fixture ".mediagit"))) {
  Write-QaGate $Phase "compat-fixture-present" $false "missing frozen fixture at $fixture"
  exit 1
}

# Work on a copy so the frozen bytes under version control are never mutated.
$work = Join-Path $QA.Work "compat-fixture-check"
if (Test-Path $work) { Remove-Item -Recurse -Force $work }
New-Item -ItemType Directory -Path $work -Force | Out-Null
Copy-Item -Recurse -Force (Join-Path $fixture "*") $work

# The freeze only means anything if the frozen bytes actually contain each
# persisted format. Assert their presence before trusting a green fsck.
$odb = Join-Path $work ".mediagit\objects\repo"
$have = [ordered]@{
  "LAYOUT-marker" = (Test-Path (Join-Path $odb "LAYOUT"))
  "pack-v3"       = @(Get-ChildItem -Recurse -File -Path (Join-Path $odb "packs") -Filter *.pack -EA SilentlyContinue).Count -gt 0
  "manifest-mgcm" = @(Get-ChildItem -Recurse -File -Path (Join-Path $odb "manifests") -EA SilentlyContinue).Count -gt 0
  "chunk-delta"   = @(Get-ChildItem -Recurse -File -Path (Join-Path $odb "chunk-deltas") -Filter *.meta -EA SilentlyContinue).Count -gt 0
}
$missing = @($have.GetEnumerator() | Where-Object { -not $_.Value } | ForEach-Object { $_.Key })
$coverageOk = $missing.Count -eq 0
Write-QaGate $Phase "compat-fixture-covers-formats" $coverageOk `
  ("present=" + (($have.GetEnumerator() | ForEach-Object { "$($_.Key)=$($_.Value)" }) -join " "))

# The gate: a current build must read the frozen bytes clean.
$r = Invoke-MG $work @("fsck") $Phase -TimeoutSec 300
# "integrity" appears in fsck's PERFECT, OK-with-warnings AND FAILED lines
# alike, so the old `PERFECT|integrity` alternation matched unconditionally and
# the gate was really just the exit code -- which tolerates warnings. The point
# of a frozen-format fixture is that a current build reads it CLEAN, so match
# the verdict itself.
$fsckOk = ($r.Exit -eq 0) -and ($r.Out -match "integrity:\s*PERFECT")
Write-QaGate $Phase "compat-fixture-fsck-clean" $fsckOk ("exit=$($r.Exit)")

# work/ scratch: the fixture copy this phase made.
Invoke-QaTeardown $Phase @("compat-fixture-check*")

Exit-QaPhase $Phase
