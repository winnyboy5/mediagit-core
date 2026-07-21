<!--
qa-suite REPORT template. Filled by scripts/09_report.ps1 via literal token replacement.
Tokens (all appear exactly once unless noted "repeated"):

  {{RUN_ID}}                 - $QA.RunId
  {{RUN_DATE}}                - human-readable date the report was generated
  {{TIER}}                    - $QA.Tier (STANDARD | STRESS)
  {{MG_VERSION}}              - output of `mediagit version`
  {{BACKENDS}}                - comma-joined $QA.Backends actually exercised this run
  {{VERDICT}}                 - short prose verdict (PASS / PASS-WITH-FINDINGS / FAIL + one-liner why)
  {{GATE_TABLE_ROWS}}         - markdown table rows, one per phase gate (from gates.tsv)
  {{FINDINGS_TABLE_ROWS}}     - markdown table rows, one per registered finding
  {{STORAGE_ECONOMICS_ROWS}}  - markdown table rows: fixture/chain vs bytes saved / % dedup
  {{REMOTE_THROUGHPUT_ROWS}}  - markdown table rows: backend vs push/pull MB/s, latency
  {{PERF_TABLE_ROWS}}         - markdown table rows: operation vs wall time / RAM peak
  {{IMPROVEMENT_SUGGESTIONS}} - free-form markdown list, filled by 09_report.ps1 or left as TODO
  {{METHODOLOGY_NOTES}}       - free-form markdown, run-specific methodology notes/deviations

If a section has no data for a given run, 09_report.ps1 should fill the *_ROWS token with a
single row: "| _none_ | | | | |" (column count matched to that table) rather than leaving the
literal token in place.
-->
# MediaGit QA Suite - Report

**Run:** {{RUN_ID}} | **Date:** {{RUN_DATE}} | **Tier:** {{TIER}} | **Build:** {{MG_VERSION}} | **Backends:** {{BACKENDS}}

---

## 1. Verdict

{{VERDICT}}

---

## 2. Per-phase gate results

| Phase | Gate | Pass | Detail |
|---|---|---|---|
{{GATE_TABLE_ROWS}}

---

## 3. Findings registry

| ID | Severity | Area | Summary | Repro | Status |
|---|---|---|---|---|---|
{{FINDINGS_TABLE_ROWS}}

---

## 4. Storage economics

| Fixture / chain | Raw bytes | Stored bytes | Saved % | Notes |
|---|---|---|---|---|
{{STORAGE_ECONOMICS_ROWS}}

---

## 5. Remote throughput

| Backend | Operation | Size | Throughput | Latency | Notes |
|---|---|---|---|---|---|
{{REMOTE_THROUGHPUT_ROWS}}

---

## 6. Performance

| Operation | Wall time | Peak RAM | Notes |
|---|---|---|---|
{{PERF_TABLE_ROWS}}

---

## 7. Improvement suggestions

{{IMPROVEMENT_SUGGESTIONS}}

---

## 8. Methodology

{{METHODOLOGY_NOTES}}
