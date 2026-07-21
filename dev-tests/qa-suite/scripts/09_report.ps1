# 09_report.ps1 - aggregation: reads all phase TSVs in $QA.Logs (missing files tolerated),
# emits $QA.Reports\summary.json and $QA.Reports\REPORT.md from templates\REPORT.template.md.
param()
. (Join-Path $PSScriptRoot "lib\common.ps1")
$Phase = "09_report"

Write-QaLog $Phase "aggregation starting for run $($QA.RunId)"

function Read-Tsv([string]$Path) {
  # returns array of PSCustomObjects, or empty array when the file is absent/empty
  if (-not (Test-Path $Path)) { return @() }
  $lines = Get-Content $Path
  if (-not $lines -or $lines.Count -lt 2) { return @() }
  return @($lines | ConvertFrom-Csv -Delimiter "`t")
}
function MdEsc([object]$v) { ("" + $v) -replace '\|', '\|' }

# ---- load everything (persona scenario TSVs live in Reports per the persona scripts) ----
$gates     = Read-Tsv (Join-Path $QA.Logs "gates.tsv")
$economics = Read-Tsv (Join-Path $QA.Logs "economics.tsv")
$remote    = Read-Tsv (Join-Path $QA.Logs "remote_results.tsv")
$matrix    = Read-Tsv (Join-Path $QA.Logs "matrix_results.tsv")
$perfBench = Read-Tsv (Join-Path $QA.Logs "perf-bench.tsv")
$perfTime  = Read-Tsv (Join-Path $QA.Logs "perf.tsv")

# ---- mediagit version ----
$mgVersion = ""
if (Test-Path $QA.MG) {
  $mgVersion = ((& $QA.MG version 2>&1 | Out-String) -replace "`r?`n", " ").Trim()
}

# ---- phases: gate pass/fail counts ----
$phases = [ordered]@{}
foreach ($g in ($gates | Group-Object phase)) {
  $passN = @($g.Group | Where-Object { $_.pass -eq "True" }).Count
  $failN = $g.Count - $passN
  $phases[$g.Name] = @{ gates = @{ pass = $passN; fail = $failN } }
}

# ---- economics: latest savedPct per family (skip SKIP rows) ----
$econSummary = [ordered]@{}
foreach ($g in ($economics | Where-Object { $_.version -ne "SKIP" } | Group-Object family)) {
  $last = $g.Group | Select-Object -Last 1
  $econSummary[$g.Name] = @{ savedPct = [double]$last.savedPct }
}

# ---- remote: backend -> op -> MBps (last row wins) ----
$remoteSummary = [ordered]@{}
foreach ($r in $remote) {
  if (-not $remoteSummary.Contains($r.backend)) { $remoteSummary[$r.backend] = [ordered]@{} }
  if ($r.MBps) { $remoteSummary[$r.backend][$r.op] = [double]$r.MBps }
}

# ---- findings: matrix anomalies (PANIC / ERRTEXT-EXIT0) - orchestrator appends narrative findings later ----
$findings = @()
$fid = 0
foreach ($m in ($matrix | Where-Object { $_.class -in @("PANIC", "ERRTEXT-EXIT0") })) {
  $fid++
  $findings += @{
    id       = "MX-{0:d3}" -f $fid
    severity = $(if ($m.class -eq "PANIC") { "P1" } else { "P2" })
    area     = "cli-matrix"
    summary  = "$($m.cmd) classified $($m.class) (exit=$($m.exit))"
    repro    = "02_matrix.ps1 -Rows $($m.row_id)"
    status   = "open"
  }
}

# ---- summary.json ----
$summary = [ordered]@{
  runId     = $QA.RunId
  tier      = $QA.Tier
  mgVersion = $mgVersion
  timestamp = (Get-Date -Format "yyyy-MM-ddTHH:mm:ss")
  phases    = $phases
  economics = $econSummary
  remote    = $remoteSummary
  perf      = $perfBench   # rows consumed by 08_perf.ps1 -Baseline on the next run
  findings  = $findings
}
$summaryPath = Join-Path $QA.Reports "summary.json"
$summary | ConvertTo-Json -Depth 10 | Set-Content $summaryPath -Encoding ASCII
Write-QaLog $Phase "wrote $summaryPath"

