# compare_dedup.ps1 - dedup regression gate for the smart-media-handling work.
#
# Compares a fresh dedup_report run against the locked baseline
# (dev-tests/dedup-baseline.json). Fails (exit 1) with a table of offenders
# if, for ANY extension:
#   - dedup_pct drops by more than 0.5 percentage points, or
#   - the stored-bytes/input-bytes ratio grows by more than 0.5%.
#
# add_ms is reported but NOT gated: it is a wall clock on whatever machine
# ran the report, and this file is compared across weeks.
# The `generated` timestamp field is ignored.
#
# Usage: pwsh dev-tests/compare_dedup.ps1 -Baseline dev-tests/dedup-baseline.json -Current fresh.json

param(
    [Parameter(Mandatory = $true)][string]$Baseline,
    [Parameter(Mandatory = $true)][string]$Current
)

$ErrorActionPreference = 'Stop'

$base = Get-Content -Raw $Baseline | ConvertFrom-Json
$curr = Get-Content -Raw $Current | ConvertFrom-Json

if ($base.schema_version -ne $curr.schema_version) {
    Write-Host "FAIL: schema_version mismatch (baseline $($base.schema_version), current $($curr.schema_version))"
    exit 1
}

$offenders = @()

foreach ($prop in $base.per_extension.PSObject.Properties) {
    $ext = $prop.Name
    $b = $prop.Value
    $c = $curr.per_extension.$ext
    if ($null -eq $c) {
        $offenders += [pscustomobject]@{ Extension = $ext; Metric = 'presence'; Baseline = 'present'; Current = 'MISSING'; Delta = '' }
        continue
    }

    $dedupDrop = $b.dedup_pct - $c.dedup_pct
    if ($dedupDrop -gt 0.5) {
        $offenders += [pscustomobject]@{
            Extension = $ext; Metric = 'dedup_pct'
            Baseline = [math]::Round($b.dedup_pct, 3); Current = [math]::Round($c.dedup_pct, 3)
            Delta = "-$([math]::Round($dedupDrop, 3))pp"
        }
    }

    # Compared as a RATIO of stored bytes to input bytes, not as absolute bytes.
    #
    # An absolute-bytes threshold cannot survive a corpus change: adding v1/v2
    # fixture pairs for psd/mkv/mov (2026-07-28) tripled the psd input, so its
    # stored bytes rose 5.3% and the gate called it a regression -- while its
    # dedup went 0% -> 64.3%, i.e. the pipeline had just done its job
    # exceptionally well. The ratio is what "did compression get worse" actually
    # means, and it is stable when only the corpus moves.
    if ($b.post_compression_bytes -gt 0 -and $b.total_bytes -gt 0 -and $c.total_bytes -gt 0) {
        $baseRatio = $b.post_compression_bytes / $b.total_bytes
        $currRatio = $c.post_compression_bytes / $c.total_bytes
        $growPct = 100.0 * ($currRatio - $baseRatio) / $baseRatio
        if ($growPct -gt 0.5) {
            $offenders += [pscustomobject]@{
                Extension = $ext; Metric = 'stored_bytes_ratio'
                Baseline = [math]::Round($baseRatio, 5); Current = [math]::Round($currRatio, 5)
                Delta = "+$([math]::Round($growPct, 3))%"
            }
        }
    }
}

if ($offenders.Count -gt 0) {
    Write-Host "FAIL: $($offenders.Count) regression(s) vs baseline:"
    $offenders | Format-Table -AutoSize | Out-String | Write-Host
    exit 1
}

$extCount = @($base.per_extension.PSObject.Properties).Count
Write-Host ("PASS: {0} extensions within thresholds (dedup_pct drop <=0.5pp, stored/input ratio growth <=0.5%)" -f $extCount)
Write-Host ("  totals: dedup_pct={0:N2}  post_compression_bytes={1:N0}  add_ms={2:N0} (informational)" -f $curr.totals.dedup_pct, $curr.totals.post_compression_bytes, $curr.totals.add_ms)
exit 0
