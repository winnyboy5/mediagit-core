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
# Phases whose drill-level detail used to be written and then never read by the report:
# their gates reached gates.tsv, but the per-drill numbers (chain depths, throughput,
# dedup percentages, auth drill outcomes) were invisible in the artifact humans read.
$scale     = Read-Tsv (Join-Path $QA.Logs "scale_results.tsv")
$branching = Read-Tsv (Join-Path $QA.Logs "branching_results.tsv")
$abuse     = Read-Tsv (Join-Path $QA.Logs "abuse_results.tsv")
$authRes   = Read-Tsv (Join-Path $QA.Logs "auth_results.tsv")
$cliAuth   = Read-Tsv (Join-Path $QA.Logs "cli_auth_results.tsv")
$credsRes  = Read-Tsv (Join-Path $QA.Logs "creds_results.tsv")
$setupRes  = Read-Tsv (Join-Path $QA.Logs "setup_results.tsv")
$usersRes  = Read-Tsv (Join-Path $QA.Logs "users_results.tsv")

# ---- mediagit version ----
$mgVersion = ""
if (Test-Path $QA.MG) {
  $mgVersion = ((& $QA.MG version 2>&1 | Out-String) -replace "`r?`n", " ").Trim()
}

# ---- phases: gate verdict counts ----
# skip/warn are counted SEPARATELY from fail. Folding them into either bucket is how a
# campaign that skipped half its drills came to read as fully passed.
$phases = [ordered]@{}
foreach ($g in ($gates | Group-Object phase)) {
  $passN = @($g.Group | Where-Object { $_.pass -eq "True" }).Count
  $failN = @($g.Group | Where-Object { $_.pass -eq "False" }).Count
  $skipN = @($g.Group | Where-Object { $_.pass -eq "SKIP" }).Count
  $warnN = @($g.Group | Where-Object { $_.pass -eq "WARN" }).Count
  $phases[$g.Name] = @{ gates = @{ pass = $passN; fail = $failN; skip = $skipN; warn = $warnN } }
}
$totalPass = @($gates | Where-Object { $_.pass -eq "True" }).Count
$totalFail = @($gates | Where-Object { $_.pass -eq "False" }).Count
$totalSkip = @($gates | Where-Object { $_.pass -eq "SKIP" }).Count
$totalWarn = @($gates | Where-Object { $_.pass -eq "WARN" }).Count

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
  gateTotals = @{ pass = $totalPass; fail = $totalFail; skip = $totalSkip; warn = $totalWarn }
  economics = $econSummary
  remote    = $remoteSummary
  perf      = $perfBench   # rows consumed by 08_perf.ps1 -Baseline on the next run
  scale     = $scale
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