# ---- REPORT.md via token replacement ----
$templatePath = Join-Path $QA.Root "templates\REPORT.template.md"
if (Test-Path $templatePath) {
  # strip the leading token-documentation comment so its token mentions aren't replaced too
  $tpl = (Get-Content -Raw $templatePath) -replace '(?s)^<!--.*?-->\s*', ''
} else {
  # self-contained fallback with the same sections as templates/REPORT.template.md
  $tpl = @"
# MediaGit QA Suite - Report

**Run:** {{RUN_ID}} | **Date:** {{RUN_DATE}} | **Tier:** {{TIER}} | **Build:** {{MG_VERSION}} | **Backends:** {{BACKENDS}}

## 1. Verdict

{{VERDICT}}

## 2. Per-phase gate results

| Phase | Gate | Pass | Detail |
|---|---|---|---|
{{GATE_TABLE_ROWS}}

## 3. Findings registry

| ID | Severity | Area | Summary | Repro | Status |
|---|---|---|---|---|---|
{{FINDINGS_TABLE_ROWS}}

## 4. Storage economics

| Fixture / chain | Raw bytes | Stored bytes | Saved % | Notes |
|---|---|---|---|---|
{{STORAGE_ECONOMICS_ROWS}}

## 5. Remote throughput

| Backend | Operation | Size | Throughput | Latency | Notes |
|---|---|---|---|---|---|
{{REMOTE_THROUGHPUT_ROWS}}

## 6. Performance

| Operation | Wall time | Peak RAM | Notes |
|---|---|---|---|
{{PERF_TABLE_ROWS}}

## 7. Improvement suggestions

{{IMPROVEMENT_SUGGESTIONS}}

## 8. Methodology

{{METHODOLOGY_NOTES}}
"@
}

$gateRows = @($gates | ForEach-Object {
  "| $(MdEsc $_.phase) | $(MdEsc $_.gate) | $(if ($_.pass -eq 'True') { 'PASS' } else { 'FAIL' }) | $(MdEsc $_.detail) |"
})
if (-not $gateRows) { $gateRows = @("| _none_ | | | |") }

$findingRows = @($findings | ForEach-Object {
  "| $($_.id) | $($_.severity) | $($_.area) | $(MdEsc $_.summary) | $(MdEsc $_.repro) | $($_.status) |"
})
if (-not $findingRows) { $findingRows = @("| _none_ | | | | | |") }

$econRows = @($economics | ForEach-Object {
  if ($_.version -eq "SKIP") {
    "| $(MdEsc $_.family) | | | | SKIP - fixtures missing |"
  } else {
    "| $(MdEsc $_.family) $($_.version) | $($_.fileMB) MB | $($_.odbGrowthMB) MB | $($_.savedPct)% | add $($_.addSec)s |"
  }
})
if (-not $econRows) { $econRows = @("| _none_ | | | | |") }

$remoteRows = @($remote | ForEach-Object {
  "| $(MdEsc $_.backend) | $(MdEsc $_.op) | $($_.sizeMB) MB | $($_.MBps) MB/s | $($_.sec)s | $(MdEsc $_.detail) |"
})
if (-not $remoteRows) { $remoteRows = @("| _none_ | | | | | |") }

$perfRows = @($perfTime | ForEach-Object {
  "| $($_.op) ($($_.sizeMB) MB) | $($_.sec)s | n/a | |"
})
if (-not $perfRows) { $perfRows = @("| _none_ | | | |") }

$failTotal = @($gates | Where-Object { $_.pass -ne "True" }).Count
$verdict =
  if ($failTotal -gt 0) { "FAIL - $failTotal gate(s) failed (see section 2)." }
  elseif ($findings.Count -gt 0) { "PASS-WITH-FINDINGS - all gates green, $($findings.Count) matrix finding(s) registered (see section 3)." }
  else { "PASS - all gates green, no findings." }

$backends = if ($remoteSummary.Count -gt 0) { ($remoteSummary.Keys -join ",") } else { ($QA.Backends -join ",") }

# {{IMPROVEMENT_SUGGESTIONS}} and {{METHODOLOGY_NOTES}} are deliberately NOT replaced:
# the orchestrator fills the narrative sections.
$report = $tpl `
  -replace '\{\{RUN_ID\}\}', $QA.RunId `
  -replace '\{\{RUN_DATE\}\}', (Get-Date -Format "yyyy-MM-dd HH:mm") `
  -replace '\{\{TIER\}\}', $QA.Tier `
  -replace '\{\{MG_VERSION\}\}', $mgVersion `
  -replace '\{\{BACKENDS\}\}', $backends `
  -replace '\{\{VERDICT\}\}', $verdict `
  -replace '\{\{GATE_TABLE_ROWS\}\}', ($gateRows -join "`n") `
  -replace '\{\{FINDINGS_TABLE_ROWS\}\}', ($findingRows -join "`n") `
  -replace '\{\{STORAGE_ECONOMICS_ROWS\}\}', ($econRows -join "`n") `
  -replace '\{\{REMOTE_THROUGHPUT_ROWS\}\}', ($remoteRows -join "`n") `
  -replace '\{\{PERF_TABLE_ROWS\}\}', ($perfRows -join "`n")
$reportPath = Join-Path $QA.Reports "REPORT.md"
$report | Set-Content $reportPath -Encoding ASCII
Write-QaLog $Phase "wrote $reportPath"

# ---- gate: summary.json must round-trip as valid JSON ----
$jsonOk = $false
try {
  $rt = Get-Content -Raw $summaryPath | ConvertFrom-Json
  $jsonOk = ($null -ne $rt) -and ($rt.runId -eq $QA.RunId)
} catch { $jsonOk = $false }
Write-QaGate $Phase "summary-json-valid" $jsonOk $summaryPath

if ($jsonOk) { exit 0 } else { exit 1 }
