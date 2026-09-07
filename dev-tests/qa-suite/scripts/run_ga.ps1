# The formal GA gate run: every phase, one campaign, start to finish.
#
# todo.md:263 is explicit about the bar - the only formal GA gate on record is
# st2-scale-04 (2026-07-27, 20/20 phases, local+MinIO only). Everything since
# has been targeted verification of specific fixes. It requires: freshly built
# release binaries, the FULL run_all (not a -Phases subset), and
# project_ga_progress.md updated with the result.
#
# The phase list is spelled out rather than left to run_all's default because
# the default omits 11_memprofile - which is precisely why 11 had never run
# once. Order otherwise matches the default SCALE ordering: 10 and 12/13 before
# 09, so the report aggregates them. 11 slots after 10; it samples peak RSS
# process-wide and must not share the machine with another phase, which
# sequential execution already guarantees.
#
# Run scratchpad\clean_before_run.ps1 FIRST. It is not called from here on
# purpose: it kills leftover servers and drops the MinIO volume, and folding a
# destructive step into the same script as a 3h gate makes a re-run of the gate
# an accidental wipe.

param([string]$RunId = "20260820-ga1")

$ErrorActionPreference = "Continue"
Set-Location "D:\own\saas\mediagit-core\dev-tests\qa-suite\scripts"

Add-Type -Name Power -Namespace Win32 -MemberDefinition @"
    [DllImport("kernel32.dll", SetLastError = true)]
    public static extern uint SetThreadExecutionState(uint esFlags);
"@
$ES_CONTINUOUS       = [uint32]2147483648
$ES_SYSTEM_REQUIRED  = [uint32]1
$ES_DISPLAY_REQUIRED = [uint32]2
$want = $ES_CONTINUOUS -bor $ES_SYSTEM_REQUIRED -bor $ES_DISPLAY_REQUIRED
$first  = [Win32.Power]::SetThreadExecutionState($want)
$second = [Win32.Power]::SetThreadExecutionState($want)
if ($first -eq 0 -or -not (($second -band $ES_SYSTEM_REQUIRED) -and ($second -band $ES_DISPLAY_REQUIRED))) {
    Write-Host ("FATAL: sleep block did not arm (first=0x{0:X8} second=0x{1:X8})." -f $first, $second)
    exit 90
}
Write-Host ("sleep blocked: OS echoed 0x{0:X8}  pid={1}" -f $second, $PID)

# Binaries must be the ones the gate is certifying, and they must postdate the
# fixes under test. Stated in the log so the run's own record answers "which
# binary was this?" without anyone reconstructing it later.
Write-Host "`n=== binaries under test ==="
foreach ($b in @("..\..\..\target\release\mediagit.exe", "..\..\..\target\release\mediagit-server.exe")) {
    if (Test-Path $b) {
        $f = Get-Item $b
        Write-Host ("{0}  {1:N0} bytes  built {2}" -f $f.Name, $f.Length, $f.LastWriteTime)
    } else { Write-Host "FATAL: MISSING $b"; exit 95 }
}
Write-Host ("HEAD = " + (git -C "D:\own\saas\mediagit-core" rev-parse --short HEAD))

$startedAt = Get-Date

# Dot-sourced, never executed: it sets cloud credentials into this process.
. .\campaign_env.ps1

$env:MG_QA_TIER   = "SCALE"
$env:MG_QA_RUN_ID = $RunId

# Measure the link for the whole run, per backend. The WLAN check below only
# sees full disconnects: ga41 and ga42 both reported "the link held" while
# packets were dropping and, in ga42, s3.ap-south-1 was gone for four minutes.
# Without this, a bad-network run and a bad-code run read identically in the
# record. Started AFTER campaign_env so the Azure account name is resolved, and
# it only records - it never trips or fails a phase.
. .\lib\linkprobe.ps1
$logDir = "..\logs\$RunId"
New-Item -ItemType Directory -Path $logDir -Force | Out-Null
Start-QaLinkProbe -LogDir $logDir -Hosts (Get-QaLinkHosts)

