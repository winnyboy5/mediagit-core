# 13_docs.ps1 - do the CLI docs describe the CLI that exists?
#
# Two pages under book/src/cli/ were found to be outright fiction (branch.md and
# diff.md in the 2026-07-18 truth-up; stats.md on 2026-08-05), each documenting
# flags and output the binary has never produced. Both were found by a human
# happening to read them. Nothing checked.
#
# This phase compares every documented `--flag` against the binary's own
# `--help`, recursing one level into subcommands. It gates on the count NOT
# GROWING, in the same shape as `coverage-placeholder-regression` in 02_matrix -
# because that debt is real and large (97 at the time of writing) and a gate that
# demanded zero would simply be switched off.
#
# Deliberately NOT gated on: real flags that are undocumented. That is a
# different and more forgivable defect (an omission, not a falsehood), and
# folding it in would let the two move against each other invisibly. It is
# reported for information.
param()
. (Join-Path $PSScriptRoot "lib\common.ps1")
$Phase = "13_docs"

$DOCS_DIR = Join-Path $QA.RepoRoot "book\src\cli"
$BASELINE = Join-Path $QA.Root "baselines\docs-invented-flags.tsv"
$TSV      = Join-Path $QA.Logs "docs_flags.tsv"
$HEADER   = @("page", "invented", "undocumented", "flags")

# Pages that document a topic rather than one command; there is no `mediagit
# <page>` to compare against.
$TOPIC_PAGES = @(
  "README", "core-commands", "branch-management", "remote-operations",
  "maintenance", "media", "media-sparse", "sparse-checkout"
)

if (-not (Test-Path $DOCS_DIR)) {
  Write-QaGate $Phase "docs-dir-present" $false "missing $DOCS_DIR"
  Exit-QaPhase $Phase $true
}

function Get-HelpText([string[]]$CmdArgs) {
  $r = Invoke-MG $null (@($CmdArgs) + @("--help")) $Phase -TimeoutSec 60
  if ($r.Exit -ne 0) { return $null }
  return $r.Out
}

function Get-Subcommands([string]$HelpText) {
  # clap prints a `Commands:` block; take the first token of each entry.
  if ($HelpText -notmatch '(?ms)^Commands:\r?\n(.*?)(\r?\n\r?\n|\r?\nOptions:)') { return @() }
  $block = $Matches[1]
  $names = @()
  foreach ($line in ($block -split "`n")) {
    if ($line -match '^\s{2,}([a-z][a-z0-9-]*)\s') { $names += $Matches[1] }
  }
  return ($names | Sort-Object -Unique)
}

$pages = Get-ChildItem $DOCS_DIR -Filter *.md | Sort-Object Name
$totalInvented = 0
$totalUndocumented = 0
$comparedPages = 0
$detail = @()