function GateLabel([string]$v) {
  switch ($v) { "True" { "PASS" } "SKIP" { "SKIP" } "WARN" { "WARN" } default { "FAIL" } }
}
$gateRows = @($gates | ForEach-Object {
  "| $(MdEsc $_.phase) | $(MdEsc $_.gate) | $(GateLabel $_.pass) | $(MdEsc $_.detail) |"
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
    "| $(MdEsc $_.family) $($_.version) | $($_.fileMB) MB | $($_.odbGrowthMB) MB | $($_.savedPct)% | add $($_.addSec)s / commit $($_.commitSec)s |"
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

# Verdict. Skips are called out explicitly instead of being averaged into a pass:
# "24 passed" and "24 passed, 30 skipped" are very different campaigns and the
# summary line has to be able to say so.
$counts = "pass=$totalPass fail=$totalFail skip=$totalSkip warn=$totalWarn"
$verdict =
  if ($totalFail -gt 0) { "FAIL - $totalFail gate(s) failed ($counts) - see section 2." }
  elseif ($totalPass -eq 0) { "NOTHING-VERIFIED - no gate passed ($counts); this run proves nothing." }
  elseif ($totalSkip -gt 0 -and $findings.Count -gt 0) { "PASS-WITH-SKIPS-AND-FINDINGS - $counts, $($findings.Count) matrix finding(s) - see sections 2 and 3." }
  elseif ($totalSkip -gt 0) { "PASS-WITH-SKIPS - $counts; $totalSkip gate(s) were NOT checked - see section 2." }
  elseif ($findings.Count -gt 0) { "PASS-WITH-FINDINGS - all gates green ($counts), $($findings.Count) matrix finding(s) - see section 3." }
  else { "PASS - all gates green ($counts), no findings." }
Write-QaLog $Phase "verdict: $verdict"

# ---- SCALE section (present only when phase 10 ran) ----
$scaleSection = ""
if ($scale.Count -gt 0) {
  $scaleRows = @($scale | ForEach-Object {
    "| $(MdEsc $_.drill) | $(MdEsc $_.backend) | $(MdEsc $_.metric) | $(MdEsc $_.value) | $(GateLabel (Get-QaVerdict $_.pass)) | $(MdEsc $_.detail) |"
  })
  $scaleSection = @"

## 9. Scale and aggression drills (phase 10)

| Drill | Backend | Metric | Value | Verdict | Detail |
|---|---|---|---|---|---|
$($scaleRows -join "`n")
"@
}

# ---- drill detail for the phases whose TSVs the report previously ignored ----
$drillSection = ""
$drillSets = @(
  @{ Name = "Branching and history (05)"; Rows = $branching; Cols = @("check", "op", "pass", "detail") },
  @{ Name = "Fault injection / abuse (07)"; Rows = $abuse; Cols = @("drill", "pass", "detail") },
  @{ Name = "Auth e2e - HTTP (07)"; Rows = $authRes; Cols = @("drill", "pass", "detail") },
  @{ Name = "Auth CLI (07)"; Rows = $cliAuth; Cols = @("drill", "pass", "detail") },
  @{ Name = "Credential handling (07)"; Rows = $credsRes; Cols = @("drill", "pass", "detail") },
  @{ Name = "First-run setup (07)"; Rows = $setupRes; Cols = @("drill", "pass", "detail") },
  @{ Name = "User management (07)"; Rows = $usersRes; Cols = @("drill", "pass", "detail") }
) | Where-Object { $_.Rows.Count -gt 0 }

if ($drillSets.Count -gt 0) {
  $blocks = @($drillSets | ForEach-Object {
    $cols = $_.Cols
    $body = @($_.Rows | ForEach-Object {
      $row = $_
      "| " + (($cols | ForEach-Object {
        if ($_ -eq "pass") { GateLabel (Get-QaVerdict $row.$_ ) } else { MdEsc $row.$_ }
      }) -join " | ") + " |"
    })
    @"
### $($_.Name)

| $($cols -join " | ") |
| $(($cols | ForEach-Object { "---" }) -join " | ") |
$($body -join "`n")
"@
  })
  $drillSection = @"

## 10. Drill detail

$($blocks -join "`n`n")
"@
}

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
$report = $report + $scaleSection + $drillSection
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

# The report phase's exit code must equal the CAMPAIGN verdict it just computed.
# It previously exited 0 whenever summary.json happened to parse, so the last phase
# of a failing campaign reported success - the single most misleading signal here.
# Failures/skips are re-stated as this phase's own gates so Exit-QaPhase (which reads
# gates.tsv) reaches the same conclusion the report prints.
Write-QaGate $Phase "campaign-no-gate-failures" ($totalFail -eq 0) "failed=$totalFail of $($gates.Count) gates"
Write-QaGate $Phase "campaign-verified-something" ($totalPass -gt 0) $counts
if ($totalSkip -gt 0) {
  Write-QaLog $Phase "NOTE $totalSkip gate(s) were SKIPPED - they were not checked and are not passes"
}

Exit-QaPhase $Phase
