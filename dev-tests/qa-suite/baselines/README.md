# Perf baselines

`perf.tsv` is the comparison point for `08_perf.ps1`. It is **not** written automatically:
a baseline that re-created itself every run would happily adopt whatever regression it just
measured.

To promote a baseline after a campaign you trust:

```powershell
Copy-Item dev-tests\qa-suite\logs\<run-id>\perf-bench.tsv dev-tests\qa-suite\baselines\perf.tsv
```

`run_all.ps1` passes this file to `08_perf.ps1` automatically when it exists. Without it the
perf gate records `WARN` (informational) and the phase stays green.

# Coverage-placeholder baseline

`coverage-placeholders.tsv` is the ceiling for `02_matrix.ps1`'s
`coverage-placeholder-regression` gate: the count of `COVERED-BY:` rows in
`coverage_matrix.tsv` (a placeholder that names a phase instead of asserting anything)
must not exceed the recorded `value`. Unlike `perf.tsv`, missing/unreadable is a
**hard fail** here, not a WARN - a placeholder count is not a first-run baseline
problem, and letting it slide would recreate the exact silent-growth bug this gate
exists to catch.

To re-lock after deliberately converting placeholders to real assertions (lowering
the count) or adding new placeholder rows (which should be rare and reviewed):

```powershell
# edit the `value` column in coverage-placeholders.tsv to the new count
```