foreach ($page in $pages) {
  $cmd = [IO.Path]::GetFileNameWithoutExtension($page.Name)
  if ($TOPIC_PAGES -contains $cmd) { continue }

  $help = Get-HelpText @($cmd)
  if (-not $help) {
    # A page for a command that does not exist is itself a finding, and a loud
    # one: it cannot be compared, so it must not pass silently.
    $detail += "page '$cmd' documents a command the binary does not have"
    Write-QaLog $Phase "NO-SUCH-COMMAND: $($page.Name)"
    continue
  }

  $real = @{}
  foreach ($m in [regex]::Matches($help, '--[a-z][a-z0-9-]+')) { $real[$m.Value] = $true }
  # TWO levels, not one.
  #
  # This used to recurse a single level, which made every flag on a
  # sub-subcommand invisible: `auth key create --name`, `auth admin create-user
  # --role` and friends read as INVENTED because the walk stopped at
  # `auth key --help`. Adding the auth page (which is the only two-level command
  # tree in the CLI) is what exposed it, and it cost 3 phantom entries in the
  # baseline.
  #
  # Worth noting what that blind spot really meant: the gate could not see
  # fabrications on a sub-subcommand AT ALL, so the auth surface - the
  # security-relevant one - was the least protected part of the docs.
  #
  # Depth is capped at two deliberately. clap trees here are at most two deep,
  # and each extra level multiplies the `--help` invocations this phase makes.
  foreach ($sub in (Get-Subcommands $help)) {
    $subHelp = Get-HelpText @($cmd, $sub)
    if (-not $subHelp) { continue }
    foreach ($m in [regex]::Matches($subHelp, '--[a-z][a-z0-9-]+')) { $real[$m.Value] = $true }
    foreach ($sub2 in (Get-Subcommands $subHelp)) {
      $sub2Help = Get-HelpText @($cmd, $sub, $sub2)
      if (-not $sub2Help) { continue }
      foreach ($m in [regex]::Matches($sub2Help, '--[a-z][a-z0-9-]+')) { $real[$m.Value] = $true }
    }
  }

  $text = Get-Content $page.FullName -Raw -EA SilentlyContinue
  $documented = @{}
  if ($text) {
    foreach ($m in [regex]::Matches($text, '--[a-z][a-z0-9-]+')) { $documented[$m.Value] = $true }
  }

  $invented = @($documented.Keys | Where-Object { -not $real.ContainsKey($_) } | Sort-Object)
  $undoc = @($real.Keys | Where-Object { -not $documented.ContainsKey($_) } | Sort-Object)

  $comparedPages++
  $totalInvented += $invented.Count
  $totalUndocumented += $undoc.Count
  Write-QaRow $TSV $HEADER @($cmd, $invented.Count, $undoc.Count, ($invented -join " "))
  if ($invented.Count -gt 0) {
    Write-QaLog $Phase "$cmd : $($invented.Count) invented -> $($invented -join ' ')"
  }
}

# A run that compared nothing must not pass. Same rule as every other gate here:
# "measured nothing" and "found nothing" have to be distinguishable.
if ($comparedPages -eq 0) {
  Write-QaGate $Phase "docs-flags-compared" $false `
    "compared 0 pages under $DOCS_DIR - the gate proved nothing"
  Exit-QaPhase $Phase $true
}
Write-QaGate $Phase "docs-flags-compared" $true "pages=$comparedPages"

# Known-real flags carrying `hide = true` never appear in --help, so a small
# residue of false positives is expected and is baked into the baseline rather
# than special-cased per flag - special-casing would rot the moment a flag is
# unhidden.
$baselineCount = $null
if (Test-Path $BASELINE) {
  foreach ($line in (Get-Content $BASELINE | Select-Object -Skip 1)) {
    $c = $line -split "`t"
    if ($c[0] -eq "invented_flags") { $baselineCount = [int]$c[1] }
  }
}

$msg = "invented=$totalInvented undocumented=$totalUndocumented pages=$comparedPages"
if ($null -eq $baselineCount) {
  Write-QaGate $Phase "docs-invented-flag-regression" $false `
    "no readable baseline at $BASELINE ($msg)"
} else {
  if ($totalInvented -lt $baselineCount) {
    $msg += " (DROPPED below baseline $baselineCount - re-lock baselines\docs-invented-flags.tsv to $totalInvented)"
  }
  Write-QaGate $Phase "docs-invented-flag-regression" ($totalInvented -le $baselineCount) `
    "$msg baseline=$baselineCount"
}
if ($detail.Count -gt 0) { foreach ($d in $detail) { Write-QaLog $Phase "FINDING: $d" } }