$sw = [Diagnostics.Stopwatch]::StartNew()
# -Command, not -File: under -File every argument is a literal string, so a
# comma-separated -Phases list arrives as ONE token and run_all rejects it.
powershell -NoProfile -Command "& '.\run_all.ps1' -Phases @('00','01','02','03','04','05','06','07','08','10','11','12','13','09') -ContinueOnFail"
$code = $LASTEXITCODE
$sw.Stop()
Stop-QaLinkProbe

Write-Host ""
Write-Host ("RUN_ALL_EXIT=$code  elapsed={0:hh\:mm\:ss}" -f $sw.Elapsed)

$summary = Join-Path $logDir "run_all-summary.tsv"
if (Test-Path $summary) {
    Write-Host "`n=== phases ==="
    Get-Content $summary
}

$gates = Join-Path $logDir "gates.tsv"
if (Test-Path $gates) {
    # The column is `pass`, holding True/False/SKIP - there is no `result`
    # column. Reading one printed "pass=0 fail=0 skip=0" for a 229-gate run,
    # i.e. a summary that reports all-clear no matter what happened. Exactly the
    # failure mode this suite keeps finding in its own gates.
    $rows = Import-Csv $gates -Delimiter "`t"
    $pass = @($rows | Where-Object { $_.pass -eq "True" }).Count
    $fail = @($rows | Where-Object { $_.pass -eq "False" }).Count
    $skip = @($rows | Where-Object { $_.pass -eq "SKIP" }).Count
    if (($pass + $fail + $skip) -ne $rows.Count) {
        Write-Host "WARNING: gate tally ($pass+$fail+$skip) does not cover all $($rows.Count) rows - the pass column holds a value this summary does not understand"
    }
    Write-Host "`n=== gates: pass=$pass fail=$fail skip=$skip ==="
    if ($fail -gt 0) { $rows | Where-Object { $_.result -eq "FAIL" } | Format-Table -AutoSize | Out-String | Write-Host }
}

# A green run that slept is not a green run - the wedge that voided gagate13
# stayed silent for 41 minutes and every phase before it still read PASS.
Write-Host "`n=== power events during the run (expect none) ==="
$slept = Get-WinEvent -FilterHashtable @{
    LogName = 'System'; Id = 42, 107, 187, 506, 507; StartTime = $startedAt
} -ErrorAction SilentlyContinue
if ($slept) {
    Write-Host "WARNING: the machine changed power state during this run:"
    $slept | Select-Object TimeCreated, Id | Format-Table -AutoSize | Out-String | Write-Host
} else { Write-Host "none - the run was continuous." }

# ga31 was voided by SEVEN Wi-Fi drops and the run itself said nothing about
# it -- the phases just failed, which reads exactly like a product regression
# until someone thinks to check the event log. This box is Wi-Fi only, and 16
# drops landed in one 3-minute burst earlier today, so the condition is live
# rather than hypothetical. Report it here so the run's own record answers
# "was this the product or the link?" before anyone starts bisecting.
Write-Host "`n=== WLAN disconnects during the run (expect none) ==="
$drops = Get-WinEvent -FilterHashtable @{
    LogName = 'Microsoft-Windows-WLAN-AutoConfig/Operational'; Id = 8003; StartTime = $startedAt
} -ErrorAction SilentlyContinue
if ($drops) {
    Write-Host ("WARNING: the link dropped {0} time(s) during this run." -f @($drops).Count)
    Write-Host "Cloud-backend failures in this campaign are SUSPECT - treat the run as VOID"
    Write-Host "unless the failing phases are local/minio only."
    $drops | Select-Object TimeCreated | Format-Table -AutoSize | Out-String | Write-Host
} else { Write-Host "none - no full disconnect (this does NOT mean the link was good - see below)." }

# What the 8003 check above cannot see: loss, latency and per-host outages that
# never disconnect the adapter. Read this BEFORE bisecting a cloud failure --
# a BAD arm here whose outage window covers the failing phase's timestamp means
# the run is suspect for that arm, and a clean report is evidence the failure
# was the product.
Write-Host "`n=== link quality per backend during the run ==="
Write-QaLinkSummary -LogDir $logDir

[void][Win32.Power]::SetThreadExecutionState($ES_CONTINUOUS)
Write-Host "`nGA_DONE exit=$code"
