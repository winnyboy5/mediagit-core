# Live QA campaign (qa-live)

Playwright drives the real release desktop app (WebView2 over CDP) and the release CLI/server on
real media, records frames, an MP4 and a result JSON per case, and a runner turns that into a
pass/fail verdict with cleanup. It is not part of CI; run it deliberately with `dev-tests/qa-suite/scripts/run_qalive.ps1`, which gives a verdict (`gates.tsv`) and cleans up like the GA runner.

## Prerequisites

- Release build, newest first: `pnpm --dir ui build`, then
  `cargo build --release -p mediagit-cli -p mediagit-server -p mediagit-desktop`.
  Global setup refuses binaries or `ui/dist` older than the last product commit (`MG_QA_ALLOW_STALE=1` overrides).
- The `test-files/` corpus (gitignored; or point `MG_QA_TESTFILES` at it). `ffmpeg` on PATH for the MP4 and contact sheet.
- S-13 only: a native Silo/MinIO on `127.0.0.1:9000` with bucket `mediagit-qa-suite`, and `MG_QA_MCLI` pointing at its `mcli.exe`.
  Without `MG_QA_MCLI` S-13 records `SKIP` with a reason (not a failure).
- Cloud cases: credentials come from `dev-tests/qa-suite/scripts/campaign_env.ps1` (dot-sourced by the runner; never printed).

## Run

```powershell
# full campaign (long)
powershell -NoProfile -ExecutionPolicy Bypass -File dev-tests/qa-suite/scripts/run_qalive.ps1
# a subset: -Cases is a comma-separated id list, matched EXACTLY (not inside longer ids, "1,000" or file names)
... run_qalive.ps1 -Cases "000"
... run_qalive.ps1 -Cases "S-01,S-03,T-06b"
# also empty the cloud test stores afterwards (clean_after_run_cloud.ps1, allowlisted)
... run_qalive.ps1 -CleanCloud
```

`-RunId` names the run folder (default `qalive-yyyyMMdd-HHmmss`). `-SkipCleanup` is for debugging only.
Results: `<QA root>\<RunId>\` with `gates.tsv` (case, result, checks, failed, detail), one `<ID>-<slug>.json/.log/.mp4`
per case, `playwright.log`, `playwright.json`. Exit code 0 only when at least one case PASSED, none FAILED (a case with no checks
of its own, a requested id with no result, or a Playwright test that wrote no JSON all count as FAIL), Playwright exited 0 and
cleanup succeeded. 1 verdict FAIL, 2 Playwright non-zero, 3 cleanup failed, 70 another qa-live Playwright run is active,
71 unsafe work path, 72 missing build, 73 bad `-Cases`/`-RunId`. An all-SKIP run is not green. Exit 1 can hide a cleanup failure too: always read the `cleanup=` field of the final `QALIVE_DONE` line. `-CleanCloud` ends 3 while any
mediagit process runs on the machine (`clean_after_run_cloud.ps1` refuses then); the QALIVE_DONE line prints the skip count.

Verdict by hand: `node ui/tests/e2e/qa-live/summarize.mjs <runDir>`. A case is FAIL if any check or ambient check is not ok
(whatever its `result` says), or if Playwright ran it and no JSON exists. PASS-PENDING-REVIEW counts as PASS here; the
contact sheet and MP4 are still for a human to look at.

## Environment variables

| Variable | Meaning | Default |
|---|---|---|
| `MG_QA_ROOT` | results root; its folder name must end in `qa-live` (the runner only ever wipes `<root>\work`) | `<repo>\..\qa-live` |
| `MG_QA_DAY` | run id = results subfolder. The runner sets it; the in-code default (`run-YYYYMMDD-HHMMSS`) is for ad-hoc `playwright test` runs only, because globalSetup and workers are separate processes | generated |
| `MG_QA_TESTFILES` | media corpus (same variable as `dev-tests/qa-suite/config.ps1`) | `<repo>\test-files` |
| `MG_QA_MCLI` | Silo/MinIO `mcli.exe` for S-13 | unset (S-13 SKIPs) |
| `MG_QA_PROFILE` | cargo profile folder under `target\` | `release` |
| `MG_QA_CLI` / `MG_QA_SERVER` / `MG_QA_DESKTOP` | explicit binary paths | `target\<profile>\...` |
| `MG_QA_CDP_PORT` | WebView2 remote-debugging port | `9333` |
| `MG_QA_NO_APP`, `MG_QA_KEEP_APP`, `MG_QA_ALLOW_STALE`, `MG_QA_UI` | skip the app / keep it after / skip the stale-build gate / use vite dev | off |

## What cleanup deletes, and what it keeps

The runner never kills or deletes what it did not start or create. Your own `mediagit`/`mediagit-server`/desktop processes
(a dev server, a push, a GA run) are snapshotted at start and left alone; only another qa-live Playwright run blocks it (exit 70).

- deletes `<QA root>\work` (path asserted to end in `qa-live\work`), nothing else;
- stops this run's processes: the PIDs (and trees) in `work\state.json` (the detached desktop app is there) and descendants of the
  runner itself. Other new `mediagit*` processes are only reported;
- deletes Windows Credential Manager entries `http://127.0.0.1:<port>[/repo].mediagit` / `http://localhost:...` (port >= 49152)
  that did NOT exist when the run started, after writing them to `<QA root>\keychain-ledger.txt`; the next run removes any
  ledger entries a crashed run left. Deletion is verified by re-listing. Your desktop app's own sign-in exists before the run and is kept
  (one that you create in the app while the run is going would be indistinguishable and be removed);
  - caveat, port reuse: a ledger entry left by a crashed run is removed by name, so if your own app later got the same ephemeral
    port the next run would remove that sign-in too (ledger entries are short-lived: written and removed within a run);
  - caveat, app restart: if your own desktop app restarts its local server on a NEW port during a run, that new entry looks new
    and is removed; sign in again. Do not use the desktop app (or restart its server) while a run is going;
  - processes are matched by PID plus recorded start time (`work\state.json` `procs`), so a PID Windows reused is never killed;
- other entries (`encryption-master.mediagit`, remote hosts, other apps) are never touched.

Kept: the run folder `<QA root>\<RunId>` (logs, MP4s, JSONs), `STATUS.md`, the repo, `test-files/`, and cloud data unless
`-CleanCloud` is given.

## C-cloud (AWS S3, GCS, Azure from the app)

`C-cloud.spec.ts` (cases `C-cloud-aws`, `C-cloud-gcs`, `C-cloud-azure`) sets up the app's local server on each cloud from
the storage form and uses it (push, clone, tag, lock, download, restart, broken credential). Credentials come from the
environment (`campaign_env.ps1`) but the spec restarts the app and its server WITHOUT them, so the form is the only
credential source. A backend with a missing variable records SKIP. Objects go under `qa-live/<RunId>/<backend>`; run
`run_qalive.ps1 -Cases "C-cloud-aws,C-cloud-gcs,C-cloud-azure" -CleanCloud` (cleanup wipes the allowlisted dev buckets).

Documented gaps: a repo whose `storage.backend` is set directly in the repo (the app cannot choose it), and the Repos-tab
flows (repo create, clone and download are driven through the CLI against the app's server, as S-13 does).