# ---- env-knob drift ------------------------------------------------------
#
# Everything above compares documented CLI FLAGS against `--help`. NOTHING has
# ever checked env knobs - which is how env-knobs.md (129) and
# book/src/reference/environment.md (101) drifted 28 apart without a single gate
# noticing, and how MEDIAGIT_STORAGE_STREAMING sat documented-and-dead from B7
# until C2.
#
# Same shape as docs-invented-flag-regression above: count-NOT-GROWING against a
# baseline, not zero. The debt is real (10 at the time of writing, several of
# them dynamic prefixes rather than knobs) and a gate demanding zero is a gate
# somebody switches off.
#
# Both halves have to work: this must go RED when a knob is added to the code
# without a row in env-knobs.md, and stay quiet otherwise. To prove it by hand,
# add a `MEDIAGIT_ZZZ_PROBE` reference to any .rs file and re-run this phase.
$KNOB_BASELINE = Join-Path $QA.Root "baselines\docs-undocumented-knobs.tsv"
$KNOB_TSV      = Join-Path $QA.Logs "docs_knobs.tsv"
$knobRx        = [regex]'MEDIAGIT_[A-Z0-9_]+'

$codeKnobs = @{}
Get-ChildItem -Path (Join-Path $QA.RepoRoot "crates") -Filter *.rs -Recurse -File |
  ForEach-Object {
    foreach ($m in $knobRx.Matches([IO.File]::ReadAllText($_.FullName))) {
      $codeKnobs[$m.Value] = $true
    }
  }

function Get-KnobsFromDoc([string]$RelPath) {
  $h = @{}
  $p = Join-Path $QA.RepoRoot $RelPath
  if (Test-Path $p) {
    foreach ($m in $knobRx.Matches([IO.File]::ReadAllText($p))) { $h[$m.Value] = $true }
  }
  return $h
}

$rootDocKnobs = Get-KnobsFromDoc "env-knobs.md"
$bookDocKnobs = Get-KnobsFromDoc "book\src\reference\environment.md"

$undocumented = @($codeKnobs.Keys | Where-Object { -not $rootDocKnobs.ContainsKey($_) } | Sort-Object)
# Reported, not gated - the book trailing the canonical list is an omission, not
# a falsehood, and folding the two counts together would let them move against
# each other invisibly (same reasoning as undocumented-vs-invented flags above).
$bookBehind   = @($rootDocKnobs.Keys | Where-Object { -not $bookDocKnobs.ContainsKey($_) } | Sort-Object)

Set-Content -Path $KNOB_TSV -Value "knob`tstate" -Encoding UTF8
foreach ($k in $undocumented) { Add-Content -Path $KNOB_TSV -Value "$k`tundocumented" }
foreach ($k in $bookBehind)   { Add-Content -Path $KNOB_TSV -Value "$k`tmissing_from_book" }

$knobBaselineCount = $null
if (Test-Path $KNOB_BASELINE) {
  foreach ($line in (Get-Content $KNOB_BASELINE | Select-Object -Skip 1)) {
    $c = $line -split "`t"
    if ($c[0] -eq "undocumented_knobs") { $knobBaselineCount = [int]$c[1] }
  }
}

$knobMsg = "code=$($codeKnobs.Count) documented=$($rootDocKnobs.Count) undocumented=$($undocumented.Count) book_behind=$($bookBehind.Count)"
if ($null -eq $knobBaselineCount) {
  Write-QaGate $Phase "docs-undocumented-knob-regression" $false `
    "no readable baseline at $KNOB_BASELINE ($knobMsg)"
} else {
  if ($undocumented.Count -lt $knobBaselineCount) {
    $knobMsg += " (DROPPED below baseline $knobBaselineCount - re-lock baselines\docs-undocumented-knobs.tsv to $($undocumented.Count))"
  }
  Write-QaGate $Phase "docs-undocumented-knob-regression" ($undocumented.Count -le $knobBaselineCount) `
    "$knobMsg baseline=$knobBaselineCount"
}
foreach ($k in $undocumented) { Write-QaLog $Phase "FINDING: knob in code but not env-knobs.md: $k" }

Write-QaLog $Phase "=== 13_docs done: invented=$totalInvented undocumented=$totalUndocumented undocumented_knobs=$($undocumented.Count) ==="
Exit-QaPhase $Phase $false
