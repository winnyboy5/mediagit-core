# Post-run cleanup of the CLOUD test stores -- run after every GA campaign.
#
# WHY. The campaign pushes multi-GB repositories to AWS, GCS and Azure on every
# run and nothing ever deleted them, so test data accumulated as a standing
# storage bill. These stores hold test data only (decided 2026-10-19): nothing
# in them is kept between runs.
#
# WHY A SEPARATE SCRIPT, NOT A STEP INSIDE THE CAMPAIGN. A destructive step
# inside a 3-hour gate turns a re-run of that gate into an accidental wipe.
# The launcher calls this after run_ga.ps1 returns, like
# clean_before_run_native.ps1 is called before.
#
# SAFETY, in the order the checks run:
#   1. Every target must be on the allowlist below. The names come from
#      campaign_env.ps1, which reads gitignored config files; if those files
#      ever point at a different bucket, this script refuses instead of
#      emptying it. Change the allowlist deliberately, never to make a run pass.
#   2. Refuses while a mediagit server or client is running -- a campaign that
#      is still writing would be emptied mid-run.
#   3. Deletes OBJECTS only. Buckets and the container stay, because a
#      campaign against a missing bucket fails like a product bug.
#   4. Verifies empty by re-listing, never by trusting a delete's exit code.
#   5. Never puts a credential on a command line. az and the AWS CLI read
#      their keys from the environment; an az argument error echoes its whole
#      command line, which is how a storage key once reached a session log.
#   6. -DryRun lists what would go and deletes nothing.
#
# GCS needs two workarounds, both scoped to this process only:
#   - Avast HTTPS scanning re-signs storage.googleapis.com, which gcloud's
#     bundled Python rejects (CERTIFICATE_VERIFY_FAILED). The Windows root
#     store, which includes Avast's root, is exported to a PEM bundle and
#     passed via CLOUDSDK_CORE_CUSTOM_CA_CERTS_FILE.
#   - The user's own gcloud login expires. A throwaway CLOUDSDK_CONFIG dir is
#     activated with the campaign's service-account key instead, so the
#     user's gcloud configuration is never touched.
#
# Exit codes: 0 clean, 80 target not on allowlist, 81 campaign still running,
# 82 a store could not be listed, 83 objects left after delete.

param([switch]$DryRun)

$ErrorActionPreference = "Continue"
$scripts = $PSScriptRoot
. (Join-Path $scripts "campaign_env.ps1") *> $null

function Say([string]$m) { Write-Host ("[cloud-clean] " + $m) }

$allow = @{
  aws   = "mediagit-s3-dev-bucket"
  gcs   = "mediagit-dev2-storage"
  azure = "mediagit-server-repos"
}
$targets = @{
  aws   = $env:MG_QA_AWS_BUCKET
  gcs   = $env:GCS_BUCKET_NAME
  azure = $env:MG_QA_AZURE_CONTAINER
}

# ---- 1. allowlist ---------------------------------------------------------
foreach ($k in $allow.Keys) {
  if ($targets[$k] -ne $allow[$k]) {
    Say "FATAL: $k target '$($targets[$k])' is not the allowlisted '$($allow[$k])' - refusing"
    exit 80
  }
}

# ---- 2. nothing may still be writing --------------------------------------
$busy = Get-Process mediagit-server, mediagit -EA SilentlyContinue
if ($busy) {
  Say "FATAL: $(@($busy).Count) mediagit process(es) still running (pids $(($busy.Id) -join ',')) - refusing"
  exit 81
}

