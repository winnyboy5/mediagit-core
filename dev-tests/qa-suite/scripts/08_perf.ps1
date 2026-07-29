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

# fields eligible for baseline regression gating, and which direction is "better"
$HIGHER_IS_BETTER = @('throughput_mbs', 'util_pct', 'hash_mbs')
$LOWER_IS_BETTER  = @('wall', 'active_sum', 'manifest_to_first_byte_ms')
$GATE_FIELDS = $HIGHER_IS_BETTER + $LOWER_IS_BETTER

# Regression threshold, in percent, against the baseline value for the same
# (sizeMB, op, field). Tighter than the old 25%: a 10% throughput loss on this
# machine class is well outside run-to-run noise and is worth a human looking.
$REGRESSION_PCT = 10.0

# Minimum wall time, in seconds, a record must have taken before a percentage
# claim about it is allowed to fail the gate.
#
# The 1 MB size class runs `add` in ~20 ms. At that scale a 10% threshold is
# measuring the Windows scheduler: a 60 ms wall (+200%) and a hash_mbs 20%
# below baseline are the same 40 ms of jitter, and the campaign reported four
# such "regressions" on a machine that had just finished 519 s of heavy I/O.
# Gating on noise is worse than not gating - it trains everyone to ignore the
# gate, so a real regression at 100 MB reads as more of the same.
#
# Every field of a record shares its wall clock (throughput and hash rate are
# derived from it), so the floor is applied per record, not per field. Sub-floor
# records are still written to the TSV; they just cannot fail the gate.
$MIN_GATED_WALL_SEC = 0.5

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
$gatedCount = 0
$recordCount = 0

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
    # Too short to time reliably -> report the numbers, but do not let them
    # fail the gate. See $MIN_GATED_WALL_SEC.
    # Two steps deliberately. Inlining the -replace into the TryParse argument
    # list makes PowerShell read its comma as an argument separator, so TryParse
    # gets three arguments and throws -- the harness swallows that, every record
    # becomes ungatable, and the gate reports PASS having checked nothing. It
    # did exactly that on run 20260729-202252. The normalisation below uses the
    # same two-step shape as the value parse further down; that one was right.
    $wallStr = ("" + $rec['wall']) -replace '[s%]$', ''
    $recWall = 0.0
    $gatable = [double]::TryParse($wallStr, [ref]$recWall) -and
               $recWall -ge $MIN_GATED_WALL_SEC
    $recordCount++
    if ($gatable) { $gatedCount++ } else {
      Write-QaLog $Phase "ungated sizeMB=$sizeMB op=$($rec['op']) wall=$($rec['wall']) below $MIN_GATED_WALL_SEC s floor"
    }
    foreach ($field in $rec.Keys) {
      if ($field -eq "op") { continue }
      $value = $rec[$field]
      $baseVal = Find-BaselineValue $sizeMB $rec['op'] $field
      Write-QaRow $BENCH_OUT $BENCH_HEADER @($sizeMB, $rec['op'], $field, $value, $(if ($null -ne $baseVal) { $baseVal } else { "none" }))

      if ($gatable -and $GATE_FIELDS -contains $field -and $null -ne $baseVal) {
        $cur = 0.0; $base = 0.0
        # strip trailing s/% units (wall=0.02s, util_pct=89%) - same normalization as diff_bench.ps1
        $curStr = "$value" -replace '[s%]$', ''
        $baseStr = "$baseVal" -replace '[s%]$', ''
        if ([double]::TryParse($curStr, [ref]$cur) -and [double]::TryParse($baseStr, [ref]$base) -and $base -ne 0) {
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
  if ($gatedCount -eq 0) {
    Write-QaGate $Phase "baseline-regression" $false `
      "gated 0 of $recordCount records against $Baseline - the gate measured nothing (min-wall=${MIN_GATED_WALL_SEC}s)"
  } else {
  Write-QaGate $Phase "baseline-regression" ($regressionCount -eq 0) `
    "count=$regressionCount gated=$gatedCount/$recordCount threshold=$REGRESSION_PCT% min-wall=${MIN_GATED_WALL_SEC}s baseline=$Baseline"
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
