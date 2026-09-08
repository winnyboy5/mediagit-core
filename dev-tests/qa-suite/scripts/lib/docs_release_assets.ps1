# Do the docs point at release assets that will actually exist?
#
# WHY THIS IS SEPARATE FROM EVERY OTHER DOCS CHECK. 13_docs compares documented
# CLI flags against `--help` and documented config keys against the schema. Both
# ask "does this name exist in the code?". A download URL is not in the code at
# all - it is a promise about what the RELEASE WORKFLOW will publish - so no
# existing gate looks at one, and none ever has.
#
# The failure is silent in the worst possible place. The version is hardcoded
# 131 times across the tracked docs. A release that bumps Cargo.toml and misses
# one leaves a page telling users to download an artifact for the PREVIOUS
# version; GitHub answers 404 and the campaign stays green. The first person to
# find out is someone installing the product for the first time.
#
# TWO checks, because there are two ways to be wrong and only one of them is
# fixed by a version bump:
#
#   1. STALE PIN  - right filename shape, wrong version. Caused by a partial
#                   bump. Every bump is 131 individual edits, and 111 of those
#                   occurrences are HISTORICAL ("deleted in 0.3.0-rc.4", the
#                   `Since` column in env-knobs.md) and must NOT move - so it is
#                   not a find-and-replace, it is 131 judgment calls. Scoping
#                   this check to download contexts sidesteps that entirely:
#                   historical prose does not contain download URLs.
#
#   2. PHANTOM ASSET - a filename the release workflow never produces, for any
#                   version. Found on 2026-09-08: linux-x64.md told Debian users
#                   to `wget` a `.deb` and RHEL users an `.rpm`, and
#                   .github/workflows/release.yml builds NEITHER - zero matches
#                   for deb/rpm/cargo-deb/nfpm anywhere in .github/workflows.
#                   Those URLs 404 today and a version bump would never fix
#                   them. A pin check alone would have called them correct.
#
# The asset list is READ FROM release.yml's `archive-name:` lines, never
# hardcoded here: the moment this file carries its own copy of the list, it can
# agree with itself while disagreeing with what actually ships.

function Get-ReleaseAssetPatterns([string]$RepoRoot) {
  # `archive-name: mediagit-${{ ... }}-x86_64-linux.tar.gz` -> the suffix after
  # the version, e.g. `-x86_64-linux.tar.gz`. Version-agnostic on purpose; the
  # version itself is checked separately.
  $wf = Join-Path $RepoRoot ".github\workflows\release.yml"
  if (-not (Test-Path $wf)) { return @() }
  $out = @()
  foreach ($m in ([regex]'archive-name:\s*mediagit-\$\{\{[^}]*\}\}(?<suffix>[-.][^\s]+)').Matches([IO.File]::ReadAllText($wf))) {
    $out += $m.Groups['suffix'].Value
  }
  return ($out | Sort-Object -Unique)
}

function Test-QaDocsReleaseAssets {
  param([string]$Phase, [string]$RepoRoot, [string]$TsvPath)

  $version = $null
  $cargo = Join-Path $RepoRoot "Cargo.toml"
  if (Test-Path $cargo) {
    foreach ($line in (Get-Content $cargo)) {
      if ($line -match '^\s*version\s*=\s*"([^"]+)"') { $version = $Matches[1]; break }
    }
  }
  $assets = Get-ReleaseAssetPatterns $RepoRoot

  # Anti-vacuity. Without the version or the asset list this inspects nothing
  # and would pass on any input - the shape this suite has now found ten times.
  if (-not $version -or $assets.Count -eq 0) {
    Write-QaGate $Phase "docs-release-assets-exist" $false `
      "version='$version' assets=$($assets.Count) - could not read Cargo.toml or release.yml; the gate proved nothing"
    return
  }

  "doc`tline`tproblem`tdetail" | Set-Content -Path $TsvPath -Encoding UTF8

  Push-Location $RepoRoot
  $tracked = @(& git ls-files "*.md" 2>$null)
  Pop-Location

  $findings = @()
  # A line is a DOWNLOAD context if it names the releases endpoint, the
  # container registry, or an artifact filename. Prose mentioning an old version
  # is untouched by design.
  $dlRx  = [regex]'releases/download/|ghcr\.io/|mediagit[-_][0-9]+\.[0-9]+\.[0-9]+'
  $verRx = [regex]'[0-9]+\.[0-9]+\.[0-9]+(?:-[a-z]+\.[0-9]+)?'

  foreach ($rel in $tracked) {
    $full = Join-Path $RepoRoot $rel
    if (-not (Test-Path $full)) { continue }
    $n = 0
    foreach ($line in (Get-Content $full -EA SilentlyContinue)) {
      $n++
      if (-not $dlRx.IsMatch($line)) { continue }

      # 1. Stale pin.
      foreach ($v in $verRx.Matches($line)) {
        if ($v.Value -ne $version) {
          $findings += "$rel`t$n`tstale-version`t$($v.Value) should be $version"
        }
      }

      # 2. Phantom asset: an artifact filename whose suffix release.yml never
      # produces. Docker tags and bare URLs carry no artifact name, so they are
      # only subject to check 1.
      foreach ($a in ([regex]'mediagit[-_][0-9][^\s"`)]*').Matches($line)) {
        $name = $a.Value
        if ($name -notmatch '\.(tar\.gz|tgz|zip|deb|rpm|msi|pkg|exe)$') { continue }
        $suffix = $name -replace '^mediagit[-_]', '' -replace '^[0-9]+\.[0-9]+\.[0-9]+(-[a-z]+\.[0-9]+)?', ''
        if (-not ($assets | Where-Object { $suffix -eq $_ })) {
          $findings += "$rel`t$n`tphantom-asset`t$name - release.yml publishes only: $($assets -join ', ')"
        }
      }
    }
  }

  foreach ($f in $findings) { Add-Content -Path $TsvPath -Value $f -Encoding UTF8 }
  foreach ($f in $findings) { Write-QaLog $Phase ("FINDING: " + ($f -replace "`t", " ")) }

  $stale   = @($findings | Where-Object { $_ -match "`tstale-version`t" }).Count
  $phantom = @($findings | Where-Object { $_ -match "`tphantom-asset`t" }).Count
  Write-QaGate $Phase "docs-release-assets-exist" ($findings.Count -eq 0) `
    "version=$version assets=$($assets.Count) docs=$($tracked.Count) stale=$stale phantom=$phantom"
}
