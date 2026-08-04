# 08_perf.ps1 - latency/throughput micro-phase: add/commit timing across size classes,
# plus [bench] line parsing (format per dev-tests/deep-tests/diff_bench.ps1, read-only reference).
param(
  # Path to a promoted baseline. Two accepted shapes:
  #   *.tsv  - a perf-bench.tsv promoted from a known-good campaign (the normal case;
  #            run_all passes baselines\perf.tsv automatically when it exists)
  #   *.json - a prior 09_report.ps1 summary.json (its .perf array holds the same rows)
  # Omitted or unreadable => the gate records WARN and the phase stays green: the very
  # first campaign has nothing to compare against, and refusing to run would mean the
  # baseline could never be created. A regression past $REGRESSION_PCT fails the gate.
  [string]$Baseline = ""
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

# fields eligible for baseline regression gating, and which direction is "better".
#
# `hash_mbs` is deliberately NOT gated: `bench.rs` computes it as
# `total_bytes / wall_s`, so it is `wall` restated as a rate, not an independent
# measurement. Gating both made every regression count twice -- a single 500 MB
# slowdown reported as "wall +75.7%" AND "hash_mbs -42.8%", which are the same
# fact (366.59 x 1.36/2.39 = 208.6). It is still written to the TSV, where a rate
# is the easier number to reason about.
$HIGHER_IS_BETTER = @('throughput_mbs', 'util_pct')
$LOWER_IS_BETTER  = @('wall', 'active_sum', 'manifest_to_first_byte_ms')
$GATE_FIELDS = $HIGHER_IS_BETTER + $LOWER_IS_BETTER

# Regression threshold, in percent, against the baseline value for the same
# (sizeMB, op, field). Tighter than the old 25%: a 10% throughput loss on this
# machine class is well outside run-to-run noise and is worth a human looking.
$REGRESSION_PCT = 10.0

# Smallest input size, in MB, whose timings may fail the gate.
#
# The 1 MB class runs `add` in ~30 ms; a 10% threshold there measures the Windows
# scheduler, and the campaign once reported four such "regressions" that were
# 20-60 ms of jitter. Gating on noise is worse than not gating -- it trains
# everyone to ignore the gate, so a real regression reads as more of the same.
#
# Keyed on INPUT SIZE, not measured wall time. A wall-time floor is circular: it
# uses the measurement to decide whether to trust the measurement, so the same
# 100 MB workload was gated at 0.50s and skipped at 0.47s on two consecutive
# runs. Input size is deterministic and decides the same way every time.
#
# Sub-floor records are still written to the TSV; they just cannot fail.
$MIN_GATED_SIZE_MB = 100

$baseRows = $null
$baselineNote = ""
if ($Baseline) {
  if (Test-Path $Baseline) {
    try {
      if ($Baseline -match '\.tsv$') {
        # promoted perf-bench.tsv: sizeMB, op, field, value, baseline
        $lines = Get-Content $Baseline
        if ($lines -and $lines.Count -ge 2) { $baseRows = @($lines | ConvertFrom-Csv -Delimiter "`t") }
        if (-not $baseRows) { $baselineNote = "baseline TSV '$Baseline' has no data rows" }
      } else {
        $baseJson = Get-Content -Raw $Baseline | ConvertFrom-Json
        $baseRows = @($baseJson.perf)
        if (-not $baseRows) { $baselineNote = "baseline JSON '$Baseline' has no .perf rows" }
      }
    } catch {
      $baselineNote = "could not parse -Baseline '$Baseline': $_"
    }
  } else {
    $baselineNote = "-Baseline '$Baseline' does not exist"
  }
} else {
  $baselineNote = "no -Baseline supplied (promote a green campaign's perf-bench.tsv to baselines\perf.tsv)"
}
if ($baselineNote) { Write-QaLog $Phase $baselineNote }
function Find-BaselineValue([int]$SizeMB, [string]$Op, [string]$Field) {
  if (-not $baseRows) { return $null }
  $m = $baseRows | Where-Object { $_.sizeMB -eq $SizeMB -and $_.op -eq $Op -and $_.field -eq $Field } | Select-Object -First 1
  if ($m) { return $m.value }
  return $null
}

$regressionCount = 0
$gatedCount = 0        # size-eligible records (>= $MIN_GATED_SIZE_MB)
$comparedCount = 0     # records actually held against a baseline value
$recordCount = 0
$missingBaseline = @() # size/op/field combos eligible to gate but absent from the baseline

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
    # Too small to support a percentage claim -> report, but do not gate.
    # See $MIN_GATED_SIZE_MB.
    $gatable = $sizeMB -ge $MIN_GATED_SIZE_MB
    $recordCount++
    if ($gatable) { $gatedCount++ } else {
      Write-QaLog $Phase "ungated sizeMB=$sizeMB op=$($rec['op']) wall=$($rec['wall']) (below ${MIN_GATED_SIZE_MB}MB gating floor)"
    }
    foreach ($field in $rec.Keys) {
      if ($field -eq "op") { continue }
      $value = $rec[$field]
      $baseVal = Find-BaselineValue $sizeMB $rec['op'] $field
      Write-QaRow $BENCH_OUT $BENCH_HEADER @($sizeMB, $rec['op'], $field, $value, $(if ($null -ne $baseVal) { $baseVal } else { "none" }))

      # A record can be size-eligible ($gatable) and still never be compared,
      # because the baseline has no row for this (sizeMB, op, field). That is
      # invisible in `gated=N/M`, which counts eligibility only: when the
      # baseline carried zero `commit` rows, every campaign still reported
      # gated=4/8 while comparing 2 add records and nothing else. Track the
      # combinations that were eligible but unbacked so a missing baseline row
      # is loud instead of silent.
      if ($gatable -and $GATE_FIELDS -contains $field -and $null -eq $baseVal) {
        $missingBaseline += "$sizeMB/$($rec['op'])/$field"
      }

      if ($gatable -and $GATE_FIELDS -contains $field -and $null -ne $baseVal) {
        $cur = 0.0; $base = 0.0
        # strip trailing s/% units (wall=0.02s, util_pct=89%) - same normalization as diff_bench.ps1
        $curStr = "$value" -replace '[s%]$', ''
        $baseStr = "$baseVal" -replace '[s%]$', ''
        if ([double]::TryParse($curStr, [ref]$cur) -and [double]::TryParse($baseStr, [ref]$base) -and $base -ne 0) {
          # Counted HERE, not at $gatable: this is the only point at which a
          # number was actually held against the baseline. A parse failure
          # above silently skips the comparison, which is how the 2026-07-29
          # dead gate passed while checking nothing.
          $comparedCount++
          $deltaPct = (($cur - $base) / [Math]::Abs($base)) * 100.0
          $regressed = ($HIGHER_IS_BETTER -contains $field -and $deltaPct -lt -$REGRESSION_PCT) -or
                       ($LOWER_IS_BETTER -contains $field -and $deltaPct -gt $REGRESSION_PCT)
          if ($regressed) {
            $regressionCount++
            Write-QaLog $Phase "REGRESSION sizeMB=$sizeMB op=$($rec['op']) field=$field baseline=$baseVal current=$value delta=$([math]::Round($deltaPct,1))%"
          }
        }
      }
    }
  }
}

