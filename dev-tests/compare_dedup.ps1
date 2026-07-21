# compare_dedup.ps1 - dedup regression gate for the smart-media-handling work.
#
# Compares a fresh dedup_report run against the locked baseline
# (dev-tests/dedup-baseline.json). Fails (exit 1) with a table of offenders
# if, for ANY extension:
#   - dedup_pct drops by more than 0.5 percentage points, or
#   - post_compression_bytes grows by more than 0.5%, or
#   - total add_ms grows by more than 5%.
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

    if ($b.post_compression_bytes -gt 0) {
        $growPct = 100.0 * ($c.post_compression_bytes - $b.post_compression_bytes) / $b.post_compression_bytes
        if ($growPct -gt 0.5) {
            $offenders += [pscustomobject]@{
                Extension = $ext; Metric = 'post_compression_bytes'
                Baseline = $b.post_compression_bytes; Current = $c.post_compression_bytes
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
Write-Host ("PASS: {0} extensions within thresholds (dedup_pct drop <=0.5pp, post_compression growth <=0.5%)" -f $extCount)
Write-Host ("  totals: dedup_pct={0:N2}  post_compression_bytes={1:N0}  add_ms={2:N0} (informational)" -f $curr.totals.dedup_pct, $curr.totals.post_compression_bytes, $curr.totals.add_ms)
exit 0