# ---- GCS setup (process-scoped) -------------------------------------------
$gcfg = Join-Path $env:TEMP "mg-cloud-clean-gcloud"
New-Item -ItemType Directory -Force $gcfg | Out-Null
$pem = Join-Path $gcfg "windows-roots.pem"
$sb = New-Object System.Text.StringBuilder
foreach ($store in "Cert:\LocalMachine\Root", "Cert:\CurrentUser\Root") {
  foreach ($c in Get-ChildItem $store -EA SilentlyContinue) {
    [void]$sb.AppendLine("-----BEGIN CERTIFICATE-----")
    [void]$sb.AppendLine([Convert]::ToBase64String($c.RawData, "InsertLineBreaks"))
    [void]$sb.AppendLine("-----END CERTIFICATE-----")
  }
}
[IO.File]::WriteAllText($pem, $sb.ToString())
$env:CLOUDSDK_CONFIG = $gcfg
$env:CLOUDSDK_CORE_CUSTOM_CA_CERTS_FILE = $pem
$env:CLOUDSDK_CORE_DISABLE_PROMPTS = "1"
if ($env:GOOGLE_APPLICATION_CREDENTIALS) {
  gcloud auth activate-service-account --key-file="$env:GOOGLE_APPLICATION_CREDENTIALS" *> $null
}

# ---- listing / deleting, one function per store ----------------------------
function Count-Objects([string]$store) {
  switch ($store) {
    "aws" {
      # `s3 ls --summarize` pages through everything; list-objects-v2 stops at
      # 1000 keys, and its JMESPath `length(Contents)` errors on an empty bucket.
      $out = aws s3 ls "s3://$($targets.aws)" --recursive --summarize 2>$null
      if ($LASTEXITCODE -ne 0) { return $null }
      $line = $out | Where-Object { $_ -match "Total Objects:\s*(\d+)" } | Select-Object -Last 1
      if (-not $line) { return $null }
      return [int]([regex]::Match($line, "\d+").Value)
    }
    "gcs" {
      $out = gcloud storage ls -r "gs://$($targets.gcs)/**" 2>&1
      if ($LASTEXITCODE -ne 0) {
        if (($out | Out-String) -match "matched no objects|One or more URLs matched no objects") { return 0 }
        return $null
      }
      return @($out | Where-Object { $_ -match "^gs://" -and $_ -notmatch "/$" }).Count
    }
    "azure" {
      # Credentials come from AZURE_STORAGE_ACCOUNT / AZURE_STORAGE_KEY in the
      # environment, never from an argument.
      $out = az storage blob list --container-name $targets.azure --num-results "*" --query "[].name" -o tsv --only-show-errors 2>$null
      if ($LASTEXITCODE -ne 0) { return $null }
      return @($out | Where-Object { $_ }).Count
    }
  }
}

function Remove-Objects([string]$store) {
  switch ($store) {
    "aws"   { aws s3 rm "s3://$($targets.aws)" --recursive --only-show-errors 2>&1 | Select-Object -Last 3 }
    "gcs"   { gcloud storage rm "gs://$($targets.gcs)/**" --quiet 2>&1 | Select-Object -Last 3 }
    "azure" { az storage blob delete-batch --source $targets.azure --only-show-errors 2>&1 | Select-Object -Last 3 }
  }
}

# ---- 3 + 4. delete, then verify by re-listing ------------------------------
$failed = $false
foreach ($s in "aws", "gcs", "azure") {
  $before = Count-Objects $s
  if ($null -eq $before) { Say "FATAL: could not list $s '$($targets[$s])'"; $failed = $true; continue }
  Say "$s '$($targets[$s])' - $before object(s) before"
  if ($before -eq 0) { continue }
  if ($DryRun) { Say "  dry run - nothing deleted"; continue }
  Remove-Objects $s | ForEach-Object { Say "  $_" }
  $after = Count-Objects $s
  if ($null -eq $after) { Say "FATAL: could not re-list $s after delete"; $failed = $true; continue }
  Say "$s - $after object(s) after"
  if ($after -ne 0) { Say "FATAL: $s still holds $after object(s)"; exit 83 }
}
Remove-Item $gcfg -Recurse -Force -EA SilentlyContinue
if ($failed) { exit 82 }
Say "CLOUD_CLEAN_DONE$(if ($DryRun) { ' (dry run)' })"