if ($baseRows) {
  # A gate that measured nothing must not report PASS. The noise floor is
  # supposed to exclude the *small* size classes, not all of them -- if not one
  # record cleared it, either the parse broke (as it did once) or the floor is
  # set above every workload, and both look identical to "no regressions" from
  # the outside. Silence is not success.
  # Report the combinations that could have been gated but had no baseline row.
  # This is what makes a half-covered baseline visible: `commit` had no rows at
  # all for months and every campaign still printed a healthy-looking gated=4/8.
  if ($missingBaseline.Count -gt 0) {
    Write-QaLog $Phase ("NOTE {0} gatable record(s) had NO baseline row and were not compared: {1}" -f `
      $missingBaseline.Count, (($missingBaseline | Sort-Object -Unique) -join ", "))
  }

  # `$comparedCount`, not `$gatedCount`. The two differ whenever the baseline is
  # missing rows, and it is precisely that case the guard has to catch -- a gate
  # that compared nothing must never read as "no regressions".
  if ($comparedCount -eq 0) {
    Write-QaGate $Phase "baseline-regression" $false `
      "compared 0 of $recordCount records against $Baseline - the gate measured nothing (gatable=$gatedCount, min-size=${MIN_GATED_SIZE_MB}MB)"
  } else {
  Write-QaGate $Phase "baseline-regression" ($regressionCount -eq 0) `
    "count=$regressionCount compared=$comparedCount gated=$gatedCount/$recordCount threshold=$REGRESSION_PCT% min-size=${MIN_GATED_SIZE_MB}MB baseline=$Baseline"
  }
} else {
  # WARN, not PASS: nothing was compared. Reported as informational so the first
  # campaign can still go green and produce the numbers the baseline is promoted from.
  Write-QaGate $Phase "baseline-regression" "WARN" $baselineNote
}

Write-QaLog $Phase "done: sizeClasses=$($sizeClassesMB -join ',') regressions=$regressionCount"
# The size-class sandboxes are ~1.2GB of incompressible blob (500+100+10+1 MB, each
# stored again in its ODB) and were being left behind for the next phase to trip over.
Invoke-QaTeardown $Phase @("perf-*")

Exit-QaPhase $Phase
