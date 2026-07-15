# 08_perf.ps1 - latency/throughput micro-phase: add/commit timing across size classes,
# plus [bench] line parsing (format per dev-tests/deep-tests/diff_bench.ps1, read-only reference).
param(
  [string]$Baseline = ""   # path to a prior 09_report.ps1 summary.json; when given, >25% regressions on
                            # throughput/wall-clock bench fields fail the gate. Omitted => informational only.
)
. (Join-Path $PSScriptRoot "lib\common.ps1")
$Phase = "08_perf"

$TIMING_OUT = Join-Path $QA.Logs "perf.tsv"
$TIMING_HEADER = @("sizeMB", "op", "sec")
$BENCH_OUT = Join-Path $QA.Logs "perf-bench.tsv"
$BENCH_HEADER = @("sizeMB", "op", "field", "value", "baseline")

Write-QaLog $Phase "perf run starting, tier=$($QA.Tier), baseline='$Baseline'"

# ---- size classes: 1MB / 10MB / 100MB / cap. Cap clamped to 500MB even in STRESS tier -
# this phase samples near-cap latency, it isn't the place to redo full stress-size sweeps. ----
$capMB = [math]::Min($QA.MaxFixtureMB, 500)
$sizeClassesMB = @(1, 10, 100, $capMB) | Where-Object { $_ -le $capMB } | Sort-Object -Unique

function New-SyntheticFile([string]$Path, [int]$SizeMB) {
  # ponytail: fixed seed so content is reproducible run-to-run (stable baseline diffs),
  # but each 1MB block is freshly drawn so the file isn't one repeated dedup-trivial block.
  $rng = [Random]::new(42)
  $buf = New-Object byte[] (1MB)
  $fs = [System.IO.File]::Open($Path, [System.IO.FileMode]::Create)
  try {
    for ($i = 0; $i -lt $SizeMB; $i++) {
      $rng.NextBytes($buf)
      $fs.Write($buf, 0, $buf.Length)
    }
  } finally {
    $fs.Close()
  }
}

function Parse-BenchLines([string]$Text) {
  # [bench] goes to stderr; Invoke-MG's 2>&1 wraps it in PS NativeCommandError rendering:
  # prefixed "mediagit.exe : ", wrapped at console width, followed by "At line..."/"+ ..." decoration.
  # Slice from each [bench] marker to the next decoration line, unwrap, then parse key=value pairs.
  $records = @()
  $chunks = $Text -split '\[bench\]'
  for ($ci = 1; $ci -lt $chunks.Count; $ci++) {
    $seg = ($chunks[$ci] -split "`r?`nAt |`r?`n\s*\+ ")[0] -replace "`r?`n", " "
    $h = @{}
    foreach ($m in [regex]::Matches($seg, '(\w+)=([^\s]+)')) { $h[$m.Groups[1].Value] = $m.Groups[2].Value }
    if ($h['op']) { $records += $h }
  }
  return $records
}

# fields eligible for baseline regression gating, and which direction is "better"
$HIGHER_IS_BETTER = @('throughput_mbs', 'util_pct', 'hash_mbs')
$LOWER_IS_BETTER  = @('wall', 'active_sum', 'manifest_to_first_byte_ms')
$GATE_FIELDS = $HIGHER_IS_BETTER + $LOWER_IS_BETTER

$baseRows = $null
if ($Baseline) {
  if (Test-Path $Baseline) {
    try {
      $baseJson = Get-Content -Raw $Baseline | ConvertFrom-Json
      $baseRows = @($baseJson.perf)
    } catch {
      Write-QaLog $Phase "could not parse -Baseline '$Baseline' as JSON: $_"
    }
  } else {
    Write-QaLog $Phase "-Baseline '$Baseline' does not exist"
  }
}
function Find-BaselineValue([int]$SizeMB, [string]$Op, [string]$Field) {
  if (-not $baseRows) { return $null }
  $m = $baseRows | Where-Object { $_.sizeMB -eq $SizeMB -and $_.op -eq $Op -and $_.field -eq $Field } | Select-Object -First 1
  if ($m) { return $m.value }
  return $null
}

$regressionCount = 0

foreach ($sizeMB in $sizeClassesMB) {
  $sb = New-SandboxRepo "perf-$sizeMB" $Phase
  $assetPath = Join-Path $sb "asset.bin"
  New-SyntheticFile $assetPath $sizeMB

  $env:MEDIAGIT_BENCH = "1"
  try {
    $addRes = Invoke-MG $sb @("add", "asset.bin") $Phase
    $commitRes = Invoke-MG $sb @("commit", "-m", "perf-$sizeMB") $Phase
  } finally {
    Remove-Item Env:\MEDIAGIT_BENCH -EA SilentlyContinue
  }

  Write-QaRow $TIMING_OUT $TIMING_HEADER @($sizeMB, "add", $addRes.Sec)
  Write-QaRow $TIMING_OUT $TIMING_HEADER @($sizeMB, "commit", $commitRes.Sec)

  # @() guard: with a single bench record PS unrolls the function's return array to the bare hashtable
  $records = @(Parse-BenchLines ($addRes.Out + "`n" + $commitRes.Out))
  foreach ($rec in $records) {
    foreach ($field in $rec.Keys) {
      if ($field -eq "op") { continue }
      $value = $rec[$field]
      $baseVal = Find-BaselineValue $sizeMB $rec['op'] $field
      Write-QaRow $BENCH_OUT $BENCH_HEADER @($sizeMB, $rec['op'], $field, $value, $(if ($null -ne $baseVal) { $baseVal } else { "none" }))

      if ($GATE_FIELDS -contains $field -and $null -ne $baseVal) {
        $cur = 0.0; $base = 0.0
        # strip trailing s/% units (wall=0.02s, util_pct=89%) - same normalization as diff_bench.ps1
        $curStr = "$value" -replace '[s%]$', ''
        $baseStr = "$baseVal" -replace '[s%]$', ''
        if ([double]::TryParse($curStr, [ref]$cur) -and [double]::TryParse($baseStr, [ref]$base) -and $base -ne 0) {
          $deltaPct = (($cur - $base) / [Math]::Abs($base)) * 100.0
          $regressed = ($HIGHER_IS_BETTER -contains $field -and $deltaPct -lt -25) -or
                       ($LOWER_IS_BETTER -contains $field -and $deltaPct -gt 25)
          if ($regressed) {
            $regressionCount++
            Write-QaLog $Phase "REGRESSION sizeMB=$sizeMB op=$($rec['op']) field=$field baseline=$baseVal current=$value delta=$([math]::Round($deltaPct,1))%"
          }
        }
      }
    }
  }
}

if ($Baseline -and $baseRows) {
  Write-QaGate $Phase "baseline-regression" ($regressionCount -eq 0) "count=$regressionCount baseline=$Baseline"
} else {
  Write-QaGate $Phase "baseline-regression" $true "no baseline supplied or unreadable - informational only"
}

Write-QaLog $Phase "done: sizeClasses=$($sizeClassesMB -join ',') regressions=$regressionCount"
if ($regressionCount -eq 0) { exit 0 } else { exit 1 }
