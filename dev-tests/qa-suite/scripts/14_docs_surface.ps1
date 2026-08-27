# 14_docs_surface - do the docs name things that exist?
#
# WHY THIS EXISTS, GIVEN 13_docs ALREADY RUNS.
#
# 13_docs compares flag tables against `--help` for the 29 pages under
# book\src\cli. That is a real gate and it works. Its blind spot is everything
# else: the root docs, the guides, the architecture pages, the installation
# pages, the crate READMEs. On 2026-08-26 a manual sweep of exactly that
# territory found 41 fabrications - 21 environment variables and 20 CLI flags
# that exist in no code path - while 13_docs was green over all of it the whole
# time.
#
# What made them expensive was not that they were wrong, but that they were
# plausible and load-bearing:
#   - Every credential example told readers to export AWS_ACCESS_KEY_ID and
#     friends. MediaGit reads no such variable; the CI/CD workflow copied
#     verbatim out of the docs fails with "access key cannot be empty".
#   - `[storage] encryption = true` was documented with a defaults table. There
#     is no such field and no SSE code, and because the client config is not
#     deny_unknown_fields it parses silently. A security setting that reads as
#     configured and does nothing.
#   - book\src\guides\delta-compression.md documented three TOML tables, six
#     commands and a chain-depth model that the enforced MAX_DELTA_DEPTH of 10
#     makes impossible.
#
# So this gate asks one question of the docs 13_docs cannot see: does every
# MEDIAGIT_* variable and every `mediagit <cmd> --flag` you name actually
# exist?
#
# RATCHET, NOT A CLIFF. Both counts are compared against a baseline that may
# only go down, matching 13_docs' own convention. A cleanup lowers the
# baseline in the same commit; a regression fails. Starting the gate at 0/0
# only works because the sweep above already drove it there.

param()
. (Join-Path $PSScriptRoot "lib\common.ps1")

$PHASE = "14_docs_surface"
$repo  = Split-Path (Split-Path (Split-Path $PSScriptRoot -Parent) -Parent) -Parent
$baselineFile = Join-Path (Split-Path $PSScriptRoot -Parent) "baselines\docs-surface.tsv"

Write-QaLog $PHASE "repo=$repo"

# ---------------------------------------------------------------- inputs
Push-Location $repo
$tracked = @(& git ls-files "*.md") | Where-Object { $_ }
Pop-Location
Write-QaLog $PHASE "tracked .md files: $($tracked.Count)"

# book\src\cli is 13_docs' territory; do not double-report it here.
$docs = $tracked | Where-Object { $_ -notmatch '^book/src/cli/' }

# ---------------------------------------------------------------- known env vars
# Source of truth is the code, plus the harness and CI, which legitimately
# define their own MG_/MEDIAGIT_ knobs that docs may reference.
Push-Location $repo
$known = New-Object System.Collections.Generic.HashSet[string]
foreach ($scope in @("crates", "dev-tests", ".github")) {
  if (-not (Test-Path $scope)) { continue }
  Get-ChildItem -Path $scope -Recurse -File -EA SilentlyContinue |
    Where-Object { $_.Extension -in ".rs", ".ps1", ".yml", ".yaml", ".toml" } |
    ForEach-Object {
      foreach ($m in [regex]::Matches((Get-Content $_.FullName -Raw -EA SilentlyContinue), 'MEDIAGIT_[A-Z0-9_]+')) {
        $null = $known.Add($m.Value)
      }
    }
}
Pop-Location
Write-QaLog $PHASE "MEDIAGIT_* names defined in source/harness/CI: $($known.Count)"

# A doc that says "there is no MEDIAGIT_FOO" is doing the right thing and must
# not be punished for naming it.
#
# Checked over a WINDOW of lines, not one line. Prose wraps: CLOUD_ARCHITECTURE.md
# names three variables on one line and says "appeared in earlier revisions of
# this document and have never been read by anything" on the next two. A
# per-line test reported all three as inventions - the gate's first run was six
# findings and all six were false positives, which is precisely the shape that
# gets a gate switched off.
#
# +/-2 lines covers a wrapped sentence without swallowing a whole section, so an
# accurate paragraph and a fabricating one can still coexist in one file.
$retraction = 'there (is|are) no|never (existed|been read|done anything|read)|do(es)? not exist|no such|not implemented|has no effect|have no effect|no caller|inert|removed|retracted|appeared in earlier|earlier revisions|not supported|REFUSE'
$RETRACT_WINDOW = 2

