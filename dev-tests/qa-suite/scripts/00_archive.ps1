# Phase 00: archive obsolete dev-tests content (v11 campaign artifacts + misc one-offs).
# Idempotent: "done" for each row reflects end-state (source gone / dest present), so reruns
# and -DryRun previews both just report what still needs doing.
#
# Usage:
#   00_archive.ps1 -DryRun    # writes archive-manifest.tsv, touches nothing
#   00_archive.ps1            # performs the moves/deletes, then writes archive-manifest.tsv
param(
  [switch]$DryRun
)

. (Join-Path $PSScriptRoot "lib\common.ps1")

$Phase = "00_archive"
$Base       = Join-Path $QA.RepoRoot "dev-tests"
$V11        = Join-Path $Base "standalone-deep-v11"
$ArchiveV11 = Join-Path $Base "archive\standalone-deep-v11"
$ArchiveMisc = Join-Path $Base "archive\misc"

Write-QaLog $Phase ("mode={0} base={1}" -f $(if ($DryRun) { "DRY-RUN" } else { "REAL" }), $Base)

function Get-SizeMB([string]$Path) {
  if (-not (Test-Path $Path)) { return 0 }
  $item = Get-Item -Force $Path
  if ($item.PSIsContainer) { return Get-DirMB $Path } else { return [math]::Round($item.Length / 1MB, 2) }
}

$moves = @(
  @{ Source = (Join-Path $V11 "REPORT.md");                      Dest = (Join-Path $ArchiveV11 "REPORT.md") },
  @{ Source = (Join-Path $V11 "logs\findings-registry.md");       Dest = (Join-Path $ArchiveV11 "findings-registry.md") },
  @{ Source = (Join-Path $V11 "repros");                          Dest = (Join-Path $ArchiveV11 "repros") },
  @{ Source = (Join-Path $V11 "dashboard.html");                  Dest = (Join-Path $ArchiveV11 "dashboard.html") },
  @{ Source = (Join-Path $V11 "scripts");                         Dest = (Join-Path $ArchiveV11 "scripts") },
  @{ Source = (Join-Path $V11 "manifest-testfiles.tsv");          Dest = (Join-Path $ArchiveV11 "manifest-testfiles.tsv") },
  @{ Source = (Join-Path $Base "m1-baseline");                    Dest = (Join-Path $ArchiveMisc "m1-baseline") },
  @{ Source = (Join-Path $Base "diag_clone_test.ps1");            Dest = (Join-Path $ArchiveMisc "diag_clone_test.ps1") },
  @{ Source = (Join-Path $Base "deep_test_report.md");            Dest = (Join-Path $ArchiveMisc "deep_test_report.md") },
  @{ Source = (Join-Path $Base "archive_reports.ps1");            Dest = (Join-Path $ArchiveMisc "archive_reports.ps1") }
)

# DELETE order matters: "logs" must run after findings-registry.md has been moved out of it (see $moves above).
$deletes = @(
  (Join-Path $V11 "repos"),
  (Join-Path $V11 "fixtures-designer"),
  (Join-Path $V11 "fixtures-synthetic"),
  (Join-Path $V11 ".venv"),
  (Join-Path $V11 "server-b"),
  (Join-Path $V11 "server-p2b"),
  (Join-Path $V11 "logs"),
  (Join-Path $Base "dev-client")
)

$manifestPath = Join-Path $QA.Logs "archive-manifest.tsv"
$header = @("action", "source", "dest", "sizeMB", "done")
$allDone = $true

foreach ($m in $moves) {
  $src = $m.Source
  $dst = $m.Dest
  $sizeMB = Get-SizeMB $src
  if ($sizeMB -eq 0) { $sizeMB = Get-SizeMB $dst }  # already-moved case: source is gone

  if (-not $DryRun) {
    $done = (-not (Test-Path $src)) -and (Test-Path $dst)
    if (-not $done) {
      try {
        if (Test-Path $src) {
          $destParent = Split-Path $dst -Parent
          if (-not (Test-Path $destParent)) { New-Item -ItemType Directory -Path $destParent -Force | Out-Null }
          Move-Item -Path $src -Destination $dst -Force
        }
      } catch {
        Write-QaLog $Phase ("MOVE FAILED: {0} -> {1} : {2}" -f $src, $dst, $_.Exception.Message)
      }
    }
  }

  $done = (-not (Test-Path $src)) -and (Test-Path $dst)
  if (-not $done) { $allDone = $false }
  Write-QaRow $manifestPath $header @("MOVE", $src, $dst, $sizeMB, $done)
}

foreach ($src in $deletes) {
  $sizeMB = Get-SizeMB $src

  if (-not $DryRun) {
    if (Test-Path $src) {
      try {
        Remove-Item -Recurse -Force $src
      } catch {
        Write-QaLog $Phase ("DELETE FAILED: {0} : {1}" -f $src, $_.Exception.Message)
      }
    }
  }

  $done = -not (Test-Path $src)
  if (-not $done) { $allDone = $false }
  Write-QaRow $manifestPath $header @("DELETE", $src, "", $sizeMB, $done)
}

# Best-effort cleanup of the now-hopefully-empty parent dir. Informational only (not gated):
# a stray directory we intentionally never touch (e.g. .claude/) can legitimately keep it non-empty.
if (-not $DryRun -and (Test-Path $V11)) {
  $remaining = @(Get-ChildItem -Force $V11 -EA SilentlyContinue)
  if ($remaining.Count -eq 0) {
    Remove-Item -Force $V11
    Write-QaRow $manifestPath $header @("RMDIR", $V11, "", 0, $true)
  } else {
    Write-QaRow $manifestPath $header @("RMDIR", $V11, "", 0, $false)
    Write-QaLog $Phase ("standalone-deep-v11 not empty, left in place: {0}" -f (($remaining | ForEach-Object { $_.Name }) -join ", "))
  }
}

Write-QaGate $Phase "all-archive-actions-done" $allDone ("manifest=" + $manifestPath)
Write-QaLog $Phase ("manifest written: {0}" -f $manifestPath)

if ($allDone) { exit 0 } else { exit 1 }