function Test-QaRetracted($Lines, $Idx, $Window, $Pattern) {
  $lo = [Math]::Max(0, $Idx - $Window)
  $hi = [Math]::Min($Lines.Count - 1, $Idx + $Window)
  for ($i = $lo; $i -le $hi; $i++) { if ($Lines[$i] -match $Pattern) { return $true } }
  return $false
}

$envRows = @()
foreach ($f in $docs) {
  $full = Join-Path $repo $f
  if (-not (Test-Path $full)) { continue }
  $lines = @(Get-Content $full -EA SilentlyContinue)
  # $li, not $i: the flag scan below nests a `for ($i = ...)` over command
  # prefixes, and sharing the name let the inner loop drive the outer one back
  # to zero every iteration - the scan never terminated and burned 611s of CPU
  # before it was killed. Distinct names in both loops, permanently.
  for ($li = 0; $li -lt $lines.Count; $li++) {
    $line = $lines[$li]
    if (Test-QaRetracted $lines $li $RETRACT_WINDOW $retraction) { continue }
    # `${{ vars.MEDIAGIT_S3_BUCKET }}` is a GitHub Actions repository variable,
    # named by the workflow author, not a MediaGit env var. Strip those spans
    # before matching or every CI example becomes a finding.
    $scan = [regex]::Replace($line, '\$\{\{[^}]*\}\}', ' ')
    foreach ($m in [regex]::Matches($scan, 'MEDIAGIT_[A-Z0-9_]+\*?')) {
      $name = $m.Value
      # `MEDIAGIT_APP_*` / `MEDIAGIT_COMPRESSION_*` name a FAMILY, not a
      # variable. The regex otherwise captures the stem without its asterisk
      # and reports a name nobody wrote.
      if ($name.EndsWith('*') -or $name.EndsWith('_')) { continue }
      if (-not $known.Contains($name)) {
        $envRows += [pscustomobject]@{ Kind = "env"; Name = $name; File = $f; Line = ($li + 1) }
      }
    }
  }
}

# ---------------------------------------------------------------- known CLI flags
# `--help` is the source of truth, resolved per command path so a subcommand's
# own flags count (auth key create --name lives on `create`, not on `auth`).
# HIDDEN FLAGS ARE STILL REAL FLAGS.
#
# `--help` is the primary source of truth but not a complete one: clap's
# `hide = true` keeps a working flag out of the listing. `reset --mixed`,
# `cherry-pick --continue` and `show --stat` are all real, all accepted by the
# binary, and all absent from `--help`. A gate that trusted `--help` alone
# would report them as invented forever, and the only way to silence it would
# be to inflate the baseline - which is how a ratchet quietly stops ratcheting.
#
# So take a second reading from the clap derives themselves: an explicit
# `long = "x"`, an `alias = "x"`, or a `pub x_y: bool` field that clap
# kebab-cases into `--x-y`. Union of the two readings is the accepted set.
$declared = New-Object System.Collections.Generic.HashSet[string]
$cliSrc = Join-Path $repo "crates\mediagit-cli\src"
if (Test-Path $cliSrc) {
  Get-ChildItem -Path $cliSrc -Recurse -File -Filter *.rs -EA SilentlyContinue | ForEach-Object {
    $text = Get-Content $_.FullName -Raw -EA SilentlyContinue
    foreach ($m in [regex]::Matches($text, '(?:long|alias)\s*=\s*"([a-zA-Z][\w-]*)"')) {
      $null = $declared.Add("--" + $m.Groups[1].Value)
    }
    foreach ($m in [regex]::Matches($text, 'pub\s+([a-z][a-z0-9_]*)\s*:')) {
      $null = $declared.Add("--" + ($m.Groups[1].Value -replace '_', '-'))
    }
  }
}
Write-QaLog $PHASE "flag names declared in the CLI clap derives: $($declared.Count)"

$helpCache = @{}
function Get-Help-Text([string[]]$Parts) {
  $key = ($Parts -join " ")
  if (-not $helpCache.ContainsKey($key)) {
    $out = & $QA.MG @Parts --help 2>&1 | Out-String
    $helpCache[$key] = if ($LASTEXITCODE -eq 0) { $out } else { $null }
  }
  return $helpCache[$key]
}

$flagRows = @()
foreach ($f in $docs) {
  $full = Join-Path $repo $f
  if (-not (Test-Path $full)) { continue }
  $lines = @(Get-Content $full -EA SilentlyContinue)
  for ($li = 0; $li -lt $lines.Count; $li++) {
    $line = $lines[$li]
    $lineNo = $li + 1
    if (Test-QaRetracted $lines $li $RETRACT_WINDOW $retraction) { continue }
    # One line at a time. A regex allowed to cross newlines attributes a flag
    # from line 3 to the command on line 1, which produced 23 false positives
    # on the first run of this check.
    foreach ($m in [regex]::Matches($line, 'mediagit\s+([a-z][a-z-]*)((?:\s+[a-z][a-z-]*){0,2})([^\n]*)')) {
      $words = @($m.Groups[1].Value) + @($m.Groups[2].Value -split '\s+' | Where-Object { $_ })
      $rest  = $m.Groups[3].Value
      # A mediagit invocation ENDS at a shell metacharacter. In
      #   mediagit completions bash > $(brew --prefix)/etc/...
      # the `--prefix` belongs to brew, and attributing it to `mediagit
      # completions` reported a flag nobody claimed. Cut at the first pipe,
      # redirect, command substitution, separator or comment.
      $cut = [regex]::Match($rest, '\||>|<|\$\(|&&|;|`|#')
      if ($cut.Success) { $rest = $rest.Substring(0, $cut.Index) }
      foreach ($fm in [regex]::Matches($rest, '(?<![\w-])(--[a-zA-Z][\w-]*)')) {
        $flag = $fm.Groups[1].Value
        $real = $false; $found = $false
        # Accept the flag if ANY prefix of the command path documents it: this
        # is what makes `mediagit lock create <p>` on the same line as
        # `lock unlock --id` not report --id as invented.
        for ($i = $words.Count; $i -ge 1; $i--) {
          $h = Get-Help-Text $words[0..($i-1)]
          if ($null -eq $h) { continue }
          $real = $true
          if ($h -like "*$flag*") { $found = $true; break }
        }
        # Second reading: accepted anywhere in the CLI's own derives.
        if (-not $found -and $declared.Contains($flag)) { $found = $true }
        # $real stays false for prose that merely looks like a command
        # ("and mediagit run --release"); those are not findings.
        if ($real -and -not $found) {
          $flagRows += [pscustomobject]@{ Kind = "flag"; Name = "$($words -join ' ') $flag"; File = $f; Line = $lineNo }
        }
      }
    }
  }
}

# ---------------------------------------------------------------- report
$detail = Join-Path $QA.Logs "docs_surface.tsv"
$all = @($envRows) + @($flagRows)
foreach ($r in $all) { Write-QaRow $detail @("kind","name","file","line") @($r.Kind, $r.Name, $r.File, $r.Line) }
foreach ($r in $all) { Write-QaLog $PHASE ("INVENTED {0,-5} {1,-46} {2}:{3}" -f $r.Kind, $r.Name, $r.File, $r.Line) }

$envN  = @($envRows).Count
$flagN = @($flagRows).Count
Write-QaLog $PHASE "scanned $($docs.Count) docs outside book\src\cli: invented-env=$envN invented-flags=$flagN"

# ---------------------------------------------------------------- ratchet
$baseEnv = 0; $baseFlag = 0
if (Test-Path $baselineFile) {
  foreach ($l in (Get-Content $baselineFile | Select-Object -Skip 1)) {
    $p = $l -split "`t"
    if ($p[0] -eq "env")  { $baseEnv  = [int]$p[1] }
    if ($p[0] -eq "flag") { $baseFlag = [int]$p[1] }
  }
} else {
  Write-QaLog $PHASE "no baseline at $baselineFile - treating as 0/0"
}

$ok = ($envN -le $baseEnv) -and ($flagN -le $baseFlag)
Write-QaGate $PHASE "docs-surface-no-invented-names" $ok `
  "env=$envN/<=$baseEnv flags=$flagN/<=$baseFlag detail=$detail"

if ($ok -and (($envN -lt $baseEnv) -or ($flagN -lt $baseFlag))) {
  Write-QaLog $PHASE "IMPROVED - re-lock the baseline in the same commit: env=$envN flag=$flagN"
}

Exit-QaPhase $PHASE $false
